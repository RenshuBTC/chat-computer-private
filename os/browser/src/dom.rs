//! A page's elements and words as a tree (the DOM, without anything a script would use): the document at the root,
//! then its elements, each with its attributes, and the words between them. Nodes are kept in one list and refer to
//! each other by their place in it.

use alloc::string::String;
use alloc::vec::Vec;

/// A node's place in `Document::nodes`.
pub type NodeId = usize;

pub struct Document {
    pub nodes: Vec<Node>,
    /// The page is in quirks mode (no doctype, or an old one): laid out as browsers lay out pages made before CSS.
    pub quirks: bool,
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
        Document { nodes: alloc::vec![Node { parent: None, children: Vec::new(), data: Data::Document }], quirks: true }
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
