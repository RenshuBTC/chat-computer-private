//! A page's scripts, run by Chat Computer's own JavaScript engine (os/js), with what scripts use in a browser: the
//! window (the global object), the document and its nodes (dom.rs), events (events.rs), timers, the address, the
//! console, storage and the like (window.rs), the network (net.rs) and WebSockets (socket.rs), and live lists,
//! class lists, styles and data attributes (lists.rs). The engine knows nothing of pages: these are its host's objects, `Class::Host` objects
//! whose state is here (the `Host`, in the VM's host slot), and the page's tree is moved into it while scripts run.

pub mod animations;
mod canvas;
mod dom;
mod events;
mod geometry;
mod lists;
mod modules;
mod net;
mod socket;
pub mod url;
mod window;

use crate::dom::{Data, Document, NodeId};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use js::object::{CONFIGURABLE, Class, HIDDEN, Key, NativeFn, Prop, SYM_PRIVATE, Slot};
use js::string::JsStr;
use js::value::{Gc, Value};
use js::vm::{I, Sym, Vm};

pub use canvas::{Canvas, Pixels};
pub use dom::{ScriptKind, runnable_type, script_kind};
pub use events::{EvValue, EventInit};
pub use geometry::{LayoutInputs, Picker, fresh_layout, rects_of};
pub use net::{NetKind, NetRequest, Response};
pub use socket::{SocketEvent, SocketOp};
pub use window::seed;

/// What a `Class::Host` object is (its number is a node's, or a list's).
pub const NODE: u32 = 1;
pub const STYLE: u32 = 2;
pub const TOKENS: u32 = 3;
pub const DATASET: u32 = 4;
pub const LIST: u32 = 5;
pub const ATTR: u32 = 6;

/// The most steps (calls and loops' turns) one script, event or timer may take before it's stopped.
const STEPS: u64 = 40_000_000;
/// The most lines the console keeps for the system to read.
const CONSOLE_LINES: usize = 400;

/// An event listener: on what, for which events, and how.
pub struct Listener {
    pub target: Gc,
    pub ty: String,
    pub callback: Value,
    pub capture: bool,
    pub once: bool,
    pub passive: bool,
    pub removed: bool,
}

/// A timer (setTimeout, setInterval).
pub struct Timer {
    pub id: u32,
    /// When it's due (ms since the page began), and how often it repeats.
    pub due: f64,
    pub every: Option<f64>,
    pub f: Value,
    pub args: Vec<Value>,
    /// How deeply timers set it (each one set from a timer is one deeper: past 5 they wait at least 4 ms).
    pub depth: u32,
}

/// A live list's kind: what of `root`'s it holds.
#[derive(Clone, PartialEq)]
pub enum ListKind {
    /// Its children (NodeList childNodes).
    ChildNodes,
    /// Its children that are elements (HTMLCollection children).
    Children,
    /// The elements inside it with this name ("*": all) (getElementsByTagName).
    Tag(String),
    /// The elements inside it with all these classes (getElementsByClassName).
    Class(Vec<String>),
    /// The elements inside it whose name attribute is this (getElementsByName).
    Name(String),
    /// A <select>'s options; a form's controls.
    Options,
    Controls,
}

