//! The window's own members: timers and animation frames, the console, the address (location, history), the
//! navigator and the screen, storage, performance's clock, base64, crypto's random numbers, styles worked out
//! (getComputedStyle), media queries (matchMedia), URL and URLSearchParams, text encoding, abort signals, blobs and
//! form data, and observers of what's seen (IntersectionObserver, ResizeObserver).

use super::events::{EvValue, proto_from};
use super::*;
use crate::css;
use crate::dom::NodeId;
use alloc::vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use js::object::{ALL, ENUMERABLE, WRITABLE};

/// How deeply the timer running now was nested (set by the engine's loop).
pub static TIMER_DEPTH: AtomicU32 = AtomicU32::new(0);
/// crypto.getRandomValues's generator's state (seeded by the system's random numbers).
static RANDOM: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);

/// Seeds crypto's random numbers (from the system's own).
pub fn seed(s: u64) {
    RANDOM.store(s | 1, Ordering::Relaxed);
}

fn random_u64() -> u64 {
    // splitmix64
    let mut z = RANDOM.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub fn init(vm: &mut Vm) {
    let w = host_ref(vm).window;
    for name in ["window", "self", "frames", "top", "parent"] {
        vm.put(w, name, Value::Obj(w), ENUMERABLE);
    }
    for (name, v) in [("name", js("")), ("closed", Value::Bool(false)), ("length", Value::Num(0.0)), ("opener", Value::Null), ("frameElement", Value::Null), ("isSecureContext", Value::Bool(true)), ("crossOriginIsolated", Value::Bool(false)), ("status", js(""))] {
        vm.put(w, name, v, ALL);
    }
    let origin = host_ref(vm).url.origin();
    vm.put(w, "origin", js(&origin), ALL);
    let wp = host_ref(vm).proto("Window").expect("made");
    for (name, f) in [
        ("innerWidth", inner_width as NativeFn),
        ("innerHeight", inner_height),
        ("outerWidth", inner_width),
        ("outerHeight", inner_height),
        ("devicePixelRatio", device_pixel_ratio),
        ("scrollX", scroll_x),
        ("scrollY", scroll_y),
        ("pageXOffset", scroll_x),
        ("pageYOffset", scroll_y),
        ("screenX", zero),
        ("screenY", zero),
        ("screenLeft", zero),
        ("screenTop", zero),
    ] {
        attribute(vm, wp, name, f, Some(ignore));
    }
    let fns: &[(&str, u32, NativeFn)] = &[
        ("setTimeout", 1, set_timeout),
        ("setInterval", 1, set_interval),
        ("clearTimeout", 1, clear_timer),
        ("clearInterval", 1, clear_timer),
        ("requestAnimationFrame", 1, request_animation_frame),
        ("cancelAnimationFrame", 1, cancel_animation_frame),
        ("requestIdleCallback", 1, request_idle_callback),
        ("cancelIdleCallback", 1, clear_timer),
        ("atob", 1, atob),
        ("btoa", 1, btoa),
        ("getComputedStyle", 1, get_computed_style),
        ("matchMedia", 1, match_media),
        ("scrollTo", 2, scroll_to),
        ("scroll", 2, scroll_to),
        ("scrollBy", 2, scroll_by),
        ("alert", 1, alert),
        ("confirm", 1, confirm),
        ("prompt", 1, prompt),
        ("print", 0, ignore),
        ("open", 0, window_open),
        ("close", 0, ignore),
        ("stop", 0, ignore),
        ("focus", 0, ignore),
        ("blur", 0, ignore),
        ("moveTo", 2, ignore),
        ("resizeTo", 2, ignore),
        ("postMessage", 2, post_message),
        ("structuredClone", 1, structured_clone),
        ("reportError", 1, report_error),
        ("getSelection", 0, get_selection),
        ("fetchLater", 1, ignore),
    ];
    for (name, len, f) in fns {
        method(vm, w, name, *len, *f);
    }
    console(vm, w);
    location(vm, w);
    history(vm, w);
    navigator(vm, w);
    screen(vm, w);
    storage(vm, w);
    performance(vm, w);
    crypto(vm, w);
    urls(vm);
    text_coding(vm);
    abort(vm);
    blobs(vm);
    observers(vm);
    misc(vm, w);
}

fn zero(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(0.0))
}

fn ignore(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Undefined)
}

fn false_value(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(false))
}

fn true_value(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(true))
}

fn null_value(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Null)
}

fn empty_array(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Obj(vm.new_array(Vec::new())))
}

/// A promise already fulfilled with a value.
pub fn resolved(vm: &mut Vm, v: Value) -> Value {
    let p = vm.new_promise();
    vm.fulfill_promise(p, v);
    Value::Obj(p)
}

/// A promise already rejected (and marked handled, so it isn't reported if nothing catches it).
pub fn rejected(vm: &mut Vm, e: Value) -> Value {
    let p = vm.new_promise();
    vm.reject_promise(p, e);
    if let Class::Promise(d) = &mut vm.heap.get_mut(p).class {
        d.handled = true;
    }
    Value::Obj(p)
}

fn inner_width(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(crate::num::round(host_ref(vm).view.0) as f64))
}
fn inner_height(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(crate::num::round(host_ref(vm).view.1) as f64))
}
fn device_pixel_ratio(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(host_ref(vm).view.2 as f64))
}
fn scroll_x(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(host_ref(vm).scroll.0 as f64))
}
fn scroll_y(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(host_ref(vm).scroll.1 as f64))
}

/// scrollTo(x, y) or scrollTo({left, top}).
fn scroll_args(vm: &mut Vm, args: &[Value]) -> Result<(Option<f64>, Option<f64>), Value> {
    if let Value::Obj(o) = arg(args, 0) {
        let l = get(vm, o, "left")?;
        let t = get(vm, o, "top")?;
        let l = if l.is_undefined() { None } else { Some(vm.to_number(&l)?) };
        let t = if t.is_undefined() { None } else { Some(vm.to_number(&t)?) };
        return Ok((l, t));
    }
    let x = vm.to_number(&arg(args, 0))?;
    let y = vm.to_number(&arg(args, 1))?;
    Ok((Some(x), Some(y)))
}

fn scroll_to(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (x, y) = scroll_args(vm, args)?;
    let h = host(vm);
    if let Some(x) = x.filter(|x| x.is_finite()) {
        h.scroll.0 = x.max(0.0) as f32;
    }
    if let Some(y) = y.filter(|y| y.is_finite()) {
        h.scroll.1 = y.max(0.0) as f32;
    }
    Ok(Value::Undefined)
}

fn scroll_by(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (x, y) = scroll_args(vm, args)?;
    let h = host(vm);
    if let Some(x) = x.filter(|x| x.is_finite()) {
        h.scroll.0 = (h.scroll.0 + x as f32).max(0.0);
    }
    if let Some(y) = y.filter(|y| y.is_finite()) {
        h.scroll.1 = (h.scroll.1 + y as f32).max(0.0);
    }
    Ok(Value::Undefined)
}

// ---------------------------------------------------------------------------------------------- timers

fn add_timer(vm: &mut Vm, args: &[Value], repeat: bool) -> R {
    let f = arg(args, 0);
    let f = if vm.is_callable(&f) {
        f
    } else {
        // (A string is code, run when it's due.)
        Value::Str(vm.to_string(&f)?)
    };
    let ms = vm.to_number(&arg(args, 1))?;
    let depth = TIMER_DEPTH.load(Ordering::Relaxed);
    let mut ms = if ms.is_finite() && ms > 0.0 { ms.min(2_147_483_647.0) } else { 0.0 };
    if depth > 5 && ms < 4.0 {
        ms = 4.0;
    }
    let extra: Vec<Value> = args.iter().skip(2).cloned().collect();
    let h = host(vm);
    let id = h.next_id();
    let due = h.now + ms;
    h.timers.push(Timer { id, due, every: if repeat { Some(ms.max(1.0)) } else { None }, f, args: extra, depth });
    Ok(Value::Num(id as f64))
}

fn set_timeout(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    add_timer(vm, args, false)
}

fn set_interval(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    add_timer(vm, args, true)
}

fn clear_timer(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let id = vm.to_number(&arg(args, 0)).unwrap_or(0.0) as u32;
    host(vm).timers.retain(|t| t.id != id);
    Ok(Value::Undefined)
}

fn request_animation_frame(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let f = arg(args, 0);
    if !vm.is_callable(&f) {
        return Err(vm.type_error("Failed to execute 'requestAnimationFrame' on 'Window': The callback provided as parameter 1 is not a function."));
    }
    let h = host(vm);
    let id = h.next_id();
    h.frames.push((id, f));
    Ok(Value::Num(id as f64))
}

fn cancel_animation_frame(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let id = vm.to_number(&arg(args, 0)).unwrap_or(0.0) as u32;
    host(vm).frames.retain(|(i, _)| *i != id);
    Ok(Value::Undefined)
}

fn request_idle_callback(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let f = arg(args, 0);
    if !vm.is_callable(&f) {
        return Err(vm.type_error("Failed to execute 'requestIdleCallback' on 'Window': parameter 1 is not of type 'Function'."));
    }
    // (Called soon, with a deadline that has time left.)
    let deadline = vm.new_object();
    vm.put(deadline, "didTimeout", Value::Bool(false), ALL);
    method(vm, deadline, "timeRemaining", 0, time_remaining);
    let wrapper = vm.native_with("", 0, idle_call, None, f);
    vm.set_data2(wrapper, Value::Obj(deadline));
    add_timer(vm, &[Value::Obj(wrapper), Value::Num(1.0)], false)
}

fn idle_call(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    let cb = vm.data(f);
    let d = vm.data2(f);
    vm.call(&cb, Value::Undefined, &[d])
}

fn time_remaining(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(40.0))
}

// --------------------------------------------------------------------------------------------- console

fn console(vm: &mut Vm, w: Gc) {
    let c = vm.new_object();
    for (name, level) in [("log", "log"), ("info", "info"), ("warn", "warn"), ("error", "error"), ("debug", "debug"), ("trace", "trace"), ("dir", "log"), ("dirxml", "log"), ("table", "log"), ("group", "log"), ("groupCollapsed", "log"), ("timeLog", "log")] {
        let f = vm.native_with(name, 0, console_log, None, js(level));
        vm.put(c, name, Value::Obj(f), HIDDEN | ENUMERABLE);
    }
    for name in ["groupEnd", "time", "timeEnd", "count", "countReset", "clear", "profile", "profileEnd", "timeStamp"] {
        method(vm, c, name, 0, ignore);
    }
    method(vm, c, "assert", 0, console_assert);
    vm.put(w, "console", Value::Obj(c), HIDDEN);
}

/// A value as the console shows it.
pub fn show(vm: &mut Vm, v: &Value, depth: u32) -> String {
    match v {
        Value::Str(s) => s.to_rust(),
        Value::Obj(g) => {
            let g = *g;
            if let Some(n) = node_of(vm, v) {
                let d = &host_ref(vm).doc;
                return match d.element(n) {
                    Some(e) => match e.id() {
                        Some(id) => format!("<{}#{id}>", e.name),
                        None => format!("<{}>", e.name),
                    },
                    None => String::from("#node"),
                };
            }
            match &vm.heap.get(g).class {
                Class::Error => describe_error(vm, v),
                Class::Function(_) | Class::Native(_) | Class::Bound(_) => String::from("[function]"),
                Class::Array(..) if depth < 2 => {
                    let items = vm.array_values(v);
                    let parts: Vec<String> = items.iter().take(20).map(|x| show_item(vm, x, depth + 1)).collect();
                    format!("[{}{}]", parts.join(", "), if items.len() > 20 { ", ..." } else { "" })
                }
                Class::Ordinary if depth < 2 => {
                    let keys = vm.own_keys_plain(g);
                    let mut parts = Vec::new();
                    for k in keys.into_iter().take(12) {
                        let Some(Prop { slot: Slot::Data(x), flags }) = vm.heap.get(g).props.get(&k).cloned() else { continue };
                        if flags & ENUMERABLE == 0 {
                            continue;
                        }
                        let name = match vm.key_value(k) {
                            Value::Str(s) => s.to_rust(),
                            _ => continue,
                        };
                        parts.push(format!("{name}: {}", show_item(vm, &x, depth + 1)));
                    }
                    format!("{{{}}}", parts.join(", "))
                }
                _ => match vm.to_string(v) {
                    Ok(s) => s.to_rust(),
                    Err(_) => String::from("[object]"),
                },
            }
        }
        _ => vm.to_string(v).map(|s| s.to_rust()).unwrap_or_default(),
    }
}

