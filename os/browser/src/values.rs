//! CSS values read from their tokens: lengths (and calc(), min(), max(), clamp()), colours (by name, #hex, rgb(),
//! hsl(), hwb(), oklab(), oklch(), lab(), lch(), color-mix()), numbers and angles.

use crate::css::Tok;
use crate::num;
use alloc::vec::Vec;

/// A colour: 0xAARRGGBB.
pub type Color = u32;

pub const TRANSPARENT: Color = 0;
pub const BLACK: Color = 0xFF00_0000;
pub const WHITE: Color = 0xFFFF_FFFF;

pub fn alpha(c: Color) -> u32 {
    c >> 24
}

/// A length: device pixels plus a percentage of what it's a part of (the containing block's width, mostly), as
/// calc() can mix them.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Len {
    pub px: f32,
    pub pct: f32,
}

impl Len {
    pub const ZERO: Len = Len { px: 0.0, pct: 0.0 };

    pub fn px(px: f32) -> Len {
        Len { px, pct: 0.0 }
    }

    pub fn pct(pct: f32) -> Len {
        Len { px: 0.0, pct }
    }

    pub fn resolve(self, base: f32) -> f32 {
        self.px + self.pct * base / 100.0
    }

    pub fn has_pct(self) -> bool {
        self.pct != 0.0
    }
}

/// What lengths are measured against: the font's size (em), the root's (rem), the window (vw, vh), and how many
/// device pixels a CSS pixel is.
#[derive(Clone, Copy, Debug)]
pub struct Units {
    pub em: f32,
    pub rem: f32,
    pub vw: f32,
    pub vh: f32,
    pub dpr: f32,
    /// The font's x-height, the width of its "0", its cap height and the width of its ideographs, as parts of the
    /// em (ex, ch, cap, ic); and the root's (rex, rch, rcap, ric, as parts of rem).
    pub font: [f32; 4],
    pub root: [f32; 4],
}

impl Units {
    /// Units with a font's proportions as Times New Roman's (Chrome's default font).
    pub fn new(em: f32, rem: f32, vw: f32, vh: f32, dpr: f32) -> Units {
        let font = text::Family::Serif.units();
        Units { em, rem, vw, vh, dpr, font, root: font }
    }
}

/// A number with what it is: a length, a percentage (of the length's base), or a plain number.
#[derive(Clone, Copy, Debug)]
enum Q {
    Num(f32),
    Len(Len),
}

/// A length from one token, in device pixels (or a percentage).
pub fn length_tok(t: &Tok, u: &Units) -> Option<Len> {
    match t {
        Tok::Dimension(n, unit) => Some(Len::px(unit_px(*n, unit, u)?)),
        Tok::Percent(p) => Some(Len::pct(*p)),
        Tok::Number(n) if *n == 0.0 => Some(Len::ZERO),
        _ => None,
    }
}

fn unit_px(n: f32, unit: &str, u: &Units) -> Option<f32> {
    Some(match unit {
        "px" => n * u.dpr,
        "em" => n * u.em,
        "rem" => n * u.rem,
        "ex" => n * u.em * u.font[0],
        "ch" => n * u.em * u.font[1],
        "cap" => n * u.em * u.font[2],
        // (The width of 水, a CJK ideograph: an em in nearly every font that has one.)
        "ic" => n * u.em * u.font[3],
        "rex" => n * u.rem * u.root[0],
        "rch" => n * u.rem * u.root[1],
        "rcap" => n * u.rem * u.root[2],
        "ric" => n * u.rem * u.root[3],

        "lh" => n * u.em * 1.2,
        "rlh" => n * u.rem * 1.2,
        "vw" | "svw" | "lvw" | "dvw" | "cqw" | "cqi" => n * u.vw / 100.0,
        "vh" | "svh" | "lvh" | "dvh" | "cqh" | "cqb" => n * u.vh / 100.0,
        "vmin" | "svmin" | "lvmin" | "dvmin" | "cqmin" => n * u.vw.min(u.vh) / 100.0,
        "vmax" | "svmax" | "lvmax" | "dvmax" | "cqmax" => n * u.vw.max(u.vh) / 100.0,
        "pt" => n * u.dpr * 4.0 / 3.0,
        "pc" => n * u.dpr * 16.0,
        "in" => n * u.dpr * 96.0,
        "cm" => n * u.dpr * 96.0 / 2.54,
        "mm" => n * u.dpr * 96.0 / 25.4,
        "q" => n * u.dpr * 96.0 / 101.6,
        _ => return None,
    })
}

