//! The Iterator helpers (ECMAScript 2025): the Iterator constructor and Iterator.from, and Iterator.prototype's
//! map, filter, take, drop and flatMap (lazy: each gives an Iterator Helper that steps the iterator it was made from
//! as it's stepped itself, closing it when it's stopped early or a function given throws) and reduce, toArray,
//! forEach, some, every and find.

use super::{arg, js_str, sym_method, tag};
use crate::names as n;
use crate::object::*;
use crate::value::{Gc, Value};
use crate::vm::{I, Sym, Vm};
use alloc::boxed::Box;
use alloc::vec::Vec;

type R = Result<Value, Value>;

/// An Iterator Helper's state: the iterator it steps (and that iterator's next method), what it does, the function
/// it was given, take's and drop's limit, how many values it's handed the function, flatMap's inner iterator while
/// it's stepping one, and where it is (as a generator would be).
pub struct Helper {
    pub it: Value,
    pub next: Value,
    pub kind: u8,
    pub f: Value,
    pub limit: f64,
    pub counter: f64,
    pub inner: Option<(Value, Value)>,
    pub state: u8,
}

const MAP: u8 = 0;
const FILTER: u8 = 1;
const TAKE: u8 = 2;
const DROP: u8 = 3;
const FLAT_MAP: u8 = 4;

const START: u8 = 0;
const SUSPENDED: u8 = 1;
const RUNNING: u8 = 2;
const DONE: u8 = 3;

pub fn init(vm: &mut Vm) {
    let ip = vm.proto(I::IteratorProto);
    // The Iterator constructor: abstract (only a subclass's new makes one).
    let c = vm.native_with("Iterator", 0, iterator_call, Some(iterator_construct), Value::Undefined);
    vm.put_key(c, Key::Str(n::PROTOTYPE), Value::Obj(ip), 0);
    let g = vm.global;
    vm.put(g, "Iterator", Value::Obj(c), HIDDEN);
    vm.method(c, "from", 1, iterator_from);
    let tag_key = Key::Sym(vm.sym(Sym::ToStringTag));
    vm.accessor(ip, tag_key, "[Symbol.toStringTag]", Some(tag_get), Some(tag_set));
    vm.accessor(ip, Key::Str(n::CONSTRUCTOR), "constructor", Some(ctor_get), Some(ctor_set));
    vm.intrinsics[I::IteratorCtor as usize] = c;
    for (name, length, f) in [
        ("map", 1, map as NativeFn),
        ("filter", 1, filter),
        ("take", 1, take),
        ("drop", 1, drop),
        ("flatMap", 1, flat_map),
        ("reduce", 1, reduce),
        ("toArray", 0, to_array),
        ("forEach", 1, for_each),
        ("some", 1, some),
        ("every", 1, every),
        ("find", 1, find),
    ] {
        vm.method(ip, name, length, f);
    }
    // %IteratorHelperPrototype%, and %WrapForValidIteratorPrototype% (what Iterator.from wraps an iterator in).
    let hp = vm.alloc(Some(ip), Class::Ordinary);
    vm.intrinsics[I::IteratorHelperProto as usize] = hp;
    vm.method(hp, "next", 0, helper_next);
    vm.method(hp, "return", 0, helper_return);
    tag(vm, hp, "Iterator Helper");
    let wp = vm.alloc(Some(ip), Class::Ordinary);
    vm.intrinsics[I::WrapForValidIterProto as usize] = wp;
    vm.method(wp, "next", 0, wrap_next);
    vm.method(wp, "return", 0, wrap_return);
    let _ = sym_method;
}

fn iterator_call(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Err(vm.type_error("Constructor Iterator requires 'new'"))
}

fn iterator_construct(vm: &mut Vm, nt: &Value, _args: &[Value], f: Gc) -> R {
    if matches!(nt, Value::Obj(g) if *g == f) || nt.is_undefined() {
        return Err(vm.type_error("Abstract class Iterator not directly constructable"));
    }
    Ok(Value::Obj(vm.create_from_constructor(nt, I::IteratorProto)?))
}

fn tag_get(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js_str("Iterator"))
}

fn ctor_get(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Obj(vm.proto(I::IteratorCtor)))
}

