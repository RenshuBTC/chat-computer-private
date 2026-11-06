//! Which picture an <img> shows, as HTML's "select an image source" and Chrome have it: the candidates of the first
//! <source> of its <picture> whose media matches and whose type is one that's read, else its own srcset (and its src
//! as the 1x one, where the srcset has no 1x and no widths); each candidate's density (a width's: it over the size
//! `sizes` gives the picture); and of them, the one Chrome picks for the screen's density. The picture's size in CSS
//! pixels is then its own divided by that density.

use crate::css::{self, Tok, Viewport};
use crate::values::{self, Units};
use alloc::string::String;
use alloc::vec::Vec;

/// A candidate from a srcset: its address, and its width (`w`) or density (`x`); neither is 1x.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub url: String,
    pub width: Option<f32>,
    pub density: Option<f32>,
}

/// A srcset's candidates, as HTML parses them (an address may hold commas; a candidate whose descriptors don't make
/// sense is left out).
pub fn parse(input: &str) -> Vec<Candidate> {
    let s: Vec<char> = input.chars().collect();
    let ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r');
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        while i < s.len() && (ws(s[i]) || s[i] == ',') {
            i += 1;
        }
        if i >= s.len() {
            return out;
        }
        let start = i;
        while i < s.len() && !ws(s[i]) {
            i += 1;
        }
        let mut url: String = s[start..i].iter().collect();
        let mut descriptors: Vec<String> = Vec::new();
        if url.ends_with(',') {
            while url.ends_with(',') {
                url.pop();
            }
        } else {
            // The descriptors, up to a comma outside parentheses.
            while i < s.len() && ws(s[i]) {
                i += 1;
            }
            let mut current = String::new();
            let mut parens = false;
            loop {
                let Some(&c) = s.get(i) else {
                    if !current.is_empty() {
                        descriptors.push(current);
                    }
                    break;
                };
                i += 1;
                if parens {
                    current.push(c);
                    if c == ')' {
                        parens = false;
                    }
                } else if ws(c) {
                    if !current.is_empty() {
                        descriptors.push(core::mem::take(&mut current));
                    }
                } else if c == ',' {
                    if !current.is_empty() {
                        descriptors.push(current);
                    }
                    break;
                } else {
                    if c == '(' {
                        parens = true;
                    }
                    current.push(c);
                }
            }
        }
        let (mut width, mut density, mut height) = (None, None, None);
        let mut error = false;
        for d in &descriptors {
            let (num, kind) = d.split_at(d.len() - d.chars().last().map_or(0, char::len_utf8));
            match kind {
                "w" if width.is_none() && density.is_none() => match num.parse::<u32>() {
                    Ok(v) if v > 0 && num.bytes().all(|b| b.is_ascii_digit()) => width = Some(v as f32),
                    _ => error = true,
                },
                "x" if width.is_none() && density.is_none() && height.is_none() => match num.parse::<f32>() {
                    Ok(v) if v >= 0.0 && v.is_finite() && !num.ends_with('.') && !num.starts_with('+') => density = Some(v),
                    _ => error = true,
                },
                "h" if height.is_none() && density.is_none() => match num.parse::<u32>() {
                    Ok(v) if v > 0 && num.bytes().all(|b| b.is_ascii_digit()) => height = Some(v),
                    _ => error = true,
                },
                _ => error = true,
            }
        }
        if height.is_some() && width.is_none() {
            error = true;
        }
        if !error && !url.is_empty() {
            out.push(Candidate { url, width, density });
        }
    }
}

/// The size (CSS pixels) a sizes attribute gives the picture in this window: the first of its sizes whose media
/// condition matches (or that has none), else 100vw. Font-relative units are of the initial font size, 16px.
pub fn source_size(sizes: Option<&str>, view: Viewport) -> f32 {
    let u = Units::new(16.0, 16.0, view.width, view.height, 1.0);
    if let Some(sizes) = sizes {
        let toks = css::tokens(sizes);
        for part in split_commas(&toks) {
            let part = values::trim(part);
            // Its size: the last component (a length, or a math function); before it, a media condition.
            let Some(last_start) = last_component(part) else { continue };
            let (cond, size) = part.split_at(last_start);
            let Some(len) = values::length(size, &u, view.width) else { continue };
            if len.has_pct() || len.px < 0.0 {
                continue;
            }
            let cond = values::trim(cond);
            if cond.is_empty() || css::media_matches(cond, view) {
                return len.px;
            }
        }
    }
    view.width
}

/// Where a list of tokens' last component starts: its last token, or the function its closing parenthesis ends.
fn last_component(t: &[Tok]) -> Option<usize> {
    let last = t.len().checked_sub(1)?;
    if t[last] != Tok::CloseParen {
        return Some(last);
    }
    let mut depth = 0;
    for k in (0..t.len()).rev() {
        match &t[k] {
            Tok::CloseParen => depth += 1,
            Tok::Function(_) | Tok::OpenParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_commas(t: &[Tok]) -> Vec<&[Tok]> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (k, tok) in t.iter().enumerate() {
        match tok {
            Tok::Function(_) | Tok::OpenParen => depth += 1,
            Tok::CloseParen => depth -= 1,
            Tok::Comma if depth == 0 => {
                out.push(&t[start..k]);
                start = k + 1;
            }
            _ => {}
        }
    }
    out.push(&t[start..]);
    out
}

/// Whether a <source>'s type is a picture that's read.
pub fn type_supported(t: &str) -> bool {
    let t = t.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    matches!(
        t.as_str(),
        "image/jpeg" | "image/jpg" | "image/pjpeg" | "image/png" | "image/apng" | "image/gif" | "image/webp" | "image/svg+xml"
    )
}

/// The candidate Chrome picks for a screen `dpr` device pixels to a CSS pixel, the picture being `size` CSS pixels
/// wide (for widths): its address and density.
pub fn pick(mut candidates: Vec<Candidate>, size: f32, dpr: f32) -> Option<(String, f32)> {
    let size = size.max(1.0);
    let mut dense: Vec<(f32, String)> = candidates
        .drain(..)
        .map(|c| {
            let d = match (c.width, c.density) {
                (Some(w), _) => w / size,
                (None, Some(d)) => d,
                (None, None) => 1.0,
            };
            (d, c.url)
        })
        .collect();
    if dense.is_empty() {
        return None;
    }
    // By density (the first of each density kept).
    dense.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
    dense.dedup_by(|b, a| a.0 == b.0);
    // The least dense that's at least as dense as the screen, unless the one below it is nearer (beyond their
    // geometric mean), or, on a screen of a pixel or less a CSS pixel, the one below is less dense than the screen.
    let mut i = 0;
    while i + 1 < dense.len() {
        let next = dense[i + 1].0;
        if next < dpr {
            i += 1;
            continue;
        }
        let current = dense[i].0;
        if (dpr <= 1.0 && dpr > current) || dpr >= crate::num::sqrt(current * next) {
            i += 1;
        }
        break;
    }
    let (d, url) = dense.swap_remove(i);
    Some((url, d))
}
