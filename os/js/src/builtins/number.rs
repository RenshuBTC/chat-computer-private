//! Number (with toFixed, toExponential and toPrecision exact, in integers), BigInt, Math, and the global
//! parseInt, parseFloat, isNaN and isFinite.

use super::{arg, constructor, js_str, tag};
use crate::bigint::BigInt;
use crate::math;
use crate::object::*;
use crate::string::{JsStr, is_space, is_line_terminator};
use crate::value::{Gc, Value, number_to_radix, number_to_string, shortest_digits};
use crate::vm::{I, Vm};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

type R = Result<Value, Value>;

pub fn init(vm: &mut Vm) {
    let np = vm.proto(I::NumberProto);
    let c = constructor(vm, "Number", 1, number_call, Some(number_construct), Some(np));
    for (name, v) in [
        ("EPSILON", f64::EPSILON),
        ("MAX_SAFE_INTEGER", 9007199254740991.0),
        ("MIN_SAFE_INTEGER", -9007199254740991.0),
        ("MAX_VALUE", f64::MAX),
        ("MIN_VALUE", 5e-324),
        ("NaN", f64::NAN),
        ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
        ("POSITIVE_INFINITY", f64::INFINITY),
    ] {
        vm.put(c, name, Value::Num(v), 0);
    }
    vm.method(c, "isFinite", 1, number_is_finite);
    vm.method(c, "isInteger", 1, number_is_integer);
    vm.method(c, "isNaN", 1, number_is_nan);
    vm.method(c, "isSafeInteger", 1, number_is_safe_integer);
    let g = vm.global;
    let pf = vm.method(g, "parseFloat", 1, parse_float);
    let pi = vm.method(g, "parseInt", 2, parse_int);
    vm.put(c, "parseFloat", Value::Obj(pf), HIDDEN);
    vm.put(c, "parseInt", Value::Obj(pi), HIDDEN);
    vm.method(g, "isNaN", 1, global_is_nan);
    vm.method(g, "isFinite", 1, global_is_finite);
    vm.method(np, "toString", 1, number_to_string_m);
    vm.method(np, "toLocaleString", 0, number_to_locale_string);
    vm.method(np, "valueOf", 0, number_value_of);
    vm.method(np, "toFixed", 1, number_to_fixed);
    vm.method(np, "toExponential", 1, number_to_exponential);
    vm.method(np, "toPrecision", 1, number_to_precision);

    let bp = vm.proto(I::BigIntProto);
    let b = constructor(vm, "BigInt", 1, bigint_call, Some(bigint_construct), Some(bp));
    vm.method(b, "asIntN", 2, bigint_as_int_n);
    vm.method(b, "asUintN", 2, bigint_as_uint_n);
    vm.method(bp, "toString", 0, bigint_to_string);
    vm.method(bp, "toLocaleString", 0, bigint_to_string);
    vm.method(bp, "valueOf", 0, bigint_value_of);
    tag(vm, bp, "BigInt");

    math_object(vm);
}

fn number_call(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(to_number_value(vm, args)?))
}

fn to_number_value(vm: &mut Vm, args: &[Value]) -> Result<f64, Value> {
    if args.is_empty() {
        return Ok(0.0);
    }
    match vm.to_numeric(&args[0])? {
        Value::Big(b) => Ok(b.to_f64()),
        Value::Num(x) => Ok(x),
        _ => Ok(f64::NAN),
    }
}

fn number_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let x = to_number_value(vm, args)?;
    let p = vm.proto_from_constructor(nt, I::NumberProto)?;
    Ok(Value::Obj(vm.alloc(Some(p), Class::Number(x))))
}

fn this_number(vm: &mut Vm, this: &Value) -> Result<f64, Value> {
    match this {
        Value::Num(x) => Ok(*x),
        Value::Obj(g) => match vm.heap.get(*g).class {
            Class::Number(x) => Ok(x),
            _ => Err(vm.type_error("Number.prototype method called on an incompatible receiver")),
        },
        _ => Err(vm.type_error("Number.prototype method called on an incompatible receiver")),
    }
}

fn number_is_finite(_vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Num(x) if x.is_finite())))
}

fn is_integral(x: f64) -> bool {
    x.is_finite() && math::trunc(x) == x
}

fn number_is_integer(_vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Num(x) if is_integral(x))))
}

fn number_is_nan(_vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Num(x) if x.is_nan())))
}

