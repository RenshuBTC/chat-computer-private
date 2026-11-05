//! RegExp: the constructor, exec and test, the flag getters, and the Symbol methods String.prototype's pattern
//! methods call (match, matchAll, replace, search, split), as the language specifies them so that subclasses and
//! objects that look like regular expressions work; with quick ways when it's a plain RegExp of ours.

use super::{arg, constructor, getter, js_str, species_getter, sym_getter, sym_method, tag};
use crate::names as n;
use crate::object::*;
use crate::regexp::{Flags, Program, compile};
use crate::string::JsStr;
use crate::value::{Gc, Value};
use crate::vm::{I, Sym, Vm};
use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

type R = Result<Value, Value>;

pub fn init(vm: &mut Vm) {
    let rp = vm.proto(I::RegExpProto);
    let c = constructor(vm, "RegExp", 2, regexp_call, Some(regexp_construct), Some(rp));
    vm.intrinsics[I::RegExpCtor as usize] = c;
    sym_getter(vm, c, Sym::Species, "[Symbol.species]", species_getter);
    let ex = vm.method(rp, "exec", 1, exec);
    vm.intrinsics[I::RegExpExec as usize] = ex;
    vm.method(rp, "test", 1, test);
    vm.method(rp, "toString", 0, to_string);
    vm.method(rp, "compile", 2, compile_method);
    getter(vm, rp, "flags", flags_getter);
    getter(vm, rp, "source", source_getter);
    for (name, f) in [
        ("hasIndices", has_indices as NativeFn),
        ("global", global),
        ("ignoreCase", ignore_case),
        ("multiline", multiline),
        ("dotAll", dot_all),
        ("unicode", unicode),
        ("unicodeSets", unicode_sets),
        ("sticky", sticky),
    ] {
        getter(vm, rp, name, f);
    }
    sym_method(vm, rp, Sym::Match, "[Symbol.match]", 1, symbol_match);
    sym_method(vm, rp, Sym::MatchAll, "[Symbol.matchAll]", 1, symbol_match_all);
    sym_method(vm, rp, Sym::Replace, "[Symbol.replace]", 2, symbol_replace);
    sym_method(vm, rp, Sym::Search, "[Symbol.search]", 1, symbol_search);
    sym_method(vm, rp, Sym::Split, "[Symbol.split]", 2, symbol_split);
    let ip = vm.proto(I::RegExpStringIterProto);
    vm.method(ip, "next", 0, string_iter_next);
    tag(vm, ip, "RegExp String Iterator");
    // The legacy statics (RegExp.$1 and the rest): the last match's.
    for (name, i) in [("$1", 1), ("$2", 2), ("$3", 3), ("$4", 4), ("$5", 5), ("$6", 6), ("$7", 7), ("$8", 8), ("$9", 9), ("lastMatch", 0), ("$&", 0), ("input", 100), ("$_", 100), ("lastParen", 101), ("$+", 101), ("leftContext", 102), ("$`", 102), ("rightContext", 103), ("$'", 103)] {
        let g = vm.native_with(&format!("get {name}"), 0, legacy_get, None, Value::Num(i as f64));
        let a = vm.atom(name);
        vm.heap.get_mut(c).props.set(Key::Str(a), Prop { slot: Slot::Accessor(Value::Obj(g), Value::Undefined), flags: CONFIGURABLE });
    }
}

// ------------------------------------------------------------------------------------------------- making them

/// RegExpCreate: a new RegExp of ours from a pattern and flags (any values).
pub fn regexp_new(vm: &mut Vm, pattern: &Value, flags: &Value) -> Result<Gc, Value> {
    let p = vm.proto(I::RegExpProto);
    let o = alloc_regexp(vm, p);
    initialize(vm, o, pattern, flags)?;
    Ok(o)
}

