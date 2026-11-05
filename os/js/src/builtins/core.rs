//! Object, Function, the errors, Symbol, Boolean, Reflect, Proxy, and the global functions (eval, the URI ones).

use super::{arg, callable_arg, constructor, js_str, sym_method, tag};
use crate::names as n;
use crate::object::*;
use crate::props::Desc;
use crate::string::JsStr;
use crate::value::{Gc, Value, same_value};
use crate::vm::{ErrorKind, I, SYMBOL_NAMES, Sym, Vm};
use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

type R = Result<Value, Value>;

pub fn init(vm: &mut Vm) {
    object(vm);
    function(vm);
    errors(vm);
    symbol(vm);
    boolean(vm);
    reflect(vm);
    proxy(vm);
    globals(vm);
}

/// CreateListFromArrayLike.
pub fn list_from_array_like(vm: &mut Vm, v: &Value) -> Result<Vec<Value>, Value> {
    let Value::Obj(g) = v else { return Err(vm.type_error("CreateListFromArrayLike called on non-object")) };
    let g = *g;
    if let Class::Array(..) = vm.heap.get(g).class {
        return Ok(vm.array_values(v));
    }
    let len = vm.length_of(g)?;
    let mut out = Vec::with_capacity(len.min(1 << 16) as usize);
    for i in 0..len {
        out.push(vm.get(g, Key::Index(i as u32), v)?);
    }
    Ok(out)
}

// ------------------------------------------------------------------------------------------------------- Object

fn object(vm: &mut Vm) {
    let op = vm.proto(I::ObjectProto);
    let c = constructor(vm, "Object", 1, object_call, Some(object_construct), Some(op));
    vm.intrinsics[I::ObjectCtor as usize] = c;
    vm.method(c, "assign", 2, object_assign);
    vm.method(c, "create", 2, object_create);
    vm.method(c, "defineProperty", 3, object_define_property);
    vm.method(c, "defineProperties", 2, object_define_properties);
    vm.method(c, "entries", 1, object_entries);
    vm.method(c, "keys", 1, object_keys);
    vm.method(c, "values", 1, object_values);
    vm.method(c, "freeze", 1, object_freeze);
    vm.method(c, "isFrozen", 1, object_is_frozen);
    vm.method(c, "seal", 1, object_seal);
    vm.method(c, "isSealed", 1, object_is_sealed);
    vm.method(c, "preventExtensions", 1, object_prevent_extensions);
    vm.method(c, "isExtensible", 1, object_is_extensible);
    vm.method(c, "fromEntries", 1, object_from_entries);
    vm.method(c, "getOwnPropertyDescriptor", 2, object_gopd);
    vm.method(c, "getOwnPropertyDescriptors", 1, object_gopds);
    vm.method(c, "getOwnPropertyNames", 1, object_names);
    vm.method(c, "getOwnPropertySymbols", 1, object_symbols);
    vm.method(c, "getPrototypeOf", 1, object_get_proto);
    vm.method(c, "setPrototypeOf", 2, object_set_proto);
    vm.method(c, "is", 2, object_is);
    vm.method(c, "hasOwn", 2, object_has_own);
    vm.method(c, "groupBy", 2, object_group_by);
    vm.method(op, "hasOwnProperty", 1, proto_has_own);
    vm.method(op, "isPrototypeOf", 1, proto_is_prototype_of);
    vm.method(op, "propertyIsEnumerable", 1, proto_property_is_enumerable);
    vm.method(op, "toString", 0, proto_to_string);
    vm.method(op, "toLocaleString", 0, proto_to_locale_string);
    vm.method(op, "valueOf", 0, proto_value_of);
    vm.method(op, "__defineGetter__", 2, proto_define_getter);
    vm.method(op, "__defineSetter__", 2, proto_define_setter);
    vm.method(op, "__lookupGetter__", 1, proto_lookup_getter);
    vm.method(op, "__lookupSetter__", 1, proto_lookup_setter);
    vm.accessor(op, Key::Str(n::PROTO), "__proto__", Some(proto_get_proto), Some(proto_set_proto));
}

fn object_call(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    object_construct(vm, &Value::Obj(f), args, f)
}

fn object_construct(vm: &mut Vm, nt: &Value, args: &[Value], f: Gc) -> R {
    if let Value::Obj(t) = nt
        && *t != f
    {
        return Ok(Value::Obj(vm.create_from_constructor(nt, I::ObjectProto)?));
    }
    let v = arg(args, 0);
    if v.is_nullish() {
        return Ok(Value::Obj(vm.new_object()));
    }
    Ok(Value::Obj(vm.to_object(&v)?))
}

fn object_assign(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let to = vm.to_object(&arg(args, 0))?;
    for src in args.iter().skip(1) {
        if src.is_nullish() {
            continue;
        }
        let from = vm.to_object(src)?;
        for k in vm.own_keys(from)? {
            if let Some(p) = vm.get_own_property(from, k)?
                && p.enumerable()
            {
                let v = vm.get(from, k, &Value::Obj(from))?;
                if !vm.set(to, k, v, &Value::Obj(to))? {
                    return Err(vm.type_error("Cannot assign to read only property"));
                }
            }
        }
    }
    Ok(Value::Obj(to))
}

fn object_create(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let proto = match arg(args, 0) {
        Value::Obj(p) => Some(p),
        Value::Null => None,
        _ => return Err(vm.type_error("Object prototype may only be an Object or null")),
    };
    let o = vm.alloc(proto, Class::Ordinary);
    let props = arg(args, 1);
    if !props.is_undefined() {
        define_properties(vm, o, &props)?;
    }
    Ok(Value::Obj(o))
}

fn define_properties(vm: &mut Vm, o: Gc, props: &Value) -> Result<(), Value> {
    let p = vm.to_object(props)?;
    let mut descs = Vec::new();
    for k in vm.own_keys(p)? {
        if let Some(pd) = vm.get_own_property(p, k)?
            && pd.enumerable()
        {
            let dv = vm.get(p, k, &Value::Obj(p))?;
            let d = vm.to_descriptor(&dv)?;
            descs.push((k, d));
        }
    }
    for (k, d) in descs {
        vm.define_own_property_or_throw(o, k, d)?;
    }
    Ok(())
}

