//! Drawing vector shapes into pixels: paths flattened through a transform (lines, Bézier curves, elliptical arcs),
//! filled antialiased (each pixel's covered area worked out from the edges crossing it, the signed-area
//! accumulation of font rasterizers; non-zero or even-odd), and strokes made outlines to fill (caps, joins,
//! dashes); and CSS's colours read. For SVG pictures (svg.rs) and the browser's canvas.

use alloc::vec;
use alloc::vec::Vec;

/// floor, ceil and the remainder with the divisor's sign, without the standard library (for numbers a picture's
/// size could be).
pub fn floor(x: f32) -> f32 {
    if !x.is_finite() || x.abs() >= 8_388_608.0 {
        return x;
    }
    let t = x as i32 as f32;
    if t > x { t - 1.0 } else { t }
}

pub fn ceil(x: f32) -> f32 {
    -floor(-x)
}

pub fn rem_euclid(a: f32, b: f32) -> f32 {
    let r = a % b;
    if r < 0.0 { r + b } else { r }
}

/// An affine transform: x' = a x + c y + e, y' = b x + d y + f.
#[derive(Clone, Copy, Debug)]
pub struct Matrix(pub [f32; 6]);

impl Matrix {
    pub const ID: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// self ∘ m: m applied first.
    pub fn then(&self, m: &Matrix) -> Matrix {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = m.0;
        Matrix([a * a2 + c * b2, b * a2 + d * b2, a * c2 + c * d2, b * c2 + d * d2, a * e2 + c * f2 + e, b * e2 + d * f2 + f])
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }

    pub fn translate(x: f32, y: f32) -> Matrix {
        Matrix([1.0, 0.0, 0.0, 1.0, x, y])
    }

    pub fn scale(x: f32, y: f32) -> Matrix {
        Matrix([x, 0.0, 0.0, y, 0.0, 0.0])
    }

    pub fn invert(&self) -> Option<Matrix> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-12 {
            return None;
        }
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        Some(Matrix([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)]))
    }

    /// How much it scales lengths (on average).
    pub fn scale_of(&self) -> f32 {
        let [a, b, c, d, _, _] = self.0;
        libm_sqrt((a * d - b * c).abs())
    }
}

pub fn libm_sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    // Newton's method from a good guess (the float's exponent halved).
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1FBD_1DF5);
    for _ in 0..4 {
        y = 0.5 * (y + x / y);
    }
    y
}

/// sin and cos (radians), by a polynomial after reducing to [-π/4, π/4].
pub fn sin_cos(x: f32) -> (f32, f32) {
    let x = x as f64;
    let q = { let v = x / core::f64::consts::FRAC_PI_2 + 0.5; let t = v as i64 as f64; if t > v { t - 1.0 } else { t } };
    let r = x - q * core::f64::consts::FRAC_PI_2;
    let r2 = r * r;
    let s = r * (1.0 - r2 / 6.0 * (1.0 - r2 / 20.0 * (1.0 - r2 / 42.0 * (1.0 - r2 / 72.0))));
    let c = 1.0 - r2 / 2.0 * (1.0 - r2 / 12.0 * (1.0 - r2 / 30.0 * (1.0 - r2 / 56.0)));
    let (s, c) = match (q as i64).rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    };
    (s as f32, c as f32)
}

/// atan2, by a polynomial (good to about 1e-6 radians).
pub fn atan2(y: f32, x: f32) -> f32 {
    let (y, x) = (y as f64, x as f64);
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let (ax, ay) = (x.abs(), y.abs());
    let (t, swap) = if ay > ax { (ax / ay, true) } else { (ay / ax, false) };
    // atan on [0, 1]: reduced to [0, tan(π/12)] and a series.
    let (t, add) = if t > 0.267_949_192_431_122_7 { ((t - 0.577_350_269_189_625_7) / (1.0 + 0.577_350_269_189_625_7 * t), core::f64::consts::PI / 6.0) } else { (t, 0.0) };
    let t2 = t * t;
    let mut a = add + t * (1.0 - t2 * (1.0 / 3.0 - t2 * (1.0 / 5.0 - t2 * (1.0 / 7.0 - t2 / 9.0))));
    if swap {
        a = core::f64::consts::FRAC_PI_2 - a;
    }
    if x < 0.0 {
        a = core::f64::consts::PI - a;
    }
    if y < 0.0 {
        a = -a;
    }
    a as f32
}