fn alloc_regexp(vm: &mut Vm, proto: Gc) -> Gc {
    let program = Rc::new(compile(&[], Flags::default()).ok().expect("an empty pattern compiles"));
    let o = vm.alloc(Some(proto), Class::RegExp(Box::new(RegExpData { source: JsStr::empty(), flags: String::new(), program })));
    vm.put_key(o, Key::Str(n::LAST_INDEX), Value::Num(0.0), WRITABLE);
    o
}

/// A compiled program (from the VM's cache), or a SyntaxError.
fn program(vm: &mut Vm, source: &JsStr, flags: &str) -> Result<Rc<Program>, Value> {
    if let Some(p) = vm.regexps.get(&(source.clone(), String::from(flags))) {
        return Ok(p.clone());
    }
    let Some(f) = Flags::parse(flags) else { return Err(vm.syntax_error(&format!("Invalid regular expression flags '{flags}'"))) };
    match compile(source.units(), f) {
        Ok(p) => {
            let p = Rc::new(p);
            if vm.regexps.len() > 4096 {
                vm.regexps.clear();
            }
            vm.regexps.insert((source.clone(), String::from(flags)), p.clone());
            Ok(p)
        }
        Err(e) => Err(vm.syntax_error(&format!("Invalid regular expression: /{}/{}: {}", source, flags, e.0))),
    }
}

/// RegExpInitialize.
fn initialize(vm: &mut Vm, o: Gc, pattern: &Value, flags: &Value) -> Result<(), Value> {
    let p = if pattern.is_undefined() { JsStr::empty() } else { vm.to_string(pattern)? };
    let f = if flags.is_undefined() { String::new() } else { vm.to_string(flags)?.to_rust() };
    let prog = program(vm, &p, &f)?;
    if let Class::RegExp(d) = &mut vm.heap.get_mut(o).class {
        d.source = p;
        d.flags = f;
        d.program = prog;
    }
    if !vm.set(o, Key::Str(n::LAST_INDEX), Value::Num(0.0), &Value::Obj(o))? {
        return Err(vm.type_error("Cannot assign to read only property 'lastIndex'"));
    }
    Ok(())
}

impl Vm {
    /// A regular expression literal's object.
    pub fn regexp_create(&mut self, src: &JsStr, flags: &str) -> Result<Gc, Value> {
        let p = self.proto(I::RegExpProto);
        let o = alloc_regexp(self, p);
        let prog = program(self, src, flags)?;
        if let Class::RegExp(d) = &mut self.heap.get_mut(o).class {
            d.source = src.clone();
            d.flags = String::from(flags);
            d.program = prog;
        }
        Ok(o)
    }
}

fn data_of(vm: &Vm, v: &Value) -> Option<(JsStr, String, Rc<Program>)> {
    match v {
        Value::Obj(g) => match &vm.heap.get(*g).class {
            Class::RegExp(d) => Some((d.source.clone(), d.flags.clone(), d.program.clone())),
            _ => None,
        },
        _ => None,
    }
}

fn regexp_call(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let pattern = arg(args, 0);
    let flags = arg(args, 1);
    if crate::builtins::string::is_regexp(vm, &pattern)? && flags.is_undefined() {
        let c = vm.get_value(&pattern, Key::Str(n::CONSTRUCTOR))?;
        if matches!(c, Value::Obj(cg) if cg == f) {
            return Ok(pattern);
        }
    }
    regexp_construct(vm, &Value::Obj(f), args, f)
}

fn regexp_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let pattern = arg(args, 0);
    let flags = arg(args, 1);
    let (p, fl) = if let Some((src, f, _)) = data_of(vm, &pattern) {
        (Value::Str(src), if flags.is_undefined() { js_str(&f) } else { flags })
    } else if crate::builtins::string::is_regexp(vm, &pattern)? {
        let src = vm.get_value(&pattern, Key::Str(n::SOURCE))?;
        let f = if flags.is_undefined() { vm.get_value(&pattern, Key::Str(n::FLAGS))? } else { flags };
        (src, f)
    } else {
        (pattern, flags)
    };
    let proto = vm.proto_from_constructor(nt, I::RegExpProto)?;
    let o = alloc_regexp(vm, proto);
    initialize(vm, o, &p, &fl)?;
    Ok(Value::Obj(o))
}

