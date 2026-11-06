//! What's drawn under, through and over a box beyond its colours and borders, read from CSS's values: its
//! background's and its mask's layers (pictures and gradients, each sized, placed and repeated as CSS Backgrounds
//! says), its transform (2D: what's turned in 3D seen flat from the front), its filters, the shape it's clipped to
//! (clip-path) and how it's blended with what's under it. layout.rs works out where they go; paint.rs draws them.

use crate::css::{self, Tok};
use crate::num;
use crate::values::{self, Color, Len, Units};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

// ------------------------------------------------------------------------------------------------------- types

/// A background or mask layer's picture: an address's (made whole), or a gradient.
#[derive(Clone, Debug, PartialEq)]
pub enum Image {
    /// A picture's address, and its resolution (image-set's: its size in CSS pixels is its own divided by it).
    Url(String, f32),
    Gradient(Arc<Gradient>),
}

/// A CSS gradient: its kind, whether it repeats, and its colour stops.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    pub repeating: bool,
    pub stops: Vec<Stop>,
}

/// A colour stop: its colour and where it is along the gradient's line (device pixels and a percentage of the line's
/// length; for a conic gradient, degrees and a percentage of the turn), if said. A hint (`hint`) has no colour of its
/// own: it's where its neighbours are mixed half and half.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    pub color: Color,
    pub at: Option<Len>,
    pub hint: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientKind {
    /// Along an angle (degrees: 0 up, 90 right), or towards a corner (x and y: -1 or 1), whose angle depends on the
    /// box's shape.
    Linear { angle: f32, corner: Option<(i8, i8)> },
    /// Out from a centre, as a circle or an ellipse whose size is given or a keyword's.
    Radial { circle: bool, size: RadialSize, at: (Pos, Pos) },
    /// Round a centre, from an angle (degrees, 0 up).
    Conic { from: f32, at: (Pos, Pos) },
}

/// A radial gradient's ending shape's size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RadialSize {
    ClosestSide,
    FarthestSide,
    ClosestCorner,
    FarthestCorner,
    /// Its radii (a circle's two the same; percentages of the box's width and height).
    Radii(Len, Len),
}

/// A place along one side of a box: a length (and a percentage of the room left over) from its start, or from its
/// end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pos {
    pub len: Len,
    pub from_end: bool,
}

impl Pos {
    pub const START: Pos = Pos { len: Len::ZERO, from_end: false };
    pub const CENTER: Pos = Pos { len: Len { px: 0.0, pct: 50.0 }, from_end: false };

    /// Where it is, `room` (the side's length less what's placed's) to place it in.
    pub fn resolve(self, room: f32) -> f32 {
        let v = self.len.resolve(room);
        if self.from_end { room - v } else { v }
    }
}

/// background-size's (and mask-size's) value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BgSize {
    /// A width and a height (None: auto, from the picture's own size or proportions).
    Sized(Option<Len>, Option<Len>),
    Cover,
    Contain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Repeat,
    NoRepeat,
    Space,
    Round,
}

/// Which of a box's edges: its border box's, its padding box's, its content box's (or its words', for a background
/// clipped to them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxArea {
    Border,
    Padding,
    Content,
    Text,
}

/// A box's background or mask layers: each list one entry a layer (the first on top), or fewer, repeated, as CSS's
/// lists for layers are; empty for the initial value.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Layers {
    pub images: Vec<Option<Image>>,
    pub pos_x: Vec<Pos>,
    pub pos_y: Vec<Pos>,
    pub sizes: Vec<BgSize>,
    pub repeats: Vec<(Repeat, Repeat)>,
    pub origins: Vec<BoxArea>,
    pub clips: Vec<BoxArea>,
    pub fixed: Vec<bool>,
    /// A mask's: its luminance shows (else its alpha), and how it's combined with the layers under it (0 add, 1
    /// subtract, 2 intersect, 3 exclude).
    pub luminance: Vec<bool>,
    pub composite: Vec<u8>,
}

/// The `i`th layer's entry of one of a layer list's properties (`default` if it has none).
pub fn nth<T: Copy>(list: &[T], i: usize, default: T) -> T {
    if list.is_empty() { default } else { list[i % list.len()] }
}

impl Layers {
    /// Whether it has a picture or gradient to draw.
    pub fn any(&self) -> bool {
        self.images.iter().any(|i| i.is_some())
    }
}

/// object-fit's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectFit {
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

impl ObjectFit {
    pub fn from_word(w: &str) -> Option<ObjectFit> {
        Some(match w {
            "fill" => ObjectFit::Fill,
            "contain" => ObjectFit::Contain,
            "cover" => ObjectFit::Cover,
            "none" => ObjectFit::None,
            "scale-down" => ObjectFit::ScaleDown,
            _ => return None,
        })
    }
}

/// A transform function: a move (its percentages of the box's own size), or the rest as a 2D matrix (a b c d e f:
/// x' = a x + c y + e, y' = b x + d y + f, in device pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TFn {
    Translate(Len, Len),
    Matrix([f32; 6]),
}

/// A filter function: a blur (its standard deviation, device pixels), a drop shadow, or one of the colour ones
/// (brightness 0, contrast 1, grayscale 2, hue-rotate 3 (degrees), invert 4, opacity 5, saturate 6, sepia 7) and its
/// amount.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    Blur(f32),
    DropShadow { x: f32, y: f32, blur: f32, color: Color },
    Color(u8, f32),
}

/// A clip-path's shape, and the box it's measured from.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipPath {
    pub shape: Shape,
    pub area: BoxArea,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Insets from the top, right, bottom and left, and the corners' rounding.
    Inset([Len; 4], [Len; 4]),
    /// A circle's radius (None: closest-side; Some(None)... see `CircleRadius`) and centre.
    Circle(ShapeRadius, (Pos, Pos)),
    Ellipse(ShapeRadius, ShapeRadius, (Pos, Pos)),
    /// Points (percentages of the box's width and height), filled even-odd or non-zero.
    Polygon(bool, Vec<(Len, Len)>),
    /// An SVG path (in CSS pixels from the box's top left), filled even-odd or non-zero.
    Path(bool, String),
    /// The box itself (clip-path: border-box and the like).
    Box,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeRadius {
    Len(Len),
    ClosestSide,
    FarthestSide,
}

