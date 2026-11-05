//! Objects and the heap they live in. An object has a prototype, its own properties (an ordered map: a key's place
//! is where it was first added, as the language orders them; indexed once they're many), whether more may be added,
//! and what kind of object it is (`Class`: an array's elements, a function's code and scope, a date's time, ...).
//! The heap names each object by number (`Gc`); the collector marks what's reachable and frees the rest.

use crate::bigint::BigInt;
use crate::code::Code;
use crate::string::{Atom, JsStr};
use crate::value::{Gc, Value};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;

/// A property's key: a name (interned), an array index (a canonical number below 2^32 - 1), or a symbol.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Key {
    Str(Atom),
    Index(u32),
    Sym(Gc),
}

pub const WRITABLE: u8 = 1;
pub const ENUMERABLE: u8 = 2;
pub const CONFIGURABLE: u8 = 4;
/// Writable, enumerable and configurable: what an assignment makes.
pub const ALL: u8 = 7;
/// Writable and configurable, not enumerable: the built-ins' methods.
pub const HIDDEN: u8 = WRITABLE | CONFIGURABLE;

#[derive(Clone, Debug)]
pub enum Slot {
    Data(Value),
    /// A getter and a setter (Undefined: none).
    Accessor(Value, Value),
}

#[derive(Clone, Debug)]
pub struct Prop {
    pub slot: Slot,
    pub flags: u8,
}

impl Prop {
    pub fn data(v: Value, flags: u8) -> Prop {
        Prop { slot: Slot::Data(v), flags }
    }

    pub fn writable(&self) -> bool {
        self.flags & WRITABLE != 0
    }

    pub fn enumerable(&self) -> bool {
        self.flags & ENUMERABLE != 0
    }

    pub fn configurable(&self) -> bool {
        self.flags & CONFIGURABLE != 0
    }
}

/// An object's own properties, in the order they were added.
#[derive(Default, Clone, Debug)]
pub struct PropMap {
    entries: Vec<(Key, Prop)>,
    /// Open addressing, once there are many: each slot an entry's place + 1 (0: empty).
    index: Vec<u32>,
}

const INDEXED_FROM: usize = 12;

fn hash_key(k: &Key) -> u32 {
    let (tag, v) = match k {
        Key::Str(a) => (1u32, a.0),
        Key::Index(i) => (2, *i),
        Key::Sym(g) => (3, g.0),
    };
    (v ^ tag.wrapping_mul(0x9E37_79B9)).wrapping_mul(0x85EB_CA6B) ^ (v >> 13)
}

impl PropMap {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn find(&self, k: &Key) -> Option<usize> {
        if self.index.is_empty() {
            return self.entries.iter().position(|(e, _)| e == k);
        }
        let mask = self.index.len() - 1;
        let mut i = hash_key(k) as usize & mask;
        loop {
            match self.index[i] {
                0 => return None,
                at if self.entries[at as usize - 1].0 == *k => return Some(at as usize - 1),
                _ => i = (i + 1) & mask,
            }
        }
    }

    fn rebuild(&mut self) {
        if self.entries.len() < INDEXED_FROM {
            self.index.clear();
            return;
        }
        let size = (self.entries.len() * 2).next_power_of_two();
        self.index = alloc::vec![0; size];
        for (n, (k, _)) in self.entries.iter().enumerate() {
            let mut i = hash_key(k) as usize & (size - 1);
            while self.index[i] != 0 {
                i = (i + 1) & (size - 1);
            }
            self.index[i] = n as u32 + 1;
        }
    }

    pub fn get(&self, k: &Key) -> Option<&Prop> {
        self.find(k).map(|i| &self.entries[i].1)
    }

    pub fn get_mut(&mut self, k: &Key) -> Option<&mut Prop> {
        self.find(k).map(move |i| &mut self.entries[i].1)
    }

    /// Sets a property (added at the end if it's new).
    pub fn set(&mut self, k: Key, p: Prop) {
        if let Some(i) = self.find(&k) {
            self.entries[i].1 = p;
            return;
        }
        self.entries.push((k, p));
        if self.entries.len() == INDEXED_FROM || (!self.index.is_empty() && self.entries.len() * 2 > self.index.len()) {
            self.rebuild();
        } else if !self.index.is_empty() {
            let mask = self.index.len() - 1;
            let mut i = hash_key(&k) as usize & mask;
            while self.index[i] != 0 {
                i = (i + 1) & mask;
            }
            self.index[i] = self.entries.len() as u32;
        }
    }

