//! HTML read into a tree as browsers read it (the WHATWG's parsing rules, the parts pages lean on): tags, their
//! attributes (quoted or not), words with their entities read (&amp;, &#8217;...), comments and the doctype dropped,
//! the insides of <script> and <style> taken whole (a "<" there starts no tag), <title> and <textarea> the same but
//! with entities read. Pages leave much out and browsers fill it in, so this does too: <html>, <head> and <body>
//! where they're missing (what belongs in the head going there), a <p> closed by a block starting, an <li> by the
//! next <li>, a cell by the next cell or row, a heading by another; an end tag closes what's open inside it, and one
//! for nothing open is dropped. Scripts don't run here, so <noscript>'s insides are the page's (as in a browser
//! with scripting off). Elements nest at most `DEEPEST` deep: deeper ones go in the deepest (nothing here recurses
//! that deep, and neither does what lays the page out).

use crate::dom::{Data, Document, Element, NodeId};
use alloc::string::String;
use alloc::vec::Vec;

/// How deep elements nest at most.
pub const DEEPEST: usize = 120;

/// Elements with no insides and no end tag.
const VOID: [&str; 15] =
    ["area", "base", "br", "col", "embed", "hr", "img", "input", "keygen", "link", "meta", "param", "source", "track", "wbr"];

/// Elements whose insides are text up to their end tag (without entities), and those with entities read.
const RAW: [&str; 6] = ["script", "style", "xmp", "iframe", "noembed", "noframes"];
const RCDATA: [&str; 2] = ["title", "textarea"];

/// Elements that belong in the head when they come before the body has started.
const HEAD: [&str; 8] = ["base", "basefont", "bgsound", "link", "meta", "title", "style", "script"];

/// Elements whose start closes an open <p> (the HTML standard's list).
const CLOSES_P: [&str; 39] = [
    "address", "article", "aside", "blockquote", "center", "details", "dialog", "dir", "div", "dl", "fieldset",
    "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hgroup", "hr", "main",
    "menu", "nav", "ol", "p", "pre", "section", "summary", "table", "ul", "li", "dd", "dt", "listing", "search",
];

/// What a search for an open element stops at (the standard's "scope").
const SCOPE: [&str; 10] = ["applet", "caption", "html", "table", "td", "th", "marquee", "object", "template", "button"];

fn heading(name: &str) -> bool {
    name.len() == 2 && name.as_bytes()[0] == b'h' && (b'1'..=b'6').contains(&name.as_bytes()[1])
}

/// A page's HTML, read.
pub fn parse(html: &str) -> Document {
    let mut b = Builder::new();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let end = bytes[i..].iter().position(|&c| c == b'<').map_or(bytes.len(), |n| i + n);
            b.text(&web::page::entities(&html[i..end]));
            i = end;
            continue;
        }
        let rest = &html[i..];
        if rest.starts_with("<!--") {
            i = rest[4..].find("-->").map_or(bytes.len(), |n| i + 4 + n + 3);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            let end = rest.find('>').map_or(rest.len(), |n| n + 1);
            // A doctype: standards mode, but for the old ones (HTML 2 and 3.2, and 4's transitional without its
            // address) that put browsers in quirks mode.
            let tag = rest[..end].to_ascii_lowercase();
            if tag.starts_with("<!doctype") && tag.contains("html") {
                let old = tag.contains("html 2") || tag.contains("html 3") || tag.contains("html level")
                    || ((tag.contains("transitional") || tag.contains("frameset")) && !tag.contains("http"));
                b.quirks(!old);
            }
            i += end;
            continue;
        }
        let closing = bytes.get(i + 1) == Some(&b'/');
        let name_at = i + 1 + closing as usize;
        if !bytes.get(name_at).is_some_and(|c| c.is_ascii_alphabetic()) {
            if closing {
                // "</" not followed by a name: a comment of sorts, up to ">".
                i = rest.find('>').map_or(bytes.len(), |n| i + n + 1);
            } else {
                b.text("<");
                i += 1;
            }
            continue;
        }
        let name_len = bytes[name_at..].iter().take_while(|c| !(c.is_ascii_whitespace() || **c == b'/' || **c == b'>')).count();
        let name = html[name_at..name_at + name_len].to_ascii_lowercase();
        let (attrs, self_closing, end) = attributes(html, name_at + name_len);
        i = end;
        if closing {
            b.end(&name);
            continue;
        }
        let raw = RAW.contains(&name.as_str());
        let rc = RCDATA.contains(&name.as_str());
        b.start(&name, attrs, self_closing);
        if (raw || rc) && !self_closing {
            // Everything up to its end tag is its text.
            let close = alloc::format!("</{name}");
            let stop = find_ci(&html[i..], &close).map_or(bytes.len(), |n| i + n);
            let inside = &html[i..stop];
            if !inside.is_empty() {
                if rc {
                    b.text(&web::page::entities(inside));
                } else {
                    b.text(inside);
                }
            }
            b.end(&name);
            i = if stop < bytes.len() { tag_end(bytes, stop + close.len()) } else { stop };
        } else if name == "plaintext" {
            b.text(&html[i..]);
            break;
        }
    }
    b.finish()
}