/// CSS's named colours.
const NAMED: &[(&str, u32)] = &[
    ("aliceblue", 0xF0F8FF), ("antiquewhite", 0xFAEBD7), ("aqua", 0x00FFFF), ("aquamarine", 0x7FFFD4), ("azure", 0xF0FFFF),
    ("beige", 0xF5F5DC), ("bisque", 0xFFE4C4), ("black", 0x000000), ("blanchedalmond", 0xFFEBCD), ("blue", 0x0000FF),
    ("blueviolet", 0x8A2BE2), ("brown", 0xA52A2A), ("burlywood", 0xDEB887), ("cadetblue", 0x5F9EA0), ("chartreuse", 0x7FFF00),
    ("chocolate", 0xD2691E), ("coral", 0xFF7F50), ("cornflowerblue", 0x6495ED), ("cornsilk", 0xFFF8DC), ("crimson", 0xDC143C),
    ("cyan", 0x00FFFF), ("darkblue", 0x00008B), ("darkcyan", 0x008B8B), ("darkgoldenrod", 0xB8860B), ("darkgray", 0xA9A9A9),
    ("darkgreen", 0x006400), ("darkgrey", 0xA9A9A9), ("darkkhaki", 0xBDB76B), ("darkmagenta", 0x8B008B), ("darkolivegreen", 0x556B2F),
    ("darkorange", 0xFF8C00), ("darkorchid", 0x9932CC), ("darkred", 0x8B0000), ("darksalmon", 0xE9967A), ("darkseagreen", 0x8FBC8F),
    ("darkslateblue", 0x483D8B), ("darkslategray", 0x2F4F4F), ("darkslategrey", 0x2F4F4F), ("darkturquoise", 0x00CED1),
    ("darkviolet", 0x9400D3), ("deeppink", 0xFF1493), ("deepskyblue", 0x00BFFF), ("dimgray", 0x696969), ("dimgrey", 0x696969),
    ("dodgerblue", 0x1E90FF), ("firebrick", 0xB22222), ("floralwhite", 0xFFFAF0), ("forestgreen", 0x228B22), ("fuchsia", 0xFF00FF),
    ("gainsboro", 0xDCDCDC), ("ghostwhite", 0xF8F8FF), ("gold", 0xFFD700), ("goldenrod", 0xDAA520), ("gray", 0x808080),
    ("green", 0x008000), ("greenyellow", 0xADFF2F), ("grey", 0x808080), ("honeydew", 0xF0FFF0), ("hotpink", 0xFF69B4),
    ("indianred", 0xCD5C5C), ("indigo", 0x4B0082), ("ivory", 0xFFFFF0), ("khaki", 0xF0E68C), ("lavender", 0xE6E6FA),
    ("lavenderblush", 0xFFF0F5), ("lawngreen", 0x7CFC00), ("lemonchiffon", 0xFFFACD), ("lightblue", 0xADD8E6), ("lightcoral", 0xF08080),
    ("lightcyan", 0xE0FFFF), ("lightgoldenrodyellow", 0xFAFAD2), ("lightgray", 0xD3D3D3), ("lightgreen", 0x90EE90),
    ("lightgrey", 0xD3D3D3), ("lightpink", 0xFFB6C1), ("lightsalmon", 0xFFA07A), ("lightseagreen", 0x20B2AA),
    ("lightskyblue", 0x87CEFA), ("lightslategray", 0x778899), ("lightslategrey", 0x778899), ("lightsteelblue", 0xB0C4DE),
    ("lightyellow", 0xFFFFE0), ("lime", 0x00FF00), ("limegreen", 0x32CD32), ("linen", 0xFAF0E6), ("magenta", 0xFF00FF),
    ("maroon", 0x800000), ("mediumaquamarine", 0x66CDAA), ("mediumblue", 0x0000CD), ("mediumorchid", 0xBA55D3),
    ("mediumpurple", 0x9370DB), ("mediumseagreen", 0x3CB371), ("mediumslateblue", 0x7B68EE), ("mediumspringgreen", 0x00FA9A),
    ("mediumturquoise", 0x48D1CC), ("mediumvioletred", 0xC71585), ("midnightblue", 0x191970), ("mintcream", 0xF5FFFA),
    ("mistyrose", 0xFFE4E1), ("moccasin", 0xFFE4B5), ("navajowhite", 0xFFDEAD), ("navy", 0x000080), ("oldlace", 0xFDF5E6),
    ("olive", 0x808000), ("olivedrab", 0x6B8E23), ("orange", 0xFFA500), ("orangered", 0xFF4500), ("orchid", 0xDA70D6),
    ("palegoldenrod", 0xEEE8AA), ("palegreen", 0x98FB98), ("paleturquoise", 0xAFEEEE), ("palevioletred", 0xDB7093),
    ("papayawhip", 0xFFEFD5), ("peachpuff", 0xFFDAB9), ("peru", 0xCD853F), ("pink", 0xFFC0CB), ("plum", 0xDDA0DD),
    ("powderblue", 0xB0E0E6), ("purple", 0x800080), ("rebeccapurple", 0x663399), ("red", 0xFF0000), ("rosybrown", 0xBC8F8F),
    ("royalblue", 0x4169E1), ("saddlebrown", 0x8B4513), ("salmon", 0xFA8072), ("sandybrown", 0xF4A460), ("seagreen", 0x2E8B57),
    ("seashell", 0xFFF5EE), ("sienna", 0xA0522D), ("silver", 0xC0C0C0), ("skyblue", 0x87CEEB), ("slateblue", 0x6A5ACD),
    ("slategray", 0x708090), ("slategrey", 0x708090), ("snow", 0xFFFAFA), ("springgreen", 0x00FF7F), ("steelblue", 0x4682B4),
    ("tan", 0xD2B48C), ("teal", 0x008080), ("thistle", 0xD8BFD8), ("tomato", 0xFF6347), ("turquoise", 0x40E0D0),
    ("violet", 0xEE82EE), ("wheat", 0xF5DEB3), ("white", 0xFFFFFF), ("whitesmoke", 0xF5F5F5), ("yellow", 0xFFFF00),
    ("yellowgreen", 0x9ACD32),
];

