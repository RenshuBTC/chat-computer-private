//! Array: the constructor (isArray, of, from) and Array.prototype's methods, written as the language specifies them
//! for any array-like object, with quick ways for the engine's own arrays where nothing can tell the difference.

use super::{arg, callable_arg, constructor, species_getter, sym_getter};
use crate::builtins::iter::array_iterator;
use crate::names as n;
use crate::object::*;
use crate::string::JsStr;
use crate::value::{Gc, Value, same_value_zero, strict_equals};
use crate::vm::{I, Sym, Vm};
use alloc::format;
use alloc::vec::Vec;

type R = Result<Value, Value>;

const MAX_SAFE: u64 = 9007199254740991;

pub fn init(vm: &mut Vm) {
    let ap = vm.proto(I::ArrayProto);
    let c = constructor(vm, "Array", 1, array_call, Some(array_construct), Some(ap));
    vm.intrinsics[I::ArrayCtor as usize] = c;
    vm.method(c, "isArray", 1, array_is_array);
    vm.method(c, "of", 0, array_of);
    vm.method(c, "from", 1, array_from);
    sym_getter(vm, c, Sym::Species, "[Symbol.species]", species_getter);
    let methods: [(&str, u32, NativeFn); 38] = [
        ("at", 1, at),
        ("concat", 1, concat),
        ("copyWithin", 2, copy_within),
        ("entries", 0, entries),
        ("every", 1, every),
        ("fill", 1, fill),
        ("filter", 1, filter),
        ("find", 1, find),
        ("findIndex", 1, find_index),
        ("findLast", 1, find_last),
        ("findLastIndex", 1, find_last_index),
        ("flat", 0, flat),
        ("flatMap", 1, flat_map),
        ("forEach", 1, for_each),
        ("includes", 1, includes),
        ("indexOf", 1, index_of),
        ("join", 1, join),
        ("keys", 0, keys),
        ("lastIndexOf", 1, last_index_of),
        ("map", 1, map),
        ("pop", 0, pop),
        ("push", 1, push),
        ("reduce", 1, reduce),
        ("reduceRight", 1, reduce_right),
        ("reverse", 0, reverse),
        ("shift", 0, shift),
        ("slice", 2, slice),
        ("some", 1, some),
        ("sort", 1, sort),
        ("splice", 2, splice),
        ("toLocaleString", 0, to_locale_string),
        ("toReversed", 0, to_reversed),
        ("toSorted", 1, to_sorted),
        ("toSpliced", 2, to_spliced),
        ("toString", 0, to_string),
        ("unshift", 1, unshift),
        ("values", 0, values),
        ("with", 2, with),
    ];
    for (name, len, f) in methods {
        let g = vm.method(ap, name, len, f);
        if name == "values" {
            vm.intrinsics[I::ArrayValues as usize] = g;
            let k = Key::Sym(vm.sym(Sym::Iterator));
            vm.put_key(ap, k, Value::Obj(g), HIDDEN);
        }
    }
    let un = vm.alloc(None, Class::Ordinary);
    for name in ["at", "copyWithin", "entries", "fill", "find", "findIndex", "findLast", "findLastIndex", "flat", "flatMap", "includes", "keys", "toReversed", "toSorted", "toSpliced", "values"] {
        vm.put(un, name, Value::Bool(true), ALL);
    }
    let k = Key::Sym(vm.sym(Sym::Unscopables));
    vm.put_key(ap, k, Value::Obj(un), CONFIGURABLE);
}

// --------------------------------------------------------------------------------------------------- helpers

/// The key of an index (as a name past 2^32 - 2).
pub fn index_key(vm: &mut Vm, i: u64) -> Key {
    if i < u32::MAX as u64 {
        Key::Index(i as u32)
    } else {
        let s = JsStr::from_str(&format!("{i}"));
        vm.key_from_str(&s)
    }
}

pub fn get_i(vm: &mut Vm, o: Gc, i: u64) -> R {
    let k = index_key(vm, i);
    vm.get(o, k, &Value::Obj(o))
}

pub fn has_i(vm: &mut Vm, o: Gc, i: u64) -> Result<bool, Value> {
    let k = index_key(vm, i);
    vm.has_property(o, k)
}

pub fn set_i(vm: &mut Vm, o: Gc, i: u64, v: Value) -> Result<(), Value> {
    let k = index_key(vm, i);
    if !vm.set(o, k, v, &Value::Obj(o))? {
        return Err(vm.type_error(&format!("Cannot assign to read only property '{i}' of object")));
    }
    Ok(())
}

pub fn create_i(vm: &mut Vm, o: Gc, i: u64, v: Value) -> Result<(), Value> {
    let k = index_key(vm, i);
    vm.create_data_property_or_throw(o, k, v)
}

