//! Marks: accents and the like, drawn on a letter or on another mark, where the font's GPOS table puts them in its
//! 'mark' and 'mkmk' features: mark-to-base attachment (a letter's anchor for each class of marks, and each mark's
//! own anchor, put on it) and mark-to-mark attachment (a mark's anchors for the marks stacked on it). In a variable
//! font an anchor moves with the instance, by deltas in GDEF's item variation store.
//!
//! As HarfBuzz applies them: each mark first goes on the letter, by the first of the mark-to-base subtables that has
//! both; then each mark-to-mark lookup in turn puts it on the mark before it, if that's one the lookup attaches it to
//! (the lookup's mark filtering set, if it has one, says which marks before it are looked past).

use crate::kern::{class_of, coverage_index};
use crate::store::Store;
use crate::{i16_at, u16_at, u32_at};
use alloc::vec::Vec;

const MARK_TO_BASE: u16 = 4;
const MARK_TO_MARK: u16 = 6;
const EXTENSION: u16 = 9;
/// GDEF's glyph class for marks.
const MARK: u16 = 3;
/// A lookup flag: its mark filtering set follows its subtables' offsets.
const USE_MARK_FILTERING_SET: u16 = 0x0010;
/// A device table that is really a variation index.
const VARIATION_INDEX: u16 = 0x8000;

/// A mark-to-mark lookup: which marks before a mark it looks at (a mark filtering set, if any), and its subtables.
struct OnMarks {
    filter: Option<u16>,
    subtables: Vec<usize>,
}

/// A font's mark positioning, at one instance (or the font's default).
pub struct Marks<'a> {
    gpos: &'a [u8],
    /// The mark-to-base subtables, in the order they apply, and the mark-to-mark lookups.
    on_base: Vec<usize>,
    on_marks: Vec<OnMarks>,
    /// GDEF, where its glyph classes and its mark glyph sets are (0: none).
    gdef: &'a [u8],
    classes: usize,
    sets: usize,
    store: Option<Store<'a>>,
}

