//! Properties: the object internal methods ([[Get]], [[Set]], [[DefineOwnProperty]] and the rest) for ordinary
//! objects, the exotic ones (arrays: elements and length; strings; typed arrays: their buffer's elements) and
//! proxies (each call to its handler's trap, and the invariants the language holds traps to).

use crate::names as n;
use crate::object::*;
use crate::string::{Atom, JsStr};
use crate::value::{Gc, Value, array_index, same_value};
use crate::vm::{I, Sym, Vm};
use alloc::format;
use alloc::vec::Vec;

/// A property descriptor: each field may be absent.
#[derive(Clone, Default, Debug)]
pub struct Desc {
    pub value: Option<Value>,
    pub writable: Option<bool>,
    pub get: Option<Value>,
    pub set: Option<Value>,
    pub enumerable: Option<bool>,
    pub configurable: Option<bool>,
}

impl Desc {
    pub fn data(v: Value, w: bool, e: bool, c: bool) -> Desc {
        Desc { value: Some(v), writable: Some(w), enumerable: Some(e), configurable: Some(c), ..Desc::default() }
    }

    pub fn is_accessor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }

    pub fn is_data(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_none() && self.writable.is_none() && self.get.is_none() && self.set.is_none() && self.enumerable.is_none() && self.configurable.is_none()
    }

    pub fn from_prop(p: &Prop) -> Desc {
        match &p.slot {
            Slot::Data(v) => Desc::data(v.clone(), p.writable(), p.enumerable(), p.configurable()),
            Slot::Accessor(g, s) => Desc { get: Some(g.clone()), set: Some(s.clone()), enumerable: Some(p.enumerable()), configurable: Some(p.configurable()), ..Desc::default() },
        }
    }

    /// The property it describes, its absent fields their defaults.
    pub fn to_prop(&self) -> Prop {
        let mut flags = 0;
        if self.enumerable == Some(true) {
            flags |= ENUMERABLE;
        }
        if self.configurable == Some(true) {
            flags |= CONFIGURABLE;
        }
        if self.is_accessor() {
            return Prop { slot: Slot::Accessor(self.get.clone().unwrap_or(Value::Undefined), self.set.clone().unwrap_or(Value::Undefined)), flags };
        }
        if self.writable == Some(true) {
            flags |= WRITABLE;
        }
        Prop { slot: Slot::Data(self.value.clone().unwrap_or(Value::Undefined)), flags }
    }
}

/// How many holes an array's elements may grow by in one write before the element goes to its properties.
const GAP: usize = 1024;

impl Vm {
    /// A name's key: an array index's text is an index.
    pub fn atom_key(&self, a: Atom) -> Key {
        let s = self.atoms.get(a);
        match s.at(0) {
            Some(c) if (b'0' as u16..=b'9' as u16).contains(&c) => match array_index(s.units()) {
                Some(i) => Key::Index(i),
                None => Key::Str(a),
            },
            _ => Key::Str(a),
        }
    }

    pub fn key_from_str(&mut self, s: &JsStr) -> Key {
        if let Some(i) = array_index(s.units()) {
            return Key::Index(i);
        }
        Key::Str(self.atoms.intern(s))
    }

    /// A key as a value (a string, or a symbol).
    pub fn key_value(&self, k: Key) -> Value {
        match k {
            Key::Str(a) => Value::Str(self.str_of(a)),
            Key::Index(i) => Value::Str(JsStr::from_str(&format!("{i}"))),
            Key::Sym(s) => Value::Sym(s),
        }
    }

    pub fn is_private(&self, k: Key) -> bool {
        matches!(k, Key::Sym(s) if matches!(self.heap.get(s).class, Class::Symbol(_, f) if f & SYM_PRIVATE != 0))
    }

    // --------------------------------------------------------------------------------------------- typed arrays

    /// A typed array's length now (0 if its buffer is detached or too short).
    pub fn typed_length(&self, t: &TypedArray) -> usize {
        let buf_len = match &self.heap.get(t.buffer).class {
            Class::ArrayBuffer(Some(b), _) => b.len(),
            _ => return 0,
        };
        match t.length {
            Some(l) if t.offset + l * t.kind.size() <= buf_len => l,
            Some(_) => 0,
            None if buf_len >= t.offset => (buf_len - t.offset) / t.kind.size(),
            None => 0,
        }
    }

    pub fn typed_get(&self, t: &TypedArray, i: u32) -> Option<Value> {
        let i = i as usize;
        if i >= self.typed_length(t) {
            return None;
        }
        let Class::ArrayBuffer(Some(b), _) = &self.heap.get(t.buffer).class else { return None };
        let at = t.offset + i * t.kind.size();
        Some(crate::builtins::typed::read_elem(t.kind, &b[at..at + t.kind.size()], true))
    }

    /// IsTypedArrayOutOfBounds: its buffer detached, or too short for it.
    pub fn typed_out_of_bounds(&self, t: &TypedArray) -> bool {
        let Class::ArrayBuffer(Some(b), _) = &self.heap.get(t.buffer).class else { return true };
        match t.length {
            Some(l) => t.offset + l * t.kind.size() > b.len(),
            None => t.offset > b.len(),
        }
    }

    /// Writes a typed array's element (the value already converted to its kind).
    pub fn typed_put(&mut self, g: Gc, i: usize, v: &Value) {
        let (kind, buffer, offset, len) = match &self.heap.get(g).class {
            Class::TypedArray(t) => (t.kind, t.buffer, t.offset, self.typed_length(t)),
            _ => return,
        };
        if i >= len {
            return;
        }
        if let Class::ArrayBuffer(Some(b), _) = &mut self.heap.get_mut(buffer).class {
            let at = offset + i * kind.size();
            crate::builtins::typed::write_elem(kind, &mut b[at..at + kind.size()], v, true);
        }
    }

    /// TypedArraySetElement: the value converted (which may run code), then written if the index is still valid.
    pub fn typed_set(&mut self, g: Gc, i: u32, v: &Value) -> Result<(), Value> {
        let kind = match &self.heap.get(g).class {
            Class::TypedArray(t) => t.kind,
            _ => return Ok(()),
        };
        let nv = if matches!(kind, ElemKind::BigInt64 | ElemKind::BigUint64) { Value::Big(alloc::rc::Rc::new(self.to_bigint(v)?)) } else { Value::Num(self.to_number(v)?) };
        self.typed_put(g, i as usize, &nv);
        Ok(())
    }