fn delete_i(vm: &mut Vm, o: Gc, i: u64) -> Result<(), Value> {
    let k = index_key(vm, i);
    if !vm.delete_property(o, k)? {
        return Err(vm.type_error(&format!("Cannot delete property '{i}'")));
    }
    Ok(())
}

pub fn set_len(vm: &mut Vm, o: Gc, len: u64) -> Result<(), Value> {
    if !vm.set(o, Key::Str(n::LENGTH), Value::Num(len as f64), &Value::Obj(o))? {
        return Err(vm.type_error("Cannot assign to read only property 'length'"));
    }
    Ok(())
}

/// An engine array with nothing unusual: elements only in its vector, Array.prototype its prototype.
fn plain(vm: &Vm, o: Gc) -> bool {
    let ob = vm.heap.get(o);
    let (ap, op) = (vm.proto(I::ArrayProto), vm.proto(I::ObjectProto));
    matches!(ob.class, Class::Array(..))
        && ob.extensible
        && !vm.proto_indexes
        && ob.proto == Some(ap)
        && vm.heap.get(ap).proto == Some(op)
        && !ob.props.iter().any(|(k, _)| matches!(k, Key::Index(_)) || *k == Key::Str(n::LENGTH))
}

/// ArraySpeciesCreate.
fn species_create(vm: &mut Vm, original: Gc, len: u64) -> Result<Gc, Value> {
    if !vm.is_array(&Value::Obj(original))? {
        return array_create(vm, len, None);
    }
    let mut c = vm.get(original, Key::Str(n::CONSTRUCTOR), &Value::Obj(original))?;
    if let Value::Obj(cg) = c {
        let sp = vm.sym(Sym::Species);
        c = vm.get(cg, Key::Sym(sp), &Value::Obj(cg))?;
        if matches!(c, Value::Null) {
            c = Value::Undefined;
        }
    }
    if c.is_undefined() {
        return array_create(vm, len, None);
    }
    if !vm.is_constructor(&c) {
        return Err(vm.type_error("object.constructor[Symbol.species] is not a constructor"));
    }
    match vm.construct(&c, &[Value::Num(len as f64)], &c)? {
        Value::Obj(g) => Ok(g),
        _ => Err(vm.type_error("not an object")),
    }
}

/// ArrayCreate (a length, with an optional prototype).
pub fn array_create(vm: &mut Vm, len: u64, proto: Option<Gc>) -> Result<Gc, Value> {
    if len > u32::MAX as u64 {
        return Err(vm.range_error("Invalid array length"));
    }
    let p = proto.unwrap_or(vm.proto(I::ArrayProto));
    Ok(vm.alloc(Some(p), Class::Array(Vec::new(), len as u32)))
}

// ------------------------------------------------------------------------------------------------ constructor

fn array_call(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    array_construct(vm, &Value::Obj(f), args, f)
}

fn array_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let proto = vm.proto_from_constructor(nt, I::ArrayProto)?;
    if args.len() == 1 {
        if let Value::Num(x) = args[0] {
            let len = crate::value::to_uint32(x);
            if len as f64 != x {
                return Err(vm.range_error("Invalid array length"));
            }
            return Ok(Value::Obj(array_create(vm, len as u64, Some(proto))?));
        }
        let a = vm.alloc(Some(proto), Class::Array(alloc::vec![args[0].clone()], 1));
        return Ok(Value::Obj(a));
    }
    let a = vm.alloc(Some(proto), Class::Array(args.to_vec(), args.len() as u32));
    Ok(Value::Obj(a))
}

fn array_is_array(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(vm.is_array(&arg(args, 0))?))
}

fn array_of(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let a = if vm.is_constructor(this) {
        match vm.construct(this, &[Value::Num(args.len() as f64)], this)? {
            Value::Obj(g) => g,
            _ => return Err(vm.type_error("not an object")),
        }
    } else {
        array_create(vm, args.len() as u64, None)?
    };
    for (i, v) in args.iter().enumerate() {
        create_i(vm, a, i as u64, v.clone())?;
    }
    set_len(vm, a, args.len() as u64)?;
    Ok(Value::Obj(a))
}