/// A length from a value's tokens: one token, or calc() and its kin. `base` guesses what a percentage is of, where
/// min() and max() must compare one with a length.
pub fn length(t: &[Tok], u: &Units, base: f32) -> Option<Len> {
    let t = trim(t);
    match t {
        [one] => length_tok(one, u),
        [Tok::Function(f), rest @ ..] if is_math(f) && rest.last() == Some(&Tok::CloseParen) => {
            match math(f, &rest[..rest.len() - 1], u, base)? {
                Q::Len(l) => Some(l),
                Q::Num(n) if n == 0.0 => Some(Len::ZERO),
                Q::Num(_) => None,
            }
        }
        _ => None,
    }
}

/// A plain number from a value's tokens (calc() too).
pub fn number(t: &[Tok], u: &Units) -> Option<f32> {
    let t = trim(t);
    match t {
        [Tok::Number(n)] => Some(*n),
        [Tok::Function(f), rest @ ..] if is_math(f) && rest.last() == Some(&Tok::CloseParen) => {
            match math(f, &rest[..rest.len() - 1], u, 0.0)? {
                Q::Num(n) => Some(n),
                Q::Len(_) => None,
            }
        }
        _ => None,
    }
}

fn is_math(f: &str) -> bool {
    matches!(f, "calc" | "min" | "max" | "clamp" | "-webkit-calc" | "-moz-calc")
}

pub fn trim(t: &[Tok]) -> &[Tok] {
    let mut a = 0;
    let mut b = t.len();
    while a < b && t[a] == Tok::Ws {
        a += 1;
    }
    while b > a && t[b - 1] == Tok::Ws {
        b -= 1;
    }
    &t[a..b]
}

/// calc(), min(), max(), clamp(), over the tokens inside them.
fn math(f: &str, inner: &[Tok], u: &Units, base: f32) -> Option<Q> {
    match f {
        "min" | "max" | "clamp" => {
            let args: Vec<Q> = crate::css::split_commas(inner).iter().map(|a| expr(a, u, base)).collect::<Option<_>>()?;
            let value = |q: &Q| match q {
                Q::Num(n) => *n,
                Q::Len(l) => l.resolve(base),
            };
            let pick = |a: Q, b: Q, less: bool| if (value(&a) < value(&b)) == less { a } else { b };
            match f {
                "min" => args.into_iter().reduce(|a, b| pick(a, b, true)),
                "max" => args.into_iter().reduce(|a, b| pick(a, b, false)),
                _ => {
                    if args.len() != 3 {
                        return None;
                    }
                    // clamp(lo, x, hi) = max(lo, min(x, hi)).
                    Some(pick(args[0], pick(args[1], args[2], true), false))
                }
            }
        }
        _ => expr(inner, u, base),
    }
}