fn compile_method(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(o) = this else { return Err(vm.type_error("RegExp.prototype.compile called on incompatible receiver")) };
    if data_of(vm, this).is_none() {
        return Err(vm.type_error("RegExp.prototype.compile called on incompatible receiver"));
    }
    let pattern = arg(args, 0);
    let flags = arg(args, 1);
    let (p, f) = match data_of(vm, &pattern) {
        Some((src, fl, _)) => {
            if !flags.is_undefined() {
                return Err(vm.type_error("Cannot supply flags when constructing one RegExp from another"));
            }
            (Value::Str(src), js_str(&fl))
        }
        None => (pattern, flags),
    };
    initialize(vm, *o, &p, &f)?;
    Ok(this.clone())
}

// ------------------------------------------------------------------------------------------------- the getters

fn flag_getter(vm: &mut Vm, this: &Value, c: char, name: &str) -> R {
    if let Some((_, f, _)) = data_of(vm, this) {
        return Ok(Value::Bool(f.contains(c)));
    }
    if matches!(this, Value::Obj(g) if *g == vm.proto(I::RegExpProto)) {
        return Ok(Value::Undefined);
    }
    Err(vm.type_error(&format!("RegExp.prototype.{name} getter called on non-RegExp")))
}

fn has_indices(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, 'd', "hasIndices")
}
fn global(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, 'g', "global")
}
fn ignore_case(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, 'i', "ignoreCase")
}
fn multiline(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, 'm', "multiline")
}
fn dot_all(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, 's', "dotAll")
}
fn unicode(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, 'u', "unicode")
}
fn unicode_sets(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, 'v', "unicodeSets")
}
fn sticky(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, 'y', "sticky")
}

fn flags_getter(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Value::Obj(o) = this else { return Err(vm.type_error("RegExp.prototype.flags getter called on non-object")) };
    let mut out = String::new();
    for (c, name) in [('d', n::HAS_INDICES), ('g', n::GLOBAL), ('i', n::IGNORE_CASE), ('m', n::MULTILINE), ('s', n::DOT_ALL), ('u', n::UNICODE), ('v', n::UNICODE_SETS), ('y', n::STICKY)] {
        if vm.get(*o, Key::Str(name), this)?.truthy() {
            out.push(c);
        }
    }
    Ok(js_str(&out))
}

/// EscapeRegExpPattern: the source as a literal would show it.
fn escape_source(s: &JsStr) -> JsStr {
    if s.is_empty() {
        return JsStr::from_str("(?:)");
    }
    let mut out = Vec::with_capacity(s.len());
    let mut in_class = false;
    let u = s.units();
    let mut i = 0;
    while i < u.len() {
        let c = u[i];
        match c {
            0x5C => {
                out.push(c);
                if let Some(&d) = u.get(i + 1) {
                    out.push(d);
                    i += 1;
                }
            }
            0x5B => {
                in_class = true;
                out.push(c);
            }
            0x5D => {
                in_class = false;
                out.push(c);
            }
            0x2F if !in_class => out.extend("\\/".encode_utf16()),
            0x0A => out.extend("\\n".encode_utf16()),
            0x0D => out.extend("\\r".encode_utf16()),
            0x2028 => out.extend("\\u2028".encode_utf16()),
            0x2029 => out.extend("\\u2029".encode_utf16()),
            _ => out.push(c),
        }
        i += 1;
    }
    JsStr::from_units(out)
}

fn source_getter(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    if let Some((src, _, _)) = data_of(vm, this) {
        return Ok(Value::Str(escape_source(&src)));
    }
    if matches!(this, Value::Obj(g) if *g == vm.proto(I::RegExpProto)) {
        return Ok(js_str("(?:)"));
    }
    Err(vm.type_error("RegExp.prototype.source getter called on non-RegExp"))
}

