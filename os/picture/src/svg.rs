//! SVG pictures (the logos and icons of web pages): the XML read, each element's style worked out (presentation
//! attributes, a <style>'s rules, style attributes, inherited as SVG says), and drawn: paths (lines, Bézier curves,
//! arcs), rect, circle, ellipse, line, polyline and polygon, in groups and nested viewports, `use` of symbols and
//! other elements, filled (non-zero or even-odd) and stroked (butt, round or square ends; round, miter or bevel
//! joins; dashes), in colours or linear and radial gradients, through transforms, with opacity. Antialiased: each
//! pixel's covered area is worked out from the edges crossing it (the signed-area accumulation of font
//! rasterizers). Not drawn: text, filters, patterns and pictures inside; clip paths and masks don't clip.
//!
//! Drawn as RGBA (straight alpha), at any size: the picture's own (width and height, else its viewBox's) fitted
//! into the size asked for.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

// ---------------------------------------------------------------------------------------------------------- XML

struct Node {
    /// Lower case, without a namespace's prefix ("svg:path" is "path"); "" for the document.
    name: String,
    /// Names in lower case (viewBox is "viewbox"; xlink:href is "xlink:href").
    attrs: Vec<(String, String)>,
    children: Vec<usize>,
    parent: usize,
    /// Its text (a <style>'s rules).
    text: String,
}

impl Node {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

struct Doc {
    nodes: Vec<Node>,
}

/// Text with its entities (&amp;, &#38;, &#x26;) as the characters they are.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.as_bytes()[..rest.len().min(12)].iter().position(|&b| b == b';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{A0}'),
            _ if name.starts_with("#x") || name.starts_with("#X") => u32::from_str_radix(&name[2..], 16).ok().and_then(char::from_u32),
            _ if name.starts_with('#') => name[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase()
}

/// The XML read into a tree (forgiving: what isn't well formed is read as well as it can be).
fn parse(text: &str) -> Doc {
    let mut doc = Doc { nodes: vec![Node { name: String::new(), attrs: Vec::new(), children: Vec::new(), parent: 0, text: String::new() }] };
    let mut stack: Vec<usize> = vec![0];
    let b = text.as_bytes();
    let mut i = 0;
    let find = |from: usize, pat: &str| text[from..].find(pat).map(|k| from + k);
    while i < b.len() {
        if b[i] != b'<' {
            let end = find(i, "<").unwrap_or(b.len());
            let cur = *stack.last().unwrap_or(&0);
            doc.nodes[cur].text.push_str(&unescape(&text[i..end]));
            i = end;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i = find(i + 4, "-->").map_or(b.len(), |k| k + 3);
        } else if rest.starts_with("<![CDATA[") {
            let end = find(i + 9, "]]>").unwrap_or(b.len());
            let cur = *stack.last().unwrap_or(&0);
            doc.nodes[cur].text.push_str(&text[i + 9..end]);
            i = (end + 3).min(b.len());
        } else if rest.starts_with("<?") {
            i = find(i + 2, "?>").map_or(b.len(), |k| k + 2);
        } else if rest.starts_with("<!") {
            // A DOCTYPE (and its internal subset, if it has one).
            let bracket = find(i, "[");
            let close = find(i, ">").unwrap_or(b.len());
            i = match bracket {
                Some(k) if k < close => find(k, "]>").map_or(b.len(), |k| k + 2),
                _ => (close + 1).min(b.len()),
            };
        } else if rest.starts_with("</") {
            let end = find(i, ">").unwrap_or(b.len());
            let name = local(text[i + 2..end].trim());
            if let Some(k) = stack.iter().rposition(|&n| doc.nodes[n].name == name)
                && k > 0
            {
                stack.truncate(k);
            }
            i = (end + 1).min(b.len());
        } else {
            // An element: its name, its attributes, and whether it closes itself.
            let mut j = i + 1;
            while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
                j += 1;
            }
            let name = local(&text[i + 1..j]);
            let mut attrs = Vec::new();
            let mut closed = false;
            loop {
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= b.len() {
                    break;
                }
                if b[j] == b'>' {
                    j += 1;
                    break;
                }
                if b[j] == b'/' {
                    closed = true;
                    j += 1;
                    continue;
                }
                let start = j;
                while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'=' && b[j] != b'>' && b[j] != b'/' {
                    j += 1;
                }
                let key = text[start..j].to_ascii_lowercase();
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                let mut value = String::new();
                if j < b.len() && b[j] == b'=' {
                    j += 1;
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                        let q = b[j];
                        let end = text[j + 1..].find(q as char).map_or(b.len(), |k| j + 1 + k);
                        value = unescape(&text[j + 1..end]);
                        j = (end + 1).min(b.len());
                    } else {
                        let start = j;
                        while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                            j += 1;
                        }
                        value = unescape(&text[start..j]);
                    }
                }
                if !key.is_empty() {
                    attrs.push((key, value));
                }
            }
            let parent = *stack.last().unwrap_or(&0);
            doc.nodes.push(Node { name, attrs, children: Vec::new(), parent, text: String::new() });
            let id = doc.nodes.len() - 1;
            doc.nodes[parent].children.push(id);
            if !closed {
                stack.push(id);
            }
            i = j;
            if doc.nodes.len() > 200_000 {
                break;
            }
        }
    }
    doc
}

// ------------------------------------------------------------------------------------------------- numbers

/// floor, ceil and the remainder with the divisor's sign, without the standard library (for numbers a picture's
/// size could be).
fn floor(x: f32) -> f32 {
    if !x.is_finite() || x.abs() >= 8_388_608.0 {
        return x;
    }
    let t = x as i32 as f32;
    if t > x { t - 1.0 } else { t }
}

fn ceil(x: f32) -> f32 {
    -floor(-x)
}

fn rem_euclid(a: f32, b: f32) -> f32 {
    let r = a % b;
    if r < 0.0 { r + b } else { r }
}

/// Reads numbers as SVG writes them: "10-5.5.5e2" is 10, -5.5 and 50.
struct Numbers<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Numbers<'a> {
    fn new(s: &'a str) -> Numbers<'a> {
        Numbers { s: s.as_bytes(), i: 0 }
    }

    fn skip(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }

    fn number(&mut self) -> Option<f32> {
        self.skip();
        let start = self.i;
        let s = self.s;
        let mut j = self.i;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        let mut digits = false;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
            digits = true;
        }
        if j < s.len() && s[j] == b'.' {
            j += 1;
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
                digits = true;
            }
        }
        if !digits {
            return None;
        }
        if j < s.len() && (s[j] == b'e' || s[j] == b'E') {
            let mut k = j + 1;
            if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
                k += 1;
            }
            if k < s.len() && s[k].is_ascii_digit() {
                while k < s.len() && s[k].is_ascii_digit() {
                    k += 1;
                }
                j = k;
            }
        }
        self.i = j;
        core::str::from_utf8(&s[start..j]).ok()?.parse::<f32>().ok().filter(|x| x.is_finite())
    }

    /// An arc's flag: one digit, 0 or 1 (it needs nothing after it).
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let c = *self.s.get(self.i)?;
        if c == b'0' || c == b'1' {
            self.i += 1;
            Some(c == b'1')
        } else {
            None
        }
    }

    fn done(&mut self) -> bool {
        self.skip();
        self.i >= self.s.len()
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.s.get(self.i).copied()
    }
}

