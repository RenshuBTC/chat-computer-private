//! Checks against values read from the same font with fontTools, and exact coverage of simple shapes.

use super::raster::{Pen, Raster, ONE};
use super::*;

fn atkinson() -> Vec<u8> {
    // (Atkinson Hyperlegible Next, Chat Computer's font before Lora, kept here as the engine's test font: testdata/.)
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/AtkinsonHyperlegibleNext.ttf");
    std::fs::read(path).expect("the font file")
}

#[test]
fn reads_the_measures() {
    let data = atkinson();
    let f = Font::parse(&data).unwrap();
    let m = f.metrics;
    assert_eq!((m.units_per_em, m.ascender, m.descender, m.line_gap), (1000, 984, -316, 0));
    assert_eq!((m.x_height, m.cap_height), (496, 668));
}

// (character, glyph number, advance) as fontTools reads them.
const GLYPHS: [(char, u16, u16); 9] = [
    ('A', 1, 626),
    ('a', 112, 525),
    ('g', 143, 558),
    ('o', 175, 548),
    ('0', 365, 648),
    (' ', 231, 280),
    ('é', 134, 538),
    ('€', 292, 664),
    ('→', 0, 0), // not in the font: the "missing" glyph
];

#[test]
fn finds_glyphs_and_widths() {
    let data = atkinson();
    let f = Font::parse(&data).unwrap();
    for (c, glyph, advance) in GLYPHS {
        assert_eq!(f.glyph_index(c), glyph, "glyph for {c:?}");
        if glyph != 0 {
            assert_eq!(f.advance(glyph), advance, "advance of {c:?}");
        }
    }
}

/// The outline's signed area by the shoelace formula, curves cut into many small lines.
#[derive(Default)]
struct Area {
    twice: f64,
    start: (f64, f64),
    at: (f64, f64),
}

impl Area {
    fn to(&mut self, p: (f64, f64)) {
        self.twice += self.at.0 * p.1 - p.0 * self.at.1;
        self.at = p;
    }
}

impl Outline for Area {
    fn move_to(&mut self, x: i32, y: i32) {
        self.start = (x as f64 / 64.0, y as f64 / 64.0);
        self.at = self.start;
    }
    fn line_to(&mut self, x: i32, y: i32) {
        self.to((x as f64 / 64.0, y as f64 / 64.0));
    }
    fn quad_to(&mut self, cx: i32, cy: i32, x: i32, y: i32) {
        let (p0, c, p2) = (self.at, (cx as f64 / 64.0, cy as f64 / 64.0), (x as f64 / 64.0, y as f64 / 64.0));
        for i in 1..=256 {
            let t = i as f64 / 256.0;
            let u = 1.0 - t;
            self.to((u * u * p0.0 + 2.0 * t * u * c.0 + t * t * p2.0, u * u * p0.1 + 2.0 * t * u * c.1 + t * t * p2.1));
        }
    }
    fn close(&mut self) {
        let s = self.start;
        self.to(s);
    }
}

// Signed areas in font units² from fontTools' AreaPen (negative: TrueType draws outer contours clockwise).
const AREAS: [(char, f64); 7] = [
    ('A', -143232.0),
    ('a', -111939.8),
    ('g', -150121.5),
    ('o', -109202.6),
    ('0', -175800.7),
    ('é', -125420.6), // built from 'e' and an accent
    ('€', -143358.5),
];

#[test]
fn outlines_match_fonttools() {
    let data = atkinson();
    let f = Font::parse(&data).unwrap();
    for (c, expected) in AREAS {
        let mut a = Area::default();
        assert!(f.outline(f.glyph_index(c), &mut a));
        let area = a.twice / 2.0;
        assert!((area - expected).abs() < expected.abs() * 0.0005, "{c:?}: area {area}, fontTools {expected}");
    }
}