/// SetterThatIgnoresPrototypeProperties: a property put on the object itself, never on %IteratorPrototype%.
fn set_own(vm: &mut Vm, this: &Value, key: Key, v: Value) -> R {
    let Value::Obj(o) = this else { return Err(vm.type_error("Iterator.prototype's setter called on a value that isn't an object")) };
    if *o == vm.proto(I::IteratorProto) {
        return Err(vm.type_error("Cannot set a property of Iterator.prototype this way"));
    }
    if vm.get_own_property(*o, key)?.is_none() {
        vm.create_data_property_or_throw(*o, key, v)?;
    } else {
        vm.put_value(this, key, v, true)?;
    }
    Ok(Value::Undefined)
}

fn tag_set(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let key = Key::Sym(vm.sym(Sym::ToStringTag));
    set_own(vm, this, key, arg(args, 0))
}

fn ctor_set(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_own(vm, this, Key::Str(n::CONSTRUCTOR), arg(args, 0))
}

// ------------------------------------------------------------------------------------------- the operations

/// GetIteratorDirect: an object's iterator record (it, and its next method, read once).
fn direct(vm: &mut Vm, o: &Value) -> Result<(Value, Value), Value> {
    let next = vm.get_value(o, Key::Str(n::NEXT))?;
    Ok((o.clone(), next))
}

/// IteratorStepValue: the next value, or None when it's done.
fn step_value(vm: &mut Vm, it: &Value, next: &Value) -> Result<Option<Value>, Value> {
    vm.iter_step(it, next)
}

/// IteratorStep: whether there was a next value (its value not read).
fn step(vm: &mut Vm, it: &Value, next: &Value) -> Result<bool, Value> {
    let r = vm.call(next, it.clone(), &[])?;
    let Value::Obj(rg) = r else { return Err(vm.type_error("Iterator result is not an object")) };
    Ok(!vm.get(rg, Key::Str(n::DONE), &r)?.truthy())
}

/// IteratorClose with a throw completion: return() called, what it does ignored, and the error given back.
pub fn close_abrupt(vm: &mut Vm, it: &Value, error: Value) -> Value {
    if let Ok(m) = vm.get_method(it, Key::Str(n::RETURN))
        && !m.is_undefined()
    {
        let _ = vm.call(&m, it.clone(), &[]);
    }
    error
}

/// The receiver of a helper method: an object (else a TypeError).
fn receiver(vm: &mut Vm, this: &Value, what: &str) -> Result<Value, Value> {
    if !this.is_object() {
        return Err(vm.type_error(&alloc::format!("Iterator.prototype.{what} called on a value that isn't an object")));
    }
    Ok(this.clone())
}

/// A function argument: callable, else a TypeError, with the iterator closed.
fn callable(vm: &mut Vm, o: &Value, f: &Value, what: &str) -> Result<(), Value> {
    if !vm.is_callable(f) {
        let e = vm.type_error(&alloc::format!("Iterator.prototype.{what}'s argument is not a function"));
        return Err(close_abrupt(vm, o, e));
    }
    Ok(())
}

/// take's and drop's limit: a number, not NaN, not negative (else a RangeError, with the iterator closed).
fn limit(vm: &mut Vm, o: &Value, v: &Value) -> Result<f64, Value> {
    let x = match vm.to_number(v) {
        Ok(x) => x,
        Err(e) => return Err(close_abrupt(vm, o, e)),
    };
    if x.is_nan() || (x.is_finite() && x > 9007199254740991.0) {
        let e = vm.range_error("The limit must be a number no bigger than 2^53 - 1");
        return Err(close_abrupt(vm, o, e));
    }
    let x = crate::value::to_integer(x);
    if x < 0.0 {
        let e = vm.range_error("The limit must not be negative");
        return Err(close_abrupt(vm, o, e));
    }
    Ok(x)
}

fn helper(vm: &mut Vm, o: &Value, kind: u8, f: Value, lim: f64) -> R {
    let (it, next) = direct(vm, o)?;
    let p = vm.proto(I::IteratorHelperProto);
    let h = Helper { it, next, kind, f, limit: lim, counter: 0.0, inner: None, state: START };
    Ok(Value::Obj(vm.alloc(Some(p), Class::IterHelper(Box::new(h)))))
}

fn map(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = receiver(vm, this, "map")?;
    let f = arg(args, 0);
    callable(vm, &o, &f, "map")?;
    helper(vm, &o, MAP, f, 0.0)
}

fn filter(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = receiver(vm, this, "filter")?;
    let f = arg(args, 0);
    callable(vm, &o, &f, "filter")?;
    helper(vm, &o, FILTER, f, 0.0)
}

fn take(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = receiver(vm, this, "take")?;
    let lim = limit(vm, &o, &arg(args, 0))?;
    helper(vm, &o, TAKE, Value::Undefined, lim)
}

