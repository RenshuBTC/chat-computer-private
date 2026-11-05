//! The compiler's second pass (scope.rs is the first): the syntax tree written as bytecode (code.rs), a function at
//! a time. An expression leaves its value on the stack; a statement leaves the stack as it found it.
//!
//! A break or continue undoes what it leaves: a scope's environment, a for-in's enumerator, a for-of's iterator
//! (closed), a try's handler. One that leaves a `finally` goes through it: the finally's code is entered with what
//! to do after it in a register (0 go on, 1 throw, 2 return, 3 and up: jump out, by a table of where to). A throw,
//! and a return, are the VM's: it finds the frame's innermost handler (a catch, a finally, a for-of closing its
//! iterator) and goes there.

use crate::ast::*;
use crate::bigint::BigInt;
use crate::code::{ClassInfo, Code, Kind, Op};
use crate::names as n;
use crate::object::{CONST, FUNC_NAME, IMPORT, LET, VAR};
use crate::scope::{self, ScopeKind, Scopes, HIDDEN, addr, fields_addr, hidden, rc_addr};
use crate::string::{Atom, Atoms, JsStr};
use crate::value::number_to_string;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub struct CompileError {
    pub message: String,
}

fn err<T>(m: &str) -> Result<T, CompileError> {
    Err(CompileError { message: String::from(m) })
}

type R<T> = Result<T, CompileError>;

/// EndFinally's table when there's no jumping out; AsyncIterClose's kind register when it's a break's.
pub const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
enum Loc {
    Reg(u32, u8),
    Env(u16, u16, u8),
    Callee,
    Dynamic,
    Global,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Store {
    Init,
    Assign,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Label,
    Loop,
    Switch,
}

/// What a break or continue leaves on its way out.
enum Ctl {
    /// A statement it can go to.
    Jump { kind: Target, labels: Vec<Atom>, breaks: Vec<usize>, continues: Vec<usize> },
    /// A scope's environment.
    Env,
    /// A try's catch handler.
    Catch,
    /// A for-in's enumerator, or a for-of's iterator (and its handler), on the stack.
    Iter { for_of: bool, is_async: bool },
    /// A try's finally: the ways out through it (target, is a continue), and the jumps into it to patch.
    Finally { exits: Vec<(usize, bool)>, entries: Vec<usize> },
}

struct FState {
    code: Code,
    fi: usize,
    scope: usize,
    next_reg: u32,
    completion: Option<u32>,
    ctl: Vec<Ctl>,
    labels: Vec<Atom>,
    strings: BTreeMap<JsStr, u32>,
    /// `this` may not be bound yet (a derived constructor's, and its arrow functions').
    this_checked: bool,
    async_gen: bool,
    /// Optional chains being compiled: their `?.`s' jumps (and how many values below the one tested to pop).
    chains: Vec<Vec<(usize, u32)>>,
}

pub struct Compiler<'a> {
    atoms: &'a mut Atoms,
    sc: Scopes,
    source: Rc<str>,
    /// Free names are looked up as the code runs (eval's code).
    eval: bool,
    fs: Vec<FState>,
}

/// Compiles a script (or the code of an eval, `eval`: then its free names are its caller's, looked up as it runs).
pub fn compile_script(script: &Script, atoms: &mut Atoms, source: Rc<str>, eval: bool) -> R<Rc<Code>> {
    let sc = scope::analyze(&script.body, script.strict, eval).map_err(|m| CompileError { message: m })?;
    let mut c = Compiler { atoms, sc, source: source.clone(), eval, fs: Vec::new() };
    c.begin(0, n::EMPTY, if eval { Kind::Eval } else { Kind::Script }, script.strict);
    c.f().code.end = source.len();
    let completion = c.temp();
    c.f().completion = Some(completion);
    c.emit(Op::Undefined);
    c.emit(Op::InitLocal(completion));
    let top = c.sc.funcs[0].body;
    c.enter_scope(top);
    if !eval {
        c.global_decls(&script.body)?;
    } else if !script.strict {
        c.eval_decls(&script.body)?;
    } else {
        c.hoist(&script.body)?;
    }
    c.statements(&script.body)?;
    c.leave_scope(top);
    c.emit(Op::GetLocal(completion));
    c.emit(Op::Return);
    Ok(Rc::new(c.end()))
}

/// A module compiled: the code that makes its functions (run as it's linked, so that the modules importing them can
/// call them before it runs), its own code, its environment's names, and what it imports and exports.
pub struct ModuleCode {
    pub init: Rc<Code>,
    /// (An async function's code, if it awaits at its top level.)
    pub body: Rc<Code>,
    /// Its environment's names, by slot, and how each is declared (slot 0: `*module*`).
    pub names: Rc<[(Atom, u8)]>,
    /// The modules it imports from, as written (and their attributes), in the order they're first named.
    pub requests: Vec<Request>,
    /// Its imports: the request, the name imported (None: the module's namespace), and the slot it goes in.
    pub imports: Vec<(usize, Option<JsStr>, u16)>,
    /// Its own names it exports: the name exported, and the name's slot.
    pub locals: Vec<(JsStr, u16)>,
    /// Other modules' names it exports: the name exported, the request, and the name there (None: the other
    /// module's namespace, `export * as x from`).
    pub indirect: Vec<(JsStr, usize, Option<JsStr>)>,
    /// `export * from`'s requests.
    pub stars: Vec<usize>,
    pub tla: bool,
}

/// Compiles a module.
pub fn compile_module(script: &Script, atoms: &mut Atoms, source: Rc<str>) -> R<ModuleCode> {
    let sc = scope::analyze_module(&script.body).map_err(|m| CompileError { message: m })?;
    let mut c = Compiler { atoms, sc, source: source.clone(), eval: false, fs: Vec::new() };
    let top = c.sc.funcs[0].body;
    let slot = |c: &Compiler, a: Atom| c.sc.scopes[top].bindings.iter().find(|b| b.name == a).map_or(0, |b| b.slot);
    // Its functions, made as it's linked (its environment is the frame's from the start: no PushEnv).
    c.begin(0, n::EMPTY, Kind::Module, true);
    c.f().code.end = source.len();
    c.f().scope = top;
    for s in &script.body {
        let found = match s {
            Stmt::Func(f) => f.name.map(|a| (f, a)),
            Stmt::Export(Export::Decl(d)) => match &**d {
                Stmt::Func(f) => f.name.map(|a| (f, a)),
                _ => None,
            },
            Stmt::Export(Export::Default(e, name)) => match &**e {
                Expr::Func(f) => Some((f, name.unwrap_or(n::DEFAULT_EXPORT))),
                _ => None,
            },
            _ => None,
        };
        if let Some((f, binding)) = found {
            let i = c.function(f, f.name.unwrap_or(n::DEFAULT))?;
            c.emit(Op::Closure(i));
            c.store(binding, Store::Init, false);
        }
    }
    c.emit(Op::Undefined);
    c.emit(Op::Return);
    let init = Rc::new(c.end());
    // Its own code.
    c.begin(0, n::EMPTY, Kind::Module, true);
    c.f().code.is_async = script.tla;
    c.f().code.end = source.len();
    c.f().scope = top;
    c.statements(&script.body)?;
    c.emit(Op::Undefined);
    c.emit(Op::Return);
    let body = Rc::new(c.end());
    let mut names: Vec<(Atom, u8, u16)> = c.sc.scopes[top].bindings.iter().map(|b| (b.name, b.kind, b.slot)).collect();
    names.sort_by_key(|x| x.2);
    let names: Rc<[(Atom, u8)]> = names.into_iter().map(|(a, k, _)| (a, k)).collect();
    // What it imports and exports (ParseModule's entries).
    let mut requests: Vec<Request> = Vec::new();
    let request = |requests: &mut Vec<Request>, s: &Request| match requests.iter().position(|r| r == s) {
        Some(i) => i,
        None => {
            requests.push(s.clone());
            requests.len() - 1
        }
    };
    let mut imports = Vec::new();
    // (local, request, imported name)
    let mut imported: Vec<(Atom, usize, Option<Atom>)> = Vec::new();
    for s in &script.body {
        match s {
            Stmt::Import(im) => {
                let r = request(&mut requests, &im.from);
                for (name, local) in &im.names {
                    imports.push((r, name.map(|a| c.atoms.get(a).clone()), slot(&c, *local)));
                    imported.push((*local, r, *name));
                }
            }
            Stmt::Export(Export::Names(_, Some(from)) | Export::All(_, from)) => {
                request(&mut requests, from);
            }
            _ => {}
        }
    }
    let mut locals = Vec::new();
    let mut indirect = Vec::new();
    let mut stars = Vec::new();
    let text = |c: &Compiler, a: Atom| c.atoms.get(a).clone();
    let local_export = |c: &Compiler, locals: &mut Vec<(JsStr, u16)>, indirect: &mut Vec<(JsStr, usize, Option<JsStr>)>, ex: Atom, local: Atom| match imported.iter().find(|i| i.0 == local) {
        // (An imported name exported again: the other module's, or its namespace.)
        Some((_, r, name)) => indirect.push((text(c, ex), *r, name.map(|n| text(c, n)))),
        None => locals.push((text(c, ex), slot(c, local))),
    };
    for s in &script.body {
        let Stmt::Export(e) = s else { continue };
        match e {
            Export::Decl(d) => {
                let mut bound = Vec::new();
                match &**d {
                    Stmt::Var(_, decls) => {
                        for d in decls {
                            crate::parser::bound_names(&d.pat, &mut bound);
                        }
                    }
                    Stmt::Class(cl) => bound.extend(cl.name),
                    Stmt::Func(f) => bound.extend(f.name),
                    _ => {}
                }
                for a in bound {
                    local_export(&c, &mut locals, &mut indirect, a, a);
                }
            }
            Export::Default(e, name) => {
                let local = match &**e {
                    Expr::Func(_) | Expr::Class(_) => name.unwrap_or(n::DEFAULT_EXPORT),
                    _ => n::DEFAULT_EXPORT,
                };
                locals.push((text(&c, n::DEFAULT), slot(&c, local)));
            }
            Export::Names(names, None) => {
                for (local, ex) in names {
                    local_export(&c, &mut locals, &mut indirect, *ex, *local);
                }
            }
            Export::Names(names, Some(from)) => {
                let r = request(&mut requests, from);
                for (name, ex) in names {
                    indirect.push((text(&c, *ex), r, Some(text(&c, *name))));
                }
            }
            Export::All(Some(alias), from) => {
                let r = request(&mut requests, from);
                indirect.push((text(&c, *alias), r, None));
            }
            Export::All(None, from) => {
                let r = request(&mut requests, from);
                stars.push(r);
            }
        }
    }
    // (The only attribute there is: a module's type.)
    if let Some((k, _)) = requests.iter().flat_map(|r| r.attrs.iter()).find(|(k, _)| !k.is("type")) {
        return Err(CompileError { message: alloc::format!("The import attribute \"{}\" isn't supported", k.to_rust()) });
    }
    Ok(ModuleCode { init, body, names, requests, imports, locals, indirect, stars, tla: script.tla })
}