    /// A typed array's view of a key (CanonicalNumericIndexString): None for an ordinary property's, Some(Some(i)) for
    /// an element's index, and Some(None) for a number that's never an element ("-0", "1.5", "-1", "Infinity"...).
    pub fn typed_key(&self, k: Key) -> Option<Option<u32>> {
        match k {
            Key::Index(i) => Some(Some(i)),
            Key::Sym(_) => None,
            Key::Str(a) => {
                let s = self.atoms.get(a);
                let c = s.at(0)?;
                if !(c == b'-' as u16 || c == b'I' as u16 || c == b'N' as u16 || (b'0' as u16..=b'9' as u16).contains(&c)) {
                    return None;
                }
                if s.units() == [b'-' as u16, b'0' as u16] {
                    return Some(None);
                }
                let back = crate::value::number_to_string(crate::value::string_to_number(s));
                if back.encode_utf16().eq(s.units().iter().copied()) { Some(None) } else { None }
            }
        }
    }

    /// A mapped arguments object's element's parameter: its environment and slot.
    fn arg_slot(&self, g: Gc, k: Key) -> Option<(Gc, u16)> {
        if let (Class::Arguments(Some(m)), Key::Index(i)) = (&self.heap.get(g).class, k)
            && let Some(Some(s)) = m.slots.get(i as usize)
        {
            return Some((m.env, *s));
        }
        None
    }

    fn slot_value(&self, env: Gc, s: u16) -> Value {
        match &self.heap.get(env).class {
            Class::Env(e) => e.vars.get(s as usize).cloned().unwrap_or(Value::Undefined),
            _ => Value::Undefined,
        }
    }

    fn set_slot(&mut self, env: Gc, s: u16, v: Value) {
        if let Class::Env(e) = &mut self.heap.get_mut(env).class
            && let Some(x) = e.vars.get_mut(s as usize)
        {
            *x = v;
        }
    }

    /// An arguments object's element no longer its parameter.
    fn unmap(&mut self, g: Gc, i: u32) {
        if let Class::Arguments(Some(m)) = &mut self.heap.get_mut(g).class
            && let Some(s) = m.slots.get_mut(i as usize)
        {
            *s = None;
        }
    }

    fn typed_valid(&self, g: Gc, i: u32) -> bool {
        match &self.heap.get(g).class {
            Class::TypedArray(t) => (i as usize) < self.typed_length(t),
            _ => false,
        }
    }

    // --------------------------------------------------------------------------------------------- own properties

    /// [[GetOwnProperty]] of an object that isn't a proxy.
    pub fn own_prop(&self, g: Gc, k: Key) -> Option<Prop> {
        let o = self.heap.get(g);
        match (&o.class, k) {
            (Class::Array(els, _), Key::Index(i)) => {
                if let Some(v) = els.get(i as usize)
                    && !matches!(v, Value::Empty)
                {
                    return Some(Prop::data(v.clone(), ALL));
                }
            }
            (Class::Array(_, len), Key::Str(n::LENGTH)) => {
                let w = o.props.get(&Key::Str(n::LENGTH)).map_or(WRITABLE, |p| p.flags & WRITABLE);
                return Some(Prop::data(Value::Num(*len as f64), w));
            }
            (Class::String(s), Key::Index(i)) if (i as usize) < s.len() => return Some(Prop::data(Value::Str(s.slice(i as usize, i as usize + 1)), ENUMERABLE)),
            (Class::String(s), Key::Str(n::LENGTH)) => return Some(Prop::data(Value::Num(s.len() as f64), 0)),
            (Class::Arguments(Some(_)), Key::Index(_)) => {
                let mut p = o.props.get(&k).cloned()?;
                if let Some((env, s)) = self.arg_slot(g, k) {
                    p.slot = Slot::Data(self.slot_value(env, s));
                }
                return Some(p);
            }
            (Class::TypedArray(t), _) => match self.typed_key(k) {
                Some(Some(i)) => return self.typed_get(t, i).map(|v| Prop::data(v, ALL)),
                Some(None) => return None,
                None => {}
            },
            _ => {}
        }
        o.props.get(&k).cloned()
    }

    /// [[GetOwnProperty]].
    pub fn get_own_property(&mut self, g: Gc, k: Key) -> Result<Option<Prop>, Value> {
        match self.heap.get(g).class {
            Class::Proxy(..) => return self.proxy_get_own(g, k),
            Class::Namespace(_) => return self.ns_own(g, k),
            _ => {}
        }
        Ok(self.own_prop(g, k))
    }

    /// [[Get]].
    pub fn get(&mut self, g: Gc, k: Key, receiver: &Value) -> Result<Value, Value> {
        let mut cur = g;
        loop {
            let o = self.heap.get(cur);
            match o.class {
                Class::Proxy(..) => return self.proxy_get(cur, k, receiver),
                Class::Namespace(_) => return self.ns_get(cur, k),
                _ => {}
            }
            // (The common case first: an ordinary object's own data property.)
            let p = match (&o.class, k) {
                (Class::Ordinary | Class::Function(_) | Class::Native(_) | Class::Error | Class::Arguments(None), _) => o.props.get(&k).cloned(),
                _ => self.own_prop(cur, k),
            };
            match p {
                Some(Prop { slot: Slot::Data(v), .. }) => return Ok(v),
                Some(Prop { slot: Slot::Accessor(getter, _), .. }) => {
                    return if getter.is_undefined() { Ok(Value::Undefined) } else { self.call(&getter, receiver.clone(), &[]) };
                }
                None => {}
            }
            let o = self.heap.get(cur);
            if let Class::TypedArray(_) = o.class
                && self.typed_key(k).is_some()
            {
                return Ok(Value::Undefined);
            }
            match o.proto {
                Some(p) => cur = p,
                None => return Ok(Value::Undefined),
            }
        }
    }

    /// A property of any value (a primitive's from its prototype).
    pub fn get_value(&mut self, v: &Value, k: Key) -> Result<Value, Value> {
        let p = match v {
            Value::Obj(g) => return self.get(*g, k, v),
            Value::Str(s) => {
                match k {
                    Key::Index(i) if (i as usize) < s.len() => return Ok(Value::Str(s.slice(i as usize, i as usize + 1))),
                    Key::Str(n::LENGTH) => return Ok(Value::Num(s.len() as f64)),
                    _ => {}
                }
                I::StringProto
            }
            Value::Num(_) => I::NumberProto,
            Value::Bool(_) => I::BooleanProto,
            Value::Sym(_) => I::SymbolProto,
            Value::Big(_) => I::BigIntProto,
            Value::Undefined | Value::Null | Value::Empty => {
                let name = self.key_value(k);
                let name = match name {
                    Value::Str(s) => s.to_rust(),
                    _ => alloc::string::String::from("Symbol()"),
                };
                return Err(self.type_error(&format!("Cannot read properties of {} (reading '{}')", self.describe(v), name)));
            }
        };
        let p = self.proto(p);
        self.get(p, k, v)
    }

