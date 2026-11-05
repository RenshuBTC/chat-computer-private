//! JavaScript's values, and the conversions between them that need nothing but the value (those that may call a
//! function, ToPrimitive's valueOf and toString, are the VM's): numbers written as text and read from it as the
//! language says, the integer conversions of the bitwise operators, equality without conversions (strict, SameValue,
//! SameValueZero).

use crate::bigint::BigInt;
use crate::string::JsStr;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

/// An object (or symbol) in the heap, by its number there.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Gc(pub u32);

#[derive(Clone, Debug)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(JsStr),
    /// A symbol: in the heap (its description), so it can be a property's key and be collected.
    Sym(Gc),
    Obj(Gc),
    Big(Rc<BigInt>),
    /// Never a value scripts see: an array's hole, a `let` not yet initialized, "no value" in the VM.
    Empty,
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(JsStr::from_str(s))
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Undefined)
    }

    pub fn is_nullish(&self) -> bool {
        matches!(self, Value::Undefined | Value::Null)
    }

    pub fn is_object(&self) -> bool {
        matches!(self, Value::Obj(_))
    }

    pub fn as_obj(&self) -> Option<Gc> {
        match self {
            Value::Obj(g) => Some(*g),
            _ => None,
        }
    }

    /// ToBoolean.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Undefined | Value::Null | Value::Empty => false,
            Value::Bool(b) => *b,
            Value::Num(n) => !(*n == 0.0 || n.is_nan()),
            Value::Str(s) => !s.is_empty(),
            Value::Sym(_) | Value::Obj(_) => true,
            Value::Big(b) => !b.is_zero(),
        }
    }

    /// typeof's answer, but for objects (callable or not: the VM tells).
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Undefined | Value::Empty => "undefined",
            Value::Null => "object",
            Value::Bool(_) => "boolean",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Sym(_) => "symbol",
            Value::Obj(_) => "object",
            Value::Big(_) => "bigint",
        }
    }
}

/// IsStrictlyEqual (===).
pub fn strict_equals(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Num(x), Value::Num(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Sym(x), Value::Sym(y)) | (Value::Obj(x), Value::Obj(y)) => x == y,
        (Value::Big(x), Value::Big(y)) => x == y,
        _ => false,
    }
}

/// SameValue (Object.is): NaN is NaN, and +0 isn't -0.
pub fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => (x.is_nan() && y.is_nan()) || (x == y && x.is_sign_negative() == y.is_sign_negative()),
        _ => strict_equals(a, b),
    }
}

/// SameValueZero (includes, Map's keys): NaN is NaN, +0 is -0.
pub fn same_value_zero(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => (x.is_nan() && y.is_nan()) || x == y,
        _ => strict_equals(a, b),
    }
}

// ------------------------------------------------------------------------------------------------- numbers as text

/// Number::toString(x) (radix 10): the shortest digits that read back as the same number, in the language's layout
/// (plain from 1e-7 to 1e21, with an exponent outside).
pub fn number_to_string(x: f64) -> String {
    if x.is_nan() {
        return String::from("NaN");
    }
    if x == 0.0 {
        return String::from("0");
    }
    if x.is_infinite() {
        return String::from(if x > 0.0 { "Infinity" } else { "-Infinity" });
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let (digits, e) = shortest_digits(x.abs());
    // x = 0.d1d2...dk × 10^(n) where n = e + 1 in the specification's terms: digits × 10^(n - k).
    let k = digits.len() as i32;
    let n = e + 1;
    let mut out = String::from(sign);
    if k <= n && n <= 21 {
        out.push_str(&digits);
        for _ in 0..(n - k) {
            out.push('0');
        }
    } else if 0 < n && n <= 21 {
        out.push_str(&digits[..n as usize]);
        out.push('.');
        out.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        for _ in 0..(-n) {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if n - 1 >= 0 { '+' } else { '-' });
        out.push_str(&format!("{}", (n - 1).abs()));
    }
    out
}

/// The shortest decimal digits of a positive finite number that read back as it, and the exponent of the first
/// (x = d.ddd × 10^e).
pub fn shortest_digits(x: f64) -> (String, i32) {
    // (Rust's formatting gives the shortest round-trip digits in its exponent form: "1.2345e-7".)
    let s = format!("{x:e}");
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = String::from(digits.trim_end_matches('0'));
    let digits = if digits.is_empty() { String::from("0") } else { digits };
    (digits, exp.parse().unwrap_or(0))
}

/// Number.prototype.toString(radix) for a radix other than 10.
pub fn number_to_radix(x: f64, radix: u32) -> String {
    if x.is_nan() {
        return String::from("NaN");
    }
    if x.is_infinite() {
        return String::from(if x > 0.0 { "Infinity" } else { "-Infinity" });
    }
    if x == 0.0 {
        return String::from("0");
    }
    let neg = x < 0.0;
    let x = x.abs();
    let mut int = trunc(x);
    let mut frac = x - int;
    let digit = |d: u32| core::char::from_digit(d, radix).unwrap_or('0');
    let mut int_digits = Vec::new();
    if int == 0.0 {
        int_digits.push('0');
    }
    while int >= 1.0 {
        let d = rem_euclid_f(int, radix as f64) as u32;
        int_digits.push(digit(d));
        int = trunc(int / radix as f64);
    }
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    out.extend(int_digits.iter().rev());
    if frac > 0.0 {
        out.push('.');
        // (As many digits as the number's 52 bits of fraction can tell apart.)
        let bits_per_digit = (31 - radix.leading_zeros()).max(1);
        let most = 52 / bits_per_digit + 2;
        let mut n = 0;
        while frac > 0.0 && n < most {
            frac *= radix as f64;
            let d = trunc(frac);
            frac -= d;
            out.push(digit(d as u32));
            n += 1;
        }
        while out.ends_with('0') {
            out.pop();
        }
    }
    out
}

pub fn trunc(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() || x.abs() >= 4503599627370496.0 {
        return x;
    }
    (x as i64) as f64
}

pub fn floor(x: f64) -> f64 {
    let t = trunc(x);
    if t > x { t - 1.0 } else { t }
}

fn rem_euclid_f(a: f64, b: f64) -> f64 {
    let r = a - trunc(a / b) * b;
    if r < 0.0 { r + b } else { r }
}

/// StringToNumber: white space around, decimal (with Infinity), or 0x / 0o / 0b; an empty string is 0; anything else
/// is NaN.
pub fn string_to_number(s: &JsStr) -> f64 {
    let units = crate::string::trim(s.units());
    if units.is_empty() {
        return 0.0;
    }
    let Ok(text) = String::from_utf16(units) else { return f64::NAN };
    let t = text.as_str();
    let radix = |rest: &str, r: u32| -> f64 {
        if rest.is_empty() {
            return f64::NAN;
        }
        let mut v = 0f64;
        for c in rest.chars() {
            match c.to_digit(r) {
                Some(d) => v = v * r as f64 + d as f64,
                None => return f64::NAN,
            }
        }
        v
    };
    if let Some(r) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return radix(r, 16);
    }
    if let Some(r) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        return radix(r, 8);
    }
    if let Some(r) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        return radix(r, 2);
    }
    let (sign, body) = match t.as_bytes()[0] {
        b'+' => (1.0, &t[1..]),
        b'-' => (-1.0, &t[1..]),
        _ => (1.0, t),
    };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    // (Rust reads "inf", "nan" and "1_000", which JavaScript doesn't; JavaScript reads ".5" and "5.", as Rust does.)
    if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-')) {
        return f64::NAN;
    }
    match body.parse::<f64>() {
        Ok(v) => sign * v,
        Err(_) => f64::NAN,
    }
}