/// Compiles a function made by `Function(...)` (its free names are the global ones).
pub fn compile_function(f: &Rc<Func>, atoms: &mut Atoms, source: Rc<str>) -> R<Rc<Code>> {
    let sc = scope::analyze_function(f).map_err(|m| CompileError { message: m })?;
    let mut c = Compiler { atoms, sc, source, eval: false, fs: Vec::new() };
    c.begin(0, n::EMPTY, Kind::Script, f.strict);
    let i = c.function(f, f.name.unwrap_or(n::ANONYMOUS))?;
    let outer = c.end();
    Ok(outer.funcs[i as usize].clone())
}

fn strip(e: &Expr) -> &Expr {
    match e {
        Expr::Paren(x) => strip(x),
        e => e,
    }
}

fn is_anonymous(e: &Expr) -> bool {
    match strip(e) {
        Expr::Func(f) => f.name.is_none(),
        Expr::Class(c) => c.name.is_none(),
        _ => false,
    }
}

fn pat_name(p: &Pat) -> Atom {
    if let Pat::Ident(a) = p { *a } else { n::EMPTY }
}

fn binop(op: BinOp) -> Op {
    match op {
        BinOp::Add => Op::Add,
        BinOp::Sub => Op::Sub,
        BinOp::Mul => Op::Mul,
        BinOp::Div => Op::Div,
        BinOp::Rem => Op::Rem,
        BinOp::Exp => Op::Exp,
        BinOp::Shl => Op::Shl,
        BinOp::Shr => Op::Shr,
        BinOp::UShr => Op::UShr,
        BinOp::BitAnd => Op::BitAnd,
        BinOp::BitOr => Op::BitOr,
        BinOp::BitXor => Op::BitXor,
        BinOp::Eq => Op::Eq,
        BinOp::Ne => Op::Ne,
        BinOp::StrictEq => Op::StrictEq,
        BinOp::StrictNe => Op::StrictNe,
        BinOp::Lt => Op::Lt,
        BinOp::Gt => Op::Gt,
        BinOp::Le => Op::Le,
        BinOp::Ge => Op::Ge,
        BinOp::In => Op::In,
        BinOp::InstanceOf => Op::InstanceOf,
    }
}

fn method_flags(k: MethodKind) -> u8 {
    match k {
        MethodKind::Method => 0,
        MethodKind::Get => 1,
        MethodKind::Set => 2,
    }
}

fn logical_jump(l: LogOp) -> Op {
    match l {
        LogOp::And => Op::JumpIfFalseKeep(0),
        LogOp::Or => Op::JumpIfTrueKeep(0),
        LogOp::Coalesce => Op::JumpIfNotNullishKeep(0),
    }
}

impl<'a> Compiler<'a> {
    // ------------------------------------------------------------------------------------------------- the basics

    fn f(&mut self) -> &mut FState {
        self.fs.last_mut().expect("a function being compiled")
    }

    fn fr(&self) -> &FState {
        self.fs.last().expect("a function being compiled")
    }

    fn emit(&mut self, op: Op) -> usize {
        let ops = &mut self.f().code.ops;
        ops.push(op);
        ops.len() - 1
    }

    fn here(&self) -> usize {
        self.fr().code.ops.len()
    }

    fn patch(&mut self, at: usize) {
        let to = self.here() as u32;
        self.patch_to(at, to);
    }

    fn patch_to(&mut self, at: usize, to: u32) {
        let op = &mut self.f().code.ops[at];
        *op = match *op {
            Op::Jump(_) => Op::Jump(to),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(to),
            Op::JumpIfTrue(_) => Op::JumpIfTrue(to),
            Op::JumpIfFalseKeep(_) => Op::JumpIfFalseKeep(to),
            Op::JumpIfTrueKeep(_) => Op::JumpIfTrueKeep(to),
            Op::JumpIfNotNullishKeep(_) => Op::JumpIfNotNullishKeep(to),
            Op::JumpIfNullish(_) => Op::JumpIfNullish(to),
            Op::JumpIfNotUndefined(_) => Op::JumpIfNotUndefined(to),
            Op::Try(_) => Op::Try(to),
            Op::TryFinally(_) => Op::TryFinally(to),
            Op::IterNext(_) => Op::IterNext(to),
            Op::IterResult(_) => Op::IterResult(to),
            Op::ForInNext(_) => Op::ForInNext(to),
            other => panic!("patching {other:?}"),
        };
    }

    fn temp(&mut self) -> u32 {
        let f = self.f();
        f.next_reg += 1;
        f.next_reg - 1
    }

    fn string(&mut self, s: JsStr) -> u32 {
        let f = self.f();
        if let Some(i) = f.strings.get(&s) {
            return *i;
        }
        f.code.strings.push(s.clone());
        let i = f.code.strings.len() as u32 - 1;
        f.strings.insert(s, i);
        i
    }

    fn atom_str(&mut self, a: Atom) -> u32 {
        let s = if a.0 >= HIDDEN { JsStr::empty() } else { self.atoms.get(a).clone() };
        self.string(s)
    }

    fn message(&mut self, m: &str) -> u32 {
        self.string(JsStr::from_str(m))
    }

    fn num(&mut self, x: f64) {
        if (-2147483648.0..=2147483647.0).contains(&x) && (x as i32) as f64 == x && !(x == 0.0 && x.is_sign_negative()) {
            self.emit(Op::Int(x as i32));
        } else {
            self.emit(Op::Num(x));
        }
    }

    fn num_atom(&mut self, x: f64) -> Atom {
        self.atoms.intern_str(&number_to_string(x))
    }

    fn begin(&mut self, fi: usize, name: Atom, kind: Kind, strict: bool) {
        let mut code = Code::new(name, kind);
        code.strict = strict;
        code.source = Some(self.source.clone());
        let regs = self.sc.funcs[fi].registers;
        let top = self.sc.funcs[fi].top;
        self.fs.push(FState {
            code,
            fi,
            scope: top,
            next_reg: regs,
            completion: None,
            ctl: Vec::new(),
            labels: Vec::new(),
            strings: BTreeMap::new(),
            this_checked: false,
            async_gen: false,
            chains: Vec::new(),
        });
    }

    fn end(&mut self) -> Code {
        let f = self.fs.pop().expect("a function being compiled");
        let mut code = f.code;
        code.registers = f.next_reg;
        code
    }

    /// The finished function `code` added to the one being compiled: its index there.
    fn add_func(&mut self, code: Code) -> u32 {
        let funcs = &mut self.f().code.funcs;
        funcs.push(Rc::new(code));
        funcs.len() as u32 - 1
    }

    // --------------------------------------------------------------------------------------------- scopes, names

    /// A scope entered: its environment made (if it has one), its lexical names in registers set uninitialized.
    fn enter_scope(&mut self, s: usize) {
        self.f().scope = s;
        if self.sc.scopes[s].env {
            let t = self.table(s);
            self.emit(Op::PushEnv(t));
            self.f().ctl.push(Ctl::Env);
        }
        let regs: Vec<u32> = self.sc.scopes[s].bindings.iter().filter(|b| !b.captured && (b.kind == LET || b.kind == CONST)).map(|b| b.reg).collect();
        for r in regs {
            self.emit(Op::Empty);
            self.emit(Op::InitLocal(r));
        }
    }

    fn leave_scope(&mut self, s: usize) {
        if self.sc.scopes[s].env {
            self.emit(Op::PopEnv);
            self.f().ctl.pop();
        }
        if let Some(p) = self.sc.scopes[s].parent {
            self.f().scope = p;
        }
    }

    /// A scope's names (those in its environment), for PushEnv.
    fn table(&mut self, s: usize) -> u32 {
        let sc = &self.sc.scopes[s];
        let mut names: Vec<(Atom, u8, u16)> = sc.bindings.iter().filter(|b| b.captured).map(|b| (b.name, b.kind, b.slot)).collect();
        names.sort_by_key(|x| x.2);
        let names: Rc<[(Atom, u8)]> = names.into_iter().map(|(a, k, _)| (a, k)).collect();
        let var = matches!(sc.kind, ScopeKind::Function | ScopeKind::Script | ScopeKind::Module);
        let scopes = &mut self.f().code.scopes;
        scopes.push((names, var));
        scopes.len() as u32 - 1
    }

    fn resolve(&self, name: Atom) -> Loc {
        self.resolve_from(self.fr().scope, name)
    }

    fn resolve_from(&self, start: usize, name: Atom) -> Loc {
        let mut s = Some(start);
        let mut hops = 0u16;
        let mut dynamic = false;
        while let Some(i) = s {
            let sc = &self.sc.scopes[i];
            if sc.kind == ScopeKind::With {
                dynamic = true;
            }
            if let Some(b) = sc.bindings.iter().find(|b| b.name == name) {
                if dynamic {
                    return Loc::Dynamic;
                }
                if b.captured {
                    return Loc::Env(hops, b.slot, b.kind);
                }
                if b.kind == FUNC_NAME {
                    return Loc::Callee;
                }
                return Loc::Reg(b.reg, b.kind);
            }
            // (A direct eval in a function may add vars to it; at the top level, it adds globals, which a global
            // lookup finds anyway.)
            if sc.forced && sc.kind != ScopeKind::Script && sc.kind != ScopeKind::Module {
                dynamic = true;
            }
            if sc.env {
                hops += 1;
            }
            s = sc.parent;
        }
        if dynamic || self.eval { Loc::Dynamic } else { Loc::Global }
    }

    fn load(&mut self, name: Atom) {
        let loc = self.resolve(name);
        match loc {
            Loc::Global if name == n::UNDEFINED => {
                self.emit(Op::Undefined);
            }
            Loc::Global if name == n::NAN => {
                self.emit(Op::Num(f64::NAN));
            }
            Loc::Global if name == n::INFINITY => {
                self.emit(Op::Num(f64::INFINITY));
            }
            _ => self.load_loc(loc, name),
        }
    }

    fn load_loc(&mut self, loc: Loc, name: Atom) {
        match loc {
            Loc::Reg(r, k) if k == LET || k == CONST => {
                let s = self.atom_str(name);
                self.emit(Op::GetLocalChecked(r, s));
            }
            Loc::Reg(r, _) => {
                self.emit(Op::GetLocal(r));
            }
            Loc::Env(h, sl, k) if k == LET || k == CONST => {
                let s = self.atom_str(name);
                self.emit(Op::GetEnvChecked(h, sl, s));
            }
            Loc::Env(h, sl, IMPORT) => {
                let s = self.atom_str(name);
                self.emit(Op::GetImport(h, sl, s));
            }
            Loc::Env(h, sl, _) => {
                self.emit(Op::GetEnv(h, sl));
            }
            Loc::Callee => {
                self.emit(Op::Callee);
            }
            Loc::Dynamic => {
                self.emit(Op::GetName(name));
            }
            Loc::Global => {
                self.emit(Op::GetGlobal(name));
            }
        }
    }

    /// A class's private name, or one of its hidden names (always set before they're read).
    fn load_hidden(&mut self, name: Atom) {
        match self.resolve(name) {
            Loc::Reg(r, _) => self.emit(Op::GetLocal(r)),
            Loc::Env(h, s, _) => self.emit(Op::GetEnv(h, s)),
            Loc::Dynamic => self.emit(Op::GetName(name)),
            _ => {
                let m = self.message("Private field must be declared in an enclosing class");
                self.emit(Op::ThrowTypeError(m))
            }
        };
    }

