//! The network as scripts reach it: fetch (with Request, Response and Headers) and XMLHttpRequest. What scripts ask
//! for is queued (Host::requests) for the system to fetch over its own connections; its answer comes back through
//! `answer`, which settles the fetch's promise or moves the request's readyState on with its events. data: addresses
//! are answered here.

use super::events::{EvValue, proto_from};
use super::window::{array_buffer, blob_parts, bytes_of, rejected, resolved, uint8_array};
use super::*;
use alloc::vec;
use js::object::{ALL, ENUMERABLE};

/// What a request is for: a fetch (its promise, and its abort signal), or an XMLHttpRequest (its object).
pub enum NetKind {
    Fetch(Gc, Option<Gc>),
    Xhr(Gc),
}

/// A request a script made, for the system to send.
pub struct NetRequest {
    pub id: u32,
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub kind: NetKind,
}

impl NetRequest {
    pub fn trace(&self, roots: &mut Vec<Value>) {
        match self.kind {
            NetKind::Fetch(p, s) => {
                roots.push(Value::Obj(p));
                if let Some(s) = s {
                    roots.push(Value::Obj(s));
                }
            }
            NetKind::Xhr(x) => roots.push(Value::Obj(x)),
        }
    }
}

/// The network's answer to a request.
#[derive(Clone, Debug, Default)]
pub struct Response {
    pub status: u16,
    pub status_text: String,
    /// Where it came from in the end (after redirects).
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub fn init(vm: &mut Vm) {
    let w = host_ref(vm).window;
    method(vm, w, "fetch", 1, fetch);
    // Headers.
    let hp = interface(vm, "Headers", None, 0, Some(headers_new));
    for (name, len, f) in [
        ("append", 2, headers_append as NativeFn),
        ("delete", 1, headers_delete),
        ("get", 1, headers_get),
        ("has", 1, headers_has),
        ("set", 2, headers_set),
        ("forEach", 1, headers_for_each),
        ("entries", 0, headers_entries),
        ("keys", 0, headers_keys),
        ("values", 0, headers_values),
        ("getSetCookie", 0, headers_set_cookie),
    ] {
        method(vm, hp, name, len, f);
    }
    let entries = get(vm, hp, "entries").unwrap_or(Value::Undefined);
    let k = Key::Sym(vm.sym(Sym::Iterator));
    vm.put_key(hp, k, entries, HIDDEN);
    // Body (Request's and Response's).
    for iface in ["Request", "Response"] {
        let p = interface(vm, iface, None, if iface == "Request" { 1 } else { 0 }, Some(if iface == "Request" { request_new } else { response_new }));
        method(vm, p, "text", 0, body_text);
        method(vm, p, "json", 0, body_json);
        method(vm, p, "arrayBuffer", 0, body_array_buffer);
        method(vm, p, "bytes", 0, body_bytes);
        method(vm, p, "blob", 0, body_blob);
        method(vm, p, "formData", 0, body_form_data);
        method(vm, p, "clone", 0, body_clone);
        attribute(vm, p, "bodyUsed", body_used, None);
        attribute(vm, p, "body", body_stream, None);
    }
    let rp = host_ref(vm).proto("Response").expect("made");
    attribute(vm, rp, "ok", response_ok, None);
    if let Some(Value::Obj(c)) = super::events::ctor_of(vm, rp) {
        method(vm, c, "json", 1, response_json_static);
        method(vm, c, "error", 0, response_error_static);
        method(vm, c, "redirect", 1, response_redirect_static);
    }
    // XMLHttpRequest.
    let xt = interface(vm, "XMLHttpRequestEventTarget", Some("EventTarget"), 0, None);
    for name in ["abort", "error", "load", "loadend", "loadstart", "progress", "timeout"] {
        super::events::handler_property(vm, xt, name);
    }
    interface(vm, "XMLHttpRequestUpload", Some("XMLHttpRequestEventTarget"), 0, None);
    let xp = interface(vm, "XMLHttpRequest", Some("XMLHttpRequestEventTarget"), 0, Some(xhr_new));
    super::events::handler_property(vm, xp, "readystatechange");
    for (i, name) in ["UNSENT", "OPENED", "HEADERS_RECEIVED", "LOADING", "DONE"].iter().enumerate() {
        constant(vm, xp, name, i as f64);
    }
    for (name, len, f) in [
        ("open", 2, xhr_open as NativeFn),
        ("setRequestHeader", 2, xhr_set_header),
        ("send", 0, xhr_send),
        ("abort", 0, xhr_abort),
        ("getResponseHeader", 1, xhr_get_header),
        ("getAllResponseHeaders", 0, xhr_all_headers),
        ("overrideMimeType", 1, xhr_override_mime),
    ] {
        method(vm, xp, name, len, f);
    }
    for (name, f) in [("readyState", xhr_ready_state as NativeFn), ("status", xhr_status), ("statusText", xhr_status_text), ("responseURL", xhr_response_url), ("responseText", xhr_response_text), ("response", xhr_response), ("responseXML", xhr_response_xml), ("upload", xhr_upload)] {
        attribute(vm, xp, name, f, None);
    }
    for (name, f, s) in [("responseType", xhr_response_type as NativeFn, xhr_set_response_type as NativeFn), ("timeout", xhr_timeout, xhr_set_timeout), ("withCredentials", xhr_with_credentials, xhr_set_with_credentials)] {
        attribute(vm, xp, name, f, Some(s));
    }
}

// --------------------------------------------------------------------------------------------- headers

fn headers_list(vm: &mut Vm, this: &Value) -> Result<Vec<(String, String)>, Value> {
    match slot(vm, this, host_ref(vm).syms.data) {
        Value::Obj(a) => {
            let mut out = Vec::new();
            for pair in vm.array_values(&Value::Obj(a)) {
                let p = vm.array_values(&pair);
                let k = string(vm, p.first().unwrap_or(&Value::Undefined))?;
                let v = string(vm, p.get(1).unwrap_or(&Value::Undefined))?;
                out.push((k, v));
            }
            Ok(out)
        }
        _ => Err(vm.type_error("Illegal invocation")),
    }
}

fn write_headers(vm: &mut Vm, g: Gc, list: &[(String, String)]) {
    let items: Vec<Value> = list.iter().map(|(k, v)| Value::Obj(vm.new_array(vec![js(k), js(v)]))).collect();
    let a = vm.new_array(items);
    set_slot(vm, g, host_ref(vm).syms.data, Value::Obj(a));
}

/// A new Headers object with these headers.
pub fn make_headers(vm: &mut Vm, list: &[(String, String)]) -> Value {
    let p = host_ref(vm).proto("Headers").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let g = vm.alloc(Some(p), Class::Ordinary);
    let lower: Vec<(String, String)> = list.iter().map(|(k, v)| (k.to_ascii_lowercase(), String::from(v.trim()))).collect();
    write_headers(vm, g, &lower);
    Value::Obj(g)
}

/// Headers from what a script gives (a Headers object, an object of names, or pairs).
fn headers_from(vm: &mut Vm, init: &Value) -> Result<Vec<(String, String)>, Value> {
    let mut out = Vec::new();
    match init {
        Value::Undefined | Value::Null => {}
        Value::Obj(o) => {
            let o = *o;
            if let Ok(list) = headers_list(vm, init) {
                return Ok(list);
            }
            let it = vm.sym(Sym::Iterator);
            let m = vm.get(o, Key::Sym(it), init)?;
            if vm.is_callable(&m) {
                for pair in vm.iterate_to_vec(init)? {
                    let p = vm.iterate_to_vec(&pair)?;
                    if p.len() != 2 {
                        return Err(vm.type_error("Failed to construct 'Headers': Invalid value"));
                    }
                    out.push((string(vm, &p[0])?.to_ascii_lowercase(), string(vm, &p[1])?));
                }
            } else {
                for k in vm.own_keys(o)? {
                    if let Value::Str(name) = vm.key_value(k) {
                        let v = vm.get(o, k, init)?;
                        out.push((name.to_rust().to_ascii_lowercase(), string(vm, &v)?));
                    }
                }
            }
        }
        _ => return Err(vm.type_error("Failed to construct 'Headers': The provided value is not of type 'HeadersInit'.")),
    }
    Ok(out)
}

fn headers_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let list = headers_from(vm, &arg(args, 0))?;
    let p = proto_from(vm, nt, "Headers")?;
    let g = vm.alloc(Some(p), Class::Ordinary);
    write_headers(vm, g, &list);
    Ok(Value::Obj(g))
}