/// The page's side of the engine: its tree (while scripts run), each node's object, the listeners, the timers, the
/// console's lines, what scripts asked of the network.
pub struct Host {
    pub doc: Document,
    /// The page's address (as location shows it), and where links lead from (<base href>'s).
    pub url: url::Url,
    pub base: url::Url,
    /// Each node's object, once a script has it (by node).
    pub wrappers: Vec<Option<Gc>>,
    /// The interfaces' prototypes, by name ("HTMLDivElement").
    pub protos: BTreeMap<&'static str, Gc>,
    pub window: Gc,
    pub listeners: Vec<Listener>,
    pub timers: Vec<Timer>,
    pub next_id: u32,
    /// requestAnimationFrame's callbacks (id, function).
    pub frames: Vec<(u32, Value)>,
    /// The page's clock: ms since it began (performance.now), and Date.now's at its start.
    pub now: f64,
    pub epoch: f64,
    /// The tree changed: to lay out again.
    pub mutated: bool,
    /// Something scripts changed that the page reads again: style sheets, pictures (and scripts to fetch).
    pub rescan: bool,
    pub console: Vec<String>,
    pub local: BTreeMap<String, String>,
    pub session: BTreeMap<String, String>,
    /// What scripts asked the network for, not yet handed to the system; and those it has, waiting for answers.
    pub requests: Vec<NetRequest>,
    pub waiting: Vec<NetRequest>,
    /// The page's WebSockets not yet closed (by their numbers), and what scripts asked of them, not yet handed to
    /// the system.
    pub sockets: Vec<(u32, Gc)>,
    pub socket_ops: Vec<SocketOp>,
    /// Where a script sends the page (location.href = ...).
    pub navigate: Option<url::Url>,
    /// Live lists: (root, kind), by number; and each list's elements as last worked out (with the tree's version).
    pub lists: Vec<(NodeId, ListKind)>,
    pub list_cache: BTreeMap<u32, (u64, Vec<NodeId>)>,
    /// Objects belonging to a node kept for it ((node, which): its style, class list, dataset, lists...).
    pub cache: BTreeMap<(NodeId, u32), Gc>,
    /// The <script> running now (document.currentScript).
    pub current_script: Option<NodeId>,
    /// Scripts put in the page by scripts, to fetch and run (their elements).
    pub new_scripts: Vec<NodeId>,
    /// The scripts that have run or are coming (each element once).
    pub started: Vec<NodeId>,
    /// The view: its width and height in CSS pixels, the device's pixels to one, how far it's scrolled.
    pub view: (f32, f32, f32),
    pub scroll: (f32, f32),
    /// Where each node's box was when the page was last laid out (x, y, width, height in CSS pixels).
    pub rects: Vec<Option<(f32, f32, f32, f32)>>,
    /// The element with the focus.
    pub focus: Option<NodeId>,
    /// Private symbols for objects' inner state.
    pub syms: Syms,
    /// Mutation observers, and records waiting for them.
    pub observers: Vec<events::Observer>,
    pub observer_queued: bool,
    /// Tasks posted to run soon (postMessage's events, a script's load event): (function, this, arguments).
    pub tasks: Vec<(Value, Value, Vec<Value>)>,
    /// Cookies a script set (name=value), for the page's requests.
    pub cookies: Vec<(String, String)>,
    pub title_changed: bool,
    /// Whether the page's load event has been fired.
    pub loaded: bool,
    /// The custom elements defined: (name, constructor); and the attributes each observes (its class's
    /// observedAttributes, read when it was defined).
    pub custom: Vec<(String, Value)>,
    pub observed: BTreeMap<String, Vec<String>>,
    /// A form a script sent (form.submit(), or its button's click): (the form, the button).
    pub submit: Option<(NodeId, Option<NodeId>)>,
    /// history.back() and the like: how far to go.
    pub history_go: Option<i32>,
    /// Storage changed (for the system to keep).
    pub storage_changed: bool,
    /// Styles the page worked out at its last layout, by (element, property), for getComputedStyle.
    pub computed: BTreeMap<(NodeId, String), String>,
    /// The page's import maps, merged.
    pub import_map: modules::ImportMap,
    /// What a layout on the spot takes (the page's sheets and pictures), the tree's version `rects` (and `styles`)
    /// are of, the styles worked out then, and how many layouts this task has made on the spot.
    pub layout_inputs: Option<alloc::rc::Rc<LayoutInputs>>,
    pub laid_version: u64,
    pub styles: Option<alloc::sync::Arc<crate::style::Styles>>,
    pub forced_layouts: u32,
    /// The page's canvases (each one's pixels and its context's state), and whether one was drawn on since the
    /// page last looked.
    pub canvases: Vec<canvas::Canvas>,
    pub canvas_drawn: bool,
    /// The page's pictures' pixels as they've come (by resource), for drawing on canvases.
    pub pixels: Vec<Option<alloc::sync::Arc<Pixels>>>,
    /// Pictures scripts made that aren't in the page (new Image(), its src set): the page fetches them too.
    pub images: Vec<NodeId>,
    /// The <img>s whose picture has come (its size) or couldn't be (None): complete, naturalWidth.
    pub settled: BTreeMap<NodeId, Option<(u32, u32)>>,
    /// Cookies its scripts set (document.cookie = ...: as they wrote them), for the system's cookies.
    pub cookies_set: Vec<String>,
    /// <style> sheets scripts changed through their rules (insertRule, deleteRule): the text each was read from,
    /// and its rules now.
    pub cssom: BTreeMap<NodeId, (String, Vec<String>)>,
    /// The animations its scripts made (element.animate).
    pub animations: Vec<animations::ScriptAnimation>,
}