/// A colour, RGBA from 0 to 1 (alpha straight).
pub type Rgba = [f32; 4];

fn hue(p: f32, q: f32, t: f32) -> f32 {
    let t = t - floor(t);
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

/// A colour as CSS writes it (None: not one this reads; "currentColor" is the caller's).
pub fn color(s: &str) -> Option<Rgba> {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix('#') {
        let d: Vec<u32> = hex.chars().map(|c| c.to_digit(16)).collect::<Option<Vec<u32>>>()?;
        let f = |v: u32| v as f32 / 255.0;
        return match d.len() {
            3 | 4 => Some([f(d[0] * 17), f(d[1] * 17), f(d[2] * 17), if d.len() == 4 { f(d[3] * 17) } else { 1.0 }]),
            6 | 8 => Some([f(d[0] * 16 + d[1]), f(d[2] * 16 + d[3]), f(d[4] * 16 + d[5]), if d.len() == 8 { f(d[6] * 16 + d[7]) } else { 1.0 }]),
            _ => None,
        };
    }
    if lower == "transparent" {
        return Some([0.0, 0.0, 0.0, 0.0]);
    }
    if let Some(open) = lower.find('(') {
        let name = &lower[..open];
        let inner = lower[open + 1..].trim_end_matches(')');
        let parts: Vec<&str> = inner.split(|c: char| c == ',' || c == '/' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
        let num = |p: &str, whole: f32| -> Option<f32> {
            if let Some(v) = p.strip_suffix('%') { v.parse::<f32>().ok().map(|v| v / 100.0 * whole) } else { p.trim_end_matches("deg").parse::<f32>().ok() }
        };
        let alpha = parts.get(3).and_then(|p| num(p, 1.0)).unwrap_or(1.0).clamp(0.0, 1.0);
        return match name {
            "rgb" | "rgba" if parts.len() >= 3 => Some([num(parts[0], 255.0)? / 255.0, num(parts[1], 255.0)? / 255.0, num(parts[2], 255.0)? / 255.0, alpha]).map(|c| c.map(|v| v.clamp(0.0, 1.0))),
            "hsl" | "hsla" if parts.len() >= 3 => {
                let h = num(parts[0], 360.0)? / 360.0;
                let sat = num(parts[1], 1.0)?.clamp(0.0, 1.0);
                let l = num(parts[2], 1.0)?.clamp(0.0, 1.0);
                let q = if l < 0.5 { l * (1.0 + sat) } else { l + sat - l * sat };
                let p = 2.0 * l - q;
                Some([hue(p, q, h + 1.0 / 3.0), hue(p, q, h), hue(p, q, h - 1.0 / 3.0), alpha])
            }
            _ => None,
        };
    }
    let v = NAMED.binary_search_by(|(n, _)| (*n).cmp(lower.as_str())).ok().map(|i| NAMED[i].1)?;
    Some([((v >> 16) & 0xFF) as f32 / 255.0, ((v >> 8) & 0xFF) as f32 / 255.0, (v & 0xFF) as f32 / 255.0, 1.0])
}

/// A shape's outline, in the picture's pixels: closed polygons (and open polylines, for strokes).
pub struct Outline {
    /// Each subpath's points, and whether it's closed.
    pub paths: Vec<(Vec<(f32, f32)>, bool)>,
}

/// Builds an outline from path commands in user units, flattened in the picture's pixels.
pub struct Builder {
    m: Matrix,
    paths: Vec<(Vec<(f32, f32)>, bool)>,
    cur: Vec<(f32, f32)>,
    /// The pen, and the subpath's start, in user units.
    pub pen: (f32, f32),
    pub start: (f32, f32),
    tolerance: f32,
    points: usize,
}

/// The most points one shape's outline has (past them, it's left as it is).
const MOST_POINTS: usize = 1 << 20;

impl Builder {
    pub fn new(m: Matrix) -> Builder {
        Builder { m, paths: Vec::new(), cur: Vec::new(), pen: (0.0, 0.0), start: (0.0, 0.0), tolerance: 0.2, points: 0 }
    }

    fn point(&mut self, p: (f32, f32)) {
        let d = self.m.apply(p.0, p.1);
        if self.cur.last() != Some(&d) && self.points < MOST_POINTS {
            self.cur.push(d);
            self.points += 1;
        }
    }

    fn flush(&mut self, closed: bool) {
        if self.cur.len() >= 2 || (self.cur.len() == 1 && !closed) {
            let pts = core::mem::take(&mut self.cur);
            self.paths.push((pts, closed));
        }
        self.cur.clear();
    }

    pub fn move_to(&mut self, p: (f32, f32)) {
        self.flush(false);
        self.pen = p;
        self.start = p;
        self.point(p);
    }

    pub fn line_to(&mut self, p: (f32, f32)) {
        if self.cur.is_empty() {
            self.point(self.pen);
        }
        self.pen = p;
        self.point(p);
    }

    /// How many lines a curve over these points (in user units) is drawn with.
    fn steps(&self, pts: &[(f32, f32)]) -> usize {
        let mut len = 0.0;
        for w in pts.windows(2) {
            let (a, b) = (self.m.apply(w[0].0, w[0].1), self.m.apply(w[1].0, w[1].1));
            len += libm_sqrt((b.0 - a.0) * (b.0 - a.0) + (b.1 - a.1) * (b.1 - a.1));
        }
        ((libm_sqrt(len / self.tolerance) * 0.6) as usize).clamp(2, 200)
    }

    pub fn cubic_to(&mut self, c1: (f32, f32), c2: (f32, f32), p: (f32, f32)) {
        if self.cur.is_empty() {
            self.point(self.pen);
        }
        let p0 = self.pen;
        let n = self.steps(&[p0, c1, c2, p]);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.point((a * p0.0 + b * c1.0 + c * c2.0 + d * p.0, a * p0.1 + b * c1.1 + c * c2.1 + d * p.1));
        }
        self.pen = p;
    }

    pub fn quad_to(&mut self, c: (f32, f32), p: (f32, f32)) {
        if self.cur.is_empty() {
            self.point(self.pen);
        }
        let p0 = self.pen;
        let n = self.steps(&[p0, c, p]);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            self.point((u * u * p0.0 + 2.0 * u * t * c.0 + t * t * p.0, u * u * p0.1 + 2.0 * u * t * c.1 + t * t * p.1));
        }
        self.pen = p;
    }

    /// An elliptical arc to `p` (as SVG's path data has it: radii, the x axis's rotation in degrees, which arc).
    pub fn arc_to(&mut self, rx: f32, ry: f32, rot: f32, large: bool, sweep: bool, p: (f32, f32)) {
        let p0 = self.pen;
        if (p0.0 - p.0).abs() < 1e-6 && (p0.1 - p.1).abs() < 1e-6 {
            return;
        }
        let (mut rx, mut ry) = (rx.abs(), ry.abs());
        if rx < 1e-6 || ry < 1e-6 {
            self.line_to(p);
            return;
        }
        if self.cur.is_empty() {
            self.point(p0);
        }
        // From the endpoints to the centre (SVG's implementation notes, F.6.5).
        let (s, c) = sin_cos(rot.to_radians());
        let (dx, dy) = ((p0.0 - p.0) / 2.0, (p0.1 - p.1) / 2.0);
        let (x1, y1) = (c * dx + s * dy, -s * dx + c * dy);
        let lambda = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
        if lambda > 1.0 {
            let k = libm_sqrt(lambda);
            rx *= k;
            ry *= k;
        }
        let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
        let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
        let mut k = if den > 0.0 { libm_sqrt((num / den).max(0.0)) } else { 0.0 };
        if large == sweep {
            k = -k;
        }
        let (cx1, cy1) = (k * rx * y1 / ry, -k * ry * x1 / rx);
        let (cx, cy) = (c * cx1 - s * cy1 + (p0.0 + p.0) / 2.0, s * cx1 + c * cy1 + (p0.1 + p.1) / 2.0);
        let theta = atan2((y1 - cy1) / ry, (x1 - cx1) / rx);
        let mut delta = atan2((-y1 - cy1) / ry, (-x1 - cx1) / rx) - theta;
        let tau = core::f32::consts::TAU;
        if sweep && delta < 0.0 {
            delta += tau;
        } else if !sweep && delta > 0.0 {
            delta -= tau;
        }
        let r = (rx.max(ry)) * self.m.scale_of();
        let n = ((delta.abs() * libm_sqrt(r.max(1.0) / self.tolerance) * 0.35) as usize).clamp(2, 256);
        for i in 1..=n {
            let a = theta + delta * i as f32 / n as f32;
            let (sa, ca) = sin_cos(a);
            let (ex, ey) = (rx * ca, ry * sa);
            let q = if i == n { p } else { (c * ex - s * ey + cx, s * ex + c * ey + cy) };
            self.point(q);
        }
        self.pen = p;
    }

    /// The transform points go through from now on (a canvas's, which can change as its path is made): the pen and
    /// the subpath's start stay where they are in the picture.
    pub fn set_matrix(&mut self, m: Matrix) {
        let (pen, start) = (self.m.apply(self.pen.0, self.pen.1), self.m.apply(self.start.0, self.start.1));
        self.m = m;
        if let Some(inv) = m.invert() {
            self.pen = inv.apply(pen.0, pen.1);
            self.start = inv.apply(start.0, start.1);
        }
    }

    /// Whether a subpath has begun.
    pub fn has_point(&self) -> bool {
        !self.cur.is_empty()
    }

    /// Whether the path has nothing in it yet (no subpath, not even a closed one).
    pub fn is_empty(&self) -> bool {
        self.cur.is_empty() && self.paths.is_empty()
    }

    /// An arc of the ellipse centred at `c` with radii `rx` and `ry` (its x axis turned `rot` radians), from angle `a0`
    /// to `a1` (radians, anticlockwise if `ccw`), as a canvas's arc() and ellipse() make it: joined to the subpath so
    /// far by a line (or starting one), the whole ellipse if the angles are a turn or more apart.
    #[allow(clippy::too_many_arguments)]
    pub fn arc(&mut self, c: (f32, f32), rx: f32, ry: f32, rot: f32, a0: f32, a1: f32, ccw: bool) {
        let tau = core::f32::consts::TAU;
        let sweep = if !ccw {
            if a1 - a0 >= tau { tau } else { rem_euclid(a1 - a0, tau) }
        } else if a0 - a1 >= tau {
            -tau
        } else {
            -rem_euclid(a0 - a1, tau)
        };
        let (sr, cr) = sin_cos(rot);
        let at = |a: f32| {
            let (s, co) = sin_cos(a);
            let (x, y) = (rx * co, ry * s);
            (c.0 + cr * x - sr * y, c.1 + sr * x + cr * y)
        };
        let p0 = at(a0);
        if self.is_empty() {
            self.move_to(p0);
        } else {
            self.line_to(p0);
        }
        let r = rx.abs().max(ry.abs()) * self.m.scale_of();
        let n = ((sweep.abs() * libm_sqrt(r.max(1.0) / self.tolerance) * 0.35) as usize).clamp(2, 256);
        for i in 1..=n {
            self.point(at(a0 + sweep * i as f32 / n as f32));
        }
        self.pen = at(a0 + sweep);
    }

    /// The outline so far (the builder goes on: a canvas fills its path, then adds to it).
    pub fn outline(&self) -> Outline {
        let mut paths = self.paths.clone();
        if !self.cur.is_empty() {
            paths.push((self.cur.clone(), false));
        }
        Outline { paths }
    }

    pub fn close(&mut self) {
        self.pen = self.start;
        self.flush(true);
    }

    pub fn finish(mut self) -> Outline {
        self.flush(false);
        Outline { paths: self.paths }
    }
}

