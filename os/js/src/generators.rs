//! Generators, async functions and async generators (frames set aside and taken up again), promises (their
//! reactions run as jobs, after the script that made them), iteration, and for-in's enumeration.

use crate::code::Op;
use crate::names as n;
use crate::object::*;
use crate::value::{Gc, Value};
use crate::vm::{GEN_ASYNC, GEN_ASYNC_FN, GEN_SYNC, I, Job, Resume, Signal, Sym, Vm};
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

impl Vm {
    /// A call of a generator function (its generator made) or of an async function (its promise).
    pub fn call_special(&mut self, g: Gc, this: Value, args: &[Value]) -> Result<Value, Value> {
        let Class::Function(c) = &self.heap.get(g).class else { return Ok(Value::Undefined) };
        let code = c.code.clone();
        self.stack.push(Value::Obj(g));
        self.stack.push(this);
        self.stack.extend_from_slice(args);
        self.enter_closure(g, args.len(), Value::Undefined, false)?;
        let fi = self.frames.len() - 1;
        if code.is_generator {
            self.nesting += 1;
            let r = self.execute(fi, Resume::Normal);
            self.nesting -= 1;
            match r? {
                Signal::Start => {
                    let pv = self.get(g, crate::object::Key::Str(n::PROTOTYPE), &Value::Obj(g))?;
                    let proto = match pv {
                        Value::Obj(p) => p,
                        _ => self.proto(if code.is_async { I::AsyncGeneratorProto } else { I::GeneratorProto }),
                    };
                    let (frame, stack) = self.suspend();
                    let kind = if code.is_async { GEN_ASYNC } else { GEN_SYNC };
                    let obj = self.alloc(Some(proto), Class::Generator(Box::new(GenState { state: GenRun::Start, frame: Some(frame), stack, kind, promise: Value::Undefined, queue: Vec::new(), star: 0 })));
                    Ok(Value::Obj(obj))
                }
                Signal::Return(v) => Ok(v),
                _ => Ok(Value::Undefined),
            }
        } else {
            let promise = self.new_promise();
            let state = self.alloc(None, Class::Generator(Box::new(GenState { state: GenRun::Running, frame: None, stack: Vec::new(), kind: GEN_ASYNC_FN, promise: Value::Obj(promise), queue: Vec::new(), star: 0 })));
            self.frames[fi].owner = Some(state);
            self.nesting += 1;
            let r = self.execute(fi, Resume::Normal);
            self.nesting -= 1;
            self.async_after(state, r);
            Ok(Value::Obj(promise))
        }
    }

    fn gen_state(&mut self, g: Gc) -> Option<&mut GenState> {
        match &mut self.heap.get_mut(g).class {
            Class::Generator(gs) => Some(gs),
            _ => None,
        }
    }

    /// An async function's run that stopped: at an await (set aside until the promise settles), or done.
    fn async_after(&mut self, state: Gc, r: Result<Signal, Value>) {
        match r {
            Ok(Signal::Await(v)) => {
                let (f, s) = self.suspend();
                if let Some(gs) = self.gen_state(state) {
                    gs.frame = Some(f);
                    gs.stack = s;
                }
                if let Err(e) = self.await_on(state, v) {
                    self.async_continue(state, true, e);
                }
            }
            Ok(Signal::Return(v)) => {
                let p = self.gen_state(state).map(|gs| {
                    gs.state = GenRun::Done;
                    gs.promise.clone()
                });
                if let Some(Value::Obj(p)) = p {
                    let _ = self.resolve_promise(p, v);
                }
            }
            Err(e) => {
                let p = self.gen_state(state).map(|gs| {
                    gs.state = GenRun::Done;
                    gs.promise.clone()
                });
                if let Some(Value::Obj(p)) = p {
                    self.reject_promise(p, e);
                }
            }
            _ => {}
        }
    }

