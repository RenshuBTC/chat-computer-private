//! The DOM's lists and maps: live lists of nodes (childNodes, children, getElementsByTagName's: proxies whose
//! elements are worked out from the tree as it is when they're read), NodeList and HTMLCollection, class lists
//! (DOMTokenList), an element's inline style (CSSStyleDeclaration, its properties written into the style
//! attribute) and its data attributes (dataset).

use super::*;
use crate::dom::NodeId;
use js::object::{ALL, ENUMERABLE};

/// Which object of a node's the cache holds (beside the lists: their kinds' numbers).
const CLASS_LIST: u32 = 0x100;
const REL_LIST: u32 = 0x101;
const STYLE_OBJ: u32 = 0x102;
const DATASET_OBJ: u32 = 0x103;

pub fn init(vm: &mut Vm) {
    let ap = vm.proto(I::ArrayProto);
    let iter_key = Key::Sym(vm.sym(Sym::Iterator));
    let values = get(vm, ap, "values").unwrap_or(Value::Undefined);
    let nl = interface(vm, "NodeList", None, 0, None);
    method(vm, nl, "item", 1, list_item);
    for name in ["forEach", "entries", "keys", "values"] {
        let f = get(vm, ap, name).unwrap_or(Value::Undefined);
        vm.put(nl, name, f, HIDDEN | ENUMERABLE);
    }
    vm.put_key(nl, iter_key, values.clone(), HIDDEN);
    interface(vm, "RadioNodeList", Some("NodeList"), 0, None);
    let hc = interface(vm, "HTMLCollection", None, 0, None);
    method(vm, hc, "item", 1, list_item);
    method(vm, hc, "namedItem", 1, named_item);
    vm.put_key(hc, iter_key, values.clone(), HIDDEN);
    interface(vm, "HTMLAllCollection", Some("HTMLCollection"), 0, None);
    let oc = interface(vm, "HTMLOptionsCollection", Some("HTMLCollection"), 0, None);
    let _ = oc;
    interface(vm, "HTMLFormControlsCollection", Some("HTMLCollection"), 0, None);
    let nnm = interface(vm, "NamedNodeMap", None, 0, None);
    method(vm, nnm, "item", 1, array_item);
    method(vm, nnm, "getNamedItem", 1, named_node);
    vm.put_key(nnm, iter_key, values.clone(), HIDDEN);
    // The handler every live list's proxy shares.
    let h = vm.new_object();
    method(vm, h, "get", 3, trap_get);
    method(vm, h, "has", 2, trap_has);
    method(vm, h, "ownKeys", 1, trap_own_keys);
    method(vm, h, "getOwnPropertyDescriptor", 2, trap_descriptor);
    method(vm, h, "set", 4, trap_set);
    method(vm, h, "deleteProperty", 2, trap_delete);
    host(vm).protos.insert("(list handler)", h);

    // DOMTokenList.
    let tl = interface(vm, "DOMTokenList", None, 0, None);
    attribute(vm, tl, "length", tokens_length, None);
    attribute(vm, tl, "value", tokens_value, Some(set_tokens_value));
    method(vm, tl, "item", 1, tokens_item);
    method(vm, tl, "contains", 1, tokens_contains);
    method(vm, tl, "add", 0, tokens_add);
    method(vm, tl, "remove", 0, tokens_remove);
    method(vm, tl, "toggle", 1, tokens_toggle);
    method(vm, tl, "replace", 2, tokens_replace);
    method(vm, tl, "supports", 1, tokens_supports);
    method(vm, tl, "toString", 0, tokens_value);
    method(vm, tl, "forEach", 1, tokens_for_each);
    method(vm, tl, "values", 0, tokens_values);
    method(vm, tl, "entries", 0, tokens_entries);
    method(vm, tl, "keys", 0, tokens_keys);
    let tv = get(vm, tl, "values").unwrap_or(Value::Undefined);
    vm.put_key(tl, iter_key, tv, HIDDEN);
    let ep = host_ref(vm).proto("Element").expect("made");
    attribute(vm, ep, "classList", class_list, Some(set_class_list));
    let hp = host_ref(vm).proto("HTMLElement").expect("made");
    attribute(vm, hp, "style", style, Some(set_style));
    attribute(vm, hp, "dataset", dataset, None);

    // CSSStyleDeclaration.
    let sd = interface(vm, "CSSStyleDeclaration", None, 0, None);
    attribute(vm, sd, "cssText", css_text, Some(set_css_text));
    attribute(vm, sd, "length", style_length, None);
    attribute(vm, sd, "parentRule", null_value, None);
    attribute(vm, sd, "cssFloat", float_get, Some(float_set));
    method(vm, sd, "item", 1, style_item);
    method(vm, sd, "getPropertyValue", 1, get_property_value);
    method(vm, sd, "getPropertyPriority", 1, get_property_priority);
    method(vm, sd, "setProperty", 2, set_property);
    method(vm, sd, "removeProperty", 1, remove_property);
    for name in CSS_PROPERTIES {
        let camel = camel_case(name);
        attribute_with(vm, sd, &camel, prop_get, Some(prop_set), js(name));
        if camel != *name && !name.starts_with('-') {
            // (style["background-color"] works too.)
            attribute_with(vm, sd, name, prop_get, Some(prop_set), js(name));
        }
        if let Some(rest) = name.strip_prefix("-webkit-") {
            // webkitTransform and WebkitTransform.
            let c = camel_case(rest);
            let upper = format!("Webkit{}{}", c[..1].to_ascii_uppercase(), &c[1..]);
            attribute_with(vm, sd, &upper, prop_get, Some(prop_set), js(name));
        }
    }
    let ds = interface(vm, "DOMStringMap", None, 0, None);
    let _ = ds;
    let dh = vm.new_object();
    method(vm, dh, "get", 3, data_get);
    method(vm, dh, "set", 4, data_set);
    method(vm, dh, "has", 2, data_has);
    method(vm, dh, "deleteProperty", 2, data_delete);
    method(vm, dh, "ownKeys", 1, data_own_keys);
    method(vm, dh, "getOwnPropertyDescriptor", 2, data_descriptor);
    host(vm).protos.insert("(dataset handler)", dh);
}