    pub fn remove(&mut self, k: &Key) -> Option<Prop> {
        let i = self.find(k)?;
        let (_, p) = self.entries.remove(i);
        self.rebuild();
        Some(p)
    }

    /// The keys, in the language's order: indices ascending, then names and then symbols as they were added.
    pub fn keys(&self) -> Vec<Key> {
        let mut idx: Vec<u32> = self.entries.iter().filter_map(|(k, _)| if let Key::Index(i) = k { Some(*i) } else { None }).collect();
        idx.sort_unstable();
        let mut out: Vec<Key> = idx.into_iter().map(Key::Index).collect();
        out.extend(self.entries.iter().filter(|(k, _)| matches!(k, Key::Str(_))).map(|(k, _)| *k));
        out.extend(self.entries.iter().filter(|(k, _)| matches!(k, Key::Sym(_))).map(|(k, _)| *k));
        out
    }

    pub fn iter(&self) -> impl Iterator<Item = &(Key, Prop)> {
        self.entries.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut (Key, Prop)> {
        self.entries.iter_mut()
    }
}

/// A built-in function: called with `this` and the arguments (where they are on the VM's stack), and the function
/// object itself (for its data); Err is an exception thrown.
pub type NativeFn = fn(&mut crate::vm::Vm, &Value, &[Value], Gc) -> Result<Value, Value>;

/// A function written in JavaScript, made (a closure): its code, the scope it was made in, and what it carries
/// from there.
pub struct Closure {
    pub code: Rc<Code>,
    pub env: Option<Gc>,
    /// An arrow function's `this`, `new.target` and the function it's inside (its super's home): from where it was
    /// made.
    pub this: Value,
    pub new_target: Value,
    /// A method's object (for super), and a class constructor's field initializer (run on `this` once it's made).
    pub home: Option<Gc>,
    pub fields: Option<Gc>,
    /// A class's private names' environment (the brand's object), for its methods' #x.
    pub privates: Option<Gc>,
}

pub struct Native {
    pub f: NativeFn,
    /// What `new` does with it (None: it isn't a constructor).
    pub construct: Option<NativeFn>,
    /// Data for the function (a resolve function's promise, a bound value).
    pub data: Value,
    pub data2: Value,
}

pub struct Bound {
    pub target: Gc,
    pub this: Value,
    pub args: Vec<Value>,
}

/// A sloppy function's arguments object's elements that are its parameters: the environment, and each element's
/// slot in it (None once unmapped).
pub struct ArgMap {
    pub env: Gc,
    pub slots: Vec<Option<u16>>,
}

/// A scope's variables that inner functions use (each scope with any gets one as it's entered).
pub struct Env {
    pub vars: Vec<Value>,
    pub parent: Option<Gc>,
    /// The scope's names, for code that looks names up as it runs (a direct eval's, a with's), and which are
    /// constants (an assignment to one throws).
    pub names: Rc<[(Atom, u8)]>,
    /// A `with` statement's object (its properties are the names).
    pub with: Option<Gc>,
    /// A function's own scope (or a script's): a sloppy direct eval's vars are declared in it.
    pub var_scope: bool,
}

pub const SYM_REGISTERED: u8 = 1;
pub const SYM_PRIVATE: u8 = 2;

/// How a scope's name is declared (Env::names).
pub const VAR: u8 = 0;
pub const LET: u8 = 1;
pub const CONST: u8 = 2;
/// A function's own name in a named function expression: assigning to it does nothing (throws in strict code).
pub const FUNC_NAME: u8 = 3;

/// The entries of a Map or Set, in the order they were added (deleted ones left as None, so iterators going over it
/// keep their place), with an index from key to place.
#[derive(Default)]
pub struct MapData {
    pub entries: Vec<Option<(Value, Value)>>,
    pub index: Vec<(u64, u32)>,
    pub size: usize,
}

pub enum PromiseState {
    Pending,
    Fulfilled(Value),
    Rejected(Value),
}

pub struct PromiseData {
    pub state: PromiseState,
    /// Reactions waiting: (on fulfilled, on rejected, the derived promise's capability (promise, resolve, reject)).
    pub reactions: Vec<(Value, Value, Value, Value, Value)>,
    pub handled: bool,
}

/// A generator's frame and its part of the stack, set aside between `next`s (or an async function's, between
/// awaits).
pub struct GenState {
    pub state: GenRun,
    pub frame: Option<Box<crate::vm::Frame>>,
    pub stack: Vec<Value>,
    /// vm.rs's GEN_SYNC, GEN_ASYNC_FN, GEN_ASYNC.
    pub kind: u8,
    /// An async function's promise.
    pub promise: Value,
    /// An async generator's requests waiting: (0 next, 1 throw, 2 return; the value; the promise to settle).
    pub queue: Vec<(u8, Value, Value)>,
    /// An async yield* awaiting its inner iterator's result (3; 4: a return's).
    pub star: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GenRun {
    /// Made, not yet started.
    Start,
    Suspended,
    Running,
    Done,
}

pub struct RegExpData {
    pub source: JsStr,
    pub flags: alloc::string::String,
    pub program: Rc<crate::regexp::Program>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ElemKind {
    Int8,
    Uint8,
    Uint8Clamped,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float32,
    Float64,
    BigInt64,
    BigUint64,
}

impl ElemKind {
    pub fn size(self) -> usize {
        match self {
            ElemKind::Int8 | ElemKind::Uint8 | ElemKind::Uint8Clamped => 1,
            ElemKind::Int16 | ElemKind::Uint16 => 2,
            ElemKind::Int32 | ElemKind::Uint32 | ElemKind::Float32 => 4,
            ElemKind::Float64 | ElemKind::BigInt64 | ElemKind::BigUint64 => 8,
        }
    }
}

pub struct TypedArray {
    pub kind: ElemKind,
    pub buffer: Gc,
    pub offset: usize,
    /// Its length in elements (None: as long as the buffer goes, for a resizable one).
    pub length: Option<usize>,
}

pub enum Class {
    Ordinary,
    /// An array's elements (Empty: a hole) and its length (more than the elements when it ends in holes).
    Array(Vec<Value>, u32),
    Function(Box<Closure>),
    Native(Box<Native>),
    Bound(Box<Bound>),
    Error,
    Boolean(bool),
    Number(f64),
    String(JsStr),
    SymbolObj(Gc),
    BigIntObj(Rc<BigInt>),
    /// A symbol itself: its description, and what kind (SYM_REGISTERED: Symbol.for's, its key the description;
    /// SYM_PRIVATE: a class's private name, never seen by scripts as a key).
    Symbol(Option<JsStr>, u8),
    Date(f64),
    RegExp(Box<RegExpData>),
    Map(Box<MapData>),
    Set(Box<MapData>),
    WeakMap(Box<MapData>),
    WeakSet(Box<MapData>),
    WeakRef(Value),
    Promise(Box<PromiseData>),
    /// A proxy's target and handler (None once it's revoked).
    /// A proxy's target and handler (None once revoked), and whether it's callable (1) and a constructor (2).
    Proxy(Option<(Gc, Gc)>, u8),
    ArrayBuffer(Option<Vec<u8>>, Option<usize>),
    TypedArray(Box<TypedArray>),
    DataView(Gc, usize, Option<usize>),
    /// An arguments object (its elements are its own properties); whether it's a strict function's.
    Arguments(Option<Box<ArgMap>>),
    Generator(Box<GenState>),
    Env(Box<Env>),
    /// An iterator over an array's (or array-like's) elements: the object, where it's got, and what it gives
    /// (0 keys, 1 values, 2 entries).
    ArrayIter(Value, u32, u8),
    /// An iterator over a Map's or Set's entries: the map, where it's got, what it gives.
    MapIter(Value, u32, u8),
    StringIter(JsStr, u32),
    /// for-in's walk: the keys still to give, and the object (each key is given only if it's still there).
    ForIn(Vec<Key>, Value),
    RegExpStringIter(Value, JsStr, bool, bool, bool),
    /// An object of the host's (the browser's DOM, its window), with its kind and number.
    Host(u32, u64),
    /// A module's namespace object.
    Namespace,
    /// An async-from-sync iterator (for await over a sync iterator): the iterator, and its next method.
    AsyncFromSync(Value, Value),
}

pub struct Object {
    pub proto: Option<Gc>,
    pub props: PropMap,
    pub class: Class,
    pub extensible: bool,
    marked: bool,
}

impl Object {
    pub fn new(proto: Option<Gc>, class: Class) -> Object {
        Object { proto, props: PropMap::default(), class, extensible: true, marked: false }
    }
}

/// Every object, by number; those freed are reused.
#[derive(Default)]
pub struct Heap {
    slots: Vec<Option<Object>>,
    free: Vec<u32>,
    /// Objects made since the last collection, and how many live after it.
    pub made: usize,
    pub live: usize,
}

impl Heap {
    pub fn alloc(&mut self, o: Object) -> Gc {
        self.made += 1;
        if let Some(i) = self.free.pop() {
            self.slots[i as usize] = Some(o);
            return Gc(i);
        }
        self.slots.push(Some(o));
        Gc(self.slots.len() as u32 - 1)
    }

