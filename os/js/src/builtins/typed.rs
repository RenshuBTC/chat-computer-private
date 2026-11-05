//! ArrayBuffer, DataView, %TypedArray% and its eleven kinds (Int8Array to BigUint64Array): a buffer of bytes, and
//! views reading and writing its elements.

use super::{arg, callable_arg, constructor, getter, js_str, species_getter, sym_getter, sym_method};
use crate::bigint::BigInt;
use crate::builtins::array::merge_sort;
use crate::builtins::iter::array_iterator;
use crate::names as n;
use crate::object::*;
use crate::value::{Gc, Value, same_value_zero, strict_equals, to_int32, to_uint32};
use crate::vm::{I, Sym, Vm};
use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;

type R = Result<Value, Value>;

const KINDS: [(ElemKind, &str, I); 11] = [
    (ElemKind::Int8, "Int8Array", I::Int8ArrayProto),
    (ElemKind::Uint8, "Uint8Array", I::Uint8ArrayProto),
    (ElemKind::Uint8Clamped, "Uint8ClampedArray", I::Uint8ClampedArrayProto),
    (ElemKind::Int16, "Int16Array", I::Int16ArrayProto),
    (ElemKind::Uint16, "Uint16Array", I::Uint16ArrayProto),
    (ElemKind::Int32, "Int32Array", I::Int32ArrayProto),
    (ElemKind::Uint32, "Uint32Array", I::Uint32ArrayProto),
    (ElemKind::Float32, "Float32Array", I::Float32ArrayProto),
    (ElemKind::Float64, "Float64Array", I::Float64ArrayProto),
    (ElemKind::BigInt64, "BigInt64Array", I::BigInt64ArrayProto),
    (ElemKind::BigUint64, "BigUint64Array", I::BigUint64ArrayProto),
];

fn kind_index(k: ElemKind) -> usize {
    KINDS.iter().position(|(x, _, _)| *x == k).unwrap_or(0)
}

fn is_big(k: ElemKind) -> bool {
    matches!(k, ElemKind::BigInt64 | ElemKind::BigUint64)
}

/// An element read from its bytes.
pub fn read_elem(kind: ElemKind, b: &[u8], little: bool) -> Value {
    let mut buf = [0u8; 8];
    let n = kind.size();
    buf[..n].copy_from_slice(&b[..n]);
    if !little {
        buf[..n].reverse();
    }
    match kind {
        ElemKind::Int8 => Value::Num(buf[0] as i8 as f64),
        ElemKind::Uint8 | ElemKind::Uint8Clamped => Value::Num(buf[0] as f64),
        ElemKind::Int16 => Value::Num(i16::from_le_bytes([buf[0], buf[1]]) as f64),
        ElemKind::Uint16 => Value::Num(u16::from_le_bytes([buf[0], buf[1]]) as f64),
        ElemKind::Int32 => Value::Num(i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as f64),
        ElemKind::Uint32 => Value::Num(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as f64),
        ElemKind::Float32 => Value::Num(f32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as f64),
        ElemKind::Float64 => Value::Num(f64::from_le_bytes(buf)),
        ElemKind::BigInt64 => Value::Big(Rc::new(BigInt::from_i64(i64::from_le_bytes(buf)))),
        ElemKind::BigUint64 => Value::Big(Rc::new(BigInt::from_u64(u64::from_le_bytes(buf)))),
    }
}

/// An element's bytes written (the value already a number, or a BigInt for the 64-bit integer kinds).
pub fn write_elem(kind: ElemKind, b: &mut [u8], v: &Value, little: bool) {
    let x = match v {
        Value::Num(x) => *x,
        _ => 0.0,
    };
    let mut buf = [0u8; 8];
    let n = kind.size();
    match kind {
        ElemKind::Int8 | ElemKind::Uint8 => buf[0] = to_int32(x) as u8,
        ElemKind::Uint8Clamped => {
            buf[0] = if x.is_nan() || x <= 0.0 {
                0
            } else if x >= 255.0 {
                255
            } else {
                // Round half to even.
                let f = crate::math::floor(x);
                let r = if x - f > 0.5 || (x - f == 0.5 && (f as u32) % 2 == 1) { f + 1.0 } else { f };
                r as u8
            }
        }
        ElemKind::Int16 | ElemKind::Uint16 => buf[..2].copy_from_slice(&(to_int32(x) as u16).to_le_bytes()),
        ElemKind::Int32 | ElemKind::Uint32 => buf[..4].copy_from_slice(&to_uint32(x).to_le_bytes()),
        ElemKind::Float32 => buf[..4].copy_from_slice(&(x as f32).to_le_bytes()),
        ElemKind::Float64 => buf.copy_from_slice(&x.to_le_bytes()),
        ElemKind::BigInt64 | ElemKind::BigUint64 => {
            let w = match v {
                Value::Big(b) => b.to_u64_wrapping(),
                _ => 0,
            };
            buf.copy_from_slice(&w.to_le_bytes());
        }
    }
    if !little {
        buf[..n].reverse();
    }
    b[..n].copy_from_slice(&buf[..n]);
}

