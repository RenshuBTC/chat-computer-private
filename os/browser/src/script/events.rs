//! Events (the DOM standard's): EventTarget's listeners, Event and its kinds (MouseEvent, KeyboardEvent,
//! CustomEvent...), an event sent through the tree (capturing down from the window, at its target, bubbling back
//! up), handlers set as properties or attributes (onclick), and mutation observers.

use super::*;
use crate::dom::NodeId;
use alloc::vec;
use js::object::ALL;

// An event's state, kept in its hidden object's flags.
const BUBBLES: u32 = 1;
const CANCELABLE: u32 = 2;
const COMPOSED: u32 = 4;
const CANCELED: u32 = 8;
const STOP: u32 = 16;
const STOP_NOW: u32 = 32;
const DISPATCHING: u32 = 64;
const IN_PASSIVE: u32 = 128;
const TRUSTED: u32 = 256;

/// What the page's own events carry (a click's place, a key...).
#[derive(Clone, Debug)]
pub enum EvValue {
    Num(f64),
    Bool(bool),
    Str(String),
    Node(Option<NodeId>),
}

/// An event the page fires (not a script's).
#[derive(Clone, Debug)]
pub struct EventInit {
    pub ty: String,
    /// Its interface ("Event", "MouseEvent", "KeyboardEvent", "FocusEvent", "InputEvent").
    pub interface: &'static str,
    pub bubbles: bool,
    pub cancelable: bool,
    pub props: Vec<(&'static str, EvValue)>,
}

impl EventInit {
    pub fn new(ty: &str, interface: &'static str, bubbles: bool, cancelable: bool) -> EventInit {
        EventInit { ty: String::from(ty), interface, bubbles, cancelable, props: Vec::new() }
    }

