//! CSS counters as a page's boxes are made, in document order: counter-reset, counter-increment and counter-set, and
//! counter() and counters() reading them, scoped as Chrome scopes them; list items' numbers, as Chrome numbers them
//! (by their list: its start, reversed, and the items' value attributes and own list-item counters); and quotes'
//! depth, for open-quote and close-quote.
//!
//! A counter an element makes is in scope for what's inside it and, unless it's inside a counter of the same name
//! that one of the element's ancestors made, for the element's following siblings and what's in them too (as Chrome
//! has it: `<div style="counter-reset: c 1"><p style="counter-reset: c 5">` gives the <p>'s next sibling "1", not
//! "1.5").

use crate::dom::{Document, NodeId};
use crate::style::{CounterChange, Display, Style, Styles};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
struct Instance {
    name: String,
    value: i32,
    /// The element (or pseudo-element) that made it, and its parent.
    origin: u32,
    origin_parent: u32,
    /// It's in scope for its element's insides alone.
    inside_only: bool,
}

/// What an element is to HTML's lists.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ListRole {
    /// Neither.
    None,
    /// An <ol>, <ul> or <menu>: it begins the list-item counter again (at this, the value before its first item's).
    List(i32),
    /// An <li> shown as a list item: it counts on the list-item counter (by this).
    Item(i32),
}

pub struct Counters {
    /// The counters in scope, outermost first.
    live: Vec<Instance>,
    /// The elements being walked through, outermost first.
    path: Vec<u32>,
    next: u32,
    /// How deep in quotes the page is.
    pub quote_depth: usize,
    /// Each list's last item's number.
    last: BTreeMap<NodeId, i32>,
    /// Each reversed list's number of items.
    counts: BTreeMap<NodeId, i32>,
}

impl Default for Counters {
    fn default() -> Self {
        Counters::new()
    }
}

impl Counters {
    pub fn new() -> Counters {
        Counters { live: Vec::new(), path: alloc::vec![0], next: 1, quote_depth: 0, last: BTreeMap::new(), counts: BTreeMap::new() }
    }

    /// An element (or ::before or ::after) begins: its counter-reset, then counter-increment, then counter-set, with
    /// HTML's lists' own changes to the list-item counter. Returns its place, for `leave` once what's inside it is
    /// done (counters' uses, by counter() and counters(), in between).
    pub fn enter(&mut self, style: &Style, role: ListRole) -> u32 {
        let me = self.next;
        self.next += 1;
        let names = |c: &Option<alloc::sync::Arc<Vec<CounterChange>>>| c.as_ref().is_some_and(|v| v.iter().any(|x| x.name == "list-item"));
        let reset_names_item = names(&style.counter_reset);
        if let Some(r) = &style.counter_reset {
            for c in r.iter() {
                self.instantiate(me, &c.name, c.value);
            }
        }
        if let ListRole::List(v) = role
            && !reset_names_item
        {
            self.instantiate(me, "list-item", v);
        }
        if let Some(r) = &style.counter_increment {
            for c in r.iter() {
                let k = self.innermost_or_new(me, &c.name);
                self.live[k].value = self.live[k].value.saturating_add(c.value);
            }
        }
        // (An item counts on the list-item counter unless its own counter-reset or counter-increment names it.)
        if let ListRole::Item(by) = role
            && !reset_names_item
            && !names(&style.counter_increment)
        {
            let k = self.innermost_or_new(me, "list-item");
            self.live[k].value = self.live[k].value.saturating_add(by);
        }
        if let Some(r) = &style.counter_set {
            for c in r.iter() {
                let k = self.innermost_or_new(me, &c.name);
                self.live[k].value = c.value;
            }
        }
        self.path.push(me);
        me
    }

    /// An element (or pseudo-element) ends, what's inside it done: the counters its children made, and those it made
    /// for its insides alone, go out of scope.
    pub fn leave(&mut self, me: u32) {
        self.live.retain(|i| i.origin_parent != me && !(i.inside_only && i.origin == me));
        if self.path.last() == Some(&me) {
            self.path.pop();
        }
    }