pub fn init(vm: &mut Vm) {
    // ArrayBuffer.
    let abp = vm.proto(I::ArrayBufferProto);
    let ab = constructor(vm, "ArrayBuffer", 1, needs_new, Some(ab_construct), Some(abp));
    vm.intrinsics[I::ArrayBufferCtor as usize] = ab;
    vm.method(ab, "isView", 1, ab_is_view);
    sym_getter(vm, ab, Sym::Species, "[Symbol.species]", species_getter);
    getter(vm, abp, "byteLength", ab_byte_length);
    getter(vm, abp, "maxByteLength", ab_max_byte_length);
    getter(vm, abp, "resizable", ab_resizable);
    getter(vm, abp, "detached", ab_detached);
    vm.method(abp, "slice", 2, ab_slice);
    vm.method(abp, "resize", 1, ab_resize);
    vm.method(abp, "transfer", 0, ab_transfer);
    vm.method(abp, "transferToFixedLength", 0, ab_transfer_fixed);
    super::tag(vm, abp, "ArrayBuffer");

    // DataView.
    let dvp = vm.proto(I::DataViewProto);
    constructor(vm, "DataView", 1, needs_new, Some(dv_construct), Some(dvp));
    getter(vm, dvp, "buffer", dv_buffer);
    getter(vm, dvp, "byteLength", dv_byte_length);
    getter(vm, dvp, "byteOffset", dv_byte_offset);
    for (i, (_, name, _)) in KINDS.iter().enumerate() {
        if i == 2 {
            continue;
        }
        let short = &name[..name.len() - 5];
        let g = vm.native_with(&format!("get{short}"), 1, dv_get, None, Value::Num(i as f64));
        vm.put(dvp, &format!("get{short}"), Value::Obj(g), HIDDEN);
        let s = vm.native_with(&format!("set{short}"), 2, dv_set, None, Value::Num(i as f64));
        vm.put(dvp, &format!("set{short}"), Value::Obj(s), HIDDEN);
    }
    super::tag(vm, dvp, "DataView");

    // %TypedArray%.
    let tap = vm.proto(I::TypedArrayProto);
    let ta = vm.native_with("TypedArray", 0, ta_abstract, Some(ta_abstract), Value::Undefined);
    vm.intrinsics[I::TypedArrayCtor as usize] = ta;
    vm.put_key(ta, Key::Str(n::PROTOTYPE), Value::Obj(tap), 0);
    vm.put_key(tap, Key::Str(n::CONSTRUCTOR), Value::Obj(ta), HIDDEN);
    vm.method(ta, "from", 1, ta_from);
    vm.method(ta, "of", 0, ta_of);
    sym_getter(vm, ta, Sym::Species, "[Symbol.species]", species_getter);
    getter(vm, tap, "buffer", ta_buffer);
    getter(vm, tap, "byteLength", ta_byte_length);
    getter(vm, tap, "byteOffset", ta_byte_offset);
    getter(vm, tap, "length", ta_length);
    sym_getter(vm, tap, Sym::ToStringTag, "[Symbol.toStringTag]", ta_tag);
    let methods: [(&str, u32, NativeFn); 30] = [
        ("at", 1, ta_at),
        ("copyWithin", 2, ta_copy_within),
        ("entries", 0, ta_entries),
        ("every", 1, ta_every),
        ("fill", 1, ta_fill),
        ("filter", 1, ta_filter),
        ("find", 1, ta_find),
        ("findIndex", 1, ta_find_index),
        ("findLast", 1, ta_find_last),
        ("findLastIndex", 1, ta_find_last_index),
        ("forEach", 1, ta_for_each),
        ("includes", 1, ta_includes),
        ("indexOf", 1, ta_index_of),
        ("join", 1, ta_join),
        ("keys", 0, ta_keys),
        ("lastIndexOf", 1, ta_last_index_of),
        ("map", 1, ta_map),
        ("reduce", 1, ta_reduce),
        ("reduceRight", 1, ta_reduce_right),
        ("reverse", 0, ta_reverse),
        ("set", 1, ta_set),
        ("slice", 2, ta_slice),
        ("some", 1, ta_some),
        ("sort", 1, ta_sort),
        ("subarray", 2, ta_subarray),
        ("toLocaleString", 0, ta_to_locale_string),
        ("toReversed", 0, ta_to_reversed),
        ("toSorted", 1, ta_to_sorted),
        ("values", 0, ta_values),
        ("with", 2, ta_with),
    ];
    for (name, len, f) in methods {
        let g = vm.method(tap, name, len, f);
        if name == "values" {
            let k = Key::Sym(vm.sym(Sym::Iterator));
            vm.put_key(tap, k, Value::Obj(g), HIDDEN);
        }
    }
    // (%TypedArray%.prototype.toString is Array.prototype.toString itself.)
    let ap = vm.proto(I::ArrayProto);
    if let Some(Prop { slot: Slot::Data(f), .. }) = vm.heap.get(ap).props.get(&Key::Str(n::TO_STRING)).cloned() {
        vm.put_key(tap, Key::Str(n::TO_STRING), f, HIDDEN);
    }
    for (i, (kind, name, proto)) in KINDS.iter().enumerate() {
        let p = vm.proto(*proto);
        let c = constructor(vm, name, 3, needs_new, Some(kind_construct), Some(p));
        vm.intrinsics[I::Int8ArrayCtor as usize + i] = c;
        if let Class::Native(nat) = &mut vm.heap.get_mut(c).class {
            nat.data = Value::Num(i as f64);
        }
        vm.heap.get_mut(c).proto = Some(ta);
        vm.put(c, "BYTES_PER_ELEMENT", Value::Num(kind.size() as f64), 0);
        vm.put(p, "BYTES_PER_ELEMENT", Value::Num(kind.size() as f64), 0);
    }
    let _ = sym_method;
}

fn needs_new(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Err(vm.type_error("Constructor requires 'new'"))
}

fn ta_abstract(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Err(vm.type_error("Abstract class TypedArray not directly constructable"))
}

// ----------------------------------------------------------------------------------------------- ArrayBuffer

/// The most bytes a buffer may hold (a page asking for more gets a RangeError).
pub const MAX_BYTES: u64 = 1 << 30;

/// `len` zeroed bytes, or a RangeError.
fn zeroed(vm: &mut Vm, len: u64) -> Result<Vec<u8>, Value> {
    let mut v = Vec::new();
    if len > MAX_BYTES || v.try_reserve_exact(len as usize).is_err() {
        return Err(vm.range_error("Array buffer allocation failed"));
    }
    v.resize(len as usize, 0);
    Ok(v)
}

/// A new ArrayBuffer (zeroed), its prototype from new.target.
pub fn new_buffer(vm: &mut Vm, nt: &Value, len: u64, max: Option<u64>) -> Result<Gc, Value> {
    let p = vm.proto_from_constructor(nt, I::ArrayBufferProto)?;
    if max.is_some_and(|m| m > MAX_BYTES) {
        return Err(vm.range_error("Invalid array buffer max length"));
    }
    let data = zeroed(vm, len)?;
    Ok(vm.alloc(Some(p), Class::ArrayBuffer(Some(data), max.map(|m| m as usize))))
}

fn ab_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let len = vm.to_index(&arg(args, 0))?;
    let mut max = None;
    if let Value::Obj(o) = arg(args, 1) {
        let a = vm.atom("maxByteLength");
        let m = vm.get(o, Key::Str(a), &Value::Obj(o))?;
        if !m.is_undefined() {
            let mv = vm.to_index(&m)?;
            if len > mv {
                return Err(vm.range_error("Invalid array buffer length"));
            }
            max = Some(mv);
        }
    }
    Ok(Value::Obj(new_buffer(vm, nt, len, max)?))
}

fn ab_is_view(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Obj(g) if matches!(vm.heap.get(g).class, Class::TypedArray(_) | Class::DataView(..)))))
}

/// `this` as an ArrayBuffer: (object, its bytes or None if detached, its max length).
fn this_buffer(vm: &mut Vm, this: &Value, what: &str) -> Result<(Gc, Option<usize>, Option<usize>), Value> {
    if let Value::Obj(g) = this
        && let Class::ArrayBuffer(b, max) = &vm.heap.get(*g).class
    {
        return Ok((*g, b.as_ref().map(|b| b.len()), *max));
    }
    Err(vm.type_error(&format!("ArrayBuffer.prototype.{what} called on incompatible receiver")))
}

fn ab_byte_length(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, len, _) = this_buffer(vm, this, "byteLength")?;
    Ok(Value::Num(len.unwrap_or(0) as f64))
}

fn ab_max_byte_length(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, len, max) = this_buffer(vm, this, "maxByteLength")?;
    Ok(Value::Num(if len.is_none() { 0.0 } else { max.or(len).unwrap_or(0) as f64 }))
}

fn ab_resizable(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, _, max) = this_buffer(vm, this, "resizable")?;
    Ok(Value::Bool(max.is_some()))
}