// --------------------------------------------------------------------------------------------------- coverage

/// Each pixel's covered area, accumulated from the edges crossing its row (signed by which way they go): the sum
/// along a row is how much of each pixel is inside.
pub struct Coverage {
    pub w: usize,
    pub h: usize,
    /// Rows `w + 2` long (an edge past the right is kept off the next row).
    pub a: Vec<f32>,
    /// Where the area is in the picture, and its size.
    pub x0: i32,
    pub y0: i32,
}

impl Coverage {
    fn new(x0: i32, y0: i32, w: usize, h: usize) -> Coverage {
        Coverage { w, h, a: vec![0.0; (w + 2) * h + 2], x0, y0 }
    }

    fn line(&mut self, p0: (f32, f32), p1: (f32, f32)) {
        let (mut p0, mut p1) = ((p0.0 - self.x0 as f32, p0.1 - self.y0 as f32), (p1.0 - self.x0 as f32, p1.1 - self.y0 as f32));
        if (p0.1 - p1.1).abs() <= f32::EPSILON || !(p0.0.is_finite() && p0.1.is_finite() && p1.0.is_finite() && p1.1.is_finite()) {
            return;
        }
        let dir = if p0.1 < p1.1 {
            1.0
        } else {
            core::mem::swap(&mut p0, &mut p1);
            -1.0
        };
        if p1.1 <= 0.0 || p0.1 >= self.h as f32 {
            return;
        }
        let stride = self.w + 2;
        let wmax = self.w as f32;
        let dxdy = (p1.0 - p0.0) / (p1.1 - p0.1);
        let mut x = p0.0;
        if p0.1 < 0.0 {
            x -= p0.1 * dxdy;
        }
        let y_start = p0.1.max(0.0) as usize;
        let y_end = (self.h as f32).min(ceil(p1.1)) as usize;
        for y in y_start..y_end {
            let line = y * stride;
            let dy = ((y + 1) as f32).min(p1.1) - (y as f32).max(p0.1);
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            // (Left of the picture counts at its left edge; right of it, past its right.)
            let (xa, xb) = (x.clamp(0.0, wmax), xnext.clamp(0.0, wmax));
            let (x0, x1) = if xa < xb { (xa, xb) } else { (xb, xa) };
            let x0floor = floor(x0);
            let x0i = x0floor as usize;
            let x1ceil = ceil(x1);
            let x1i = x1ceil as usize;
            if x1i <= x0i + 1 {
                let xmf = 0.5 * (xa + xb) - x0floor;
                self.a[line + x0i] += d - d * xmf;
                self.a[line + x0i + 1] += d * xmf;
            } else {
                let s = 1.0 / (x1 - x0);
                let x0f = x0 - x0floor;
                let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
                let x1f = x1 - x1ceil + 1.0;
                let am = 0.5 * s * x1f * x1f;
                self.a[line + x0i] += d * a0;
                if x1i == x0i + 2 {
                    self.a[line + x0i + 1] += d * (1.0 - a0 - am);
                } else {
                    let a1 = s * (1.5 - x0f);
                    self.a[line + x0i + 1] += d * (a1 - a0);
                    for xi in x0i + 2..x1i - 1 {
                        self.a[line + xi] += d * s;
                    }
                    let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                    self.a[line + x1i - 1] += d * (1.0 - a2 - am);
                }
                self.a[line + x1i] += d * am;
            }
            x = xnext;
        }
    }