/// A sum of products ("a + b * c - d / 2").
fn expr(t: &[Tok], u: &Units, base: f32) -> Option<Q> {
    let t = trim(t);
    // Split at top-level + and - (which calc() wants spaces around; "-" right after a space and before a number is a
    // sign of that number, which the tokens have already).
    let mut terms: Vec<(f32, &[Tok])> = Vec::new();
    let mut sign = 1.0f32;
    let mut start = 0;
    let mut depth = 0;
    for (i, tok) in t.iter().enumerate() {
        match tok {
            Tok::Function(_) | Tok::OpenParen => depth += 1,
            Tok::CloseParen => depth -= 1,
            Tok::Delim(c) if depth == 0 && (*c == '+' || *c == '-') => {
                terms.push((sign, &t[start..i]));
                sign = if *c == '+' { 1.0 } else { -1.0 };
                start = i + 1;
            }
            _ => {}
        }
    }
    terms.push((sign, &t[start..]));
    let mut total: Option<Q> = None;
    for (s, term) in terms {
        let term = trim(term);
        if term.is_empty() {
            continue;
        }
        let v = product(term, u, base)?;
        let v = scale(v, s);
        total = Some(match total {
            None => v,
            Some(acc) => add(acc, v)?,
        });
    }
    total
}

fn scale(q: Q, s: f32) -> Q {
    match q {
        Q::Num(n) => Q::Num(n * s),
        Q::Len(l) => Q::Len(Len { px: l.px * s, pct: l.pct * s }),
    }
}

fn add(a: Q, b: Q) -> Option<Q> {
    Some(match (a, b) {
        (Q::Num(x), Q::Num(y)) => Q::Num(x + y),
        (Q::Len(x), Q::Len(y)) => Q::Len(Len { px: x.px + y.px, pct: x.pct + y.pct }),
        // (0 + a length, which calc() wouldn't allow, but harmless.)
        (Q::Num(x), Q::Len(l)) | (Q::Len(l), Q::Num(x)) if x == 0.0 => Q::Len(l),
        _ => return None,
    })
}

fn product(t: &[Tok], u: &Units, base: f32) -> Option<Q> {
    let mut acc: Option<Q> = None;
    let mut op = '*';
    let mut i = 0;
    while i < t.len() {
        let tok = &t[i];
        if *tok == Tok::Ws {
            i += 1;
            continue;
        }
        if let Tok::Delim(c) = tok
            && (*c == '*' || *c == '/')
        {
            op = *c;
            i += 1;
            continue;
        }
        // One factor: a token, or a bracket, or a function.
        let (v, used) = factor(&t[i..], u, base)?;
        i += used;
        acc = Some(match acc {
            None => v,
            Some(a) => match (op, a, v) {
                ('*', Q::Num(x), Q::Num(y)) => Q::Num(x * y),
                ('*', Q::Num(x), l @ Q::Len(_)) | ('*', l @ Q::Len(_), Q::Num(x)) => scale(l, x),
                ('/', a, Q::Num(y)) if y != 0.0 => scale(a, 1.0 / y),
                _ => return None,
            },
        });
    }
    acc
}