/// A length's number of user units (px: CSS pixels), `pct` what 100% is.
fn length(s: &str, pct: f32, em: f32) -> Option<f32> {
    let s = s.trim();
    let mut n = Numbers::new(s);
    let v = n.number()?;
    let unit = core::str::from_utf8(&n.s[n.i..]).ok()?.trim();
    Some(match unit {
        "" | "px" => v,
        "%" => v * pct / 100.0,
        "pt" => v * 96.0 / 72.0,
        "pc" => v * 16.0,
        "mm" => v * 96.0 / 25.4,
        "cm" => v * 96.0 / 2.54,
        "in" => v * 96.0,
        "em" | "rem" => v * em,
        "ex" => v * em / 2.0,
        _ => return None,
    })
}

// --------------------------------------------------------------------------------------------- transforms

/// An affine transform: x' = a x + c y + e, y' = b x + d y + f.
#[derive(Clone, Copy, Debug)]
struct Matrix([f32; 6]);

impl Matrix {
    const ID: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// self ∘ m: m applied first.
    fn then(&self, m: &Matrix) -> Matrix {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = m.0;
        Matrix([a * a2 + c * b2, b * a2 + d * b2, a * c2 + c * d2, b * c2 + d * d2, a * e2 + c * f2 + e, b * e2 + d * f2 + f])
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }

    fn translate(x: f32, y: f32) -> Matrix {
        Matrix([1.0, 0.0, 0.0, 1.0, x, y])
    }

    fn scale(x: f32, y: f32) -> Matrix {
        Matrix([x, 0.0, 0.0, y, 0.0, 0.0])
    }

    fn invert(&self) -> Option<Matrix> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-12 {
            return None;
        }
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        Some(Matrix([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)]))
    }

    /// How much it scales lengths (on average).
    fn scale_of(&self) -> f32 {
        let [a, b, c, d, _, _] = self.0;
        libm_sqrt((a * d - b * c).abs())
    }
}

fn libm_sqrt(x: f32) -> f32 {
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
fn sin_cos(x: f32) -> (f32, f32) {
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
fn atan2(y: f32, x: f32) -> f32 {
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

fn transform(s: &str) -> Matrix {
    let mut m = Matrix::ID;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let Some(close) = rest[open..].find(')') else { break };
        let mut nums = Numbers::new(&rest[open + 1..open + close]);
        let mut v: Vec<f32> = Vec::new();
        while let Some(x) = nums.number() {
            v.push(x);
        }
        let arg = |i: usize, d: f32| v.get(i).copied().unwrap_or(d);
        let t = match name {
            "matrix" if v.len() == 6 => Matrix([v[0], v[1], v[2], v[3], v[4], v[5]]),
            "translate" => Matrix::translate(arg(0, 0.0), arg(1, 0.0)),
            "scale" => Matrix::scale(arg(0, 1.0), arg(1, arg(0, 1.0))),
            "rotate" => {
                let (s, c) = sin_cos(arg(0, 0.0).to_radians());
                let r = Matrix([c, s, -s, c, 0.0, 0.0]);
                if v.len() >= 3 { Matrix::translate(v[1], v[2]).then(&r).then(&Matrix::translate(-v[1], -v[2])) } else { r }
            }
            "skewx" | "skewX" => {
                let (s, c) = sin_cos(arg(0, 0.0).to_radians());
                Matrix([1.0, 0.0, s / c, 1.0, 0.0, 0.0])
            }
            "skewy" | "skewY" => {
                let (s, c) = sin_cos(arg(0, 0.0).to_radians());
                Matrix([1.0, s / c, 0.0, 1.0, 0.0, 0.0])
            }
            _ => Matrix::ID,
        };
        m = m.then(&t);
        rest = &rest[open + close + 1..];
    }
    m
}

// ----------------------------------------------------------------------------------------------------- colours

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
type Rgba = [f32; 4];

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
fn color(s: &str) -> Option<Rgba> {
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

// ------------------------------------------------------------------------------------------------------- style

#[derive(Clone, Debug)]
enum Paint {
    None,
    Color(Rgba),
    Current,
    /// A gradient's id, and what to use if there's no such gradient.
    Url(String, Option<Rgba>),
}

fn paint(s: &str) -> Option<Paint> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") {
        return Some(Paint::None);
    }
    if t.eq_ignore_ascii_case("currentcolor") {
        return Some(Paint::Current);
    }
    if let Some(rest) = t.strip_prefix("url(") {
        let close = rest.find(')')?;
        let id = rest[..close].trim().trim_matches(['"', '\'']).trim_start_matches('#').to_string();
        let fallback = color(rest[close + 1..].trim());
        return Some(Paint::Url(id, fallback));
    }
    color(t).map(Paint::Color)
}

/// What an element is drawn with (inherited from its parent's, then its own).
#[derive(Clone)]
struct Style {
    fill: Paint,
    fill_opacity: f32,
    even_odd: bool,
    stroke: Paint,
    stroke_opacity: f32,
    stroke_width: f32,
    /// 0 butt, 1 round, 2 square.
    cap: u8,
    /// 0 miter, 1 round, 2 bevel.
    join: u8,
    miter_limit: f32,
    dashes: Vec<f32>,
    dash_offset: f32,
    color: Rgba,
    /// Its own (not inherited): multiplied into the opacity of what's inside it.
    opacity: f32,
    visible: bool,
    display: bool,
    font_size: f32,
    stop_color: Rgba,
    stop_opacity: f32,
}

impl Style {
    fn initial() -> Style {
        Style {
            fill: Paint::Color([0.0, 0.0, 0.0, 1.0]),
            fill_opacity: 1.0,
            even_odd: false,
            stroke: Paint::None,
            stroke_opacity: 1.0,
            stroke_width: 1.0,
            cap: 0,
            join: 0,
            miter_limit: 4.0,
            dashes: Vec::new(),
            dash_offset: 0.0,
            color: [0.0, 0.0, 0.0, 1.0],
            opacity: 1.0,
            visible: true,
            display: true,
            font_size: 16.0,
            stop_color: [0.0, 0.0, 0.0, 1.0],
            stop_opacity: 1.0,
        }
    }

    /// An element's style starting from its parent's: what's inherited kept, the rest as it starts.
    fn child(&self) -> Style {
        let mut s = self.clone();
        s.opacity = 1.0;
        s.display = true;
        s.stop_color = [0.0, 0.0, 0.0, 1.0];
        s.stop_opacity = 1.0;
        s
    }

    fn set(&mut self, name: &str, value: &str, viewport: (f32, f32)) {
        let v = value.trim();
        if v.eq_ignore_ascii_case("inherit") {
            return;
        }
        let num = |v: &str| -> Option<f32> {
            if let Some(p) = v.strip_suffix('%') { p.trim().parse::<f32>().ok().map(|x| x / 100.0) } else { v.parse::<f32>().ok() }
        };
        let diag = libm_sqrt((viewport.0 * viewport.0 + viewport.1 * viewport.1) / 2.0);
        match name {
            "fill" => {
                if let Some(p) = paint(v) {
                    self.fill = p;
                }
            }
            "stroke" => {
                if let Some(p) = paint(v) {
                    self.stroke = p;
                }
            }
            "fill-opacity" => self.fill_opacity = num(v).unwrap_or(1.0).clamp(0.0, 1.0),
            "stroke-opacity" => self.stroke_opacity = num(v).unwrap_or(1.0).clamp(0.0, 1.0),
            "opacity" => self.opacity = num(v).unwrap_or(1.0).clamp(0.0, 1.0),
            "fill-rule" => self.even_odd = v == "evenodd",
            "stroke-width" => {
                if let Some(w) = length(v, diag, self.font_size) {
                    self.stroke_width = w.max(0.0);
                }
            }
            "stroke-linecap" => self.cap = match v {
                "round" => 1,
                "square" => 2,
                _ => 0,
            },
            "stroke-linejoin" => self.join = match v {
                "round" => 1,
                "bevel" => 2,
                _ => 0,
            },
            "stroke-miterlimit" => self.miter_limit = v.parse().unwrap_or(4.0),
            "stroke-dasharray" => {
                self.dashes = if v == "none" { Vec::new() } else { v.split([',', ' ']).filter(|p| !p.is_empty()).filter_map(|p| length(p, diag, self.font_size)).collect() };
                if self.dashes.len() % 2 == 1 {
                    let again = self.dashes.clone();
                    self.dashes.extend(again);
                }
                if self.dashes.iter().all(|d| *d <= 0.0) {
                    self.dashes.clear();
                }
            }
            "stroke-dashoffset" => self.dash_offset = length(v, diag, self.font_size).unwrap_or(0.0),
            "color" => {
                if let Some(c) = color(v) {
                    self.color = c;
                }
            }
            "visibility" => self.visible = v == "visible",
            "display" => self.display = v != "none",
            "font-size" => {
                if let Some(f) = length(v, self.font_size, self.font_size) {
                    self.font_size = f;
                }
            }
            "stop-color" => {
                if v.eq_ignore_ascii_case("currentcolor") {
                    self.stop_color = self.color;
                } else if let Some(c) = color(v) {
                    self.stop_color = c;
                }
            }
            "stop-opacity" => self.stop_opacity = num(v).unwrap_or(1.0).clamp(0.0, 1.0),
            _ => {}
        }
    }
}

/// The properties a style sets, by name, in a declaration list ("fill: red; stroke: none").
fn declarations(s: &str) -> Vec<(String, String)> {
    s.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let v = v.trim().trim_end_matches("!important").trim();
            Some((k.trim().to_ascii_lowercase(), v.to_string()))
        })
        .collect()
}

