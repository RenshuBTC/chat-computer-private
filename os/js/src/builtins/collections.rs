//! Map, Set, WeakMap, WeakSet, WeakRef and FinalizationRegistry. A map's entries are kept in the order they were
//! added (deleted ones left as holes, so iterators keep their place), with a hash table from key to place.

use super::{arg, callable_arg, constructor, species_getter, sym_getter, sym_method, tag};
use crate::names as n;
use crate::object::*;
use crate::string::hash_units;
use crate::value::{Gc, Value, same_value_zero};
use crate::vm::{I, Sym, Vm};
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

type R = Result<Value, Value>;

fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^ (x >> 33)
}

fn hash(v: &Value) -> u64 {
    mix(match v {
        Value::Undefined | Value::Empty => 1,
        Value::Null => 2,
        Value::Bool(b) => 3 + *b as u64,
        Value::Num(x) => {
            if x.is_nan() {
                5
            } else if *x == 0.0 {
                6
            } else {
                x.to_bits()
            }
        }
        Value::Str(s) => hash_units(s.units()) as u64 | (1 << 40),
        Value::Sym(g) | Value::Obj(g) => g.0 as u64 | (1 << 41),
        Value::Big(b) => b.to_string_radix(16).bytes().fold(7u64, |a, c| a.wrapping_mul(31).wrapping_add(c as u64)),
    })
}

/// A key as a map holds it (-0 is +0).
fn normalize(v: Value) -> Value {
    match v {
        Value::Num(x) if x == 0.0 => Value::Num(0.0),
        v => v,
    }
}

impl MapData {
    fn find(&self, key: &Value) -> Option<usize> {
        if self.index.is_empty() {
            return None;
        }
        let h = hash(key);
        let mask = self.index.len() - 1;
        let mut i = h as usize & mask;
        loop {
            let (sh, at) = self.index[i];
            if at == 0 {
                return None;
            }
            if sh == h
                && let Some((k, _)) = &self.entries[at as usize - 1]
                && same_value_zero(k, key)
            {
                return Some(at as usize - 1);
            }
            i = (i + 1) & mask;
        }
    }

    /// The index made again, for where the entries are now.
    pub(crate) fn rebuild(&mut self) {
        let size = ((self.entries.len() + 1) * 2).next_power_of_two().max(8);
        self.index = vec![(0, 0); size];
        for (pos, e) in self.entries.iter().enumerate() {
            if let Some((k, _)) = e {
                let h = hash(k);
                let mut i = h as usize & (size - 1);
                while self.index[i].1 != 0 {
                    i = (i + 1) & (size - 1);
                }
                self.index[i] = (h, pos as u32 + 1);
            }
        }
    }

    pub fn get(&self, key: &Value) -> Option<Value> {
        self.find(key).and_then(|i| self.entries[i].as_ref().map(|(_, v)| v.clone()))
    }

    pub fn has(&self, key: &Value) -> bool {
        self.find(key).is_some()
    }

    pub fn set(&mut self, key: Value, v: Value) {
        let key = normalize(key);
        if let Some(i) = self.find(&key) {
            if let Some(e) = &mut self.entries[i] {
                e.1 = v;
            }
            return;
        }
        if self.size == 0 && !self.entries.is_empty() && self.entries.len() > 64 {
            // (Its index goes with them: its places name entries that are no longer there.)
            self.entries.clear();
            self.index.clear();
        }
        self.entries.push(Some((key, v)));
        self.size += 1;
        if self.entries.len() * 2 >= self.index.len() {
            self.rebuild();
        } else {
            let (k, _) = self.entries.last().and_then(|e| e.as_ref()).expect("the entry just added");
            let h = hash(k);
            let mask = self.index.len() - 1;
            let mut i = h as usize & mask;
            while self.index[i].1 != 0 {
                i = (i + 1) & mask;
            }
            self.index[i] = (h, self.entries.len() as u32);
        }
    }

    pub fn delete(&mut self, key: &Value) -> bool {
        match self.find(key) {
            Some(i) => {
                self.entries[i] = None;
                self.size -= 1;
                true
            }
            None => false,
        }
    }

    pub fn clear(&mut self) {
        for e in self.entries.iter_mut() {
            *e = None;
        }
        self.size = 0;
        self.index.clear();
    }
}