fn array_from(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let items = arg(args, 0);
    let map_fn = arg(args, 1);
    let this_arg = arg(args, 2);
    let mapping = !map_fn.is_undefined();
    if mapping && !vm.is_callable(&map_fn) {
        return Err(vm.type_error("Array.from: when provided, the second argument must be a function"));
    }
    let it_sym = vm.sym(Sym::Iterator);
    let using = vm.get_method(&items, Key::Sym(it_sym))?;
    if !using.is_undefined() {
        let a = if vm.is_constructor(this) {
            match vm.construct(this, &[], this)? {
                Value::Obj(g) => g,
                _ => return Err(vm.type_error("not an object")),
            }
        } else {
            array_create(vm, 0, None)?
        };
        let (it, next) = vm.get_iterator(&items, false)?;
        let mut k = 0u64;
        loop {
            let Some(v) = vm.iter_step(&it, &next)? else { break };
            let r = (|| -> Result<(), Value> {
                let v = if mapping { vm.call(&map_fn, this_arg.clone(), &[v, Value::Num(k as f64)])? } else { v };
                create_i(vm, a, k, v)
            })();
            if let Err(e) = r {
                let _ = vm.iterator_close(&it);
                return Err(e);
            }
            k += 1;
        }
        set_len(vm, a, k)?;
        return Ok(Value::Obj(a));
    }
    let o = vm.to_object(&items)?;
    let len = vm.length_of(o)?;
    let a = if vm.is_constructor(this) {
        match vm.construct(this, &[Value::Num(len as f64)], this)? {
            Value::Obj(g) => g,
            _ => return Err(vm.type_error("not an object")),
        }
    } else {
        array_create(vm, len, None)?
    };
    for k in 0..len {
        let v = get_i(vm, o, k)?;
        let v = if mapping { vm.call(&map_fn, this_arg.clone(), &[v, Value::Num(k as f64)])? } else { v };
        create_i(vm, a, k, v)?;
    }
    set_len(vm, a, len)?;
    Ok(Value::Obj(a))
}

// ------------------------------------------------------------------------------------------------- the methods

fn obj_len(vm: &mut Vm, this: &Value) -> Result<(Gc, u64), Value> {
    let o = vm.to_object(this)?;
    let len = vm.length_of(o)?;
    Ok((o, len))
}

fn at(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    let rel = vm.to_integer(&arg(args, 0))?;
    let k = if rel >= 0.0 { rel } else { len as f64 + rel };
    if k < 0.0 || k >= len as f64 {
        return Ok(Value::Undefined);
    }
    get_i(vm, o, k as u64)
}

fn is_spreadable(vm: &mut Vm, v: &Value) -> Result<bool, Value> {
    let Value::Obj(g) = v else { return Ok(false) };
    let s = vm.sym(Sym::IsConcatSpreadable);
    let sv = vm.get(*g, Key::Sym(s), v)?;
    if !sv.is_undefined() {
        return Ok(sv.truthy());
    }
    vm.is_array(v)
}

fn concat(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(this)?;
    let a = species_create(vm, o, 0)?;
    let mut n = 0u64;
    let items = core::iter::once(Value::Obj(o)).chain(args.iter().cloned());
    for e in items {
        if is_spreadable(vm, &e)? {
            let Value::Obj(eg) = e else { continue };
            let len = vm.length_of(eg)?;
            if n + len > MAX_SAFE {
                return Err(vm.type_error("Array too long"));
            }
            for k in 0..len {
                if has_i(vm, eg, k)? {
                    let v = get_i(vm, eg, k)?;
                    create_i(vm, a, n, v)?;
                }
                n += 1;
            }
        } else {
            if n >= MAX_SAFE {
                return Err(vm.type_error("Array too long"));
            }
            create_i(vm, a, n, e)?;
            n += 1;
        }
    }
    set_len(vm, a, n)?;
    Ok(Value::Obj(a))
}

fn copy_within(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    let to = vm.relative_index(&arg(args, 0), len, 0)?;
    let from = vm.relative_index(&arg(args, 1), len, 0)?;
    let fin = vm.relative_index(&arg(args, 2), len, len)?;
    let count = fin.saturating_sub(from).min(len - to);
    if count == 0 {
        return Ok(Value::Obj(o));
    }
    let (mut from, mut to, dir): (i64, i64, i64) = if from < to && to < from + count { ((from + count - 1) as i64, (to + count - 1) as i64, -1) } else { (from as i64, to as i64, 1) };
    for _ in 0..count {
        if has_i(vm, o, from as u64)? {
            let v = get_i(vm, o, from as u64)?;
            set_i(vm, o, to as u64, v)?;
        } else {
            delete_i(vm, o, to as u64)?;
        }
        from += dir;
        to += dir;
    }
    Ok(Value::Obj(o))
}

fn entries(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(this)?;
    Ok(array_iterator(vm, Value::Obj(o), 2))
}

fn keys(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(this)?;
    Ok(array_iterator(vm, Value::Obj(o), 0))
}

fn values(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(this)?;
    Ok(array_iterator(vm, Value::Obj(o), 1))
}