    pub fn polygon(&mut self, pts: &[(f32, f32)]) {
        for i in 0..pts.len() {
            self.line(pts[i], pts[(i + 1) % pts.len()]);
        }
    }
}

// ------------------------------------------------------------------------------------------------------ strokes

fn sub(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    (a.0 - b.0, a.1 - b.1)
}

fn norm(v: (f32, f32)) -> Option<(f32, f32)> {
    let l = libm_sqrt(v.0 * v.0 + v.1 * v.1);
    if l < 1e-9 { None } else { Some((v.0 / l, v.1 / l)) }
}

/// A polygon's points, turned to go round anticlockwise (as the picture's y goes down: positive area), so polygons
/// of a stroke add up rather than cancel where they overlap.
pub fn positive(mut pts: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    let mut area = 0.0;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        area += a.0 * b.1 - b.0 * a.1;
    }
    if area < 0.0 {
        pts.reverse();
    }
    pts
}

pub fn circle(c: (f32, f32), r: f32) -> Vec<(f32, f32)> {
    let n = ((libm_sqrt(r.max(0.5)) * 6.0) as usize).clamp(8, 64);
    (0..n)
        .map(|i| {
            let (s, co) = sin_cos(core::f32::consts::TAU * i as f32 / n as f32);
            (c.0 + r * co, c.1 + r * s)
        })
        .collect()
}

