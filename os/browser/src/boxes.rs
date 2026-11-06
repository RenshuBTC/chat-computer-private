//! The boxes a page is laid out in, made from its elements and their styles (CSS's box tree): one for each element
//! that shows (none for display: none; its children in its place for display: contents), one for each run of words,
//! ::before's and ::after's, pictures and form fields as boxes of their own (replaced elements), and anonymous blocks
//! round the words and inline boxes that sit among blocks. A block inside an inline box (a link round a whole card)
//! makes the inline box a block (CSS splits the inline box; the result looks the same in the common cases).

use crate::counter_style;
use crate::counters::{self, Counters, ListRole};
use crate::dom::{Data, Document, NodeId};
use crate::style::{ContentPart, Display, Float, ListStyle, Size, Style, Styles, WhiteSpace};
use crate::values::Len;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub type BoxId = usize;

#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// A text field: what's in it (or its placeholder, in grey), whether it's a password's (dots), and how many
    /// characters wide it's made (its size attribute: 20 if it has none).
    Text { value: String, placeholder: String, password: bool, size: u16 },
    /// A button made by an input (submit, reset, button): its words.
    Button(String),
    /// A checkbox or a radio button.
    Check { checked: bool, radio: bool },
    /// A drop-down list: the option chosen, and the widest option's words (it's made as wide as they are).
    Select(String, String),
    /// Several lines of text: what's in it (or its placeholder), and how many characters wide and lines high it's
    /// made (its cols and rows: 20 and 2 if it has none).
    Area { value: String, placeholder: String, cols: u16, rows: u16 },
}

#[derive(Clone, Debug)]
pub enum Content {
    /// A box with boxes inside.
    Children,
    /// Words.
    Text(String),
    /// A picture: its place in the page's list of pictures, what it says it is (shown when it can't be), and its
    /// density (device pixels of the picture to a CSS pixel: its size in CSS pixels is its own divided by it).
    Image { image: Option<usize>, alt: String, density: f32 },
    /// A form's field.
    Control(Control),
    /// A line break (<br>).
    Break,
}