fn factor(t: &[Tok], u: &Units, base: f32) -> Option<(Q, usize)> {
    let first = t.first()?;
    match first {
        Tok::Number(n) => Some((Q::Num(*n), 1)),
        Tok::Dimension(..) | Tok::Percent(_) => Some((Q::Len(length_tok(first, u)?), 1)),
        Tok::Ident(i) => {
            let v = match i.to_ascii_lowercase().as_str() {
                "pi" => core::f32::consts::PI,
                "e" => core::f32::consts::E,
                _ => return None,
            };
            Some((Q::Num(v), 1))
        }
        Tok::OpenParen | Tok::Function(_) => {
            // To its closing bracket.
            let mut depth = 0;
            let mut end = 0;
            for (k, tok) in t.iter().enumerate() {
                match tok {
                    Tok::OpenParen | Tok::Function(_) => depth += 1,
                    Tok::CloseParen => {
                        depth -= 1;
                        if depth == 0 {
                            end = k;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if end == 0 {
                return None;
            }
            let inner = &t[1..end];
            let q = match first {
                Tok::Function(f) if is_math(f) => math(f, inner, u, base)?,
                Tok::OpenParen => expr(inner, u, base)?,
                _ => return None,
            };
            Some((q, end + 1))
        }
        _ => None,
    }
}

// ------------------------------------------------------------------------------------------------ colours

/// A colour from a value's tokens. `current` is what currentColor is.
pub fn color(t: &[Tok], current: Color) -> Option<Color> {
    let t = trim(t);
    match t {
        [Tok::Ident(name)] => named(&name.to_ascii_lowercase(), current),
        [Tok::Hash(h, _)] => hex(h),
        [Tok::Function(f), rest @ ..] if rest.last() == Some(&Tok::CloseParen) => function(f, &rest[..rest.len() - 1], current),
        _ => None,
    }
}

fn hex(h: &str) -> Option<Color> {
    let d: Vec<u32> = h.chars().map(|c| c.to_digit(16)).collect::<Option<_>>()?;
    let (r, g, b, a) = match d.len() {
        3 => (d[0] * 17, d[1] * 17, d[2] * 17, 255),
        4 => (d[0] * 17, d[1] * 17, d[2] * 17, d[3] * 17),
        6 => (d[0] * 16 + d[1], d[2] * 16 + d[3], d[4] * 16 + d[5], 255),
        8 => (d[0] * 16 + d[1], d[2] * 16 + d[3], d[4] * 16 + d[5], d[6] * 16 + d[7]),
        _ => return None,
    };
    Some(rgba(r as f32, g as f32, b as f32, a as f32 / 255.0))
}

pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    let c = |v: f32| num::round(num::clamp(v, 0.0, 255.0)) as u32;
    (c(a * 255.0) << 24) | (c(r) << 16) | (c(g) << 8) | c(b)
}

/// The numbers inside a colour function: commas or spaces between them, and "/ alpha".
fn args(t: &[Tok]) -> Vec<Tok> {
    t.iter().filter(|x| !matches!(x, Tok::Ws | Tok::Comma)).cloned().collect()
}

/// A number from a colour's argument: a number, or a percentage of `full`.
fn channel(t: Option<&Tok>, full: f32) -> Option<f32> {
    match t? {
        Tok::Number(n) => Some(*n),
        Tok::Percent(p) => Some(p / 100.0 * full),
        Tok::Ident(i) if i.eq_ignore_ascii_case("none") => Some(0.0),
        _ => None,
    }
}

/// An angle in degrees (a number is degrees).
pub fn angle(t: &Tok) -> Option<f32> {
    match t {
        Tok::Number(n) => Some(*n),
        Tok::Dimension(n, u) => Some(match u.as_str() {
            "deg" => *n,
            "rad" => n * 180.0 / core::f32::consts::PI,
            "grad" => n * 0.9,
            "turn" => n * 360.0,
            _ => return None,
        }),
        Tok::Ident(i) if i.eq_ignore_ascii_case("none") => Some(0.0),
        _ => None,
    }
}

/// The alpha after "/" (or as a fourth argument), 1 if none.
fn alpha_arg(a: &[Tok], at: usize) -> f32 {
    let t = match a.get(at) {
        Some(Tok::Delim('/')) => a.get(at + 1),
        other => other,
    };
    match t {
        Some(Tok::Number(n)) => num::clamp(*n, 0.0, 1.0),
        Some(Tok::Percent(p)) => num::clamp(p / 100.0, 0.0, 1.0),
        _ => 1.0,
    }
}

fn function(f: &str, inner: &[Tok], current: Color) -> Option<Color> {
    let a = args(inner);
    match f {
        "rgb" | "rgba" => {
            let r = channel(a.first(), 255.0)?;
            let g = channel(a.get(1), 255.0)?;
            let b = channel(a.get(2), 255.0)?;
            Some(rgba(r, g, b, alpha_arg(&a, 3)))
        }
        "hsl" | "hsla" => {
            let h = angle(a.first()?)?;
            let s = channel(a.get(1), 1.0)?.min(1.0).max(0.0);
            let l = channel(a.get(2), 1.0)?.min(1.0).max(0.0);
            // (Numbers without % in the new syntax are percentages too.)
            let (s, l) = (if s > 1.0 { s / 100.0 } else { s }, if l > 1.0 { l / 100.0 } else { l });
            let (r, g, b) = hsl(h, s, l);
            Some(rgba(r * 255.0, g * 255.0, b * 255.0, alpha_arg(&a, 3)))
        }
        "hwb" => {
            let h = angle(a.first()?)?;
            let w = channel(a.get(1), 1.0)?;
            let bl = channel(a.get(2), 1.0)?;
            let (w, bl) = if w + bl >= 1.0 { (w / (w + bl), bl / (w + bl)) } else { (w, bl) };
            let (r, g, b) = hsl(h, 1.0, 0.5);
            let f = |c: f32| (c * (1.0 - w - bl) + w) * 255.0;
            Some(rgba(f(r), f(g), f(b), alpha_arg(&a, 3)))
        }
        "oklab" => {
            let l = channel(a.first(), 1.0)?;
            let x = channel(a.get(1), 0.4)?;
            let y = channel(a.get(2), 0.4)?;
            let (r, g, b) = oklab(l, x, y);
            Some(rgba(r, g, b, alpha_arg(&a, 3)))
        }
        "oklch" => {
            let l = channel(a.first(), 1.0)?;
            let c = channel(a.get(1), 0.4)?;
            let h = angle(a.get(2)?)?.to_radians_f();
            let (r, g, b) = oklab(l, c * num::cos(h), c * num::sin(h));
            Some(rgba(r, g, b, alpha_arg(&a, 3)))
        }
        "lab" => {
            let l = channel(a.first(), 100.0)?;
            let x = channel(a.get(1), 125.0)?;
            let y = channel(a.get(2), 125.0)?;
            let (r, g, b) = lab(l, x, y);
            Some(rgba(r, g, b, alpha_arg(&a, 3)))
        }
        "lch" => {
            let l = channel(a.first(), 100.0)?;
            let c = channel(a.get(1), 150.0)?;
            let h = angle(a.get(2)?)?.to_radians_f();
            let (r, g, b) = lab(l, c * num::cos(h), c * num::sin(h));
            Some(rgba(r, g, b, alpha_arg(&a, 3)))
        }
        "color" => {
            // color(srgb r g b / a) and its kin: taken as sRGB.
            let r = channel(a.get(1), 1.0)?;
            let g = channel(a.get(2), 1.0)?;
            let b = channel(a.get(3), 1.0)?;
            Some(rgba(r * 255.0, g * 255.0, b * 255.0, alpha_arg(&a, 4)))
        }
        "color-mix" => {
            // color-mix(in srgb, A p%, B q%): mixed in sRGB.
            let parts = crate::css::split_commas(inner);
            if parts.len() < 3 {
                return None;
            }
            let side = |p: &[Tok]| -> Option<(Color, Option<f32>)> {
                let p = trim(p);
                let pct = p.iter().find_map(|t| if let Tok::Percent(x) = t { Some(*x / 100.0) } else { None });
                let col: Vec<Tok> = p.iter().filter(|t| !matches!(t, Tok::Percent(_))).cloned().collect();
                Some((color(&col, current)?, pct))
            };
            let (c1, p1) = side(parts[1])?;
            let (c2, p2) = side(parts[2])?;
            let w1 = match (p1, p2) {
                (Some(a), _) => a,
                (None, Some(b)) => 1.0 - b,
                _ => 0.5,
            };
            Some(mix(c1, c2, num::clamp(w1, 0.0, 1.0)))
        }
        "light-dark" => {
            let parts = crate::css::split_commas(inner);
            color(parts.first()?, current)
        }
        "var" => None,
        _ => None,
    }
}

trait Radians {
    fn to_radians_f(self) -> f32;
}

impl Radians for f32 {
    fn to_radians_f(self) -> f32 {
        self * core::f32::consts::PI / 180.0
    }
}

/// `a` mixed with `b`, `w` of it `a`'s.
pub fn mix(a: Color, b: Color, w: f32) -> Color {
    let ch = |c: Color, s: u32| ((c >> s) & 0xFF) as f32;
    let m = |s: u32| ch(a, s) * w + ch(b, s) * (1.0 - w);
    rgba(m(16), m(8), m(0), m(24) / 255.0)
}

fn hsl(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = ((h % 360.0) + 360.0) % 360.0;
    let f = |n: f32| {
        let k = (n + h / 30.0) % 12.0;
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).min(1.0).max(-1.0)
    };
    (f(0.0), f(8.0), f(4.0))
}

/// Linear light to sRGB's 0..255.
fn gamma(x: f32) -> f32 {
    let x = num::clamp(x, 0.0, 1.0);
    255.0 * if x <= 0.003_130_8 { 12.92 * x } else { 1.055 * num::pow(x, 1.0 / 2.4) - 0.055 }
}

fn oklab(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3;
    let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3;
    let bl = -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;
    (gamma(r), gamma(g), gamma(bl))
}

/// CIE Lab (D50), to sRGB.
fn lab(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let inv = |t: f32| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f32 / 29.0) * (6.0 / 29.0) * (t - 4.0 / 29.0) };
    let (x, y, z) = (0.964_22 * inv(fx), inv(fy), 0.825_21 * inv(fz));
    // D50 XYZ to linear sRGB (Bradford-adapted).
    let r = 3.134_136 * x - 1.617_386 * y - 0.490_662 * z;
    let g = -0.978_795 * x + 1.916_254 * y + 0.033_443 * z;
    let bl = 0.071_955 * x - 0.228_977 * y + 1.405_386 * z;
    (gamma(r), gamma(g), gamma(bl))
}