pub fn init(vm: &mut Vm) {
    let mp = vm.proto(I::MapProto);
    let mc = constructor(vm, "Map", 0, needs_new, Some(map_construct), Some(mp));
    vm.method(mc, "groupBy", 2, map_group_by);
    sym_getter(vm, mc, Sym::Species, "[Symbol.species]", species_getter);
    vm.method(mp, "clear", 0, map_clear);
    vm.method(mp, "delete", 1, map_delete);
    let entries = vm.method(mp, "entries", 0, map_entries);
    vm.method(mp, "forEach", 1, map_for_each);
    vm.method(mp, "get", 1, map_get);
    vm.method(mp, "has", 1, map_has);
    vm.method(mp, "keys", 0, map_keys);
    vm.method(mp, "set", 2, map_set);
    super::getter(vm, mp, "size", map_size);
    vm.method(mp, "values", 0, map_values);
    let k = Key::Sym(vm.sym(Sym::Iterator));
    vm.put_key(mp, k, Value::Obj(entries), HIDDEN);
    tag(vm, mp, "Map");

    let sp = vm.proto(I::SetProto);
    let sc = constructor(vm, "Set", 0, needs_new, Some(set_construct), Some(sp));
    sym_getter(vm, sc, Sym::Species, "[Symbol.species]", species_getter);
    vm.method(sp, "add", 1, set_add);
    vm.method(sp, "clear", 0, set_clear);
    vm.method(sp, "delete", 1, set_delete);
    vm.method(sp, "entries", 0, set_entries);
    vm.method(sp, "forEach", 1, set_for_each);
    vm.method(sp, "has", 1, set_has);
    super::getter(vm, sp, "size", set_size);
    let values = vm.method(sp, "values", 0, set_values_method);
    vm.put(sp, "keys", Value::Obj(values), HIDDEN);
    vm.put_key(sp, k, Value::Obj(values), HIDDEN);
    vm.method(sp, "union", 1, set_union);
    vm.method(sp, "intersection", 1, set_intersection);
    vm.method(sp, "difference", 1, set_difference);
    vm.method(sp, "symmetricDifference", 1, set_symmetric_difference);
    vm.method(sp, "isSubsetOf", 1, set_is_subset_of);
    vm.method(sp, "isSupersetOf", 1, set_is_superset_of);
    vm.method(sp, "isDisjointFrom", 1, set_is_disjoint_from);
    tag(vm, sp, "Set");

    for (p, name) in [(I::MapIterProto, "Map Iterator"), (I::SetIterProto, "Set Iterator")] {
        let ip = vm.proto(p);
        vm.method(ip, "next", 0, map_iter_next);
        tag(vm, ip, name);
    }

    let wmp = vm.proto(I::WeakMapProto);
    constructor(vm, "WeakMap", 0, needs_new, Some(weakmap_construct), Some(wmp));
    vm.method(wmp, "delete", 1, weak_delete);
    vm.method(wmp, "get", 1, weakmap_get);
    vm.method(wmp, "has", 1, weak_has);
    vm.method(wmp, "set", 2, weakmap_set);
    tag(vm, wmp, "WeakMap");
    let wsp = vm.proto(I::WeakSetProto);
    constructor(vm, "WeakSet", 0, needs_new, Some(weakset_construct), Some(wsp));
    vm.method(wsp, "add", 1, weakset_add);
    vm.method(wsp, "delete", 1, weak_delete);
    vm.method(wsp, "has", 1, weak_has);
    tag(vm, wsp, "WeakSet");
    let wrp = vm.proto(I::WeakRefProto);
    constructor(vm, "WeakRef", 1, needs_new, Some(weakref_construct), Some(wrp));
    vm.method(wrp, "deref", 0, weakref_deref);
    tag(vm, wrp, "WeakRef");
    let frp = vm.proto(I::FinalizationRegistryProto);
    constructor(vm, "FinalizationRegistry", 1, needs_new, Some(finreg_construct), Some(frp));
    vm.method(frp, "register", 2, finreg_register);
    vm.method(frp, "unregister", 1, finreg_unregister);
    tag(vm, frp, "FinalizationRegistry");
    let _ = sym_method;
}

fn needs_new(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Err(vm.type_error("Constructor requires 'new'"))
}

