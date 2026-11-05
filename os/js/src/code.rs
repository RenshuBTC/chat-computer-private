//! Bytecode: a function compiled (compiler.rs) for the VM (vm.rs) to run. The VM is a stack machine: each
//! instruction takes its operands from the top of the stack and leaves its result there; a frame's registers (its
//! parameters and the variables no inner function uses, and the compiler's own temporaries) sit below its operands.

use crate::bigint::BigInt;
use crate::string::{Atom, JsStr};
use crate::value::Gc;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

/// An instruction. Stack effects are written `before -> after`, the top on the right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Nop,
    // -- constants: -> v
    Undefined,
    Null,
    True,
    False,
    Empty,
    Int(i32),
    Num(f64),
    Str(u32),
    BigInt(u32),
    // -- the stack
    Pop,
    /// a -> a a
    Dup,
    /// a b -> a b a b
    Dup2,
    /// a b -> b a
    Swap,
    /// a b c -> c a b (the top below the two under it)
    Rot3,
    /// a b c d -> d a b c
    Rot4,
    /// a b -> a b a
    Over,
    // -- registers: GetLocal -> v; SetLocal v -> v; InitLocal v -> (pops)
    GetLocal(u32),
    SetLocal(u32),
    InitLocal(u32),
    /// A `let` or `const` read before it's initialized throws (its name is string n).
    GetLocalChecked(u32, u32),
    SetLocalChecked(u32, u32),
    // -- scopes on the heap: (hops up from the current scope, slot)
    GetEnv(u16, u16),
    SetEnv(u16, u16),
    InitEnv(u16, u16),
    GetEnvChecked(u16, u16, u32),
    SetEnvChecked(u16, u16, u32),
    /// A new scope (its names: scopes[n]) inside the current one.
    PushEnv(u32),
    PopEnv,
    /// The current scope made anew with the same values (a `let` loop's next time round).
    CopyEnv,
    /// v -> throws TypeError: assignment to the constant named string n.
    ConstAssign(u32),
    // -- names the compiler couldn't resolve: the global scope's (its lexical names, then the global object)
    GetGlobal(Atom),
    /// v -> v (a strict code's assignment to a name that isn't there throws)
    SetGlobal(Atom),
    TypeofGlobal(Atom),
    /// Names looked up as the code runs, through every scope (with's objects, eval's variables).
    GetName(Atom),
    SetName(Atom),
    TypeofName(Atom),
    DeleteName(Atom),
    /// -> f this: a name looked up for a call (in a with: the with's object is `this`).
    GetNameForCall(Atom),
    /// v -> (a global let, const or class initialized)
    InitGlobal(Atom),
    /// obj -> (a with's scope entered: its object's properties are names)
    PushWith,
    // -- this and the function
    This,
    /// A derived constructor's this, which throws if super() hasn't been called yet.
    ThisChecked,
    NewTarget,
    /// The function running.
    Callee,
    /// -> the arguments object (an unmapped one's callee throws).
    Arguments,
    /// -> a sloppy function's arguments object whose elements are its parameters (code.arg_maps[n]: each
    /// argument's parameter's slot in the environment).
    MappedArguments(u32),
    /// -> an array of the arguments from n on (a rest parameter).
    Rest(u32),
    /// The number of arguments given (for parameters' defaults: an argument missing).
    ArgCount,
    // -- properties
    /// obj -> v
    GetProp(Atom),
    /// obj v -> v
    SetProp(Atom),
    /// obj key -> v
    GetElem,
    /// obj key v -> v
    SetElem,
    /// obj -> f obj: a method looked up for a call (the object stays, as its this).
    GetMethod(Atom),
    /// obj key -> f obj
    GetElemMethod,
    DeleteProp(Atom),
    DeleteElem,
    /// key obj -> bool
    In,
    /// The super object's property (this, and the function's home object's prototype).
    GetSuper(Atom),
    /// key -> v
    GetSuperElem,
    /// v -> v
    SetSuper(Atom),
    /// key v -> v
    SetSuperElem,
    /// -> f this: a super method for a call.
    GetSuperMethod(Atom),
    /// key -> f this
    GetSuperElemMethod,
    /// obj private -> v (a private name: a symbol of its class's)
    GetPrivate,
    /// obj private v -> v
    SetPrivate,
    /// obj private v -> obj (a private field defined)
    DefinePrivate,
    /// obj private f -> obj: a private method (0), getter (1) or setter (2) installed.
    DefinePrivateMethod(u8),
    /// private obj -> bool
    HasPrivate,
    // -- literals
    NewObject,
    /// -> a new array
    NewArray,
    /// obj v -> obj (a data property made)
    DefineProp(Atom),
    /// obj key v -> obj
    DefineElem,
    /// obj key f -> obj: a method (its home object set; flags: getter 1, setter 2, enumerable 4).
    DefineMethod(u8),
    /// obj src -> obj: {...src}
    SpreadObject,
    /// obj proto -> obj: `__proto__: proto`
    SetProtoLiteral,
    /// arr v -> arr
    ArrayPush,
    /// arr -> arr (a hole)
    ArrayHole,
    /// arr iterable -> arr
    ArraySpread,
    /// -> a function made from funcs[n], in the current scope.
    Closure(u32),
    /// heritage? -> ctor proto: a class made from classes[n] (heritage on the stack if it has one).
    Class(u32, bool),
    /// -> the template's strings array (made once for each place in the code)
    Template(u32),
    /// -> a new regular expression from regexps[n]
    RegExp(u32),
    // -- operators: a b -> v; a -> v
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Exp,
    Shl,
    Shr,
    UShr,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Lt,
    Gt,
    Le,
    Ge,
    InstanceOf,
    Neg,
    /// ToNumber
    Pos,
    Not,
    BitNot,
    TypeOf,
    /// ToNumeric (before ++ and --)
    ToNumeric,
    Inc,
    Dec,
    /// ToPropertyKey
    ToKey,
    /// obj key -> obj key: a TypeError if obj is null or undefined (before the key is converted)
    CheckBase,
    /// ToString (a template's substitutions)
    ToStr,
    // -- jumps (to an instruction's index)
    Jump(u32),
    /// v -> (pops)
    JumpIfFalse(u32),
    JumpIfTrue(u32),
    /// v -> v, jumps if falsy/truthy keeping it (&&, ||); else pops it.
    JumpIfFalseKeep(u32),
    JumpIfTrueKeep(u32),
    /// v -> v: jumps keeping it if it isn't null or undefined (??); else pops it.
    JumpIfNotNullishKeep(u32),
    /// v -> v: jumps (popping it) if it's null or undefined (an optional chain: undefined pushed there)
    JumpIfNullish(u32),
    /// v -> v: jumps if it isn't undefined (a default's not needed).
    JumpIfNotUndefined(u32),
    // -- calls
    /// f this args(n) -> result
    Call(u32),
    /// f this array -> result
    CallSpread,
    /// f args(n) -> obj
    New(u32),
    NewSpread,
    /// args(n) -> this (a derived constructor's super(): this made and fields run)
    SuperCall(u32),
    SuperCallSpread,
    /// A direct eval: f this args(n) -> result (an ordinary call if f isn't the real eval)
    CallEval(u32),
    /// v -> (returns)
    Return,
    // -- exceptions
    Throw,
    /// A try: on a throw, the stack is cut back to what it is now, the exception pushed, and it goes to n.
    Try(u32),
    /// A try's finally: on a throw, or a return (a return statement's, or a generator's return()) leaving it, the
    /// stack is cut back, the value and the kind (1 throw, 2 return) pushed, and it goes to n.
    TryFinally(u32),
    EndTry,
    /// Throws a TypeError (its message string n).
    ThrowTypeError(u32),
    ThrowRefError(u32),
    /// After a finally: what to do (registers: the kind, the value): 0 go on, 1 throw, 2 return, 3+ jump to
    /// jumps[kind - 3] of table n.
    EndFinally(u32, u32, u32),
    // -- iteration
    /// v -> iterator next
    GetIterator,
    GetAsyncIterator,
    /// iterator next -> iterator next value, or jumps (popping nothing) when done
    IterNext(u32),
    /// iterator next -> (closes it: its return method)
    IterClose,
    /// iterator next -> (closes it as a loop left by kind register n's completion: a throw's exception wins over
    /// what return() throws)
    IterCloseAfter(u32),
    /// iterator next result -> iterator next value, or jumps (popping the result) when done (for await's result,
    /// awaited)
    IterResult(u32),
    /// iterator next -> iterator next result: next() called (for await: the result is awaited next)
    IterCall,
    /// iterator next -> iterator next value: destructuring's next value (undefined once it's done: register n is
    /// set true then)
    IterValue(u32),
    /// iterator next -> iterator next array: destructuring's rest (register n set true)
    IterRest(u32),
    /// iterator next -> (closed unless register n says it's done: destructuring's end)
    IterEnd(u32),
    /// iterator next -> v: an async iterator's return() called (v: its result to await, or undefined if it has none);
    /// a throw's (kind register n is 1) errors ignored.
    AsyncIterClose(u32),
    /// obj -> enumerator
    ForIn,
    /// enumerator -> enumerator key, or jumps when done
    ForInNext(u32),
    // -- generators and async functions
    /// A generator's start: it's made, and waits for the first next().
    Generator,
    /// v -> sent value (or a return or throw resumes it so)
    Yield,
    /// iterable -> result: yield* (delegation, the whole loop in the VM)
    YieldStar,
    /// v -> resolved value (or throws the rejection)
    Await,
    /// For an async generator's yield: v -> await v first.
    AsyncYield,
    // -- the rest
    Debugger,
    ImportMeta,
    /// specifier -> promise
    ImportCall,
    /// v -> object (a with's object: ToObject, throwing on null and undefined)
    ToObject,
    /// obj -> obj (throws a TypeError if it's null or undefined: destructuring)
    RequireObject,
    /// obj excluded(n keys on the stack) -> copy: {a, ...rest} (the keys already taken)
    CopyRest(u32),
    /// A function's own name set: f -> f
    SetFunctionName(Atom),
    /// key f -> key f: a computed name given to the function
    SetFunctionNameKey,
    /// The class field initializers run on this (a base class's constructor, at its start).
    RunFields,
    /// -> a new private name (a symbol described by string n)
    PrivateName(u32),
    /// ctor fields -> ctor: the instance fields' initializer given to the constructor.
    SetFields,
    /// home f -> f: a method's home object (for super).
    SetHome,
    /// Declares the global var or function names of global code (its declarations: decls[n]).
    GlobalDecls(u32),
}

