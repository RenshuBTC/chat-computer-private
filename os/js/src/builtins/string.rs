//! String: the constructor (fromCharCode, fromCodePoint, raw), String.prototype's methods (those taking patterns go
//! to a regular expression's own Symbol methods when given one), and string iterators.

use super::{arg, constructor, js_str, sym_method, tag};
use crate::names as n;
use crate::object::*;
use crate::string::{JsStr, is_line_terminator, is_space};
use crate::value::{Gc, Value};
use crate::vm::{I, Sym, Vm};
use alloc::string::String;
use alloc::vec::Vec;

type R = Result<Value, Value>;

pub fn init(vm: &mut Vm) {
    let sp = vm.proto(I::StringProto);
    vm.put_key(sp, Key::Str(n::LENGTH), Value::Num(0.0), 0);
    let c = constructor(vm, "String", 1, string_call, Some(string_construct), Some(sp));
    vm.method(c, "fromCharCode", 1, from_char_code);
    vm.method(c, "fromCodePoint", 1, from_code_point);
    vm.method(c, "raw", 1, raw);
    let methods: [(&str, u32, NativeFn); 48] = [
        ("at", 1, at),
        ("charAt", 1, char_at),
        ("charCodeAt", 1, char_code_at),
        ("codePointAt", 1, code_point_at),
        ("concat", 1, concat),
        ("endsWith", 1, ends_with),
        ("includes", 1, includes),
        ("indexOf", 1, index_of),
        ("isWellFormed", 0, is_well_formed),
        ("lastIndexOf", 1, last_index_of),
        ("localeCompare", 1, locale_compare),
        ("match", 1, match_),
        ("matchAll", 1, match_all),
        ("normalize", 0, normalize),
        ("padEnd", 1, pad_end),
        ("padStart", 1, pad_start),
        ("repeat", 1, repeat),
        ("replace", 2, replace),
        ("replaceAll", 2, replace_all),
        ("search", 1, search),
        ("slice", 2, slice),
        ("split", 2, split),
        ("startsWith", 1, starts_with),
        ("substring", 2, substring),
        ("substr", 2, substr),
        ("toLocaleLowerCase", 0, to_lower),
        ("toLocaleUpperCase", 0, to_upper),
        ("toLowerCase", 0, to_lower),
        ("toUpperCase", 0, to_upper),
        ("toString", 0, value_of),
        ("valueOf", 0, value_of),
        ("toWellFormed", 0, to_well_formed),
        ("trim", 0, trim),
        ("trimEnd", 0, trim_end),
        ("trimStart", 0, trim_start),
        ("anchor", 1, anchor),
        ("big", 0, big),
        ("blink", 0, blink),
        ("bold", 0, bold),
        ("fixed", 0, fixed),
        ("fontcolor", 1, fontcolor),
        ("fontsize", 1, fontsize),
        ("italics", 0, italics),
        ("link", 1, link),
        ("small", 0, small),
        ("strike", 0, strike),
        ("sub", 0, sub),
        ("sup", 0, sup),
    ];
    for (name, len, f) in methods {
        let g = vm.method(sp, name, len, f);
        // (trimLeft and trimRight are trimStart and trimEnd themselves.)
        match name {
            "trimStart" => vm.put(sp, "trimLeft", Value::Obj(g), HIDDEN),
            "trimEnd" => vm.put(sp, "trimRight", Value::Obj(g), HIDDEN),
            _ => {}
        }
    }
    sym_method(vm, sp, Sym::Iterator, "[Symbol.iterator]", 0, iterator);
    let si = vm.proto(I::StringIterProto);
    vm.method(si, "next", 0, string_iter_next);
    tag(vm, si, "String Iterator");
}

fn string_call(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    if args.is_empty() {
        return Ok(Value::Str(JsStr::empty()));
    }
    if let Value::Sym(s) = &args[0] {
        return Ok(Value::Str(super::core::symbol_descriptive(vm, *s)));
    }
    Ok(Value::Str(vm.to_string(&args[0])?))
}

fn string_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let s = if args.is_empty() { JsStr::empty() } else { vm.to_string(&args[0])? };
    let p = vm.proto_from_constructor(nt, I::StringProto)?;
    Ok(Value::Obj(vm.alloc(Some(p), Class::String(s))))
}