/// The map (or set) data of `this` (its class checked by `want`).
fn data_of<'a>(vm: &'a mut Vm, this: &Value, want: fn(&Class) -> bool, what: &str) -> Result<Gc, Value> {
    if let Value::Obj(g) = this
        && want(&vm.heap.get(*g).class)
    {
        return Ok(*g);
    }
    Err(vm.type_error(&alloc::format!("Method {what} called on incompatible receiver")))
}

fn md(vm: &mut Vm, g: Gc) -> &mut MapData {
    match &mut vm.heap.get_mut(g).class {
        Class::Map(m) | Class::Set(m) | Class::WeakMap(m) | Class::WeakSet(m) => m,
        _ => unreachable!("map data"),
    }
}

fn is_map(c: &Class) -> bool {
    matches!(c, Class::Map(_))
}

fn is_set(c: &Class) -> bool {
    matches!(c, Class::Set(_))
}

/// A collection made by `new`, filled from an iterable with its adder method (`set` or `add`).
fn fill(vm: &mut Vm, o: Gc, iterable: &Value, adder_name: &str, pairs: bool) -> Result<(), Value> {
    if iterable.is_nullish() {
        return Ok(());
    }
    let a = vm.atom(adder_name);
    let adder = vm.get(o, Key::Str(a), &Value::Obj(o))?;
    if !vm.is_callable(&adder) {
        return Err(vm.type_error(&alloc::format!("'{adder_name}' is not a function")));
    }
    let (it, next) = vm.get_iterator(iterable, false)?;
    while let Some(item) = vm.iter_step(&it, &next)? {
        let r = (|| -> Result<(), Value> {
            if pairs {
                let Value::Obj(ig) = item else { return Err(vm.type_error("Iterator value is not an entry object")) };
                let k = vm.get(ig, Key::Index(0), &item)?;
                let v = vm.get(ig, Key::Index(1), &item)?;
                vm.call(&adder, Value::Obj(o), &[k, v])?;
            } else {
                vm.call(&adder, Value::Obj(o), &[item])?;
            }
            Ok(())
        })();
        if let Err(e) = r {
            let _ = vm.iterator_close(&it);
            return Err(e);
        }
    }
    Ok(())
}

fn new_coll(vm: &mut Vm, nt: &Value, proto: I, class: fn(Box<MapData>) -> Class) -> Result<Gc, Value> {
    let p = vm.proto_from_constructor(nt, proto)?;
    Ok(vm.alloc(Some(p), class(Box::default())))
}

fn map_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let o = new_coll(vm, nt, I::MapProto, Class::Map)?;
    fill(vm, o, &arg(args, 0), "set", true)?;
    Ok(Value::Obj(o))
}

fn set_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let o = new_coll(vm, nt, I::SetProto, Class::Set)?;
    fill(vm, o, &arg(args, 0), "add", false)?;
    Ok(Value::Obj(o))
}

fn map_group_by(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let groups = super::core::group_by(vm, &arg(args, 0), &arg(args, 1), false)?;
    let p = vm.proto(I::MapProto);
    let m = vm.alloc(Some(p), Class::Map(Box::default()));
    for (k, vs) in groups {
        let a = vm.new_array(vs);
        md(vm, m).set(k, Value::Obj(a));
    }
    Ok(Value::Obj(m))
}

fn map_clear(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_map, "Map.prototype.clear")?;
    md(vm, g).clear();
    Ok(Value::Undefined)
}

fn set_clear(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "Set.prototype.clear")?;
    md(vm, g).clear();
    Ok(Value::Undefined)
}

fn map_delete(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_map, "Map.prototype.delete")?;
    Ok(Value::Bool(md(vm, g).delete(&arg(args, 0))))
}

fn set_delete(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "Set.prototype.delete")?;
    Ok(Value::Bool(md(vm, g).delete(&arg(args, 0))))
}

fn map_get(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_map, "Map.prototype.get")?;
    Ok(md(vm, g).get(&arg(args, 0)).unwrap_or(Value::Undefined))
}

fn map_has(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_map, "Map.prototype.has")?;
    Ok(Value::Bool(md(vm, g).has(&arg(args, 0))))
}

fn set_has(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "Set.prototype.has")?;
    Ok(Value::Bool(md(vm, g).has(&arg(args, 0))))
}

