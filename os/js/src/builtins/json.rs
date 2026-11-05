//! JSON.parse (with a reviver) and JSON.stringify (with a replacer function or property list, and indentation).

use super::{arg, tag};
use crate::names as n;
use crate::object::*;
use crate::string::JsStr;
use crate::value::{Gc, Value, number_to_string};
use crate::vm::{I, Vm};
use alloc::vec::Vec;

type R = Result<Value, Value>;

pub fn init(vm: &mut Vm) {
    let j = vm.new_object();
    let g = vm.global;
    vm.put(g, "JSON", Value::Obj(j), HIDDEN);
    vm.method(j, "parse", 2, json_parse);
    vm.method(j, "stringify", 3, json_stringify);
    tag(vm, j, "JSON");
}

// ------------------------------------------------------------------------------------------------------- parse

struct Parser<'a> {
    u: &'a [u16],
    at: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while let Some(&c) = self.u.get(self.at) {
            if c == 0x20 || c == 0x09 || c == 0x0A || c == 0x0D {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u16> {
        self.u.get(self.at).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c as u16) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn word(&mut self, w: &str) -> bool {
        let wu: Vec<u16> = w.encode_utf16().collect();
        if self.u[self.at..].starts_with(&wu) {
            self.at += wu.len();
            true
        } else {
            false
        }
    }

    fn string(&mut self) -> Option<JsStr> {
        if !self.eat(b'"') {
            return None;
        }
        let mut out: Vec<u16> = Vec::new();
        loop {
            let c = self.peek()?;
            self.at += 1;
            match c {
                0x22 => return Some(JsStr::from_units(out)),
                0x5C => {
                    let e = self.peek()?;
                    self.at += 1;
                    out.push(match e {
                        0x22 => 0x22,
                        0x5C => 0x5C,
                        0x2F => 0x2F,
                        0x62 => 0x08,
                        0x66 => 0x0C,
                        0x6E => 0x0A,
                        0x72 => 0x0D,
                        0x74 => 0x09,
                        0x75 => {
                            let mut v = 0u16;
                            for _ in 0..4 {
                                let h = self.peek()?;
                                self.at += 1;
                                let d = char::from_u32(h as u32)?.to_digit(16)?;
                                v = v * 16 + d as u16;
                            }
                            v
                        }
                        _ => return None,
                    });
                }
                c if c < 0x20 => return None,
                c => out.push(c),
            }
        }
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.at;
        self.eat(b'-');
        let digit = |c: Option<u16>| c.is_some_and(|c| (0x30..=0x39).contains(&c));
        if self.eat(b'0') {
        } else if digit(self.peek()) {
            while digit(self.peek()) {
                self.at += 1;
            }
        } else {
            return None;
        }
        if self.eat(b'.') {
            if !digit(self.peek()) {
                return None;
            }
            while digit(self.peek()) {
                self.at += 1;
            }
        }
        if self.peek() == Some(b'e' as u16) || self.peek() == Some(b'E' as u16) {
            self.at += 1;
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            if !digit(self.peek()) {
                return None;
            }
            while digit(self.peek()) {
                self.at += 1;
            }
        }
        let s: alloc::string::String = self.u[start..self.at].iter().map(|c| *c as u8 as char).collect();
        s.parse().ok()
    }

    fn value(&mut self, vm: &mut Vm, depth: usize) -> Result<Value, ()> {
        if depth > 5000 {
            return Err(());
        }
        self.ws();
        match self.peek().ok_or(())? {
            0x7B => {
                self.at += 1;
                let o = vm.new_object();
                self.ws();
                if self.eat(b'}') {
                    return Ok(Value::Obj(o));
                }
                loop {
                    self.ws();
                    let k = self.string().ok_or(())?;
                    self.ws();
                    if !self.eat(b':') {
                        return Err(());
                    }
                    let v = self.value(vm, depth + 1)?;
                    let key = vm.key_from_str(&k);
                    // (A later duplicate wins: defined over the earlier.)
                    let _ = vm.create_data_property(o, key, v);
                    self.ws();
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b'}') {
                        return Ok(Value::Obj(o));
                    }
                    return Err(());
                }
            }
            0x5B => {
                self.at += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat(b']') {
                    return Ok(Value::Obj(vm.new_array(items)));
                }
                loop {
                    items.push(self.value(vm, depth + 1)?);
                    self.ws();
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b']') {
                        return Ok(Value::Obj(vm.new_array(items)));
                    }
                    return Err(());
                }
            }
            0x22 => Ok(Value::Str(self.string().ok_or(())?)),
            0x74 if self.word("true") => Ok(Value::Bool(true)),
            0x66 if self.word("false") => Ok(Value::Bool(false)),
            0x6E if self.word("null") => Ok(Value::Null),
            _ => Ok(Value::Num(self.number().ok_or(())?)),
        }
    }
}