fn from_char_code(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        out.push(vm.to_uint32(a)? as u16);
    }
    Ok(Value::Str(JsStr::from_units(out)))
}

fn from_code_point(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        let x = vm.to_number(a)?;
        if crate::math::trunc(x) != x || !(0.0..=1114111.0).contains(&x) {
            return Err(vm.range_error(&alloc::format!("Invalid code point {}", crate::value::number_to_string(x))));
        }
        push_code_point(&mut out, x as u32);
    }
    Ok(Value::Str(JsStr::from_units(out)))
}

pub fn push_code_point(out: &mut Vec<u16>, cp: u32) {
    if cp < 0x10000 {
        out.push(cp as u16);
    } else {
        let c = cp - 0x10000;
        out.push(0xD800 + (c >> 10) as u16);
        out.push(0xDC00 + (c & 0x3FF) as u16);
    }
}

/// The code point at a place (a lone surrogate is itself), and its length in units.
pub fn code_point_at_units(u: &[u16], i: usize) -> (u32, usize) {
    let c = u[i];
    if (0xD800..0xDC00).contains(&c)
        && let Some(&d) = u.get(i + 1)
        && (0xDC00..0xE000).contains(&d)
    {
        return (0x10000 + (((c as u32) - 0xD800) << 10) + (d as u32 - 0xDC00), 2);
    }
    (c as u32, 1)
}

fn raw(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let cooked = vm.to_object(&arg(args, 0))?;
    let rv = vm.get(cooked, Key::Str(n::RAW), &Value::Obj(cooked))?;
    let raw = vm.to_object(&rv)?;
    let len = vm.length_of(raw)?;
    let mut out: Vec<u16> = Vec::new();
    for i in 0..len {
        let seg = vm.get(raw, Key::Index(i as u32), &Value::Obj(raw))?;
        let s = vm.to_string(&seg)?;
        out.extend_from_slice(s.units());
        if i + 1 < len
            && let Some(sub) = args.get(i as usize + 1)
        {
            let s = vm.to_string(sub)?;
            out.extend_from_slice(s.units());
        }
    }
    Ok(Value::Str(JsStr::from_units(out)))
}

/// `this` as a string (RequireObjectCoercible then ToString).
fn this_str(vm: &mut Vm, this: &Value) -> Result<JsStr, Value> {
    if this.is_nullish() {
        return Err(vm.type_error(&alloc::format!("String.prototype method called on {}", vm.describe(this))));
    }
    vm.to_string(this)
}

fn at(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let len = s.len() as f64;
    let rel = vm.to_integer(&arg(args, 0))?;
    let k = if rel >= 0.0 { rel } else { len + rel };
    if k < 0.0 || k >= len {
        return Ok(Value::Undefined);
    }
    Ok(Value::Str(s.slice(k as usize, k as usize + 1)))
}

fn char_at(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let p = vm.to_integer(&arg(args, 0))?;
    if p < 0.0 || p >= s.len() as f64 {
        return Ok(Value::Str(JsStr::empty()));
    }
    Ok(Value::Str(s.slice(p as usize, p as usize + 1)))
}

fn char_code_at(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let p = vm.to_integer(&arg(args, 0))?;
    if p < 0.0 || p >= s.len() as f64 {
        return Ok(Value::Num(f64::NAN));
    }
    Ok(Value::Num(s.units()[p as usize] as f64))
}

fn code_point_at(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let p = vm.to_integer(&arg(args, 0))?;
    if p < 0.0 || p >= s.len() as f64 {
        return Ok(Value::Undefined);
    }
    Ok(Value::Num(code_point_at_units(s.units(), p as usize).0 as f64))
}

fn concat(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let mut s = this_str(vm, this)?;
    for a in args {
        let t = vm.to_string(a)?;
        s = s.concat(&t);
    }
    Ok(Value::Str(s))
}

/// IsRegExp: an object with a truthy Symbol.match (or, without one, a RegExp).
pub fn is_regexp(vm: &mut Vm, v: &Value) -> Result<bool, Value> {
    let Value::Obj(g) = v else { return Ok(false) };
    let m = vm.sym(Sym::Match);
    let mv = vm.get(*g, Key::Sym(m), v)?;
    if !mv.is_undefined() {
        return Ok(mv.truthy());
    }
    Ok(matches!(vm.heap.get(*g).class, Class::RegExp(_)))
}

