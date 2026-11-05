//! Modules: ECMAScript's, as a page's `<script type="module">` and `import()` load them.
//!
//! A module is compiled (compiler.rs's compile_module) into the code that makes its functions, its own code, and
//! what it imports and exports. Its top-level names live in its environment, made as it's parsed; a name it imports
//! is an ImportCell in its environment: the exporting module's environment and slot, read as the code runs (so an
//! import sees the exporter's assignments: a live binding).
//!
//! The host fetches modules. The VM keeps the loads waiting (import()'s, the host's own), says which addresses they
//! want (module_wanted), is given each one's text (module_add) or told it couldn't be had (module_failed), and
//! finishes each load whose modules have all come (module_progress): an import() links and evaluates its module and
//! settles its promise with the module's namespace; a host's load is handed back to it (Modules::ready) to run when
//! it likes (module_link, module_evaluate). A host that has the modules at hand (a test runner reading files) gives
//! a fetch_sync, and each load is finished as soon as it's asked for.
//!
//! Linking resolves each import (ResolveExport, through `export * from` and re-exports, with cycles) and makes each
//! environment; evaluating runs each module's code once, those it imports first. A module that awaits at its top
//! level runs as an async function, and the modules importing it wait for it (ECMAScript 2025's Cyclic Module
//! Records: InnerModuleLinking, InnerModuleEvaluation, ExecuteAsyncModule, AsyncModuleExecutionFulfilled/Rejected).

use crate::compiler::{self, ModuleCode};
use crate::names as n;
use crate::object::{Class, ENUMERABLE, Env, Key, Prop, Slot, VAR, WRITABLE};
use crate::props::Desc;
use crate::string::JsStr;
use crate::value::{Gc, Value};
use crate::vm::{Job, Sym, Vm};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// Parsed (or not: its error is set), not linked.
    New,
    Linking,
    Linked,
    Evaluating,
    /// Run as far as it can go: it waits for an await (its own, or a module's it imports).
    EvaluatingAsync,
    /// Run (or its error is set).
    Evaluated,
}

/// [[AsyncEvaluationOrder]]: not set, and done.
const UNSET: u64 = 0;
const DONE: u64 = u64::MAX;

pub struct Module {
    /// Its address (import.meta.url, and what its imports resolve against).
    pub url: String,
    pub status: Status,
    /// None: it didn't parse (its error is the SyntaxError).
    pub code: Option<Rc<ModuleCode>>,
    /// Each request's address (Err: why its specifier doesn't resolve).
    pub urls: Vec<Result<String, String>>,
    pub env: Option<Gc>,
    pub namespace: Option<Gc>,
    /// The namespace's names (sorted), and where each one's value is.
    pub exports: Vec<(JsStr, Target)>,
    pub meta: Option<Gc>,
    /// Its parse error, or what its evaluation threw.
    pub error: Option<Value>,
    pub dfs_index: usize,
    pub dfs_ancestor: usize,
    pub async_order: u64,
    pub pending: usize,
    pub async_parents: Vec<usize>,
    pub cycle_root: usize,
    /// [[TopLevelCapability]]: the promise Evaluate gave for it.
    pub capability: Option<Gc>,
    /// A JSON module's value (its default export, set as it's evaluated).
    pub synthetic: Option<Value>,
}

/// Where an exported name's value is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// A module's environment, and the slot there.
    Slot(Gc, u16),
    /// A module's namespace object (`export * as ns from`).
    Namespace(usize),
}

/// ResolveExport's answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Resolution {
    None,
    Ambiguous,
    Slot(usize, u16),
    Namespace(usize),
}

pub enum Entry {
    /// Asked of the host.
    Fetching,
    Loaded(usize),
    /// It couldn't be had: the error each import of it fails with.
    Failed(Value),
}

pub enum LoadKind {
    /// import()'s: its promise.
    Dynamic(Gc),
    /// The host's (a page's module script), by its number.
    Host(u32),
}

pub struct Load {
    pub url: String,
    pub kind: LoadKind,
}

/// A host's resolution of a specifier: (the referrer's address, the specifier) -> the module's address, or why not.
pub type Resolve = fn(&mut Vm, &str, &str) -> Result<String, String>;

/// A module's key in the module map: its address, and its type if it isn't JavaScript (a JSON module, imported
/// `with { type: "json" }`, is another module than a script at the same address).
pub fn module_key(url: &str, ty: Option<&str>) -> String {
    match ty {
        Some(t) => format!("{url}\u{0}{t}"),
        None => url.to_string(),
    }
}

/// A key's address, and its module's type (None: JavaScript).
pub fn split_key(key: &str) -> (&str, Option<&str>) {
    match key.split_once('\u{0}') {
        Some((u, t)) => (u, Some(t)),
        None => (key, None),
    }
}

/// A module request's type (its `type` attribute).
fn request_type(r: &crate::ast::Request) -> Option<String> {
    r.attrs.iter().find(|(k, _)| k.is("type")).map(|(_, v)| v.to_rust())
}

/// A host's reading of a module's text at once (its address) -> the text, or why not.
pub type FetchSync = fn(&mut Vm, &str) -> Result<String, String>;