/// One compound selector: an element's name (None: any), its id, its classes; and how it's tied to the one before
/// (true: a child of it; false: inside it).
struct Compound {
    name: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    child: bool,
}

struct Rule {
    selector: Vec<Compound>,
    /// (ids, classes, names), for which rule wins.
    specificity: (u32, u32, u32),
    order: usize,
    decls: Vec<(String, String)>,
}

fn selector(s: &str) -> Option<Vec<Compound>> {
    let mut out: Vec<Compound> = Vec::new();
    let mut child = false;
    for part in s.split_whitespace() {
        if part == ">" {
            child = true;
            continue;
        }
        if part.contains([':', '[', '+', '~']) {
            return None;
        }
        let mut c = Compound { name: None, id: None, classes: Vec::new(), child };
        child = false;
        let mut rest = part;
        let end = rest.find(['.', '#']).unwrap_or(rest.len());
        let name = &rest[..end];
        if !name.is_empty() && name != "*" {
            c.name = Some(name.to_ascii_lowercase());
        }
        rest = &rest[end..];
        while !rest.is_empty() {
            let kind = rest.as_bytes()[0];
            let end = rest[1..].find(['.', '#']).map_or(rest.len(), |k| k + 1);
            let v = rest[1..end].to_string();
            if kind == b'#' {
                c.id = Some(v);
            } else {
                c.classes.push(v);
            }
            rest = &rest[end..];
        }
        out.push(c);
    }
    if out.is_empty() { None } else { Some(out) }
}

/// A <style>'s rules (those with simple selectors: names, ids, classes, descendants and children).
fn rules(css: &str, out: &mut Vec<Rule>) {
    let mut rest = css;
    // (Comments out first.)
    let mut text = String::new();
    while let Some(i) = rest.find("/*") {
        text.push_str(&rest[..i]);
        rest = rest[i + 2..].find("*/").map_or("", |k| &rest[i + 2 + k + 2..]);
    }
    text.push_str(rest);
    let mut rest = text.as_str();
    while let Some(open) = rest.find('{') {
        let head = rest[..open].trim();
        // An at-rule's block (and what's nested in it) is passed over.
        let mut depth = 1;
        let mut k = open + 1;
        let bytes = rest.as_bytes();
        while k < bytes.len() && depth > 0 {
            match bytes[k] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            k += 1;
        }
        let body = if depth == 0 { &rest[open + 1..k - 1] } else { &rest[open + 1..] };
        if !head.starts_with('@') {
            let decls = declarations(body);
            for sel in head.split(',') {
                if let Some(s) = selector(sel.trim()) {
                    let specificity = s.iter().fold((0, 0, 0), |(a, b, c), x| (a + x.id.is_some() as u32, b + x.classes.len() as u32, c + x.name.is_some() as u32));
                    out.push(Rule { selector: s, specificity, order: out.len(), decls: decls.clone() });
                }
            }
        }
        rest = &rest[k.min(rest.len())..];
    }
}