/// ToInt32 of a number.
pub fn to_int32(x: f64) -> i32 {
    to_uint32(x) as i32
}

/// ToUint32 of a number.
pub fn to_uint32(x: f64) -> u32 {
    if !x.is_finite() || x == 0.0 {
        return 0;
    }
    if x.abs() < 2147483648.0 * 2.0 {
        return (trunc(x) as i64) as u32;
    }
    let t = trunc(x);
    let m = rem_euclid_f(t, 4294967296.0);
    m as u32
}

/// ToIntegerOrInfinity of a number.
pub fn to_integer(x: f64) -> f64 {
    if x.is_nan() {
        return 0.0;
    }
    if x.is_infinite() {
        return x;
    }
    let t = trunc(x);
    if t == 0.0 { 0.0 } else { t }
}

/// A string that's an array index ("0" to "4294967294", in canonical form), as its number.
pub fn array_index(u: &[u16]) -> Option<u32> {
    if u.is_empty() || u.len() > 10 {
        return None;
    }
    if u.len() > 1 && u[0] == b'0' as u16 {
        return None;
    }
    let mut v: u64 = 0;
    for &c in u {
        if !(b'0' as u16..=b'9' as u16).contains(&c) {
            return None;
        }
        v = v * 10 + (c - b'0' as u16) as u64;
    }
    (v < 4294967295).then_some(v as u32)
}

/// A number's text, as a property key's (ToString of a number).
pub fn number_key(x: f64) -> JsStr {
    JsStr::from_str(&number_to_string(x))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_written_as_javascript_writes_them() {
        for (x, s) in [
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123456789012345680000.0, "123456789012345680000"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (0.000001, "0.000001"),
            (0.1 + 0.2, "0.30000000000000004"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (f64::NAN, "NaN"),
            (-0.0, "0"),
            (f64::NEG_INFINITY, "-Infinity"),
            (100.0, "100"),
            (2.5e-5, "0.000025"),
        ] {
            assert_eq!(number_to_string(x), s, "{x:e}");
        }
        assert_eq!(number_to_radix(255.0, 16), "ff");
        assert_eq!(number_to_radix(-10.0, 2), "-1010");
        assert_eq!(number_to_radix(0.5, 2), "0.1");
    }

    #[test]
    fn text_read_as_numbers() {
        let n = |s: &str| string_to_number(&JsStr::from_str(s));
        assert_eq!(n("  42  "), 42.0);
        assert_eq!(n(""), 0.0);
        assert_eq!(n("0x1F"), 31.0);
        assert_eq!(n("-Infinity"), f64::NEG_INFINITY);
        assert_eq!(n(".5"), 0.5);
        assert_eq!(n("5."), 5.0);
        assert!(n("1_000").is_nan() && n("inf").is_nan() && n("12px").is_nan() && n("-0x10").is_nan());
        assert_eq!(n("1e3"), 1000.0);
        assert_eq!(to_int32(4294967296.0 + 5.0), 5);
        assert_eq!(to_int32(2147483648.0), -2147483648);
        assert_eq!(to_uint32(-1.0), 4294967295);
        assert_eq!(array_index(&"123".encode_utf16().collect::<Vec<_>>()), Some(123));
        assert_eq!(array_index(&"0123".encode_utf16().collect::<Vec<_>>()), None);
        assert_eq!(array_index(&"4294967295".encode_utf16().collect::<Vec<_>>()), None);
    }
}
