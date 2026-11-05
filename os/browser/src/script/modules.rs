//! Module scripts (`<script type="module">`, import()) for the engine's module loader (js/src/module.rs): the
//! page's import maps (`<script type="importmap">`), how a specifier becomes an address (the map's, else relative
//! to the importing module's address), import.meta's url and resolve(), and running a module script once its
//! modules have come (the page fetches them: js's module_wanted).

use super::url::Url;
use super::{STEPS, Script, host, host_ref, report};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use js::object::{ALL, Class, Key};
use js::string::JsStr;
use js::value::{Gc, Value};
use js::vm::Vm;

/// An import map: specifiers (whole, or prefixes ending in "/") and the addresses they stand for (None: blocked,
/// an address that wasn't one), for every module, and for those under a scope's address. Each list is sorted with
/// the longest (most particular) first.
#[derive(Default)]
pub struct ImportMap {
    pub imports: Vec<(String, Option<String>)>,
    pub scopes: Vec<(String, Vec<(String, Option<String>)>)>,
}

/// Whether a specifier is written as an address relative to the importer's ("/x", "./x", "../x").
fn url_like(spec: &str) -> bool {
    spec.starts_with('/') || spec.starts_with("./") || spec.starts_with("../")
}

/// A specifier as an address, if it's one (relative ones against `base`).
fn as_url(spec: &str, base: &Url) -> Option<Url> {
    if url_like(spec) { Url::parse(spec, Some(base)) } else { Url::parse(spec, None) }
}

/// An import map's specifier map: each key normalized (an address-like key made absolute), each value an address.
fn specifier_map(vm: &mut Vm, obj: &Value, base: &Url) -> Vec<(String, Option<String>)> {
    let Value::Obj(g) = obj else { return Vec::new() };
    let keys = vm.own_keys(*g).unwrap_or_default();
    let mut out = Vec::new();
    for k in keys {
        if let Key::Sym(_) = k {
            continue;
        }
        let name = match vm.key_value(k) {
            Value::Str(s) => s.to_rust(),
            _ => continue,
        };
        if name.is_empty() {
            continue;
        }
        let key = as_url(&name, base).map(|u| u.href()).unwrap_or(name);
        let value = match vm.get(*g, k, obj) {
            Ok(Value::Str(s)) => Url::parse(&s.to_rust(), Some(base)).map(|u| u.href()),
            _ => None,
        };
        // (A prefix's address must be one too.)
        let value = value.filter(|v| !key.ends_with('/') || v.ends_with('/'));
        out.push((key, value));
    }
    out
}

/// A specifier matched against a specifier map: Some(address) if the map has it (an error if it's blocked).
fn match_map(normalized: &str, map: &[(String, Option<String>)]) -> Result<Option<String>, String> {
    for (key, value) in map {
        if key == normalized {
            return value.clone().map(Some).ok_or_else(|| format!("The import map blocks \"{normalized}\""));
        }
        if key.ends_with('/') && normalized.starts_with(key.as_str()) {
            let Some(v) = value else { return Err(format!("The import map blocks \"{normalized}\"")) };
            let rest = &normalized[key.len()..];
            let Some(base) = Url::parse(v, None) else { return Err(format!("The import map's address for \"{key}\" isn't one")) };
            let Some(u) = Url::parse(rest, Some(&base)) else { return Err(format!("\"{normalized}\" isn't an address")) };
            let href = u.href();
            // (Not out of the prefix's address: "../" can't climb above it.)
            if !href.starts_with(v.as_str()) {
                return Err(format!("\"{normalized}\" goes back above \"{key}\""));
            }
            return Ok(Some(href));
        }
    }
    Ok(None)
}