    /// obj[key] (key any value).
    pub fn get_elem(&mut self, o: &Value, k: &Value) -> Result<Value, Value> {
        if let (Value::Obj(g), Value::Num(x)) = (o, k)
            && let Class::Array(els, _) = &self.heap.get(*g).class
        {
            let i = *x as usize;
            if i as f64 == *x && i < els.len() && !matches!(els[i], Value::Empty) {
                return Ok(els[i].clone());
            }
        }
        if o.is_nullish() {
            let key = self.to_key(k)?;
            return self.get_value(o, key);
        }
        let key = self.to_key(k)?;
        self.get_value(o, key)
    }

    /// GetMethod: a property that's undefined, null or a function.
    pub fn get_method(&mut self, v: &Value, k: Key) -> Result<Value, Value> {
        let f = self.get_value(v, k)?;
        if f.is_nullish() {
            return Ok(Value::Undefined);
        }
        if !self.is_callable(&f) {
            return Err(self.type_error(&format!("{} is not a function", self.describe(&f))));
        }
        Ok(f)
    }

    /// [[Set]]: true if it was set.
    pub fn set(&mut self, g: Gc, k: Key, v: Value, receiver: &Value) -> Result<bool, Value> {
        // Fast: an own writable data property, the receiver the object itself.
        if let Value::Obj(r) = receiver
            && *r == g
        {
            let o = self.heap.get_mut(g);
            match (&mut o.class, k) {
                (Class::Array(els, _), Key::Index(i)) => {
                    if (i as usize) < els.len() && !matches!(els[i as usize], Value::Empty) {
                        els[i as usize] = v;
                        return Ok(true);
                    }
                }
                (Class::Ordinary | Class::Function(_) | Class::Native(_) | Class::Error | Class::Arguments(None), _) => {
                    if let Some(p) = o.props.get_mut(&k)
                        && let Slot::Data(cur) = &mut p.slot
                        && p.flags & WRITABLE != 0
                    {
                        *cur = v;
                        return Ok(true);
                    }
                }
                _ => {}
            }
        }
        let mut cur = g;
        let found = loop {
            match self.heap.get(cur).class {
                Class::Proxy(..) => return self.proxy_set(cur, k, v, receiver),
                Class::Namespace(_) => return Ok(false),
                _ => {}
            }
            if let Class::TypedArray(_) = self.heap.get(cur).class
                && let Some(i) = self.typed_key(k)
            {
                if matches!(receiver, Value::Obj(r) if *r == cur) {
                    // (The value converted even when the index isn't an element's.)
                    self.typed_set(cur, i.unwrap_or(u32::MAX), &v)?;
                    return Ok(true);
                }
                if !i.is_some_and(|i| self.typed_valid(cur, i)) {
                    return Ok(true);
                }
            }
            match self.own_prop(cur, k) {
                Some(p) => break Some(p),
                None => match self.heap.get(cur).proto {
                    Some(p) => cur = p,
                    None => break None,
                },
            }
        };
        match found {
            Some(Prop { slot: Slot::Accessor(_, setter), .. }) => {
                if setter.is_undefined() {
                    return Ok(false);
                }
                self.call(&setter, receiver.clone(), &[v])?;
                Ok(true)
            }
            Some(Prop { slot: Slot::Data(_), flags }) if flags & WRITABLE == 0 => Ok(false),
            _ => {
                let Value::Obj(r) = receiver else { return Ok(false) };
                let r = *r;
                match self.get_own_property(r, k)? {
                    Some(Prop { slot: Slot::Accessor(..), .. }) => Ok(false),
                    Some(Prop { slot: Slot::Data(_), flags }) => {
                        if flags & WRITABLE == 0 {
                            return Ok(false);
                        }
                        self.define_own_property(r, k, Desc { value: Some(v), ..Desc::default() })
                    }
                    None => self.define_own_property(r, k, Desc::data(v, true, true, true)),
                }
            }
        }
    }

    /// An assignment to a property of any value (strict code throws when it isn't set).
    pub fn put_value(&mut self, o: &Value, k: Key, v: Value, strict: bool) -> Result<(), Value> {
        let ok = match o {
            Value::Obj(g) => self.set(*g, k, v, o)?,
            Value::Undefined | Value::Null | Value::Empty => {
                let name = match self.key_value(k) {
                    Value::Str(s) => s.to_rust(),
                    _ => alloc::string::String::from("Symbol()"),
                };
                return Err(self.type_error(&format!("Cannot set properties of {} (setting '{}')", self.describe(o), name)));
            }
            _ => {
                let p = match o {
                    Value::Str(_) => I::StringProto,
                    Value::Num(_) => I::NumberProto,
                    Value::Bool(_) => I::BooleanProto,
                    Value::Sym(_) => I::SymbolProto,
                    _ => I::BigIntProto,
                };
                if let (Value::Str(s), Key::Index(i)) = (o, k)
                    && (i as usize) < s.len()
                {
                    false
                } else if let (Value::Str(_), Key::Str(n::LENGTH)) = (o, k) {
                    false
                } else {
                    let p = self.proto(p);
                    self.set(p, k, v, o)?
                }
            }
        };
        if !ok && strict {
            let name = match self.key_value(k) {
                Value::Str(s) => s.to_rust(),
                _ => alloc::string::String::from("Symbol()"),
            };
            return Err(self.type_error(&format!("Cannot assign to read only property '{name}' of {}", self.describe(o))));
        }
        Ok(())
    }

    /// [[HasProperty]].
    pub fn has_property(&mut self, g: Gc, k: Key) -> Result<bool, Value> {
        let mut cur = g;
        loop {
            match self.heap.get(cur).class {
                Class::Proxy(..) => return self.proxy_has(cur, k),
                Class::Namespace(_) => return Ok(self.ns_has(cur, k)),
                _ => {}
            }
            if let Class::TypedArray(_) = self.heap.get(cur).class
                && let Some(i) = self.typed_key(k)
            {
                return Ok(i.is_some_and(|i| self.typed_valid(cur, i)));
            }
            if self.own_prop(cur, k).is_some() {
                return Ok(true);
            }
            match self.heap.get(cur).proto {
                Some(p) => cur = p,
                None => return Ok(false),
            }
        }
    }

    pub fn has_own_property(&mut self, g: Gc, k: Key) -> Result<bool, Value> {
        Ok(self.get_own_property(g, k)?.is_some())
    }

