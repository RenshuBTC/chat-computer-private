//! CSS animations as they run: where each is at a moment (CSS Animations' timing, as Web Animations defines it: its
//! delay, duration, iterations, direction, fill mode and play state), its timing functions (the cubic Béziers,
//! steps(), linear()), and a property's value between two keyframes' (numbers, lengths and percentages, colours,
//! and functions of them, part by part; `none` against a list of transforms or filters as their identities; what
//! can't be mixed changes halfway, as CSS's discrete animation does).

use crate::css::Tok;
use crate::values;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// A timing function.
#[derive(Clone, Debug, PartialEq)]
pub enum Easing {
    Linear,
    /// cubic-bezier(x1, y1, x2, y2).
    Bezier(f32, f32, f32, f32),
    /// steps(n, <jump>): jump-start, jump-end, jump-none, jump-both as 0 to 3.
    Steps(u32, u8),
    /// linear(...): its points (input, output), inputs in order.
    Points(Vec<(f32, f32)>),
}

pub const EASE: Easing = Easing::Bezier(0.25, 0.1, 0.25, 1.0);

impl Easing {
    /// A timing function from its value's tokens.
    pub fn parse(t: &[Tok]) -> Option<Easing> {
        match values::trim(t) {
            [Tok::Ident(w)] => Some(match w.to_ascii_lowercase().as_str() {
                "linear" => Easing::Linear,
                "ease" => EASE,
                "ease-in" => Easing::Bezier(0.42, 0.0, 1.0, 1.0),
                "ease-out" => Easing::Bezier(0.0, 0.0, 0.58, 1.0),
                "ease-in-out" => Easing::Bezier(0.42, 0.0, 0.58, 1.0),
                "step-start" => Easing::Steps(1, 0),
                "step-end" => Easing::Steps(1, 1),
                _ => return None,
            }),
            [Tok::Function(f), rest @ ..] if rest.last() == Some(&Tok::CloseParen) => {
                let inner = &rest[..rest.len() - 1];
                let args = crate::css::split_commas(inner);
                match f.as_str() {
                    "cubic-bezier" => {
                        let n: Vec<f32> = args.iter().filter_map(|a| number(a)).collect();
                        if n.len() != 4 || !(0.0..=1.0).contains(&n[0]) || !(0.0..=1.0).contains(&n[2]) {
                            return None;
                        }
                        Some(Easing::Bezier(n[0], n[1], n[2], n[3]))
                    }
                    "steps" => {
                        let n = number(args.first()?)?;
                        let jump = match args.get(1).and_then(|a| ident(a)).as_deref() {
                            None | Some("end") | Some("jump-end") => 1,
                            Some("start") | Some("jump-start") => 0,
                            Some("jump-none") => 2,
                            Some("jump-both") => 3,
                            _ => return None,
                        };
                        let n = n as u32;
                        if n < 1 || (jump == 2 && n < 2) {
                            return None;
                        }
                        Some(Easing::Steps(n, jump))
                    }
                    "linear" => linear_points(&args).map(Easing::Points),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The output for an input progress (0 to 1).
    pub fn at(&self, p: f32) -> f32 {
        match self {
            Easing::Linear => p,
            Easing::Bezier(x1, y1, x2, y2) => bezier(*x1, *y1, *x2, *y2, p),
            Easing::Steps(n, jump) => {
                let n = *n as f32;
                let jumps = match jump {
                    2 => n - 1.0,
                    3 => n + 1.0,
                    _ => n,
                };
                let mut step = crate::num::floor(p * n);
                if *jump == 0 || *jump == 3 {
                    step += 1.0;
                }
                if p >= 0.0 && step < 0.0 {
                    step = 0.0;
                }
                if p <= 1.0 && step > jumps {
                    step = jumps;
                }
                step / jumps
            }
            Easing::Points(pts) => {
                let Some(&(x0, y0)) = pts.first() else { return p };
                if p <= x0 {
                    return y0;
                }
                for w in pts.windows(2) {
                    let ((xa, ya), (xb, yb)) = (w[0], w[1]);
                    if p <= xb {
                        return if xb > xa { ya + (yb - ya) * (p - xa) / (xb - xa) } else { yb };
                    }
                }
                pts.last().map_or(p, |l| l.1)
            }
        }
    }
}

/// linear()'s points: outputs, each with no input (spread evenly between its neighbours'), one, or two.
fn linear_points(args: &[&[Tok]]) -> Option<Vec<(f32, f32)>> {
    let mut raw: Vec<(f32, Option<f32>)> = Vec::new();
    for a in args {
        let parts = parts(a);
        let out = number(parts.first()?)?;
        let ins: Vec<f32> = parts[1..].iter().filter_map(|p| match values::trim(p) {
            [Tok::Percent(v)] => Some(v / 100.0),
            _ => None,
        }).collect();
        if ins.is_empty() {
            raw.push((out, None));
        }
        for x in ins.into_iter().take(2) {
            raw.push((out, Some(x)));
        }
    }
    if raw.len() < 2 {
        return None;
    }
    let n = raw.len();
    if raw[0].1.is_none() {
        raw[0].1 = Some(0.0);
    }
    if raw[n - 1].1.is_none() {
        raw[n - 1].1 = Some(1.0);
    }
    // (Each input no less than those before it; missing ones spread evenly between the known ones round them.)
    let mut last = raw[0].1.unwrap_or(0.0);
    for r in raw.iter_mut() {
        if let Some(x) = r.1 {
            let x = x.max(last);
            r.1 = Some(x);
            last = x;
        }
    }
    let mut i = 0;
    while i < n {
        if raw[i].1.is_some() {
            i += 1;
            continue;
        }
        let start = i - 1;
        let mut end = i;
        while raw[end].1.is_none() {
            end += 1;
        }
        let (a, b) = (raw[start].1.unwrap_or(0.0), raw[end].1.unwrap_or(1.0));
        for (k, r) in raw.iter_mut().enumerate().take(end).skip(i) {
            r.1 = Some(a + (b - a) * (k - start) as f32 / (end - start) as f32);
        }
        i = end;
    }
    Some(raw.into_iter().map(|(y, x)| (x.unwrap_or(0.0), y)).collect())
}

/// A cubic Bézier timing function's output at x: its curve's parameter for x found (Newton's method, then halving),
/// then its y there.
fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    if x <= 0.0 {
        // (Before 0 and after 1: on the tangent at the end, as CSS has it.)
        return if x1 > 0.0 { y1 / x1 * x } else if y1 == 0.0 && x2 > 0.0 { y2 / x2 * x } else { 0.0 };
    }
    if x >= 1.0 {
        return if x2 < 1.0 { 1.0 + (y2 - 1.0) / (x2 - 1.0) * (x - 1.0) } else if y2 == 1.0 && x1 < 1.0 { 1.0 + (y1 - 1.0) / (x1 - 1.0) * (x - 1.0) } else { 1.0 };
    }
    let curve = |a: f32, b: f32, t: f32| 3.0 * (1.0 - t) * (1.0 - t) * t * a + 3.0 * (1.0 - t) * t * t * b + t * t * t;
    let slope = |a: f32, b: f32, t: f32| 3.0 * (1.0 - t) * (1.0 - t) * a + 6.0 * (1.0 - t) * t * (b - a) + 3.0 * t * t * (1.0 - b);
    let mut t = x;
    for _ in 0..8 {
        let err = curve(x1, x2, t) - x;
        if err.abs() < 1e-6 {
            return curve(y1, y2, t);
        }
        let d = slope(x1, x2, t);
        if d.abs() < 1e-6 {
            break;
        }
        t -= err / d;
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    t = x;
    for _ in 0..40 {
        let v = curve(x1, x2, t);
        if (v - x).abs() < 1e-6 {
            break;
        }
        if v < x {
            lo = t;
        } else {
            hi = t;
        }
        t = (lo + hi) / 2.0;
    }
    curve(y1, y2, t)
}

// ------------------------------------------------------------------------------------------------- timing

/// One animation's timing: its duration and delay (seconds), iterations (infinity for infinite), direction (0
/// normal, 1 reverse, 2 alternate, 3 alternate-reverse), fill mode (0 none, 1 forwards, 2 backwards, 3 both), and
/// whether it's paused (at its start: its current time 0).
pub struct Timing {
    pub duration: f32,
    pub delay: f32,
    pub count: f32,
    pub direction: u8,
    pub fill: u8,
    pub paused: bool,
}

impl Timing {
    /// Where in its keyframes (0 to 1, before its keyframes' own timing functions) it has the box at `now` (seconds
    /// since it started, as the page's time is), or None where it has no effect (before its delay, or after it's
    /// over, filling neither way).
    pub fn progress(&self, now: f32) -> Option<f32> {
        let current = if self.paused { 0.0 } else { now };
        let local = current - self.delay;
        let count = self.count.max(0.0);
        let duration = self.duration.max(0.0);
        let active = if duration == 0.0 || count == 0.0 { 0.0 } else { duration * count };
        let (overall, iteration_end) = if local < 0.0 {
            if self.fill != 2 && self.fill != 3 {
                return None;
            }
            (0.0, false)
        } else if local >= active {
            if self.fill != 1 && self.fill != 3 {
                return None;
            }
            (count, true)
        } else {
            (local / duration, false)
        };
        if overall.is_infinite() {
            // (An infinite animation's end: never reached.)
            return Some(0.0);
        }
        let mut iteration = crate::num::floor(overall);
        let mut simple = overall - iteration;
        // (Its end, on a whole number of iterations: the end of the last, not the start of one more.)
        if iteration_end && simple == 0.0 && overall > 0.0 {
            simple = 1.0;
            iteration -= 1.0;
        }
        let odd = (iteration as u64) % 2 == 1;
        Some(match self.direction {
            1 => 1.0 - simple,
            2 if odd => 1.0 - simple,
            3 if !odd => 1.0 - simple,
            _ => simple,
        })
    }

    /// Whether it's still running at `now`: changing as time goes on.
    pub fn running(&self, now: f32) -> bool {
        if self.paused || self.duration <= 0.0 {
            return false;
        }
        let local = now - self.delay;
        self.count.is_infinite() || local < self.duration * self.count
    }
}

// ------------------------------------------------------------------------------------------------- values

fn number(t: &[Tok]) -> Option<f32> {
    match values::trim(t) {
        [Tok::Number(n)] => Some(*n),
        _ => None,
    }
}

fn ident(t: &[Tok]) -> Option<String> {
    match values::trim(t) {
        [Tok::Ident(w)] => Some(w.to_ascii_lowercase()),
        _ => None,
    }
}

/// A value's parts: split at whitespace outside brackets.
pub fn parts(t: &[Tok]) -> Vec<&[Tok]> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = None;
    for (k, tok) in t.iter().enumerate() {
        match tok {
            Tok::Ws if depth == 0 => {
                if let Some(s) = start.take() {
                    out.push(&t[s..k]);
                }
                continue;
            }
            Tok::Function(_) | Tok::OpenParen | Tok::OpenSquare | Tok::OpenCurly => depth += 1,
            Tok::CloseParen | Tok::CloseSquare | Tok::CloseCurly => depth -= 1,
            _ => {}
        }
        if start.is_none() {
            start = Some(k);
        }
    }
    if let Some(s) = start {
        out.push(&t[s..]);
    }
    out
}

/// A property's value between two keyframes' (`p`: 0 at `a`, 1 at `b`; perhaps past them, as some timing functions
/// go): None if they can't be mixed (the caller changes from one to the other halfway).
pub fn interpolate(a: &[Tok], b: &[Tok], p: f32) -> Option<Vec<Tok>> {
    let (a, b) = (values::trim(a), values::trim(b));
    if a == b {
        return Some(a.to_vec());
    }
    // `none` against a list of functions (transforms, filters): as the list's identity.
    let is_none = |t: &[Tok]| matches!(t, [Tok::Ident(w)] if w.eq_ignore_ascii_case("none"));
    if is_none(a) && !is_none(b) {
        let id = identity(b)?;
        return interpolate(&id, b, p);
    }
    if is_none(b) && !is_none(a) {
        let id = identity(a)?;
        return interpolate(a, &id, p);
    }
    let (ga, gb) = (crate::css::split_commas(a), crate::css::split_commas(b));
    if ga.len() != gb.len() {
        return None;
    }
    let mut out = Vec::new();
    for (k, (x, y)) in ga.iter().zip(&gb).enumerate() {
        if k > 0 {
            out.push(Tok::Comma);
            out.push(Tok::Ws);
        }
        let (px, py) = (parts(x), parts(y));
        if px.len() != py.len() {
            // (A colour on one side only, as a shadow may leave its out: not mixed.)
            return None;
        }
        for (j, (u, v)) in px.iter().zip(&py).enumerate() {
            if j > 0 {
                out.push(Tok::Ws);
            }
            out.extend(part(u, v, p)?);
        }
    }
    Some(out)
}

/// One part of a value between two keyframes'.
fn part(a: &[Tok], b: &[Tok], p: f32) -> Option<Vec<Tok>> {
    if a == b {
        return Some(a.to_vec());
    }
    let mix = |x: f32, y: f32| x + (y - x) * p;
    match (a, b) {
        ([Tok::Number(x)], [Tok::Number(y)]) => return Some(vec![Tok::Number(mix(*x, *y))]),
        ([Tok::Percent(x)], [Tok::Percent(y)]) => return Some(vec![Tok::Percent(mix(*x, *y))]),
        ([Tok::Dimension(x, u)], [Tok::Dimension(y, v)]) if u == v => return Some(vec![Tok::Dimension(mix(*x, *y), u.clone())]),
        _ => {}
    }
    // A 0 against a length: the length's unit. A percentage against a number (a filter's amount, opacity): as the
    // number it is (50% as 0.5).
    match (a, b) {
        ([Tok::Percent(x)], [Tok::Number(y)]) if *y != 0.0 => return part(&[Tok::Number(x / 100.0)], b, p),
        ([Tok::Number(x)], [Tok::Percent(y)]) if *x != 0.0 => return part(a, &[Tok::Number(y / 100.0)], p),
        ([Tok::Number(z)], [Tok::Dimension(_, u)]) if *z == 0.0 => return part(&[Tok::Dimension(0.0, u.clone())], b, p),
        ([Tok::Dimension(_, u)], [Tok::Number(z)]) if *z == 0.0 => return part(a, &[Tok::Dimension(0.0, u.clone())], p),
        ([Tok::Number(z)], [Tok::Percent(_)]) if *z == 0.0 => return part(&[Tok::Percent(0.0)], b, p),
        ([Tok::Percent(_)], [Tok::Number(z)]) if *z == 0.0 => return part(a, &[Tok::Percent(0.0)], p),
        _ => {}
    }
    // Lengths of different units (or a length and a percentage): calc() of the two.
    let lengthy = |t: &[Tok]| matches!(t, [Tok::Dimension(..)] | [Tok::Percent(_)]) || matches!(t.first(), Some(Tok::Function(f)) if f == "calc");
    if lengthy(a) && lengthy(b) {
        let mut out = vec![Tok::Function(String::from("calc"))];
        let side = |t: &[Tok], k: f32, out: &mut Vec<Tok>| {
            out.push(Tok::OpenParen);
            out.extend(t.iter().cloned());
            out.push(Tok::CloseParen);
            out.extend([Tok::Ws, Tok::Delim('*'), Tok::Ws, Tok::Number(k)]);
        };
        side(a, 1.0 - p, &mut out);
        out.extend([Tok::Ws, Tok::Delim('+'), Tok::Ws]);
        side(b, p, &mut out);
        out.push(Tok::CloseParen);
        return Some(out);
    }
    // Colours: mixed premultiplied, in sRGB.
    if let (Some(x), Some(y)) = (values::color(a, values::BLACK), values::color(b, values::BLACK)) {
        let ch = |c: u32, s: u32| ((c >> s) & 0xFF) as f32;
        let (ax, ay) = (ch(x, 24) / 255.0, ch(y, 24) / 255.0);
        let al = mix(ax, ay).clamp(0.0, 1.0);
        let c = |s: u32| if al > 0.0 { (mix(ch(x, s) * ax, ch(y, s) * ay) / al).clamp(0.0, 255.0) } else { 0.0 };
        return Some(vec![
            Tok::Function(String::from("rgba")),
            Tok::Number(c(16)),
            Tok::Comma,
            Tok::Number(c(8)),
            Tok::Comma,
            Tok::Number(c(0)),
            Tok::Comma,
            Tok::Number(al),
            Tok::CloseParen,
        ]);
    }
    // Two SVG paths' data (path()): mixed number by number where their commands are the same.
    if let ([Tok::Str(x)], [Tok::Str(y)]) = (a, b) {
        return path_data(x, y, p).map(|d| vec![Tok::Str(d)]);
    }
    // Functions of the same name: their arguments, each mixed.
    if let ([Tok::Function(f), ra @ ..], [Tok::Function(g), rb @ ..]) = (a, b)
        && f == g
        && ra.last() == Some(&Tok::CloseParen)
        && rb.last() == Some(&Tok::CloseParen)
    {
        let inner = interpolate(&ra[..ra.len() - 1], &rb[..rb.len() - 1], p)?;
        let mut out = vec![Tok::Function(f.clone())];
        out.extend(inner);
        out.push(Tok::CloseParen);
        return Some(out);
    }
    None
}

/// Two paths' data mixed: the same commands (letter for letter) in the same order, their numbers each mixed.
fn path_data(a: &str, b: &str, p: f32) -> Option<String> {
    let split = |d: &str| -> Option<Vec<(char, Vec<f32>)>> {
        let mut out: Vec<(char, Vec<f32>)> = Vec::new();
        let mut num = String::new();
        let flush = |num: &mut String, out: &mut Vec<(char, Vec<f32>)>| -> Option<()> {
            if !num.is_empty() {
                out.last_mut()?.1.push(num.parse().ok()?);
                num.clear();
            }
            Some(())
        };
        for c in d.chars() {
            if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
                flush(&mut num, &mut out)?;
                out.push((c, Vec::new()));
            } else if c == ',' || c.is_whitespace() {
                flush(&mut num, &mut out)?;
            } else if c == '-' && !num.is_empty() && !num.ends_with(['e', 'E']) {
                flush(&mut num, &mut out)?;
                num.push(c);
            } else if c == '.' && num.contains('.') && !num.contains(['e', 'E']) {
                flush(&mut num, &mut out)?;
                num.push(c);
            } else {
                num.push(c);
            }
        }
        flush(&mut num, &mut out)?;
        Some(out)
    };
    let (x, y) = (split(a)?, split(b)?);
    if x.len() != y.len() {
        return None;
    }
    let mut out = String::new();
    for ((ca, na), (cb, nb)) in x.iter().zip(&y) {
        if ca != cb || na.len() != nb.len() {
            return None;
        }
        out.push(*ca);
        for (k, (u, v)) in na.iter().zip(nb).enumerate() {
            // (An arc's flags stay as the first path has them.)
            let flag = matches!(ca, 'a' | 'A') && (k % 7 == 3 || k % 7 == 4);
            let n = if flag { *u } else { u + (v - u) * p };
            out.push_str(&alloc::format!("{}{n}", if k > 0 { " " } else { "" }));
        }
        out.push(' ');
    }
    Some(String::from(out.trim_end()))
}

/// A list of transform or filter functions' identity: each the same function, doing nothing.
fn identity(t: &[Tok]) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    for (k, p) in parts(t).iter().enumerate() {
        let [Tok::Function(f), ..] = p else { return None };
        if f == "drop-shadow" {
            // (A shadow of no offset, no blur and no colour: its lengths 0, its colour transparent.)
            if k > 0 {
                out.push(Tok::Ws);
            }
            out.push(Tok::Function(f.clone()));
            for (j, a) in parts(&p[1..p.len().saturating_sub(1)]).iter().enumerate() {
                if j > 0 {
                    out.push(Tok::Ws);
                }
                out.push(match a {
                    [Tok::Dimension(_, u)] => Tok::Dimension(0.0, u.clone()),
                    [Tok::Number(_)] => Tok::Number(0.0),
                    _ => Tok::Ident(String::from("transparent")),
                });
            }
            out.push(Tok::CloseParen);
            continue;
        }
        let args: &[&str] = match f.as_str() {
            "translate" | "translatex" | "translatey" | "translatez" => &["0px"],
            "translate3d" => &["0px", "0px", "0px"],
            "scale" | "scalex" | "scaley" | "scalez" => &["1"],
            "scale3d" => &["1", "1", "1"],
            "rotate" | "rotatex" | "rotatey" | "rotatez" | "skew" | "skewx" | "skewy" | "hue-rotate" => &["0deg"],
            "matrix" => &["1", "0", "0", "1", "0", "0"],
            "blur" => &["0px"],
            "brightness" | "contrast" | "opacity" | "saturate" => &["1"],
            "grayscale" | "invert" | "sepia" => &["0"],
            _ => return None,
        };
        if k > 0 {
            out.push(Tok::Ws);
        }
        out.push(Tok::Function(f.clone()));
        // (As many arguments as it has: translate(x, y) against translate(0px, 0px).)
        let given = crate::css::split_commas(&p[1..p.len().saturating_sub(1)]).len().max(1);
        for i in 0..given.max(args.len()).min(if f == "translate" || f == "scale" || f == "skew" { given } else { args.len() }) {
            if i > 0 {
                out.push(Tok::Comma);
            }
            let a = args.get(i).or(args.last()).copied().unwrap_or("0");
            out.push(match a.strip_suffix("px").or(a.strip_suffix("deg")) {
                Some(n) => Tok::Dimension(n.parse().unwrap_or(0.0), String::from(if a.ends_with("px") { "px" } else { "deg" })),
                None => Tok::Number(a.parse().unwrap_or(0.0)),
            });
        }
        out.push(Tok::CloseParen);
    }
    Some(out)
}