fn object_define_property(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(o) = arg(args, 0) else { return Err(vm.type_error("Object.defineProperty called on non-object")) };
    let k = vm.to_key(&arg(args, 1))?;
    let d = vm.to_descriptor(&arg(args, 2))?;
    vm.define_own_property_or_throw(o, k, d)?;
    Ok(Value::Obj(o))
}

fn object_define_properties(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(o) = arg(args, 0) else { return Err(vm.type_error("Object.defineProperties called on non-object")) };
    define_properties(vm, o, &arg(args, 1))?;
    Ok(Value::Obj(o))
}

/// EnumerableOwnProperties: 0 keys, 1 values, 2 entries.
pub fn enumerable_own(vm: &mut Vm, o: Gc, kind: u8) -> Result<Vec<Value>, Value> {
    let mut out = Vec::new();
    for k in vm.own_keys(o)? {
        if matches!(k, Key::Sym(_)) {
            continue;
        }
        if let Some(p) = vm.get_own_property(o, k)?
            && p.enumerable()
        {
            let kv = vm.key_value(k);
            match kind {
                0 => out.push(kv),
                1 => out.push(vm.get(o, k, &Value::Obj(o))?),
                _ => {
                    let v = vm.get(o, k, &Value::Obj(o))?;
                    out.push(Value::Obj(vm.new_array(alloc::vec![kv, v])));
                }
            }
        }
    }
    Ok(out)
}

fn object_keys(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(&arg(args, 0))?;
    let v = enumerable_own(vm, o, 0)?;
    Ok(Value::Obj(vm.new_array(v)))
}

fn object_values(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(&arg(args, 0))?;
    let v = enumerable_own(vm, o, 1)?;
    Ok(Value::Obj(vm.new_array(v)))
}

fn object_entries(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(&arg(args, 0))?;
    let v = enumerable_own(vm, o, 2)?;
    Ok(Value::Obj(vm.new_array(v)))
}

/// SetIntegrityLevel (frozen: sealed and every data property read-only).
pub fn set_integrity(vm: &mut Vm, o: Gc, frozen: bool) -> Result<bool, Value> {
    if !vm.prevent_extensions(o)? {
        return Ok(false);
    }
    for k in vm.own_keys(o)? {
        let d = if frozen {
            match vm.get_own_property(o, k)? {
                Some(p) if matches!(p.slot, Slot::Accessor(..)) => Desc { configurable: Some(false), ..Desc::default() },
                Some(_) => Desc { configurable: Some(false), writable: Some(false), ..Desc::default() },
                None => continue,
            }
        } else {
            Desc { configurable: Some(false), ..Desc::default() }
        };
        vm.define_own_property_or_throw(o, k, d)?;
    }
    Ok(true)
}

