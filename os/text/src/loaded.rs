//! Fonts pages load (CSS's @font-face): TrueType, WOFF or WOFF2 (font::woff), each kept for as long as the system
//! runs, and kept once (the same font from another page, or another site, is the one already here), so text can be set
//! and drawn in it as in the built-in fonts. A font with a weight axis gets its bold instance made, as the built-in
//! variable fonts do.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use font::{Font, Instance, Kerning};

/// A loaded font: the font, its bold instance (with every glyph's advance in it) if it's variable, its kerning at its
/// default and at its bold, and its vertical measures in its own units.
pub(crate) struct Loaded {
    pub font: Font<'static>,
    pub hash: u64,
    pub upem: i32,
    pub bold: Option<Instance>,
    pub bold_advances: Vec<u16>,
    pub kerning: Kerning<'static>,
    pub bold_kerning: Kerning<'static>,
}

/// At most this many fonts, and this many bytes of them, kept.
const MOST: usize = 512;
const MOST_BYTES: usize = 192 << 20;

static FONTS: [AtomicPtr<Loaded>; MOST] = [const { AtomicPtr::new(core::ptr::null_mut()) }; MOST];
static COUNT: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

fn hash(d: &[u8]) -> u64 {
    d.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// A font a page loaded (TrueType, WOFF or WOFF2), kept: its number, or None if it's damaged, of a kind not read
/// (CFF outlines), or there's no more room for fonts.
pub fn load(bytes: &[u8]) -> Option<u16> {
    let sfnt = font::woff::sfnt(bytes, 32 << 20)?;
    let h = hash(&sfnt);
    let n = COUNT.load(Ordering::Acquire).min(MOST);
    for i in 0..n {
        if get(i as u16).is_some_and(|f| f.hash == h) {
            return Some(i as u16);
        }
    }
    // (Read first from what's here, so a font that can't be read isn't kept.)
    if Font::parse(&sfnt).is_none() || BYTES.load(Ordering::Acquire) + sfnt.len() > MOST_BYTES {
        return None;
    }
    let i = COUNT.fetch_add(1, Ordering::AcqRel);
    if i >= MOST {
        return None;
    }
    BYTES.fetch_add(sfnt.len(), Ordering::AcqRel);
    // (Kept for good: the font refers into its bytes, and text drawn in it may be drawn again at any time.)
    let data: &'static [u8] = Box::leak(sfnt.into_boxed_slice());
    let font = Font::parse(data)?;
    let upem = font.metrics.units_per_em as i32;
    let bold = font.instance(b"wght", 700);
    let bold_advances = match &bold {
        Some(at) => font.advances_at(at, 0..font.glyph_count()),
        None => Vec::new(),
    };
    let (kerning, bold_kerning) = (font.kerning(None), font.kerning(bold.as_ref()));
    let loaded = Box::new(Loaded { font, hash: h, upem, bold, bold_advances, kerning, bold_kerning });
    FONTS[i].store(Box::into_raw(loaded), Ordering::Release);
    Some(i as u16)
}

/// A loaded font, by its number.
pub(crate) fn get(id: u16) -> Option<&'static Loaded> {
    let p = FONTS.get(id as usize)?.load(Ordering::Acquire);
    // SAFETY: a non-null pointer here is a Loaded leaked by `load`, never freed or changed after it's stored.
    if p.is_null() { None } else { Some(unsafe { &*p }) }
}

/// A loaded font's ascent, descent and line gap at `size` pixels, each rounded to a whole pixel (as Chrome's font
/// metrics are): its hhea's, or its OS/2's typographic ones if it says to use those.
pub fn rounded(id: u16, size: f32) -> Option<(f32, f32, f32)> {
    let f = get(id)?;
    let m = f.font.metrics;
    let px = |units: i32| crate::libm_round(units as f32 * size / f.upem as f32);
    Some((px(m.ascender as i32), px(-(m.descender as i32)), px(m.line_gap as i32)))
}

/// A loaded font's ex, ch, cap and ic as parts of its em: its OS/2's x-height and cap height (where it gives them),
/// its "0"'s width, and its 水's (the CJK water ideograph's).
pub fn units(id: u16) -> Option<[Option<f32>; 4]> {
    let f = get(id)?;
    let m = f.font.metrics;
    let em = f.upem.max(1) as f32;
    let part = |v: i16| (v > 0).then_some(v as f32 / em);
    let advance = |c: char| {
        let g = f.font.glyph_index(c);
        (g != 0).then(|| f.font.advance(g) as f32 / em)
    };
    Some([part(m.x_height), advance('0'), part(m.cap_height), advance('\u{6C34}')])
}

/// A line's height in `normal` line height at `size` pixels in a loaded font, as Chrome works it out.
pub fn normal_line(id: u16, size: f32) -> Option<f32> {
    let (a, d, g) = rounded(id, size)?;
    Some(a + d + g.max(0.0))
}