    fn store_hidden(&mut self, name: Atom) {
        match self.resolve(name) {
            Loc::Reg(r, _) => self.emit(Op::InitLocal(r)),
            Loc::Env(h, s, _) => self.emit(Op::InitEnv(h, s)),
            _ => self.emit(Op::Pop),
        };
    }

    /// The value on the stack stored in a name (kept there if `keep`).
    fn store(&mut self, name: Atom, mode: Store, keep: bool) {
        let loc = self.resolve(name);
        self.store_loc(loc, name, mode, keep);
    }

    fn store_loc(&mut self, loc: Loc, name: Atom, mode: Store, keep: bool) {
        let strict = self.fr().code.strict;
        match (loc, mode) {
            (Loc::Reg(r, _), Store::Init) => {
                if keep {
                    self.emit(Op::Dup);
                }
                self.emit(Op::InitLocal(r));
            }
            (Loc::Env(h, s, _), Store::Init) => {
                if keep {
                    self.emit(Op::Dup);
                }
                self.emit(Op::InitEnv(h, s));
            }
            (Loc::Global, Store::Init) => {
                if keep {
                    self.emit(Op::Dup);
                }
                self.emit(Op::InitGlobal(name));
            }
            (Loc::Reg(r, k), Store::Assign) => {
                let s = self.atom_str(name);
                match k {
                    CONST => {
                        self.emit(Op::GetLocalChecked(r, s));
                        self.emit(Op::Pop);
                        self.emit(Op::ConstAssign(s));
                    }
                    LET => {
                        self.emit(Op::SetLocalChecked(r, s));
                    }
                    _ => {
                        self.emit(Op::SetLocal(r));
                    }
                }
                if !keep {
                    self.emit(Op::Pop);
                }
            }
            (Loc::Env(h, sl, k), Store::Assign) => {
                let s = self.atom_str(name);
                match k {
                    CONST => {
                        self.emit(Op::GetEnvChecked(h, sl, s));
                        self.emit(Op::Pop);
                        self.emit(Op::ConstAssign(s));
                    }
                    FUNC_NAME => {
                        if strict {
                            self.emit(Op::ConstAssign(s));
                        }
                    }
                    // (An import is always initialized, and never assignable.)
                    IMPORT => {
                        self.emit(Op::ConstAssign(s));
                    }
                    LET => {
                        self.emit(Op::SetEnvChecked(h, sl, s));
                    }
                    _ => {
                        self.emit(Op::SetEnv(h, sl));
                    }
                }
                if !keep {
                    self.emit(Op::Pop);
                }
            }
            (Loc::Callee, _) => {
                if strict {
                    let s = self.atom_str(name);
                    self.emit(Op::ConstAssign(s));
                }
                if !keep {
                    self.emit(Op::Pop);
                }
            }
            (Loc::Dynamic, _) => {
                self.emit(Op::SetName(name));
                if !keep {
                    self.emit(Op::Pop);
                }
            }
            (Loc::Global, Store::Assign) => {
                self.emit(Op::SetGlobal(name));
                if !keep {
                    self.emit(Op::Pop);
                }
            }
        }
    }

    // ----------------------------------------------------------------------------------------------- declarations

    /// Global code's declarations, made before it runs: its functions, vars and lexical names.
    fn global_decls(&mut self, body: &[Stmt]) -> R<()> {
        let mut decls: Vec<(Atom, Option<u32>, u8)> = Vec::new();
        self.func_decls(body, &mut decls)?;
        for v in scope::var_names(body) {
            if !decls.iter().any(|d| d.0 == v) {
                decls.push((v, None, VAR));
            }
        }
        for a in self.sc.funcs[0].annex.clone() {
            if !decls.iter().any(|d| d.0 == a) {
                decls.push((a, None, VAR));
            }
        }
        for s in body {
            let s = match s {
                Stmt::Export(Export::Decl(d)) => &**d,
                s => s,
            };
            match s {
                Stmt::Var(k @ (VarKind::Let | VarKind::Const), ds) => {
                    for d in ds {
                        let mut names = Vec::new();
                        crate::parser::bound_names(&d.pat, &mut names);
                        for a in names {
                            decls.push((a, None, if *k == VarKind::Let { LET } else { CONST }));
                        }
                    }
                }
                Stmt::Class(c) => {
                    if let Some(a) = c.name {
                        decls.push((a, None, LET));
                    }
                }
                _ => {}
            }
        }
        self.emit_decls(decls);
        Ok(())
    }

    /// A sloppy eval's vars and functions: its caller's, declared as it runs.
    fn eval_decls(&mut self, body: &[Stmt]) -> R<()> {
        let mut decls: Vec<(Atom, Option<u32>, u8)> = Vec::new();
        self.func_decls(body, &mut decls)?;
        for v in scope::var_names(body) {
            if !decls.iter().any(|d| d.0 == v) {
                decls.push((v, None, VAR));
            }
        }
        self.emit_decls(decls);
        Ok(())
    }

    fn emit_decls(&mut self, decls: Vec<(Atom, Option<u32>, u8)>) {
        if !decls.is_empty() {
            let code = &mut self.f().code;
            code.decls.push(decls);
            let i = code.decls.len() as u32 - 1;
            self.emit(Op::GlobalDecls(i));
        }
    }

    fn func_decls(&mut self, body: &[Stmt], decls: &mut Vec<(Atom, Option<u32>, u8)>) -> R<()> {
        for s in body {
            let s = match s {
                Stmt::Export(Export::Decl(d)) => &**d,
                s => s,
            };
            if let Stmt::Func(f) = s
                && let Some(name) = f.name
            {
                let i = self.function(f, name)?;
                match decls.iter_mut().find(|d| d.0 == name) {
                    Some(d) => d.1 = Some(i),
                    None => decls.push((name, Some(i), VAR)),
                }
            }
        }
        Ok(())
    }

    /// The functions a block (or a function's body) declares, made as it's entered.
    fn hoist(&mut self, stmts: &[Stmt]) -> R<()> {
        for s in stmts {
            let s = match s {
                Stmt::Export(Export::Decl(d)) => &**d,
                s => s,
            };
            if let Stmt::Func(f) = s
                && let Some(name) = f.name
            {
                let i = self.function(f, name)?;
                self.emit(Op::Closure(i));
                self.store(name, Store::Init, false);
            }
        }
        Ok(())
    }

    // -------------------------------------------------------------------------------------------------- functions

    /// Compiles a function (into the one being compiled's funcs: its index there).
    fn function(&mut self, f: &Rc<Func>, name: Atom) -> R<u32> {
        let fi = self.sc.func(rc_addr(f));
        let kind = match f.kind {
            FuncKind::Normal => Kind::Normal,
            FuncKind::Arrow => Kind::Arrow,
            FuncKind::Method => Kind::Method,
            FuncKind::Getter => Kind::Getter,
            FuncKind::Setter => Kind::Setter,
            FuncKind::Constructor { derived: true } => Kind::DerivedConstructor,
            FuncKind::Constructor { derived: false } => Kind::BaseConstructor,
            FuncKind::Fields => Kind::Fields,
        };
        let this_checked = kind == Kind::DerivedConstructor || (kind == Kind::Arrow && self.fs.last().is_some_and(|p| p.this_checked));
        self.begin(fi, name, kind, f.strict);
        let (top, body, arguments, mapped) = {
            let info = &self.sc.funcs[fi];
            (info.top, info.body, info.arguments, info.mapped)
        };
        {
            let st = self.f();
            st.this_checked = this_checked;
            st.async_gen = f.is_async && f.is_generator;
            let c = &mut st.code;
            c.is_async = f.is_async;
            c.is_generator = f.is_generator;
            c.params = f.params.len() as u32;
            c.length = f.params.iter().take_while(|p| p.default.is_none()).count() as u32;
            c.keeps_args = f.rest.is_some() || arguments;
            c.start = f.start;
            c.end = f.end;
        }
        // A named function expression's own name.
        if top != body {
            self.f().scope = top;
            if self.sc.scopes[top].env {
                let t = self.table(top);
                self.emit(Op::PushEnv(t));
                self.emit(Op::Callee);
                self.emit(Op::InitEnv(0, 0));
            }
        }
        self.enter_scope(body);
        if mapped {
            // Each argument's parameter's slot (of a name given twice, the last).
            let names: Vec<Option<Atom>> = f.params.iter().map(|p| if let Pat::Ident(a) = p.pat { Some(a) } else { None }).collect();
            let mut slots = Vec::new();
            for (i, a) in names.iter().enumerate() {
                let slot = match a {
                    Some(a) if !names[i + 1..].contains(&Some(*a)) => match self.resolve(*a) {
                        Loc::Env(0, s, _) => Some(s),
                        _ => None,
                    },
                    _ => None,
                };
                slots.push(slot);
            }
            let c = &mut self.f().code;
            c.arg_maps.push(slots);
            let m = c.arg_maps.len() as u32 - 1;
            self.emit(Op::MappedArguments(m));
            self.store(n::ARGUMENTS, Store::Init, false);
        } else if arguments {
            self.emit(Op::Arguments);
            self.store(n::ARGUMENTS, Store::Init, false);
        }
        for (i, p) in f.params.iter().enumerate() {
            let alias = matches!(p.pat, Pat::Ident(a) if matches!(self.resolve(a), Loc::Reg(r, _) if r == i as u32));
            if alias && p.default.is_none() {
                continue;
            }
            self.emit(Op::GetLocal(i as u32));
            if let Some(d) = &p.default {
                let j = self.emit(Op::JumpIfNotUndefined(0));
                self.emit(Op::Pop);
                self.expr_named(d, pat_name(&p.pat))?;
                self.patch(j);
            }
            self.assign_pattern(&p.pat, Store::Init)?;
        }
        if let Some(r) = &f.rest {
            self.emit(Op::Rest(f.params.len() as u32));
            self.assign_pattern(r, Store::Init)?;
        }
        if let Body::Block(stmts) = &f.body {
            self.hoist(stmts)?;
        }
        if f.is_generator {
            self.emit(Op::Generator);
        }
        match &f.body {
            Body::Block(stmts) => {
                self.statements(stmts)?;
                self.emit(Op::Undefined);
                self.emit(Op::Return);
            }
            Body::Expr(e) => {
                self.expr(e)?;
                self.emit(Op::Return);
            }
        }
        let code = self.end();
        Ok(self.add_func(code))
    }

    // ------------------------------------------------------------------------------------------------- statements

