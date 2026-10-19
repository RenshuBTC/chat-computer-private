//! Checks against values read from the same font with fontTools, and exact coverage of simple shapes.

use super::raster::{Pen, Raster, ONE};
use super::*;

fn atkinson() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../chatcomputer/ui/assets/fonts/AtkinsonHyperlegibleNext.ttf");
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