fn fill(w: usize, h: usize, points: &[(f64, f64)]) -> Vec<u8> {
    let mut r = Raster::new(w, h);
    let fx = |v: f64| (v * ONE as f64).round() as i32;
    for i in 0..points.len() {
        let (a, b) = (points[i], points[(i + 1) % points.len()]);
        r.line(fx(a.0), fx(a.1), fx(b.0), fx(b.1));
    }
    let mut out = vec![0; w * h];
    r.coverage(&mut out);
    out
}

#[test]
fn covers_a_square_exactly() {
    // From (0.5, 0.5) to (2.5, 2.5): corners a quarter covered, sides half, the middle fully.
    let c = fill(4, 4, &[(0.5, 0.5), (2.5, 0.5), (2.5, 2.5), (0.5, 2.5)]);
    #[rustfmt::skip]
    let expected = [
        64, 128, 64, 0,
        128, 255, 128, 0,
        64, 128, 64, 0,
        0, 0, 0, 0,
    ];
    assert_eq!(c, expected);
}

#[test]
fn direction_does_not_matter() {
    let a = fill(5, 5, &[(0.3, 0.7), (4.2, 1.1), (2.6, 4.9)]);
    let b = fill(5, 5, &[(2.6, 4.9), (4.2, 1.1), (0.3, 0.7)]);
    assert_eq!(a, b);
}

#[test]
fn covers_a_triangle_by_area() {
    // Half of a 4×4 square: 8 pixels' worth.
    let c = fill(4, 4, &[(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)]);
    let total: u32 = c.iter().map(|v| *v as u32).sum();
    assert!((total as f64 / 255.0 - 8.0).abs() < 0.02, "total {total}");
    // The diagonal pixels are exactly half covered.
    for i in 0..4 {
        assert_eq!(c[i * 4 + (3 - i)], 128);
    }
}

#[test]
fn clips_shapes_partly_outside() {
    // A square from x = -1 to 2 (left part outside the bitmap), and one entirely right of it.
    let c = fill(4, 2, &[(-1.0, 0.0), (2.0, 0.0), (2.0, 2.0), (-1.0, 2.0)]);
    assert_eq!(c, [255, 255, 0, 0, 255, 255, 0, 0]);
    let d = fill(4, 2, &[(5.0, 0.0), (7.0, 0.0), (7.0, 2.0), (5.0, 2.0)]);
    assert_eq!(d, [0; 8]);
    let e = fill(3, 3, &[(-2.0, -2.0), (5.0, -2.0), (5.0, 5.0), (-2.0, 5.0)]);
    assert_eq!(e, [255; 9]);
}

#[test]
fn leaves_holes_empty() {
    // A 4×4 square with a 2×2 hole drawn the other way round.
    let mut r = Raster::new(4, 4);
    let sq = |r: &mut Raster, pts: [(i32, i32); 4]| {
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            r.line(a.0 * ONE, a.1 * ONE, b.0 * ONE, b.1 * ONE);
        }
    };
    sq(&mut r, [(0, 0), (4, 0), (4, 4), (0, 4)]);
    sq(&mut r, [(1, 1), (1, 3), (3, 3), (3, 1)]);
    let mut c = vec![0; 16];
    r.coverage(&mut c);
    #[rustfmt::skip]
    let expected = [
        255, 255, 255, 255,
        255, 0, 0, 255,
        255, 0, 0, 255,
        255, 255, 255, 255,
    ];
    assert_eq!(c, expected);
}

#[test]
fn glyph_ink_matches_its_area() {
    // At 100 pixels per em, a glyph's ink (in pixels) is its area in font units² / 100.
    let data = atkinson();
    let f = Font::parse(&data).unwrap();
    for (c, expected) in AREAS {
        let mut r = Raster::new(80, 120);
        let mut pen = Pen::new(&mut r, 100 * ONE, f.metrics.units_per_em);
        pen.x = 10 * ONE;
        pen.y = 100 * ONE;
        assert!(f.outline(f.glyph_index(c), &mut pen));
        let mut out = vec![0; 80 * 120];
        r.coverage(&mut out);
        let ink: f64 = out.iter().map(|v| *v as f64 / 255.0).sum();
        let want = expected.abs() / 100.0;
        assert!((ink - want).abs() < want * 0.005, "{c:?}: ink {ink:.1} px, expected {want:.1}");
    }
}