/// The private symbols holding objects' inner state (scripts never see these keys).
#[derive(Default, Clone, Copy)]
pub struct Syms {
    /// An event's state (events.rs), and a few other objects' (their numbers or values).
    pub event: Option<Gc>,
    pub data: Option<Gc>,
    pub target: Option<Gc>,
}

impl Host {
    fn new(url: url::Url, window: Gc) -> Host {
        Host {
            doc: Document::new(),
            base: url.clone(),
            url,
            wrappers: Vec::new(),
            protos: BTreeMap::new(),
            window,
            listeners: Vec::new(),
            timers: Vec::new(),
            next_id: 1,
            frames: Vec::new(),
            now: 0.0,
            epoch: 0.0,
            mutated: false,
            rescan: false,
            console: Vec::new(),
            local: BTreeMap::new(),
            session: BTreeMap::new(),
            requests: Vec::new(),
            waiting: Vec::new(),
            sockets: Vec::new(),
            socket_ops: Vec::new(),
            navigate: None,
            lists: Vec::new(),
            list_cache: BTreeMap::new(),
            cache: BTreeMap::new(),
            current_script: None,
            new_scripts: Vec::new(),
            started: Vec::new(),
            view: (1024.0, 768.0, 1.0),
            scroll: (0.0, 0.0),
            rects: Vec::new(),
            focus: None,
            syms: Syms::default(),
            observers: Vec::new(),
            observer_queued: false,
            tasks: Vec::new(),
            cookies: Vec::new(),
            title_changed: false,
            loaded: false,
            custom: Vec::new(),
            observed: BTreeMap::new(),
            submit: None,
            history_go: None,
            storage_changed: false,
            computed: BTreeMap::new(),
            import_map: modules::ImportMap::default(),
            layout_inputs: None,
            laid_version: u64::MAX,
            styles: None,
            forced_layouts: 0,
            canvases: Vec::new(),
            canvas_drawn: false,
            pixels: Vec::new(),
            images: Vec::new(),
            settled: BTreeMap::new(),
            cookies_set: Vec::new(),
            cssom: BTreeMap::new(),
            animations: Vec::new(),
        }
    }

    /// A line for the console (kept for the system to read; the oldest let go).
    pub fn log(&mut self, line: String) {
        if self.console.len() >= CONSOLE_LINES {
            self.console.remove(0);
        }
        self.console.push(line);
    }

    pub fn proto(&self, name: &str) -> Option<Gc> {
        self.protos.get(name).copied()
    }

    pub fn next_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// The tree changed (by a script): laid out again, lists worked out again.
    pub fn changed(&mut self) {
        self.mutated = true;
        self.doc.version += 1;
    }
}

/// The values the host holds, for the collector.
fn trace(h: &dyn core::any::Any, roots: &mut Vec<Value>) {
    let Some(h) = h.downcast_ref::<Host>() else { return };
    roots.extend(h.wrappers.iter().flatten().map(|g| Value::Obj(*g)));
    roots.extend(h.protos.values().map(|g| Value::Obj(*g)));
    roots.extend(h.cache.values().map(|g| Value::Obj(*g)));
    roots.push(Value::Obj(h.window));
    for l in &h.listeners {
        roots.push(Value::Obj(l.target));
        roots.push(l.callback.clone());
    }
    for t in &h.timers {
        roots.push(t.f.clone());
        roots.extend(t.args.iter().cloned());
    }
    roots.extend(h.frames.iter().map(|(_, f)| f.clone()));
    for r in h.requests.iter().chain(h.waiting.iter()) {
        r.trace(roots);
    }
    roots.extend(h.sockets.iter().map(|(_, g)| Value::Obj(*g)));
    for o in &h.observers {
        o.trace(roots);
    }
    for (f, t, a) in &h.tasks {
        roots.push(f.clone());
        roots.push(t.clone());
        roots.extend(a.iter().cloned());
    }
    roots.extend(h.custom.iter().map(|(_, c)| c.clone()));
    for s in [h.syms.event, h.syms.data, h.syms.target].into_iter().flatten() {
        roots.push(Value::Sym(s));
    }
    canvas::trace(h, roots);
}

/// The host, from inside a built-in function.
pub fn host(vm: &mut Vm) -> &mut Host {
    vm.host.as_mut().and_then(|h| h.downcast_mut::<Host>()).expect("a page's scripts have their host")
}