fn show_item(vm: &mut Vm, v: &Value, depth: u32) -> String {
    match v {
        Value::Str(s) => format!("\"{}\"", s.to_rust()),
        _ => show(vm, v, depth),
    }
}

fn console_log(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let level = string(vm, &vm.data(f)).unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    // (A first argument with %s and the like: the rest put in its place.)
    let mut rest = args;
    if let Some(Value::Str(fmt)) = args.first()
        && fmt.to_rust().contains('%')
    {
        let fmt = fmt.to_rust();
        let mut out = String::new();
        let mut i = 1;
        let mut chars = fmt.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%'
                && let Some(&k) = chars.peek()
                && matches!(k, 's' | 'd' | 'i' | 'f' | 'o' | 'O' | 'c' | '%')
            {
                chars.next();
                match k {
                    '%' => out.push('%'),
                    'c' => i += 1,
                    _ => {
                        if let Some(a) = args.get(i) {
                            out.push_str(&show(vm, a, 1));
                        }
                        i += 1;
                    }
                }
                continue;
            }
            out.push(c);
        }
        parts.push(out);
        rest = &args[i.min(args.len())..];
    }
    for a in rest {
        parts.push(show(vm, a, 0));
    }
    let line = parts.join(" ");
    let line = if level == "log" || level == "info" || level == "debug" { line } else { format!("{level}: {line}") };
    host(vm).log(line);
    Ok(Value::Undefined)
}

fn console_assert(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    if !arg(args, 0).truthy() {
        let parts: Vec<String> = args.iter().skip(1).map(|a| show(vm, a, 0)).collect();
        host(vm).log(format!("Assertion failed: {}", parts.join(" ")));
    }
    Ok(Value::Undefined)
}

fn alert(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let m = show(vm, &arg(args, 0), 0);
    host(vm).log(format!("alert: {m}"));
    Ok(Value::Undefined)
}

fn confirm(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let m = show(vm, &arg(args, 0), 0);
    host(vm).log(format!("confirm (answered no): {m}"));
    Ok(Value::Bool(false))
}

fn prompt(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let m = show(vm, &arg(args, 0), 0);
    host(vm).log(format!("prompt (no answer): {m}"));
    Ok(Value::Null)
}

fn window_open(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    // (A new window isn't opened: the page is sent to the address if it's a link-like open of one.)
    let u = arg(args, 0);
    if !u.is_undefined() {
        let s = string(vm, &u)?;
        let target = string(vm, &arg(args, 1)).unwrap_or_default();
        if !s.is_empty() && (target == "_self" || target == "_top") {
            let to = super::dom::resolve(vm, &s);
            if let Some(u) = url::Url::parse(&to, None) {
                navigate(vm, u);
            }
        }
    }
    Ok(Value::Null)
}

fn report_error(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    super::report(vm, &arg(args, 0), "reportError");
    Ok(Value::Undefined)
}

fn get_selection(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let d = wrap(vm, 0);
    let Value::Obj(dg) = d else { return Ok(Value::Null) };
    let m = get(vm, dg, "getSelection")?;
    vm.call(&m, d, &[])
}

// --------------------------------------------------------------------------------- location and history

/// Sends the page to an address (a script's): one that only changes its #fragment stays on this page.
pub fn navigate(vm: &mut Vm, to: url::Url) {
    if to.scheme == "javascript" {
        let code = url::unescape(&to.path, false);
        if let Err(e) = vm.indirect_eval(&code) {
            super::report(vm, &e, "a javascript: address");
        }
        return;
    }
    let cur = host_ref(vm).url.clone();
    let mut a = cur.clone();
    let mut b = to.clone();
    a.fragment = None;
    b.fragment = None;
    if a == b && to.fragment.is_some() {
        let old = cur.href();
        host(vm).url = to.clone();
        // hashchange, soon.
        let w = host_ref(vm).window;
        let f = vm.native_with("", 0, fire_hashchange, None, js(&old));
        vm.set_data2(f, js(&to.href()));
        host(vm).tasks.push((Value::Obj(f), Value::Obj(w), Vec::new()));
        return;
    }
    host(vm).navigate = Some(to);
}

fn fire_hashchange(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    let old = string(vm, &vm.data(f))?;
    let new = string(vm, &vm.data2(f))?;
    let w = Value::Obj(host_ref(vm).window);
    let init = EventInit::new("hashchange", "HashChangeEvent", false, false).with("oldURL", EvValue::Str(old)).with("newURL", EvValue::Str(new));
    super::events::fire(vm, &w, &init);
    Ok(Value::Undefined)
}

/// A form to send (the page works out its address and body, as a click on its button would).
pub fn request_submit(vm: &mut Vm, form: NodeId, submitter: Option<NodeId>) {
    host(vm).submit = Some((form, submitter));
}

fn location(vm: &mut Vm, w: Gc) {
    let lp = interface(vm, "Location", None, 0, None);
    for part in ["href", "protocol", "host", "hostname", "port", "pathname", "search", "hash", "origin"] {
        attribute_with(vm, lp, part, location_part, if part == "origin" { None } else { Some(set_location_part) }, js(part));
    }
    method(vm, lp, "assign", 1, location_assign);
    method(vm, lp, "replace", 1, location_assign);
    method(vm, lp, "reload", 0, location_reload);
    method(vm, lp, "toString", 0, location_href);
    let l = vm.alloc(Some(lp), Class::Ordinary);
    let ancestors = vm.new_array(Vec::new());
    vm.put(l, "ancestorOrigins", Value::Obj(ancestors), ALL);
    host(vm).cache.insert((usize::MAX, 1), l);
    let wp = host_ref(vm).proto("Window").expect("made");
    attribute(vm, wp, "location", window_location, Some(set_window_location));
    let _ = w;
}

fn window_location(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(host_ref(vm).cache.get(&(usize::MAX, 1)).map_or(Value::Null, |g| Value::Obj(*g)))
}

fn set_window_location(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = string(vm, &arg(args, 0))?;
    go_to(vm, &s);
    Ok(Value::Undefined)
}

fn go_to(vm: &mut Vm, s: &str) {
    let to = super::dom::resolve(vm, s);
    if let Some(u) = url::Url::parse(&to, None) {
        navigate(vm, u);
    }
}

fn location_part(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    let part = string(vm, &vm.data(f))?;
    Ok(js(&super::dom::url_part(&host_ref(vm).url, &part)))
}

fn location_href(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js(&host_ref(vm).url.href()))
}

fn set_location_part(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let part = string(vm, &vm.data(f))?;
    let v = string(vm, &arg(args, 0))?;
    if part == "href" {
        go_to(vm, &v);
        return Ok(Value::Undefined);
    }
    let mut u = host_ref(vm).url.clone();
    if u.set(&part, &v) {
        navigate(vm, u);
    }
    Ok(Value::Undefined)
}

fn location_assign(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = string(vm, &arg(args, 0))?;
    go_to(vm, &s);
    Ok(Value::Undefined)
}

fn location_reload(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let u = host_ref(vm).url.clone();
    host(vm).navigate = Some(u);
    Ok(Value::Undefined)
}

fn history(vm: &mut Vm, w: Gc) {
    let hp = interface(vm, "History", None, 0, None);
    attribute(vm, hp, "length", history_length, None);
    attribute(vm, hp, "state", history_state, None);
    method(vm, hp, "pushState", 2, push_state);
    method(vm, hp, "replaceState", 2, replace_state);
    method(vm, hp, "back", 0, history_back);
    method(vm, hp, "forward", 0, ignore);
    method(vm, hp, "go", 0, history_go);
    let h = vm.alloc(Some(hp), Class::Ordinary);
    vm.put(h, "scrollRestoration", js("auto"), ALL);
    vm.put(w, "history", Value::Obj(h), ENUMERABLE);
    host(vm).cache.insert((usize::MAX, 2), h);
}

fn history_obj(vm: &Vm) -> Gc {
    *host_ref(vm).cache.get(&(usize::MAX, 2)).expect("made")
}

fn history_length(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let h = history_obj(vm);
    let n = get(vm, h, "__length")?;
    Ok(if n.is_undefined() { Value::Num(1.0) } else { n })
}

fn history_state(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let h = history_obj(vm);
    let s = get(vm, h, "__state")?;
    Ok(if s.is_undefined() { Value::Null } else { s })
}

fn change_state(vm: &mut Vm, args: &[Value], push: bool) -> R {
    let h = history_obj(vm);
    let state = structured_clone_value(vm, &arg(args, 0))?;
    if let Value::Str(s) = arg(args, 2) {
        let to = super::dom::resolve(vm, &s.to_rust());
        let Some(u) = url::Url::parse(&to, None) else {
            return Err(dom_exception(vm, "SecurityError", "The new address can't be used."));
        };
        if u.origin() != host_ref(vm).url.origin() {
            return Err(dom_exception(vm, "SecurityError", "A history state object with URL '…' cannot be created in a document with origin '…'."));
        }
        host(vm).url = u;
    } else if !arg(args, 2).is_nullish() {
        let s = string(vm, &arg(args, 2))?;
        let to = super::dom::resolve(vm, &s);
        if let Some(u) = url::Url::parse(&to, None) {
            host(vm).url = u;
        }
    }
    vm.put(h, "__state", state, WRITABLE);
    if push {
        let n = get(vm, h, "__length")?;
        let n = if let Value::Num(n) = n { n } else { 1.0 };
        vm.put(h, "__length", Value::Num(n + 1.0), WRITABLE);
    }
    Ok(Value::Undefined)
}

fn push_state(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    change_state(vm, args, true)
}

fn replace_state(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    change_state(vm, args, false)
}

fn history_back(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    host(vm).history_go = Some(-1);
    Ok(Value::Undefined)
}

fn history_go(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let d = vm.to_int32(&arg(args, 0))?;
    if d == 0 {
        let u = host_ref(vm).url.clone();
        host(vm).navigate = Some(u);
    } else if d < 0 {
        host(vm).history_go = Some(d);
    }
    Ok(Value::Undefined)
}

// --------------------------------------------------------------------------------- navigator and screen

fn navigator(vm: &mut Vm, w: Gc) {
    let np = interface(vm, "Navigator", None, 0, None);
    let n = vm.alloc(Some(np), Class::Ordinary);
    let ua = web::USER_AGENT;
    let app_version = ua.strip_prefix("Mozilla/").unwrap_or(ua);
    for (name, v) in [
        ("userAgent", js(ua)),
        ("appVersion", js(app_version)),
        ("appName", js("Netscape")),
        ("appCodeName", js("Mozilla")),
        ("platform", js("Linux x86_64")),
        ("product", js("Gecko")),
        ("productSub", js("20030107")),
        ("vendor", js("")),
        ("vendorSub", js("")),
        ("language", js("en-US")),
        ("onLine", Value::Bool(true)),
        ("cookieEnabled", Value::Bool(true)),
        ("doNotTrack", Value::Null),
        ("hardwareConcurrency", Value::Num(4.0)),
        ("maxTouchPoints", Value::Num(0.0)),
        ("deviceMemory", Value::Num(8.0)),
        ("webdriver", Value::Bool(false)),
        ("pdfViewerEnabled", Value::Bool(false)),
        ("javaEnabled", Value::Undefined),
    ] {
        vm.put(np, name, v, ENUMERABLE | js::object::CONFIGURABLE);
    }
    let langs = vm.new_array(vec![js("en-US"), js("en")]);
    vm.put(np, "languages", Value::Obj(langs), ENUMERABLE | js::object::CONFIGURABLE);
    let empty = vm.new_array(Vec::new());
    vm.put(np, "plugins", Value::Obj(empty), ENUMERABLE);
    let empty2 = vm.new_array(Vec::new());
    vm.put(np, "mimeTypes", Value::Obj(empty2), ENUMERABLE);
    method(vm, np, "javaEnabled", 0, false_value);
    method(vm, np, "sendBeacon", 1, true_value);
    method(vm, np, "vibrate", 1, false_value);
    method(vm, np, "registerProtocolHandler", 2, ignore);
    let conn = vm.new_object();
    for (k, v) in [("effectiveType", js("4g")), ("downlink", Value::Num(10.0)), ("rtt", Value::Num(50.0)), ("saveData", Value::Bool(false)), ("type", js("ethernet"))] {
        vm.put(conn, k, v, ALL);
    }
    method(vm, conn, "addEventListener", 2, ignore);
    method(vm, conn, "removeEventListener", 2, ignore);
    vm.put(np, "connection", Value::Obj(conn), ENUMERABLE);
    let perms = vm.new_object();
    method(vm, perms, "query", 1, permission_query);
    vm.put(np, "permissions", Value::Obj(perms), ENUMERABLE);
    let storage = vm.new_object();
    method(vm, storage, "estimate", 0, storage_estimate);
    method(vm, storage, "persist", 0, resolved_false);
    method(vm, storage, "persisted", 0, resolved_false);
    vm.put(np, "storage", Value::Obj(storage), ENUMERABLE);
    let clip = vm.new_object();
    method(vm, clip, "writeText", 1, resolved_undefined);
    method(vm, clip, "readText", 0, rejected_fn);
    method(vm, clip, "write", 1, resolved_undefined);
    method(vm, clip, "read", 0, rejected_fn);
    vm.put(np, "clipboard", Value::Obj(clip), ENUMERABLE);
    let ua_data = vm.new_object();
    let brands = vm.new_array(Vec::new());
    vm.put(ua_data, "brands", Value::Obj(brands), ALL);
    vm.put(ua_data, "mobile", Value::Bool(false), ALL);
    vm.put(ua_data, "platform", js("Linux"), ALL);
    method(vm, ua_data, "getHighEntropyValues", 1, high_entropy);
    vm.put(np, "userAgentData", Value::Obj(ua_data), ENUMERABLE);
    vm.put(w, "navigator", Value::Obj(n), ENUMERABLE);
    vm.put(w, "clientInformation", Value::Obj(n), HIDDEN);
}