fn ab_detached(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, len, _) = this_buffer(vm, this, "detached")?;
    Ok(Value::Bool(len.is_none()))
}

fn ab_slice(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, len, _) = this_buffer(vm, this, "slice")?;
    let Some(len) = len else { return Err(vm.type_error("Cannot perform ArrayBuffer.prototype.slice on a detached ArrayBuffer")) };
    let first = vm.relative_index(&arg(args, 0), len as u64, 0)?;
    let fin = vm.relative_index(&arg(args, 1), len as u64, len as u64)?;
    let count = fin.saturating_sub(first);
    let c = vm.species_constructor(g, I::ArrayBufferCtor)?;
    let new = vm.construct(&c, &[Value::Num(count as f64)], &c)?;
    let Value::Obj(ng) = new else { return Err(vm.type_error("not an object")) };
    match &vm.heap.get(ng).class {
        Class::ArrayBuffer(Some(b), _) if b.len() >= count as usize && ng != g => {}
        Class::ArrayBuffer(None, _) => return Err(vm.type_error("The new ArrayBuffer is detached")),
        Class::ArrayBuffer(..) if ng == g => return Err(vm.type_error("ArrayBuffer subclass returned this from species constructor")),
        Class::ArrayBuffer(..) => return Err(vm.type_error("The new ArrayBuffer is too small")),
        _ => return Err(vm.type_error("The species constructor didn't make an ArrayBuffer")),
    }
    let src: Vec<u8> = match &vm.heap.get(g).class {
        Class::ArrayBuffer(Some(b), _) => {
            let end = (first + count).min(b.len() as u64) as usize;
            let start = (first as usize).min(end);
            b[start..end].to_vec()
        }
        _ => return Err(vm.type_error("Cannot perform ArrayBuffer.prototype.slice on a detached ArrayBuffer")),
    };
    if let Class::ArrayBuffer(Some(b), _) = &mut vm.heap.get_mut(ng).class {
        b[..src.len()].copy_from_slice(&src);
    }
    Ok(new)
}

fn ab_resize(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, len, max) = this_buffer(vm, this, "resize")?;
    let Some(max) = max else { return Err(vm.type_error("ArrayBuffer.prototype.resize called on a fixed-length ArrayBuffer")) };
    let nl = vm.to_index(&arg(args, 0))?;
    let _ = len;
    if matches!(vm.heap.get(g).class, Class::ArrayBuffer(None, _)) {
        return Err(vm.type_error("Cannot resize a detached ArrayBuffer"));
    }
    if nl as usize > max {
        return Err(vm.range_error("Invalid array buffer length"));
    }
    if let Class::ArrayBuffer(Some(b), _) = &mut vm.heap.get_mut(g).class {
        b.resize(nl as usize, 0);
    }
    Ok(Value::Undefined)
}

fn transfer(vm: &mut Vm, this: &Value, args: &[Value], fixed: bool) -> R {
    let (g, len, max) = this_buffer(vm, this, "transfer")?;
    let nl = if arg(args, 0).is_undefined() { len.unwrap_or(0) as u64 } else { vm.to_index(&arg(args, 0))? };
    let Some(_) = len else { return Err(vm.type_error("Cannot transfer a detached ArrayBuffer")) };
    let new_max = if fixed { None } else { max };
    if new_max.is_some_and(|m| nl as usize > m) {
        return Err(vm.range_error("Invalid array buffer length"));
    }
    let mut data = match &mut vm.heap.get_mut(g).class {
        Class::ArrayBuffer(b, _) => b.take().unwrap_or_default(),
        _ => Vec::new(),
    };
    data.resize(nl as usize, 0);
    let p = vm.proto(I::ArrayBufferProto);
    Ok(Value::Obj(vm.alloc(Some(p), Class::ArrayBuffer(Some(data), new_max))))
}

fn ab_transfer(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    transfer(vm, this, args, false)
}

fn ab_transfer_fixed(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    transfer(vm, this, args, true)
}

// -------------------------------------------------------------------------------------------------- DataView

fn dv_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let b = arg(args, 0);
    let Value::Obj(bg) = b else { return Err(vm.type_error("First argument to DataView constructor must be an ArrayBuffer")) };
    if !matches!(vm.heap.get(bg).class, Class::ArrayBuffer(..)) {
        return Err(vm.type_error("First argument to DataView constructor must be an ArrayBuffer"));
    }
    let offset = vm.to_index(&arg(args, 1))?;
    let (blen, max) = match &vm.heap.get(bg).class {
        Class::ArrayBuffer(Some(d), m) => (d.len() as u64, *m),
        _ => return Err(vm.type_error("Cannot construct a DataView on a detached ArrayBuffer")),
    };
    if offset > blen {
        return Err(vm.range_error("Start offset is outside the bounds of the buffer"));
    }
    let len_arg = arg(args, 2);
    let view_len = if len_arg.is_undefined() {
        if max.is_some() { None } else { Some(blen - offset) }
    } else {
        let l = vm.to_index(&len_arg)?;
        if offset + l > blen {
            return Err(vm.range_error("Invalid DataView length"));
        }
        Some(l)
    };
    let p = vm.proto_from_constructor(nt, I::DataViewProto)?;
    // (The buffer may have been detached or shrunk by the conversions above.)
    let blen2 = match &vm.heap.get(bg).class {
        Class::ArrayBuffer(Some(d), _) => d.len() as u64,
        _ => return Err(vm.type_error("Cannot construct a DataView on a detached ArrayBuffer")),
    };
    if offset > blen2 || view_len.is_some_and(|l| offset + l > blen2) {
        return Err(vm.range_error("Invalid DataView length"));
    }
    Ok(Value::Obj(vm.alloc(Some(p), Class::DataView(bg, offset as usize, view_len.map(|l| l as usize)))))
}

/// `this` as a DataView: (buffer, offset, length now) — a TypeError if it's detached or out of bounds.
fn this_view(vm: &mut Vm, this: &Value, what: &str, check: bool) -> Result<(Gc, usize, usize), Value> {
    if let Value::Obj(g) = this
        && let Class::DataView(b, off, len) = vm.heap.get(*g).class
    {
        let blen = match &vm.heap.get(b).class {
            Class::ArrayBuffer(Some(d), _) => Some(d.len()),
            _ => None,
        };
        let Some(blen) = blen else {
            if check {
                return Err(vm.type_error(&format!("Cannot perform DataView.prototype.{what} on a detached ArrayBuffer")));
            }
            return Ok((b, off, 0));
        };
        let l = match len {
            Some(l) => l,
            None => blen.saturating_sub(off),
        };
        if check && (off > blen || off + l > blen) {
            return Err(vm.type_error(&format!("DataView.prototype.{what}: the view is out of bounds")));
        }
        return Ok((b, off, l));
    }
    Err(vm.type_error(&format!("DataView.prototype.{what} called on incompatible receiver")))
}

fn dv_buffer(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (b, _, _) = this_view(vm, this, "buffer", false)?;
    Ok(Value::Obj(b))
}