fn test_integrity(vm: &mut Vm, o: Gc, frozen: bool) -> Result<bool, Value> {
    if vm.is_extensible(o)? {
        return Ok(false);
    }
    for k in vm.own_keys(o)? {
        if let Some(p) = vm.get_own_property(o, k)? {
            if p.configurable() {
                return Ok(false);
            }
            if frozen && matches!(p.slot, Slot::Data(_)) && p.writable() {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn object_freeze(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let v = arg(args, 0);
    if let Value::Obj(o) = v
        && !set_integrity(vm, o, true)?
    {
        return Err(vm.type_error("Cannot freeze"));
    }
    Ok(v)
}

fn object_is_frozen(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    match arg(args, 0) {
        Value::Obj(o) => Ok(Value::Bool(test_integrity(vm, o, true)?)),
        _ => Ok(Value::Bool(true)),
    }
}

fn object_seal(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let v = arg(args, 0);
    if let Value::Obj(o) = v
        && !set_integrity(vm, o, false)?
    {
        return Err(vm.type_error("Cannot seal"));
    }
    Ok(v)
}

fn object_is_sealed(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    match arg(args, 0) {
        Value::Obj(o) => Ok(Value::Bool(test_integrity(vm, o, false)?)),
        _ => Ok(Value::Bool(true)),
    }
}

fn object_prevent_extensions(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let v = arg(args, 0);
    if let Value::Obj(o) = v
        && !vm.prevent_extensions(o)?
    {
        return Err(vm.type_error("Cannot prevent extensions"));
    }
    Ok(v)
}

fn object_is_extensible(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    match arg(args, 0) {
        Value::Obj(o) => Ok(Value::Bool(vm.is_extensible(o)?)),
        _ => Ok(Value::Bool(false)),
    }
}

fn object_from_entries(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let it = arg(args, 0);
    if it.is_nullish() {
        return Err(vm.type_error("Object.fromEntries requires an iterable"));
    }
    let o = vm.new_object();
    let (iter, next) = vm.get_iterator(&it, false)?;
    while let Some(e) = vm.iter_step(&iter, &next)? {
        let r = (|| -> Result<(), Value> {
            let Value::Obj(eg) = e else { return Err(vm.type_error("Iterator value is not an entry object")) };
            let k = vm.get(eg, Key::Index(0), &e)?;
            let v = vm.get(eg, Key::Index(1), &e)?;
            let key = vm.to_key(&k)?;
            vm.create_data_property(o, key, v)?;
            Ok(())
        })();
        if let Err(err) = r {
            let _ = vm.iterator_close(&iter);
            return Err(err);
        }
    }
    Ok(Value::Obj(o))
}

fn object_gopd(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(&arg(args, 0))?;
    let k = vm.to_key(&arg(args, 1))?;
    match vm.get_own_property(o, k)? {
        Some(p) => Ok(vm.from_descriptor(&Desc::from_prop(&p))),
        None => Ok(Value::Undefined),
    }
}

fn object_gopds(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(&arg(args, 0))?;
    let out = vm.new_object();
    for k in vm.own_keys(o)? {
        if let Some(p) = vm.get_own_property(o, k)? {
            let d = vm.from_descriptor(&Desc::from_prop(&p));
            vm.create_data_property(out, k, d)?;
        }
    }
    Ok(Value::Obj(out))
}

fn object_names(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(&arg(args, 0))?;
    let keys: Vec<Value> = vm.own_keys(o)?.into_iter().filter(|k| !matches!(k, Key::Sym(_))).map(|k| vm.key_value(k)).collect();
    Ok(Value::Obj(vm.new_array(keys)))
}

fn object_symbols(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(&arg(args, 0))?;
    let keys: Vec<Value> = vm.own_keys(o)?.into_iter().filter(|k| matches!(k, Key::Sym(_))).map(|k| vm.key_value(k)).collect();
    Ok(Value::Obj(vm.new_array(keys)))
}

fn object_get_proto(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(&arg(args, 0))?;
    Ok(vm.get_prototype_of(o)?.map_or(Value::Null, Value::Obj))
}

fn object_set_proto(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let v = arg(args, 0);
    if v.is_nullish() {
        return Err(vm.type_error("Object.setPrototypeOf called on null or undefined"));
    }
    let p = match arg(args, 1) {
        Value::Obj(p) => Some(p),
        Value::Null => None,
        _ => return Err(vm.type_error("Object prototype may only be an Object or null")),
    };
    if let Value::Obj(o) = v
        && !vm.set_prototype_of(o, p)?
    {
        return Err(vm.type_error("Cannot set the prototype"));
    }
    Ok(v)
}

fn object_is(_vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(same_value(&arg(args, 0), &arg(args, 1))))
}

fn object_has_own(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(&arg(args, 0))?;
    let k = vm.to_key(&arg(args, 1))?;
    Ok(Value::Bool(vm.has_own_property(o, k)?))
}

/// GroupBy: (key, values) in the order keys were first seen (keys made property keys if `property`).
pub fn group_by(vm: &mut Vm, items: &Value, cb: &Value, property: bool) -> Result<Vec<(Value, Vec<Value>)>, Value> {
    if items.is_nullish() {
        return Err(vm.type_error("groupBy called on null or undefined"));
    }
    if !vm.is_callable(cb) {
        return Err(vm.type_error("callback is not a function"));
    }
    let mut groups: Vec<(Value, Vec<Value>)> = Vec::new();
    let (it, next) = vm.get_iterator(items, false)?;
    let mut i = 0u64;
    while let Some(v) = vm.iter_step(&it, &next)? {
        let k = match vm.call(cb, Value::Undefined, &[v.clone(), Value::Num(i as f64)]) {
            Ok(k) => k,
            Err(e) => {
                let _ = vm.iterator_close(&it);
                return Err(e);
            }
        };
        let k = if property {
            match vm.to_key(&k) {
                Ok(key) => vm.key_value(key),
                Err(e) => {
                    let _ = vm.iterator_close(&it);
                    return Err(e);
                }
            }
        } else if let Value::Num(x) = k
            && x == 0.0
        {
            Value::Num(0.0)
        } else {
            k
        };
        match groups.iter_mut().find(|(g, _)| crate::value::same_value_zero(g, &k)) {
            Some((_, vs)) => vs.push(v),
            None => groups.push((k, alloc::vec![v])),
        }
        i += 1;
    }
    Ok(groups)
}

fn object_group_by(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let groups = group_by(vm, &arg(args, 0), &arg(args, 1), true)?;
    let o = vm.alloc(None, Class::Ordinary);
    for (k, vs) in groups {
        let key = vm.to_key(&k)?;
        let a = vm.new_array(vs);
        vm.create_data_property(o, key, Value::Obj(a))?;
    }
    Ok(Value::Obj(o))
}

fn proto_has_own(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let k = vm.to_key(&arg(args, 0))?;
    let o = vm.to_object(this)?;
    Ok(Value::Bool(vm.has_own_property(o, k)?))
}

fn proto_is_prototype_of(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(v) = arg(args, 0) else { return Ok(Value::Bool(false)) };
    let o = vm.to_object(this)?;
    let mut cur = vm.get_prototype_of(v)?;
    while let Some(c) = cur {
        if c == o {
            return Ok(Value::Bool(true));
        }
        cur = vm.get_prototype_of(c)?;
    }
    Ok(Value::Bool(false))
}

fn proto_property_is_enumerable(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let k = vm.to_key(&arg(args, 0))?;
    let o = vm.to_object(this)?;
    Ok(Value::Bool(vm.get_own_property(o, k)?.is_some_and(|p| p.enumerable())))
}

/// Object.prototype.toString's tag of a value.
pub fn builtin_tag(vm: &mut Vm, v: &Value) -> Result<String, Value> {
    match v {
        Value::Undefined => return Ok(String::from("[object Undefined]")),
        Value::Null => return Ok(String::from("[object Null]")),
        _ => {}
    }
    let o = vm.to_object(v)?;
    let is_array = vm.is_array(&Value::Obj(o))?;
    let base = if is_array {
        "Array"
    } else {
        match &vm.heap.get(o).class {
            Class::Arguments(_) => "Arguments",
            Class::Function(_) | Class::Native(_) | Class::Bound(_) => "Function",
            Class::Proxy(Some(_), _) if vm.is_callable(&Value::Obj(o)) => "Function",
            Class::Error => "Error",
            Class::Boolean(_) => "Boolean",
            Class::Number(_) => "Number",
            Class::String(_) => "String",
            Class::Date(_) => "Date",
            Class::RegExp(_) => "RegExp",
            _ => "Object",
        }
    };
    let t = vm.sym(Sym::ToStringTag);
    let tv = vm.get(o, Key::Sym(t), &Value::Obj(o))?;
    let name = match tv {
        Value::Str(s) => s.to_rust(),
        _ => String::from(base),
    };
    Ok(format!("[object {name}]"))
}

fn proto_to_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = builtin_tag(vm, this)?;
    Ok(js_str(&s))
}

fn proto_to_locale_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let f = vm.get_value(this, Key::Str(n::TO_STRING))?;
    vm.call(&f, this.clone(), &[])
}

fn proto_value_of(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Obj(vm.to_object(this)?))
}

fn proto_define_getter(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(this)?;
    let g = callable_arg(vm, args, 1)?;
    let k = vm.to_key(&arg(args, 0))?;
    vm.define_own_property_or_throw(o, k, Desc { get: Some(g), enumerable: Some(true), configurable: Some(true), ..Desc::default() })?;
    Ok(Value::Undefined)
}

fn proto_define_setter(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(this)?;
    let s = callable_arg(vm, args, 1)?;
    let k = vm.to_key(&arg(args, 0))?;
    vm.define_own_property_or_throw(o, k, Desc { set: Some(s), enumerable: Some(true), configurable: Some(true), ..Desc::default() })?;
    Ok(Value::Undefined)
}