fn null_value(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Null)
}

// ------------------------------------------------------------------------------------------- live lists

/// A live list of a node's (made once for each node and kind, so it's the same object each time).
pub fn live(vm: &mut Vm, root: NodeId, kind: ListKind) -> Value {
    let k = match &kind {
        ListKind::ChildNodes => 1,
        ListKind::Children => 2,
        ListKind::Options => 3,
        ListKind::Controls => 4,
        ListKind::Tag(t) => 0x1000 + (super::events::fnv(t) & 0xfff_ffff),
        ListKind::Class(c) => 0x1000_0000 + (super::events::fnv(&c.join(" ")) & 0xfff_ffff),
        ListKind::Name(n) => 0x2000_0000 + (super::events::fnv(n) & 0xfff_ffff),
    };
    let key = (root, 0x4000_0000 | (k & 0x3fff_ffff));
    if let Some(&g) = host_ref(vm).cache.get(&key) {
        return Value::Obj(g);
    }
    let iface = match kind {
        ListKind::ChildNodes => "NodeList",
        ListKind::Options => "HTMLOptionsCollection",
        ListKind::Controls => "HTMLFormControlsCollection",
        _ => "HTMLCollection",
    };
    let idx = {
        let h = host(vm);
        h.lists.push((root, kind));
        h.lists.len() - 1
    };
    let proto = host_ref(vm).proto(iface).unwrap_or_else(|| vm.proto(I::ObjectProto));
    let target = vm.alloc(Some(proto), Class::Host(LIST, idx as u64));
    let handler = host_ref(vm).proto("(list handler)").expect("made");
    let p = vm.alloc(None, Class::Proxy(Some((target, handler)), 0));
    host(vm).cache.insert(key, p);
    Value::Obj(p)
}

/// A live list's nodes now.
pub fn items(vm: &mut Vm, idx: usize) -> Vec<NodeId> {
    let version = host_ref(vm).doc.version;
    if let Some((v, list)) = host_ref(vm).list_cache.get(&(idx as u32))
        && *v == version
    {
        return list.clone();
    }
    let (root, kind) = host_ref(vm).lists[idx].clone();
    let d = &host_ref(vm).doc;
    let list: Vec<NodeId> = match &kind {
        ListKind::ChildNodes => d.children(root).to_vec(),
        ListKind::Children => d.children(root).iter().copied().filter(|&c| d.element(c).is_some()).collect(),
        ListKind::Tag(t) => d.descendants(root).skip(1).filter(|&e| d.element(e).is_some() && (t == "*" || d.name(e) == t)).collect(),
        ListKind::Class(cs) => {
            if cs.is_empty() {
                Vec::new()
            } else {
                d.descendants(root).skip(1).filter(|&e| d.element(e).is_some_and(|el| cs.iter().all(|c| el.has_class(c)))).collect()
            }
        }
        ListKind::Name(n) => d.descendants(root).skip(1).filter(|&e| d.element(e).and_then(|el| el.attr("name")) == Some(n.as_str())).collect(),
        ListKind::Options => d.descendants(root).filter(|&o| d.name(o) == "option").collect(),
        ListKind::Controls => controls(d, root),
    };
    host(vm).list_cache.insert(idx as u32, (version, list.clone()));
    list
}

/// A form's controls (those inside it, and those naming it in their form attribute), in the tree's order.
pub fn controls(d: &crate::dom::Document, form: NodeId) -> Vec<NodeId> {
    let id = d.element(form).and_then(|e| e.attr("id")).map(String::from);
    d.descendants(0)
        .filter(|&c| {
            let listed = matches!(d.name(c), "button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea")
                && !(d.name(c) == "input" && d.element(c).and_then(|e| e.attr("type")).is_some_and(|t| t.eq_ignore_ascii_case("image")));
            if !listed {
                return false;
            }
            match d.element(c).and_then(|e| e.attr("form")) {
                Some(f) => id.as_deref() == Some(f),
                None => d.ancestor_named(c, "form") == Some(form),
            }
        })
        .collect()
}