// (Both 0 when the view is out of bounds or its buffer detached.)
fn dv_byte_length(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    this_view(vm, this, "byteLength", false)?;
    Ok(Value::Num(this_view(vm, this, "byteLength", true).map_or(0.0, |(_, _, l)| l as f64)))
}

fn dv_byte_offset(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    this_view(vm, this, "byteOffset", false)?;
    Ok(Value::Num(this_view(vm, this, "byteOffset", true).map_or(0.0, |(_, o, _)| o as f64)))
}

fn dv_kind(vm: &Vm, f: Gc) -> ElemKind {
    match vm.data(f) {
        Value::Num(i) => KINDS[i as usize].0,
        _ => ElemKind::Uint8,
    }
}

fn dv_get(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let kind = dv_kind(vm, f);
    this_view(vm, this, "get", false)?;
    let idx = vm.to_index(&arg(args, 0))? as usize;
    let little = arg(args, 1).truthy();
    let (b, off, len) = this_view(vm, this, "get", true)?;
    if idx + kind.size() > len {
        return Err(vm.range_error("Offset is outside the bounds of the DataView"));
    }
    match &vm.heap.get(b).class {
        Class::ArrayBuffer(Some(d), _) => Ok(read_elem(kind, &d[off + idx..off + idx + kind.size()], little)),
        _ => Err(vm.type_error("detached")),
    }
}

