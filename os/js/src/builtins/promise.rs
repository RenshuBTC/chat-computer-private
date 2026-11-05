//! Promise: the constructor, then/catch/finally, and the combinators (all, allSettled, any, race), with the
//! capabilities the language uses so that subclasses of Promise work too.

use super::{arg, constructor, species_getter, sym_getter, tag};
use crate::names as n;
use crate::object::*;
use crate::value::{Gc, Value};
use crate::vm::{ErrorKind, I, Sym, Vm};
use alloc::vec::Vec;

type R = Result<Value, Value>;

pub fn init(vm: &mut Vm) {
    let pp = vm.proto(I::PromiseProto);
    let c = constructor(vm, "Promise", 1, promise_call, Some(promise_construct), Some(pp));
    vm.intrinsics[I::PromiseCtor as usize] = c;
    let then = vm.method(pp, "then", 2, promise_then);
    vm.intrinsics[I::PromiseThen as usize] = then;
    vm.method(pp, "catch", 1, promise_catch);
    vm.method(pp, "finally", 1, promise_finally);
    tag(vm, pp, "Promise");
    vm.method(c, "resolve", 1, promise_resolve_static);
    vm.method(c, "reject", 1, promise_reject_static);
    vm.method(c, "all", 1, promise_all);
    vm.method(c, "allSettled", 1, promise_all_settled);
    vm.method(c, "any", 1, promise_any);
    vm.method(c, "race", 1, promise_race);
    vm.method(c, "withResolvers", 0, promise_with_resolvers);
    sym_getter(vm, c, Sym::Species, "[Symbol.species]", species_getter);
}

fn promise_call(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Err(vm.type_error("Promise constructor cannot be invoked without 'new'"))
}

fn promise_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let ex = arg(args, 0);
    if !vm.is_callable(&ex) {
        return Err(vm.type_error("Promise resolver is not a function"));
    }
    let proto = vm.proto_from_constructor(nt, I::PromiseProto)?;
    let p = vm.new_promise();
    vm.heap.get_mut(p).proto = Some(proto);
    let (res, rej) = vm.resolving_functions(p);
    if let Err(e) = vm.call(&ex, Value::Undefined, &[res, rej.clone()]) {
        vm.call(&rej, Value::Undefined, &[e])?;
    }
    Ok(Value::Obj(p))
}

/// A promise capability: the promise, and its resolve and reject functions.
pub struct Capability {
    pub promise: Value,
    pub resolve: Value,
    pub reject: Value,
}

/// NewPromiseCapability(C).
pub fn new_capability(vm: &mut Vm, c: &Value) -> Result<Capability, Value> {
    if let Value::Obj(cg) = c
        && *cg == vm.proto(I::PromiseCtor)
    {
        let p = vm.new_promise();
        let (resolve, reject) = vm.resolving_functions(p);
        return Ok(Capability { promise: Value::Obj(p), resolve, reject });
    }
    if !vm.is_constructor(c) {
        return Err(vm.type_error("Promise capability: not a constructor"));
    }
    let record = vm.alloc(None, Class::Ordinary);
    let ex = vm.native_with("", 2, capability_executor, None, Value::Obj(record));
    let promise = vm.construct(c, &[Value::Obj(ex)], c)?;
    let resolve = vm.get(record, Key::Str(n::RESOLVE), &Value::Obj(record))?;
    let reject = vm.get(record, Key::Str(n::REJECT), &Value::Obj(record))?;
    if !vm.is_callable(&resolve) || !vm.is_callable(&reject) {
        return Err(vm.type_error("Promise resolve or reject function is not callable"));
    }
    Ok(Capability { promise, resolve, reject })
}

fn capability_executor(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let Value::Obj(record) = vm.data(f) else { return Ok(Value::Undefined) };
    for k in [n::RESOLVE, n::REJECT] {
        let cur = vm.heap.get(record).props.get(&Key::Str(k)).map(|p| p.slot.clone());
        if let Some(Slot::Data(v)) = cur
            && !v.is_undefined()
        {
            return Err(vm.type_error("Promise executor has already been invoked with non-undefined arguments"));
        }
    }
    vm.put_key(record, Key::Str(n::RESOLVE), arg(args, 0), ALL);
    vm.put_key(record, Key::Str(n::REJECT), arg(args, 1), ALL);
    Ok(Value::Undefined)
}