fn this_headers(vm: &mut Vm, this: &Value) -> Result<(Gc, Vec<(String, String)>), Value> {
    let Value::Obj(g) = this else { return Err(vm.type_error("Illegal invocation")) };
    Ok((*g, headers_list(vm, this)?))
}

fn header_name(vm: &mut Vm, args: &[Value], what: &str) -> Result<String, Value> {
    let n = str_arg(vm, args, 0, what)?;
    if n.is_empty() || n.contains(|c: char| c.is_whitespace() || matches!(c, ':' | '"' | '(' | ')' | ',' | '/' | ';' | '<' | '=' | '>' | '?' | '@' | '[' | '\\' | ']' | '{' | '}')) {
        return Err(vm.type_error(&format!("Failed to execute '{what}' on 'Headers': Invalid name")));
    }
    Ok(n.to_ascii_lowercase())
}

fn headers_append(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, mut list) = this_headers(vm, this)?;
    let k = header_name(vm, args, "append")?;
    let v = str_arg(vm, args, 1, "append")?;
    list.push((k, String::from(v.trim())));
    write_headers(vm, g, &list);
    Ok(Value::Undefined)
}

fn headers_delete(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, mut list) = this_headers(vm, this)?;
    let k = header_name(vm, args, "delete")?;
    list.retain(|(a, _)| *a != k);
    write_headers(vm, g, &list);
    Ok(Value::Undefined)
}

fn headers_get(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, list) = this_headers(vm, this)?;
    let k = header_name(vm, args, "get")?;
    let vals: Vec<&str> = list.iter().filter(|(a, _)| *a == k).map(|(_, v)| v.as_str()).collect();
    Ok(if vals.is_empty() { Value::Null } else { js(&vals.join(", ")) })
}

fn headers_has(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, list) = this_headers(vm, this)?;
    let k = header_name(vm, args, "has")?;
    Ok(Value::Bool(list.iter().any(|(a, _)| *a == k)))
}

fn headers_set(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (g, mut list) = this_headers(vm, this)?;
    let k = header_name(vm, args, "set")?;
    let v = str_arg(vm, args, 1, "set")?;
    match list.iter().position(|(a, _)| *a == k) {
        Some(i) => {
            list[i].1 = String::from(v.trim());
            let mut seen = 0;
            list.retain(|(a, _)| {
                if *a != k {
                    return true;
                }
                seen += 1;
                seen == 1
            });
        }
        None => list.push((k, String::from(v.trim()))),
    }
    write_headers(vm, g, &list);
    Ok(Value::Undefined)
}