/// The engine's resolution, for a page: the import map's address for the specifier (its scopes for the importer's
/// address first), else the specifier as an address relative to the importer's.
pub fn resolve(vm: &mut Vm, base: &str, spec: &str) -> Result<String, String> {
    let h = host_ref(vm);
    let base_url = Url::parse(base, None).unwrap_or_else(|| h.base.clone());
    let as_address = as_url(spec, &base_url);
    let normalized = as_address.as_ref().map(|u| u.href()).unwrap_or_else(|| spec.to_string());
    for (prefix, map) in &h.import_map.scopes {
        if prefix == base || (prefix.ends_with('/') && base.starts_with(prefix.as_str())) {
            if let Some(r) = match_map(&normalized, map)? {
                return Ok(r);
            }
        }
    }
    if let Some(r) = match_map(&normalized, &h.import_map.imports)? {
        return Ok(r);
    }
    match as_address {
        Some(u) => Ok(u.href()),
        None => Err(format!("Failed to resolve module specifier \"{spec}\". Relative references must start with either \"/\", \"./\", or \"../\".")),
    }
}

/// import.meta's properties: the module's address, and resolve() (a specifier as this module would import it).
pub fn meta(vm: &mut Vm, m: usize, g: Gc) {
    let url = vm.modules.list[m].url.clone();
    vm.put(g, "url", Value::str(&url), ALL);
    let f = vm.native_with("resolve", 1, meta_resolve, None, Value::str(&url));
    vm.put(g, "resolve", Value::Obj(f), ALL);
}