/// A key as an index, if it's one ("0", "1"...).
fn index_of(vm: &Vm, k: &Value) -> Option<usize> {
    match k {
        Value::Str(s) => {
            let t = s.to_rust();
            if t.is_empty() || (t.len() > 1 && t.starts_with('0')) || !t.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let _ = vm;
            t.parse().ok()
        }
        Value::Num(n) if *n >= 0.0 && js::math::trunc(*n) == *n => Some(*n as usize),
        _ => None,
    }
}

fn list_idx(vm: &Vm, target: &Value) -> Option<usize> {
    match target {
        Value::Obj(g) => match vm.heap.get(*g).class {
            Class::Host(LIST, i) => Some(i as usize),
            _ => None,
        },
        _ => None,
    }
}

/// An element in a list with this id or name (HTMLCollection's named items).
fn named(vm: &mut Vm, list: &[NodeId], name: &str) -> Option<NodeId> {
    let d = &host_ref(vm).doc;
    list.iter().copied().find(|&e| d.element(e).is_some_and(|el| el.attr("id") == Some(name) || (el.attr("name") == Some(name) && matches!(d.name(e), "a" | "button" | "embed" | "form" | "frame" | "frameset" | "iframe" | "img" | "input" | "map" | "meta" | "object" | "select" | "textarea"))))
}

fn trap_get(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (target, key, receiver) = (arg(args, 0), arg(args, 1), arg(args, 2));
    let Some(idx) = list_idx(vm, &target) else { return Ok(Value::Undefined) };
    if let Some(i) = index_of(vm, &key) {
        let list = items(vm, idx);
        return Ok(match list.get(i) {
            Some(&n) => wrap(vm, n),
            None => Value::Undefined,
        });
    }
    if let Value::Str(s) = &key
        && s.to_rust() == "length"
    {
        return Ok(Value::Num(items(vm, idx).len() as f64));
    }
    let Value::Obj(t) = target else { return Ok(Value::Undefined) };
    let k = vm.to_key(&key)?;
    if !vm.has_property(t, k)?
        && let Value::Str(s) = &key
        && !matches!(host_ref(vm).lists[idx].1, ListKind::ChildNodes)
    {
        let list = items(vm, idx);
        let name = s.to_rust();
        if let Some(n) = named(vm, &list, &name) {
            return Ok(wrap(vm, n));
        }
    }
    vm.get(t, k, &receiver)
}

fn trap_has(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (target, key) = (arg(args, 0), arg(args, 1));
    let Some(idx) = list_idx(vm, &target) else { return Ok(Value::Bool(false)) };
    if let Some(i) = index_of(vm, &key) {
        return Ok(Value::Bool(i < items(vm, idx).len()));
    }
    let Value::Obj(t) = target else { return Ok(Value::Bool(false)) };
    let k = vm.to_key(&key)?;
    Ok(Value::Bool(vm.has_property(t, k)?))
}

fn trap_own_keys(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let Some(idx) = list_idx(vm, &arg(args, 0)) else { return Ok(Value::Obj(vm.new_array(Vec::new()))) };
    let n = items(vm, idx).len();
    let keys: Vec<Value> = (0..n).map(|i| js(&format!("{i}"))).collect();
    Ok(Value::Obj(vm.new_array(keys)))
}

fn trap_descriptor(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (target, key) = (arg(args, 0), arg(args, 1));
    let Some(idx) = list_idx(vm, &target) else { return Ok(Value::Undefined) };
    if let Some(i) = index_of(vm, &key) {
        let list = items(vm, idx);
        if let Some(&n) = list.get(i) {
            let w = wrap(vm, n);
            let d = vm.new_object();
            vm.put(d, "value", w, ALL);
            vm.put(d, "writable", Value::Bool(false), ALL);
            vm.put(d, "enumerable", Value::Bool(true), ALL);
            vm.put(d, "configurable", Value::Bool(true), ALL);
            return Ok(Value::Obj(d));
        }
    }
    Ok(Value::Undefined)
}

fn trap_set(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (target, key, v, receiver) = (arg(args, 0), arg(args, 1), arg(args, 2), arg(args, 3));
    if index_of(vm, &key).is_some() {
        // (A list's elements can't be set: ignored.)
        return Ok(Value::Bool(true));
    }
    let Value::Obj(t) = target else { return Ok(Value::Bool(false)) };
    let k = vm.to_key(&key)?;
    // (An expando on the list: on the list's target, with the proxy as the receiver it'd be defined on it.)
    let r = if js::value::strict_equals(&receiver, &Value::Obj(t)) { receiver } else { Value::Obj(t) };
    Ok(Value::Bool(vm.set(t, k, v, &r)?))
}