fn this_promise(vm: &mut Vm, this: &Value) -> Result<Gc, Value> {
    match this {
        Value::Obj(g) if matches!(vm.heap.get(*g).class, Class::Promise(_)) => Ok(*g),
        _ => Err(vm.type_error("Method Promise.prototype.then called on incompatible receiver")),
    }
}

fn promise_then(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = this_promise(vm, this)?;
    let c = vm.species_constructor(p, I::PromiseCtor)?;
    let (on_ful, on_rej) = (arg(args, 0), arg(args, 1));
    if let Value::Obj(cg) = c
        && cg == vm.proto(I::PromiseCtor)
    {
        // Our own: the derived promise settled directly.
        let d = vm.new_promise();
        vm.perform_then(p, on_ful, on_rej, Value::Obj(d), Value::Undefined, Value::Undefined);
        return Ok(Value::Obj(d));
    }
    let cap = new_capability(vm, &c)?;
    vm.perform_then(p, on_ful, on_rej, cap.promise.clone(), cap.resolve, cap.reject);
    Ok(cap.promise)
}

fn invoke_then(vm: &mut Vm, p: &Value, args: &[Value]) -> R {
    let then = vm.get_value(p, Key::Str(n::THEN))?;
    vm.call(&then, p.clone(), args)
}

fn promise_catch(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    invoke_then(vm, this, &[Value::Undefined, arg(args, 0)])
}

fn promise_finally(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(p) = this else { return Err(vm.type_error("Promise.prototype.finally called on a non-object")) };
    let c = vm.species_constructor(*p, I::PromiseCtor)?;
    let on_finally = arg(args, 0);
    if !vm.is_callable(&on_finally) {
        return invoke_then(vm, this, &[on_finally.clone(), on_finally]);
    }
    let record = vm.alloc(None, Class::Ordinary);
    vm.put_key(record, Key::Str(n::VALUE), on_finally, 0);
    vm.put_key(record, Key::Str(n::CONSTRUCTOR), c, 0);
    let then_f = vm.native_with("", 1, then_finally, None, Value::Obj(record));
    let catch_f = vm.native_with("", 1, catch_finally, None, Value::Obj(record));
    invoke_then(vm, this, &[Value::Obj(then_f), Value::Obj(catch_f)])
}

fn finally_parts(vm: &mut Vm, f: Gc) -> (Value, Value) {
    let Value::Obj(r) = vm.data(f) else { return (Value::Undefined, Value::Undefined) };
    let get = |vm: &Vm, k| match vm.heap.get(r).props.get(&Key::Str(k)) {
        Some(Prop { slot: Slot::Data(v), .. }) => v.clone(),
        _ => Value::Undefined,
    };
    (get(vm, n::VALUE), get(vm, n::CONSTRUCTOR))
}

fn then_finally(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let (on_finally, c) = finally_parts(vm, f);
    let result = vm.call(&on_finally, Value::Undefined, &[])?;
    let p = promise_resolve_with(vm, &c, result)?;
    let value_thunk = vm.native_with("", 0, thunk_return, None, arg(args, 0));
    invoke_then(vm, &p, &[Value::Obj(value_thunk)])
}

fn catch_finally(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let (on_finally, c) = finally_parts(vm, f);
    let result = vm.call(&on_finally, Value::Undefined, &[])?;
    let p = promise_resolve_with(vm, &c, result)?;
    let thrower = vm.native_with("", 0, thunk_throw, None, arg(args, 0));
    invoke_then(vm, &p, &[Value::Obj(thrower)])
}

fn thunk_return(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    Ok(vm.data(f))
}

fn thunk_throw(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    Err(vm.data(f))
}

/// PromiseResolve(C, x).
pub fn promise_resolve_with(vm: &mut Vm, c: &Value, x: Value) -> R {
    if vm.is_promise(&x) {
        let xc = vm.get_value(&x, Key::Str(n::CONSTRUCTOR))?;
        if crate::value::same_value(&xc, c) {
            return Ok(x);
        }
    }
    if let Value::Obj(cg) = c
        && *cg == vm.proto(I::PromiseCtor)
    {
        let p = vm.new_promise();
        vm.resolve_promise(p, x)?;
        return Ok(Value::Obj(p));
    }
    let cap = new_capability(vm, c)?;
    vm.call(&cap.resolve, Value::Undefined, &[x])?;
    Ok(cap.promise)
}