    /// A counter's value where the page is (its innermost; one made, at 0, when there's none).
    pub fn value(&mut self, name: &str) -> i32 {
        let me = self.path.last().copied().unwrap_or(0);
        let k = self.innermost_or_new(me, name);
        self.live[k].value
    }

    /// All of a counter's values where the page is, outermost first (counters()).
    pub fn values(&mut self, name: &str) -> Vec<i32> {
        if !self.live.iter().any(|i| i.name == name) {
            self.value(name);
        }
        self.live.iter().filter(|i| i.name == name).map(|i| i.value).collect()
    }

    fn innermost_or_new(&mut self, me: u32, name: &str) -> usize {
        match self.live.iter().rposition(|i| i.name == name) {
            Some(k) => k,
            None => self.instantiate(me, name, 0),
        }
    }

    /// A new counter, made by element `me`: in place of the innermost of its name if that one was made by this element
    /// or a sibling before it; inside the rest.
    fn instantiate(&mut self, me: u32, name: &str, value: i32) -> usize {
        let parent = self.path.last().copied().unwrap_or(0);
        if let Some(k) = self.live.iter().rposition(|i| i.name == name)
            && (self.live[k].origin == me || self.live[k].origin_parent == parent)
        {
            self.live.remove(k);
        }
        // (Inside a counter of its name an ancestor made, it's in scope for this element's insides alone.)
        let inside_only = self.live.iter().rposition(|i| i.name == name).is_some_and(|k| self.path.contains(&self.live[k].origin));
        self.live.push(Instance { name: String::from(name), value, origin: me, origin_parent: parent, inside_only });
        self.live.len() - 1
    }

    /// A list item's number, as Chrome numbers it: its counter-set's of list-item, else its value attribute's (an
    /// <li>'s), else the number of the item before it in its list (its nearest <ol>, <ul> or <menu>, else its parent)
    /// plus its step (its counter-increment's of list-item, all added up, else 1, or -1 in a reversed <ol>); the first
    /// item's from the list's start (an <ol>'s start attribute; a reversed one's, else its number of items).
    /// `ancestors` are the item's (the flat tree's: a ::before's or ::after's element last); `node` None for those.
    pub fn number(&mut self, doc: &Document, styles: &Styles, ancestors: &[NodeId], node: Option<NodeId>, style: &Style) -> i32 {
        let owner = list_owner(doc, ancestors);
        let ol = owner.filter(|&o| doc.name(o) == "ol");
        let reversed = ol.is_some_and(|o| doc.element(o).is_some_and(|e| e.has_attr("reversed")));
        let set = style.counter_set.as_ref().and_then(|v| v.iter().rev().find(|x| x.name == "list-item").map(|x| x.value));
        let increments: Vec<i32> = style.counter_increment.as_ref().map(|v| v.iter().filter(|x| x.name == "list-item").map(|x| x.value).collect()).unwrap_or_default();
        let value_attr = node.filter(|&n| doc.name(n) == "li").and_then(|n| doc.element(n)).and_then(|e| e.attr("value")).and_then(html_integer);
        let value = if let Some(v) = set {
            v
        } else if let Some(v) = value_attr {
            v
        } else {
            let step = if increments.is_empty() { if reversed { -1 } else { 1 } } else { increments.iter().fold(0i32, |a, &b| a.saturating_add(b)) };
            let key = owner.unwrap_or(usize::MAX);
            let before = match self.last.get(&key) {
                Some(&v) => v,
                None => match ol {
                    Some(o) => {
                        let start = doc.element(o).and_then(|e| e.attr("start")).and_then(html_integer);
                        if reversed {
                            start.unwrap_or_else(|| self.count(doc, styles, o)).saturating_add(1)
                        } else {
                            start.unwrap_or(1).saturating_sub(1)
                        }
                    }
                    None => 0,
                },
            };
            before.saturating_add(step)
        };
        self.last.insert(owner.unwrap_or(usize::MAX), value);
        value
    }