/// An outline's control box (every point, on the curve or off it), in font units × 64.
#[derive(Default)]
struct Bounds {
    min: (i32, i32),
    max: (i32, i32),
    any: bool,
}

impl Bounds {
    fn add(&mut self, x: i32, y: i32) {
        if !self.any {
            (self.min, self.max, self.any) = ((x, y), (x, y), true);
        }
        self.min = (self.min.0.min(x), self.min.1.min(y));
        self.max = (self.max.0.max(x), self.max.1.max(y));
    }
}

impl Outline for Bounds {
    fn move_to(&mut self, x: i32, y: i32) {
        self.add(x, y);
    }
    fn line_to(&mut self, x: i32, y: i32) {
        self.add(x, y);
    }
    fn quad_to(&mut self, cx: i32, cy: i32, x: i32, y: i32) {
        self.add(cx, cy);
        self.add(x, y);
    }
    fn close(&mut self) {}
}

// (weight, character, advance, control box: x min, y min, x max, y max) as fontTools' own reading of the variable
// font has them (TTFont.getGlyphSet(location=...) with a ControlBoundsPen, fontTools 4.65). 250, 300 and 600 fall
// inside the font's regions, 700 is bold, 800 its heaviest; é is a composite, and the variations of %, o, E, ? and Ů
// leave points out (their deltas inferred from their neighbours').
#[rustfmt::skip]
const VARIED: [(i32, char, f64, [f64; 4]); 32] = [
    (250, 'A', 592.485, [10.0, 0.0, 582.48, 668.0]),
    (250, 'g', 540.073, [39.12, -187.32, 466.96, 505.56]),
    (250, '&', 731.779, [62.9, -11.22, 716.44, 681.22]),
    (250, 'é', 525.529, [39.12, -12.0, 490.18, 698.21]),
    (250, '%', 880.029, [39.12, -12.0, 840.91, 680.0]),
    (250, 'ß', 560.956, [81.12, -12.78, 525.06, 680.0]),
    (600, 'A', 662.001, [7.19, 0.0, 654.81, 668.0]),
    (600, 'g', 582.188, [31.5, -198.19, 522.88, 507.94]),
    (600, '€', 673.563, [9.37, -12.0, 640.06, 680.0]),
    (600, '0', 655.875, [51.56, -12.0, 604.31, 680.87]),
    (600, 'R', 638.751, [65.62, 0.0, 621.75, 668.0]),
    (600, 'w', 722.813, [15.0, 0.0, 707.81, 496.0]),
    (700, 'A', 678.0, [5.94, 0.0, 672.06, 668.0]),
    (700, 'g', 592.937, [29.5, -200.94, 538.37, 509.69]),
    (700, '&', 725.313, [44.38, -12.81, 715.56, 700.69]),
    (700, 'é', 563.187, [29.5, -12.0, 545.31, 722.25]),
    (700, '%', 968.562, [29.5, -12.0, 939.06, 680.0]),
    (700, 'w', 740.062, [15.0, 0.0, 725.06, 496.0]),
    (700, 'ß', 621.0, [60.13, -12.0, 597.94, 680.0]),
    (800, 'A', 702.0, [4.0, 0.0, 698.0, 668.0]),
    (800, 'g', 610.0, [28.0, -205.0, 563.0, 512.0]),
    (800, 'é', 576.0, [28.0, -12.0, 561.0, 731.0]),
    (800, 'R', 668.0, [54.0, 0.0, 661.0, 668.0]),
    (800, 'ß', 640.0, [54.0, -13.0, 621.0, 680.0]),
    (300, 'o', 536.823, [38.24, -12.0, 498.59, 505.12]),
    (300, 'E', 553.911, [80.24, 0.0, 514.79, 668.0]),
    (300, '?', 455.411, [33.35, -12.0, 437.09, 681.12]),
    (300, 'Ů', 679.941, [78.24, -12.0, 601.71, 878.5]),
    (700, 'o', 575.625, [29.5, -12.0, 546.12, 508.87]),
    (700, 'E', 597.437, [60.13, 0.0, 562.69, 668.0]),
    (700, '?', 546.625, [20.25, -12.0, 526.62, 680.0]),
    (700, 'Ů', 685.938, [58.94, -12.0, 627.0, 914.81]),
];