/// The iteration methods that call back for each element present: (o, len, callback, thisArg).
fn callback_args(vm: &mut Vm, this: &Value, args: &[Value]) -> Result<(Gc, u64, Value, Value), Value> {
    let (o, len) = obj_len(vm, this)?;
    let f = callable_arg(vm, args, 0)?;
    Ok((o, len, f, arg(args, 1)))
}

fn every(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len, f, t) = callback_args(vm, this, args)?;
    for k in 0..len {
        if has_i(vm, o, k)? {
            let v = get_i(vm, o, k)?;
            if !vm.call(&f, t.clone(), &[v, Value::Num(k as f64), Value::Obj(o)])?.truthy() {
                return Ok(Value::Bool(false));
            }
        }
    }
    Ok(Value::Bool(true))
}

fn some(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len, f, t) = callback_args(vm, this, args)?;
    for k in 0..len {
        if has_i(vm, o, k)? {
            let v = get_i(vm, o, k)?;
            if vm.call(&f, t.clone(), &[v, Value::Num(k as f64), Value::Obj(o)])?.truthy() {
                return Ok(Value::Bool(true));
            }
        }
    }
    Ok(Value::Bool(false))
}

fn for_each(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len, f, t) = callback_args(vm, this, args)?;
    for k in 0..len {
        if has_i(vm, o, k)? {
            let v = get_i(vm, o, k)?;
            vm.call(&f, t.clone(), &[v, Value::Num(k as f64), Value::Obj(o)])?;
        }
    }
    Ok(Value::Undefined)
}

fn map(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len, f, t) = callback_args(vm, this, args)?;
    let a = species_create(vm, o, len)?;
    for k in 0..len {
        if has_i(vm, o, k)? {
            let v = get_i(vm, o, k)?;
            let r = vm.call(&f, t.clone(), &[v, Value::Num(k as f64), Value::Obj(o)])?;
            create_i(vm, a, k, r)?;
        }
    }
    Ok(Value::Obj(a))
}

fn filter(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len, f, t) = callback_args(vm, this, args)?;
    let a = species_create(vm, o, 0)?;
    let mut to = 0;
    for k in 0..len {
        if has_i(vm, o, k)? {
            let v = get_i(vm, o, k)?;
            if vm.call(&f, t.clone(), &[v.clone(), Value::Num(k as f64), Value::Obj(o)])?.truthy() {
                create_i(vm, a, to, v)?;
                to += 1;
            }
        }
    }
    Ok(Value::Obj(a))
}

/// find, findIndex, findLast, findLastIndex (every index, holes too).
fn find_impl(vm: &mut Vm, this: &Value, args: &[Value], last: bool, index: bool) -> R {
    let (o, len, f, t) = callback_args(vm, this, args)?;
    for i in 0..len {
        let k = if last { len - 1 - i } else { i };
        let v = get_i(vm, o, k)?;
        if vm.call(&f, t.clone(), &[v.clone(), Value::Num(k as f64), Value::Obj(o)])?.truthy() {
            return Ok(if index { Value::Num(k as f64) } else { v });
        }
    }
    Ok(if index { Value::Num(-1.0) } else { Value::Undefined })
}

fn find(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    find_impl(vm, this, args, false, false)
}

fn find_index(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    find_impl(vm, this, args, false, true)
}

fn find_last(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    find_impl(vm, this, args, true, false)
}

fn find_last_index(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    find_impl(vm, this, args, true, true)
}

fn fill(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    let v = arg(args, 0);
    let from = vm.relative_index(&arg(args, 1), len, 0)?;
    let to = vm.relative_index(&arg(args, 2), len, len)?;
    for k in from..to {
        set_i(vm, o, k, v.clone())?;
    }
    Ok(Value::Obj(o))
}

fn flatten(vm: &mut Vm, target: Gc, source: Gc, source_len: u64, start: u64, depth: f64, mapper: Option<(&Value, &Value)>) -> Result<u64, Value> {
    let mut to = start;
    for k in 0..source_len {
        if !has_i(vm, source, k)? {
            continue;
        }
        let mut e = get_i(vm, source, k)?;
        if let Some((f, t)) = mapper {
            e = vm.call(f, t.clone(), &[e, Value::Num(k as f64), Value::Obj(source)])?;
        }
        if depth > 0.0 && vm.is_array(&e)? {
            let Value::Obj(eg) = e else { continue };
            let el = vm.length_of(eg)?;
            to = flatten(vm, target, eg, el, to, depth - 1.0, None)?;
        } else {
            if to >= MAX_SAFE {
                return Err(vm.type_error("Array too long"));
            }
            create_i(vm, target, to, e)?;
            to += 1;
        }
    }
    Ok(to)
}

