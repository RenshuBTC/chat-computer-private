//! The compiler's first pass (compiler.rs is the second): where each name lives.
//!
//! It finds every scope's names: a function's parameters, its `var`s and functions (hoisted to its top), each
//! block's `let`, `const`, `class` and functions, a catch's parameter, a class's own name and private names, a named
//! function expression's own name. Then every name each piece of code uses is resolved to the scope that declares
//! it; a name an inner function uses is "captured": it lives in its scope's environment on the heap, made as the
//! scope is entered. Every other name lives in a register of its function's frame. Inside `with`, and around a
//! direct `eval`, names can't be resolved before the code runs: they're kept in environments and looked up by name.
//!
//! A script's own top-level names are the global object's (and the global lexical scope's), shared with every other
//! script: they aren't bindings here; the compiler looks them up as globals.

use crate::ast::*;
use crate::names as n;
use crate::object::{CONST, FUNC_NAME, LET, VAR};
use crate::parser::bound_names;
use crate::string::Atom;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScopeKind {
    Script,
    Function,
    Block,
    Catch,
    Class,
    FuncName,
    With,
}

#[derive(Clone, Debug)]
pub struct Binding {
    pub name: Atom,
    pub kind: u8,
    pub captured: bool,
    pub reg: u32,
    pub slot: u16,
}

#[derive(Debug)]
pub struct Scope {
    pub kind: ScopeKind,
    pub parent: Option<usize>,
    /// The function its code runs in.
    pub func: usize,
    pub bindings: Vec<Binding>,
    /// It has an environment on the heap, made as it's entered.
    pub env: bool,
    /// A direct eval may look its names up, or add to them, as it runs.
    pub forced: bool,
}

#[derive(Debug, Default)]
pub struct FuncInfo {
    /// Its outermost scope (a named function expression's own name's), and its function scope.
    pub top: usize,
    pub body: usize,
    /// Its parameters that are names alone, by place.
    pub params: Vec<Option<Atom>>,
    /// The registers its names use (its parameters' first).
    pub registers: u32,
    /// Sloppy code's functions declared in blocks that are its vars too (Annex B).
    pub annex: Vec<Atom>,
    /// It makes an `arguments` object; a mapped one (a sloppy function's with only names as parameters, which are
    /// then in its environment).
    pub arguments: bool,
    pub mapped: bool,
    pub has_eval: bool,
}

pub struct Scopes {
    pub scopes: Vec<Scope>,
    pub funcs: Vec<FuncInfo>,
    map: BTreeMap<(usize, u8), usize>,
    fmap: BTreeMap<usize, usize>,
}

impl Scopes {
    /// The scope a syntax node made.
    pub fn scope(&self, at: usize, kind: ScopeKind) -> usize {
        *self.map.get(&(at, kind as u8)).expect("a scope the first pass made")
    }

    pub fn has_scope(&self, at: usize, kind: ScopeKind) -> Option<usize> {
        self.map.get(&(at, kind as u8)).copied()
    }

    /// The function a syntax node is.
    pub fn func(&self, at: usize) -> usize {
        *self.fmap.get(&at).expect("a function the first pass found")
    }

    pub fn funcs_at(&self, at: usize) -> Option<usize> {
        self.fmap.get(&at).copied()
    }
}

/// Hidden names in a class's scope: a computed field key's value (kept from when the class is made until its
/// fields are), or a private method (made once with the class, given to each instance), by the member's place.
pub const HIDDEN: u32 = 0xF000_0000;

pub fn hidden(member: usize) -> Atom {
    Atom(HIDDEN + member as u32)
}

/// A syntax node's address: its name in the maps.
pub fn addr<T>(r: &T) -> usize {
    r as *const T as usize
}

pub fn rc_addr<T>(r: &Rc<T>) -> usize {
    Rc::as_ptr(r) as usize
}

/// The key the instance fields' function of a class is found by (the class's address, plus one: no node's).
pub fn fields_addr(c: &Rc<Class>) -> usize {
    rc_addr(c) + 1
}

/// Works out the scopes of a script, or of code a direct eval runs (`eval`: its free names are the caller's).
pub fn analyze(body: &[Stmt], strict: bool, eval: bool) -> Result<Scopes, String> {
    let mut a = Analyzer { s: Scopes { scopes: Vec::new(), funcs: Vec::new(), map: BTreeMap::new(), fmap: BTreeMap::new() }, cur: 0, func: 0, strict };
    a.s.funcs.push(FuncInfo::default());
    let top = a.new_scope(ScopeKind::Script, body.as_ptr() as usize, None);
    a.s.funcs[0].top = top;
    a.s.funcs[0].body = top;
    a.cur = top;
    if eval {
        // An eval's let, const and class are its own; a strict eval's vars and functions too (a sloppy one's are
        // its caller's, declared as it runs).
        if strict {
            for name in var_names(body) {
                a.declare(top, name, VAR)?;
            }
        }
        a.declare_lexicals(body, top, if strict { Some(VAR) } else { None })?;
    } else if !strict {
        let mut lex = Vec::new();
        for s in body {
            let_names(s, &mut lex);
        }
        let mut names = Vec::new();
        annex_names(body, &mut Vec::new(), &mut names, false);
        names.retain(|x| !lex.contains(x));
        a.s.funcs[0].annex = names;
    }
    a.statements(body)?;
    a.finish();
    Ok(a.s)
}