pub fn host_ref(vm: &Vm) -> &Host {
    vm.host.as_ref().and_then(|h| h.downcast_ref::<Host>()).expect("a page's scripts have their host")
}

// ------------------------------------------------------------------------------------------------- helpers

type R = Result<Value, Value>;

pub fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}

pub fn js(s: &str) -> Value {
    Value::str(s)
}

/// A value as a Rust string (ToString).
pub fn string(vm: &mut Vm, v: &Value) -> Result<String, Value> {
    Ok(vm.to_string(v)?.to_rust())
}

/// An argument as a string (ToString), or a TypeError if it wasn't given.
pub fn str_arg(vm: &mut Vm, args: &[Value], i: usize, what: &str) -> Result<String, Value> {
    if i >= args.len() {
        return Err(vm.type_error(&format!("Failed to execute '{what}': {} argument required, but only {} present.", i + 1, args.len())));
    }
    string(vm, &args[i])
}

/// A DOMException (its name, as the standard has them: "NotFoundError", "SyntaxError"...).
pub fn dom_exception(vm: &mut Vm, name: &str, message: &str) -> Value {
    let p = host_ref(vm).proto("DOMException").unwrap_or_else(|| vm.proto(I::ErrorProto));
    let g = vm.alloc(Some(p), Class::Error);
    vm.put(g, "message", js(message), HIDDEN);
    vm.put(g, "name", js(name), HIDDEN);
    let code = match name {
        "IndexSizeError" => 1,
        "HierarchyRequestError" => 3,
        "WrongDocumentError" => 4,
        "InvalidCharacterError" => 5,
        "NotFoundError" => 8,
        "NotSupportedError" => 9,
        "InvalidStateError" => 11,
        "SyntaxError" => 12,
        "InvalidAccessError" => 15,
        "SecurityError" => 18,
        "NetworkError" => 19,
        "AbortError" => 20,
        "QuotaExceededError" => 22,
        "TimeoutError" => 23,
        "DataCloneError" => 25,
        _ => 0,
    };
    vm.put(g, "code", Value::Num(code as f64), HIDDEN);
    let stack = vm.stack_trace(js::vm::ErrorKind::Error, message);
    vm.put(g, "stack", Value::Str(stack), HIDDEN);
    Value::Obj(g)
}

/// A node's object (made the first time a script has it).
pub fn wrap(vm: &mut Vm, id: NodeId) -> Value {
    if let Some(Some(g)) = host_ref(vm).wrappers.get(id) {
        return Value::Obj(*g);
    }
    let proto = dom::proto_for(vm, id);
    let mut g = vm.alloc(Some(proto), Class::Host(NODE, id as u64));
    // (A form's a proxy over that, for its controls by name too, as browsers have them: form.amount.)
    if host_ref(vm).doc.name(id) == "form"
        && let Some(handler) = host_ref(vm).proto("(form handler)")
    {
        g = vm.alloc(None, Class::Proxy(Some((g, handler)), 0));
    }
    let h = host(vm);
    if h.wrappers.len() <= id {
        h.wrappers.resize(id + 1, None);
    }
    h.wrappers[id] = Some(g);
    Value::Obj(g)
}

pub fn wrap_opt(vm: &mut Vm, id: Option<NodeId>) -> Value {
    match id {
        Some(n) => wrap(vm, n),
        None => Value::Null,
    }
}