    /// [[Delete]].
    pub fn delete_property(&mut self, g: Gc, k: Key) -> Result<bool, Value> {
        let o = self.heap.get_mut(g);
        match (&mut o.class, k) {
            (Class::Proxy(..), _) => return self.proxy_delete(g, k),
            (Class::Namespace(_), Key::Str(_) | Key::Index(_)) => return Ok(self.ns_delete(g, k)),
            (Class::Array(els, _), Key::Index(i)) => {
                let i = i as usize;
                if i < els.len() && !matches!(els[i], Value::Empty) {
                    els[i] = Value::Empty;
                    while matches!(els.last(), Some(Value::Empty)) {
                        els.pop();
                    }
                    return Ok(true);
                }
            }
            (Class::Array(..), Key::Str(n::LENGTH)) => return Ok(false),
            (Class::String(s), Key::Index(i)) if (i as usize) < s.len() => return Ok(false),
            (Class::String(_), Key::Str(n::LENGTH)) => return Ok(false),
            (Class::TypedArray(_), _) => {
                if let Some(i) = self.typed_key(k) {
                    return Ok(!i.is_some_and(|i| self.typed_valid(g, i)));
                }
            }
            (Class::Arguments(Some(_)), Key::Index(i)) => {
                let configurable = self.heap.get(g).props.get(&k).is_none_or(|p| p.configurable());
                if configurable {
                    self.heap.get_mut(g).props.remove(&k);
                    self.unmap(g, i);
                }
                return Ok(configurable);
            }
            _ => {}
        }
        let o = self.heap.get_mut(g);
        match o.props.get(&k) {
            None => Ok(true),
            Some(p) if p.configurable() => {
                o.props.remove(&k);
                Ok(true)
            }
            Some(_) => Ok(false),
        }
    }

    /// delete o[k] (strict code throws when it can't).
    pub fn delete_value(&mut self, o: &Value, k: Key, strict: bool) -> Result<bool, Value> {
        let g = self.to_object(o)?;
        let ok = self.delete_property(g, k)?;
        if !ok && strict {
            return Err(self.type_error("Cannot delete property"));
        }
        Ok(ok)
    }

    /// [[OwnPropertyKeys]]: indices ascending, then names and symbols in the order they were made.
    pub fn own_keys(&mut self, g: Gc) -> Result<Vec<Key>, Value> {
        match self.heap.get(g).class {
            Class::Proxy(..) => return self.proxy_own_keys(g),
            Class::Namespace(_) => return Ok(self.ns_keys(g)),
            _ => {}
        }
        Ok(self.own_keys_plain(g))
    }

    pub fn own_keys_plain(&self, g: Gc) -> Vec<Key> {
        let o = self.heap.get(g);
        let mut out: Vec<Key> = Vec::new();
        match &o.class {
            Class::Array(els, _) => {
                let mut idx: Vec<u32> = els.iter().enumerate().filter(|(_, v)| !matches!(v, Value::Empty)).map(|(i, _)| i as u32).collect();
                let extra: Vec<u32> = o.props.iter().filter_map(|(k, _)| if let Key::Index(i) = k { Some(*i) } else { None }).collect();
                if !extra.is_empty() {
                    idx.extend(extra);
                    idx.sort_unstable();
                    idx.dedup();
                }
                out.extend(idx.into_iter().map(Key::Index));
                out.push(Key::Str(n::LENGTH));
                for k in o.props.keys() {
                    if !matches!(k, Key::Index(_)) && k != Key::Str(n::LENGTH) {
                        out.push(k);
                    }
                }
            }
            Class::String(s) => {
                out.extend((0..s.len() as u32).map(Key::Index));
                for k in o.props.keys() {
                    if let Key::Index(_) = k {
                        out.push(k);
                    }
                }
                out.push(Key::Str(n::LENGTH));
                for k in o.props.keys() {
                    if !matches!(k, Key::Index(_)) {
                        out.push(k);
                    }
                }
            }
            Class::TypedArray(t) => {
                out.extend((0..self.typed_length(t) as u32).map(Key::Index));
                out.extend(o.props.keys().into_iter().filter(|k| !matches!(k, Key::Index(_))));
            }
            _ => out = o.props.keys(),
        }
        out.retain(|k| !self.is_private(*k));
        out
    }

    // ----------------------------------------------------------------------------------------------- defining

    /// [[DefineOwnProperty]]: true if it was defined.
    pub fn define_own_property(&mut self, g: Gc, k: Key, desc: Desc) -> Result<bool, Value> {
        match (&self.heap.get(g).class, k) {
            (Class::Proxy(..), _) => return self.proxy_define(g, k, desc),
            (Class::Namespace(_), Key::Str(_) | Key::Index(_)) => return self.ns_define(g, k, desc),
            (Class::Array(..), Key::Str(n::LENGTH)) => return self.array_define_length(g, desc),
            (Class::Array(_, len), Key::Index(i)) => {
                let len = *len;
                let len_writable = self.heap.get(g).props.get(&Key::Str(n::LENGTH)).is_none_or(|p| p.writable());
                if i >= len && !len_writable {
                    return Ok(false);
                }
                if !self.ordinary_define(g, k, desc) {
                    return Ok(false);
                }
                if i >= len
                    && let Class::Array(_, l) = &mut self.heap.get_mut(g).class
                {
                    *l = i + 1;
                }
                return Ok(true);
            }
            (Class::Arguments(Some(_)), Key::Index(i)) if self.arg_slot(g, k).is_some() => {
                let Some((env, s)) = self.arg_slot(g, k) else { return Ok(false) };
                let mut d = desc.clone();
                if !desc.is_accessor() && desc.value.is_none() && desc.writable == Some(false) {
                    d.value = Some(self.slot_value(env, s));
                }
                if !self.ordinary_define(g, k, d) {
                    return Ok(false);
                }
                if desc.is_accessor() {
                    self.unmap(g, i);
                } else {
                    if let Some(v) = desc.value {
                        self.set_slot(env, s, v);
                    }
                    if desc.writable == Some(false) {
                        self.unmap(g, i);
                    }
                }
                return Ok(true);
            }
            (Class::TypedArray(_), _) if self.typed_key(k).is_some() => {
                let Some(i) = self.typed_key(k).flatten().filter(|i| self.typed_valid(g, *i)) else { return Ok(false) };
                if desc.configurable == Some(false) || desc.enumerable == Some(false) || desc.is_accessor() || desc.writable == Some(false) {
                    return Ok(false);
                }
                if let Some(v) = desc.value {
                    self.typed_set(g, i, &v)?;
                }
                return Ok(true);
            }
            _ => {}
        }
        Ok(self.ordinary_define(g, k, desc))
    }

    pub fn define_own_property_or_throw(&mut self, g: Gc, k: Key, desc: Desc) -> Result<(), Value> {
        if self.define_own_property(g, k, desc)? {
            return Ok(());
        }
        let name = match self.key_value(k) {
            Value::Str(s) => s.to_rust(),
            _ => alloc::string::String::from("Symbol()"),
        };
        Err(self.type_error(&format!("Cannot redefine property: {name}")))
    }

    /// CreateDataProperty.
    pub fn create_data_property(&mut self, g: Gc, k: Key, v: Value) -> Result<bool, Value> {
        // Fast: a new property of an ordinary extensible object, or the next element of an array.
        let o = self.heap.get_mut(g);
        if o.extensible {
            match (&mut o.class, k) {
                (Class::Ordinary, _) if o.props.get(&k).is_none() => {
                    o.props.set(k, Prop::data(v, ALL));
                    if matches!(k, Key::Index(_)) && g == self.intrinsics[I::ObjectProto as usize] {
                        self.proto_indexes = true;
                    }
                    return Ok(true);
                }
                (Class::Array(els, len), Key::Index(i)) if i as usize == els.len() && i == *len && o.props.is_empty() => {
                    els.push(v);
                    *len += 1;
                    return Ok(true);
                }
                _ => {}
            }
        }
        self.define_own_property(g, k, Desc::data(v, true, true, true))
    }