fn lookup_accessor(vm: &mut Vm, this: &Value, args: &[Value], getter: bool) -> R {
    let o = vm.to_object(this)?;
    let k = vm.to_key(&arg(args, 0))?;
    let mut cur = Some(o);
    while let Some(c) = cur {
        if let Some(p) = vm.get_own_property(c, k)? {
            return Ok(match p.slot {
                Slot::Accessor(g, s) => {
                    if getter {
                        g
                    } else {
                        s
                    }
                }
                Slot::Data(_) => Value::Undefined,
            });
        }
        cur = vm.get_prototype_of(c)?;
    }
    Ok(Value::Undefined)
}

fn proto_lookup_getter(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    lookup_accessor(vm, this, args, true)
}

fn proto_lookup_setter(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    lookup_accessor(vm, this, args, false)
}

fn proto_get_proto(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(this)?;
    Ok(vm.get_prototype_of(o)?.map_or(Value::Null, Value::Obj))
}

fn proto_set_proto(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if this.is_nullish() {
        return Err(vm.type_error("Object.prototype.__proto__ called on null or undefined"));
    }
    let p = match arg(args, 0) {
        Value::Obj(p) => Some(p),
        Value::Null => None,
        _ => return Ok(Value::Undefined),
    };
    if let Value::Obj(o) = this
        && !vm.set_prototype_of(*o, p)?
    {
        return Err(vm.type_error("Object.prototype.__proto__ setter failed"));
    }
    Ok(Value::Undefined)
}

// ----------------------------------------------------------------------------------------------------- Function

fn function(vm: &mut Vm) {
    let fp = vm.proto(I::FunctionProto);
    let c = constructor(vm, "Function", 1, function_call, Some(function_construct), Some(fp));
    vm.intrinsics[I::FunctionCtor as usize] = c;
    vm.method(fp, "call", 1, fn_call);
    vm.method(fp, "apply", 2, fn_apply);
    vm.method(fp, "bind", 1, fn_bind);
    vm.method(fp, "toString", 0, fn_to_string);
    let hi = sym_method(vm, fp, Sym::HasInstance, "[Symbol.hasInstance]", 1, fn_has_instance);
    // (Function.prototype[@@hasInstance] can't be changed.)
    let k = Key::Sym(vm.sym(Sym::HasInstance));
    vm.put_key(fp, k, Value::Obj(hi), 0);
    // %ThrowTypeError%, and the restricted caller and arguments.
    let tte = vm.native("", 0, throw_type_error);
    for (_, p) in vm.heap.get_mut(tte).props.iter_mut() {
        p.flags = 0;
    }
    vm.heap.get_mut(tte).extensible = false;
    vm.intrinsics[I::ThrowTypeError as usize] = tte;
    for name in [n::CALLER, n::ARGUMENTS] {
        vm.heap.get_mut(fp).props.set(Key::Str(name), Prop { slot: Slot::Accessor(Value::Obj(tte), Value::Obj(tte)), flags: CONFIGURABLE });
    }
    // The constructors of generator and async functions (not globals).
    for (proto, name, kind) in [(I::GeneratorFunctionProto, "GeneratorFunction", 1.0), (I::AsyncFunctionProto, "AsyncFunction", 2.0), (I::AsyncGeneratorFunctionProto, "AsyncGeneratorFunction", 3.0)] {
        let p = vm.proto(proto);
        let c = vm.native_with(name, 1, function_call, Some(function_construct), Value::Num(kind));
        vm.heap.get_mut(c).proto = Some(vm.proto(I::FunctionCtor));
        vm.put_key(c, Key::Str(n::PROTOTYPE), Value::Obj(p), 0);
        vm.put_key(p, Key::Str(n::CONSTRUCTOR), Value::Obj(c), CONFIGURABLE);
        tag(vm, p, name);
        let inner = match proto {
            I::GeneratorFunctionProto => Some(I::GeneratorProto),
            I::AsyncGeneratorFunctionProto => Some(I::AsyncGeneratorProto),
            _ => None,
        };
        if let Some(i) = inner {
            let ip = vm.proto(i);
            vm.put_key(p, Key::Str(n::PROTOTYPE), Value::Obj(ip), CONFIGURABLE);
            vm.put_key(ip, Key::Str(n::CONSTRUCTOR), Value::Obj(p), CONFIGURABLE);
        }
    }
}

fn throw_type_error(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Err(vm.type_error("'caller', 'callee', and 'arguments' properties may not be accessed on strict mode functions or the arguments objects for calls to them"))
}

fn function_call(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    function_construct(vm, &Value::Obj(f), args, f)
}

/// new Function(params..., body) (and its generator and async kinds).
fn function_construct(vm: &mut Vm, nt: &Value, args: &[Value], f: Gc) -> R {
    let kind = match vm.data(f) {
        Value::Num(k) => k as u8,
        _ => 0,
    };
    let mut params = String::new();
    let mut body = String::new();
    for (i, a) in args.iter().enumerate() {
        let s = vm.to_string(a)?.to_rust();
        if i + 1 == args.len() {
            body = s;
        } else {
            if i > 0 {
                params.push(',');
            }
            params.push_str(&s);
        }
    }
    let k = ["function", "function*", "async function", "async function*"][kind as usize];
    let func = match crate::parser::parse_function_parts(&params, &body, k, &mut vm.atoms) {
        Ok(func) => func,
        Err(e) => return Err(vm.syntax_error(&e.message)),
    };
    let src = format!("({k} anonymous({params}\n) {{\n{body}\n}})");
    let code = match crate::compiler::compile_function(&func, &mut vm.atoms, Rc::from(src.as_str())) {
        Ok(c) => c,
        Err(e) => return Err(vm.syntax_error(&e.message)),
    };
    let g = vm.make_closure(code, None, Value::Undefined, Value::Undefined, None);
    let default = [I::FunctionProto, I::GeneratorFunctionProto, I::AsyncFunctionProto, I::AsyncGeneratorFunctionProto][kind as usize];
    let p = vm.proto_from_constructor(nt, default)?;
    vm.heap.get_mut(g).proto = Some(p);
    Ok(Value::Obj(g))
}