/// The headers as iteration gives them: sorted by name, a name's values joined (but each Set-Cookie alone).
fn sorted_headers(list: &[(String, String)]) -> Vec<(String, String)> {
    let mut names: Vec<&str> = list.iter().map(|(k, _)| k.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    let mut out = Vec::new();
    for n in names {
        if n == "set-cookie" {
            for (_, v) in list.iter().filter(|(k, _)| k == n) {
                out.push((String::from(n), v.clone()));
            }
            continue;
        }
        let vals: Vec<&str> = list.iter().filter(|(k, _)| k == n).map(|(_, v)| v.as_str()).collect();
        out.push((String::from(n), vals.join(", ")));
    }
    out
}

fn headers_iter(vm: &mut Vm, this: &Value, which: u8) -> R {
    let (_, list) = this_headers(vm, this)?;
    let items: Vec<Value> = sorted_headers(&list)
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

fn headers_entries(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    headers_iter(vm, this, 2)
}
fn headers_keys(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    headers_iter(vm, this, 0)
}
fn headers_values(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    headers_iter(vm, this, 1)
}

fn headers_for_each(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, list) = this_headers(vm, this)?;
    let f = arg(args, 0);
    if !vm.is_callable(&f) {
        return Err(vm.type_error("forEach's argument isn't a function"));
    }
    for (k, v) in sorted_headers(&list) {
        vm.call(&f, arg(args, 1), &[js(&v), js(&k), this.clone()])?;
    }
    Ok(Value::Undefined)
}

fn headers_set_cookie(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, list) = this_headers(vm, this)?;
    let items: Vec<Value> = list.iter().filter(|(k, _)| k == "set-cookie").map(|(_, v)| js(v)).collect();
    Ok(Value::Obj(vm.new_array(items)))
}

// ------------------------------------------------------------------------------------ Request, Response

/// A body a script gives (fetch's, a Response's, XMLHttpRequest's): its bytes and the type it implies.
fn body_from(vm: &mut Vm, v: &Value) -> Result<Option<(Vec<u8>, Option<String>)>, Value> {
    match v {
        Value::Undefined | Value::Null => Ok(None),
        Value::Str(s) => Ok(Some((s.to_rust().into_bytes(), Some(String::from("text/plain;charset=UTF-8"))))),
        Value::Obj(g) => {
            let g = *g;
            if let Some((b, t)) = blob_parts(vm, v) {
                return Ok(Some((b, if t.is_empty() { None } else { Some(t) })));
            }
            if let Some(b) = bytes_of(vm, v) {
                return Ok(Some((b, None)));
            }
            let is = |vm: &mut Vm, iface: &str| -> bool {
                let p = host_ref(vm).proto(iface);
                let mut at = vm.heap.get(g).proto;
                while let Some(x) = at {
                    if Some(x) == p {
                        return true;
                    }
                    at = vm.heap.get(x).proto;
                }
                false
            };
            if is(vm, "URLSearchParams") {
                let m = get(vm, g, "toString")?;
                let s = vm.call(&m, v.clone(), &[])?;
                let s = string(vm, &s)?;
                return Ok(Some((s.into_bytes(), Some(String::from("application/x-www-form-urlencoded;charset=UTF-8")))));
            }
            if is(vm, "FormData") {
                // multipart/form-data, with a boundary of its own.
                let entries = get(vm, g, "entries")?;
                let it = vm.call(&entries, v.clone(), &[])?;
                let pairs = vm.iterate_to_vec(&it)?;
                let boundary = format!("----ChatComputerFormBoundary{:08x}", host_ref(vm).next_id.wrapping_mul(2654435761));
                let mut out: Vec<u8> = Vec::new();
                for p in pairs {
                    let kv = vm.array_values(&p);
                    let k = string(vm, kv.first().unwrap_or(&Value::Undefined))?;
                    let val = kv.get(1).cloned().unwrap_or(Value::Undefined);
                    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
                    if let Some((b, t)) = blob_parts(vm, &val) {
                        let name = match &val {
                            Value::Obj(o) => get(vm, *o, "name").ok().and_then(|n| if let Value::Str(s) = n { Some(s.to_rust()) } else { None }).unwrap_or_else(|| String::from("blob")),
                            _ => String::from("blob"),
                        };
                        out.extend_from_slice(format!("Content-Disposition: form-data; name=\"{k}\"; filename=\"{name}\"\r\nContent-Type: {}\r\n\r\n", if t.is_empty() { "application/octet-stream" } else { &t }).as_bytes());
                        out.extend_from_slice(&b);
                    } else {
                        let s = string(vm, &val)?;
                        out.extend_from_slice(format!("Content-Disposition: form-data; name=\"{k}\"\r\n\r\n{s}").as_bytes());
                    }
                    out.extend_from_slice(b"\r\n");
                }
                out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
                return Ok(Some((out, Some(format!("multipart/form-data; boundary={boundary}")))));
            }
            let s = string(vm, v)?;
            Ok(Some((s.into_bytes(), Some(String::from("text/plain;charset=UTF-8")))))
        }
        other => {
            let s = string(vm, other)?;
            Ok(Some((s.into_bytes(), Some(String::from("text/plain;charset=UTF-8")))))
        }
    }
}

/// A Request's or Response's state: its body (an ArrayBuffer, or null), whether it's been read, and its other
/// members as own properties.
fn set_body(vm: &mut Vm, g: Gc, body: Option<Vec<u8>>) {
    let b = match body {
        Some(b) => array_buffer(vm, b),
        None => Value::Null,
    };
    set_slot(vm, g, host_ref(vm).syms.data, b);
    set_slot(vm, g, host_ref(vm).syms.target, Value::Bool(false));
}

