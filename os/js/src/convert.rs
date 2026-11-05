//! The language's conversions that may run code (ToPrimitive calls valueOf and toString, or @@toPrimitive) and its
//! operators: + - * / % ** and the bitwise ones (on numbers and BigInts), the comparisons, == and instanceof.

use crate::bigint::BigInt;
use crate::code::Op;
use crate::math;
use crate::names as n;
use crate::object::*;
use crate::string::JsStr;
use crate::value::{Gc, Value, number_to_string, strict_equals, string_to_number, to_int32, to_uint32};
use crate::vm::{I, Sym, Vm};
use alloc::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hint {
    Default,
    Number,
    String,
}

impl Vm {
    /// ToPrimitive.
    pub fn to_primitive(&mut self, v: &Value, hint: Hint) -> Result<Value, Value> {
        let Value::Obj(g) = v else { return Ok(v.clone()) };
        let g = *g;
        let tp = self.sym(Sym::ToPrimitive);
        let exotic = self.get_method(v, Key::Sym(tp))?;
        if !exotic.is_undefined() {
            let h = match hint {
                Hint::Default => "default",
                Hint::Number => "number",
                Hint::String => "string",
            };
            let r = self.call(&exotic, v.clone(), &[Value::str(h)])?;
            if r.is_object() {
                return Err(self.type_error("Cannot convert object to primitive value"));
            }
            return Ok(r);
        }
        let order = if hint == Hint::String { [n::TO_STRING, n::VALUE_OF] } else { [n::VALUE_OF, n::TO_STRING] };
        for name in order {
            let m = self.get(g, Key::Str(name), v)?;
            if self.is_callable(&m) {
                let r = self.call(&m, v.clone(), &[])?;
                if !r.is_object() {
                    return Ok(r);
                }
            }
        }
        Err(self.type_error("Cannot convert object to primitive value"))
    }

    pub fn to_number(&mut self, v: &Value) -> Result<f64, Value> {
        match v {
            Value::Num(x) => Ok(*x),
            Value::Undefined | Value::Empty => Ok(f64::NAN),
            Value::Null => Ok(0.0),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Str(s) => Ok(string_to_number(s)),
            Value::Sym(_) => Err(self.type_error("Cannot convert a Symbol value to a number")),
            Value::Big(_) => Err(self.type_error("Cannot convert a BigInt value to a number")),
            Value::Obj(_) => {
                let p = self.to_primitive(v, Hint::Number)?;
                self.to_number(&p)
            }
        }
    }

    /// ToNumeric: a number or a BigInt.
    pub fn to_numeric(&mut self, v: &Value) -> Result<Value, Value> {
        match v {
            Value::Num(_) | Value::Big(_) => Ok(v.clone()),
            Value::Obj(_) => {
                let p = self.to_primitive(v, Hint::Number)?;
                self.to_numeric(&p)
            }
            _ => Ok(Value::Num(self.to_number(v)?)),
        }
    }

    pub fn to_bigint(&mut self, v: &Value) -> Result<BigInt, Value> {
        let p = self.to_primitive(v, Hint::Number)?;
        match &p {
            Value::Big(b) => Ok((**b).clone()),
            Value::Bool(b) => Ok(BigInt::from_i64(*b as i64)),
            Value::Str(s) => match BigInt::parse_js(&s.to_rust()) {
                Some(b) => Ok(b),
                None => Err(self.syntax_error(&alloc::format!("Cannot convert {} to a BigInt", s))),
            },
            Value::Num(_) => Err(self.type_error(&alloc::format!("Cannot convert {} to a BigInt", self.describe(&p)))),
            Value::Sym(_) => Err(self.type_error("Cannot convert a Symbol value to a BigInt")),
            _ => Err(self.type_error(&alloc::format!("Cannot convert {} to a BigInt", self.describe(&p)))),
        }
    }

    pub fn to_string(&mut self, v: &Value) -> Result<JsStr, Value> {
        Ok(match v {
            Value::Str(s) => s.clone(),
            Value::Num(x) => JsStr::from_str(&number_to_string(*x)),
            Value::Undefined | Value::Empty => JsStr::from_str("undefined"),
            Value::Null => JsStr::from_str("null"),
            Value::Bool(b) => JsStr::from_str(if *b { "true" } else { "false" }),
            Value::Big(b) => JsStr::from_str(&b.to_string_radix(10)),
            Value::Sym(_) => return Err(self.type_error("Cannot convert a Symbol value to a string")),
            Value::Obj(_) => {
                let p = self.to_primitive(v, Hint::String)?;
                return self.to_string(&p);
            }
        })
    }