pub struct Modules {
    pub list: Vec<Module>,
    pub map: BTreeMap<String, Entry>,
    pub loads: Vec<Load>,
    /// The host's loads that have finished: their numbers, and the module or the error.
    pub ready: Vec<(u32, Result<usize, Value>)>,
    pub resolve: Resolve,
    /// A module's text, for a host that has it at hand (None: modules are fetched, module_wanted says which).
    pub fetch_sync: Option<FetchSync>,
    /// Puts the host's properties on a module's import.meta (its url).
    pub meta: Option<fn(&mut Vm, usize, Gc)>,
    /// What a script's import() resolves against.
    pub script_base: String,
    order: u64,
    next_load: u32,
}

impl Default for Modules {
    fn default() -> Self {
        Modules { list: Vec::new(), map: BTreeMap::new(), loads: Vec::new(), ready: Vec::new(), resolve: resolve_path, fetch_sync: None, meta: None, script_base: String::new(), order: 1, next_load: 1 }
    }
}

impl Modules {
    /// The values the modules hold, for the collector.
    pub fn trace(&self, work: &mut Vec<Gc>, roots: &mut Vec<Value>) {
        for m in &self.list {
            work.extend(m.env);
            work.extend(m.namespace);
            work.extend(m.meta);
            work.extend(m.capability);
            if let Some(e) = &m.error {
                roots.push(e.clone());
            }
            if let Some(v) = &m.synthetic {
                roots.push(v.clone());
            }
            if let Some(c) = &m.code {
                c.init.trace(work);
                c.body.trace(work);
            }
        }
        for e in self.map.values() {
            if let Entry::Failed(v) = e {
                roots.push(v.clone());
            }
        }
        for l in &self.loads {
            if let LoadKind::Dynamic(p) = l.kind {
                work.push(p);
            }
        }
        for (_, r) in &self.ready {
            if let Err(e) = r {
                roots.push(e.clone());
            }
        }
    }
}

/// The default resolution: a path (or an address) relative to the referrer's (`./a.js`, `../b.js`, `/c.js`), or a
/// whole address; a bare name (`lodash`) isn't one.
pub fn resolve_path(_vm: &mut Vm, base: &str, spec: &str) -> Result<String, String> {
    if spec.contains("://") {
        return Ok(spec.to_string());
    }
    if !(spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/')) {
        return Err(format!("Failed to resolve module specifier \"{spec}\": relative references must start with \"/\", \"./\" or \"../\""));
    }
    // The base's scheme and host (kept), and its directory.
    let (root, path) = match base.find("://") {
        Some(i) => {
            let rest = &base[i + 3..];
            let j = rest.find('/').map_or(base.len(), |j| i + 3 + j);
            (&base[..j], &base[j..])
        }
        None => ("", base),
    };
    let mut parts: Vec<&str> = Vec::new();
    if !spec.starts_with('/') {
        let dir = match path.rfind('/') {
            Some(k) => &path[..k],
            None => "",
        };
        parts.extend(dir.split('/').filter(|p| !p.is_empty()));
    }
    for p in spec.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    let mut out = String::from(root);
    for p in &parts {
        out.push('/');
        out.push_str(p);
    }
    if parts.is_empty() {
        out.push('/');
    }
    Ok(out)
}

impl Vm {
    // -------------------------------------------------------------------------------------------------- loading

    /// A module's text arrived, under `key` (its address in the module map; `url` is import.meta.url and what its
    /// imports resolve against, the same but for a page's inline module): parsed and compiled. Its number (if it
    /// doesn't parse, every import of it fails with the SyntaxError).
    pub fn module_add(&mut self, key: &str, url: &str, source: &str) -> usize {
        let i = self.modules.list.len();
        let mut m = Module {
            url: url.to_string(),
            status: Status::New,
            code: None,
            urls: Vec::new(),
            env: None,
            namespace: None,
            exports: Vec::new(),
            meta: None,
            error: None,
            dfs_index: 0,
            dfs_ancestor: 0,
            async_order: UNSET,
            pending: 0,
            async_parents: Vec::new(),
            cycle_root: i,
            capability: None,
            synthetic: None,
        };
        let compiled = match split_key(key).1 {
            None => crate::parser::parse_module(source, &mut self.atoms).map_err(|e| e.message).and_then(|s| compiler::compile_module(&s, &mut self.atoms, Rc::from(source)).map_err(|e| e.message)),
            // A JSON module: its value, the default export of a module that has only that.
            Some("json") => match crate::builtins::json::parse(self, &JsStr::from_str(source)) {
                Some(v) => {
                    m.synthetic = Some(v);
                    Ok(json_code())
                }
                None => Err(String::from("The module's JSON isn't valid")),
            },
            Some(t) => {
                m.error = Some(self.type_error(&format!("Modules of type \"{t}\" aren't supported")));
                Err(String::new())
            }
        };
        match compiled {
            Ok(code) => {
                let resolve = self.modules.resolve;
                m.urls = code.requests.iter().map(|r| resolve(self, url, &r.spec.to_rust()).map(|u| module_key(&u, request_type(r).as_deref()))).collect();
                let vars = code.names.iter().map(|_| Value::Empty).collect();
                m.env = Some(self.alloc(None, Class::Env(Box::new(Env { vars, parent: None, names: code.names.clone(), with: None, var_scope: true }))));
                m.code = Some(Rc::new(code));
            }
            Err(message) => {
                if m.error.is_none() {
                    m.error = Some(self.syntax_error(&message));
                }
            }
        }
        self.modules.list.push(m);
        self.modules.map.insert(key.to_string(), Entry::Loaded(i));
        i
    }