fn request_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "Request")?;
    let g = vm.alloc(Some(p), Class::Ordinary);
    let (url, method, headers, body) = request_parts(vm, &arg(args, 0), &arg(args, 1))?;
    vm.put(g, "url", js(&url), ENUMERABLE);
    vm.put(g, "method", js(&method), ENUMERABLE);
    let h = make_headers(vm, &headers);
    vm.put(g, "headers", h, ENUMERABLE);
    for (k, v) in [("mode", "cors"), ("credentials", "same-origin"), ("cache", "default"), ("redirect", "follow"), ("referrer", "about:client"), ("referrerPolicy", ""), ("integrity", ""), ("destination", "")] {
        vm.put(g, k, js(v), ENUMERABLE);
    }
    vm.put(g, "keepalive", Value::Bool(false), ENUMERABLE);
    let signal = match arg(args, 1) {
        Value::Obj(o) => get(vm, o, "signal")?,
        _ => Value::Null,
    };
    vm.put(g, "signal", signal, ENUMERABLE);
    set_body(vm, g, body);
    Ok(Value::Obj(g))
}

/// fetch's (or new Request's) input and options, worked out: (url, method, headers, body).
#[allow(clippy::type_complexity)]
fn request_parts(vm: &mut Vm, input: &Value, init: &Value) -> Result<(String, String, Vec<(String, String)>, Option<Vec<u8>>), Value> {
    let (mut url, mut method, mut headers, mut body) = match input {
        Value::Obj(o) if host_ref(vm).proto("Request").is_some_and(|rp| vm.heap.get(*o).proto == Some(rp)) => {
            let o = *o;
            let u = get(vm, o, "url")?;
            let m = get(vm, o, "method")?;
            let h = get(vm, o, "headers")?;
            let b = slot(vm, input, host_ref(vm).syms.data);
            (string(vm, &u)?, string(vm, &m)?, headers_from(vm, &h)?, bytes_of(vm, &b))
        }
        v => {
            let s = string(vm, v)?;
            (s, String::from("GET"), Vec::new(), None)
        }
    };
    let resolved_url = match url::Url::parse(&url, Some(&host_ref(vm).base)) {
        Some(u) => u,
        None => return Err(vm.type_error(&format!("Failed to execute 'fetch' on 'Window': Failed to parse URL from {url}"))),
    };
    url = resolved_url.href();
    if let Value::Obj(o) = init {
        let o = *o;
        let m = get(vm, o, "method")?;
        if !m.is_undefined() {
            method = string(vm, &m)?.to_ascii_uppercase();
        }
        let h = get(vm, o, "headers")?;
        if !h.is_undefined() {
            headers = headers_from(vm, &h)?;
        }
        let b = get(vm, o, "body")?;
        if let Some((bytes, ty)) = body_from(vm, &b)? {
            if method == "GET" || method == "HEAD" {
                return Err(vm.type_error("Failed to execute 'fetch' on 'Window': Request with GET/HEAD method cannot have body."));
            }
            if let Some(t) = ty
                && !headers.iter().any(|(k, _)| k == "content-type")
            {
                headers.push((String::from("content-type"), t));
            }
            body = Some(bytes);
        }
    }
    Ok((url, method, headers, body))
}

fn response_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "Response")?;
    let init = arg(args, 1);
    let (mut status, mut status_text, mut headers) = (200u16, String::new(), Vec::new());
    if let Value::Obj(o) = &init {
        let o = *o;
        let s = get(vm, o, "status")?;
        if !s.is_undefined() {
            let n = vm.to_number(&s)?;
            if !(200.0..=599.0).contains(&n) {
                return Err(vm.range_error("Failed to construct 'Response': The status provided is outside the range [200, 599]."));
            }
            status = n as u16;
        }
        let t = get(vm, o, "statusText")?;
        if !t.is_undefined() {
            status_text = string(vm, &t)?;
        }
        let h = get(vm, o, "headers")?;
        headers = headers_from(vm, &h)?;
    }
    let body = body_from(vm, &arg(args, 0))?;
    if let Some((_, Some(t))) = &body
        && !headers.iter().any(|(k, _)| k == "content-type")
    {
        headers.push((String::from("content-type"), t.clone()));
    }
    let r = Response { status, status_text, url: String::new(), headers, body: body.map(|b| b.0).unwrap_or_default() };
    let g = make_response(vm, &r);
    vm.heap.get_mut(g).proto = Some(p);
    Ok(Value::Obj(g))
}

/// A Response object for what the network gave.
fn make_response(vm: &mut Vm, r: &Response) -> Gc {
    let p = host_ref(vm).proto("Response").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let g = vm.alloc(Some(p), Class::Ordinary);
    vm.put(g, "status", Value::Num(r.status as f64), ENUMERABLE);
    vm.put(g, "statusText", js(&r.status_text), ENUMERABLE);
    vm.put(g, "url", js(&r.url), ENUMERABLE);
    vm.put(g, "type", js(if r.status == 0 { "error" } else { "basic" }), ENUMERABLE);
    vm.put(g, "redirected", Value::Bool(false), ENUMERABLE);
    let h = make_headers(vm, &r.headers);
    vm.put(g, "headers", h, ENUMERABLE);
    set_body(vm, g, Some(r.body.clone()));
    g
}

fn response_ok(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Value::Obj(g) = this else { return Err(vm.type_error("Illegal invocation")) };
    let s = get(vm, *g, "status")?;
    let s = vm.to_number(&s)?;
    Ok(Value::Bool((200.0..300.0).contains(&s)))
}

fn response_json_static(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let w = host_ref(vm).window;
    let json = get(vm, w, "JSON")?;
    let Value::Obj(jg) = json else { return Ok(Value::Undefined) };
    let stringify = get(vm, jg, "stringify")?;
    let s = vm.call(&stringify, json, &[arg(args, 0)])?;
    let s = string(vm, &s)?;
    let r = Response { status: 200, status_text: String::new(), url: String::new(), headers: vec![(String::from("content-type"), String::from("application/json"))], body: s.into_bytes() };
    Ok(Value::Obj(make_response(vm, &r)))
}

fn response_error_static(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Obj(make_response(vm, &Response::default())))
}

