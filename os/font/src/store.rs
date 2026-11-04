//! Item variation stores: the deltas a variable font keeps for values that aren't points of an outline, each picked by
//! an (outer, inner) pair of indexes: the advances (HVAR), and the kerning (GDEF's, for GPOS's device tables). A
//! delta set gives one delta for each of its regions; at an instance, each region counts as much as its scalar says
//! (how far the instance is into it), and the value moves by the sum.

use crate::{i16_at, u16_at, u32_at};
use alloc::vec::Vec;

/// Scalars' fixed point: 32 fractional bits.
pub(crate) const ONE: i128 = 1 << 32;

/// An item variation store at one instance: where it is (`at`, in `data`), and each of its regions' scalar there.
pub(crate) struct Store<'a> {
    data: &'a [u8],
    at: usize,
    scalars: Vec<i128>,
}

impl<'a> Store<'a> {
    /// The store at `at` in `data`, at `coords` (an instance's normalized coordinates, 2.14). None where nothing
    /// varies (the default instance) or the store can't be read.
    pub(crate) fn new(data: &'a [u8], at: usize, coords: &[i32]) -> Option<Store<'a>> {
        if at == 0 || coords.iter().all(|&c| c == 0) || u16_at(data, at)? != 1 {
            return None;
        }
        let scalars = region_scalars(data, at, coords)?;
        Some(Store { data, at, scalars })
    }

    /// Item (`outer`, `inner`)'s delta at this instance (32 fractional bits).
    pub(crate) fn delta(&self, outer: u16, inner: u16) -> Option<i128> {
        let (d, store) = (self.data, self.at);
        if outer >= u16_at(d, store + 6)? {
            return None;
        }
        let data = store + u32_at(d, store + 8 + 4 * outer as usize)? as usize;
        if inner >= u16_at(d, data)? {
            return None;
        }
        let words_field = u16_at(d, data + 2)?;
        let (long, words) = (words_field & 0x8000 != 0, (words_field & 0x7FFF) as usize);
        let regions = u16_at(d, data + 4)? as usize;
        let (big, small) = if long { (4, 2) } else { (2, 1) };
        let row = data + 6 + 2 * regions + inner as usize * (words * big + regions.saturating_sub(words) * small);
        let mut sum: i128 = 0;
        let mut at = row;
        for r in 0..regions {
            let size = if r < words { big } else { small };
            let delta = match size {
                4 => u32_at(d, at)? as i32 as i128,
                2 => i16_at(d, at)? as i128,
                _ => *d.get(at)? as i8 as i128,
            };
            at += size;
            let region = u16_at(d, data + 6 + 2 * r)? as usize;
            sum += delta * self.scalars.get(region).copied().unwrap_or(0);
        }
        Some(sum)
    }

    /// Item (`outer`, `inner`)'s delta at this instance, rounded to a whole unit as HarfBuzz rounds it (halves away
    /// from zero).
    pub(crate) fn rounded(&self, outer: u16, inner: u16) -> Option<i32> {
        let d = self.delta(outer, inner)?;
        let half = ONE / 2;
        Some((if d >= 0 { (d + half) / ONE } else { -((-d + half) / ONE) }) as i32)
    }
}

/// Each region's scalar at `coords` in the item variation store at `store`: how much of its deltas apply (32
/// fractional bits), as OpenType's interpolation algorithm gives it.
fn region_scalars(d: &[u8], store: usize, coords: &[i32]) -> Option<Vec<i128>> {
    let list = store + u32_at(d, store + 2)? as usize;
    let (axes, count) = (u16_at(d, list)? as usize, u16_at(d, list + 2)? as usize);
    let mut scalars = Vec::with_capacity(count);
    for r in 0..count {
        let mut s = ONE;
        for a in 0..axes {
            let at = list + 4 + (r * axes + a) * 6;
            let (start, peak, end) = (i16_at(d, at)? as i128, i16_at(d, at + 2)? as i128, i16_at(d, at + 4)? as i128);
            let v = coords.get(a).copied().unwrap_or(0) as i128;
            // An axis the region doesn't need, a region that makes no sense, or one across the default: no effect.
            if peak == 0 || v == peak || start > peak || peak > end || (start < 0 && end > 0) {
                continue;
            }
            if v <= start || v >= end {
                s = 0;
                break;
            }
            s = if v < peak { s * (v - start) / (peak - start) } else { s * (end - v) / (end - peak) };
        }
        scalars.push(s);
    }
    Some(scalars)
}

/// The (outer, inner) pair a delta-set index map gives item `i` (a glyph): its entry, or its last one for items past
/// its end (HVAR's advance mapping). Without a map, the item is inner index `i` in the first delta set.
pub(crate) fn mapped(d: &[u8], map: usize, i: u32) -> Option<(u16, u16)> {
    if map == 0 {
        return Some((0, u16::try_from(i).ok()?));
    }
    let (format, entry) = (*d.get(map)?, *d.get(map + 1)?);
    let (count, data) = match format {
        0 => (u16_at(d, map + 2)? as u32, map + 4),
        1 => (u32_at(d, map + 2)?, map + 6),
        _ => return None,
    };
    if count == 0 {
        return None;
    }
    let (size, inner_bits) = (((entry & 0x30) >> 4) as usize + 1, (entry & 0x0F) as u32 + 1);
    let at = data + i.min(count - 1) as usize * size;
    let value = d.get(at..at + size)?.iter().fold(0u32, |v, &b| (v << 8) | b as u32);
    Some(((value >> inner_bits) as u16, (value & ((1 << inner_bits) - 1)) as u16))
}