fn number_is_safe_integer(_vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Num(x) if is_integral(x) && x.abs() <= 9007199254740991.0)))
}

fn global_is_nan(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(vm.to_number(&arg(args, 0))?.is_nan()))
}

fn global_is_finite(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(vm.to_number(&arg(args, 0))?.is_finite()))
}

fn trim_start(u: &[u16]) -> &[u16] {
    let i = u.iter().position(|c| !is_space(*c as u32) && !is_line_terminator(*c as u32)).unwrap_or(u.len());
    &u[i..]
}

fn parse_float(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = vm.to_string(&arg(args, 0))?;
    let u = trim_start(s.units());
    let text: String = u.iter().take_while(|c| **c < 128).map(|c| *c as u8 as char).collect();
    let (sign, body) = match text.as_bytes().first() {
        Some(b'-') => (-1.0, &text[1..]),
        Some(b'+') => (1.0, &text[1..]),
        _ => (1.0, &text[..]),
    };
    if body.starts_with("Infinity") {
        return Ok(Value::Num(sign * f64::INFINITY));
    }
    // The longest prefix that's a decimal literal.
    let b = body.as_bytes();
    let mut i = 0;
    let mut digits = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return Ok(Value::Num(f64::NAN));
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > start {
            i = j;
        }
    }
    let lit = &body[..i];
    let lit = if lit.ends_with('.') { &lit[..lit.len() - 1] } else { lit };
    let lit = if lit.starts_with('.') { format!("0{lit}") } else { String::from(lit) };
    Ok(Value::Num(sign * lit.parse::<f64>().unwrap_or(f64::NAN)))
}

fn parse_int(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = vm.to_string(&arg(args, 0))?;
    let mut radix = vm.to_int32(&arg(args, 1))?;
    let mut u = trim_start(s.units());
    let mut sign = 1.0;
    if let Some(&c) = u.first()
        && (c == b'-' as u16 || c == b'+' as u16)
    {
        if c == b'-' as u16 {
            sign = -1.0;
        }
        u = &u[1..];
    }
    let mut strip_prefix = true;
    if radix != 0 {
        if !(2..=36).contains(&radix) {
            return Ok(Value::Num(f64::NAN));
        }
        if radix != 16 {
            strip_prefix = false;
        }
    } else {
        radix = 10;
    }
    if strip_prefix && u.len() >= 2 && u[0] == b'0' as u16 && (u[1] == b'x' as u16 || u[1] == b'X' as u16) {
        u = &u[2..];
        radix = 16;
    }
    let digits: Vec<u32> = u.iter().map_while(|c| char::from_u32(*c as u32).and_then(|ch| ch.to_digit(radix as u32))).collect();
    if digits.is_empty() {
        return Ok(Value::Num(f64::NAN));
    }
    // Exact for radix 10 (the decimal text read as a number); others by the digits' values.
    let v = if radix == 10 {
        let t: String = digits.iter().map(|d| char::from_digit(*d, 10).unwrap_or('0')).collect();
        t.parse::<f64>().unwrap_or(f64::NAN)
    } else if radix.count_ones() == 1 {
        let big = BigInt::parse_radix(&digits.iter().map(|d| char::from_digit(*d, radix as u32).unwrap_or('0')).collect::<String>(), radix as u32);
        big.map_or(f64::NAN, |b| b.to_f64())
    } else {
        digits.iter().fold(0.0, |a, d| a * radix as f64 + *d as f64)
    };
    Ok(Value::Num(sign * v))
}

fn number_to_string_m(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let x = this_number(vm, this)?;
    let r = arg(args, 0);
    let radix = if r.is_undefined() { 10.0 } else { vm.to_integer(&r)? };
    if !(2.0..=36.0).contains(&radix) {
        return Err(vm.range_error("toString() radix must be between 2 and 36"));
    }
    if radix == 10.0 {
        return Ok(js_str(&number_to_string(x)));
    }
    Ok(js_str(&number_to_radix(x, radix as u32)))
}

/// A number written with digit groups (en-US's way), for toLocaleString.
pub fn grouped(x: f64, max_fraction: usize) -> String {
    if !x.is_finite() {
        return if x.is_nan() { String::from("NaN") } else if x > 0.0 { String::from("∞") } else { String::from("-∞") };
    }
    let s = to_fixed(x.abs(), max_fraction);
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let frac = frac.trim_end_matches('0');
    let mut out = String::new();
    if x < 0.0 && (int != "0" || !frac.is_empty()) {
        out.push('-');
    }
    let len = int.len();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    out
}