/// Parses JSON text into values (None: it isn't JSON).
pub fn parse(vm: &mut Vm, text: &JsStr) -> Option<Value> {
    let mut p = Parser { u: text.units(), at: 0 };
    let v = p.value(vm, 0).ok()?;
    p.ws();
    (p.at == p.u.len()).then_some(v)
}

fn json_parse(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let text = vm.to_string(&arg(args, 0))?;
    let Some(v) = parse(vm, &text) else {
        let s = text.to_rust();
        let shown: alloc::string::String = s.chars().take(40).collect();
        return Err(vm.syntax_error(&alloc::format!("Unexpected token in JSON: \"{shown}\" is not valid JSON")));
    };
    let reviver = arg(args, 1);
    if vm.is_callable(&reviver) {
        let root = vm.new_object();
        vm.create_data_property(root, Key::Str(n::EMPTY), v)?;
        return internalize(vm, root, Key::Str(n::EMPTY), &reviver);
    }
    Ok(v)
}

fn internalize(vm: &mut Vm, holder: Gc, key: Key, reviver: &Value) -> R {
    let val = vm.get(holder, key, &Value::Obj(holder))?;
    if let Value::Obj(o) = val {
        if vm.is_array(&val)? {
            let len = vm.length_of(o)?;
            for i in 0..len {
                let k = crate::builtins::array::index_key(vm, i);
                let nv = internalize(vm, o, k, reviver)?;
                if nv.is_undefined() {
                    vm.delete_property(o, k)?;
                } else {
                    vm.create_data_property(o, k, nv)?;
                }
            }
        } else {
            for k in vm.own_keys(o)? {
                if matches!(k, Key::Sym(_)) {
                    continue;
                }
                if !vm.get_own_property(o, k)?.is_some_and(|p| p.enumerable()) {
                    continue;
                }
                let nv = internalize(vm, o, k, reviver)?;
                if nv.is_undefined() {
                    vm.delete_property(o, k)?;
                } else {
                    vm.create_data_property(o, k, nv)?;
                }
            }
        }
    }
    let kv = vm.key_value(key);
    vm.call(reviver, Value::Obj(holder), &[kv, val])
}

// --------------------------------------------------------------------------------------------------- stringify

struct Ser {
    replacer: Value,
    list: Option<Vec<Key>>,
    gap: Vec<u16>,
    indent: Vec<u16>,
    stack: Vec<Gc>,
    out: Vec<u16>,
}