fn dv_set(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let kind = dv_kind(vm, f);
    this_view(vm, this, "set", false)?;
    let idx = vm.to_index(&arg(args, 0))? as usize;
    let v = if is_big(kind) { Value::Big(Rc::new(vm.to_bigint(&arg(args, 1))?)) } else { Value::Num(vm.to_number(&arg(args, 1))?) };
    let little = arg(args, 2).truthy();
    let (b, off, len) = this_view(vm, this, "set", true)?;
    if idx + kind.size() > len {
        return Err(vm.range_error("Offset is outside the bounds of the DataView"));
    }
    if let Class::ArrayBuffer(Some(d), _) = &mut vm.heap.get_mut(b).class {
        write_elem(kind, &mut d[off + idx..off + idx + kind.size()], &v, little);
    }
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------------------------------------ typed arrays

/// A typed array's parts: (kind, buffer, offset, length now, whether it's out of bounds or detached).
fn parts(vm: &Vm, g: Gc) -> Option<(ElemKind, Gc, usize, usize, bool)> {
    match &vm.heap.get(g).class {
        Class::TypedArray(t) => {
            let out = vm.typed_out_of_bounds(t);
            Some((t.kind, t.buffer, t.offset, if out { 0 } else { vm.typed_length(t) }, out))
        }
        _ => None,
    }
}

/// Whether a typed array has an element at an index now (IsValidIntegerIndex).
fn has_index(vm: &Vm, g: Gc, i: usize) -> bool {
    matches!(&vm.heap.get(g).class, Class::TypedArray(t) if !vm.typed_out_of_bounds(t) && i < vm.typed_length(t))
}

/// ValidateTypedArray: (object, kind, length).
fn validate(vm: &mut Vm, this: &Value, what: &str) -> Result<(Gc, ElemKind, usize), Value> {
    if let Value::Obj(g) = this
        && let Some((kind, _, _, len, out)) = parts(vm, *g)
    {
        if out {
            return Err(vm.type_error(&format!("Cannot perform {what} on a detached or out-of-bounds TypedArray")));
        }
        return Ok((*g, kind, len));
    }
    Err(vm.type_error(&format!("{what}: this is not a typed array")))
}

/// A new typed array of a kind over a new buffer of `len` elements (prototype from new.target).
fn allocate(vm: &mut Vm, kind: ElemKind, nt: &Value, default_proto: I, len: usize) -> Result<Gc, Value> {
    let p = vm.proto_from_constructor(nt, default_proto)?;
    make(vm, kind, p, len)
}

/// A new typed array of a kind with a prototype, over a new buffer of `len` elements.
/// A new Uint8ClampedArray holding `bytes` (for the embedder: a canvas's ImageData).
pub fn clamped_array(vm: &mut Vm, bytes: Vec<u8>) -> Gc {
    let len = bytes.len();
    let abp = vm.proto(I::ArrayBufferProto);
    let buf = vm.alloc(Some(abp), Class::ArrayBuffer(Some(bytes), None));
    let p = vm.proto(I::Uint8ClampedArrayProto);
    vm.alloc(Some(p), Class::TypedArray(Box::new(TypedArray { kind: ElemKind::Uint8Clamped, buffer: buf, offset: 0, length: Some(len) })))
}

/// The bytes a typed array (or an ArrayBuffer) holds, if it's one and they're there (not detached, in bounds).
pub fn bytes_of(vm: &Vm, g: Gc) -> Option<&[u8]> {
    match &vm.heap.get(g).class {
        Class::TypedArray(t) => {
            if vm.typed_out_of_bounds(t) {
                return None;
            }
            let Class::ArrayBuffer(Some(data), _) = &vm.heap.get(t.buffer).class else { return None };
            let len = vm.typed_length(t) * t.kind.size();
            data.get(t.offset..t.offset + len)
        }
        Class::ArrayBuffer(Some(data), _) => Some(data),
        _ => None,
    }
}

fn make(vm: &mut Vm, kind: ElemKind, p: Gc, len: usize) -> Result<Gc, Value> {
    let bytes = (len as u64).saturating_mul(kind.size() as u64);
    let data = zeroed(vm, bytes)?;
    let abp = vm.proto(I::ArrayBufferProto);
    let buf = vm.alloc(Some(abp), Class::ArrayBuffer(Some(data), None));
    Ok(vm.alloc(Some(p), Class::TypedArray(Box::new(TypedArray { kind, buffer: buf, offset: 0, length: Some(len) }))))
}

fn get_index(vm: &mut Vm, g: Gc, i: usize) -> Value {
    match &vm.heap.get(g).class {
        Class::TypedArray(t) => vm.typed_get(t, i as u32).unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    }
}

fn kind_construct(vm: &mut Vm, nt: &Value, args: &[Value], f: Gc) -> R {
    let ki = match vm.data(f) {
        Value::Num(i) => i as usize,
        _ => 0,
    };
    let (kind, _, proto) = KINDS[ki];
    let first = arg(args, 0);
    let Value::Obj(src) = first else {
        let len = vm.to_index(&first)?;
        if len > MAX_BYTES {
            return Err(vm.range_error("Invalid typed array length"));
        }
        return Ok(Value::Obj(allocate(vm, kind, nt, proto, len as usize)?));
    };
    let p = vm.proto_from_constructor(nt, proto)?;
    // From a buffer.
    if let Class::ArrayBuffer(..) = vm.heap.get(src).class {
        let offset = vm.to_index(&arg(args, 1))? as usize;
        if offset % kind.size() != 0 {
            return Err(vm.range_error(&format!("start offset of {} should be a multiple of {}", KINDS[ki].1, kind.size())));
        }
        let len_arg = arg(args, 2);
        let new_len = if len_arg.is_undefined() { None } else { Some(vm.to_index(&len_arg)? as usize) };
        let (blen, resizable) = match &vm.heap.get(src).class {
            Class::ArrayBuffer(Some(d), m) => (d.len(), m.is_some()),
            _ => return Err(vm.type_error("Cannot construct a typed array on a detached ArrayBuffer")),
        };
        let length = match new_len {
            None if resizable => {
                if offset > blen {
                    return Err(vm.range_error("Start offset is outside the bounds of the buffer"));
                }
                None
            }
            None => {
                if blen % kind.size() != 0 {
                    return Err(vm.range_error(&format!("byte length of {} should be a multiple of {}", KINDS[ki].1, kind.size())));
                }
                if offset > blen {
                    return Err(vm.range_error("Start offset is outside the bounds of the buffer"));
                }
                Some((blen - offset) / kind.size())
            }
            Some(l) => {
                if offset + l * kind.size() > blen {
                    return Err(vm.range_error("Invalid typed array length"));
                }
                Some(l)
            }
        };
        return Ok(Value::Obj(vm.alloc(Some(p), Class::TypedArray(Box::new(TypedArray { kind, buffer: src, offset, length })))));
    }
    // From another typed array.
    if let Some((sk, _, _, slen, out)) = parts(vm, src) {
        if out {
            return Err(vm.type_error("Cannot construct from a detached or out-of-bounds typed array"));
        }
        if is_big(sk) != is_big(kind) {
            return Err(vm.type_error("Content types differ (BigInt and Number)"));
        }
        let a = make(vm, kind, p, slen)?;
        for i in 0..slen {
            let v = get_index(vm, src, i);
            vm.typed_put(a, i, &v);
        }
        return Ok(Value::Obj(a));
    }
    // From an iterable or an array-like.
    let it_sym = vm.sym(Sym::Iterator);
    let using = vm.get_method(&first, Key::Sym(it_sym))?;
    if using.is_undefined() {
        let len = vm.length_of(src)?;
        if len > MAX_BYTES {
            return Err(vm.range_error("Invalid typed array length"));
        }
        let a = make(vm, kind, p, len as usize)?;
        for i in 0..len {
            let v = vm.get(src, Key::Index(i as u32), &first)?;
            vm.typed_set(a, i as u32, &v)?;
        }
        return Ok(Value::Obj(a));
    }
    let values = if vm.plain_array_iteration(src) {
        vm.array_values(&first)
    } else {
        let it = vm.call(&using, first.clone(), &[])?;
        if !it.is_object() {
            return Err(vm.type_error("Result of the Symbol.iterator method is not an object"));
        }
        let next = vm.get_value(&it, Key::Str(n::NEXT))?;
        let mut out = Vec::new();
        while let Some(v) = vm.iter_step(&it, &next)? {
            out.push(v);
        }
        out
    };
    let a = make(vm, kind, p, values.len())?;
    for (i, v) in values.iter().enumerate() {
        vm.typed_set(a, i as u32, v)?;
    }
    Ok(Value::Obj(a))
}

/// TypedArrayCreateFromConstructor (with a length): the new one must be a typed array, long enough.
fn create_from(vm: &mut Vm, c: &Value, args: &[Value], min_len: Option<usize>) -> Result<Gc, Value> {
    let r = vm.construct(c, args, c)?;
    let Value::Obj(g) = r else { return Err(vm.type_error("not a typed array")) };
    let (_, _, len) = validate(vm, &r, "TypedArray creation")?;
    if let Some(m) = min_len
        && len < m
    {
        return Err(vm.type_error("The derived typed array is too short"));
    }
    Ok(g)
}

/// TypedArraySpeciesCreate.
fn species_create(vm: &mut Vm, exemplar: Gc, args: &[Value], min_len: Option<usize>) -> Result<Gc, Value> {
    let (kind, ..) = parts(vm, exemplar).ok_or_else(|| vm.type_error("not a typed array"))?;
    let default = Value::Obj(vm.intrinsics[I::Int8ArrayCtor as usize + kind_index(kind)]);
    let c = vm.get(exemplar, Key::Str(n::CONSTRUCTOR), &Value::Obj(exemplar))?;
    let c = if c.is_undefined() {
        default
    } else {
        let Value::Obj(cg) = c else { return Err(vm.type_error("constructor is not an object")) };
        let sp = vm.sym(Sym::Species);
        let s = vm.get(cg, Key::Sym(sp), &c)?;
        if s.is_nullish() {
            default
        } else if vm.is_constructor(&s) {
            s
        } else {
            return Err(vm.type_error("Symbol.species is not a constructor"));
        }
    };
    let r = create_from(vm, &c, args, min_len)?;
    let (rk, ..) = parts(vm, r).ok_or_else(|| vm.type_error("not a typed array"))?;
    if is_big(rk) != is_big(kind) {
        return Err(vm.type_error("Content types differ (BigInt and Number)"));
    }
    Ok(r)
}

fn ta_from(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if !vm.is_constructor(this) {
        return Err(vm.type_error("TypedArray.from: this is not a constructor"));
    }
    let map_fn = arg(args, 1);
    let mapping = !map_fn.is_undefined();
    if mapping && !vm.is_callable(&map_fn) {
        return Err(vm.type_error("TypedArray.from: mapFn is not a function"));
    }
    let t = arg(args, 2);
    let src = arg(args, 0);
    let it_sym = vm.sym(Sym::Iterator);
    let using = vm.get_method(&src, Key::Sym(it_sym))?;
    if using.is_undefined() {
        let o = vm.to_object(&src)?;
        let len = vm.length_of(o)?;
        let a = create_from(vm, this, &[Value::Num(len as f64)], Some(len as usize))?;
        for k in 0..len {
            let v = vm.get(o, Key::Index(k as u32), &Value::Obj(o))?;
            let v = if mapping { vm.call(&map_fn, t.clone(), &[v, Value::Num(k as f64)])? } else { v };
            vm.set(a, Key::Index(k as u32), v, &Value::Obj(a))?;
        }
        return Ok(Value::Obj(a));
    }
    let values = {
        let it = vm.call(&using, src.clone(), &[])?;
        if !it.is_object() {
            return Err(vm.type_error("Result of the Symbol.iterator method is not an object"));
        }
        let next = vm.get_value(&it, Key::Str(n::NEXT))?;
        let mut out = Vec::new();
        while let Some(v) = vm.iter_step(&it, &next)? {
            out.push(v);
        }
        out
    };
    let a = create_from(vm, this, &[Value::Num(values.len() as f64)], Some(values.len()))?;
    for (k, v) in values.into_iter().enumerate() {
        let v = if mapping { vm.call(&map_fn, t.clone(), &[v, Value::Num(k as f64)])? } else { v };
        vm.set(a, Key::Index(k as u32), v, &Value::Obj(a))?;
    }
    Ok(Value::Obj(a))
}

fn ta_of(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if !vm.is_constructor(this) {
        return Err(vm.type_error("TypedArray.of: this is not a constructor"));
    }
    let a = create_from(vm, this, &[Value::Num(args.len() as f64)], Some(args.len()))?;
    for (k, v) in args.iter().enumerate() {
        vm.set(a, Key::Index(k as u32), v.clone(), &Value::Obj(a))?;
    }
    Ok(Value::Obj(a))
}

fn this_ta(vm: &mut Vm, this: &Value, what: &str) -> Result<(Gc, ElemKind, Gc, usize, usize, bool), Value> {
    if let Value::Obj(g) = this
        && let Some((k, b, off, len, out)) = parts(vm, *g)
    {
        return Ok((*g, k, b, off, len, out));
    }
    Err(vm.type_error(&format!("%TypedArray%.prototype.{what} called on incompatible receiver")))
}

fn ta_buffer(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, _, b, ..) = this_ta(vm, this, "buffer")?;
    Ok(Value::Obj(b))
}