fn to_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Value::Obj(o) = this else { return Err(vm.type_error("RegExp.prototype.toString called on non-object")) };
    let src = vm.get(*o, Key::Str(n::SOURCE), this)?;
    let src = vm.to_string(&src)?;
    let fl = vm.get(*o, Key::Str(n::FLAGS), this)?;
    let fl = vm.to_string(&fl)?;
    Ok(Value::Str(JsStr::from_str("/").concat(&src).concat(&JsStr::from_str("/")).concat(&fl)))
}

// ---------------------------------------------------------------------------------------------------- exec

fn last_index(vm: &mut Vm, o: Gc) -> Result<u64, Value> {
    let v = vm.get(o, Key::Str(n::LAST_INDEX), &Value::Obj(o))?;
    vm.to_length(&v)
}

fn set_last_index(vm: &mut Vm, o: Gc, i: f64) -> Result<(), Value> {
    if !vm.set(o, Key::Str(n::LAST_INDEX), Value::Num(i), &Value::Obj(o))? {
        return Err(vm.type_error("Cannot assign to read only property 'lastIndex'"));
    }
    Ok(())
}

/// AdvanceStringIndex.
pub fn advance(s: &[u16], i: u64, unicode: bool) -> u64 {
    if !unicode || i + 1 >= s.len() as u64 {
        return i + 1;
    }
    let c = s[i as usize];
    if (0xD800..0xDC00).contains(&c) && (0xDC00..0xE000).contains(&s[i as usize + 1]) { i + 2 } else { i + 1 }
}

/// The last successful match (for the legacy statics): kept on the RegExp constructor's native data.
fn remember(vm: &mut Vm, s: &JsStr, caps: &[Option<(usize, usize)>]) {
    let c = vm.proto(I::RegExpCtor);
    let mut parts: Vec<Value> = vec![Value::Str(s.clone())];
    for c in caps {
        match c {
            Some((a, b)) => {
                parts.push(Value::Num(*a as f64));
                parts.push(Value::Num(*b as f64));
            }
            None => {
                parts.push(Value::Undefined);
                parts.push(Value::Undefined);
            }
        }
    }
    let arr = vm.new_array(parts);
    vm.set_data2(c, Value::Obj(arr));
}

fn legacy_get(vm: &mut Vm, this: &Value, _args: &[Value], f: Gc) -> R {
    let which = match vm.data(f) {
        Value::Num(x) => x as usize,
        _ => 0,
    };
    let Value::Obj(c) = this else { return Err(vm.type_error("RegExp legacy static accessor called on non-object")) };
    let Value::Obj(arr) = vm.data2(*c) else { return Ok(Value::Str(JsStr::empty())) };
    let parts = vm.array_values(&Value::Obj(arr));
    let Some(Value::Str(s)) = parts.first().cloned() else { return Ok(Value::Str(JsStr::empty())) };
    let span = |k: usize| -> Option<(usize, usize)> {
        match (parts.get(1 + 2 * k), parts.get(2 + 2 * k)) {
            (Some(Value::Num(a)), Some(Value::Num(b))) => Some((*a as usize, *b as usize)),
            _ => None,
        }
    };
    let groups = (parts.len() - 1) / 2;
    let sub = |sp: Option<(usize, usize)>| sp.map_or(JsStr::empty(), |(a, b)| s.slice(a, b));
    Ok(Value::Str(match which {
        100 => s.clone(),
        101 => sub(if groups > 1 { span(groups - 1) } else { None }),
        102 => span(0).map_or(JsStr::empty(), |(a, _)| s.slice(0, a)),
        103 => span(0).map_or(JsStr::empty(), |(_, b)| s.slice(b, s.len())),
        k => sub(if k < groups { span(k) } else { None }),
    }))
}