fn number_to_locale_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let x = this_number(vm, this)?;
    Ok(js_str(&grouped(x, 3)))
}

fn number_value_of(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(this_number(vm, this)?))
}

/// round(x × 10^k) (x finite and not negative; halves up), exactly.
fn scaled_round(x: f64, k: i32) -> BigInt {
    if x == 0.0 {
        return BigInt::zero();
    }
    let b = x.to_bits();
    let e = ((b >> 52) & 0x7FF) as i32;
    let frac = b & ((1u64 << 52) - 1);
    let (m, q) = if e == 0 { (frac, -1074) } else { (frac | (1u64 << 52), e - 1075) };
    let mut num = BigInt::from_u64(m);
    let mut den = BigInt::from_u64(1);
    if q >= 0 {
        num = num.shl(q as i64);
    } else {
        den = den.shl(-q as i64);
    }
    let p = BigInt::from_u64(10).pow(&BigInt::from_i64(k.abs() as i64)).unwrap_or_else(|| BigInt::from_u64(1));
    if k >= 0 {
        num = num.mul(&p);
    } else {
        den = den.mul(&p);
    }
    let two = BigInt::from_u64(2);
    num.mul(&two).add(&den).div(&den.mul(&two)).unwrap_or_else(BigInt::zero)
}

/// x (not negative) with `d` digits after the point.
pub fn to_fixed(x: f64, d: usize) -> String {
    let n = scaled_round(x, d as i32).to_string_radix(10);
    if d == 0 {
        return n;
    }
    let n = if n.len() <= d { format!("{}{}", "0".repeat(d + 1 - n.len()), n) } else { n };
    let (a, b) = n.split_at(n.len() - d);
    format!("{a}.{b}")
}

fn fraction_digits(vm: &mut Vm, v: &Value, most: f64) -> Result<f64, Value> {
    let f = vm.to_integer(v)?;
    if !(0.0..=most).contains(&f) {
        return Err(vm.range_error("toFixed() digits argument must be between 0 and 100"));
    }
    Ok(f)
}

fn number_to_fixed(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let x = this_number(vm, this)?;
    let f = fraction_digits(vm, &arg(args, 0), 100.0)?;
    if !x.is_finite() {
        return Ok(js_str(&number_to_string(x)));
    }
    if x.abs() >= 1e21 {
        return Ok(js_str(&number_to_string(x)));
    }
    let s = to_fixed(x.abs(), f as usize);
    Ok(js_str(&if x < 0.0 { format!("-{s}") } else { s }))
}

/// The digits of x (> 0) to `p` significant figures, and the exponent of the first.
fn significant(x: f64, p: usize) -> (String, i32) {
    let (_, mut e) = shortest_digits(x);
    loop {
        let n = scaled_round(x, p as i32 - 1 - e).to_string_radix(10);
        if n.len() > p {
            e += 1;
            continue;
        }
        if n.len() < p {
            e -= 1;
            continue;
        }
        return (n, e);
    }
}

fn exponent(e: i32) -> String {
    format!("e{}{}", if e >= 0 { "+" } else { "-" }, e.abs())
}

