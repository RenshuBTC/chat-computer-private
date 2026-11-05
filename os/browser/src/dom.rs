//! A page's elements and words as a tree (the DOM): the document at the root, then its elements, each with its
//! attributes, and the words and comments between them. Nodes are kept in one list and refer to each other by their
//! place in it; a node taken out of the tree (by a script) stays in the list, with no parent, and can be put back.

use alloc::string::String;
use alloc::vec::Vec;

/// A node's place in `Document::nodes`.
pub type NodeId = usize;

pub struct Document {
    pub nodes: Vec<Node>,
    /// The page is in quirks mode (no doctype, or an old one): laid out as browsers lay out pages made before CSS.
    pub quirks: bool,
    /// Each <template>'s contents: (the element, the fragment its insides are in).
    pub templates: Vec<(NodeId, NodeId)>,
    /// Counts the tree's changes (what's worked out from the tree is kept while it stays the same).
    pub version: u64,
    /// Shadow roots: (their host element, the fragment holding the shadow tree).
    pub shadows: Vec<(NodeId, NodeId)>,
}

pub struct Node {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub data: Data,
}

pub enum Data {
    Document,
    Element(Element),
    Text(String),
    Comment(String),
    /// A document fragment (a script's, or a <template>'s contents): its children are what it holds.
    Fragment,
}

pub struct Element {
    /// Its name, in lower case ("div").
    pub name: String,
    /// Its attributes, names in lower case, in the order they came.
    pub attrs: Vec<(String, String)>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn has_attr(&self, name: &str) -> bool {
        self.attrs.iter().any(|(k, _)| k == name)
    }

    pub fn id(&self) -> Option<&str> {
        self.attr("id").filter(|v| !v.is_empty())
    }

    /// Its classes (the words of its class attribute).
    pub fn classes(&self) -> impl Iterator<Item = &str> {
        self.attr("class").unwrap_or("").split_ascii_whitespace()
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.classes().any(|c| c == class)
    }

    pub fn set_attr(&mut self, name: &str, value: &str) {
        match self.attrs.iter_mut().find(|(k, _)| k == name) {
            Some(a) => a.1 = String::from(value),
            None => self.attrs.push((String::from(name), String::from(value))),
        }
    }

    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|(k, _)| k != name);
    }
}

impl Document {
    pub fn new() -> Document {
        Document { nodes: alloc::vec![Node { parent: None, children: Vec::new(), data: Data::Document }], quirks: true, templates: Vec::new(), version: 0, shadows: Vec::new() }
    }

    pub fn element(&self, id: NodeId) -> Option<&Element> {
        match &self.nodes.get(id)?.data {
            Data::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn element_mut(&mut self, id: NodeId) -> Option<&mut Element> {
        match &mut self.nodes.get_mut(id)?.data {
            Data::Element(e) => Some(e),
            _ => None,
        }
    }

    /// The element's name, or "" for what isn't an element.
    pub fn name(&self, id: NodeId) -> &str {
        self.element(id).map_or("", |e| e.name.as_str())
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(id)?.parent
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.nodes.get(id).map_or(&[], |n| n.children.as_slice())
    }

    /// The element's parent, if it's an element.
    pub fn parent_element(&self, id: NodeId) -> Option<NodeId> {
        self.parent(id).filter(|&p| self.element(p).is_some())
    }

    /// A new node, at the end of `parent`'s children.
    pub fn append(&mut self, parent: NodeId, data: Data) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node { parent: Some(parent), children: Vec::new(), data });
        if let Some(p) = self.nodes.get_mut(parent) {
            p.children.push(id);
        }
        id
    }

