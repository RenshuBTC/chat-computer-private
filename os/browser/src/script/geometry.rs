//! Where the page's boxes are, for its scripts (getBoundingClientRect, offsetWidth, getComputedStyle...): the
//! page's own layout while it's current; once a script has changed the page, a layout made on the spot (as a
//! browser does when a script asks where something is after changing it), from what the page gave for it: its
//! style sheets, parsed, and its pictures' sizes.

use super::{Vm, host};
use crate::boxes::{Pictures, Tree};
use crate::css::{Sheet, Viewport};
use crate::dom::{Document, NodeId};
use crate::layout::{Laid, Layout, Rect, Sizes};
use crate::style::{self, Sheets};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// The most layouts one task (a script, an event's listeners, a timer) may have made on the spot: past that, what
/// it reads is the last one's (a script that changes the page and asks where things are, over and over, isn't
/// stopped for long).
const MOST_FORCED: u32 = 40;

/// What laying the page out takes besides its tree.
pub struct LayoutInputs {
    pub ua: Arc<Sheet>,
    pub author: Arc<Vec<Sheet>>,
    pub fonts: Arc<Vec<crate::style::WebFont>>,
    /// Which element shows which picture (by the page's resource), and each resource's size, if it's a picture
    /// that has come.
    pub picture_of: Vec<(NodeId, usize)>,
    /// The densities of the pictures shown (from srcsets) that aren't 1.
    pub densities: Vec<(NodeId, f32)>,
    pub sizes: Vec<Option<(u32, u32)>>,
    /// Each picture's own size as CSS sizes it, where it isn't simply its pixels' (an SVG's).
    pub own: Vec<Option<picture::Intrinsic>>,
}

/// A page's pictures by element, for the box tree.
pub struct Picker<'a> {
    pub of: &'a [(NodeId, usize)],
    pub densities: &'a [(NodeId, f32)],
}

impl Pictures for Picker<'_> {
    fn picture(&mut self, _doc: &Document, node: NodeId) -> Option<usize> {
        self.of.iter().find(|(n, _)| *n == node).map(|(_, r)| *r)
    }

    fn density(&self, node: NodeId) -> f32 {
        self.densities.iter().find(|(n, _)| *n == node).map_or(1.0, |(_, d)| *d)
    }
}

struct SizesOf<'a>(&'a [Option<(u32, u32)>], &'a [Option<picture::Intrinsic>]);

impl Sizes for SizesOf<'_> {
    fn size(&self, n: usize) -> Option<(u32, u32)> {
        self.0.get(n).copied().flatten()
    }

    fn intrinsic(&self, n: usize) -> Option<picture::Intrinsic> {
        let (w, h) = self.size(n)?;
        Some(self.1.get(n).copied().flatten().unwrap_or(picture::Intrinsic::sized(w, h)))
    }
}

/// Each node's box from a layout (in device pixels: the union of its pieces; an element with no box of its own,
/// as big as what's inside it), in CSS pixels.
pub fn rects_of(laid: &Laid, doc: &Document, w: f32, dpr: f32) -> Vec<Option<(f32, f32, f32, f32)>> {
    let n = doc.nodes.len();
    let mut rects: Vec<Option<(f32, f32, f32, f32)>> = alloc::vec![None; n];
    let grow = |rects: &mut Vec<Option<(f32, f32, f32, f32)>>, node: usize, r: Rect| {
        if node >= rects.len() || (r.w <= 0.0 && r.h <= 0.0) {
            return;
        }
        rects[node] = Some(match rects[node] {
            None => (r.x, r.y, r.w, r.h),
            Some((x, y, rw, rh)) => {
                let (x0, y0) = (x.min(r.x), y.min(r.y));
                (x0, y0, (x + rw).max(r.x + r.w) - x0, (y + rh).max(r.y + r.h) - y0)
            }
        });
    };
    for (node, r) in &laid.boxes {
        grow(&mut rects, *node, *r);
    }
    for i in &laid.items {
        if let Some(node) = i.node {
            grow(&mut rects, node, crate::page::bounds(i));
        }
    }
    let order: Vec<NodeId> = doc.descendants(0).collect();
    for &node in order.iter().rev() {
        if let (Some(r), Some(p)) = (rects[node], doc.parent(node))
            && p < n
            && rects[p].is_none_or(|_| doc.element(p).is_none())
        {
            let (x, y, w2, h2) = r;
            grow(&mut rects, p, Rect::new(x, y, w2, h2));
        }
    }
    if n > 0 {
        rects[0] = Some((0.0, 0.0, w, laid.height));
    }
    let scale = if dpr > 0.0 { dpr } else { 1.0 };
    rects.into_iter().map(|r| r.map(|(x, y, w2, h2)| (x / scale, y / scale, w2 / scale, h2 / scale))).collect()
}

/// Makes what scripts read of the page's boxes current: laid out again on the spot if the tree changed since the
/// last layout (and the page gave what it takes).
pub fn fresh_layout(vm: &mut Vm) {
    let h = host(vm);
    if h.laid_version == h.doc.version || h.forced_layouts >= MOST_FORCED {
        return;
    }
    let Some(inputs) = h.layout_inputs.clone() else { return };
    h.forced_layouts += 1;
    let (w, ht, dpr) = h.view;
    let dpr = if dpr > 0.0 { dpr } else { 1.0 };
    let view = Viewport { width: w, height: ht };
    let styles = style::compute(&h.doc, &Sheets { ua: &inputs.ua, author: &inputs.author, fonts: &inputs.fonts }, view, dpr);
    let mut tree = Tree::build(&h.doc, &styles, &mut Picker { of: &inputs.picture_of, densities: &inputs.densities });
    let laid = Layout::new(&mut tree, text::Type::shared(), &SizesOf(&inputs.sizes, &inputs.own), w * dpr, ht * dpr, dpr).run();
    h.rects = rects_of(&laid, &h.doc, w * dpr, dpr);
    h.styles = Some(Arc::new(styles));
    h.laid_version = h.doc.version;
}