fn search_string(vm: &mut Vm, args: &[Value], what: &str) -> Result<JsStr, Value> {
    let v = arg(args, 0);
    if is_regexp(vm, &v)? {
        return Err(vm.type_error(&alloc::format!("First argument to String.prototype.{what} must not be a regular expression")));
    }
    vm.to_string(&v)
}

fn ends_with(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let t = search_string(vm, args, "endsWith")?;
    let len = s.len();
    let end = if arg(args, 1).is_undefined() { len } else { vm.to_integer(&arg(args, 1))?.clamp(0.0, len as f64) as usize };
    let tl = t.len();
    if tl > end {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(&s.units()[end - tl..end] == t.units()))
}

fn starts_with(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let t = search_string(vm, args, "startsWith")?;
    let len = s.len();
    let start = vm.to_integer(&arg(args, 1))?.clamp(0.0, len as f64) as usize;
    let tl = t.len();
    if start + tl > len {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(&s.units()[start..start + tl] == t.units()))
}

fn includes(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let t = search_string(vm, args, "includes")?;
    let start = vm.to_integer(&arg(args, 1))?.clamp(0.0, s.len() as f64) as usize;
    Ok(Value::Bool(s.find(t.units(), start).is_some()))
}

fn index_of(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let t = vm.to_string(&arg(args, 0))?;
    let start = vm.to_integer(&arg(args, 1))?.clamp(0.0, s.len() as f64) as usize;
    Ok(Value::Num(s.find(t.units(), start).map_or(-1.0, |i| i as f64)))
}

fn last_index_of(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let t = vm.to_string(&arg(args, 0))?;
    let p = vm.to_number(&arg(args, 1))?;
    let len = s.len();
    let start = if p.is_nan() { len } else { crate::value::to_integer(p).clamp(0.0, len as f64) as usize };
    let tl = t.len();
    if tl > len {
        return Ok(Value::Num(-1.0));
    }
    let mut i = start.min(len - tl) as i64;
    while i >= 0 {
        if &s.units()[i as usize..i as usize + tl] == t.units() {
            return Ok(Value::Num(i as f64));
        }
        i -= 1;
    }
    Ok(Value::Num(-1.0))
}

fn is_well_formed(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let u = s.units();
    let mut i = 0;
    while i < u.len() {
        let (cp, l) = code_point_at_units(u, i);
        if (0xD800..0xE000).contains(&cp) {
            return Ok(Value::Bool(false));
        }
        i += l;
    }
    Ok(Value::Bool(true))
}

fn to_well_formed(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let u = s.units();
    let mut out = Vec::with_capacity(u.len());
    let mut i = 0;
    while i < u.len() {
        let (cp, l) = code_point_at_units(u, i);
        if (0xD800..0xE000).contains(&cp) {
            out.push(0xFFFD);
        } else {
            out.extend_from_slice(&u[i..i + l]);
        }
        i += l;
    }
    Ok(Value::Str(JsStr::from_units(out)))
}

fn locale_compare(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let t = vm.to_string(&arg(args, 0))?;
    // A simple collation: letters compared without case first, then by case, then by code unit.
    let fold = |x: &JsStr| -> Vec<u32> {
        let mut out = Vec::new();
        let u = x.units();
        let mut i = 0;
        while i < u.len() {
            let (cp, l) = code_point_at_units(u, i);
            let c = char::from_u32(cp).map_or(cp, |c| c.to_lowercase().next().map_or(cp, |l| l as u32));
            out.push(c);
            i += l;
        }
        out
    };
    let (a, b) = (fold(&s), fold(&t));
    let ord = a.cmp(&b).then_with(|| t.units().cmp(s.units()));
    Ok(Value::Num(match ord {
        core::cmp::Ordering::Less => -1.0,
        core::cmp::Ordering::Equal => 0.0,
        core::cmp::Ordering::Greater => 1.0,
    }))
}

/// A pattern method: the pattern's own Symbol method if it has one, else `fallback` with the strings.
fn by_symbol(vm: &mut Vm, this: &Value, args: &[Value], sym: Sym, extra: Option<Value>) -> Result<Option<Value>, Value> {
    if this.is_nullish() {
        return Err(vm.type_error("String.prototype method called on null or undefined"));
    }
    let p = arg(args, 0);
    if !p.is_nullish() {
        let s = vm.sym(sym);
        let m = vm.get_method(&p, Key::Sym(s))?;
        if !m.is_undefined() {
            let mut a = alloc::vec![this.clone()];
            if let Some(x) = extra {
                a.push(x);
            }
            return Ok(Some(vm.call(&m, p, &a)?));
        }
    }
    Ok(None)
}

