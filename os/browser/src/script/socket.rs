//! WebSockets: connections a page's scripts keep open to a server, messages going both ways (RFC 6455, as the WHATWG's
//! WebSocket interface gives them to scripts). What scripts ask (to open one, to send a message, to close it) is
//! queued (Host::socket_ops) for the system, which keeps the connection (over its own TCP and TLS); what comes of it
//! comes back through `event`: it's open, each message, it's closed.

use super::events::proto_from;
use super::window::{array_buffer, blob_parts, bytes_of, make_blob};
use super::*;
use alloc::vec;
use js::object::ALL;

/// What a page's scripts ask of their WebSockets, for the system to do.
#[derive(Clone, Debug, PartialEq)]
pub enum SocketOp {
    /// Open one: its number, its address (ws: or wss:), and the subprotocols it offers the server.
    Open { id: u32, url: String, protocols: Vec<String> },
    /// A message to send: text (UTF-8), or binary.
    Send { id: u32, binary: bool, data: Vec<u8> },
    /// Close it, with the code and reason the script gave (if it gave them).
    Close { id: u32, code: Option<u16>, reason: String },
}

/// What came of a WebSocket, from the system.
#[derive(Clone, Debug, PartialEq)]
pub enum SocketEvent {
    /// It's open: the subprotocol the server chose ("" if none).
    Open(String),
    Text(String),
    Binary(Vec<u8>),
    /// It's closed: its code, its reason, and whether it closed cleanly (both sides said so). Not cleanly (1006):
    /// it couldn't connect, or the connection failed or was dropped.
    Closed(u16, String, bool),
}

const CONNECTING: f64 = 0.0;
const OPEN: f64 = 1.0;
const CLOSING: f64 = 2.0;
const CLOSED: f64 = 3.0;

/// The longest close reason, in UTF-8 bytes (a close frame's payload is at most 125 bytes, 2 of them its code).
const MOST_REASON: usize = 123;

pub fn init(vm: &mut Vm) {
    let p = interface(vm, "WebSocket", Some("EventTarget"), 1, Some(ws_new));
    for (i, name) in ["CONNECTING", "OPEN", "CLOSING", "CLOSED"].iter().enumerate() {
        constant(vm, p, name, i as f64);
    }
    for (name, f) in [("url", ws_url as NativeFn), ("readyState", ws_ready_state), ("bufferedAmount", ws_buffered), ("extensions", ws_extensions), ("protocol", ws_protocol)] {
        attribute(vm, p, name, f, None);
    }
    for name in ["open", "error", "close", "message"] {
        super::events::handler_property(vm, p, name);
    }
    attribute(vm, p, "binaryType", ws_binary_type, Some(ws_set_binary_type));
    method(vm, p, "close", 0, ws_close);
    method(vm, p, "send", 1, ws_send);
}

/// A WebSocket's state object (its fields are its state's).
fn ws_state(vm: &mut Vm, this: &Value) -> Result<Gc, Value> {
    if let Value::Obj(s) = slot(vm, this, host_ref(vm).syms.data)
        && matches!(get(vm, s, "kind"), Ok(Value::Str(k)) if k.to_rust() == "WebSocket")
    {
        return Ok(s);
    }
    Err(vm.type_error("Illegal invocation"))
}

fn st(vm: &mut Vm, s: Gc, name: &str) -> Value {
    get(vm, s, name).unwrap_or(Value::Undefined)
}

fn st_set(vm: &mut Vm, s: Gc, name: &str, v: Value) {
    vm.put(s, name, v, ALL);
}

fn st_num(vm: &mut Vm, s: Gc, name: &str) -> f64 {
    match st(vm, s, name) {
        Value::Num(n) => n,
        _ => 0.0,
    }
}

fn st_str(vm: &mut Vm, s: Gc, name: &str) -> String {
    match st(vm, s, name) {
        Value::Str(t) => t.to_rust(),
        _ => String::new(),
    }
}