fn number_to_exponential(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let x = this_number(vm, this)?;
    let fv = arg(args, 0);
    let f = vm.to_integer(&fv)?;
    if !x.is_finite() {
        return Ok(js_str(&number_to_string(x)));
    }
    if !(0.0..=100.0).contains(&f) {
        return Err(vm.range_error("toExponential() argument must be between 0 and 100"));
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let a = x.abs();
    let (digits, e) = if a == 0.0 {
        let d = if fv.is_undefined() { 1 } else { f as usize + 1 };
        ("0".repeat(d), 0)
    } else if fv.is_undefined() {
        shortest_digits(a)
    } else {
        significant(a, f as usize + 1)
    };
    let (first, rest) = digits.split_at(1);
    let body = if rest.is_empty() { String::from(first) } else { format!("{first}.{rest}") };
    Ok(js_str(&format!("{sign}{body}{}", exponent(e))))
}

fn number_to_precision(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let x = this_number(vm, this)?;
    let pv = arg(args, 0);
    if pv.is_undefined() {
        return Ok(js_str(&number_to_string(x)));
    }
    let p = vm.to_integer(&pv)?;
    if !x.is_finite() {
        return Ok(js_str(&number_to_string(x)));
    }
    if !(1.0..=100.0).contains(&p) {
        return Err(vm.range_error("toPrecision() argument must be between 1 and 100"));
    }
    let p = p as usize;
    let sign = if x < 0.0 { "-" } else { "" };
    let a = x.abs();
    let (digits, e) = if a == 0.0 { ("0".repeat(p), 0) } else { significant(a, p) };
    let s = if e < -6 || e >= p as i32 {
        let (first, rest) = digits.split_at(1);
        let body = if rest.is_empty() { String::from(first) } else { format!("{first}.{rest}") };
        format!("{body}{}", exponent(e))
    } else if e == p as i32 - 1 {
        digits
    } else if e >= 0 {
        let (a, b) = digits.split_at(e as usize + 1);
        format!("{a}.{b}")
    } else {
        format!("0.{}{}", "0".repeat((-(e + 1)) as usize), digits)
    };
    Ok(js_str(&format!("{sign}{s}")))
}

// ------------------------------------------------------------------------------------------------------ BigInt

fn bigint_call(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.to_primitive(&arg(args, 0), crate::convert::Hint::Number)?;
    if let Value::Num(x) = p {
        if !is_integral(x) {
            return Err(vm.range_error(&format!("The number {} cannot be converted to a BigInt because it is not an integer", number_to_string(x))));
        }
        return Ok(Value::Big(Rc::new(BigInt::from_f64(x).unwrap_or_else(BigInt::zero))));
    }
    Ok(Value::Big(Rc::new(vm.to_bigint(&p)?)))
}

fn bigint_construct(vm: &mut Vm, _nt: &Value, _args: &[Value], _f: Gc) -> R {
    Err(vm.type_error("BigInt is not a constructor"))
}

fn this_bigint(vm: &mut Vm, this: &Value) -> Result<Rc<BigInt>, Value> {
    match this {
        Value::Big(b) => Ok(b.clone()),
        Value::Obj(g) => match &vm.heap.get(*g).class {
            Class::BigIntObj(b) => Ok(b.clone()),
            _ => Err(vm.type_error("BigInt.prototype method called on an incompatible receiver")),
        },
        _ => Err(vm.type_error("BigInt.prototype method called on an incompatible receiver")),
    }
}

fn bigint_to_string(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let b = this_bigint(vm, this)?;
    let r = arg(args, 0);
    let radix = if r.is_undefined() { 10.0 } else { vm.to_integer(&r)? };
    if !(2.0..=36.0).contains(&radix) {
        return Err(vm.range_error("toString() radix must be between 2 and 36"));
    }
    Ok(js_str(&b.to_string_radix(radix as u32)))
}

fn bigint_value_of(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Big(this_bigint(vm, this)?))
}

fn bits_arg(vm: &mut Vm, args: &[Value]) -> Result<(u32, BigInt), Value> {
    let bits = vm.to_index(&arg(args, 0))?;
    if bits > u32::MAX as u64 {
        return Err(vm.range_error("Invalid number of bits"));
    }
    let b = vm.to_bigint(&arg(args, 1))?;
    Ok((bits as u32, b))
}

fn bigint_as_int_n(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (bits, b) = bits_arg(vm, args)?;
    Ok(Value::Big(Rc::new(b.as_int_n(bits))))
}

fn bigint_as_uint_n(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (bits, b) = bits_arg(vm, args)?;
    Ok(Value::Big(Rc::new(b.as_uint_n(bits))))
}

// -------------------------------------------------------------------------------------------------------- Math