fn drop(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = receiver(vm, this, "drop")?;
    let lim = limit(vm, &o, &arg(args, 0))?;
    helper(vm, &o, DROP, Value::Undefined, lim)
}

fn flat_map(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = receiver(vm, this, "flatMap")?;
    let f = arg(args, 0);
    callable(vm, &o, &f, "flatMap")?;
    helper(vm, &o, FLAT_MAP, f, 0.0)
}

fn reduce(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = receiver(vm, this, "reduce")?;
    let f = arg(args, 0);
    callable(vm, &o, &f, "reduce")?;
    let (it, next) = direct(vm, &o)?;
    let (mut acc, mut counter) = if args.len() < 2 {
        match step_value(vm, &it, &next)? {
            Some(v) => (v, 1.0),
            None => return Err(vm.type_error("Reduce of an empty iterator with no initial value")),
        }
    } else {
        (args[1].clone(), 0.0)
    };
    while let Some(v) = step_value(vm, &it, &next)? {
        acc = match vm.call(&f, Value::Undefined, &[acc, v, Value::Num(counter)]) {
            Ok(r) => r,
            Err(e) => return Err(close_abrupt(vm, &it, e)),
        };
        counter += 1.0;
    }
    Ok(acc)
}

fn to_array(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = receiver(vm, this, "toArray")?;
    let (it, next) = direct(vm, &o)?;
    let mut items: Vec<Value> = Vec::new();
    while let Some(v) = step_value(vm, &it, &next)? {
        items.push(v);
    }
    Ok(Value::Obj(vm.new_array(items)))
}

fn for_each(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = receiver(vm, this, "forEach")?;
    let f = arg(args, 0);
    callable(vm, &o, &f, "forEach")?;
    let (it, next) = direct(vm, &o)?;
    let mut counter = 0.0;
    while let Some(v) = step_value(vm, &it, &next)? {
        if let Err(e) = vm.call(&f, Value::Undefined, &[v, Value::Num(counter)]) {
            return Err(close_abrupt(vm, &it, e));
        }
        counter += 1.0;
    }
    Ok(Value::Undefined)
}

/// some, every and find: the predicate called for each value until it says (for `stop_when`); then the iterator
/// closed, and that value's answer given.
fn search(vm: &mut Vm, this: &Value, args: &[Value], what: &str, stop_when: bool) -> Result<Option<Value>, Value> {
    let o = receiver(vm, this, what)?;
    let f = arg(args, 0);
    callable(vm, &o, &f, what)?;
    let (it, next) = direct(vm, &o)?;
    let mut counter = 0.0;
    while let Some(v) = step_value(vm, &it, &next)? {
        let r = match vm.call(&f, Value::Undefined, &[v.clone(), Value::Num(counter)]) {
            Ok(r) => r,
            Err(e) => return Err(close_abrupt(vm, &it, e)),
        };
        if r.truthy() == stop_when {
            vm.iterator_close(&it)?;
            return Ok(Some(v));
        }
        counter += 1.0;
    }
    Ok(None)
}

fn some(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(search(vm, this, args, "some", true)?.is_some()))
}

fn every(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(search(vm, this, args, "every", false)?.is_none()))
}

fn find(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(search(vm, this, args, "find", true)?.unwrap_or(Value::Undefined))
}

// -------------------------------------------------------------------------------------------------- Iterator.from

/// GetIteratorFlattenable: an iterable's iterator (or an iterator itself), its record; primitives refused (but for
/// strings, if `strings`).
fn flattenable(vm: &mut Vm, v: &Value, strings: bool) -> Result<(Value, Value), Value> {
    if !v.is_object() && !(strings && matches!(v, Value::Str(_))) {
        return Err(vm.type_error("Not an iterator or an iterable object"));
    }
    let it_key = Key::Sym(vm.sym(Sym::Iterator));
    let m = vm.get_method(v, it_key)?;
    let it = if m.is_undefined() { v.clone() } else { vm.call(&m, v.clone(), &[])? };
    if !it.is_object() {
        return Err(vm.type_error("The iterator isn't an object"));
    }
    direct(vm, &it)
}

fn iterator_from(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (it, next) = flattenable(vm, &arg(args, 0), true)?;
    // An iterator that inherits from Iterator.prototype already: itself.
    let ip = vm.proto(I::IteratorProto);
    if let Value::Obj(mut o) = it.clone() {
        while let Some(p) = vm.get_prototype_of(o)? {
            if p == ip {
                return Ok(it);
            }
            o = p;
        }
    }
    let p = vm.proto(I::WrapForValidIterProto);
    Ok(Value::Obj(vm.alloc(Some(p), Class::IterWrap(it, next))))
}