fn flat(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    let d = arg(args, 0);
    let depth = if d.is_undefined() { 1.0 } else { vm.to_integer(&d)?.max(0.0) };
    let a = species_create(vm, o, 0)?;
    flatten(vm, a, o, len, 0, depth, None)?;
    Ok(Value::Obj(a))
}

fn flat_map(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len, f, t) = callback_args(vm, this, args)?;
    let a = species_create(vm, o, 0)?;
    flatten(vm, a, o, len, 0, 1.0, Some((&f, &t)))?;
    Ok(Value::Obj(a))
}

fn includes(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    if len == 0 {
        return Ok(Value::Bool(false));
    }
    let target = arg(args, 0);
    let from = vm.relative_index(&arg(args, 1), len, 0)?;
    for k in from..len {
        let v = get_i(vm, o, k)?;
        if same_value_zero(&v, &target) {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn index_of(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    if len == 0 {
        return Ok(Value::Num(-1.0));
    }
    let target = arg(args, 0);
    let from = vm.relative_index(&arg(args, 1), len, 0)?;
    if plain(vm, o) {
        if let Class::Array(els, _) = &vm.heap.get(o).class {
            for (k, v) in els.iter().enumerate().skip(from as usize) {
                if !matches!(v, Value::Empty) && strict_equals(v, &target) {
                    return Ok(Value::Num(k as f64));
                }
            }
        }
        return Ok(Value::Num(-1.0));
    }
    for k in from..len {
        if has_i(vm, o, k)? {
            let v = get_i(vm, o, k)?;
            if strict_equals(&v, &target) {
                return Ok(Value::Num(k as f64));
            }
        }
    }
    Ok(Value::Num(-1.0))
}

fn last_index_of(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    if len == 0 {
        return Ok(Value::Num(-1.0));
    }
    let target = arg(args, 0);
    let from = if args.len() > 1 {
        let x = vm.to_integer(&args[1])?;
        if x < 0.0 { len as f64 + x } else { x.min(len as f64 - 1.0) }
    } else {
        len as f64 - 1.0
    };
    if from < 0.0 {
        return Ok(Value::Num(-1.0));
    }
    let mut k = from as i64;
    while k >= 0 {
        if has_i(vm, o, k as u64)? {
            let v = get_i(vm, o, k as u64)?;
            if strict_equals(&v, &target) {
                return Ok(Value::Num(k as f64));
            }
        }
        k -= 1;
    }
    Ok(Value::Num(-1.0))
}

fn join(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    let sep = arg(args, 0);
    let sep = if sep.is_undefined() { JsStr::from_str(",") } else { vm.to_string(&sep)? };
    let mut units: Vec<u16> = Vec::new();
    for k in 0..len {
        if k > 0 {
            units.extend_from_slice(sep.units());
        }
        let v = get_i(vm, o, k)?;
        if !v.is_nullish() {
            let s = vm.to_string(&v)?;
            units.extend_from_slice(s.units());
        }
    }
    Ok(Value::Str(JsStr::from_units(units)))
}

fn to_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(this)?;
    let j = vm.get(o, Key::Str(n::JOIN), &Value::Obj(o))?;
    if vm.is_callable(&j) {
        return vm.call(&j, Value::Obj(o), &[]);
    }
    let s = super::core::builtin_tag(vm, &Value::Obj(o))?;
    Ok(super::js_str(&s))
}

fn to_locale_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    let mut units: Vec<u16> = Vec::new();
    for k in 0..len {
        if k > 0 {
            units.push(b',' as u16);
        }
        let v = get_i(vm, o, k)?;
        if !v.is_nullish() {
            let a = vm.atom("toLocaleString");
            let m = vm.get_value(&v, Key::Str(a))?;
            let r = vm.call(&m, v, &[])?;
            let s = vm.to_string(&r)?;
            units.extend_from_slice(s.units());
        }
    }
    Ok(Value::Str(JsStr::from_units(units)))
}

fn pop(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    if let Value::Obj(o) = this
        && plain(vm, *o)
        && let Class::Array(els, len) = &mut vm.heap.get_mut(*o).class
    {
        if *len == 0 {
            return Ok(Value::Undefined);
        }
        *len -= 1;
        if els.len() > *len as usize {
            let v = els.pop().unwrap_or(Value::Undefined);
            return Ok(if matches!(v, Value::Empty) { Value::Undefined } else { v });
        }
        let l = *len;
        // A hole at the end: its prototype's element (rare).
        if let Class::Array(_, len) = &mut vm.heap.get_mut(*o).class {
            *len = l + 1;
        }
    }
    let (o, len) = obj_len(vm, this)?;
    if len == 0 {
        set_len(vm, o, 0)?;
        return Ok(Value::Undefined);
    }
    let v = get_i(vm, o, len - 1)?;
    delete_i(vm, o, len - 1)?;
    set_len(vm, o, len - 1)?;
    Ok(v)
}

fn push(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if let Value::Obj(o) = this
        && plain(vm, *o)
        && let Class::Array(els, len) = &mut vm.heap.get_mut(*o).class
        && els.len() == *len as usize
        && (*len as u64 + args.len() as u64) < u32::MAX as u64
    {
        els.extend_from_slice(args);
        *len += args.len() as u32;
        return Ok(Value::Num(*len as f64));
    }
    let (o, len) = obj_len(vm, this)?;
    if len + args.len() as u64 > MAX_SAFE {
        return Err(vm.type_error("Pushing too many elements"));
    }
    for (i, v) in args.iter().enumerate() {
        set_i(vm, o, len + i as u64, v.clone())?;
    }
    let nl = len + args.len() as u64;
    set_len(vm, o, nl)?;
    Ok(Value::Num(nl as f64))
}

fn reduce_impl(vm: &mut Vm, this: &Value, args: &[Value], right: bool) -> R {
    let (o, len) = obj_len(vm, this)?;
    let f = callable_arg(vm, args, 0)?;
    let order = |i: u64| if right { len - 1 - i } else { i };
    let mut i = 0;
    let mut acc = if args.len() > 1 {
        args[1].clone()
    } else {
        loop {
            if i >= len {
                return Err(vm.type_error("Reduce of empty array with no initial value"));
            }
            let k = order(i);
            i += 1;
            if has_i(vm, o, k)? {
                break get_i(vm, o, k)?;
            }
        }
    };
    while i < len {
        let k = order(i);
        if has_i(vm, o, k)? {
            let v = get_i(vm, o, k)?;
            acc = vm.call(&f, Value::Undefined, &[acc, v, Value::Num(k as f64), Value::Obj(o)])?;
        }
        i += 1;
    }
    Ok(acc)
}

fn reduce(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    reduce_impl(vm, this, args, false)
}

fn reduce_right(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    reduce_impl(vm, this, args, true)
}

fn reverse(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    if plain(vm, o)
        && let Class::Array(els, l) = &mut vm.heap.get_mut(o).class
        && els.len() == *l as usize
        && !els.iter().any(|v| matches!(v, Value::Empty))
    {
        els.reverse();
        return Ok(Value::Obj(o));
    }
    let mut lower = 0;
    while lower < len / 2 {
        let upper = len - 1 - lower;
        let le = has_i(vm, o, lower)?;
        let lv = if le { get_i(vm, o, lower)? } else { Value::Undefined };
        let ue = has_i(vm, o, upper)?;
        let uv = if ue { get_i(vm, o, upper)? } else { Value::Undefined };
        match (le, ue) {
            (true, true) => {
                set_i(vm, o, lower, uv)?;
                set_i(vm, o, upper, lv)?;
            }
            (false, true) => {
                set_i(vm, o, lower, uv)?;
                delete_i(vm, o, upper)?;
            }
            (true, false) => {
                delete_i(vm, o, lower)?;
                set_i(vm, o, upper, lv)?;
            }
            (false, false) => {}
        }
        lower += 1;
    }
    Ok(Value::Obj(o))
}

fn shift(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    if let Value::Obj(o) = this
        && plain(vm, *o)
        && let Class::Array(els, len) = &mut vm.heap.get_mut(*o).class
        && els.len() == *len as usize
        && !els.iter().any(|v| matches!(v, Value::Empty))
    {
        if els.is_empty() {
            return Ok(Value::Undefined);
        }
        *len -= 1;
        return Ok(els.remove(0));
    }
    let (o, len) = obj_len(vm, this)?;
    if len == 0 {
        set_len(vm, o, 0)?;
        return Ok(Value::Undefined);
    }
    let first = get_i(vm, o, 0)?;
    for k in 1..len {
        if has_i(vm, o, k)? {
            let v = get_i(vm, o, k)?;
            set_i(vm, o, k - 1, v)?;
        } else {
            delete_i(vm, o, k - 1)?;
        }
    }
    delete_i(vm, o, len - 1)?;
    set_len(vm, o, len - 1)?;
    Ok(first)
}

fn unshift(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    let n = args.len() as u64;
    if n > 0 {
        if len + n > MAX_SAFE {
            return Err(vm.type_error("Array too long"));
        }
        if plain(vm, o)
            && let Class::Array(els, l) = &mut vm.heap.get_mut(o).class
            && els.len() == *l as usize
            && (*l as u64 + n) < u32::MAX as u64
        {
            els.splice(0..0, args.iter().cloned());
            *l += n as u32;
            return Ok(Value::Num(*l as f64));
        }
        let mut k = len;
        while k > 0 {
            let from = k - 1;
            let to = k + n - 1;
            if has_i(vm, o, from)? {
                let v = get_i(vm, o, from)?;
                set_i(vm, o, to, v)?;
            } else {
                delete_i(vm, o, to)?;
            }
            k -= 1;
        }
        for (j, v) in args.iter().enumerate() {
            set_i(vm, o, j as u64, v.clone())?;
        }
    }
    set_len(vm, o, len + n)?;
    Ok(Value::Num((len + n) as f64))
}

fn slice(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    let k = vm.relative_index(&arg(args, 0), len, 0)?;
    let fin = vm.relative_index(&arg(args, 1), len, len)?;
    let count = fin.saturating_sub(k);
    let a = species_create(vm, o, count)?;
    let mut to = 0;
    for i in k..k + count {
        if has_i(vm, o, i)? {
            let v = get_i(vm, o, i)?;
            create_i(vm, a, to, v)?;
        }
        to += 1;
    }
    set_len(vm, a, count)?;
    Ok(Value::Obj(a))
}

/// The comparison sort uses: undefined last, then the comparator (or the strings' order).
fn compare(vm: &mut Vm, cmp: &Value, a: &Value, b: &Value) -> Result<core::cmp::Ordering, Value> {
    use core::cmp::Ordering;
    match (a.is_undefined(), b.is_undefined()) {
        (true, true) => return Ok(Ordering::Equal),
        (true, false) => return Ok(Ordering::Greater),
        (false, true) => return Ok(Ordering::Less),
        _ => {}
    }
    if !cmp.is_undefined() {
        let r = vm.call(cmp, Value::Undefined, &[a.clone(), b.clone()])?;
        let x = vm.to_number(&r)?;
        return Ok(if x < 0.0 {
            Ordering::Less
        } else if x > 0.0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        });
    }
    if let (Value::Num(x), Value::Num(y)) = (a, b)
        && crate::math::trunc(*x) == *x
        && crate::math::trunc(*y) == *y
        && *x >= 0.0
        && *y >= 0.0
        && *x < 1e15
        && *y < 1e15
    {
        // (Non-negative integers: their strings' order without making them.)
        let (sx, sy) = (format!("{}", *x as u64), format!("{}", *y as u64));
        return Ok(sx.cmp(&sy));
    }
    let sa = vm.to_string(a)?;
    let sb = vm.to_string(b)?;
    Ok(sa.units().cmp(sb.units()))
}