fn match_(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if let Some(r) = by_symbol(vm, this, args, Sym::Match, None)? {
        return Ok(r);
    }
    let s = this_str(vm, this)?;
    let rx = crate::builtins::regexp::regexp_new(vm, &arg(args, 0), &Value::Undefined)?;
    let m = vm.sym(Sym::Match);
    let f = vm.get(rx, Key::Sym(m), &Value::Obj(rx))?;
    vm.call(&f, Value::Obj(rx), &[Value::Str(s)])
}

fn match_all(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if this.is_nullish() {
        return Err(vm.type_error("String.prototype.matchAll called on null or undefined"));
    }
    let p = arg(args, 0);
    if !p.is_nullish() {
        if is_regexp(vm, &p)? {
            let flags = vm.get_value(&p, Key::Str(n::FLAGS))?;
            if flags.is_nullish() {
                return Err(vm.type_error("String.prototype.matchAll called with a non-global RegExp argument"));
            }
            let fs = vm.to_string(&flags)?;
            if !fs.units().contains(&(b'g' as u16)) {
                return Err(vm.type_error("String.prototype.matchAll called with a non-global RegExp argument"));
            }
        }
        if let Some(r) = by_symbol(vm, this, args, Sym::MatchAll, None)? {
            return Ok(r);
        }
    }
    let s = this_str(vm, this)?;
    let rx = crate::builtins::regexp::regexp_new(vm, &p, &js_str("g"))?;
    let m = vm.sym(Sym::MatchAll);
    let f = vm.get(rx, Key::Sym(m), &Value::Obj(rx))?;
    vm.call(&f, Value::Obj(rx), &[Value::Str(s)])
}

fn search(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if let Some(r) = by_symbol(vm, this, args, Sym::Search, None)? {
        return Ok(r);
    }
    let s = this_str(vm, this)?;
    let rx = crate::builtins::regexp::regexp_new(vm, &arg(args, 0), &Value::Undefined)?;
    let m = vm.sym(Sym::Search);
    let f = vm.get(rx, Key::Sym(m), &Value::Obj(rx))?;
    vm.call(&f, Value::Obj(rx), &[Value::Str(s)])
}

fn normalize(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let f = arg(args, 0);
    if !f.is_undefined() {
        let form = vm.to_string(&f)?.to_rust();
        if !matches!(form.as_str(), "NFC" | "NFD" | "NFKC" | "NFKD") {
            return Err(vm.range_error("The normalization form should be one of NFC, NFD, NFKC, NFKD."));
        }
    }
    Ok(Value::Str(s))
}

fn pad(vm: &mut Vm, this: &Value, args: &[Value], start: bool) -> R {
    let s = this_str(vm, this)?;
    let max = vm.to_length(&arg(args, 0))? as usize;
    let len = s.len();
    if max <= len {
        return Ok(Value::Str(s));
    }
    let fill = arg(args, 1);
    let fill = if fill.is_undefined() { JsStr::from_str(" ") } else { vm.to_string(&fill)? };
    if fill.is_empty() {
        return Ok(Value::Str(s));
    }
    if max > (1 << 29) {
        return Err(vm.range_error("Invalid string length"));
    }
    let need = max - len;
    let fu = fill.units();
    let filler: Vec<u16> = fu.iter().cycle().take(need).copied().collect();
    let f = JsStr::from_units(filler);
    Ok(Value::Str(if start { f.concat(&s) } else { s.concat(&f) }))
}

fn pad_end(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    pad(vm, this, args, false)
}

fn pad_start(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    pad(vm, this, args, true)
}

fn repeat(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let n = vm.to_integer(&arg(args, 0))?;
    if n < 0.0 || n.is_infinite() {
        return Err(vm.range_error("Invalid count value"));
    }
    if s.is_empty() || n == 0.0 {
        return Ok(Value::Str(JsStr::empty()));
    }
    if n * s.len() as f64 > (1u64 << 29) as f64 {
        return Err(vm.range_error("Invalid string length"));
    }
    let mut out = Vec::with_capacity(s.len() * n as usize);
    for _ in 0..n as usize {
        out.extend_from_slice(s.units());
    }
    Ok(Value::Str(JsStr::from_units(out)))
}