fn fn_call(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if !vm.is_callable(this) {
        return Err(vm.type_error("Function.prototype.call called on a value that isn't a function"));
    }
    let rest = if args.len() > 1 { &args[1..] } else { &[] };
    vm.call(this, arg(args, 0), rest)
}

fn fn_apply(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if !vm.is_callable(this) {
        return Err(vm.type_error("Function.prototype.apply was called on a value that isn't a function"));
    }
    let a = arg(args, 1);
    let list = if a.is_nullish() { Vec::new() } else { list_from_array_like(vm, &a)? };
    vm.call(this, arg(args, 0), &list)
}

fn fn_bind(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(target) = this else { return Err(vm.type_error("Bind must be called on a function")) };
    if !vm.is_callable(this) {
        return Err(vm.type_error("Bind must be called on a function"));
    }
    let target = *target;
    let bound_args: Vec<Value> = args.iter().skip(1).cloned().collect();
    let proto = vm.get_prototype_of(target)?;
    let g = vm.alloc(proto, Class::Bound(Box::new(Bound { target, this: arg(args, 0), args: bound_args.clone() })));
    let mut len = 0.0;
    if vm.has_own_property(target, Key::Str(n::LENGTH))? {
        let l = vm.get(target, Key::Str(n::LENGTH), this)?;
        if let Value::Num(x) = l {
            len = if x == f64::INFINITY {
                x
            } else if x == f64::NEG_INFINITY {
                0.0
            } else {
                (crate::value::to_integer(x) - bound_args.len() as f64).max(0.0)
            };
        }
    }
    let name = vm.get(target, Key::Str(n::NAME), this)?;
    let name = match name {
        Value::Str(s) => s,
        _ => JsStr::empty(),
    };
    let o = vm.heap.get_mut(g);
    o.props.set(Key::Str(n::LENGTH), Prop::data(Value::Num(len), CONFIGURABLE));
    o.props.set(Key::Str(n::NAME), Prop::data(Value::Str(JsStr::from_str("bound ").concat(&name)), CONFIGURABLE));
    Ok(Value::Obj(g))
}

fn fn_to_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Value::Obj(g) = this else { return Err(vm.type_error("Function.prototype.toString requires that 'this' be a Function")) };
    let name_of = |vm: &mut Vm, g: Gc| -> String {
        match vm.get(g, Key::Str(n::NAME), &Value::Obj(g)) {
            Ok(Value::Str(s)) => s.to_rust(),
            _ => String::new(),
        }
    };
    match &vm.heap.get(*g).class {
        Class::Function(c) => {
            if let Some(src) = &c.code.source
                && c.code.end > c.code.start
                && c.code.end <= src.len()
            {
                return Ok(js_str(&src[c.code.start..c.code.end]));
            }
            let name = name_of(vm, *g);
            Ok(js_str(&format!("function {name}() {{ [native code] }}")))
        }
        Class::Native(_) | Class::Bound(_) => {
            let name = name_of(vm, *g);
            Ok(js_str(&format!("function {name}() {{ [native code] }}")))
        }
        Class::Proxy(..) if vm.is_callable(this) => Ok(js_str("function () { [native code] }")),
        _ => Err(vm.type_error("Function.prototype.toString requires that 'this' be a Function")),
    }
}

fn fn_has_instance(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(vm.ordinary_has_instance(this, &arg(args, 0))?))
}

// ------------------------------------------------------------------------------------------------------- errors

const ERRORS: [(&str, ErrorKind); 8] = [
    ("Error", ErrorKind::Error),
    ("EvalError", ErrorKind::Eval),
    ("RangeError", ErrorKind::Range),
    ("ReferenceError", ErrorKind::Reference),
    ("SyntaxError", ErrorKind::Syntax),
    ("TypeError", ErrorKind::Type),
    ("URIError", ErrorKind::Uri),
    ("AggregateError", ErrorKind::Aggregate),
];

fn errors(vm: &mut Vm) {
    let mut error_ctor = None;
    for (i, (name, kind)) in ERRORS.iter().enumerate() {
        let p = vm.proto(kind.proto());
        let len = if *kind == ErrorKind::Aggregate { 2 } else { 1 };
        let c = constructor(vm, name, len, error_call, Some(error_construct), Some(p));
        if let Class::Native(nat) = &mut vm.heap.get_mut(c).class {
            nat.data = Value::Num(i as f64);
        }
        vm.put(p, "name", js_str(name), HIDDEN);
        vm.put(p, "message", js_str(""), HIDDEN);
        match error_ctor {
            None => {
                error_ctor = Some(c);
                vm.method(p, "toString", 0, error_to_string);
                vm.method(c, "captureStackTrace", 1, error_capture_stack);
            }
            Some(e) => vm.heap.get_mut(c).proto = Some(e),
        }
    }
}

fn error_call(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    error_construct(vm, &Value::Obj(f), args, f)
}

fn error_construct(vm: &mut Vm, nt: &Value, args: &[Value], f: Gc) -> R {
    let i = match vm.data(f) {
        Value::Num(i) => i as usize,
        _ => 0,
    };
    let kind = ERRORS[i].1;
    let p = vm.proto_from_constructor(nt, kind.proto())?;
    let o = vm.alloc(Some(p), Class::Error);
    let (msg, opts) = if kind == ErrorKind::Aggregate { (arg(args, 1), arg(args, 2)) } else { (arg(args, 0), arg(args, 1)) };
    let mut text = String::new();
    if !msg.is_undefined() {
        let s = vm.to_string(&msg)?;
        text = s.to_rust();
        vm.put_key(o, Key::Str(n::MESSAGE), Value::Str(s), HIDDEN);
    }
    if let Value::Obj(og) = opts
        && vm.has_property(og, Key::Str(n::CAUSE))?
    {
        let c = vm.get(og, Key::Str(n::CAUSE), &opts)?;
        vm.put_key(o, Key::Str(n::CAUSE), c, HIDDEN);
    }
    if kind == ErrorKind::Aggregate {
        let errs = vm.iterate_to_vec(&arg(args, 0))?;
        let a = vm.new_array(errs);
        vm.put_key(o, Key::Str(n::ERRORS), Value::Obj(a), HIDDEN);
    }
    let stack = vm.stack_trace(kind, &text);
    vm.put_key(o, Key::Str(n::STACK), Value::Str(stack), HIDDEN);
    Ok(Value::Obj(o))
}