/// Where `needle` (ASCII, lower case) first is in `hay`, whatever the case there.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h[i..i + n.len()].iter().zip(n).all(|(a, b)| a.to_ascii_lowercase() == *b))
}

/// Just past the ">" ending the tag that goes on from `from` (a ">" inside quotes doesn't end it).
fn tag_end(b: &[u8], from: usize) -> usize {
    let mut quote = 0u8;
    let mut i = from;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// A tag's attributes, from just after its name: (name, value) with names in lower case and values' entities read,
/// whether it ended "/>", and where the tag ends (just past its ">").
fn attributes(html: &str, from: usize) -> (Vec<(String, String)>, bool, usize) {
    let b = html.as_bytes();
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut i = from;
    let mut self_closing = false;
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            self_closing = b[i] == b'/';
            i += 1;
        }
        if i >= b.len() {
            return (attrs, false, b.len());
        }
        if b[i] == b'>' {
            return (attrs, self_closing, i + 1);
        }
        self_closing = false;
        let start = i;
        // (A name can start with "=", oddly, but nothing else ends it but these.)
        i += 1;
        while i < b.len() && !(b[i].is_ascii_whitespace() || b[i] == b'/' || b[i] == b'>' || b[i] == b'=') {
            i += 1;
        }
        let name = html[start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let vs = i + 1;
                let ve = b[vs..].iter().position(|&c| c == q).map_or(b.len(), |n| vs + n);
                value = web::page::entities(&html[vs..ve]);
                i = (ve + 1).min(b.len());
            } else {
                let vs = i;
                while i < b.len() && !(b[i].is_ascii_whitespace() || b[i] == b'>') {
                    i += 1;
                }
                value = web::page::entities(&html[vs..i]);
            }
        }
        if !name.is_empty() && !attrs.iter().any(|(k, _)| *k == name) {
            attrs.push((name, value));
        }
    }
}

struct Builder {
    doc: Document,
    /// The elements open, the outermost first.
    stack: Vec<NodeId>,
    html: NodeId,
    head: Option<NodeId>,
    body: Option<NodeId>,
    /// Whether the head is over (something for the body came).
    after_head: bool,
}

impl Builder {
    /// The doctype said standards mode (true) or quirks mode.
    fn quirks(&mut self, standards: bool) {
        self.doc.quirks = !standards;
    }

    fn new() -> Builder {
        let mut doc = Document::new();
        let html = doc.append(0, Data::Element(Element { name: String::from("html"), attrs: Vec::new() }));
        Builder { doc, stack: alloc::vec![html], html, head: None, body: None, after_head: false }
    }

    fn current(&self) -> NodeId {
        self.stack.last().copied().unwrap_or(self.html)
    }