/// `new WebSocket(url, protocols)`: its address checked (ws: or wss:, http: and https: taken as them, no #fragment),
/// its subprotocols checked, and it's asked of the system to open.
fn ws_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "WebSocket")?;
    if args.is_empty() {
        return Err(vm.type_error("Failed to construct 'WebSocket': 1 argument required, but only 0 present."));
    }
    let given = string(vm, &args[0])?;
    let base = host_ref(vm).base.clone();
    let Some(mut u) = url::Url::parse(&given, Some(&base)) else {
        return Err(dom_exception(vm, "SyntaxError", &format!("Failed to construct 'WebSocket': The URL '{given}' is invalid.")));
    };
    match u.scheme.as_str() {
        "http" => u.scheme = String::from("ws"),
        "https" => u.scheme = String::from("wss"),
        "ws" | "wss" => {}
        other => {
            let m = format!("Failed to construct 'WebSocket': The URL's scheme must be either 'http', 'https', 'ws', or 'wss'. '{other}' is not allowed.");
            return Err(dom_exception(vm, "SyntaxError", &m));
        }
    }
    if let Some(f) = &u.fragment {
        let m = format!("Failed to construct 'WebSocket': The URL contains a fragment identifier ('{f}'). Fragment identifiers are not allowed in WebSocket URLs.");
        return Err(dom_exception(vm, "SyntaxError", &m));
    }
    let protocols = protocols_of(vm, &arg(args, 1))?;
    let x = vm.alloc(Some(p), Class::Ordinary);
    let s = vm.new_object();
    let id = host(vm).next_id();
    let href = u.href();
    let origin = u.origin();
    for (k, v) in [
        ("kind", js("WebSocket")),
        ("id", Value::Num(id as f64)),
        ("url", js(&href)),
        ("origin", js(&origin)),
        ("readyState", Value::Num(CONNECTING)),
        ("bufferedAmount", Value::Num(0.0)),
        ("protocol", js("")),
        ("extensions", js("")),
        ("binaryType", js("blob")),
    ] {
        st_set(vm, s, k, v);
    }
    set_slot(vm, x, host_ref(vm).syms.data, Value::Obj(s));
    let h = host(vm);
    h.sockets.push((id, x));
    h.socket_ops.push(SocketOp::Open { id, url: href, protocols });
    Ok(Value::Obj(x))
}

/// The subprotocols a WebSocket offers (a string, or a list of them): each a token (printable ASCII, none of HTTP's
/// separators), none twice.
fn protocols_of(vm: &mut Vm, v: &Value) -> Result<Vec<String>, Value> {
    let list = match v {
        Value::Undefined => return Ok(Vec::new()),
        Value::Obj(_) => {
            let it = Key::Sym(vm.sym(Sym::Iterator));
            if vm.get_method(v, it)?.is_undefined() {
                vec![string(vm, v)?]
            } else {
                let mut out = Vec::new();
                for p in vm.iterate_to_vec(v)? {
                    out.push(string(vm, &p)?);
                }
                out
            }
        }
        _ => vec![string(vm, v)?],
    };
    for (i, p) in list.iter().enumerate() {
        let token = !p.is_empty() && p.bytes().all(|b| (0x21..=0x7e).contains(&b) && !b"()<>@,;:\\\"/[]?={}".contains(&b));
        if !token {
            return Err(dom_exception(vm, "SyntaxError", &format!("Failed to construct 'WebSocket': The subprotocol '{p}' is invalid.")));
        }
        if list[..i].contains(p) {
            return Err(dom_exception(vm, "SyntaxError", &format!("Failed to construct 'WebSocket': The subprotocol '{p}' is duplicated.")));
        }
    }
    Ok(list)
}

fn ws_url(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = ws_state(vm, this)?;
    Ok(st(vm, s, "url"))
}

fn ws_ready_state(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = ws_state(vm, this)?;
    Ok(st(vm, s, "readyState"))
}