fn response_redirect_static(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let to = string(vm, &arg(args, 0))?;
    let status = if arg(args, 1).is_undefined() { 302 } else { vm.to_number(&arg(args, 1))? as u16 };
    let r = Response { status, status_text: String::new(), url: String::new(), headers: vec![(String::from("location"), to)], body: Vec::new() };
    Ok(Value::Obj(make_response(vm, &r)))
}

/// A body's bytes, read once (a TypeError the second time).
fn consume(vm: &mut Vm, this: &Value) -> Result<Vec<u8>, Value> {
    let Value::Obj(g) = this else { return Err(vm.type_error("Illegal invocation")) };
    if slot(vm, this, host_ref(vm).syms.target).truthy() {
        return Err(vm.type_error("Failed to execute: body stream already read"));
    }
    set_slot(vm, *g, host_ref(vm).syms.target, Value::Bool(true));
    let b = slot(vm, this, host_ref(vm).syms.data);
    Ok(bytes_of(vm, &b).unwrap_or_default())
}

/// A promise for a body read as something (its exceptions rejecting it).
fn body_promise(vm: &mut Vm, this: &Value, f: impl FnOnce(&mut Vm, Vec<u8>) -> R) -> R {
    match consume(vm, this).and_then(|b| f(vm, b)) {
        Ok(v) => Ok(resolved(vm, v)),
        Err(e) => {
            let p = vm.new_promise();
            vm.reject_promise(p, e);
            Ok(Value::Obj(p))
        }
    }
}

fn body_text(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    body_promise(vm, this, |_, b| Ok(js(&String::from_utf8_lossy(b.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&b)))))
}

fn body_json(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    body_promise(vm, this, |vm, b| {
        let s = String::from_utf8_lossy(b.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&b)).into_owned();
        let w = host_ref(vm).window;
        let json = get(vm, w, "JSON")?;
        let Value::Obj(jg) = json else { return Ok(Value::Undefined) };
        let parse = get(vm, jg, "parse")?;
        vm.call(&parse, json, &[js(&s)])
    })
}

fn body_array_buffer(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    body_promise(vm, this, |vm, b| Ok(array_buffer(vm, b)))
}

fn body_bytes(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    body_promise(vm, this, uint8_array)
}

fn body_blob(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let ty = match this {
        Value::Obj(g) => {
            let h = get(vm, *g, "headers")?;
            let list = headers_list(vm, &h).unwrap_or_default();
            list.into_iter().find(|(k, _)| k == "content-type").map(|(_, v)| v).unwrap_or_default()
        }
        _ => String::new(),
    };
    body_promise(vm, this, move |vm, b| {
        let w = host_ref(vm).window;
        let c = get(vm, w, "Blob")?;
        let buf = array_buffer(vm, b);
        let parts = vm.new_array(vec![buf]);
        let o = vm.new_object();
        vm.put(o, "type", js(&ty), ALL);
        vm.construct(&c, &[Value::Obj(parts), Value::Obj(o)], &c)
    })
}

fn body_form_data(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let e = vm.type_error("Reading a body as form data isn't supported yet.");
    Ok(rejected(vm, e))
}

fn body_clone(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Value::Obj(g) = this else { return Err(vm.type_error("Illegal invocation")) };
    if slot(vm, this, host_ref(vm).syms.target).truthy() {
        return Err(vm.type_error("Failed to execute 'clone': body has already been used"));
    }
    let proto = vm.heap.get(*g).proto;
    let c = vm.alloc(proto, Class::Ordinary);
    for k in vm.own_keys_plain(*g) {
        if let Some(Prop { slot: Slot::Data(v), flags }) = vm.heap.get(*g).props.get(&k).cloned() {
            vm.put_key(c, k, v, flags);
        }
    }
    let b = slot(vm, this, host_ref(vm).syms.data);
    let bytes = bytes_of(vm, &b);
    set_body(vm, c, bytes);
    // (Its own headers, a copy.)
    let h = get(vm, *g, "headers")?;
    let list = headers_list(vm, &h).unwrap_or_default();
    let nh = make_headers(vm, &list);
    vm.put(c, "headers", nh, ENUMERABLE);
    Ok(Value::Obj(c))
}

fn body_used(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(slot(vm, this, host_ref(vm).syms.target).truthy()))
}

fn body_stream(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    // (No streams yet: a body is there as a whole.)
    let b = slot(vm, this, host_ref(vm).syms.data);
    Ok(if matches!(b, Value::Null) { Value::Null } else { Value::Undefined })
}

// ----------------------------------------------------------------------------------------------- fetch

/// A data: address's bytes and type.
fn data_url(s: &str) -> Option<(Vec<u8>, String)> {
    let rest = s.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let base64 = meta.to_ascii_lowercase().ends_with(";base64");
    let ty = meta.trim_end_matches(";base64").trim_end_matches(";BASE64");
    let ty = if ty.is_empty() { String::from("text/plain;charset=US-ASCII") } else { String::from(ty) };
    let bytes = if base64 { super::window::base64_decode(&url::unescape(data, false))? } else { url::unescape(data, false).into_bytes() };
    Some((bytes, ty))
}