/// GetSubstitution: a replacement template's $-patterns filled in.
pub fn substitution(vm: &mut Vm, matched: &[u16], s: &[u16], position: usize, captures: &[Value], named: &Value, replacement: &[u16]) -> Result<Vec<u16>, Value> {
    let mut out = Vec::new();
    let tail = (position + matched.len()).min(s.len());
    let m = captures.len();
    let mut i = 0;
    while i < replacement.len() {
        let c = replacement[i];
        if c != b'$' as u16 || i + 1 >= replacement.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let d = replacement[i + 1];
        match d {
            0x24 => {
                out.push(b'$' as u16);
                i += 2;
            }
            0x26 => {
                out.extend_from_slice(matched);
                i += 2;
            }
            0x60 => {
                out.extend_from_slice(&s[..position.min(s.len())]);
                i += 2;
            }
            0x27 => {
                out.extend_from_slice(&s[tail..]);
                i += 2;
            }
            0x30..=0x39 => {
                let d1 = (d - 0x30) as usize;
                let two = replacement.get(i + 2).filter(|c| (0x30..=0x39).contains(*c)).map(|c| d1 * 10 + (*c - 0x30) as usize);
                let (idx, used) = match two {
                    Some(t) if t >= 1 && t <= m => (t, 3),
                    _ => (d1, 2),
                };
                if idx >= 1 && idx <= m {
                    let cap = &captures[idx - 1];
                    if !cap.is_undefined() {
                        let cs = vm.to_string(cap)?;
                        out.extend_from_slice(cs.units());
                    }
                    i += used;
                } else {
                    out.push(b'$' as u16);
                    i += 1;
                }
            }
            0x3C if !named.is_undefined() => {
                let close = replacement[i + 2..].iter().position(|c| *c == b'>' as u16);
                match close {
                    None => {
                        out.push(b'$' as u16);
                        i += 1;
                    }
                    Some(e) => {
                        let name = JsStr::from_slice(&replacement[i + 2..i + 2 + e]);
                        let k = vm.key_from_str(&name);
                        let v = vm.get_value(named, k)?;
                        if !v.is_undefined() {
                            let vs = vm.to_string(&v)?;
                            out.extend_from_slice(vs.units());
                        }
                        i += 3 + e;
                    }
                }
            }
            _ => {
                out.push(b'$' as u16);
                i += 1;
            }
        }
    }
    Ok(out)
}

fn replace_impl(vm: &mut Vm, this: &Value, args: &[Value], all: bool) -> R {
    if this.is_nullish() {
        return Err(vm.type_error("String.prototype.replace called on null or undefined"));
    }
    let p = arg(args, 0);
    let rep = arg(args, 1);
    if !p.is_nullish() {
        if all && is_regexp(vm, &p)? {
            let flags = vm.get_value(&p, Key::Str(n::FLAGS))?;
            if flags.is_nullish() {
                return Err(vm.type_error("replaceAll must be called with a global RegExp"));
            }
            let fs = vm.to_string(&flags)?;
            if !fs.units().contains(&(b'g' as u16)) {
                return Err(vm.type_error("replaceAll must be called with a global RegExp"));
            }
        }
        if let Some(r) = by_symbol(vm, this, args, Sym::Replace, Some(rep.clone()))? {
            return Ok(r);
        }
    }
    let s = this_str(vm, this)?;
    let search = vm.to_string(&p)?;
    let functional = vm.is_callable(&rep);
    let rep_s = if functional { JsStr::empty() } else { vm.to_string(&rep)? };
    let su = s.units();
    let tu = search.units();
    let adv = tu.len().max(1);
    let mut positions = Vec::new();
    let mut at = s.find(tu, 0);
    while let Some(pos) = at {
        positions.push(pos);
        if !all {
            break;
        }
        at = if pos + adv <= su.len() { s.find(tu, pos + adv) } else { None };
    }
    if positions.is_empty() {
        return Ok(Value::Str(s));
    }
    let mut out = Vec::new();
    let mut end = 0;
    for pos in positions {
        out.extend_from_slice(&su[end..pos]);
        let r = if functional {
            let v = vm.call(&rep, Value::Undefined, &[Value::Str(search.clone()), Value::Num(pos as f64), Value::Str(s.clone())])?;
            vm.to_string(&v)?.units().to_vec()
        } else {
            substitution(vm, tu, su, pos, &[], &Value::Undefined, rep_s.units())?
        };
        out.extend(r);
        end = pos + tu.len();
    }
    out.extend_from_slice(&su[end..]);
    Ok(Value::Str(JsStr::from_units(out)))
}