    /// An await's promise settled: the frame taken up again with its value (or its reason thrown).
    pub fn async_continue(&mut self, state: Gc, rejected: bool, v: Value) {
        let (frame, stack, kind, star) = match self.gen_state(state) {
            Some(gs) => (gs.frame.take(), core::mem::take(&mut gs.stack), gs.kind, core::mem::replace(&mut gs.star, 0)),
            None => return,
        };
        let Some(frame) = frame else { return };
        self.restore(frame, stack);
        let fi = self.frames.len() - 1;
        self.frames[fi].owner = Some(state);
        let resume = if star != 0 {
            self.stack.push(v);
            self.stack.push(Value::Num(if rejected { 5.0 } else { star as f64 }));
            Resume::Normal
        } else if rejected {
            Resume::Throw(v)
        } else {
            self.stack.push(v);
            Resume::Normal
        };
        self.nesting += 1;
        let r = self.execute(fi, resume);
        self.nesting -= 1;
        if kind == GEN_ASYNC {
            self.async_gen_after(state, r);
        } else {
            self.async_after(state, r);
        }
    }

    /// Waits for a value (as a promise): the state object is the reaction's handler.
    fn await_on(&mut self, state: Gc, v: Value) -> Result<(), Value> {
        let p = self.promise_resolve(v)?;
        self.perform_then(p, Value::Obj(state), Value::Obj(state), Value::Undefined, Value::Undefined, Value::Undefined);
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ generators

    /// A generator's next (0), throw (1) or return (2).
    pub fn generator_resume(&mut self, this: &Value, mode: u8, v: Value) -> Result<Value, Value> {
        let g = match this {
            Value::Obj(g) if matches!(&self.heap.get(*g).class, Class::Generator(gs) if gs.kind == GEN_SYNC) => *g,
            _ => return Err(self.type_error("next method called on incompatible receiver")),
        };
        let state = self.gen_state(g).map(|gs| gs.state).unwrap_or(GenRun::Done);
        match state {
            GenRun::Running => return Err(self.type_error("Generator is already running")),
            GenRun::Done => {
                return match mode {
                    0 => Ok(self.iter_result(Value::Undefined, true)),
                    1 => Err(v),
                    _ => Ok(self.iter_result(v, true)),
                };
            }
            GenRun::Start if mode != 0 => {
                if let Some(gs) = self.gen_state(g) {
                    gs.state = GenRun::Done;
                    gs.frame = None;
                    gs.stack = Vec::new();
                }
                return if mode == 1 { Err(v) } else { Ok(self.iter_result(v, true)) };
            }
            _ => {}
        }
        let started = state != GenRun::Start;
        let (frame, stack) = {
            let gs = self.gen_state(g).expect("a generator");
            gs.state = GenRun::Running;
            (gs.frame.take(), core::mem::take(&mut gs.stack))
        };
        let Some(frame) = frame else { return Ok(self.iter_result(Value::Undefined, true)) };
        self.restore(frame, stack);
        let fi = self.frames.len() - 1;
        self.frames[fi].owner = Some(g);
        let resume = self.resumption(fi, started, mode, v);
        self.nesting += 1;
        let r = self.execute(fi, resume);
        self.nesting -= 1;
        match r {
            Ok(Signal::Yield(y)) => {
                self.set_aside(g);
                Ok(self.iter_result(y, false))
            }
            Ok(Signal::YieldRaw(r)) => {
                self.set_aside(g);
                Ok(r)
            }
            Ok(Signal::Return(v)) => {
                self.finish_gen(g);
                Ok(self.iter_result(v, true))
            }
            Err(e) => {
                self.finish_gen(g);
                Err(e)
            }
            _ => {
                self.finish_gen(g);
                Ok(Value::Undefined)
            }
        }
    }

    /// How a generator's frame goes on: at a yield* the request goes to it; else a value, a throw or a return.
    fn resumption(&mut self, fi: usize, started: bool, mode: u8, v: Value) -> Resume {
        if !started {
            return Resume::Normal;
        }
        let f = &self.frames[fi];
        if f.code.ops.get(f.pc) == Some(&Op::YieldStar) {
            self.stack.push(v);
            self.stack.push(Value::Num(mode as f64));
            return Resume::Normal;
        }
        match mode {
            0 => {
                self.stack.push(v);
                Resume::Normal
            }
            1 => Resume::Throw(v),
            _ => Resume::Return(v),
        }
    }

    fn set_aside(&mut self, g: Gc) {
        let (f, s) = self.suspend();
        if let Some(gs) = self.gen_state(g) {
            gs.frame = Some(f);
            gs.stack = s;
            gs.state = GenRun::Suspended;
        }
    }

    fn finish_gen(&mut self, g: Gc) {
        if let Some(gs) = self.gen_state(g) {
            gs.state = GenRun::Done;
            gs.frame = None;
            gs.stack = Vec::new();
        }
    }

    /// yield*: the stack holds the inner iterator, its next, and the value and kind of request (0 next, 1 throw,
    /// 2 return; an async generator's 3 and 4: the inner result awaited, 5: the await rejected).
    pub fn yield_star(&mut self, fi: usize, depth: usize) -> Result<Option<Signal>, Value> {
        let mode = match self.stack.pop() {
            Some(Value::Num(m)) => m as u8,
            _ => 0,
        };
        let received = self.stack.pop().unwrap_or(Value::Undefined);
        let l = self.stack.len();
        let (iter, next) = (self.stack[l - 2].clone(), self.stack[l - 1].clone());
        let is_async = self.frames[fi].owner.is_some_and(|g| matches!(&self.heap.get(g).class, Class::Generator(gs) if gs.kind == GEN_ASYNC));
        let r = match mode {
            5 => return Err(received),
            3 | 4 => received,
            _ => {
                let r = match mode {
                    0 => self.call(&next, iter.clone(), &[received])?,
                    1 => {
                        let t = self.get_method(&iter, Key::Str(n::THROW))?;
                        if t.is_undefined() {
                            self.stack.truncate(l - 2);
                            self.iterator_close(&iter)?;
                            return Err(self.type_error("The iterator does not provide a 'throw' method"));
                        }
                        self.call(&t, iter.clone(), &[received])?
                    }
                    _ => {
                        let rm = self.get_method(&iter, Key::Str(n::RETURN))?;
                        if rm.is_undefined() {
                            self.stack.truncate(l - 2);
                            return self.do_return(received, depth);
                        }
                        self.call(&rm, iter.clone(), &[received])?
                    }
                };
                if is_async {
                    if let Some(g) = self.frames[fi].owner
                        && let Some(gs) = self.gen_state(g)
                    {
                        gs.star = if mode == 2 { 4 } else { 3 };
                    }
                    self.frames[fi].pc -= 1;
                    return Ok(Some(Signal::Await(r)));
                }
                r
            }
        };
        let Value::Obj(rg) = r else { return Err(self.type_error("Iterator result is not an object")) };
        let done = self.get(rg, Key::Str(n::DONE), &r)?.truthy();
        if done {
            let v = self.get(rg, Key::Str(n::VALUE), &r)?;
            self.stack.truncate(l - 2);
            if mode == 2 || mode == 4 {
                return self.do_return(v, depth);
            }
            self.stack.push(v);
            return Ok(None);
        }
        self.frames[fi].pc -= 1;
        if is_async {
            let v = self.get(rg, Key::Str(n::VALUE), &r)?;
            return Ok(Some(Signal::Yield(v)));
        }
        Ok(Some(Signal::YieldRaw(r)))
    }

    // ------------------------------------------------------------------------------------------ async generators

    /// An async generator's next (0), throw (1) or return (2): a promise of its result.
    pub fn async_gen_enqueue(&mut self, this: &Value, mode: u8, v: Value) -> Value {
        let p = self.new_promise();
        let g = match this {
            Value::Obj(g) if matches!(&self.heap.get(*g).class, Class::Generator(gs) if gs.kind == GEN_ASYNC) => *g,
            _ => {
                let e = self.type_error("not an async generator");
                self.reject_promise(p, e);
                return Value::Obj(p);
            }
        };
        let running = {
            let gs = self.gen_state(g).expect("a generator");
            gs.queue.push((mode, v, Value::Obj(p)));
            gs.state == GenRun::Running
        };
        if !running {
            self.async_gen_drain(g);
        }
        Value::Obj(p)
    }

    fn async_gen_drain(&mut self, g: Gc) {
        loop {
            let (state, front) = match self.gen_state(g) {
                Some(gs) => (gs.state, gs.queue.first().cloned()),
                None => return,
            };
            let Some((mode, v, p)) = front else { return };
            let state = if state == GenRun::Start && mode != 0 {
                self.finish_gen(g);
                GenRun::Done
            } else {
                state
            };
            match state {
                GenRun::Running => return,
                GenRun::Done => {
                    if let Some(gs) = self.gen_state(g) {
                        gs.queue.remove(0);
                    }
                    let Value::Obj(p) = p else { continue };
                    match mode {
                        0 => {
                            let r = self.iter_result(Value::Undefined, true);
                            let _ = self.resolve_promise(p, r);
                        }
                        1 => self.reject_promise(p, v),
                        _ => {
                            let r = self.iter_result(v, true);
                            let _ = self.resolve_promise(p, r);
                        }
                    }
                    continue;
                }
                _ => {}
            }
            let started = state != GenRun::Start;
            let (frame, stack) = {
                let gs = self.gen_state(g).expect("a generator");
                gs.state = GenRun::Running;
                (gs.frame.take(), core::mem::take(&mut gs.stack))
            };
            let Some(frame) = frame else { return };
            self.restore(frame, stack);
            let fi = self.frames.len() - 1;
            self.frames[fi].owner = Some(g);
            let resume = self.resumption(fi, started, mode, v);
            self.nesting += 1;
            let r = self.execute(fi, resume);
            self.nesting -= 1;
            self.async_gen_after(g, r);
            return;
        }
    }

    fn async_gen_after(&mut self, g: Gc, r: Result<Signal, Value>) {
        match r {
            Ok(Signal::Await(v)) => {
                let (f, s) = self.suspend();
                if let Some(gs) = self.gen_state(g) {
                    gs.frame = Some(f);
                    gs.stack = s;
                }
                if let Err(e) = self.await_on(g, v) {
                    self.async_continue(g, true, e);
                }
            }
            Ok(Signal::Yield(v)) | Ok(Signal::YieldRaw(v)) => {
                self.set_aside(g);
                let req = self.gen_state(g).and_then(|gs| if gs.queue.is_empty() { None } else { Some(gs.queue.remove(0)) });
                if let Some((_, _, Value::Obj(p))) = req {
                    let r = self.iter_result(v, false);
                    let _ = self.resolve_promise(p, r);
                }
                self.async_gen_drain(g);
            }
            Ok(Signal::Return(v)) => {
                self.finish_gen(g);
                let req = self.gen_state(g).and_then(|gs| if gs.queue.is_empty() { None } else { Some(gs.queue.remove(0)) });
                if let Some((_, _, Value::Obj(p))) = req {
                    let r = self.iter_result(v, true);
                    let _ = self.resolve_promise(p, r);
                }
                self.async_gen_drain(g);
            }
            Err(e) => {
                self.finish_gen(g);
                let req = self.gen_state(g).and_then(|gs| if gs.queue.is_empty() { None } else { Some(gs.queue.remove(0)) });
                if let Some((_, _, Value::Obj(p))) = req {
                    self.reject_promise(p, e);
                }
                self.async_gen_drain(g);
            }
            Ok(Signal::Start) => {}
        }
    }

    // ---------------------------------------------------------------------------------------------- promises

    pub fn new_promise(&mut self) -> Gc {
        let p = self.proto(I::PromiseProto);
        self.alloc(Some(p), Class::Promise(Box::new(PromiseData { state: PromiseState::Pending, reactions: Vec::new(), handled: false })))
    }

    pub fn is_promise(&self, v: &Value) -> bool {
        matches!(v, Value::Obj(g) if matches!(self.heap.get(*g).class, Class::Promise(_)))
    }

    /// A promise resolved with a value: fulfilled, or following it if it's a thenable.
    pub fn resolve_promise(&mut self, p: Gc, v: Value) -> Result<(), Value> {
        if let Value::Obj(g) = v
            && g == p
        {
            let e = self.type_error("Chaining cycle detected for promise");
            self.reject_promise(p, e);
            return Ok(());
        }
        let Value::Obj(g) = v else {
            self.fulfill_promise(p, v);
            return Ok(());
        };
        let then = match self.get(g, Key::Str(n::THEN), &v) {
            Ok(t) => t,
            Err(e) => {
                self.reject_promise(p, e);
                return Ok(());
            }
        };
        if !self.is_callable(&then) {
            self.fulfill_promise(p, v);
            return Ok(());
        }
        self.jobs.push_back(Job::Thenable { promise: p, thenable: v, then });
        Ok(())
    }

    pub fn fulfill_promise(&mut self, p: Gc, v: Value) {
        let reactions = match &mut self.heap.get_mut(p).class {
            Class::Promise(d) if matches!(d.state, PromiseState::Pending) => {
                d.state = PromiseState::Fulfilled(v.clone());
                core::mem::take(&mut d.reactions)
            }
            _ => return,
        };
        for (on_ful, _, derived, resolve, reject) in reactions {
            self.jobs.push_back(Job::Reaction { handler: on_ful, arg: v.clone(), rejected: false, derived, resolve, reject });
        }
    }

    pub fn reject_promise(&mut self, p: Gc, e: Value) {
        let (reactions, handled) = match &mut self.heap.get_mut(p).class {
            Class::Promise(d) if matches!(d.state, PromiseState::Pending) => {
                d.state = PromiseState::Rejected(e.clone());
                (core::mem::take(&mut d.reactions), d.handled)
            }
            _ => return,
        };
        if !handled {
            self.unhandled.push(p);
        }
        for (_, on_rej, derived, resolve, reject) in reactions {
            self.jobs.push_back(Job::Reaction { handler: on_rej, arg: e.clone(), rejected: true, derived, resolve, reject });
        }
    }

    /// PerformPromiseThen: reactions added (or queued, if it's settled).
    pub fn perform_then(&mut self, p: Gc, on_ful: Value, on_rej: Value, derived: Value, resolve: Value, reject: Value) {
        let keep = |vm: &Vm, h: Value| if vm.is_callable(&h) || matches!(&h, Value::Obj(g) if matches!(vm.heap.get(*g).class, Class::Generator(_))) { h } else { Value::Undefined };
        let on_ful = keep(self, on_ful);
        let on_rej = keep(self, on_rej);
        let (state, was_handled) = match &mut self.heap.get_mut(p).class {
            Class::Promise(d) => {
                let s = match &d.state {
                    PromiseState::Pending => None,
                    PromiseState::Fulfilled(v) => Some((v.clone(), false)),
                    PromiseState::Rejected(e) => Some((e.clone(), true)),
                };
                let h = d.handled;
                d.handled = true;
                if s.is_none() {
                    d.reactions.push((on_ful.clone(), on_rej.clone(), derived.clone(), resolve.clone(), reject.clone()));
                }
                (s, h)
            }
            _ => return,
        };
        match state {
            None => {}
            Some((v, false)) => self.jobs.push_back(Job::Reaction { handler: on_ful, arg: v, rejected: false, derived, resolve, reject }),
            Some((e, true)) => {
                if !was_handled {
                    self.unhandled.retain(|x| *x != p);
                }
                self.jobs.push_back(Job::Reaction { handler: on_rej, arg: e, rejected: true, derived, resolve, reject });
            }
        }
    }

    /// PromiseResolve(%Promise%, v): v if it's a promise of ours, else a new promise resolved with it.
    pub fn promise_resolve(&mut self, v: Value) -> Result<Gc, Value> {
        if let Value::Obj(g) = v
            && matches!(self.heap.get(g).class, Class::Promise(_))
        {
            let c = self.get(g, Key::Str(n::CONSTRUCTOR), &v)?;
            if matches!(c, Value::Obj(c) if c == self.proto(I::PromiseCtor)) {
                return Ok(g);
            }
        }
        let p = self.new_promise();
        self.resolve_promise(p, v)?;
        Ok(p)
    }

    /// A promise's resolve and reject functions (sharing whether either was called).
    pub fn resolving_functions(&mut self, p: Gc) -> (Value, Value) {
        let flag = self.alloc(None, Class::Boolean(false));
        let res = self.native_with("", 1, promise_resolve_fn, None, Value::Obj(p));
        self.set_data2(res, Value::Obj(flag));
        let rej = self.native_with("", 1, promise_reject_fn, None, Value::Obj(p));
        self.set_data2(rej, Value::Obj(flag));
        (Value::Obj(res), Value::Obj(rej))
    }

    /// Runs the jobs waiting (and those they queue), until there are none.
    pub fn run_jobs(&mut self) {
        while let Some(job) = self.jobs.pop_front() {
            match job {
                Job::Reaction { handler, arg, rejected, derived, resolve, reject } => {
                    if let Value::Obj(h) = handler
                        && matches!(self.heap.get(h).class, Class::Generator(_))
                    {
                        self.async_continue(h, rejected, arg);
                        continue;
                    }
                    let r = if self.is_callable(&handler) {
                        self.call(&handler, Value::Undefined, &[arg])
                    } else if rejected {
                        Err(arg)
                    } else {
                        Ok(arg)
                    };
                    if resolve.is_undefined() {
                        if let Value::Obj(d) = derived {
                            match r {
                                Ok(v) => {
                                    let _ = self.resolve_promise(d, v);
                                }
                                Err(e) => self.reject_promise(d, e),
                            }
                        }
                    } else {
                        let _ = match r {
                            Ok(v) => self.call(&resolve, Value::Undefined, &[v]),
                            Err(e) => self.call(&reject, Value::Undefined, &[e]),
                        };
                    }
                }
                Job::Thenable { promise, thenable, then } => {
                    let (res, rej) = self.resolving_functions(promise);
                    if let Err(e) = self.call(&then, thenable, &[res, rej.clone()]) {
                        let _ = self.call(&rej, Value::Undefined, &[e]);
                    }
                }
                Job::Call { f, args } => {
                    let _ = self.call(&f, Value::Undefined, &args);
                }
            }
        }
    }

    // ---------------------------------------------------------------------------------------------- iteration

    /// { value, done }.
    pub fn iter_result(&mut self, v: Value, done: bool) -> Value {
        let o = self.new_object();
        let props = &mut self.heap.get_mut(o).props;
        props.set(Key::Str(n::VALUE), Prop::data(v, ALL));
        props.set(Key::Str(n::DONE), Prop::data(Value::Bool(done), ALL));
        Value::Obj(o)
    }

    /// GetIterator: the iterator and its next method.
    pub fn get_iterator(&mut self, v: &Value, is_async: bool) -> Result<(Value, Value), Value> {
        if is_async {
            let ai = self.sym(Sym::AsyncIterator);
            let m = self.get_method(v, Key::Sym(ai))?;
            if m.is_undefined() {
                let (it, next) = self.get_iterator(v, false)?;
                let p = self.proto(I::AsyncFromSyncIterProto);
                let o = self.alloc(Some(p), Class::AsyncFromSync(it, next));
                let next = self.get(o, Key::Str(n::NEXT), &Value::Obj(o))?;
                return Ok((Value::Obj(o), next));
            }
            let it = self.call(&m, v.clone(), &[])?;
            if !it.is_object() {
                return Err(self.type_error("Result of the Symbol.asyncIterator method is not an object"));
            }
            let next = self.get_value(&it, Key::Str(n::NEXT))?;
            return Ok((it, next));
        }
        let si = self.sym(Sym::Iterator);
        let m = self.get_method(v, Key::Sym(si))?;
        if m.is_undefined() {
            return Err(self.type_error(&alloc::format!("{} is not iterable", self.describe(v))));
        }
        let it = self.call(&m, v.clone(), &[])?;
        if !it.is_object() {
            return Err(self.type_error("Result of the Symbol.iterator method is not an object"));
        }
        let next = self.get_value(&it, Key::Str(n::NEXT))?;
        Ok((it, next))
    }

    /// IteratorStep and IteratorValue: the next value, or None when it's done.
    pub fn iter_step(&mut self, it: &Value, next: &Value) -> Result<Option<Value>, Value> {
        if let (Value::Obj(ig), Value::Obj(nf)) = (it, next)
            && *nf == self.proto(I::ArrayIterNext)
            && matches!(self.heap.get(*ig).class, Class::ArrayIter(..))
        {
            return self.array_iter_step(*ig);
        }
        let r = self.call(next, it.clone(), &[])?;
        let Value::Obj(rg) = r else { return Err(self.type_error(&alloc::format!("Iterator result {} is not an object", self.describe(&r)))) };
        if self.get(rg, Key::Str(n::DONE), &r)?.truthy() {
            return Ok(None);
        }
        Ok(Some(self.get(rg, Key::Str(n::VALUE), &r)?))
    }

    /// An array iterator's next value (without its result object).
    pub fn array_iter_step(&mut self, ig: Gc) -> Result<Option<Value>, Value> {
        let Class::ArrayIter(obj, idx, kind) = &self.heap.get(ig).class else { return Ok(None) };
        let (obj, idx, kind) = (obj.clone(), *idx, *kind);
        let Value::Obj(a) = obj else { return Ok(None) };
        let len = match &self.heap.get(a).class {
            Class::Array(_, l) => *l as u64,
            Class::TypedArray(t) => {
                if self.typed_out_of_bounds(t) {
                    return Err(self.type_error("Cannot perform %ArrayIteratorPrototype%.next on a detached or out-of-bounds TypedArray"));
                }
                self.typed_length(t) as u64
            }
            _ => self.length_of(a)?,
        };
        if idx as u64 >= len {
            if let Class::ArrayIter(o, _, _) = &mut self.heap.get_mut(ig).class {
                *o = Value::Undefined;
            }
            return Ok(None);
        }
        if let Class::ArrayIter(_, i, _) = &mut self.heap.get_mut(ig).class {
            *i += 1;
        }
        Ok(Some(match kind {
            0 => Value::Num(idx as f64),
            1 => self.get(a, Key::Index(idx), &Value::Obj(a))?,
            _ => {
                let v = self.get(a, Key::Index(idx), &Value::Obj(a))?;
                Value::Obj(self.new_array(vec![Value::Num(idx as f64), v]))
            }
        }))
    }

    /// IteratorClose (a normal completion's: return()'s errors, and a result not an object, throw).
    pub fn iterator_close(&mut self, it: &Value) -> Result<(), Value> {
        if let Value::Obj(ig) = it
            && matches!(self.heap.get(*ig).class, Class::ArrayIter(..))
            && self.get_method(it, Key::Str(n::RETURN))?.is_undefined()
        {
            return Ok(());
        }
        let m = self.get_method(it, Key::Str(n::RETURN))?;
        if m.is_undefined() {
            return Ok(());
        }
        let r = self.call(&m, it.clone(), &[])?;
        if !r.is_object() {
            return Err(self.type_error("Iterator result is not an object"));
        }
        Ok(())
    }

    /// Whether iterating an array is the built-in way (so spreading it can copy its elements directly).
    pub fn plain_array_iteration(&self, g: Gc) -> bool {
        let o = self.heap.get(g);
        if !matches!(o.class, Class::Array(..)) || o.proto != Some(self.proto(I::ArrayProto)) {
            return false;
        }
        let it = Key::Sym(self.sym(Sym::Iterator));
        if o.props.get(&it).is_some() {
            return false;
        }
        let ap = self.heap.get(self.proto(I::ArrayProto));
        let ok1 = matches!(ap.props.get(&it), Some(Prop { slot: Slot::Data(Value::Obj(f)), .. }) if *f == self.proto(I::ArrayValues));
        let aip = self.heap.get(self.proto(I::ArrayIterProto));
        let ok2 = matches!(aip.props.get(&Key::Str(n::NEXT)), Some(Prop { slot: Slot::Data(Value::Obj(f)), .. }) if *f == self.proto(I::ArrayIterNext));
        ok1 && ok2
    }

    /// All an iterable's values.
    pub fn iterate_to_vec(&mut self, v: &Value) -> Result<Vec<Value>, Value> {
        if let Value::Obj(g) = v
            && self.plain_array_iteration(*g)
        {
            return Ok(self.array_values(v));
        }
        let (it, next) = self.get_iterator(v, false)?;
        let mut out = Vec::new();
        while let Some(x) = self.iter_step(&it, &next)? {
            out.push(x);
        }
        Ok(out)
    }

    // ----------------------------------------------------------------------------------------------- for-in

    /// for-in's enumeration: the enumerable string keys of an object and its prototypes (each name once).
    pub fn for_in_enumerator(&mut self, v: &Value) -> Result<Gc, Value> {
        let mut keys = Vec::new();
        let target = if v.is_nullish() { Value::Undefined } else { Value::Obj(self.to_object(v)?) };
        if let Value::Obj(start) = target {
            let mut seen: Vec<Key> = Vec::new();
            let mut cur = Some(start);
            while let Some(o) = cur {
                for k in self.own_keys(o)? {
                    if matches!(k, Key::Sym(_)) || seen.contains(&k) {
                        continue;
                    }
                    seen.push(k);
                    if let Some(p) = self.get_own_property(o, k)?
                        && p.enumerable()
                    {
                        keys.push(k);
                    }
                }
                cur = self.get_prototype_of(o)?;
            }
        }
        keys.reverse();
        Ok(self.alloc(None, Class::ForIn(keys, target)))
    }

    pub fn for_in_next(&mut self, e: Gc) -> Result<Option<Value>, Value> {
        loop {
            let (k, obj) = match &mut self.heap.get_mut(e).class {
                Class::ForIn(keys, obj) => match keys.pop() {
                    Some(k) => (k, obj.clone()),
                    None => return Ok(None),
                },
                _ => return Ok(None),
            };
            // (A property deleted before it's reached isn't visited.)
            if let Value::Obj(o) = obj
                && !self.has_property(o, k)?
            {
                continue;
            }
            return Ok(Some(self.key_value(k)));
        }
    }
}

fn promise_resolve_fn(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> Result<Value, Value> {
    let (Value::Obj(p), Value::Obj(flag)) = (vm.data(f), vm.data2(f)) else { return Ok(Value::Undefined) };
    if let Class::Boolean(done) = &mut vm.heap.get_mut(flag).class {
        if *done {
            return Ok(Value::Undefined);
        }
        *done = true;
    }
    let v = args.first().cloned().unwrap_or(Value::Undefined);
    vm.resolve_promise(p, v)?;
    Ok(Value::Undefined)
}

fn promise_reject_fn(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> Result<Value, Value> {
    let (Value::Obj(p), Value::Obj(flag)) = (vm.data(f), vm.data2(f)) else { return Ok(Value::Undefined) };
    if let Class::Boolean(done) = &mut vm.heap.get_mut(flag).class {
        if *done {
            return Ok(Value::Undefined);
        }
        *done = true;
    }
    let v = args.first().cloned().unwrap_or(Value::Undefined);
    vm.reject_promise(p, v);
    Ok(Value::Undefined)
}
