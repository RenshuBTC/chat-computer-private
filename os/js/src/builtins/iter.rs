//! Iterators: %IteratorPrototype%, array iterators, generators' and async generators' prototypes, and the
//! async-from-sync iterator (for await over a sync iterable).

use super::{arg, sym_method, tag};
use crate::names as n;
use crate::object::*;
use crate::value::{Gc, Value};
use crate::vm::{I, Sym, Vm};

type R = Result<Value, Value>;

pub fn init(vm: &mut Vm) {
    let ip = vm.proto(I::IteratorProto);
    sym_method(vm, ip, Sym::Iterator, "[Symbol.iterator]", 0, return_this);
    let aip = vm.proto(I::AsyncIteratorProto);
    sym_method(vm, aip, Sym::AsyncIterator, "[Symbol.asyncIterator]", 0, return_this);
    let ai = vm.proto(I::ArrayIterProto);
    let next = vm.method(ai, "next", 0, array_iter_next);
    vm.intrinsics[I::ArrayIterNext as usize] = next;
    tag(vm, ai, "Array Iterator");
    let gp = vm.proto(I::GeneratorProto);
    vm.method(gp, "next", 1, gen_next);
    vm.method(gp, "return", 1, gen_return);
    vm.method(gp, "throw", 1, gen_throw);
    tag(vm, gp, "Generator");
    let agp = vm.proto(I::AsyncGeneratorProto);
    vm.method(agp, "next", 1, async_gen_next);
    vm.method(agp, "return", 1, async_gen_return);
    vm.method(agp, "throw", 1, async_gen_throw);
    tag(vm, agp, "AsyncGenerator");
    let afs = vm.proto(I::AsyncFromSyncIterProto);
    vm.method(afs, "next", 1, afs_next);
    vm.method(afs, "return", 1, afs_return);
    vm.method(afs, "throw", 1, afs_throw);
}

pub fn return_this(_vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(this.clone())
}

fn array_iter_next(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let g = match this {
        Value::Obj(g) if matches!(vm.heap.get(*g).class, Class::ArrayIter(..)) => *g,
        _ => return Err(vm.type_error("next method called on incompatible receiver")),
    };
    match vm.array_iter_step(g)? {
        Some(v) => Ok(vm.iter_result(v, false)),
        None => Ok(vm.iter_result(Value::Undefined, true)),
    }
}

/// An array iterator over an object (0 keys, 1 values, 2 entries).
pub fn array_iterator(vm: &mut Vm, o: Value, kind: u8) -> Value {
    let p = vm.proto(I::ArrayIterProto);
    Value::Obj(vm.alloc(Some(p), Class::ArrayIter(o, 0, kind)))
}

fn gen_next(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    vm.generator_resume(this, 0, arg(args, 0))
}

fn gen_return(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    vm.generator_resume(this, 2, arg(args, 0))
}

fn gen_throw(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    vm.generator_resume(this, 1, arg(args, 0))
}

fn async_gen_next(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(vm.async_gen_enqueue(this, 0, arg(args, 0)))
}

fn async_gen_return(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(vm.async_gen_enqueue(this, 2, arg(args, 0)))
}

fn async_gen_throw(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(vm.async_gen_enqueue(this, 1, arg(args, 0)))
}

fn sync_of(vm: &mut Vm, this: &Value) -> Result<(Value, Value), Value> {
    if let Value::Obj(g) = this
        && let Class::AsyncFromSync(it, next) = &vm.heap.get(*g).class
    {
        return Ok((it.clone(), next.clone()));
    }
    Err(vm.type_error("not an async-from-sync iterator"))
}

/// AsyncFromSyncIteratorContinuation: the sync result's value awaited, then { value, done }.
fn continuation(vm: &mut Vm, result: Value, p: Gc, it: &Value, close_on_reject: bool) {
    let r = (|| -> Result<(), Value> {
        let Value::Obj(rg) = result else { return Err(vm.type_error("Iterator result is not an object")) };
        let done = vm.get(rg, Key::Str(n::DONE), &result)?.truthy();
        let value = vm.get(rg, Key::Str(n::VALUE), &result)?;
        let wrapper = match vm.promise_resolve(value) {
            Ok(w) => w,
            Err(e) => {
                if !done && close_on_reject {
                    let _ = vm.iterator_close(it);
                }
                return Err(e);
            }
        };
        let on_ful = vm.native_with("", 1, unwrap, None, Value::Bool(done));
        let on_rej = if done || !close_on_reject {
            Value::Undefined
        } else {
            let f = vm.native_with("", 1, close_and_rethrow, None, it.clone());
            Value::Obj(f)
        };
        vm.perform_then(wrapper, Value::Obj(on_ful), on_rej, Value::Obj(p), Value::Undefined, Value::Undefined);
        Ok(())
    })();
    if let Err(e) = r {
        vm.reject_promise(p, e);
    }
}

fn unwrap(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let done = vm.data(f).truthy();
    Ok(vm.iter_result(arg(args, 0), done))
}

fn close_and_rethrow(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let it = vm.data(f);
    let _ = vm.iterator_close(&it);
    Err(arg(args, 0))
}

fn afs_next(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.new_promise();
    let (it, next) = sync_of(vm, this)?;
    let r = if args.is_empty() { vm.call(&next, it.clone(), &[]) } else { vm.call(&next, it.clone(), &[arg(args, 0)]) };
    match r {
        Ok(r) => continuation(vm, r, p, &it, true),
        Err(e) => vm.reject_promise(p, e),
    }
    Ok(Value::Obj(p))
}

fn afs_return(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.new_promise();
    let (it, _) = sync_of(vm, this)?;
    let m = match vm.get_method(&it, Key::Str(n::RETURN)) {
        Ok(m) => m,
        Err(e) => {
            vm.reject_promise(p, e);
            return Ok(Value::Obj(p));
        }
    };
    if m.is_undefined() {
        let r = vm.iter_result(arg(args, 0), true);
        let _ = vm.resolve_promise(p, r);
        return Ok(Value::Obj(p));
    }
    let r = if args.is_empty() { vm.call(&m, it.clone(), &[]) } else { vm.call(&m, it.clone(), &[arg(args, 0)]) };
    match r {
        Ok(r) if !r.is_object() => {
            let e = vm.type_error("iterator.return() did not return an object");
            vm.reject_promise(p, e);
        }
        Ok(r) => continuation(vm, r, p, &it, false),
        Err(e) => vm.reject_promise(p, e),
    }
    Ok(Value::Obj(p))
}

fn afs_throw(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.new_promise();
    let (it, _) = sync_of(vm, this)?;
    let m = match vm.get_method(&it, Key::Str(n::THROW)) {
        Ok(m) => m,
        Err(e) => {
            vm.reject_promise(p, e);
            return Ok(Value::Obj(p));
        }
    };
    if m.is_undefined() {
        let _ = vm.iterator_close(&it);
        let e = vm.type_error("The iterator does not provide a 'throw' method");
        vm.reject_promise(p, e);
        return Ok(Value::Obj(p));
    }
    match vm.call(&m, it.clone(), &[arg(args, 0)]) {
        Ok(r) if !r.is_object() => {
            let e = vm.type_error("iterator.throw() did not return an object");
            vm.reject_promise(p, e);
        }
        Ok(r) => continuation(vm, r, p, &it, true),
        Err(e) => vm.reject_promise(p, e),
    }
    Ok(Value::Obj(p))
}