// ------------------------------------------------------------------------------------------------------- helpers

fn trim(t: &[Tok]) -> &[Tok] {
    values::trim(t)
}

fn ident(v: &[Tok]) -> Option<String> {
    match trim(v) {
        [Tok::Ident(i)] => Some(i.to_ascii_lowercase()),
        _ => None,
    }
}

/// The value's parts, split at spaces (a function and what's in it are one part), and at "/".
fn parts(v: &[Tok]) -> Vec<&[Tok]> {
    let mut out = Vec::new();
    let mut i = 0;
    let v = trim(v);
    while i < v.len() {
        if v[i] == Tok::Ws {
            i += 1;
            continue;
        }
        let start = i;
        if matches!(v[i], Tok::Function(_) | Tok::OpenParen | Tok::OpenSquare) {
            let mut depth = 0;
            while i < v.len() {
                match v[i] {
                    Tok::Function(_) | Tok::OpenParen | Tok::OpenSquare => depth += 1,
                    Tok::CloseParen | Tok::CloseSquare => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            i += 1;
        } else {
            i += 1;
        }
        out.push(&v[start..i.min(v.len())]);
    }
    out
}

/// A function's name and what's between its brackets.
fn function(v: &[Tok]) -> Option<(&str, &[Tok])> {
    match trim(v) {
        [Tok::Function(f), rest @ ..] => {
            let inner = if rest.last() == Some(&Tok::CloseParen) { &rest[..rest.len() - 1] } else { rest };
            Some((f.as_str(), inner))
        }
        _ => None,
    }
}

fn length(v: &[Tok], u: &Units) -> Option<Len> {
    values::length(v, u, 0.0)
}

fn number_or_pct(v: &[Tok], u: &Units) -> Option<f32> {
    match trim(v) {
        [Tok::Percent(p)] => Some(p / 100.0),
        t => values::number(t, u),
    }
}

fn angle(v: &[Tok]) -> Option<f32> {
    match trim(v) {
        [Tok::Number(n)] if *n == 0.0 => Some(0.0),
        [t @ Tok::Dimension(..)] => values::angle(t),
        [Tok::Function(f), ..] if f == "calc" => {
            // calc() of angles: degrees, added up (the common "calc(90deg + 45deg)" and the like).
            let (_, inner) = function(v)?;
            let mut sum = 0.0;
            let mut sign = 1.0;
            for t in inner {
                match t {
                    Tok::Delim('+') => sign = 1.0,
                    Tok::Delim('-') => sign = -1.0,
                    Tok::Ws => {}
                    _ => {
                        sum += sign * values::angle(t)?;
                        sign = 1.0;
                    }
                }
            }
            Some(sum)
        }
        _ => None,
    }
}

// ------------------------------------------------------------------------------------------------------- images

/// An image value: Some(None) for none, None if it isn't one.
pub fn image(v: &[Tok], current: Color, u: &Units) -> Option<Option<Image>> {
    match trim(v) {
        [Tok::Ident(i)] if i.eq_ignore_ascii_case("none") => Some(None),
        [Tok::Url(a)] => Some(Some(Image::Url(a.clone(), 1.0))),
        _ => {
            let (f, inner) = function(v)?;
            match f {
                "url" | "src" => match trim(inner) {
                    [Tok::Str(a), ..] => Some(Some(Image::Url(a.clone(), 1.0))),
                    _ => None,
                },
                "image-set" | "-webkit-image-set" => {
                    // Its pictures of types that are read, each with its resolution (1x unless it says): of them,
                    // the least that's at least the screen's, else the greatest, as Chrome picks.
                    let mut options: Vec<(f32, Image)> = Vec::new();
                    for c in css::split_commas(inner) {
                        let p = parts(c);
                        let img = match p.first().map(|t| trim(t)) {
                            Some([Tok::Str(a)]) => Some(Image::Url(a.clone(), 1.0)),
                            Some(t) => image(t, current, u).flatten(),
                            None => None,
                        };
                        let Some(img) = img else { continue };
                        let mut res = 1.0;
                        let mut ok = true;
                        for q in &p[1..] {
                            match trim(q) {
                                [Tok::Dimension(n, unit)] => match unit.as_str() {
                                    "x" | "dppx" => res = *n,
                                    "dpi" => res = *n / 96.0,
                                    "dpcm" => res = *n * 2.54 / 96.0,
                                    _ => ok = false,
                                },
                                [Tok::Function(f), rest @ ..] if f == "type" => {
                                    if !matches!(trim(rest), [Tok::Str(t), Tok::CloseParen] if crate::srcset::type_supported(t)) {
                                        ok = false;
                                    }
                                }
                                _ => {}
                            }
                        }
                        if ok && res > 0.0 {
                            options.push((res, img));
                        }
                    }
                    options.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
                    let at = options.iter().position(|(r, _)| *r >= u.dpr).unwrap_or(options.len().checked_sub(1)?);
                    let (res, img) = options.swap_remove(at);
                    Some(Some(match img {
                        Image::Url(a, _) => Image::Url(a, res),
                        other => other,
                    }))
                }
                "image" => image(css::split_commas(inner).into_iter().next()?, current, u),
                _ if f.contains("gradient") => Some(gradient(f, inner, current, u).map(|g| Image::Gradient(Arc::new(g)))),
                _ => None,
            }
        }
    }
}

/// A gradient from its function's name and arguments.
pub fn gradient(f: &str, inner: &[Tok], current: Color, u: &Units) -> Option<Gradient> {
    let legacy = f.starts_with("-webkit-") || f.starts_with("-moz-") || f.starts_with("-o-");
    let name = f.trim_start_matches("-webkit-").trim_start_matches("-moz-").trim_start_matches("-o-");
    let repeating = name.starts_with("repeating-");
    let name = name.trim_start_matches("repeating-");
    let args = css::split_commas(inner);
    let first = parts(args.first()?);
    let words: Vec<String> = first.iter().filter_map(|p| ident(p)).collect();
    // (A colour space for the colours to be mixed in, "in oklab": ignored; sRGB.)
    let first_is_stop = stop_parts(&first, current, u).is_some();
    let (kind, from) = match name {
        "linear-gradient" => {
            let mut kind = GradientKind::Linear { angle: 180.0, corner: None };
            let mut from = 0;
            if !first_is_stop {
                from = 1;
                if let Some(a) = first.first().and_then(|p| angle(p)) {
                    // (The prefixed syntax's angles go anticlockwise from the right.)
                    kind = GradientKind::Linear { angle: if legacy { 90.0 - a } else { a }, corner: None };
                } else {
                    let has = |w: &str| words.iter().any(|x| x == w);
                    let to = has("to") || !legacy;
                    // (The prefixed syntax names where it starts, not where it goes.)
                    let flip = if to { 1i8 } else { -1i8 };
                    let x = if has("left") { -flip } else if has("right") { flip } else { 0 };
                    let y = if has("top") { -flip } else if has("bottom") { flip } else { 0 };
                    kind = match (x, y) {
                        (0, 0) => GradientKind::Linear { angle: 180.0, corner: None },
                        (0, y) => GradientKind::Linear { angle: if y < 0 { 0.0 } else { 180.0 }, corner: None },
                        (x, 0) => GradientKind::Linear { angle: if x < 0 { 270.0 } else { 90.0 }, corner: None },
                        (x, y) => GradientKind::Linear { angle: 0.0, corner: Some((x, y)) },
                    };
                }
            }
            (kind, from)
        }
        "radial-gradient" => {
            let mut circle = false;
            let mut size = RadialSize::FarthestCorner;
            let mut at = (Pos::CENTER, Pos::CENTER);
            let mut from = 0;
            if !first_is_stop {
                from = 1;
                // (The prefixed syntax: its centre first, then its shape and size in an argument of their own.)
                let second: Vec<&[Tok]> = args.get(1).map(|a| parts(a)).unwrap_or_default();
                let mut shape_src: &[&[Tok]] = &first;
                if legacy
                    && !first.iter().any(|p| ident(p).as_deref() == Some("at"))
                    && let Some(p) = position(&first, u)
                {
                    at = p;
                    shape_src = &[];
                    if !second.is_empty() && stop_parts(&second, current, u).is_none() {
                        shape_src = &second;
                        from = 2;
                    }
                }
                let at_k = shape_src.iter().position(|p| ident(p).as_deref() == Some("at"));
                let (shape_parts, at_parts) = match at_k {
                    Some(k) => (&shape_src[..k], &shape_src[k + 1..]),
                    None => (shape_src, &shape_src[..0]),
                };
                if !at_parts.is_empty() {
                    at = position(at_parts, u)?;
                }
                let mut lens = Vec::new();
                let mut ellipse = false;
                for p in shape_parts {
                    match ident(p).as_deref() {
                        Some("circle") => circle = true,
                        Some("ellipse") => ellipse = true,
                        Some("closest-side") | Some("contain") => size = RadialSize::ClosestSide,
                        Some("farthest-side") => size = RadialSize::FarthestSide,
                        Some("closest-corner") => size = RadialSize::ClosestCorner,
                        Some("farthest-corner") | Some("cover") => size = RadialSize::FarthestCorner,
                        Some("in" | "srgb" | "oklab" | "oklch" | "lab" | "lch" | "hsl" | "hwb" | "display-p3" | "srgb-linear" | "xyz" | "shorter" | "longer" | "hue") => {}
                        _ => lens.push(length(p, u)?),
                    }
                }
                match lens.len() {
                    0 => {}
                    1 => {
                        circle = !ellipse;
                        size = RadialSize::Radii(lens[0], lens[0]);
                    }
                    _ => size = RadialSize::Radii(lens[0], lens[1]),
                }
            }
            (GradientKind::Radial { circle, size, at }, from)
        }
        "conic-gradient" => {
            let mut from_angle = 0.0;
            let mut at = (Pos::CENTER, Pos::CENTER);
            let mut from = 0;
            if !first_is_stop {
                from = 1;
                let mut k = 0;
                while k < first.len() {
                    match ident(first[k]).as_deref() {
                        Some("from") => {
                            from_angle = first.get(k + 1).and_then(|p| angle(p))?;
                            k += 2;
                        }
                        Some("at") => {
                            let end = first[k + 1..].iter().position(|p| matches!(ident(p).as_deref(), Some("from" | "in"))).map_or(first.len(), |e| k + 1 + e);
                            at = position(&first[k + 1..end], u)?;
                            k = end;
                        }
                        _ => k += 1,
                    }
                }
            }
            (GradientKind::Conic { from: from_angle, at }, from)
        }
        _ => return None,
    };
    let conic = matches!(kind, GradientKind::Conic { .. });
    let mut stops = Vec::new();
    for a in args.iter().skip(from) {
        let p = parts(a);
        if let Some((color, ats)) = stop_parts_conic(&p, current, u, conic) {
            match ats.len() {
                0 => stops.push(Stop { color, at: None, hint: false }),
                1 => stops.push(Stop { color, at: Some(ats[0]), hint: false }),
                _ => {
                    stops.push(Stop { color, at: Some(ats[0]), hint: false });
                    stops.push(Stop { color, at: Some(ats[1]), hint: false });
                }
            }
        } else if p.len() == 1 {
            // A hint: where between its neighbours they're half each.
            let at = if conic { conic_len(p[0])? } else { length(p[0], u)? };
            stops.push(Stop { color: 0, at: Some(at), hint: true });
        } else {
            return None;
        }
    }
    // (At least one colour; hints only between colours.)
    if stops.iter().all(|s| s.hint) || stops.first().is_some_and(|s| s.hint) || stops.last().is_some_and(|s| s.hint) {
        return None;
    }
    if stops.len() == 1 {
        let s = stops[0];
        stops.push(s);
    }
    Some(Gradient { kind, repeating, stops })
}

/// A conic gradient's stop's place: an angle (as degrees in `px`) or a percentage of the turn.
fn conic_len(p: &[Tok]) -> Option<Len> {
    match trim(p) {
        [Tok::Percent(x)] => Some(Len::pct(*x)),
        _ => angle(p).map(Len::px),
    }
}

fn stop_parts(p: &[&[Tok]], current: Color, u: &Units) -> Option<(Color, Vec<Len>)> {
    stop_parts_conic(p, current, u, false)
}

/// A colour stop's colour and its places (none, one or two), in either order.
fn stop_parts_conic(p: &[&[Tok]], current: Color, u: &Units, conic: bool) -> Option<(Color, Vec<Len>)> {
    let mut color = None;
    let mut ats = Vec::new();
    for x in p {
        if color.is_none()
            && let Some(c) = values::color(x, current)
        {
            color = Some(c);
            continue;
        }
        let at = if conic { conic_len(x)? } else { length(x, u)? };
        ats.push(at);
    }
    if ats.len() > 2 {
        return None;
    }
    Some((color?, ats))
}

// ------------------------------------------------------------------------------------------------------- positions

fn is_x(w: &str) -> bool {
    matches!(w, "left" | "right" | "x-start" | "x-end")
}

fn is_y(w: &str) -> bool {
    matches!(w, "top" | "bottom" | "y-start" | "y-end")
}

fn keyword_pos(w: &str) -> Option<Pos> {
    Some(match w {
        "left" | "top" | "x-start" | "y-start" => Pos::START,
        "right" | "bottom" | "x-end" | "y-end" => Pos { len: Len::ZERO, from_end: true },
        "center" => Pos::CENTER,
        _ => return None,
    })
}

/// A <position> (background-position's, object-position's, a gradient's "at"): one to four parts.
pub fn position(p: &[&[Tok]], u: &Units) -> Option<(Pos, Pos)> {
    let kw: Vec<Option<String>> = p.iter().map(|x| ident(x)).collect();
    match p.len() {
        1 => match kw[0].as_deref() {
            Some(w) if is_y(w) => Some((Pos::CENTER, keyword_pos(w)?)),
            Some(w) => Some((keyword_pos(w)?, Pos::CENTER)),
            None => Some((Pos { len: length(p[0], u)?, from_end: false }, Pos::CENTER)),
        },
        2 => {
            let one = |k: usize| -> Option<Pos> {
                match kw[k].as_deref() {
                    Some(w) => keyword_pos(w),
                    None => Some(Pos { len: length(p[k], u)?, from_end: false }),
                }
            };
            let (a, b) = (kw[0].as_deref(), kw[1].as_deref());
            // "top left" and "center left": y first.
            let swap = a.is_some_and(is_y) || b.is_some_and(is_x);
            if (a.is_some_and(is_x) && b.is_some_and(is_x)) || (a.is_some_and(is_y) && b.is_some_and(is_y)) {
                return None;
            }
            if swap {
                if a.is_none() || b.is_none() {
                    // ("top 10px" isn't a two-value position; "10px top" neither.)
                    return None;
                }
                Some((one(1)?, one(0)?))
            } else {
                Some((one(0)?, one(1)?))
            }
        }
        3 | 4 => {
            // Keywords each with an offset after it (or none: center or an edge).
            let mut x = None;
            let mut y = None;
            let mut k = 0;
            while k < p.len() {
                let w = kw[k].as_deref()?;
                let base = keyword_pos(w)?;
                let off = if k + 1 < p.len() && kw[k + 1].is_none() {
                    k += 1;
                    Some(length(p[k], u)?)
                } else {
                    None
                };
                let pos = match off {
                    Some(l) if w != "center" => Pos { len: l, from_end: base.from_end },
                    Some(_) => return None,
                    None => base,
                };
                if is_x(w) {
                    if x.is_some() {
                        return None;
                    }
                    x = Some(pos);
                } else if is_y(w) {
                    if y.is_some() {
                        return None;
                    }
                    y = Some(pos);
                } else if x.is_none() && y.is_some() || (x.is_none() && k + 1 < p.len()) {
                    x = Some(pos);
                } else {
                    y = Some(pos);
                }
                k += 1;
            }
            Some((x.unwrap_or(Pos::CENTER), y.unwrap_or(Pos::CENTER)))
        }
        _ => None,
    }
}

/// One side's position (background-position-x or -y's): a keyword, a length, or a keyword and an offset.
fn axis_position(p: &[&[Tok]], u: &Units) -> Option<Pos> {
    match p {
        [one] => match ident(one) {
            Some(w) => keyword_pos(&w),
            None => Some(Pos { len: length(one, u)?, from_end: false }),
        },
        [k, off] => {
            let base = keyword_pos(&ident(k)?)?;
            Some(Pos { len: length(off, u)?, from_end: base.from_end })
        }
        _ => None,
    }
}

// ------------------------------------------------------------------------------------------------------- layers

fn repeat_word(w: &str) -> Option<Repeat> {
    Some(match w {
        "repeat" => Repeat::Repeat,
        "no-repeat" => Repeat::NoRepeat,
        "space" => Repeat::Space,
        "round" => Repeat::Round,
        _ => return None,
    })
}

fn repeat_style(p: &[&[Tok]]) -> Option<(Repeat, Repeat)> {
    let w: Vec<String> = p.iter().map(|x| ident(x)).collect::<Option<_>>()?;
    match w.as_slice() {
        [one] if one == "repeat-x" => Some((Repeat::Repeat, Repeat::NoRepeat)),
        [one] if one == "repeat-y" => Some((Repeat::NoRepeat, Repeat::Repeat)),
        [one] => {
            let r = repeat_word(one)?;
            Some((r, r))
        }
        [a, b] => Some((repeat_word(a)?, repeat_word(b)?)),
        _ => None,
    }
}

fn box_word(w: &str) -> Option<BoxArea> {
    Some(match w {
        "border-box" | "border" | "fill-box" | "stroke-box" | "view-box" | "margin-box" => BoxArea::Border,
        "padding-box" | "padding" => BoxArea::Padding,
        "content-box" | "content" => BoxArea::Content,
        "text" => BoxArea::Text,
        _ => return None,
    })
}

fn size_value(p: &[&[Tok]], u: &Units) -> Option<BgSize> {
    let one = |x: &[Tok]| -> Option<Option<Len>> {
        if ident(x).as_deref() == Some("auto") {
            return Some(None);
        }
        let l = length(x, u)?;
        if l.px < 0.0 || l.pct < 0.0 {
            return None;
        }
        Some(Some(l))
    };
    match p {
        [x] => match ident(x).as_deref() {
            Some("cover") => Some(BgSize::Cover),
            Some("contain") => Some(BgSize::Contain),
            _ => Some(BgSize::Sized(one(x)?, None)),
        },
        [a, b] => Some(BgSize::Sized(one(a)?, one(b)?)),
        _ => None,
    }
}

/// Which property of a layer list a name is (backgrounds' and masks' alike).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayerProp {
    Image,
    Position,
    PositionX,
    PositionY,
    Size,
    Repeat,
    Origin,
    Clip,
    Attachment,
    Mode,
    Composite,
}

/// Sets one of a layer list's properties from its value (comma-separated, a layer each). Returns false if it isn't
/// a valid value (the declaration is then dropped).
pub fn set_layer_prop(l: &mut Layers, prop: LayerProp, v: &[Tok], current: Color, u: &Units) -> bool {
    let items = css::split_commas(v);
    macro_rules! each {
        ($f:expr) => {{
            let mut out = Vec::new();
            for it in &items {
                let p = parts(it);
                match $f(&p) {
                    Some(x) => out.push(x),
                    None => return false,
                }
            }
            out
        }};
    }
    match prop {
        LayerProp::Image => l.images = each!(|p: &Vec<&[Tok]>| if p.len() == 1 { image(p[0], current, u) } else { None }),
        LayerProp::Position => {
            let both: Vec<(Pos, Pos)> = each!(|p: &Vec<&[Tok]>| position(p, u));
            l.pos_x = both.iter().map(|b| b.0).collect();
            l.pos_y = both.iter().map(|b| b.1).collect();
        }
        LayerProp::PositionX => l.pos_x = each!(|p: &Vec<&[Tok]>| axis_position(p, u)),
        LayerProp::PositionY => l.pos_y = each!(|p: &Vec<&[Tok]>| axis_position(p, u)),
        LayerProp::Size => l.sizes = each!(|p: &Vec<&[Tok]>| size_value(p, u)),
        LayerProp::Repeat => l.repeats = each!(|p: &Vec<&[Tok]>| repeat_style(p)),
        LayerProp::Origin => l.origins = each!(|p: &Vec<&[Tok]>| if p.len() == 1 { ident(p[0]).and_then(|w| box_word(&w)) } else { None }),
        LayerProp::Clip => {
            l.clips = each!(|p: &Vec<&[Tok]>| if p.len() == 1 {
                ident(p[0]).and_then(|w| if w == "no-clip" { Some(BoxArea::Border) } else { box_word(&w) })
            } else {
                None
            })
        }
        LayerProp::Attachment => {
            l.fixed = each!(|p: &Vec<&[Tok]>| match ident(p.first().copied().unwrap_or(&[])).as_deref() {
                Some("fixed") => Some(true),
                Some("scroll" | "local") => Some(false),
                _ => None,
            })
        }
        LayerProp::Mode => {
            l.luminance = each!(|p: &Vec<&[Tok]>| match ident(p.first().copied().unwrap_or(&[])).as_deref() {
                Some("luminance") => Some(true),
                Some("alpha" | "match-source") => Some(false),
                _ => None,
            })
        }
        LayerProp::Composite => l.composite = each!(|p: &Vec<&[Tok]>| ident(p.first().copied().unwrap_or(&[])).and_then(|w| composite_word(&w))),
    }
    true
}

fn composite_word(w: &str) -> Option<u8> {
    Some(match w {
        "add" | "source-over" | "destination-over" => 0,
        "subtract" | "source-out" | "destination-out" => 1,
        "intersect" | "source-in" | "destination-in" => 2,
        "exclude" | "xor" => 3,
        _ => return None,
    })
}

/// The background (or mask, `mask`) shorthand: its layers, and (a background's) the colour its last one gives.
/// None if it isn't a valid value.
pub fn layers_shorthand(v: &[Tok], current: Color, u: &Units, mask: bool) -> Option<(Layers, Option<Color>)> {
    let items = css::split_commas(v);
    let mut l = Layers::default();
    let mut colour = None;
    for (n, it) in items.iter().enumerate() {
        let p = parts(it);
        let mut img: Option<Option<Image>> = None;
        let mut pos: Option<(Pos, Pos)> = None;
        let mut size: Option<BgSize> = None;
        let mut rep: Option<(Repeat, Repeat)> = None;
        let mut boxes: Vec<BoxArea> = Vec::new();
        let mut fixed = false;
        let mut lum = false;
        let mut comp = 0u8;
        let mut k = 0;
        while k < p.len() {
            let x = p[k];
            let w = ident(x);
            // A position: its parts up to what isn't one, then "/ size".
            let is_pos_part = |t: &[Tok]| ident(t).is_some_and(|w| keyword_pos(&w).is_some()) || (ident(t).is_none() && length(t, u).is_some());
            if pos.is_none() && is_pos_part(x) {
                let mut e = k;
                while e < p.len() && e - k < 4 && is_pos_part(p[e]) {
                    e += 1;
                }
                // (The longest run of parts that's a position.)
                let mut found = None;
                let mut end = e;
                while end > k {
                    if let Some(ps) = position(&p[k..end], u) {
                        found = Some(ps);
                        break;
                    }
                    end -= 1;
                }
                let Some(ps) = found else { return None };
                pos = Some(ps);
                k = end;
                if k < p.len() && trim(p[k]) == [Tok::Delim('/')] {
                    k += 1;
                    let mut e = k;
                    while e < p.len() && e - k < 2 && (ident(p[e]).is_some_and(|w| matches!(w.as_str(), "auto" | "cover" | "contain")) || length(p[e], u).is_some()) {
                        e += 1;
                    }
                    size = Some(size_value(&p[k..e], u)?);
                    k = e;
                }
                continue;
            }
            if let Some(w) = &w {
                if let Some(r) = repeat_word(w).map(|r| (r, r)).or(match w.as_str() {
                    "repeat-x" => Some((Repeat::Repeat, Repeat::NoRepeat)),
                    "repeat-y" => Some((Repeat::NoRepeat, Repeat::Repeat)),
                    _ => None,
                }) {
                    // ("repeat no-repeat": two words.)
                    if let Some(second) = p.get(k + 1).and_then(|y| ident(y)).and_then(|y| repeat_word(&y))
                        && !matches!(w.as_str(), "repeat-x" | "repeat-y")
                    {
                        rep = Some((r.0, second));
                        k += 2;
                    } else {
                        rep = Some(r);
                        k += 1;
                    }
                    continue;
                }
                if let Some(b) = box_word(w).or(if w == "no-clip" { Some(BoxArea::Border) } else { None }) {
                    boxes.push(b);
                    k += 1;
                    continue;
                }
                match w.as_str() {
                    "fixed" => {
                        fixed = true;
                        k += 1;
                        continue;
                    }
                    "scroll" | "local" => {
                        k += 1;
                        continue;
                    }
                    "none" if img.is_none() => {
                        img = Some(None);
                        k += 1;
                        continue;
                    }
                    "luminance" if mask => {
                        lum = true;
                        k += 1;
                        continue;
                    }
                    "alpha" | "match-source" if mask => {
                        k += 1;
                        continue;
                    }
                    _ => {}
                }
                if mask && let Some(c) = composite_word(w) {
                    comp = c;
                    k += 1;
                    continue;
                }
            }
            if img.is_none()
                && let Some(i) = image(x, current, u)
            {
                img = Some(i);
                k += 1;
                continue;
            }
            if !mask
                && n == items.len() - 1
                && colour.is_none()
                && let Some(c) = values::color(x, current)
            {
                colour = Some(c);
                k += 1;
                continue;
            }
            return None;
        }
        l.images.push(img.unwrap_or(None));
        let (px, py) = pos.unwrap_or((Pos::START, Pos::START));
        l.pos_x.push(px);
        l.pos_y.push(py);
        l.sizes.push(size.unwrap_or(BgSize::Sized(None, None)));
        l.repeats.push(rep.unwrap_or((Repeat::Repeat, Repeat::Repeat)));
        let default_origin = if mask { BoxArea::Border } else { BoxArea::Padding };
        l.origins.push(boxes.first().copied().unwrap_or(default_origin));
        l.clips.push(boxes.get(1).or(boxes.first()).copied().unwrap_or(BoxArea::Border));
        l.fixed.push(fixed);
        l.luminance.push(lum);
        l.composite.push(comp);
    }
    Some((l, colour))
}

// ------------------------------------------------------------------------------------------------------- transforms

pub const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// a then b: the matrix that does b first, then a (as "transform: a b" applies them, b to the point first).
pub fn mul(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

pub fn invert(m: [f32; 6]) -> Option<[f32; 6]> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])])
}

