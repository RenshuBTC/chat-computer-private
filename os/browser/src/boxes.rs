//! The boxes a page is laid out in, made from its elements and their styles (CSS's box tree): one for each element
//! that shows (none for display: none; its children in its place for display: contents), one for each run of words,
//! ::before's and ::after's, pictures and form fields as boxes of their own (replaced elements), and anonymous blocks
//! round the words and inline boxes that sit among blocks. A block inside an inline box (a link round a whole card)
//! makes the inline box a block (CSS splits the inline box; the result looks the same in the common cases).

use crate::dom::{Data, Document, NodeId};
use crate::style::{Display, Float, Style, Styles, WhiteSpace};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub type BoxId = usize;

#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// A text field: what's in it (or its placeholder, in grey), and whether it's a password's (dots).
    Text { value: String, placeholder: String, password: bool },
    /// A button made by an input (submit, reset, button): its words.
    Button(String),
    /// A checkbox or a radio button.
    Check { checked: bool, radio: bool },
    /// A drop-down list: the option chosen.
    Select(String),
    /// Several lines of text.
    Area { value: String, placeholder: String },
}

#[derive(Clone, Debug)]
pub enum Content {
    /// A box with boxes inside.
    Children,
    /// Words.
    Text(String),
    /// A picture: its place in the page's list of pictures, and what it says it is (shown when it can't be).
    Image { image: Option<usize>, alt: String },
    /// A form's field.
    Control(Control),
    /// A line break (<br>).
    Break,
}

pub struct LBox {
    /// The element (or the text node) it's made for; None for an anonymous box.
    pub node: Option<NodeId>,
    pub style: Arc<Style>,
    pub content: Content,
    pub children: Vec<BoxId>,
    pub parent: Option<BoxId>,
    // What the layout works out (device pixels): its border box, relative to its parent's (the root's: the page's).
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub margin: [f32; 4],
    pub border: [f32; 4],
    pub padding: [f32; 4],
    /// Where its last line's baseline is, from its border box's top (for inline-blocks on a line).
    pub baseline: Option<f32>,
    /// It's laid out (out-of-flow boxes wait for their containing block).
    pub done: bool,
}

impl LBox {
    pub fn display(&self) -> Display {
        self.style.display
    }

    /// Whether it sits on a line (rather than being a block of its own).
    pub fn inline_level(&self) -> bool {
        match self.content {
            Content::Text(_) | Content::Break => true,
            Content::Image { .. } | Content::Control(_) => self.style.display.is_inline_level() || self.style.display == Display::Inline,
            Content::Children => self.style.display.is_inline_level(),
        }
    }

    /// Whether it's an inline box whose insides flow on the lines (not an atomic inline-block).
    pub fn is_inline_box(&self) -> bool {
        matches!(self.content, Content::Children) && self.style.display == Display::Inline
    }

    /// Its content box's top left, relative to its border box.
    pub fn content_x(&self) -> f32 {
        self.border[3] + self.padding[3]
    }
    pub fn content_y(&self) -> f32 {
        self.border[0] + self.padding[0]
    }
    pub fn content_w(&self) -> f32 {
        (self.w - self.border[1] - self.border[3] - self.padding[1] - self.padding[3]).max(0.0)
    }
    pub fn content_h(&self) -> f32 {
        (self.h - self.border[0] - self.border[2] - self.padding[0] - self.padding[2]).max(0.0)
    }
}

/// The boxes, the root's first.
pub struct Tree {
    pub boxes: Vec<LBox>,
}

/// How a page's pictures are found: an element's picture's address, its place in the page's list.
pub trait Pictures {
    fn picture(&mut self, doc: &Document, node: NodeId) -> Option<usize>;
}

impl Tree {
    /// The boxes for a page's elements, their styles worked out.
    pub fn build(doc: &Document, styles: &Styles, pictures: &mut dyn Pictures) -> Tree {
        let mut t = Tree { boxes: Vec::new() };
        // The root box: the page's canvas, a block for the <html> element.
        let html = doc.children(0).iter().copied().find(|&c| doc.element(c).is_some());
        let root_style = html
            .and_then(|h| styles.of.get(h).cloned().flatten())
            .unwrap_or_else(|| Arc::new(Style::initial(1.0)));
        let root = t.push(html, root_style, Content::Children, None);
        if let Some(h) = html {
            let mut b = Builder { doc, styles, pictures, tree: &mut t, depth: 0 };
            b.children_of(h, root);
        }
        t.fix_mixed(0);
        t
    }

    fn push(&mut self, node: Option<NodeId>, style: Arc<Style>, content: Content, parent: Option<BoxId>) -> BoxId {
        let id = self.boxes.len();
        self.boxes.push(LBox {
            node,
            style,
            content,
            children: Vec::new(),
            parent,
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            margin: [0.0; 4],
            border: [0.0; 4],
            padding: [0.0; 4],
            baseline: None,
            done: false,
        });
        if let Some(p) = parent {
            self.boxes[p].children.push(id);
        }
        id
    }

