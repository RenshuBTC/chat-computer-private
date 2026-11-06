//! The virtual machine: the heap of objects, the stack, the frames of the functions running, and the interpreter
//! that runs their bytecode (code.rs). A JavaScript function called from JavaScript gets a frame of its own without
//! the interpreter calling itself; a built-in function (Rust) is called directly, and may call JavaScript back (a
//! nested run, until the function it called returns). A generator's or async function's frame is set aside when it
//! yields or awaits (with its part of the stack) and taken up again later.

use crate::code::{Code, Kind, Op};
use crate::compiler::NONE;
use crate::names as n;
use crate::object::*;
use crate::props::Desc;
use crate::string::{Atom, Atoms, JsStr};
use crate::value::{Gc, Value, strict_equals};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// The built-in objects every realm has, by number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum I {
    ObjectProto,
    FunctionProto,
    ArrayProto,
    StringProto,
    NumberProto,
    BooleanProto,
    SymbolProto,
    BigIntProto,
    ErrorProto,
    EvalErrorProto,
    RangeErrorProto,
    ReferenceErrorProto,
    SyntaxErrorProto,
    TypeErrorProto,
    UriErrorProto,
    AggregateErrorProto,
    IteratorProto,
    AsyncIteratorProto,
    ArrayIterProto,
    MapIterProto,
    SetIterProto,
    StringIterProto,
    RegExpStringIterProto,
    GeneratorProto,
    AsyncGeneratorProto,
    GeneratorFunctionProto,
    AsyncFunctionProto,
    AsyncGeneratorFunctionProto,
    AsyncFromSyncIterProto,
    PromiseProto,
    RegExpProto,
    DateProto,
    MapProto,
    SetProto,
    WeakMapProto,
    WeakSetProto,
    WeakRefProto,
    FinalizationRegistryProto,
    ArrayBufferProto,
    SharedArrayBufferProto,
    DataViewProto,
    TypedArrayProto,
    Int8ArrayProto,
    Uint8ArrayProto,
    Uint8ClampedArrayProto,
    Int16ArrayProto,
    Uint16ArrayProto,
    Int32ArrayProto,
    Uint32ArrayProto,
    Float32ArrayProto,
    Float64ArrayProto,
    BigInt64ArrayProto,
    BigUint64ArrayProto,
    ObjectCtor,
    FunctionCtor,
    ArrayCtor,
    PromiseCtor,
    TypedArrayCtor,
    Eval,
    ThrowTypeError,
    ArrayValues,
    ArrayIterNext,
    PromiseThen,
    RegExpCtor,
    RegExpExec,
    ArrayBufferCtor,
    // (The typed arrays' constructors, in the order of KINDS in builtins/typed.rs.)
    Int8ArrayCtor,
    Uint8ArrayCtor,
    Uint8ClampedArrayCtor,
    Int16ArrayCtor,
    Uint16ArrayCtor,
    Int32ArrayCtor,
    Uint32ArrayCtor,
    Float32ArrayCtor,
    Float64ArrayCtor,
    BigInt64ArrayCtor,
    BigUint64ArrayCtor,
    // Intl's prototypes (builtins/intl.rs).
    IntlNumberFormatProto,
    IntlDateTimeFormatProto,
    IntlCollatorProto,
    IntlPluralRulesProto,
    IntlRelativeTimeFormatProto,
    IntlListFormatProto,
    IntlSegmenterProto,
    IntlDisplayNamesProto,
    IntlLocaleProto,
    IntlSegmentsProto,
    IntlSegmentIterProto,
    // The Iterator helpers' (builtins/iterhelp.rs).
    IteratorCtor,
    IteratorHelperProto,
    WrapForValidIterProto,
    Count,
}

/// The well-known symbols.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Sym {
    AsyncIterator,
    HasInstance,
    IsConcatSpreadable,
    Iterator,
    Match,
    MatchAll,
    Replace,
    Search,
    Species,
    Split,
    ToPrimitive,
    ToStringTag,
    Unscopables,
}