pub fn apply(m: [f32; 6], x: f32, y: f32) -> (f32, f32) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

fn rotation(deg: f32) -> [f32; 6] {
    let r = deg * core::f32::consts::PI / 180.0;
    let (s, c) = (num::sin(r), num::cos(r));
    // (Whole quarter turns exactly.)
    let snap = |v: f32| if (v - num::round(v)).abs() < 1e-6 { num::round(v) } else { v };
    [snap(c), snap(s), -snap(s), snap(c), 0.0, 0.0]
}

/// A 3D rotation about (x, y, z) by `deg`, seen flat from the front: its matrix's 2D part.
fn rotation3d(x: f32, y: f32, z: f32, deg: f32) -> [f32; 6] {
    let len = num::sqrt(x * x + y * y + z * z);
    if len < 1e-9 {
        return IDENTITY;
    }
    let (x, y, z) = (x / len, y / len, z / len);
    let r = deg * core::f32::consts::PI / 180.0;
    let (s, c) = (num::sin(r), num::cos(r));
    let t = 1.0 - c;
    // (Row-major rotation matrix; CSS's column vectors: m11 = a, m12 = b, m21 = c, m22 = d.)
    let m11 = t * x * x + c;
    let m12 = t * x * y + s * z;
    let m21 = t * x * y - s * z;
    let m22 = t * y * y + c;
    [m11, m12, m21, m22, 0.0, 0.0]
}