fn wrapped(vm: &mut Vm, this: &Value) -> Result<(Value, Value), Value> {
    if let Value::Obj(g) = this
        && let Class::IterWrap(it, next) = &vm.heap.get(*g).class
    {
        return Ok((it.clone(), next.clone()));
    }
    Err(vm.type_error("Method called on an object that isn't a wrapped iterator"))
}

fn wrap_next(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (it, next) = wrapped(vm, this)?;
    vm.call(&next, it, &[])
}

fn wrap_return(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (it, _) = wrapped(vm, this)?;
    let m = vm.get_method(&it, Key::Str(n::RETURN))?;
    if m.is_undefined() {
        return Ok(vm.iter_result(Value::Undefined, true));
    }
    vm.call(&m, it, &[])
}

// ----------------------------------------------------------------------------------------- Iterator Helpers

fn helper_of(vm: &mut Vm, this: &Value, what: &str) -> Result<Gc, Value> {
    if let Value::Obj(g) = this
        && matches!(vm.heap.get(*g).class, Class::IterHelper(_))
    {
        return Ok(*g);
    }
    Err(vm.type_error(&alloc::format!("%IteratorHelperPrototype%.{what} called on an object that isn't an Iterator Helper")))
}

fn with<T>(vm: &mut Vm, g: Gc, f: impl FnOnce(&mut Helper) -> T) -> T {
    match &mut vm.heap.get_mut(g).class {
        Class::IterHelper(h) => f(h),
        _ => unreachable!("an Iterator Helper"),
    }
}

/// One step of a helper: the value it gives next, or None when it's done.
fn advance(vm: &mut Vm, g: Gc) -> Result<Option<Value>, Value> {
    let (it, next, kind, f) = with(vm, g, |h| (h.it.clone(), h.next.clone(), h.kind, h.f.clone()));
    match kind {
        MAP => {
            let Some(v) = step_value(vm, &it, &next)? else { return Ok(None) };
            let counter = with(vm, g, |h| h.counter);
            let mapped = match vm.call(&f, Value::Undefined, &[v, Value::Num(counter)]) {
                Ok(m) => m,
                Err(e) => return Err(close_abrupt(vm, &it, e)),
            };
            with(vm, g, |h| h.counter += 1.0);
            Ok(Some(mapped))
        }
        FILTER => loop {
            let Some(v) = step_value(vm, &it, &next)? else { return Ok(None) };
            let counter = with(vm, g, |h| h.counter);
            let keep = match vm.call(&f, Value::Undefined, &[v.clone(), Value::Num(counter)]) {
                Ok(r) => r.truthy(),
                Err(e) => return Err(close_abrupt(vm, &it, e)),
            };
            with(vm, g, |h| h.counter += 1.0);
            if keep {
                return Ok(Some(v));
            }
        },
        TAKE => {
            let remaining = with(vm, g, |h| h.limit);
            if remaining == 0.0 {
                vm.iterator_close(&it)?;
                return Ok(None);
            }
            if remaining != f64::INFINITY {
                with(vm, g, |h| h.limit -= 1.0);
            }
            step_value(vm, &it, &next)
        }
        DROP => {
            loop {
                let remaining = with(vm, g, |h| h.limit);
                if remaining <= 0.0 {
                    break;
                }
                if remaining != f64::INFINITY {
                    with(vm, g, |h| h.limit -= 1.0);
                }
                if !step(vm, &it, &next)? {
                    return Ok(None);
                }
            }
            step_value(vm, &it, &next)
        }
        _ => loop {
            if let Some((inner, inner_next)) = with(vm, g, |h| h.inner.clone()) {
                match step_value(vm, &inner, &inner_next) {
                    Err(e) => return Err(close_abrupt(vm, &it, e)),
                    Ok(Some(v)) => return Ok(Some(v)),
                    Ok(None) => with(vm, g, |h| {
                        h.inner = None;
                        h.counter += 1.0;
                    }),
                }
                continue;
            }
            let Some(v) = step_value(vm, &it, &next)? else { return Ok(None) };
            let counter = with(vm, g, |h| h.counter);
            let mapped = match vm.call(&f, Value::Undefined, &[v, Value::Num(counter)]) {
                Ok(m) => m,
                Err(e) => return Err(close_abrupt(vm, &it, e)),
            };
            let inner = match flattenable(vm, &mapped, false) {
                Ok(x) => x,
                Err(e) => return Err(close_abrupt(vm, &it, e)),
            };
            with(vm, g, |h| h.inner = Some(inner));
        },
    }
}

