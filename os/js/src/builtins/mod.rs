//! The language's built-in objects, made when a VM starts: the prototypes first (so every function made after has
//! Function.prototype), then each part's constructors and methods, and the global object's properties.

pub mod array;
pub mod collections;
pub mod core;
pub mod date;
pub mod intl;
pub mod iter;
pub mod iterhelp;
pub mod json;
pub mod number;
pub mod promise;
pub mod regexp;
pub mod string;
pub mod typed;

use crate::names as n;
use crate::object::*;
use crate::string::JsStr;
use crate::value::{Gc, Value};
use crate::vm::{I, SYMBOL_NAMES, Sym, Vm};
use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;

/// An argument (undefined if it wasn't given).
pub fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}

/// A function that does nothing: Function.prototype's call.
fn nothing(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> Result<Value, Value> {
    Ok(Value::Undefined)
}

pub fn init(vm: &mut Vm) {
    let op = vm.alloc(None, Class::Ordinary);
    vm.intrinsics[I::ObjectProto as usize] = op;
    let fp = vm.alloc(Some(op), Class::Native(Box::new(Native { f: nothing, construct: None, data: Value::Undefined, data2: Value::Undefined })));
    vm.intrinsics[I::FunctionProto as usize] = fp;
    vm.fn_props(fp, 0, Value::Str(JsStr::empty()));
    let plain = [
        I::StringProto,
        I::NumberProto,
        I::BooleanProto,
        I::SymbolProto,
        I::BigIntProto,
        I::ErrorProto,
        I::IteratorProto,
        I::AsyncIteratorProto,
        I::PromiseProto,
        I::RegExpProto,
        I::DateProto,
        I::MapProto,
        I::SetProto,
        I::WeakMapProto,
        I::WeakSetProto,
        I::WeakRefProto,
        I::FinalizationRegistryProto,
        I::ArrayBufferProto,
        I::SharedArrayBufferProto,
        I::DataViewProto,
        I::TypedArrayProto,
        I::GeneratorFunctionProto,
        I::AsyncFunctionProto,
        I::AsyncGeneratorFunctionProto,
    ];
    for i in plain {
        let parent = match i {
            I::GeneratorFunctionProto | I::AsyncFunctionProto | I::AsyncGeneratorFunctionProto => fp,
            _ => op,
        };
        vm.intrinsics[i as usize] = vm.alloc(Some(parent), Class::Ordinary);
    }
    // Some prototypes are objects of their kind.
    vm.heap.get_mut(vm.proto(I::StringProto)).class = Class::String(JsStr::empty());
    vm.heap.get_mut(vm.proto(I::NumberProto)).class = Class::Number(0.0);
    vm.heap.get_mut(vm.proto(I::BooleanProto)).class = Class::Boolean(false);
    let ap = vm.alloc(Some(op), Class::Array(Vec::new(), 0));
    vm.intrinsics[I::ArrayProto as usize] = ap;
    let ep = vm.proto(I::ErrorProto);
    for i in [I::EvalErrorProto, I::RangeErrorProto, I::ReferenceErrorProto, I::SyntaxErrorProto, I::TypeErrorProto, I::UriErrorProto, I::AggregateErrorProto] {
        vm.intrinsics[i as usize] = vm.alloc(Some(ep), Class::Ordinary);
    }
    let ip = vm.proto(I::IteratorProto);
    for i in [I::ArrayIterProto, I::MapIterProto, I::SetIterProto, I::StringIterProto, I::RegExpStringIterProto, I::GeneratorProto] {
        vm.intrinsics[i as usize] = vm.alloc(Some(ip), Class::Ordinary);
    }
    let aip = vm.proto(I::AsyncIteratorProto);
    for i in [I::AsyncGeneratorProto, I::AsyncFromSyncIterProto] {
        vm.intrinsics[i as usize] = vm.alloc(Some(aip), Class::Ordinary);
    }
    let tp = vm.proto(I::TypedArrayProto);
    for i in [I::Int8ArrayProto, I::Uint8ArrayProto, I::Uint8ClampedArrayProto, I::Int16ArrayProto, I::Uint16ArrayProto, I::Int32ArrayProto, I::Uint32ArrayProto, I::Float32ArrayProto, I::Float64ArrayProto, I::BigInt64ArrayProto, I::BigUint64ArrayProto] {
        vm.intrinsics[i as usize] = vm.alloc(Some(tp), Class::Ordinary);
    }
    for name in SYMBOL_NAMES {
        let s = vm.new_symbol(Some(JsStr::from_str(&format!("Symbol.{name}"))), 0);
        vm.symbols.push(s);
    }
    vm.global = vm.alloc(Some(op), Class::Ordinary);
    let g = vm.global;
    vm.put(g, "globalThis", Value::Obj(g), HIDDEN);

    core::init(vm);
    iter::init(vm);
    iterhelp::init(vm);
    array::init(vm);
    string::init(vm);
    number::init(vm);
    json::init(vm);
    promise::init(vm);
    collections::init(vm);
    regexp::init(vm);
    date::init(vm);
    typed::init(vm);
    intl::init(vm);
}

/// A constructor: its function, prototype linked both ways, and a global of its name.
pub fn constructor(vm: &mut Vm, name: &str, length: u32, call: NativeFn, construct: Option<NativeFn>, proto: Option<Gc>) -> Gc {
    let c = vm.native_with(name, length, call, construct, Value::Undefined);
    if let Some(p) = proto {
        vm.put_key(c, Key::Str(n::PROTOTYPE), Value::Obj(p), 0);
        vm.put_key(p, Key::Str(n::CONSTRUCTOR), Value::Obj(c), HIDDEN);
    }
    let g = vm.global;
    vm.put(g, name, Value::Obj(c), HIDDEN);
    c
}

/// A Symbol.toStringTag property.
pub fn tag(vm: &mut Vm, obj: Gc, name: &str) {
    let t = vm.sym(Sym::ToStringTag);
    vm.put_key(obj, Key::Sym(t), Value::str(name), CONFIGURABLE);
}

/// A getter whose key is a well-known symbol.
pub fn sym_getter(vm: &mut Vm, obj: Gc, s: Sym, name: &str, f: NativeFn) {
    let k = Key::Sym(vm.sym(s));
    vm.accessor(obj, k, name, Some(f), None);
}

/// A method whose key is a well-known symbol.
pub fn sym_method(vm: &mut Vm, obj: Gc, s: Sym, name: &str, length: u32, f: NativeFn) -> Gc {
    let g = vm.native(name, length, f);
    let k = Key::Sym(vm.sym(s));
    vm.put_key(obj, k, Value::Obj(g), HIDDEN);
    g
}

/// A getter (non-enumerable, configurable) by name.
pub fn getter(vm: &mut Vm, obj: Gc, name: &str, f: NativeFn) {
    let a = vm.atom(name);
    vm.accessor(obj, Key::Str(a), name, Some(f), None);
}

/// `this` as an object of the class `test` matches, or a TypeError.
pub fn this_class<T>(vm: &mut Vm, this: &Value, what: &str, test: impl Fn(&Class) -> Option<T>) -> Result<T, Value> {
    if let Value::Obj(g) = this
        && let Some(t) = test(&vm.heap.get(*g).class)
    {
        return Ok(t);
    }
    Err(vm.type_error(&format!("{what} called on an incompatible receiver")))
}

/// `species` getter: returns this.
pub fn species_getter(_vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> Result<Value, Value> {
    Ok(this.clone())
}

/// A callback argument that must be a function.
pub fn callable_arg(vm: &mut Vm, args: &[Value], i: usize) -> Result<Value, Value> {
    let f = arg(args, i);
    if !vm.is_callable(&f) {
        return Err(vm.type_error(&format!("{} is not a function", vm.describe(&f))));
    }
    Ok(f)
}

/// The `name` used for a value in messages.
pub fn js_str(s: &str) -> Value {
    Value::Str(JsStr::from_str(s))
}