    pub fn get(&self, g: Gc) -> &Object {
        self.slots[g.0 as usize].as_ref().expect("an object that was collected")
    }

    pub fn get_mut(&mut self, g: Gc) -> &mut Object {
        self.slots[g.0 as usize].as_mut().expect("an object that was collected")
    }

    pub fn count(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    /// Marks everything reachable from `roots`, then frees the rest; how many were freed.
    pub fn collect(&mut self, roots: &[Value], more: &[Gc]) -> usize {
        let mut work: Vec<Gc> = Vec::new();
        for v in roots {
            if let Value::Obj(g) | Value::Sym(g) = v {
                work.push(*g);
            }
        }
        work.extend_from_slice(more);
        while let Some(g) = work.pop() {
            let Some(o) = self.slots.get_mut(g.0 as usize).and_then(|s| s.as_mut()) else { continue };
            if o.marked {
                continue;
            }
            o.marked = true;
            trace(o, &mut work);
        }
        // Weak maps' entries whose keys are gone go too; a WeakRef whose target is gone empties.
        let marked = |slots: &Vec<Option<Object>>, v: &Value| match v {
            Value::Obj(g) | Value::Sym(g) => slots.get(g.0 as usize).and_then(|s| s.as_ref()).is_some_and(|o| o.marked),
            _ => true,
        };
        let weak: Vec<usize> = (0..self.slots.len())
            .filter(|&i| self.slots[i].as_ref().is_some_and(|o| o.marked && matches!(o.class, Class::WeakMap(_) | Class::WeakSet(_) | Class::WeakRef(_))))
            .collect();
        for i in weak {
            let dead: Vec<bool> = match &self.slots[i].as_ref().map(|o| &o.class) {
                Some(Class::WeakMap(m) | Class::WeakSet(m)) => m.entries.iter().map(|e| e.as_ref().is_some_and(|(k, _)| !marked(&self.slots, k))).collect(),
                Some(Class::WeakRef(v)) => alloc::vec![!marked(&self.slots, v)],
                _ => Vec::new(),
            };
            if let Some(o) = self.slots[i].as_mut() {
                match &mut o.class {
                    Class::WeakMap(m) | Class::WeakSet(m) => {
                        for (e, d) in m.entries.iter_mut().zip(dead) {
                            if d {
                                *e = None;
                                m.size -= 1;
                            }
                        }
                        m.index.retain(|(_, at)| m.entries[*at as usize].is_some());
                    }
                    Class::WeakRef(v) if dead.first() == Some(&true) => *v = Value::Undefined,
                    _ => {}
                }
            }
        }
        let mut freed = 0;
        for (i, s) in self.slots.iter_mut().enumerate() {
            match s {
                Some(o) if o.marked => o.marked = false,
                Some(_) => {
                    *s = None;
                    self.free.push(i as u32);
                    freed += 1;
                }
                None => {}
            }
        }
        self.live = self.count();
        self.made = 0;
        freed
    }
}

/// What an object holds on to, for the collector.
fn trace(o: &Object, work: &mut Vec<Gc>) {
    let mut v = |x: &Value| {
        if let Value::Obj(g) | Value::Sym(g) = x {
            work.push(*g);
        }
    };
    if let Some(p) = o.proto {
        v(&Value::Obj(p));
    }
    for (k, p) in o.props.iter() {
        if let Key::Sym(g) = k {
            v(&Value::Sym(*g));
        }
        match &p.slot {
            Slot::Data(x) => v(x),
            Slot::Accessor(g, s) => {
                v(g);
                v(s);
            }
        }
    }
    let mut g = |x: Option<Gc>| {
        if let Some(x) = x {
            work.push(x);
        }
    };
    match &o.class {
        Class::Array(elems, _) => {
            for e in elems {
                if let Value::Obj(x) | Value::Sym(x) = e {
                    work.push(*x);
                }
            }
        }
        Class::Function(c) => {
            g(c.env);
            g(c.home);
            g(c.fields);
            g(c.privates);
            for x in [&c.this, &c.new_target] {
                if let Value::Obj(x) = x {
                    work.push(*x);
                }
            }
            c.code.trace(work);
        }
        Class::Native(n) => {
            for x in [&n.data, &n.data2] {
                if let Value::Obj(x) | Value::Sym(x) = x {
                    work.push(*x);
                }
            }
        }
        Class::Bound(b) => {
            work.push(b.target);
            for x in core::iter::once(&b.this).chain(b.args.iter()) {
                if let Value::Obj(x) | Value::Sym(x) = x {
                    work.push(*x);
                }
            }
        }
        Class::SymbolObj(s) => work.push(*s),
        Class::Map(m) | Class::Set(m) => {
            for (k, val) in m.entries.iter().flatten() {
                for x in [k, val] {
                    if let Value::Obj(x) | Value::Sym(x) = x {
                        work.push(*x);
                    }
                }
            }
        }
        // (A weak map's values are kept while their keys are: marked after, with the weak entries: simply kept here.)
        Class::WeakMap(m) => {
            for (_, val) in m.entries.iter().flatten() {
                if let Value::Obj(x) | Value::Sym(x) = val {
                    work.push(*x);
                }
            }
        }
        Class::Promise(p) => {
            match &p.state {
                PromiseState::Fulfilled(x) | PromiseState::Rejected(x) => {
                    if let Value::Obj(x) | Value::Sym(x) = x {
                        work.push(*x);
                    }
                }
                PromiseState::Pending => {}
            }
            for r in &p.reactions {
                for x in [&r.0, &r.1, &r.2, &r.3, &r.4] {
                    if let Value::Obj(x) = x {
                        work.push(*x);
                    }
                }
            }
        }
        Class::Proxy(Some((t, h)), _) => {
            work.push(*t);
            work.push(*h);
        }
        Class::TypedArray(t) => work.push(t.buffer),
        Class::DataView(b, _, _) => work.push(*b),
        Class::Generator(gs) => {
            for x in &gs.stack {
                if let Value::Obj(x) | Value::Sym(x) = x {
                    work.push(*x);
                }
            }
            if let Some(f) = &gs.frame {
                f.trace(work);
            }
            for x in core::iter::once(&gs.promise).chain(gs.queue.iter().flat_map(|q| [&q.1, &q.2])) {
                if let Value::Obj(x) | Value::Sym(x) = x {
                    work.push(*x);
                }
            }
        }
        Class::Arguments(Some(m)) => work.push(m.env),
        Class::Env(e) => {
            g(e.parent);
            g(e.with);
            for x in &e.vars {
                if let Value::Obj(x) | Value::Sym(x) = x {
                    work.push(*x);
                }
            }
        }
        Class::ArrayIter(x, _, _) | Class::MapIter(x, _, _) | Class::ForIn(_, x) | Class::RegExpStringIter(x, ..) => {
            if let Value::Obj(x) = x {
                work.push(*x);
            }
            if let Class::ForIn(keys, _) = &o.class {
                for k in keys {
                    if let Key::Sym(s) = k {
                        work.push(*s);
                    }
                }
            }
        }
        Class::AsyncFromSync(a, b) => {
            for x in [a, b] {
                if let Value::Obj(x) = x {
                    work.push(*x);
                }
            }
        }
        _ => {}
    }
}