    fn head(&mut self) -> NodeId {
        if let Some(h) = self.head {
            return h;
        }
        let h = self.doc.append(self.html, Data::Element(Element { name: String::from("head"), attrs: Vec::new() }));
        // (Before the body, if that has started already.)
        if let Some(b) = self.body
            && let Some(kids) = self.doc.nodes.get_mut(self.html).map(|n| &mut n.children)
        {
            kids.retain(|&k| k != h);
            let at = kids.iter().position(|&k| k == b).unwrap_or(kids.len());
            kids.insert(at, h);
        }
        self.head = Some(h);
        h
    }

    /// The body, started if it hasn't been (the head over then).
    fn body(&mut self) -> NodeId {
        if let Some(b) = self.body {
            return b;
        }
        self.head();
        let b = self.doc.append(self.html, Data::Element(Element { name: String::from("body"), attrs: Vec::new() }));
        self.body = Some(b);
        self.after_head = true;
        self.stack = alloc::vec![self.html, b];
        b
    }

    /// Whether what's open is only <html> (and maybe <head>): nothing of the body yet.
    fn in_head_part(&self) -> bool {
        self.body.is_none()
    }

    fn text(&mut self, t: &str) {
        if t.is_empty() {
            return;
        }
        if self.in_head_part() && self.current() == self.html {
            // Spaces before the body are nothing; anything else starts it. (The words of the head's elements, a
            // <title>'s or a <style>'s, go in them.)
            let rest = t.trim_start_matches(|c: char| c.is_ascii_whitespace());
            if rest.is_empty() {
                return;
            }
            self.body();
            self.append_text(rest);
            return;
        }
        self.append_text(t);
    }

    fn append_text(&mut self, t: &str) {
        let parent = self.current();
        // (Joined to words just before it.)
        if let Some(&last) = self.doc.children(parent).last()
            && let Some(crate::dom::Node { data: Data::Text(s), .. }) = self.doc.nodes.get_mut(last)
        {
            s.push_str(t);
            return;
        }
        self.doc.append(parent, Data::Text(String::from(t)));
    }

    /// Whether an element named `name` is open within the current scope (not past `SCOPE`'s, nor `also`'s).
    fn open_in_scope(&self, name: &str, also: &[&str]) -> Option<usize> {
        for (i, &id) in self.stack.iter().enumerate().rev() {
            let n = self.doc.name(id);
            if n == name {
                return Some(i);
            }
            if SCOPE.contains(&n) || also.contains(&n) {
                return None;
            }
        }
        None
    }

    fn pop_to(&mut self, at: usize) {
        // (Never <html> itself, nor the body once it's started.)
        let keep = if self.body.is_some() { 2 } else { 1 };
        self.stack.truncate(at.max(keep));
    }

    fn close_p(&mut self) {
        if let Some(at) = self.open_in_scope("p", &[]) {
            self.pop_to(at);
        }
    }

    fn start(&mut self, name: &str, attrs: Vec<(String, String)>, self_closing: bool) {
        match name {
            "html" => {
                merge(&mut self.doc, self.html, attrs);
                return;
            }
            "head" => {
                if self.body.is_none() && !self.after_head {
                    let h = self.head();
                    merge(&mut self.doc, h, attrs);
                }
                return;
            }
            "body" => {
                let b = self.body();
                merge(&mut self.doc, b, attrs);
                return;
            }
            _ => {}
        }
        let foreign = self.stack.iter().any(|&id| matches!(self.doc.name(id), "svg" | "math"));
        let parent = if self.body.is_none() && (HEAD.contains(&name) || (name == "noscript" && !self.after_head)) {
            self.head()
        } else {
            self.body();
            if !foreign {
                self.close_for(name);
            }
            self.current()
        };
        let id = self.doc.append(parent, Data::Element(Element { name: String::from(name), attrs }));
        let leaf = VOID.contains(&name) || (self_closing && (foreign || name == "svg" || name == "math"));
        // (Too deep, its insides go beside it.)
        if !leaf && self.stack.len() < DEEPEST {
            self.stack.push(id);
        }
    }