/// RegExpBuiltinExec: a match array, or null.
fn builtin_exec(vm: &mut Vm, o: Gc, s: &JsStr) -> R {
    let Some((_, flags, prog)) = data_of(vm, &Value::Obj(o)) else { return Err(vm.type_error("not a RegExp")) };
    let len = s.len() as u64;
    let mut li = last_index(vm, o)?;
    let (g, y, d) = (flags.contains('g'), flags.contains('y'), flags.contains('d'));
    if !g && !y {
        li = 0;
    }
    if li > len {
        if g || y {
            set_last_index(vm, o, 0.0)?;
        }
        return Ok(Value::Null);
    }
    let caps = match prog.exec(s.units(), li as usize, y) {
        Ok(Some(c)) => c,
        Ok(None) => {
            if g || y {
                set_last_index(vm, o, 0.0)?;
            }
            return Ok(Value::Null);
        }
        Err(_) => return Err(vm.make_error(crate::vm::ErrorKind::Range, "Regular expression too complex")),
    };
    let (start, end) = caps[0].unwrap_or((li as usize, li as usize));
    if g || y {
        set_last_index(vm, o, end as f64)?;
    }
    remember(vm, s, &caps);
    Ok(Value::Obj(match_array(vm, s, &caps, &prog, start, d)))
}

/// The array exec returns.
fn match_array(vm: &mut Vm, s: &JsStr, caps: &[Option<(usize, usize)>], prog: &Program, start: usize, indices: bool) -> Gc {
    let values: Vec<Value> = caps.iter().map(|c| c.map_or(Value::Undefined, |(a, b)| Value::Str(s.slice(a, b)))).collect();
    let a = vm.new_array(values.clone());
    vm.put_key(a, Key::Str(n::INDEX), Value::Num(start as f64), ALL);
    vm.put_key(a, Key::Str(n::INPUT), Value::Str(s.clone()), ALL);
    let groups = if prog.names.is_empty() {
        Value::Undefined
    } else {
        let gobj = vm.alloc(None, Class::Ordinary);
        for (name, i) in &prog.names {
            let k = vm.key_from_str(&JsStr::from_slice(name));
            vm.put_key(gobj, k, values[*i].clone(), ALL);
        }
        Value::Obj(gobj)
    };
    vm.put_key(a, Key::Str(n::GROUPS), groups, ALL);
    if indices {
        let pairs: Vec<Value> = caps
            .iter()
            .map(|c| match c {
                Some((x, y)) => Value::Obj(vm.new_array(vec![Value::Num(*x as f64), Value::Num(*y as f64)])),
                None => Value::Undefined,
            })
            .collect();
        let ia = vm.new_array(pairs.clone());
        let ig = if prog.names.is_empty() {
            Value::Undefined
        } else {
            let gobj = vm.alloc(None, Class::Ordinary);
            for (name, i) in &prog.names {
                let k = vm.key_from_str(&JsStr::from_slice(name));
                vm.put_key(gobj, k, pairs[*i].clone(), ALL);
            }
            Value::Obj(gobj)
        };
        vm.put_key(ia, Key::Str(n::GROUPS), ig, ALL);
        vm.put_key(a, Key::Str(n::INDICES), Value::Obj(ia), ALL);
    }
    a
}

/// RegExpExec: the object's own exec (if it's changed), else the built-in one.
fn regexp_exec(vm: &mut Vm, o: Gc, s: &JsStr) -> R {
    let ex = vm.get(o, Key::Str(n::EXEC), &Value::Obj(o))?;
    if vm.is_callable(&ex) {
        let is_builtin = matches!(&ex, Value::Obj(f) if *f == vm.proto(I::RegExpExec));
        if is_builtin && data_of(vm, &Value::Obj(o)).is_some() {
            return builtin_exec(vm, o, s);
        }
        let r = vm.call(&ex, Value::Obj(o), &[Value::Str(s.clone())])?;
        if !r.is_object() && !matches!(r, Value::Null) {
            return Err(vm.type_error("exec result must be an object or null"));
        }
        return Ok(r);
    }
    if data_of(vm, &Value::Obj(o)).is_none() {
        return Err(vm.type_error("RegExp exec called on incompatible receiver"));
    }
    builtin_exec(vm, o, s)
}