    /// A module that couldn't be fetched (`why`): each import of it fails with a TypeError.
    pub fn module_failed(&mut self, url: &str, why: &str) {
        let e = self.type_error(why);
        self.modules.map.insert(url.to_string(), Entry::Failed(e));
    }

    /// Loads a module (and those it imports) for the host: its number in Modules::ready when it's done.
    pub fn module_load(&mut self, url: &str) -> u32 {
        let id = self.modules.next_load;
        self.modules.next_load += 1;
        self.modules.loads.push(Load { url: url.to_string(), kind: LoadKind::Host(id) });
        self.module_progress();
        id
    }

    /// The addresses the loads waiting want that haven't been asked for: each is asked for once (the host fetches
    /// them, and gives them with module_add or module_failed).
    pub fn module_wanted(&mut self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for i in 0..self.modules.loads.len() {
            let url = self.modules.loads[i].url.clone();
            if let Graph::Waiting(w) = self.graph(&url) {
                for u in w {
                    if !out.contains(&u) {
                        out.push(u);
                    }
                }
            }
        }
        for u in &out {
            self.modules.map.insert(u.clone(), Entry::Fetching);
        }
        out
    }

    /// Finishes the loads whose modules have all come (or one of which failed).
    pub fn module_progress(&mut self) {
        if let Some(fetch) = self.modules.fetch_sync {
            loop {
                let wanted = self.module_wanted();
                if wanted.is_empty() {
                    break;
                }
                for u in wanted {
                    match fetch(self, &u) {
                        Ok(text) => {
                            self.module_add(&u, &u, &text);
                        }
                        Err(why) => self.module_failed(&u, &why),
                    }
                }
            }
        }
        let mut i = 0;
        while i < self.modules.loads.len() {
            let url = self.modules.loads[i].url.clone();
            let r = match self.graph(&url) {
                Graph::Waiting(_) => {
                    i += 1;
                    continue;
                }
                Graph::Complete(m) => Ok(m),
                Graph::Failed(e) => Err(e),
            };
            let load = self.modules.loads.remove(i);
            match (load.kind, r) {
                (LoadKind::Dynamic(p), Err(e)) => self.reject_promise(p, e),
                (LoadKind::Dynamic(p), Ok(m)) => {
                    // (Linked and evaluated in a job, as the loading promise's reaction.)
                    let f = self.native_with("", 0, import_link_evaluate, None, Value::Num(m as f64));
                    self.set_data2(f, Value::Obj(p));
                    self.jobs.push_back(Job::Call { f: Value::Obj(f), args: Vec::new() });
                }
                (LoadKind::Host(id), r) => self.modules.ready.push((id, r)),
            }
        }
    }

    /// Whether a module's graph has all come: what's missing, or the error that stops it.
    fn graph(&mut self, root: &str) -> Graph {
        let mut wanted = Vec::new();
        let mut waiting = false;
        let mut seen: Vec<usize> = Vec::new();
        let mut todo = vec![root.to_string()];
        while let Some(u) = todo.pop() {
            match self.modules.map.get(&u) {
                None => {
                    if !wanted.contains(&u) {
                        wanted.push(u);
                    }
                }
                Some(Entry::Fetching) => waiting = true,
                Some(Entry::Failed(e)) => return Graph::Failed(e.clone()),
                Some(Entry::Loaded(m)) => {
                    let m = *m;
                    if seen.contains(&m) {
                        continue;
                    }
                    seen.push(m);
                    let md = &self.modules.list[m];
                    if md.code.is_none() {
                        return Graph::Failed(md.error.clone().unwrap_or(Value::Undefined));
                    }
                    for r in md.urls.iter().rev() {
                        match r {
                            Ok(u) => todo.push(u.clone()),
                            Err(why) => {
                                let why = why.clone();
                                return Graph::Failed(self.type_error(&why));
                            }
                        }
                    }
                }
            }
        }
        if waiting || !wanted.is_empty() {
            return Graph::Waiting(wanted);
        }
        match self.modules.map.get(root) {
            Some(Entry::Loaded(m)) => Graph::Complete(*m),
            _ => Graph::Waiting(Vec::new()),
        }
    }

    /// The module a module's request is (its graph has come).
    fn imported(&self, m: usize, r: usize) -> Option<usize> {
        let url = self.modules.list[m].urls.get(r)?.as_ref().ok()?;
        match self.modules.map.get(url) {
            Some(Entry::Loaded(i)) => Some(*i),
            _ => None,
        }
    }

    fn imported_or_throw(&mut self, m: usize, r: usize) -> Result<usize, Value> {
        match self.imported(m, r) {
            Some(i) => Ok(i),
            None => Err(self.type_error("A module it imports isn't loaded")),
        }
    }