fn fetch(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.new_promise();
    let parts = request_parts(vm, &arg(args, 0), &arg(args, 1));
    let (url, method, headers, body) = match parts {
        Ok(x) => x,
        Err(e) => {
            vm.reject_promise(p, e);
            return Ok(Value::Obj(p));
        }
    };
    let signal = match arg(args, 1) {
        Value::Obj(o) => match get(vm, o, "signal")? {
            Value::Obj(s) => Some(s),
            _ => None,
        },
        _ => None,
    };
    if let Some(s) = signal
        && slot(vm, &Value::Obj(s), host_ref(vm).syms.data).truthy()
    {
        let r = slot(vm, &Value::Obj(s), host_ref(vm).syms.target);
        vm.reject_promise(p, r);
        return Ok(Value::Obj(p));
    }
    if url.starts_with("data:") {
        match data_url(&url) {
            Some((bytes, ty)) => {
                let r = Response { status: 200, status_text: String::from("OK"), url: url.clone(), headers: vec![(String::from("content-type"), ty)], body: bytes };
                let g = make_response(vm, &r);
                vm.fulfill_promise(p, Value::Obj(g));
            }
            None => {
                let e = vm.type_error("Failed to fetch");
                vm.reject_promise(p, e);
            }
        }
        return Ok(Value::Obj(p));
    }
    if !(url.starts_with("https:") || url.starts_with("http:")) {
        let e = vm.type_error("Failed to fetch");
        vm.reject_promise(p, e);
        return Ok(Value::Obj(p));
    }
    let h = host(vm);
    let id = h.next_id();
    h.requests.push(NetRequest { id, url, method, headers, body, kind: NetKind::Fetch(p, signal) });
    Ok(Value::Obj(p))
}

/// The network's answer to request `id` (or why there was none).
pub fn answer(vm: &mut Vm, id: u32, response: Result<Response, String>) {
    let at = host_ref(vm).waiting.iter().position(|r| r.id == id);
    let Some(at) = at else { return };
    let req = host(vm).waiting.remove(at);
    match req.kind {
        NetKind::Fetch(p, signal) => {
            if let Some(s) = signal
                && slot(vm, &Value::Obj(s), host_ref(vm).syms.data).truthy()
            {
                let r = slot(vm, &Value::Obj(s), host_ref(vm).syms.target);
                vm.reject_promise(p, r);
                return;
            }
            match response {
                Ok(r) => {
                    let g = make_response(vm, &r);
                    vm.fulfill_promise(p, Value::Obj(g));
                }
                Err(why) => {
                    let e = vm.type_error(&format!("Failed to fetch ({why})"));
                    vm.reject_promise(p, e);
                }
            }
        }
        NetKind::Xhr(x) => xhr_done(vm, x, response),
    }
}

// -------------------------------------------------------------------------------------- XMLHttpRequest

/// An XMLHttpRequest's state object (its fields are its state's).
fn xhr_state(vm: &mut Vm, this: &Value) -> Result<Gc, Value> {
    match slot(vm, this, host_ref(vm).syms.data) {
        Value::Obj(s) => Ok(s),
        _ => Err(vm.type_error("Illegal invocation")),
    }
}

fn st(vm: &mut Vm, s: Gc, name: &str) -> Value {
    get(vm, s, name).unwrap_or(Value::Undefined)
}

fn st_set(vm: &mut Vm, s: Gc, name: &str, v: Value) {
    vm.put(s, name, v, ALL);
}

fn xhr_new(vm: &mut Vm, nt: &Value, _args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "XMLHttpRequest")?;
    let x = vm.alloc(Some(p), Class::Ordinary);
    let s = vm.new_object();
    for (k, v) in [("readyState", Value::Num(0.0)), ("status", Value::Num(0.0)), ("statusText", js("")), ("responseType", js("")), ("timeout", Value::Num(0.0)), ("withCredentials", Value::Bool(false)), ("url", js("")), ("method", js("GET")), ("responseURL", js("")), ("body", Value::Null), ("id", Value::Num(0.0))] {
        st_set(vm, s, k, v);
    }
    let headers = vm.new_array(Vec::new());
    st_set(vm, s, "requestHeaders", Value::Obj(headers));
    let up = host_ref(vm).proto("XMLHttpRequestUpload").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let upload = vm.alloc(Some(up), Class::Ordinary);
    st_set(vm, s, "upload", Value::Obj(upload));
    set_slot(vm, x, host_ref(vm).syms.data, Value::Obj(s));
    Ok(Value::Obj(x))
}

fn ready_state_change(vm: &mut Vm, x: &Value, s: Gc, state: u8) {
    st_set(vm, s, "readyState", Value::Num(state as f64));
    super::events::fire(vm, x, &EventInit::new("readystatechange", "Event", false, false));
}

fn progress(vm: &mut Vm, x: &Value, ty: &str, loaded: f64) {
    let init = EventInit::new(ty, "ProgressEvent", false, false).with("loaded", EvValue::Num(loaded)).with("total", EvValue::Num(loaded)).with("lengthComputable", EvValue::Bool(loaded > 0.0));
    super::events::fire(vm, x, &init);
}

fn xhr_open(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = xhr_state(vm, this)?;
    let method = str_arg(vm, args, 0, "open")?.to_ascii_uppercase();
    let u = str_arg(vm, args, 1, "open")?;
    let Some(full) = url::Url::parse(&u, Some(&host_ref(vm).base)) else {
        return Err(dom_exception(vm, "SyntaxError", &format!("Failed to execute 'open' on 'XMLHttpRequest': Invalid URL '{u}'")));
    };
    st_set(vm, s, "method", js(&method));
    st_set(vm, s, "url", js(&full.href()));
    st_set(vm, s, "status", Value::Num(0.0));
    st_set(vm, s, "statusText", js(""));
    st_set(vm, s, "response", Value::Undefined);
    let headers = vm.new_array(Vec::new());
    st_set(vm, s, "requestHeaders", Value::Obj(headers));
    st_set(vm, s, "aborted", Value::Bool(false));
    ready_state_change(vm, this, s, 1);
    Ok(Value::Undefined)
}

fn xhr_set_header(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = xhr_state(vm, this)?;
    let k = str_arg(vm, args, 0, "setRequestHeader")?;
    let v = str_arg(vm, args, 1, "setRequestHeader")?;
    let Value::Obj(a) = st(vm, s, "requestHeaders") else { return Ok(Value::Undefined) };
    let pair = vm.new_array(vec![js(&k.to_ascii_lowercase()), js(&v)]);
    let mut items = vm.array_values(&Value::Obj(a));
    items.push(Value::Obj(pair));
    let na = vm.new_array(items);
    st_set(vm, s, "requestHeaders", Value::Obj(na));
    Ok(Value::Undefined)
}

