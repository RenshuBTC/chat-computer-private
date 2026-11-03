//! How big Chat Computer draws text on this screen: as big as the Claude app draws it there. The same rules as
//! Chat Computer's screen on Linux (chatcomputer/ui/layout.py, where they're written down): the Claude app's sizes
//! on a computer at 100% display scaling (measured from it on a 1920 x 1080 screen): its words 16px on lines 22px
//! apart, the conversation 924px wide, Claude's words in it up to 865px, the message box 962px wide; made bigger on
//! a sharper screen, or one farther away, as a computer's display scaling makes the Claude app bigger. The screen's
//! real size comes from the monitor (EDID); the viewing distance is estimated from it.

/// Text size at 100%: the Claude app's words.
pub const BASE_FONT_PX: f64 = 16.0;
/// Its lines' height, in em (22px).
pub const LINE_HEIGHT: f64 = 1.375;
/// Claude's words' widest line, and the message box's width, in rem (865px and 962px).
pub const TEXT_REM: f64 = 54.0625;
pub const BOX_REM: f64 = 60.125;
const REF_PX_PER_MM: f64 = 96.0 / 25.4;
const REF_DISTANCE_MM: f64 = 600.0;
/// The conversation's width, in rem (924px).
const COLUMN_REM: f64 = 57.75;
const MARGIN_REM: f64 = 1.5;
const SCALE_STEP: f64 = 0.125;
const SCALE_MIN: f64 = 0.75;
const SCALE_MAX: f64 = 4.0;
const TAN_15_DEG: f64 = 0.2679491924311227; // half of the 30° a TV should fill
const BOGUS_MM: [(i64, i64); 6] = [(160, 90), (160, 100), (16, 9), (16, 10), (150, 90), (0, 0)];

#[derive(Clone, Copy)]
pub struct Layout {
    pub font_px: f64,
    pub column_px: f64,
    pub scale: f64,
    pub distance_mm: f64,
    pub diagonal_in: f64,
    pub size_known: bool,
}

fn abs(x: f64) -> f64 {
    if x < 0.0 { -x } else { x }
}

fn sqrt(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut r = if x > 1.0 { x / 2.0 } else { 1.0 };
    for _ in 0..64 {
        let next = 0.5 * (r + x / r);
        if abs(next - r) <= 1e-12 * r {
            return next;
        }
        r = next;
    }
    r
}

fn hypot(a: f64, b: f64) -> f64 {
    sqrt(a * a + b * b)
}

/// Rounds half to even, like Python's round().
fn round_even(x: f64) -> f64 {
    let f = if x >= 0.0 { x as i64 } else { x as i64 - 1 } as f64; // floor (the values here are small)
    let d = x - f;
    if d > 0.5 || (d == 0.5 && (f as i64) % 2 != 0) { f + 1.0 } else { f }
}

fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    if x < lo { lo } else if x > hi { hi } else { x }
}

/// The monitor's size in mm if believable, else the size it would have at 96 dpi (and `false`).
fn monitor_mm(px_w: f64, px_h: f64, reported: Option<(u32, u32)>) -> (f64, f64, bool) {
    if let Some((rw, rh)) = reported {
        let (mut w, mut h) = (rw as f64, rh as f64);
        if w > 0.0 && h > 0.0 && (w > h) != (px_w > px_h) {
            core::mem::swap(&mut w, &mut h); // rotated monitors can report width and height swapped
        }
        let mut ok = w >= 150.0 && h >= 80.0 && !BOGUS_MM.contains(&(round_even(w) as i64, round_even(h) as i64));
        if ok {
            let aspect_err = abs((w / h) / (px_w / px_h) - 1.0);
            let density = px_w / w;
            ok = aspect_err < 0.15 && (0.8..=25.0).contains(&density);
        }
        if ok {
            return (w, h, true);
        }
    }
    (px_w / REF_PX_PER_MM, px_h / REF_PX_PER_MM, false)
}

fn estimate_distance_mm(w_mm: f64, h_mm: f64) -> f64 {
    let diagonal_in = hypot(w_mm, h_mm) / 25.4;
    let monitor = clamp(0.6 * w_mm + 300.0, 500.0, 1000.0);
    let tv = w_mm / (2.0 * TAN_15_DEG);
    if diagonal_in <= 17.5 {
        500.0
    } else if diagonal_in <= 34.5 {
        monitor
    } else if diagonal_in >= 40.0 {
        monitor.max(tv)
    } else {
        let t = (diagonal_in - 34.5) / (40.0 - 34.5);
        (1.0 - t) * monitor + t * monitor.max(tv)
    }
}

pub fn compute(width_px: u32, height_px: u32, reported_mm: Option<(u32, u32)>) -> Layout {
    let (px_w, px_h) = (width_px as f64, height_px as f64);
    let (w_mm, h_mm, known) = monitor_mm(px_w, px_h, reported_mm);
    let px_per_mm = px_w / w_mm;
    let distance = estimate_distance_mm(w_mm, h_mm);
    // How many times bigger a pixel must be drawn to look like one on the reference monitor.
    let base = (px_per_mm / REF_PX_PER_MM) * (distance / REF_DISTANCE_MM);
    let scale = clamp(round_even(base / SCALE_STEP) * SCALE_STEP, SCALE_MIN, SCALE_MAX);
    let font_px = BASE_FONT_PX * scale;
    let column_px = (COLUMN_REM * font_px).min((px_w - 2.0 * MARGIN_REM * font_px).max(200.0));
    Layout {
        font_px,
        column_px,
        scale,
        distance_mm: distance,
        diagonal_in: hypot(w_mm, h_mm) / 25.4,
        size_known: known,
    }
}

/// `base` with its text `factor` times the size (the person asked for bigger or smaller), and the column to match.
pub fn scaled(base: Layout, width_px: u32, factor: f64) -> Layout {
    let font_px = base.font_px * factor;
    let px_w = width_px as f64;
    Layout {
        font_px,
        column_px: (COLUMN_REM * font_px).min((px_w - 2.0 * MARGIN_REM * font_px).max(200.0)),
        scale: base.scale * factor,
        ..base
    }
}