    // ---------------------------------------------------------------------------------------------- import()

    /// import(spec, options)'s specifier and the module's type its options give (EvaluateImportCall's checks).
    fn import_request(&mut self, spec: &Value, options: &Value) -> Result<(JsStr, Option<String>), Value> {
        let s = self.to_string(spec)?;
        let mut ty = None;
        if !options.is_undefined() {
            let Value::Obj(o) = options else { return Err(self.type_error("import()'s options must be an object")) };
            let with = self.get(*o, Key::Str(n::WITH), options)?;
            if !with.is_undefined() {
                let Value::Obj(w) = with else { return Err(self.type_error("import()'s 'with' option must be an object")) };
                let mut attrs: Vec<(JsStr, JsStr)> = Vec::new();
                for k in self.own_keys(w)? {
                    if let Key::Sym(_) = k {
                        continue;
                    }
                    let Some(p) = self.get_own_property(w, k)? else { continue };
                    if p.flags & ENUMERABLE == 0 {
                        continue;
                    }
                    let v = self.get(w, k, &with)?;
                    let Value::Str(v) = v else { return Err(self.type_error("An import attribute's value must be a string")) };
                    let Value::Str(key) = self.key_value(k) else { continue };
                    attrs.push((key, v));
                }
                if let Some((k, _)) = attrs.iter().find(|(k, _)| !k.is("type")) {
                    let m = format!("The import attribute \"{}\" isn't supported", k.to_rust());
                    return Err(self.syntax_error(&m));
                }
                ty = attrs.iter().find(|(k, _)| k.is("type")).map(|(_, v)| v.to_rust());
            }
        }
        Ok((s, ty))
    }

    /// import(spec, options) in code of module `referrer` (None: a script's): a promise of the module's namespace.
    pub fn dynamic_import(&mut self, referrer: Option<usize>, spec: Value, options: Value) -> Value {
        let p = self.new_promise();
        let (s, ty) = match self.import_request(&spec, &options) {
            Ok(r) => r,
            Err(e) => {
                self.reject_promise(p, e);
                return Value::Obj(p);
            }
        };
        let base = match referrer {
            Some(m) => self.modules.list[m].url.clone(),
            None => self.modules.script_base.clone(),
        };
        let resolve = self.modules.resolve;
        match resolve(self, &base, &s.to_rust()) {
            Err(why) => {
                let e = self.type_error(&why);
                self.reject_promise(p, e);
            }
            Ok(url) => {
                self.modules.loads.push(Load { url: module_key(&url, ty.as_deref()), kind: LoadKind::Dynamic(p) });
                self.module_progress();
            }
        }
        Value::Obj(p)
    }

    /// The module whose code is running (the one whose environment is around the running code's), if it's a
    /// module's.
    pub fn current_module(&self) -> Option<usize> {
        let mut e = self.frames.last()?.env;
        while let Some(g) = e {
            let Class::Env(env) = &self.heap.get(g).class else { return None };
            if env.names.first().map(|x| x.0) == Some(n::MODULE_SLOT)
                && let Some(Value::Num(x)) = env.vars.first()
            {
                return Some(*x as usize);
            }
            e = env.parent;
        }
        None
    }

    /// A module's import.meta: made once (an object with no prototype, and the host's properties).
    pub fn import_meta(&mut self, m: usize) -> Gc {
        if let Some(g) = self.modules.list[m].meta {
            return g;
        }
        let g = self.alloc(None, Class::Ordinary);
        self.modules.list[m].meta = Some(g);
        if let Some(f) = self.modules.meta {
            f(self, m, g);
        }
        g
    }

    // -------------------------------------------------------------------------------------------------- linking

    /// Links a module and those it imports (each import resolved, each environment made, functions made): a
    /// SyntaxError if an import names what isn't exported.
    pub fn module_link(&mut self, m: usize) -> Result<(), Value> {
        let mut stack = Vec::new();
        match self.inner_link(m, &mut stack, 0) {
            Ok(_) => Ok(()),
            Err(e) => {
                for x in stack {
                    self.modules.list[x].status = Status::New;
                }
                if self.modules.list[m].status == Status::Linking {
                    self.modules.list[m].status = Status::New;
                }
                Err(e)
            }
        }
    }

    fn inner_link(&mut self, m: usize, stack: &mut Vec<usize>, mut index: usize) -> Result<usize, Value> {
        let md = &self.modules.list[m];
        if md.status != Status::New {
            return Ok(index);
        }
        let Some(code) = md.code.clone() else { return Err(md.error.clone().unwrap_or(Value::Undefined)) };
        let md = &mut self.modules.list[m];
        md.status = Status::Linking;
        md.dfs_index = index;
        md.dfs_ancestor = index;
        index += 1;
        stack.push(m);
        for r in 0..code.requests.len() {
            let req = self.imported_or_throw(m, r)?;
            index = self.inner_link(req, stack, index)?;
            if self.modules.list[req].status == Status::Linking {
                let a = self.modules.list[req].dfs_ancestor;
                let md = &mut self.modules.list[m];
                md.dfs_ancestor = md.dfs_ancestor.min(a);
            }
        }
        self.initialize_environment(m, &code)?;
        if self.modules.list[m].dfs_ancestor == self.modules.list[m].dfs_index {
            while let Some(x) = stack.pop() {
                self.modules.list[x].status = Status::Linked;
                if x == m {
                    break;
                }
            }
        }
        Ok(index)
    }