impl<'a> Marks<'a> {
    /// No mark positioning (a font without it).
    pub fn none() -> Marks<'a> {
        Marks { gpos: &[], on_base: Vec::new(), on_marks: Vec::new(), gdef: &[], classes: 0, sets: 0, store: None }
    }

    /// The mark positioning in `gpos` (and `gdef`), at `coords` (an instance's normalized coordinates, 2.14; empty:
    /// the default).
    pub(crate) fn new(gpos: Option<&'a [u8]>, gdef: Option<&'a [u8]>, coords: &[i32]) -> Marks<'a> {
        let (Some(gpos), gdef) = (gpos, gdef.unwrap_or(&[])) else { return Marks::none() };
        let Some((on_base, on_marks)) = mark_lookups(gpos) else { return Marks::none() };
        let classes = u16_at(gdef, 4).map_or(0, |o| o as usize);
        // The mark glyph sets are in GDEF 1.2 and later, the item variation store in 1.3.
        let minor = match u16_at(gdef, 0) {
            Some(1) => u16_at(gdef, 2).unwrap_or(0),
            _ => 0,
        };
        let sets = if minor >= 2 { u16_at(gdef, 12).map_or(0, |o| o as usize) } else { 0 };
        let store = if minor >= 3 { u32_at(gdef, 14).map_or(0, |o| o as usize) } else { 0 };
        let store = Store::new(gdef, store, coords);
        Marks { gpos, on_base, on_marks, gdef, classes, sets, store }
    }

    /// Whether the font places marks at all (it has a 'mark' feature).
    pub fn is_empty(&self) -> bool {
        self.on_base.is_empty()
    }

    /// Whether the font says the glyph is a mark (GDEF's glyph classes).
    pub fn is_mark(&self, glyph: u16) -> bool {
        self.classes != 0 && class_of(self.gdef, self.classes, glyph) == MARK
    }

    /// Where `mark` goes on the letter `base`: its origin's offset from the letter's, in font units (y up), if the
    /// font has anchors for the two.
    pub fn on_base(&self, base: u16, mark: u16) -> Option<(i32, i32)> {
        self.on_base.iter().find_map(|&st| self.attach(st, mark, base))
    }

    /// Where `mark` goes on a mark before it (`before`: the cluster's marks before it, nearest last, each with its
    /// origin): its origin, if a mark-to-mark lookup puts it on one of them (the last lookup that does: each applies
    /// after the one before).
    pub fn on_mark(&self, before: &[(u16, (i32, i32))], mark: u16) -> Option<(i32, i32)> {
        let mut at = None;
        for lookup in &self.on_marks {
            // The nearest mark before it that the lookup doesn't look past.
            let Some(&(under, origin)) = before.iter().rev().find(|(g, _)| match lookup.filter {
                Some(set) => self.in_set(set, *g),
                None => true,
            }) else {
                continue;
            };
            if let Some((dx, dy)) = lookup.subtables.iter().find_map(|&st| self.attach(st, mark, under)) {
                at = Some((origin.0 + dx, origin.1 + dy));
            }
        }
        at
    }

    /// Whether the glyph is in GDEF's mark glyph set `set`.
    fn in_set(&self, set: u16, glyph: u16) -> bool {
        let g = self.gdef;
        let in_it = || -> Option<bool> {
            let sets = self.sets;
            if sets == 0 || set >= u16_at(g, sets + 2)? {
                return Some(false);
            }
            let coverage = sets + u32_at(g, sets + 4 + 4 * set as usize)? as usize;
            Some(coverage_index(g, coverage, glyph).is_some())
        };
        in_it().unwrap_or(false)
    }

    /// A mark-to-base or mark-to-mark subtable's place for `mark` on `under` (a letter, or a mark): the mark's
    /// origin's offset from `under`'s, if the subtable has both.
    fn attach(&self, st: usize, mark: u16, under: u16) -> Option<(i32, i32)> {
        let g = self.gpos;
        if u16_at(g, st)? != 1 {
            return None;
        }
        let index = coverage_index(g, st + u16_at(g, st + 2)? as usize, mark)?;
        let other = coverage_index(g, st + u16_at(g, st + 4)? as usize, under)?;
        let class_count = u16_at(g, st + 6)? as usize;
        let (marks, unders) = (st + u16_at(g, st + 8)? as usize, st + u16_at(g, st + 10)? as usize);
        // The mark's record: its class, and its anchor (from the mark array).
        if index >= u16_at(g, marks)? as usize {
            return None;
        }
        let record = marks + 2 + 4 * index;
        let class = u16_at(g, record)? as usize;
        let mark_anchor = marks + u16_at(g, record + 2)? as usize;
        // The letter's (or mark's) anchor for that class (from its array; none: it has no place for such a mark).
        if other >= u16_at(g, unders)? as usize || class >= class_count {
            return None;
        }
        let offset = u16_at(g, unders + 2 + 2 * (other * class_count + class))? as usize;
        if offset == 0 {
            return None;
        }
        let (bx, by) = self.anchor(unders + offset)?;
        let (mx, my) = self.anchor(mark_anchor)?;
        Some((bx - mx, by - my))
    }

    /// An anchor's point (formats 1, 2 and 3; format 3's device tables, where they're variation indexes, move it at
    /// this instance, rounded as HarfBuzz rounds).
    fn anchor(&self, at: usize) -> Option<(i32, i32)> {
        let g = self.gpos;
        let format = u16_at(g, at)?;
        let (mut x, mut y) = (i16_at(g, at + 2)? as i32, i16_at(g, at + 4)? as i32);
        if format == 3
            && let Some(store) = &self.store
        {
            for (field, value) in [(6, &mut x), (8, &mut y)] {
                let device = u16_at(g, at + field)? as usize;
                if device != 0
                    && let (Some(outer), Some(inner), Some(VARIATION_INDEX)) =
                        (u16_at(g, at + device), u16_at(g, at + device + 2), u16_at(g, at + device + 4))
                {
                    *value += store.rounded(outer, inner).unwrap_or(0);
                }
            }
        }
        Some((x, y))
    }
}