/// A transform list's functions (transform's value). Some(empty) for none, None if it isn't valid.
pub fn transform_list(v: &[Tok], u: &Units) -> Option<Vec<TFn>> {
    if ident(v).as_deref() == Some("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for p in parts(v) {
        let (f, inner) = function(p)?;
        let args = css::split_commas(inner);
        // (Space-separated too, as some pages write them.)
        let args: Vec<&[Tok]> = if args.len() == 1 { parts(args[0]) } else { args };
        let len = |k: usize| args.get(k).map(|a| length(a, u)).unwrap_or(Some(Len::ZERO));
        let num = |k: usize, d: f32| args.get(k).map(|a| number_or_pct(a, u)).unwrap_or(Some(d));
        let ang = |k: usize| args.get(k).map(|a| angle(a)).unwrap_or(Some(0.0));
        let px = |k: usize| args.get(k).and_then(|a| values::number(a, u)).map(|n| n * u.dpr);
        let t = match f {
            "translate" => TFn::Translate(len(0)?, len(1)?),
            "translate3d" => {
                len(2)?;
                TFn::Translate(len(0)?, len(1)?)
            }
            "translatex" => TFn::Translate(len(0)?, Len::ZERO),
            "translatey" => TFn::Translate(Len::ZERO, len(0)?),
            "translatez" | "perspective" => {
                len(0)?;
                TFn::Translate(Len::ZERO, Len::ZERO)
            }
            "scale" | "scale3d" => {
                let sx = num(0, 1.0)?;
                let sy = if args.len() > 1 { num(1, 1.0)? } else { sx };
                TFn::Matrix([sx, 0.0, 0.0, sy, 0.0, 0.0])
            }
            "scalex" => TFn::Matrix([num(0, 1.0)?, 0.0, 0.0, 1.0, 0.0, 0.0]),
            "scaley" => TFn::Matrix([1.0, 0.0, 0.0, num(0, 1.0)?, 0.0, 0.0]),
            "scalez" => TFn::Matrix(IDENTITY),
            "rotate" | "rotatez" => TFn::Matrix(rotation(ang(0)?)),
            "rotatex" => TFn::Matrix(rotation3d(1.0, 0.0, 0.0, ang(0)?)),
            "rotatey" => TFn::Matrix(rotation3d(0.0, 1.0, 0.0, ang(0)?)),
            "rotate3d" => TFn::Matrix(rotation3d(num(0, 0.0)?, num(1, 0.0)?, num(2, 0.0)?, ang(3)?)),
            "skew" => {
                let (ax, ay) = (ang(0)?, ang(1)?);
                TFn::Matrix([1.0, tan(ay), tan(ax), 1.0, 0.0, 0.0])
            }
            "skewx" => TFn::Matrix([1.0, 0.0, tan(ang(0)?), 1.0, 0.0, 0.0]),
            "skewy" => TFn::Matrix([1.0, tan(ang(0)?), 0.0, 1.0, 0.0, 0.0]),
            "matrix" => {
                if args.len() != 6 {
                    return None;
                }
                TFn::Matrix([num(0, 1.0)?, num(1, 0.0)?, num(2, 0.0)?, num(3, 1.0)?, px(4)?, px(5)?])
            }
            "matrix3d" => {
                if args.len() != 16 {
                    return None;
                }
                TFn::Matrix([num(0, 1.0)?, num(1, 0.0)?, num(4, 0.0)?, num(5, 1.0)?, px(12)?, px(13)?])
            }
            _ => return None,
        };
        out.push(t);
    }
    Some(out)
}