/// A stable merge sort whose comparisons may throw.
pub fn merge_sort(vm: &mut Vm, v: &mut Vec<Value>, cmp: &Value) -> Result<(), Value> {
    let n = v.len();
    if n < 2 {
        return Ok(());
    }
    let mut buf: Vec<Value> = v.clone();
    let mut width = 1;
    // Insertion sort runs of 8 first.
    let run = 8;
    let mut i = 0;
    while i < n {
        let end = (i + run).min(n);
        for j in i + 1..end {
            let mut k = j;
            while k > i && compare(vm, cmp, &v[k - 1], &v[k])? == core::cmp::Ordering::Greater {
                v.swap(k - 1, k);
                k -= 1;
            }
        }
        i = end;
    }
    width = width.max(run);
    while width < n {
        let mut lo = 0;
        while lo < n {
            let mid = (lo + width).min(n);
            let hi = (lo + 2 * width).min(n);
            let (mut a, mut b, mut k) = (lo, mid, lo);
            while a < mid && b < hi {
                if compare(vm, cmp, &v[b], &v[a])? == core::cmp::Ordering::Less {
                    buf[k] = v[b].clone();
                    b += 1;
                } else {
                    buf[k] = v[a].clone();
                    a += 1;
                }
                k += 1;
            }
            while a < mid {
                buf[k] = v[a].clone();
                a += 1;
                k += 1;
            }
            while b < hi {
                buf[k] = v[b].clone();
                b += 1;
                k += 1;
            }
            lo = hi;
        }
        core::mem::swap(v, &mut buf);
        width *= 2;
    }
    Ok(())
}