fn trap_delete(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (target, key) = (arg(args, 0), arg(args, 1));
    if index_of(vm, &key).is_some() {
        return Ok(Value::Bool(false));
    }
    let Value::Obj(t) = target else { return Ok(Value::Bool(false)) };
    let k = vm.to_key(&key)?;
    Ok(Value::Bool(vm.delete_property(t, k)?))
}

/// item(i) for any list (a live one, or an array-like).
fn list_item(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let i = vm.to_uint32(&arg(args, 0))?;
    vm.get_value(this, Key::Index(i)).map(|v| if v.is_undefined() { Value::Null } else { v })
}

fn array_item(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    list_item(vm, this, args, f)
}

fn named_item(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let name = string(vm, &arg(args, 0))?;
    let len = vm.get_value(this, Key::Str(js::names::LENGTH))?;
    let len = vm.to_uint32(&len)?;
    let mut list = Vec::new();
    for i in 0..len {
        let v = vm.get_value(this, Key::Index(i))?;
        if let Some(n) = node_of(vm, &v) {
            list.push(n);
        }
    }
    let found = named(vm, &list, &name);
    Ok(wrap_opt(vm, found))
}

fn named_node(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let name = string(vm, &arg(args, 0))?.to_ascii_lowercase();
    let Value::Obj(g) = this else { return Ok(Value::Null) };
    let v = get(vm, *g, &name)?;
    Ok(if v.is_undefined() { Value::Null } else { v })
}

// ------------------------------------------------------------------------------------------- class lists

/// An element's class list (or its link's rel list).
fn token_list(vm: &mut Vm, this: &Value, which: u32) -> R {
    let n = this_element(vm, this)?;
    if let Some(&g) = host_ref(vm).cache.get(&(n, which)) {
        return Ok(Value::Obj(g));
    }
    let p = host_ref(vm).proto("DOMTokenList").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let id = n as u64 | if which == REL_LIST { 1 << 48 } else { 0 };
    let g = vm.alloc(Some(p), Class::Host(TOKENS, id));
    host(vm).cache.insert((n, which), g);
    Ok(Value::Obj(g))
}

pub fn class_list(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    token_list(vm, this, CLASS_LIST)
}

pub fn rel_list(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    token_list(vm, this, REL_LIST)
}

fn set_class_list(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    super::dom::set_attr(vm, n, "class", &v);
    Ok(Value::Undefined)
}

/// A token list's element and attribute.
fn this_tokens(vm: &mut Vm, this: &Value) -> Result<(NodeId, &'static str), Value> {
    let id = this_host(vm, this, TOKENS)?;
    Ok(((id & 0xffff_ffff_ffff) as usize, if id >> 48 == 1 { "rel" } else { "class" }))
}

fn tokens(vm: &Vm, n: NodeId, attr: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in super::dom::attr(vm, n, attr).unwrap_or_default().split_ascii_whitespace() {
        if !out.iter().any(|x| x == t) {
            out.push(String::from(t));
        }
    }
    out
}

fn write_tokens(vm: &mut Vm, n: NodeId, attr: &str, list: &[String]) {
    super::dom::set_attr(vm, n, attr, &list.join(" "));
}

fn token_arg(vm: &mut Vm, v: &Value) -> Result<String, Value> {
    let t = string(vm, v)?;
    if t.is_empty() {
        return Err(dom_exception(vm, "SyntaxError", "The token provided must not be empty."));
    }
    if t.contains(|c: char| c.is_ascii_whitespace()) {
        return Err(dom_exception(vm, "InvalidCharacterError", &format!("The token provided ('{t}') contains HTML space characters, which are not valid in tokens.")));
    }
    Ok(t)
}

fn tokens_length(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (n, a) = this_tokens(vm, this)?;
    Ok(Value::Num(tokens(vm, n, a).len() as f64))
}

fn tokens_value(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (n, a) = this_tokens(vm, this)?;
    Ok(js(&super::dom::attr(vm, n, a).unwrap_or_default()))
}

fn set_tokens_value(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, a) = this_tokens(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    super::dom::set_attr(vm, n, a, &v);
    Ok(Value::Undefined)
}

fn tokens_item(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, a) = this_tokens(vm, this)?;
    let i = vm.to_uint32(&arg(args, 0))? as usize;
    Ok(tokens(vm, n, a).get(i).map_or(Value::Null, |t| js(t)))
}

fn tokens_contains(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, a) = this_tokens(vm, this)?;
    let t = string(vm, &arg(args, 0))?;
    Ok(Value::Bool(tokens(vm, n, a).iter().any(|x| *x == t)))
}

fn tokens_add(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, a) = this_tokens(vm, this)?;
    let mut list = tokens(vm, n, a);
    for v in args {
        let t = token_arg(vm, v)?;
        if !list.contains(&t) {
            list.push(t);
        }
    }
    write_tokens(vm, n, a, &list);
    Ok(Value::Undefined)
}