fn compound_matches(doc: &Doc, n: usize, c: &Compound) -> bool {
    let node = &doc.nodes[n];
    if c.name.as_ref().is_some_and(|name| *name != node.name) {
        return false;
    }
    if c.id.as_ref().is_some_and(|id| node.attr("id") != Some(id.as_str())) {
        return false;
    }
    let classes = node.attr("class").unwrap_or("");
    c.classes.iter().all(|k| classes.split_whitespace().any(|x| x == k))
}

fn matches(doc: &Doc, n: usize, sel: &[Compound]) -> bool {
    let Some((last, before)) = sel.split_last() else { return false };
    if !compound_matches(doc, n, last) {
        return false;
    }
    let mut at = n;
    let mut child = last.child;
    for c in before.iter().rev() {
        loop {
            if at == 0 {
                return false;
            }
            at = doc.nodes[at].parent;
            if at == 0 && doc.nodes[0].name.is_empty() {
                return false;
            }
            if compound_matches(doc, at, c) {
                break;
            }
            if child {
                return false;
            }
        }
        child = c.child;
    }
    true
}

/// The properties SVG lets an element's attributes set.
const PRESENTATION: &[&str] = &[
    "fill", "fill-opacity", "fill-rule", "stroke", "stroke-width", "stroke-opacity", "stroke-linecap", "stroke-linejoin",
    "stroke-miterlimit", "stroke-dasharray", "stroke-dashoffset", "opacity", "color", "visibility", "display", "font-size",
    "stop-color", "stop-opacity",
];

// -------------------------------------------------------------------------------------------------------- shapes

/// A shape's outline, in the picture's pixels: closed polygons (and open polylines, for strokes).
struct Outline {
    /// Each subpath's points, and whether it's closed.
    paths: Vec<(Vec<(f32, f32)>, bool)>,
}

/// Builds an outline from path commands in user units, flattened in the picture's pixels.
struct Builder {
    m: Matrix,
    paths: Vec<(Vec<(f32, f32)>, bool)>,
    cur: Vec<(f32, f32)>,
    /// The pen, the subpath's start, and the last control point (for S and T), in user units.
    pen: (f32, f32),
    start: (f32, f32),
    tolerance: f32,
    points: usize,
}

/// The most points one shape's outline has (past them, it's left as it is).
const MOST_POINTS: usize = 1 << 20;