fn replace(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    replace_impl(vm, this, args, false)
}

fn replace_all(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    replace_impl(vm, this, args, true)
}

fn slice(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let len = s.len() as u64;
    let from = vm.relative_index(&arg(args, 0), len, 0)? as usize;
    let to = vm.relative_index(&arg(args, 1), len, len)? as usize;
    if from >= to {
        return Ok(Value::Str(JsStr::empty()));
    }
    Ok(Value::Str(s.slice(from, to)))
}

fn split(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if let Some(r) = by_symbol(vm, this, args, Sym::Split, Some(arg(args, 1)))? {
        return Ok(r);
    }
    let s = this_str(vm, this)?;
    let lim = arg(args, 1);
    let limit = if lim.is_undefined() { u32::MAX } else { vm.to_uint32(&lim)? };
    let sep = vm.to_string(&arg(args, 0))?;
    if limit == 0 {
        return Ok(Value::Obj(vm.new_array(Vec::new())));
    }
    if arg(args, 0).is_undefined() {
        return Ok(Value::Obj(vm.new_array(alloc::vec![Value::Str(s)])));
    }
    let su = s.units();
    let tu = sep.units();
    let mut parts = Vec::new();
    if tu.is_empty() {
        for i in 0..su.len().min(limit as usize) {
            parts.push(Value::Str(s.slice(i, i + 1)));
        }
        return Ok(Value::Obj(vm.new_array(parts)));
    }
    if su.is_empty() {
        return Ok(Value::Obj(vm.new_array(alloc::vec![Value::Str(s)])));
    }
    let mut start = 0;
    while let Some(pos) = s.find(tu, start) {
        parts.push(Value::Str(s.slice(start, pos)));
        if parts.len() as u32 >= limit {
            return Ok(Value::Obj(vm.new_array(parts)));
        }
        start = pos + tu.len();
    }
    parts.push(Value::Str(s.slice(start, su.len())));
    Ok(Value::Obj(vm.new_array(parts)))
}

fn substring(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let len = s.len() as f64;
    let a = vm.to_integer(&arg(args, 0))?.clamp(0.0, len) as usize;
    let b = if arg(args, 1).is_undefined() { len as usize } else { vm.to_integer(&arg(args, 1))?.clamp(0.0, len) as usize };
    let (from, to) = (a.min(b), a.max(b));
    Ok(Value::Str(s.slice(from, to)))
}

fn substr(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let len = s.len() as u64;
    let start = vm.relative_index(&arg(args, 0), len, 0)?;
    let l = if arg(args, 1).is_undefined() { len as f64 } else { vm.to_integer(&arg(args, 1))? };
    let end = (start as f64 + l.max(0.0)).min(len as f64) as u64;
    if start >= end {
        return Ok(Value::Str(JsStr::empty()));
    }
    Ok(Value::Str(s.slice(start as usize, end as usize)))
}

fn map_case(s: &JsStr, upper: bool) -> JsStr {
    let u = s.units();
    let mut out = Vec::with_capacity(u.len());
    let mut i = 0;
    let cased = |cp: u32| char::from_u32(cp).is_some_and(|c| c.is_alphabetic());
    while i < u.len() {
        let (cp, l) = code_point_at_units(u, i);
        match char::from_u32(cp) {
            Some('\u{3A3}') if !upper => {
                // Final sigma: after a letter, and not before one.
                let before = i > 0 && cased(code_point_at_units(u, i - 1).0);
                let after = i + l < u.len() && cased(code_point_at_units(u, i + l).0);
                out.push(if before && !after { 0x3C2 } else { 0x3C3 });
            }
            Some(c) => {
                let mut buf = [0u16; 2];
                if upper {
                    for m in c.to_uppercase() {
                        out.extend_from_slice(m.encode_utf16(&mut buf));
                    }
                } else {
                    for m in c.to_lowercase() {
                        out.extend_from_slice(m.encode_utf16(&mut buf));
                    }
                }
            }
            None => out.extend_from_slice(&u[i..i + l]),
        }
        i += l;
    }
    JsStr::from_units(out)
}