/// SortIndexedProperties (skipping holes): the values to sort.
fn sort_values(vm: &mut Vm, o: Gc, len: u64, holes: bool, cmp: &Value) -> Result<Vec<Value>, Value> {
    let mut items = Vec::new();
    for k in 0..len {
        if !holes || has_i(vm, o, k)? {
            items.push(get_i(vm, o, k)?);
        }
    }
    merge_sort(vm, &mut items, cmp)?;
    Ok(items)
}

fn sort(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let cmp = arg(args, 0);
    if !cmp.is_undefined() && !vm.is_callable(&cmp) {
        return Err(vm.type_error("The comparison function must be either a function or undefined"));
    }
    let (o, len) = obj_len(vm, this)?;
    let items = sort_values(vm, o, len, true, &cmp)?;
    let count = items.len() as u64;
    for (i, v) in items.into_iter().enumerate() {
        set_i(vm, o, i as u64, v)?;
    }
    for k in count..len {
        if has_i(vm, o, k)? {
            delete_i(vm, o, k)?;
        }
    }
    Ok(Value::Obj(o))
}

fn to_sorted(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let cmp = arg(args, 0);
    if !cmp.is_undefined() && !vm.is_callable(&cmp) {
        return Err(vm.type_error("The comparison function must be either a function or undefined"));
    }
    let (o, len) = obj_len(vm, this)?;
    let a = array_create(vm, len, None)?;
    let items = sort_values(vm, o, len, false, &cmp)?;
    for (i, v) in items.into_iter().enumerate() {
        create_i(vm, a, i as u64, v)?;
    }
    Ok(Value::Obj(a))
}