/// A list item's marker: words ("1. ", or a string of the page's), or one of the shapes Chrome draws for disc,
/// circle, square and the disclosure triangles.
#[derive(Clone, Debug, PartialEq)]
pub enum Marker {
    Text(String),
    /// A picture (list-style-image): the page's resource.
    Image(usize),
    Disc,
    Circle,
    Square,
    Open,
    Closed,
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
    /// Its insides are centred in its height (a <button>'s, as Chrome centres them).
    pub centered: bool,
    /// An <iframe>'s: its document is drawn in its content box.
    pub frame: bool,
    /// Its size (CSS pixels) where it has none of its own and CSS gives none: a frame's and a video's 300 by 150, as
    /// Chrome makes them; nothing for a picture that hasn't come.
    pub default_size: (f32, f32),
    /// A list item's marker, outside it (beside its first line).
    pub marker: Option<Marker>,
    /// An inside marker's own box: the shape it draws (Text for one of words or a picture).
    pub marker_box: Option<Marker>,
    /// An outside marker's ::marker style, if a rule gives it one (its font and colour).
    pub marker_style: Option<Arc<Style>>,
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
    /// A picture a style names (content: url()), by its address as written.
    fn url(&self, _url: &str) -> Option<usize> {
        None
    }
    /// The density of the picture an element shows (its srcset's candidate's), if it isn't 1.
    fn density(&self, _node: NodeId) -> f32 {
        1.0
    }
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
            let mut b = Builder { doc, styles, pictures, tree: &mut t, depth: 0, counters: Counters::new(), ancestors: Vec::new() };
            // (The root's own counters.)
            let root_style = b.tree.boxes[root].style.clone();
            let me = b.counters.enter(&root_style, Counters::role(doc, &[], Some(h), &root_style));
            b.children_of(h, root);
            b.counters.leave(me);
        }
        // Overflow goes to the page's view: the root's, or, when the root's is visible, its <body>'s, which then
        // doesn't clip what's in it itself (CSS 2.1, 11.1.1: a body of no height with overflow-y: scroll, as
        // YouTube's, holds a whole page).
        if !t.boxes[root].style.clip
            && let Some(body) = t.boxes[root].children.iter().copied().find(|&k| t.boxes[k].node.is_some_and(|n| doc.name(n) == "body"))
            && t.boxes[body].style.clip
        {
            let mut s = (*t.boxes[body].style).clone();
            s.clip = false;
            s.scrolls = false;
            t.boxes[body].style = Arc::new(s);
        }
        t.fix_tables(0);
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
            centered: false,
            frame: false,
            default_size: (0.0, 0.0),
            marker: None,
            marker_box: None,
            marker_style: None,
        });
        if let Some(p) = parent {
            self.boxes[p].children.push(id);
        }
        id
    }

    /// An anonymous box (one no element makes) in `parent`, in a style its own display inherits from the parent's,
    /// round `children` (taken out of the parent's: the caller puts it in their place).
    fn anonymous(&mut self, parent: BoxId, display: Display, children: Vec<BoxId>) -> BoxId {
        let style = Arc::new(self.boxes[parent].style.inherit(1.0).with_display(display));
        let a = self.boxes.len();
        self.boxes.push(LBox {
            node: None,
            style,
            content: Content::Children,
            children: Vec::new(),
            parent: Some(parent),
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            margin: [0.0; 4],
            border: [0.0; 4],
            padding: [0.0; 4],
            baseline: None,
            done: false,
            centered: false,
            frame: false,
            default_size: (0.0, 0.0),
            marker: None,
            marker_box: None,
            marker_style: None,
        });
        for &k in &children {
            self.boxes[k].parent = Some(a);
        }
        self.boxes[a].children = children;
        a
    }

    /// CSS's anonymous table boxes (CSS 2.1, 17.2.1), as Chrome makes them: spaces between a table's parts go; what's
    /// in a table that isn't one of its parts goes in an anonymous row, what's in a row group that isn't a row in an
    /// anonymous row, and what's in a row that isn't a cell in an anonymous cell; cells not in a row are put in an
    /// anonymous row, and rows, row groups and captions not in a table in an anonymous table (an inline-table, in an
    /// inline box): <div style="display: table-cell"> side by side are a table's row.
    fn fix_tables(&mut self, root: BoxId) {
        let internal = |d: Display| matches!(d, Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup | Display::TableRow | Display::TableCell | Display::TableColumn | Display::TableColumnGroup);
        let proper = |d: Display| matches!(d, Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup | Display::TableRow | Display::TableColumn | Display::TableColumnGroup | Display::TableCaption);
        let mut stack = alloc::vec![root];
        while let Some(b) = stack.pop() {
            if !matches!(self.boxes[b].content, Content::Children) || self.boxes[b].children.is_empty() {
                continue;
            }
            let d = self.boxes[b].style.display;
            let table = matches!(d, Display::Table | Display::InlineTable);
            let group = matches!(d, Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup);
            let row = d == Display::TableRow;
            let shown = |tree: &Tree, k: BoxId| tree.boxes[k].style.display;
            let part = |tree: &Tree, k: BoxId| !tree.boxes[k].style.out_of_flow() && (internal(shown(tree, k)) || shown(tree, k) == Display::TableCaption);
            // Spaces that go: all in a table, a row group or a row; elsewhere, those between two of a table's parts.
            let kids = self.boxes[b].children.clone();
            let mut kept: Vec<BoxId> = Vec::with_capacity(kids.len());
            for (i, &k) in kids.iter().enumerate() {
                if self.is_blank(k) && matches!(self.boxes[k].content, Content::Text(_)) {
                    if table || group || row {
                        continue;
                    }
                    let before = kept.last().is_some_and(|&p| part(self, p));
                    let after = kids[i + 1..].iter().find(|&&n| !self.is_blank(n)).is_some_and(|&n| part(self, n));
                    if before && after {
                        continue;
                    }
                }
                kept.push(k);
            }
            // Runs of children wrapped: `wrap(k)` for those that go in one, made `display`.
            let regroup = |tree: &mut Tree, kids: Vec<BoxId>, wrap: &dyn Fn(&Tree, BoxId) -> bool, display: Display| -> Vec<BoxId> {
                let mut out: Vec<BoxId> = Vec::with_capacity(kids.len());
                let mut run: Vec<BoxId> = Vec::new();
                for k in kids {
                    if wrap(tree, k) {
                        run.push(k);
                    } else {
                        if !run.is_empty() {
                            out.push(tree.anonymous(b, display, core::mem::take(&mut run)));
                        }
                        out.push(k);
                    }
                }
                if !run.is_empty() {
                    out.push(tree.anonymous(b, display, run));
                }
                out
            };
            let in_flow = |tree: &Tree, k: BoxId| !tree.boxes[k].style.out_of_flow();
            // (A column group holds columns only, and a column nothing: what else is in them isn't shown.)
            if d == Display::TableColumnGroup || d == Display::TableColumn {
                let cols: Vec<BoxId> = if d == Display::TableColumnGroup { kept.into_iter().filter(|&k| shown(self, k) == Display::TableColumn).collect() } else { Vec::new() };
                self.boxes[b].children = cols;
                continue;
            }
            let kids = if table {
                regroup(self, kept, &|t: &Tree, k| in_flow(t, k) && !proper(shown(t, k)), Display::TableRow)
            } else if group {
                regroup(self, kept, &|t: &Tree, k| in_flow(t, k) && shown(t, k) != Display::TableRow, Display::TableRow)
            } else if row {
                regroup(self, kept, &|t: &Tree, k| in_flow(t, k) && shown(t, k) != Display::TableCell, Display::TableCell)
            } else {
                // Cells not in a row: an anonymous row round each run of them; then rows, row groups and captions
                // not in a table: an anonymous table round each run of them.
                let kids = regroup(self, kept, &|t: &Tree, k| in_flow(t, k) && shown(t, k) == Display::TableCell, Display::TableRow);
                let around = if d == Display::Inline { Display::InlineTable } else { Display::Table };
                regroup(self, kids, &|t: &Tree, k| in_flow(t, k) && proper(shown(t, k)), around)
            };
            self.boxes[b].children = kids;
            for &k in self.boxes[b].children.iter().rev() {
                stack.push(k);
            }
        }
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
            centered: false,
            frame: false,
            default_size: (0.0, 0.0),
            marker: None,
            marker_box: None,
            marker_style: None,
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
            // A flex or grid container's runs of text: each an anonymous block, an item of its own (its elements are
            // items already, made blocks).
            let items = matches!(self.boxes[b].content, Content::Children)
                && matches!(self.boxes[b].style.display, Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid);
            let kids = self.boxes[b].children.clone();
            let texty = |tree: &Tree, k: BoxId| matches!(tree.boxes[k].content, Content::Text(_) | Content::Break);
            if items && kids.iter().any(|&k| texty(self, k) && !self.is_blank(k)) {
                let anon_style = Arc::new(self.boxes[b].style.inherit(1.0).with_display(Display::Block));
                let mut out: Vec<BoxId> = Vec::new();
                let mut run: Vec<BoxId> = Vec::new();
                let wrap = |tree: &mut Tree, run: &mut Vec<BoxId>, out: &mut Vec<BoxId>| {
                    if run.is_empty() {
                        return;
                    }
                    if run.iter().all(|&k| tree.is_blank(k)) {
                        // (Spaces between items are nothing.)
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
            centered: false,
            frame: false,
            default_size: (0.0, 0.0),
            marker: None,
            marker_box: None,
            marker_style: None,
                    });
                    let kids: Vec<BoxId> = tree.boxes[a].children.clone();
                    for k in kids {
                        tree.boxes[k].parent = Some(a);
                    }
                    out.push(a);
                };
                for k in kids {
                    if texty(self, k) {
                        run.push(k);
                    } else {
                        wrap(self, &mut run, &mut out);
                        out.push(k);
                    }
                }
                wrap(self, &mut run, &mut out);
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

/// A piece of a pseudo-element's content.
enum Piece {
    Text(String),
    Image(String),
}

struct Builder<'a> {
    doc: &'a Document,
    styles: &'a Styles,
    pictures: &'a mut dyn Pictures,
    tree: &'a mut Tree,
    depth: u32,
    counters: Counters,
    /// The elements whose children are being made, outermost first (the flat tree's: a slot's assigned nodes are
    /// its children).
    ancestors: Vec<NodeId>,
}