    pub fn create_data_property_or_throw(&mut self, g: Gc, k: Key, v: Value) -> Result<(), Value> {
        if self.create_data_property(g, k, v)? {
            return Ok(());
        }
        Err(self.type_error("Cannot define property"))
    }

    fn ordinary_define(&mut self, g: Gc, k: Key, desc: Desc) -> bool {
        let current = self.own_prop(g, k);
        let extensible = self.heap.get(g).extensible;
        if let (Class::String(s), Key::Index(i)) = (&self.heap.get(g).class, k)
            && (i as usize) < s.len()
        {
            return validate(&desc, current.as_ref(), extensible).is_some();
        }
        match validate(&desc, current.as_ref(), extensible) {
            Some(p) => {
                self.write_prop(g, k, p);
                true
            }
            None => false,
        }
    }

    /// Stores a property (an array's plain element in its elements).
    fn write_prop(&mut self, g: Gc, k: Key, p: Prop) {
        if matches!(k, Key::Index(_)) && (g == self.intrinsics[I::ObjectProto as usize] || g == self.intrinsics[I::ArrayProto as usize]) {
            self.proto_indexes = true;
        }
        let o = self.heap.get_mut(g);
        if let (Class::Array(els, _), Key::Index(i)) = (&mut o.class, k) {
            let i = i as usize;
            if let (Slot::Data(v), ALL) = (&p.slot, p.flags) {
                if i < els.len() {
                    els[i] = v.clone();
                    o.props.remove(&k);
                    return;
                }
                if i <= els.len() + GAP && !o.props.iter().any(|(k, _)| matches!(k, Key::Index(_))) {
                    els.resize(i, Value::Empty);
                    els.push(v.clone());
                    return;
                }
            } else if i < els.len() {
                els[i] = Value::Empty;
            }
        }
        o.props.set(k, p);
    }

    /// An array's length defined (ArraySetLength): elements past it deleted.
    fn array_define_length(&mut self, g: Gc, desc: Desc) -> Result<bool, Value> {
        let cur_writable = self.heap.get(g).props.get(&Key::Str(n::LENGTH)).is_none_or(|p| p.writable());
        let Some(v) = desc.value.clone() else {
            // Attributes only.
            if desc.configurable == Some(true) || desc.enumerable == Some(true) || desc.is_accessor() || (!cur_writable && desc.writable == Some(true)) {
                return Ok(false);
            }
            if desc.writable == Some(false) {
                self.heap.get_mut(g).props.set(Key::Str(n::LENGTH), Prop::data(Value::Undefined, 0));
            }
            return Ok(true);
        };
        let new_len = self.to_uint32(&v)?;
        let num = self.to_number(&v)?;
        if new_len as f64 != num {
            return Err(self.range_error("Invalid array length"));
        }
        // (Whether it's writable is read after the conversions, which may have frozen the array.)
        let cur_writable = self.heap.get(g).props.get(&Key::Str(n::LENGTH)).is_none_or(|p| p.writable());
        if desc.configurable == Some(true) || desc.enumerable == Some(true) || desc.is_accessor() {
            return Ok(false);
        }
        let len = match &self.heap.get(g).class {
            Class::Array(_, l) => *l,
            _ => 0,
        };
        if !cur_writable {
            return Ok(new_len == len && desc.writable != Some(true));
        }
        let mut ok = true;
        let mut final_len = new_len;
        if new_len < len {
            let o = self.heap.get_mut(g);
            if let Class::Array(els, _) = &mut o.class
                && els.len() > new_len as usize
            {
                els.truncate(new_len as usize);
            }
            // Elements kept as properties: deleted from the last, stopping at one that can't be.
            let mut idx: Vec<u32> = o.props.iter().filter_map(|(k, _)| if let Key::Index(i) = k { Some(*i) } else { None }).filter(|i| *i >= new_len).collect();
            idx.sort_unstable_by(|a, b| b.cmp(a));
            for i in idx {
                if o.props.get(&Key::Index(i)).is_some_and(|p| !p.configurable()) {
                    final_len = i + 1;
                    ok = false;
                    break;
                }
                o.props.remove(&Key::Index(i));
            }
        }
        if let Class::Array(_, l) = &mut self.heap.get_mut(g).class {
            *l = final_len;
        }
        if desc.writable == Some(false) {
            self.heap.get_mut(g).props.set(Key::Str(n::LENGTH), Prop::data(Value::Undefined, 0));
        }
        Ok(ok)
    }