fn tan(deg: f32) -> f32 {
    let r = deg * core::f32::consts::PI / 180.0;
    let c = num::cos(r);
    if c.abs() < 1e-9 { 1e9 } else { num::sin(r) / c }
}

/// The translate property: none, or x [y [z]].
pub fn translate_prop(v: &[Tok], u: &Units) -> Option<Option<TFn>> {
    if ident(v).as_deref() == Some("none") {
        return Some(None);
    }
    let p = parts(v);
    if p.is_empty() || p.len() > 3 {
        return None;
    }
    let x = length(p[0], u)?;
    let y = match p.get(1) {
        Some(t) => length(t, u)?,
        None => Len::ZERO,
    };
    Some(Some(TFn::Translate(x, y)))
}

/// The rotate property: none, an angle, or an axis and an angle.
pub fn rotate_prop(v: &[Tok], u: &Units) -> Option<Option<TFn>> {
    if ident(v).as_deref() == Some("none") {
        return Some(None);
    }
    let p = parts(v);
    let mut deg = None;
    let mut axis = (0.0, 0.0, 1.0);
    let mut nums = Vec::new();
    for x in &p {
        if let Some(a) = angle(x).filter(|_| !matches!(trim(x), [Tok::Number(_)])) {
            deg = Some(a);
            continue;
        }
        match ident(x).as_deref() {
            Some("x") => axis = (1.0, 0.0, 0.0),
            Some("y") => axis = (0.0, 1.0, 0.0),
            Some("z") => axis = (0.0, 0.0, 1.0),
            _ => nums.push(values::number(x, u)?),
        }
    }
    if nums.len() == 3 {
        axis = (nums[0], nums[1], nums[2]);
    } else if !nums.is_empty() {
        // (A lone 0 is an angle.)
        if nums.len() == 1 && nums[0] == 0.0 && deg.is_none() {
            deg = Some(0.0);
        } else {
            return None;
        }
    }
    let deg = deg?;
    Some(Some(TFn::Matrix(if axis == (0.0, 0.0, 1.0) { rotation(deg) } else { rotation3d(axis.0, axis.1, axis.2, deg) })))
}