fn tokens_remove(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, a) = this_tokens(vm, this)?;
    let mut list = tokens(vm, n, a);
    for v in args {
        let t = token_arg(vm, v)?;
        list.retain(|x| *x != t);
    }
    if super::dom::attr(vm, n, a).is_some() {
        write_tokens(vm, n, a, &list);
    }
    Ok(Value::Undefined)
}

fn tokens_toggle(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, a) = this_tokens(vm, this)?;
    let t = token_arg(vm, &arg(args, 0))?;
    let mut list = tokens(vm, n, a);
    let has = list.contains(&t);
    let want = if args.len() > 1 && !args[1].is_undefined() { args[1].truthy() } else { !has };
    if want && !has {
        list.push(t);
        write_tokens(vm, n, a, &list);
    } else if !want && has {
        list.retain(|x| *x != t);
        write_tokens(vm, n, a, &list);
    }
    Ok(Value::Bool(want))
}

fn tokens_replace(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, a) = this_tokens(vm, this)?;
    let old = token_arg(vm, &arg(args, 0))?;
    let new = token_arg(vm, &arg(args, 1))?;
    let mut list = tokens(vm, n, a);
    let Some(at) = list.iter().position(|x| *x == old) else { return Ok(Value::Bool(false)) };
    if list.contains(&new) {
        list.remove(at);
    } else {
        list[at] = new;
    }
    write_tokens(vm, n, a, &list);
    Ok(Value::Bool(true))
}

fn tokens_supports(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(true))
}

fn tokens_array(vm: &mut Vm, this: &Value) -> Result<Value, Value> {
    let (n, a) = this_tokens(vm, this)?;
    let items: Vec<Value> = tokens(vm, n, a).iter().map(|t| js(t)).collect();
    Ok(Value::Obj(vm.new_array(items)))
}

fn tokens_for_each(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let arr = tokens_array(vm, this)?;
    let f = arg(args, 0);
    if !vm.is_callable(&f) {
        return Err(vm.type_error("forEach's argument isn't a function"));
    }
    let items = vm.array_values(&arr);
    for (i, t) in items.into_iter().enumerate() {
        vm.call(&f, arg(args, 1), &[t, Value::Num(i as f64), this.clone()])?;
    }
    Ok(Value::Undefined)
}

fn array_method(vm: &mut Vm, this: &Value, name: &str) -> R {
    let arr = tokens_array(vm, this)?;
    let Value::Obj(a) = arr else { return Ok(Value::Undefined) };
    let m = get(vm, a, name)?;
    vm.call(&m, arr, &[])
}

fn tokens_values(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    array_method(vm, this, "values")
}
fn tokens_entries(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    array_method(vm, this, "entries")
}
fn tokens_keys(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    array_method(vm, this, "keys")
}

// ------------------------------------------------------------------------------------------- inline style

pub fn style(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if let Some(&g) = host_ref(vm).cache.get(&(n, STYLE_OBJ)) {
        return Ok(Value::Obj(g));
    }
    let p = host_ref(vm).proto("CSSStyleDeclaration").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let g = vm.alloc(Some(p), Class::Host(STYLE, n as u64));
    host(vm).cache.insert((n, STYLE_OBJ), g);
    Ok(Value::Obj(g))
}

pub fn set_style(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    super::dom::set_attr(vm, n, "style", &v);
    Ok(Value::Undefined)
}

/// A style declaration's element (a computed style's has the 1 << 48 bit: it can't be changed).
fn this_style(vm: &mut Vm, this: &Value) -> Result<(NodeId, bool), Value> {
    let id = this_host(vm, this, STYLE)?;
    Ok(((id & 0xffff_ffff_ffff) as usize, id >> 48 == 1))
}