    /// CopyDataProperties: the enumerable own properties of `src` copied to `target` (but `excluded`).
    pub fn copy_data_properties(&mut self, target: Gc, src: &Value, excluded: &[Key]) -> Result<(), Value> {
        if src.is_nullish() {
            return Ok(());
        }
        let from = self.to_object(src)?;
        let keys = self.own_keys(from)?;
        for k in keys {
            if excluded.contains(&k) {
                continue;
            }
            if let Some(p) = self.get_own_property(from, k)?
                && p.enumerable()
            {
                let v = self.get(from, k, &Value::Obj(from))?;
                self.create_data_property(target, k, v)?;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------ prototypes

    pub fn get_prototype_of(&mut self, g: Gc) -> Result<Option<Gc>, Value> {
        if let Class::Proxy(..) = self.heap.get(g).class {
            return self.proxy_get_proto(g);
        }
        Ok(self.heap.get(g).proto)
    }

    pub fn set_prototype_of(&mut self, g: Gc, p: Option<Gc>) -> Result<bool, Value> {
        if let Class::Proxy(..) = self.heap.get(g).class {
            return self.proxy_set_proto(g, p);
        }
        let o = self.heap.get(g);
        if o.proto == p {
            return Ok(true);
        }
        // %Object.prototype%'s prototype can't change.
        if g == self.proto(I::ObjectProto) || !o.extensible {
            return Ok(false);
        }
        let mut cur = p;
        while let Some(c) = cur {
            if c == g {
                return Ok(false);
            }
            if let Class::Proxy(..) = self.heap.get(c).class {
                break;
            }
            cur = self.heap.get(c).proto;
        }
        self.heap.get_mut(g).proto = p;
        Ok(true)
    }

    pub fn is_extensible(&mut self, g: Gc) -> Result<bool, Value> {
        if let Class::Proxy(..) = self.heap.get(g).class {
            return self.proxy_is_extensible(g);
        }
        Ok(self.heap.get(g).extensible)
    }

    pub fn prevent_extensions(&mut self, g: Gc) -> Result<bool, Value> {
        if let Class::Proxy(..) = self.heap.get(g).class {
            return self.proxy_prevent_extensions(g);
        }
        // (A typed array whose length can change can't be made non-extensible.)
        if let Class::TypedArray(t) = &self.heap.get(g).class
            && (t.length.is_none() || matches!(self.heap.get(t.buffer).class, Class::ArrayBuffer(_, Some(_))))
        {
            return Ok(false);
        }
        self.heap.get_mut(g).extensible = false;
        Ok(true)
    }

    // ------------------------------------------------------------------------------------------ descriptors

    /// ToPropertyDescriptor.
    pub fn to_descriptor(&mut self, v: &Value) -> Result<Desc, Value> {
        let Value::Obj(g) = v else { return Err(self.type_error("Property description must be an object")) };
        let g = *g;
        let mut d = Desc::default();
        let fields = [n::ENUMERABLE, n::CONFIGURABLE, n::VALUE, n::WRITABLE];
        for (i, f) in fields.iter().enumerate() {
            if self.has_property(g, Key::Str(*f))? {
                let x = self.get(g, Key::Str(*f), v)?;
                match i {
                    0 => d.enumerable = Some(x.truthy()),
                    1 => d.configurable = Some(x.truthy()),
                    2 => d.value = Some(x),
                    _ => d.writable = Some(x.truthy()),
                }
            }
        }
        let (get, set) = (n::GET, n::SET);
        if self.has_property(g, Key::Str(get))? {
            let x = self.get(g, Key::Str(get), v)?;
            if !x.is_undefined() && !self.is_callable(&x) {
                return Err(self.type_error("Getter must be a function"));
            }
            d.get = Some(x);
        }
        if self.has_property(g, Key::Str(set))? {
            let x = self.get(g, Key::Str(set), v)?;
            if !x.is_undefined() && !self.is_callable(&x) {
                return Err(self.type_error("Setter must be a function"));
            }
            d.set = Some(x);
        }
        if d.is_accessor() && d.is_data() {
            return Err(self.type_error("Invalid property descriptor. Cannot both specify accessors and a value or writable attribute"));
        }
        Ok(d)
    }

    /// FromPropertyDescriptor.
    pub fn from_descriptor(&mut self, d: &Desc) -> Value {
        let o = self.new_object();
        if let Some(v) = &d.value {
            self.put_key(o, Key::Str(n::VALUE), v.clone(), ALL);
        }
        if let Some(w) = d.writable {
            self.put_key(o, Key::Str(n::WRITABLE), Value::Bool(w), ALL);
        }
        if let Some(g) = &d.get {
            self.put_key(o, Key::Str(n::GET), g.clone(), ALL);
        }
        if let Some(s) = &d.set {
            self.put_key(o, Key::Str(n::SET), s.clone(), ALL);
        }
        if let Some(e) = d.enumerable {
            self.put_key(o, Key::Str(n::ENUMERABLE), Value::Bool(e), ALL);
        }
        if let Some(c) = d.configurable {
            self.put_key(o, Key::Str(n::CONFIGURABLE), Value::Bool(c), ALL);
        }
        Value::Obj(o)
    }

    // ----------------------------------------------------------------------------------------------- proxies

    /// A proxy's target and handler, and its trap (undefined if none); a revoked proxy throws.
    fn trap(&mut self, g: Gc, name: &str) -> Result<(Gc, Gc, Value), Value> {
        let Class::Proxy(Some((t, h)), _) = self.heap.get(g).class else {
            return Err(self.type_error(&format!("Cannot perform '{name}' on a proxy that has been revoked")));
        };
        let a = self.atom(name);
        let trap = self.get_method(&Value::Obj(h), Key::Str(a))?;
        Ok((t, h, trap))
    }

    fn proxy_get_proto(&mut self, g: Gc) -> Result<Option<Gc>, Value> {
        let (t, h, trap) = self.trap(g, "getPrototypeOf")?;
        if trap.is_undefined() {
            return self.get_prototype_of(t);
        }
        let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t)])?;
        let p = match r {
            Value::Obj(p) => Some(p),
            Value::Null => None,
            _ => return Err(self.type_error("'getPrototypeOf' on proxy: trap returned neither object nor null")),
        };
        if !self.is_extensible(t)? && self.get_prototype_of(t)? != p {
            return Err(self.type_error("'getPrototypeOf' on proxy: proxy target is non-extensible but the trap did not return its actual prototype"));
        }
        Ok(p)
    }