fn ta_byte_length(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, k, _, _, len, _) = this_ta(vm, this, "byteLength")?;
    Ok(Value::Num((len * k.size()) as f64))
}

fn ta_byte_offset(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, _, _, off, _, out) = this_ta(vm, this, "byteOffset")?;
    Ok(Value::Num(if out { 0.0 } else { off as f64 }))
}

fn ta_length(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, _, _, _, len, _) = this_ta(vm, this, "length")?;
    Ok(Value::Num(len as f64))
}

fn ta_tag(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    match this {
        Value::Obj(g) => match parts(vm, *g) {
            Some((k, ..)) => Ok(js_str(KINDS[kind_index(k)].1)),
            None => Ok(Value::Undefined),
        },
        _ => Ok(Value::Undefined),
    }
}

fn ta_at(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, _, len) = validate(vm, this, "%TypedArray%.prototype.at")?;
    let rel = vm.to_integer(&arg(args, 0))?;
    let k = if rel >= 0.0 { rel } else { len as f64 + rel };
    if k < 0.0 || k >= len as f64 {
        return Ok(Value::Undefined);
    }
    Ok(get_index(vm, g, k as usize))
}

fn ta_copy_within(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, kind, len) = validate(vm, this, "%TypedArray%.prototype.copyWithin")?;
    let to = vm.relative_index(&arg(args, 0), len as u64, 0)? as usize;
    let from = vm.relative_index(&arg(args, 1), len as u64, 0)? as usize;
    let fin = vm.relative_index(&arg(args, 2), len as u64, len as u64)? as usize;
    let count = fin.saturating_sub(from).min(len.saturating_sub(to));
    if count > 0 {
        let (_, kind2, len2) = validate(vm, this, "%TypedArray%.prototype.copyWithin")?;
        let _ = kind2;
        let count = count.min(len2.saturating_sub(from)).min(len2.saturating_sub(to));
        let (buf, off) = match &vm.heap.get(g).class {
            Class::TypedArray(t) => (t.buffer, t.offset),
            _ => return Ok(this.clone()),
        };
        let sz = kind.size();
        if let Class::ArrayBuffer(Some(d), _) = &mut vm.heap.get_mut(buf).class {
            d.copy_within(off + from * sz..off + (from + count) * sz, off + to * sz);
        }
    }
    Ok(this.clone())
}

fn ta_entries(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    validate(vm, this, "%TypedArray%.prototype.entries")?;
    Ok(array_iterator(vm, this.clone(), 2))
}

fn ta_keys(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    validate(vm, this, "%TypedArray%.prototype.keys")?;
    Ok(array_iterator(vm, this.clone(), 0))
}

fn ta_values(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    validate(vm, this, "%TypedArray%.prototype.values")?;
    Ok(array_iterator(vm, this.clone(), 1))
}

fn cb(vm: &mut Vm, this: &Value, args: &[Value], what: &str) -> Result<(Gc, usize, Value, Value), Value> {
    let (g, _, len) = validate(vm, this, what)?;
    let f = callable_arg(vm, args, 0)?;
    Ok((g, len, f, arg(args, 1)))
}

fn ta_every(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, len, f, t) = cb(vm, this, args, "%TypedArray%.prototype.every")?;
    for k in 0..len {
        let v = get_index(vm, g, k);
        if !vm.call(&f, t.clone(), &[v, Value::Num(k as f64), this.clone()])?.truthy() {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn ta_some(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, len, f, t) = cb(vm, this, args, "%TypedArray%.prototype.some")?;
    for k in 0..len {
        let v = get_index(vm, g, k);
        if vm.call(&f, t.clone(), &[v, Value::Num(k as f64), this.clone()])?.truthy() {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn ta_for_each(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, len, f, t) = cb(vm, this, args, "%TypedArray%.prototype.forEach")?;
    for k in 0..len {
        let v = get_index(vm, g, k);
        vm.call(&f, t.clone(), &[v, Value::Num(k as f64), this.clone()])?;
    }
    Ok(Value::Undefined)
}

fn find_impl(vm: &mut Vm, this: &Value, args: &[Value], last: bool, index: bool) -> R {
    let (g, len, f, t) = cb(vm, this, args, "%TypedArray%.prototype.find")?;
    for i in 0..len {
        let k = if last { len - 1 - i } else { i };
        let v = get_index(vm, g, k);
        if vm.call(&f, t.clone(), &[v.clone(), Value::Num(k as f64), this.clone()])?.truthy() {
            return Ok(if index { Value::Num(k as f64) } else { v });
        }
    }
    Ok(if index { Value::Num(-1.0) } else { Value::Undefined })
}

fn ta_find(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    find_impl(vm, this, args, false, false)
}
fn ta_find_index(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    find_impl(vm, this, args, false, true)
}
fn ta_find_last(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    find_impl(vm, this, args, true, false)
}
fn ta_find_last_index(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    find_impl(vm, this, args, true, true)
}

fn ta_fill(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, kind, len) = validate(vm, this, "%TypedArray%.prototype.fill")?;
    let v = if is_big(kind) { Value::Big(Rc::new(vm.to_bigint(&arg(args, 0))?)) } else { Value::Num(vm.to_number(&arg(args, 0))?) };
    let from = vm.relative_index(&arg(args, 1), len as u64, 0)? as usize;
    let to = vm.relative_index(&arg(args, 2), len as u64, len as u64)? as usize;
    let (_, _, len2) = validate(vm, this, "%TypedArray%.prototype.fill")?;
    for k in from..to.min(len2) {
        vm.typed_put(g, k, &v);
    }
    Ok(this.clone())
}

fn ta_filter(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, len, f, t) = cb(vm, this, args, "%TypedArray%.prototype.filter")?;
    let mut kept = Vec::new();
    for k in 0..len {
        let v = get_index(vm, g, k);
        if vm.call(&f, t.clone(), &[v.clone(), Value::Num(k as f64), this.clone()])?.truthy() {
            kept.push(v);
        }
    }
    let a = species_create(vm, g, &[Value::Num(kept.len() as f64)], Some(kept.len()))?;
    for (i, v) in kept.into_iter().enumerate() {
        vm.set(a, Key::Index(i as u32), v, &Value::Obj(a))?;
    }
    Ok(Value::Obj(a))
}

fn ta_includes(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, _, len) = validate(vm, this, "%TypedArray%.prototype.includes")?;
    if len == 0 {
        return Ok(Value::Bool(false));
    }
    let from = vm.relative_index(&arg(args, 1), len as u64, 0)? as usize;
    let target = arg(args, 0);
    for k in from..len {
        let v = get_index(vm, g, k);
        if same_value_zero(&v, &target) {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn ta_index_of(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, _, len) = validate(vm, this, "%TypedArray%.prototype.indexOf")?;
    if len == 0 {
        return Ok(Value::Num(-1.0));
    }
    let from = vm.relative_index(&arg(args, 1), len as u64, 0)? as usize;
    let target = arg(args, 0);
    for k in from..len {
        if !has_index(vm, g, k) {
            continue;
        }
        let v = get_index(vm, g, k);
        if strict_equals(&v, &target) {
            return Ok(Value::Num(k as f64));
        }
    }
    Ok(Value::Num(-1.0))
}

fn ta_last_index_of(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, _, len) = validate(vm, this, "%TypedArray%.prototype.lastIndexOf")?;
    if len == 0 {
        return Ok(Value::Num(-1.0));
    }
    let from = if args.len() > 1 {
        let x = vm.to_integer(&args[1])?;
        if x < 0.0 { len as f64 + x } else { x.min(len as f64 - 1.0) }
    } else {
        len as f64 - 1.0
    };
    let target = arg(args, 0);
    let mut k = from as i64;
    while k >= 0 {
        if has_index(vm, g, k as usize) {
            let v = get_index(vm, g, k as usize);
            if strict_equals(&v, &target) {
                return Ok(Value::Num(k as f64));
            }
        }
        k -= 1;
    }
    Ok(Value::Num(-1.0))
}

fn ta_join(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, _, len) = validate(vm, this, "%TypedArray%.prototype.join")?;
    let sep = arg(args, 0);
    let sep = if sep.is_undefined() { crate::string::JsStr::from_str(",") } else { vm.to_string(&sep)? };
    let mut units: Vec<u16> = Vec::new();
    for k in 0..len {
        if k > 0 {
            units.extend_from_slice(sep.units());
        }
        let v = get_index(vm, g, k);
        if !v.is_undefined() {
            units.extend_from_slice(vm.to_string(&v)?.units());
        }
    }
    Ok(Value::Str(crate::string::JsStr::from_units(units)))
}