    pub fn with(mut self, name: &'static str, v: EvValue) -> EventInit {
        self.props.push((name, v));
        self
    }
}

/// The interfaces of events, their parents and the members their dictionaries add (with defaults).
const KINDS: &[(&str, &str, &[(&str, Def)])] = &[
    ("UIEvent", "Event", &[("view", Def::Null), ("detail", Def::Num(0.0)), ("which", Def::Num(0.0))]),
    ("MouseEvent", "UIEvent", &[
        ("screenX", Def::Num(0.0)), ("screenY", Def::Num(0.0)), ("clientX", Def::Num(0.0)), ("clientY", Def::Num(0.0)),
        ("ctrlKey", Def::Bool(false)), ("shiftKey", Def::Bool(false)), ("altKey", Def::Bool(false)), ("metaKey", Def::Bool(false)),
        ("button", Def::Num(0.0)), ("buttons", Def::Num(0.0)), ("relatedTarget", Def::Null), ("movementX", Def::Num(0.0)),
        ("movementY", Def::Num(0.0)),
    ]),
    ("PointerEvent", "MouseEvent", &[
        ("pointerId", Def::Num(1.0)), ("width", Def::Num(1.0)), ("height", Def::Num(1.0)), ("pressure", Def::Num(0.0)),
        ("tiltX", Def::Num(0.0)), ("tiltY", Def::Num(0.0)), ("pointerType", Def::Str("mouse")), ("isPrimary", Def::Bool(true)),
    ]),
    ("WheelEvent", "MouseEvent", &[("deltaX", Def::Num(0.0)), ("deltaY", Def::Num(0.0)), ("deltaZ", Def::Num(0.0)), ("deltaMode", Def::Num(0.0))]),
    ("KeyboardEvent", "UIEvent", &[
        ("key", Def::Str("")), ("code", Def::Str("")), ("location", Def::Num(0.0)), ("repeat", Def::Bool(false)),
        ("isComposing", Def::Bool(false)), ("ctrlKey", Def::Bool(false)), ("shiftKey", Def::Bool(false)),
        ("altKey", Def::Bool(false)), ("metaKey", Def::Bool(false)), ("keyCode", Def::Num(0.0)), ("charCode", Def::Num(0.0)),
    ]),
    ("FocusEvent", "UIEvent", &[("relatedTarget", Def::Null)]),
    ("InputEvent", "UIEvent", &[("data", Def::Null), ("inputType", Def::Str("")), ("isComposing", Def::Bool(false))]),
    ("CompositionEvent", "UIEvent", &[("data", Def::Str(""))]),
    ("TouchEvent", "UIEvent", &[("touches", Def::Null), ("targetTouches", Def::Null), ("changedTouches", Def::Null)]),
    ("CustomEvent", "Event", &[("detail", Def::Null)]),
    ("MessageEvent", "Event", &[("data", Def::Null), ("origin", Def::Str("")), ("lastEventId", Def::Str("")), ("source", Def::Null), ("ports", Def::Null)]),
    ("CloseEvent", "Event", &[("wasClean", Def::Bool(false)), ("code", Def::Num(0.0)), ("reason", Def::Str(""))]),
    ("ErrorEvent", "Event", &[("message", Def::Str("")), ("filename", Def::Str("")), ("lineno", Def::Num(0.0)), ("colno", Def::Num(0.0)), ("error", Def::Null)]),
    ("ProgressEvent", "Event", &[("lengthComputable", Def::Bool(false)), ("loaded", Def::Num(0.0)), ("total", Def::Num(0.0))]),
    ("PopStateEvent", "Event", &[("state", Def::Null)]),
    ("HashChangeEvent", "Event", &[("oldURL", Def::Str("")), ("newURL", Def::Str(""))]),
    ("PageTransitionEvent", "Event", &[("persisted", Def::Bool(false))]),
    ("TransitionEvent", "Event", &[("propertyName", Def::Str("")), ("elapsedTime", Def::Num(0.0)), ("pseudoElement", Def::Str(""))]),
    ("AnimationEvent", "Event", &[("animationName", Def::Str("")), ("elapsedTime", Def::Num(0.0)), ("pseudoElement", Def::Str(""))]),
    ("SubmitEvent", "Event", &[("submitter", Def::Null)]),
    ("StorageEvent", "Event", &[("key", Def::Null), ("oldValue", Def::Null), ("newValue", Def::Null), ("url", Def::Str("")), ("storageArea", Def::Null)]),
    ("PromiseRejectionEvent", "Event", &[("promise", Def::Null), ("reason", Def::Undefined)]),
    ("DragEvent", "MouseEvent", &[("dataTransfer", Def::Null)]),
    ("ClipboardEvent", "Event", &[("clipboardData", Def::Null)]),
    ("BeforeUnloadEvent", "Event", &[]),
];

#[derive(Clone, Copy)]
enum Def {
    Num(f64),
    Bool(bool),
    Str(&'static str),
    Null,
    Undefined,
}

impl Def {
    fn value(self) -> Value {
        match self {
            Def::Num(n) => Value::Num(n),
            Def::Bool(b) => Value::Bool(b),
            Def::Str(s) => js(s),
            Def::Null => Value::Null,
            Def::Undefined => Value::Undefined,
        }
    }
}

/// The events whose handlers can be set as properties (onclick) and attributes.
pub const HANDLERS: &[&str] = &[
    "abort", "animationend", "animationiteration", "animationstart", "auxclick", "beforeinput", "beforeunload", "blur",
    "cancel", "canplay", "canplaythrough", "change", "click", "close", "contextmenu", "copy", "cut", "dblclick", "drag",
    "dragend", "dragenter", "dragleave", "dragover", "dragstart", "drop", "durationchange", "ended", "error", "focus",
    "focusin", "focusout", "hashchange", "input", "invalid", "keydown", "keypress", "keyup", "load", "loadeddata",
    "loadedmetadata", "loadend", "loadstart", "message", "mousedown", "mouseenter", "mouseleave", "mousemove",
    "mouseout", "mouseover", "mouseup", "offline", "online", "pagehide", "pageshow", "paste", "pause", "play",
    "playing", "pointercancel", "pointerdown", "pointerenter", "pointerleave", "pointermove", "pointerout",
    "pointerover", "pointerup", "popstate", "progress", "readystatechange", "reset", "resize", "scroll", "scrollend",
    "search", "seeked", "seeking", "select", "selectionchange", "storage", "submit", "timeout", "timeupdate", "toggle",
    "touchcancel", "touchend", "touchmove", "touchstart", "transitionend", "unhandledrejection", "unload",
    "visibilitychange", "volumechange", "waiting", "wheel",
];

pub fn init(vm: &mut Vm) {
    let et = interface(vm, "EventTarget", None, 0, Some(event_target_new));
    method(vm, et, "addEventListener", 2, add_event_listener);
    method(vm, et, "removeEventListener", 2, remove_event_listener);
    method(vm, et, "dispatchEvent", 1, dispatch_event);
    // The window is an EventTarget (its prototype chain: Window.prototype, EventTarget.prototype).
    let wp = interface(vm, "Window", Some("EventTarget"), 0, None);
    let w = host_ref(vm).window;
    vm.heap.get_mut(w).proto = Some(wp);
    for name in HANDLERS {
        handler_property(vm, wp, name);
    }

    let ep = interface(vm, "Event", None, 1, Some(event_new));
    for (name, f) in [
        ("type", ev_type as NativeFn),
        ("target", ev_target),
        ("srcElement", ev_target),
        ("currentTarget", ev_current_target),
        ("eventPhase", ev_phase),
        ("bubbles", ev_bubbles),
        ("cancelable", ev_cancelable),
        ("defaultPrevented", ev_default_prevented),
        ("composed", ev_composed),
        ("isTrusted", ev_trusted),
        ("timeStamp", ev_time),
    ] {
        attribute(vm, ep, name, f, None);
    }
    attribute(vm, ep, "returnValue", ev_return_value, Some(ev_set_return_value));
    attribute(vm, ep, "cancelBubble", ev_cancel_bubble, Some(ev_set_cancel_bubble));
    method(vm, ep, "preventDefault", 0, prevent_default);
    method(vm, ep, "stopPropagation", 0, stop_propagation);
    method(vm, ep, "stopImmediatePropagation", 0, stop_immediate);
    method(vm, ep, "composedPath", 0, composed_path);
    method(vm, ep, "initEvent", 3, init_event);
    constant(vm, ep, "NONE", 0.0);
    constant(vm, ep, "CAPTURING_PHASE", 1.0);
    constant(vm, ep, "AT_TARGET", 2.0);
    constant(vm, ep, "BUBBLING_PHASE", 3.0);
    for (name, parent, _) in KINDS {
        let p = interface(vm, name, Some(parent), 1, Some(event_new));
        if let Some(Prop { slot: Slot::Data(Value::Obj(c)), .. }) = vm.heap.get(p).props.get(&Key::Str(js::names::CONSTRUCTOR)).cloned()
            && let Class::Native(n) = &mut vm.heap.get_mut(c).class
        {
            n.data = js(name);
        }
        match *name {
            "MouseEvent" => {
                attribute(vm, p, "pageX", mouse_page_x, None);
                attribute(vm, p, "pageY", mouse_page_y, None);
                attribute(vm, p, "x", mouse_client_x, None);
                attribute(vm, p, "y", mouse_client_y, None);
                attribute(vm, p, "offsetX", mouse_offset_x, None);
                attribute(vm, p, "offsetY", mouse_offset_y, None);
                method(vm, p, "getModifierState", 1, get_modifier_state);
                method(vm, p, "initMouseEvent", 15, init_mouse_event);
            }
            "KeyboardEvent" => {
                method(vm, p, "getModifierState", 1, get_modifier_state);
                constant(vm, p, "DOM_KEY_LOCATION_STANDARD", 0.0);
                constant(vm, p, "DOM_KEY_LOCATION_LEFT", 1.0);
                constant(vm, p, "DOM_KEY_LOCATION_RIGHT", 2.0);
                constant(vm, p, "DOM_KEY_LOCATION_NUMPAD", 3.0);
            }
            "CustomEvent" => method(vm, p, "initCustomEvent", 4, init_custom_event),
            "UIEvent" => method(vm, p, "initUIEvent", 5, init_event),
            _ => {}
        }
    }
    // MutationObserver.
    let mo = interface(vm, "MutationObserver", None, 1, Some(observer_new));
    method(vm, mo, "observe", 2, observe);
    method(vm, mo, "disconnect", 0, disconnect);
    method(vm, mo, "takeRecords", 0, take_records);
    let w = host_ref(vm).window;
    if let Some(c) = ctor_of(vm, mo) {
        vm.put(w, "WebKitMutationObserver", c, HIDDEN);
    }
}

/// An interface's constructor (from its prototype).
pub fn ctor_of(vm: &Vm, proto: Gc) -> Option<Value> {
    match vm.heap.get(proto).props.get(&Key::Str(js::names::CONSTRUCTOR)) {
        Some(Prop { slot: Slot::Data(v), .. }) => Some(v.clone()),
        _ => None,
    }
}

/// The prototype for an object `new` makes: new.target's prototype, if it's an object; else `fallback`'s.
pub fn proto_from(vm: &mut Vm, nt: &Value, fallback: &str) -> Result<Gc, Value> {
    if let Value::Obj(c) = nt {
        let p = vm.get(*c, Key::Str(js::names::PROTOTYPE), nt)?;
        if let Value::Obj(p) = p {
            return Ok(p);
        }
    }
    Ok(host_ref(vm).proto(fallback).unwrap_or_else(|| vm.proto(I::ObjectProto)))
}

fn event_target_new(vm: &mut Vm, nt: &Value, _args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "EventTarget")?;
    Ok(Value::Obj(vm.alloc(Some(p), Class::Ordinary)))
}

// ------------------------------------------------------------------------------------------- listeners

/// The object listeners are kept for: the window for the global object's `this`, the object itself otherwise.
fn target_obj(vm: &mut Vm, this: &Value) -> Result<Gc, Value> {
    match this {
        Value::Obj(g) => Ok(*g),
        Value::Undefined | Value::Null => Ok(host_ref(vm).window),
        _ => Err(vm.type_error("Illegal invocation")),
    }
}

/// addEventListener's options: (capture, once, passive, signal).
fn options(vm: &mut Vm, v: &Value) -> Result<(bool, bool, bool, Value), Value> {
    match v {
        Value::Obj(o) => {
            let o = *o;
            let c = get(vm, o, "capture")?.truthy();
            let once = get(vm, o, "once")?.truthy();
            let passive = get(vm, o, "passive")?.truthy();
            let signal = get(vm, o, "signal")?;
            Ok((c, once, passive, signal))
        }
        other => Ok((other.truthy(), false, false, Value::Undefined)),
    }
}

fn add_event_listener(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_obj(vm, this)?;
    let ty = str_arg(vm, args, 0, "addEventListener")?;
    let cb = arg(args, 1);
    if !cb.is_object() {
        return Ok(Value::Undefined);
    }
    let (capture, once, passive, signal) = options(vm, &arg(args, 2))?;
    if let Value::Obj(s) = &signal
        && get(vm, *s, "aborted")?.truthy()
    {
        return Ok(Value::Undefined);
    }
    let h = host(vm);
    if h.listeners.iter().any(|l| !l.removed && l.target == t && l.ty == ty && l.capture == capture && js::value::strict_equals(&l.callback, &cb)) {
        return Ok(Value::Undefined);
    }
    h.listeners.push(Listener { target: t, ty: ty.clone(), callback: cb.clone(), capture, once, passive, removed: false });
    // (An abort signal takes it off again.)
    if let Value::Obj(s) = signal {
        let remover = vm.native_with("", 1, remove_on_abort, None, Value::Obj(t));
        let arr = vm.new_array(vec![js(&ty), cb, Value::Bool(capture)]);
        vm.set_data2(remover, Value::Obj(arr));
        let add = get(vm, s, "addEventListener")?;
        if vm.is_callable(&add) {
            vm.call(&add, Value::Obj(s), &[js("abort"), Value::Obj(remover)])?;
        }
    }
    Ok(Value::Undefined)
}

fn remove_on_abort(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    let t = vm.data(f);
    let a = vm.data2(f);
    if let (Value::Obj(t), Value::Obj(a)) = (t, a) {
        let ty = vm.get(a, Key::Index(0), &Value::Obj(a))?;
        let cb = vm.get(a, Key::Index(1), &Value::Obj(a))?;
        let cap = vm.get(a, Key::Index(2), &Value::Obj(a))?;
        remove_event_listener(vm, &Value::Obj(t), &[ty, cb, cap], f)?;
    }
    Ok(Value::Undefined)
}

fn remove_event_listener(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_obj(vm, this)?;
    let ty = str_arg(vm, args, 0, "removeEventListener")?;
    let cb = arg(args, 1);
    let (capture, _, _, _) = options(vm, &arg(args, 2))?;
    let h = host(vm);
    for l in h.listeners.iter_mut() {
        if !l.removed && l.target == t && l.ty == ty && l.capture == capture && js::value::strict_equals(&l.callback, &cb) {
            l.removed = true;
        }
    }
    h.listeners.retain(|l| !l.removed);
    Ok(Value::Undefined)
}

fn dispatch_event(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let t = target_obj(vm, this)?;
    let ev = arg(args, 0);
    let Value::Obj(e) = ev else { return Err(vm.type_error("Failed to execute 'dispatchEvent': parameter 1 is not of type 'Event'.")) };
    let st = state(vm, &ev).ok_or_else(|| vm.type_error("Failed to execute 'dispatchEvent': parameter 1 is not of type 'Event'."))?;
    if flags(vm, st) & DISPATCHING != 0 {
        return Err(dom_exception(vm, "InvalidStateError", "The event is already being dispatched."));
    }
    let f = flags(vm, st) & !TRUSTED;
    set_flags(vm, st, f);
    let ok = dispatch(vm, &Value::Obj(t), e);
    Ok(Value::Bool(ok))
}

// ---------------------------------------------------------------------------------------------- events

/// An event's hidden state object.
fn state(vm: &Vm, ev: &Value) -> Option<Gc> {
    match slot(vm, ev, host_ref(vm).syms.event) {
        Value::Obj(g) => Some(g),
        _ => None,
    }
}

fn st_get(vm: &mut Vm, st: Gc, name: &str) -> Value {
    let a = vm.atom(name);
    match vm.heap.get(st).props.get(&Key::Str(a)) {
        Some(Prop { slot: Slot::Data(v), .. }) => v.clone(),
        _ => Value::Undefined,
    }
}

fn st_set(vm: &mut Vm, st: Gc, name: &str, v: Value) {
    vm.put(st, name, v, ALL);
}

fn flags(vm: &mut Vm, st: Gc) -> u32 {
    match st_get(vm, st, "flags") {
        Value::Num(n) => n as u32,
        _ => 0,
    }
}

fn set_flags(vm: &mut Vm, st: Gc, f: u32) {
    st_set(vm, st, "flags", Value::Num(f as f64));
}

/// A new event object of an interface (its prototype given), with its type and flags.
fn make_event(vm: &mut Vm, proto: Gc, ty: &str, f: u32) -> Gc {
    let e = vm.alloc(Some(proto), Class::Ordinary);
    let st = vm.new_object();
    st_set(vm, st, "type", js(ty));
    set_flags(vm, st, f);
    st_set(vm, st, "target", Value::Null);
    st_set(vm, st, "current", Value::Null);
    st_set(vm, st, "phase", Value::Num(0.0));
    let now = host_ref(vm).now;
    st_set(vm, st, "time", Value::Num(now));
    set_slot(vm, e, host_ref(vm).syms.event, Value::Obj(st));
    e
}

fn event_new(vm: &mut Vm, nt: &Value, args: &[Value], f: Gc) -> R {
    let iface = match vm.data(f) {
        Value::Str(s) => s.to_rust(),
        _ => String::from("Event"),
    };
    if args.is_empty() {
        return Err(vm.type_error(&format!("Failed to construct '{iface}': 1 argument required, but only 0 present.")));
    }
    let ty = string(vm, &args[0])?;
    let init = arg(args, 1);
    let mut fl = 0;
    if let Value::Obj(o) = &init {
        let o = *o;
        for (name, bit) in [("bubbles", BUBBLES), ("cancelable", CANCELABLE), ("composed", COMPOSED)] {
            let a = vm.atom(name);
            if vm.get(o, Key::Str(a), &init)?.truthy() {
                fl |= bit;
            }
        }
    }
    let proto = proto_from(vm, nt, &iface)?;
    let e = make_event(vm, proto, &ty, fl);
    // The dictionary's members of this interface and those it inherits.
    let mut chain: Vec<&str> = Vec::new();
    let mut at = iface.as_str();
    while let Some((name, parent, _)) = KINDS.iter().find(|k| k.0 == at) {
        chain.push(name);
        at = parent;
    }
    for name in chain.iter().rev() {
        let members = KINDS.iter().find(|k| k.0 == *name).map(|k| k.2).unwrap_or(&[]);
        for (m, def) in members {
            let v = match &init {
                Value::Obj(o) => {
                    let a = vm.atom(m);
                    let v = vm.get(*o, Key::Str(a), &init)?;
                    if v.is_undefined() { def.value() } else { v }
                }
                _ => def.value(),
            };
            let v = match (def, &v) {
                (Def::Num(_), Value::Num(_)) | (Def::Bool(_), Value::Bool(_)) => v,
                (Def::Num(_), _) => Value::Num(vm.to_number(&v)?),
                (Def::Bool(_), _) => Value::Bool(v.truthy()),
                (Def::Str(_), Value::Str(_)) => v,
                (Def::Str(_), _) => Value::Str(vm.to_string(&v)?),
                _ => v,
            };
            vm.put(e, m, v, js::object::CONFIGURABLE);
        }
    }
    if matches!(iface.as_str(), "KeyboardEvent") {
        // (which: the key's code, as old pages read it.)
        let kc = get(vm, e, "keyCode")?;
        vm.put(e, "which", kc, js::object::CONFIGURABLE);
    }
    Ok(Value::Obj(e))
}

/// `this`'s state, or a TypeError.
fn this_state(vm: &mut Vm, this: &Value) -> Result<Gc, Value> {
    state(vm, this).ok_or_else(|| vm.type_error("Illegal invocation"))
}

fn ev_type(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let st = this_state(vm, this)?;
    Ok(st_get(vm, st, "type"))
}

fn ev_target(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let st = this_state(vm, this)?;
    Ok(st_get(vm, st, "target"))
}

fn ev_current_target(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let st = this_state(vm, this)?;
    Ok(st_get(vm, st, "current"))
}

fn ev_phase(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let st = this_state(vm, this)?;
    Ok(st_get(vm, st, "phase"))
}

fn flag_getter(vm: &mut Vm, this: &Value, bit: u32) -> R {
    let st = this_state(vm, this)?;
    Ok(Value::Bool(flags(vm, st) & bit != 0))
}

fn ev_bubbles(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, BUBBLES)
}
fn ev_cancelable(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, CANCELABLE)
}
fn ev_default_prevented(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, CANCELED)
}
fn ev_composed(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, COMPOSED)
}
fn ev_trusted(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, TRUSTED)
}