fn error_to_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Value::Obj(o) = this else { return Err(vm.type_error("Error.prototype.toString called on non-object")) };
    let name = vm.get(*o, Key::Str(n::NAME), this)?;
    let name = if name.is_undefined() { JsStr::from_str("Error") } else { vm.to_string(&name)? };
    let msg = vm.get(*o, Key::Str(n::MESSAGE), this)?;
    let msg = if msg.is_undefined() { JsStr::empty() } else { vm.to_string(&msg)? };
    if name.is_empty() {
        return Ok(Value::Str(msg));
    }
    if msg.is_empty() {
        return Ok(Value::Str(name));
    }
    Ok(Value::Str(name.concat(&JsStr::from_str(": ")).concat(&msg)))
}

fn error_capture_stack(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    if let Value::Obj(o) = arg(args, 0) {
        let s = vm.stack_trace(ErrorKind::Error, "");
        vm.put_key(o, Key::Str(n::STACK), Value::Str(s), HIDDEN);
    }
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------------------------------------------- Symbol

fn symbol(vm: &mut Vm) {
    let sp = vm.proto(I::SymbolProto);
    let c = constructor(vm, "Symbol", 0, symbol_call, Some(symbol_construct), Some(sp));
    for (i, name) in SYMBOL_NAMES.iter().enumerate() {
        let s = vm.symbols[i];
        vm.put(c, name, Value::Sym(s), 0);
    }
    vm.method(c, "for", 1, symbol_for);
    vm.method(c, "keyFor", 1, symbol_key_for);
    vm.method(sp, "toString", 0, symbol_to_string);
    vm.method(sp, "valueOf", 0, symbol_value_of);
    super::getter(vm, sp, "description", symbol_description);
    let tp = sym_method(vm, sp, Sym::ToPrimitive, "[Symbol.toPrimitive]", 1, symbol_value_of);
    let k = Key::Sym(vm.sym(Sym::ToPrimitive));
    vm.put_key(sp, k, Value::Obj(tp), CONFIGURABLE);
    tag(vm, sp, "Symbol");
}

fn symbol_call(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let d = arg(args, 0);
    let desc = if d.is_undefined() { None } else { Some(vm.to_string(&d)?) };
    Ok(Value::Sym(vm.new_symbol(desc, 0)))
}

fn symbol_construct(vm: &mut Vm, _nt: &Value, _args: &[Value], _f: Gc) -> R {
    Err(vm.type_error("Symbol is not a constructor"))
}

fn symbol_for(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let k = vm.to_string(&arg(args, 0))?;
    if let Some(s) = vm.registry.get(&k) {
        return Ok(Value::Sym(*s));
    }
    let s = vm.new_symbol(Some(k.clone()), SYM_REGISTERED);
    vm.registry.insert(k, s);
    Ok(Value::Sym(s))
}

fn symbol_key_for(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Sym(s) = arg(args, 0) else { return Err(vm.type_error("Symbol.keyFor: not a symbol")) };
    match &vm.heap.get(s).class {
        Class::Symbol(Some(d), f) if f & SYM_REGISTERED != 0 => Ok(Value::Str(d.clone())),
        _ => Ok(Value::Undefined),
    }
}

fn this_symbol(vm: &mut Vm, this: &Value) -> Result<Gc, Value> {
    match this {
        Value::Sym(s) => Ok(*s),
        Value::Obj(g) => match vm.heap.get(*g).class {
            Class::SymbolObj(s) => Ok(s),
            _ => Err(vm.type_error("not a Symbol")),
        },
        _ => Err(vm.type_error("not a Symbol")),
    }
}

/// "Symbol(description)".
pub fn symbol_descriptive(vm: &Vm, s: Gc) -> JsStr {
    let d = match &vm.heap.get(s).class {
        Class::Symbol(Some(d), _) => d.clone(),
        _ => JsStr::empty(),
    };
    JsStr::from_str("Symbol(").concat(&d).concat(&JsStr::from_str(")"))
}

fn symbol_to_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = this_symbol(vm, this)?;
    Ok(Value::Str(symbol_descriptive(vm, s)))
}

fn symbol_value_of(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Sym(this_symbol(vm, this)?))
}

fn symbol_description(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = this_symbol(vm, this)?;
    Ok(match &vm.heap.get(s).class {
        Class::Symbol(Some(d), _) => Value::Str(d.clone()),
        _ => Value::Undefined,
    })
}

// ------------------------------------------------------------------------------------------------------ Boolean

fn boolean(vm: &mut Vm) {
    let bp = vm.proto(I::BooleanProto);
    constructor(vm, "Boolean", 1, boolean_call, Some(boolean_construct), Some(bp));
    vm.method(bp, "toString", 0, boolean_to_string);
    vm.method(bp, "valueOf", 0, boolean_value_of);
}

fn boolean_call(_vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(arg(args, 0).truthy()))
}

fn boolean_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let b = arg(args, 0).truthy();
    let p = vm.proto_from_constructor(nt, I::BooleanProto)?;
    Ok(Value::Obj(vm.alloc(Some(p), Class::Boolean(b))))
}

fn this_boolean(vm: &mut Vm, this: &Value) -> Result<bool, Value> {
    match this {
        Value::Bool(b) => Ok(*b),
        Value::Obj(g) => match vm.heap.get(*g).class {
            Class::Boolean(b) => Ok(b),
            _ => Err(vm.type_error("not a Boolean")),
        },
        _ => Err(vm.type_error("not a Boolean")),
    }
}

fn boolean_to_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let b = this_boolean(vm, this)?;
    Ok(js_str(if b { "true" } else { "false" }))
}

fn boolean_value_of(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(this_boolean(vm, this)?))
}

// ------------------------------------------------------------------------------------------------------ Reflect