    /// What a start tag named `name` closes first.
    fn close_for(&mut self, name: &str) {
        if CLOSES_P.contains(&name) {
            self.close_p();
        }
        match name {
            "li" => {
                if let Some(at) = self.open_in_scope("li", &["ul", "ol", "menu"]) {
                    self.pop_to(at);
                }
            }
            "dt" | "dd" => {
                let at = self.open_in_scope("dt", &["dl"]).max(self.open_in_scope("dd", &["dl"]));
                if let Some(at) = at {
                    self.pop_to(at);
                }
            }
            n if heading(n) => {
                if heading(self.doc.name(self.current())) {
                    let last = self.stack.len() - 1;
                    self.pop_to(last);
                }
            }
            "option" => {
                if self.doc.name(self.current()) == "option" {
                    let last = self.stack.len() - 1;
                    self.pop_to(last);
                }
            }
            "optgroup" => {
                for _ in 0..2 {
                    if matches!(self.doc.name(self.current()), "option" | "optgroup") {
                        let last = self.stack.len() - 1;
                        self.pop_to(last);
                    }
                }
            }
            "tr" => {
                for open in ["td", "th", "tr"] {
                    if let Some(at) = self.open_in_scope(open, &["tbody", "thead", "tfoot"]) {
                        self.pop_to(at);
                    }
                }
            }
            "td" | "th" => {
                for open in ["td", "th"] {
                    if let Some(at) = self.open_in_scope(open, &["tr"]) {
                        self.pop_to(at);
                    }
                }
            }
            "thead" | "tbody" | "tfoot" => {
                for open in ["td", "th", "tr", "thead", "tbody", "tfoot"] {
                    if let Some(at) = self.open_in_scope(open, &[]) {
                        self.pop_to(at);
                    }
                }
            }
            "a" => {
                if let Some(at) = self.open_in_scope("a", &[]) {
                    self.pop_to(at);
                }
            }
            "button" => {
                if let Some(at) = self.stack.iter().rposition(|&id| self.doc.name(id) == "button") {
                    self.pop_to(at);
                }
            }
            "form" => {}
            _ => {}
        }
    }

    fn end(&mut self, name: &str) {
        match name {
            "html" | "body" => return,
            "head" => {
                if self.body.is_none() {
                    self.after_head = true;
                    self.stack.truncate(1);
                }
                return;
            }
            "br" => {
                self.start("br", Vec::new(), false);
                return;
            }
            "p" => {
                if self.open_in_scope("p", &[]).is_none() && self.body.is_some() {
                    // (A </p> with no <p> open is an empty paragraph.)
                    let cur = self.current();
                    self.doc.append(cur, Data::Element(Element { name: String::from("p"), attrs: Vec::new() }));
                    return;
                }
                self.close_p();
                return;
            }
            _ => {}
        }
        // The nearest one open (any heading for a heading), unless a table's boundary comes first.
        let boundary: &[&str] = if matches!(name, "td" | "th" | "tr" | "tbody" | "thead" | "tfoot" | "table" | "caption") {
            &["table", "html"]
        } else {
            &["html"]
        };
        let mut found = None;
        for (i, &id) in self.stack.iter().enumerate().rev() {
            let n = self.doc.name(id);
            if n == name || (heading(name) && heading(n)) {
                found = Some(i);
                break;
            }
            if boundary.contains(&n) && n != name {
                break;
            }
        }
        if let Some(at) = found {
            self.pop_to(at);
        }
    }

    fn finish(mut self) -> Document {
        self.body();
        self.doc
    }
}

/// Attributes a repeated <html> or <body> tag brings that the element hasn't got yet.
fn merge(doc: &mut Document, id: NodeId, attrs: Vec<(String, String)>) {
    if let Some(e) = doc.element_mut(id) {
        for (k, v) in attrs {
            if !e.has_attr(&k) {
                e.attrs.push((k, v));
            }
        }
    }
}