fn resolved_false(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(resolved(vm, Value::Bool(false)))
}

fn resolved_undefined(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(resolved(vm, Value::Undefined))
}

fn rejected_fn(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let e = dom_exception(vm, "NotAllowedError", "Not allowed here.");
    Ok(rejected(vm, e))
}

fn permission_query(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = vm.new_object();
    vm.put(s, "state", js("denied"), ALL);
    vm.put(s, "onchange", Value::Null, ALL);
    method(vm, s, "addEventListener", 2, ignore);
    Ok(resolved(vm, Value::Obj(s)))
}

fn storage_estimate(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.new_object();
    vm.put(o, "quota", Value::Num(1e9), ALL);
    vm.put(o, "usage", Value::Num(0.0), ALL);
    Ok(resolved(vm, Value::Obj(o)))
}

fn high_entropy(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.new_object();
    for (k, v) in [("platform", "Linux"), ("platformVersion", ""), ("architecture", "x86"), ("model", ""), ("uaFullVersion", "1.0"), ("bitness", "64")] {
        vm.put(o, k, js(v), ALL);
    }
    vm.put(o, "mobile", Value::Bool(false), ALL);
    let b = vm.new_array(Vec::new());
    vm.put(o, "brands", Value::Obj(b), ALL);
    Ok(resolved(vm, Value::Obj(o)))
}

fn screen(vm: &mut Vm, w: Gc) {
    let sp = interface(vm, "Screen", None, 0, None);
    for name in ["width", "availWidth"] {
        attribute(vm, sp, name, screen_width, None);
    }
    for name in ["height", "availHeight"] {
        attribute(vm, sp, name, screen_height, None);
    }
    vm.put(sp, "colorDepth", Value::Num(24.0), ALL);
    vm.put(sp, "pixelDepth", Value::Num(24.0), ALL);
    vm.put(sp, "availLeft", Value::Num(0.0), ALL);
    vm.put(sp, "availTop", Value::Num(0.0), ALL);
    let s = vm.alloc(Some(sp), Class::Ordinary);
    let o = vm.new_object();
    vm.put(o, "type", js("landscape-primary"), ALL);
    vm.put(o, "angle", Value::Num(0.0), ALL);
    method(vm, o, "addEventListener", 2, ignore);
    method(vm, o, "removeEventListener", 2, ignore);
    method(vm, o, "lock", 1, rejected_fn);
    vm.put(s, "orientation", Value::Obj(o), ALL);
    vm.put(w, "screen", Value::Obj(s), ENUMERABLE);
}

fn screen_width(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(crate::num::round(host_ref(vm).view.0) as f64))
}

fn screen_height(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(crate::num::round(host_ref(vm).view.1) as f64))
}

// --------------------------------------------------------------------------------------------- storage

fn storage(vm: &mut Vm, w: Gc) {
    let sp = interface(vm, "Storage", None, 0, None);
    attribute(vm, sp, "length", storage_length, None);
    method(vm, sp, "getItem", 1, storage_get);
    method(vm, sp, "setItem", 2, storage_set);
    method(vm, sp, "removeItem", 1, storage_remove);
    method(vm, sp, "clear", 0, storage_clear);
    method(vm, sp, "key", 1, storage_key);
    let h = vm.new_object();
    method(vm, h, "get", 3, storage_trap_get);
    method(vm, h, "set", 4, storage_trap_set);
    method(vm, h, "has", 2, storage_trap_has);
    method(vm, h, "deleteProperty", 2, storage_trap_delete);
    method(vm, h, "ownKeys", 1, storage_trap_keys);
    method(vm, h, "getOwnPropertyDescriptor", 2, storage_trap_descriptor);
    for (name, which) in [("localStorage", 0u64), ("sessionStorage", 1)] {
        let t = vm.alloc(Some(sp), Class::Host(super::LIST + 100, which));
        let p = vm.alloc(None, Class::Proxy(Some((t, h)), 0));
        vm.put(w, name, Value::Obj(p), ENUMERABLE);
        host(vm).cache.insert((usize::MAX, 10 + which as u32), p);
        host(vm).cache.insert((usize::MAX, 20 + which as u32), t);
    }
}

/// Which storage `this` is (0 local, 1 session): the proxy or its target.
fn which_storage(vm: &Vm, v: &Value) -> Option<u64> {
    let Value::Obj(g) = v else { return None };
    match &vm.heap.get(*g).class {
        Class::Host(k, w) if *k == super::LIST + 100 => Some(*w),
        Class::Proxy(Some((t, _)), _) => match vm.heap.get(*t).class {
            Class::Host(k, w) if k == super::LIST + 100 => Some(w),
            _ => None,
        },
        _ => None,
    }
}

fn store(vm: &mut Vm, which: u64) -> &mut BTreeMap<String, String> {
    let h = host(vm);
    if which == 0 { &mut h.local } else { &mut h.session }
}

fn this_storage(vm: &mut Vm, this: &Value) -> Result<u64, Value> {
    which_storage(vm, this).ok_or_else(|| vm.type_error("Illegal invocation"))
}

fn storage_length(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let w = this_storage(vm, this)?;
    Ok(Value::Num(store(vm, w).len() as f64))
}

fn storage_get(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let w = this_storage(vm, this)?;
    let k = str_arg(vm, args, 0, "getItem")?;
    Ok(store(vm, w).get(&k).map_or(Value::Null, |v| js(v)))
}

fn storage_set(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let w = this_storage(vm, this)?;
    let k = str_arg(vm, args, 0, "setItem")?;
    let v = str_arg(vm, args, 1, "setItem")?;
    let s = store(vm, w);
    let total: usize = s.iter().map(|(a, b)| a.len() + b.len()).sum::<usize>() + k.len() + v.len();
    if total > 5 << 20 {
        return Err(dom_exception(vm, "QuotaExceededError", "Setting the value exceeded the quota."));
    }
    store(vm, w).insert(k, v);
    host(vm).storage_changed = true;
    Ok(Value::Undefined)
}

fn storage_remove(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let w = this_storage(vm, this)?;
    let k = str_arg(vm, args, 0, "removeItem")?;
    store(vm, w).remove(&k);
    host(vm).storage_changed = true;
    Ok(Value::Undefined)
}

fn storage_clear(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let w = this_storage(vm, this)?;
    store(vm, w).clear();
    host(vm).storage_changed = true;
    Ok(Value::Undefined)
}

fn storage_key(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let w = this_storage(vm, this)?;
    let i = vm.to_uint32(&arg(args, 0))? as usize;
    Ok(store(vm, w).keys().nth(i).map_or(Value::Null, |k| js(k)))
}

fn storage_trap_get(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (t, key, receiver) = (arg(args, 0), arg(args, 1), arg(args, 2));
    let Some(w) = which_storage(vm, &t) else { return Ok(Value::Undefined) };
    let Value::Obj(tg) = t else { return Ok(Value::Undefined) };
    let k = vm.to_key(&key)?;
    if vm.has_property(tg, k)? {
        // (The methods: called with the proxy's target as this, which they accept.)
        return vm.get(tg, k, &receiver);
    }
    if let Value::Str(s) = &key
        && let Some(v) = store(vm, w).get(&s.to_rust())
    {
        return Ok(js(v));
    }
    Ok(Value::Undefined)
}

fn storage_trap_set(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (t, key, v) = (arg(args, 0), arg(args, 1), arg(args, 2));
    let Some(w) = which_storage(vm, &t) else { return Ok(Value::Bool(false)) };
    if let Value::Str(s) = &key {
        let v = string(vm, &v)?;
        store(vm, w).insert(s.to_rust(), v);
        host(vm).storage_changed = true;
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(false))
}

fn storage_trap_has(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (t, key) = (arg(args, 0), arg(args, 1));
    let Some(w) = which_storage(vm, &t) else { return Ok(Value::Bool(false)) };
    if let Value::Str(s) = &key
        && store(vm, w).contains_key(&s.to_rust())
    {
        return Ok(Value::Bool(true));
    }
    let Value::Obj(tg) = t else { return Ok(Value::Bool(false)) };
    let k = vm.to_key(&key)?;
    Ok(Value::Bool(vm.has_property(tg, k)?))
}

fn storage_trap_delete(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (t, key) = (arg(args, 0), arg(args, 1));
    let Some(w) = which_storage(vm, &t) else { return Ok(Value::Bool(false)) };
    if let Value::Str(s) = &key {
        store(vm, w).remove(&s.to_rust());
        host(vm).storage_changed = true;
    }
    Ok(Value::Bool(true))
}

fn storage_trap_keys(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let Some(w) = which_storage(vm, &arg(args, 0)) else { return Ok(Value::Obj(vm.new_array(Vec::new()))) };
    let keys: Vec<Value> = store(vm, w).keys().map(|k| js(k)).collect();
    Ok(Value::Obj(vm.new_array(keys)))
}

fn storage_trap_descriptor(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (t, key) = (arg(args, 0), arg(args, 1));
    let Some(w) = which_storage(vm, &t) else { return Ok(Value::Undefined) };
    if let Value::Str(s) = &key
        && let Some(v) = store(vm, w).get(&s.to_rust()).cloned()
    {
        let d = vm.new_object();
        vm.put(d, "value", js(&v), ALL);
        vm.put(d, "writable", Value::Bool(true), ALL);
        vm.put(d, "enumerable", Value::Bool(true), ALL);
        vm.put(d, "configurable", Value::Bool(true), ALL);
        return Ok(Value::Obj(d));
    }
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------------------ performance and crypto

fn performance(vm: &mut Vm, w: Gc) {
    let pp = interface(vm, "Performance", Some("EventTarget"), 0, None);
    let p = vm.alloc(Some(pp), Class::Ordinary);
    method(vm, pp, "now", 0, performance_now);
    attribute(vm, pp, "timeOrigin", time_origin, None);
    for name in ["mark", "measure"] {
        method(vm, pp, name, 1, null_value);
    }
    for name in ["clearMarks", "clearMeasures", "clearResourceTimings", "setResourceTimingBufferSize"] {
        method(vm, pp, name, 0, ignore);
    }
    for name in ["getEntries", "getEntriesByName", "getEntriesByType"] {
        method(vm, pp, name, 1, empty_array);
    }
    method(vm, pp, "toJSON", 0, performance_json);
    let timing = vm.new_object();
    let epoch = host_ref(vm).epoch;
    for name in ["navigationStart", "fetchStart", "requestStart", "responseStart", "responseEnd", "domLoading", "domInteractive", "domContentLoadedEventStart", "domContentLoadedEventEnd", "domComplete", "loadEventStart", "loadEventEnd", "connectStart", "connectEnd", "domainLookupStart", "domainLookupEnd", "secureConnectionStart", "redirectStart", "redirectEnd", "unloadEventStart", "unloadEventEnd"] {
        vm.put(timing, name, Value::Num(js::math::floor(epoch)), ALL);
    }
    vm.put(p, "timing", Value::Obj(timing), ALL);
    let nav = vm.new_object();
    vm.put(nav, "type", Value::Num(0.0), ALL);
    vm.put(nav, "redirectCount", Value::Num(0.0), ALL);
    vm.put(p, "navigation", Value::Obj(nav), ALL);
    vm.put(w, "performance", Value::Obj(p), ENUMERABLE);
    let po = interface(vm, "PerformanceObserver", None, 1, Some(performance_observer_new));
    method(vm, po, "observe", 1, ignore);
    method(vm, po, "disconnect", 0, ignore);
    method(vm, po, "takeRecords", 0, empty_array);
    if let Some(Value::Obj(c)) = super::events::ctor_of(vm, po) {
        let types = vm.new_array(Vec::new());
        vm.put(c, "supportedEntryTypes", Value::Obj(types), ALL);
    }
}

fn performance_now(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(host_ref(vm).now))
}