fn to_lower(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    if s.units().iter().all(|c| *c < 128 && !(b'A' as u16..=b'Z' as u16).contains(c)) {
        return Ok(Value::Str(s));
    }
    Ok(Value::Str(map_case(&s, false)))
}

fn to_upper(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    Ok(Value::Str(map_case(&s, true)))
}

fn value_of(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    match this {
        Value::Str(s) => Ok(Value::Str(s.clone())),
        Value::Obj(g) => match &vm.heap.get(*g).class {
            Class::String(s) => Ok(Value::Str(s.clone())),
            _ => Err(vm.type_error("String.prototype.valueOf requires that 'this' be a String")),
        },
        _ => Err(vm.type_error("String.prototype.valueOf requires that 'this' be a String")),
    }
}

fn white(c: u16) -> bool {
    is_space(c as u32) || is_line_terminator(c as u32)
}

fn trim_impl(vm: &mut Vm, this: &Value, start: bool, end: bool) -> R {
    let s = this_str(vm, this)?;
    let u = s.units();
    let mut a = 0;
    let mut b = u.len();
    if start {
        while a < b && white(u[a]) {
            a += 1;
        }
    }
    if end {
        while b > a && white(u[b - 1]) {
            b -= 1;
        }
    }
    Ok(Value::Str(s.slice(a, b)))
}

fn trim(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    trim_impl(vm, this, true, true)
}

fn trim_start(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    trim_impl(vm, this, true, false)
}

fn trim_end(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    trim_impl(vm, this, false, true)
}

/// The HTML methods (Annex B): <tag attr="value">s</tag>.
fn html(vm: &mut Vm, this: &Value, tag: &str, attr: Option<(&str, &Value)>) -> R {
    let s = this_str(vm, this)?;
    let mut out = String::from("<");
    out.push_str(tag);
    if let Some((name, v)) = attr {
        let val = vm.to_string(v)?.to_rust().replace('"', "&quot;");
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        out.push_str(&val);
        out.push('"');
    }
    out.push('>');
    let mut r = JsStr::from_str(&out).concat(&s);
    r = r.concat(&JsStr::from_str(&alloc::format!("</{tag}>")));
    Ok(Value::Str(r))
}

fn anchor(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    html(vm, this, "a", Some(("name", &arg(args, 0))))
}
fn big(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    html(vm, this, "big", None)
}
fn blink(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    html(vm, this, "blink", None)
}
fn bold(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    html(vm, this, "b", None)
}
fn fixed(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    html(vm, this, "tt", None)
}
fn fontcolor(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    html(vm, this, "font", Some(("color", &arg(args, 0))))
}
fn fontsize(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    html(vm, this, "font", Some(("size", &arg(args, 0))))
}
fn italics(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    html(vm, this, "i", None)
}
fn link(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    html(vm, this, "a", Some(("href", &arg(args, 0))))
}
fn small(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    html(vm, this, "small", None)
}
fn strike(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    html(vm, this, "strike", None)
}
fn sub(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    html(vm, this, "sub", None)
}
fn sup(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    html(vm, this, "sup", None)
}

fn iterator(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = this_str(vm, this)?;
    let p = vm.proto(I::StringIterProto);
    Ok(Value::Obj(vm.alloc(Some(p), Class::StringIter(s, 0))))
}

fn string_iter_next(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let g = match this {
        Value::Obj(g) if matches!(vm.heap.get(*g).class, Class::StringIter(..)) => *g,
        _ => return Err(vm.type_error("next method called on incompatible receiver")),
    };
    let (s, at) = match &vm.heap.get(g).class {
        Class::StringIter(s, at) => (s.clone(), *at as usize),
        _ => unreachable!(),
    };
    if at >= s.len() {
        if let Class::StringIter(_, a) = &mut vm.heap.get_mut(g).class {
            *a = u32::MAX;
        }
        return Ok(vm.iter_result(Value::Undefined, true));
    }
    let (_, l) = code_point_at_units(s.units(), at);
    if let Class::StringIter(_, a) = &mut vm.heap.get_mut(g).class {
        *a = (at + l) as u32;
    }
    let v = Value::Str(s.slice(at, at + l));
    Ok(vm.iter_result(v, false))
}