fn xhr_send(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = xhr_state(vm, this)?;
    let Value::Obj(x) = this else { return Err(vm.type_error("Illegal invocation")) };
    let ready = st(vm, s, "readyState");
    if !matches!(ready, Value::Num(n) if n == 1.0) {
        return Err(dom_exception(vm, "InvalidStateError", "Failed to execute 'send' on 'XMLHttpRequest': The object's state must be OPENED."));
    }
    let url = { let v = st(vm, s, "url"); string(vm, &v) }?;
    let method = { let v = st(vm, s, "method"); string(vm, &v) }?;
    let mut headers: Vec<(String, String)> = Vec::new();
    let hv = st(vm, s, "requestHeaders");
    for pair in vm.array_values(&hv) {
        let p = vm.array_values(&pair);
        let k = string(vm, p.first().unwrap_or(&Value::Undefined))?;
        let v = string(vm, p.get(1).unwrap_or(&Value::Undefined))?;
        headers.push((k, v));
    }
    let body = if method == "GET" || method == "HEAD" { None } else { body_from(vm, &arg(args, 0))? };
    let body = body.map(|(b, ty)| {
        if let Some(t) = ty
            && !headers.iter().any(|(k, _)| k == "content-type")
        {
            headers.push((String::from("content-type"), t));
        }
        b
    });
    progress(vm, this, "loadstart", 0.0);
    if url.starts_with("data:") {
        let r = data_url(&url).map(|(bytes, ty)| Response { status: 200, status_text: String::from("OK"), url: url.clone(), headers: vec![(String::from("content-type"), ty)], body: bytes }).ok_or_else(|| String::from("not a data address"));
        // (Answered soon, as the network would.)
        let id = host(vm).next_id();
        host(vm).waiting.push(NetRequest { id, url, method, headers, body, kind: NetKind::Xhr(*x) });
        let f = vm.native_with("", 0, deliver_data, None, Value::Num(id as f64));
        let text = match &r {
            Ok(resp) => uint8_array(vm, resp.body.clone()).unwrap_or(Value::Undefined),
            Err(_) => Value::Undefined,
        };
        let ty = r.as_ref().ok().and_then(|resp| resp.headers.first().map(|h| h.1.clone())).unwrap_or_default();
        vm.set_data2(f, text);
        set_slot(vm, f, host_ref(vm).syms.target, js(&ty));
        host(vm).tasks.push((Value::Obj(f), Value::Undefined, Vec::new()));
        return Ok(Value::Undefined);
    }
    let h = host(vm);
    let id = h.next_id();
    h.requests.push(NetRequest { id, url, method, headers, body, kind: NetKind::Xhr(*x) });
    st_set(vm, s, "id", Value::Num(id as f64));
    Ok(Value::Undefined)
}

fn deliver_data(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    let id = vm.to_number(&vm.data(f))? as u32;
    let bytes = bytes_of(vm, &vm.data2(f));
    let ty = string(vm, &slot(vm, &Value::Obj(f), host_ref(vm).syms.target)).unwrap_or_default();
    let r = match bytes {
        Some(b) => Ok(Response { status: 200, status_text: String::from("OK"), url: String::new(), headers: vec![(String::from("content-type"), ty)], body: b }),
        None => Err(String::from("not a data address")),
    };
    answer(vm, id, r);
    Ok(Value::Undefined)
}

/// An XMLHttpRequest's answer: its state moved on to DONE, with its events.
fn xhr_done(vm: &mut Vm, x: Gc, response: Result<Response, String>) {
    let xv = Value::Obj(x);
    let Ok(s) = xhr_state(vm, &xv) else { return };
    if st(vm, s, "aborted").truthy() {
        return;
    }
    match response {
        Ok(r) => {
            st_set(vm, s, "status", Value::Num(r.status as f64));
            st_set(vm, s, "statusText", js(&r.status_text));
            st_set(vm, s, "responseURL", js(&r.url));
            let h = make_headers(vm, &r.headers);
            st_set(vm, s, "responseHeaders", h);
            let buf = array_buffer(vm, r.body.clone());
            st_set(vm, s, "bytes", buf);
            ready_state_change(vm, &xv, s, 2);
            ready_state_change(vm, &xv, s, 3);
            progress(vm, &xv, "progress", r.body.len() as f64);
            ready_state_change(vm, &xv, s, 4);
            progress(vm, &xv, "load", r.body.len() as f64);
            progress(vm, &xv, "loadend", r.body.len() as f64);
        }
        Err(_) => {
            st_set(vm, s, "status", Value::Num(0.0));
            ready_state_change(vm, &xv, s, 4);
            progress(vm, &xv, "error", 0.0);
            progress(vm, &xv, "loadend", 0.0);
        }
    }
}

fn xhr_abort(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = xhr_state(vm, this)?;
    let Value::Obj(x) = this else { return Ok(Value::Undefined) };
    let x = *x;
    st_set(vm, s, "aborted", Value::Bool(true));
    let h = host(vm);
    let before = h.requests.len() + h.waiting.len();
    h.requests.retain(|r| !matches!(r.kind, NetKind::Xhr(g) if g == x));
    h.waiting.retain(|r| !matches!(r.kind, NetKind::Xhr(g) if g == x));
    let was_sent = before != h.requests.len() + h.waiting.len();
    if was_sent {
        st_set(vm, s, "status", Value::Num(0.0));
        ready_state_change(vm, this, s, 4);
        progress(vm, this, "abort", 0.0);
        progress(vm, this, "loadend", 0.0);
    }
    st_set(vm, s, "readyState", Value::Num(0.0));
    Ok(Value::Undefined)
}