    /// Blocks whose children are some blocks and some inline: each run of inline ones wrapped in an anonymous block,
    /// and an inline box with a block inside made a block. Without recursion (the boxes can be deep).
    fn fix_mixed(&mut self, root: BoxId) {
        let mut stack = alloc::vec![root];
        while let Some(b) = stack.pop() {
            // An inline box with blocks inside: a block.
            let kids = self.boxes[b].children.clone();
            for &k in &kids {
                if self.boxes[k].is_inline_box() && self.has_block_inside(k) {
                    let mut s = (*self.boxes[k].style).clone();
                    s.display = Display::Block;
                    self.boxes[k].style = Arc::new(s);
                }
            }
            let container = matches!(self.boxes[b].content, Content::Children)
                && !matches!(self.boxes[b].style.display, Display::Inline | Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid | Display::Table | Display::InlineTable | Display::TableRow | Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup);
            let kids = self.boxes[b].children.clone();
            let any_block = kids.iter().any(|&k| !self.boxes[k].inline_level() && !self.boxes[k].style.out_of_flow() && self.boxes[k].style.float == Float::None);
            let any_inline = kids.iter().any(|&k| self.boxes[k].inline_level() && !self.is_blank(k));
            if container && any_block && any_inline {
                let mut out: Vec<BoxId> = Vec::new();
                let mut run: Vec<BoxId> = Vec::new();
                let anon_style = Arc::new(self.boxes[b].style.inherit(1.0).with_display(Display::Block));
                let flush = |tree: &mut Tree, run: &mut Vec<BoxId>, out: &mut Vec<BoxId>| {
                    if run.is_empty() {
                        return;
                    }
                    if run.iter().all(|&k| tree.is_blank(k)) {
                        // (Spaces between blocks are nothing.)
                        run.clear();
                        return;
                    }
                    let a = tree.boxes.len();
                    tree.boxes.push(LBox {
                        node: None,
                        style: anon_style.clone(),
                        content: Content::Children,
                        children: core::mem::take(run),
                        parent: Some(b),
                        x: 0.0,
                        y: 0.0,
                        w: 0.0,
                        h: 0.0,
                        margin: [0.0; 4],
                        border: [0.0; 4],
                        padding: [0.0; 4],
                        baseline: None,
                        done: false,
                    });
                    let kids: Vec<BoxId> = tree.boxes[a].children.clone();
                    for k in kids {
                        tree.boxes[k].parent = Some(a);
                    }
                    out.push(a);
                };
                for k in kids {
                    let floating = self.boxes[k].style.out_of_flow() || self.boxes[k].style.float != Float::None;
                    if self.boxes[k].inline_level() || (floating && !run.is_empty()) {
                        run.push(k);
                    } else {
                        flush(self, &mut run, &mut out);
                        out.push(k);
                    }
                }
                flush(self, &mut run, &mut out);
                self.boxes[b].children = out;
            }
            for &k in self.boxes[b].children.iter().rev() {
                stack.push(k);
            }
        }
    }

    /// Whether `b`, an inline box, has a block among its descendants that flow with it.
    fn has_block_inside(&self, b: BoxId) -> bool {
        let mut stack: Vec<BoxId> = self.boxes[b].children.clone();
        while let Some(k) = stack.pop() {
            let x = &self.boxes[k];
            if x.style.out_of_flow() || x.style.float != Float::None {
                continue;
            }
            if !x.inline_level() {
                return true;
            }
            if x.is_inline_box() {
                stack.extend(x.children.iter().copied());
            }
        }
        false
    }

    /// Whether a box is only spaces that go (collapsible white space, alone).
    pub fn is_blank(&self, b: BoxId) -> bool {
        match &self.boxes[b].content {
            Content::Text(t) => !self.boxes[b].style.white_space.keeps_spaces() && t.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')),
            Content::Children if self.boxes[b].is_inline_box() => self.boxes[b].children.iter().all(|&k| self.is_blank(k)),
            _ => false,
        }
    }
}

struct Builder<'a> {
    doc: &'a Document,
    styles: &'a Styles,
    pictures: &'a mut dyn Pictures,
    tree: &'a mut Tree,
    depth: u32,
}