fn named(n: &str, current: Color) -> Option<Color> {
    let rgb: u32 = match n {
        "transparent" => return Some(TRANSPARENT),
        "currentcolor" => return Some(current),
        // System colours, as a light page has them.
        "canvas" | "field" | "buttonhighlight" => 0xFFFFFF,
        "canvastext" | "fieldtext" | "buttontext" | "windowtext" => 0x000000,
        "linktext" => 0x0000EE,
        "visitedtext" => 0x551A8B,
        "activetext" => 0xFF0000,
        "buttonface" | "buttonborder" | "threedface" => 0xEFEFEF,
        "graytext" => 0x6D6D6D,
        "highlight" | "selecteditem" => 0x3367D6,
        "highlighttext" | "selecteditemtext" => 0xFFFFFF,
        "mark" => 0xFFFF00,
        "marktext" => 0x000000,
        "accentcolor" => 0x0075FF,
        "accentcolortext" => 0xFFFFFF,
        _ => return NAMED.binary_search_by(|(k, _)| k.cmp(&n)).ok().map(|i| 0xFF00_0000 | NAMED[i].1),
    };
    Some(0xFF00_0000 | rgb)
}

/// CSS's named colours, in order of their names.
static NAMED: &[(&str, u32)] = &[
    ("aliceblue", 0xF0F8FF), ("antiquewhite", 0xFAEBD7), ("aqua", 0x00FFFF), ("aquamarine", 0x7FFFD4),
    ("azure", 0xF0FFFF), ("beige", 0xF5F5DC), ("bisque", 0xFFE4C4), ("black", 0x000000), ("blanchedalmond", 0xFFEBCD),
    ("blue", 0x0000FF), ("blueviolet", 0x8A2BE2), ("brown", 0xA52A2A), ("burlywood", 0xDEB887), ("cadetblue", 0x5F9EA0),
    ("chartreuse", 0x7FFF00), ("chocolate", 0xD2691E), ("coral", 0xFF7F50), ("cornflowerblue", 0x6495ED),
    ("cornsilk", 0xFFF8DC), ("crimson", 0xDC143C), ("cyan", 0x00FFFF), ("darkblue", 0x00008B), ("darkcyan", 0x008B8B),
    ("darkgoldenrod", 0xB8860B), ("darkgray", 0xA9A9A9), ("darkgreen", 0x006400), ("darkgrey", 0xA9A9A9),
    ("darkkhaki", 0xBDB76B), ("darkmagenta", 0x8B008B), ("darkolivegreen", 0x556B2F), ("darkorange", 0xFF8C00),
    ("darkorchid", 0x9932CC), ("darkred", 0x8B0000), ("darksalmon", 0xE9967A), ("darkseagreen", 0x8FBC8F),
    ("darkslateblue", 0x483D8B), ("darkslategray", 0x2F4F4F), ("darkslategrey", 0x2F4F4F), ("darkturquoise", 0x00CED1),
    ("darkviolet", 0x9400D3), ("deeppink", 0xFF1493), ("deepskyblue", 0x00BFFF), ("dimgray", 0x696969),
    ("dimgrey", 0x696969), ("dodgerblue", 0x1E90FF), ("firebrick", 0xB22222), ("floralwhite", 0xFFFAF0),
    ("forestgreen", 0x228B22), ("fuchsia", 0xFF00FF), ("gainsboro", 0xDCDCDC), ("ghostwhite", 0xF8F8FF),
    ("gold", 0xFFD700), ("goldenrod", 0xDAA520), ("gray", 0x808080), ("green", 0x008000), ("greenyellow", 0xADFF2F),
    ("grey", 0x808080), ("honeydew", 0xF0FFF0), ("hotpink", 0xFF69B4), ("indianred", 0xCD5C5C), ("indigo", 0x4B0082),
    ("ivory", 0xFFFFF0), ("khaki", 0xF0E68C), ("lavender", 0xE6E6FA), ("lavenderblush", 0xFFF0F5),
    ("lawngreen", 0x7CFC00), ("lemonchiffon", 0xFFFACD), ("lightblue", 0xADD8E6), ("lightcoral", 0xF08080),
    ("lightcyan", 0xE0FFFF), ("lightgoldenrodyellow", 0xFAFAD2), ("lightgray", 0xD3D3D3), ("lightgreen", 0x90EE90),
    ("lightgrey", 0xD3D3D3), ("lightpink", 0xFFB6C1), ("lightsalmon", 0xFFA07A), ("lightseagreen", 0x20B2AA),
    ("lightskyblue", 0x87CEFA), ("lightslategray", 0x778899), ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xB0C4DE), ("lightyellow", 0xFFFFE0), ("lime", 0x00FF00), ("limegreen", 0x32CD32),
    ("linen", 0xFAF0E6), ("magenta", 0xFF00FF), ("maroon", 0x800000), ("mediumaquamarine", 0x66CDAA),
    ("mediumblue", 0x0000CD), ("mediumorchid", 0xBA55D3), ("mediumpurple", 0x9370DB), ("mediumseagreen", 0x3CB371),
    ("mediumslateblue", 0x7B68EE), ("mediumspringgreen", 0x00FA9A), ("mediumturquoise", 0x48D1CC),
    ("mediumvioletred", 0xC71585), ("midnightblue", 0x191970), ("mintcream", 0xF5FFFA), ("mistyrose", 0xFFE4E1),
    ("moccasin", 0xFFE4B5), ("navajowhite", 0xFFDEAD), ("navy", 0x000080), ("oldlace", 0xFDF5E6), ("olive", 0x808000),
    ("olivedrab", 0x6B8E23), ("orange", 0xFFA500), ("orangered", 0xFF4500), ("orchid", 0xDA70D6),
    ("palegoldenrod", 0xEEE8AA), ("palegreen", 0x98FB98), ("paleturquoise", 0xAFEEEE), ("palevioletred", 0xDB7093),
    ("papayawhip", 0xFFEFD5), ("peachpuff", 0xFFDAB9), ("peru", 0xCD853F), ("pink", 0xFFC0CB), ("plum", 0xDDA0DD),
    ("powderblue", 0xB0E0E6), ("purple", 0x800080), ("rebeccapurple", 0x663399), ("red", 0xFF0000),
    ("rosybrown", 0xBC8F8F), ("royalblue", 0x4169E1), ("saddlebrown", 0x8B4513), ("salmon", 0xFA8072),
    ("sandybrown", 0xF4A460), ("seagreen", 0x2E8B57), ("seashell", 0xFFF5EE), ("sienna", 0xA0522D), ("silver", 0xC0C0C0),
    ("skyblue", 0x87CEEB), ("slateblue", 0x6A5ACD), ("slategray", 0x708090), ("slategrey", 0x708090), ("snow", 0xFFFAFA),
    ("springgreen", 0x00FF7F), ("steelblue", 0x4682B4), ("tan", 0xD2B48C), ("teal", 0x008080), ("thistle", 0xD8BFD8),
    ("tomato", 0xFF6347), ("turquoise", 0x40E0D0), ("violet", 0xEE82EE), ("wheat", 0xF5DEB3), ("white", 0xFFFFFF),
    ("whitesmoke", 0xF5F5F5), ("yellow", 0xFFFF00), ("yellowgreen", 0x9ACD32),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::tokens;

    fn c(s: &str) -> Option<Color> {
        let t = tokens(s);
        color(&t, BLACK)
    }

    #[test]
    fn colours_are_read() {
        assert_eq!(c("red"), Some(0xFFFF0000));
        assert_eq!(c("#abc"), Some(0xFFAABBCC));
        assert_eq!(c("#11223380"), Some(0x80112233));
        assert_eq!(c("rgb(1, 2, 3)"), Some(0xFF010203));
        assert_eq!(c("rgb(1 2 3 / 50%)"), Some(0x80010203));
        assert_eq!(c("rgba(255,0,0,.5)"), Some(0x80FF0000));
        assert_eq!(c("hsl(120, 100%, 50%)"), Some(0xFF00FF00));
        assert_eq!(c("transparent"), Some(0));
        // Tailwind's gray-100, oklch(96.7% 0.003 264.542): #F3F4F6.
        let g = c("oklch(96.7% 0.003 264.542)").unwrap();
        let near = |a: u32, b: u32| (a as i32 - b as i32).abs() <= 2;
        assert!(near((g >> 16) & 255, 0xF3) && near((g >> 8) & 255, 0xF4) && near(g & 255, 0xF6), "{g:08X}");
        // Tailwind's blue-500, oklch(62.3% 0.214 259.815): #3B82F6 (roughly: sRGB gamut-clipped).
        let b = c("oklch(62.3% 0.214 259.815)").unwrap();
        assert!(near((b >> 16) & 255, 0x3B) || ((b >> 16) & 255) < 0x45, "{b:08X}");
        assert_eq!(c("color-mix(in srgb, red 50%, blue)"), Some(0xFF800080));
    }

    #[test]
    fn lengths_and_calc() {
        let u = Units::new(16.0, 16.0, 1000.0, 800.0, 1.0);
        let l = |s: &str| length(&tokens(s), &u, 500.0);
        assert_eq!(l("10px"), Some(Len::px(10.0)));
        assert_eq!(l("2em"), Some(Len::px(32.0)));
        assert_eq!(l("50%"), Some(Len::pct(50.0)));
        assert_eq!(l("calc(100% - 20px)"), Some(Len { px: -20.0, pct: 100.0 }));
        assert_eq!(l("calc(2 * (10px + 1rem))"), Some(Len::px(52.0)));
        assert_eq!(l("min(100px, 50%)"), Some(Len::px(100.0)));
        assert_eq!(l("max(100px, 50%)"), Some(Len::pct(50.0)));
        assert_eq!(l("clamp(1rem, 2.5vw, 3rem)"), Some(Len::px(25.0)));
        assert_eq!(l("10vw"), Some(Len::px(100.0)));
        assert_eq!(l("0"), Some(Len::ZERO));
        assert_eq!(l("auto"), None);
    }
}