pub const SYMBOL_NAMES: [&str; 13] = ["asyncIterator", "hasInstance", "isConcatSpreadable", "iterator", "match", "matchAll", "replace", "search", "species", "split", "toPrimitive", "toStringTag", "unscopables"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorKind {
    Error,
    Eval,
    Range,
    Reference,
    Syntax,
    Type,
    Uri,
    Aggregate,
}

impl ErrorKind {
    pub fn proto(self) -> I {
        match self {
            ErrorKind::Error => I::ErrorProto,
            ErrorKind::Eval => I::EvalErrorProto,
            ErrorKind::Range => I::RangeErrorProto,
            ErrorKind::Reference => I::ReferenceErrorProto,
            ErrorKind::Syntax => I::SyntaxErrorProto,
            ErrorKind::Type => I::TypeErrorProto,
            ErrorKind::Uri => I::UriErrorProto,
            ErrorKind::Aggregate => I::AggregateErrorProto,
        }
    }
}

/// How a run of the interpreter ended (its first frame's doing).
pub enum Signal {
    Return(Value),
    Yield(Value),
    /// yield*'s: the inner iterator's own result object, given as it is.
    YieldRaw(Value),
    Await(Value),
    /// A generator's frame, ready (its parameters set): it waits for the first next().
    Start,
}

/// How a frame taken up again goes on.
pub enum Resume {
    Normal,
    Throw(Value),
    Return(Value),
}

/// A try's handler: where it goes, the stack's height and the scope to go back to.
#[derive(Clone, Copy, Debug)]
pub struct Handler {
    pub pc: u32,
    pub sp: usize,
    pub env: Option<Gc>,
    pub finally: bool,
}

pub struct Frame {
    pub code: Rc<Code>,
    pub pc: usize,
    /// Where its registers start on the stack, and what the stack goes back to when it returns.
    pub base: usize,
    pub bottom: usize,
    pub env: Option<Gc>,
    pub this: Value,
    pub callee: Option<Gc>,
    pub new_target: Value,
    pub home: Option<Gc>,
    /// All its arguments (when its code needs them: `arguments`, a rest parameter).
    pub args: Vec<Value>,
    pub handlers: Vec<Handler>,
    /// Called by `new`: what it returns is its this unless that's an object.
    pub construct: bool,
    /// The generator or async function it runs for.
    pub owner: Option<Gc>,
}

impl Frame {
    pub fn trace(&self, work: &mut Vec<Gc>) {
        let mut v = |x: &Value| {
            if let Value::Obj(g) | Value::Sym(g) = x {
                work.push(*g);
            }
        };
        v(&self.this);
        v(&self.new_target);
        for a in &self.args {
            v(a);
        }
        for g in [self.env, self.callee, self.home, self.owner].into_iter().flatten() {
            work.push(g);
        }
        for h in &self.handlers {
            if let Some(e) = h.env {
                work.push(e);
            }
        }
        self.code.trace(work);
    }
}

/// A job waiting in the microtask queue.
pub enum Job {
    /// A promise's reaction: its handler called with the promise's value (a generator's object as the handler: an
    /// await's continuation), and the derived promise (or its capability's functions) settled with the result.
    Reaction { handler: Value, arg: Value, rejected: bool, derived: Value, resolve: Value, reject: Value },
    /// A thenable given to a promise's resolve: its then called with the promise's resolving functions.
    Thenable { promise: Gc, thenable: Value, then: Value },
    /// A function to call (queueMicrotask's, a host's).
    Call { f: Value, args: Vec<Value> },
}

pub const GEN_SYNC: u8 = 0;
pub const GEN_ASYNC_FN: u8 = 1;
pub const GEN_ASYNC: u8 = 2;

pub struct Vm {
    pub heap: Heap,
    pub atoms: Atoms,
    pub stack: Vec<Value>,
    pub frames: Vec<Frame>,
    pub global: Gc,
    /// The global let, const and class bindings (Empty: not yet initialized), and which are constants.
    pub global_lex: BTreeMap<Atom, (Value, u8)>,
    pub intrinsics: Vec<Gc>,
    pub symbols: Vec<Gc>,
    pub registry: BTreeMap<JsStr, Gc>,
    pub jobs: VecDeque<Job>,
    /// How deep the interpreter is in itself (built-in functions calling JavaScript back), and the most it may be.
    pub nesting: usize,
    /// The deepest `nesting` has gone.
    pub deepest: usize,
    pub max_nesting: usize,
    pub max_frames: usize,
    /// Built-in functions running (the collector runs only when none is: their values aren't roots).
    pub native_depth: usize,
    /// Objects made since the last collection that start the next one.
    pub gc_threshold: usize,
    /// The embedder's (the browser's) state, for its own built-in functions.
    pub host: Option<Box<dyn core::any::Any>>,
    /// The values the embedder's state holds (its objects, listeners, timers' functions): kept alive by the
    /// collector.
    pub host_trace: Option<fn(&dyn core::any::Any, &mut Vec<Value>)>,
    /// The values kept alive for the embedder (its event handlers, timers' functions).
    pub roots: Vec<Value>,
    /// Promises rejected with no handler yet (the embedder reports them).
    pub unhandled: Vec<Gc>,
    pub random_state: (u64, u64),
    /// Compiled regular expressions, by their source and flags.
    pub regexps: BTreeMap<(JsStr, String), Rc<crate::regexp::Program>>,
    /// Steps run (a budget a host can set, to stop a script that runs too long).
    pub steps: u64,
    pub step_limit: u64,
    /// Where the clock reads (the embedder's: milliseconds since 1970), and the time zone's offset in minutes.
    pub now: fn() -> f64,
    pub tz_offset: i32,
    /// The time zone's name (IANA's: "Asia/Singapore"), if the embedder knows it: what Intl says the zone is, and
    /// its daylight saving rule if it has one.
    pub tz_name: Option<alloc::string::String>,
    /// Whether an element (an index's property) was ever put on Array.prototype or Object.prototype: arrays' fast
    /// paths then go the long way, through the prototypes' setters.
    pub proto_indexes: bool,
    /// The modules loaded, and those being loaded (module.rs).
    pub modules: crate::module::Modules,
}

fn no_clock() -> f64 {
    0.0
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Vm {
        let mut vm = Vm {
            heap: Heap::default(),
            atoms: Atoms::new(),
            stack: Vec::with_capacity(1024),
            frames: Vec::new(),
            global: Gc(0),
            global_lex: BTreeMap::new(),
            proto_indexes: false,
            intrinsics: vec![Gc(0); I::Count as usize],
            symbols: Vec::new(),
            registry: BTreeMap::new(),
            jobs: VecDeque::new(),
            nesting: 0,
            max_nesting: 300,
            deepest: 0,
            max_frames: 10_000,
            native_depth: 0,
            gc_threshold: 100_000,
            host: None,
            host_trace: None,
            roots: Vec::new(),
            unhandled: Vec::new(),
            random_state: (0x9E37_79B9_7F4A_7C15, 0xD1B5_4A32_D192_ED03),
            regexps: BTreeMap::new(),
            steps: 0,
            step_limit: u64::MAX,
            now: no_clock,
            tz_offset: 0,
            tz_name: None,
            modules: crate::module::Modules::default(),
        };
        crate::builtins::init(&mut vm);
        vm
    }

    // ----------------------------------------------------------------------------------------------- the basics

    pub fn proto(&self, i: I) -> Gc {
        self.intrinsics[i as usize]
    }

    pub fn sym(&self, s: Sym) -> Gc {
        self.symbols[s as usize]
    }

    pub fn atom(&mut self, s: &str) -> Atom {
        self.atoms.intern_str(s)
    }

    pub fn alloc(&mut self, proto: Option<Gc>, class: Class) -> Gc {
        self.heap.alloc(Object::new(proto, class))
    }

    pub fn new_object(&mut self) -> Gc {
        let p = self.proto(I::ObjectProto);
        self.alloc(Some(p), Class::Ordinary)
    }

    pub fn new_array(&mut self, values: Vec<Value>) -> Gc {
        let len = values.len() as u32;
        let p = self.proto(I::ArrayProto);
        self.alloc(Some(p), Class::Array(values, len))
    }

    pub fn new_symbol(&mut self, desc: Option<JsStr>, flags: u8) -> Gc {
        self.alloc(None, Class::Symbol(desc, flags))
    }

    /// An error object (`kind`'s prototype), with its message and the stack's functions.
    pub fn make_error(&mut self, kind: ErrorKind, message: &str) -> Value {
        let p = self.proto(kind.proto());
        let g = self.alloc(Some(p), Class::Error);
        let m = Value::str(message);
        self.heap.get_mut(g).props.set(Key::Str(n::MESSAGE), Prop::data(m, HIDDEN));
        let stack = self.stack_trace(kind, message);
        self.heap.get_mut(g).props.set(Key::Str(n::STACK), Prop::data(Value::Str(stack), HIDDEN));
        Value::Obj(g)
    }

    /// "TypeError: message" and the functions running, innermost first.
    pub fn stack_trace(&self, kind: ErrorKind, message: &str) -> JsStr {
        let name = match kind {
            ErrorKind::Error => "Error",
            ErrorKind::Eval => "EvalError",
            ErrorKind::Range => "RangeError",
            ErrorKind::Reference => "ReferenceError",
            ErrorKind::Syntax => "SyntaxError",
            ErrorKind::Type => "TypeError",
            ErrorKind::Uri => "URIError",
            ErrorKind::Aggregate => "AggregateError",
        };
        let mut s = if message.is_empty() { String::from(name) } else { format!("{name}: {message}") };
        for f in self.frames.iter().rev().take(10) {
            let fname = self.atoms.get(f.code.name).to_rust();
            s.push_str("\n    at ");
            s.push_str(if fname.is_empty() { "<anonymous>" } else { &fname });
            // (Its line, as it runs, and where the function starts in its script: a minified script is one line.)
            if f.code.source.is_some() {
                let line = f.code.lines.iter().take_while(|(pc, _)| *pc as usize <= f.pc).last().map_or(1, |(_, l)| *l + 1);
                s.push_str(&format!(" (line {line}, from {})", f.code.start));
            }
        }
        JsStr::from_str(&s)
    }

    pub fn type_error(&mut self, m: &str) -> Value {
        self.make_error(ErrorKind::Type, m)
    }

    pub fn range_error(&mut self, m: &str) -> Value {
        self.make_error(ErrorKind::Range, m)
    }

    pub fn reference_error(&mut self, m: &str) -> Value {
        self.make_error(ErrorKind::Reference, m)
    }

    pub fn syntax_error(&mut self, m: &str) -> Value {
        self.make_error(ErrorKind::Syntax, m)
    }

    /// A built-in function.
    pub fn native(&mut self, name: &str, length: u32, f: NativeFn) -> Gc {
        self.native_with(name, length, f, None, Value::Undefined)
    }

    pub fn native_with(&mut self, name: &str, length: u32, f: NativeFn, construct: Option<NativeFn>, data: Value) -> Gc {
        let p = self.proto(I::FunctionProto);
        let g = self.alloc(Some(p), Class::Native(Box::new(Native { f, construct, data, data2: Value::Undefined })));
        let name = Value::str(name);
        self.fn_props(g, length, name);
        g
    }

    /// A function's own length and name.
    pub fn fn_props(&mut self, g: Gc, length: u32, name: Value) {
        let o = self.heap.get_mut(g);
        o.props.set(Key::Str(n::LENGTH), Prop::data(Value::Num(length as f64), CONFIGURABLE));
        o.props.set(Key::Str(n::NAME), Prop::data(name, CONFIGURABLE));
    }

    /// A built-in method on an object (not enumerable).
    pub fn method(&mut self, obj: Gc, name: &str, length: u32, f: NativeFn) -> Gc {
        let g = self.native(name, length, f);
        let a = self.atom(name);
        self.heap.get_mut(obj).props.set(Key::Str(a), Prop::data(Value::Obj(g), HIDDEN));
        g
    }

    /// A data property defined directly (for building the built-ins).
    pub fn put(&mut self, obj: Gc, name: &str, v: Value, flags: u8) {
        let a = self.atom(name);
        self.heap.get_mut(obj).props.set(Key::Str(a), Prop::data(v, flags));
    }

    pub fn put_key(&mut self, obj: Gc, key: Key, v: Value, flags: u8) {
        self.heap.get_mut(obj).props.set(key, Prop::data(v, flags));
    }

    /// A built-in getter (and setter) on an object.
    pub fn accessor(&mut self, obj: Gc, key: Key, name: &str, get: Option<NativeFn>, set: Option<NativeFn>) {
        let g = match get {
            Some(f) => Value::Obj(self.native(&format!("get {name}"), 0, f)),
            None => Value::Undefined,
        };
        let s = match set {
            Some(f) => Value::Obj(self.native(&format!("set {name}"), 1, f)),
            None => Value::Undefined,
        };
        self.heap.get_mut(obj).props.set(key, Prop { slot: Slot::Accessor(g, s), flags: CONFIGURABLE });
    }

    pub fn str_of(&self, a: Atom) -> JsStr {
        self.atoms.get(a).clone()
    }

    /// A native function's data.
    pub fn data(&self, f: Gc) -> Value {
        match &self.heap.get(f).class {
            Class::Native(n) => n.data.clone(),
            _ => Value::Undefined,
        }
    }

    pub fn data2(&self, f: Gc) -> Value {
        match &self.heap.get(f).class {
            Class::Native(n) => n.data2.clone(),
            _ => Value::Undefined,
        }
    }

    pub fn set_data2(&mut self, f: Gc, v: Value) {
        if let Class::Native(n) = &mut self.heap.get_mut(f).class {
            n.data2 = v;
        }
    }

    pub fn random(&mut self) -> f64 {
        // xorshift128+
        let (mut s1, s0) = self.random_state;
        self.random_state.0 = s0;
        s1 ^= s1 << 23;
        s1 ^= s1 >> 17;
        s1 ^= s0;
        s1 ^= s0 >> 26;
        self.random_state.1 = s1;
        let r = s1.wrapping_add(s0);
        (r >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn seed(&mut self, seed: u64) {
        self.random_state = (seed | 1, seed.rotate_left(29) ^ 0xD1B5_4A32_D192_ED03);
    }

    // ------------------------------------------------------------------------------------------------ functions

    /// A function object made from code, in a scope.
    pub fn make_closure(&mut self, code: Rc<Code>, env: Option<Gc>, this: Value, new_target: Value, home: Option<Gc>) -> Gc {
        let kind = code.kind;
        let proto = match (code.is_async, code.is_generator) {
            (true, true) => I::AsyncGeneratorFunctionProto,
            (true, false) => I::AsyncFunctionProto,
            (false, true) => I::GeneratorFunctionProto,
            (false, false) => I::FunctionProto,
        };
        let p = self.proto(proto);
        let length = code.length;
        let name = Value::Str(self.str_of(code.name));
        let generator = code.is_generator;
        let is_async = code.is_async;
        let g = self.alloc(Some(p), Class::Function(Box::new(Closure { code, env, this, new_target, home, fields: None, privates: None })));
        self.fn_props(g, length, name);
        if generator {
            let gp = self.proto(if is_async { I::AsyncGeneratorProto } else { I::GeneratorProto });
            let po = self.alloc(Some(gp), Class::Ordinary);
            self.put_key(g, Key::Str(n::PROTOTYPE), Value::Obj(po), WRITABLE);
        } else if kind == Kind::Normal && !is_async {
            let po = self.new_object();
            self.put_key(po, Key::Str(n::CONSTRUCTOR), Value::Obj(g), HIDDEN);
            self.put_key(g, Key::Str(n::PROTOTYPE), Value::Obj(po), WRITABLE);
        }
        g
    }

    pub fn is_callable(&self, v: &Value) -> bool {
        match v {
            Value::Obj(g) => match &self.heap.get(*g).class {
                Class::Function(_) | Class::Native(_) | Class::Bound(_) => true,
                Class::Proxy(_, f) => f & 1 != 0,
                _ => false,
            },
            _ => false,
        }
    }

    pub fn is_constructor(&self, v: &Value) -> bool {
        match v {
            Value::Obj(g) => match &self.heap.get(*g).class {
                Class::Function(c) => matches!(c.code.kind, Kind::Normal | Kind::BaseConstructor | Kind::DerivedConstructor) && !c.code.is_async && !c.code.is_generator,
                Class::Native(n) => n.construct.is_some(),
                Class::Bound(b) => self.is_constructor(&Value::Obj(b.target)),
                Class::Proxy(_, f) => f & 2 != 0,
                _ => false,
            },
            _ => false,
        }
    }

    fn sloppy_this(&mut self, t: Value) -> Result<Value, Value> {
        Ok(match t {
            Value::Undefined | Value::Null | Value::Empty => Value::Obj(self.global),
            Value::Obj(_) => t,
            other => Value::Obj(self.to_object(&other)?),
        })
    }

    /// A JavaScript function's frame pushed: the stack holds f, this and `argc` arguments.
    pub(crate) fn enter_closure(&mut self, g: Gc, argc: usize, new_target: Value, construct: bool) -> Result<(), Value> {
        if self.frames.len() >= self.max_frames {
            return Err(self.range_error("Maximum call stack size exceeded"));
        }
        let Class::Function(c) = &self.heap.get(g).class else { return Err(Value::Undefined) };
        let code = c.code.clone();
        let env = c.env;
        let home = c.home;
        let (cthis, cnt) = (c.this.clone(), c.new_target.clone());
        let base = self.stack.len() - argc;
        let bottom = base - 2;
        let this = if code.kind == Kind::Arrow {
            cthis
        } else {
            let t = self.stack[bottom + 1].clone();
            if code.strict || construct { t } else { self.sloppy_this(t)? }
        };
        let new_target = if code.kind == Kind::Arrow { cnt } else { new_target };
        let args = if code.keeps_args { self.stack[base..].to_vec() } else { Vec::new() };
        let params = code.params as usize;
        if argc > params {
            self.stack.truncate(base + params);
        }
        self.stack.resize(base + code.registers as usize, Value::Undefined);
        self.frames.push(Frame { code, pc: 0, base, bottom, env, this, callee: Some(g), new_target, home, args, handlers: Vec::new(), construct, owner: None });
        Ok(())
    }

    /// Calls a function (from Rust: a built-in, the embedder).
    pub fn call(&mut self, f: &Value, this: Value, args: &[Value]) -> Result<Value, Value> {
        let Value::Obj(g) = f else {
            return Err(self.type_error(&format!("{} is not a function", self.describe(f))));
        };
        let g = *g;
        match &self.heap.get(g).class {
            Class::Function(c) => {
                let code = &c.code;
                if matches!(code.kind, Kind::BaseConstructor | Kind::DerivedConstructor) {
                    let name = self.str_of(code.name).to_rust();
                    return Err(self.type_error(&format!("Class constructor {name} cannot be invoked without 'new'")));
                }
                if code.is_generator || code.is_async {
                    return self.call_special(g, this, args);
                }
                self.stack.push(f.clone());
                self.stack.push(this);
                self.stack.extend_from_slice(args);
                self.enter_closure(g, args.len(), Value::Undefined, false)?;
                self.run_frame()
            }
            Class::Native(nat) => {
                let f = nat.f;
                self.call_native(f, this, args, g)
            }
            Class::Bound(b) => {
                let target = Value::Obj(b.target);
                let bthis = b.this.clone();
                let mut all = b.args.clone();
                all.extend_from_slice(args);
                self.call(&target, bthis, &all)
            }
            Class::Proxy(..) => self.proxy_call(g, this, args),
            _ => Err(self.type_error(&format!("{} is not a function", self.describe(f)))),
        }
    }

    fn call_native(&mut self, f: NativeFn, this: Value, args: &[Value], g: Gc) -> Result<Value, Value> {
        if self.nesting >= self.max_nesting {
            return Err(self.range_error("Maximum call stack size exceeded"));
        }
        self.native_depth += 1;
        self.nesting += 1;
        self.deepest = self.deepest.max(self.nesting);
        let r = f(self, &this, args, g);
        self.nesting -= 1;
        self.native_depth -= 1;
        r
    }

    /// The frame just pushed run to its end (from Rust).
    fn run_frame(&mut self) -> Result<Value, Value> {
        if self.nesting >= self.max_nesting {
            let f = self.frames.pop();
            if let Some(f) = f {
                self.stack.truncate(f.bottom);
            }
            return Err(self.range_error("Maximum call stack size exceeded"));
        }
        let depth = self.frames.len() - 1;
        self.nesting += 1;
        self.deepest = self.deepest.max(self.nesting);
        let r = self.execute(depth, Resume::Normal);
        self.nesting -= 1;
        match r? {
            Signal::Return(v) => Ok(v),
            _ => Ok(Value::Undefined),
        }
    }

    /// `new f(...args)` (with new.target `nt`).
    pub fn construct(&mut self, f: &Value, args: &[Value], nt: &Value) -> Result<Value, Value> {
        let Value::Obj(g) = f else {
            return Err(self.type_error(&format!("{} is not a constructor", self.describe(f))));
        };
        let g = *g;
        if !self.is_constructor(f) {
            return Err(self.type_error(&format!("{} is not a constructor", self.describe(f))));
        }
        match &self.heap.get(g).class {
            Class::Function(c) => {
                let kind = c.code.kind;
                let fields = c.fields;
                let this = if kind == Kind::DerivedConstructor { Value::Empty } else { Value::Obj(self.create_from_constructor(nt, I::ObjectProto)?) };
                if kind == Kind::BaseConstructor
                    && let Some(fl) = fields
                {
                    self.call(&Value::Obj(fl), this.clone(), &[])?;
                }
                self.stack.push(f.clone());
                self.stack.push(this);
                self.stack.extend_from_slice(args);
                self.enter_closure(g, args.len(), nt.clone(), true)?;
                self.run_frame()
            }
            Class::Native(nat) => {
                let Some(c) = nat.construct else { return Err(self.type_error("not a constructor")) };
                self.call_native(c, nt.clone(), args, g)
            }
            Class::Bound(b) => {
                let target = Value::Obj(b.target);
                let mut all = b.args.clone();
                all.extend_from_slice(args);
                let nt = if let Value::Obj(x) = nt
                    && *x == g
                {
                    target.clone()
                } else {
                    nt.clone()
                };
                self.construct(&target, &all, &nt)
            }
            Class::Proxy(..) => self.proxy_construct(g, args, nt),
            _ => Err(self.type_error("not a constructor")),
        }
    }

    /// OrdinaryCreateFromConstructor: an object whose prototype is new.target's "prototype" (or `default`).
    pub fn create_from_constructor(&mut self, nt: &Value, default: I) -> Result<Gc, Value> {
        let p = self.proto_from_constructor(nt, default)?;
        Ok(self.alloc(Some(p), Class::Ordinary))
    }

    pub fn proto_from_constructor(&mut self, nt: &Value, default: I) -> Result<Gc, Value> {
        if let Value::Obj(c) = nt {
            let p = self.get(*c, Key::Str(n::PROTOTYPE), nt)?;
            if let Value::Obj(p) = p {
                return Ok(p);
            }
        }
        Ok(self.proto(default))
    }

    /// A short description of a value for error messages.
    pub fn describe(&self, v: &Value) -> String {
        match v {
            Value::Undefined | Value::Empty => String::from("undefined"),
            Value::Null => String::from("null"),
            Value::Bool(b) => format!("{b}"),
            Value::Num(x) => crate::value::number_to_string(*x),
            Value::Str(s) => format!("\"{}\"", s.to_rust()),
            Value::Sym(_) => String::from("Symbol()"),
            Value::Big(b) => format!("{}n", b.to_string_radix(10)),
            Value::Obj(g) => match &self.heap.get(*g).class {
                Class::Function(c) => {
                    let name = self.str_of(c.code.name).to_rust();
                    if name.is_empty() { String::from("function") } else { name }
                }
                Class::Array(..) => String::from("array"),
                _ => String::from("object"),
            },
        }
    }

    // --------------------------------------------------------------------------------------------- running code

    /// Compiles and runs a script (its completion value, or the exception it threw).
    pub fn run_script(&mut self, source: &str, name: &str) -> Result<Value, Value> {
        let script = match crate::parser::parse_script(source, &mut self.atoms) {
            Ok(s) => s,
            Err(e) => return Err(self.syntax_error(&e.message)),
        };
        let src: Rc<str> = Rc::from(source);
        let code = match crate::compiler::compile_script(&script, &mut self.atoms, src, false) {
            Ok(c) => c,
            Err(e) => return Err(self.syntax_error(&e.message)),
        };
        let _ = name;
        self.run_code(code, None, Value::Obj(self.global))
    }

    /// Runs compiled global (or eval) code.
    pub fn run_code(&mut self, code: Rc<Code>, env: Option<Gc>, this: Value) -> Result<Value, Value> {
        let base = self.stack.len();
        self.stack.resize(base + code.registers as usize, Value::Undefined);
        self.frames.push(Frame { code, pc: 0, base, bottom: base, env, this, callee: None, new_target: Value::Undefined, home: None, args: Vec::new(), handlers: Vec::new(), construct: false, owner: None });
        self.run_frame()
    }

    fn pop(&mut self) -> Value {
        self.stack.pop().unwrap_or(Value::Undefined)
    }

    fn top(&self) -> &Value {
        self.stack.last().unwrap_or(&Value::Undefined)
    }

    /// Runs frames from the one at `depth` (the top one) until it returns, yields or awaits.
    pub fn execute(&mut self, depth: usize, resume: Resume) -> Result<Signal, Value> {
        let mut pending: Option<Value> = None;
        match resume {
            Resume::Normal => {}
            Resume::Throw(e) => pending = Some(e),
            Resume::Return(v) => match self.do_return(v, depth) {
                Ok(Some(s)) => return Ok(s),
                Ok(None) => {}
                Err(e) => pending = Some(e),
            },
        }
        loop {
            if let Some(e) = pending.take() {
                self.unwind(e, depth)?;
            }
            match self.run(depth) {
                Ok(s) => return Ok(s),
                Err(e) => pending = Some(e),
            }
        }
    }

    /// An exception: to the innermost handler of a frame at `depth` or above (frames without one are left).
    fn unwind(&mut self, e: Value, depth: usize) -> Result<(), Value> {
        loop {
            if self.frames.len() <= depth {
                return Err(e);
            }
            let fi = self.frames.len() - 1;
            if let Some(h) = self.frames[fi].handlers.pop() {
                self.stack.truncate(h.sp);
                self.stack.push(e);
                if h.finally {
                    self.stack.push(Value::Num(1.0));
                }
                let f = &mut self.frames[fi];
                f.pc = h.pc as usize;
                f.env = h.env;
                return Ok(());
            }
            let f = self.frames.pop().expect("a frame");
            self.stack.truncate(f.bottom);
            if fi == depth {
                return Err(e);
            }
        }
    }

    /// A return from the top frame: through its finally handlers first; Some when the frame at `depth` returned.
    pub(crate) fn do_return(&mut self, v: Value, depth: usize) -> Result<Option<Signal>, Value> {
        let fi = self.frames.len() - 1;
        while let Some(h) = self.frames[fi].handlers.pop() {
            if h.finally {
                self.stack.truncate(h.sp);
                self.stack.push(v);
                self.stack.push(Value::Num(2.0));
                let f = &mut self.frames[fi];
                f.pc = h.pc as usize;
                f.env = h.env;
                return Ok(None);
            }
        }
        let f = self.frames.pop().expect("a frame");
        self.stack.truncate(f.bottom);
        let v = if f.construct {
            match v {
                Value::Obj(_) => v,
                Value::Undefined if matches!(f.this, Value::Empty) => return Err(self.reference_error("Must call super constructor in derived class before accessing 'this' or returning from derived constructor")),
                Value::Undefined => f.this,
                _ if f.code.kind == Kind::DerivedConstructor => return Err(self.type_error("Derived constructors may only return object or undefined")),
                _ => f.this,
            }
        } else {
            v
        };
        if fi == depth {
            return Ok(Some(Signal::Return(v)));
        }
        self.stack.push(v);
        Ok(None)
    }

    /// Sets the top frame (a generator's, or an async function's) aside: it and its part of the stack.
    pub fn suspend(&mut self) -> (Box<Frame>, Vec<Value>) {
        let mut f = self.frames.pop().expect("a frame");
        let base = f.base;
        let stack = self.stack.split_off(base);
        self.stack.truncate(f.bottom);
        for h in f.handlers.iter_mut() {
            h.sp -= base;
        }
        f.base = 0;
        f.bottom = 0;
        (Box::new(f), stack)
    }

    /// Takes a frame set aside up again, on top.
    pub fn restore(&mut self, mut f: Box<Frame>, stack: Vec<Value>) {
        let base = self.stack.len();
        f.base = base;
        f.bottom = base;
        for h in f.handlers.iter_mut() {
            h.sp += base;
        }
        self.stack.extend(stack);
        self.frames.push(*f);
    }

    fn env_at(&self, hops: u16) -> Gc {
        let mut e = self.frames.last().and_then(|f| f.env).expect("a scope");
        for _ in 0..hops {
            let Class::Env(env) = &self.heap.get(e).class else { break };
            e = env.parent.expect("a scope's parent");
        }
        e
    }

    fn env_get(&self, hops: u16, slot: u16) -> Value {
        let e = self.env_at(hops);
        match &self.heap.get(e).class {
            Class::Env(env) => env.vars[slot as usize].clone(),
            _ => Value::Undefined,
        }
    }

    fn env_set(&mut self, hops: u16, slot: u16, v: Value) {
        let e = self.env_at(hops);
        if let Class::Env(env) = &mut self.heap.get_mut(e).class {
            env.vars[slot as usize] = v;
        }
    }

    fn tdz(&mut self, s: u32) -> Value {
        let name = self.frames.last().map(|f| f.code.strings[s as usize].to_rust()).unwrap_or_default();
        self.reference_error(&format!("Cannot access '{name}' before initialization"))
    }

    fn code_str(&self, s: u32) -> JsStr {
        self.frames.last().map(|f| f.code.strings[s as usize].clone()).unwrap_or_else(JsStr::empty)
    }

    /// Runs ops until the frame at `depth` returns, yields or awaits, or something throws.
    fn run(&mut self, depth: usize) -> Result<Signal, Value> {
        loop {
            let fi = self.frames.len() - 1;
            let (op, base) = {
                let f = &mut self.frames[fi];
                let op = f.code.ops[f.pc];
                f.pc += 1;
                (op, f.base)
            };
            match op {
                Op::Nop => {}
                Op::Undefined => self.stack.push(Value::Undefined),
                Op::Null => self.stack.push(Value::Null),
                Op::True => self.stack.push(Value::Bool(true)),
                Op::False => self.stack.push(Value::Bool(false)),
                Op::Empty => self.stack.push(Value::Empty),
                Op::Int(i) => self.stack.push(Value::Num(i as f64)),
                Op::Num(x) => self.stack.push(Value::Num(x)),
                Op::Str(s) => {
                    let v = Value::Str(self.frames[fi].code.strings[s as usize].clone());
                    self.stack.push(v);
                }
                Op::BigInt(i) => {
                    let v = Value::Big(self.frames[fi].code.bigints[i as usize].clone());
                    self.stack.push(v);
                }
                Op::Pop => {
                    self.stack.pop();
                }
                Op::Dup => {
                    let v = self.top().clone();
                    self.stack.push(v);
                }
                Op::Dup2 => {
                    let l = self.stack.len();
                    let (a, b) = (self.stack[l - 2].clone(), self.stack[l - 1].clone());
                    self.stack.push(a);
                    self.stack.push(b);
                }
                Op::Swap => {
                    let l = self.stack.len();
                    self.stack.swap(l - 1, l - 2);
                }
                Op::Rot3 => {
                    let v = self.pop();
                    let l = self.stack.len();
                    self.stack.insert(l - 2, v);
                }
                Op::Rot4 => {
                    let v = self.pop();
                    let l = self.stack.len();
                    self.stack.insert(l - 3, v);
                }
                Op::Over => {
                    let v = self.stack[self.stack.len() - 2].clone();
                    self.stack.push(v);
                }
                Op::GetLocal(r) => {
                    let v = self.stack[base + r as usize].clone();
                    self.stack.push(v);
                }
                Op::SetLocal(r) => {
                    let v = self.top().clone();
                    self.stack[base + r as usize] = v;
                }
                Op::InitLocal(r) => {
                    let v = self.pop();
                    self.stack[base + r as usize] = v;
                }
                Op::GetLocalChecked(r, s) => {
                    let v = self.stack[base + r as usize].clone();
                    if matches!(v, Value::Empty) {
                        return Err(self.tdz(s));
                    }
                    self.stack.push(v);
                }
                Op::SetLocalChecked(r, s) => {
                    if matches!(self.stack[base + r as usize], Value::Empty) {
                        return Err(self.tdz(s));
                    }
                    let v = self.top().clone();
                    self.stack[base + r as usize] = v;
                }
                Op::GetEnv(h, s) => {
                    let v = self.env_get(h, s);
                    self.stack.push(v);
                }
                Op::SetEnv(h, s) => {
                    let v = self.top().clone();
                    self.env_set(h, s, v);
                }
                Op::InitEnv(h, s) => {
                    let v = self.pop();
                    self.env_set(h, s, v);
                }
                Op::GetEnvChecked(h, s, name) => {
                    let v = self.env_get(h, s);
                    if matches!(v, Value::Empty) {
                        return Err(self.tdz(name));
                    }
                    self.stack.push(v);
                }
                Op::SetEnvChecked(h, s, name) => {
                    if matches!(self.env_get(h, s), Value::Empty) {
                        return Err(self.tdz(name));
                    }
                    let v = self.top().clone();
                    self.env_set(h, s, v);
                }
                Op::GetImport(h, s, name) => {
                    let cell = self.env_get(h, s);
                    let v = self.import_value(&cell);
                    if matches!(v, Value::Empty) {
                        return Err(self.tdz(name));
                    }
                    self.stack.push(v);
                }
                Op::PushEnv(t) => {
                    let (names, var) = self.frames[fi].code.scopes[t as usize].clone();
                    let vars = names.iter().map(|(_, k)| if *k == LET || *k == CONST { Value::Empty } else { Value::Undefined }).collect();
                    let parent = self.frames[fi].env;
                    let e = self.alloc(None, Class::Env(Box::new(Env { vars, parent, names, with: None, var_scope: var })));
                    self.frames[fi].env = Some(e);
                }
                Op::PopEnv => {
                    let e = self.frames[fi].env.expect("a scope");
                    let parent = match &self.heap.get(e).class {
                        Class::Env(env) => env.parent,
                        _ => None,
                    };
                    self.frames[fi].env = parent;
                }
                Op::CopyEnv => {
                    let e = self.frames[fi].env.expect("a scope");
                    let copy = match &self.heap.get(e).class {
                        Class::Env(env) => Env { vars: env.vars.clone(), parent: env.parent, names: env.names.clone(), with: None, var_scope: env.var_scope },
                        _ => continue,
                    };
                    let c = self.alloc(None, Class::Env(Box::new(copy)));
                    self.frames[fi].env = Some(c);
                }
                Op::ConstAssign(s) => {
                    let _ = s;
                    return Err(self.type_error("Assignment to constant variable."));
                }
                Op::GetGlobal(a) => {
                    let v = self.get_global(a)?;
                    self.stack.push(v);
                }
                Op::SetGlobal(a) => {
                    let v = self.top().clone();
                    let strict = self.frames[fi].code.strict;
                    self.set_global(a, v, strict)?;
                }
                Op::InitGlobal(a) => {
                    let v = self.pop();
                    if let Some(e) = self.global_lex.get_mut(&a) {
                        e.0 = v;
                    } else {
                        let k = Key::Str(a);
                        self.set(self.global, k, v, &Value::Obj(self.global))?;
                    }
                }
                Op::TypeofGlobal(a) => {
                    let v = match self.global_lex.get(&a) {
                        Some((Value::Empty, _)) => return Err(self.reference_error(&format!("Cannot access '{}' before initialization", self.str_of(a)))),
                        Some((v, _)) => v.clone(),
                        None => {
                            let k = Key::Str(a);
                            let g = self.global;
                            if self.has_property(g, k)? { self.get(g, k, &Value::Obj(g))? } else { Value::Undefined }
                        }
                    };
                    let t = self.type_of(&v);
                    self.stack.push(Value::str(t));
                }
                Op::GetName(a) => {
                    let v = self.get_name(a)?;
                    self.stack.push(v);
                }
                Op::SetName(a) => {
                    let v = self.top().clone();
                    let strict = self.frames[fi].code.strict;
                    self.set_name(a, v, strict)?;
                }
                Op::TypeofName(a) => {
                    let v = match self.lookup_name(a)? {
                        Some(r) => self.read_ref(r, a)?,
                        None => Value::Undefined,
                    };
                    let t = self.type_of(&v);
                    self.stack.push(Value::str(t));
                }
                Op::DeleteName(a) => {
                    let r = self.delete_name(a)?;
                    self.stack.push(Value::Bool(r));
                }
                Op::GetNameForCall(a) => {
                    let r = self.lookup_name(a)?;
                    let Some(r) = r else { return Err(self.reference_error(&format!("{} is not defined", self.str_of(a)))) };
                    let this = match &r {
                        NameRef::Object(o, true) => Value::Obj(*o),
                        _ => Value::Undefined,
                    };
                    let f = self.read_ref(r, a)?;
                    self.stack.push(f);
                    self.stack.push(this);
                }
                Op::PushWith => {
                    let o = self.pop();
                    let parent = self.frames[fi].env;
                    let e = self.alloc(None, Class::Env(Box::new(Env { vars: Vec::new(), parent, names: Rc::from([]), with: o.as_obj(), var_scope: false })));
                    self.frames[fi].env = Some(e);
                }
                Op::This => {
                    let v = self.frames[fi].this.clone();
                    self.stack.push(v);
                }
                Op::ThisChecked => {
                    let v = self.frames[fi].this.clone();
                    if matches!(v, Value::Empty) {
                        return Err(self.reference_error("Must call super constructor in derived class before accessing 'this' or returning from derived constructor"));
                    }
                    self.stack.push(v);
                }
                Op::NewTarget => {
                    let v = self.frames[fi].new_target.clone();
                    self.stack.push(v);
                }
                Op::Callee => {
                    let v = self.frames[fi].callee.map_or(Value::Undefined, Value::Obj);
                    self.stack.push(v);
                }
                Op::Arguments => {
                    let a = self.arguments_object(fi, None);
                    self.stack.push(Value::Obj(a));
                }
                Op::MappedArguments(m) => {
                    let slots = self.frames[fi].code.arg_maps[m as usize].clone();
                    let map = self.frames[fi].env.map(|env| ArgMap { env, slots });
                    let a = self.arguments_object(fi, map);
                    self.stack.push(Value::Obj(a));
                }
                Op::Rest(from) => {
                    let rest: Vec<Value> = self.frames[fi].args.iter().skip(from as usize).cloned().collect();
                    let a = self.new_array(rest);
                    self.stack.push(Value::Obj(a));
                }
                Op::ArgCount => {
                    let n = self.frames[fi].args.len();
                    self.stack.push(Value::Num(n as f64));
                }
                Op::GetProp(a) => {
                    let o = self.pop();
                    let k = self.atom_key(a);
                    let v = self.get_value(&o, k)?;
                    self.stack.push(v);
                }
                Op::SetProp(a) => {
                    let v = self.pop();
                    let o = self.pop();
                    let strict = self.frames[fi].code.strict;
                    let k = self.atom_key(a);
                    self.put_value(&o, k, v.clone(), strict)?;
                    self.stack.push(v);
                }
                Op::GetElem => {
                    let k = self.pop();
                    let o = self.pop();
                    let v = self.get_elem(&o, &k)?;
                    self.stack.push(v);
                }
                Op::SetElem => {
                    let v = self.pop();
                    let k = self.pop();
                    let o = self.pop();
                    if o.is_nullish() {
                        return Err(self.type_error(&format!("Cannot set properties of {}", self.describe(&o))));
                    }
                    let key = self.to_key(&k)?;
                    let strict = self.frames[fi].code.strict;
                    self.put_value(&o, key, v.clone(), strict)?;
                    self.stack.push(v);
                }
                Op::GetMethod(a) => {
                    let o = self.pop();
                    let k = self.atom_key(a);
                    let f = self.get_value(&o, k)?;
                    self.stack.push(f);
                    self.stack.push(o);
                }
                Op::GetElemMethod => {
                    let k = self.pop();
                    let o = self.pop();
                    let f = self.get_elem(&o, &k)?;
                    self.stack.push(f);
                    self.stack.push(o);
                }
                Op::DeleteProp(a) => {
                    let o = self.pop();
                    let strict = self.frames[fi].code.strict;
                    let k = self.atom_key(a);
                    let r = self.delete_value(&o, k, strict)?;
                    self.stack.push(Value::Bool(r));
                }
                Op::DeleteElem => {
                    let k = self.pop();
                    let o = self.pop();
                    if o.is_nullish() {
                        return Err(self.type_error(&format!("Cannot convert {} to object", self.describe(&o))));
                    }
                    let key = self.to_key(&k)?;
                    let strict = self.frames[fi].code.strict;
                    let r = self.delete_value(&o, key, strict)?;
                    self.stack.push(Value::Bool(r));
                }
                Op::In => {
                    let o = self.pop();
                    let k = self.pop();
                    let Value::Obj(g) = o else {
                        return Err(self.type_error(&format!("Cannot use 'in' operator to search for a key in {}", self.describe(&o))));
                    };
                    let key = self.to_key(&k)?;
                    let r = self.has_property(g, key)?;
                    self.stack.push(Value::Bool(r));
                }
                Op::GetSuper(a) => {
                    let k = self.atom_key(a);
                    let v = self.get_super(fi, k)?;
                    self.stack.push(v);
                }
                Op::GetSuperElem => {
                    let k = self.pop();
                    let key = self.to_key(&k)?;
                    let v = self.get_super(fi, key)?;
                    self.stack.push(v);
                }
                Op::SetSuper(a) => {
                    let v = self.pop();
                    let k = self.atom_key(a);
                    self.set_super(fi, k, v.clone())?;
                    self.stack.push(v);
                }
                Op::SetSuperElem => {
                    let v = self.pop();
                    let k = self.pop();
                    let key = self.to_key(&k)?;
                    self.set_super(fi, key, v.clone())?;
                    self.stack.push(v);
                }
                Op::GetSuperMethod(a) => {
                    let k = self.atom_key(a);
                    let f = self.get_super(fi, k)?;
                    let this = self.frames[fi].this.clone();
                    self.stack.push(f);
                    self.stack.push(this);
                }
                Op::GetSuperElemMethod => {
                    let k = self.pop();
                    let key = self.to_key(&k)?;
                    let f = self.get_super(fi, key)?;
                    let this = self.frames[fi].this.clone();
                    self.stack.push(f);
                    self.stack.push(this);
                }
                Op::GetPrivate => {
                    let p = self.pop();
                    let o = self.pop();
                    let v = self.private_get(&o, &p)?;
                    self.stack.push(v);
                }
                Op::SetPrivate => {
                    let v = self.pop();
                    let p = self.pop();
                    let o = self.pop();
                    self.private_set(&o, &p, v.clone())?;
                    self.stack.push(v);
                }
                Op::DefinePrivate => {
                    let v = self.pop();
                    let p = self.pop();
                    let o = self.top().clone();
                    self.private_define(&o, &p, Slot::Data(v), WRITABLE)?;
                }
                Op::DefinePrivateMethod(k) => {
                    let f = self.pop();
                    let p = self.pop();
                    let o = self.top().clone();
                    if let (Value::Obj(fg), Value::Obj(og)) = (&f, &o)
                        && let Class::Function(c) = &mut self.heap.get_mut(*fg).class
                        && c.home.is_none()
                    {
                        c.home = Some(*og);
                    }
                    if let Value::Sym(s) = p {
                        let desc = match &self.heap.get(s).class {
                            Class::Symbol(Some(d), _) => d.clone(),
                            _ => JsStr::empty(),
                        };
                        let name = match k {
                            1 => JsStr::from_str("get ").concat(&desc),
                            2 => JsStr::from_str("set ").concat(&desc),
                            _ => desc,
                        };
                        if let Value::Obj(fg) = f {
                            self.put_key(fg, Key::Str(n::NAME), Value::Str(name), CONFIGURABLE);
                        }
                    }
                    let slot = match k {
                        1 => Slot::Accessor(f, Value::Undefined),
                        2 => Slot::Accessor(Value::Undefined, f),
                        _ => Slot::Data(f),
                    };
                    self.private_define(&o, &p, slot, 0)?;
                }
                Op::HasPrivate => {
                    let o = self.pop();
                    let p = self.pop();
                    let Value::Obj(g) = o else { return Err(self.type_error("Cannot use 'in' operator to search for a private field in a non-object")) };
                    let Value::Sym(s) = p else { return Err(self.type_error("not a private name")) };
                    let r = self.heap.get(g).props.get(&Key::Sym(s)).is_some();
                    self.stack.push(Value::Bool(r));
                }
                Op::NewObject => {
                    let g = self.new_object();
                    self.stack.push(Value::Obj(g));
                }
                Op::NewArray => {
                    let g = self.new_array(Vec::new());
                    self.stack.push(Value::Obj(g));
                }
                Op::DefineProp(a) => {
                    let v = self.pop();
                    let Value::Obj(o) = *self.top() else { continue };
                    let key = self.atom_key(a);
                    self.create_data_property(o, key, v)?;
                }
                Op::DefineElem => {
                    let v = self.pop();
                    let k = self.pop();
                    let Value::Obj(o) = *self.top() else { continue };
                    let key = self.to_key(&k)?;
                    self.create_data_property(o, key, v)?;
                }
                Op::DefineMethod(flags) => {
                    let f = self.pop();
                    let k = self.pop();
                    let Value::Obj(o) = *self.top() else { continue };
                    let key = self.to_key(&k)?;
                    self.define_method(o, key, f, flags)?;
                }
                Op::SpreadObject => {
                    let src = self.pop();
                    let Value::Obj(o) = *self.top() else { continue };
                    self.copy_data_properties(o, &src, &[])?;
                }
                Op::SetProtoLiteral => {
                    let p = self.pop();
                    let Value::Obj(o) = *self.top() else { continue };
                    match p {
                        Value::Obj(p) => self.heap.get_mut(o).proto = Some(p),
                        Value::Null => self.heap.get_mut(o).proto = None,
                        _ => {}
                    }
                }
                Op::ArrayPush => {
                    let v = self.pop();
                    if let Value::Obj(a) = *self.top()
                        && let Class::Array(els, len) = &mut self.heap.get_mut(a).class
                    {
                        els.push(v);
                        *len += 1;
                    }
                }
                Op::ArrayHole => {
                    if let Value::Obj(a) = *self.top()
                        && let Class::Array(els, len) = &mut self.heap.get_mut(a).class
                    {
                        els.push(Value::Empty);
                        *len += 1;
                    }
                }
                Op::ArraySpread => {
                    let it = self.pop();
                    let values = self.iterate_to_vec(&it)?;
                    if let Value::Obj(a) = *self.top()
                        && let Class::Array(els, len) = &mut self.heap.get_mut(a).class
                    {
                        *len += values.len() as u32;
                        els.extend(values);
                    }
                }
                Op::Closure(i) => {
                    let code = self.frames[fi].code.funcs[i as usize].clone();
                    let (env, this, nt) = {
                        let f = &self.frames[fi];
                        (f.env, f.this.clone(), f.new_target.clone())
                    };
                    let home = self.frames[fi].home;
                    let is_arrow = code.kind == Kind::Arrow;
                    let g = self.make_closure(code, env, if is_arrow { this } else { Value::Undefined }, if is_arrow { nt } else { Value::Undefined }, if is_arrow { home } else { None });
                    self.stack.push(Value::Obj(g));
                }
                Op::Class(i, heritage) => {
                    let h = if heritage { Some(self.pop()) } else { None };
                    let (ctor, proto) = self.make_class(fi, i, h)?;
                    self.stack.push(Value::Obj(ctor));
                    self.stack.push(Value::Obj(proto));
                }
                Op::Template(i) => {
                    let g = self.template_object(fi, i);
                    self.stack.push(Value::Obj(g));
                }
                Op::RegExp(i) => {
                    let (src, flags) = self.frames[fi].code.regexps[i as usize].clone();
                    let g = self.regexp_create(&src, &flags)?;
                    self.stack.push(Value::Obj(g));
                }
                Op::Add => {
                    let b = self.pop();
                    let a = self.pop();
                    let v = match (&a, &b) {
                        (Value::Num(x), Value::Num(y)) => Value::Num(x + y),
                        (Value::Str(x), Value::Str(y)) => Value::Str(x.concat(y)),
                        _ => self.add(&a, &b)?,
                    };
                    self.stack.push(v);
                }
                Op::Sub | Op::Mul | Op::Div | Op::Rem | Op::Exp | Op::Shl | Op::Shr | Op::UShr | Op::BitAnd | Op::BitOr | Op::BitXor => {
                    let b = self.pop();
                    let a = self.pop();
                    let v = match (op, &a, &b) {
                        (Op::Sub, Value::Num(x), Value::Num(y)) => Value::Num(x - y),
                        (Op::Mul, Value::Num(x), Value::Num(y)) => Value::Num(x * y),
                        (Op::Div, Value::Num(x), Value::Num(y)) => Value::Num(x / y),
                        _ => self.arith(op, &a, &b)?,
                    };
                    self.stack.push(v);
                }
                Op::Eq | Op::Ne => {
                    let b = self.pop();
                    let a = self.pop();
                    let r = self.loose_equals(&a, &b)?;
                    self.stack.push(Value::Bool(r == (op == Op::Eq)));
                }
                Op::StrictEq | Op::StrictNe => {
                    let b = self.pop();
                    let a = self.pop();
                    let r = strict_equals(&a, &b);
                    self.stack.push(Value::Bool(r == (op == Op::StrictEq)));
                }
                Op::Lt | Op::Gt | Op::Le | Op::Ge => {
                    let b = self.pop();
                    let a = self.pop();
                    let r = match (&a, &b) {
                        (Value::Num(x), Value::Num(y)) => match op {
                            Op::Lt => x < y,
                            Op::Gt => x > y,
                            Op::Le => x <= y,
                            _ => x >= y,
                        },
                        _ => match op {
                            Op::Lt => self.less_than(&a, &b, true)? == Some(true),
                            Op::Gt => self.less_than(&b, &a, false)? == Some(true),
                            Op::Le => self.less_than(&b, &a, false)? == Some(false),
                            _ => self.less_than(&a, &b, true)? == Some(false),
                        },
                    };
                    self.stack.push(Value::Bool(r));
                }
                Op::InstanceOf => {
                    let t = self.pop();
                    let v = self.pop();
                    let r = self.instance_of(&v, &t)?;
                    self.stack.push(Value::Bool(r));
                }
                Op::Neg => {
                    let a = self.pop();
                    let v = match self.to_numeric(&a)? {
                        Value::Big(b) => Value::Big(Rc::new(b.neg())),
                        Value::Num(x) => Value::Num(-x),
                        v => v,
                    };
                    self.stack.push(v);
                }
                Op::Pos => {
                    let a = self.pop();
                    let x = self.to_number(&a)?;
                    self.stack.push(Value::Num(x));
                }
                Op::Not => {
                    let a = self.pop();
                    self.stack.push(Value::Bool(!a.truthy()));
                }
                Op::BitNot => {
                    let a = self.pop();
                    let v = match self.to_numeric(&a)? {
                        Value::Big(b) => Value::Big(Rc::new(b.not())),
                        Value::Num(x) => Value::Num(!crate::value::to_int32(x) as f64),
                        v => v,
                    };
                    self.stack.push(v);
                }
                Op::TypeOf => {
                    let a = self.pop();
                    let t = self.type_of(&a);
                    self.stack.push(Value::str(t));
                }
                Op::ToNumeric => {
                    let a = self.pop();
                    let v = self.to_numeric(&a)?;
                    self.stack.push(v);
                }
                Op::Inc | Op::Dec => {
                    let a = self.pop();
                    let d = if op == Op::Inc { 1.0 } else { -1.0 };
                    let v = match a {
                        Value::Num(x) => Value::Num(x + d),
                        Value::Big(b) => Value::Big(Rc::new(b.add(&crate::bigint::BigInt::from_i64(d as i64)))),
                        other => {
                            let x = self.to_number(&other)?;
                            Value::Num(x + d)
                        }
                    };
                    self.stack.push(v);
                }
                Op::ToKey => {
                    let a = self.pop();
                    let k = self.to_key(&a)?;
                    let v = self.key_value(k);
                    self.stack.push(v);
                }
                Op::CheckBase => {
                    let o = &self.stack[self.stack.len() - 2];
                    if o.is_nullish() {
                        let d = self.describe(o);
                        return Err(self.type_error(&format!("Cannot read properties of {d}")));
                    }
                }
                Op::ToStr => {
                    let a = self.pop();
                    let v = match a {
                        Value::Str(_) => a,
                        other => Value::Str(self.to_string(&other)?),
                    };
                    self.stack.push(v);
                }
                Op::Jump(t) => self.frames[fi].pc = t as usize,
                Op::JumpIfFalse(t) => {
                    if !self.pop().truthy() {
                        self.frames[fi].pc = t as usize;
                    }
                }
                Op::JumpIfTrue(t) => {
                    if self.pop().truthy() {
                        self.frames[fi].pc = t as usize;
                    }
                }
                Op::JumpIfFalseKeep(t) => {
                    if !self.top().truthy() {
                        self.frames[fi].pc = t as usize;
                    } else {
                        self.stack.pop();
                    }
                }
                Op::JumpIfTrueKeep(t) => {
                    if self.top().truthy() {
                        self.frames[fi].pc = t as usize;
                    } else {
                        self.stack.pop();
                    }
                }
                Op::JumpIfNotNullishKeep(t) => {
                    if !self.top().is_nullish() {
                        self.frames[fi].pc = t as usize;
                    } else {
                        self.stack.pop();
                    }
                }
                Op::JumpIfNullish(t) => {
                    if self.top().is_nullish() {
                        self.stack.pop();
                        self.frames[fi].pc = t as usize;
                    }
                }
                Op::JumpIfNotUndefined(t) => {
                    if !self.top().is_undefined() {
                        self.frames[fi].pc = t as usize;
                    }
                }
                Op::Call(argc) => {
                    self.tick()?;
                    self.call_op(argc as usize)?;
                }
                Op::CallSpread => {
                    self.tick()?;
                    let arr = self.pop();
                    let this = self.pop();
                    let f = self.pop();
                    let args = self.array_values(&arr);
                    let r = self.call(&f, this, &args)?;
                    self.stack.push(r);
                }
                Op::New(argc) => {
                    self.tick()?;
                    self.new_op(argc as usize)?;
                }
                Op::NewSpread => {
                    self.tick()?;
                    let arr = self.pop();
                    let f = self.pop();
                    let args = self.array_values(&arr);
                    let r = self.construct(&f, &args, &f.clone())?;
                    self.stack.push(r);
                }
                Op::SuperCall(argc) => {
                    let args = self.stack.split_off(self.stack.len() - argc as usize);
                    let r = self.super_call(fi, &args)?;
                    self.stack.push(r);
                }
                Op::SuperCallSpread => {
                    let arr = self.pop();
                    let args = self.array_values(&arr);
                    let r = self.super_call(fi, &args)?;
                    self.stack.push(r);
                }
                Op::CallEval(argc) => {
                    self.tick()?;
                    let at = self.stack.len() - argc as usize - 2;
                    let f = self.stack[at].clone();
                    if matches!(f, Value::Obj(g) if g == self.proto(I::Eval)) {
                        let args = self.stack.split_off(at + 2);
                        self.stack.truncate(at);
                        let r = self.direct_eval(fi, args.first().cloned().unwrap_or(Value::Undefined))?;
                        self.stack.push(r);
                    } else {
                        self.call_op(argc as usize)?;
                    }
                }
                Op::Return => {
                    let v = self.pop();
                    if let Some(s) = self.do_return(v, depth)? {
                        return Ok(s);
                    }
                }
                Op::Throw => {
                    let v = self.pop();
                    return Err(v);
                }
                Op::Try(t) | Op::TryFinally(t) => {
                    let sp = self.stack.len();
                    let f = &mut self.frames[fi];
                    let env = f.env;
                    f.handlers.push(Handler { pc: t, sp, env, finally: matches!(op, Op::TryFinally(_)) });
                }
                Op::EndTry => {
                    self.frames[fi].handlers.pop();
                }
                Op::ThrowTypeError(s) => {
                    let m = self.code_str(s).to_rust();
                    return Err(self.type_error(&m));
                }
                Op::ThrowRefError(s) => {
                    let m = self.code_str(s).to_rust();
                    return Err(self.reference_error(&m));
                }
                Op::EndFinally(kr, vr, table) => {
                    let kind = match self.stack[base + kr as usize] {
                        Value::Num(k) => k as u32,
                        _ => 0,
                    };
                    let v = self.stack[base + vr as usize].clone();
                    match kind {
                        0 => {}
                        1 => return Err(v),
                        2 => {
                            if let Some(s) = self.do_return(v, depth)? {
                                return Ok(s);
                            }
                        }
                        k => {
                            if table != NONE {
                                let to = self.frames[fi].code.jumps[table as usize][(k - 3) as usize];
                                self.frames[fi].pc = to as usize;
                            }
                        }
                    }
                }
                Op::GetIterator => {
                    let v = self.pop();
                    let (it, next) = self.get_iterator(&v, false)?;
                    self.stack.push(it);
                    self.stack.push(next);
                }
                Op::GetAsyncIterator => {
                    let v = self.pop();
                    let (it, next) = self.get_iterator(&v, true)?;
                    self.stack.push(it);
                    self.stack.push(next);
                }
                Op::IterNext(t) => {
                    let l = self.stack.len();
                    let (it, next) = (self.stack[l - 2].clone(), self.stack[l - 1].clone());
                    match self.iter_step(&it, &next)? {
                        Some(v) => self.stack.push(v),
                        None => self.frames[fi].pc = t as usize,
                    }
                }
                Op::IterCall => {
                    let l = self.stack.len();
                    let (it, next) = (self.stack[l - 2].clone(), self.stack[l - 1].clone());
                    let r = self.call(&next, it, &[])?;
                    self.stack.push(r);
                }
                Op::IterResult(t) => {
                    let r = self.pop();
                    let Value::Obj(rg) = r else { return Err(self.type_error("Iterator result is not an object")) };
                    let done = self.get(rg, Key::Str(n::DONE), &r)?.truthy();
                    if done {
                        self.frames[fi].pc = t as usize;
                    } else {
                        let v = self.get(rg, Key::Str(n::VALUE), &r)?;
                        self.stack.push(v);
                    }
                }
                Op::IterClose => {
                    self.stack.pop();
                    let it = self.pop();
                    self.iterator_close(&it)?;
                }
                Op::IterCloseAfter(kr) => {
                    let kind = match self.stack[base + kr as usize] {
                        Value::Num(k) => k as u32,
                        _ => 0,
                    };
                    self.stack.pop();
                    let it = self.pop();
                    if kind == 1 {
                        let _ = self.iterator_close(&it);
                    } else {
                        self.iterator_close(&it)?;
                    }
                }
                Op::IterValue(dr) => {
                    let done = self.stack[base + dr as usize].truthy();
                    if done {
                        self.stack.push(Value::Undefined);
                    } else {
                        let l = self.stack.len();
                        let (it, next) = (self.stack[l - 2].clone(), self.stack[l - 1].clone());
                        match self.iter_step(&it, &next) {
                            Ok(Some(v)) => self.stack.push(v),
                            Ok(None) => {
                                self.stack[base + dr as usize] = Value::Bool(true);
                                self.stack.push(Value::Undefined);
                            }
                            Err(e) => {
                                self.stack[base + dr as usize] = Value::Bool(true);
                                return Err(e);
                            }
                        }
                    }
                }
                Op::IterRest(dr) => {
                    let mut out = Vec::new();
                    if !self.stack[base + dr as usize].truthy() {
                        let l = self.stack.len();
                        let (it, next) = (self.stack[l - 2].clone(), self.stack[l - 1].clone());
                        while let Some(v) = self.iter_step(&it, &next)? {
                            out.push(v);
                        }
                        self.stack[base + dr as usize] = Value::Bool(true);
                    }
                    let a = self.new_array(out);
                    self.stack.push(Value::Obj(a));
                }
                Op::IterEnd(dr) => {
                    self.stack.pop();
                    let it = self.pop();
                    if !self.stack[base + dr as usize].truthy() {
                        self.iterator_close(&it)?;
                    }
                }
                Op::AsyncIterClose(kr) => {
                    let kind = if kr == NONE {
                        0
                    } else {
                        match self.stack[base + kr as usize] {
                            Value::Num(k) => k as u32,
                            _ => 0,
                        }
                    };
                    self.stack.pop();
                    let it = self.pop();
                    let r = self.get_method(&it, Key::Str(n::RETURN));
                    let v = match r {
                        Ok(m) if !m.is_undefined() => match self.call(&m, it, &[]) {
                            Ok(v) => v,
                            Err(e) if kind != 1 => return Err(e),
                            Err(_) => Value::Undefined,
                        },
                        Ok(_) => Value::Undefined,
                        Err(e) if kind != 1 => return Err(e),
                        Err(_) => Value::Undefined,
                    };
                    self.stack.push(v);
                }
                Op::ForIn => {
                    let o = self.pop();
                    let e = self.for_in_enumerator(&o)?;
                    self.stack.push(Value::Obj(e));
                }
                Op::ForInNext(t) => {
                    let Value::Obj(e) = *self.top() else { continue };
                    match self.for_in_next(e)? {
                        Some(k) => self.stack.push(k),
                        None => self.frames[fi].pc = t as usize,
                    }
                }
                Op::Generator => {
                    if fi == depth {
                        return Ok(Signal::Start);
                    }
                }
                Op::Yield => {
                    let v = self.pop();
                    return Ok(Signal::Yield(v));
                }
                Op::YieldStar => {
                    if let Some(s) = self.yield_star(fi, depth)? {
                        return Ok(s);
                    }
                }
                Op::Await | Op::AsyncYield => {
                    let v = self.pop();
                    return Ok(Signal::Await(v));
                }
                Op::Debugger => {}
                Op::ImportMeta => {
                    let Some(m) = self.current_module() else { return Err(self.syntax_error("import.meta outside a module")) };
                    let g = self.import_meta(m);
                    self.stack.push(Value::Obj(g));
                }
                Op::ImportCall => {
                    let options = self.pop();
                    let spec = self.pop();
                    let m = self.current_module();
                    let p = self.dynamic_import(m, spec, options);
                    self.stack.push(p);
                }
                Op::ToObject => {
                    let v = self.pop();
                    let o = self.to_object(&v)?;
                    self.stack.push(Value::Obj(o));
                }
                Op::RequireObject => {
                    if self.top().is_nullish() {
                        let d = self.describe(self.top());
                        return Err(self.type_error(&format!("Cannot destructure '{d}' as it is {d}.")));
                    }
                }
                Op::CopyRest(nkeys) => {
                    let keys_v = self.stack.split_off(self.stack.len() - nkeys as usize);
                    let src = self.pop();
                    let mut keys = Vec::new();
                    for k in &keys_v {
                        keys.push(self.to_key(k)?);
                    }
                    let o = self.new_object();
                    self.copy_data_properties(o, &src, &keys)?;
                    self.stack.push(Value::Obj(o));
                }
                Op::SetFunctionName(a) => {
                    if let Value::Obj(f) = *self.top() {
                        let s = self.str_of(a);
                        self.put_key(f, Key::Str(n::NAME), Value::Str(s), CONFIGURABLE);
                    }
                }
                Op::SetFunctionNameKey => {
                    let l = self.stack.len();
                    if let (Value::Obj(f), k) = (self.stack[l - 1].clone(), self.stack[l - 2].clone()) {
                        let key = self.to_key(&k)?;
                        let name = self.key_name(key, "");
                        self.put_key(f, Key::Str(n::NAME), Value::Str(name), CONFIGURABLE);
                    }
                }
                Op::RunFields => {}
                Op::PrivateName(s) => {
                    let d = self.code_str(s);
                    let g = self.new_symbol(Some(d), SYM_PRIVATE);
                    self.stack.push(Value::Sym(g));
                }
                Op::SetFields => {
                    let fl = self.pop();
                    if let (Value::Obj(c), Value::Obj(fl)) = (self.top().clone(), fl)
                        && let Class::Function(cl) = &mut self.heap.get_mut(c).class
                    {
                        cl.fields = Some(fl);
                    }
                }
                Op::SetHome => {
                    let f = self.pop();
                    let h = self.pop();
                    if let (Value::Obj(fg), Value::Obj(hg)) = (&f, &h)
                        && let Class::Function(c) = &mut self.heap.get_mut(*fg).class
                    {
                        c.home = Some(*hg);
                    }
                    self.stack.push(f);
                }
                Op::GlobalDecls(i) => self.global_decls(fi, i)?,
            }
        }
    }

    /// Counts steps (calls and loops' back edges), for a host's budget; and collects garbage now and then.
    fn tick(&mut self) -> Result<(), Value> {
        self.steps += 1;
        if self.steps > self.step_limit {
            self.step_limit = u64::MAX;
            return Err(self.range_error("Script ran too long"));
        }
        if self.heap.made > self.gc_threshold && self.native_depth == 0 {
            self.collect();
        }
        Ok(())
    }

    /// The call on the stack (f this args): a JavaScript function's frame pushed, or anything else called and its
    /// result put in place of it all.
    fn call_op(&mut self, argc: usize) -> Result<(), Value> {
        let at = self.stack.len() - argc - 2;
        let f = self.stack[at].clone();
        if let Value::Obj(g) = f
            && let Class::Function(c) = &self.heap.get(g).class
        {
            let code = &c.code;
            if !code.is_generator && !code.is_async && !matches!(code.kind, Kind::BaseConstructor | Kind::DerivedConstructor) {
                return self.enter_closure(g, argc, Value::Undefined, false);
            }
        }
        let args = self.stack.split_off(at + 2);
        let this = self.pop();
        self.stack.pop();
        let r = self.call(&f, this, &args)?;
        self.stack.push(r);
        Ok(())
    }

    /// `new` on the stack (f args).
    fn new_op(&mut self, argc: usize) -> Result<(), Value> {
        let at = self.stack.len() - argc - 1;
        let f = self.stack[at].clone();
        if let Value::Obj(g) = f
            && let Class::Function(c) = &self.heap.get(g).class
            && matches!(c.code.kind, Kind::Normal | Kind::BaseConstructor)
            && !c.code.is_async
            && !c.code.is_generator
        {
            let fields = if c.code.kind == Kind::BaseConstructor { c.fields } else { None };
            let this = self.create_from_constructor(&f, I::ObjectProto)?;
            if let Some(fl) = fields {
                self.call(&Value::Obj(fl), Value::Obj(this), &[])?;
            }
            self.stack.insert(at + 1, Value::Obj(this));
            return self.enter_closure(g, argc, f, true);
        }
        let args = self.stack.split_off(at + 1);
        self.stack.pop();
        let r = self.construct(&f, &args, &f)?;
        self.stack.push(r);
        Ok(())
    }

    fn super_call(&mut self, fi: usize, args: &[Value]) -> Result<Value, Value> {
        let Some(callee) = self.frames[fi].callee else { return Err(self.syntax_error("'super' keyword unexpected here")) };
        let parent = self.heap.get(callee).proto.map_or(Value::Null, Value::Obj);
        if !self.is_constructor(&parent) {
            return Err(self.type_error("Super constructor is not a constructor"));
        }
        let nt = self.frames[fi].new_target.clone();
        let r = self.construct(&parent, args, &nt)?;
        if !matches!(self.frames[fi].this, Value::Empty) {
            return Err(self.reference_error("Super constructor may only be called once"));
        }
        self.frames[fi].this = r.clone();
        let fields = match &self.heap.get(callee).class {
            Class::Function(c) => c.fields,
            _ => None,
        };
        if let Some(fl) = fields {
            self.call(&Value::Obj(fl), r.clone(), &[])?;
        }
        Ok(r)
    }

    fn home_proto(&mut self, fi: usize) -> Result<Option<Gc>, Value> {
        let Some(home) = self.frames[fi].home else { return Err(self.syntax_error("'super' keyword unexpected here")) };
        self.get_prototype_of(home)
    }

    fn get_super(&mut self, fi: usize, key: Key) -> Result<Value, Value> {
        let this = self.frames[fi].this.clone();
        if matches!(this, Value::Empty) {
            return Err(self.reference_error("Must call super constructor in derived class before accessing 'this' or returning from derived constructor"));
        }
        match self.home_proto(fi)? {
            Some(p) => self.get(p, key, &this),
            None => Err(self.type_error("Cannot read properties of null (super)")),
        }
    }

    fn set_super(&mut self, fi: usize, key: Key, v: Value) -> Result<(), Value> {
        let this = self.frames[fi].this.clone();
        if matches!(this, Value::Empty) {
            return Err(self.reference_error("Must call super constructor in derived class before accessing 'this' or returning from derived constructor"));
        }
        let Some(p) = self.home_proto(fi)? else { return Err(self.type_error("Cannot set properties of null (super)")) };
        let ok = self.set(p, key, v, &this)?;
        if !ok && self.frames[fi].code.strict {
            return Err(self.type_error("Cannot assign to read only property"));
        }
        Ok(())
    }

    /// The arguments object of a frame (an unmapped one, for strict and sloppy code alike).
    fn arguments_object(&mut self, fi: usize, map: Option<ArgMap>) -> Gc {
        let args = self.frames[fi].args.clone();
        let mapped = map.is_some();
        let callee = self.frames[fi].callee;
        let p = self.proto(I::ObjectProto);
        // (Only the arguments given are mapped.)
        let map = map.map(|mut m| {
            m.slots.truncate(args.len());
            Box::new(m)
        });
        let g = self.alloc(Some(p), Class::Arguments(map));
        let len = args.len();
        for (i, a) in args.into_iter().enumerate() {
            self.heap.get_mut(g).props.set(Key::Index(i as u32), Prop::data(a, ALL));
        }
        self.put_key(g, Key::Str(n::LENGTH), Value::Num(len as f64), HIDDEN);
        let values = self.proto(I::ArrayValues);
        let it = self.sym(Sym::Iterator);
        self.put_key(g, Key::Sym(it), Value::Obj(values), HIDDEN);
        if !mapped {
            let t = Value::Obj(self.proto(I::ThrowTypeError));
            self.heap.get_mut(g).props.set(Key::Str(n::CALLEE), Prop { slot: Slot::Accessor(t.clone(), t), flags: 0 });
        } else if let Some(c) = callee {
            self.put_key(g, Key::Str(n::CALLEE), Value::Obj(c), HIDDEN);
        }
        g
    }

    /// A tagged template's strings array (made once for its place in the code).
    fn template_object(&mut self, fi: usize, i: u32) -> Gc {
        let code = self.frames[fi].code.clone();
        if let Some(Some(g)) = code.template_objects.borrow().get(i as usize) {
            return *g;
        }
        let (cooked, raw) = code.templates[i as usize].clone();
        let raw_vals: Vec<Value> = raw.into_iter().map(Value::Str).collect();
        let cooked_vals: Vec<Value> = cooked.into_iter().map(|c| c.map_or(Value::Undefined, Value::Str)).collect();
        let r = self.new_array(raw_vals);
        self.freeze(r);
        let a = self.new_array(cooked_vals);
        self.put_key(a, Key::Str(n::RAW), Value::Obj(r), 0);
        self.freeze(a);
        let mut objs = code.template_objects.borrow_mut();
        if objs.len() <= i as usize {
            objs.resize(i as usize + 1, None);
        }
        objs[i as usize] = Some(a);
        a
    }

    /// Object.freeze, for objects the engine makes (no proxies, no exotics but arrays).
    pub fn freeze(&mut self, g: Gc) {
        let o = self.heap.get_mut(g);
        if let Class::Array(els, len) = &mut o.class {
            let els = core::mem::take(els);
            let len = *len;
            for (i, v) in els.into_iter().enumerate() {
                if !matches!(v, Value::Empty) {
                    o.props.set(Key::Index(i as u32), Prop::data(v, ENUMERABLE));
                }
            }
            o.props.set(Key::Str(n::LENGTH), Prop::data(Value::Num(len as f64), 0));
            if let Class::Array(_, l) = &mut o.class {
                *l = len;
            }
        }
        for (_, p) in o.props.iter_mut() {
            p.flags &= !CONFIGURABLE;
            if matches!(p.slot, Slot::Data(_)) {
                p.flags &= !WRITABLE;
            }
        }
        o.extensible = false;
    }

    /// A class made: its constructor and prototype.
    fn make_class(&mut self, fi: usize, i: u32, heritage: Option<Value>) -> Result<(Gc, Gc), Value> {
        let code = self.frames[fi].code.clone();
        let info = &code.classes[i as usize];
        let (proto_parent, ctor_parent) = match &heritage {
            None => (Some(self.proto(I::ObjectProto)), self.proto(I::FunctionProto)),
            Some(Value::Null) => (None, self.proto(I::FunctionProto)),
            Some(h) => {
                if !self.is_constructor(h) {
                    return Err(self.type_error(&format!("Class extends value {} is not a constructor or null", self.describe(h))));
                }
                let Value::Obj(hg) = h else { unreachable!() };
                let pp = self.get(*hg, Key::Str(n::PROTOTYPE), h)?;
                let pp = match pp {
                    Value::Obj(p) => Some(p),
                    Value::Null => None,
                    _ => return Err(self.type_error("Class extends value does not have valid prototype property")),
                };
                (pp, *hg)
            }
        };
        let proto = self.alloc(proto_parent, Class::Ordinary);
        let ccode = code.funcs[info.constructor as usize].clone();
        let env = self.frames[fi].env;
        let ctor = self.make_closure(ccode, env, Value::Undefined, Value::Undefined, Some(proto));
        let o = self.heap.get_mut(ctor);
        o.proto = Some(ctor_parent);
        // (A class's prototype property isn't writable.)
        o.props.set(Key::Str(n::PROTOTYPE), Prop::data(Value::Obj(proto), 0));
        if let Some(name) = info.name {
            let s = self.str_of(name);
            self.put_key(ctor, Key::Str(n::NAME), Value::Str(s), CONFIGURABLE);
        }
        self.put_key(proto, Key::Str(n::CONSTRUCTOR), Value::Obj(ctor), HIDDEN);
        Ok((ctor, proto))
    }

    /// A method defined on an object literal or class: its home object set, its name from its key.
    fn define_method(&mut self, o: Gc, key: Key, f: Value, flags: u8) -> Result<(), Value> {
        let Value::Obj(fg) = f else { return Ok(()) };
        if let Class::Function(c) = &mut self.heap.get_mut(fg).class {
            c.home = Some(o);
        }
        let prefix = match flags & 3 {
            1 => "get ",
            2 => "set ",
            _ => "",
        };
        let name = self.key_name(key, prefix);
        self.put_key(fg, Key::Str(n::NAME), Value::Str(name), CONFIGURABLE);
        let enumerable = if flags & 4 != 0 { ENUMERABLE } else { 0 };
        let desc = match flags & 3 {
            1 => Desc { get: Some(f), enumerable: Some(enumerable != 0), configurable: Some(true), ..Desc::default() },
            2 => Desc { set: Some(f), enumerable: Some(enumerable != 0), configurable: Some(true), ..Desc::default() },
            _ => Desc { value: Some(f), writable: Some(true), enumerable: Some(enumerable != 0), configurable: Some(true), ..Desc::default() },
        };
        self.define_own_property_or_throw(o, key, desc)
    }

    /// A function's name from a property key ("[description]" for a symbol), with a prefix ("get ").
    pub fn key_name(&self, key: Key, prefix: &str) -> JsStr {
        let base = match key {
            Key::Str(a) => self.str_of(a),
            Key::Index(i) => JsStr::from_str(&format!("{i}")),
            Key::Sym(s) => match &self.heap.get(s).class {
                Class::Symbol(Some(d), f) if f & SYM_PRIVATE != 0 => d.clone(),
                Class::Symbol(Some(d), _) => JsStr::from_str("[").concat(d).concat(&JsStr::from_str("]")),
                _ => JsStr::empty(),
            },
        };
        if prefix.is_empty() { base } else { JsStr::from_str(prefix).concat(&base) }
    }

    /// A private member read.
    fn private_get(&mut self, o: &Value, p: &Value) -> Result<Value, Value> {
        let (Value::Obj(g), Value::Sym(s)) = (o, p) else { return Err(self.type_error("Cannot read private member from an object whose class did not declare it")) };
        let slot = self.heap.get(*g).props.get(&Key::Sym(*s)).map(|p| p.slot.clone());
        match slot {
            Some(Slot::Data(v)) => Ok(v),
            Some(Slot::Accessor(get, _)) if !get.is_undefined() => self.call(&get, o.clone(), &[]),
            Some(Slot::Accessor(..)) => Err(self.type_error("'#x' was defined without a getter")),
            None => Err(self.type_error("Cannot read private member from an object whose class did not declare it")),
        }
    }

    fn private_set(&mut self, o: &Value, p: &Value, v: Value) -> Result<(), Value> {
        let (Value::Obj(g), Value::Sym(s)) = (o, p) else { return Err(self.type_error("Cannot write private member to an object whose class did not declare it")) };
        let prop = self.heap.get(*g).props.get(&Key::Sym(*s)).cloned();
        match prop {
            Some(Prop { slot: Slot::Data(_), flags }) if flags & WRITABLE != 0 => {
                self.heap.get_mut(*g).props.set(Key::Sym(*s), Prop::data(v, flags));
                Ok(())
            }
            Some(Prop { slot: Slot::Data(_), .. }) => Err(self.type_error("Private method is not writable")),
            Some(Prop { slot: Slot::Accessor(_, set), .. }) if !set.is_undefined() => {
                self.call(&set, o.clone(), &[v])?;
                Ok(())
            }
            Some(_) => Err(self.type_error("'#x' was defined without a setter")),
            None => Err(self.type_error("Cannot write private member to an object whose class did not declare it")),
        }
    }

    fn private_define(&mut self, o: &Value, p: &Value, slot: Slot, flags: u8) -> Result<(), Value> {
        let (Value::Obj(g), Value::Sym(s)) = (o, p) else { return Err(self.type_error("Cannot define a private member")) };
        let key = Key::Sym(*s);
        let existing = self.heap.get(*g).props.get(&key).cloned();
        let slot = match (existing, slot) {
            (None, s) => s,
            (Some(Prop { slot: Slot::Accessor(get, Value::Undefined), .. }), Slot::Accessor(Value::Undefined, set)) if !get.is_undefined() => Slot::Accessor(get, set),
            (Some(Prop { slot: Slot::Accessor(Value::Undefined, set), .. }), Slot::Accessor(get, Value::Undefined)) if !set.is_undefined() => Slot::Accessor(get, set),
            _ => return Err(self.type_error("Cannot initialize private member twice on the same object")),
        };
        if matches!(self.heap.get(*g).class, Class::Proxy(..)) || !self.heap.get(*g).extensible && false {
            return Err(self.type_error("Cannot define a private member"));
        }
        self.heap.get_mut(*g).props.set(key, Prop { slot, flags });
        Ok(())
    }

    // ------------------------------------------------------------------------------------- names looked up late

    fn get_global(&mut self, a: Atom) -> Result<Value, Value> {
        if let Some((v, _)) = self.global_lex.get(&a) {
            if matches!(v, Value::Empty) {
                return Err(self.reference_error(&format!("Cannot access '{}' before initialization", self.str_of(a))));
            }
            return Ok(v.clone());
        }
        let g = self.global;
        let key = self.atom_key(a);
        // Fast: an own data property of the global object.
        if let Some(Prop { slot: Slot::Data(v), .. }) = self.heap.get(g).props.get(&key) {
            return Ok(v.clone());
        }
        if self.has_property(g, key)? {
            return self.get(g, key, &Value::Obj(g));
        }
        Err(self.reference_error(&format!("{} is not defined", self.str_of(a))))
    }

    fn set_global(&mut self, a: Atom, v: Value, strict: bool) -> Result<(), Value> {
        if let Some((cur, kind)) = self.global_lex.get_mut(&a) {
            if matches!(cur, Value::Empty) {
                return Err(self.reference_error(&format!("Cannot access '{}' before initialization", self.atoms.get(a))));
            }
            if *kind == CONST {
                return Err(self.type_error("Assignment to constant variable."));
            }
            *cur = v;
            return Ok(());
        }
        let g = self.global;
        let key = self.atom_key(a);
        if let Some(p) = self.heap.get_mut(g).props.get_mut(&key)
            && let Slot::Data(cur) = &mut p.slot
            && p.flags & WRITABLE != 0
        {
            *cur = v;
            return Ok(());
        }
        if strict && !self.has_property(g, key)? {
            return Err(self.reference_error(&format!("{} is not defined", self.str_of(a))));
        }
        let ok = self.set(g, key, v, &Value::Obj(g))?;
        if !ok && strict {
            return Err(self.type_error(&format!("Cannot assign to read only property '{}'", self.str_of(a))));
        }
        Ok(())
    }

    /// A name looked up through the scopes as the code runs (a with's object, an eval's names), then the globals.
    fn lookup_name(&mut self, a: Atom) -> Result<Option<NameRef>, Value> {
        let mut e = self.frames.last().and_then(|f| f.env);
        while let Some(g) = e {
            let (with, slot, parent) = match &self.heap.get(g).class {
                Class::Env(env) => (env.with, env.names.iter().position(|(x, _)| *x == a), env.parent),
                _ => (None, None, None),
            };
            if let Some(o) = with {
                let key = self.atom_key(a);
                if self.has_property(o, key)? {
                    // @@unscopables
                    let u = self.sym(Sym::Unscopables);
                    let un = self.get(o, Key::Sym(u), &Value::Obj(o))?;
                    let blocked = match un {
                        Value::Obj(ug) => self.get(ug, key, &un)?.truthy(),
                        _ => false,
                    };
                    if !blocked {
                        return Ok(Some(NameRef::Object(o, true)));
                    }
                }
            } else if let Some(s) = slot {
                return Ok(Some(NameRef::Env(g, s)));
            }
            e = parent;
        }
        if self.global_lex.contains_key(&a) {
            return Ok(Some(NameRef::Lexical));
        }
        let g = self.global;
        let key = self.atom_key(a);
        if self.has_property(g, key)? {
            return Ok(Some(NameRef::Object(g, false)));
        }
        Ok(None)
    }

    fn read_ref(&mut self, r: NameRef, a: Atom) -> Result<Value, Value> {
        match r {
            NameRef::Env(g, s) => {
                let v = match &self.heap.get(g).class {
                    Class::Env(env) if env.names[s].1 == IMPORT => self.import_value(&env.vars[s]),
                    Class::Env(env) => env.vars[s].clone(),
                    _ => Value::Undefined,
                };
                if matches!(v, Value::Empty) {
                    return Err(self.reference_error(&format!("Cannot access '{}' before initialization", self.str_of(a))));
                }
                Ok(v)
            }
            NameRef::Object(o, _) => {
                let key = self.atom_key(a);
                self.get(o, key, &Value::Obj(o))
            }
            NameRef::Lexical => self.get_global(a),
        }
    }

    fn get_name(&mut self, a: Atom) -> Result<Value, Value> {
        match self.lookup_name(a)? {
            Some(r) => self.read_ref(r, a),
            None => Err(self.reference_error(&format!("{} is not defined", self.str_of(a)))),
        }
    }

    fn set_name(&mut self, a: Atom, v: Value, strict: bool) -> Result<(), Value> {
        match self.lookup_name(a)? {
            Some(NameRef::Env(g, s)) => {
                let kind = match &self.heap.get(g).class {
                    Class::Env(env) => env.names[s].1,
                    _ => VAR,
                };
                if kind == IMPORT {
                    return Err(self.type_error("Assignment to constant variable."));
                }
                let cur_empty = matches!(&self.heap.get(g).class, Class::Env(env) if matches!(env.vars[s], Value::Empty));
                if cur_empty {
                    return Err(self.reference_error(&format!("Cannot access '{}' before initialization", self.str_of(a))));
                }
                if kind == CONST {
                    return Err(self.type_error("Assignment to constant variable."));
                }
                if kind == FUNC_NAME {
                    if strict {
                        return Err(self.type_error("Assignment to constant variable."));
                    }
                    return Ok(());
                }
                if let Class::Env(env) = &mut self.heap.get_mut(g).class {
                    env.vars[s] = v;
                }
                Ok(())
            }
            Some(NameRef::Object(o, _)) => {
                let key = self.atom_key(a);
                let ok = self.set(o, key, v, &Value::Obj(o))?;
                if !ok && strict {
                    return Err(self.type_error(&format!("Cannot assign to read only property '{}'", self.str_of(a))));
                }
                Ok(())
            }
            Some(NameRef::Lexical) | None => self.set_global(a, v, strict),
        }
    }

    fn delete_name(&mut self, a: Atom) -> Result<bool, Value> {
        match self.lookup_name(a)? {
            Some(NameRef::Object(o, _)) => {
                let key = self.atom_key(a);
                self.delete_property(o, key)
            }
            Some(_) => Ok(false),
            None => Ok(true),
        }
    }

    /// Global code's declarations (or a sloppy eval's: in its caller's function scope, or the global object).
    fn global_decls(&mut self, fi: usize, i: u32) -> Result<(), Value> {
        let code = self.frames[fi].code.clone();
        let decls = &code.decls[i as usize];
        let env = self.frames[fi].env;
        if code.kind == Kind::Eval {
            // The var scope: the nearest function scope's environment, else the global object.
            let mut e = env;
            let mut target = None;
            while let Some(g) = e {
                match &self.heap.get(g).class {
                    Class::Env(en) if en.var_scope && en.with.is_none() => {
                        target = Some(g);
                        break;
                    }
                    Class::Env(en) => e = en.parent,
                    _ => break,
                }
            }
            for (name, func, _) in decls {
                let v = func.map(|f| {
                    let c = code.funcs[f as usize].clone();
                    Value::Obj(self.make_closure(c, env, Value::Undefined, Value::Undefined, None))
                });
                match target {
                    Some(t) => {
                        let Class::Env(en) = &mut self.heap.get_mut(t).class else { continue };
                        match en.names.iter().position(|(x, _)| x == name) {
                            Some(s) => {
                                if let Some(v) = v {
                                    en.vars[s] = v;
                                }
                            }
                            None => {
                                let mut names: Vec<(Atom, u8)> = en.names.to_vec();
                                names.push((*name, VAR));
                                en.names = Rc::from(names);
                                en.vars.push(v.unwrap_or(Value::Undefined));
                            }
                        }
                    }
                    None => {
                        if self.global_lex.contains_key(name) {
                            return Err(self.syntax_error(&format!("Identifier '{}' has already been declared", self.str_of(*name))));
                        }
                        let g = self.global;
                        let key = self.atom_key(*name);
                        match v {
                            Some(v) => {
                                self.define_own_property_or_throw(g, key, Desc::data(v, true, true, true))?;
                            }
                            None => {
                                if self.get_own_property(g, key)?.is_none() {
                                    self.define_own_property_or_throw(g, key, Desc::data(Value::Undefined, true, true, true))?;
                                }
                            }
                        }
                    }
                }
            }
            return Ok(());
        }
        // A script: first the checks (a let can't redeclare a var or let; a var can't redeclare a let).
        let g = self.global;
        for (name, _, kind) in decls {
            let key = self.atom_key(*name);
            if *kind != VAR {
                if self.global_lex.contains_key(name) {
                    return Err(self.syntax_error(&format!("Identifier '{}' has already been declared", self.str_of(*name))));
                }
                if let Some(p) = self.get_own_property(g, key)?
                    && !p.configurable()
                {
                    return Err(self.syntax_error(&format!("Identifier '{}' has already been declared", self.str_of(*name))));
                }
            } else if self.global_lex.contains_key(name) {
                return Err(self.syntax_error(&format!("Identifier '{}' has already been declared", self.str_of(*name))));
            }
        }
        for (name, func, kind) in decls {
            let key = self.atom_key(*name);
            match (func, *kind) {
                (Some(f), _) => {
                    let c = code.funcs[*f as usize].clone();
                    let v = Value::Obj(self.make_closure(c, env, Value::Undefined, Value::Undefined, None));
                    let existing = self.get_own_property(g, key)?;
                    let desc = match existing {
                        Some(p) if !p.configurable() => Desc { value: Some(v), ..Desc::default() },
                        _ => Desc::data(v, true, true, false),
                    };
                    self.define_own_property_or_throw(g, key, desc)?;
                }
                (None, VAR) => {
                    if self.get_own_property(g, key)?.is_none() && self.heap.get(g).extensible {
                        self.define_own_property_or_throw(g, key, Desc::data(Value::Undefined, true, true, false))?;
                    }
                }
                (None, k) => {
                    self.global_lex.insert(*name, (Value::Empty, k));
                }
            }
        }
        Ok(())
    }

    /// A direct eval: the string compiled as eval code and run in the calling frame's scope.
    fn direct_eval(&mut self, fi: usize, arg: Value) -> Result<Value, Value> {
        let Value::Str(s) = arg else { return Ok(arg) };
        let strict = self.frames[fi].code.strict;
        let src = s.to_rust();
        let script = match crate::parser::parse(&src, &mut self.atoms, false, strict) {
            Ok(sc) => sc,
            Err(e) => return Err(self.syntax_error(&e.message)),
        };
        let code = match crate::compiler::compile_script(&script, &mut self.atoms, Rc::from(src.as_str()), true) {
            Ok(c) => c,
            Err(e) => return Err(self.syntax_error(&e.message)),
        };
        let (env, this, nt, home, callee) = {
            let f = &self.frames[fi];
            (f.env, f.this.clone(), f.new_target.clone(), f.home, f.callee)
        };
        let base = self.stack.len();
        self.stack.resize(base + code.registers as usize, Value::Undefined);
        self.frames.push(Frame { code, pc: 0, base, bottom: base, env, this, callee, new_target: nt, home, args: Vec::new(), handlers: Vec::new(), construct: false, owner: None });
        self.run_frame()
    }

    /// An indirect eval (eval called by any other name): global code.
    pub fn indirect_eval(&mut self, src: &str) -> Result<Value, Value> {
        let script = match crate::parser::parse(src, &mut self.atoms, false, false) {
            Ok(sc) => sc,
            Err(e) => return Err(self.syntax_error(&e.message)),
        };
        let code = match crate::compiler::compile_script(&script, &mut self.atoms, Rc::from(src), true) {
            Ok(c) => c,
            Err(e) => return Err(self.syntax_error(&e.message)),
        };
        let g = self.global;
        self.run_code(code, None, Value::Obj(g))
    }

    // ----------------------------------------------------------------------------------------------- collection

    /// Marks everything reachable and frees the rest.
    pub fn collect(&mut self) -> usize {
        let mut more: Vec<Gc> = Vec::new();
        more.push(self.global);
        more.extend_from_slice(&self.intrinsics);
        more.extend_from_slice(&self.symbols);
        more.extend(self.registry.values().copied());
        more.extend(self.unhandled.iter().copied());
        for f in &self.frames {
            f.trace(&mut more);
        }
        let mut roots: Vec<Value> = self.stack.clone();
        self.modules.trace(&mut more, &mut roots);
        roots.extend(self.roots.iter().cloned());
        if let (Some(trace), Some(h)) = (self.host_trace, self.host.as_deref()) {
            trace(h, &mut roots);
        }
        for (v, _) in self.global_lex.values() {
            roots.push(v.clone());
        }
        for j in &self.jobs {
            match j {
                Job::Reaction { handler, arg, derived, resolve, reject, .. } => roots.extend([handler.clone(), arg.clone(), derived.clone(), resolve.clone(), reject.clone()]),
                Job::Thenable { promise, thenable, then } => {
                    more.push(*promise);
                    roots.extend([thenable.clone(), then.clone()]);
                }
                Job::Call { f, args } => {
                    roots.push(f.clone());
                    roots.extend(args.iter().cloned());
                }
            }
        }
        let freed = self.heap.collect(&roots, &more);
        self.gc_threshold = (self.heap.live * 2).max(100_000);
        freed
    }
}

/// Where a name looked up as the code runs was found.
pub enum NameRef {
    Env(Gc, usize),
    /// An object's property (a with's object: true, it's the call's this).
    Object(Gc, bool),
    Lexical,
}