/// What kind of function a code is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A script's top level, or eval's, or a module's.
    Script,
    Eval,
    Module,
    Normal,
    Arrow,
    Method,
    Getter,
    Setter,
    /// A class's constructor (derived: it extends another; base: fields run at its start).
    BaseConstructor,
    DerivedConstructor,
    /// A class's field initializers (instance or static), run with this the object.
    Fields,
}

/// A class's facts the VM needs to make it.
#[derive(Debug)]
pub struct ClassInfo {
    pub name: Option<Atom>,
    /// The constructor's code (funcs: index).
    pub constructor: u32,
    pub derived: bool,
    pub start: usize,
    pub end: usize,
}

/// A compiled function (or script).
pub struct Code {
    pub name: Atom,
    pub kind: Kind,
    pub strict: bool,
    pub is_async: bool,
    pub is_generator: bool,
    pub ops: Vec<Op>,
    pub strings: Vec<JsStr>,
    pub bigints: Vec<Rc<BigInt>>,
    pub funcs: Vec<Rc<Code>>,
    pub classes: Vec<ClassInfo>,
    /// Each heap scope's names (and how each was declared: object.rs's VAR, LET, CONST, FUNC_NAME), and whether
    /// it's a function's (or script's) own scope (a sloppy direct eval's vars are declared in it).
    pub scopes: Vec<(Rc<[(Atom, u8)]>, bool)>,
    /// Templates' strings (cooked: None where an escape isn't valid) and raw texts.
    pub templates: Vec<(Vec<Option<JsStr>>, Vec<JsStr>)>,
    pub regexps: Vec<(JsStr, String)>,
    /// finally's jump tables.
    pub jumps: Vec<Vec<u32>>,
    /// Global code's declarations: (name, is a function (its value from funcs: index), is lexical (let/const/class))
    pub decls: Vec<Vec<(Atom, Option<u32>, u8)>>,
    /// Registers (parameters first).
    pub registers: u32,
    pub params: u32,
    /// The function's `length`.
    pub length: u32,
    /// It uses `arguments`, or a rest parameter (so the arguments are kept).
    pub keeps_args: bool,
    /// Its source, and where it is in it (Function.prototype.toString).
    pub source: Option<Rc<str>>,
    pub start: usize,
    pub end: usize,
    /// Lines: (instruction, line) where the line changes.
    pub lines: Vec<(u32, u32)>,
    /// Each template's strings array, once made.
    pub template_objects: RefCell<Vec<Option<Gc>>>,
    /// Mapped arguments objects' parameters' slots (MappedArguments).
    pub arg_maps: Vec<Vec<Option<u16>>>,
}