/// The node an object is, if it's one.
pub fn node_of(vm: &Vm, v: &Value) -> Option<NodeId> {
    match v {
        Value::Obj(g) => match vm.heap.get(*g).class {
            Class::Host(NODE, id) => Some(id as usize),
            // (A form's proxy: the node under it.)
            Class::Proxy(Some((t, _)), _) => match vm.heap.get(t).class {
                Class::Host(NODE, id) => Some(id as usize),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// `this` as a node, or a TypeError ("Illegal invocation").
pub fn this_node(vm: &mut Vm, this: &Value) -> Result<NodeId, Value> {
    match node_of(vm, this) {
        Some(n) => Ok(n),
        None => Err(vm.type_error("Illegal invocation")),
    }
}

/// `this` as an element.
pub fn this_element(vm: &mut Vm, this: &Value) -> Result<NodeId, Value> {
    let n = this_node(vm, this)?;
    if host_ref(vm).doc.element(n).is_none() {
        return Err(vm.type_error("Illegal invocation"));
    }
    Ok(n)
}

/// An argument that must be a node.
pub fn node_arg(vm: &mut Vm, args: &[Value], i: usize, what: &str) -> Result<NodeId, Value> {
    match node_of(vm, &arg(args, i)) {
        Some(n) => Ok(n),
        None => Err(vm.type_error(&format!("Failed to execute '{what}': parameter {} is not of type 'Node'.", i + 1))),
    }
}

/// `this` as a host object of a kind: its number.
pub fn this_host(vm: &mut Vm, this: &Value, kind: u32) -> Result<u64, Value> {
    if let Value::Obj(g) = this
        && let Class::Host(k, id) = vm.heap.get(*g).class
        && k == kind
    {
        return Ok(id);
    }
    Err(vm.type_error("Illegal invocation"))
}

/// An object's inner value under a private symbol (Undefined if it hasn't one).
pub fn slot(vm: &Vm, o: &Value, sym: Option<Gc>) -> Value {
    match (o, sym) {
        (Value::Obj(g), Some(s)) => match vm.heap.get(*g).props.get(&Key::Sym(s)) {
            Some(Prop { slot: Slot::Data(v), .. }) => v.clone(),
            _ => Value::Undefined,
        },
        _ => Value::Undefined,
    }
}

pub fn set_slot(vm: &mut Vm, o: Gc, sym: Option<Gc>, v: Value) {
    if let Some(s) = sym {
        vm.put_key(o, Key::Sym(s), v, js::object::WRITABLE);
    }
}

/// A property of an object, by name.
pub fn get(vm: &mut Vm, o: Gc, name: &str) -> Result<Value, Value> {
    let a = vm.atom(name);
    vm.get(o, Key::Str(a), &Value::Obj(o))
}

/// A method on an object (non-enumerable, as built-ins are).
pub fn method(vm: &mut Vm, obj: Gc, name: &str, length: u32, f: NativeFn) {
    vm.method(obj, name, length, f);
}

/// A getter and setter on an object (enumerable and configurable, as the DOM's attributes are).
pub fn attribute(vm: &mut Vm, obj: Gc, name: &str, get: NativeFn, set: Option<NativeFn>) {
    let a = vm.atom(name);
    let g = Value::Obj(vm.native(&format!("get {name}"), 0, get));
    let s = match set {
        Some(f) => Value::Obj(vm.native(&format!("set {name}"), 1, f)),
        None => Value::Undefined,
    };
    vm.heap.get_mut(obj).props.set(Key::Str(a), Prop { slot: Slot::Accessor(g, s), flags: CONFIGURABLE | js::object::ENUMERABLE });
}

/// A getter and setter whose functions carry `data` (one function for many attributes).
pub fn attribute_with(vm: &mut Vm, obj: Gc, name: &str, get: NativeFn, set: Option<NativeFn>, data: Value) {
    let a = vm.atom(name);
    let g = Value::Obj(vm.native_with(&format!("get {name}"), 0, get, None, data.clone()));
    let s = match set {
        Some(f) => Value::Obj(vm.native_with(&format!("set {name}"), 1, f, None, data)),
        None => Value::Undefined,
    };
    vm.heap.get_mut(obj).props.set(Key::Str(a), Prop { slot: Slot::Accessor(g, s), flags: CONFIGURABLE | js::object::ENUMERABLE });
}

fn illegal(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Err(vm.type_error("Illegal constructor"))
}

/// An interface: its prototype (inheriting `parent`'s), and its constructor as a global (whose own prototype is
/// the parent's constructor). `construct`: what `new` does (None: "Illegal constructor").
pub fn interface(vm: &mut Vm, name: &'static str, parent: Option<&str>, length: u32, construct: Option<NativeFn>) -> Gc {
    let parent_proto = parent.and_then(|p| host_ref(vm).proto(p));
    let op = vm.proto(I::ObjectProto);
    let proto = vm.alloc(Some(parent_proto.unwrap_or(op)), Class::Ordinary);
    let c = vm.native_with(name, length, illegal, Some(construct.unwrap_or(illegal)), Value::Undefined);
    vm.put_key(c, Key::Str(js::names::PROTOTYPE), Value::Obj(proto), 0);
    vm.put_key(proto, Key::Str(js::names::CONSTRUCTOR), Value::Obj(c), HIDDEN);
    if let Some(pp) = parent_proto
        && let Some(Prop { slot: Slot::Data(Value::Obj(pc)), .. }) = vm.heap.get(pp).props.get(&Key::Str(js::names::CONSTRUCTOR)).cloned()
    {
        vm.heap.get_mut(c).proto = Some(pc);
    }
    let t = vm.sym(Sym::ToStringTag);
    vm.put_key(proto, Key::Sym(t), js(name), CONFIGURABLE);
    let w = host_ref(vm).window;
    vm.put(w, name, Value::Obj(c), HIDDEN);
    host(vm).protos.insert(name, proto);
    proto
}

/// A constant on an interface (its constructor and prototype both).
pub fn constant(vm: &mut Vm, proto: Gc, name: &str, v: f64) {
    vm.put(proto, name, Value::Num(v), js::object::ENUMERABLE);
    if let Some(Prop { slot: Slot::Data(Value::Obj(c)), .. }) = vm.heap.get(proto).props.get(&Key::Str(js::names::CONSTRUCTOR)).cloned() {
        vm.put(c, name, Value::Num(v), js::object::ENUMERABLE);
    }
}

/// An exception's text for the console ("TypeError: x is undefined" and where).
pub fn describe_error(vm: &mut Vm, e: &Value) -> String {
    if let Value::Obj(g) = e
        && matches!(vm.heap.get(*g).class, Class::Error)
    {
        let stack = vm.get(*g, Key::Str(js::names::STACK), e).unwrap_or(Value::Undefined);
        if let Value::Str(s) = stack {
            return s.to_rust();
        }
        let m = vm.get(*g, Key::Str(js::names::MESSAGE), e).unwrap_or(Value::Undefined);
        let name = vm.get(*g, Key::Str(js::names::NAME), e).unwrap_or(Value::Undefined);
        let (n, m) = (string(vm, &name).unwrap_or_default(), string(vm, &m).unwrap_or_default());
        return format!("{n}: {m}");
    }
    match vm.to_string(e) {
        Ok(s) => s.to_rust(),
        Err(_) => String::from("(an exception)"),
    }
}

// --------------------------------------------------------------------------------------------- the engine

/// A page's scripting: the engine, its window and document.
pub struct Script {
    pub vm: Vm,
}

// SAFETY: a page's scripting is one engine owned as a whole: nothing in it (its Rc's, its host's state) is shared
// with anything outside it, so moving it to another core, as the system does with what's under its locks, moves
// all of it at once, and only one core uses it at a time.
unsafe impl Send for Script {}

impl Script {
    /// A window for the page at `url`, its clock starting at `epoch` (ms since 1970: Date.now's).
    pub fn new(page_url: &str, epoch: f64, view: (f32, f32, f32)) -> Script {
        let mut vm = Vm::new();
        let u = url::Url::parse(page_url, None).unwrap_or_else(|| url::Url::parse("about:blank", None).unwrap_or_default());
        let window = vm.global;
        let mut h = Host::new(u, window);
        h.epoch = epoch;
        h.view = view;
        vm.host = Some(Box::new(h));
        vm.host_trace = Some(trace);
        vm.now = now_ms;
        vm.tz_offset = ZONE_OFFSET.load(core::sync::atomic::Ordering::Relaxed);
        // (Each call between the engine and a built-in function, nested, takes about 2 KiB of the stack the page runs
        // on in Chat Computer's own system, whose stack is 16 MiB; 4 KiB in a build for Linux. YouTube's scripts go
        // 164 deep.)
        vm.max_nesting = 1000;
        {
            let h = host(&mut vm);
            h.syms.event = None;
        }
        let ev = vm.new_symbol(Some(JsStr::from_str("[[event]]")), SYM_PRIVATE);
        let data = vm.new_symbol(Some(JsStr::from_str("[[data]]")), SYM_PRIVATE);
        let target = vm.new_symbol(Some(JsStr::from_str("[[target]]")), SYM_PRIVATE);
        host(&mut vm).syms = Syms { event: Some(ev), data: Some(data), target: Some(target) };
        events::init(&mut vm);
        dom::init(&mut vm);
        lists::init(&mut vm);
        window::init(&mut vm);
        animations::install(&mut vm);
        net::init(&mut vm);
        socket::init(&mut vm);
        canvas::init(&mut vm);
        // The document's object is node 0's.
        let d = wrap(&mut vm, 0);
        vm.put(window, "document", d, js::object::ENUMERABLE);
        let mut s = Script { vm };
        s.init_modules();
        s
    }

    pub fn host(&mut self) -> &mut Host {
        host(&mut self.vm)
    }

    /// The page's tree, given to the scripts (taken back with `give_back`).
    pub fn lend(&mut self, doc: &mut Document) {
        core::mem::swap(&mut self.host().doc, doc);
    }

    pub fn give_back(&mut self, doc: &mut Document) {
        core::mem::swap(&mut self.host().doc, doc);
    }

    /// Sets the clock (ms since the page began) for what runs next.
    pub fn set_now(&mut self, now: f64) {
        self.host().now = now;
        let t = self.host().epoch + now;
        CLOCK.store(t.to_bits(), core::sync::atomic::Ordering::Relaxed);
    }

    /// Runs a classic script (a <script>'s text, from `node`): exceptions go to the console.
    pub fn run(&mut self, source: &str, name: &str, node: Option<NodeId>) {
        let vm = &mut self.vm;
        host(vm).current_script = node;
        vm.steps = 0;
        vm.step_limit = STEPS;
        let r = vm.run_script(source, name);
        vm.step_limit = u64::MAX;
        host(vm).current_script = None;
        if let Err(e) = r {
            report(vm, &e, name);
        }
        self.checkpoint();
    }

    /// Runs the jobs waiting (promise reactions, mutation observers' callbacks).
    pub fn checkpoint(&mut self) {
        let vm = &mut self.vm;
        for _ in 0..8 {
            vm.steps = 0;
            vm.step_limit = STEPS;
            vm.run_jobs();
            vm.step_limit = u64::MAX;
            events::deliver_mutations(vm);
            if vm.jobs.is_empty() {
                break;
            }
        }
        let unhandled: Vec<Gc> = core::mem::take(&mut vm.unhandled);
        for p in unhandled {
            if let Class::Promise(d) = &vm.heap.get(p).class
                && let js::object::PromiseState::Rejected(reason) = &d.state
                && !d.handled
            {
                let reason = reason.clone();
                let text = describe_error(vm, &reason);
                host(vm).log(format!("Uncaught (in promise) {text}"));
            }
        }
    }

    /// Calls a function (an event's, a timer's), its exceptions reported.
    pub fn call(&mut self, f: &Value, this: Value, args: &[Value]) -> Option<Value> {
        let vm = &mut self.vm;
        vm.steps = 0;
        vm.step_limit = STEPS;
        let r = vm.call(f, this, args);
        vm.step_limit = u64::MAX;
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                report(vm, &e, "a callback");
                None
            }
        }
    }

    /// Fires an event at a node (the page's own: a click, typing, the page loading): whether nothing cancelled
    /// it (its default action is to happen).
    pub fn fire(&mut self, target: Option<NodeId>, init: &EventInit) -> bool {
        let vm = &mut self.vm;
        let t = match target {
            Some(n) => wrap(vm, n),
            None => Value::Obj(host_ref(vm).window),
        };
        let ok = events::fire(vm, &t, init);
        self.checkpoint();
        ok
    }

    /// The person sends a form (its submit button clicked, or Enter in one of its fields): its submit event first,
    /// which a script may cancel (sending what it must itself, with fetch); if none does, the form goes.
    pub fn submit_form(&mut self, form: NodeId, submitter: Option<NodeId>) {
        dom::submit_form(&mut self.vm, form, submitter);
        self.checkpoint();
    }

    /// Runs what's due: timers whose time has come, tasks posted. True if anything ran.
    pub fn tick(&mut self, now: f64) -> bool {
        self.set_now(now);
        let mut ran = false;
        // Tasks posted (postMessage, a script element's load event...).
        let tasks = core::mem::take(&mut self.host().tasks);
        for (f, this, args) in tasks {
            self.call(&f, this, &args);
            self.checkpoint();
            ran = true;
        }
        // Timers, earliest first, at most a few hundred at once.
        for _ in 0..200 {
            let due = {
                let h = self.host();
                let mut best: Option<usize> = None;
                for (i, t) in h.timers.iter().enumerate() {
                    if t.due <= now && best.is_none_or(|b| t.due < h.timers[b].due) {
                        best = Some(i);
                    }
                }
                best
            };
            let Some(i) = due else { break };
            let (f, args, id, depth) = {
                let h = self.host();
                let t = &mut h.timers[i];
                let r = (t.f.clone(), t.args.clone(), t.id, t.depth);
                match t.every {
                    Some(every) => t.due = now + every.max(if t.depth >= 5 { 4.0 } else { 0.0 }),
                    None => {
                        h.timers.remove(i);
                    }
                }
                r
            };
            window::TIMER_DEPTH.store(depth + 1, core::sync::atomic::Ordering::Relaxed);
            if let Value::Str(code) = &f {
                let src = code.to_rust();
                let vm = &mut self.vm;
                vm.steps = 0;
                vm.step_limit = STEPS;
                if let Err(e) = vm.indirect_eval(&src) {
                    report(vm, &e, "a timer");
                }
                vm.step_limit = u64::MAX;
            } else {
                let w = Value::Obj(self.host().window);
                self.call(&f, w, &args);
            }
            window::TIMER_DEPTH.store(0, core::sync::atomic::Ordering::Relaxed);
            let _ = id;
            self.checkpoint();
            ran = true;
        }
        ran
    }

    /// Runs requestAnimationFrame's callbacks (before the page is drawn): true if there were any.
    pub fn animation_frame(&mut self, now: f64) -> bool {
        self.set_now(now);
        let frames = core::mem::take(&mut self.host().frames);
        if frames.is_empty() {
            return false;
        }
        let w = Value::Obj(self.host().window);
        for (_, f) in frames {
            self.call(&f, w.clone(), &[Value::Num(now)]);
        }
        self.checkpoint();
        true
    }

    /// When the next timer is due (ms since the page began), if one is waiting.
    pub fn next_due(&mut self) -> Option<f64> {
        let h = self.host();
        let t = h.timers.iter().map(|t| t.due).fold(f64::INFINITY, f64::min);
        if !h.tasks.is_empty() || !h.frames.is_empty() {
            return Some(h.now);
        }
        if t.is_finite() { Some(t) } else { None }
    }

    /// The console's lines since last asked.
    pub fn take_console(&mut self) -> Vec<String> {
        core::mem::take(&mut self.host().console)
    }

    /// An answer from the network to a request scripts made.
    pub fn answer(&mut self, id: u32, response: Result<Response, String>) {
        net::answer(&mut self.vm, id, response);
        self.checkpoint();
    }

    /// What scripts asked of their WebSockets since last asked (to open one, to send, to close), for the system:
    /// the messages handed over no longer count in their bufferedAmount.
    pub fn take_socket_ops(&mut self) -> Vec<SocketOp> {
        let ops = core::mem::take(&mut self.host().socket_ops);
        for op in &ops {
            if let SocketOp::Send { id, data, .. } = op {
                socket::sent(&mut self.vm, *id, data.len());
            }
        }
        ops
    }

    /// What came of one of the page's WebSockets (it opened, a message, it closed): its events fired.
    pub fn socket_event(&mut self, id: u32, ev: SocketEvent) {
        socket::event(&mut self.vm, id, ev);
        self.checkpoint();
    }
}

/// An uncaught exception, to the console (and to the window's error listeners).
pub fn report(vm: &mut Vm, e: &Value, source: &str) {
    let text = describe_error(vm, e);
    let _ = source;
    host(vm).log(format!("Uncaught {text}"));
    events::error_event(vm, e, &text);
}

// The clock scripts read (Date.now): the page's, set before each run (the engine's clock is a plain function).
static CLOCK: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

// The local time's offset from UTC (minutes): what pages' scripts take as their time zone (Date's local time,
// Intl's zone).
static ZONE_OFFSET: core::sync::atomic::AtomicI32 = core::sync::atomic::AtomicI32::new(0);

/// Sets the time zone pages made after this run in: its offset from UTC, in minutes.
pub fn set_time_zone(offset_minutes: i32) {
    ZONE_OFFSET.store(offset_minutes.clamp(-1440, 1440), core::sync::atomic::Ordering::Relaxed);
}

fn now_ms() -> f64 {
    f64::from_bits(CLOCK.load(core::sync::atomic::Ordering::Relaxed))
}

/// Text from what a page's node holds (a script's own text).
pub fn text_of(doc: &Document, id: NodeId) -> String {
    doc.children(id).iter().filter_map(|&c| match &doc.nodes[c].data {
        Data::Text(t) => Some(t.as_str()),
        _ => None,
    }).collect::<Vec<&str>>().concat()
}