/// Works out the scopes of a function made by `Function(...)` (its free names are the global ones).
pub fn analyze_function(f: &Rc<Func>) -> Result<Scopes, String> {
    let mut a = Analyzer { s: Scopes { scopes: Vec::new(), funcs: Vec::new(), map: BTreeMap::new(), fmap: BTreeMap::new() }, cur: 0, func: 0, strict: f.strict };
    a.s.funcs.push(FuncInfo::default());
    let top = a.new_scope(ScopeKind::Script, 0, None);
    a.s.funcs[0].top = top;
    a.s.funcs[0].body = top;
    a.cur = top;
    a.function(f, true)?;
    a.finish();
    Ok(a.s)
}

struct Analyzer {
    s: Scopes,
    cur: usize,
    func: usize,
    strict: bool,
}

type R = Result<(), String>;

impl Analyzer {
    fn new_scope(&mut self, kind: ScopeKind, at: usize, parent: Option<usize>) -> usize {
        let parent = if self.s.scopes.is_empty() { None } else { Some(parent.unwrap_or(self.cur)) };
        self.s.scopes.push(Scope { kind, parent, func: self.func, bindings: Vec::new(), env: false, forced: false });
        let i = self.s.scopes.len() - 1;
        self.s.map.insert((at, kind as u8), i);
        i
    }

    fn declare(&mut self, scope: usize, name: Atom, kind: u8) -> R {
        let s = &mut self.s.scopes[scope];
        if let Some(b) = s.bindings.iter().find(|b| b.name == name) {
            if kind == VAR && b.kind == VAR {
                return Ok(());
            }
            return Err(alloc::format!("Identifier has already been declared"));
        }
        s.bindings.push(Binding { name, kind, captured: false, reg: 0, slot: 0 });
        Ok(())
    }

