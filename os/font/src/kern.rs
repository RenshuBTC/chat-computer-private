//! Kerning: the space between particular pairs of letters made tighter or looser ("To", "AV", "y."), as the font's
//! GPOS table gives it in its 'kern' feature: pair adjustment lookups, by pairs of glyphs or by classes of glyphs. In a
//! variable font a pair's adjustment varies with the instance, by deltas in GDEF's item variation store.
//!
//! Lookups apply in the order they're listed, each adding its adjustment; within one, the first of its subtables that
//! has the pair gives it (one by classes has every pair whose first glyph it covers). Marks (accents drawn over
//! letters) are left out, as the font's lookups say. What HarfBuzz, and so browsers, do for two letters side by side.

use crate::store::Store;
use crate::{i16_at, u16_at, u32_at};
use alloc::vec::Vec;

const PAIR_ADJUSTMENT: u16 = 2;
const EXTENSION: u16 = 9;
/// GDEF's glyph class for marks.
const MARK: u16 = 3;
/// ValueRecord fields: each present one takes 2 bytes, in this order.
const X_ADVANCE: u16 = 0x0004;
const X_ADVANCE_DEVICE: u16 = 0x0040;
/// A device table that is really a variation index.
const VARIATION_INDEX: u16 = 0x8000;

/// A font's kerning, at one instance (or the font's default).
pub struct Kerning<'a> {
    gpos: &'a [u8],
    /// The 'kern' feature's pair adjustment lookups, in the order they apply: each one's subtables (where they are
    /// in GPOS).
    lookups: Vec<Vec<usize>>,
    /// GDEF, and where its glyph classes are (0: none).
    gdef: &'a [u8],
    classes: usize,
    /// GDEF's item variation store at this instance (none at the default, where nothing varies).
    store: Option<Store<'a>>,
}

impl<'a> Kerning<'a> {
    /// No kerning (a font without it).
    pub fn none() -> Kerning<'a> {
        Kerning { gpos: &[], lookups: Vec::new(), gdef: &[], classes: 0, store: None }
    }

    /// The kerning in `gpos` (and `gdef`), at `coords` (an instance's normalized coordinates, 2.14; empty: the
    /// default).
    pub(crate) fn new(gpos: Option<&'a [u8]>, gdef: Option<&'a [u8]>, coords: &[i32]) -> Kerning<'a> {
        let (Some(gpos), gdef) = (gpos, gdef.unwrap_or(&[])) else { return Kerning::none() };
        let lookups = kern_lookups(gpos).unwrap_or_default();
        let classes = u16_at(gdef, 4).map_or(0, |o| o as usize);
        // The item variation store's offset is in GDEF 1.3 and later.
        let store = match (u16_at(gdef, 0), u16_at(gdef, 2)) {
            (Some(1), Some(minor)) if minor >= 3 => u32_at(gdef, 14).map_or(0, |o| o as usize),
            _ => 0,
        };
        let store = Store::new(gdef, store, coords);
        Kerning { gpos, lookups, gdef, classes, store }
    }

    /// Whether the font kerns at all.
    pub fn is_empty(&self) -> bool {
        self.lookups.is_empty()
    }

    /// The adjustment to the space between glyphs `left` and `right` (the advance of `left` changes by it), in font
    /// units.
    pub fn pair(&self, left: u16, right: u16) -> i32 {
        if self.lookups.is_empty() || self.mark(left) || self.mark(right) {
            return 0;
        }
        let mut total = 0;
        for lookup in &self.lookups {
            if let Some(v) = lookup.iter().find_map(|&st| self.in_subtable(st, left, right)) {
                total += v;
            }
        }
        total
    }

    fn mark(&self, glyph: u16) -> bool {
        self.classes != 0 && class_of(self.gdef, self.classes, glyph) == MARK
    }

    /// The pair's adjustment in one pair adjustment subtable, if it has the pair.
    fn in_subtable(&self, st: usize, left: u16, right: u16) -> Option<i32> {
        let g = self.gpos;
        let format = u16_at(g, st)?;
        let index = coverage_index(g, st + u16_at(g, st + 2)? as usize, left)?;
        let (first, second) = (u16_at(g, st + 4)?, u16_at(g, st + 6)?);
        let (size1, size2) = (value_size(first), value_size(second));
        match format {
            // Pairs of glyphs: the first glyph's set of pairs, sorted by the second glyph.
            1 => {
                if index >= u16_at(g, st + 8)? as usize {
                    return None;
                }
                let set = st + u16_at(g, st + 10 + 2 * index)? as usize;
                let record = 2 + size1 + size2;
                let (mut lo, mut hi) = (0, u16_at(g, set)? as usize);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let at = set + 2 + mid * record;
                    let glyph = u16_at(g, at)?;
                    if glyph == right {
                        // (A pair's device table is relative to its pair set.)
                        return Some(self.x_advance(set, at + 2, first));
                    }
                    if glyph < right {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                None
            }
            // Classes of glyphs: a row for each class of first glyphs, a column for each class of second glyphs.
            2 => {
                let (class1, class2) = (u16_at(g, st + 8)? as usize, u16_at(g, st + 10)? as usize);
                let (count1, count2) = (u16_at(g, st + 12)?, u16_at(g, st + 14)?);
                let (c1, c2) = (class_of(g, st + class1, left), class_of(g, st + class2, right));
                if c1 >= count1 || c2 >= count2 {
                    return None;
                }
                let at = st + 16 + (c1 as usize * count2 as usize + c2 as usize) * (size1 + size2);
                Some(self.x_advance(st, at, first))
            }
            _ => None,
        }
    }

    /// A value record's change to the advance (its X advance, and its variation's delta at this instance, rounded as
    /// HarfBuzz rounds: halves away from zero). `parent` is where its device table offsets count from.
    fn x_advance(&self, parent: usize, record: usize, format: u16) -> i32 {
        let g = self.gpos;
        let mut value = 0;
        if format & X_ADVANCE != 0 {
            value += i16_at(g, record + field(format, X_ADVANCE)).unwrap_or(0) as i32;
        }
        if format & X_ADVANCE_DEVICE != 0
            && let Some(store) = &self.store
        {
            let device = u16_at(g, record + field(format, X_ADVANCE_DEVICE)).unwrap_or(0) as usize;
            if device != 0
                && let (Some(outer), Some(inner), Some(VARIATION_INDEX)) =
                    (u16_at(g, parent + device), u16_at(g, parent + device + 2), u16_at(g, parent + device + 4))
            {
                value += store.rounded(outer, inner).unwrap_or(0);
            }
        }
        value
    }
}