impl Code {
    pub fn new(name: Atom, kind: Kind) -> Code {
        Code {
            name,
            kind,
            strict: false,
            is_async: false,
            is_generator: false,
            ops: Vec::new(),
            strings: Vec::new(),
            bigints: Vec::new(),
            funcs: Vec::new(),
            classes: Vec::new(),
            scopes: Vec::new(),
            templates: Vec::new(),
            regexps: Vec::new(),
            jumps: Vec::new(),
            decls: Vec::new(),
            registers: 0,
            params: 0,
            length: 0,
            keeps_args: false,
            source: None,
            start: 0,
            end: 0,
            lines: Vec::new(),
            template_objects: RefCell::new(Vec::new()),
            arg_maps: Vec::new(),
        }
    }

    /// The objects it holds (its templates' arrays), for the collector.
    pub fn trace(&self, work: &mut Vec<Gc>) {
        for g in self.template_objects.borrow().iter().flatten() {
            work.push(*g);
        }
        for f in &self.funcs {
            f.trace(work);
        }
    }

    /// The line an instruction is on.
    pub fn line_at(&self, pc: usize) -> u32 {
        match self.lines.binary_search_by(|(at, _)| at.cmp(&(pc as u32))) {
            Ok(i) => self.lines[i].1,
            Err(0) => self.lines.first().map_or(0, |l| l.1),
            Err(i) => self.lines[i - 1].1,
        }
    }
}