fn reflect(vm: &mut Vm) {
    let r = vm.new_object();
    let g = vm.global;
    vm.put(g, "Reflect", Value::Obj(r), HIDDEN);
    tag(vm, r, "Reflect");
    vm.method(r, "apply", 3, reflect_apply);
    vm.method(r, "construct", 2, reflect_construct);
    vm.method(r, "defineProperty", 3, reflect_define_property);
    vm.method(r, "deleteProperty", 2, reflect_delete_property);
    vm.method(r, "get", 2, reflect_get);
    vm.method(r, "getOwnPropertyDescriptor", 2, reflect_gopd);
    vm.method(r, "getPrototypeOf", 1, reflect_get_proto);
    vm.method(r, "has", 2, reflect_has);
    vm.method(r, "isExtensible", 1, reflect_is_extensible);
    vm.method(r, "ownKeys", 1, reflect_own_keys);
    vm.method(r, "preventExtensions", 1, reflect_prevent_extensions);
    vm.method(r, "set", 3, reflect_set);
    vm.method(r, "setPrototypeOf", 2, reflect_set_proto);
}

fn target_arg(vm: &mut Vm, args: &[Value]) -> Result<Gc, Value> {
    match arg(args, 0) {
        Value::Obj(g) => Ok(g),
        _ => Err(vm.type_error("Reflect: the target must be an object")),
    }
}

fn reflect_apply(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let f = callable_arg(vm, args, 0)?;
    let list = list_from_array_like(vm, &arg(args, 2))?;
    vm.call(&f, arg(args, 1), &list)
}

fn reflect_construct(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = arg(args, 0);
    if !vm.is_constructor(&t) {
        return Err(vm.type_error("Reflect.construct: the target is not a constructor"));
    }
    let nt = if args.len() > 2 { arg(args, 2) } else { t.clone() };
    if !vm.is_constructor(&nt) {
        return Err(vm.type_error("Reflect.construct: newTarget is not a constructor"));
    }
    let list = list_from_array_like(vm, &arg(args, 1))?;
    vm.construct(&t, &list, &nt)
}

fn reflect_define_property(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_arg(vm, args)?;
    let k = vm.to_key(&arg(args, 1))?;
    let d = vm.to_descriptor(&arg(args, 2))?;
    Ok(Value::Bool(vm.define_own_property(t, k, d)?))
}

fn reflect_delete_property(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_arg(vm, args)?;
    let k = vm.to_key(&arg(args, 1))?;
    Ok(Value::Bool(vm.delete_property(t, k)?))
}

fn reflect_get(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_arg(vm, args)?;
    let k = vm.to_key(&arg(args, 1))?;
    let r = if args.len() > 2 { arg(args, 2) } else { Value::Obj(t) };
    vm.get(t, k, &r)
}

fn reflect_gopd(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_arg(vm, args)?;
    let k = vm.to_key(&arg(args, 1))?;
    match vm.get_own_property(t, k)? {
        Some(p) => Ok(vm.from_descriptor(&Desc::from_prop(&p))),
        None => Ok(Value::Undefined),
    }
}

fn reflect_get_proto(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_arg(vm, args)?;
    Ok(vm.get_prototype_of(t)?.map_or(Value::Null, Value::Obj))
}

fn reflect_has(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_arg(vm, args)?;
    let k = vm.to_key(&arg(args, 1))?;
    Ok(Value::Bool(vm.has_property(t, k)?))
}

fn reflect_is_extensible(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_arg(vm, args)?;
    Ok(Value::Bool(vm.is_extensible(t)?))
}

fn reflect_own_keys(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_arg(vm, args)?;
    let keys: Vec<Value> = vm.own_keys(t)?.into_iter().map(|k| vm.key_value(k)).collect();
    Ok(Value::Obj(vm.new_array(keys)))
}

fn reflect_prevent_extensions(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_arg(vm, args)?;
    Ok(Value::Bool(vm.prevent_extensions(t)?))
}

fn reflect_set(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_arg(vm, args)?;
    let k = vm.to_key(&arg(args, 1))?;
    let r = if args.len() > 3 { arg(args, 3) } else { Value::Obj(t) };
    Ok(Value::Bool(vm.set(t, k, arg(args, 2), &r)?))
}

fn reflect_set_proto(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_arg(vm, args)?;
    let p = match arg(args, 1) {
        Value::Obj(p) => Some(p),
        Value::Null => None,
        _ => return Err(vm.type_error("Object prototype may only be an Object or null")),
    };
    Ok(Value::Bool(vm.set_prototype_of(t, p)?))
}

// -------------------------------------------------------------------------------------------------------- Proxy

fn proxy(vm: &mut Vm) {
    let c = vm.native_with("Proxy", 2, proxy_call, Some(proxy_construct), Value::Undefined);
    let g = vm.global;
    vm.put(g, "Proxy", Value::Obj(c), HIDDEN);
    vm.method(c, "revocable", 2, proxy_revocable);
}

fn proxy_call(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Err(vm.type_error("Constructor Proxy requires 'new'"))
}

fn make_proxy(vm: &mut Vm, args: &[Value]) -> Result<Gc, Value> {
    let (Value::Obj(t), Value::Obj(h)) = (arg(args, 0), arg(args, 1)) else { return Err(vm.type_error("Cannot create proxy with a non-object as target or handler")) };
    let flags = vm.is_callable(&Value::Obj(t)) as u8 | (vm.is_constructor(&Value::Obj(t)) as u8) << 1;
    Ok(vm.alloc(None, Class::Proxy(Some((t, h)), flags)))
}

fn proxy_construct(vm: &mut Vm, _nt: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Obj(make_proxy(vm, args)?))
}

fn proxy_revocable(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let p = make_proxy(vm, args)?;
    let revoke = vm.native_with("", 0, proxy_revoke, None, Value::Obj(p));
    let o = vm.new_object();
    vm.put(o, "proxy", Value::Obj(p), ALL);
    vm.put(o, "revoke", Value::Obj(revoke), ALL);
    Ok(Value::Obj(o))
}