fn ev_time(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let st = this_state(vm, this)?;
    Ok(st_get(vm, st, "time"))
}

fn ev_return_value(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let st = this_state(vm, this)?;
    Ok(Value::Bool(flags(vm, st) & CANCELED == 0))
}

fn ev_set_return_value(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if !arg(args, 0).truthy() {
        prevent_default(vm, this, &[], _f)?;
    }
    Ok(Value::Undefined)
}

fn ev_cancel_bubble(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    flag_getter(vm, this, STOP)
}

fn ev_set_cancel_bubble(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if arg(args, 0).truthy() {
        stop_propagation(vm, this, &[], _f)?;
    }
    Ok(Value::Undefined)
}

fn prevent_default(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let st = this_state(vm, this)?;
    let f = flags(vm, st);
    if f & CANCELABLE != 0 && f & IN_PASSIVE == 0 {
        set_flags(vm, st, f | CANCELED);
    }
    Ok(Value::Undefined)
}

fn stop_propagation(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let st = this_state(vm, this)?;
    let f = flags(vm, st);
    set_flags(vm, st, f | STOP);
    Ok(Value::Undefined)
}

fn stop_immediate(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let st = this_state(vm, this)?;
    let f = flags(vm, st);
    set_flags(vm, st, f | STOP | STOP_NOW);
    Ok(Value::Undefined)
}