fn map_set(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_map, "Map.prototype.set")?;
    md(vm, g).set(arg(args, 0), arg(args, 1));
    Ok(this.clone())
}

fn set_add(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "Set.prototype.add")?;
    let v = arg(args, 0);
    md(vm, g).set(v.clone(), v);
    Ok(this.clone())
}

fn map_size(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_map, "get Map.prototype.size")?;
    Ok(Value::Num(md(vm, g).size as f64))
}

fn set_size(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "get Set.prototype.size")?;
    Ok(Value::Num(md(vm, g).size as f64))
}

fn map_for_each(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_map, "Map.prototype.forEach")?;
    for_each(vm, g, this, args)
}

fn set_for_each(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "Set.prototype.forEach")?;
    for_each(vm, g, this, args)
}

fn for_each(vm: &mut Vm, g: Gc, this: &Value, args: &[Value]) -> R {
    let f = callable_arg(vm, args, 0)?;
    let t = arg(args, 1);
    let mut i = 0;
    loop {
        let e = {
            let m = md(vm, g);
            if i >= m.entries.len() {
                break;
            }
            m.entries[i].clone()
        };
        i += 1;
        if let Some((k, v)) = e {
            vm.call(&f, t.clone(), &[v, k, this.clone()])?;
        }
    }
    Ok(Value::Undefined)
}

fn iter_of(vm: &mut Vm, this: &Value, kind: u8, want: fn(&Class) -> bool, proto: I) -> R {
    let g = data_of(vm, this, want, "iterator")?;
    let p = vm.proto(proto);
    Ok(Value::Obj(vm.alloc(Some(p), Class::MapIter(Value::Obj(g), 0, kind))))
}

fn map_entries(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    iter_of(vm, this, 2, is_map, I::MapIterProto)
}

fn map_keys(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    iter_of(vm, this, 0, is_map, I::MapIterProto)
}

fn map_values(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    iter_of(vm, this, 1, is_map, I::MapIterProto)
}

fn set_values_method(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    iter_of(vm, this, 1, is_set, I::SetIterProto)
}

fn set_entries(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    iter_of(vm, this, 2, is_set, I::SetIterProto)
}

fn map_iter_next(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let g = match this {
        Value::Obj(g) if matches!(vm.heap.get(*g).class, Class::MapIter(..)) => *g,
        _ => return Err(vm.type_error("next method called on incompatible receiver")),
    };
    let (target, mut pos, kind) = match &vm.heap.get(g).class {
        Class::MapIter(t, p, k) => (t.clone(), *p as usize, *k),
        _ => unreachable!(),
    };
    let Value::Obj(m) = target else { return Ok(vm.iter_result(Value::Undefined, true)) };
    loop {
        let e = {
            let d = md(vm, m);
            if pos >= d.entries.len() {
                None
            } else {
                Some(d.entries[pos].clone())
            }
        };
        match e {
            None => {
                if let Class::MapIter(t, _, _) = &mut vm.heap.get_mut(g).class {
                    *t = Value::Undefined;
                }
                return Ok(vm.iter_result(Value::Undefined, true));
            }
            Some(None) => pos += 1,
            Some(Some((k, v))) => {
                pos += 1;
                if let Class::MapIter(_, p, _) = &mut vm.heap.get_mut(g).class {
                    *p = pos as u32;
                }
                let r = match kind {
                    0 => k,
                    1 => v,
                    _ => Value::Obj(vm.new_array(vec![k, v])),
                };
                return Ok(vm.iter_result(r, false));
            }
        }
    }
}

// ----------------------------------------------------------------------------------------- the Set methods

/// A set-like argument: its size, has and keys.
fn set_record(vm: &mut Vm, other: &Value) -> Result<(f64, Value, Value), Value> {
    let Value::Obj(o) = other else { return Err(vm.type_error("Set method argument must be an object")) };
    let sz = vm.get(*o, Key::Str(n::SIZE), other)?;
    let size = vm.to_number(&sz)?;
    if size.is_nan() {
        return Err(vm.type_error("The 'size' property must be a number"));
    }
    let size = crate::value::to_integer(size);
    if size < 0.0 {
        return Err(vm.range_error("'size' must not be negative"));
    }
    let ha = vm.atom("has");
    let has = vm.get(*o, Key::Str(ha), other)?;
    if !vm.is_callable(&has) {
        return Err(vm.type_error("'has' is not a function"));
    }
    let ka = vm.atom("keys");
    let keys = vm.get(*o, Key::Str(ka), other)?;
    if !vm.is_callable(&keys) {
        return Err(vm.type_error("'keys' is not a function"));
    }
    Ok((size, has, keys))
}