#[test]
fn bold_and_light_come_from_the_same_font() {
    let data = atkinson();
    let f = Font::parse(&data).unwrap();
    assert!(f.instance(b"wdth", 100).is_none(), "an axis the font doesn't have");
    for (weight, c, advance, bounds) in VARIED {
        let at = f.instance(b"wght", weight).unwrap();
        let g = f.glyph_index(c);
        let got = f.advance_at(g, &at) as f64;
        assert!((got - advance).abs() <= 0.5 + 1e-9, "{c:?} at {weight}: advance {got}, fontTools {advance}");
        let mut b = Bounds::default();
        assert!(f.outline_at(g, &at, &mut b));
        let got = [b.min.0, b.min.1, b.max.0, b.max.1].map(|v| v as f64 / 64.0);
        for k in 0..4 {
            assert!((got[k] - bounds[k]).abs() <= 0.05, "{c:?} at {weight}: control box {got:?}, fontTools {bounds:?}");
        }
    }
    // The default weight is the font as it is; past the ends, the ends.
    let regular = f.instance(b"wght", 400).unwrap();
    assert_eq!(regular.coords(), &[0]);
    assert_eq!(f.instance(b"wght", 2000).unwrap().coords(), &[16384]);
    assert_eq!(f.instance(b"wght", 0).unwrap().coords(), &[-16384]);
    for (c, _, advance) in GLYPHS.iter().filter(|g| g.1 != 0) {
        assert_eq!(f.advance_at(f.glyph_index(*c), &regular), *advance, "{c:?} at the default weight");
    }
}

/// The own system's fallback fonts (os/fonts/): (character, glyph number, advance, signed area) as fontTools reads
/// them. One merged from several Noto fonts, with emoji scaled from 2048 units to the em and glyphs built from others
/// (ǅ); the code one stretched to Atkinson Hyperlegible Mono's width.
const FALLBACK: [(char, u16, u16, f64); 10] = [
    ('→', 1578, 1012, -94250.0),
    ('✓', 1215, 780, -104113.0),
    ('α', 128, 623, -133071.0),
    ('♪', 1083, 451, -62991.0),
    ('🙂', 3842, 1270, -166110.0),
    ('☀', 2025, 914, -159571.0),
    ('⌘', 2267, 814, -253561.0),
    ('①', 2709, 1000, -173016.0),
    ('ʃ', 465, 258, -98927.0),
    ('ǅ', 305, 1200, -286870.0),
];
const FALLBACK_MONO: [(char, u16, u16, f64); 6] = [
    ('└', 974, 632, -68835.0),
    ('─', 970, 632, -45724.0),
    ('│', 971, 632, -91875.0),
    ('├', 976, 632, -112110.0),
    ('→', 964, 632, -32264.0),
    ('α', 162, 632, -137777.0),
];