impl Builder<'_> {
    /// The boxes for `node`'s children, in `parent` (with its ::before and ::after).
    fn children_of(&mut self, node: NodeId, parent: BoxId) {
        if self.depth > 100 {
            return;
        }
        self.depth += 1;
        if let Some(s) = self.styles.before.get(&node) {
            self.pseudo(s.clone(), node, parent);
        }
        for &c in self.doc.children(node) {
            match &self.doc.nodes[c].data {
                Data::Text(t) => {
                    let style = self.tree.boxes[parent].style.clone();
                    let style = if self.tree.boxes[parent].node == Some(node) { style } else { self.style_of(node).unwrap_or(style) };
                    self.tree.push(Some(c), Arc::new(style.inherit(1.0).with_display(Display::Inline)), Content::Text(t.clone()), Some(parent));
                }
                Data::Element(_) => self.element(c, parent),
                Data::Document => {}
            }
        }
        if let Some(s) = self.styles.after.get(&node) {
            self.pseudo(s.clone(), node, parent);
        }
        self.depth -= 1;
    }

    fn style_of(&self, node: NodeId) -> Option<Arc<Style>> {
        self.styles.of.get(node).cloned().flatten()
    }

    fn pseudo(&mut self, s: Arc<Style>, node: NodeId, parent: BoxId) {
        let text = s.content.clone().unwrap_or_default();
        let b = self.tree.push(Some(node), s.clone(), Content::Children, Some(parent));
        if !text.is_empty() {
            self.tree.push(Some(node), Arc::new(s.inherit(1.0).with_display(Display::Inline)), Content::Text(text), Some(b));
        }
    }

    fn element(&mut self, node: NodeId, parent: BoxId) {
        let Some(style) = self.style_of(node) else { return };
        let Some(el) = self.doc.element(node) else { return };
        match style.display {
            Display::None => return,
            Display::Contents => {
                self.children_of(node, parent);
                return;
            }
            _ => {}
        }
        let name = el.name.as_str();
        // What shows something other than its children.
        let content = match name {
            "img" => Content::Image { image: self.pictures.picture(self.doc, node), alt: String::from(el.attr("alt").unwrap_or("")) },
            "svg" | "video" | "canvas" | "iframe" | "embed" | "object" | "audio" | "math" => {
                // (Their insides aren't drawn here: their place is kept, from their size.)
                let image = if name == "video" { el.attr("poster").and(self.pictures.picture(self.doc, node)) } else { None };
                Content::Image { image, alt: String::new() }
            }
            "br" => Content::Break,
            "input" => {
                let kind = el.attr("type").unwrap_or("text").to_ascii_lowercase();
                let value = String::from(el.attr("value").unwrap_or(""));
                match kind.as_str() {
                    "hidden" => return,
                    "submit" | "reset" | "button" => {
                        let words = if value.is_empty() {
                            String::from(match kind.as_str() {
                                "submit" => "Submit",
                                "reset" => "Reset",
                                _ => "",
                            })
                        } else {
                            value
                        };
                        Content::Control(Control::Button(words))
                    }
                    "checkbox" | "radio" => Content::Control(Control::Check { checked: el.has_attr("checked"), radio: kind == "radio" }),
                    "image" => Content::Image { image: self.pictures.picture(self.doc, node), alt: String::from(el.attr("alt").unwrap_or("")) },
                    "file" => Content::Control(Control::Button(String::from("Choose a file"))),
                    "range" | "color" => Content::Control(Control::Text { value: String::new(), placeholder: String::new(), password: false }),
                    _ => Content::Control(Control::Text {
                        value,
                        placeholder: String::from(el.attr("placeholder").unwrap_or("")),
                        password: kind == "password",
                    }),
                }
            }
            "textarea" => Content::Control(Control::Area {
                value: el.attr("value").map(String::from).unwrap_or_else(|| self.doc.text_of(node)),
                placeholder: String::from(el.attr("placeholder").unwrap_or("")),
            }),
            "select" => {
                // The option chosen: the one marked selected, else the first.
                let options: Vec<NodeId> = self.doc.descendants(node).filter(|&d| self.doc.name(d) == "option").collect();
                let chosen = options.iter().copied().find(|&o| self.doc.element(o).is_some_and(|e| e.has_attr("selected"))).or(options.first().copied());
                let words = chosen.map(|o| String::from(self.doc.text_of(o).trim())).unwrap_or_default();
                Content::Control(Control::Select(words))
            }
            _ => Content::Children,
        };
        let walk = matches!(content, Content::Children);
        // A table cell's colspan, kept for the layout in its style's grid_column span.
        let style = match (name, el.attr("colspan").and_then(|c| c.trim().parse::<u32>().ok())) {
            ("td" | "th", Some(span)) if span > 1 => {
                let mut s = (*style).clone();
                s.grid_column.span = span.min(1000);
                Arc::new(s)
            }
            _ => style,
        };
        let b = self.tree.push(Some(node), style, content, Some(parent));
        if walk && !matches!(name, "select" | "textarea" | "template") {
            self.children_of(node, b);
        }
    }
}

impl Style {
    /// The same style with another display.
    pub fn with_display(mut self, d: Display) -> Style {
        self.display = d;
        self
    }

    /// Whether its words are kept as they're written (pre and the like).
    pub fn pre(&self) -> bool {
        matches!(self.white_space, WhiteSpace::Pre | WhiteSpace::PreWrap)
    }
}