fn xhr_get_header(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = xhr_state(vm, this)?;
    let name = str_arg(vm, args, 0, "getResponseHeader")?.to_ascii_lowercase();
    let h = st(vm, s, "responseHeaders");
    let list = headers_list(vm, &h).unwrap_or_default();
    let vals: Vec<&str> = list.iter().filter(|(k, _)| *k == name && k != "set-cookie").map(|(_, v)| v.as_str()).collect();
    Ok(if vals.is_empty() { Value::Null } else { js(&vals.join(", ")) })
}

fn xhr_all_headers(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = xhr_state(vm, this)?;
    let h = st(vm, s, "responseHeaders");
    let list = headers_list(vm, &h).unwrap_or_default();
    let mut out = String::new();
    for (k, v) in sorted_headers(&list) {
        if k == "set-cookie" {
            continue;
        }
        out.push_str(&k);
        out.push_str(": ");
        out.push_str(&v);
        out.push_str("\r\n");
    }
    Ok(js(&out))
}

fn xhr_override_mime(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Undefined)
}

fn xhr_field(vm: &mut Vm, this: &Value, name: &str) -> R {
    let s = xhr_state(vm, this)?;
    Ok(st(vm, s, name))
}

fn xhr_ready_state(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    xhr_field(vm, this, "readyState")
}
fn xhr_status(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    xhr_field(vm, this, "status")
}
fn xhr_status_text(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    xhr_field(vm, this, "statusText")
}
fn xhr_response_url(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    xhr_field(vm, this, "responseURL")
}
fn xhr_upload(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    xhr_field(vm, this, "upload")
}
fn xhr_response_type(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    xhr_field(vm, this, "responseType")
}
fn xhr_timeout(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    xhr_field(vm, this, "timeout")
}
fn xhr_with_credentials(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    xhr_field(vm, this, "withCredentials")
}

fn xhr_set_field(vm: &mut Vm, this: &Value, name: &str, v: Value) -> R {
    let s = xhr_state(vm, this)?;
    st_set(vm, s, name, v);
    Ok(Value::Undefined)
}

fn xhr_set_response_type(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let t = string(vm, &arg(args, 0))?;
    if !matches!(t.as_str(), "" | "text" | "json" | "arraybuffer" | "blob" | "document") {
        return Ok(Value::Undefined);
    }
    xhr_set_field(vm, this, "responseType", js(&t))
}

fn xhr_set_timeout(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = vm.to_number(&arg(args, 0))?;
    xhr_set_field(vm, this, "timeout", Value::Num(n))
}

fn xhr_set_with_credentials(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let b = arg(args, 0).truthy();
    xhr_set_field(vm, this, "withCredentials", Value::Bool(b))
}

fn xhr_bytes(vm: &mut Vm, s: Gc) -> Vec<u8> {
    let b = st(vm, s, "bytes");
    bytes_of(vm, &b).unwrap_or_default()
}

fn xhr_response_text(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = xhr_state(vm, this)?;
    let t = { let v = st(vm, s, "responseType"); string(vm, &v) }?;
    if !t.is_empty() && t != "text" {
        return Err(dom_exception(vm, "InvalidStateError", "Failed to read the 'responseText' property from 'XMLHttpRequest': The value is only accessible if the object's 'responseType' is '' or 'text'."));
    }
    let b = xhr_bytes(vm, s);
    Ok(js(&String::from_utf8_lossy(b.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&b))))
}

fn xhr_response(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let s = xhr_state(vm, this)?;
    let done = matches!(st(vm, s, "readyState"), Value::Num(n) if n == 4.0);
    let t = { let v = st(vm, s, "responseType"); string(vm, &v) }?;
    match t.as_str() {
        "" | "text" => xhr_response_text(vm, this, args, f),
        _ if !done => Ok(Value::Null),
        "json" => {
            let cached = st(vm, s, "response");
            if !cached.is_undefined() {
                return Ok(cached);
            }
            let b = xhr_bytes(vm, s);
            let text = String::from_utf8_lossy(&b).into_owned();
            let w = host_ref(vm).window;
            let json = get(vm, w, "JSON")?;
            let Value::Obj(jg) = json else { return Ok(Value::Null) };
            let parse = get(vm, jg, "parse")?;
            let v = vm.call(&parse, json, &[js(&text)]).unwrap_or(Value::Null);
            st_set(vm, s, "response", v.clone());
            Ok(v)
        }
        "arraybuffer" => {
            let cached = st(vm, s, "response");
            if !cached.is_undefined() {
                return Ok(cached);
            }
            let b = xhr_bytes(vm, s);
            let buf = array_buffer(vm, b);
            st_set(vm, s, "response", buf.clone());
            Ok(buf)
        }
        "blob" => {
            let b = xhr_bytes(vm, s);
            let w = host_ref(vm).window;
            let c = get(vm, w, "Blob")?;
            let buf = array_buffer(vm, b);
            let parts = vm.new_array(vec![buf]);
            vm.construct(&c, &[Value::Obj(parts)], &c)
        }
        _ => xhr_response_xml(vm, this, args, f),
    }
}

fn xhr_response_xml(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = xhr_state(vm, this)?;
    let b = xhr_bytes(vm, s);
    let text = String::from_utf8_lossy(&b).into_owned();
    let w = host_ref(vm).window;
    let c = get(vm, w, "DOMParser")?;
    let p = vm.construct(&c, &[], &c)?;
    let Value::Obj(pg) = p else { return Ok(Value::Null) };
    let parse = get(vm, pg, "parseFromString")?;
    vm.call(&parse, p, &[js(&text), js("text/html")])
}