fn time_origin(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(host_ref(vm).epoch))
}

fn performance_json(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.new_object();
    let e = host_ref(vm).epoch;
    vm.put(o, "timeOrigin", Value::Num(e), ALL);
    Ok(Value::Obj(o))
}

fn performance_observer_new(vm: &mut Vm, nt: &Value, _args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "PerformanceObserver")?;
    Ok(Value::Obj(vm.alloc(Some(p), Class::Ordinary)))
}

fn crypto(vm: &mut Vm, w: Gc) {
    let cp = interface(vm, "Crypto", None, 0, None);
    method(vm, cp, "getRandomValues", 1, get_random_values);
    method(vm, cp, "randomUUID", 0, random_uuid);
    let c = vm.alloc(Some(cp), Class::Ordinary);
    let subtle = vm.new_object();
    method(vm, subtle, "digest", 2, digest);
    vm.put(c, "subtle", Value::Obj(subtle), ENUMERABLE);
    vm.put(w, "crypto", Value::Obj(c), ENUMERABLE);
}

fn get_random_values(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let a = arg(args, 0);
    let Value::Obj(g) = a else { return Err(vm.type_error("Failed to execute 'getRandomValues' on 'Crypto': parameter 1 is not of type 'ArrayBufferView'.")) };
    let (buf, off, len) = match &vm.heap.get(g).class {
        Class::TypedArray(t) => (t.buffer, t.offset, vm.typed_length(t) * t.kind.size()),
        Class::DataView(b, o, l) => (*b, *o, l.unwrap_or(0)),
        _ => return Err(vm.type_error("Failed to execute 'getRandomValues' on 'Crypto': parameter 1 is not of type 'ArrayBufferView'.")),
    };
    if len > 65536 {
        return Err(dom_exception(vm, "QuotaExceededError", "The ArrayBufferView's byte length exceeds the number of bytes of entropy available via this API (65536)."));
    }
    if let Class::ArrayBuffer(Some(b), _) = &mut vm.heap.get_mut(buf).class {
        let end = (off + len).min(b.len());
        for chunk in b[off.min(end)..end].chunks_mut(8) {
            let r = random_u64().to_le_bytes();
            chunk.copy_from_slice(&r[..chunk.len()]);
        }
    }
    Ok(a)
}

fn random_uuid(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let (a, b) = (random_u64(), random_u64());
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_le_bytes());
    bytes[8..].copy_from_slice(&b.to_le_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let s = h.concat();
    Ok(js(&format!("{}-{}-{}-{}-{}", &s[..8], &s[8..12], &s[12..16], &s[16..20], &s[20..])))
}

fn digest(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let e = dom_exception(vm, "NotSupportedError", "This browser's crypto.subtle can't digest yet.");
    Ok(rejected(vm, e))
}