fn ws_buffered(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = ws_state(vm, this)?;
    Ok(st(vm, s, "bufferedAmount"))
}

fn ws_extensions(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = ws_state(vm, this)?;
    Ok(st(vm, s, "extensions"))
}

fn ws_protocol(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = ws_state(vm, this)?;
    Ok(st(vm, s, "protocol"))
}

fn ws_binary_type(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = ws_state(vm, this)?;
    Ok(st(vm, s, "binaryType"))
}

/// binaryType = "blob" or "arraybuffer" (anything else is ignored, as an enumeration's attribute does).
fn ws_set_binary_type(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = ws_state(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    if v == "blob" || v == "arraybuffer" {
        st_set(vm, s, "binaryType", js(&v));
    }
    Ok(Value::Undefined)
}

/// send(data): a string as text, a Blob, an ArrayBuffer or a view of one as binary. Not while it's connecting; once
/// it's closing or closed, nothing's sent (but bufferedAmount counts it, as the standard says).
fn ws_send(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = ws_state(vm, this)?;
    if args.is_empty() {
        return Err(vm.type_error("Failed to execute 'send' on 'WebSocket': 1 argument required, but only 0 present."));
    }
    let state = st_num(vm, s, "readyState");
    if state == CONNECTING {
        return Err(dom_exception(vm, "InvalidStateError", "Failed to execute 'send' on 'WebSocket': Still in CONNECTING state."));
    }
    let data = &args[0];
    let (binary, bytes) = if let Some((b, _)) = blob_parts(vm, data) {
        (true, b)
    } else if let Some(b) = bytes_of(vm, data) {
        (true, b)
    } else {
        (false, string(vm, data)?.into_bytes())
    };
    let buffered = st_num(vm, s, "bufferedAmount");
    st_set(vm, s, "bufferedAmount", Value::Num(buffered + bytes.len() as f64));
    if state == OPEN {
        let id = st_num(vm, s, "id") as u32;
        host(vm).socket_ops.push(SocketOp::Send { id, binary, data: bytes });
    }
    Ok(Value::Undefined)
}

/// A number as WebIDL's [Clamp] unsigned short takes it: NaN as 0, held to 0 to 65535, rounded to the nearest
/// (halves to the even one).
fn clamp_u16(n: f64) -> u16 {
    if n.is_nan() {
        return 0;
    }
    let x = n.clamp(0.0, 65535.0);
    let whole = x as u32;
    let frac = x - whole as f64;
    let r = if frac > 0.5 || (frac == 0.5 && whole % 2 == 1) { whole + 1 } else { whole };
    r.min(65535) as u16
}

/// close(code, reason): the code 1000 or 3000 to 4999, the reason at most 123 bytes. Before it's open, the
/// connection's given up (it fails: an error, then closed, 1006); after, the closing handshake begins.
fn ws_close(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = ws_state(vm, this)?;
    let code = match arg(args, 0) {
        Value::Undefined => None,
        v => {
            let n = vm.to_number(&v)?;
            let c = clamp_u16(n);
            if c != 1000 && !(3000..=4999).contains(&c) {
                let m = format!("Failed to execute 'close' on 'WebSocket': The close code must be either 1000, or between 3000 and 4999. {c} is neither.");
                return Err(dom_exception(vm, "InvalidAccessError", &m));
            }
            Some(c)
        }
    };
    let reason = match arg(args, 1) {
        Value::Undefined => None,
        v => Some(string(vm, &v)?),
    };
    if reason.as_ref().is_some_and(|r| r.len() > MOST_REASON) {
        return Err(dom_exception(vm, "SyntaxError", "Failed to execute 'close' on 'WebSocket': The close reason must not be greater than 123 UTF-8 bytes."));
    }
    let state = st_num(vm, s, "readyState");
    if state == CLOSING || state == CLOSED {
        return Ok(Value::Undefined);
    }
    st_set(vm, s, "readyState", Value::Num(CLOSING));
    let id = st_num(vm, s, "id") as u32;
    // (A reason goes after a code in the close frame: given alone, it goes with 1000.)
    let code = code.or(reason.as_ref().map(|_| 1000));
    host(vm).socket_ops.push(SocketOp::Close { id, code, reason: reason.unwrap_or_default() });
    Ok(Value::Undefined)
}

/// What was sent of a WebSocket's messages (handed to the system): no longer counted in its bufferedAmount.
pub fn sent(vm: &mut Vm, id: u32, bytes: usize) {
    let Some(x) = host_ref(vm).sockets.iter().find(|(i, _)| *i == id).map(|(_, g)| *g) else { return };
    let Ok(s) = ws_state(vm, &Value::Obj(x)) else { return };
    let buffered = st_num(vm, s, "bufferedAmount");
    st_set(vm, s, "bufferedAmount", Value::Num((buffered - bytes as f64).max(0.0)));
}

/// What came of a WebSocket: its state moved on, and its events fired (open; message; error if it failed, then
/// close).
pub fn event(vm: &mut Vm, id: u32, ev: SocketEvent) {
    let Some(x) = host_ref(vm).sockets.iter().find(|(i, _)| *i == id).map(|(_, g)| *g) else { return };
    let xv = Value::Obj(x);
    let Ok(s) = ws_state(vm, &xv) else { return };
    let state = st_num(vm, s, "readyState");
    match ev {
        SocketEvent::Open(protocol) => {
            // (Closed by its script while it was connecting: it won't open.)
            if state == CONNECTING {
                st_set(vm, s, "readyState", Value::Num(OPEN));
                st_set(vm, s, "protocol", js(&protocol));
                super::events::fire(vm, &xv, &EventInit::new("open", "Event", false, false));
            }
        }
        SocketEvent::Text(t) => {
            if state == OPEN {
                let origin = st(vm, s, "origin");
                super::events::fire_with(vm, &xv, &EventInit::new("message", "MessageEvent", false, false), vec![("data", js(&t)), ("origin", origin)]);
            }
        }
        SocketEvent::Binary(b) => {
            if state == OPEN {
                let origin = st(vm, s, "origin");
                let data = if st_str(vm, s, "binaryType") == "arraybuffer" {
                    array_buffer(vm, b)
                } else {
                    let bp = host_ref(vm).proto("Blob").unwrap_or_else(|| vm.proto(I::ObjectProto));
                    Value::Obj(make_blob(vm, bp, b, ""))
                };
                super::events::fire_with(vm, &xv, &EventInit::new("message", "MessageEvent", false, false), vec![("data", data), ("origin", origin)]);
            }
        }
        SocketEvent::Closed(code, reason, clean) => {
            if state != CLOSED {
                st_set(vm, s, "readyState", Value::Num(CLOSED));
                if !clean {
                    super::events::fire(vm, &xv, &EventInit::new("error", "Event", false, false));
                }
                let init = EventInit::new("close", "CloseEvent", false, false)
                    .with("wasClean", EvValue::Bool(clean))
                    .with("code", EvValue::Num(code as f64))
                    .with("reason", EvValue::Str(reason));
                super::events::fire(vm, &xv, &init);
            }
            host(vm).sockets.retain(|(i, _)| *i != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::clamp_u16;

    #[test]
    fn close_codes_are_clamped_as_webidl_says() {
        assert_eq!(clamp_u16(f64::NAN), 0);
        assert_eq!(clamp_u16(-5.0), 0);
        assert_eq!(clamp_u16(1000.4), 1000);
        assert_eq!(clamp_u16(1000.5), 1000);
        assert_eq!(clamp_u16(1001.5), 1002);
        assert_eq!(clamp_u16(70000.0), 65535);
        assert_eq!(clamp_u16(f64::INFINITY), 65535);
    }
}