/// The 'mark' feature's mark-to-base subtables and the 'mkmk' feature's mark-to-mark lookups, for Latin text (the
/// Latin script's default language system, else the default script's), each in the order its lookups apply.
fn mark_lookups(g: &[u8]) -> Option<(Vec<usize>, Vec<OnMarks>)> {
    if u16_at(g, 0)? != 1 {
        return None;
    }
    let (scripts, features, lookups) = (u16_at(g, 4)? as usize, u16_at(g, 6)? as usize, u16_at(g, 8)? as usize);
    let mut langsys = None;
    for wanted in [b"latn", b"DFLT"] {
        for i in 0..u16_at(g, scripts)? as usize {
            let record = scripts + 2 + 6 * i;
            if g.get(record..record + 4)? == wanted {
                let script = scripts + u16_at(g, record + 4)? as usize;
                let default = u16_at(g, script)? as usize;
                if default != 0 {
                    langsys = Some(script + default);
                }
            }
        }
        if langsys.is_some() {
            break;
        }
    }
    let langsys = langsys?;
    let feature_count = u16_at(g, features)?;
    let (mut mark, mut mkmk): (Vec<u16>, Vec<u16>) = (Vec::new(), Vec::new());
    for k in 0..u16_at(g, langsys + 4)? as usize {
        let f = u16_at(g, langsys + 6 + 2 * k)?;
        if f >= feature_count {
            continue;
        }
        let record = features + 2 + 6 * f as usize;
        let into = match g.get(record..record + 4)? {
            b"mark" => &mut mark,
            b"mkmk" => &mut mkmk,
            _ => continue,
        };
        let feature = features + u16_at(g, record + 4)? as usize;
        for j in 0..u16_at(g, feature + 2)? as usize {
            into.push(u16_at(g, feature + 4 + 2 * j)?);
        }
    }
    let total = u16_at(g, lookups)?;
    // A lookup's subtables of one type, and its mark filtering set.
    let lookup = |index: u16, wanted: u16| -> Option<(Vec<usize>, Option<u16>)> {
        let at = lookups + u16_at(g, lookups + 2 + 2 * index as usize)? as usize;
        let (kind, flag, count) = (u16_at(g, at)?, u16_at(g, at + 2)?, u16_at(g, at + 4)? as usize);
        let mut tables = Vec::new();
        for s in 0..count {
            let mut st = at + u16_at(g, at + 6 + 2 * s)? as usize;
            let mut k = kind;
            if k == EXTENSION {
                // Format 1: the lookup's real type, then a 32-bit offset to the subtable.
                k = u16_at(g, st + 2)?;
                st += u32_at(g, st + 4)? as usize;
            }
            if k == wanted {
                tables.push(st);
            }
        }
        let filter = if flag & USE_MARK_FILTERING_SET != 0 { Some(u16_at(g, at + 6 + 2 * count)?) } else { None };
        Some((tables, filter))
    };
    let mut on_base = Vec::new();
    mark.sort_unstable();
    mark.dedup();
    for i in mark.into_iter().filter(|&i| i < total) {
        on_base.extend(lookup(i, MARK_TO_BASE)?.0);
    }
    let mut on_marks = Vec::new();
    mkmk.sort_unstable();
    mkmk.dedup();
    for i in mkmk.into_iter().filter(|&i| i < total) {
        let (subtables, filter) = lookup(i, MARK_TO_MARK)?;
        if !subtables.is_empty() {
            on_marks.push(OnMarks { filter, subtables });
        }
    }
    Some((on_base, on_marks))
}