/// A style attribute's declarations: (property, value, important).
pub fn declarations(text: &str) -> Vec<(String, String, bool)> {
    let mut out: Vec<(String, String, bool)> = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut start = 0;
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    let mut parts: Vec<&str> = Vec::new();
    for &(i, c) in &bytes {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ';') if depth <= 0 => {
                parts.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    for p in parts {
        let Some((k, v)) = p.split_once(':') else { continue };
        let k = k.trim();
        let k = if k.starts_with("--") { String::from(k) } else { k.to_ascii_lowercase() };
        let mut v = v.trim();
        let mut important = false;
        if let Some(at) = v.to_ascii_lowercase().rfind("!important") {
            v = v[..at].trim_end();
            important = true;
        }
        if k.is_empty() || v.is_empty() {
            continue;
        }
        out.retain(|(x, _, _)| *x != k);
        out.push((k, String::from(v), important));
    }
    out
}

fn serialize(list: &[(String, String, bool)]) -> String {
    let parts: Vec<String> = list.iter().map(|(k, v, i)| format!("{k}: {v}{};", if *i { " !important" } else { "" })).collect();
    parts.join(" ")
}

fn style_decls(vm: &Vm, n: NodeId) -> Vec<(String, String, bool)> {
    declarations(&super::dom::attr(vm, n, "style").unwrap_or_default())
}

/// A property's value in a style: the inline one, or (for a computed style) what the page worked out, or the
/// property's initial value.
fn value_of(vm: &mut Vm, n: NodeId, computed: bool, prop: &str) -> String {
    if let Some((_, v, _)) = style_decls(vm, n).into_iter().find(|(k, _, _)| k == prop) {
        return v;
    }
    if computed {
        return super::window::computed_value(vm, n, prop);
    }
    String::new()
}

fn set_decl(vm: &mut Vm, n: NodeId, prop: &str, value: &str, important: bool) {
    let mut list = style_decls(vm, n);
    let value = value.trim();
    if value.is_empty() {
        list.retain(|(k, _, _)| k != prop);
    } else {
        match list.iter_mut().find(|(k, _, _)| k == prop) {
            Some(d) => {
                d.1 = String::from(value);
                d.2 = important;
            }
            None => list.push((String::from(prop), String::from(value), important)),
        }
    }
    super::dom::set_attr(vm, n, "style", &serialize(&list));
}

fn prop_get(vm: &mut Vm, this: &Value, _args: &[Value], f: Gc) -> R {
    let (n, computed) = this_style(vm, this)?;
    let prop = string(vm, &vm.data(f))?;
    Ok(js(&value_of(vm, n, computed, &prop)))
}

fn prop_set(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let (n, computed) = this_style(vm, this)?;
    if computed {
        return Err(dom_exception(vm, "NoModificationAllowedError", "These styles are computed, and therefore read-only."));
    }
    let prop = string(vm, &vm.data(f))?;
    let v = arg(args, 0);
    let s = match &v {
        Value::Null => String::new(),
        // (A number for a length is in pixels, as old pages write them... only where it's 0.)
        _ => string(vm, &v)?,
    };
    set_decl(vm, n, &prop, &s, false);
    Ok(Value::Undefined)
}

fn float_get(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (n, computed) = this_style(vm, this)?;
    Ok(js(&value_of(vm, n, computed, "float")))
}

fn float_set(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, _) = this_style(vm, this)?;
    let s = string(vm, &arg(args, 0))?;
    set_decl(vm, n, "float", &s, false);
    Ok(Value::Undefined)
}

fn css_text(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (n, _) = this_style(vm, this)?;
    Ok(js(&serialize(&style_decls(vm, n))))
}

fn set_css_text(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, _) = this_style(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    let list = declarations(&v);
    super::dom::set_attr(vm, n, "style", &serialize(&list));
    Ok(Value::Undefined)
}

fn style_length(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (n, _) = this_style(vm, this)?;
    Ok(Value::Num(style_decls(vm, n).len() as f64))
}

fn style_item(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, _) = this_style(vm, this)?;
    let i = vm.to_uint32(&arg(args, 0))? as usize;
    Ok(js(style_decls(vm, n).get(i).map_or("", |d| d.0.as_str())))
}

fn get_property_value(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, computed) = this_style(vm, this)?;
    let p = string(vm, &arg(args, 0))?;
    let p = if p.starts_with("--") { p } else { p.to_ascii_lowercase() };
    Ok(js(&value_of(vm, n, computed, &p)))
}

fn get_property_priority(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, _) = this_style(vm, this)?;
    let p = string(vm, &arg(args, 0))?.to_ascii_lowercase();
    Ok(js(if style_decls(vm, n).iter().any(|(k, _, i)| *k == p && *i) { "important" } else { "" }))
}

fn set_property(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, computed) = this_style(vm, this)?;
    if computed {
        return Err(dom_exception(vm, "NoModificationAllowedError", "These styles are computed, and therefore read-only."));
    }
    let p = string(vm, &arg(args, 0))?;
    let p = if p.starts_with("--") { p } else { p.to_ascii_lowercase() };
    let v = arg(args, 1);
    let v = if v.is_nullish() { String::new() } else { string(vm, &v)? };
    let important = string(vm, &arg(args, 2)).unwrap_or_default().eq_ignore_ascii_case("important");
    set_decl(vm, n, &p, &v, important);
    Ok(Value::Undefined)
}

fn remove_property(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, _) = this_style(vm, this)?;
    let p = string(vm, &arg(args, 0))?;
    let p = if p.starts_with("--") { p } else { p.to_ascii_lowercase() };
    let old = value_of(vm, n, false, &p);
    set_decl(vm, n, &p, "", false);
    Ok(js(&old))
}

/// A computed style for an element (getComputedStyle's).
pub fn computed_style(vm: &mut Vm, n: NodeId) -> Value {
    let p = host_ref(vm).proto("CSSStyleDeclaration").unwrap_or_else(|| vm.proto(I::ObjectProto));
    Value::Obj(vm.alloc(Some(p), Class::Host(STYLE, n as u64 | 1 << 48)))
}