fn meta_resolve(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> Result<Value, Value> {
    let base = match vm.data(f) {
        Value::Str(s) => s.to_rust(),
        _ => String::new(),
    };
    let spec = vm.to_string(args.first().unwrap_or(&Value::Undefined))?.to_rust();
    match resolve(vm, &base, &spec) {
        Ok(u) => Ok(Value::str(&u)),
        Err(why) => Err(vm.type_error(&why)),
    }
}

/// A module script's evaluation that failed (at once, or after an await): reported as an uncaught exception.
fn module_rejected(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> Result<Value, Value> {
    let e = args.first().cloned().unwrap_or(Value::Undefined);
    report(vm, &e, "a module");
    Ok(Value::Undefined)
}

impl Script {
    /// The engine set up for the page's modules (its resolution and import.meta).
    pub(super) fn init_modules(&mut self) {
        let base = host_ref(&self.vm).base.href();
        self.vm.modules.resolve = resolve;
        self.vm.modules.meta = Some(meta);
        self.vm.modules.script_base = base;
    }

    /// A `<script type="importmap">`'s text: its imports and scopes added to the page's (those it had first win).
    pub fn add_import_map(&mut self, json: &str) {
        let vm = &mut self.vm;
        let base = host_ref(vm).base.clone();
        let Some(v) = js::builtins::json::parse(vm, &JsStr::from_str(json)) else {
            let e = vm.syntax_error("An import map isn't JSON");
            report(vm, &e, "an import map");
            return;
        };
        let Value::Obj(g) = v else { return };
        let (imports_key, scopes_key) = (vm.atom("imports"), vm.atom("scopes"));
        let imports = match vm.get(g, Key::Str(imports_key), &v) {
            Ok(o @ Value::Obj(_)) => specifier_map(vm, &o, &base),
            _ => Vec::new(),
        };
        let mut scopes = Vec::new();
        if let Ok(s @ Value::Obj(sg)) = vm.get(g, Key::Str(scopes_key), &v) {
            for k in vm.own_keys(sg).unwrap_or_default() {
                let Value::Str(prefix) = vm.key_value(k) else { continue };
                let Some(prefix) = Url::parse(&prefix.to_rust(), Some(&base)).map(|u| u.href()) else { continue };
                let map = vm.get(sg, k, &s).unwrap_or(Value::Undefined);
                let map = specifier_map(vm, &map, &base);
                scopes.push((prefix, map));
            }
        }
        let h = host(vm);
        for (k, v) in imports {
            if !h.import_map.imports.iter().any(|(x, _)| *x == k) {
                h.import_map.imports.push((k, v));
            }
        }
        for (prefix, map) in scopes {
            match h.import_map.scopes.iter_mut().find(|(x, _)| *x == prefix) {
                Some((_, have)) => {
                    for (k, v) in map {
                        if !have.iter().any(|(x, _)| *x == k) {
                            have.push((k, v));
                        }
                    }
                }
                None => h.import_map.scopes.push((prefix, map)),
            }
        }
        h.import_map.imports.sort_by(|a, b| b.0.cmp(&a.0));
        h.import_map.scopes.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, map) in h.import_map.scopes.iter_mut() {
            map.sort_by(|a, b| b.0.cmp(&a.0));
        }
    }

    /// Starts loading a module script: one with an address (`src`, absolute), or an inline one (its text). Its
    /// load's number (it's ready when module_ready says).
    pub fn load_module(&mut self, src: Option<&str>, inline: Option<&str>) -> u32 {
        let vm = &mut self.vm;
        let base = host_ref(vm).base.href();
        vm.modules.script_base = base.clone();
        match (src, inline) {
            (Some(u), _) => vm.module_load(u),
            (None, text) => {
                // (An inline module isn't in the module map under an address of its own: no import finds it.)
                let n = vm.modules.list.len();
                let key = format!("{base}\u{1}inline {n}");
                vm.module_add(&key, &base, text.unwrap_or(""));
                vm.module_load(&key)
            }
        }
    }

    /// The modules' addresses the page should fetch now (each asked for once).
    pub fn modules_wanted(&mut self) -> Vec<String> {
        self.vm.module_wanted()
    }

    /// A module the page fetched (`key`: the module loader's, from modules_wanted): its text (or None: it couldn't
    /// be had).
    pub fn module_came(&mut self, key: &str, text: Option<&str>) {
        let url = js::module::split_key(key).0.to_string();
        match text {
            Some(t) => {
                self.vm.module_add(key, &url, t);
            }
            None => self.vm.module_failed(key, &format!("Failed to fetch module: {url}")),
        }
        self.vm.module_progress();
    }

    /// Whether a module script's modules have all come (or failed).
    pub fn module_ready(&self, id: u32) -> bool {
        self.vm.modules.ready.iter().any(|(i, _)| *i == id)
    }

    /// Runs a module script whose modules have come: linked, evaluated (what it throws, now or after an await,
    /// reported). False if it couldn't be had (the element's error event).
    pub fn run_module(&mut self, id: u32, name: &str) -> bool {
        let vm = &mut self.vm;
        let Some(pos) = vm.modules.ready.iter().position(|(i, _)| *i == id) else { return false };
        let (_, r) = vm.modules.ready.remove(pos);
        vm.steps = 0;
        vm.step_limit = STEPS;
        let fetched = !matches!(&r, Err(e) if is_type_error(vm, e));
        let r = r.and_then(|m| {
            vm.module_link(m)?;
            Ok(vm.module_evaluate(m))
        });
        vm.step_limit = u64::MAX;
        match r {
            Err(e) => {
                if fetched {
                    report(vm, &e, name);
                } else {
                    let text = super::describe_error(vm, &e);
                    host(vm).log(text);
                }
            }
            Ok(p) => {
                let f = vm.native("", 1, module_rejected);
                vm.perform_then(p, Value::Undefined, Value::Obj(f), Value::Undefined, Value::Undefined, Value::Undefined);
            }
        }
        self.checkpoint();
        fetched
    }
}

/// Whether an error is a TypeError (a module that couldn't be fetched, or a specifier that isn't one).
fn is_type_error(vm: &mut Vm, e: &Value) -> bool {
    let Value::Obj(g) = e else { return false };
    if !matches!(vm.heap.get(*g).class, Class::Error) {
        return false;
    }
    let p = vm.proto(js::vm::I::TypeErrorProto);
    vm.heap.get(*g).proto == Some(p)
}