#[test]
fn reads_the_fallback_fonts() {
    for (file, expected) in [("ChatComputerFallback.ttf", &FALLBACK[..]), ("ChatComputerFallbackMono.ttf", &FALLBACK_MONO[..])] {
        let data = std::fs::read(format!("{}/../fonts/{file}", env!("CARGO_MANIFEST_DIR"))).expect("the font file");
        let f = Font::parse(&data).unwrap();
        assert_eq!(f.metrics.units_per_em, 1000, "{file}");
        for &(c, glyph, advance, area) in expected {
            assert_eq!((f.glyph_index(c), f.advance(glyph)), (glyph, advance), "{file}: {c:?}");
            let mut a = Area::default();
            assert!(f.outline(glyph, &mut a));
            let got = a.twice / 2.0;
            assert!((got - area).abs() < area.abs() * 0.0005, "{file}: {c:?}: area {got}, fontTools {area}");
        }
        // The main fonts' own characters aren't in them (Lora's, and the code font's).
        assert_eq!(f.glyph_index('a'), 0, "{file}");
    }
}

/// Every glyph's advance at the weights other than the default, as HarfBuzz reads them (advance_reference.txt, from
/// reference.py): from the advances' own variations (HVAR), which a browser uses, rather than the outlines' (gvar),
/// which can round the other way (the bold space: 312, not 313). All at once, or one at a time, the same.
#[test]
fn advances_vary_as_harfbuzz_has_them() {
    let data = atkinson();
    let f = Font::parse(&data).unwrap();
    let reference: Vec<Vec<u16>> = include_str!("advance_reference.txt")
        .lines()
        .map(|l| l.split(' ').map(|v| v.parse().unwrap()).collect())
        .collect();
    assert_eq!(reference.len(), f.glyph_count() as usize);
    for (k, weight) in [700, 200, 800].into_iter().enumerate() {
        let at = f.instance(b"wght", weight).unwrap();
        let all = f.advances_at(&at, 0..f.glyph_count());
        let want: Vec<u16> = reference.iter().map(|r| r[k]).collect();
        assert_eq!(all, want, "at {weight}");
        for g in [0, 1, f.glyph_index(' '), f.glyph_count() - 1] {
            assert_eq!(f.advance_at(g, &at), want[g as usize], "glyph {g} at {weight}");
        }
    }
    assert_eq!(f.advance_at(f.glyph_index(' '), &f.instance(b"wght", 700).unwrap()), 312);
}