    fn proxy_set_proto(&mut self, g: Gc, p: Option<Gc>) -> Result<bool, Value> {
        let (t, h, trap) = self.trap(g, "setPrototypeOf")?;
        if trap.is_undefined() {
            return self.set_prototype_of(t, p);
        }
        let pv = p.map_or(Value::Null, Value::Obj);
        let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t), pv])?.truthy();
        if !r {
            return Ok(false);
        }
        if !self.is_extensible(t)? && self.get_prototype_of(t)? != p {
            return Err(self.type_error("'setPrototypeOf' on proxy: trap returned truish for setting a new prototype on the non-extensible proxy target"));
        }
        Ok(true)
    }

    fn proxy_is_extensible(&mut self, g: Gc) -> Result<bool, Value> {
        let (t, h, trap) = self.trap(g, "isExtensible")?;
        if trap.is_undefined() {
            return self.is_extensible(t);
        }
        let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t)])?.truthy();
        if r != self.is_extensible(t)? {
            return Err(self.type_error("'isExtensible' on proxy: trap result does not reflect extensibility of proxy target"));
        }
        Ok(r)
    }

    fn proxy_prevent_extensions(&mut self, g: Gc) -> Result<bool, Value> {
        let (t, h, trap) = self.trap(g, "preventExtensions")?;
        if trap.is_undefined() {
            return self.prevent_extensions(t);
        }
        let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t)])?.truthy();
        if r && self.is_extensible(t)? {
            return Err(self.type_error("'preventExtensions' on proxy: trap returned truish but the proxy target is extensible"));
        }
        Ok(r)
    }

    fn proxy_get_own(&mut self, g: Gc, k: Key) -> Result<Option<Prop>, Value> {
        let (t, h, trap) = self.trap(g, "getOwnPropertyDescriptor")?;
        if trap.is_undefined() {
            return self.get_own_property(t, k);
        }
        let kv = self.key_value(k);
        let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t), kv])?;
        let target_desc = self.get_own_property(t, k)?;
        if r.is_undefined() {
            if let Some(td) = target_desc {
                if !td.configurable() {
                    return Err(self.type_error("'getOwnPropertyDescriptor' on proxy: trap returned undefined for a non-configurable property"));
                }
                if !self.is_extensible(t)? {
                    return Err(self.type_error("'getOwnPropertyDescriptor' on proxy: trap returned undefined for a property of a non-extensible target"));
                }
            }
            return Ok(None);
        }
        if !r.is_object() {
            return Err(self.type_error("'getOwnPropertyDescriptor' on proxy: trap returned neither object nor undefined"));
        }
        let extensible = self.is_extensible(t)?;
        let mut d = self.to_descriptor(&r)?;
        // CompletePropertyDescriptor.
        if !d.is_accessor() {
            d.value.get_or_insert(Value::Undefined);
            d.writable.get_or_insert(false);
        } else {
            d.get.get_or_insert(Value::Undefined);
            d.set.get_or_insert(Value::Undefined);
        }
        d.enumerable.get_or_insert(false);
        d.configurable.get_or_insert(false);
        if validate(&d, target_desc.as_ref(), extensible).is_none() {
            return Err(self.type_error("'getOwnPropertyDescriptor' on proxy: trap returned descriptor incompatible with the target's property"));
        }
        if d.configurable == Some(false) {
            match &target_desc {
                None => return Err(self.type_error("'getOwnPropertyDescriptor' on proxy: trap reported non-configurability for a property that doesn't exist")),
                Some(td) if td.configurable() => return Err(self.type_error("'getOwnPropertyDescriptor' on proxy: trap reported non-configurability for a configurable property")),
                Some(td) => {
                    if d.writable == Some(false) && td.writable() && matches!(td.slot, Slot::Data(_)) {
                        return Err(self.type_error("'getOwnPropertyDescriptor' on proxy: trap reported non-writable for a writable property"));
                    }
                }
            }
        }
        Ok(Some(d.to_prop()))
    }

    fn proxy_define(&mut self, g: Gc, k: Key, desc: Desc) -> Result<bool, Value> {
        let (t, h, trap) = self.trap(g, "defineProperty")?;
        if trap.is_undefined() {
            return self.define_own_property(t, k, desc);
        }
        let kv = self.key_value(k);
        let dv = self.from_descriptor(&desc);
        let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t), kv, dv])?.truthy();
        if !r {
            return Ok(false);
        }
        let target_desc = self.get_own_property(t, k)?;
        let extensible = self.is_extensible(t)?;
        let setting_nc = desc.configurable == Some(false);
        match target_desc {
            None => {
                if !extensible {
                    return Err(self.type_error("'defineProperty' on proxy: trap returned truish for adding a property to a non-extensible target"));
                }
                if setting_nc {
                    return Err(self.type_error("'defineProperty' on proxy: trap returned truish for defining a non-configurable property that doesn't exist on the target"));
                }
            }
            Some(td) => {
                if validate(&desc, Some(&td), extensible).is_none() {
                    return Err(self.type_error("'defineProperty' on proxy: trap returned truish for adding a property incompatible with the target's"));
                }
                if setting_nc && td.configurable() {
                    return Err(self.type_error("'defineProperty' on proxy: trap returned truish for a non-configurable property that's configurable on the target"));
                }
                if matches!(td.slot, Slot::Data(_)) && !td.configurable() && td.writable() && desc.writable == Some(false) {
                    return Err(self.type_error("'defineProperty' on proxy: trap returned truish for a non-writable property that's writable on the target"));
                }
            }
        }
        Ok(true)
    }

    fn proxy_has(&mut self, g: Gc, k: Key) -> Result<bool, Value> {
        let (t, h, trap) = self.trap(g, "has")?;
        if trap.is_undefined() {
            return self.has_property(t, k);
        }
        let kv = self.key_value(k);
        let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t), kv])?.truthy();
        if !r && let Some(td) = self.get_own_property(t, k)? {
            if !td.configurable() {
                return Err(self.type_error("'has' on proxy: trap returned falsish for a non-configurable property"));
            }
            if !self.is_extensible(t)? {
                return Err(self.type_error("'has' on proxy: trap returned falsish for a property of a non-extensible target"));
            }
        }
        Ok(r)
    }

    fn proxy_get(&mut self, g: Gc, k: Key, receiver: &Value) -> Result<Value, Value> {
        let (t, h, trap) = self.trap(g, "get")?;
        if trap.is_undefined() {
            return self.get(t, k, receiver);
        }
        let kv = self.key_value(k);
        let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t), kv, receiver.clone()])?;
        if let Some(td) = self.get_own_property(t, k)?
            && !td.configurable()
        {
            match &td.slot {
                Slot::Data(v) if !td.writable() && !same_value(v, &r) => return Err(self.type_error("'get' on proxy: property is a read-only and non-configurable data property on the target but the trap did not return its actual value")),
                Slot::Accessor(Value::Undefined, _) if !r.is_undefined() => return Err(self.type_error("'get' on proxy: property is a non-configurable accessor without a getter on the target but the trap did not return undefined")),
                _ => {}
            }
        }
        Ok(r)
    }

    fn proxy_set(&mut self, g: Gc, k: Key, v: Value, receiver: &Value) -> Result<bool, Value> {
        let (t, h, trap) = self.trap(g, "set")?;
        if trap.is_undefined() {
            return self.set(t, k, v, receiver);
        }
        let kv = self.key_value(k);
        let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t), kv, v.clone(), receiver.clone()])?.truthy();
        if !r {
            return Ok(false);
        }
        if let Some(td) = self.get_own_property(t, k)?
            && !td.configurable()
        {
            match &td.slot {
                Slot::Data(tv) if !td.writable() && !same_value(tv, &v) => return Err(self.type_error("'set' on proxy: trap returned truish for a read-only non-configurable property with a different value")),
                Slot::Accessor(_, Value::Undefined) => return Err(self.type_error("'set' on proxy: trap returned truish for a non-configurable accessor without a setter")),
                _ => {}
            }
        }
        Ok(true)
    }

    fn proxy_delete(&mut self, g: Gc, k: Key) -> Result<bool, Value> {
        let (t, h, trap) = self.trap(g, "deleteProperty")?;
        if trap.is_undefined() {
            return self.delete_property(t, k);
        }
        let kv = self.key_value(k);
        let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t), kv])?.truthy();
        if !r {
            return Ok(false);
        }
        if let Some(td) = self.get_own_property(t, k)? {
            if !td.configurable() {
                return Err(self.type_error("'deleteProperty' on proxy: trap returned truish for a non-configurable property"));
            }
            if !self.is_extensible(t)? {
                return Err(self.type_error("'deleteProperty' on proxy: trap returned truish for a property of a non-extensible target"));
            }
        }
        Ok(true)
    }

    fn proxy_own_keys(&mut self, g: Gc) -> Result<Vec<Key>, Value> {
        let (t, h, trap) = self.trap(g, "ownKeys")?;
        if trap.is_undefined() {
            return self.own_keys(t);
        }
        let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t)])?;
        let Value::Obj(rg) = r else { return Err(self.type_error("CreateListFromArrayLike called on non-object")) };
        let len = self.length_of(rg)?;
        let mut keys = Vec::new();
        for i in 0..len {
            let v = self.get(rg, Key::Index(i as u32), &r)?;
            let k = match &v {
                Value::Str(s) => self.key_from_str(s),
                Value::Sym(s) => Key::Sym(*s),
                _ => return Err(self.type_error("proxy [[OwnPropertyKeys]] must return an array with only string and symbol elements")),
            };
            if keys.contains(&k) {
                return Err(self.type_error("'ownKeys' on proxy: trap returned duplicate entries"));
            }
            keys.push(k);
        }
        let extensible = self.is_extensible(t)?;
        let target_keys = self.own_keys(t)?;
        let mut nonconfig = Vec::new();
        let mut config = Vec::new();
        for k in target_keys {
            match self.get_own_property(t, k)? {
                Some(p) if !p.configurable() => nonconfig.push(k),
                _ => config.push(k),
            }
        }
        if extensible && nonconfig.is_empty() {
            return Ok(keys);
        }
        let mut unchecked = keys.clone();
        for k in nonconfig {
            match unchecked.iter().position(|x| *x == k) {
                Some(i) => {
                    unchecked.remove(i);
                }
                None => return Err(self.type_error("'ownKeys' on proxy: trap result did not include a non-configurable key")),
            }
        }
        if extensible {
            return Ok(keys);
        }
        for k in config {
            match unchecked.iter().position(|x| *x == k) {
                Some(i) => {
                    unchecked.remove(i);
                }
                None => return Err(self.type_error("'ownKeys' on proxy: trap result did not include all keys of a non-extensible target")),
            }
        }
        if !unchecked.is_empty() {
            return Err(self.type_error("'ownKeys' on proxy: trap returned extra keys but proxy target is non-extensible"));
        }
        Ok(keys)
    }

    pub fn proxy_call(&mut self, g: Gc, this: Value, args: &[Value]) -> Result<Value, Value> {
        let (t, h, trap) = self.trap(g, "apply")?;
        if trap.is_undefined() {
            return self.call(&Value::Obj(t), this, args);
        }
        let arr = self.new_array(args.to_vec());
        self.call(&trap, Value::Obj(h), &[Value::Obj(t), this, Value::Obj(arr)])
    }

    pub fn proxy_construct(&mut self, g: Gc, args: &[Value], nt: &Value) -> Result<Value, Value> {
        let (t, h, trap) = self.trap(g, "construct")?;
        if trap.is_undefined() {
            return self.construct(&Value::Obj(t), args, nt);
        }
        let arr = self.new_array(args.to_vec());
        let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t), Value::Obj(arr), nt.clone()])?;
        if !r.is_object() {
            return Err(self.type_error("proxy [[Construct]] must return an object"));
        }
        Ok(r)
    }

    /// LengthOfArrayLike.
    pub fn length_of(&mut self, g: Gc) -> Result<u64, Value> {
        if let Class::Array(_, len) = &self.heap.get(g).class {
            return Ok(*len as u64);
        }
        let v = self.get(g, Key::Str(n::LENGTH), &Value::Obj(g))?;
        self.to_length(&v)
    }

    /// The values of an array the engine made (spread arguments): holes undefined.
    pub fn array_values(&mut self, v: &Value) -> Vec<Value> {
        let Value::Obj(g) = v else { return Vec::new() };
        match &self.heap.get(*g).class {
            Class::Array(els, len) => {
                let mut out: Vec<Value> = els.iter().map(|x| if matches!(x, Value::Empty) { Value::Undefined } else { x.clone() }).collect();
                out.resize(*len as usize, Value::Undefined);
                out
            }
            _ => Vec::new(),
        }
    }

    /// Whether a value is an array (seeing through proxies).
    pub fn is_array(&mut self, v: &Value) -> Result<bool, Value> {
        let Value::Obj(g) = v else { return Ok(false) };
        match &self.heap.get(*g).class {
            Class::Array(..) => Ok(true),
            Class::Proxy(Some((t, _)), _) => {
                let t = *t;
                self.is_array(&Value::Obj(t))
            }
            Class::Proxy(None, _) => Err(self.type_error("Cannot perform 'IsArray' on a proxy that has been revoked")),
            _ => Ok(false),
        }
    }

    /// The well-known symbol `s` as a key.
    pub fn sym_key(&self, s: Sym) -> Key {
        Key::Sym(self.sym(s))
    }
}