    /// A new node, in no tree yet.
    pub fn create(&mut self, data: Data) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node { parent: None, children: Vec::new(), data });
        id
    }

    /// Takes a node out of its parent's children (it keeps its own).
    pub fn detach(&mut self, id: NodeId) {
        if let Some(p) = self.nodes.get(id).and_then(|n| n.parent) {
            if let Some(pn) = self.nodes.get_mut(p) {
                pn.children.retain(|&c| c != id);
            }
            self.nodes[id].parent = None;
            self.version += 1;
        }
    }

    /// Puts `child` in `parent`'s children, before `before` (at the end if that's None or isn't a child), taking it
    /// out of where it was first.
    pub fn insert(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) {
        if child >= self.nodes.len() || parent >= self.nodes.len() {
            return;
        }
        self.detach(child);
        let kids = &mut self.nodes[parent].children;
        let at = before.and_then(|b| kids.iter().position(|&c| c == b)).unwrap_or(kids.len());
        kids.insert(at, child);
        self.nodes[child].parent = Some(parent);
        self.version += 1;
    }

    /// Whether `a` is `b` or has it inside.
    pub fn contains(&self, a: NodeId, b: NodeId) -> bool {
        let mut at = Some(b);
        let mut steps = 0;
        while let Some(n) = at {
            if n == a {
                return true;
            }
            steps += 1;
            if steps > 100_000 {
                return false;
            }
            at = self.parent(n);
        }
        false
    }

    /// The outermost node `id` is in (the document, if it's in the page).
    pub fn root_of(&self, id: NodeId) -> NodeId {
        let mut at = id;
        while let Some(p) = self.parent(at) {
            at = p;
        }
        at
    }

    /// Whether the node is in the page (its root the document, or a shadow root whose host is in the page).
    pub fn connected(&self, id: NodeId) -> bool {
        let mut at = id;
        for _ in 0..64 {
            let r = self.root_of(at);
            if r == 0 {
                return true;
            }
            match self.shadows.iter().find(|(_, f)| *f == r) {
                Some((h, _)) => at = *h,
                None => return false,
            }
        }
        false
    }

    /// A shadow root's host.
    pub fn shadow_host(&self, root: NodeId) -> Option<NodeId> {
        self.shadows.iter().find(|(_, f)| *f == root).map(|(h, _)| *h)
    }

    /// An element's shadow root.
    pub fn shadow_of(&self, host: NodeId) -> Option<NodeId> {
        self.shadows.iter().find(|(h, _)| *h == host).map(|(_, f)| *f)
    }

    /// What a <slot> in a shadow tree shows of its host's children: those with its name in their slot attribute
    /// (for the slot with no name, those with none, and the host's text). Empty for a slot not in a shadow tree.
    pub fn assigned(&self, slot: NodeId) -> Vec<NodeId> {
        let Some(host) = self.shadow_host(self.root_of(slot)) else { return Vec::new() };
        let name = self.element(slot).and_then(|e| e.attr("name")).unwrap_or("");
        self.children(host)
            .iter()
            .copied()
            .filter(|&c| match &self.nodes[c].data {
                Data::Element(e) => e.attr("slot").unwrap_or("") == name,
                Data::Text(_) => name.is_empty(),
                _ => false,
            })
            .collect()
    }

    /// The node after and before `id` among its parent's children.
    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        let kids = self.children(self.parent(id)?);
        let at = kids.iter().position(|&c| c == id)?;
        kids.get(at + 1).copied()
    }

    pub fn previous_sibling(&self, id: NodeId) -> Option<NodeId> {
        let kids = self.children(self.parent(id)?);
        let at = kids.iter().position(|&c| c == id)?;
        if at == 0 { None } else { kids.get(at - 1).copied() }
    }

    /// A <template>'s contents (a fragment), made if it has none yet.
    pub fn template_content(&mut self, id: NodeId) -> NodeId {
        if let Some(&(_, f)) = self.templates.iter().find(|(t, _)| *t == id) {
            return f;
        }
        let f = self.create(Data::Fragment);
        self.templates.push((id, f));
        f
    }

    /// A copy of a node (and of what's inside it, if `deep`), in no tree yet.
    pub fn clone_node(&mut self, id: NodeId, deep: bool) -> NodeId {
        let data = match &self.nodes[id].data {
            Data::Document | Data::Fragment => Data::Fragment,
            Data::Element(e) => Data::Element(Element { name: e.name.clone(), attrs: e.attrs.clone() }),
            Data::Text(t) => Data::Text(t.clone()),
            Data::Comment(t) => Data::Comment(t.clone()),
        };
        let copy = self.create(data);
        if deep {
            // (Without recursion: (original, its copy) pairs to do.)
            let mut work = alloc::vec![(id, copy)];
            while let Some((from, to)) = work.pop() {
                let kids: Vec<NodeId> = self.children(from).to_vec();
                for k in kids {
                    let c = self.clone_node(k, false);
                    self.nodes[c].parent = Some(to);
                    self.nodes[to].children.push(c);
                    work.push((k, c));
                }
                if self.name(from) == "template"
                    && let Some(&(_, f)) = self.templates.iter().find(|(t, _)| *t == from)
                {
                    let fc = self.clone_node(f, true);
                    self.templates.push((to, fc));
                }
            }
        }
        copy
    }

    /// The node's text (a text node's or a comment's words), if it has any of its own.
    pub fn data_of(&self, id: NodeId) -> Option<&str> {
        match &self.nodes.get(id)?.data {
            Data::Text(t) | Data::Comment(t) => Some(t),
            _ => None,
        }
    }

    /// The node as HTML (an element with what's inside it, as `outerHTML` gives it).
    pub fn outer_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.serialize(id, &mut out, true);
        out
    }

    /// What's inside the node, as HTML (`innerHTML`).
    pub fn inner_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.serialize(id, &mut out, false);
        out
    }

    fn serialize(&self, id: NodeId, out: &mut String, with_self: bool) {
        // (Without recursion: what's left to write, a node to open or an end tag to close.)
        enum Step {
            Open(NodeId),
            Close(NodeId),
        }
        let mut work: Vec<Step> = Vec::new();
        if with_self {
            work.push(Step::Open(id));
        } else {
            let start = if self.name(id) == "template" { self.templates.iter().find(|(t, _)| *t == id).map_or(id, |x| x.1) } else { id };
            for &k in self.children(start).iter().rev() {
                work.push(Step::Open(k));
            }
        }
        while let Some(step) = work.pop() {
            match step {
                Step::Close(n) => {
                    out.push_str("</");
                    out.push_str(self.name(n));
                    out.push('>');
                }
                Step::Open(n) => match &self.nodes[n].data {
                    Data::Text(t) => {
                        let raw = self.parent(n).is_some_and(|p| matches!(self.name(p), "script" | "style" | "xmp" | "iframe" | "noembed" | "noframes" | "plaintext" | "noscript"));
                        if raw {
                            out.push_str(t);
                        } else {
                            escape_into(out, t, false);
                        }
                    }
                    Data::Comment(t) => {
                        out.push_str("<!--");
                        out.push_str(t);
                        out.push_str("-->");
                    }
                    Data::Document | Data::Fragment => {
                        for &k in self.children(n).iter().rev() {
                            work.push(Step::Open(k));
                        }
                    }
                    Data::Element(e) => {
                        out.push('<');
                        out.push_str(&e.name);
                        for (k, v) in &e.attrs {
                            out.push(' ');
                            out.push_str(k);
                            out.push_str("=\"");
                            escape_into(out, v, true);
                            out.push('"');
                        }
                        out.push('>');
                        if VOID_ELEMENTS.contains(&e.name.as_str()) {
                            continue;
                        }
                        work.push(Step::Close(n));
                        let inside = if e.name == "template" { self.templates.iter().find(|(t, _)| *t == n).map_or(n, |x| x.1) } else { n };
                        for &k in self.children(inside).iter().rev() {
                            work.push(Step::Open(k));
                        }
                    }
                },
            }
        }
    }

    /// The first element named `name`, in the order they come.
    pub fn find(&self, name: &str) -> Option<NodeId> {
        self.descendants(0).find(|&id| self.name(id) == name)
    }

    /// `id` and everything inside it, in the order they come (without recursion: a page's elements can be deep).
    pub fn descendants(&self, id: NodeId) -> Descendants<'_> {
        Descendants { doc: self, stack: alloc::vec![id] }
    }

    /// The words inside `id`, as they are.
    pub fn text_of(&self, id: NodeId) -> String {
        let mut s = String::new();
        for n in self.descendants(id) {
            if let Some(Node { data: Data::Text(t), .. }) = self.nodes.get(n) {
                s.push_str(t);
            }
        }
        s
    }

    /// The element's siblings that are elements, before it and after it (in order), and its place among them.
    pub fn element_siblings(&self, id: NodeId) -> (Vec<NodeId>, usize) {
        let Some(p) = self.parent(id) else { return (alloc::vec![id], 0) };
        let all: Vec<NodeId> = self.children(p).iter().copied().filter(|&c| self.element(c).is_some()).collect();
        let at = all.iter().position(|&c| c == id).unwrap_or(0);
        (all, at)
    }

    /// The nearest element around `id` (itself included) named `name`.
    pub fn ancestor_named(&self, id: NodeId, name: &str) -> Option<NodeId> {
        let mut at = Some(id);
        while let Some(n) = at {
            if self.name(n) == name {
                return Some(n);
            }
            at = self.parent(n);
        }
        None
    }
}

/// Elements with no insides and no end tag.
pub const VOID_ELEMENTS: [&str; 15] =
    ["area", "base", "br", "col", "embed", "hr", "img", "input", "keygen", "link", "meta", "param", "source", "track", "wbr"];

/// Text written as HTML: & < > (and " in an attribute's value) as entities, and the no-break space.
fn escape_into(out: &mut String, t: &str, attr: bool) {
    for c in t.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '"' if attr => out.push_str("&quot;"),
            '<' if !attr => out.push_str("&lt;"),
            '>' if !attr => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

impl Default for Document {
    fn default() -> Self {
        Document::new()
    }
}

pub struct Descendants<'a> {
    doc: &'a Document,
    stack: Vec<NodeId>,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let id = self.stack.pop()?;
        if let Some(n) = self.doc.nodes.get(id) {
            self.stack.extend(n.children.iter().rev());
        }
        Some(id)
    }
}