fn math_object(vm: &mut Vm) {
    let m = vm.new_object();
    let g = vm.global;
    vm.put(g, "Math", Value::Obj(m), HIDDEN);
    tag(vm, m, "Math");
    for (name, v) in [
        ("E", core::f64::consts::E),
        ("LN10", core::f64::consts::LN_10),
        ("LN2", core::f64::consts::LN_2),
        ("LOG10E", core::f64::consts::LOG10_E),
        ("LOG2E", core::f64::consts::LOG2_E),
        ("PI", core::f64::consts::PI),
        ("SQRT1_2", core::f64::consts::FRAC_1_SQRT_2),
        ("SQRT2", core::f64::consts::SQRT_2),
    ] {
        vm.put(m, name, Value::Num(v), 0);
    }
    for (i, (name, _)) in UNARY.iter().enumerate() {
        let f = vm.native_with(name, 1, math_unary, None, Value::Num(i as f64));
        vm.put(m, name, Value::Obj(f), HIDDEN);
    }
    vm.method(m, "atan2", 2, math_atan2);
    vm.method(m, "clz32", 1, math_clz32);
    vm.method(m, "hypot", 2, math_hypot);
    vm.method(m, "imul", 2, math_imul);
    vm.method(m, "max", 2, math_max);
    vm.method(m, "min", 2, math_min);
    vm.method(m, "pow", 2, math_pow);
    vm.method(m, "random", 0, math_random);
}

fn fround(x: f64) -> f64 {
    x as f32 as f64
}

fn sign(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 { x } else { x.signum() }
}

fn trunc(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() {
        return x;
    }
    let t = math::trunc(x);
    if t == 0.0 && x.is_sign_negative() { -0.0 } else { t }
}

fn floor(x: f64) -> f64 {
    if x == 0.0 { x } else { math::floor(x) }
}

/// The one-argument functions (a native's data is its place here).
const UNARY: [(&str, fn(f64) -> f64); 27] = [
    ("abs", f64::abs),
    ("acos", math::acos),
    ("acosh", math::acosh),
    ("asin", math::asin),
    ("asinh", math::asinh),
    ("atan", math::atan),
    ("atanh", math::atanh),
    ("cbrt", math::cbrt),
    ("ceil", math::ceil),
    ("cos", math::cos),
    ("cosh", math::cosh),
    ("exp", math::exp),
    ("expm1", math::expm1),
    ("floor", floor),
    ("fround", fround),
    ("log", math::log),
    ("log1p", math::log1p),
    ("log10", math::log10),
    ("log2", math::log2),
    ("round", math::round),
    ("sign", sign),
    ("sin", math::sin),
    ("sinh", math::sinh),
    ("sqrt", math::sqrt),
    ("tan", math::tan),
    ("tanh", math::tanh),
    ("trunc", trunc),
];

fn math_unary(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let i = match vm.data(f) {
        Value::Num(i) => i as usize,
        _ => 0,
    };
    let x = vm.to_number(&arg(args, 0))?;
    Ok(Value::Num((UNARY[i.min(UNARY.len() - 1)].1)(x)))
}

fn math_atan2(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let y = vm.to_number(&arg(args, 0))?;
    let x = vm.to_number(&arg(args, 1))?;
    Ok(Value::Num(math::atan2(y, x)))
}

fn math_clz32(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let x = vm.to_uint32(&arg(args, 0))?;
    Ok(Value::Num(x.leading_zeros() as f64))
}

fn math_hypot(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let mut xs = Vec::new();
    for a in args {
        xs.push(vm.to_number(a)?);
    }
    Ok(Value::Num(math::hypot(&xs)))
}

fn math_imul(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let a = vm.to_uint32(&arg(args, 0))?;
    let b = vm.to_uint32(&arg(args, 1))?;
    Ok(Value::Num(a.wrapping_mul(b) as i32 as f64))
}

fn math_max(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let mut r = f64::NEG_INFINITY;
    let mut nan = false;
    for a in args {
        let x = vm.to_number(a)?;
        if x.is_nan() {
            nan = true;
        } else if x > r || (x == 0.0 && r == 0.0 && !x.is_sign_negative()) {
            r = x;
        }
    }
    Ok(Value::Num(if nan { f64::NAN } else { r }))
}

fn math_min(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let mut r = f64::INFINITY;
    let mut nan = false;
    for a in args {
        let x = vm.to_number(a)?;
        if x.is_nan() {
            nan = true;
        } else if x < r || (x == 0.0 && r == 0.0 && x.is_sign_negative()) {
            r = x;
        }
    }
    Ok(Value::Num(if nan { f64::NAN } else { r }))
}

fn math_pow(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let x = vm.to_number(&arg(args, 0))?;
    let y = vm.to_number(&arg(args, 1))?;
    Ok(Value::Num(math::pow(x, y)))
}

fn math_random(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(vm.random()))
}

/// A JsStr of a Rust string (for the other modules).
pub fn jstr(s: &str) -> JsStr {
    JsStr::from_str(s)
}