/// A polyline cut into its dashes (lengths in the picture's pixels, starting `offset` into the pattern).
fn dashed(pts: &[(f32, f32)], dashes: &[f32], offset: f32) -> Vec<Vec<(f32, f32)>> {
    let total: f32 = dashes.iter().sum();
    if total <= 0.0 {
        return vec![pts.to_vec()];
    }
    let mut out = Vec::new();
    let mut k = 0;
    let mut left = dashes[0];
    let mut off = offset % total;
    if off < 0.0 {
        off += total;
    }
    while off > 0.0 {
        if off >= left {
            off -= left;
            k = (k + 1) % dashes.len();
            left = dashes[k];
        } else {
            left -= off;
            off = 0.0;
        }
    }
    let mut cur: Vec<(f32, f32)> = Vec::new();
    if k % 2 == 0 {
        cur.push(pts[0]);
    }
    for w in pts.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut seg = libm_sqrt((b.0 - a.0) * (b.0 - a.0) + (b.1 - a.1) * (b.1 - a.1));
        while seg > 0.0 {
            if left >= seg {
                left -= seg;
                if k % 2 == 0 {
                    cur.push(b);
                }
                seg = 0.0;
            } else {
                let t = left / seg;
                let m = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                if k % 2 == 0 {
                    cur.push(m);
                    out.push(core::mem::take(&mut cur));
                } else {
                    cur = vec![m];
                }
                seg -= left;
                a = m;
                k = (k + 1) % dashes.len();
                left = dashes[k];
            }
        }
    }
    if k % 2 == 0 && cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// A stroke's outline, as polygons to fill (non-zero): each segment's band, and its joins and ends.