fn this_regexp(vm: &mut Vm, this: &Value, what: &str) -> Result<Gc, Value> {
    match this {
        Value::Obj(g) if data_of(vm, this).is_some() => Ok(*g),
        _ => Err(vm.type_error(&format!("RegExp.prototype.{what} called on incompatible receiver"))),
    }
}

fn exec(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = this_regexp(vm, this, "exec")?;
    let s = vm.to_string(&arg(args, 0))?;
    builtin_exec(vm, o, &s)
}

fn test(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(o) = this else { return Err(vm.type_error("RegExp.prototype.test called on non-object")) };
    let s = vm.to_string(&arg(args, 0))?;
    let r = regexp_exec(vm, *o, &s)?;
    Ok(Value::Bool(!matches!(r, Value::Null)))
}

fn flags_of(vm: &mut Vm, o: Gc) -> Result<String, Value> {
    let f = vm.get(o, Key::Str(n::FLAGS), &Value::Obj(o))?;
    Ok(vm.to_string(&f)?.to_rust())
}

fn symbol_match(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(o) = this else { return Err(vm.type_error("RegExp.prototype[Symbol.match] called on non-object")) };
    let o = *o;
    let s = vm.to_string(&arg(args, 0))?;
    let flags = flags_of(vm, o)?;
    if !flags.contains('g') {
        return regexp_exec(vm, o, &s);
    }
    let full = flags.contains('u') || flags.contains('v');
    set_last_index(vm, o, 0.0)?;
    let mut out = Vec::new();
    loop {
        let r = regexp_exec(vm, o, &s)?;
        let Value::Obj(rg) = r else { break };
        let m = vm.get(rg, Key::Index(0), &r)?;
        let ms = vm.to_string(&m)?;
        if ms.is_empty() {
            let li = last_index(vm, o)?;
            set_last_index(vm, o, advance(s.units(), li, full) as f64)?;
        }
        out.push(Value::Str(ms));
    }
    if out.is_empty() {
        return Ok(Value::Null);
    }
    Ok(Value::Obj(vm.new_array(out)))
}

fn symbol_match_all(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(o) = this else { return Err(vm.type_error("RegExp.prototype[Symbol.matchAll] called on non-object")) };
    let o = *o;
    let s = vm.to_string(&arg(args, 0))?;
    let c = vm.species_constructor(o, I::RegExpCtor)?;
    let flags = flags_of(vm, o)?;
    let matcher = vm.construct(&c, &[Value::Obj(o), js_str(&flags)], &c)?;
    let Value::Obj(m) = matcher else { return Err(vm.type_error("not an object")) };
    let li = last_index(vm, o)?;
    set_last_index(vm, m, li as f64)?;
    let p = vm.proto(I::RegExpStringIterProto);
    let it = vm.alloc(Some(p), Class::RegExpStringIter(Value::Obj(m), s, flags.contains('g'), flags.contains('u') || flags.contains('v'), false));
    Ok(Value::Obj(it))
}

fn string_iter_next(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let g = match this {
        Value::Obj(g) if matches!(vm.heap.get(*g).class, Class::RegExpStringIter(..)) => *g,
        _ => return Err(vm.type_error("next method called on incompatible receiver")),
    };
    let Class::RegExpStringIter(m, s, global, full, done) = &vm.heap.get(g).class else { unreachable!() };
    let (m, s, global, full, done) = (m.clone(), s.clone(), *global, *full, *done);
    if done {
        return Ok(vm.iter_result(Value::Undefined, true));
    }
    let Value::Obj(mg) = m else { return Ok(vm.iter_result(Value::Undefined, true)) };
    let r = regexp_exec(vm, mg, &s)?;
    let finish = |vm: &mut Vm| {
        if let Class::RegExpStringIter(_, _, _, _, d) = &mut vm.heap.get_mut(g).class {
            *d = true;
        }
    };
    let Value::Obj(rg) = r else {
        finish(vm);
        return Ok(vm.iter_result(Value::Undefined, true));
    };
    if !global {
        finish(vm);
        return Ok(vm.iter_result(r, false));
    }
    let m0 = vm.get(rg, Key::Index(0), &r)?;
    let ms = vm.to_string(&m0)?;
    if ms.is_empty() {
        let li = last_index(vm, mg)?;
        set_last_index(vm, mg, advance(s.units(), li, full) as f64)?;
    }
    Ok(vm.iter_result(r, false))
}