    pub fn to_object(&mut self, v: &Value) -> Result<Gc, Value> {
        let (proto, class) = match v {
            Value::Obj(g) => return Ok(*g),
            Value::Undefined | Value::Null | Value::Empty => return Err(self.type_error(&alloc::format!("Cannot convert {} to object", self.describe(v)))),
            Value::Bool(b) => (I::BooleanProto, Class::Boolean(*b)),
            Value::Num(x) => (I::NumberProto, Class::Number(*x)),
            Value::Str(s) => (I::StringProto, Class::String(s.clone())),
            Value::Sym(s) => (I::SymbolProto, Class::SymbolObj(*s)),
            Value::Big(b) => (I::BigIntProto, Class::BigIntObj(b.clone())),
        };
        let p = self.proto(proto);
        Ok(self.alloc(Some(p), class))
    }

    /// ToPropertyKey.
    pub fn to_key(&mut self, v: &Value) -> Result<Key, Value> {
        match v {
            Value::Str(s) => Ok(self.key_from_str(s)),
            Value::Sym(s) => Ok(Key::Sym(*s)),
            Value::Num(x) => {
                let x = *x;
                if x >= 0.0 && x < 4294967295.0 && (x as u32) as f64 == x && !(x == 0.0 && x.is_sign_negative()) {
                    return Ok(Key::Index(x as u32));
                }
                let s = JsStr::from_str(&number_to_string(x));
                Ok(self.key_from_str(&s))
            }
            Value::Obj(_) => {
                let p = self.to_primitive(v, Hint::String)?;
                self.to_key(&p)
            }
            other => {
                let s = self.to_string(other)?;
                Ok(self.key_from_str(&s))
            }
        }
    }

    pub fn to_int32(&mut self, v: &Value) -> Result<i32, Value> {
        Ok(to_int32(self.to_number(v)?))
    }

    pub fn to_uint32(&mut self, v: &Value) -> Result<u32, Value> {
        Ok(to_uint32(self.to_number(v)?))
    }

    /// ToIntegerOrInfinity.
    pub fn to_integer(&mut self, v: &Value) -> Result<f64, Value> {
        let x = self.to_number(v)?;
        Ok(crate::value::to_integer(x))
    }

    /// ToLength.
    pub fn to_length(&mut self, v: &Value) -> Result<u64, Value> {
        let x = self.to_integer(v)?;
        Ok(if x <= 0.0 { 0 } else { x.min(9007199254740991.0) as u64 })
    }

    /// ToIndex.
    pub fn to_index(&mut self, v: &Value) -> Result<u64, Value> {
        if v.is_undefined() {
            return Ok(0);
        }
        let x = self.to_integer(v)?;
        if !(0.0..=9007199254740991.0).contains(&x) {
            return Err(self.range_error("Invalid index"));
        }
        Ok(x as u64)
    }

    /// A relative index (from the end if negative), clamped to 0..=len.
    pub fn relative_index(&mut self, v: &Value, len: u64, default: u64) -> Result<u64, Value> {
        if v.is_undefined() {
            return Ok(default);
        }
        let x = self.to_integer(v)?;
        Ok(if x < 0.0 { (len as f64 + x).max(0.0) as u64 } else { x.min(len as f64) as u64 })
    }