    /// The name a module's request is written as (for messages).
    fn request_name(&self, m: usize, r: usize) -> String {
        self.modules.list[m].code.as_ref().and_then(|c| c.requests.get(r)).map(|s| s.spec.to_rust()).unwrap_or_default()
    }

    fn unresolved(&mut self, m: usize, r: usize, name: &JsStr, res: Resolution) -> Value {
        let from = self.request_name(m, r);
        let msg = if res == Resolution::Ambiguous {
            format!("The requested module '{from}' contains conflicting star exports for name '{}'", name.to_rust())
        } else {
            format!("The requested module '{from}' does not provide an export named '{}'", name.to_rust())
        };
        self.syntax_error(&msg)
    }

    fn initialize_environment(&mut self, m: usize, code: &Rc<ModuleCode>) -> Result<(), Value> {
        for (name, r, _) in &code.indirect {
            let res = self.resolve_export(m, name, &mut Vec::new());
            if matches!(res, Resolution::None | Resolution::Ambiguous) {
                return Err(self.unresolved(m, *r, name, res));
            }
        }
        let env = self.modules.list[m].env.expect("a module's environment");
        // Its names: vars undefined, lets, consts and classes uninitialized.
        let mut vars: Vec<Value> = code.names.iter().map(|(_, k)| if *k == VAR { Value::Undefined } else { Value::Empty }).collect();
        vars[0] = Value::Num(m as f64);
        for (r, name, slot) in &code.imports {
            let from = self.imported_or_throw(m, *r)?;
            let v = match name {
                None => Value::Obj(self.module_namespace(from)),
                Some(name) => match self.resolve_export(from, name, &mut Vec::new()) {
                    Resolution::Slot(x, s) => {
                        let e = self.modules.list[x].env.expect("a module's environment");
                        Value::Obj(self.alloc(None, Class::ImportCell(e, s)))
                    }
                    Resolution::Namespace(x) => Value::Obj(self.module_namespace(x)),
                    res => return Err(self.unresolved(m, *r, name, res)),
                },
            };
            vars[*slot as usize] = v;
        }
        if let Class::Env(e) = &mut self.heap.get_mut(env).class {
            e.vars = vars;
        }
        // Its functions.
        self.run_code(code.init.clone(), Some(env), Value::Undefined)?;
        Ok(())
    }

    /// ResolveExport: where a module's export of `name` comes from (through re-exports and `export * from`).
    fn resolve_export(&self, m: usize, name: &JsStr, seen: &mut Vec<(usize, JsStr)>) -> Resolution {
        if seen.iter().any(|(x, y)| *x == m && y == name) {
            // (A circular import request.)
            return Resolution::None;
        }
        seen.push((m, name.clone()));
        let Some(code) = &self.modules.list[m].code else { return Resolution::None };
        if let Some((_, s)) = code.locals.iter().find(|(x, _)| x == name) {
            return Resolution::Slot(m, *s);
        }
        if let Some((_, r, imported)) = code.indirect.iter().find(|(x, _, _)| x == name) {
            let Some(from) = self.imported(m, *r) else { return Resolution::None };
            return match imported {
                None => Resolution::Namespace(from),
                Some(i) => self.resolve_export(from, i, seen),
            };
        }
        if name.is("default") {
            // (A default export can't come from `export * from`.)
            return Resolution::None;
        }
        let mut star = Resolution::None;
        for r in &code.stars {
            let Some(from) = self.imported(m, *r) else { continue };
            match self.resolve_export(from, name, seen) {
                Resolution::Ambiguous => return Resolution::Ambiguous,
                Resolution::None => {}
                res => {
                    if star == Resolution::None {
                        star = res;
                    } else if star != res {
                        return Resolution::Ambiguous;
                    }
                }
            }
        }
        star
    }