fn symbol_search(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(o) = this else { return Err(vm.type_error("RegExp.prototype[Symbol.search] called on non-object")) };
    let o = *o;
    let s = vm.to_string(&arg(args, 0))?;
    let prev = vm.get(o, Key::Str(n::LAST_INDEX), this)?;
    if !crate::value::same_value(&prev, &Value::Num(0.0)) {
        set_last_index(vm, o, 0.0)?;
    }
    let r = regexp_exec(vm, o, &s)?;
    let cur = vm.get(o, Key::Str(n::LAST_INDEX), this)?;
    if !crate::value::same_value(&cur, &prev) && !vm.set(o, Key::Str(n::LAST_INDEX), prev, this)? {
        return Err(vm.type_error("Cannot assign to read only property 'lastIndex'"));
    }
    match r {
        Value::Obj(rg) => vm.get(rg, Key::Str(n::INDEX), &r),
        _ => Ok(Value::Num(-1.0)),
    }
}

fn symbol_replace(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(o) = this else { return Err(vm.type_error("RegExp.prototype[Symbol.replace] called on non-object")) };
    let o = *o;
    let s = vm.to_string(&arg(args, 0))?;
    let rv = arg(args, 1);
    let functional = vm.is_callable(&rv);
    let rep = if functional { JsStr::empty() } else { vm.to_string(&rv)? };
    let flags = flags_of(vm, o)?;
    let global = flags.contains('g');
    let full = flags.contains('u') || flags.contains('v');
    if global {
        set_last_index(vm, o, 0.0)?;
    }
    let mut results = Vec::new();
    loop {
        let r = regexp_exec(vm, o, &s)?;
        let Value::Obj(rg) = r else { break };
        results.push(rg);
        if !global {
            break;
        }
        let m0 = vm.get(rg, Key::Index(0), &r)?;
        let ms = vm.to_string(&m0)?;
        if ms.is_empty() {
            let li = last_index(vm, o)?;
            set_last_index(vm, o, advance(s.units(), li, full) as f64)?;
        }
    }
    let su = s.units();
    let mut out: Vec<u16> = Vec::new();
    let mut next = 0usize;
    for rg in results {
        let rv2 = Value::Obj(rg);
        let n_caps = vm.length_of(rg)?.saturating_sub(1);
        let m0 = vm.get(rg, Key::Index(0), &rv2)?;
        let matched = vm.to_string(&m0)?;
        let pos = vm.get(rg, Key::Str(n::INDEX), &rv2)?;
        let pos = vm.to_integer(&pos)?.clamp(0.0, su.len() as f64) as usize;
        let mut caps = Vec::new();
        for k in 1..=n_caps {
            let c = vm.get(rg, Key::Index(k as u32), &rv2)?;
            caps.push(if c.is_undefined() { c } else { Value::Str(vm.to_string(&c)?) });
        }
        let named = vm.get(rg, Key::Str(n::GROUPS), &rv2)?;
        let replacement: Vec<u16> = if functional {
            let mut a = vec![Value::Str(matched.clone())];
            a.extend(caps.iter().cloned());
            a.push(Value::Num(pos as f64));
            a.push(Value::Str(s.clone()));
            if !named.is_undefined() {
                a.push(named);
            }
            let r = vm.call(&rv, Value::Undefined, &a)?;
            vm.to_string(&r)?.units().to_vec()
        } else {
            let named = if named.is_undefined() { named } else { Value::Obj(vm.to_object(&named)?) };
            crate::builtins::string::substitution(vm, matched.units(), su, pos, &caps, &named, rep.units())?
        };
        if pos >= next {
            out.extend_from_slice(&su[next..pos]);
            out.extend(replacement);
            next = (pos + matched.len()).min(su.len());
        }
    }
    if next < su.len() {
        out.extend_from_slice(&su[next..]);
    }
    Ok(Value::Str(JsStr::from_units(out)))
}