fn set_values(vm: &mut Vm, g: Gc) -> Vec<Value> {
    md(vm, g).entries.iter().flatten().map(|(k, _)| k.clone()).collect()
}

fn other_keys(vm: &mut Vm, other: &Value, keys: &Value) -> Result<Vec<Value>, Value> {
    let it = vm.call(keys, other.clone(), &[])?;
    if !it.is_object() {
        return Err(vm.type_error("keys() did not return an object"));
    }
    let next = vm.get_value(&it, Key::Str(n::NEXT))?;
    let mut out = Vec::new();
    while let Some(v) = vm.iter_step(&it, &next)? {
        out.push(normalize(v));
    }
    Ok(out)
}

fn new_set(vm: &mut Vm, values: Vec<Value>) -> Value {
    let p = vm.proto(I::SetProto);
    let s = vm.alloc(Some(p), Class::Set(Box::default()));
    for v in values {
        md(vm, s).set(v.clone(), v);
    }
    Value::Obj(s)
}

fn set_union(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "Set.prototype.union")?;
    let other = arg(args, 0);
    let (_, _, keys) = set_record(vm, &other)?;
    let mut vals = set_values(vm, g);
    vals.extend(other_keys(vm, &other, &keys)?);
    Ok(new_set(vm, vals))
}

fn set_intersection(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "Set.prototype.intersection")?;
    let other = arg(args, 0);
    let (size, has, keys) = set_record(vm, &other)?;
    let mine = set_values(vm, g);
    let mut out = Vec::new();
    if (mine.len() as f64) <= size {
        for v in mine {
            if vm.call(&has, other.clone(), &[v.clone()])?.truthy() {
                out.push(v);
            }
        }
    } else {
        for v in other_keys(vm, &other, &keys)? {
            if md(vm, g).has(&v) {
                out.push(v);
            }
        }
    }
    Ok(new_set(vm, out))
}

fn set_difference(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "Set.prototype.difference")?;
    let other = arg(args, 0);
    let (size, has, keys) = set_record(vm, &other)?;
    let mine = set_values(vm, g);
    let result = new_set(vm, mine.clone());
    let Value::Obj(r) = result else { unreachable!() };
    if (mine.len() as f64) <= size {
        for v in mine {
            if vm.call(&has, other.clone(), &[v.clone()])?.truthy() {
                md(vm, r).delete(&v);
            }
        }
    } else {
        for v in other_keys(vm, &other, &keys)? {
            md(vm, r).delete(&v);
        }
    }
    Ok(result)
}

fn set_symmetric_difference(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "Set.prototype.symmetricDifference")?;
    let other = arg(args, 0);
    let (_, _, keys) = set_record(vm, &other)?;
    let result = {
        let mine = set_values(vm, g);
        new_set(vm, mine)
    };
    let Value::Obj(r) = result else { unreachable!() };
    for v in other_keys(vm, &other, &keys)? {
        if md(vm, g).has(&v) {
            md(vm, r).delete(&v);
        } else {
            md(vm, r).set(v.clone(), v);
        }
    }
    Ok(result)
}