    fn statements(&mut self, stmts: &[Stmt]) -> R<()> {
        for s in stmts {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn block(&mut self, stmts: &Vec<Stmt>) -> R<()> {
        let s = self.sc.scope(addr(stmts), ScopeKind::Block);
        self.enter_scope(s);
        self.hoist(stmts)?;
        self.statements(stmts)?;
        self.leave_scope(s);
        Ok(())
    }

    /// An if's branch or a loop's body (a function declared there has a scope of its own).
    fn sub_stmt(&mut self, s: &Stmt) -> R<()> {
        if let Stmt::Func(_) = s {
            let sc = self.sc.scope(addr(s), ScopeKind::Block);
            self.enter_scope(sc);
            self.hoist(core::slice::from_ref(s))?;
            self.stmt(s)?;
            self.leave_scope(sc);
            return Ok(());
        }
        self.stmt(s)
    }

    fn push_jump(&mut self, kind: Target, labels: Vec<Atom>) -> usize {
        let ctl = &mut self.f().ctl;
        ctl.push(Ctl::Jump { kind, labels, breaks: Vec::new(), continues: Vec::new() });
        ctl.len() - 1
    }

    /// A breakable statement's end: its breaks go here, its continues to `cont`.
    fn pop_jump(&mut self, cont: Option<usize>) {
        let Some(Ctl::Jump { breaks, continues, .. }) = self.f().ctl.pop() else { panic!("the control stack") };
        for b in breaks {
            self.patch(b);
        }
        for c in continues {
            self.patch_to(c, cont.unwrap_or(0) as u32);
        }
    }

    fn find_target(&self, label: Option<Atom>, cont: bool) -> R<usize> {
        for (i, c) in self.fr().ctl.iter().enumerate().rev() {
            if let Ctl::Jump { kind, labels, .. } = c {
                let ok = match label {
                    Some(l) => labels.contains(&l) && (!cont || *kind == Target::Loop),
                    None => *kind == Target::Loop || (!cont && *kind == Target::Switch),
                };
                if ok {
                    return Ok(i);
                }
            }
        }
        err("Illegal break or continue")
    }

    /// A break or continue: what it leaves undone, then the jump.
    fn jump_out(&mut self, target: usize, cont: bool) {
        let from = self.fr().ctl.len();
        self.jump_out_from(from, target, cont);
    }

    fn jump_out_from(&mut self, from: usize, target: usize, cont: bool) {
        let mut d = from;
        while d > target + 1 {
            d -= 1;
            let (kind, for_of, is_async) = match &self.fr().ctl[d] {
                Ctl::Env => (0, false, false),
                Ctl::Catch => (1, false, false),
                Ctl::Iter { for_of, is_async } => (2, *for_of, *is_async),
                Ctl::Finally { .. } => (3, false, false),
                Ctl::Jump { .. } => (4, false, false),
            };
            match kind {
                0 => {
                    self.emit(Op::PopEnv);
                }
                1 => {
                    self.emit(Op::EndTry);
                }
                2 => {
                    let own = cont && target + 1 == d;
                    if for_of {
                        self.emit(Op::EndTry);
                        if !own {
                            if is_async {
                                self.emit(Op::AsyncIterClose(NONE));
                                self.emit(Op::Await);
                                self.emit(Op::Pop);
                            } else {
                                self.emit(Op::IterClose);
                            }
                        }
                    } else if !own {
                        self.emit(Op::Pop);
                    }
                }
                3 => {
                    let i = {
                        let Ctl::Finally { exits, .. } = &mut self.f().ctl[d] else { unreachable!() };
                        match exits.iter().position(|e| *e == (target, cont)) {
                            Some(i) => i,
                            None => {
                                exits.push((target, cont));
                                exits.len() - 1
                            }
                        }
                    };
                    self.emit(Op::EndTry);
                    self.emit(Op::Undefined);
                    self.emit(Op::Int(3 + i as i32));
                    let at = self.emit(Op::Jump(0));
                    if let Ctl::Finally { entries, .. } = &mut self.f().ctl[d] {
                        entries.push(at);
                    }
                    return;
                }
                _ => {}
            }
        }
        let at = self.emit(Op::Jump(0));
        if let Ctl::Jump { breaks, continues, .. } = &mut self.f().ctl[target] {
            if cont { continues.push(at) } else { breaks.push(at) }
        }
    }

    fn completion(&mut self) {
        match self.fr().completion {
            Some(r) => self.emit(Op::InitLocal(r)),
            None => self.emit(Op::Pop),
        };
    }

    fn stmt(&mut self, s: &Stmt) -> R<()> {
        let labels = core::mem::take(&mut self.f().labels);
        if let Some(r) = self.fr().completion
            && matches!(s, Stmt::If(..) | Stmt::For(..) | Stmt::ForIn(..) | Stmt::ForOf(..) | Stmt::While(..) | Stmt::DoWhile(..) | Stmt::Switch(..) | Stmt::Try(..) | Stmt::With(..))
        {
            self.emit(Op::Undefined);
            self.emit(Op::InitLocal(r));
        }
        match s {
            Stmt::Var(kind, decls) => self.var_decl(*kind, decls)?,
            Stmt::Func(f) => self.func_stmt(f),
            Stmt::Class(c) => {
                self.class(c, None)?;
                if let Some(a) = c.name {
                    self.store(a, Store::Init, false);
                } else {
                    self.emit(Op::Pop);
                }
            }
            Stmt::Expr(e) => {
                self.expr(e)?;
                self.completion();
            }
            Stmt::Block(b) => {
                if labels.is_empty() {
                    self.block(b)?;
                } else {
                    self.push_jump(Target::Label, labels);
                    self.block(b)?;
                    self.pop_jump(None);
                }
            }
            Stmt::Empty | Stmt::Debugger => {}
            Stmt::If(t, a, b) => {
                self.expr(t)?;
                let j = self.emit(Op::JumpIfFalse(0));
                self.sub_stmt(a)?;
                if let Some(b) = b {
                    let over = self.emit(Op::Jump(0));
                    self.patch(j);
                    self.sub_stmt(b)?;
                    self.patch(over);
                } else {
                    self.patch(j);
                }
            }
            Stmt::For(init, test, update, body) => {
                let sl = self.sc.scope(addr(s), ScopeKind::Block);
                self.enter_scope(sl);
                match init {
                    Some(ForInit::Var(k, decls)) => self.var_decl(*k, decls)?,
                    Some(ForInit::Expr(e)) => {
                        self.expr(e)?;
                        self.emit(Op::Pop);
                    }
                    None => {}
                }
                let per_iteration = self.sc.scopes[sl].env && matches!(init, Some(ForInit::Var(VarKind::Let | VarKind::Const, _)));
                if per_iteration {
                    self.emit(Op::CopyEnv);
                }
                let start = self.here();
                let exit = match test {
                    Some(t) => {
                        self.expr(t)?;
                        Some(self.emit(Op::JumpIfFalse(0)))
                    }
                    None => None,
                };
                self.push_jump(Target::Loop, labels);
                self.sub_stmt(body)?;
                let cont = self.here();
                if per_iteration {
                    self.emit(Op::CopyEnv);
                }
                if let Some(u) = update {
                    self.expr(u)?;
                    self.emit(Op::Pop);
                }
                self.emit(Op::Jump(start as u32));
                if let Some(x) = exit {
                    self.patch(x);
                }
                self.pop_jump(Some(cont));
                self.leave_scope(sl);
            }
            Stmt::ForIn(left, right, body) => {
                let sl = self.sc.scope(addr(s), ScopeKind::Block);
                self.expr(right)?;
                self.emit(Op::ForIn);
                self.push_jump(Target::Loop, labels);
                self.f().ctl.push(Ctl::Iter { for_of: false, is_async: false });
                let next = self.here();
                let done = self.emit(Op::ForInNext(0));
                self.enter_scope(sl);
                self.for_target(left)?;
                self.sub_stmt(body)?;
                self.leave_scope(sl);
                self.emit(Op::Jump(next as u32));
                self.patch(done);
                self.f().ctl.pop();
                self.emit(Op::Pop);
                self.pop_jump(Some(next));
            }
            Stmt::ForOf(left, right, body, is_await) => {
                let sl = self.sc.scope(addr(s), ScopeKind::Block);
                self.expr(right)?;
                self.emit(if *is_await { Op::GetAsyncIterator } else { Op::GetIterator });
                self.push_jump(Target::Loop, labels);
                self.f().ctl.push(Ctl::Iter { for_of: true, is_async: *is_await });
                let (val, kind, value) = (self.temp(), self.temp(), self.temp());
                let next = self.here();
                let done = if *is_await {
                    self.emit(Op::IterCall);
                    self.emit(Op::Await);
                    self.emit(Op::IterResult(0))
                } else {
                    self.emit(Op::IterNext(0))
                };
                self.emit(Op::InitLocal(val));
                let tf = self.emit(Op::TryFinally(0));
                self.enter_scope(sl);
                self.emit(Op::GetLocal(val));
                self.for_target(left)?;
                self.sub_stmt(body)?;
                self.leave_scope(sl);
                self.emit(Op::EndTry);
                self.emit(Op::Jump(next as u32));
                self.patch(done);
                self.emit(Op::Pop);
                self.emit(Op::Pop);
                let end = self.emit(Op::Jump(0));
                // Left by a throw or a return: the iterator closed first.
                self.patch(tf);
                self.emit(Op::InitLocal(kind));
                self.emit(Op::InitLocal(value));
                if *is_await {
                    self.emit(Op::AsyncIterClose(kind));
                    self.emit(Op::Await);
                    self.emit(Op::Pop);
                } else {
                    self.emit(Op::IterCloseAfter(kind));
                }
                self.emit(Op::EndFinally(kind, value, NONE));
                self.patch(end);
                self.f().ctl.pop();
                self.pop_jump(Some(next));
            }
            Stmt::While(t, body) => {
                let start = self.here();
                self.expr(t)?;
                let exit = self.emit(Op::JumpIfFalse(0));
                self.push_jump(Target::Loop, labels);
                self.sub_stmt(body)?;
                self.emit(Op::Jump(start as u32));
                self.patch(exit);
                self.pop_jump(Some(start));
            }
            Stmt::DoWhile(body, t) => {
                let start = self.here();
                self.push_jump(Target::Loop, labels);
                self.sub_stmt(body)?;
                let cont = self.here();
                self.expr(t)?;
                self.emit(Op::JumpIfTrue(start as u32));
                self.pop_jump(Some(cont));
            }
            Stmt::Continue(label) => {
                let t = self.find_target(*label, true)?;
                self.jump_out(t, true);
            }
            Stmt::Break(label) => {
                let t = self.find_target(*label, false)?;
                self.jump_out(t, false);
            }
            Stmt::Return(e) => {
                match e {
                    Some(e) => self.expr(e)?,
                    None => {
                        self.emit(Op::Undefined);
                    }
                }
                if self.fr().async_gen {
                    self.emit(Op::Await);
                }
                self.emit(Op::Return);
            }
            Stmt::With(obj, body) => {
                self.expr(obj)?;
                self.emit(Op::ToObject);
                self.emit(Op::PushWith);
                let ws = self.sc.scope(addr(s), ScopeKind::With);
                let saved = self.fr().scope;
                self.f().scope = ws;
                self.f().ctl.push(Ctl::Env);
                self.sub_stmt(body)?;
                self.f().ctl.pop();
                self.emit(Op::PopEnv);
                self.f().scope = saved;
            }
            Stmt::Switch(d, cases) => {
                self.expr(d)?;
                let dr = self.temp();
                self.emit(Op::InitLocal(dr));
                let cs = self.sc.scope(addr(cases), ScopeKind::Block);
                self.enter_scope(cs);
                for c in cases {
                    self.hoist(&c.body)?;
                }
                self.push_jump(Target::Switch, labels);
                let mut jumps = Vec::new();
                for c in cases {
                    match &c.test {
                        Some(t) => {
                            self.emit(Op::GetLocal(dr));
                            self.expr(t)?;
                            self.emit(Op::StrictEq);
                            jumps.push(Some(self.emit(Op::JumpIfTrue(0))));
                        }
                        None => jumps.push(None),
                    }
                }
                let to_default = self.emit(Op::Jump(0));
                let mut default_at = None;
                for (c, j) in cases.iter().zip(jumps) {
                    match j {
                        Some(j) => self.patch(j),
                        None => default_at = Some(self.here()),
                    }
                    self.statements(&c.body)?;
                }
                match default_at {
                    Some(d) => self.patch_to(to_default, d as u32),
                    None => self.patch(to_default),
                }
                self.pop_jump(None);
                self.leave_scope(cs);
            }
            Stmt::Labeled(a, body) => {
                let mut labels = labels;
                labels.push(*a);
                match &**body {
                    Stmt::For(..) | Stmt::ForIn(..) | Stmt::ForOf(..) | Stmt::While(..) | Stmt::DoWhile(..) | Stmt::Switch(..) | Stmt::Labeled(..) => {
                        self.f().labels = labels;
                        self.stmt(body)?;
                    }
                    _ => {
                        self.push_jump(Target::Label, labels);
                        self.sub_stmt(body)?;
                        self.pop_jump(None);
                    }
                }
            }
            Stmt::Throw(e) => {
                self.expr(e)?;
                self.emit(Op::Throw);
            }
            Stmt::Try(block, param, handler, finalizer) => self.try_stmt(block, param, handler, finalizer)?,
            Stmt::Import(_) => {}
            Stmt::Export(e) => match e {
                Export::Decl(d) => self.stmt(d)?,
                Export::Default(e, name) => match &**e {
                    // (Made as the module was linked.)
                    Expr::Func(_) => {}
                    Expr::Class(c) => {
                        self.class(c, if c.name.is_none() { Some(n::DEFAULT) } else { None })?;
                        self.store(name.unwrap_or(n::DEFAULT_EXPORT), Store::Init, false);
                    }
                    _ => {
                        self.expr_named(e, n::DEFAULT)?;
                        self.store(n::DEFAULT_EXPORT, Store::Init, false);
                    }
                },
                Export::Names(..) | Export::All(..) => {}
            },
        }
        Ok(())
    }

    fn try_stmt(&mut self, block: &Vec<Stmt>, param: &Option<Pat>, handler: &Option<Vec<Stmt>>, finalizer: &Option<Vec<Stmt>>) -> R<()> {
        let fin = match finalizer {
            Some(_) => {
                let depth = self.fr().ctl.len();
                self.f().ctl.push(Ctl::Finally { exits: Vec::new(), entries: Vec::new() });
                Some((depth, self.emit(Op::TryFinally(0))))
            }
            None => None,
        };
        match handler {
            Some(h) => {
                self.f().ctl.push(Ctl::Catch);
                let t = self.emit(Op::Try(0));
                self.block(block)?;
                self.f().ctl.pop();
                self.emit(Op::EndTry);
                let over = self.emit(Op::Jump(0));
                self.patch(t);
                let cs = self.sc.scope(addr(h), ScopeKind::Catch);
                self.enter_scope(cs);
                match param {
                    Some(p) => self.assign_pattern(p, Store::Init)?,
                    None => {
                        self.emit(Op::Pop);
                    }
                }
                self.block(h)?;
                self.leave_scope(cs);
                self.patch(over);
            }
            None => self.block(block)?,
        }
        if let (Some((depth, tf)), Some(f)) = (fin, finalizer) {
            let Some(Ctl::Finally { exits, entries }) = self.f().ctl.pop() else { panic!("the control stack") };
            self.emit(Op::EndTry);
            self.emit(Op::Undefined);
            self.emit(Op::Int(0));
            self.patch(tf);
            for e in entries {
                self.patch(e);
            }
            let (kind, value) = (self.temp(), self.temp());
            self.emit(Op::InitLocal(kind));
            self.emit(Op::InitLocal(value));
            // (A finally's own completion value doesn't count when it ends normally.)
            let saved = self.fr().completion.map(|r| {
                let t = self.temp();
                self.emit(Op::GetLocal(r));
                self.emit(Op::InitLocal(t));
                (r, t)
            });
            self.block(f)?;
            if let Some((r, t)) = saved {
                self.emit(Op::GetLocal(t));
                self.emit(Op::InitLocal(r));
            }
            let table = if exits.is_empty() {
                NONE
            } else {
                let jumps = &mut self.f().code.jumps;
                jumps.push(vec![0; exits.len()]);
                jumps.len() as u32 - 1
            };
            self.emit(Op::EndFinally(kind, value, table));
            if !exits.is_empty() {
                let after = self.emit(Op::Jump(0));
                for (i, (t, cont)) in exits.iter().enumerate() {
                    let at = self.here() as u32;
                    self.f().code.jumps[table as usize][i] = at;
                    self.jump_out_from(depth, *t, *cont);
                }
                self.patch(after);
            }
        }
        Ok(())
    }

    fn var_decl(&mut self, kind: VarKind, decls: &[Decl]) -> R<()> {
        let mode = if kind == VarKind::Var { Store::Assign } else { Store::Init };
        for d in decls {
            match &d.init {
                Some(e) => {
                    self.expr_named(e, pat_name(&d.pat))?;
                    self.assign_pattern(&d.pat, mode)?;
                }
                None if kind != VarKind::Var => {
                    self.emit(Op::Undefined);
                    self.assign_pattern(&d.pat, mode)?;
                }
                None => {}
            }
        }
        Ok(())
    }

    /// A function declaration's place: in a sloppy block, its value given to the var of its name too (Annex B).
    fn func_stmt(&mut self, f: &Rc<Func>) {
        let Some(name) = f.name else { return };
        let fi = self.fr().fi;
        let body = self.sc.funcs[fi].body;
        if self.fr().scope != body && self.sc.funcs[fi].annex.contains(&name) {
            self.load(name);
            let loc = self.resolve_from(body, name);
            self.store_loc(loc, name, Store::Assign, false);
        }
    }

    fn for_target(&mut self, left: &ForLeft) -> R<()> {
        match left {
            ForLeft::Var(VarKind::Var, p) | ForLeft::Pat(p) => self.assign_pattern(p, Store::Assign),
            ForLeft::Var(_, p) => self.assign_pattern(p, Store::Init),
        }
    }

    // --------------------------------------------------------------------------------------------------- patterns

    /// Stores the value on the stack in a pattern's names (or targets), taking it off.
    fn assign_pattern(&mut self, p: &Pat, mode: Store) -> R<()> {
        match p {
            Pat::Ident(a) => self.store(*a, mode, false),
            Pat::Expr(t) => self.assign_target(t)?,
            Pat::Array(elems, rest) => {
                self.emit(Op::GetIterator);
                let done = self.temp();
                self.emit(Op::False);
                self.emit(Op::InitLocal(done));
                for el in elems {
                    self.emit(Op::IterValue(done));
                    match el {
                        None => {
                            self.emit(Op::Pop);
                        }
                        Some(PatElem { pat, default }) => {
                            if let Some(d) = default {
                                let j = self.emit(Op::JumpIfNotUndefined(0));
                                self.emit(Op::Pop);
                                self.expr_named(d, pat_name(pat))?;
                                self.patch(j);
                            }
                            self.assign_pattern(pat, mode)?;
                        }
                    }
                }
                if let Some(r) = rest {
                    self.emit(Op::IterRest(done));
                    self.assign_pattern(r, mode)?;
                }
                self.emit(Op::IterEnd(done));
            }
            Pat::Object(props, rest) => {
                self.emit(Op::RequireObject);
                let mut keys: Vec<Result<u32, u32>> = Vec::new();
                for p in props {
                    self.emit(Op::Dup);
                    match &p.key {
                        PropKey::Name(a) => {
                            self.emit(Op::GetProp(*a));
                            let s = self.atom_str(*a);
                            keys.push(Ok(s));
                        }
                        PropKey::Num(x) => {
                            let a = self.num_atom(*x);
                            self.emit(Op::GetProp(a));
                            let s = self.atom_str(a);
                            keys.push(Ok(s));
                        }
                        PropKey::Computed(e) => {
                            self.expr(e)?;
                            self.emit(Op::ToKey);
                            if rest.is_some() {
                                let r = self.temp();
                                self.emit(Op::Dup);
                                self.emit(Op::InitLocal(r));
                                keys.push(Err(r));
                            }
                            self.emit(Op::GetElem);
                        }
                        PropKey::Private(_) => return err("Unexpected private name"),
                    }
                    if let Some(d) = &p.default {
                        let j = self.emit(Op::JumpIfNotUndefined(0));
                        self.emit(Op::Pop);
                        self.expr_named(d, pat_name(&p.pat))?;
                        self.patch(j);
                    }
                    self.assign_pattern(&p.pat, mode)?;
                }
                match rest {
                    Some(r) => {
                        for k in &keys {
                            match k {
                                Ok(s) => self.emit(Op::Str(*s)),
                                Err(r) => self.emit(Op::GetLocal(*r)),
                            };
                        }
                        self.emit(Op::CopyRest(keys.len() as u32));
                        self.assign_pattern(r, mode)?;
                    }
                    None => {
                        self.emit(Op::Pop);
                    }
                }
            }
        }
        Ok(())
    }

    /// The value on the stack stored in an expression assigned to (a property, or a name in parentheses).
    fn assign_target(&mut self, t: &Expr) -> R<()> {
        match strip(t) {
            Expr::Ident(a) => self.store(*a, Store::Assign, false),
            Expr::Member(o, a, _) => {
                self.expr(o)?;
                self.emit(Op::Swap);
                self.emit(Op::SetProp(*a));
                self.emit(Op::Pop);
            }
            Expr::Index(o, k, _) => {
                let r = self.temp();
                self.emit(Op::InitLocal(r));
                self.expr(o)?;
                self.expr(k)?;
                self.emit(Op::GetLocal(r));
                self.emit(Op::SetElem);
                self.emit(Op::Pop);
            }
            Expr::Private(o, a, _) => {
                let r = self.temp();
                self.emit(Op::InitLocal(r));
                self.expr(o)?;
                self.load_hidden(*a);
                self.emit(Op::GetLocal(r));
                self.emit(Op::SetPrivate);
                self.emit(Op::Pop);
            }
            Expr::SuperMember(a) => {
                self.emit(Op::SetSuper(*a));
                self.emit(Op::Pop);
            }
            Expr::SuperIndex(k) => {
                let r = self.temp();
                self.emit(Op::InitLocal(r));
                self.expr(k)?;
                self.emit(Op::GetLocal(r));
                self.emit(Op::SetSuperElem);
                self.emit(Op::Pop);
            }
            other => {
                self.emit(Op::Pop);
                self.expr(other)?;
                self.emit(Op::Pop);
                let m = self.message("Invalid left-hand side in assignment");
                self.emit(Op::ThrowRefError(m));
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ expressions

    /// An expression whose value, if it's an anonymous function or class, is given `name`.
    fn expr_named(&mut self, e: &Expr, name: Atom) -> R<()> {
        match strip(e) {
            Expr::Func(f) if f.name.is_none() => {
                let i = self.function(f, name)?;
                self.emit(Op::Closure(i));
                Ok(())
            }
            Expr::Class(c) if c.name.is_none() => self.class(c, Some(name)),
            _ => self.expr(e),
        }
    }

    /// A value whose key is on the stack under it (a computed key): an anonymous function's name is the key's.
    fn expr_keyed(&mut self, e: &Expr) -> R<()> {
        self.expr(e)?;
        if is_anonymous(e) {
            self.emit(Op::SetFunctionNameKey);
        }
        Ok(())
    }

    /// A property's key pushed (a computed one converted to a key).
    fn key(&mut self, k: &PropKey) -> R<()> {
        match k {
            PropKey::Name(a) => {
                let s = self.atom_str(*a);
                self.emit(Op::Str(s));
            }
            PropKey::Num(x) => {
                let s = self.string(JsStr::from_str(&number_to_string(*x)));
                self.emit(Op::Str(s));
            }
            PropKey::Computed(e) => {
                self.expr(e)?;
                self.emit(Op::ToKey);
            }
            PropKey::Private(a) => self.load_hidden(*a),
        }
        Ok(())
    }

    fn key_name(&mut self, k: &PropKey) -> Atom {
        match k {
            PropKey::Name(a) | PropKey::Private(a) => *a,
            PropKey::Num(x) => self.num_atom(*x),
            PropKey::Computed(_) => n::EMPTY,
        }
    }

    fn opt_check(&mut self, extra: u32) -> R<()> {
        let j = self.emit(Op::JumpIfNullish(0));
        match self.f().chains.last_mut() {
            Some(c) => {
                c.push((j, extra));
                Ok(())
            }
            None => err("Invalid optional chain"),
        }
    }

    fn expr(&mut self, e: &Expr) -> R<()> {
        match e {
            Expr::Num(x) => self.num(*x),
            Expr::Str(s) => {
                let i = self.string(s.clone());
                self.emit(Op::Str(i));
            }
            Expr::BigInt(text) => {
                let Some(b) = BigInt::parse_js(text) else { return err("Invalid BigInt") };
                let bigs = &mut self.f().code.bigints;
                bigs.push(Rc::new(b));
                let i = bigs.len() as u32 - 1;
                self.emit(Op::BigInt(i));
            }
            Expr::Bool(b) => {
                self.emit(if *b { Op::True } else { Op::False });
            }
            Expr::Null => {
                self.emit(Op::Null);
            }
            Expr::Regex(src, flags) => {
                match crate::regexp::Flags::parse(flags) {
                    None => return err("Invalid regular expression flags"),
                    Some(f) => {
                        if let Err(e) = crate::regexp::compile(src.units(), f) {
                            return Err(CompileError { message: alloc::format!("Invalid regular expression: /{}/: {}", src, e.0) });
                        }
                    }
                }
                let rs = &mut self.f().code.regexps;
                rs.push((src.clone(), flags.clone()));
                let i = rs.len() as u32 - 1;
                self.emit(Op::RegExp(i));
            }
            Expr::Template(strs, es) => {
                let s = self.string(strs[0].clone());
                self.emit(Op::Str(s));
                for (i, x) in es.iter().enumerate() {
                    self.expr(x)?;
                    self.emit(Op::ToStr);
                    self.emit(Op::Add);
                    if !strs[i + 1].is_empty() {
                        let s = self.string(strs[i + 1].clone());
                        self.emit(Op::Str(s));
                        self.emit(Op::Add);
                    }
                }
            }
            Expr::Tagged(tag, strings, es) => {
                self.callee(tag)?;
                let ts = &mut self.f().code.templates;
                ts.push((strings.cooked.clone(), strings.raw.clone()));
                let i = ts.len() as u32 - 1;
                self.emit(Op::Template(i));
                for x in es {
                    self.expr(x)?;
                }
                self.emit(Op::Call(1 + es.len() as u32));
            }
            Expr::Ident(a) => self.load(*a),
            Expr::This => {
                let op = if self.fr().this_checked { Op::ThisChecked } else { Op::This };
                self.emit(op);
            }
            Expr::NewTarget => {
                self.emit(Op::NewTarget);
            }
            Expr::ImportMeta => {
                self.emit(Op::ImportMeta);
            }
            Expr::Array(els) => {
                self.emit(Op::NewArray);
                for el in els {
                    match el {
                        ArrayElem::Hole => {
                            self.emit(Op::ArrayHole);
                        }
                        ArrayElem::Item(x) => {
                            self.expr(x)?;
                            self.emit(Op::ArrayPush);
                        }
                        ArrayElem::Spread(x) => {
                            self.expr(x)?;
                            self.emit(Op::ArraySpread);
                        }
                    }
                }
            }
            Expr::Object(props) => self.object(props)?,
            Expr::Func(f) => {
                let i = self.function(f, f.name.unwrap_or(n::EMPTY))?;
                self.emit(Op::Closure(i));
            }
            Expr::Class(c) => self.class(c, None)?,
            Expr::Unary(op, x) => self.unary(*op, x)?,
            Expr::Update(t, inc, prefix) => self.update(t, *inc, *prefix)?,
            Expr::Binary(op, a, b) => {
                self.expr(a)?;
                self.expr(b)?;
                self.emit(binop(*op));
            }
            Expr::Logical(op, a, b) => {
                self.expr(a)?;
                let j = self.emit(logical_jump(*op));
                self.expr(b)?;
                self.patch(j);
            }
            Expr::Assign(op, p, v) => self.assign(*op, p, v)?,
            Expr::Cond(t, a, b) => {
                self.expr(t)?;
                let j = self.emit(Op::JumpIfFalse(0));
                self.expr(a)?;
                let over = self.emit(Op::Jump(0));
                self.patch(j);
                self.expr(b)?;
                self.patch(over);
            }
            Expr::Call(callee, args, optional) => self.call(callee, args, *optional)?,
            Expr::New(callee, args) => {
                self.expr(callee)?;
                if args.iter().any(|a| matches!(a, Arg::Spread(_))) {
                    self.spread_args(args)?;
                    self.emit(Op::NewSpread);
                } else {
                    for a in args {
                        let (Arg::Item(x) | Arg::Spread(x)) = a;
                        self.expr(x)?;
                    }
                    self.emit(Op::New(args.len() as u32));
                }
            }
            Expr::Member(o, a, opt) => {
                self.expr(o)?;
                if *opt {
                    self.opt_check(0)?;
                }
                self.emit(Op::GetProp(*a));
            }
            Expr::Index(o, k, opt) => {
                self.expr(o)?;
                if *opt {
                    self.opt_check(0)?;
                }
                self.expr(k)?;
                self.emit(Op::GetElem);
            }
            Expr::Private(o, a, opt) => {
                self.expr(o)?;
                if *opt {
                    self.opt_check(0)?;
                }
                self.load_hidden(*a);
                self.emit(Op::GetPrivate);
            }
            Expr::PrivateIn(a, o) => {
                self.load_hidden(*a);
                self.expr(o)?;
                self.emit(Op::HasPrivate);
            }
            Expr::SuperMember(a) => {
                self.emit(Op::GetSuper(*a));
            }
            Expr::SuperIndex(k) => {
                self.expr(k)?;
                self.emit(Op::GetSuperElem);
            }
            Expr::SuperCall(args) => {
                if args.iter().any(|a| matches!(a, Arg::Spread(_))) {
                    self.spread_args(args)?;
                    self.emit(Op::SuperCallSpread);
                } else {
                    for a in args {
                        let (Arg::Item(x) | Arg::Spread(x)) = a;
                        self.expr(x)?;
                    }
                    self.emit(Op::SuperCall(args.len() as u32));
                }
            }
            Expr::Chain(x) => {
                self.f().chains.push(Vec::new());
                self.expr(x)?;
                let sites = self.f().chains.pop().unwrap_or_default();
                self.chain_end(sites, Op::Undefined);
            }
            Expr::Seq(es) => {
                for (i, x) in es.iter().enumerate() {
                    self.expr(x)?;
                    if i + 1 < es.len() {
                        self.emit(Op::Pop);
                    }
                }
            }
            Expr::Yield(x, delegate) => {
                let async_gen = self.fr().async_gen;
                match x {
                    Some(x) => self.expr(x)?,
                    None => {
                        self.emit(Op::Undefined);
                    }
                }
                if *delegate {
                    self.emit(if async_gen { Op::GetAsyncIterator } else { Op::GetIterator });
                    self.emit(Op::Undefined);
                    self.emit(Op::Int(0));
                    self.emit(Op::YieldStar);
                } else {
                    if async_gen {
                        self.emit(Op::Await);
                    }
                    self.emit(Op::Yield);
                }
            }
            Expr::Await(x) => {
                self.expr(x)?;
                self.emit(Op::Await);
            }
            Expr::ImportCall(x, o) => {
                self.expr(x)?;
                match o {
                    Some(o) => self.expr(o)?,
                    None => {
                        self.emit(Op::Undefined);
                    }
                }
                self.emit(Op::ImportCall);
            }
            Expr::Paren(x) => self.expr(x)?,
        }
        Ok(())
    }

    /// An optional chain's end: where its `?.`s go when they meet null or undefined (`value` pushed there).
    fn chain_end(&mut self, sites: Vec<(usize, u32)>, value: Op) {
        if sites.is_empty() {
            return;
        }
        let end = self.emit(Op::Jump(0));
        if sites.iter().any(|s| s.1 > 0) {
            for (j, _) in sites.iter().filter(|s| s.1 > 0) {
                self.patch(*j);
            }
            self.emit(Op::Pop);
        }
        for (j, _) in sites.iter().filter(|s| s.1 == 0) {
            self.patch(*j);
        }
        self.emit(value);
        self.patch(end);
    }

    /// Arguments with a spread among them: an array of them all.
    fn spread_args(&mut self, args: &[Arg]) -> R<()> {
        self.emit(Op::NewArray);
        for a in args {
            match a {
                Arg::Item(x) => {
                    self.expr(x)?;
                    self.emit(Op::ArrayPush);
                }
                Arg::Spread(x) => {
                    self.expr(x)?;
                    self.emit(Op::ArraySpread);
                }
            }
        }
        Ok(())
    }

    /// A call's function and its this pushed (f this).
    fn callee(&mut self, callee: &Expr) -> R<()> {
        match strip(callee) {
            Expr::Member(o, a, opt) => {
                self.expr(o)?;
                if *opt {
                    self.opt_check(0)?;
                }
                self.emit(Op::GetMethod(*a));
            }
            Expr::Index(o, k, opt) => {
                self.expr(o)?;
                if *opt {
                    self.opt_check(0)?;
                }
                self.expr(k)?;
                self.emit(Op::GetElemMethod);
            }
            Expr::Private(o, a, opt) => {
                self.expr(o)?;
                if *opt {
                    self.opt_check(0)?;
                }
                self.emit(Op::Dup);
                self.load_hidden(*a);
                self.emit(Op::GetPrivate);
                self.emit(Op::Swap);
            }
            Expr::SuperMember(a) => {
                self.emit(Op::GetSuperMethod(*a));
            }
            Expr::SuperIndex(k) => {
                self.expr(k)?;
                self.emit(Op::GetSuperElemMethod);
            }
            Expr::Ident(a) if matches!(self.resolve(*a), Loc::Dynamic) => {
                self.emit(Op::GetNameForCall(*a));
            }
            other => {
                self.expr(other)?;
                self.emit(Op::Undefined);
            }
        }
        Ok(())
    }

    fn call(&mut self, callee: &Expr, args: &[Arg], optional: bool) -> R<()> {
        let spread = args.iter().any(|a| matches!(a, Arg::Spread(_)));
        let direct_eval = matches!(callee, Expr::Ident(n::EVAL)) && !spread && matches!(self.resolve(n::EVAL), Loc::Global | Loc::Dynamic);
        if direct_eval {
            self.load(n::EVAL);
            self.emit(Op::Undefined);
        } else {
            self.callee(callee)?;
        }
        if optional {
            self.emit(Op::Swap);
            self.opt_check(1)?;
            self.emit(Op::Swap);
        }
        if spread {
            self.spread_args(args)?;
            self.emit(Op::CallSpread);
            return Ok(());
        }
        for a in args {
            let (Arg::Item(x) | Arg::Spread(x)) = a;
            self.expr(x)?;
        }
        self.emit(if direct_eval { Op::CallEval(args.len() as u32) } else { Op::Call(args.len() as u32) });
        Ok(())
    }

    fn object(&mut self, props: &[Prop]) -> R<()> {
        self.emit(Op::NewObject);
        for p in props {
            match p {
                Prop::Init(k, v) => match k {
                    PropKey::Name(a) => {
                        self.expr_named(v, *a)?;
                        self.emit(Op::DefineProp(*a));
                    }
                    PropKey::Num(x) => {
                        let a = self.num_atom(*x);
                        self.expr_named(v, a)?;
                        self.emit(Op::DefineProp(a));
                    }
                    PropKey::Computed(_) => {
                        self.key(k)?;
                        self.expr_keyed(v)?;
                        self.emit(Op::DefineElem);
                    }
                    PropKey::Private(_) => return err("Unexpected private name"),
                },
                Prop::Short(a, _) => {
                    self.load(*a);
                    self.emit(Op::DefineProp(*a));
                }
                Prop::Method(k, f, kind) => {
                    self.key(k)?;
                    let name = self.key_name(k);
                    let i = self.function(f, name)?;
                    self.emit(Op::Closure(i));
                    self.emit(Op::DefineMethod(method_flags(*kind) | 4));
                }
                Prop::Spread(v) => {
                    self.expr(v)?;
                    self.emit(Op::SpreadObject);
                }
                Prop::Proto(v) => {
                    self.expr(v)?;
                    self.emit(Op::SetProtoLiteral);
                }
            }
        }
        Ok(())
    }

    fn unary(&mut self, op: UnOp, x: &Expr) -> R<()> {
        match op {
            UnOp::Neg => {
                if let Expr::Num(v) = strip(x) {
                    self.num(-*v);
                } else {
                    self.expr(x)?;
                    self.emit(Op::Neg);
                }
            }
            UnOp::Plus => {
                self.expr(x)?;
                self.emit(Op::Pos);
            }
            UnOp::Not => {
                self.expr(x)?;
                self.emit(Op::Not);
            }
            UnOp::BitNot => {
                self.expr(x)?;
                self.emit(Op::BitNot);
            }
            UnOp::Void => {
                self.expr(x)?;
                self.emit(Op::Pop);
                self.emit(Op::Undefined);
            }
            UnOp::TypeOf => match strip(x) {
                Expr::Ident(a) => match self.resolve(*a) {
                    Loc::Global => {
                        self.emit(Op::TypeofGlobal(*a));
                    }
                    Loc::Dynamic => {
                        self.emit(Op::TypeofName(*a));
                    }
                    loc => {
                        self.load_loc(loc, *a);
                        self.emit(Op::TypeOf);
                    }
                },
                _ => {
                    self.expr(x)?;
                    self.emit(Op::TypeOf);
                }
            },
            UnOp::Delete => match strip(x) {
                Expr::Ident(a) => match self.resolve(*a) {
                    Loc::Global | Loc::Dynamic => {
                        self.emit(Op::DeleteName(*a));
                    }
                    _ => {
                        self.emit(Op::False);
                    }
                },
                Expr::Member(o, a, _) => {
                    self.expr(o)?;
                    self.emit(Op::DeleteProp(*a));
                }
                Expr::Index(o, k, _) => {
                    self.expr(o)?;
                    self.expr(k)?;
                    self.emit(Op::DeleteElem);
                }
                Expr::Chain(inner) => {
                    self.f().chains.push(Vec::new());
                    match strip(inner) {
                        Expr::Member(o, a, opt) => {
                            self.expr(o)?;
                            if *opt {
                                self.opt_check(0)?;
                            }
                            self.emit(Op::DeleteProp(*a));
                        }
                        Expr::Index(o, k, opt) => {
                            self.expr(o)?;
                            if *opt {
                                self.opt_check(0)?;
                            }
                            self.expr(k)?;
                            self.emit(Op::DeleteElem);
                        }
                        other => {
                            self.expr(other)?;
                            self.emit(Op::Pop);
                            self.emit(Op::True);
                        }
                    }
                    let sites = self.f().chains.pop().unwrap_or_default();
                    self.chain_end(sites, Op::True);
                }
                Expr::SuperMember(_) | Expr::SuperIndex(_) => {
                    let m = self.message("Unsupported reference to 'super'");
                    self.emit(Op::ThrowRefError(m));
                }
                other => {
                    self.expr(other)?;
                    self.emit(Op::Pop);
                    self.emit(Op::True);
                }
            },
        }
        Ok(())
    }

    fn update(&mut self, t: &Expr, inc: bool, prefix: bool) -> R<()> {
        let op = if inc { Op::Inc } else { Op::Dec };
        match strip(t) {
            Expr::Ident(a) => {
                self.load(*a);
                self.emit(Op::ToNumeric);
                if prefix {
                    self.emit(op);
                    self.store(*a, Store::Assign, true);
                } else {
                    self.emit(Op::Dup);
                    self.emit(op);
                    self.store(*a, Store::Assign, false);
                }
            }
            Expr::Member(o, a, _) => {
                self.expr(o)?;
                self.emit(Op::Dup);
                self.emit(Op::GetProp(*a));
                self.emit(Op::ToNumeric);
                if prefix {
                    self.emit(op);
                    self.emit(Op::SetProp(*a));
                } else {
                    self.emit(Op::Dup);
                    self.emit(Op::Rot3);
                    self.emit(op);
                    self.emit(Op::SetProp(*a));
                    self.emit(Op::Pop);
                }
            }
            Expr::Index(o, _, _) | Expr::Private(o, _, _) => {
                let private = if let Expr::Private(_, a, _) = strip(t) { Some(*a) } else { None };
                self.expr(o)?;
                match (private, strip(t)) {
                    (Some(a), _) => self.load_hidden(a),
                    (None, Expr::Index(_, k, _)) => {
                        self.expr(k)?;
                        self.emit(Op::CheckBase);
                        self.emit(Op::ToKey);
                    }
                    _ => {}
                }
                self.emit(Op::Dup2);
                self.emit(if private.is_some() { Op::GetPrivate } else { Op::GetElem });
                self.emit(Op::ToNumeric);
                let set = if private.is_some() { Op::SetPrivate } else { Op::SetElem };
                if prefix {
                    self.emit(op);
                    self.emit(set);
                } else {
                    self.emit(Op::Dup);
                    self.emit(Op::Rot4);
                    self.emit(op);
                    self.emit(set);
                    self.emit(Op::Pop);
                }
            }
            Expr::SuperMember(a) => {
                self.emit(Op::GetSuper(*a));
                self.emit(Op::ToNumeric);
                if prefix {
                    self.emit(op);
                    self.emit(Op::SetSuper(*a));
                } else {
                    self.emit(Op::Dup);
                    self.emit(op);
                    self.emit(Op::SetSuper(*a));
                    self.emit(Op::Pop);
                }
            }
            Expr::SuperIndex(k) => {
                self.expr(k)?;
                self.emit(Op::ToKey);
                self.emit(Op::Dup);
                self.emit(Op::GetSuperElem);
                self.emit(Op::ToNumeric);
                if prefix {
                    self.emit(op);
                    self.emit(Op::SetSuperElem);
                } else {
                    self.emit(Op::Dup);
                    self.emit(Op::Rot3);
                    self.emit(op);
                    self.emit(Op::SetSuperElem);
                    self.emit(Op::Pop);
                }
            }
            other => {
                self.expr(other)?;
                self.emit(Op::Pop);
                let m = self.message("Invalid left-hand side expression in update operation");
                self.emit(Op::ThrowRefError(m));
            }
        }
        Ok(())
    }

    fn assign(&mut self, op: AssignOp, p: &Pat, v: &Expr) -> R<()> {
        let target = match p {
            Pat::Ident(a) => {
                return self.assign_name(op, *a, v);
            }
            Pat::Expr(t) => strip(t),
            Pat::Array(..) | Pat::Object(..) => {
                // (Only `=` destructures.)
                self.expr(v)?;
                self.emit(Op::Dup);
                return self.assign_pattern(p, Store::Assign);
            }
        };
        if let Expr::Ident(a) = target {
            return self.assign_name(op, *a, v);
        }
        match op {
            AssignOp::Assign => match target {
                Expr::Member(o, a, _) => {
                    self.expr(o)?;
                    self.expr(v)?;
                    self.emit(Op::SetProp(*a));
                }
                Expr::Index(o, k, _) => {
                    self.expr(o)?;
                    self.expr(k)?;
                    self.expr(v)?;
                    self.emit(Op::SetElem);
                }
                Expr::Private(o, a, _) => {
                    self.expr(o)?;
                    self.load_hidden(*a);
                    self.expr(v)?;
                    self.emit(Op::SetPrivate);
                }
                Expr::SuperMember(a) => {
                    self.expr(v)?;
                    self.emit(Op::SetSuper(*a));
                }
                Expr::SuperIndex(k) => {
                    self.expr(k)?;
                    self.emit(Op::ToKey);
                    self.expr(v)?;
                    self.emit(Op::SetSuperElem);
                }
                other => {
                    self.expr(other)?;
                    self.emit(Op::Pop);
                    let m = self.message("Invalid left-hand side in assignment");
                    self.emit(Op::ThrowRefError(m));
                }
            },
            AssignOp::Op(b) => match target {
                Expr::Member(o, a, _) => {
                    self.expr(o)?;
                    self.emit(Op::Dup);
                    self.emit(Op::GetProp(*a));
                    self.expr(v)?;
                    self.emit(binop(b));
                    self.emit(Op::SetProp(*a));
                }
                Expr::Index(o, k, _) => {
                    self.expr(o)?;
                    self.expr(k)?;
                    self.emit(Op::CheckBase);
                    self.emit(Op::ToKey);
                    self.emit(Op::Dup2);
                    self.emit(Op::GetElem);
                    self.expr(v)?;
                    self.emit(binop(b));
                    self.emit(Op::SetElem);
                }
                Expr::Private(o, a, _) => {
                    self.expr(o)?;
                    self.load_hidden(*a);
                    self.emit(Op::Dup2);
                    self.emit(Op::GetPrivate);
                    self.expr(v)?;
                    self.emit(binop(b));
                    self.emit(Op::SetPrivate);
                }
                Expr::SuperMember(a) => {
                    self.emit(Op::GetSuper(*a));
                    self.expr(v)?;
                    self.emit(binop(b));
                    self.emit(Op::SetSuper(*a));
                }
                Expr::SuperIndex(k) => {
                    self.expr(k)?;
                    self.emit(Op::ToKey);
                    self.emit(Op::Dup);
                    self.emit(Op::GetSuperElem);
                    self.expr(v)?;
                    self.emit(binop(b));
                    self.emit(Op::SetSuperElem);
                }
                _ => return err("Invalid left-hand side in assignment"),
            },
            AssignOp::Logical(l) => match target {
                Expr::Member(o, a, _) => {
                    self.expr(o)?;
                    self.emit(Op::Dup);
                    self.emit(Op::GetProp(*a));
                    let j = self.emit(logical_jump(l));
                    self.expr_named(v, *a)?;
                    self.emit(Op::SetProp(*a));
                    let end = self.emit(Op::Jump(0));
                    self.patch(j);
                    self.emit(Op::Swap);
                    self.emit(Op::Pop);
                    self.patch(end);
                }
                Expr::Index(o, k, _) => {
                    self.expr(o)?;
                    self.expr(k)?;
                    self.emit(Op::CheckBase);
                    self.emit(Op::ToKey);
                    self.emit(Op::Dup2);
                    self.emit(Op::GetElem);
                    let j = self.emit(logical_jump(l));
                    self.expr(v)?;
                    self.emit(Op::SetElem);
                    let end = self.emit(Op::Jump(0));
                    self.patch(j);
                    self.emit(Op::Rot3);
                    self.emit(Op::Pop);
                    self.emit(Op::Pop);
                    self.patch(end);
                }
                Expr::Private(o, a, _) => {
                    self.expr(o)?;
                    self.load_hidden(*a);
                    self.emit(Op::Dup2);
                    self.emit(Op::GetPrivate);
                    let j = self.emit(logical_jump(l));
                    self.expr(v)?;
                    self.emit(Op::SetPrivate);
                    let end = self.emit(Op::Jump(0));
                    self.patch(j);
                    self.emit(Op::Rot3);
                    self.emit(Op::Pop);
                    self.emit(Op::Pop);
                    self.patch(end);
                }
                Expr::SuperMember(a) => {
                    self.emit(Op::GetSuper(*a));
                    let j = self.emit(logical_jump(l));
                    self.expr(v)?;
                    self.emit(Op::SetSuper(*a));
                    self.patch(j);
                }
                Expr::SuperIndex(k) => {
                    self.expr(k)?;
                    self.emit(Op::ToKey);
                    self.emit(Op::Dup);
                    self.emit(Op::GetSuperElem);
                    let j = self.emit(logical_jump(l));
                    self.expr(v)?;
                    self.emit(Op::SetSuperElem);
                    let end = self.emit(Op::Jump(0));
                    self.patch(j);
                    self.emit(Op::Swap);
                    self.emit(Op::Pop);
                    self.patch(end);
                }
                _ => return err("Invalid left-hand side in assignment"),
            },
        }
        Ok(())
    }

    fn assign_name(&mut self, op: AssignOp, a: Atom, v: &Expr) -> R<()> {
        match op {
            AssignOp::Assign => {
                self.expr_named(v, a)?;
                self.store(a, Store::Assign, true);
            }
            AssignOp::Op(b) => {
                self.load(a);
                self.expr(v)?;
                self.emit(binop(b));
                self.store(a, Store::Assign, true);
            }
            AssignOp::Logical(l) => {
                self.load(a);
                let j = self.emit(logical_jump(l));
                self.expr_named(v, a)?;
                self.store(a, Store::Assign, true);
                self.patch(j);
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------------- classes

    /// A class made: its constructor left on the stack.
    fn class(&mut self, c: &Rc<Class>, name: Option<Atom>) -> R<()> {
        let cname = c.name.or(name).unwrap_or(n::EMPTY);
        let derived = c.extends.is_some();
        if let Some(e) = &c.extends {
            self.expr(e)?;
        }
        let s = self.sc.scope(rc_addr(c), ScopeKind::Class);
        self.enter_scope(s);
        let privates: Vec<Atom> = self.sc.scopes[s].bindings.iter().map(|b| b.name).filter(|a| a.0 < HIDDEN && self.atoms.get(*a).at(0) == Some(b'#' as u16)).collect();
        for a in privates {
            let st = self.atom_str(a);
            self.emit(Op::PrivateName(st));
            self.store_hidden(a);
        }
        let ctor = match &c.constructor {
            Some(f) => self.function(f, cname)?,
            None => self.default_constructor(cname, derived, c.start, c.end),
        };
        let ci = {
            let classes = &mut self.f().code.classes;
            classes.push(ClassInfo { name: Some(cname), constructor: ctor, derived, start: c.start, end: c.end });
            classes.len() as u32 - 1
        };
        // -> ctor proto
        self.emit(Op::Class(ci, derived));
        for (i, m) in c.members.iter().enumerate() {
            match m {
                Member::Method { key, func, kind, is_static } => {
                    let flags = method_flags(*kind);
                    match key {
                        PropKey::Private(a) if *is_static => {
                            self.emit(Op::Over);
                            self.load_hidden(*a);
                            let f = self.function(func, *a)?;
                            self.emit(Op::Closure(f));
                            self.emit(Op::DefinePrivateMethod(flags));
                            self.emit(Op::Pop);
                        }
                        PropKey::Private(a) => {
                            // Made once, given to each instance (by the fields' initializer).
                            self.emit(Op::Dup);
                            let f = self.function(func, *a)?;
                            self.emit(Op::Closure(f));
                            self.emit(Op::SetHome);
                            self.store_hidden(hidden(i));
                        }
                        _ => {
                            self.emit(if *is_static { Op::Over } else { Op::Dup });
                            self.key(key)?;
                            let name = self.key_name(key);
                            let f = self.function(func, name)?;
                            self.emit(Op::Closure(f));
                            self.emit(Op::DefineMethod(flags));
                            self.emit(Op::Pop);
                        }
                    }
                }
                Member::Field { key: PropKey::Computed(e), .. } => {
                    self.expr(e)?;
                    self.emit(Op::ToKey);
                    self.store_hidden(hidden(i));
                }
                _ => {}
            }
        }
        if let Some(fi) = self.sc.funcs_at(fields_addr(c)) {
            let f = self.fields_function(c, fi)?;
            self.emit(Op::Over);
            self.emit(Op::Over);
            self.emit(Op::Closure(f));
            self.emit(Op::SetHome);
            self.emit(Op::SetFields);
            self.emit(Op::Pop);
        }
        self.emit(Op::Pop);
        if let Some(a) = c.name {
            self.emit(Op::Dup);
            self.store(a, Store::Init, false);
        }
        for (i, m) in c.members.iter().enumerate() {
            match m {
                Member::Field { key, value: None, is_static: true } => {
                    self.emit(Op::Dup);
                    self.field(key, i, None)?;
                    self.emit(Op::Pop);
                }
                Member::Field { key, value: Some(v), is_static: true } => {
                    let fi = self.sc.func(addr(v));
                    let f = self.static_part(fi, Some((key, i, v)), None)?;
                    self.run_static(f);
                }
                Member::StaticBlock(stmts) => {
                    let fi = self.sc.func(addr(stmts));
                    let f = self.static_part(fi, None, Some(stmts))?;
                    self.run_static(f);
                }
                _ => {}
            }
        }
        self.leave_scope(s);
        Ok(())
    }

    /// ctor -> ctor: a static field's initializer or a static block run, with the class as its this.
    fn run_static(&mut self, f: u32) {
        self.emit(Op::Dup);
        self.emit(Op::Dup);
        self.emit(Op::Closure(f));
        self.emit(Op::SetHome);
        self.emit(Op::Swap);
        self.emit(Op::Call(0));
        self.emit(Op::Pop);
    }

    fn field_key(&mut self, key: &PropKey, member: usize) {
        match key {
            PropKey::Computed(_) => self.load_hidden(hidden(member)),
            PropKey::Private(a) => self.load_hidden(*a),
            PropKey::Name(a) => {
                let s = self.atom_str(*a);
                self.emit(Op::Str(s));
            }
            PropKey::Num(x) => {
                let s = self.string(JsStr::from_str(&number_to_string(*x)));
                self.emit(Op::Str(s));
            }
        }
    }

    /// obj -> obj: a field defined on the object (its key, and its initializer's value).
    fn field(&mut self, key: &PropKey, member: usize, value: Option<&Expr>) -> R<()> {
        self.field_key(key, member);
        match value {
            Some(v) if matches!(key, PropKey::Computed(_)) => self.expr_keyed(v)?,
            Some(v) => {
                let name = self.key_name(key);
                self.expr_named(v, name)?;
            }
            None => {
                self.emit(Op::Undefined);
            }
        }
        self.emit(if matches!(key, PropKey::Private(_)) { Op::DefinePrivate } else { Op::DefineElem });
        Ok(())
    }

    /// The instance fields' initializer (private methods installed first), run on each new instance.
    fn fields_function(&mut self, c: &Rc<Class>, fi: usize) -> R<u32> {
        self.begin(fi, n::EMPTY, Kind::Fields, true);
        let body = self.sc.funcs[fi].body;
        self.enter_scope(body);
        for (i, m) in c.members.iter().enumerate() {
            if let Member::Method { key: PropKey::Private(a), kind, is_static: false, .. } = m {
                self.emit(Op::This);
                self.load_hidden(*a);
                self.load_hidden(hidden(i));
                self.emit(Op::DefinePrivateMethod(method_flags(*kind)));
                self.emit(Op::Pop);
            }
        }
        for (i, m) in c.members.iter().enumerate() {
            if let Member::Field { key, value, is_static: false } = m {
                self.emit(Op::This);
                self.field(key, i, value.as_ref())?;
                self.emit(Op::Pop);
            }
        }
        self.emit(Op::Undefined);
        self.emit(Op::Return);
        let code = self.end();
        Ok(self.add_func(code))
    }

    fn static_part(&mut self, fi: usize, field: Option<(&PropKey, usize, &Expr)>, block: Option<&Vec<Stmt>>) -> R<u32> {
        self.begin(fi, n::EMPTY, Kind::Fields, true);
        let body = self.sc.funcs[fi].body;
        self.enter_scope(body);
        if let Some((key, i, v)) = field {
            self.emit(Op::This);
            self.field(key, i, Some(v))?;
            self.emit(Op::Pop);
        }
        if let Some(stmts) = block {
            self.hoist(stmts)?;
            self.statements(stmts)?;
        }
        self.emit(Op::Undefined);
        self.emit(Op::Return);
        let code = self.end();
        Ok(self.add_func(code))
    }

    /// A class without a constructor: `constructor() {}`, or `constructor(...args) { super(...args) }`.
    fn default_constructor(&mut self, name: Atom, derived: bool, start: usize, end: usize) -> u32 {
        let mut code = Code::new(name, if derived { Kind::DerivedConstructor } else { Kind::BaseConstructor });
        code.strict = true;
        code.source = Some(self.source.clone());
        code.start = start;
        code.end = end;
        if derived {
            code.ops = vec![Op::Rest(0), Op::SuperCallSpread, Op::Return];
            code.keeps_args = true;
        } else {
            code.ops = vec![Op::Undefined, Op::Return];
        }
        self.add_func(code)
    }
}
