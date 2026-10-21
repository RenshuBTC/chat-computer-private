//! Converting values from one type to another, as ACPI does it when an operator wants a type its operand isn't
//! (implicitly), or when asked (ToInteger, ToBuffer, ToHexString...): as ACPICA, the reference implementation, does.

use crate::{Aml, Error, Object, Reference, Value};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

/// An EISA ID as its seven characters: three letters (5 bits each) and four hex digits, as _HID packs them
/// ("PNP0C0A").
pub fn eisa_id(id: u32) -> String {
    let id = id.swap_bytes();
    let letter = |shift: u32| (((id >> shift) & 0x1F) as u8 + 0x40) as char;
    format!("{}{}{}{:04X}", letter(26), letter(21), letter(16), id & 0xFFFF)
}

/// Hex digits of a string as a number, from its start to its first other character (implicit conversion).
fn hex_prefix(s: &str) -> u64 {
    let mut n: u64 = 0;
    for c in s.trim_start().chars() {
        let Some(d) = c.to_digit(16) else { break };
        n = n.wrapping_mul(16).wrapping_add(d as u64);
    }
    n
}

impl Aml {
    /// All bits set: Ones, and True.
    pub(crate) fn ones(&self) -> u64 {
        if self.narrow { 0xFFFF_FFFF } else { u64::MAX }
    }

    /// An integer cut to the width integers have.
    pub(crate) fn trim(&self, n: u64) -> u64 {
        if self.narrow { n & 0xFFFF_FFFF } else { n }
    }

    /// Bytes an integer takes in a buffer.
    pub(crate) fn int_bytes(&self) -> usize {
        if self.narrow { 4 } else { 8 }
    }

    /// What a reference refers to, read.
    pub(crate) fn deref(&self, r: &Reference) -> Result<Value, Error> {
        match r {
            Reference::Node(n) => Ok(match &self.nodes[*n].object {
                Object::Data(v) => v.clone(),
                _ => Value::Object(*n),
            }),
            Reference::Cell(c) => Ok(c.borrow().clone()),
            Reference::Element(p, i) => p.borrow().get(*i).cloned().ok_or(Error::Index),
            Reference::Byte(b, i) => b.borrow().get(*i).map(|&x| Value::Integer(x as u64)).ok_or(Error::Index),
            Reference::Unresolved(name, _) => Err(Error::NotFound(name.clone())),
        }
    }

    /// A value as an integer: a buffer's first bytes (little-endian), a string's hex digits.
    pub(crate) fn to_integer(&self, v: &Value) -> Result<u64, Error> {
        match v {
            Value::Integer(n) => Ok(self.trim(*n)),
            Value::Buffer(b) => {
                let b = b.borrow();
                Ok(b.iter().take(self.int_bytes()).rev().fold(0u64, |n, &x| n << 8 | x as u64))
            }
            Value::String(s) => Ok(self.trim(hex_prefix(s))),
            Value::Reference(r) => self.to_integer(&self.deref(r)?),
            Value::Uninitialized => Ok(0),
            _ => Err(Error::Type("an integer")),
        }
    }

    /// A value as a buffer: an integer's bytes (little-endian), a string's characters and its NUL.
    pub(crate) fn to_buffer(&self, v: &Value) -> Result<Vec<u8>, Error> {
        match v {
            Value::Integer(n) => Ok(n.to_le_bytes()[..self.int_bytes()].to_vec()),
            Value::Buffer(b) => Ok(b.borrow().clone()),
            Value::String(s) => {
                let mut b: Vec<u8> = s.bytes().collect();
                if !b.is_empty() {
                    b.push(0);
                }
                Ok(b)
            }
            Value::Reference(r) => self.to_buffer(&self.deref(r)?),
            Value::Uninitialized => Ok(Vec::new()),
            _ => Err(Error::Type("a buffer")),
        }
    }

    /// A value as a string, implicitly: an integer's hex digits (all of them), a buffer's bytes as "0x01 0xAB".
    pub(crate) fn to_text(&self, v: &Value) -> Result<String, Error> {
        match v {
            Value::Integer(n) => Ok(if self.narrow { format!("{:08X}", n & 0xFFFF_FFFF) } else { format!("{n:016X}") }),
            Value::String(s) => Ok(s.clone()),
            Value::Buffer(b) => Ok(b.borrow().iter().map(|x| format!("0x{x:02X}")).collect::<Vec<String>>().join(" ")),
            Value::Reference(r) => self.to_text(&self.deref(r)?),
            Value::Uninitialized => Ok(String::new()),
            _ => Err(Error::Type("a string")),
        }
    }