fn set_is_subset_of(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "Set.prototype.isSubsetOf")?;
    let other = arg(args, 0);
    let (size, has, _) = set_record(vm, &other)?;
    let mine = set_values(vm, g);
    if mine.len() as f64 > size {
        return Ok(Value::Bool(false));
    }
    for v in mine {
        if !vm.call(&has, other.clone(), &[v])?.truthy() {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn set_is_superset_of(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "Set.prototype.isSupersetOf")?;
    let other = arg(args, 0);
    let (size, _, keys) = set_record(vm, &other)?;
    if (md(vm, g).size as f64) < size {
        return Ok(Value::Bool(false));
    }
    for v in other_keys(vm, &other, &keys)? {
        if !md(vm, g).has(&v) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn set_is_disjoint_from(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_set, "Set.prototype.isDisjointFrom")?;
    let other = arg(args, 0);
    let (size, has, keys) = set_record(vm, &other)?;
    let mine = set_values(vm, g);
    if (mine.len() as f64) <= size {
        for v in mine {
            if vm.call(&has, other.clone(), &[v])?.truthy() {
                return Ok(Value::Bool(false));
            }
        }
    } else {
        for v in other_keys(vm, &other, &keys)? {
            if md(vm, g).has(&v) {
                return Ok(Value::Bool(false));
            }
        }
    }
    Ok(Value::Bool(true))
}

// --------------------------------------------------------------------------------------------- the weak ones

/// CanBeHeldWeakly: an object, or a symbol not from Symbol.for.
fn weak_ok(vm: &Vm, v: &Value) -> bool {
    match v {
        Value::Obj(_) => true,
        Value::Sym(s) => !matches!(vm.heap.get(*s).class, Class::Symbol(_, f) if f & SYM_REGISTERED != 0),
        _ => false,
    }
}

fn is_weakmap(c: &Class) -> bool {
    matches!(c, Class::WeakMap(_))
}

fn is_weakset(c: &Class) -> bool {
    matches!(c, Class::WeakSet(_))
}

fn is_weak(c: &Class) -> bool {
    matches!(c, Class::WeakMap(_) | Class::WeakSet(_))
}

fn weakmap_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let o = new_coll(vm, nt, I::WeakMapProto, Class::WeakMap)?;
    fill(vm, o, &arg(args, 0), "set", true)?;
    Ok(Value::Obj(o))
}

fn weakset_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let o = new_coll(vm, nt, I::WeakSetProto, Class::WeakSet)?;
    fill(vm, o, &arg(args, 0), "add", false)?;
    Ok(Value::Obj(o))
}

fn weak_delete(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_weak, "delete")?;
    Ok(Value::Bool(md(vm, g).delete(&arg(args, 0))))
}

fn weak_has(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_weak, "has")?;
    Ok(Value::Bool(md(vm, g).has(&arg(args, 0))))
}

fn weakmap_get(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_weakmap, "WeakMap.prototype.get")?;
    Ok(md(vm, g).get(&arg(args, 0)).unwrap_or(Value::Undefined))
}

fn weakmap_set(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_weakmap, "WeakMap.prototype.set")?;
    let k = arg(args, 0);
    if !weak_ok(vm, &k) {
        return Err(vm.type_error("Invalid value used as weak map key"));
    }
    md(vm, g).set(k, arg(args, 1));
    Ok(this.clone())
}

fn weakset_add(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = data_of(vm, this, is_weakset, "WeakSet.prototype.add")?;
    let k = arg(args, 0);
    if !weak_ok(vm, &k) {
        return Err(vm.type_error("Invalid value used in weak set"));
    }
    md(vm, g).set(k.clone(), k);
    Ok(this.clone())
}

fn weakref_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let t = arg(args, 0);
    if !weak_ok(vm, &t) {
        return Err(vm.type_error("WeakRef: invalid target"));
    }
    let p = vm.proto_from_constructor(nt, I::WeakRefProto)?;
    Ok(Value::Obj(vm.alloc(Some(p), Class::WeakRef(t))))
}

fn weakref_deref(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    if let Value::Obj(g) = this
        && let Class::WeakRef(t) = &vm.heap.get(*g).class
    {
        return Ok(t.clone());
    }
    Err(vm.type_error("WeakRef.prototype.deref called on incompatible receiver"))
}

fn finreg_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let cb = callable_arg(vm, args, 0)?;
    let o = vm.create_from_constructor(nt, I::FinalizationRegistryProto)?;
    let k = vm.atom("cleanup");
    vm.heap.get_mut(o).props.set(Key::Str(k), Prop::data(cb, 0));
    Ok(Value::Obj(o))
}

fn finreg_register(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = arg(args, 0);
    if !weak_ok(vm, &t) {
        return Err(vm.type_error("FinalizationRegistry.prototype.register: invalid target"));
    }
    if crate::value::same_value(&t, &arg(args, 1)) {
        return Err(vm.type_error("FinalizationRegistry.prototype.register: target and holdings must not be the same"));
    }
    Ok(Value::Undefined)
}

fn finreg_unregister(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    if !weak_ok(vm, &arg(args, 0)) {
        return Err(vm.type_error("Invalid unregisterToken"));
    }
    Ok(Value::Bool(false))
}