fn proxy_revoke(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    if let Value::Obj(p) = vm.data(f) {
        if let Class::Proxy(slot, _) = &mut vm.heap.get_mut(p).class {
            *slot = None;
        }
        if let Class::Native(nat) = &mut vm.heap.get_mut(f).class {
            nat.data = Value::Undefined;
        }
    }
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------------------------------------------ globals

fn globals(vm: &mut Vm) {
    let g = vm.global;
    let e = vm.method(g, "eval", 1, global_eval);
    vm.intrinsics[I::Eval as usize] = e;
    vm.method(g, "encodeURIComponent", 1, encode_uri_component);
    vm.method(g, "encodeURI", 1, encode_uri);
    vm.method(g, "decodeURIComponent", 1, decode_uri_component);
    vm.method(g, "decodeURI", 1, decode_uri);
    vm.method(g, "escape", 1, escape);
    vm.method(g, "unescape", 1, unescape);
    vm.method(g, "queueMicrotask", 1, queue_microtask);
    vm.put(g, "undefined", Value::Undefined, 0);
    vm.put(g, "NaN", Value::Num(f64::NAN), 0);
    vm.put(g, "Infinity", Value::Num(f64::INFINITY), 0);
}

fn global_eval(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    match arg(args, 0) {
        Value::Str(s) => {
            let src = s.to_rust();
            vm.indirect_eval(&src)
        }
        other => Ok(other),
    }
}

fn queue_microtask(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let f = callable_arg(vm, args, 0)?;
    vm.jobs.push_back(crate::vm::Job::Call { f, args: Vec::new() });
    Ok(Value::Undefined)
}

const URI_RESERVED: &str = ";/?:@&=+$,";
const URI_UNESCAPED_EXTRA: &str = "-_.!~*'()";

fn encode(vm: &mut Vm, args: &[Value], keep: &str) -> R {
    let s = vm.to_string(&arg(args, 0))?;
    let u = s.units();
    let mut out = String::new();
    let mut i = 0;
    while i < u.len() {
        let c = u[i];
        if c < 128 && ((c as u8).is_ascii_alphanumeric() || URI_UNESCAPED_EXTRA.contains(c as u8 as char) || keep.contains(c as u8 as char)) {
            out.push(c as u8 as char);
            i += 1;
            continue;
        }
        let cp = if (0xDC00..0xE000).contains(&c) {
            return Err(vm.make_error(ErrorKind::Uri, "URI malformed"));
        } else if (0xD800..0xDC00).contains(&c) {
            match u.get(i + 1) {
                Some(&d) if (0xDC00..0xE000).contains(&d) => {
                    i += 1;
                    0x10000 + (((c as u32) - 0xD800) << 10) + (d as u32 - 0xDC00)
                }
                _ => return Err(vm.make_error(ErrorKind::Uri, "URI malformed")),
            }
        } else {
            c as u32
        };
        let mut buf = [0u8; 4];
        let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
        for b in ch.encode_utf8(&mut buf).bytes() {
            out.push_str(&format!("%{b:02X}"));
        }
        i += 1;
    }
    Ok(js_str(&out))
}

fn encode_uri_component(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    encode(vm, args, "")
}

fn encode_uri(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    encode(vm, args, ";/?:@&=+$,#")
}

fn hex(c: u16) -> Option<u8> {
    match c {
        0x30..=0x39 => Some((c - 0x30) as u8),
        0x41..=0x46 => Some((c - 0x41 + 10) as u8),
        0x61..=0x66 => Some((c - 0x61 + 10) as u8),
        _ => None,
    }
}

fn decode(vm: &mut Vm, args: &[Value], reserved: &str) -> R {
    let s = vm.to_string(&arg(args, 0))?;
    let u = s.units();
    let mut out: Vec<u16> = Vec::with_capacity(u.len());
    let mut i = 0;
    let bad = |vm: &mut Vm| vm.make_error(ErrorKind::Uri, "URI malformed");
    while i < u.len() {
        if u[i] != b'%' as u16 {
            out.push(u[i]);
            i += 1;
            continue;
        }
        let byte = |j: usize| -> Option<u8> { Some(hex(*u.get(j + 1)?)? * 16 + hex(*u.get(j + 2)?)?) };
        let Some(b) = byte(i) else { return Err(bad(vm)) };
        if b < 0x80 {
            if reserved.contains(b as char) {
                out.extend_from_slice(&u[i..i + 3]);
            } else {
                out.push(b as u16);
            }
            i += 3;
            continue;
        }
        let n = if b & 0xE0 == 0xC0 {
            2
        } else if b & 0xF0 == 0xE0 {
            3
        } else if b & 0xF8 == 0xF0 {
            4
        } else {
            return Err(bad(vm));
        };
        let mut bytes = alloc::vec![b];
        for k in 1..n {
            let j = i + 3 * k;
            if u.get(j) != Some(&(b'%' as u16)) {
                return Err(bad(vm));
            }
            let Some(c) = byte(j) else { return Err(bad(vm)) };
            if c & 0xC0 != 0x80 {
                return Err(bad(vm));
            }
            bytes.push(c);
        }
        let Ok(st) = core::str::from_utf8(&bytes) else { return Err(bad(vm)) };
        for c in st.encode_utf16() {
            out.push(c);
        }
        i += 3 * n;
    }
    Ok(Value::Str(JsStr::from_units(out)))
}

fn decode_uri_component(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    decode(vm, args, "")
}

fn decode_uri(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    decode(vm, args, &format!("{URI_RESERVED}#"))
}

fn escape(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = vm.to_string(&arg(args, 0))?;
    let mut out = String::new();
    for &c in s.units() {
        if c < 128 && ((c as u8).is_ascii_alphanumeric() || "@*_+-./".contains(c as u8 as char)) {
            out.push(c as u8 as char);
        } else if c < 256 {
            out.push_str(&format!("%{c:02X}"));
        } else {
            out.push_str(&format!("%u{c:04X}"));
        }
    }
    Ok(js_str(&out))
}

fn unescape(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = vm.to_string(&arg(args, 0))?;
    let u = s.units();
    let mut out = Vec::with_capacity(u.len());
    let mut i = 0;
    while i < u.len() {
        if u[i] == b'%' as u16 {
            if u.get(i + 1) == Some(&(b'u' as u16)) && i + 6 <= u.len() {
                let hs: Option<Vec<u8>> = u[i + 2..i + 6].iter().map(|c| hex(*c)).collect();
                if let Some(h) = hs {
                    out.push(h.iter().fold(0u16, |a, d| a * 16 + *d as u16));
                    i += 6;
                    continue;
                }
            }
            if i + 3 <= u.len()
                && let (Some(a), Some(b)) = (hex(u[i + 1]), hex(u[i + 2]))
            {
                out.push((a * 16 + b) as u16);
                i += 3;
                continue;
            }
        }
        out.push(u[i]);
        i += 1;
    }
    Ok(Value::Str(JsStr::from_units(out)))
}
