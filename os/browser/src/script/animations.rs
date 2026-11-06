//! The Web Animations API, for scripts: element.animate() (its keyframes, as a list of keyframes or an object of
//! lists, and its timing), each animation's object (currentTime, playState, playbackRate, startTime, effect; play,
//! pause, cancel, finish, reverse, commitStyles, updatePlaybackRate; its ready and finished promises, and its finish
//! and cancel events), getAnimations() on elements and the document, and document.timeline. The page's styles apply
//! them (style.rs's ScriptedAnimation), over CSS's own animations.

use super::*;
use crate::css::{self, Decl};
use js::object::ALL;

/// An animation a script made: its element, its keyframes (offset, its properties' CSS names and values, its
/// easing), its timing (ms; iterations; direction 0 normal, 1 reverse, 2 alternate, 3 alternate-reverse; fill 0
/// none, 1 forwards, 2 backwards, 3 both; its easing), and where it is: its start on the page's clock (ms), or its
/// current time where it's paused or finished; its speed; its state.
pub struct ScriptAnimation {
    pub node: NodeId,
    pub frames: Vec<(f32, Vec<(String, String)>, Option<String>)>,
    pub duration: f64,
    pub delay: f64,
    pub end_delay: f64,
    pub iterations: f64,
    pub direction: u8,
    pub fill: u8,
    pub easing: String,
    pub start: f64,
    pub held: Option<f64>,
    pub rate: f64,
    pub state: State,
    pub object: Gc,
    pub finished: Gc,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Running,
    Paused,
    Finished,
    Idle,
}

impl ScriptAnimation {
    /// Its current time (ms) at the page's time `now`.
    pub fn current(&self, now: f64) -> f64 {
        self.held.unwrap_or((now - self.start) * self.rate)
    }

    /// When it ends (ms of its own time): its delay, its iterations and its end delay.
    pub fn end(&self) -> f64 {
        (self.delay + self.duration * self.iterations + self.end_delay).max(0.0)
    }

    fn timing(&self) -> crate::animate::Timing {
        crate::animate::Timing {
            duration: (self.duration / 1000.0) as f32,
            delay: (self.delay / 1000.0) as f32,
            count: self.iterations as f32,
            direction: self.direction,
            fill: self.fill,
            paused: false,
        }
    }

    /// Where it has its element in its keyframes at the page's time (its own easing applied), or None.
    pub fn progress(&self, now: f64) -> Option<f32> {
        if self.state == State::Idle {
            return None;
        }
        let p = self.timing().progress((self.current(now) / 1000.0) as f32)?;
        let e = crate::animate::Easing::parse(&css::tokens(&self.easing)).unwrap_or(crate::animate::Easing::Linear);
        Some(e.at(p))
    }

    /// Its keyframes as style.rs takes them.
    pub fn keyframes(&self) -> css::Keyframes {
        let frames = self
            .frames
            .iter()
            .map(|(offset, props, easing)| {
                let mut decls: Vec<Decl> = props.iter().map(|(n, v)| Decl { name: n.clone(), value: css::tokens(v), important: false }).collect();
                if let Some(e) = easing {
                    decls.push(Decl { name: String::from("animation-timing-function"), value: css::tokens(e), important: false });
                }
                (alloc::vec![*offset], decls)
            })
            .collect();
        css::Keyframes { name: String::new(), frames }
    }

    /// Whether it's changing as time goes on.
    pub fn running(&self, now: f64) -> bool {
        self.state == State::Running && (self.iterations.is_infinite() || self.current(now) < self.end())
    }
}