/// QuoteJSONString.
pub fn quote(out: &mut Vec<u16>, s: &[u16]) {
    out.push(b'"' as u16);
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        match c {
            0x08 => out.extend("\\b".encode_utf16()),
            0x09 => out.extend("\\t".encode_utf16()),
            0x0A => out.extend("\\n".encode_utf16()),
            0x0C => out.extend("\\f".encode_utf16()),
            0x0D => out.extend("\\r".encode_utf16()),
            0x22 => out.extend("\\\"".encode_utf16()),
            0x5C => out.extend("\\\\".encode_utf16()),
            c if c < 0x20 => out.extend(alloc::format!("\\u{c:04x}").encode_utf16()),
            c if (0xD800..0xDC00).contains(&c) && s.get(i + 1).is_some_and(|d| (0xDC00..0xE000).contains(d)) => {
                out.push(c);
                out.push(s[i + 1]);
                i += 1;
            }
            c if (0xD800..0xE000).contains(&c) => out.extend(alloc::format!("\\u{c:04x}").encode_utf16()),
            c => out.push(c),
        }
        i += 1;
    }
    out.push(b'"' as u16);
}

impl Ser {
    /// SerializeJSONProperty: false if the value is undefined (nothing written).
    fn property(&mut self, vm: &mut Vm, holder: Gc, key: Key) -> Result<bool, Value> {
        let mut v = vm.get(holder, key, &Value::Obj(holder))?;
        if matches!(v, Value::Obj(_) | Value::Big(_)) {
            let tj = vm.get_value(&v, Key::Str(n::TO_JSON))?;
            if vm.is_callable(&tj) {
                let kv = vm.key_value(key);
                v = vm.call(&tj, v, &[kv])?;
            }
        }
        if !self.replacer.is_undefined() {
            let kv = vm.key_value(key);
            v = vm.call(&self.replacer.clone(), Value::Obj(holder), &[kv, v])?;
        }
        if let Value::Obj(o) = v {
            v = match &vm.heap.get(o).class {
                Class::Number(_) => Value::Num(vm.to_number(&v)?),
                Class::String(_) => Value::Str(vm.to_string(&v)?),
                Class::Boolean(b) => Value::Bool(*b),
                Class::BigIntObj(b) => Value::Big(b.clone()),
                _ => v,
            };
        }
        match &v {
            Value::Null => self.out.extend("null".encode_utf16()),
            Value::Bool(b) => self.out.extend(if *b { "true" } else { "false" }.encode_utf16()),
            Value::Str(s) => quote(&mut self.out, s.units()),
            Value::Num(x) => {
                if x.is_finite() {
                    self.out.extend(number_to_string(*x).encode_utf16());
                } else {
                    self.out.extend("null".encode_utf16());
                }
            }
            Value::Big(_) => return Err(vm.type_error("Do not know how to serialize a BigInt")),
            Value::Obj(o) if !vm.is_callable(&v) => {
                let o = *o;
                if vm.is_array(&v)? {
                    self.array(vm, o)?;
                } else {
                    self.object(vm, o)?;
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn enter(&mut self, vm: &mut Vm, o: Gc) -> Result<Vec<u16>, Value> {
        if self.stack.contains(&o) {
            return Err(vm.type_error("Converting circular structure to JSON"));
        }
        if self.stack.len() > 5000 {
            return Err(vm.range_error("Maximum call stack size exceeded"));
        }
        self.stack.push(o);
        let saved = self.indent.clone();
        self.indent.extend_from_slice(&self.gap);
        Ok(saved)
    }

    fn line(&mut self) {
        if !self.gap.is_empty() {
            self.out.push(b'\n' as u16);
            self.out.extend_from_slice(&self.indent);
        }
    }

    fn object(&mut self, vm: &mut Vm, o: Gc) -> Result<(), Value> {
        let saved = self.enter(vm, o)?;
        let keys = match &self.list {
            Some(l) => l.clone(),
            None => {
                let mut ks = Vec::new();
                for k in vm.own_keys(o)? {
                    if !matches!(k, Key::Sym(_)) && vm.get_own_property(o, k)?.is_some_and(|p| p.enumerable()) {
                        ks.push(k);
                    }
                }
                ks
            }
        };
        self.out.push(b'{' as u16);
        let mut any = false;
        for k in keys {
            let mark = self.out.len();
            if any {
                self.out.push(b',' as u16);
            }
            self.line();
            let ks = match vm.key_value(k) {
                Value::Str(s) => s,
                _ => JsStr::empty(),
            };
            quote(&mut self.out, ks.units());
            self.out.push(b':' as u16);
            if !self.gap.is_empty() {
                self.out.push(b' ' as u16);
            }
            if self.property(vm, o, k)? {
                any = true;
            } else {
                self.out.truncate(mark);
            }
        }
        self.indent = saved;
        if any {
            self.line();
        }
        self.out.push(b'}' as u16);
        self.stack.pop();
        Ok(())
    }

    fn array(&mut self, vm: &mut Vm, o: Gc) -> Result<(), Value> {
        let saved = self.enter(vm, o)?;
        let len = vm.length_of(o)?;
        self.out.push(b'[' as u16);
        for i in 0..len {
            if i > 0 {
                self.out.push(b',' as u16);
            }
            self.line();
            let k = crate::builtins::array::index_key(vm, i);
            if !self.property(vm, o, k)? {
                self.out.extend("null".encode_utf16());
            }
        }
        self.indent = saved;
        if len > 0 {
            self.line();
        }
        self.out.push(b']' as u16);
        self.stack.pop();
        Ok(())
    }
}

/// JSON.stringify(value) with no replacer or indentation (None: undefined).
pub fn stringify(vm: &mut Vm, v: Value) -> Result<Option<JsStr>, Value> {
    let mut s = Ser { replacer: Value::Undefined, list: None, gap: Vec::new(), indent: Vec::new(), stack: Vec::new(), out: Vec::new() };
    let wrapper = vm.new_object();
    vm.create_data_property(wrapper, Key::Str(n::EMPTY), v)?;
    if !s.property(vm, wrapper, Key::Str(n::EMPTY))? {
        return Ok(None);
    }
    Ok(Some(JsStr::from_units(s.out)))
}

fn json_stringify(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let mut s = Ser { replacer: Value::Undefined, list: None, gap: Vec::new(), indent: Vec::new(), stack: Vec::new(), out: Vec::new() };
    let r = arg(args, 1);
    if vm.is_callable(&r) {
        s.replacer = r;
    } else if vm.is_array(&r)? {
        let Value::Obj(rg) = r else { unreachable!() };
        let len = vm.length_of(rg)?;
        let mut list: Vec<Key> = Vec::new();
        for i in 0..len {
            let v = vm.get(rg, Key::Index(i as u32), &r)?;
            let item = match &v {
                Value::Str(x) => Some(x.clone()),
                Value::Num(x) => Some(JsStr::from_str(&number_to_string(*x))),
                Value::Obj(o) if matches!(vm.heap.get(*o).class, Class::String(_) | Class::Number(_)) => Some(vm.to_string(&v)?),
                _ => None,
            };
            if let Some(item) = item {
                let k = vm.key_from_str(&item);
                if !list.contains(&k) {
                    list.push(k);
                }
            }
        }
        s.list = Some(list);
    }
    let mut space = arg(args, 2);
    if let Value::Obj(o) = space {
        space = match vm.heap.get(o).class {
            Class::Number(_) => Value::Num(vm.to_number(&space)?),
            Class::String(_) => Value::Str(vm.to_string(&space)?),
            _ => space,
        };
    }
    match &space {
        Value::Num(x) => {
            let k = crate::value::to_integer(*x).clamp(0.0, 10.0) as usize;
            s.gap = alloc::vec![b' ' as u16; k];
        }
        Value::Str(st) => s.gap = st.units().iter().take(10).copied().collect(),
        _ => {}
    }
    let wrapper = vm.new_object();
    vm.create_data_property(wrapper, Key::Str(n::EMPTY), arg(args, 0))?;
    if !s.property(vm, wrapper, Key::Str(n::EMPTY))? {
        return Ok(Value::Undefined);
    }
    let _ = I::ObjectProto;
    Ok(Value::Str(JsStr::from_units(s.out)))
}