fn composed_path(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let st = this_state(vm, this)?;
    let path = st_get(vm, st, "path");
    if let Value::Obj(p) = path {
        let items = vm.array_values(&Value::Obj(p));
        return Ok(Value::Obj(vm.new_array(items)));
    }
    Ok(Value::Obj(vm.new_array(Vec::new())))
}

fn init_event(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let st = this_state(vm, this)?;
    let f = flags(vm, st);
    if f & DISPATCHING != 0 {
        return Ok(Value::Undefined);
    }
    let ty = string(vm, &arg(args, 0))?;
    st_set(vm, st, "type", js(&ty));
    let mut nf = f & !(BUBBLES | CANCELABLE | CANCELED | STOP | STOP_NOW);
    if arg(args, 1).truthy() {
        nf |= BUBBLES;
    }
    if arg(args, 2).truthy() {
        nf |= CANCELABLE;
    }
    set_flags(vm, st, nf);
    Ok(Value::Undefined)
}

fn init_custom_event(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    init_event(vm, this, args, f)?;
    if let Value::Obj(e) = this {
        vm.put(*e, "detail", arg(args, 3), js::object::CONFIGURABLE);
    }
    Ok(Value::Undefined)
}

fn init_mouse_event(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    init_event(vm, this, args, f)?;
    if let Value::Obj(e) = this {
        let e = *e;
        let names = ["", "", "", "view", "detail", "screenX", "screenY", "clientX", "clientY", "ctrlKey", "altKey", "shiftKey", "metaKey", "button", "relatedTarget"];
        for (i, n) in names.iter().enumerate().skip(3) {
            if i < args.len() {
                vm.put(e, n, args[i].clone(), js::object::CONFIGURABLE);
            }
        }
    }
    Ok(Value::Undefined)
}