impl Builder {
    fn new(m: Matrix) -> Builder {
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

    fn move_to(&mut self, p: (f32, f32)) {
        self.flush(false);
        self.pen = p;
        self.start = p;
        self.point(p);
    }

    fn line_to(&mut self, p: (f32, f32)) {
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

    fn cubic_to(&mut self, c1: (f32, f32), c2: (f32, f32), p: (f32, f32)) {
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

    fn quad_to(&mut self, c: (f32, f32), p: (f32, f32)) {
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
    fn arc_to(&mut self, rx: f32, ry: f32, rot: f32, large: bool, sweep: bool, p: (f32, f32)) {
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

    fn close(&mut self) {
        self.pen = self.start;
        self.flush(true);
    }

    fn finish(mut self) -> Outline {
        self.flush(false);
        Outline { paths: self.paths }
    }
}

/// A path's `d`, drawn into a builder.
fn path_data(d: &str, b: &mut Builder) {
    let mut n = Numbers::new(d);
    let mut cmd = b'M';
    let mut last_ctrl: Option<(f32, f32)> = None;
    let mut last_cmd = b' ';
    while let Some(c) = n.peek() {
        if c.is_ascii_alphabetic() {
            cmd = c;
            n.i += 1;
        } else if cmd == b'Z' || cmd == b'z' {
            // (Numbers after Z: a stray; stop.)
            break;
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { b.pen } else { (0.0, 0.0) };
        let up = cmd.to_ascii_uppercase();
        let ok = (|| -> Option<()> {
            match up {
                b'M' => {
                    let p = (n.number()? + ox, n.number()? + oy);
                    b.move_to(p);
                    // (More pairs after M are lines.)
                    cmd = if rel { b'l' } else { b'L' };
                }
                b'L' => {
                    let p = (n.number()? + ox, n.number()? + oy);
                    b.line_to(p);
                }
                b'H' => {
                    let x = n.number()? + ox;
                    let y = b.pen.1;
                    b.line_to((x, y));
                }
                b'V' => {
                    let y = n.number()? + oy;
                    let x = b.pen.0;
                    b.line_to((x, y));
                }
                b'C' => {
                    let c1 = (n.number()? + ox, n.number()? + oy);
                    let c2 = (n.number()? + ox, n.number()? + oy);
                    let p = (n.number()? + ox, n.number()? + oy);
                    b.cubic_to(c1, c2, p);
                    last_ctrl = Some(c2);
                }
                b'S' => {
                    let c2 = (n.number()? + ox, n.number()? + oy);
                    let p = (n.number()? + ox, n.number()? + oy);
                    let c1 = match (last_ctrl, last_cmd) {
                        (Some(lc), b'C' | b'S') => (2.0 * b.pen.0 - lc.0, 2.0 * b.pen.1 - lc.1),
                        _ => b.pen,
                    };
                    b.cubic_to(c1, c2, p);
                    last_ctrl = Some(c2);
                }
                b'Q' => {
                    let c1 = (n.number()? + ox, n.number()? + oy);
                    let p = (n.number()? + ox, n.number()? + oy);
                    b.quad_to(c1, p);
                    last_ctrl = Some(c1);
                }
                b'T' => {
                    let p = (n.number()? + ox, n.number()? + oy);
                    let c1 = match (last_ctrl, last_cmd) {
                        (Some(lc), b'Q' | b'T') => (2.0 * b.pen.0 - lc.0, 2.0 * b.pen.1 - lc.1),
                        _ => b.pen,
                    };
                    b.quad_to(c1, p);
                    last_ctrl = Some(c1);
                }
                b'A' => {
                    let rx = n.number()?;
                    let ry = n.number()?;
                    let rot = n.number()?;
                    let large = n.flag()?;
                    let sweep = n.flag()?;
                    let p = (n.number()? + ox, n.number()? + oy);
                    b.arc_to(rx, ry, rot, large, sweep, p);
                }
                b'Z' => b.close(),
                _ => return None,
            }
            Some(())
        })();
        if ok.is_none() {
            break;
        }
        last_cmd = up;
        if !matches!(up, b'C' | b'S' | b'Q' | b'T') {
            last_ctrl = None;
        }
        if up == b'Z' {
            // (A Z needs no numbers: the next letter follows it.)
            if let Some(c) = n.peek()
                && !c.is_ascii_alphabetic()
            {
                break;
            }
        }
        if n.done() {
            break;
        }
    }
}

// --------------------------------------------------------------------------------------------------- coverage

/// Each pixel's covered area, accumulated from the edges crossing its row (signed by which way they go): the sum
/// along a row is how much of each pixel is inside.
struct Coverage {
    w: usize,
    h: usize,
    /// Rows `w + 2` long (an edge past the right is kept off the next row).
    a: Vec<f32>,
    /// Where the area is in the picture, and its size.
    x0: i32,
    y0: i32,
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

    fn polygon(&mut self, pts: &[(f32, f32)]) {
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
fn positive(mut pts: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
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

fn circle(c: (f32, f32), r: f32) -> Vec<(f32, f32)> {
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
fn stroke(outline: &Outline, width: f32, cap: u8, join: u8, miter: f32, dashes: &[f32], offset: f32) -> Vec<Vec<(f32, f32)>> {
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

// ---------------------------------------------------------------------------------------------------- gradients

struct Gradient {
    radial: bool,
    /// x1 y1 x2 y2 (linear) or cx cy r fx fy (radial), in its units.
    at: [f32; 5],
    user_units: bool,
    m: Matrix,
    /// 0 pad, 1 reflect, 2 repeat.
    spread: u8,
    stops: Vec<(f32, Rgba)>,
}

impl Gradient {
    fn color(&self, t: f32) -> Rgba {
        let t = match self.spread {
            1 => {
                let f = rem_euclid(t, 2.0);
                if f > 1.0 { 2.0 - f } else { f }
            }
            2 => rem_euclid(t, 1.0),
            _ => t.clamp(0.0, 1.0),
        };
        let stops = &self.stops;
        if t <= stops[0].0 {
            return stops[0].1;
        }
        for w in stops.windows(2) {
            if t <= w[1].0 {
                let span = w[1].0 - w[0].0;
                let f = if span > 0.0 { (t - w[0].0) / span } else { 1.0 };
                let (a, b) = (w[0].1, w[1].1);
                return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f, a[3] + (b[3] - a[3]) * f];
            }
        }
        stops[stops.len() - 1].1
    }
}

// ------------------------------------------------------------------------------------------------------ drawing

struct Canvas {
    w: usize,
    h: usize,
    /// Premultiplied RGBA, 0 to 1.
    px: Vec<[f32; 4]>,
}

struct Painter<'a> {
    doc: &'a Doc,
    rules: Vec<Rule>,
    canvas: Canvas,
    /// Elements by id.
    ids: Vec<(String, usize)>,
    /// `use`s being drawn (so one that uses itself stops).
    using: Vec<usize>,
    shapes: usize,
    /// Pixels the shapes' coverage may still be worked out for (a picture with very many big shapes stops).
    budget: u64,
}

const MOST_SHAPES: usize = 20_000;
const MOST_PIXELS_WORKED: u64 = 400_000_000;

impl Painter<'_> {
    fn by_id(&self, id: &str) -> Option<usize> {
        self.ids.iter().find(|(k, _)| k == id).map(|(_, n)| *n)
    }

    /// An element's style: its parent's (inherited), then its attributes, its <style>'s rules, its style attribute.
    fn style_of(&self, n: usize, parent: &Style, viewport: (f32, f32)) -> Style {
        let node = &self.doc.nodes[n];
        let mut s = parent.child();
        // (currentColor's colour first: the others may use it.)
        if let Some(c) = node.attr("color") {
            s.set("color", c, viewport);
        }
        for (k, v) in &node.attrs {
            if PRESENTATION.contains(&k.as_str()) {
                s.set(k, v, viewport);
            }
        }
        let mut matched: Vec<&Rule> = self.rules.iter().filter(|r| matches(self.doc, n, &r.selector)).collect();
        matched.sort_by(|a, b| a.specificity.cmp(&b.specificity).then(a.order.cmp(&b.order)));
        for r in matched {
            for (k, v) in &r.decls {
                s.set(k, v, viewport);
            }
        }
        if let Some(st) = node.attr("style") {
            for (k, v) in declarations(st) {
                s.set(&k, &v, viewport);
            }
        }
        s
    }

    fn num(&self, n: usize, name: &str, pct: f32, em: f32) -> Option<f32> {
        self.doc.nodes[n].attr(name).and_then(|v| length(v, pct, em))
    }

    /// A gradient (and those it takes its stops and attributes from, by its href).
    fn gradient(&self, id: &str, viewport: (f32, f32)) -> Option<Gradient> {
        let mut n = self.by_id(id)?;
        let first = n;
        let radial = match self.doc.nodes[n].name.as_str() {
            "lineargradient" => false,
            "radialgradient" => true,
            _ => return None,
        };
        let mut g = Gradient { radial, at: if radial { [0.5, 0.5, 0.5, f32::NAN, f32::NAN] } else { [0.0, 0.0, 1.0, 0.0, 0.0] }, user_units: false, m: Matrix::ID, spread: 0, stops: Vec::new() };
        let names: [&str; 5] = if radial { ["cx", "cy", "r", "fx", "fy"] } else { ["x1", "y1", "x2", "y2", ""] };
        let mut set = [false; 5];
        let (mut units_set, mut m_set, mut spread_set) = (false, false, false);
        for _ in 0..8 {
            let node = &self.doc.nodes[n];
            if !units_set && let Some(u) = node.attr("gradientunits") {
                g.user_units = u == "userSpaceOnUse";
                units_set = true;
            }
            if !m_set && let Some(t) = node.attr("gradienttransform") {
                g.m = transform(t);
                m_set = true;
            }
            if !spread_set && let Some(s) = node.attr("spreadmethod") {
                g.spread = match s {
                    "reflect" => 1,
                    "repeat" => 2,
                    _ => 0,
                };
                spread_set = true;
            }
            for (i, name) in names.iter().enumerate() {
                if !set[i] && !name.is_empty() && let Some(v) = node.attr(name) {
                    let v = v.trim();
                    g.at[i] = if let Some(p) = v.strip_suffix('%') { p.parse::<f32>().unwrap_or(0.0) / 100.0 } else { length(v, 1.0, 16.0).unwrap_or(0.0) };
                    set[i] = true;
                }
            }
            if g.stops.is_empty() {
                let mut style = Style::initial();
                for &c in &node.children {
                    if self.doc.nodes[c].name != "stop" {
                        continue;
                    }
                    let st = self.style_of(c, &style, viewport);
                    let off = self.doc.nodes[c].attr("offset").map_or(0.0, |o| {
                        let o = o.trim();
                        if let Some(p) = o.strip_suffix('%') { p.parse::<f32>().unwrap_or(0.0) / 100.0 } else { o.parse::<f32>().unwrap_or(0.0) }
                    });
                    let off = off.clamp(0.0, 1.0).max(g.stops.last().map_or(0.0, |s| s.0));
                    let mut c = st.stop_color;
                    c[3] *= st.stop_opacity;
                    g.stops.push((off, c));
                    style = Style::initial();
                }
            }
            let href = node.attr("href").or_else(|| node.attr("xlink:href")).map(|h| h.trim_start_matches('#').to_string());
            match href.and_then(|h| self.by_id(&h)) {
                Some(next) if next != first && next != n => n = next,
                _ => break,
            }
        }
        if g.stops.is_empty() {
            return None;
        }
        if radial {
            if g.at[3].is_nan() {
                g.at[3] = g.at[0];
            }
            if g.at[4].is_nan() {
                g.at[4] = g.at[1];
            }
        }
        Some(g)
    }

    /// Fills the coverage with a paint (at an opacity), onto the canvas.
    #[allow(clippy::too_many_arguments)]
    fn composite(&mut self, cov: &Coverage, even_odd: bool, paint: &Paint, current: Rgba, alpha: f32, bbox: (f32, f32, f32, f32), m: &Matrix, viewport: (f32, f32)) {
        let work = cov.w as u64 * cov.h as u64;
        if work > self.budget {
            self.budget = 0;
            return;
        }
        self.budget -= work;
        enum Fill {
            Solid(Rgba),
            Grad(Gradient, Matrix),
        }
        let fill = match paint {
            Paint::None => return,
            Paint::Color(c) => Fill::Solid(*c),
            Paint::Current => Fill::Solid(current),
            Paint::Url(id, fallback) => match self.gradient(id, viewport) {
                Some(g) => {
                    // The gradient's space to the picture's: its units' box (the shape's, or the user's), and its own
                    // transform.
                    let units = if g.user_units { Matrix::ID } else { Matrix([bbox.2 - bbox.0, 0.0, 0.0, bbox.3 - bbox.1, bbox.0, bbox.1]) };
                    let to_px = m.then(&units).then(&g.m);
                    match to_px.invert() {
                        Some(inv) if g.stops.len() > 1 => Fill::Grad(g, inv),
                        _ => Fill::Solid(g.stops[g.stops.len() - 1].1),
                    }
                }
                None => match fallback {
                    Some(c) => Fill::Solid(*c),
                    None => return,
                },
            },
        };
        let stride = cov.w + 2;
        for y in 0..cov.h {
            let py = y as i32 + cov.y0;
            if py < 0 || py >= self.canvas.h as i32 {
                continue;
            }
            let mut acc = 0.0f32;
            for x in 0..cov.w {
                acc += cov.a[y * stride + x];
                let px = x as i32 + cov.x0;
                let c = if even_odd {
                    let f = acc.abs() % 2.0;
                    if f > 1.0 { 2.0 - f } else { f }
                } else {
                    acc.abs().min(1.0)
                };
                if c < 1.0 / 512.0 || px < 0 || px >= self.canvas.w as i32 {
                    continue;
                }
                let col = match &fill {
                    Fill::Solid(c) => *c,
                    Fill::Grad(g, inv) => {
                        let (gx, gy) = inv.apply(px as f32 + 0.5, py as f32 + 0.5);
                        let t = if g.radial {
                            let [cx, cy, r, fx, fy] = g.at;
                            if r <= 0.0 {
                                1.0
                            } else if (fx - cx).abs() < 1e-6 && (fy - cy).abs() < 1e-6 {
                                libm_sqrt((gx - cx) * (gx - cx) + (gy - cy) * (gy - cy)) / r
                            } else {
                                // From the focus: where the ray through the point meets the circle.
                                let (dx, dy) = (gx - fx, gy - fy);
                                let (ex, ey) = (fx - cx, fy - cy);
                                let a = dx * dx + dy * dy;
                                let b = 2.0 * (dx * ex + dy * ey);
                                let cc = ex * ex + ey * ey - r * r;
                                let disc = b * b - 4.0 * a * cc;
                                if a <= 0.0 || disc < 0.0 { 0.0 } else { 2.0 * a / (-b + libm_sqrt(disc)) }
                            }
                        } else {
                            let [x1, y1, x2, y2, _] = g.at;
                            let (dx, dy) = (x2 - x1, y2 - y1);
                            let l2 = dx * dx + dy * dy;
                            if l2 <= 0.0 { 1.0 } else { ((gx - x1) * dx + (gy - y1) * dy) / l2 }
                        };
                        g.color(t)
                    }
                };
                let a = col[3] * c * alpha;
                if a <= 0.0 {
                    continue;
                }
                let d = &mut self.canvas.px[py as usize * self.canvas.w + px as usize];
                let k = 1.0 - a;
                *d = [col[0] * a + d[0] * k, col[1] * a + d[1] * k, col[2] * a + d[2] * k, a + d[3] * k];
            }
        }
    }

    /// Draws a shape's outline (filled, then stroked).
    fn shape(&mut self, outline: Outline, st: &Style, m: &Matrix, alpha: f32, viewport: (f32, f32)) {
        self.shapes += 1;
        if self.shapes > MOST_SHAPES || !st.visible {
            return;
        }
        let (cw, ch) = (self.canvas.w as f32, self.canvas.h as f32);
        // The shape's box in user units (for gradients) is worked out from the picture's back again.
        let inv = m.invert().unwrap_or(Matrix::ID);
        let mut ub = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (pts, _) in &outline.paths {
            for p in pts {
                let u = inv.apply(p.0, p.1);
                ub = (ub.0.min(u.0), ub.1.min(u.1), ub.2.max(u.0), ub.3.max(u.1));
            }
        }
        let fill_closed = |o: &Outline| -> Vec<Vec<(f32, f32)>> { o.paths.iter().filter(|(p, _)| p.len() >= 3).map(|(p, _)| p.clone()).collect() };
        let current = st.color;
        // Fill.
        if !matches!(st.fill, Paint::None) {
            let polys = fill_closed(&outline);
            if let Some(cov) = coverage_of(&polys, cw, ch) {
                self.composite(&cov, st.even_odd, &st.fill.clone(), current, alpha * st.fill_opacity, ub, m, viewport);
            }
        }
        // Stroke (its width as the transform scales it).
        if !matches!(st.stroke, Paint::None) && st.stroke_width > 0.0 {
            let scale = m.scale_of();
            let width = (st.stroke_width * scale).max(0.0);
            let dashes: Vec<f32> = st.dashes.iter().map(|d| d * scale).collect();
            let polys = stroke(&outline, width, st.cap, st.join, st.miter_limit, &dashes, st.dash_offset * scale);
            if let Some(cov) = coverage_of(&polys, cw, ch) {
                self.composite(&cov, false, &st.stroke.clone(), current, alpha * st.stroke_opacity, ub, m, viewport);
            }
        }
    }

    /// Draws an element and what's inside it.
    fn element(&mut self, n: usize, parent: &Style, m: Matrix, alpha: f32, viewport: (f32, f32), depth: u32) {
        if depth > 64 || self.shapes > MOST_SHAPES {
            return;
        }
        let node = &self.doc.nodes[n];
        let name = node.name.clone();
        if matches!(name.as_str(), "defs" | "symbol" | "lineargradient" | "radialgradient" | "clippath" | "mask" | "pattern" | "marker" | "style" | "title" | "desc" | "metadata" | "filter" | "text" | "script" | "foreignobject" | "image" | "switch") {
            // (A switch would choose its first child it can draw: rare in pictures.)
            if name != "switch" {
                return;
            }
        }
        let st = self.style_of(n, parent, viewport);
        if !st.display {
            return;
        }
        let mut m = m;
        if let Some(t) = node.attr("transform") {
            m = m.then(&transform(t));
        }
        let alpha = alpha * st.opacity;
        if alpha <= 0.0 {
            return;
        }
        let (vw, vh) = viewport;
        let diag = libm_sqrt((vw * vw + vh * vh) / 2.0);
        let em = st.font_size;
        let mut b = Builder::new(m);
        match name.as_str() {
            "g" | "a" | "switch" => {
                for c in node.children.clone() {
                    self.element(c, &st, m, alpha, viewport, depth + 1);
                }
                return;
            }
            "svg" => {
                // A viewport inside: its place, its size, and its viewBox.
                let x = self.num(n, "x", vw, em).unwrap_or(0.0);
                let y = self.num(n, "y", vh, em).unwrap_or(0.0);
                let w = self.num(n, "width", vw, em).unwrap_or(vw);
                let h = self.num(n, "height", vh, em).unwrap_or(vh);
                let (inner, vp) = viewbox(node, w, h);
                let m = m.then(&Matrix::translate(x, y)).then(&inner);
                for c in node.children.clone() {
                    self.element(c, &st, m, alpha, vp, depth + 1);
                }
                return;
            }
            "use" => {
                let Some(href) = node.attr("href").or_else(|| node.attr("xlink:href")) else { return };
                let Some(target) = self.by_id(href.trim().trim_start_matches('#')) else { return };
                if self.using.contains(&target) {
                    return;
                }
                let x = self.num(n, "x", vw, em).unwrap_or(0.0);
                let y = self.num(n, "y", vh, em).unwrap_or(0.0);
                let m = m.then(&Matrix::translate(x, y));
                self.using.push(target);
                if self.doc.nodes[target].name == "symbol" {
                    let w = self.num(n, "width", vw, em).unwrap_or(vw);
                    let h = self.num(n, "height", vh, em).unwrap_or(vh);
                    let (inner, vp) = viewbox(&self.doc.nodes[target], w, h);
                    let st2 = self.style_of(target, &st, viewport);
                    let m = m.then(&inner);
                    for c in self.doc.nodes[target].children.clone() {
                        self.element(c, &st2, m, alpha, vp, depth + 1);
                    }
                } else {
                    self.element(target, &st, m, alpha, viewport, depth + 1);
                }
                self.using.pop();
                return;
            }
            "path" => {
                if let Some(d) = node.attr("d") {
                    path_data(d, &mut b);
                }
            }
            "rect" => {
                let x = self.num(n, "x", vw, em).unwrap_or(0.0);
                let y = self.num(n, "y", vh, em).unwrap_or(0.0);
                let w = self.num(n, "width", vw, em).unwrap_or(0.0);
                let h = self.num(n, "height", vh, em).unwrap_or(0.0);
                if w <= 0.0 || h <= 0.0 {
                    return;
                }
                let rx = self.num(n, "rx", vw, em);
                let ry = self.num(n, "ry", vh, em);
                let (rx, ry) = match (rx, ry) {
                    (Some(a), Some(b)) => (a, b),
                    (Some(a), None) | (None, Some(a)) => (a, a),
                    _ => (0.0, 0.0),
                };
                let (rx, ry) = (rx.clamp(0.0, w / 2.0), ry.clamp(0.0, h / 2.0));
                if rx > 0.0 && ry > 0.0 {
                    b.move_to((x + rx, y));
                    b.line_to((x + w - rx, y));
                    b.arc_to(rx, ry, 0.0, false, true, (x + w, y + ry));
                    b.line_to((x + w, y + h - ry));
                    b.arc_to(rx, ry, 0.0, false, true, (x + w - rx, y + h));
                    b.line_to((x + rx, y + h));
                    b.arc_to(rx, ry, 0.0, false, true, (x, y + h - ry));
                    b.line_to((x, y + ry));
                    b.arc_to(rx, ry, 0.0, false, true, (x + rx, y));
                } else {
                    b.move_to((x, y));
                    b.line_to((x + w, y));
                    b.line_to((x + w, y + h));
                    b.line_to((x, y + h));
                }
                b.close();
            }
            "circle" | "ellipse" => {
                let cx = self.num(n, "cx", vw, em).unwrap_or(0.0);
                let cy = self.num(n, "cy", vh, em).unwrap_or(0.0);
                let (rx, ry) = if name == "circle" {
                    let r = self.num(n, "r", diag, em).unwrap_or(0.0);
                    (r, r)
                } else {
                    let rx = self.num(n, "rx", vw, em);
                    let ry = self.num(n, "ry", vh, em);
                    (rx.or(ry).unwrap_or(0.0), ry.or(rx).unwrap_or(0.0))
                };
                if rx <= 0.0 || ry <= 0.0 {
                    return;
                }
                b.move_to((cx + rx, cy));
                b.arc_to(rx, ry, 0.0, false, true, (cx - rx, cy));
                b.arc_to(rx, ry, 0.0, false, true, (cx + rx, cy));
                b.close();
            }
            "line" => {
                let x1 = self.num(n, "x1", vw, em).unwrap_or(0.0);
                let y1 = self.num(n, "y1", vh, em).unwrap_or(0.0);
                let x2 = self.num(n, "x2", vw, em).unwrap_or(0.0);
                let y2 = self.num(n, "y2", vh, em).unwrap_or(0.0);
                b.move_to((x1, y1));
                b.line_to((x2, y2));
            }
            "polyline" | "polygon" => {
                let mut nums = Numbers::new(node.attr("points").unwrap_or(""));
                let mut first = true;
                while let (Some(x), Some(y)) = (nums.number(), nums.number()) {
                    if first {
                        b.move_to((x, y));
                        first = false;
                    } else {
                        b.line_to((x, y));
                    }
                }
                if name == "polygon" {
                    b.close();
                }
            }
            _ => {
                // An element this doesn't know: what's inside it, as a group's.
                for c in node.children.clone() {
                    self.element(c, &st, m, alpha, viewport, depth + 1);
                }
                return;
            }
        }
        let outline = b.finish();
        self.shape(outline, &st, &m, alpha, viewport);
    }
}

/// The coverage of polygons (in the picture's pixels), in the box they take in it.
fn coverage_of(polys: &[Vec<(f32, f32)>], cw: f32, ch: f32) -> Option<Coverage> {
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

/// A viewport's viewBox fitted into its width and height (preserveAspectRatio): the transform into it, and the
/// size of its user space (for percentages).
fn viewbox(node: &Node, w: f32, h: f32) -> (Matrix, (f32, f32)) {
    let Some(vb) = node.attr("viewbox") else { return (Matrix::ID, (w, h)) };
    let mut nums = Numbers::new(vb);
    let (Some(x), Some(y), Some(vw), Some(vh)) = (nums.number(), nums.number(), nums.number(), nums.number()) else { return (Matrix::ID, (w, h)) };
    if vw <= 0.0 || vh <= 0.0 {
        return (Matrix::ID, (w, h));
    }
    let par = node.attr("preserveaspectratio").unwrap_or("xMidYMid meet").trim().to_string();
    let mut sx = w / vw;
    let mut sy = h / vh;
    let (mut tx, mut ty) = (-x * sx, -y * sy);
    if !par.starts_with("none") {
        let slice = par.ends_with("slice");
        let s = if slice { sx.max(sy) } else { sx.min(sy) };
        sx = s;
        sy = s;
        let align = par.split_whitespace().next().unwrap_or("xMidYMid");
        let fx = if align.contains("xMin") { 0.0 } else if align.contains("xMax") { 1.0 } else { 0.5 };
        let fy = if align.contains("YMin") { 0.0 } else if align.contains("YMax") { 1.0 } else { 0.5 };
        tx = -x * s + (w - vw * s) * fx;
        ty = -y * s + (h - vh * s) * fy;
    }
    (Matrix([sx, 0.0, 0.0, sy, tx, ty]), (vw, vh))
}

/// The root <svg> element.
fn root(doc: &Doc) -> Option<usize> {
    doc.nodes[0].children.iter().copied().find(|&n| doc.nodes[n].name == "svg")
}

/// Whether bytes are an SVG picture (XML whose first element is <svg>).
pub fn is_svg(data: &[u8]) -> bool {
    let head = &data[..data.len().min(4096)];
    let Ok(text) = core::str::from_utf8(head).or_else(|e| core::str::from_utf8(&head[..e.valid_up_to()])) else { return false };
    let t = text.trim_start_matches('\u{FEFF}').trim_start();
    if !t.starts_with('<') {
        return false;
    }
    // Past the XML declaration, comments and the DOCTYPE, the first element.
    let mut rest = t;
    loop {
        rest = rest.trim_start();
        if rest.starts_with("<?") {
            let Some(k) = rest.find("?>") else { return false };
            rest = &rest[k + 2..];
        } else if rest.starts_with("<!--") {
            let Some(k) = rest.find("-->") else { return false };
            rest = &rest[k + 3..];
        } else if rest.starts_with("<!") {
            let Some(k) = rest.find('>') else { return false };
            rest = &rest[k + 1..];
        } else {
            let name: String = rest[1..].chars().take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/').collect();
            return local(&name) == "svg";
        }
    }
}

/// The size an SVG picture says it is (CSS pixels): its width and height, else its viewBox's (in proportion to the
/// one it has), else 300 × 150.
pub fn size(data: &[u8]) -> Option<(u32, u32)> {
    let text = core::str::from_utf8(data).ok()?;
    let doc = parse(text);
    let r = root(&doc)?;
    let (w, h) = own_size(&doc.nodes[r]);
    Some((((w + 0.5) as u32).max(1), ((h + 0.5) as u32).max(1)))
}

fn own_size(node: &Node) -> (f32, f32) {
    let abs = |name: &str| node.attr(name).filter(|v| !v.trim().ends_with('%')).and_then(|v| length(v, 0.0, 16.0)).filter(|v| *v > 0.0);
    let vb = node.attr("viewbox").and_then(|v| {
        let mut n = Numbers::new(v);
        let (_, _, w, h) = (n.number()?, n.number()?, n.number()?, n.number()?);
        (w > 0.0 && h > 0.0).then_some((w, h))
    });
    match (abs("width"), abs("height"), vb) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some((vw, vh))) => (w, w * vh / vw),
        (None, Some(h), Some((vw, vh))) => (h * vw / vh, h),
        (Some(w), None, None) => (w, 150.0),
        (None, Some(h), None) => (300.0, h),
        (None, None, Some((vw, vh))) => (vw, vh),
        (None, None, None) => (300.0, 150.0),
    }
}

/// An SVG picture drawn `width` × `height` pixels: RGBA, alpha straight. None if it isn't an SVG picture.
pub fn render(data: &[u8], width: u32, height: u32) -> Option<(Vec<u8>, u32, u32)> {
    let text = core::str::from_utf8(data).ok()?;
    let doc = parse(text);
    let r = root(&doc)?;
    let (w, h) = (width.max(1) as usize, height.max(1) as usize);
    let mut rules_list = Vec::new();
    let mut ids = Vec::new();
    for (i, n) in doc.nodes.iter().enumerate() {
        if n.name == "style" {
            rules(&n.text, &mut rules_list);
        }
        if let Some(id) = n.attr("id") {
            ids.push((id.to_string(), i));
        }
    }
    let mut p = Painter { doc: &doc, rules: rules_list, canvas: Canvas { w, h, px: vec![[0.0; 4]; w * h] }, ids, using: Vec::new(), shapes: 0, budget: MOST_PIXELS_WORKED };
    // The root's own size, fitted into the picture; its viewBox into that.
    let (ow, oh) = own_size(&doc.nodes[r]);
    let m = Matrix::scale(w as f32 / ow, h as f32 / oh);
    let (inner, vp) = viewbox(&doc.nodes[r], ow, oh);
    let st = p.style_of(r, &Style::initial(), vp);
    let m = m.then(&inner);
    if st.display {
        for c in doc.nodes[r].children.clone() {
            p.element(c, &st, m, st.opacity, vp, 1);
        }
    }
    let mut out = vec![0u8; w * h * 4];
    for (i, px) in p.canvas.px.iter().enumerate() {
        let a = px[3].clamp(0.0, 1.0);
        let un = |v: f32| if a > 0.0 { ((v / a).clamp(0.0, 1.0) * 255.0 + 0.5) as u8 } else { 0 };
        out[i * 4] = un(px[0]);
        out[i * 4 + 1] = un(px[1]);
        out[i * 4 + 2] = un(px[2]);
        out[i * 4 + 3] = (a * 255.0 + 0.5) as u8;
    }
    Some((out, w as u32, h as u32))
}