/// ValidateAndApplyPropertyDescriptor: the property as it should become (None: the change isn't allowed).
pub fn validate(desc: &Desc, current: Option<&Prop>, extensible: bool) -> Option<Prop> {
    let Some(cur) = current else {
        if !extensible {
            return None;
        }
        return Some(desc.to_prop());
    };
    if desc.is_empty() {
        return Some(cur.clone());
    }
    let cur_accessor = matches!(cur.slot, Slot::Accessor(..));
    if !cur.configurable() {
        if desc.configurable == Some(true) {
            return None;
        }
        if let Some(e) = desc.enumerable
            && e != cur.enumerable()
        {
            return None;
        }
        let generic = !desc.is_accessor() && !desc.is_data();
        if !generic && desc.is_accessor() != cur_accessor {
            return None;
        }
        match &cur.slot {
            Slot::Accessor(g, s) => {
                if let Some(dg) = &desc.get
                    && !same_value(dg, g)
                {
                    return None;
                }
                if let Some(ds) = &desc.set
                    && !same_value(ds, s)
                {
                    return None;
                }
            }
            Slot::Data(v) => {
                if !cur.writable() {
                    if desc.writable == Some(true) {
                        return None;
                    }
                    if let Some(dv) = &desc.value
                        && !same_value(dv, v)
                    {
                        return None;
                    }
                }
            }
        }
    }
    let enumerable = desc.enumerable.unwrap_or(cur.enumerable());
    let configurable = desc.configurable.unwrap_or(cur.configurable());
    let mut flags = 0;
    if enumerable {
        flags |= ENUMERABLE;
    }
    if configurable {
        flags |= CONFIGURABLE;
    }
    let slot = if desc.is_accessor() {
        let (g, s) = match &cur.slot {
            Slot::Accessor(g, s) => (g.clone(), s.clone()),
            Slot::Data(_) => (Value::Undefined, Value::Undefined),
        };
        Slot::Accessor(desc.get.clone().unwrap_or(g), desc.set.clone().unwrap_or(s))
    } else if desc.is_data() || !cur_accessor {
        let (v, w) = match &cur.slot {
            Slot::Data(v) => (v.clone(), cur.writable()),
            Slot::Accessor(..) => (Value::Undefined, false),
        };
        if desc.writable.unwrap_or(w) {
            flags |= WRITABLE;
        }
        Slot::Data(desc.value.clone().unwrap_or(v))
    } else {
        cur.slot.clone()
    };
    Some(Prop { slot, flags })
}