fn own_num(vm: &mut Vm, this: &Value, name: &str) -> Result<f64, Value> {
    match this {
        Value::Obj(g) => {
            let a = vm.atom(name);
            let v = vm.get(*g, Key::Str(a), this)?;
            vm.to_number(&v)
        }
        _ => Err(vm.type_error("Illegal invocation")),
    }
}

fn mouse_page_x(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(own_num(vm, this, "clientX")? + host_ref(vm).scroll.0 as f64))
}
fn mouse_page_y(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(own_num(vm, this, "clientY")? + host_ref(vm).scroll.1 as f64))
}
fn mouse_client_x(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(own_num(vm, this, "clientX")?))
}
fn mouse_client_y(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(own_num(vm, this, "clientY")?))
}

/// offsetX/Y: the click's place within the target's box.
fn offset(vm: &mut Vm, this: &Value, y: bool) -> R {
    let c = own_num(vm, this, if y { "clientY" } else { "clientX" })?;
    let st = this_state(vm, this)?;
    let t = st_get(vm, st, "target");
    let at = node_of(vm, &t).and_then(|n| host_ref(vm).rects.get(n).copied().flatten());
    let scroll = if y { host_ref(vm).scroll.1 } else { host_ref(vm).scroll.0 } as f64;
    Ok(Value::Num(match at {
        Some((x0, y0, _, _)) => c + scroll - if y { y0 } else { x0 } as f64,
        None => c,
    }))
}

fn mouse_offset_x(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    offset(vm, this, false)
}
fn mouse_offset_y(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    offset(vm, this, true)
}

fn get_modifier_state(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let k = string(vm, &arg(args, 0))?;
    let name = match k.as_str() {
        "Control" => "ctrlKey",
        "Shift" => "shiftKey",
        "Alt" => "altKey",
        "Meta" => "metaKey",
        _ => return Ok(Value::Bool(false)),
    };
    let Value::Obj(g) = this else { return Ok(Value::Bool(false)) };
    let a = vm.atom(name);
    Ok(Value::Bool(vm.get(*g, Key::Str(a), this)?.truthy()))
}

// -------------------------------------------------------------------------------------------- dispatch

/// An event the page fires at a target (trusted): whether it wasn't cancelled.
pub fn fire(vm: &mut Vm, target: &Value, init: &EventInit) -> bool {
    fire_with(vm, target, init, Vec::new())
}