    /// ToHexString: an integer's hex digits; a buffer's bytes as "0x01,0xAB".
    pub(crate) fn hex_string(&self, v: &Value) -> Result<String, Error> {
        match v {
            Value::Buffer(b) => Ok(b.borrow().iter().map(|x| format!("0x{x:02X}")).collect::<Vec<String>>().join(",")),
            Value::Reference(r) => self.hex_string(&self.deref(r)?),
            other => self.to_text(other),
        }
    }

    /// ToDecimalString: an integer in decimal; a buffer's bytes as "1,171,3".
    pub(crate) fn decimal_string(&self, v: &Value) -> Result<String, Error> {
        match v {
            Value::Integer(n) => Ok(format!("{}", self.trim(*n))),
            Value::Buffer(b) => Ok(b.borrow().iter().map(|x| format!("{x}")).collect::<Vec<String>>().join(",")),
            Value::String(s) => Ok(s.clone()),
            Value::Reference(r) => self.decimal_string(&self.deref(r)?),
            _ => Err(Error::Type("a string")),
        }
    }

    /// ToInteger: a string in decimal, or hex after "0x".
    pub(crate) fn explicit_integer(&self, v: &Value) -> Result<u64, Error> {
        match v {
            Value::String(s) => {
                let t = s.trim();
                let n = if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                    hex_prefix(hex)
                } else {
                    t.chars().take_while(|c| c.is_ascii_digit()).fold(0u64, |n, c| n.wrapping_mul(10).wrapping_add(c as u64 - b'0' as u64))
                };
                Ok(self.trim(n))
            }
            Value::Reference(r) => self.explicit_integer(&self.deref(r)?),
            other => self.to_integer(other),
        }
    }

    /// A value converted to the type of `like` (a store into a named Integer, String or Buffer keeps its type).
    pub(crate) fn convert_like(&self, v: &Value, like: &Value) -> Result<Value, Error> {
        match like {
            Value::Integer(_) => Ok(Value::Integer(self.to_integer(v)?)),
            Value::String(_) => Ok(Value::String(match v {
                Value::Buffer(b) => b.borrow().iter().take_while(|&&x| x != 0).map(|&x| x as char).collect(),
                other => self.to_text(other)?,
            })),
            Value::Buffer(old) => {
                // A store into a buffer keeps its length: the new bytes, cut or padded with zeros.
                let mut b = self.to_buffer(v)?;
                b.resize(old.borrow().len(), 0);
                Ok(Value::buffer(b))
            }
            _ => Ok(copy(v)),
        }
    }

    /// ObjectType's number for a value.
    pub(crate) fn type_of(&self, v: &Value) -> u64 {
        match v {
            Value::Uninitialized => 0,
            Value::Integer(_) => 1,
            Value::String(_) => 2,
            Value::Buffer(_) => 3,
            Value::Package(_) => 4,
            Value::Reference(r) => match self.deref(r) {
                Ok(v) => self.type_of(&v),
                Err(_) => 0,
            },
            Value::Object(n) => match &self.nodes[*n].object {
                Object::Data(v) => self.type_of(v),
                Object::Field(_) => 5,
                Object::Device => 6,
                Object::Event(_) => 7,
                Object::Method(_) | Object::Osi => 8,
                Object::Mutex => 9,
                Object::Region(_) => 10,
                Object::PowerResource => 11,
                Object::Processor => 12,
                Object::ThermalZone => 13,
                Object::BufferField { .. } => 14,
                _ => 0,
            },
        }
    }
}

/// A copy of a value: a buffer's or a package's own, not shared (as Store copies).
pub(crate) fn copy(v: &Value) -> Value {
    match v {
        Value::Buffer(b) => Value::buffer(b.borrow().clone()),
        Value::Package(p) => Value::Package(Rc::new(RefCell::new(p.borrow().iter().map(copy).collect()))),
        other => other.clone(),
    }
}