/// The scale property: none, or one to three numbers (or percentages).
pub fn scale_prop(v: &[Tok], u: &Units) -> Option<Option<TFn>> {
    if ident(v).as_deref() == Some("none") {
        return Some(None);
    }
    let p = parts(v);
    if p.is_empty() || p.len() > 3 {
        return None;
    }
    let sx = number_or_pct(p[0], u)?;
    let sy = match p.get(1) {
        Some(t) => number_or_pct(t, u)?,
        None => sx,
    };
    Some(Some(TFn::Matrix([sx, 0.0, 0.0, sy, 0.0, 0.0])))
}

/// transform-origin: x and y (keywords, lengths, percentages of the box's border box); a z after them is ignored.
pub fn origin(v: &[Tok], u: &Units) -> Option<(Len, Len)> {
    let p = parts(v);
    let as_len = |x: Pos| if x.from_end { Len { px: -x.len.px, pct: 100.0 - x.len.pct } } else { x.len };
    match p.len() {
        1 | 2 => {
            let (x, y) = position(&p, u)?;
            Some((as_len(x), as_len(y)))
        }
        3 => {
            let (x, y) = position(&p[..2], u)?;
            length(p[2], u)?;
            Some((as_len(x), as_len(y)))
        }
        _ => None,
    }
}