/// As `fire`, with members whose values are scripts' own (a message's data: a string, an ArrayBuffer, a Blob).
pub fn fire_with(vm: &mut Vm, target: &Value, init: &EventInit, values: Vec<(&'static str, Value)>) -> bool {
    let proto = host_ref(vm).proto(init.interface).or_else(|| host_ref(vm).proto("Event")).unwrap_or_else(|| vm.proto(I::ObjectProto));
    let mut f = TRUSTED;
    if init.bubbles {
        f |= BUBBLES;
    }
    if init.cancelable {
        f |= CANCELABLE;
    }
    let e = make_event(vm, proto, &init.ty, f);
    // Its interface's members' defaults, then what's given.
    let mut chain: Vec<&str> = Vec::new();
    let mut at = init.interface;
    while let Some((name, parent, _)) = KINDS.iter().find(|k| k.0 == at) {
        chain.push(name);
        at = parent;
    }
    for name in chain.iter().rev() {
        let members = KINDS.iter().find(|k| k.0 == *name).map(|k| k.2).unwrap_or(&[]);
        for (m, def) in members {
            vm.put(e, m, def.value(), js::object::CONFIGURABLE);
        }
    }
    if chain.contains(&"UIEvent") {
        let w = host_ref(vm).window;
        vm.put(e, "view", Value::Obj(w), js::object::CONFIGURABLE);
    }
    for (name, v) in &init.props {
        let v = match v {
            EvValue::Num(n) => Value::Num(*n),
            EvValue::Bool(b) => Value::Bool(*b),
            EvValue::Str(s) => js(s),
            EvValue::Node(n) => wrap_opt(vm, *n),
        };
        vm.put(e, name, v, js::object::CONFIGURABLE);
    }
    for (name, v) in values {
        vm.put(e, name, v, js::object::CONFIGURABLE);
    }
    dispatch(vm, target, e)
}

/// The path an event takes from its target: the target, its ancestors, the document and the window.
fn path_of(vm: &mut Vm, target: &Value) -> Vec<Value> {
    let Some(n) = node_of(vm, target) else { return vec![target.clone()] };
    let mut nodes = vec![n];
    let mut at = n;
    loop {
        let d = &host_ref(vm).doc;
        let next = d.parent(at).or_else(|| d.shadow_host(at));
        let Some(p) = next else { break };
        nodes.push(p);
        at = p;
        if nodes.len() > 4000 {
            break;
        }
    }
    let connected = at == 0;
    let mut path: Vec<Value> = nodes.into_iter().map(|id| wrap(vm, id)).collect();
    if connected {
        path.push(Value::Obj(host_ref(vm).window));
    }
    path
}

/// Sends an event through its path: whether it wasn't cancelled.
pub fn dispatch(vm: &mut Vm, target: &Value, e: Gc) -> bool {
    let ev = Value::Obj(e);
    let Some(st) = state(vm, &ev) else { return true };
    let f = flags(vm, st);
    set_flags(vm, st, (f | DISPATCHING) & !(STOP | STOP_NOW | CANCELED));
    st_set(vm, st, "target", target.clone());
    let path = path_of(vm, target);
    let pa = vm.new_array(path.clone());
    st_set(vm, st, "path", Value::Obj(pa));
    let ty = match st_get(vm, st, "type") {
        Value::Str(s) => s.to_rust(),
        _ => String::new(),
    };
    // Handlers in attributes (onclick="...") made into functions as their events come.
    for p in &path {
        if let Some(n) = node_of(vm, p) {
            attribute_handler(vm, n, &ty);
        }
    }
    let bubbles = f & BUBBLES != 0;
    // Capturing, from the window down.
    for i in (1..path.len()).rev() {
        invoke(vm, &path[i], &ev, st, &ty, 1, Some(true));
        if flags(vm, st) & STOP != 0 {
            break;
        }
    }
    // At the target.
    if flags(vm, st) & STOP == 0 {
        invoke(vm, &path[0], &ev, st, &ty, 2, Some(true));
    }
    if flags(vm, st) & (STOP | STOP_NOW) == 0 {
        invoke(vm, &path[0], &ev, st, &ty, 2, Some(false));
    }
    // Bubbling back up.
    if bubbles {
        for p in path.iter().skip(1) {
            if flags(vm, st) & STOP != 0 {
                break;
            }
            invoke(vm, p, &ev, st, &ty, 3, Some(false));
        }
    }
    st_set(vm, st, "phase", Value::Num(0.0));
    st_set(vm, st, "current", Value::Null);
    let f = flags(vm, st);
    set_flags(vm, st, f & !(DISPATCHING | STOP | STOP_NOW));
    f & CANCELED == 0
}

/// Calls the listeners on one object of the path (capturing ones, or not).
fn invoke(vm: &mut Vm, obj: &Value, ev: &Value, st: Gc, ty: &str, phase: u8, capture: Option<bool>) {
    let Value::Obj(o) = obj else { return };
    let o = *o;
    // (The listeners as they are now: ones added while it's called aren't called this time.)
    let list: Vec<(usize, Value, bool, bool, bool)> = host_ref(vm)
        .listeners
        .iter()
        .enumerate()
        .filter(|(_, l)| l.target == o && l.ty == ty && !l.removed && capture.is_none_or(|c| c == l.capture))
        .map(|(i, l)| (i, l.callback.clone(), l.once, l.passive, l.capture))
        .collect();
    if list.is_empty() {
        return;
    }
    st_set(vm, st, "current", obj.clone());
    st_set(vm, st, "phase", Value::Num(phase as f64));
    for (_, cb, once, passive, cap) in list {
        // (Removed by an earlier one?)
        let still = host_ref(vm).listeners.iter().any(|l| l.target == o && l.ty == ty && !l.removed && l.capture == cap && js::value::strict_equals(&l.callback, &cb));
        if !still {
            continue;
        }
        if once {
            let h = host(vm);
            for l in h.listeners.iter_mut() {
                if l.target == o && l.ty == ty && l.capture == cap && js::value::strict_equals(&l.callback, &cb) {
                    l.removed = true;
                }
            }
            h.listeners.retain(|l| !l.removed);
        }
        let f = flags(vm, st);
        if passive {
            set_flags(vm, st, f | IN_PASSIVE);
        }
        let handler = is_handler(vm, &cb);
        vm.steps = 0;
        let r = if vm.is_callable(&cb) {
            vm.call(&cb, obj.clone(), core::slice::from_ref(ev))
        } else if let Value::Obj(c) = &cb {
            let a = vm.atom("handleEvent");
            match vm.get(*c, Key::Str(a), &cb) {
                Ok(m) if vm.is_callable(&m) => vm.call(&m, cb.clone(), core::slice::from_ref(ev)),
                Ok(_) => Ok(Value::Undefined),
                Err(e) => Err(e),
            }
        } else {
            Ok(Value::Undefined)
        };
        match r {
            // (A handler property returning false cancels the event: onclick="return false".)
            Ok(Value::Bool(false)) if handler && ty != "error" => {
                let f = flags(vm, st);
                if f & CANCELABLE != 0 {
                    set_flags(vm, st, f | CANCELED);
                }
            }
            Ok(_) => {}
            Err(e) => super::report(vm, &e, "an event listener"),
        }
        let f = flags(vm, st);
        set_flags(vm, st, f & !IN_PASSIVE);
        if f & STOP_NOW != 0 {
            break;
        }
    }
}

// ------------------------------------------------------------------------------------ handler properties

/// Whether a listener's callback is a handler property's (marked with a hidden flag on the function).
fn is_handler(vm: &Vm, cb: &Value) -> bool {
    matches!(slot(vm, cb, host_ref(vm).syms.target), Value::Bool(true))
}

/// onclick and the like, as properties on a prototype.
pub fn handler_property(vm: &mut Vm, proto: Gc, name: &str) {
    let prop = format!("on{name}");
    attribute_with(vm, proto, &prop, handler_get, Some(handler_set), js(name));
}

fn handler_get(vm: &mut Vm, this: &Value, _args: &[Value], f: Gc) -> R {
    let ty = match vm.data(f) {
        Value::Str(s) => s.to_rust(),
        _ => return Ok(Value::Null),
    };
    let t = target_obj(vm, this)?;
    if let Some(n) = node_of(vm, &Value::Obj(t)) {
        attribute_handler(vm, n, &ty);
    }
    let h = host_ref(vm);
    for l in &h.listeners {
        if l.target == t && l.ty == ty && !l.removed && is_handler(vm, &l.callback) {
            return Ok(l.callback.clone());
        }
    }
    Ok(Value::Null)
}

fn handler_set(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let ty = match vm.data(f) {
        Value::Str(s) => s.to_rust(),
        _ => return Ok(Value::Undefined),
    };
    let t = target_obj(vm, this)?;
    set_handler(vm, t, &ty, arg(args, 0));
    Ok(Value::Undefined)
}

/// Sets (or clears) an object's handler for an event (in the place its first one had among its listeners).
pub fn set_handler(vm: &mut Vm, t: Gc, ty: &str, v: Value) {
    let callable = vm.is_callable(&v);
    if let Value::Obj(g) = &v
        && callable
    {
        set_slot(vm, *g, host_ref(vm).syms.target, Value::Bool(true));
    }
    let existing = {
        let h = host_ref(vm);
        h.listeners.iter().position(|l| l.target == t && l.ty == ty && !l.removed && matches!(slot(vm, &l.callback, h.syms.target), Value::Bool(true)))
    };
    let h = host(vm);
    match (existing, callable) {
        (Some(i), true) => h.listeners[i].callback = v,
        (Some(i), false) => {
            h.listeners.remove(i);
        }
        (None, true) => h.listeners.push(Listener { target: t, ty: String::from(ty), callback: v, capture: false, once: false, passive: false, removed: false }),
        (None, false) => {}
    }
}

/// A node's on<event> attribute made into its handler, if it has one and it hasn't been made yet (or changed).
pub fn attribute_handler(vm: &mut Vm, n: NodeId, ty: &str) {
    let attr = format!("on{ty}");
    let code = match host_ref(vm).doc.element(n).and_then(|e| e.attr(&attr)) {
        Some(c) => String::from(c),
        None => return,
    };
    let key = (n, 0x1000_0000 + fnv(&attr) % 0x0fff_ffff);
    // (Made already from this text?)
    if let Some(&made) = host_ref(vm).cache.get(&key) {
        let same = matches!(slot(vm, &Value::Obj(made), host_ref(vm).syms.data), Value::Str(ref s) if s.to_rust() == code);
        if same {
            return;
        }
    }
    let w = wrap(vm, n);
    // The function: `event` its parameter, the element (and its form, and the document) in scope as `with`.
    let src = format!("(function(event){{with(this.ownerDocument||{{}}){{with(this.form||{{}}){{with(this){{return (function(){{{code}\n}}).call(this);}}}}}}}})");
    let f = match vm.indirect_eval(&src) {
        Ok(f) => f,
        Err(e) => {
            super::report(vm, &e, "an event handler attribute");
            return;
        }
    };
    let Value::Obj(fg) = f else { return };
    set_slot(vm, fg, host_ref(vm).syms.data, js(&code));
    host(vm).cache.insert(key, fg);
    if let Value::Obj(t) = w {
        set_handler(vm, t, ty, f);
    }
}

pub fn fnv(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// An uncaught exception as the window's error event (and its onerror handler's five arguments).
pub fn error_event(vm: &mut Vm, e: &Value, text: &str) {
    use core::sync::atomic::{AtomicBool, Ordering};
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY.swap(true, Ordering::Relaxed) {
        return;
    }
    let w = host_ref(vm).window;
    let has = host_ref(vm).listeners.iter().any(|l| l.target == w && l.ty == "error" && !l.removed);
    if has {
        let proto = host_ref(vm).proto("ErrorEvent").unwrap_or_else(|| vm.proto(I::ObjectProto));
        let ev = make_event(vm, proto, "error", CANCELABLE | TRUSTED);
        vm.put(ev, "message", js(text), js::object::CONFIGURABLE);
        vm.put(ev, "error", e.clone(), js::object::CONFIGURABLE);
        vm.put(ev, "filename", js(""), js::object::CONFIGURABLE);
        vm.put(ev, "lineno", Value::Num(0.0), js::object::CONFIGURABLE);
        vm.put(ev, "colno", Value::Num(0.0), js::object::CONFIGURABLE);
        // (onerror gets the five arguments; the listeners get the event.)
        let handler = host_ref(vm).listeners.iter().find(|l| l.target == w && l.ty == "error" && !l.removed && is_handler(vm, &l.callback)).map(|l| l.callback.clone());
        if let Some(hf) = handler {
            let _ = vm.call(&hf, Value::Obj(w), &[js(text), js(""), Value::Num(0.0), Value::Num(0.0), e.clone()]);
            let h = host(vm);
            for l in h.listeners.iter_mut() {
                if l.target == w && l.ty == "error" && js::value::strict_equals(&l.callback, &hf) {
                    l.removed = true;
                }
            }
            let _ = dispatch(vm, &Value::Obj(w), ev);
            let h = host(vm);
            for l in h.listeners.iter_mut() {
                if l.target == w && l.ty == "error" && js::value::strict_equals(&l.callback, &hf) {
                    l.removed = false;
                }
            }
        } else {
            let _ = dispatch(vm, &Value::Obj(w), ev);
        }
    }
    BUSY.store(false, Ordering::Relaxed);
}

// --------------------------------------------------------------------------------------- mutation observers

/// What a mutation observer watches on a node.
#[derive(Clone, Default)]
pub struct Watch {
    pub node: NodeId,
    pub child_list: bool,
    pub attributes: bool,
    pub character_data: bool,
    pub subtree: bool,
    pub old_attribute: bool,
    pub old_data: bool,
    pub filter: Option<Vec<String>>,
}

pub struct Observer {
    pub obj: Gc,
    pub callback: Value,
    pub watches: Vec<Watch>,
    pub records: Vec<Value>,
}

impl Observer {
    pub fn trace(&self, roots: &mut Vec<Value>) {
        roots.push(Value::Obj(self.obj));
        roots.push(self.callback.clone());
        roots.extend(self.records.iter().cloned());
    }
}

fn observer_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let cb = arg(args, 0);
    if !vm.is_callable(&cb) {
        return Err(vm.type_error("Failed to construct 'MutationObserver': parameter 1 is not of type 'MutationCallback'."));
    }
    let p = proto_from(vm, nt, "MutationObserver")?;
    let o = vm.alloc(Some(p), Class::Ordinary);
    host(vm).observers.push(Observer { obj: o, callback: cb, watches: Vec::new(), records: Vec::new() });
    Ok(Value::Obj(o))
}

fn this_observer(vm: &mut Vm, this: &Value) -> Result<usize, Value> {
    let Value::Obj(g) = this else { return Err(vm.type_error("Illegal invocation")) };
    host_ref(vm).observers.iter().position(|o| o.obj == *g).ok_or_else(|| vm.type_error("Illegal invocation"))
}

fn observe(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let i = this_observer(vm, this)?;
    let n = node_arg(vm, args, 0, "observe")?;
    let o = arg(args, 1);
    let mut w = Watch { node: n, ..Watch::default() };
    if let Value::Obj(g) = &o {
        let g = *g;
        let get = |vm: &mut Vm, name: &str| -> Result<Value, Value> {
            let a = vm.atom(name);
            vm.get(g, Key::Str(a), &Value::Obj(g))
        };
        w.child_list = get(vm, "childList")?.truthy();
        let attrs = get(vm, "attributes")?;
        let filter = get(vm, "attributeFilter")?;
        let old_attr = get(vm, "attributeOldValue")?;
        let cdata = get(vm, "characterData")?;
        let old_data = get(vm, "characterDataOldValue")?;
        w.subtree = get(vm, "subtree")?.truthy();
        w.old_attribute = old_attr.truthy();
        w.old_data = old_data.truthy();
        if !filter.is_undefined() {
            let names = vm.iterate_to_vec(&filter)?;
            let mut out = Vec::new();
            for n in names {
                out.push(string(vm, &n)?);
            }
            w.filter = Some(out);
        }
        w.attributes = attrs.truthy() || (attrs.is_undefined() && (!filter.is_undefined() || old_attr.truthy()));
        w.character_data = cdata.truthy() || (cdata.is_undefined() && old_data.truthy());
    }
    if !w.child_list && !w.attributes && !w.character_data {
        return Err(vm.type_error("Failed to execute 'observe' on 'MutationObserver': The options object must set at least one of 'attributes', 'characterData', or 'childList' to true."));
    }
    let obs = &mut host(vm).observers[i];
    obs.watches.retain(|x| x.node != n);
    obs.watches.push(w);
    Ok(Value::Undefined)
}

fn disconnect(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let i = this_observer(vm, this)?;
    let o = &mut host(vm).observers[i];
    o.watches.clear();
    o.records.clear();
    Ok(Value::Undefined)
}

fn take_records(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let i = this_observer(vm, this)?;
    let r = core::mem::take(&mut host(vm).observers[i].records);
    Ok(Value::Obj(vm.new_array(r)))
}

/// A change to the tree, for the observers watching it.
pub enum Change<'a> {
    /// Children added and removed in a node (and the siblings around them).
    Children(NodeId, Vec<NodeId>, Vec<NodeId>, Option<NodeId>, Option<NodeId>),
    /// An attribute changed: the element, its name, its old value.
    Attribute(NodeId, &'a str, Option<String>),
    /// A text's (or comment's) words changed: the node, its old words.
    Data(NodeId, String),
}

/// Records a change for the observers that watch it.
pub fn record(vm: &mut Vm, change: Change) {
    if host_ref(vm).observers.iter().all(|o| o.watches.is_empty()) {
        return;
    }
    let target = match &change {
        Change::Children(n, ..) | Change::Attribute(n, ..) | Change::Data(n, _) => *n,
    };
    // Which observers, and whether each wants the old value.
    let mut who: Vec<(usize, bool)> = Vec::new();
    {
        let h = host_ref(vm);
        for (i, o) in h.observers.iter().enumerate() {
            for w in &o.watches {
                if !(w.node == target || (w.subtree && h.doc.contains(w.node, target))) {
                    continue;
                }
                let (wants, old) = match &change {
                    Change::Children(..) => (w.child_list, false),
                    Change::Attribute(_, name, _) => (w.attributes && w.filter.as_ref().is_none_or(|f| f.iter().any(|x| x == name)), w.old_attribute),
                    Change::Data(..) => (w.character_data, w.old_data),
                };
                if wants {
                    who.push((i, old));
                    break;
                }
            }
        }
    }
    if who.is_empty() {
        return;
    }
    let t = wrap(vm, target);
    for (i, old) in who {
        let r = vm.new_object();
        vm.put(r, "target", t.clone(), ALL);
        let (ty, added, removed, prev, next, name, old_value) = match &change {
            Change::Children(_, a, rm, p, nx) => ("childList", a.clone(), rm.clone(), *p, *nx, None, None),
            Change::Attribute(_, n, o) => ("attributes", Vec::new(), Vec::new(), None, None, Some(String::from(*n)), o.clone()),
            Change::Data(_, o) => ("characterData", Vec::new(), Vec::new(), None, None, None, Some(o.clone())),
        };
        vm.put(r, "type", js(ty), ALL);
        let added: Vec<Value> = added.into_iter().map(|n| wrap(vm, n)).collect();
        let removed: Vec<Value> = removed.into_iter().map(|n| wrap(vm, n)).collect();
        let a = vm.new_array(added);
        let rm = vm.new_array(removed);
        vm.put(r, "addedNodes", Value::Obj(a), ALL);
        vm.put(r, "removedNodes", Value::Obj(rm), ALL);
        let p = wrap_opt(vm, prev);
        let nx = wrap_opt(vm, next);
        vm.put(r, "previousSibling", p, ALL);
        vm.put(r, "nextSibling", nx, ALL);
        vm.put(r, "attributeName", name.map_or(Value::Null, |n| js(&n)), ALL);
        vm.put(r, "attributeNamespace", Value::Null, ALL);
        vm.put(r, "oldValue", if old { old_value.map_or(Value::Null, |v| js(&v)) } else { Value::Null }, ALL);
        let mrp = host_ref(vm).proto("MutationRecord");
        if let Some(mrp) = mrp {
            vm.heap.get_mut(r).proto = Some(mrp);
        }
        host(vm).observers[i].records.push(Value::Obj(r));
    }
    host(vm).observer_queued = true;
}

/// The observers' records, to their callbacks (a microtask's worth: at the checkpoint after each script).
pub fn deliver_mutations(vm: &mut Vm) {
    if !host_ref(vm).observer_queued {
        return;
    }
    host(vm).observer_queued = false;
    let ready: Vec<(Gc, Value, Vec<Value>)> = {
        let h = host(vm);
        h.observers.iter_mut().filter(|o| !o.records.is_empty()).map(|o| (o.obj, o.callback.clone(), core::mem::take(&mut o.records))).collect()
    };
    for (obj, cb, records) in ready {
        let list = vm.new_array(records);
        vm.steps = 0;
        if let Err(e) = vm.call(&cb, Value::Obj(obj), &[Value::Obj(list), Value::Obj(obj)]) {
            super::report(vm, &e, "a mutation observer");
        }
        vm.run_jobs();
    }
}