    /// A reversed list's number of items (the elements shown as list items whose list it is).
    fn count(&mut self, doc: &Document, styles: &Styles, list: NodeId) -> i32 {
        if let Some(&n) = self.counts.get(&list) {
            return n;
        }
        let mut n = 0;
        let mut stack: Vec<NodeId> = doc.children(list).iter().rev().copied().collect();
        while let Some(k) = stack.pop() {
            let Some(s) = styles.of.get(k).cloned().flatten() else { continue };
            if s.display == Display::None {
                continue;
            }
            if s.display == Display::ListItem {
                n += 1;
            }
            if !is_list(doc, k) {
                stack.extend(doc.children(k).iter().rev().copied());
            }
        }
        self.counts.insert(list, n);
        n
    }

    /// The list-item counter's role for an element (`ancestors` its own): an <ol>, <ul> or <menu> begins it again; an
    /// <li> shown as a list item counts on it (down, in a reversed <ol>).
    pub fn role(doc: &Document, ancestors: &[NodeId], node: Option<NodeId>, style: &Style) -> ListRole {
        let Some(node) = node else { return ListRole::None };
        match doc.name(node) {
            "ol" | "ul" | "menu" if is_list(doc, node) => {
                if doc.name(node) != "ol" {
                    return ListRole::List(0);
                }
                let e = doc.element(node);
                let start = e.and_then(|e| e.attr("start")).and_then(html_integer);
                // (A reversed <ol>'s counter, as Chrome keeps it, begins one above its start, else at one.)
                if e.is_some_and(|e| e.has_attr("reversed")) {
                    ListRole::List(start.unwrap_or(0).saturating_add(1))
                } else {
                    ListRole::List(start.unwrap_or(1).saturating_sub(1))
                }
            }
            "li" if style.display == Display::ListItem => {
                let reversed = list_owner(doc, ancestors).is_some_and(|o| doc.name(o) == "ol" && doc.element(o).is_some_and(|e| e.has_attr("reversed")));
                ListRole::Item(if reversed { -1 } else { 1 })
            }
            _ => ListRole::None,
        }
    }
}

/// Whether an element is one of HTML's lists.
fn is_list(doc: &Document, node: NodeId) -> bool {
    matches!(doc.name(node), "ol" | "ul" | "menu") && doc.element(node).is_some()
}

/// A list item's list, from its ancestors (its parent last): the nearest <ol>, <ul> or <menu>, else its parent.
fn list_owner(doc: &Document, ancestors: &[NodeId]) -> Option<NodeId> {
    ancestors.iter().rev().copied().find(|&a| is_list(doc, a)).or(ancestors.last().copied())
}

/// HTML's rules for parsing an integer: spaces, a sign, digits (what follows them ignored).
pub fn html_integer(s: &str) -> Option<i32> {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0C']);
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let end = digits.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let mut v: i64 = 0;
    for b in digits[..end].bytes() {
        v = (v * 10 + (b - b'0') as i64).min(i64::from(i32::MAX) + 1);
    }
    let v = if negative { -v } else { v };
    Some(v.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

/// The quotes a language uses, outer then inner (Chrome's, for quotes: auto).
pub fn language_quotes(lang: &str) -> [(&'static str, &'static str); 2] {
    let lang = lang.to_ascii_lowercase();
    let base = lang.split(['-', '_']).next().unwrap_or("");
    match (base, lang.as_str()) {
        (_, "fr-ch") => [("«", "»"), ("‹", "›")],
        ("de" | "cs", _) => [("„", "“"), ("‚", "‘")],
        ("fr", _) => [("«", "»"), ("«", "»")],
        ("ja", _) => [("「", "」"), ("『", "』")],
        ("ru" | "uk", _) => [("«", "»"), ("„", "“")],
        ("it" | "el", _) => [("«", "»"), ("“", "”")],
        ("nl", _) => [("‘", "’"), ("‘", "’")],
        ("sv" | "fi" | "he", _) => [("”", "”"), ("’", "’")],
        ("pl", _) => [("„", "”"), ("«", "»")],
        ("ar", _) => [("”", "“"), ("’", "‘")],
        ("nb", _) => [("«", "»"), ("‘", "’")],
        ("hu", _) => [("„", "”"), ("»", "«")],
        _ => [("“", "”"), ("‘", "’")],
    }
}