/// The 2D matrix of a box's transform functions, its percentages of its border box's size (`w` by `h`), about
/// `origin` (a point in page coordinates): translate(origin) · the functions · translate(-origin).
pub fn matrix_of(fns: &[TFn], w: f32, h: f32, origin: (f32, f32)) -> [f32; 6] {
    let mut m = [1.0, 0.0, 0.0, 1.0, origin.0, origin.1];
    for f in fns {
        let fm = match *f {
            TFn::Translate(x, y) => [1.0, 0.0, 0.0, 1.0, x.resolve(w), y.resolve(h)],
            TFn::Matrix(m) => m,
        };
        m = mul(m, fm);
    }
    mul(m, [1.0, 0.0, 0.0, 1.0, -origin.0, -origin.1])
}

/// Whether functions only move (and by how much, its percentages of the box's size).
pub fn only_moves(fns: &[TFn]) -> Option<(Len, Len)> {
    let mut x = Len::ZERO;
    let mut y = Len::ZERO;
    for f in fns {
        match *f {
            TFn::Translate(a, b) => {
                x = Len { px: x.px + a.px, pct: x.pct + a.pct };
                y = Len { px: y.px + b.px, pct: y.pct + b.pct };
            }
            TFn::Matrix(m) if m[0] == 1.0 && m[1] == 0.0 && m[2] == 0.0 && m[3] == 1.0 => {
                x.px += m[4];
                y.px += m[5];
            }
            _ => return None,
        }
    }
    Some((x, y))
}

// ------------------------------------------------------------------------------------------------------- filters