/// The interfaces: Animation (an event target), and document.timeline.
pub fn install(vm: &mut Vm) {
    let ap = interface(vm, "Animation", Some("EventTarget"), 0, None);
    for (name, get, set) in [
        ("currentTime", current_time as NativeFn, Some(set_current_time as NativeFn)),
        ("playState", play_state, None),
        ("playbackRate", playback_rate, Some(update_playback_rate as NativeFn)),
        ("startTime", start_time, None),
        ("pending", false_value, None),
        ("replaceState", replace_state, None),
    ] {
        attribute(vm, ap, name, get, set);
    }
    for (name, f) in [
        ("play", play as NativeFn),
        ("pause", pause),
        ("cancel", cancel),
        ("finish", finish),
        ("reverse", reverse),
        ("commitStyles", commit_styles),
        ("persist", ignore),
        ("updatePlaybackRate", update_playback_rate),
    ] {
        method(vm, ap, name, 0, f);
    }
    events::handler_property(vm, ap, "finish");
    events::handler_property(vm, ap, "cancel");
    events::handler_property(vm, ap, "remove");
    if let Some(dp) = host_ref(vm).proto("Document") {
        attribute(vm, dp, "timeline", timeline, None);
        method(vm, dp, "getAnimations", 0, document_animations);
    }
}

fn no_keyframes(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Obj(vm.new_array(Vec::new())))
}

fn false_value(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(false))
}

fn ignore(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Undefined)
}

fn replace_state(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js("active"))
}

fn timeline(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let t = vm.new_object();
    let now = host_ref(vm).now;
    vm.put(t, "currentTime", Value::Num(now), ALL);
    method(vm, t, "getAnimations", 0, document_animations);
    Ok(Value::Obj(t))
}