// ------------------------------------------------------------------------------------------- base64

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Forgiving base64 (spaces ignored, padding optional): None if it isn't base64.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut chars: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if chars.len() % 4 == 0 && chars.ends_with(b"==") {
        chars.truncate(chars.len() - 2);
    } else if chars.len() % 4 == 0 && chars.ends_with(b"=") {
        chars.truncate(chars.len() - 1);
    }
    if chars.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(chars.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in chars {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

fn btoa(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = vm.to_string(&arg(args, 0))?;
    let mut bytes = Vec::with_capacity(s.len());
    for u in s.units() {
        if *u > 0xff {
            return Err(dom_exception(vm, "InvalidCharacterError", "Failed to execute 'btoa' on 'Window': The string to be encoded contains characters outside of the Latin1 range."));
        }
        bytes.push(*u as u8);
    }
    Ok(js(&base64_encode(&bytes)))
}

fn atob(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = string(vm, &arg(args, 0))?;
    match base64_decode(&s) {
        Some(b) => Ok(Value::Str(JsStr::from_units(b.into_iter().map(|x| x as u16).collect()))),
        None => Err(dom_exception(vm, "InvalidCharacterError", "Failed to execute 'atob' on 'Window': The string to be decoded is not correctly encoded.")),
    }
}

// -------------------------------------------------------------------------------- styles, media queries

fn get_computed_style(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let n = node_arg(vm, args, 0, "getComputedStyle")?;
    if host_ref(vm).doc.element(n).is_none() {
        return Err(vm.type_error("Failed to execute 'getComputedStyle' on 'Window': parameter 1 is not of type 'Element'."));
    }
    Ok(super::lists::computed_style(vm, n))
}

/// A CSS keyword from a Rust enum's name ("InlineBlock" as "inline-block").
fn keyword(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    if out == "no-wrap" { String::from("nowrap") } else { out }
}

/// A number of CSS pixels as CSS writes it ("12px", "12.5px").
fn css_px(v: f32) -> String {
    let r = crate::num::round(v * 1000.0) / 1000.0;
    format!("{}px", js::value::number_to_string(r as f64))
}

/// A colour as a computed style gives it: rgb(), or rgba() if it isn't opaque.
fn css_color(c: u32) -> String {
    let (a, r, g, b) = (c >> 24, (c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    if a == 0xFF {
        format!("rgb({r}, {g}, {b})")
    } else {
        let alpha = crate::num::round(a as f32 / 255.0 * 1000.0) / 1000.0;
        format!("rgba({r}, {g}, {b}, {})", js::value::number_to_string(alpha as f64))
    }
}

/// A property's value from the styles the page worked out for an element (laid out again first if a script
/// changed the page), for the properties the browser works out; None for the rest.
pub fn styled_value(vm: &mut Vm, n: NodeId, prop: &str) -> Option<String> {
    use crate::style::{LineHeight, Size};
    super::fresh_layout(vm);
    let h = host_ref(vm);
    let st = h.styles.as_ref()?.of.get(n)?.clone()?;
    let dpr = if h.view.2 > 0.0 { h.view.2 } else { 1.0 };
    let rect = h.rects.get(n).copied().flatten();
    let px = |v: f32| css_px(v / dpr);
    let side = |p: &str| ["top", "right", "bottom", "left"].iter().position(|s| *s == p);
    let size = |s: Size| match s {
        Size::Len(l) => px(l.px),
        Size::None => String::from("none"),
        _ => String::from("auto"),
    };
    // (The sides' order in the style: top, right, bottom, left.)
    let parts: Vec<&str> = prop.split('-').collect();
    Some(match prop {
        "display" => keyword(&format!("{:?}", st.display)),
        "position" => keyword(&format!("{:?}", st.position)),
        "float" => keyword(&format!("{:?}", st.float)),
        "clear" => {
            if st.clear_both {
                String::from("both")
            } else {
                keyword(&format!("{:?}", st.clear))
            }
        }
        "visibility" => String::from(if st.visible { "visible" } else { "hidden" }),
        "opacity" => js::value::number_to_string(st.opacity as f64),
        "z-index" => st.z_index.map_or(String::from("auto"), |z| format!("{z}")),
        "color" => css_color(st.color),
        "background-color" => css_color(st.background),
        "font-size" => px(st.font_size),
        "font-weight" => format!("{}", st.font_weight),
        "font-style" => String::from(if st.italic { "italic" } else { "normal" }),
        "line-height" => match st.line_height {
            LineHeight::Normal => String::from("normal"),
            LineHeight::Factor(f) => px(f * st.font_size),
            LineHeight::Px(p) => px(p),
        },
        "text-align" => keyword(&format!("{:?}", st.text_align)),
        "white-space" => keyword(&format!("{:?}", st.white_space)),
        "overflow" | "overflow-x" | "overflow-y" => String::from(if st.clip { "hidden" } else { "visible" }),
        "box-sizing" => String::from(if st.border_box { "border-box" } else { "content-box" }),
        "flex-direction" => keyword(&format!("{:?}", st.flex_direction)),
        "flex-grow" => js::value::number_to_string(st.grow as f64),
        "flex-shrink" => js::value::number_to_string(st.shrink as f64),
        "order" => format!("{}", st.order),
        "width" | "height" => {
            let tall = prop == "height";
            match rect {
                Some(r) if st.display != crate::style::Display::None && !st.display.is_inline_level() || st.display == crate::style::Display::InlineBlock => {
                    let whole = if tall { r.3 } else { r.2 };
                    let (a, b) = if tall { (0, 2) } else { (3, 1) };
                    let inner = whole * dpr - st.padding[a].px - st.padding[b].px - st.border_width[a] - st.border_width[b];
                    if st.border_box { css_px(whole) } else { px(inner.max(0.0)) }
                }
                _ => size(if tall { st.height } else { st.width }),
            }
        }
        "top" | "right" | "bottom" | "left" => size(st.inset[side(prop)?]),
        _ if parts.len() == 2 && parts[0] == "margin" => match st.margin[side(parts[1])?] {
            Size::Len(l) => px(l.px),
            _ => String::from("0px"),
        },
        _ if parts.len() == 2 && parts[0] == "padding" => px(st.padding[side(parts[1])?].px),
        _ if parts.len() == 3 && parts[0] == "border" => {
            let i = side(parts[1])?;
            match parts[2] {
                "width" => px(if st.border_style[i] == crate::style::BorderStyle::None { 0.0 } else { st.border_width[i] }),
                "style" => keyword(&format!("{:?}", st.border_style[i])),
                "color" => css_color(st.border_color[i]),
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// A property's value as the page worked it out for an element (what the system gave the scripts after the last
/// layout), else a value that fits what the element is.
pub fn computed_value(vm: &mut Vm, n: NodeId, prop: &str) -> String {
    if let Some(v) = host_ref(vm).computed.get(&(n, String::from(prop))) {
        return v.clone();
    }
    let h = host_ref(vm);
    let d = &h.doc;
    let name = d.name(n);
    let r = h.rects.get(n).copied().flatten();
    let hidden = d.element(n).is_some_and(|e| e.has_attr("hidden")) || matches!(name, "head" | "script" | "style" | "title" | "meta" | "link" | "template" | "noscript");
    match prop {
        "display" => String::from(if hidden || (r.is_none() && !h.rects.is_empty() && d.connected(n)) {
            "none"
        } else if matches!(name, "span" | "a" | "b" | "i" | "em" | "strong" | "small" | "code" | "label" | "img" | "input" | "button" | "select" | "textarea" | "abbr" | "cite" | "q" | "sub" | "sup" | "u" | "s" | "kbd" | "var" | "time" | "mark" | "bdi" | "bdo" | "data" | "dfn" | "br" | "wbr" | "svg" | "canvas" | "video" | "audio" | "iframe" | "picture" | "slot") {
            "inline"
        } else if name == "li" {
            "list-item"
        } else if name == "table" {
            "table"
        } else if name == "tr" {
            "table-row"
        } else if matches!(name, "td" | "th") {
            "table-cell"
        } else {
            "block"
        }),
        "visibility" => String::from("visible"),
        "position" => String::from("static"),
        "opacity" => String::from("1"),
        "color" => String::from("rgb(0, 0, 0)"),
        "background-color" => String::from("rgba(0, 0, 0, 0)"),
        "background-image" | "transform" | "filter" | "box-shadow" | "text-shadow" | "max-width" | "max-height" | "float" | "clear" => String::from("none"),
        "font-size" => String::from("16px"),
        "font-weight" => String::from(if matches!(name, "b" | "strong" | "th" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6") { "700" } else { "400" }),
        "font-style" => String::from("normal"),
        "font-family" => String::from("sans-serif"),
        "line-height" => String::from("normal"),
        "width" => r.map_or(String::from("auto"), |r| format!("{}px", r.2)),
        "height" => r.map_or(String::from("auto"), |r| format!("{}px", r.3)),
        "overflow" | "overflow-x" | "overflow-y" => String::from("visible"),
        "box-sizing" => String::from("content-box"),
        "z-index" | "top" | "left" | "right" | "bottom" => String::from("auto"),
        "pointer-events" | "cursor" => String::from("auto"),
        "direction" => String::from("ltr"),
        "white-space" => String::from("normal"),
        "text-align" => String::from("start"),
        "transition-duration" | "animation-duration" | "transition-delay" | "animation-delay" => String::from("0s"),
        "animation-name" | "transition-property" => String::from(if prop == "animation-name" { "none" } else { "all" }),
        p if p.starts_with("margin") || p.starts_with("padding") || (p.starts_with("border") && p.ends_with("width")) => String::from("0px"),
        p if p.starts_with("border") && p.ends_with("style") => String::from("none"),
        _ => String::new(),
    }
}

fn match_media(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let q = str_arg(vm, args, 0, "matchMedia")?;
    let (w, h, _) = host_ref(vm).view;
    let matches = css::media_matches(&css::tokens(&q), css::Viewport { width: w, height: h });
    let p = host_ref(vm).proto("MediaQueryList").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let o = vm.alloc(Some(p), Class::Ordinary);
    vm.put(o, "matches", Value::Bool(matches), ALL);
    vm.put(o, "media", js(&q), ALL);
    vm.put(o, "onchange", Value::Null, ALL);
    Ok(Value::Obj(o))
}

// -------------------------------------------------------------------------------- URL, URLSearchParams

fn urls(vm: &mut Vm) {
    let up = interface(vm, "URL", None, 1, Some(url_new));
    for part in ["href", "protocol", "username", "password", "host", "hostname", "port", "pathname", "search", "hash", "origin"] {
        attribute_with(vm, up, part, url_get, if part == "origin" { None } else { Some(url_set) }, js(part));
    }
    attribute(vm, up, "searchParams", url_search_params, None);
    method(vm, up, "toString", 0, url_href);
    method(vm, up, "toJSON", 0, url_href);
    if let Some(Value::Obj(c)) = super::events::ctor_of(vm, up) {
        method(vm, c, "createObjectURL", 1, create_object_url);
        method(vm, c, "revokeObjectURL", 1, ignore);
        method(vm, c, "canParse", 1, url_can_parse);
        method(vm, c, "parse", 1, url_parse);
    }
    let w = host_ref(vm).window;
    if let Some(c) = super::events::ctor_of(vm, up) {
        vm.put(w, "webkitURL", c, HIDDEN);
    }
    let sp = interface(vm, "URLSearchParams", None, 0, Some(params_new));
    for (name, len, f) in [
        ("append", 2, params_append as NativeFn),
        ("delete", 1, params_delete),
        ("get", 1, params_get),
        ("getAll", 1, params_get_all),
        ("has", 1, params_has),
        ("set", 2, params_set),
        ("sort", 0, params_sort),
        ("toString", 0, params_to_string),
        ("forEach", 1, params_for_each),
        ("entries", 0, params_entries),
        ("keys", 0, params_keys),
        ("values", 0, params_values),
    ] {
        method(vm, sp, name, len, f);
    }
    attribute(vm, sp, "size", params_size, None);
    let entries = get(vm, sp, "entries").unwrap_or(Value::Undefined);
    let k = Key::Sym(vm.sym(Sym::Iterator));
    vm.put_key(sp, k, entries, HIDDEN);
}

fn this_url(vm: &mut Vm, this: &Value) -> Result<(Gc, url::Url), Value> {
    let Value::Obj(g) = this else { return Err(vm.type_error("Illegal invocation")) };
    match slot(vm, this, host_ref(vm).syms.data) {
        Value::Str(s) => match url::Url::parse(&s.to_rust(), None) {
            Some(u) => Ok((*g, u)),
            None => Err(vm.type_error("Invalid URL")),
        },
        _ => Err(vm.type_error("Illegal invocation")),
    }
}

/// new URL(url, base).
fn parse_url_args(vm: &mut Vm, args: &[Value]) -> Result<Option<url::Url>, Value> {
    let s = str_arg(vm, args, 0, "URL")?;
    let base = match arg(args, 1) {
        Value::Undefined => None,
        b => {
            let bs = string(vm, &b)?;
            match url::Url::parse(&bs, None) {
                Some(u) => Some(u),
                None => return Ok(None),
            }
        }
    };
    Ok(url::Url::parse(&s, base.as_ref()))
}

fn url_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let Some(u) = parse_url_args(vm, args)? else {
        let s = string(vm, &arg(args, 0))?;
        return Err(vm.type_error(&format!("Failed to construct 'URL': Invalid URL '{s}'")));
    };
    let p = proto_from(vm, nt, "URL")?;
    let g = vm.alloc(Some(p), Class::Ordinary);
    set_slot(vm, g, host_ref(vm).syms.data, js(&u.href()));
    Ok(Value::Obj(g))
}

fn url_can_parse(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(parse_url_args(vm, args)?.is_some()))
}

fn url_parse(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    if parse_url_args(vm, args)?.is_none() {
        return Ok(Value::Null);
    }
    let w = host_ref(vm).window;
    let c = get(vm, w, "URL")?;
    let _ = f;
    vm.construct(&c, args, &c)
}

fn url_get(vm: &mut Vm, this: &Value, _args: &[Value], f: Gc) -> R {
    let (_, u) = this_url(vm, this)?;
    let part = string(vm, &vm.data(f))?;
    Ok(js(&super::dom::url_part(&u, &part)))
}

fn url_href(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, u) = this_url(vm, this)?;
    Ok(js(&u.href()))
}

fn url_set(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let (g, mut u) = this_url(vm, this)?;
    let part = string(vm, &vm.data(f))?;
    let v = string(vm, &arg(args, 0))?;
    if !u.set(&part, &v) && part == "href" {
        return Err(vm.type_error(&format!("Failed to set the 'href' property on 'URL': Invalid URL '{v}'")));
    }
    set_slot(vm, g, host_ref(vm).syms.data, js(&u.href()));
    Ok(Value::Undefined)
}

fn url_search_params(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (g, _) = this_url(vm, this)?;
    let existing = slot(vm, this, host_ref(vm).syms.target);
    if existing.is_object() {
        return Ok(existing);
    }
    let p = host_ref(vm).proto("URLSearchParams").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let sp = vm.alloc(Some(p), Class::Ordinary);
    set_slot(vm, sp, host_ref(vm).syms.target, Value::Obj(g));
    set_slot(vm, g, host_ref(vm).syms.target, Value::Obj(sp));
    Ok(Value::Obj(sp))
}

fn create_object_url(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let (a, b) = (random_u64(), random_u64());
    Ok(js(&format!("blob:null/{a:016x}{b:016x}")))
}

/// A URLSearchParams's pairs: its own (a string in its slot), or its URL's query's.
fn params_of(vm: &mut Vm, this: &Value) -> Result<Vec<(String, String)>, Value> {
    let Value::Obj(_) = this else { return Err(vm.type_error("Illegal invocation")) };
    let linked = slot(vm, this, host_ref(vm).syms.target);
    if linked.is_object() {
        let (_, u) = this_url(vm, &linked)?;
        return Ok(url::form_decode(u.query.as_deref().unwrap_or("")));
    }
    match slot(vm, this, host_ref(vm).syms.data) {
        Value::Str(s) => Ok(url::form_decode(&s.to_rust())),
        _ => Err(vm.type_error("Illegal invocation")),
    }
}

fn write_params(vm: &mut Vm, this: &Value, pairs: &[(String, String)]) -> Result<(), Value> {
    let text = url::form_encode(pairs);
    let linked = slot(vm, this, host_ref(vm).syms.target);
    if linked.is_object() {
        let (g, mut u) = this_url(vm, &linked)?;
        u.query = if text.is_empty() { None } else { Some(text) };
        set_slot(vm, g, host_ref(vm).syms.data, js(&u.href()));
        return Ok(());
    }
    if let Value::Obj(g) = this {
        set_slot(vm, *g, host_ref(vm).syms.data, js(&text));
    }
    Ok(())
}

fn params_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "URLSearchParams")?;
    let g = vm.alloc(Some(p), Class::Ordinary);
    let init = arg(args, 0);
    let pairs: Vec<(String, String)> = match &init {
        Value::Undefined | Value::Null => Vec::new(),
        Value::Obj(o) => {
            let o = *o;
            let it = vm.sym(Sym::Iterator);
            let m = vm.get(o, Key::Sym(it), &init)?;
            let mut out = Vec::new();
            if vm.is_callable(&m) {
                for pair in vm.iterate_to_vec(&init)? {
                    let items = vm.iterate_to_vec(&pair)?;
                    if items.len() != 2 {
                        return Err(vm.type_error("Failed to construct 'URLSearchParams': Sequence initializer must only contain pair elements"));
                    }
                    out.push((string(vm, &items[0])?, string(vm, &items[1])?));
                }
            } else {
                for k in vm.own_keys(o)? {
                    if let Value::Str(name) = vm.key_value(k) {
                        let v = vm.get(o, k, &init)?;
                        out.push((name.to_rust(), string(vm, &v)?));
                    }
                }
            }
            out
        }
        v => {
            let s = string(vm, v)?;
            url::form_decode(&s)
        }
    };
    set_slot(vm, g, host_ref(vm).syms.data, js(&url::form_encode(&pairs)));
    Ok(Value::Obj(g))
}

fn params_append(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let mut p = params_of(vm, this)?;
    let k = str_arg(vm, args, 0, "append")?;
    let v = str_arg(vm, args, 1, "append")?;
    p.push((k, v));
    write_params(vm, this, &p)?;
    Ok(Value::Undefined)
}

fn params_delete(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let mut p = params_of(vm, this)?;
    let k = str_arg(vm, args, 0, "delete")?;
    let v = if args.len() > 1 && !args[1].is_undefined() { Some(string(vm, &args[1])?) } else { None };
    p.retain(|(a, b)| !(*a == k && v.as_ref().is_none_or(|v| v == b)));
    write_params(vm, this, &p)?;
    Ok(Value::Undefined)
}

fn params_get(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = params_of(vm, this)?;
    let k = str_arg(vm, args, 0, "get")?;
    Ok(p.iter().find(|(a, _)| *a == k).map_or(Value::Null, |(_, v)| js(v)))
}

fn params_get_all(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = params_of(vm, this)?;
    let k = str_arg(vm, args, 0, "getAll")?;
    let items: Vec<Value> = p.iter().filter(|(a, _)| *a == k).map(|(_, v)| js(v)).collect();
    Ok(Value::Obj(vm.new_array(items)))
}

fn params_has(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = params_of(vm, this)?;
    let k = str_arg(vm, args, 0, "has")?;
    let v = if args.len() > 1 && !args[1].is_undefined() { Some(string(vm, &args[1])?) } else { None };
    Ok(Value::Bool(p.iter().any(|(a, b)| *a == k && v.as_ref().is_none_or(|v| v == b))))
}

fn params_set(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let mut p = params_of(vm, this)?;
    let k = str_arg(vm, args, 0, "set")?;
    let v = str_arg(vm, args, 1, "set")?;
    match p.iter().position(|(a, _)| *a == k) {
        Some(i) => {
            p[i].1 = v;
            let mut seen = false;
            p.retain(|(a, _)| {
                if *a != k {
                    return true;
                }
                let keep = !seen;
                seen = true;
                keep
            });
        }
        None => p.push((k, v)),
    }
    write_params(vm, this, &p)?;
    Ok(Value::Undefined)
}

fn params_sort(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let mut p = params_of(vm, this)?;
    p.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
    write_params(vm, this, &p)?;
    Ok(Value::Undefined)
}

fn params_to_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let p = params_of(vm, this)?;
    Ok(js(&url::form_encode(&p)))
}

fn params_size(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let p = params_of(vm, this)?;
    Ok(Value::Num(p.len() as f64))
}

fn params_for_each(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = params_of(vm, this)?;
    let f = arg(args, 0);
    if !vm.is_callable(&f) {
        return Err(vm.type_error("forEach's argument isn't a function"));
    }
    for (k, v) in p {
        vm.call(&f, arg(args, 1), &[js(&v), js(&k), this.clone()])?;
    }
    Ok(Value::Undefined)
}

fn params_iter(vm: &mut Vm, this: &Value, which: u8) -> R {
    let p = params_of(vm, this)?;
    let items: Vec<Value> = p
        .into_iter()
        .map(|(k, v)| match which {
            0 => js(&k),
            1 => js(&v),
            _ => Value::Obj(vm.new_array(vec![js(&k), js(&v)])),
        })
        .collect();
    let a = vm.new_array(items);
    let m = get(vm, a, "values")?;
    vm.call(&m, Value::Obj(a), &[])
}

fn params_entries(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    params_iter(vm, this, 2)
}
fn params_keys(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    params_iter(vm, this, 0)
}
fn params_values(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    params_iter(vm, this, 1)
}

// ---------------------------------------------------------------------------------------- text coding

fn text_coding(vm: &mut Vm) {
    let te = interface(vm, "TextEncoder", None, 0, Some(text_encoder_new));
    vm.put(te, "encoding", js("utf-8"), ALL);
    method(vm, te, "encode", 0, encode);
    method(vm, te, "encodeInto", 2, encode_into);
    let td = interface(vm, "TextDecoder", None, 0, Some(text_decoder_new));
    method(vm, td, "decode", 0, decode);
}

fn text_encoder_new(vm: &mut Vm, nt: &Value, _args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "TextEncoder")?;
    Ok(Value::Obj(vm.alloc(Some(p), Class::Ordinary)))
}

/// A new Uint8Array holding bytes.
pub fn uint8_array(vm: &mut Vm, bytes: Vec<u8>) -> R {
    let abp = vm.proto(I::ArrayBufferProto);
    let buf = vm.alloc(Some(abp), Class::ArrayBuffer(Some(bytes), None));
    let w = host_ref(vm).window;
    let c = get(vm, w, "Uint8Array")?;
    vm.construct(&c, &[Value::Obj(buf)], &c)
}

/// A new ArrayBuffer holding bytes.
pub fn array_buffer(vm: &mut Vm, bytes: Vec<u8>) -> Value {
    let abp = vm.proto(I::ArrayBufferProto);
    Value::Obj(vm.alloc(Some(abp), Class::ArrayBuffer(Some(bytes), None)))
}

/// The bytes of an ArrayBuffer, a typed array or a DataView.
pub fn bytes_of(vm: &Vm, v: &Value) -> Option<Vec<u8>> {
    let Value::Obj(g) = v else { return None };
    let (buf, off, len) = match &vm.heap.get(*g).class {
        Class::ArrayBuffer(Some(b), _) => return Some(b.clone()),
        Class::TypedArray(t) => (t.buffer, t.offset, vm.typed_length(t) * t.kind.size()),
        Class::DataView(b, o, l) => (*b, *o, l.unwrap_or(usize::MAX)),
        _ => return None,
    };
    match &vm.heap.get(buf).class {
        Class::ArrayBuffer(Some(b), _) => {
            let end = off.saturating_add(len).min(b.len());
            Some(b[off.min(end)..end].to_vec())
        }
        _ => None,
    }
}

fn encode(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = if args.is_empty() { String::new() } else { string(vm, &args[0])? };
    uint8_array(vm, s.into_bytes())
}

fn encode_into(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = string(vm, &arg(args, 0))?;
    let dest = arg(args, 1);
    let Value::Obj(g) = dest else { return Err(vm.type_error("Failed to execute 'encodeInto' on 'TextEncoder': parameter 2 is not of type 'Uint8Array'.")) };
    let (buf, off, len) = match &vm.heap.get(g).class {
        Class::TypedArray(t) => (t.buffer, t.offset, vm.typed_length(t)),
        _ => return Err(vm.type_error("Failed to execute 'encodeInto' on 'TextEncoder': parameter 2 is not of type 'Uint8Array'.")),
    };
    let mut read = 0usize;
    let mut written = 0usize;
    let mut out: Vec<u8> = Vec::new();
    for c in s.chars() {
        let mut b = [0u8; 4];
        let e = c.encode_utf8(&mut b);
        if written + e.len() > len {
            break;
        }
        out.extend_from_slice(e.as_bytes());
        written += e.len();
        read += c.len_utf16();
    }
    if let Class::ArrayBuffer(Some(b), _) = &mut vm.heap.get_mut(buf).class {
        let end = (off + out.len()).min(b.len());
        b[off..end].copy_from_slice(&out[..end - off]);
    }
    let r = vm.new_object();
    vm.put(r, "read", Value::Num(read as f64), ALL);
    vm.put(r, "written", Value::Num(written as f64), ALL);
    Ok(Value::Obj(r))
}

fn text_decoder_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let label = match arg(args, 0) {
        Value::Undefined => String::from("utf-8"),
        v => string(vm, &v)?.trim().to_ascii_lowercase(),
    };
    let enc = match label.as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => "utf-8",
        "utf-16le" | "utf-16" => "utf-16le",
        "utf-16be" => "utf-16be",
        "latin1" | "iso-8859-1" | "windows-1252" | "ascii" | "us-ascii" => "windows-1252",
        _ => return Err(vm.range_error(&format!("Failed to construct 'TextDecoder': The encoding label provided ('{label}') is invalid."))),
    };
    let p = proto_from(vm, nt, "TextDecoder")?;
    let g = vm.alloc(Some(p), Class::Ordinary);
    vm.put(g, "encoding", js(enc), ALL);
    let fatal = match arg(args, 1) {
        Value::Obj(o) => get(vm, o, "fatal")?.truthy(),
        _ => false,
    };
    vm.put(g, "fatal", Value::Bool(fatal), ALL);
    vm.put(g, "ignoreBOM", Value::Bool(false), ALL);
    Ok(Value::Obj(g))
}

fn decode(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(g) = this else { return Err(vm.type_error("Illegal invocation")) };
    let enc = get(vm, *g, "encoding")?;
    let enc = string(vm, &enc)?;
    let fatal = get(vm, *g, "fatal")?.truthy();
    let input = arg(args, 0);
    let bytes = if input.is_undefined() {
        Vec::new()
    } else {
        bytes_of(vm, &input).ok_or_else(|| vm.type_error("Failed to execute 'decode' on 'TextDecoder': The provided value is not of type '(ArrayBuffer or ArrayBufferView)'."))?
    };
    let units: Vec<u16> = match enc.as_str() {
        "utf-16le" => bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect(),
        "utf-16be" => bytes.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect(),
        "windows-1252" => bytes.iter().map(|&b| b as u16).collect(),
        _ => {
            let b = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
            match core::str::from_utf8(b) {
                Ok(s) => s.encode_utf16().collect(),
                Err(_) if fatal => return Err(vm.type_error("Failed to execute 'decode' on 'TextDecoder': The encoded data was not valid.")),
                Err(_) => String::from_utf8_lossy(b).encode_utf16().collect(),
            }
        }
    };
    Ok(Value::Str(JsStr::from_units(units)))
}

// ----------------------------------------------------------------------------------- abort controllers

fn abort(vm: &mut Vm) {
    let sp = interface(vm, "AbortSignal", Some("EventTarget"), 0, None);
    attribute(vm, sp, "aborted", signal_aborted, None);
    attribute(vm, sp, "reason", signal_reason, None);
    method(vm, sp, "throwIfAborted", 0, throw_if_aborted);
    super::events::handler_property(vm, sp, "abort");
    if let Some(Value::Obj(c)) = super::events::ctor_of(vm, sp) {
        method(vm, c, "abort", 1, signal_abort_static);
        method(vm, c, "timeout", 1, signal_timeout);
        method(vm, c, "any", 1, signal_any);
    }
    let cp = interface(vm, "AbortController", None, 0, Some(controller_new));
    attribute(vm, cp, "signal", controller_signal, None);
    method(vm, cp, "abort", 0, controller_abort);
}

fn new_signal(vm: &mut Vm) -> Gc {
    let p = host_ref(vm).proto("AbortSignal").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let s = vm.alloc(Some(p), Class::Ordinary);
    set_slot(vm, s, host_ref(vm).syms.data, Value::Bool(false));
    s
}

fn signal_aborted(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(slot(vm, this, host_ref(vm).syms.data).truthy()))
}

fn signal_reason(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(slot(vm, this, host_ref(vm).syms.target))
}

fn throw_if_aborted(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    if slot(vm, this, host_ref(vm).syms.data).truthy() {
        return Err(slot(vm, this, host_ref(vm).syms.target));
    }
    Ok(Value::Undefined)
}

/// Aborts a signal (once): its reason kept, its abort event fired.
pub fn abort_signal(vm: &mut Vm, s: Gc, reason: Value) {
    if slot(vm, &Value::Obj(s), host_ref(vm).syms.data).truthy() {
        return;
    }
    let reason = if reason.is_undefined() { dom_exception(vm, "AbortError", "signal is aborted without reason") } else { reason };
    set_slot(vm, s, host_ref(vm).syms.data, Value::Bool(true));
    set_slot(vm, s, host_ref(vm).syms.target, reason);
    super::events::fire(vm, &Value::Obj(s), &EventInit::new("abort", "Event", false, false));
}

fn signal_abort_static(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = new_signal(vm);
    abort_signal(vm, s, arg(args, 0));
    Ok(Value::Obj(s))
}

fn signal_timeout(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = new_signal(vm);
    let f = vm.native_with("", 0, timeout_fire, None, Value::Obj(s));
    add_timer(vm, &[Value::Obj(f), arg(args, 0)], false)?;
    Ok(Value::Obj(s))
}

fn timeout_fire(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    if let Value::Obj(s) = vm.data(f) {
        let e = dom_exception(vm, "TimeoutError", "signal timed out");
        abort_signal(vm, s, e);
    }
    Ok(Value::Undefined)
}

fn signal_any(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = new_signal(vm);
    let list = vm.iterate_to_vec(&arg(args, 0))?;
    for other in list {
        if slot(vm, &other, host_ref(vm).syms.data).truthy() {
            let r = slot(vm, &other, host_ref(vm).syms.target);
            abort_signal(vm, s, r);
            break;
        }
        if let Value::Obj(o) = other {
            let f = vm.native_with("", 0, follow_abort, None, Value::Obj(s));
            vm.set_data2(f, Value::Obj(o));
            host(vm).listeners.push(Listener { target: o, ty: String::from("abort"), callback: Value::Obj(f), capture: false, once: true, passive: false, removed: false });
        }
    }
    Ok(Value::Obj(s))
}

fn follow_abort(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    if let (Value::Obj(s), o) = (vm.data(f), vm.data2(f)) {
        let r = slot(vm, &o, host_ref(vm).syms.target);
        abort_signal(vm, s, r);
    }
    Ok(Value::Undefined)
}

fn controller_new(vm: &mut Vm, nt: &Value, _args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "AbortController")?;
    let c = vm.alloc(Some(p), Class::Ordinary);
    let s = new_signal(vm);
    set_slot(vm, c, host_ref(vm).syms.target, Value::Obj(s));
    Ok(Value::Obj(c))
}