    /// GetExportedNames.
    fn exported_names(&self, m: usize, seen: &mut Vec<usize>) -> Vec<JsStr> {
        if seen.contains(&m) {
            return Vec::new();
        }
        seen.push(m);
        let Some(code) = &self.modules.list[m].code else { return Vec::new() };
        let mut out: Vec<JsStr> = code.locals.iter().map(|x| x.0.clone()).chain(code.indirect.iter().map(|x| x.0.clone())).collect();
        for r in &code.stars {
            let Some(from) = self.imported(m, *r) else { continue };
            for name in self.exported_names(from, seen) {
                if !name.is("default") && !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        out
    }

    /// A module's namespace object (made once): its exports, as properties, sorted, read as they are now.
    pub fn module_namespace(&mut self, m: usize) -> Gc {
        if let Some(g) = self.modules.list[m].namespace {
            return g;
        }
        let names = self.exported_names(m, &mut Vec::new());
        let mut exports = Vec::new();
        for name in names {
            match self.resolve_export(m, &name, &mut Vec::new()) {
                Resolution::Slot(x, s) => exports.push((name, Target::Slot(self.modules.list[x].env.expect("a module's environment"), s))),
                Resolution::Namespace(x) => exports.push((name, Target::Namespace(x))),
                _ => {}
            }
        }
        exports.sort_by(|a, b| a.0.cmp(&b.0));
        let g = self.alloc(None, Class::Namespace(m as u32));
        let tag = self.sym(Sym::ToStringTag);
        self.put_key(g, Key::Sym(tag), Value::str("Module"), 0);
        self.heap.get_mut(g).extensible = false;
        let md = &mut self.modules.list[m];
        md.namespace = Some(g);
        md.exports = exports;
        g
    }

    // ----------------------------------------------------------------------------------------------- evaluating

    /// Evaluates a linked module (those it imports first): the promise that settles when it has run (rejected with
    /// what it threw).
    pub fn module_evaluate(&mut self, m: usize) -> Gc {
        let mut module = m;
        if matches!(self.modules.list[m].status, Status::EvaluatingAsync | Status::Evaluated) {
            module = self.modules.list[m].cycle_root;
        }
        if let Some(p) = self.modules.list[module].capability {
            return p;
        }
        let p = self.new_promise();
        self.modules.list[module].capability = Some(p);
        let mut stack = Vec::new();
        match self.inner_evaluate(module, &mut stack, 0) {
            Err(e) => {
                for x in stack {
                    let md = &mut self.modules.list[x];
                    md.status = Status::Evaluated;
                    md.error = Some(e.clone());
                }
                self.reject_promise(p, e);
            }
            Ok(_) => {
                let o = self.modules.list[module].async_order;
                if o == UNSET || o == DONE {
                    let _ = self.resolve_promise(p, Value::Undefined);
                }
            }
        }
        p
    }

    fn inner_evaluate(&mut self, m: usize, stack: &mut Vec<usize>, mut index: usize) -> Result<usize, Value> {
        match self.modules.list[m].status {
            Status::EvaluatingAsync | Status::Evaluated => {
                return match &self.modules.list[m].error {
                    None => Ok(index),
                    Some(e) => Err(e.clone()),
                };
            }
            Status::Evaluating => return Ok(index),
            Status::Linked => {}
            _ => return Err(self.type_error("A module was run before it was linked")),
        }
        let code = self.modules.list[m].code.clone().expect("a linked module's code");
        let md = &mut self.modules.list[m];
        md.status = Status::Evaluating;
        md.dfs_index = index;
        md.dfs_ancestor = index;
        md.pending = 0;
        index += 1;
        stack.push(m);
        for r in 0..code.requests.len() {
            let mut req = self.imported_or_throw(m, r)?;
            index = self.inner_evaluate(req, stack, index)?;
            if self.modules.list[req].status == Status::Evaluating {
                let a = self.modules.list[req].dfs_ancestor;
                let md = &mut self.modules.list[m];
                md.dfs_ancestor = md.dfs_ancestor.min(a);
            } else {
                req = self.modules.list[req].cycle_root;
                if let Some(e) = &self.modules.list[req].error {
                    return Err(e.clone());
                }
            }
            let o = self.modules.list[req].async_order;
            if o != UNSET && o != DONE {
                self.modules.list[m].pending += 1;
                self.modules.list[req].async_parents.push(m);
            }
        }
        if self.modules.list[m].pending > 0 || code.tla {
            let order = self.modules.order;
            self.modules.order += 1;
            self.modules.list[m].async_order = order;
            if self.modules.list[m].pending == 0 {
                self.execute_async_module(m);
            }
        } else {
            self.execute_module(m)?;
        }
        if self.modules.list[m].dfs_ancestor == self.modules.list[m].dfs_index {
            while let Some(x) = stack.pop() {
                let md = &mut self.modules.list[x];
                md.status = if md.async_order == UNSET || md.async_order == DONE { Status::Evaluated } else { Status::EvaluatingAsync };
                md.cycle_root = m;
                if x == m {
                    break;
                }
            }
        }
        Ok(index)
    }

    /// A module's code run (one that doesn't await at its top level).
    fn execute_module(&mut self, m: usize) -> Result<(), Value> {
        let code = self.modules.list[m].code.clone().expect("a module's code");
        let env = self.modules.list[m].env;
        if let (Some(v), Some(env)) = (self.modules.list[m].synthetic.clone(), env) {
            // (A JSON module: its default export set.)
            if let Class::Env(e) = &mut self.heap.get_mut(env).class {
                e.vars[1] = v;
            }
            return Ok(());
        }
        self.run_code(code.body.clone(), env, Value::Undefined)?;
        Ok(())
    }

    /// ExecuteAsyncModule: a module that awaits at its top level started, as an async function; its end goes on to
    /// the modules waiting for it.
    fn execute_async_module(&mut self, m: usize) {
        let code = self.modules.list[m].code.clone().expect("a module's code");
        let env = self.modules.list[m].env;
        let f = self.make_closure(code.body.clone(), env, Value::Undefined, Value::Undefined, None);
        let on_ful = self.native_with("", 0, async_module_fulfilled, None, Value::Num(m as f64));
        let on_rej = self.native_with("", 1, async_module_rejected, None, Value::Num(m as f64));
        let p = match self.call_special(f, Value::Undefined, &[]) {
            Ok(Value::Obj(p)) => p,
            Ok(_) => return,
            Err(e) => {
                let p = self.new_promise();
                self.reject_promise(p, e);
                p
            }
        };
        self.perform_then(p, Value::Obj(on_ful), Value::Obj(on_rej), Value::Undefined, Value::Undefined, Value::Undefined);
    }

    fn gather_ancestors(&mut self, m: usize, out: &mut Vec<usize>) {
        let parents = self.modules.list[m].async_parents.clone();
        for p in parents {
            let root = self.modules.list[p].cycle_root;
            if out.contains(&p) || self.modules.list[root].error.is_some() {
                continue;
            }
            let pm = &mut self.modules.list[p];
            pm.pending = pm.pending.saturating_sub(1);
            if pm.pending == 0 {
                out.push(p);
                let tla = pm.code.as_ref().is_some_and(|c| c.tla);
                if !tla {
                    self.gather_ancestors(p, out);
                }
            }
        }
    }

    /// AsyncModuleExecutionFulfilled.
    fn async_fulfilled(&mut self, m: usize) {
        if self.modules.list[m].status == Status::Evaluated {
            return;
        }
        let md = &mut self.modules.list[m];
        md.async_order = DONE;
        md.status = Status::Evaluated;
        if let Some(p) = md.capability {
            let _ = self.resolve_promise(p, Value::Undefined);
        }
        let mut list = Vec::new();
        self.gather_ancestors(m, &mut list);
        list.sort_by_key(|x| self.modules.list[*x].async_order);
        for x in list {
            let md = &self.modules.list[x];
            if md.status == Status::Evaluated {
                continue;
            }
            if md.code.as_ref().is_some_and(|c| c.tla) {
                self.execute_async_module(x);
            } else {
                match self.execute_module(x) {
                    Err(e) => self.async_rejected(x, e),
                    Ok(()) => {
                        let md = &mut self.modules.list[x];
                        md.async_order = DONE;
                        md.status = Status::Evaluated;
                        if let Some(p) = md.capability {
                            let _ = self.resolve_promise(p, Value::Undefined);
                        }
                    }
                }
            }
        }
    }

    /// AsyncModuleExecutionRejected.
    fn async_rejected(&mut self, m: usize, e: Value) {
        if self.modules.list[m].status == Status::Evaluated {
            return;
        }
        let md = &mut self.modules.list[m];
        md.error = Some(e.clone());
        md.status = Status::Evaluated;
        let parents = md.async_parents.clone();
        if let Some(p) = md.capability {
            self.reject_promise(p, e.clone());
        }
        for p in parents {
            self.async_rejected(p, e.clone());
        }
    }

    // ------------------------------------------------------------------------------------------------ namespace

    /// The namespace's export a key names (its place in the sorted names).
    fn ns_find(&self, g: Gc, k: Key) -> Option<(usize, usize)> {
        let Class::Namespace(m) = self.heap.get(g).class else { return None };
        let name = match k {
            Key::Str(a) => self.str_of(a),
            Key::Index(i) => JsStr::from_str(&i.to_string()),
            Key::Sym(_) => return None,
        };
        let exports = &self.modules.list[m as usize].exports;
        exports.binary_search_by(|x| x.0.cmp(&name)).ok().map(|i| (m as usize, i))
    }

    /// An export's value now (a ReferenceError before it's initialized).
    fn ns_value(&mut self, m: usize, i: usize) -> Result<Value, Value> {
        let (name, t) = self.modules.list[m].exports[i].clone();
        match t {
            Target::Namespace(x) => Ok(Value::Obj(self.module_namespace(x))),
            Target::Slot(env, s) => {
                let v = match &self.heap.get(env).class {
                    Class::Env(e) => e.vars[s as usize].clone(),
                    _ => Value::Undefined,
                };
                if matches!(v, Value::Empty) {
                    return Err(self.reference_error(&format!("Cannot access '{}' before initialization", name.to_rust())));
                }
                Ok(v)
            }
        }
    }

    /// [[GetOwnProperty]] of a namespace's string key.
    pub(crate) fn ns_own(&mut self, g: Gc, k: Key) -> Result<Option<Prop>, Value> {
        if let Key::Sym(_) = k {
            return Ok(self.heap.get(g).props.get(&k).cloned());
        }
        match self.ns_find(g, k) {
            None => Ok(None),
            Some((m, i)) => {
                let v = self.ns_value(m, i)?;
                Ok(Some(Prop::data(v, WRITABLE | ENUMERABLE)))
            }
        }
    }

    /// [[Get]].
    pub(crate) fn ns_get(&mut self, g: Gc, k: Key) -> Result<Value, Value> {
        if let Key::Sym(_) = k {
            return Ok(match self.heap.get(g).props.get(&k) {
                Some(Prop { slot: Slot::Data(v), .. }) => v.clone(),
                _ => Value::Undefined,
            });
        }
        match self.ns_find(g, k) {
            None => Ok(Value::Undefined),
            Some((m, i)) => self.ns_value(m, i),
        }
    }

    pub(crate) fn ns_has(&self, g: Gc, k: Key) -> bool {
        match k {
            Key::Sym(_) => self.heap.get(g).props.get(&k).is_some(),
            _ => self.ns_find(g, k).is_some(),
        }
    }

    /// [[Delete]] of a string key: only what isn't an export.
    pub(crate) fn ns_delete(&self, g: Gc, k: Key) -> bool {
        self.ns_find(g, k).is_none()
    }

    /// [[DefineOwnProperty]] of a string key: only what changes nothing.
    pub(crate) fn ns_define(&mut self, g: Gc, k: Key, desc: Desc) -> Result<bool, Value> {
        let Some(cur) = self.ns_own(g, k)? else { return Ok(false) };
        if desc.configurable == Some(true) || desc.enumerable == Some(false) || desc.is_accessor() || desc.writable == Some(false) {
            return Ok(false);
        }
        if let (Some(v), Slot::Data(c)) = (&desc.value, &cur.slot) {
            return Ok(crate::value::same_value(v, c));
        }
        Ok(true)
    }

    /// [[OwnPropertyKeys]]: the exports, then the symbols.
    pub(crate) fn ns_keys(&mut self, g: Gc) -> Vec<Key> {
        let Class::Namespace(m) = self.heap.get(g).class else { return Vec::new() };
        let names: Vec<JsStr> = self.modules.list[m as usize].exports.iter().map(|x| x.0.clone()).collect();
        let mut out: Vec<Key> = names.iter().map(|s| self.key_from_str(s)).collect();
        out.extend(self.heap.get(g).props.keys().into_iter().filter(|k| matches!(k, Key::Sym(_))));
        out
    }

    /// An imported name's value (its slot's ImportCell followed), for code that looks names up as it runs.
    pub(crate) fn import_value(&self, v: &Value) -> Value {
        if let Value::Obj(c) = v
            && let Class::ImportCell(env, s) = self.heap.get(*c).class
            && let Class::Env(e) = &self.heap.get(env).class
        {
            return e.vars[s as usize].clone();
        }
        v.clone()
    }
}

enum Graph {
    Waiting(Vec<String>),
    Complete(usize),
    Failed(Value),
}

/// A JSON module's code: nothing but its default export (`*default*`, slot 1), which evaluating it sets.
fn json_code() -> ModuleCode {
    let code = || {
        let mut c = crate::code::Code::new(n::EMPTY, crate::code::Kind::Module);
        c.strict = true;
        c.ops = vec![crate::code::Op::Undefined, crate::code::Op::Return];
        Rc::new(c)
    };
    let names: Rc<[(crate::string::Atom, u8)]> = Rc::from(vec![(n::MODULE_SLOT, VAR), (n::DEFAULT_EXPORT, crate::object::CONST)]);
    ModuleCode { init: code(), body: code(), names, requests: Vec::new(), imports: Vec::new(), locals: vec![(JsStr::from_str("default"), 1)], indirect: Vec::new(), stars: Vec::new(), tla: false }
}

fn module_of(vm: &Vm, f: Gc) -> usize {
    match vm.data(f) {
        Value::Num(x) => x as usize,
        _ => 0,
    }
}

/// import()'s continuation once its modules have come: link, evaluate, and settle its promise with the namespace.
fn import_link_evaluate(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> Result<Value, Value> {
    let m = module_of(vm, f);
    let Value::Obj(p) = vm.data2(f) else { return Ok(Value::Undefined) };
    if let Err(e) = vm.module_link(m) {
        vm.reject_promise(p, e);
        return Ok(Value::Undefined);
    }
    let ev = vm.module_evaluate(m);
    let on_ful = vm.native_with("", 0, import_fulfilled, None, Value::Num(m as f64));
    vm.set_data2(on_ful, Value::Obj(p));
    let on_rej = vm.native_with("", 1, import_rejected, None, Value::Undefined);
    vm.set_data2(on_rej, Value::Obj(p));
    vm.perform_then(ev, Value::Obj(on_ful), Value::Obj(on_rej), Value::Undefined, Value::Undefined, Value::Undefined);
    Ok(Value::Undefined)
}

fn import_fulfilled(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> Result<Value, Value> {
    let m = module_of(vm, f);
    if let Value::Obj(p) = vm.data2(f) {
        let ns = vm.module_namespace(m);
        vm.resolve_promise(p, Value::Obj(ns))?;
    }
    Ok(Value::Undefined)
}

fn import_rejected(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> Result<Value, Value> {
    if let Value::Obj(p) = vm.data2(f) {
        vm.reject_promise(p, args.first().cloned().unwrap_or(Value::Undefined));
    }
    Ok(Value::Undefined)
}

fn async_module_fulfilled(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> Result<Value, Value> {
    let m = module_of(vm, f);
    vm.async_fulfilled(m);
    Ok(Value::Undefined)
}

fn async_module_rejected(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> Result<Value, Value> {
    let m = module_of(vm, f);
    vm.async_rejected(m, args.first().cloned().unwrap_or(Value::Undefined));
    Ok(Value::Undefined)
}