fn to_reversed(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    let a = array_create(vm, len, None)?;
    for k in 0..len {
        let v = get_i(vm, o, len - 1 - k)?;
        create_i(vm, a, k, v)?;
    }
    Ok(Value::Obj(a))
}

fn splice(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    let start = vm.relative_index(&arg(args, 0), len, 0)?;
    let items: Vec<Value> = args.iter().skip(2).cloned().collect();
    let del = match args.len() {
        0 => 0,
        1 => len - start,
        _ => {
            let d = vm.to_integer(&args[1])?;
            (d.max(0.0) as u64).min(len - start)
        }
    };
    if len + items.len() as u64 - del > MAX_SAFE {
        return Err(vm.type_error("Array too long"));
    }
    let a = species_create(vm, o, del)?;
    for k in 0..del {
        if has_i(vm, o, start + k)? {
            let v = get_i(vm, o, start + k)?;
            create_i(vm, a, k, v)?;
        }
    }
    set_len(vm, a, del)?;
    let ic = items.len() as u64;
    if ic < del {
        for k in start..len - del {
            let (from, to) = (k + del, k + ic);
            if has_i(vm, o, from)? {
                let v = get_i(vm, o, from)?;
                set_i(vm, o, to, v)?;
            } else {
                delete_i(vm, o, to)?;
            }
        }
        let mut k = len;
        while k > len - del + ic {
            delete_i(vm, o, k - 1)?;
            k -= 1;
        }
    } else if ic > del {
        let mut k = len - del;
        while k > start {
            let (from, to) = (k + del - 1, k + ic - 1);
            if has_i(vm, o, from)? {
                let v = get_i(vm, o, from)?;
                set_i(vm, o, to, v)?;
            } else {
                delete_i(vm, o, to)?;
            }
            k -= 1;
        }
    }
    for (j, v) in items.into_iter().enumerate() {
        set_i(vm, o, start + j as u64, v)?;
    }
    set_len(vm, o, len - del + ic)?;
    Ok(Value::Obj(a))
}

fn to_spliced(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    let start = vm.relative_index(&arg(args, 0), len, 0)?;
    let items: Vec<Value> = args.iter().skip(2).cloned().collect();
    let skip = match args.len() {
        0 => 0,
        1 => len - start,
        _ => {
            let d = vm.to_integer(&args[1])?;
            (d.max(0.0) as u64).min(len - start)
        }
    };
    let new_len = len + items.len() as u64 - skip;
    if new_len > MAX_SAFE {
        return Err(vm.type_error("Array too long"));
    }
    let a = array_create(vm, new_len, None)?;
    let mut i = 0;
    for k in 0..start {
        let v = get_i(vm, o, k)?;
        create_i(vm, a, i, v)?;
        i += 1;
    }
    for v in items {
        create_i(vm, a, i, v)?;
        i += 1;
    }
    for k in start + skip..len {
        let v = get_i(vm, o, k)?;
        create_i(vm, a, i, v)?;
        i += 1;
    }
    Ok(Value::Obj(a))
}

fn with(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (o, len) = obj_len(vm, this)?;
    let rel = vm.to_integer(&arg(args, 0))?;
    let idx = if rel >= 0.0 { rel } else { len as f64 + rel };
    if idx < 0.0 || idx >= len as f64 {
        return Err(vm.range_error("Invalid index"));
    }
    let a = array_create(vm, len, None)?;
    let v = arg(args, 1);
    for k in 0..len {
        let x = if k == idx as u64 { v.clone() } else { get_i(vm, o, k)? };
        create_i(vm, a, k, x)?;
    }
    Ok(Value::Obj(a))
}