/// "background-color" as "backgroundColor".
fn camel_case(name: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in name.trim_start_matches('-').chars() {
        if c == '-' {
            up = true;
        } else if up {
            out.push(c.to_ascii_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    if name.starts_with("-webkit-") {
        // (style.webkitTransform)
        return out;
    }
    out
}

/// Whether a CSS property is one this browser knows.
pub fn known_property(p: &str) -> bool {
    CSS_PROPERTIES.contains(&p)
}

/// The CSS properties scripts set through style's named properties.
const CSS_PROPERTIES: &[&str] = &[
    "align-content", "align-items", "align-self", "all", "animation", "animation-delay", "animation-direction",
    "animation-duration", "animation-fill-mode", "animation-iteration-count", "animation-name", "animation-play-state",
    "animation-timing-function", "appearance", "aspect-ratio", "backdrop-filter", "backface-visibility", "background",
    "background-attachment", "background-blend-mode", "background-clip", "background-color", "background-image",
    "background-origin", "background-position", "background-position-x", "background-position-y", "background-repeat",
    "background-size", "block-size", "border", "border-block", "border-block-end", "border-block-start", "border-bottom",
    "border-bottom-color", "border-bottom-left-radius", "border-bottom-right-radius", "border-bottom-style",
    "border-bottom-width", "border-collapse", "border-color", "border-image", "border-inline", "border-inline-end",
    "border-inline-start", "border-left", "border-left-color", "border-left-style", "border-left-width", "border-radius",
    "border-right", "border-right-color", "border-right-style", "border-right-width", "border-spacing", "border-style",
    "border-top", "border-top-color", "border-top-left-radius", "border-top-right-radius", "border-top-style",
    "border-top-width", "border-width", "bottom", "box-shadow", "box-sizing", "break-after", "break-before",
    "break-inside", "caption-side", "caret-color", "clear", "clip", "clip-path", "color", "color-scheme", "column-count",
    "column-gap", "column-rule", "column-span", "column-width", "columns", "contain", "container", "content",
    "content-visibility", "counter-increment", "counter-reset", "cursor", "direction", "display", "empty-cells", "fill",
    "filter", "flex", "flex-basis", "flex-direction", "flex-flow", "flex-grow", "flex-shrink", "flex-wrap", "float",
    "font", "font-family", "font-feature-settings", "font-kerning", "font-size", "font-stretch", "font-style",
    "font-variant", "font-variant-numeric", "font-weight", "gap", "grid", "grid-area", "grid-auto-columns",
    "grid-auto-flow", "grid-auto-rows", "grid-column", "grid-column-end", "grid-column-gap", "grid-column-start",
    "grid-gap", "grid-row", "grid-row-end", "grid-row-gap", "grid-row-start", "grid-template", "grid-template-areas",
    "grid-template-columns", "grid-template-rows", "height", "hyphens", "image-rendering", "inline-size", "inset",
    "inset-block", "inset-inline", "isolation", "justify-content", "justify-items", "justify-self", "left",
    "letter-spacing", "line-break", "line-clamp", "line-height", "list-style", "list-style-image",
    "list-style-position", "list-style-type", "margin", "margin-block", "margin-block-end", "margin-block-start",
    "margin-bottom", "margin-inline", "margin-inline-end", "margin-inline-start", "margin-left", "margin-right",
    "margin-top", "mask", "mask-image", "max-block-size", "max-height", "max-inline-size", "max-width",
    "min-block-size", "min-height", "min-inline-size", "min-width", "mix-blend-mode", "object-fit", "object-position",
    "offset", "opacity", "order", "orphans", "outline", "outline-color", "outline-offset", "outline-style",
    "outline-width", "overflow", "overflow-anchor", "overflow-wrap", "overflow-x", "overflow-y", "overscroll-behavior",
    "padding", "padding-block", "padding-block-end", "padding-block-start", "padding-bottom", "padding-inline",
    "padding-inline-end", "padding-inline-start", "padding-left", "padding-right", "padding-top", "page-break-after",
    "page-break-before", "page-break-inside", "perspective", "perspective-origin", "place-content", "place-items",
    "place-self", "pointer-events", "position", "quotes", "resize", "right", "rotate", "row-gap", "scale",
    "scroll-behavior", "scroll-margin", "scroll-padding", "scroll-snap-align", "scroll-snap-type", "scrollbar-color",
    "scrollbar-gutter", "scrollbar-width", "shape-outside", "stroke", "stroke-width", "tab-size", "table-layout",
    "text-align", "text-align-last", "text-decoration", "text-decoration-color", "text-decoration-line",
    "text-decoration-style", "text-decoration-thickness", "text-indent", "text-overflow", "text-rendering",
    "text-shadow", "text-transform", "text-underline-offset", "text-wrap", "top", "touch-action", "transform",
    "transform-origin", "transform-style", "transition", "transition-delay", "transition-duration",
    "transition-property", "transition-timing-function", "translate", "unicode-bidi", "user-select", "vertical-align",
    "visibility", "white-space", "widows", "width", "will-change", "word-break", "word-spacing", "word-wrap",
    "writing-mode", "z-index", "zoom", "-webkit-transform", "-webkit-transition", "-webkit-animation",
    "-webkit-user-select", "-webkit-appearance", "-webkit-line-clamp", "-webkit-box-orient", "-webkit-font-smoothing",
    "-webkit-tap-highlight-color", "-webkit-overflow-scrolling", "-webkit-backdrop-filter", "-webkit-mask-image",
    "-webkit-text-size-adjust", "-webkit-box-flex", "-webkit-flex", "-webkit-transform-origin",
    "-webkit-transition-duration", "-webkit-filter",
];

// -------------------------------------------------------------------------------------------- dataset

pub fn dataset(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if let Some(&g) = host_ref(vm).cache.get(&(n, DATASET_OBJ)) {
        return Ok(Value::Obj(g));
    }
    let p = host_ref(vm).proto("DOMStringMap").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let target = vm.alloc(Some(p), Class::Host(DATASET, n as u64));
    let handler = host_ref(vm).proto("(dataset handler)").expect("made");
    let px = vm.alloc(None, Class::Proxy(Some((target, handler)), 0));
    host(vm).cache.insert((n, DATASET_OBJ), px);
    Ok(Value::Obj(px))
}

fn data_node(vm: &Vm, target: &Value) -> Option<NodeId> {
    match target {
        Value::Obj(g) => match vm.heap.get(*g).class {
            Class::Host(DATASET, n) => Some(n as usize),
            _ => None,
        },
        _ => None,
    }
}

/// "fooBar" as "data-foo-bar".
fn data_attr(name: &str) -> String {
    let mut out = String::from("data-");
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// "data-foo-bar" as "fooBar".
fn data_name(attr: &str) -> Option<String> {
    let rest = attr.strip_prefix("data-")?;
    let mut out = String::new();
    let mut up = false;
    for c in rest.chars() {
        if c == '-' {
            up = true;
        } else if up && c.is_ascii_lowercase() {
            out.push(c.to_ascii_uppercase());
            up = false;
        } else {
            if up {
                out.push('-');
            }
            out.push(c);
            up = false;
        }
    }
    Some(out)
}

fn data_get(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (target, key, receiver) = (arg(args, 0), arg(args, 1), arg(args, 2));
    let Some(n) = data_node(vm, &target) else { return Ok(Value::Undefined) };
    if let Value::Str(s) = &key {
        let a = data_attr(&s.to_rust());
        if let Some(v) = super::dom::attr(vm, n, &a) {
            return Ok(js(&v));
        }
    }
    let Value::Obj(t) = target else { return Ok(Value::Undefined) };
    let k = vm.to_key(&key)?;
    vm.get(t, k, &receiver)
}

fn data_set(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (target, key, v) = (arg(args, 0), arg(args, 1), arg(args, 2));
    let Some(n) = data_node(vm, &target) else { return Ok(Value::Bool(false)) };
    if let Value::Str(s) = &key {
        let name = s.to_rust();
        if name.contains(|c: char| c == '-') && name.split('-').skip(1).any(|p| p.starts_with(|c: char| c.is_ascii_lowercase())) {
            return Err(dom_exception(vm, "SyntaxError", &format!("'{name}' is not a valid property name.")));
        }
        let v = string(vm, &v)?;
        super::dom::set_attr(vm, n, &data_attr(&name), &v);
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(false))
}

fn data_has(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (target, key) = (arg(args, 0), arg(args, 1));
    let Some(n) = data_node(vm, &target) else { return Ok(Value::Bool(false)) };
    if let Value::Str(s) = &key
        && super::dom::attr(vm, n, &data_attr(&s.to_rust())).is_some()
    {
        return Ok(Value::Bool(true));
    }
    let Value::Obj(t) = target else { return Ok(Value::Bool(false)) };
    let k = vm.to_key(&key)?;
    Ok(Value::Bool(vm.has_property(t, k)?))
}

fn data_delete(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (target, key) = (arg(args, 0), arg(args, 1));
    let Some(n) = data_node(vm, &target) else { return Ok(Value::Bool(false)) };
    if let Value::Str(s) = &key {
        super::dom::remove_attr(vm, n, &data_attr(&s.to_rust()));
    }
    Ok(Value::Bool(true))
}

fn data_own_keys(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let Some(n) = data_node(vm, &arg(args, 0)) else { return Ok(Value::Obj(vm.new_array(Vec::new()))) };
    let names: Vec<Value> = host_ref(vm).doc.element(n).map(|e| e.attrs.iter().filter_map(|(k, _)| data_name(k)).map(|s| js(&s)).collect()).unwrap_or_default();
    Ok(Value::Obj(vm.new_array(names)))
}

fn data_descriptor(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let (target, key) = (arg(args, 0), arg(args, 1));
    let Some(n) = data_node(vm, &target) else { return Ok(Value::Undefined) };
    if let Value::Str(s) = &key
        && let Some(v) = super::dom::attr(vm, n, &data_attr(&s.to_rust()))
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