/// Atkinson Hyperlegible Next's kerning, as HarfBuzz shapes it (kern_reference.txt, from reference.py): every
/// pair of 152 characters (ASCII, accented letters, quotes and dashes) at four weights, those that kern and those
/// that don't. Its 'kern' feature has two lookups (glyph pairs and classes of glyphs), the second adding to the first,
/// and its adjustments vary with the weight (GDEF's item variation store).
#[test]
fn kerns_as_harfbuzz_does() {
    let data = atkinson();
    let f = Font::parse(&data).unwrap();
    let reference = include_str!("kern_reference.txt");
    let mut lines = reference.lines();
    let hex = |s: &str| char::from_u32(u32::from_str_radix(s, 16).unwrap()).unwrap();
    let chars: Vec<char> = lines.next().unwrap().split(' ').skip(1).map(hex).collect();
    assert_eq!(chars.len(), 152);
    let mut kerned = std::collections::HashMap::new();
    for line in lines {
        let v: Vec<&str> = line.split(' ').collect();
        let k: Vec<i32> = v[2..].iter().map(|x| x.parse().unwrap()).collect();
        kerned.insert((hex(v[0]), hex(v[1])), k);
    }
    assert_eq!(kerned.len(), 4368);
    let at = |w: i32| (w != 400).then(|| f.instance(b"wght", w).unwrap());
    let instances = [400, 700, 200, 800].map(at);
    let kernings: Vec<Kerning> = instances.iter().map(|i| f.kerning(i.as_ref())).collect();
    assert!(kernings.iter().all(|k| !k.is_empty()));
    let mut wrong = Vec::new();
    for &a in &chars {
        for &b in &chars {
            let want = kerned.get(&(a, b)).cloned().unwrap_or(vec![0; 4]);
            let got: Vec<i32> = kernings.iter().map(|k| k.pair(f.glyph_index(a), f.glyph_index(b))).collect();
            if got != want {
                wrong.push(format!("{a}{b}: {got:?}, HarfBuzz {want:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{} of {} pairs differ, e.g. {:?}", wrong.len(), chars.len() * chars.len(), &wrong[..wrong.len().min(8)]);
    // ("AV", "To" and "y." closer; nothing moves the glyph before a mark: an accent is placed by the 'mark' feature.)
    let k = &kernings[0];
    let g = |c| f.glyph_index(c);
    assert!(k.pair(g('A'), g('V')) < 0 && k.pair(g('T'), g('o')) < 0 && k.pair(g('y'), g('.')) < 0);
    assert_eq!(k.pair(g('A'), g('\u{301}')), 0);
}

#[test]
fn a_font_without_kerning_kerns_nothing() {
    let data = std::fs::read(format!("{}/../fonts/ChatComputerFallback.ttf", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let f = Font::parse(&data).unwrap();
    let k = f.kerning(None);
    assert!(k.is_empty());
    assert_eq!(k.pair(f.glyph_index('→'), f.glyph_index('α')), 0);
    let mono = std::fs::read(format!("{}/../../chatcomputer/ui/assets/fonts/AtkinsonHyperlegibleMono.ttf", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let m = Font::parse(&mono).unwrap();
    assert!(m.kerning(None).is_empty(), "the code font has marks positioned, but no kerning");
}

/// Where Lora puts marks (accents typed as characters of their own) on letters and on each other, as HarfBuzz has
/// them (marks_reference.txt, from marks_reference.py), at its regular weight and its bold: each mark on the letter
/// by the font's mark-to-base anchors, then on the mark before it by its mark-to-mark ones; where it has none for
/// the two, where the pen is after the letter (marks take no room).
#[test]
fn places_marks_as_harfbuzz_does() {
    let data = std::fs::read(format!("{}/../../chatcomputer/ui/assets/fonts/Lora.ttf", env!("CARGO_MANIFEST_DIR"))).expect("Lora");
    let f = Font::parse(&data).unwrap();
    let bold = f.instance(b"wght", 700).unwrap();
    let (regular_marks, bold_marks) = (f.marks(None), f.marks(Some(&bold)));
    assert!(!regular_marks.is_empty());
    let mut placed = 0;
    for line in include_str!("marks_reference.txt").lines() {
        let mut words = line.split(' ');
        let (text, weight) = (words.next().unwrap(), words.next().unwrap());
        let glyphs: Vec<(u16, i32, i32)> = words
            .map(|w| {
                let v: Vec<i32> = w.split(':').map(|n| n.parse().unwrap()).collect();
                (v[0] as u16, v[1], v[2])
            })
            .collect();
        let (marks, pen) = if weight == "700" {
            (&bold_marks, f.advance_at(glyphs[0].0, &bold) as i32)
        } else {
            (&regular_marks, f.advance(glyphs[0].0) as i32)
        };
        let base = glyphs[0].0;
        assert!(!marks.is_mark(base), "{text}");
        let mut before: Vec<(u16, (i32, i32))> = Vec::new();
        for &(g, x, y) in &glyphs[1..] {
            assert!(marks.is_mark(g), "{text}: {g}");
            let mut at = marks.on_base(base, g).unwrap_or((pen, 0));
            if let Some(on) = marks.on_mark(&before, g) {
                at = on;
            }
            assert_eq!(at, (x, y), "{text} at {weight}: glyph {g}");
            before.push((g, at));
            placed += 1;
        }
    }
    assert!(placed >= 25, "{placed}");
    // A font without them (the fallbacks are made without GPOS) places none.
    let data = std::fs::read(format!("{}/../fonts/ChatComputerFallback.ttf", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let none = Font::parse(&data).unwrap().marks(None);
    assert!(none.is_empty() && none.on_base(1, 2).is_none() && !none.is_mark(1));
}