fn symbol_split(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(o) = this else { return Err(vm.type_error("RegExp.prototype[Symbol.split] called on non-object")) };
    let o = *o;
    let s = vm.to_string(&arg(args, 0))?;
    let c = vm.species_constructor(o, I::RegExpCtor)?;
    let flags = flags_of(vm, o)?;
    let unicode = flags.contains('u') || flags.contains('v');
    let new_flags = if flags.contains('y') { flags.clone() } else { format!("{flags}y") };
    let splitter = vm.construct(&c, &[Value::Obj(o), js_str(&new_flags)], &c)?;
    let Value::Obj(sp) = splitter else { return Err(vm.type_error("not an object")) };
    let lim = arg(args, 1);
    let lim = if lim.is_undefined() { u32::MAX } else { vm.to_uint32(&lim)? };
    let mut out: Vec<Value> = Vec::new();
    if lim == 0 {
        return Ok(Value::Obj(vm.new_array(out)));
    }
    let su = s.units();
    let size = su.len();
    if size == 0 {
        let z = regexp_exec(vm, sp, &s)?;
        if matches!(z, Value::Null) {
            out.push(Value::Str(s));
        }
        return Ok(Value::Obj(vm.new_array(out)));
    }
    // Quick: a plain RegExp of ours (nothing to observe), searched directly.
    let plain_exec = matches!(vm.get(sp, Key::Str(n::EXEC), &splitter)?, Value::Obj(f) if f == vm.proto(I::RegExpExec));
    if plain_exec && let Some((_, _, prog)) = data_of(vm, &splitter) {
        let (mut p, mut q) = (0usize, 0usize);
        while q < size {
            let caps = match prog.exec(su, q, false) {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(_) => return Err(vm.make_error(crate::vm::ErrorKind::Range, "Regular expression too complex")),
            };
            let Some((start, end)) = caps[0] else { break };
            if start >= size {
                break;
            }
            if end == p || end == start && start == p {
                q = advance(su, start as u64, unicode) as usize;
                continue;
            }
            out.push(Value::Str(s.slice(p, start)));
            if out.len() as u32 == lim {
                return Ok(Value::Obj(vm.new_array(out)));
            }
            p = end;
            for c in caps.iter().skip(1) {
                out.push(c.map_or(Value::Undefined, |(a, b)| Value::Str(s.slice(a, b))));
                if out.len() as u32 == lim {
                    return Ok(Value::Obj(vm.new_array(out)));
                }
            }
            q = p;
        }
        out.push(Value::Str(s.slice(p, size)));
        return Ok(Value::Obj(vm.new_array(out)));
    }
    let (mut p, mut q) = (0u64, 0u64);
    while (q as usize) < size {
        set_last_index(vm, sp, q as f64)?;
        let z = regexp_exec(vm, sp, &s)?;
        let Value::Obj(zg) = z else {
            q = advance(su, q, unicode);
            continue;
        };
        let e = last_index(vm, sp)?.min(size as u64);
        if e == p {
            q = advance(su, q, unicode);
            continue;
        }
        out.push(Value::Str(s.slice(p as usize, q as usize)));
        if out.len() as u32 == lim {
            return Ok(Value::Obj(vm.new_array(out)));
        }
        p = e;
        let ncap = vm.length_of(zg)?.saturating_sub(1);
        for i in 1..=ncap {
            let v = vm.get(zg, Key::Index(i as u32), &z)?;
            out.push(v);
            if out.len() as u32 == lim {
                return Ok(Value::Obj(vm.new_array(out)));
            }
        }
        q = p;
    }
    out.push(Value::Str(s.slice(p as usize, size)));
    Ok(Value::Obj(vm.new_array(out)))
}