/// A value record's field's place in it, in bytes.
fn field(format: u16, which: u16) -> usize {
    2 * (format & (which - 1)).count_ones() as usize
}

/// A value record's size, in bytes.
fn value_size(format: u16) -> usize {
    2 * (format & 0xFF).count_ones() as usize
}

/// Where the glyph is in a coverage table, if it is.
pub(crate) fn coverage_index(d: &[u8], at: usize, glyph: u16) -> Option<usize> {
    let count = u16_at(d, at + 2)? as usize;
    match u16_at(d, at)? {
        // A sorted list of glyphs.
        1 => {
            let (mut lo, mut hi) = (0, count);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let g = u16_at(d, at + 4 + 2 * mid)?;
                if g == glyph {
                    return Some(mid);
                }
                if g < glyph {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            None
        }
        // Sorted ranges of glyphs, each with the index of its first.
        2 => {
            let (mut lo, mut hi) = (0, count);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let r = at + 4 + 6 * mid;
                let (start, end) = (u16_at(d, r)?, u16_at(d, r + 2)?);
                if glyph < start {
                    hi = mid;
                } else if glyph > end {
                    lo = mid + 1;
                } else {
                    return Some(u16_at(d, r + 4)? as usize + (glyph - start) as usize);
                }
            }
            None
        }
        _ => None,
    }
}

/// A glyph's class in a class definition table (0 for glyphs it doesn't list).
pub(crate) fn class_of(d: &[u8], at: usize, glyph: u16) -> u16 {
    let class = || -> Option<u16> {
        match u16_at(d, at)? {
            // The classes of a run of glyphs from the first.
            1 => {
                let (start, count) = (u16_at(d, at + 2)?, u16_at(d, at + 4)?);
                let i = glyph.checked_sub(start)?;
                if i >= count {
                    return None;
                }
                u16_at(d, at + 6 + 2 * i as usize)
            }
            // Sorted ranges of glyphs, each with its class.
            2 => {
                let (mut lo, mut hi) = (0, u16_at(d, at + 2)? as usize);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let r = at + 4 + 6 * mid;
                    let (start, end) = (u16_at(d, r)?, u16_at(d, r + 2)?);
                    if glyph < start {
                        hi = mid;
                    } else if glyph > end {
                        lo = mid + 1;
                    } else {
                        return u16_at(d, r + 4);
                    }
                }
                None
            }
            _ => None,
        }
    };
    class().unwrap_or(0)
}

/// The 'kern' feature's pair adjustment lookups for Latin text (the Latin script's default language system, else the
/// default script's), in the order they apply, each as its subtables' places in GPOS.
fn kern_lookups(g: &[u8]) -> Option<Vec<Vec<usize>>> {
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
    let mut indices: Vec<u16> = Vec::new();
    for k in 0..u16_at(g, langsys + 4)? as usize {
        let f = u16_at(g, langsys + 6 + 2 * k)?;
        if f >= feature_count {
            continue;
        }
        let record = features + 2 + 6 * f as usize;
        if g.get(record..record + 4)? != b"kern" {
            continue;
        }
        let feature = features + u16_at(g, record + 4)? as usize;
        for j in 0..u16_at(g, feature + 2)? as usize {
            indices.push(u16_at(g, feature + 4 + 2 * j)?);
        }
    }
    indices.sort_unstable();
    indices.dedup();
    let total = u16_at(g, lookups)?;
    let mut out = Vec::new();
    for index in indices.into_iter().filter(|&i| i < total) {
        let lookup = lookups + u16_at(g, lookups + 2 + 2 * index as usize)? as usize;
        let kind = u16_at(g, lookup)?;
        let mut tables = Vec::new();
        for s in 0..u16_at(g, lookup + 4)? as usize {
            let mut st = lookup + u16_at(g, lookup + 6 + 2 * s)? as usize;
            let mut k = kind;
            if k == EXTENSION {
                // Format 1: the lookup's real type, then a 32-bit offset to the subtable.
                k = u16_at(g, st + 2)?;
                st += u32_at(g, st + 4)? as usize;
            }
            if k == PAIR_ADJUSTMENT {
                tables.push(st);
            }
        }
        if !tables.is_empty() {
            out.push(tables);
        }
    }
    Some(out)
}