fn controller_signal(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(slot(vm, this, host_ref(vm).syms.target))
}

fn controller_abort(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if let Value::Obj(s) = slot(vm, this, host_ref(vm).syms.target) {
        abort_signal(vm, s, arg(args, 0));
    }
    Ok(Value::Undefined)
}

// --------------------------------------------------------------------------------- blobs and form data

fn blobs(vm: &mut Vm) {
    let bp = interface(vm, "Blob", None, 0, Some(blob_new));
    attribute(vm, bp, "size", blob_size, None);
    attribute(vm, bp, "type", blob_type, None);
    method(vm, bp, "text", 0, blob_text);
    method(vm, bp, "arrayBuffer", 0, blob_array_buffer);
    method(vm, bp, "bytes", 0, blob_bytes);
    method(vm, bp, "slice", 0, blob_slice);
    let fp = interface(vm, "File", Some("Blob"), 2, Some(file_new));
    let _ = fp;
    let fd = interface(vm, "FormData", None, 0, Some(form_data_new));
    for (name, len, f) in [
        ("append", 2, form_data_append as NativeFn),
        ("set", 2, form_data_set),
        ("delete", 1, form_data_delete),
        ("get", 1, form_data_get),
        ("getAll", 1, form_data_get_all),
        ("has", 1, form_data_has),
        ("entries", 0, form_data_entries),
        ("keys", 0, form_data_keys),
        ("values", 0, form_data_values),
        ("forEach", 1, form_data_for_each),
    ] {
        method(vm, fd, name, len, f);
    }
    let entries = get(vm, fd, "entries").unwrap_or(Value::Undefined);
    let k = Key::Sym(vm.sym(Sym::Iterator));
    vm.put_key(fd, k, entries, HIDDEN);
}

/// A blob's bytes and type.
pub fn blob_parts(vm: &Vm, v: &Value) -> Option<(Vec<u8>, String)> {
    let b = slot(vm, v, host_ref(vm).syms.data);
    let t = slot(vm, v, host_ref(vm).syms.target);
    match (bytes_of(vm, &b), t) {
        (Some(bytes), Value::Str(t)) => Some((bytes, t.to_rust())),
        _ => None,
    }
}

fn make_blob(vm: &mut Vm, proto: Gc, bytes: Vec<u8>, ty: &str) -> Gc {
    let g = vm.alloc(Some(proto), Class::Ordinary);
    let buf = array_buffer(vm, bytes);
    set_slot(vm, g, host_ref(vm).syms.data, buf);
    set_slot(vm, g, host_ref(vm).syms.target, js(&ty.to_ascii_lowercase()));
    g
}