impl Builder<'_> {
    /// The boxes for `node`'s children, in `parent` (with its ::before and ::after).
    fn children_of(&mut self, node: NodeId, parent: BoxId) {
        if self.depth > 100 {
            return;
        }
        self.depth += 1;
        self.ancestors.push(node);
        if let Some(s) = self.styles.before.get(&node) {
            self.pseudo(s.clone(), node, parent);
        }
        // (A closed <details> shows its first <summary> alone.)
        let closed = self.doc.name(node) == "details" && self.doc.element(node).is_some_and(|e| !e.has_attr("open"));
        let summary = closed.then(|| self.doc.children(node).iter().copied().find(|&c| self.doc.name(c) == "summary")).flatten();
        for c in self.flat_children(node) {
            if closed && Some(c) != summary {
                continue;
            }
            match &self.doc.nodes[c].data {
                Data::Text(t) => {
                    let style = self.tree.boxes[parent].style.clone();
                    let style = if self.tree.boxes[parent].node == Some(node) { style } else { self.style_of(node).unwrap_or(style) };
                    self.tree.push(Some(c), Arc::new(style.inherit(1.0).with_display(Display::Inline)), Content::Text(t.clone()), Some(parent));
                }
                Data::Element(_) => self.element(c, parent),
                Data::Document | Data::Comment(_) | Data::Fragment => {}
            }
        }
        if let Some(s) = self.styles.after.get(&node) {
            self.pseudo(s.clone(), node, parent);
        }
        self.ancestors.pop();
        self.depth -= 1;
    }

    /// A pseudo-element's content as words alone (pictures left out).
    fn content_text(&mut self, s: &Style, node: NodeId) -> String {
        self.content_pieces(s, node).into_iter().filter_map(|p| if let Piece::Text(t) = p { Some(t) } else { None }).collect()
    }

    /// A pseudo-element's content: words (its strings and attributes, its counters' values, counter-reset,
    /// counter-increment and counter-set of its own first, and its quotes, at their depth) and pictures.
    fn content_pieces(&mut self, s: &Style, node: NodeId) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let mut out = String::new();
        let Some(parts) = s.content.clone() else { return pieces };
        for p in parts.iter() {
            match p {
                ContentPart::Text(t) => out.push_str(t),
                ContentPart::Counter { name, between, style } => {
                    let values = match between {
                        Some(_) => self.counters.values(name),
                        None => alloc::vec![self.counters.value(name)],
                    };
                    if let Some(st) = style {
                        let words: Vec<String> = values.iter().map(|&v| counter_style::represent(st, v as i64, &self.styles.counter_styles)).collect();
                        out.push_str(&words.join(between.as_deref().unwrap_or("")));
                    }
                }
                ContentPart::OpenQuote | ContentPart::CloseQuote => {
                    let open = matches!(p, ContentPart::OpenQuote);
                    if !open {
                        if self.counters.quote_depth == 0 {
                            continue;
                        }
                        self.counters.quote_depth -= 1;
                    }
                    let depth = self.counters.quote_depth;
                    let q = match &s.quotes {
                        Some(list) => list.get(depth.min(list.len().saturating_sub(1))).map(|(o, c)| if open { o.clone() } else { c.clone() }).unwrap_or_default(),
                        None => {
                            let pair = counters::language_quotes(&self.language(node))[depth.min(1)];
                            String::from(if open { pair.0 } else { pair.1 })
                        }
                    };
                    out.push_str(&q);
                    if open {
                        self.counters.quote_depth += 1;
                    }
                }
                ContentPart::NoOpenQuote => self.counters.quote_depth += 1,
                ContentPart::NoCloseQuote => self.counters.quote_depth = self.counters.quote_depth.saturating_sub(1),
                ContentPart::Image(u) => {
                    if !out.is_empty() {
                        pieces.push(Piece::Text(core::mem::take(&mut out)));
                    }
                    pieces.push(Piece::Image(u.clone()));
                }
            }
        }
        if !out.is_empty() {
            pieces.push(Piece::Text(out));
        }
        pieces
    }

    /// An element's language: its own lang attribute's, or its nearest ancestor's.
    fn language(&self, node: NodeId) -> String {
        let mut at = Some(node);
        while let Some(n) = at {
            if let Some(l) = self.doc.element(n).and_then(|e| e.attr("lang").or(e.attr("xml:lang"))) {
                return String::from(l);
            }
            at = self.doc.parent(n);
        }
        String::new()
    }

    /// A list item's marker (an element's, or a ::before's or ::after's, `node` None), with its number, worked out for
    /// every list item, shown or not, as each counts on its list; and its ::marker style, if a rule gives it one. Its
    /// ::marker's content, if it has some, in place of the list's own (none: no marker).
    fn list_marker(&mut self, node: Option<NodeId>, origin: NodeId, style: &Style) -> (Option<Marker>, Option<Arc<Style>>) {
        let n = self.counters.number(self.doc, self.styles, &self.ancestors, node, style);
        let ms = node.and_then(|k| self.styles.marker.get(&k).cloned());
        if let Some(m) = &ms {
            if m.content_none {
                return (None, ms);
            }
            if m.content.is_some() {
                let m = m.clone();
                return (Some(Marker::Text(self.content_text(&m, origin))), ms);
            }
        }
        // (A picture for a marker: drawn once it's come; nothing in its place till then, nor if it doesn't come, as
        // Chrome has it.)
        if let Some(u) = &style.list_image {
            return (self.pictures.url(u).map(Marker::Image), ms);
        }
        let marker = match &style.list_style {
            ListStyle::None => None,
            ListStyle::Text(t) => Some(Marker::Text(String::from(&**t))),
            ListStyle::Named(name) => Some(match &**name {
                "disc" => Marker::Disc,
                "circle" => Marker::Circle,
                "square" => Marker::Square,
                "disclosure-open" => Marker::Open,
                "disclosure-closed" => Marker::Closed,
                _ => Marker::Text(counter_style::marker(name, n as i64, &self.styles.counter_styles)),
            }),
        };
        (marker, ms)
    }

    /// A list item's marker put where it goes: beside it (outside), or a box at the start of its first line.
    fn place_marker(&mut self, b: BoxId, node: NodeId, marker: Option<Marker>, ms: Option<Arc<Style>>, walk: bool) {
        let Some(m) = marker else { return };
        let inside = self.tree.boxes[b].style.list_inside;
        if inside && walk {
            self.inside_marker(b, node, m, ms);
        } else {
            self.tree.boxes[b].marker = Some(m);
            self.tree.boxes[b].marker_style = ms;
        }
    }

    /// An inside marker: a box at the start of the item's first line. Words are an inline box of them (their spaces
    /// kept, as ::marker's white-space: pre keeps them); a shape, a box as wide as Chrome makes its: the shape, a pixel
    /// to either side of it, and an em after (a triangle and 0.4em after it), one pixel back to start.
    fn inside_marker(&mut self, item: BoxId, node: NodeId, marker: Marker, ms: Option<Arc<Style>>) {
        let li = ms.unwrap_or_else(|| {
            let mut m = self.tree.boxes[item].style.inherit(1.0);
            m.transform_text = crate::style::Transform::None;
            Arc::new(m)
        });
        let mut s = li.inherit(1.0).with_display(Display::Inline);
        s.white_space = WhiteSpace::Pre;
        match marker {
            // A picture: on the line, 7 pixels before the words.
            Marker::Image(n) => {
                s.margin = [Size::Len(Len::ZERO), Size::Len(Len::px(7.0)), Size::Len(Len::ZERO), Size::Len(Len::ZERO)];
                let b = self.tree.push(Some(node), Arc::new(s), Content::Image { image: Some(n), alt: String::new(), density: 1.0 }, Some(item));
                self.tree.boxes[b].marker_box = Some(Marker::Text(String::new()));
            }
            Marker::Text(t) => {
                let s = Arc::new(s);
                let b = self.tree.push(Some(node), s.clone(), Content::Children, Some(item));
                self.tree.boxes[b].marker_box = Some(Marker::Text(String::new()));
                self.tree.push(Some(node), Arc::new(s.inherit(1.0).with_display(Display::Inline)), Content::Text(t), Some(b));
            }
            shape => {
                s.display = Display::InlineBlock;
                let size = li.font_size;
                let (w, before, after) = if matches!(shape, Marker::Open | Marker::Closed) {
                    (crate::layout::disclosure_size(size), 0.0, size * 0.4)
                } else {
                    (crate::layout::bullet_size(li.ascent_px()) as f32 + 2.0, -1.0, size)
                };
                s.width = Size::Len(Len::px(w));
                s.height = Size::Len(Len::ZERO);
                s.margin = [Size::Len(Len::ZERO), Size::Len(Len::px(after)), Size::Len(Len::ZERO), Size::Len(Len::px(before))];
                let b = self.tree.push(Some(node), Arc::new(s), Content::Children, Some(item));
                self.tree.boxes[b].marker_box = Some(shape);
            }
        }
    }

    fn style_of(&self, node: NodeId) -> Option<Arc<Style>> {
        self.styles.of.get(node).cloned().flatten()
    }

    /// What's shown inside a node: a shadow host's shadow tree (in place of its own children); in a shadow tree, a
    /// <slot>'s share of the host's children (those with its name in their slot attribute; for the slot with no
    /// name, those with none, and the host's text), or its own children if it gets none.
    fn flat_children(&self, node: NodeId) -> Vec<NodeId> {
        if let Some(root) = self.doc.shadow_of(node) {
            return self.doc.children(root).to_vec();
        }
        if self.doc.name(node) == "slot" {
            let given = self.doc.assigned(node);
            if !given.is_empty() {
                return given;
            }
        }
        self.doc.children(node).to_vec()
    }

    fn pseudo(&mut self, s: Arc<Style>, node: NodeId, parent: BoxId) {
        let me = self.counters.enter(&s, ListRole::None);
        let (marker, ms) = if s.display == Display::ListItem { self.list_marker(None, node, &s) } else { (None, None) };
        let pieces = self.content_pieces(&s, node);
        self.counters.leave(me);
        let b = self.tree.push(Some(node), s.clone(), Content::Children, Some(parent));
        self.place_marker(b, node, marker, ms, true);
        let inner = Arc::new(s.inherit(1.0).with_display(Display::Inline));
        for p in pieces {
            match p {
                Piece::Text(t) => {
                    self.tree.push(Some(node), inner.clone(), Content::Text(t), Some(b));
                }
                // (A picture, url(): as an <img> would be, its own size.)
                Piece::Image(u) => {
                    let image = self.pictures.url(&u);
                    self.tree.push(Some(node), inner.clone(), Content::Image { image, alt: String::new(), density: 1.0 }, Some(b));
                }
            }
        }
    }

    fn element(&mut self, node: NodeId, parent: BoxId) {
        let Some(style) = self.style_of(node) else { return };
        let Some(el) = self.doc.element(node) else { return };
        match style.display {
            Display::None => return,
            Display::Contents => {
                let me = self.counters.enter(&style, Counters::role(self.doc, &self.ancestors, Some(node), &style));
                self.children_of(node, parent);
                self.counters.leave(me);
                return;
            }
            _ => {}
        }
        let name = el.name.as_str();
        // What shows something other than its children.
        let content = match name {
            "img" => Content::Image {
                image: self.pictures.picture(self.doc, node),
                alt: String::from(el.attr("alt").unwrap_or("")),
                density: self.pictures.density(node),
            },
            // (An <object> that shows neither a document nor a picture shows its fallback, its children: one with
            // nothing to show at all is an empty box.)
            "object" if crate::page::embedded(el) == crate::page::Embedded::Neither && el.attr("data").is_some_and(|d| !d.trim().is_empty()) => Content::Children,
            "object" | "embed" if crate::page::embedded(el) == crate::page::Embedded::Picture => Content::Image {
                image: self.pictures.picture(self.doc, node),
                alt: String::new(),
                density: 1.0,
            },
            "svg" | "video" | "canvas" | "iframe" | "embed" | "object" | "audio" | "math" => {
                // (Their insides aren't drawn here: their place is kept, from their size; an <svg> is a picture, drawn
                // from its own text, and a <canvas> one its scripts draw.)
                let image = match name {
                    "video" => el.attr("poster").and(self.pictures.picture(self.doc, node)),
                    "svg" | "canvas" => self.pictures.picture(self.doc, node),
                    _ => None,
                };
                Content::Image { image, alt: String::new(), density: 1.0 }
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
                    "image" => Content::Image {
                        image: self.pictures.picture(self.doc, node),
                        alt: String::from(el.attr("alt").unwrap_or("")),
                        density: self.pictures.density(node),
                    },
                    "file" => Content::Control(Control::Button(String::from("Choose a file"))),
                    "range" | "color" => Content::Control(Control::Text { value: String::new(), placeholder: String::new(), password: false, size: 20 }),
                    _ => Content::Control(Control::Text {
                        value,
                        placeholder: String::from(el.attr("placeholder").unwrap_or("")),
                        password: kind == "password",
                        size: el.attr("size").and_then(|n| n.trim().parse::<u16>().ok()).filter(|n| (1..=1000).contains(n)).unwrap_or(20),
                    }),
                }
            }
            "textarea" => {
                let number = |name: &str, default: u16| el.attr(name).and_then(|n| n.trim().parse::<u16>().ok()).filter(|n| (1..=1000).contains(n)).unwrap_or(default);
                Content::Control(Control::Area {
                    value: el.attr("value").map(String::from).unwrap_or_else(|| self.doc.text_of(node)),
                    placeholder: String::from(el.attr("placeholder").unwrap_or("")),
                    cols: number("cols", 20),
                    rows: number("rows", 2),
                })
            }
            "select" => {
                // The option chosen: the one marked selected, else the first.
                let options: Vec<NodeId> = self.doc.descendants(node).filter(|&d| self.doc.name(d) == "option").collect();
                let chosen = options.iter().copied().find(|&o| self.doc.element(o).is_some_and(|e| e.has_attr("selected"))).or(options.first().copied());
                let words = chosen.map(|o| String::from(self.doc.text_of(o).trim())).unwrap_or_default();
                let widest = options
                    .iter()
                    .map(|&o| String::from(self.doc.text_of(o).trim()))
                    .max_by_key(|w| w.chars().count())
                    .unwrap_or_default();
                Content::Control(Control::Select(words, widest))
            }
            _ => Content::Children,
        };
        let walk = matches!(content, Content::Children);
        // A table cell's colspan and rowspan, kept for the layout in its style's grid_column and grid_row spans (a
        // rowspan of 0: to the end of its row group, kept as u32::MAX).
        let span = |a: &str| el.attr(a).and_then(|c| counters::html_integer(c)).filter(|&n| n >= 0);
        let style = match (name, span("colspan"), span("rowspan")) {
            // (A <col>'s or <colgroup>'s span: the columns it's for.)
            ("col" | "colgroup", _, _) if span("span").is_some_and(|c| c > 1) => {
                let mut s = (*style).clone();
                s.grid_column.span = (span("span").unwrap_or(1) as u32).min(1000);
                Arc::new(s)
            }
            ("td" | "th", cols, rows) if cols.is_some_and(|c| c > 1) || rows.is_some_and(|r| r != 1) => {
                let mut s = (*style).clone();
                if let Some(c) = cols.filter(|&c| c > 1) {
                    s.grid_column.span = (c as u32).min(1000);
                }
                if let Some(r) = rows.filter(|&r| r != 1) {
                    s.grid_row.span = if r == 0 { u32::MAX } else { (r as u32).min(65534) };
                }
                Arc::new(s)
            }
            _ => style,
        };
        // Its counters, and (a list item) its number and marker.
        let me = self.counters.enter(&style, Counters::role(self.doc, &self.ancestors, Some(node), &style));
        let (marker, ms) = if style.display == Display::ListItem { self.list_marker(Some(node), node, &style) } else { (None, None) };
        let b = self.tree.push(Some(node), style, content, Some(parent));
        self.place_marker(b, node, marker, ms, walk);
        self.tree.boxes[b].centered = name == "button";
        let frame = name == "iframe" || (matches!(name, "object" | "embed") && crate::page::embedded(el) == crate::page::Embedded::Document);
        self.tree.boxes[b].frame = frame;
        if frame || matches!(name, "video" | "object") {
            self.tree.boxes[b].default_size = (300.0, 150.0);
        }
        if walk && !matches!(name, "select" | "textarea" | "template") {
            self.children_of(node, b);
            if let Some(fl) = self.styles.first_letter.get(&node).cloned() {
                self.first_letter(b, fl);
            }
        }
        self.counters.leave(me);
    }

    /// A block's ::first-letter: the first letter of its first line (in the boxes in it, in flow, ::before's too),
    /// with the punctuation round it, made a box of its own in the first letter's style (inline, or floated). None if
    /// its first line starts with something else (a picture, an inline-block, a line break).
    fn first_letter(&mut self, root: BoxId, style: Arc<Style>) {
        let mut stack: Vec<BoxId> = self.tree.boxes[root].children.iter().rev().copied().collect();
        while let Some(k) = stack.pop() {
            let bx = &self.tree.boxes[k];
            // (An inside marker isn't the first letter's: the letter after it is.)
            if bx.style.float != Float::None || bx.style.out_of_flow() || bx.marker_box.is_some() {
                continue;
            }
            match &bx.content {
                Content::Text(t) => {
                    let keep = matches!(bx.style.white_space, WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine | WhiteSpace::BreakSpaces);
                    if !keep && t.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C')) {
                        continue;
                    }
                    let n = crate::first_letter::length(t, keep);
                    if n > 0 {
                        let (head, tail) = (String::from(&t[..n]), String::from(&t[n..]));
                        self.split_first_letter(k, head, tail, style);
                    }
                    return;
                }
                Content::Children if bx.style.display.is_inline_level() && bx.style.display != Display::Inline => return,
                // (A block in it: its first line is the first, if it has one; what's after it isn't on it.)
                Content::Children if bx.style.display != Display::Inline => {
                    stack.clear();
                    stack.extend(bx.children.iter().rev().copied());
                }
                Content::Children => stack.extend(bx.children.iter().rev().copied()),
                Content::Break | Content::Image { .. } | Content::Control(_) => return,
            }
        }
    }

    /// The text box `k` made two: its first letter (`head`) in a box of the first letter's style, then the rest.
    fn split_first_letter(&mut self, k: BoxId, head: String, tail: String, style: Arc<Style>) {
        let Some(parent) = self.tree.boxes[k].parent else { return };
        let node = self.tree.boxes[k].node;
        let letter = self.tree.push(node, style.clone(), Content::Children, None);
        self.tree.push(node, Arc::new(style.inherit(1.0).with_display(Display::Inline)), Content::Text(head), Some(letter));
        let at = self.tree.boxes[parent].children.iter().position(|&c| c == k).unwrap_or(0);
        self.tree.boxes[letter].parent = Some(parent);
        self.tree.boxes[parent].children.insert(at, letter);
        if tail.is_empty() {
            self.tree.boxes[parent].children.retain(|&c| c != k);
        } else {
            self.tree.boxes[k].content = Content::Text(tail);
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
        matches!(self.white_space, WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces)
    }
}