fn helper_next(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let g = helper_of(vm, this, "next")?;
    match with(vm, g, |h| h.state) {
        RUNNING => return Err(vm.type_error("Iterator Helper is already running")),
        DONE => return Ok(vm.iter_result(Value::Undefined, true)),
        _ => {}
    }
    with(vm, g, |h| h.state = RUNNING);
    match advance(vm, g) {
        Ok(Some(v)) => {
            with(vm, g, |h| h.state = SUSPENDED);
            Ok(vm.iter_result(v, false))
        }
        Ok(None) => {
            with(vm, g, |h| {
                h.state = DONE;
                h.inner = None;
            });
            Ok(vm.iter_result(Value::Undefined, true))
        }
        Err(e) => {
            with(vm, g, |h| {
                h.state = DONE;
                h.inner = None;
            });
            Err(e)
        }
    }
}

fn helper_return(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let g = helper_of(vm, this, "return")?;
    let (state, it, inner) = with(vm, g, |h| (h.state, h.it.clone(), h.inner.clone()));
    match state {
        RUNNING => return Err(vm.type_error("Iterator Helper is already running")),
        DONE => return Ok(vm.iter_result(Value::Undefined, true)),
        START => {
            with(vm, g, |h| h.state = DONE);
            vm.iterator_close(&it)?;
            return Ok(vm.iter_result(Value::Undefined, true));
        }
        _ => {}
    }
    // Stopped where it gave its last value: what it was stepping closed (flatMap's inner iterator first).
    with(vm, g, |h| {
        h.state = RUNNING;
    });
    let closed = match inner {
        Some((inner, _)) => match vm.iterator_close(&inner) {
            Err(e) => Err(close_abrupt(vm, &it, e)),
            Ok(()) => vm.iterator_close(&it),
        },
        None => vm.iterator_close(&it),
    };
    with(vm, g, |h| {
        h.state = DONE;
        h.inner = None;
    });
    closed?;
    Ok(vm.iter_result(Value::Undefined, true))
}

#[cfg(test)]
mod tests {
    use crate::vm::Vm;
    use alloc::string::String;

    fn run(src: &str) -> String {
        let mut vm = Vm::new();
        match vm.run_script(src, "test") {
            Ok(v) => vm.to_string(&v).map(|s| s.to_rust()).unwrap_or_default(),
            Err(e) => alloc::format!("threw {}", vm.to_string(&e).map(|s| s.to_rust()).unwrap_or_default()),
        }
    }

    #[test]
    fn helpers_are_lazy_and_close_what_they_stop() {
        for (src, want) in [
            ("function* g() { yield 1; yield 2; yield 3; yield 4 } g().map(x => x * 10).filter(x => x > 10).take(2).toArray().join()", "20,30"),
            ("function* g() { yield 1; yield 2; yield 3 } g().drop(1).flatMap(x => [x, x]).toArray().join('')", "2233"),
            ("[1, 2, 3].values().reduce((a, b) => a + b) + ' ' + [].values().reduce((a, b) => a + b, 7)", "6 7"),
            ("var log = []; var it = { __proto__: Iterator.prototype, i: 0, next() { log.push('next'); return { done: this.i > 5, value: this.i++ } }, return() { log.push('return'); return {} } };
              var h = it.map(x => x); log.push('made'); h.next(); h.return(); log.join()", "made,next,return"),
            ("var closed = 0; var it = { __proto__: Iterator.prototype, next() { return { done: false, value: 1 } }, return() { closed++; return {} } };
              it.some(x => x === 1) + ' ' + it.find(x => true) + ' ' + closed", "true 1 2"),
            ("Iterator.from({ next() { return { done: true } } }).toArray().length + ' ' + (Iterator.from('ab').toArray().join('-'))", "0 a-b"),
            ("try { new Iterator() } catch (e) { e.name }", "TypeError"),
            ("class MyIt extends Iterator { next() { return { done: true } } } new MyIt() instanceof Iterator", "true"),
            ("try { [].values().take(-1) } catch (e) { e.name }", "RangeError"),
            ("Object.prototype.toString.call([].values().map(x => x)) + ' ' + Iterator.prototype[Symbol.toStringTag]", "[object Iterator Helper] Iterator"),
        ] {
            assert_eq!(run(src), want, "{src}");
        }
    }
}