    /// Declares the lexical names of a block's (or a function's, or an eval's) statements: let, const, class, and
    /// functions (`funcs`: how; None: not here).
    fn declare_lexicals(&mut self, stmts: &[Stmt], scope: usize, funcs: Option<u8>) -> R {
        for s in stmts {
            let s = match s {
                Stmt::Export(Export::Decl(d)) => &**d,
                s => s,
            };
            match s {
                Stmt::Var(k @ (VarKind::Let | VarKind::Const), decls) => {
                    for d in decls {
                        let mut names = Vec::new();
                        bound_names(&d.pat, &mut names);
                        for a in names {
                            self.declare(scope, a, if *k == VarKind::Let { LET } else { CONST })?;
                        }
                    }
                }
                Stmt::Class(c) => {
                    if let Some(a) = c.name {
                        self.declare(scope, a, LET)?;
                    }
                }
                Stmt::Func(f) => {
                    if let (Some(a), Some(kind)) = (f.name, funcs) {
                        // (Sloppy code may declare a function in a block twice.)
                        let sc = &mut self.s.scopes[scope];
                        match sc.bindings.iter().find(|b| b.name == a) {
                            Some(b) if b.kind == VAR => {}
                            Some(_) => return Err(alloc::format!("Identifier has already been declared")),
                            None => sc.bindings.push(Binding { name: a, kind, captured: false, reg: 0, slot: 0 }),
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A name used: the binding it resolves to is captured if it's another function's, or is reached through a
    /// with (whose code looks names up as it runs).
    fn reference(&mut self, name: Atom) {
        let mut s = Some(self.cur);
        let mut dynamic = false;
        while let Some(i) = s {
            let sc = &mut self.s.scopes[i];
            if sc.kind == ScopeKind::With {
                dynamic = true;
            }
            let other = sc.func != self.func;
            if let Some(b) = sc.bindings.iter_mut().find(|b| b.name == name) {
                if other || dynamic {
                    b.captured = true;
                }
                return;
            }
            s = sc.parent;
        }
    }

    fn statements(&mut self, stmts: &[Stmt]) -> R {
        for s in stmts {
            self.statement(s)?;
        }
        Ok(())
    }

    fn block(&mut self, stmts: &Vec<Stmt>) -> R {
        let saved = self.cur;
        let s = self.new_scope(ScopeKind::Block, addr(stmts), None);
        self.cur = s;
        self.declare_lexicals(stmts, s, Some(VAR))?;
        self.statements(stmts)?;
        self.cur = saved;
        Ok(())
    }

    fn declare_pattern(&mut self, scope: usize, p: &Pat, kind: u8) -> R {
        let mut names = Vec::new();
        bound_names(p, &mut names);
        for a in names {
            self.declare(scope, a, kind)?;
        }
        Ok(())
    }

    fn statement(&mut self, s: &Stmt) -> R {
        match s {
            Stmt::Var(_, decls) => {
                for d in decls {
                    self.pattern(&d.pat)?;
                    if let Some(e) = &d.init {
                        self.expr(e)?;
                    }
                }
            }
            Stmt::Func(f) => self.function(f, false)?,
            Stmt::Class(c) => self.class(c)?,
            Stmt::Expr(e) | Stmt::Throw(e) => self.expr(e)?,
            Stmt::Block(b) => self.block(b)?,
            Stmt::Empty | Stmt::Debugger | Stmt::Continue(_) | Stmt::Break(_) => {}
            Stmt::If(t, a, b) => {
                self.expr(t)?;
                self.sub_statement(a)?;
                if let Some(b) = b {
                    self.sub_statement(b)?;
                }
            }
            Stmt::For(init, test, update, body) => {
                let saved = self.cur;
                let sc = self.new_scope(ScopeKind::Block, addr(s), None);
                self.cur = sc;
                if let Some(ForInit::Var(k @ (VarKind::Let | VarKind::Const), decls)) = init {
                    for d in decls {
                        self.declare_pattern(sc, &d.pat, if *k == VarKind::Let { LET } else { CONST })?;
                    }
                }
                match init {
                    Some(ForInit::Var(_, decls)) => {
                        for d in decls {
                            self.pattern(&d.pat)?;
                            if let Some(e) = &d.init {
                                self.expr(e)?;
                            }
                        }
                    }
                    Some(ForInit::Expr(e)) => self.expr(e)?,
                    None => {}
                }
                if let Some(t) = test {
                    self.expr(t)?;
                }
                if let Some(u) = update {
                    self.expr(u)?;
                }
                self.sub_statement(body)?;
                self.cur = saved;
            }
            Stmt::ForIn(left, right, body) | Stmt::ForOf(left, right, body, _) => {
                self.expr(right)?;
                let saved = self.cur;
                let sc = self.new_scope(ScopeKind::Block, addr(s), None);
                self.cur = sc;
                if let ForLeft::Var(k @ (VarKind::Let | VarKind::Const), p) = left {
                    self.declare_pattern(sc, p, if *k == VarKind::Let { LET } else { CONST })?;
                }
                match left {
                    ForLeft::Var(_, p) | ForLeft::Pat(p) => self.pattern(p)?,
                }
                self.sub_statement(body)?;
                self.cur = saved;
            }
            Stmt::While(t, b) => {
                self.expr(t)?;
                self.sub_statement(b)?;
            }
            Stmt::DoWhile(b, t) => {
                self.sub_statement(b)?;
                self.expr(t)?;
            }
            Stmt::Return(e) => {
                if let Some(e) = e {
                    self.expr(e)?;
                }
            }
            Stmt::With(obj, body) => {
                self.expr(obj)?;
                let saved = self.cur;
                let sc = self.new_scope(ScopeKind::With, addr(s), None);
                self.cur = sc;
                self.sub_statement(body)?;
                self.cur = saved;
            }
            Stmt::Switch(d, cases) => {
                self.expr(d)?;
                let saved = self.cur;
                let sc = self.new_scope(ScopeKind::Block, addr(cases), None);
                self.cur = sc;
                for c in cases {
                    self.declare_lexicals(&c.body, sc, Some(VAR))?;
                }
                for c in cases {
                    if let Some(t) = &c.test {
                        self.expr(t)?;
                    }
                    self.statements(&c.body)?;
                }
                self.cur = saved;
            }
            Stmt::Labeled(_, b) => self.sub_statement(b)?,
            Stmt::Try(block, param, handler, finalizer) => {
                self.block(block)?;
                if let Some(h) = handler {
                    let saved = self.cur;
                    let sc = self.new_scope(ScopeKind::Catch, addr(h), None);
                    self.cur = sc;
                    if let Some(p) = param {
                        self.declare_pattern(sc, p, LET)?;
                        self.pattern(p)?;
                    }
                    self.block(h)?;
                    self.cur = saved;
                }
                if let Some(f) = finalizer {
                    self.block(f)?;
                }
            }
            Stmt::Import(_) => {}
            Stmt::Export(e) => match e {
                Export::Decl(d) => self.statement(d)?,
                Export::Default(e, _) => self.expr(e)?,
                Export::Names(names, from) => {
                    if from.is_none() {
                        for (local, _) in names {
                            self.reference(*local);
                        }
                    }
                }
                Export::All(..) => {}
            },
        }
        Ok(())
    }

    /// An if's branch or a loop's body: a function declared there is in a scope of its own.
    fn sub_statement(&mut self, s: &Stmt) -> R {
        if let Stmt::Func(_) = s {
            let saved = self.cur;
            let sc = self.new_scope(ScopeKind::Block, addr(s), None);
            self.cur = sc;
            self.declare_lexicals(core::slice::from_ref(s), sc, Some(VAR))?;
            self.statement(s)?;
            self.cur = saved;
            return Ok(());
        }
        self.statement(s)
    }

    fn pattern(&mut self, p: &Pat) -> R {
        match p {
            Pat::Ident(a) => self.reference(*a),
            Pat::Expr(e) => self.expr(e)?,
            Pat::Array(elems, rest) => {
                for e in elems.iter().flatten() {
                    self.pattern(&e.pat)?;
                    if let Some(d) = &e.default {
                        self.expr(d)?;
                    }
                }
                if let Some(r) = rest {
                    self.pattern(r)?;
                }
            }
            Pat::Object(props, rest) => {
                for p in props {
                    if let PropKey::Computed(e) = &p.key {
                        self.expr(e)?;
                    }
                    self.pattern(&p.pat)?;
                    if let Some(d) = &p.default {
                        self.expr(d)?;
                    }
                }
                if let Some(r) = rest {
                    self.pattern(r)?;
                }
            }
        }
        Ok(())
    }

    fn new_func(&mut self, at: usize) -> usize {
        self.s.funcs.push(FuncInfo::default());
        let f = self.s.funcs.len() - 1;
        self.s.fmap.insert(at, f);
        f
    }

    /// A function: `is_expr` if it's a function expression (a named one has its own name in a scope around it).
    fn function(&mut self, f: &Rc<Func>, is_expr: bool) -> R {
        let saved_cur = self.cur;
        let saved_func = self.func;
        let saved_strict = self.strict;
        self.strict = f.strict;
        let fi = self.new_func(rc_addr(f));
        self.func = fi;
        let mut top = None;
        if is_expr
            && f.kind == FuncKind::Normal
            && let Some(name) = f.name
        {
            let s = self.new_scope(ScopeKind::FuncName, rc_addr(f), None);
            self.s.scopes[s].bindings.push(Binding { name, kind: FUNC_NAME, captured: false, reg: 0, slot: 0 });
            self.cur = s;
            top = Some(s);
        }
        let fs = self.new_scope(ScopeKind::Function, rc_addr(f), None);
        self.s.funcs[fi].top = top.unwrap_or(fs);
        self.s.funcs[fi].body = fs;
        self.cur = fs;
        // Parameters (in sloppy code, a name may be given twice: the last wins).
        let mut params = Vec::new();
        for p in &f.params {
            params.push(if let Pat::Ident(a) = p.pat { Some(a) } else { None });
            let mut names = Vec::new();
            bound_names(&p.pat, &mut names);
            for a in names {
                let _ = self.declare(fs, a, VAR);
            }
        }
        if let Some(r) = &f.rest {
            let mut names = Vec::new();
            bound_names(r, &mut names);
            for a in names {
                let _ = self.declare(fs, a, VAR);
            }
        }
        self.s.funcs[fi].params = params;
        let stmts: &[Stmt] = match &f.body {
            Body::Block(stmts) => stmts,
            Body::Expr(_) => &[],
        };
        // `arguments`, if the code uses it and it isn't a parameter's, a function's or a lexical name.
        if f.kind != FuncKind::Arrow && f.kind != FuncKind::Fields && uses_arguments(f) && !self.s.scopes[fs].bindings.iter().any(|b| b.name == n::ARGUMENTS) {
            let mut lex = Vec::new();
            for s in stmts {
                let_names(s, &mut lex);
                if let Stmt::Func(g) = s
                    && let Some(a) = g.name
                {
                    lex.push(a);
                }
            }
            if !lex.contains(&n::ARGUMENTS) {
                self.declare(fs, n::ARGUMENTS, VAR)?;
                self.s.funcs[fi].arguments = true;
                if !f.strict && f.rest.is_none() && f.params.iter().all(|p| p.default.is_none() && matches!(p.pat, Pat::Ident(_))) {
                    self.s.funcs[fi].mapped = true;
                    for b in self.s.scopes[fs].bindings.iter_mut() {
                        if self.s.funcs[fi].params.contains(&Some(b.name)) {
                            b.captured = true;
                        }
                    }
                }
            }
        }
        for v in var_names(stmts) {
            self.declare(fs, v, VAR)?;
        }
        self.declare_lexicals(stmts, fs, Some(VAR))?;
        if !f.strict {
            let mut names = Vec::new();
            annex_names(stmts, &mut Vec::new(), &mut names, false);
            let mut annex = Vec::new();
            for a in names {
                if self.s.funcs[fi].params.contains(&Some(a)) {
                    continue;
                }
                match self.s.scopes[fs].bindings.iter().find(|b| b.name == a) {
                    Some(b) if b.kind != VAR => continue,
                    Some(_) => {}
                    None => self.s.scopes[fs].bindings.push(Binding { name: a, kind: VAR, captured: false, reg: 0, slot: 0 }),
                }
                annex.push(a);
            }
            self.s.funcs[fi].annex = annex;
        }
        for p in &f.params {
            if let Some(d) = &p.default {
                self.expr(d)?;
            }
            self.pattern(&p.pat)?;
        }
        if let Some(r) = &f.rest {
            self.pattern(r)?;
        }
        match &f.body {
            Body::Block(stmts) => self.statements(stmts)?,
            Body::Expr(e) => self.expr(e)?,
        }
        self.cur = saved_cur;
        self.func = saved_func;
        self.strict = saved_strict;
        Ok(())
    }

    /// A function of a class's that isn't in the syntax tree as one: its instance fields' initializers (one function
    /// for them all), a static field's initializer, a static block.
    fn class_part(&mut self, at: usize, run: &mut dyn FnMut(&mut Analyzer, usize) -> R) -> R {
        let saved_cur = self.cur;
        let saved_func = self.func;
        let fi = self.new_func(at);
        self.func = fi;
        let s = self.new_scope(ScopeKind::Function, at, None);
        self.s.funcs[fi].top = s;
        self.s.funcs[fi].body = s;
        self.cur = s;
        run(self, s)?;
        self.cur = saved_cur;
        self.func = saved_func;
        Ok(())
    }

    fn class(&mut self, c: &Rc<Class>) -> R {
        if let Some(e) = &c.extends {
            self.expr(e)?;
        }
        let saved = self.cur;
        let s = self.new_scope(ScopeKind::Class, rc_addr(c), None);
        self.cur = s;
        if let Some(name) = c.name {
            self.s.scopes[s].bindings.push(Binding { name, kind: CONST, captured: false, reg: 0, slot: 0 });
        }
        let mut has_instance = false;
        for (i, m) in c.members.iter().enumerate() {
            let (key, is_static, is_method) = match m {
                Member::Method { key, is_static, .. } => (key, *is_static, true),
                Member::Field { key, is_static, .. } => (key, *is_static, false),
                Member::StaticBlock(_) => continue,
            };
            if !is_static {
                has_instance |= !is_method || matches!(key, PropKey::Private(_));
            }
            if let PropKey::Private(a) = key
                && !self.s.scopes[s].bindings.iter().any(|b| b.name == *a)
            {
                self.s.scopes[s].bindings.push(Binding { name: *a, kind: CONST, captured: true, reg: 0, slot: 0 });
            }
            let keeps = match key {
                PropKey::Computed(_) => !is_method,
                PropKey::Private(_) => is_method && !is_static,
                _ => false,
            };
            if keeps {
                self.s.scopes[s].bindings.push(Binding { name: hidden(i), kind: CONST, captured: true, reg: 0, slot: 0 });
            }
        }
        if let Some(ctor) = &c.constructor {
            self.function(ctor, false)?;
        }
        for m in &c.members {
            match m {
                Member::Method { key, func, .. } => {
                    if let PropKey::Computed(e) = key {
                        self.expr(e)?;
                    }
                    self.function(func, false)?;
                }
                Member::Field { key: PropKey::Computed(e), .. } => self.expr(e)?,
                _ => {}
            }
        }
        if has_instance {
            self.class_part(fields_addr(c), &mut |a, _| {
                for m in &c.members {
                    if let Member::Field { value: Some(v), is_static: false, .. } = m {
                        a.expr(v)?;
                    }
                }
                Ok(())
            })?;
        }
        for m in &c.members {
            match m {
                Member::Field { value: Some(v), is_static: true, .. } => self.class_part(addr(v), &mut |a, _| a.expr(v))?,
                Member::StaticBlock(stmts) => self.class_part(addr(stmts), &mut |a, fs| {
                    for v in var_names(stmts) {
                        a.declare(fs, v, VAR)?;
                    }
                    a.declare_lexicals(stmts, fs, Some(VAR))?;
                    a.statements(stmts)
                })?,
                _ => {}
            }
        }
        self.cur = saved;
        Ok(())
    }

    fn expr(&mut self, e: &Expr) -> R {
        match e {
            Expr::Ident(a) => self.reference(*a),
            Expr::Num(_) | Expr::Str(_) | Expr::BigInt(_) | Expr::Bool(_) | Expr::Null | Expr::Regex(..) | Expr::This | Expr::NewTarget | Expr::ImportMeta | Expr::SuperMember(_) => {}
            Expr::Template(_, es) | Expr::Seq(es) => {
                for x in es {
                    self.expr(x)?;
                }
            }
            Expr::Tagged(t, _, es) => {
                self.expr(t)?;
                for x in es {
                    self.expr(x)?;
                }
            }
            Expr::Array(els) => {
                for el in els {
                    if let ArrayElem::Item(x) | ArrayElem::Spread(x) = el {
                        self.expr(x)?;
                    }
                }
            }
            Expr::Object(props) => {
                for p in props {
                    match p {
                        Prop::Init(k, v) => {
                            if let PropKey::Computed(e) = k {
                                self.expr(e)?;
                            }
                            self.expr(v)?;
                        }
                        Prop::Short(a, d) => {
                            self.reference(*a);
                            if let Some(d) = d {
                                self.expr(d)?;
                            }
                        }
                        Prop::Method(k, f, _) => {
                            if let PropKey::Computed(e) = k {
                                self.expr(e)?;
                            }
                            self.function(f, false)?;
                        }
                        Prop::Spread(v) | Prop::Proto(v) => self.expr(v)?,
                    }
                }
            }
            Expr::Func(f) => self.function(f, true)?,
            Expr::Class(c) => self.class(c)?,
            Expr::Unary(_, x) | Expr::Await(x) | Expr::Paren(x) | Expr::Chain(x) | Expr::ImportCall(x) | Expr::SuperIndex(x) | Expr::Update(x, _, _) => self.expr(x)?,
            Expr::Binary(_, a, b) | Expr::Logical(_, a, b) => {
                self.expr(a)?;
                self.expr(b)?;
            }
            Expr::Assign(_, p, v) => {
                self.pattern(p)?;
                self.expr(v)?;
            }
            Expr::Cond(a, b, c) => {
                self.expr(a)?;
                self.expr(b)?;
                self.expr(c)?;
            }
            Expr::Call(callee, args, _) => {
                if let Expr::Ident(n::EVAL) = &**callee {
                    self.direct_eval();
                }
                self.expr(callee)?;
                self.args(args)?;
            }
            Expr::New(callee, args) => {
                self.expr(callee)?;
                self.args(args)?;
            }
            Expr::SuperCall(args) => self.args(args)?,
            Expr::Member(o, _, _) => self.expr(o)?,
            Expr::Private(o, a, _) => {
                self.expr(o)?;
                self.reference(*a);
            }
            Expr::Index(o, k, _) => {
                self.expr(o)?;
                self.expr(k)?;
            }
            Expr::PrivateIn(a, o) => {
                self.reference(*a);
                self.expr(o)?;
            }
            Expr::Yield(x, _) => {
                if let Some(x) = x {
                    self.expr(x)?;
                }
            }
        }
        Ok(())
    }

    fn args(&mut self, args: &[Arg]) -> R {
        for a in args {
            match a {
                Arg::Item(x) | Arg::Spread(x) => self.expr(x)?,
            }
        }
        Ok(())
    }

    /// A direct eval here (perhaps: `eval` might be a variable of the code's own, which isn't known until the
    /// scopes are all found; then this is merely slower): this scope and every one around keep their names in
    /// environments.
    fn direct_eval(&mut self) {
        self.s.funcs[self.func].has_eval = true;
        let mut s = Some(self.cur);
        while let Some(i) = s {
            self.s.scopes[i].forced = true;
            s = self.s.scopes[i].parent;
        }
    }

    /// Decides each scope's environment, and each name's place.
    fn finish(&mut self) {
        let mut next: Vec<u32> = self.s.funcs.iter().map(|f| f.params.len() as u32).collect();
        let bodies: Vec<usize> = self.s.funcs.iter().map(|f| f.body).collect();
        for si in 0..self.s.scopes.len() {
            let s = &mut self.s.scopes[si];
            if s.forced {
                for b in s.bindings.iter_mut() {
                    b.captured = true;
                }
            }
            s.env = s.forced || s.kind == ScopeKind::With || s.bindings.iter().any(|b| b.captured);
            let f = s.func;
            let params = &self.s.funcs[f].params;
            let mut slot = 0u16;
            for b in s.bindings.iter_mut() {
                if b.captured {
                    b.slot = slot;
                    slot += 1;
                    continue;
                }
                if b.kind == FUNC_NAME {
                    continue;
                }
                // A parameter given by name alone (and once) is its argument's register.
                if bodies[f] == si
                    && let Some(i) = params.iter().position(|p| *p == Some(b.name))
                    && params.iter().filter(|p| **p == Some(b.name)).count() == 1
                {
                    b.reg = i as u32;
                    continue;
                }
                b.reg = next[f];
                next[f] += 1;
            }
        }
        for (f, r) in next.into_iter().enumerate() {
            self.s.funcs[f].registers = r;
        }
    }
}

/// The names declared with `var` in a function's (or script's) code, not looking into functions inside it.
pub fn var_names(stmts: &[Stmt]) -> Vec<Atom> {
    let mut out = Vec::new();
    for s in stmts {
        var_names_stmt(s, &mut out);
    }
    let mut seen = Vec::new();
    out.retain(|a| {
        if seen.contains(a) {
            false
        } else {
            seen.push(*a);
            true
        }
    });
    out
}

fn var_names_stmt(s: &Stmt, out: &mut Vec<Atom>) {
    match s {
        Stmt::Var(VarKind::Var, decls) => {
            for d in decls {
                bound_names(&d.pat, out);
            }
        }
        Stmt::Block(b) => b.iter().for_each(|s| var_names_stmt(s, out)),
        Stmt::If(_, a, b) => {
            var_names_stmt(a, out);
            if let Some(b) = b {
                var_names_stmt(b, out);
            }
        }
        Stmt::For(init, _, _, body) => {
            if let Some(ForInit::Var(VarKind::Var, decls)) = init {
                for d in decls {
                    bound_names(&d.pat, out);
                }
            }
            var_names_stmt(body, out);
        }
        Stmt::ForIn(left, _, body) | Stmt::ForOf(left, _, body, _) => {
            if let ForLeft::Var(VarKind::Var, p) = left {
                bound_names(p, out);
            }
            var_names_stmt(body, out);
        }
        Stmt::While(_, b) | Stmt::DoWhile(b, _) | Stmt::Labeled(_, b) | Stmt::With(_, b) => var_names_stmt(b, out),
        Stmt::Switch(_, cases) => cases.iter().for_each(|c| c.body.iter().for_each(|s| var_names_stmt(s, out))),
        Stmt::Try(b, _, h, f) => {
            b.iter().for_each(|s| var_names_stmt(s, out));
            if let Some(h) = h {
                h.iter().for_each(|s| var_names_stmt(s, out));
            }
            if let Some(f) = f {
                f.iter().for_each(|s| var_names_stmt(s, out));
            }
        }
        Stmt::Export(Export::Decl(d)) => var_names_stmt(d, out),
        _ => {}
    }
}

/// The let, const and class names a statement declares.
pub fn let_names(s: &Stmt, out: &mut Vec<Atom>) {
    match s {
        Stmt::Var(VarKind::Let | VarKind::Const, decls) => {
            for d in decls {
                bound_names(&d.pat, out);
            }
        }
        Stmt::Class(c) => {
            if let Some(a) = c.name {
                out.push(a);
            }
        }
        Stmt::Export(Export::Decl(d)) => let_names(d, out),
        _ => {}
    }
}

/// Sloppy code's functions declared in blocks (Annex B.3.3), where a var of the name wouldn't clash with a let,
/// const or class around them.
fn annex_names(stmts: &[Stmt], around: &mut Vec<Atom>, out: &mut Vec<Atom>, in_block: bool) {
    let before = around.len();
    if in_block {
        for s in stmts {
            let_names(s, around);
        }
    }
    for s in stmts {
        annex_stmt(s, around, out, in_block);
    }
    around.truncate(before);
}

fn annex_stmt(s: &Stmt, around: &mut Vec<Atom>, out: &mut Vec<Atom>, in_block: bool) {
    match s {
        Stmt::Func(f) if in_block => {
            if !f.is_async
                && !f.is_generator
                && let Some(a) = f.name
                && !around.contains(&a)
                && !out.contains(&a)
            {
                out.push(a);
            }
        }
        Stmt::Block(b) => annex_names(b, around, out, true),
        Stmt::If(_, a, b) => {
            annex_stmt(a, around, out, true);
            if let Some(b) = b {
                annex_stmt(b, around, out, true);
            }
        }
        Stmt::For(init, _, _, body) => {
            let before = around.len();
            if let Some(ForInit::Var(VarKind::Let | VarKind::Const, decls)) = init {
                for d in decls {
                    bound_names(&d.pat, around);
                }
            }
            annex_stmt(body, around, out, true);
            around.truncate(before);
        }
        Stmt::ForIn(left, _, body) | Stmt::ForOf(left, _, body, _) => {
            let before = around.len();
            if let ForLeft::Var(VarKind::Let | VarKind::Const, p) = left {
                bound_names(p, around);
            }
            annex_stmt(body, around, out, true);
            around.truncate(before);
        }
        Stmt::While(_, b) | Stmt::DoWhile(b, _) | Stmt::With(_, b) => annex_stmt(b, around, out, true),
        Stmt::Labeled(_, b) => annex_stmt(b, around, out, in_block),
        Stmt::Switch(_, cases) => {
            let before = around.len();
            for c in cases {
                for s in &c.body {
                    let_names(s, around);
                }
            }
            for c in cases {
                for s in &c.body {
                    annex_stmt(s, around, out, true);
                }
            }
            around.truncate(before);
        }
        Stmt::Try(b, p, h, f) => {
            annex_names(b, around, out, true);
            if let Some(h) = h {
                let before = around.len();
                if let Some(p) = p
                    && !matches!(p, Pat::Ident(_))
                {
                    bound_names(p, around);
                }
                annex_names(h, around, out, true);
                around.truncate(before);
            }
            if let Some(f) = f {
                annex_names(f, around, out, true);
            }
        }
        _ => {}
    }
}

/// Whether a function's own code (and its arrow functions', not its other functions') uses `arguments`, or has a
/// direct eval (which might).
fn uses_arguments(f: &Func) -> bool {
    let mut found = false;
    let mut visit = |e: &Expr| {
        if matches!(e, Expr::Ident(n::ARGUMENTS)) {
            found = true;
        }
        if let Expr::Call(c, _, _) = e
            && matches!(**c, Expr::Ident(n::EVAL))
        {
            found = true;
        }
    };
    for p in &f.params {
        if let Some(d) = &p.default {
            walk_expr(d, &mut visit);
        }
        walk_pat(&p.pat, &mut visit);
    }
    match &f.body {
        Body::Block(stmts) => stmts.iter().for_each(|s| walk_stmt(s, &mut visit)),
        Body::Expr(e) => walk_expr(e, &mut visit),
    }
    found
}

/// Visits every expression in a statement, going into arrow functions but not other functions.
fn walk_stmt(s: &Stmt, f: &mut dyn FnMut(&Expr)) {
    match s {
        Stmt::Var(_, decls) => {
            for d in decls {
                walk_pat(&d.pat, f);
                if let Some(e) = &d.init {
                    walk_expr(e, f);
                }
            }
        }
        Stmt::Expr(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => walk_expr(e, f),
        Stmt::Block(b) => b.iter().for_each(|s| walk_stmt(s, f)),
        Stmt::If(t, a, b) => {
            walk_expr(t, f);
            walk_stmt(a, f);
            if let Some(b) = b {
                walk_stmt(b, f);
            }
        }
        Stmt::For(init, t, u, b) => {
            match init {
                Some(ForInit::Var(_, decls)) => {
                    for d in decls {
                        walk_pat(&d.pat, f);
                        if let Some(e) = &d.init {
                            walk_expr(e, f);
                        }
                    }
                }
                Some(ForInit::Expr(e)) => walk_expr(e, f),
                None => {}
            }
            if let Some(t) = t {
                walk_expr(t, f);
            }
            if let Some(u) = u {
                walk_expr(u, f);
            }
            walk_stmt(b, f);
        }
        Stmt::ForIn(l, r, b) | Stmt::ForOf(l, r, b, _) => {
            match l {
                ForLeft::Var(_, p) | ForLeft::Pat(p) => walk_pat(p, f),
            }
            walk_expr(r, f);
            walk_stmt(b, f);
        }
        Stmt::While(t, b) | Stmt::DoWhile(b, t) | Stmt::With(t, b) => {
            walk_expr(t, f);
            walk_stmt(b, f);
        }
        Stmt::Labeled(_, b) => walk_stmt(b, f),
        Stmt::Switch(d, cases) => {
            walk_expr(d, f);
            for c in cases {
                if let Some(t) = &c.test {
                    walk_expr(t, f);
                }
                c.body.iter().for_each(|s| walk_stmt(s, f));
            }
        }
        Stmt::Try(b, p, h, fin) => {
            b.iter().for_each(|s| walk_stmt(s, f));
            if let Some(p) = p {
                walk_pat(p, f);
            }
            if let Some(h) = h {
                h.iter().for_each(|s| walk_stmt(s, f));
            }
            if let Some(fin) = fin {
                fin.iter().for_each(|s| walk_stmt(s, f));
            }
        }
        Stmt::Class(c) => walk_class(c, f),
        _ => {}
    }
}

fn walk_class(c: &Class, f: &mut dyn FnMut(&Expr)) {
    if let Some(e) = &c.extends {
        walk_expr(e, f);
    }
    for m in &c.members {
        if let Member::Method { key: PropKey::Computed(e), .. } | Member::Field { key: PropKey::Computed(e), .. } = m {
            walk_expr(e, f);
        }
    }
}

fn walk_pat(p: &Pat, f: &mut dyn FnMut(&Expr)) {
    match p {
        Pat::Ident(_) => {}
        Pat::Expr(e) => walk_expr(e, f),
        Pat::Array(els, rest) => {
            for el in els.iter().flatten() {
                walk_pat(&el.pat, f);
                if let Some(d) = &el.default {
                    walk_expr(d, f);
                }
            }
            if let Some(r) = rest {
                walk_pat(r, f);
            }
        }
        Pat::Object(props, rest) => {
            for p in props {
                if let PropKey::Computed(e) = &p.key {
                    walk_expr(e, f);
                }
                walk_pat(&p.pat, f);
                if let Some(d) = &p.default {
                    walk_expr(d, f);
                }
            }
            if let Some(r) = rest {
                walk_pat(r, f);
            }
        }
    }
}

fn walk_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match e {
        Expr::Template(_, es) | Expr::Seq(es) => es.iter().for_each(|x| walk_expr(x, f)),
        Expr::Tagged(t, _, es) => {
            walk_expr(t, f);
            es.iter().for_each(|x| walk_expr(x, f));
        }
        Expr::Array(els) => {
            for el in els {
                if let ArrayElem::Item(x) | ArrayElem::Spread(x) = el {
                    walk_expr(x, f);
                }
            }
        }
        Expr::Object(props) => {
            for p in props {
                match p {
                    Prop::Init(k, v) => {
                        if let PropKey::Computed(x) = k {
                            walk_expr(x, f);
                        }
                        walk_expr(v, f);
                    }
                    Prop::Short(_, Some(d)) => walk_expr(d, f),
                    Prop::Method(PropKey::Computed(x), _, _) => walk_expr(x, f),
                    Prop::Spread(v) | Prop::Proto(v) => walk_expr(v, f),
                    _ => {}
                }
            }
        }
        Expr::Func(func) if func.kind == FuncKind::Arrow => {
            for p in &func.params {
                if let Some(d) = &p.default {
                    walk_expr(d, f);
                }
                walk_pat(&p.pat, f);
            }
            match &func.body {
                Body::Block(stmts) => stmts.iter().for_each(|s| walk_stmt(s, f)),
                Body::Expr(x) => walk_expr(x, f),
            }
        }
        Expr::Class(c) => walk_class(c, f),
        Expr::Unary(_, x) | Expr::Await(x) | Expr::Paren(x) | Expr::Chain(x) | Expr::ImportCall(x) | Expr::SuperIndex(x) | Expr::Update(x, _, _) => walk_expr(x, f),
        Expr::Binary(_, a, b) | Expr::Logical(_, a, b) => {
            walk_expr(a, f);
            walk_expr(b, f);
        }
        Expr::Assign(_, p, v) => {
            walk_pat(p, f);
            walk_expr(v, f);
        }
        Expr::Cond(a, b, c) => {
            walk_expr(a, f);
            walk_expr(b, f);
            walk_expr(c, f);
        }
        Expr::Call(c, args, _) | Expr::New(c, args) => {
            walk_expr(c, f);
            for a in args {
                let (Arg::Item(x) | Arg::Spread(x)) = a;
                walk_expr(x, f);
            }
        }
        Expr::SuperCall(args) => {
            for a in args {
                let (Arg::Item(x) | Arg::Spread(x)) = a;
                walk_expr(x, f);
            }
        }
        Expr::Member(o, _, _) | Expr::Private(o, _, _) | Expr::PrivateIn(_, o) => walk_expr(o, f),
        Expr::Index(o, k, _) => {
            walk_expr(o, f);
            walk_expr(k, f);
        }
        Expr::Yield(Some(x), _) => walk_expr(x, f),
        _ => {}
    }
}