/// A property's name as CSS writes it, from the name a script's keyframe gives it ("backgroundColor").
fn css_name(js_name: &str) -> String {
    if js_name == "cssFloat" {
        return String::from("float");
    }
    if js_name.starts_with("--") {
        return String::from(js_name);
    }
    let mut out = String::new();
    for c in js_name.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn value_text(vm: &mut Vm, v: &Value) -> Result<String, Value> {
    match v {
        Value::Num(n) => Ok(alloc::format!("{n}")),
        _ => string(vm, v),
    }
}

/// An object's own enumerable property names and values.
fn entries(vm: &mut Vm, o: Gc) -> Result<Vec<(String, Value)>, Value> {
    let mut out = Vec::new();
    for k in vm.own_keys(o)? {
        if let Key::Str(_) = k {
            let name = vm.key_name(k, "").to_rust();
            let v = vm.get(o, k, &Value::Obj(o))?;
            out.push((name, v));
        }
    }
    Ok(out)
}

/// Keyframes as a script gives them: a list of keyframes (each its properties, perhaps its offset and easing), or an
/// object of lists (each property's values, spread evenly); offsets not given spread between those that are.
fn read_keyframes(vm: &mut Vm, v: &Value) -> Result<Vec<(f32, Vec<(String, String)>, Option<String>)>, Value> {
    let mut frames: Vec<(Option<f32>, Vec<(String, String)>, Option<String>)> = Vec::new();
    match v {
        Value::Obj(o) if vm.is_array(v)? => {
            let _ = o;
            for k in vm.iterate_to_vec(v)? {
                let Value::Obj(kf) = k else { continue };
                let mut offset = None;
                let mut easing = None;
                let mut props = Vec::new();
                for (name, val) in entries(vm, kf)? {
                    match name.as_str() {
                        "offset" => offset = match val {
                            Value::Null | Value::Undefined => None,
                            _ => Some(vm.to_number(&val)? as f32),
                        },
                        "easing" => easing = Some(value_text(vm, &val)?),
                        "composite" => {}
                        _ => props.push((css_name(&name), value_text(vm, &val)?)),
                    }
                }
                frames.push((offset, props, easing));
            }
        }
        Value::Obj(o) => {
            // { opacity: [0, 1], transform: [...], easing: "ease-out" }
            let mut lists: Vec<(String, Vec<String>)> = Vec::new();
            let mut offsets: Vec<f32> = Vec::new();
            let mut easing = None;
            for (name, val) in entries(vm, *o)? {
                let values: Vec<Value> = if vm.is_array(&val)? { vm.iterate_to_vec(&val)? } else { alloc::vec![val] };
                match name.as_str() {
                    "offset" => {
                        for x in values {
                            offsets.push(vm.to_number(&x)? as f32);
                        }
                    }
                    "easing" => easing = values.first().map(|x| value_text(vm, x)).transpose()?,
                    "composite" => {}
                    _ => {
                        let mut texts = Vec::new();
                        for x in values {
                            texts.push(value_text(vm, &x)?);
                        }
                        lists.push((css_name(&name), texts));
                    }
                }
            }
            let n = lists.iter().map(|(_, l)| l.len()).max().unwrap_or(0);
            for i in 0..n {
                let mut props = Vec::new();
                for (name, l) in &lists {
                    // (A property's values spread over the keyframes as their own count says.)
                    if l.len() == n {
                        props.push((name.clone(), l[i].clone()));
                    } else if l.len() == 1 && i == n - 1 {
                        props.push((name.clone(), l[0].clone()));
                    } else if l.len() > 1 && n > 1 && (i * (l.len() - 1)) % (n - 1) == 0 {
                        props.push((name.clone(), l[i * (l.len() - 1) / (n - 1)].clone()));
                    }
                }
                frames.push((offsets.get(i).copied(), props, easing.clone()));
            }
        }
        _ => {}
    }
    // Offsets: the first 0 and the last 1 where they aren't given, those between spread evenly.
    let n = frames.len();
    if n == 1 {
        frames[0].0.get_or_insert(1.0);
    } else if n > 1 {
        frames[0].0.get_or_insert(0.0);
        frames[n - 1].0.get_or_insert(1.0);
    }
    let mut i = 0;
    while i < n {
        if frames[i].0.is_some() {
            i += 1;
            continue;
        }
        let start = i - 1;
        let mut end = i;
        while frames[end].0.is_none() {
            end += 1;
        }
        let (a, b) = (frames[start].0.unwrap_or(0.0), frames[end].0.unwrap_or(1.0));
        for (k, f) in frames.iter_mut().enumerate().take(end).skip(i) {
            f.0 = Some(a + (b - a) * (k - start) as f32 / (end - start) as f32);
        }
        i = end;
    }
    Ok(frames.into_iter().map(|(o, p, e)| (o.unwrap_or(0.0).clamp(0.0, 1.0), p, e)).collect())
}

/// element.animate(keyframes, options): the animation, running from now.
pub fn animate(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let node = this_element(vm, this)?;
    let frames = read_keyframes(vm, &arg(args, 0))?;
    let (mut duration, mut delay, mut end_delay, mut iterations, mut direction, mut fill, mut easing) = (0.0, 0.0, 0.0, 1.0, 0u8, 0u8, String::from("linear"));
    match arg(args, 1) {
        Value::Num(n) => duration = n,
        Value::Obj(o) => {
            for (name, v) in entries(vm, o)? {
                match name.as_str() {
                    "duration" => duration = if let Value::Str(_) = v { 0.0 } else { vm.to_number(&v)? },
                    "delay" => delay = vm.to_number(&v)?,
                    "endDelay" => end_delay = vm.to_number(&v)?,
                    "iterations" => iterations = vm.to_number(&v)?,
                    "direction" => direction = match string(vm, &v)?.as_str() {
                        "reverse" => 1,
                        "alternate" => 2,
                        "alternate-reverse" => 3,
                        _ => 0,
                    },
                    "fill" => fill = match string(vm, &v)?.as_str() {
                        "forwards" => 1,
                        "backwards" => 2,
                        "both" => 3,
                        _ => 0,
                    },
                    "easing" => easing = string(vm, &v)?,
                    _ => {}
                }
            }
        }
        _ => {}
    }
    let duration = if duration.is_finite() { duration.max(0.0) } else { 0.0 };
    let iterations = if iterations.is_nan() { 1.0 } else { iterations.max(0.0) };
    let proto = host_ref(vm).proto("Animation").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let a = vm.alloc(Some(proto), Class::Ordinary);
    let ready = vm.new_promise();
    vm.fulfill_promise(ready, Value::Obj(a));
    let finished = vm.new_promise();
    vm.put(a, "ready", Value::Obj(ready), ALL);
    vm.put(a, "finished", Value::Obj(finished), ALL);
    vm.put(a, "id", js(""), ALL);
    let effect = vm.new_object();
    let target = wrap(vm, node);
    vm.put(effect, "target", target, ALL);
    method(vm, effect, "getTiming", 0, get_timing);
    method(vm, effect, "getComputedTiming", 0, get_timing);
    method(vm, effect, "getKeyframes", 0, no_keyframes);
    vm.put(a, "effect", Value::Obj(effect), ALL);
    let now = host_ref(vm).now;
    let h = host(vm);
    let index = h.animations.len();
    h.animations.push(ScriptAnimation {
        node,
        frames,
        duration,
        delay,
        end_delay,
        iterations,
        direction,
        fill,
        easing,
        start: now,
        held: None,
        rate: 1.0,
        state: State::Running,
        object: a,
        finished,
    });
    set_slot(vm, a, host_ref(vm).syms.data, Value::Num(index as f64));
    set_slot(vm, effect, host_ref(vm).syms.data, Value::Num(index as f64));
    host(vm).changed();
    schedule(vm, index);
    Ok(Value::Obj(a))
}

/// The animation an object is (its slot's number).
fn index_of(vm: &Vm, this: &Value) -> Option<usize> {
    match slot(vm, this, host_ref(vm).syms.data) {
        Value::Num(n) if (n as usize) < host_ref(vm).animations.len() => Some(n as usize),
        _ => None,
    }
}

/// A timer for when a running animation ends (on the page's clock): it's then finished.
fn schedule(vm: &mut Vm, index: usize) {
    let a = &host_ref(vm).animations[index];
    if a.state != State::Running || a.rate <= 0.0 || a.iterations.is_infinite() {
        return;
    }
    let due = a.start + a.end() / a.rate;
    let f = vm.native_with("finish", 0, finished_timer, None, Value::Num(index as f64));
    let h = host(vm);
    let id = h.next_id();
    h.timers.push(Timer { id, due: due.max(h.now), every: None, f: Value::Obj(f), args: Vec::new(), depth: 0 });
}

fn finished_timer(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    let Value::Num(n) = vm.data(f) else { return Ok(Value::Undefined) };
    let i = n as usize;
    let now = host_ref(vm).now;
    let Some(a) = host_ref(vm).animations.get(i) else { return Ok(Value::Undefined) };
    if a.state == State::Running && a.current(now) >= a.end() - 0.5 {
        settle(vm, i);
    }
    Ok(Value::Undefined)
}

/// An animation finished: held at its end, its finished promise kept, its finish event fired.
fn settle(vm: &mut Vm, i: usize) {
    let (obj, finished) = {
        let a = &mut host(vm).animations[i];
        a.held = Some(a.end());
        a.state = State::Finished;
        (a.object, a.finished)
    };
    vm.fulfill_promise(finished, Value::Obj(obj));
    host(vm).changed();
    events::fire(vm, &Value::Obj(obj), &events::EventInit::new("finish", "Event", false, false));
}

fn get_timing(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let t = vm.new_object();
    if let Some(i) = index_of(vm, this) {
        let now = host_ref(vm).now;
        let a = &host_ref(vm).animations[i];
        let (duration, delay, iterations, current, progress) = (a.duration, a.delay, a.iterations, a.current(now), a.progress(now));
        vm.put(t, "duration", Value::Num(duration), ALL);
        vm.put(t, "delay", Value::Num(delay), ALL);
        vm.put(t, "iterations", Value::Num(iterations), ALL);
        vm.put(t, "localTime", Value::Num(current), ALL);
        vm.put(t, "progress", progress.map_or(Value::Null, |p| Value::Num(p as f64)), ALL);
    }
    Ok(Value::Obj(t))
}

fn current_time(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Some(i) = index_of(vm, this) else { return Ok(Value::Null) };
    let a = &host_ref(vm).animations[i];
    if a.state == State::Idle {
        return Ok(Value::Null);
    }
    Ok(Value::Num(a.current(host_ref(vm).now)))
}

fn set_current_time(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Some(i) = index_of(vm, this) else { return Ok(Value::Undefined) };
    let t = vm.to_number(&arg(args, 0))?;
    if !t.is_finite() {
        return Ok(Value::Undefined);
    }
    let now = host_ref(vm).now;
    {
        let a = &mut host(vm).animations[i];
        match a.state {
            State::Paused | State::Finished | State::Idle => {
                a.held = Some(t);
                if a.state != State::Paused {
                    a.state = State::Paused;
                }
            }
            State::Running => a.start = now - t / a.rate,
        }
    }
    host(vm).changed();
    if host_ref(vm).animations[i].state == State::Running {
        let a = &host_ref(vm).animations[i];
        if a.current(now) >= a.end() && !a.iterations.is_infinite() {
            settle(vm, i);
        } else {
            schedule(vm, i);
        }
    }
    Ok(Value::Undefined)
}

fn play_state(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Some(i) = index_of(vm, this) else { return Ok(js("idle")) };
    Ok(js(match host_ref(vm).animations[i].state {
        State::Running => "running",
        State::Paused => "paused",
        State::Finished => "finished",
        State::Idle => "idle",
    }))
}

fn playback_rate(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Some(i) = index_of(vm, this) else { return Ok(Value::Num(1.0)) };
    Ok(Value::Num(host_ref(vm).animations[i].rate))
}

fn update_playback_rate(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Some(i) = index_of(vm, this) else { return Ok(Value::Undefined) };
    let r = vm.to_number(&arg(args, 0))?;
    if !r.is_finite() {
        return Ok(Value::Undefined);
    }
    let now = host_ref(vm).now;
    let a = &mut host(vm).animations[i];
    let current = a.current(now);
    a.rate = r;
    if a.held.is_none() && r != 0.0 {
        a.start = now - current / r;
    }
    schedule(vm, i);
    Ok(Value::Undefined)
}

fn start_time(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Some(i) = index_of(vm, this) else { return Ok(Value::Null) };
    let a = &host_ref(vm).animations[i];
    Ok(if a.held.is_some() { Value::Null } else { Value::Num(a.start) })
}

fn play(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Some(i) = index_of(vm, this) else { return Ok(Value::Undefined) };
    let now = host_ref(vm).now;
    {
        let a = &mut host(vm).animations[i];
        let mut t = a.held.take().unwrap_or_else(|| (now - a.start) * a.rate);
        // (Played again after it's over, or never started: from its start.)
        if a.state == State::Idle || (a.state == State::Finished && t >= a.end()) {
            t = 0.0;
        }
        a.start = now - t / if a.rate != 0.0 { a.rate } else { 1.0 };
        a.state = State::Running;
    }
    host(vm).changed();
    schedule(vm, i);
    Ok(Value::Undefined)
}

fn pause(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Some(i) = index_of(vm, this) else { return Ok(Value::Undefined) };
    let now = host_ref(vm).now;
    let a = &mut host(vm).animations[i];
    if a.state != State::Paused {
        a.held = Some(if a.state == State::Idle { 0.0 } else { a.current(now) });
        a.state = State::Paused;
    }
    host(vm).changed();
    Ok(Value::Undefined)
}

fn cancel(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Some(i) = index_of(vm, this) else { return Ok(Value::Undefined) };
    let obj = {
        let a = &mut host(vm).animations[i];
        if a.state == State::Idle {
            return Ok(Value::Undefined);
        }
        a.state = State::Idle;
        a.held = None;
        a.object
    };
    host(vm).changed();
    events::fire(vm, &Value::Obj(obj), &events::EventInit::new("cancel", "Event", false, false));
    Ok(Value::Undefined)
}

fn finish(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Some(i) = index_of(vm, this) else { return Ok(Value::Undefined) };
    if host_ref(vm).animations[i].iterations.is_infinite() {
        return Err(dom_exception(vm, "InvalidStateError", "An animation that goes on for ever can't be finished."));
    }
    settle(vm, i);
    Ok(Value::Undefined)
}

fn reverse(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Some(i) = index_of(vm, this) else { return Ok(Value::Undefined) };
    let now = host_ref(vm).now;
    let a = &mut host(vm).animations[i];
    let current = a.current(now);
    a.rate = -a.rate;
    if a.held.is_none() && a.rate != 0.0 {
        a.start = now - current / a.rate;
    }
    host(vm).changed();
    Ok(Value::Undefined)
}

/// commitStyles(): its values now written into the element's style attribute.
fn commit_styles(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Some(i) = index_of(vm, this) else { return Ok(Value::Undefined) };
    let now = host_ref(vm).now;
    let (node, at, frames) = {
        let a = &host_ref(vm).animations[i];
        (a.node, a.progress(now), a.frames.clone())
    };
    let Some(at) = at else { return Ok(Value::Undefined) };
    let mut names: Vec<String> = Vec::new();
    for (_, props, _) in &frames {
        for (n, _) in props {
            if !names.contains(n) {
                names.push(n.clone());
            }
        }
    }
    let mut set: Vec<(String, String)> = Vec::new();
    for name in names {
        let mut f: Vec<(f32, &str)> = frames.iter().filter_map(|(o, props, _)| props.iter().find(|(n, _)| *n == name).map(|(_, v)| (*o, v.as_str()))).collect();
        f.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
        let before = f.iter().rposition(|x| x.0 <= at);
        let after = f.iter().position(|x| x.0 > at);
        let value = match (before, after) {
            (Some(b), Some(a)) => {
                let (o0, o1) = (f[b].0, f[a].0);
                let q = if o1 > o0 { (at - o0) / (o1 - o0) } else { 0.0 };
                crate::animate::interpolate(&css::tokens(f[b].1), &css::tokens(f[a].1), q).map(|t| css::serialize(&t)).unwrap_or_else(|| String::from(if q < 0.5 { f[b].1 } else { f[a].1 }))
            }
            (Some(b), None) => String::from(f[b].1),
            (None, Some(a)) => String::from(f[a].1),
            (None, None) => continue,
        };
        set.push((name, value));
    }
    let h = host(vm);
    let old = String::from(h.doc.element(node).and_then(|e| e.attr("style")).unwrap_or(""));
    let mut decls: Vec<String> = old.split(';').map(|d| String::from(d.trim())).filter(|d| !d.is_empty() && !set.iter().any(|(n, _)| d.split(':').next().is_some_and(|k| k.trim() == n))).collect();
    for (n, v) in set {
        decls.push(alloc::format!("{n}: {v}"));
    }
    if let Some(e) = h.doc.element_mut(node) {
        e.set_attr("style", &decls.join("; "));
    }
    h.changed();
    Ok(Value::Undefined)
}

/// document.getAnimations(): the animations scripts made that are running or paused, or hold their end.
fn document_animations(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let list: Vec<Value> = host_ref(vm).animations.iter().filter(|a| a.state != State::Idle && (a.state != State::Finished || a.fill & 1 == 1)).map(|a| Value::Obj(a.object)).collect();
    Ok(Value::Obj(vm.new_array(list)))
}

/// element.getAnimations(): those on it.
pub fn element_animations(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let list: Vec<Value> = host_ref(vm).animations.iter().filter(|a| a.node == n && a.state != State::Idle && (a.state != State::Finished || a.fill & 1 == 1)).map(|a| Value::Obj(a.object)).collect();
    Ok(Value::Obj(vm.new_array(list)))
}