/// A blob's bytes from its parts (strings, buffers, other blobs).
fn blob_bytes_from(vm: &mut Vm, parts: &Value) -> Result<Vec<u8>, Value> {
    let mut out = Vec::new();
    if parts.is_undefined() {
        return Ok(out);
    }
    for p in vm.iterate_to_vec(parts)? {
        if let Some((b, _)) = blob_parts(vm, &p) {
            out.extend(b);
        } else if let Some(b) = bytes_of(vm, &p) {
            out.extend(b);
        } else {
            out.extend(string(vm, &p)?.into_bytes());
        }
    }
    Ok(out)
}

fn blob_type_opt(vm: &mut Vm, o: &Value) -> Result<String, Value> {
    match o {
        Value::Obj(g) => {
            let t = get(vm, *g, "type")?;
            if t.is_undefined() { Ok(String::new()) } else { string(vm, &t) }
        }
        _ => Ok(String::new()),
    }
}

fn blob_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let bytes = blob_bytes_from(vm, &arg(args, 0))?;
    let ty = blob_type_opt(vm, &arg(args, 1))?;
    let p = proto_from(vm, nt, "Blob")?;
    Ok(Value::Obj(make_blob(vm, p, bytes, &ty)))
}

fn file_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let bytes = blob_bytes_from(vm, &arg(args, 0))?;
    let name = str_arg(vm, args, 1, "File")?;
    let ty = blob_type_opt(vm, &arg(args, 2))?;
    let p = proto_from(vm, nt, "File")?;
    let g = make_blob(vm, p, bytes, &ty);
    vm.put(g, "name", js(&name), ALL);
    let now = host_ref(vm).epoch + host_ref(vm).now;
    vm.put(g, "lastModified", Value::Num(js::math::floor(now)), ALL);
    Ok(Value::Obj(g))
}

fn this_blob(vm: &mut Vm, this: &Value) -> Result<(Vec<u8>, String), Value> {
    blob_parts(vm, this).ok_or_else(|| vm.type_error("Illegal invocation"))
}

fn blob_size(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (b, _) = this_blob(vm, this)?;
    Ok(Value::Num(b.len() as f64))
}

fn blob_type(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, t) = this_blob(vm, this)?;
    Ok(js(&t))
}

fn blob_text(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (b, _) = this_blob(vm, this)?;
    let s = String::from_utf8_lossy(&b).into_owned();
    Ok(resolved(vm, js(&s)))
}

fn blob_array_buffer(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (b, _) = this_blob(vm, this)?;
    let buf = array_buffer(vm, b);
    Ok(resolved(vm, buf))
}

fn blob_bytes(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (b, _) = this_blob(vm, this)?;
    let a = uint8_array(vm, b)?;
    Ok(resolved(vm, a))
}

fn blob_slice(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (b, t) = this_blob(vm, this)?;
    let len = b.len() as u64;
    let start = vm.relative_index(&arg(args, 0), len, 0)? as usize;
    let end = vm.relative_index(&arg(args, 1), len, len)? as usize;
    let ty = match arg(args, 2) {
        Value::Undefined => t,
        v => string(vm, &v)?,
    };
    let p = host_ref(vm).proto("Blob").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let part = b[start.min(end)..end].to_vec();
    Ok(Value::Obj(make_blob(vm, p, part, &ty)))
}

/// A form data's entries (kept as an array of [name, value] arrays in its slot).
fn form_entries(vm: &mut Vm, this: &Value) -> Result<Gc, Value> {
    match slot(vm, this, host_ref(vm).syms.data) {
        Value::Obj(a) => Ok(a),
        _ => Err(vm.type_error("Illegal invocation")),
    }
}

fn form_data_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "FormData")?;
    let g = vm.alloc(Some(p), Class::Ordinary);
    let mut items: Vec<Value> = Vec::new();
    if let Some(form) = node_of(vm, &arg(args, 0)) {
        for (k, v) in form_set(vm, form) {
            items.push(Value::Obj(vm.new_array(vec![js(&k), js(&v)])));
        }
    }
    let a = vm.new_array(items);
    set_slot(vm, g, host_ref(vm).syms.data, Value::Obj(a));
    Ok(Value::Obj(g))
}

/// A form's successful controls' names and values (what sending it would send).
pub fn form_set(vm: &mut Vm, form: NodeId) -> Vec<(String, String)> {
    let d = &host_ref(vm).doc;
    let mut out = Vec::new();
    for c in super::lists::controls(d, form) {
        let Some(e) = d.element(c) else { continue };
        let Some(name) = e.attr("name").filter(|n| !n.is_empty()) else { continue };
        if e.has_attr("disabled") {
            continue;
        }
        let ty = e.attr("type").unwrap_or("").to_ascii_lowercase();
        match d.name(c) {
            "input" => {
                if matches!(ty.as_str(), "submit" | "button" | "reset" | "file" | "image") {
                    continue;
                }
                if matches!(ty.as_str(), "checkbox" | "radio") {
                    if e.has_attr("checked") {
                        out.push((String::from(name), String::from(e.attr("value").unwrap_or("on"))));
                    }
                    continue;
                }
                out.push((String::from(name), String::from(e.attr("value").unwrap_or(""))));
            }
            "textarea" => out.push((String::from(name), e.attr("value").map(String::from).unwrap_or_else(|| d.text_of(c)))),
            "select" => {
                let opts: Vec<NodeId> = d.descendants(c).filter(|&o| d.name(o) == "option").collect();
                let chosen = opts.iter().copied().find(|&o| d.element(o).is_some_and(|x| x.has_attr("selected"))).or_else(|| opts.first().copied());
                if let Some(o) = chosen {
                    let v = d.element(o).and_then(|x| x.attr("value")).map(String::from).unwrap_or_else(|| d.text_of(o).trim().into());
                    out.push((String::from(name), v));
                }
            }
            _ => {}
        }
    }
    out
}

fn entry_value(vm: &mut Vm, v: &Value) -> Result<Value, Value> {
    if blob_parts(vm, v).is_some() {
        return Ok(v.clone());
    }
    Ok(Value::Str(vm.to_string(v)?))
}

fn form_data_append(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let a = form_entries(vm, this)?;
    let k = str_arg(vm, args, 0, "append")?;
    let v = entry_value(vm, &arg(args, 1))?;
    let pair = vm.new_array(vec![js(&k), v]);
    let mut items = vm.array_values(&Value::Obj(a));
    items.push(Value::Obj(pair));
    let na = vm.new_array(items);
    if let Value::Obj(g) = this {
        set_slot(vm, *g, host_ref(vm).syms.data, Value::Obj(na));
    }
    Ok(Value::Undefined)
}

fn form_pairs(vm: &mut Vm, this: &Value) -> Result<Vec<(String, Value)>, Value> {
    let a = form_entries(vm, this)?;
    let mut out = Vec::new();
    for item in vm.array_values(&Value::Obj(a)) {
        let parts = vm.array_values(&item);
        let k = string(vm, parts.first().unwrap_or(&Value::Undefined))?;
        out.push((k, parts.get(1).cloned().unwrap_or(Value::Undefined)));
    }
    Ok(out)
}

fn write_form_pairs(vm: &mut Vm, this: &Value, pairs: Vec<(String, Value)>) {
    let items: Vec<Value> = pairs.into_iter().map(|(k, v)| Value::Obj(vm.new_array(vec![js(&k), v]))).collect();
    let a = vm.new_array(items);
    if let Value::Obj(g) = this {
        set_slot(vm, *g, host_ref(vm).syms.data, Value::Obj(a));
    }
}

fn form_data_set(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let mut p = form_pairs(vm, this)?;
    let k = str_arg(vm, args, 0, "set")?;
    let v = entry_value(vm, &arg(args, 1))?;
    match p.iter().position(|(a, _)| *a == k) {
        Some(i) => {
            p[i].1 = v;
            let mut seen = 0;
            p.retain(|(a, _)| {
                if *a != k {
                    return true;
                }
                seen += 1;
                seen == 1
            });
        }
        None => p.push((k, v)),
    }
    write_form_pairs(vm, this, p);
    Ok(Value::Undefined)
}

fn form_data_delete(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let mut p = form_pairs(vm, this)?;
    let k = str_arg(vm, args, 0, "delete")?;
    p.retain(|(a, _)| *a != k);
    write_form_pairs(vm, this, p);
    Ok(Value::Undefined)
}

fn form_data_get(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = form_pairs(vm, this)?;
    let k = str_arg(vm, args, 0, "get")?;
    Ok(p.into_iter().find(|(a, _)| *a == k).map_or(Value::Null, |(_, v)| v))
}

fn form_data_get_all(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = form_pairs(vm, this)?;
    let k = str_arg(vm, args, 0, "getAll")?;
    let items: Vec<Value> = p.into_iter().filter(|(a, _)| *a == k).map(|(_, v)| v).collect();
    Ok(Value::Obj(vm.new_array(items)))
}

fn form_data_has(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = form_pairs(vm, this)?;
    let k = str_arg(vm, args, 0, "has")?;
    Ok(Value::Bool(p.iter().any(|(a, _)| *a == k)))
}

fn form_data_iter(vm: &mut Vm, this: &Value, which: u8) -> R {
    let p = form_pairs(vm, this)?;
    let items: Vec<Value> = p
        .into_iter()
        .map(|(k, v)| match which {
            0 => js(&k),
            1 => v,
            _ => Value::Obj(vm.new_array(vec![js(&k), v])),
        })
        .collect();
    let a = vm.new_array(items);
    let m = get(vm, a, "values")?;
    vm.call(&m, Value::Obj(a), &[])
}

fn form_data_entries(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    form_data_iter(vm, this, 2)
}
fn form_data_keys(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    form_data_iter(vm, this, 0)
}
fn form_data_values(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    form_data_iter(vm, this, 1)
}

fn form_data_for_each(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = form_pairs(vm, this)?;
    let f = arg(args, 0);
    if !vm.is_callable(&f) {
        return Err(vm.type_error("forEach's argument isn't a function"));
    }
    for (k, v) in p {
        vm.call(&f, arg(args, 1), &[v, js(&k), this.clone()])?;
    }
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------------------------------ observers

fn observers(vm: &mut Vm) {
    for (name, ctor) in [("IntersectionObserver", intersection_new as NativeFn), ("ResizeObserver", resize_new)] {
        let p = interface(vm, name, None, 1, Some(ctor));
        method(vm, p, "observe", 1, observer_observe);
        method(vm, p, "unobserve", 1, ignore);
        method(vm, p, "disconnect", 0, ignore);
        method(vm, p, "takeRecords", 0, empty_array);
        vm.put(p, "root", Value::Null, ALL);
        vm.put(p, "rootMargin", js("0px 0px 0px 0px"), ALL);
        let t = vm.new_array(vec![Value::Num(0.0)]);
        vm.put(p, "thresholds", Value::Obj(t), ALL);
    }
    interface(vm, "IntersectionObserverEntry", None, 0, None);
    interface(vm, "ResizeObserverEntry", None, 0, None);
    let mq = interface(vm, "MediaQueryList", Some("EventTarget"), 0, None);
    method(vm, mq, "addListener", 1, ignore);
    method(vm, mq, "removeListener", 1, ignore);
}

fn intersection_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    observer_new(vm, nt, args, "IntersectionObserver")
}

fn resize_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    observer_new(vm, nt, args, "ResizeObserver")
}

fn observer_new(vm: &mut Vm, nt: &Value, args: &[Value], iface: &str) -> R {
    let cb = arg(args, 0);
    if !vm.is_callable(&cb) {
        return Err(vm.type_error(&format!("Failed to construct '{iface}': parameter 1 is not of type 'Function'.")));
    }
    let p = proto_from(vm, nt, iface)?;
    let g = vm.alloc(Some(p), Class::Ordinary);
    set_slot(vm, g, host_ref(vm).syms.data, cb);
    set_slot(vm, g, host_ref(vm).syms.target, js(iface));
    Ok(Value::Obj(g))
}