pub fn stroke(outline: &Outline, width: f32, cap: u8, join: u8, miter: f32, dashes: &[f32], offset: f32) -> Vec<Vec<(f32, f32)>> {
    let hw = width / 2.0;
    let mut polys: Vec<Vec<(f32, f32)>> = Vec::new();
    for (pts, closed) in &outline.paths {
        let mut line = pts.clone();
        if *closed && line.len() >= 2 {
            line.push(line[0]);
        }
        let pieces = if dashes.is_empty() { vec![line] } else { dashed(&line, dashes, offset) };
        let whole_loop = *closed && dashes.is_empty();
        for piece in pieces {
            if piece.len() == 1 || (piece.len() == 2 && piece[0] == piece[1]) {
                // A dot: a round or square end shows it.
                let p = piece[0];
                match cap {
                    1 => polys.push(circle(p, hw)),
                    2 => polys.push(vec![(p.0 - hw, p.1 - hw), (p.0 + hw, p.1 - hw), (p.0 + hw, p.1 + hw), (p.0 - hw, p.1 + hw)]),
                    _ => {}
                }
                continue;
            }
            let n = piece.len();
            for i in 0..n - 1 {
                let (a, b) = (piece[i], piece[i + 1]);
                let Some(d) = norm(sub(b, a)) else { continue };
                let nrm = (-d.1 * hw, d.0 * hw);
                let (mut a2, mut b2) = (a, b);
                // Square ends reach past the ends by half the width.
                if cap == 2 && !whole_loop {
                    if i == 0 {
                        a2 = (a.0 - d.0 * hw, a.1 - d.1 * hw);
                    }
                    if i == n - 2 {
                        b2 = (b.0 + d.0 * hw, b.1 + d.1 * hw);
                    }
                }
                polys.push(positive(vec![(a2.0 + nrm.0, a2.1 + nrm.1), (b2.0 + nrm.0, b2.1 + nrm.1), (b2.0 - nrm.0, b2.1 - nrm.1), (a2.0 - nrm.0, a2.1 - nrm.1)]));
            }
            // Joins, between each two segments (and round the loop's start).
            let mut corners: Vec<usize> = (1..n - 1).collect();
            if whole_loop && n >= 3 {
                corners.push(0);
            }
            for &i in &corners {
                let p = piece[i];
                let prev = if i == 0 { piece[n - 2] } else { piece[i - 1] };
                let next = piece[i + 1];
                let (Some(d0), Some(d1)) = (norm(sub(p, prev)), norm(sub(next, p))) else { continue };
                let cross = d0.0 * d1.1 - d0.1 * d1.0;
                let dot = d0.0 * d1.0 + d0.1 * d1.1;
                if cross.abs() < 1e-4 && dot > 0.0 {
                    continue;
                }
                if join == 1 {
                    polys.push(circle(p, hw));
                    continue;
                }
                // The outer side's corners of the two bands, and the bevel (or miter) between them.
                let s = if cross > 0.0 { -1.0 } else { 1.0 };
                let n0 = (-d0.1 * hw * s, d0.0 * hw * s);
                let n1 = (-d1.1 * hw * s, d1.0 * hw * s);
                let (o0, o1) = ((p.0 + n0.0, p.1 + n0.1), (p.0 + n1.0, p.1 + n1.1));
                let half = libm_sqrt(((1.0 + dot) / 2.0).max(0.0));
                if join == 0 && half > 1e-3 && 1.0 / half <= miter {
                    // The miter's tip: along the bisector, hw / cos(half the angle between) out.
                    if let Some(bis) = norm((n0.0 + n1.0, n0.1 + n1.1)) {
                        let len = hw / half;
                        let tip = (p.0 + bis.0 * len, p.1 + bis.1 * len);
                        polys.push(positive(vec![p, o0, tip, o1]));
                        continue;
                    }
                }
                polys.push(positive(vec![p, o0, o1]));
            }
            if cap == 1 && !whole_loop {
                polys.push(circle(piece[0], hw));
                polys.push(circle(piece[n - 1], hw));
            }
        }
    }
    polys
}

/// The coverage of polygons (in the picture's pixels), in the box they take in it.
pub fn coverage_of(polys: &[Vec<(f32, f32)>], cw: f32, ch: f32) -> Option<Coverage> {
    let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in polys {
        for q in p {
            b = (b.0.min(q.0), b.1.min(q.1), b.2.max(q.0), b.3.max(q.1));
        }
    }
    if b.0 > b.2 {
        return None;
    }
    let x0 = floor(b.0).max(0.0);
    let y0 = floor(b.1).max(0.0);
    let x1 = (ceil(b.2) + 1.0).min(cw);
    let y1 = (ceil(b.3) + 1.0).min(ch);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut cov = Coverage::new(x0 as i32, y0 as i32, (x1 - x0) as usize, (y1 - y0) as usize);
    for p in polys {
        cov.polygon(p);
    }
    Some(cov)
}