fn ta_to_locale_string(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, _, len) = validate(vm, this, "%TypedArray%.prototype.toLocaleString")?;
    let mut units: Vec<u16> = Vec::new();
    for k in 0..len {
        if k > 0 {
            units.push(b',' as u16);
        }
        let v = get_index(vm, g, k);
        if !v.is_nullish() {
            let a = vm.atom("toLocaleString");
            let m = vm.get_value(&v, Key::Str(a))?;
            let r = vm.call(&m, v, &[arg(args, 0), arg(args, 1)])?;
            units.extend_from_slice(vm.to_string(&r)?.units());
        }
    }
    Ok(Value::Str(crate::string::JsStr::from_units(units)))
}

fn ta_map(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, len, f, t) = cb(vm, this, args, "%TypedArray%.prototype.map")?;
    let a = species_create(vm, g, &[Value::Num(len as f64)], Some(len))?;
    for k in 0..len {
        let v = get_index(vm, g, k);
        let r = vm.call(&f, t.clone(), &[v, Value::Num(k as f64), this.clone()])?;
        vm.set(a, Key::Index(k as u32), r, &Value::Obj(a))?;
    }
    Ok(Value::Obj(a))
}

fn reduce_impl(vm: &mut Vm, this: &Value, args: &[Value], right: bool) -> R {
    let (g, _, len) = validate(vm, this, "%TypedArray%.prototype.reduce")?;
    let f = callable_arg(vm, args, 0)?;
    let order = |i: usize| if right { len - 1 - i } else { i };
    let mut i = 0;
    let mut acc = if args.len() > 1 {
        args[1].clone()
    } else {
        if len == 0 {
            return Err(vm.type_error("Reduce of empty array with no initial value"));
        }
        i = 1;
        get_index(vm, g, order(0))
    };
    while i < len {
        let k = order(i);
        let v = get_index(vm, g, k);
        acc = vm.call(&f, Value::Undefined, &[acc, v, Value::Num(k as f64), this.clone()])?;
        i += 1;
    }
    Ok(acc)
}

fn ta_reduce(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    reduce_impl(vm, this, args, false)
}

fn ta_reduce_right(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    reduce_impl(vm, this, args, true)
}

fn ta_reverse(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (g, _, len) = validate(vm, this, "%TypedArray%.prototype.reverse")?;
    for k in 0..len / 2 {
        let (a, b) = (get_index(vm, g, k), get_index(vm, g, len - 1 - k));
        vm.typed_put(g, k, &b);
        vm.typed_put(g, len - 1 - k, &a);
    }
    Ok(this.clone())
}

fn ta_set(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(g) = this else { return Err(vm.type_error("%TypedArray%.prototype.set called on incompatible receiver")) };
    let g = *g;
    if parts(vm, g).is_none() {
        return Err(vm.type_error("%TypedArray%.prototype.set called on incompatible receiver"));
    }
    let off = vm.to_integer(&arg(args, 1))?;
    if off < 0.0 {
        return Err(vm.range_error("offset is out of bounds"));
    }
    let (kind, _, _, len, out) = parts(vm, g).unwrap_or((ElemKind::Uint8, g, 0, 0, true));
    if out {
        return Err(vm.type_error("Cannot perform %TypedArray%.prototype.set on a detached ArrayBuffer"));
    }
    let src = arg(args, 0);
    if let Value::Obj(s) = src
        && let Some((sk, _, _, slen, sout)) = parts(vm, s)
    {
        if sout {
            return Err(vm.type_error("Cannot perform %TypedArray%.prototype.set on a detached ArrayBuffer"));
        }
        if is_big(sk) != is_big(kind) {
            return Err(vm.type_error("Content types differ (BigInt and Number)"));
        }
        if off + slen as f64 > len as f64 {
            return Err(vm.range_error("offset is out of bounds"));
        }
        let values: Vec<Value> = (0..slen).map(|i| get_index(vm, s, i)).collect();
        for (i, v) in values.iter().enumerate() {
            vm.typed_put(g, off as usize + i, v);
        }
        return Ok(Value::Undefined);
    }
    let o = vm.to_object(&src)?;
    let slen = vm.length_of(o)?;
    if off + slen as f64 > len as f64 {
        return Err(vm.range_error("offset is out of bounds"));
    }
    for i in 0..slen {
        let v = vm.get(o, Key::Index(i as u32), &Value::Obj(o))?;
        vm.typed_set(g, off as u32 + i as u32, &v)?;
    }
    Ok(Value::Undefined)
}