/// observe(): the observer told soon what it sees of the element (as if all of it were in view, and its box as it
/// was laid out).
fn observer_observe(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Some(n) = node_of(vm, &arg(args, 0)) else { return Err(vm.type_error("Failed to execute 'observe': parameter 1 is not of type 'Element'.")) };
    let cb = slot(vm, this, host_ref(vm).syms.data);
    let kind = string(vm, &slot(vm, this, host_ref(vm).syms.target))?;
    let target = arg(args, 0);
    super::fresh_layout(vm);
    let r = host_ref(vm).rects.get(n).copied().flatten().unwrap_or((0.0, 0.0, 0.0, 0.0));
    let (sx, sy) = host_ref(vm).scroll;
    let e = vm.new_object();
    vm.put(e, "target", target, ALL);
    if kind == "ResizeObserver" {
        let rect = super::dom::dom_rect(vm, 0.0, 0.0, r.2 as f64, r.3 as f64);
        vm.put(e, "contentRect", rect, ALL);
        let size = vm.new_object();
        vm.put(size, "inlineSize", Value::Num(r.2 as f64), ALL);
        vm.put(size, "blockSize", Value::Num(r.3 as f64), ALL);
        for name in ["borderBoxSize", "contentBoxSize", "devicePixelContentBoxSize"] {
            let a = vm.new_array(vec![Value::Obj(size)]);
            vm.put(e, name, Value::Obj(a), ALL);
        }
    } else {
        let rect = super::dom::dom_rect(vm, (r.0 - sx) as f64, (r.1 - sy) as f64, r.2 as f64, r.3 as f64);
        vm.put(e, "boundingClientRect", rect.clone(), ALL);
        vm.put(e, "intersectionRect", rect, ALL);
        let (vw, vh, _) = host_ref(vm).view;
        let root = super::dom::dom_rect(vm, 0.0, 0.0, vw as f64, vh as f64);
        vm.put(e, "rootBounds", root, ALL);
        vm.put(e, "isIntersecting", Value::Bool(true), ALL);
        vm.put(e, "isVisible", Value::Bool(true), ALL);
        vm.put(e, "intersectionRatio", Value::Num(1.0), ALL);
        let now = host_ref(vm).now;
        vm.put(e, "time", Value::Num(now), ALL);
    }
    let entries = vm.new_array(vec![Value::Obj(e)]);
    host(vm).tasks.push((cb, this.clone(), vec![Value::Obj(entries), this.clone()]));
    Ok(Value::Undefined)
}

// --------------------------------------------------------------------------------------------- the rest

fn misc(vm: &mut Vm, w: Gc) {
    // CSS.supports, CSS.escape.
    let css = vm.new_object();
    method(vm, css, "supports", 1, css_supports);
    method(vm, css, "escape", 1, css_escape);
    method(vm, css, "registerProperty", 1, ignore);
    vm.put(w, "CSS", Value::Obj(css), HIDDEN);
    // Notifications: never allowed here.
    let np = interface(vm, "Notification", Some("EventTarget"), 1, None);
    if let Some(Value::Obj(c)) = super::events::ctor_of(vm, np) {
        vm.put(c, "permission", js("denied"), ALL);
        method(vm, c, "requestPermission", 0, permission_denied);
    }
    // MessageChannel: two ports that post to each other.
    let mc = interface(vm, "MessageChannel", None, 0, Some(channel_new));
    let _ = mc;
    let mp = interface(vm, "MessagePort", Some("EventTarget"), 0, None);
    method(vm, mp, "postMessage", 1, port_post);
    method(vm, mp, "start", 0, ignore);
    method(vm, mp, "close", 0, ignore);
    super::events::handler_property(vm, mp, "message");
    let bc = interface(vm, "BroadcastChannel", Some("EventTarget"), 1, Some(broadcast_new));
    method(vm, bc, "postMessage", 1, ignore);
    method(vm, bc, "close", 0, ignore);
    interface(vm, "Worker", Some("EventTarget"), 1, Some(no_worker));
    interface(vm, "SharedWorker", Some("EventTarget"), 1, Some(no_worker));
    interface(vm, "WebSocket", Some("EventTarget"), 1, Some(no_socket));
    interface(vm, "EventSource", Some("EventTarget"), 1, Some(no_socket));
}

fn css_supports(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let a = string(vm, &arg(args, 0))?;
    if args.len() > 1 {
        let p = a.trim().to_ascii_lowercase();
        return Ok(Value::Bool(p.starts_with("--") || super::lists::known_property(&p)));
    }
    // "(display: grid)": the property known.
    let t = a.trim().trim_start_matches('(');
    let p = t.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
    Ok(Value::Bool(!p.is_empty() && (p.starts_with("--") || super::lists::known_property(&p))))
}

fn css_escape(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = str_arg(vm, args, 0, "escape")?;
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        match c {
            '\0' => out.push('\u{fffd}'),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\{:x} ", c as u32)),
            c if i == 0 && c.is_ascii_digit() => out.push_str(&format!("\\{:x} ", c as u32)),
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || (c as u32) >= 0x80 => out.push(c),
            c => {
                out.push('\\');
                out.push(c);
            }
        }
    }
    Ok(js(&out))
}

fn permission_denied(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(resolved(vm, js("denied")))
}

fn channel_new(vm: &mut Vm, nt: &Value, _args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "MessageChannel")?;
    let c = vm.alloc(Some(p), Class::Ordinary);
    let pp = host_ref(vm).proto("MessagePort").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let a = vm.alloc(Some(pp), Class::Ordinary);
    let b = vm.alloc(Some(pp), Class::Ordinary);
    set_slot(vm, a, host_ref(vm).syms.target, Value::Obj(b));
    set_slot(vm, b, host_ref(vm).syms.target, Value::Obj(a));
    vm.put(c, "port1", Value::Obj(a), ALL);
    vm.put(c, "port2", Value::Obj(b), ALL);
    Ok(Value::Obj(c))
}

/// A port's message, to the other port's listeners (soon).
fn port_post(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let other = slot(vm, this, host_ref(vm).syms.target);
    let data = structured_clone_value(vm, &arg(args, 0))?;
    let f = vm.native_with("", 0, deliver_message, None, other.clone());
    vm.set_data2(f, data);
    host(vm).tasks.push((Value::Obj(f), other, Vec::new()));
    Ok(Value::Undefined)
}

fn deliver_message(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    let target = vm.data(f);
    let data = vm.data2(f);
    let proto = host_ref(vm).proto("MessageEvent").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let w = host_ref(vm).window;
    let c = get(vm, w, "MessageEvent")?;
    let init = vm.new_object();
    vm.put(init, "data", data, ALL);
    let origin = host_ref(vm).url.origin();
    vm.put(init, "origin", js(&origin), ALL);
    if js::value::strict_equals(&target, &Value::Obj(w)) {
        vm.put(init, "source", Value::Obj(w), ALL);
    }
    let e = vm.construct(&c, &[js("message"), Value::Obj(init)], &c)?;
    let _ = proto;
    if let Value::Obj(eg) = e {
        super::events::dispatch(vm, &target, eg);
    }
    Ok(Value::Undefined)
}

fn post_message(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let w = Value::Obj(host_ref(vm).window);
    let data = structured_clone_value(vm, &arg(args, 0))?;
    let f = vm.native_with("", 0, deliver_message, None, w.clone());
    vm.set_data2(f, data);
    host(vm).tasks.push((Value::Obj(f), w, Vec::new()));
    Ok(Value::Undefined)
}

fn broadcast_new(vm: &mut Vm, nt: &Value, _args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "BroadcastChannel")?;
    Ok(Value::Obj(vm.alloc(Some(p), Class::Ordinary)))
}

fn no_worker(vm: &mut Vm, _nt: &Value, _args: &[Value], _f: Gc) -> R {
    Err(dom_exception(vm, "NotSupportedError", "Workers aren't supported by this browser yet."))
}

fn no_socket(vm: &mut Vm, _nt: &Value, _args: &[Value], _f: Gc) -> R {
    Err(dom_exception(vm, "NotSupportedError", "This browser can't open a connection of this kind yet."))
}

fn structured_clone(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    structured_clone_value(vm, &arg(args, 0))
}

/// A deep copy of a value as structuredClone and postMessage make it.
pub fn structured_clone_value(vm: &mut Vm, v: &Value) -> R {
    let mut seen: Vec<(Gc, Gc)> = Vec::new();
    clone_in(vm, v, &mut seen, 0)
}

fn clone_in(vm: &mut Vm, v: &Value, seen: &mut Vec<(Gc, Gc)>, depth: u32) -> R {
    let Value::Obj(g) = v else {
        if let Value::Sym(_) = v {
            return Err(dom_exception(vm, "DataCloneError", "Symbol() could not be cloned."));
        }
        return Ok(v.clone());
    };
    let g = *g;
    if let Some((_, c)) = seen.iter().find(|(o, _)| *o == g) {
        return Ok(Value::Obj(*c));
    }
    if depth > 200 {
        return Err(dom_exception(vm, "DataCloneError", "The object is too deep to clone."));
    }
    let op = vm.proto(I::ObjectProto);
    let copy = match &vm.heap.get(g).class {
        Class::Ordinary | Class::Error => {
            let c = vm.alloc(Some(op), Class::Ordinary);
            seen.push((g, c));
            for k in vm.own_keys_plain(g) {
                if let Key::Sym(_) = k {
                    continue;
                }
                let Some(Prop { slot: Slot::Data(x), flags }) = vm.heap.get(g).props.get(&k).cloned() else { continue };
                if flags & ENUMERABLE == 0 && !matches!(vm.heap.get(g).class, Class::Error) {
                    continue;
                }
                let xc = clone_in(vm, &x, seen, depth + 1)?;
                vm.put_key(c, k, xc, ALL);
            }
            return Ok(Value::Obj(c));
        }
        Class::Array(..) => {
            let items = vm.array_values(v);
            let c = vm.new_array(Vec::new());
            seen.push((g, c));
            let mut out = Vec::new();
            for x in items {
                out.push(clone_in(vm, &x, seen, depth + 1)?);
            }
            let na = vm.new_array(out);
            // (Its place in `seen` is the array made first; copy the items into it.)
            let vals = vm.array_values(&Value::Obj(na));
            if let Class::Array(els, len) = &mut vm.heap.get_mut(c).class {
                *len = vals.len() as u32;
                *els = vals;
            }
            return Ok(Value::Obj(c));
        }
        Class::Date(t) => {
            let t = *t;
            let p = vm.proto(I::DateProto);
            vm.alloc(Some(p), Class::Date(t))
        }
        Class::Boolean(b) => {
            let b = *b;
            let p = vm.proto(I::BooleanProto);
            vm.alloc(Some(p), Class::Boolean(b))
        }
        Class::Number(x) => {
            let x = *x;
            let p = vm.proto(I::NumberProto);
            vm.alloc(Some(p), Class::Number(x))
        }
        Class::String(s) => {
            let s = s.clone();
            let p = vm.proto(I::StringProto);
            vm.alloc(Some(p), Class::String(s))
        }
        Class::ArrayBuffer(Some(b), _) => {
            let b = b.clone();
            return Ok(array_buffer(vm, b));
        }
        Class::TypedArray(_) | Class::DataView(..) => {
            // A copy of the view (with its own buffer).
            let ctor = get(vm, g, "constructor")?;
            let bytes = bytes_of(vm, v).unwrap_or_default();
            let buf = array_buffer(vm, bytes);
            return vm.construct(&ctor, &[buf], &ctor);
        }
        Class::Map(_) | Class::Set(_) => {
            let is_map = matches!(vm.heap.get(g).class, Class::Map(_));
            let w = host_ref(vm).window;
            let ctor = get(vm, w, if is_map { "Map" } else { "Set" })?;
            let c = vm.construct(&ctor, &[], &ctor)?;
            let Value::Obj(cg) = c else { return Ok(c) };
            seen.push((g, cg));
            let entries = vm.iterate_to_vec(v)?;
            for e in entries {
                if is_map {
                    let kv = vm.array_values(&e);
                    let k = clone_in(vm, kv.first().unwrap_or(&Value::Undefined), seen, depth + 1)?;
                    let x = clone_in(vm, kv.get(1).unwrap_or(&Value::Undefined), seen, depth + 1)?;
                    let set = get(vm, cg, "set")?;
                    vm.call(&set, c.clone(), &[k, x])?;
                } else {
                    let x = clone_in(vm, &e, seen, depth + 1)?;
                    let add = get(vm, cg, "add")?;
                    vm.call(&add, c.clone(), &[x])?;
                }
            }
            return Ok(c);
        }
        Class::RegExp(_) => {
            let src = get(vm, g, "source")?;
            let flags = get(vm, g, "flags")?;
            let w = host_ref(vm).window;
            let ctor = get(vm, w, "RegExp")?;
            return vm.construct(&ctor, &[src, flags], &ctor);
        }
        _ => {
            if let Some((b, t)) = blob_parts(vm, v) {
                let p = host_ref(vm).proto("Blob").unwrap_or(op);
                return Ok(Value::Obj(make_blob(vm, p, b, &t)));
            }
            let what = vm.describe(v);
            return Err(dom_exception(vm, "DataCloneError", &format!("{what} could not be cloned.")));
        }
    };
    seen.push((g, copy));
    Ok(Value::Obj(copy))
}