/// filter's (and backdrop-filter's) value: Some(empty) for none, None if it isn't valid (a url() to an SVG filter
/// isn't read: the whole list is then as if it weren't there).
pub fn filters(v: &[Tok], current: Color, u: &Units) -> Option<Vec<Filter>> {
    if ident(v).as_deref() == Some("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for p in parts(v) {
        if matches!(trim(p), [Tok::Url(_)]) || function(p).is_some_and(|(f, _)| f == "url") {
            return Some(Vec::new());
        }
        let (f, inner) = function(p)?;
        let amount = |d: f32| -> Option<f32> {
            if trim(inner).is_empty() {
                return Some(d);
            }
            let a = number_or_pct(inner, u)?;
            if a < 0.0 { None } else { Some(a) }
        };
        let k = |code: u8, d: f32, cap: bool| -> Option<Filter> {
            let a = amount(d)?;
            Some(Filter::Color(code, if cap { a.min(1.0) } else { a }))
        };
        out.push(match f {
            "blur" => {
                let r = if trim(inner).is_empty() { 0.0 } else { length(inner, u)?.px };
                if r < 0.0 {
                    return None;
                }
                Filter::Blur(r)
            }
            "brightness" => k(0, 1.0, false)?,
            "contrast" => k(1, 1.0, false)?,
            "grayscale" => k(2, 1.0, true)?,
            "hue-rotate" => Filter::Color(3, if trim(inner).is_empty() { 0.0 } else { angle(inner)? }),
            "invert" => k(4, 1.0, true)?,
            "opacity" => k(5, 1.0, true)?,
            "saturate" => k(6, 1.0, false)?,
            "sepia" => k(7, 1.0, true)?,
            "drop-shadow" => {
                let mut lens = Vec::new();
                let mut color = current;
                for q in parts(inner) {
                    if let Some(l) = length(q, u) {
                        lens.push(l.px);
                    } else {
                        color = values::color(q, current)?;
                    }
                }
                if lens.len() < 2 || lens.len() > 3 {
                    return None;
                }
                // (The blur's length, as box-shadow's, is twice its standard deviation.)
                Filter::DropShadow { x: lens[0], y: lens[1], blur: lens.get(2).copied().unwrap_or(0.0) / 2.0, color }
            }
            _ => return None,
        });
    }
    Some(out)
}

/// mix-blend-mode's value: 255 for normal, else the blend's number (as the canvas has them: multiply 0 ... luminosity
/// 14).
pub fn blend_mode(w: &str) -> Option<u8> {
    Some(match w {
        "normal" => 255,
        "multiply" => 0,
        "screen" => 1,
        "overlay" => 2,
        "darken" => 3,
        "lighten" => 4,
        "color-dodge" => 5,
        "color-burn" => 6,
        "hard-light" => 7,
        "soft-light" => 8,
        "difference" => 9,
        "exclusion" => 10,
        "hue" => 11,
        "saturation" => 12,
        "color" => 13,
        "luminosity" => 14,
        "plus-lighter" | "plus-darker" => 255,
        _ => return None,
    })
}

// ------------------------------------------------------------------------------------------------------- clip-path

fn shape_radius(p: &[Tok], u: &Units) -> Option<ShapeRadius> {
    match ident(p).as_deref() {
        Some("closest-side") => Some(ShapeRadius::ClosestSide),
        Some("farthest-side") => Some(ShapeRadius::FarthestSide),
        Some(_) => None,
        None => Some(ShapeRadius::Len(length(p, u)?)),
    }
}

/// clip-path's value: None for none (or what isn't read: a url() to an SVG clipPath), else the shape and its box.
pub fn clip_path(v: &[Tok], u: &Units) -> Option<ClipPath> {
    if ident(v).as_deref() == Some("none") {
        return None;
    }
    let mut shape = None;
    let mut area = BoxArea::Border;
    for p in parts(v) {
        if let Some(w) = ident(p) {
            area = box_word(&w)?;
            continue;
        }
        let (f, inner) = function(p)?;
        let args = css::split_commas(inner);
        shape = Some(match f {
            "inset" => {
                let ps = parts(inner);
                let round = ps.iter().position(|x| ident(x).as_deref() == Some("round"));
                let (sides, radii) = match round {
                    Some(k) => (&ps[..k], &ps[k + 1..]),
                    None => (&ps[..], &ps[..0]),
                };
                let lens: Vec<Len> = sides.iter().map(|x| length(x, u)).collect::<Option<_>>()?;
                let four = |l: &[Len]| -> Option<[Len; 4]> {
                    Some(match l.len() {
                        1 => [l[0]; 4],
                        2 => [l[0], l[1], l[0], l[1]],
                        3 => [l[0], l[1], l[2], l[1]],
                        4 => [l[0], l[1], l[2], l[3]],
                        _ => return None,
                    })
                };
                let r: Vec<Len> = radii.iter().take_while(|x| trim(x) != [Tok::Delim('/')]).map(|x| length(x, u)).collect::<Option<_>>()?;
                Shape::Inset(four(&lens)?, if r.is_empty() { [Len::ZERO; 4] } else { four(&r)? })
            }
            "circle" | "ellipse" => {
                let ps = parts(inner);
                let at = ps.iter().position(|x| ident(x).as_deref() == Some("at"));
                let (radii, place) = match at {
                    Some(k) => (&ps[..k], &ps[k + 1..]),
                    None => (&ps[..], &ps[..0]),
                };
                let centre = if place.is_empty() { (Pos::CENTER, Pos::CENTER) } else { position(place, u)? };
                if f == "circle" {
                    let r = match radii {
                        [] => ShapeRadius::ClosestSide,
                        [x] => shape_radius(x, u)?,
                        _ => return None,
                    };
                    Shape::Circle(r, centre)
                } else {
                    let (rx, ry) = match radii {
                        [] => (ShapeRadius::ClosestSide, ShapeRadius::ClosestSide),
                        [a, b] => (shape_radius(a, u)?, shape_radius(b, u)?),
                        _ => return None,
                    };
                    Shape::Ellipse(rx, ry, centre)
                }
            }
            "polygon" => {
                let mut even_odd = false;
                let mut points = Vec::new();
                for (k, a) in args.iter().enumerate() {
                    let ps = parts(a);
                    if k == 0 && ps.len() == 1 && let Some(w) = ident(ps[0]) {
                        even_odd = w == "evenodd";
                        continue;
                    }
                    if ps.len() != 2 {
                        return None;
                    }
                    points.push((length(ps[0], u)?, length(ps[1], u)?));
                }
                Shape::Polygon(even_odd, points)
            }
            "path" => {
                let mut even_odd = false;
                let mut d = None;
                for a in &args {
                    match trim(a) {
                        [Tok::Ident(w)] => even_odd = w.eq_ignore_ascii_case("evenodd"),
                        [Tok::Str(s)] => d = Some(s.clone()),
                        _ => return None,
                    }
                }
                Shape::Path(even_odd, d?)
            }
            _ => return None,
        });
    }
    Some(ClipPath { shape: shape.unwrap_or(Shape::Box), area })
}