fn ta_slice(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, kind, len) = validate(vm, this, "%TypedArray%.prototype.slice")?;
    let k = vm.relative_index(&arg(args, 0), len as u64, 0)? as usize;
    let fin = vm.relative_index(&arg(args, 1), len as u64, len as u64)? as usize;
    let count = fin.saturating_sub(k);
    let a = species_create(vm, g, &[Value::Num(count as f64)], Some(count))?;
    if count > 0 {
        let (_, _, len2) = validate(vm, this, "%TypedArray%.prototype.slice")?;
        let end = fin.min(len2);
        let (ak, ..) = parts(vm, a).unwrap_or((kind, a, 0, 0, true));
        if ak == kind {
            let (src_buf, src_off) = match &vm.heap.get(g).class {
                Class::TypedArray(t) => (t.buffer, t.offset),
                _ => return Ok(Value::Obj(a)),
            };
            let (dst_buf, dst_off) = match &vm.heap.get(a).class {
                Class::TypedArray(t) => (t.buffer, t.offset),
                _ => return Ok(Value::Obj(a)),
            };
            let sz = kind.size();
            if src_buf == dst_buf {
                if let Class::ArrayBuffer(Some(d), _) = &mut vm.heap.get_mut(src_buf).class {
                    let (mut from, mut to) = (src_off + k * sz, dst_off);
                    let limit = (dst_off + (end.saturating_sub(k)) * sz).min(d.len());
                    while to < limit && from < d.len() {
                        d[to] = d[from];
                        from += 1;
                        to += 1;
                    }
                }
                return Ok(Value::Obj(a));
            }
            let bytes: Vec<u8> = match &vm.heap.get(src_buf).class {
                Class::ArrayBuffer(Some(d), _) => {
                    let hi = (src_off + end.max(k) * sz).min(d.len());
                    d[(src_off + k * sz).min(hi)..hi].to_vec()
                }
                _ => Vec::new(),
            };
            if let Class::ArrayBuffer(Some(d), _) = &mut vm.heap.get_mut(dst_buf).class {
                let n = bytes.len().min(d.len().saturating_sub(dst_off));
                d[dst_off..dst_off + n].copy_from_slice(&bytes[..n]);
            }
        } else {
            for (n, i) in (k..end).enumerate() {
                let v = get_index(vm, g, i);
                vm.set(a, Key::Index(n as u32), v, &Value::Obj(a))?;
            }
        }
    }
    Ok(Value::Obj(a))
}

/// The default order of a typed array's elements (numbers with -0 before +0 and NaN last; BigInts).
fn numeric_order(a: &Value, b: &Value) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => {
            if x.is_nan() && y.is_nan() {
                Ordering::Equal
            } else if x.is_nan() {
                Ordering::Greater
            } else if y.is_nan() {
                Ordering::Less
            } else if x == y {
                match (x.is_sign_negative(), y.is_sign_negative()) {
                    (true, false) if *x == 0.0 => Ordering::Less,
                    (false, true) if *x == 0.0 => Ordering::Greater,
                    _ => Ordering::Equal,
                }
            } else {
                x.partial_cmp(y).unwrap_or(Ordering::Equal)
            }
        }
        (Value::Big(x), Value::Big(y)) => x.cmp(y),
        _ => Ordering::Equal,
    }
}

fn sorted(vm: &mut Vm, g: Gc, len: usize, cmp: &Value) -> Result<Vec<Value>, Value> {
    let mut items: Vec<Value> = (0..len).map(|i| get_index(vm, g, i)).collect();
    if cmp.is_undefined() {
        items.sort_by(numeric_order);
    } else {
        // (The comparator's results as numbers: undefined values never occur in typed arrays.)
        merge_sort(vm, &mut items, cmp)?;
    }
    Ok(items)
}

fn ta_sort(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let cmp = arg(args, 0);
    if !cmp.is_undefined() && !vm.is_callable(&cmp) {
        return Err(vm.type_error("The comparison function must be either a function or undefined"));
    }
    let (g, _, len) = validate(vm, this, "%TypedArray%.prototype.sort")?;
    let items = sorted(vm, g, len, &cmp)?;
    for (i, v) in items.iter().enumerate() {
        vm.typed_put(g, i, v);
    }
    Ok(this.clone())
}

fn ta_to_sorted(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let cmp = arg(args, 0);
    if !cmp.is_undefined() && !vm.is_callable(&cmp) {
        return Err(vm.type_error("The comparison function must be either a function or undefined"));
    }
    let (g, kind, len) = validate(vm, this, "%TypedArray%.prototype.toSorted")?;
    let proto = KINDS[kind_index(kind)].2;
    let a = allocate(vm, kind, &Value::Undefined, proto, len)?;
    let items = sorted(vm, g, len, &cmp)?;
    for (i, v) in items.iter().enumerate() {
        vm.typed_put(a, i, v);
    }
    Ok(Value::Obj(a))
}

fn ta_to_reversed(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (g, kind, len) = validate(vm, this, "%TypedArray%.prototype.toReversed")?;
    let proto = KINDS[kind_index(kind)].2;
    let a = allocate(vm, kind, &Value::Undefined, proto, len)?;
    for k in 0..len {
        let v = get_index(vm, g, len - 1 - k);
        vm.typed_put(a, k, &v);
    }
    Ok(Value::Obj(a))
}

fn ta_with(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, kind, len) = validate(vm, this, "%TypedArray%.prototype.with")?;
    let rel = vm.to_integer(&arg(args, 0))?;
    let idx = if rel >= 0.0 { rel } else { len as f64 + rel };
    let v = if is_big(kind) { Value::Big(Rc::new(vm.to_bigint(&arg(args, 1))?)) } else { Value::Num(vm.to_number(&arg(args, 1))?) };
    let (_, _, len2) = validate(vm, this, "%TypedArray%.prototype.with")?;
    if idx < 0.0 || idx >= len2 as f64 {
        return Err(vm.range_error("Invalid typed array index"));
    }
    let proto = KINDS[kind_index(kind)].2;
    let a = allocate(vm, kind, &Value::Undefined, proto, len)?;
    for k in 0..len {
        let x = if k == idx as usize { v.clone() } else { get_index(vm, g, k) };
        vm.typed_set(a, k as u32, &x)?;
    }
    Ok(Value::Obj(a))
}

fn ta_subarray(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, kind, buf, off, len, _) = this_ta(vm, this, "subarray")?;
    let begin = vm.relative_index(&arg(args, 0), len as u64, 0)? as usize;
    let tracking = matches!(&vm.heap.get(g).class, Class::TypedArray(t) if t.length.is_none());
    let end_arg = arg(args, 1);
    let mut new_args = vec![Value::Obj(buf), Value::Num((off + begin * kind.size()) as f64)];
    if !(tracking && end_arg.is_undefined()) {
        let fin = vm.relative_index(&end_arg, len as u64, len as u64)? as usize;
        new_args.push(Value::Num(fin.saturating_sub(begin) as f64));
    }
    let a = species_create(vm, g, &new_args, None)?;
    Ok(Value::Obj(a))
}