    pub fn type_of(&self, v: &Value) -> &'static str {
        match v {
            Value::Obj(_) if self.is_callable(v) => "function",
            _ => v.type_name(),
        }
    }

    /// IsLooselyEqual (==).
    pub fn loose_equals(&mut self, a: &Value, b: &Value) -> Result<bool, Value> {
        Ok(match (a, b) {
            (Value::Undefined | Value::Null, Value::Undefined | Value::Null) => true,
            (Value::Undefined | Value::Null, _) | (_, Value::Undefined | Value::Null) => false,
            (Value::Num(_), Value::Num(_)) | (Value::Str(_), Value::Str(_)) | (Value::Bool(_), Value::Bool(_)) | (Value::Sym(_), Value::Sym(_)) | (Value::Obj(_), Value::Obj(_)) | (Value::Big(_), Value::Big(_)) => strict_equals(a, b),
            (Value::Num(x), Value::Str(s)) | (Value::Str(s), Value::Num(x)) => *x == string_to_number(s),
            (Value::Big(x), Value::Str(s)) | (Value::Str(s), Value::Big(x)) => match BigInt::parse_js(&s.to_rust()) {
                Some(y) => x.cmp(&y) == core::cmp::Ordering::Equal,
                None => false,
            },
            (Value::Bool(x), _) => {
                let y = Value::Num(if *x { 1.0 } else { 0.0 });
                return self.loose_equals(&y, b);
            }
            (_, Value::Bool(y)) => {
                let x = Value::Num(if *y { 1.0 } else { 0.0 });
                return self.loose_equals(a, &x);
            }
            (Value::Obj(_), _) => {
                let p = self.to_primitive(a, Hint::Default)?;
                return self.loose_equals(&p, b);
            }
            (_, Value::Obj(_)) => {
                let p = self.to_primitive(b, Hint::Default)?;
                return self.loose_equals(a, &p);
            }
            (Value::Big(x), Value::Num(y)) | (Value::Num(y), Value::Big(x)) => x.cmp_f64(*y) == Some(core::cmp::Ordering::Equal),
            _ => false,
        })
    }

    /// IsLessThan (None: undefined, a NaN was compared).
    pub fn less_than(&mut self, a: &Value, b: &Value, left_first: bool) -> Result<Option<bool>, Value> {
        let (px, py) = if left_first {
            let x = self.to_primitive(a, Hint::Number)?;
            let y = self.to_primitive(b, Hint::Number)?;
            (x, y)
        } else {
            let y = self.to_primitive(b, Hint::Number)?;
            let x = self.to_primitive(a, Hint::Number)?;
            (x, y)
        };
        if let (Value::Str(x), Value::Str(y)) = (&px, &py) {
            return Ok(Some(x.units() < y.units()));
        }
        if let (Value::Big(x), Value::Str(y)) = (&px, &py) {
            return Ok(BigInt::parse_js(&y.to_rust()).map(|y| x.cmp(&y) == core::cmp::Ordering::Less));
        }
        if let (Value::Str(x), Value::Big(y)) = (&px, &py) {
            return Ok(BigInt::parse_js(&x.to_rust()).map(|x| x.cmp(y) == core::cmp::Ordering::Less));
        }
        let nx = self.to_numeric(&px)?;
        let ny = self.to_numeric(&py)?;
        Ok(match (&nx, &ny) {
            (Value::Num(x), Value::Num(y)) => {
                if x.is_nan() || y.is_nan() {
                    None
                } else {
                    Some(x < y)
                }
            }
            (Value::Big(x), Value::Big(y)) => Some(x.cmp(y) == core::cmp::Ordering::Less),
            (Value::Big(x), Value::Num(y)) => {
                if y.is_nan() {
                    None
                } else {
                    x.cmp_f64(*y).map(|o| o == core::cmp::Ordering::Less)
                }
            }
            (Value::Num(x), Value::Big(y)) => {
                if x.is_nan() {
                    None
                } else {
                    y.cmp_f64(*x).map(|o| o == core::cmp::Ordering::Greater)
                }
            }
            _ => None,
        })
    }

    /// a + b.
    pub fn add(&mut self, a: &Value, b: &Value) -> Result<Value, Value> {
        let pa = self.to_primitive(a, Hint::Default)?;
        let pb = self.to_primitive(b, Hint::Default)?;
        if matches!(pa, Value::Str(_)) || matches!(pb, Value::Str(_)) {
            let sa = self.to_string(&pa)?;
            let sb = self.to_string(&pb)?;
            return Ok(Value::Str(sa.concat(&sb)));
        }
        self.arith(Op::Add, &pa, &pb)
    }

    /// The numeric operators (a and b may be anything: converted first).
    pub fn arith(&mut self, op: Op, a: &Value, b: &Value) -> Result<Value, Value> {
        let x = self.to_numeric(a)?;
        let y = self.to_numeric(b)?;
        match (&x, &y) {
            (Value::Num(x), Value::Num(y)) => {
                let (x, y) = (*x, *y);
                Ok(Value::Num(match op {
                    Op::Add => x + y,
                    Op::Sub => x - y,
                    Op::Mul => x * y,
                    Op::Div => x / y,
                    Op::Rem => math::fmod(x, y),
                    Op::Exp => math::pow(x, y),
                    Op::Shl => to_int32(x).wrapping_shl(to_uint32(y) & 31) as f64,
                    Op::Shr => (to_int32(x) >> (to_uint32(y) & 31)) as f64,
                    Op::UShr => (to_uint32(x) >> (to_uint32(y) & 31)) as f64,
                    Op::BitAnd => (to_int32(x) & to_int32(y)) as f64,
                    Op::BitOr => (to_int32(x) | to_int32(y)) as f64,
                    Op::BitXor => (to_int32(x) ^ to_int32(y)) as f64,
                    _ => f64::NAN,
                }))
            }
            (Value::Big(x), Value::Big(y)) => {
                let r = match op {
                    Op::Add => x.add(y),
                    Op::Sub => x.sub(y),
                    Op::Mul => x.mul(y),
                    Op::Div => match x.div(y) {
                        Some(r) => r,
                        None => return Err(self.range_error("Division by zero")),
                    },
                    Op::Rem => match x.rem(y) {
                        Some(r) => r,
                        None => return Err(self.range_error("Division by zero")),
                    },
                    Op::Exp => match x.pow(y) {
                        Some(r) => r,
                        None => return Err(self.range_error("Exponent must be non-negative")),
                    },
                    Op::Shl | Op::Shr => {
                        let Some(s) = y.to_i64() else { return Err(self.range_error("Maximum BigInt size exceeded")) };
                        let s = if op == Op::Shl { s } else { s.saturating_neg() };
                        if s > (1 << 30) {
                            return Err(self.range_error("Maximum BigInt size exceeded"));
                        }
                        x.shl(s)
                    }
                    Op::UShr => return Err(self.type_error("BigInts have no unsigned right shift, use >> instead")),
                    Op::BitAnd => x.and(y),
                    Op::BitOr => x.or(y),
                    Op::BitXor => x.xor(y),
                    _ => return Err(self.type_error("Invalid BigInt operation")),
                };
                Ok(Value::Big(Rc::new(r)))
            }
            _ => Err(self.type_error("Cannot mix BigInt and other types, use explicit conversions")),
        }
    }

    /// `v instanceof target`.
    pub fn instance_of(&mut self, v: &Value, target: &Value) -> Result<bool, Value> {
        if !target.is_object() {
            return Err(self.type_error("Right-hand side of 'instanceof' is not an object"));
        }
        let hi = self.sym(Sym::HasInstance);
        let h = self.get_method(target, Key::Sym(hi))?;
        if !h.is_undefined() {
            return Ok(self.call(&h, target.clone(), &[v.clone()])?.truthy());
        }
        if !self.is_callable(target) {
            return Err(self.type_error("Right-hand side of 'instanceof' is not callable"));
        }
        self.ordinary_has_instance(target, v)
    }

    pub fn ordinary_has_instance(&mut self, c: &Value, v: &Value) -> Result<bool, Value> {
        if !self.is_callable(c) {
            return Ok(false);
        }
        let Value::Obj(cg) = c else { return Ok(false) };
        if let Class::Bound(b) = &self.heap.get(*cg).class {
            let t = Value::Obj(b.target);
            return self.instance_of(v, &t);
        }
        let Value::Obj(o) = v else { return Ok(false) };
        let p = self.get(*cg, Key::Str(n::PROTOTYPE), c)?;
        let Value::Obj(p) = p else { return Err(self.type_error("Function has non-object prototype in instanceof check")) };
        let mut cur = self.get_prototype_of(*o)?;
        while let Some(x) = cur {
            if x == p {
                return Ok(true);
            }
            cur = self.get_prototype_of(x)?;
        }
        Ok(false)
    }

    /// SpeciesConstructor: the constructor an object's species says to make new ones with.
    pub fn species_constructor(&mut self, o: Gc, default: I) -> Result<Value, Value> {
        let c = self.get(o, Key::Str(n::CONSTRUCTOR), &Value::Obj(o))?;
        if c.is_undefined() {
            return Ok(Value::Obj(self.proto(default)));
        }
        let Value::Obj(cg) = c else { return Err(self.type_error("object.constructor is not an object")) };
        let sp = self.sym(Sym::Species);
        let s = self.get(cg, Key::Sym(sp), &c)?;
        if s.is_nullish() {
            return Ok(Value::Obj(self.proto(default)));
        }
        if self.is_constructor(&s) {
            return Ok(s);
        }
        Err(self.type_error("object.constructor[Symbol.species] is not a constructor"))
    }
}