fn promise_resolve_static(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if !this.is_object() {
        return Err(vm.type_error("PromiseResolve called on non-object"));
    }
    promise_resolve_with(vm, this, arg(args, 0))
}

fn promise_reject_static(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let cap = new_capability(vm, this)?;
    vm.call(&cap.reject, Value::Undefined, &[arg(args, 0)])?;
    Ok(cap.promise)
}

fn promise_with_resolvers(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let cap = new_capability(vm, this)?;
    let o = vm.new_object();
    vm.put(o, "promise", cap.promise, ALL);
    vm.put(o, "resolve", cap.resolve, ALL);
    vm.put(o, "reject", cap.reject, ALL);
    Ok(Value::Obj(o))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Combinator {
    All,
    AllSettled,
    Any,
    Race,
}

/// The shared state of a combinator's element functions: (values, remaining count, the capability's functions).
fn record(vm: &mut Vm, values: Gc, cap: &Capability) -> Gc {
    let r = vm.alloc(None, Class::Ordinary);
    vm.put_key(r, Key::Str(n::VALUE), Value::Obj(values), WRITABLE);
    let rem = vm.atom("remaining");
    vm.put_key(r, Key::Str(rem), Value::Num(1.0), WRITABLE);
    vm.put_key(r, Key::Str(n::RESOLVE), cap.resolve.clone(), 0);
    vm.put_key(r, Key::Str(n::REJECT), cap.reject.clone(), 0);
    r
}

fn rec_get(vm: &Vm, r: Gc, k: crate::string::Atom) -> Value {
    match vm.heap.get(r).props.get(&Key::Str(k)) {
        Some(Prop { slot: Slot::Data(v), .. }) => v.clone(),
        _ => Value::Undefined,
    }
}

/// remaining += d; the new count.
fn remaining(vm: &mut Vm, r: Gc, d: f64) -> f64 {
    let rem = vm.atom("remaining");
    let cur = match rec_get(vm, r, rem) {
        Value::Num(x) => x,
        _ => 0.0,
    };
    vm.put_key(r, Key::Str(rem), Value::Num(cur + d), WRITABLE);
    cur + d
}

fn combinator(vm: &mut Vm, this: &Value, args: &[Value], kind: Combinator) -> R {
    let c = this.clone();
    let cap = new_capability(vm, &c)?;
    let reject_with = |vm: &mut Vm, cap: &Capability, e: Value| -> R {
        vm.call(&cap.reject, Value::Undefined, &[e])?;
        Ok(cap.promise.clone())
    };
    let resolve_fn = match vm.get_value(&c, Key::Str(n::RESOLVE)) {
        Ok(f) if vm.is_callable(&f) => f,
        Ok(_) => {
            let e = vm.type_error("Promise resolve is not a function");
            return reject_with(vm, &cap, e);
        }
        Err(e) => return reject_with(vm, &cap, e),
    };
    let (it, next) = match vm.get_iterator(&arg(args, 0), false) {
        Ok(x) => x,
        Err(e) => return reject_with(vm, &cap, e),
    };
    let values = vm.new_array(Vec::new());
    let rec = record(vm, values, &cap);
    let mut index = 0usize;
    loop {
        let v = match vm.iter_step(&it, &next) {
            Ok(Some(v)) => v,
            Ok(None) => break,
            Err(e) => return reject_with(vm, &cap, e),
        };
        let step = (|| -> Result<(), Value> {
            if kind != Combinator::Race {
                vm.create_data_property(values, Key::Index(index as u32), Value::Undefined)?;
            }
            let next_promise = vm.call(&resolve_fn, c.clone(), &[v])?;
            let (on_ful, on_rej) = match kind {
                Combinator::Race => (cap.resolve.clone(), cap.reject.clone()),
                _ => {
                    let slot = vm.alloc(None, Class::Ordinary);
                    vm.put_key(slot, Key::Str(n::INDEX), Value::Num(index as f64), 0);
                    vm.put_key(slot, Key::Str(n::DONE), Value::Bool(false), WRITABLE);
                    vm.put_key(slot, Key::Str(n::VALUE), Value::Obj(rec), 0);
                    remaining(vm, rec, 1.0);
                    let mk = |vm: &mut Vm, f: NativeFn| Value::Obj(vm.native_with("", 1, f, None, Value::Obj(slot)));
                    match kind {
                        Combinator::All => (mk(vm, all_resolve_element), cap.reject.clone()),
                        Combinator::AllSettled => (mk(vm, settled_fulfilled), mk(vm, settled_rejected)),
                        _ => (cap.resolve.clone(), mk(vm, any_reject_element)),
                    }
                }
            };
            invoke_then(vm, &next_promise, &[on_ful, on_rej])?;
            Ok(())
        })();
        if let Err(e) = step {
            let _ = vm.iterator_close(&it);
            return reject_with(vm, &cap, e);
        }
        index += 1;
    }
    if kind != Combinator::Race && remaining(vm, rec, -1.0) == 0.0 {
        finish(vm, rec, kind == Combinator::Any)?;
    }
    Ok(cap.promise)
}

/// All the elements are in: the combinator's promise resolved (all) or rejected (any).
fn finish(vm: &mut Vm, rec: Gc, any: bool) -> Result<(), Value> {
    let values = rec_get(vm, rec, n::VALUE);
    if any {
        let e = vm.make_error(ErrorKind::Aggregate, "All promises were rejected");
        if let Value::Obj(eg) = e {
            vm.put_key(eg, Key::Str(n::ERRORS), values, HIDDEN);
        }
        let reject = rec_get(vm, rec, n::REJECT);
        vm.call(&reject, Value::Undefined, &[e])?;
    } else {
        let resolve = rec_get(vm, rec, n::RESOLVE);
        vm.call(&resolve, Value::Undefined, &[values])?;
    }
    Ok(())
}

/// An element function's slot: its index and the shared record (None if it was already called).
fn element(vm: &mut Vm, f: Gc) -> Option<(u32, Gc)> {
    let Value::Obj(slot) = vm.data(f) else { return None };
    if rec_get(vm, slot, n::DONE).truthy() {
        return None;
    }
    vm.put_key(slot, Key::Str(n::DONE), Value::Bool(true), WRITABLE);
    let i = match rec_get(vm, slot, n::INDEX) {
        Value::Num(x) => x as u32,
        _ => 0,
    };
    let Value::Obj(rec) = rec_get(vm, slot, n::VALUE) else { return None };
    Some((i, rec))
}

fn store_element(vm: &mut Vm, f: Gc, v: Value, any: bool) -> R {
    let Some((i, rec)) = element(vm, f) else { return Ok(Value::Undefined) };
    if let Value::Obj(values) = rec_get(vm, rec, n::VALUE) {
        vm.create_data_property(values, Key::Index(i), v)?;
    }
    if remaining(vm, rec, -1.0) == 0.0 {
        finish(vm, rec, any)?;
    }
    Ok(Value::Undefined)
}

fn all_resolve_element(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    store_element(vm, f, arg(args, 0), false)
}

fn any_reject_element(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    store_element(vm, f, arg(args, 0), true)
}

fn settled(vm: &mut Vm, f: Gc, v: Value, ok: bool) -> R {
    // (A fulfilled and a rejected function share one slot: whichever comes first counts.)
    let o = vm.new_object();
    vm.put_key(o, Key::Str(n::STATUS), crate::builtins::js_str(if ok { "fulfilled" } else { "rejected" }), ALL);
    vm.put_key(o, Key::Str(if ok { n::VALUE } else { n::REASON }), v, ALL);
    store_element(vm, f, Value::Obj(o), false)
}

fn settled_fulfilled(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    settled(vm, f, arg(args, 0), true)
}

fn settled_rejected(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    settled(vm, f, arg(args, 0), false)
}

fn promise_all(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    combinator(vm, this, args, Combinator::All)
}

fn promise_all_settled(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    combinator(vm, this, args, Combinator::AllSettled)
}

fn promise_any(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    combinator(vm, this, args, Combinator::Any)
}

fn promise_race(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    combinator(vm, this, args, Combinator::Race)
}
