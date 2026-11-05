//! Websites, in the middle of the screen: a site's address typed in the message box ("google.com", "open
//! en.wikipedia.org/wiki/Penang") opens the site there, as the website and the app show one, in a rounded frame with
//! its name and a close button (x) above it, the conversation put away until it's closed. The page is fetched over
//! Chat Computer's own TCP and TLS 1.3 (web.rs), read and laid out by its own browser engine (os/browser: HTML, CSS,
//! the layout, no scripts), its style sheets and pictures fetched as it asks for them, a few at a time, and drawn
//! (browser::paint). It scrolls with the mouse's wheel and the keys; a click follows a link, ticks a box, or puts the
//! caret in a field to type in; Enter in a field sends its form (GET or POST, with the site's cookies).

use crate::web::Fetch;
use crate::{clock, time};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use browser::page::{Kind, Page, State};
use browser::paint::{Painter, Shown, Surface};
pub use browser::address::name_of;
use browser::{Hit, Request};
use text::Type;
use web::Url;

/// The most fetches for a page's style sheets and pictures at once.
const AT_ONCE: usize = 4;
/// The most <meta refresh>es followed in a row.
const MOST_REFRESHES: u32 = 3;
/// A picture's pixels are kept at most this big (in device pixels, each side), and at most this much memory a page.
const PICTURE_SIDE: u32 = 2400;
const PICTURES_BYTES: usize = 160 << 20;
/// The most pages kept to go back to.
const MOST_BACK: usize = 50;
/// While a page loads, it's laid out again at most this often (each style sheet or picture that comes changes it).
const RELAYOUT_MS: u64 = 250;
/// The stack a page is laid out on (Wikipedia's longest pages take about a twentieth of it).
const LAYOUT_STACK: usize = 4 << 20;

/// The page's pictures, decoded (by their resource's place).
struct Pictures(Vec<Option<(Vec<u8>, u32, u32)>>);

impl Shown for Pictures {
    fn picture(&self, n: usize) -> Option<(&[u8], u32, u32)> {
        self.0.get(n)?.as_ref().map(|(p, w, h)| (p.as_slice(), *w, *h))
    }
}

/// What a click on the site did, for the screen.
pub enum Clicked {
    Nothing,
    /// The page changes (a box ticked, a field to type in): drawn again.
    Changed,
    /// Off to another page.
    Going,
}

pub struct Site {
    /// The page shown (None until the first one comes).
    page: Option<Page>,
    /// The page coming (a link followed, a form sent), and where on it to go (its #fragment).
    loading: Option<(Fetch, Option<String>)>,
    /// What's being fetched for the page: (its resource, the fetch).
    fetching: Vec<(usize, Fetch)>,
    pictures: Pictures,
    pictures_bytes: usize,
    /// How far down the page is scrolled (device pixels).
    pub scroll: f32,
    /// The field typed in, and where its caret is (a byte in its value).
    pub focus: Option<usize>,
    pub caret: usize,
    back: Vec<Url>,
    /// Where the page is (the one shown, or the one coming).
    pub url: Url,
    /// What the frame says when there's no page to show (it's coming, or it couldn't be).
    pub note: Option<String>,
    painter: Painter,
    /// The #fragment to scroll to once the page is laid out.
    fragment: Option<String>,
    refreshes: u32,
    /// Something changed that's to be drawn.
    pub dirty: bool,
    laid_at: u64,
    /// The page's size as last laid out (its width and height, in device pixels), and the panel's.
    pub page_h: f32,
    view: (f32, f32),
}

impl Site {
    /// Opens a site: its page asked for.
    pub fn open(url: Url) -> Site {
        log!("Chat Computer kernel: site: opening {url}");
        let mut s = Site {
            page: None,
            loading: None,
            fetching: Vec::new(),
            pictures: Pictures(Vec::new()),
            pictures_bytes: 0,
            scroll: 0.0,
            focus: None,
            caret: 0,
            back: Vec::new(),
            url: url.clone(),
            note: None,
            painter: Painter::new(),
            fragment: None,
            refreshes: 0,
            dirty: true,
            laid_at: 0,
            page_h: 0.0,
            view: (0.0, 0.0),
        };
        s.go(Request { url, body: None }, None, false);
        s
    }

    /// Whether a page has come to show.
    pub fn has_page(&self) -> bool {
        self.page.is_some()
    }

    /// The page's title, for the frame: the page's own, else the site's name.
    pub fn title(&self) -> String {
        match &self.page {
            Some(p) if !self.loading.is_some() => {
                let t = p.title();
                if t.is_empty() { name_of(&self.url) } else { t }
            }
            _ => name_of(&self.url),
        }
    }

    /// Whether a page is coming.
    pub fn loading(&self) -> bool {
        self.loading.is_some() || !self.fetching.is_empty()
    }

    /// Goes to another page (a link, a form sent, an address typed), keeping this one to go back to.
    pub fn go(&mut self, r: Request, fragment: Option<String>, keep: bool) {
        if keep
            && let Some(p) = &self.page
        {
            self.back.push(p.url.clone());
            if self.back.len() > MOST_BACK {
                self.back.remove(0);
            }
        }
        let referer = self.page.as_ref().map(|p| p.url.clone());
        self.stop();
        self.url = r.url.clone();
        let accept = web::http::ACCEPT_PAGE;
        self.loading = Some((Fetch::request(r.url, accept, r.body, referer), fragment));
        self.note = Some(format!("Opening {}\u{2026}", self.url.host));
        self.dirty = true;
    }

    /// Back to the page before, if there is one.
    pub fn go_back(&mut self) -> bool {
        let Some(u) = self.back.pop() else { return false };
        self.stop();
        self.url = u.clone();
        self.loading = Some((Fetch::request(u, web::http::ACCEPT_PAGE, None, None), None));
        self.dirty = true;
        true
    }

    /// Stops what's being fetched (the page coming, and what the one shown asks for): their connections let go.
    pub fn stop(&mut self) {
        if let Some((mut f, _)) = self.loading.take() {
            f.stop();
        }
        for (_, mut f) in self.fetching.drain(..) {
            f.stop();
        }
    }

    /// Moves the page's fetches on (from the main loop): true when something changed that's to be drawn.
    pub fn work(&mut self, ty: &Type) -> bool {
        let _ = ty;
        let mut changed = false;
        // The page itself.
        if let Some((fetch, _)) = self.loading.as_mut()
            && let Some(result) = fetch.step()
        {
            let (_, fragment) = self.loading.take().expect("it's there");
            changed = true;
            match result {
                Ok(p) => self.page_came(p, fragment),
                Err(why) => {
                    log!("Chat Computer kernel: site: couldn't open {}: {why}", self.url);
                    self.note = Some(format!("Couldn't open {}: {why}.", self.url.host));
                }
            }
        }
        // What the page asks for, a few at a time.
        if let Some(page) = self.page.as_mut() {
            while self.fetching.len() < AT_ONCE {
                let Some((i, kind, url, inline)) = page.next_wanted() else { break };
                match (inline, url) {
                    (Some(bytes), _) => {
                        if kind == Kind::Picture {
                            picture_came(page, &mut self.pictures, &mut self.pictures_bytes, i, &bytes);
                        } else {
                            page.failed(i);
                        }
                        changed = true;
                    }
                    (None, Some(u)) => {
                        let accept = if kind == Kind::Sheet { web::http::ACCEPT_CSS } else { web::http::ACCEPT_PICTURE };
                        self.fetching.push((i, Fetch::request(u, accept, None, Some(page.url.clone()))));
                    }
                    (None, None) => page.failed(i),
                }
            }
            let mut k = 0;
            while k < self.fetching.len() {
                let Some(result) = self.fetching[k].1.step() else {
                    k += 1;
                    continue;
                };
                let (i, _) = self.fetching.remove(k);
                changed = true;
                let kind = page.resources.get(i).map(|r| r.kind);
                match (result, kind) {
                    (Ok(got), Some(Kind::Sheet)) => {
                        let css = web::page::decode(&got.bytes, &got.kind);
                        page.sheet_came(i, css);
                    }
                    (Ok(got), Some(Kind::Picture)) => picture_came(page, &mut self.pictures, &mut self.pictures_bytes, i, &got.bytes),
                    _ => page.failed(i),
                }
            }
            if page.changed {
                self.dirty = true;
            }
            if self.fetching.is_empty() && page.resources.iter().all(|r| r.asked) && changed {
                let sheets = page.resources.iter().filter(|r| r.kind == Kind::Sheet && matches!(r.state, State::Css(_))).count();
                let pics = page.resources.iter().filter(|r| r.kind == Kind::Picture && matches!(r.state, State::Picture(..))).count();
                let failed = page.resources.iter().filter(|r| r.state == State::Failed).count();
                log!("Chat Computer kernel: site: {} has come: {sheets} style sheets, {pics} pictures ({failed} couldn't be had)", page.url);
            }
        }
        changed || self.dirty
    }

    /// The page came: read, its #fragment and its fields ready.
    fn page_came(&mut self, got: crate::web::Page, fragment: Option<String>) {
        let kind = got.kind.to_ascii_lowercase();
        self.url = got.url.clone();
        self.pictures = Pictures(Vec::new());
        self.pictures_bytes = 0;
        self.scroll = 0.0;
        self.focus = None;
        self.caret = 0;
        self.painter = Painter::new();
        let html = if kind.contains("html") || kind.contains("xml") || kind.is_empty() {
            web::page::decode(&got.bytes, &got.kind)
        } else if kind.starts_with("text/") {
            // Plain text, as browsers show it.
            let t = web::page::decode(&got.bytes, &got.kind);
            format!("<pre style=\"white-space:pre-wrap\">{}</pre>", escape(&t))
        } else if kind.starts_with("image/") {
            format!("<body style=\"margin:0;background:#0e0e0e;display:flex;justify-content:center\"><img src=\"{}\"></body>", got.url)
        } else {
            let what = kind.split(';').next().unwrap_or("").trim();
            format!(
                "<body style=\"font-family:sans-serif;padding:2em\"><p>This is a file ({}), not a page, so it isn't shown here.</p></body>",
                escape(what)
            )
        };
        let page = Page::new(got.url.clone(), &html);
        log!("Chat Computer kernel: site: read {} ({} bytes{}): {:?}", got.url, got.bytes.len(), if got.cut { ", cut" } else { "" }, page.title());
        // A page that sends itself on at once (<meta http-equiv=refresh>), followed.
        if let Some(to) = page.refresh()
            && self.refreshes < MOST_REFRESHES
            && to != got.url
        {
            self.refreshes += 1;
            log!("Chat Computer kernel: site: the page sends itself on to {to}");
            self.page = Some(page);
            self.go(Request { url: to, body: None }, None, false);
            return;
        }
        self.refreshes = 0;
        self.focus = page.autofocus();
        self.caret = self.focus.map_or(0, |f| page.value(f).len());
        self.page = Some(page);
        self.fragment = fragment;
        self.note = None;
        self.dirty = true;
    }

    /// Draws the page in `area` (x, y, w, h) of the canvas's pixels (`width` to a row), within `clip`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, ty: &Type, pixels: &mut [u32], width: usize, height: usize, area: (i32, i32, i32, i32), clip: (i32, i32, i32, i32), dpr: f32) {
        self.dirty = false;
        let (x, y, w, h) = area;
        let loading = self.loading();
        let Some(page) = self.page.as_mut() else { return };
        let now = time::uptime_ms();
        let size_changed = self.view != (w as f32, h as f32);
        // (While the page comes, laid out again at most every RELAYOUT_MS.)
        if page.laid().is_none() || size_changed || (page.changed && (now.saturating_sub(self.laid_at) >= RELAYOUT_MS || !loading)) {
            let started = time::uptime_ms();
            // (On a stack of its own: laying a page out goes as deep as its elements nest.)
            let (items, tall) = crate::cpu::on_own_stack(LAYOUT_STACK, || {
                let laid = page.lay_out(ty, w as f32, h as f32, dpr);
                (laid.items.len(), laid.height)
            });
            self.page_h = tall;
            self.view = (w as f32, h as f32);
            self.laid_at = now;
            if option_env!("CHATCOMPUTER_TEST").is_some() || time::uptime_ms() - started > 200 {
                log!(
                    "Chat Computer kernel: site: laid out {} ({items} things to draw, {} px tall) in {} ms",
                    page.url,
                    tall as u32,
                    time::uptime_ms() - started
                );
            }
            if let Some(f) = self.fragment.take()
                && let Some(top) = page.fragment_top(&f)
            {
                self.scroll = top;
            }
            if page.changed {
                // (More came while it was laid out: again soon.)
                self.dirty = true;
            }
        }
        let most = (self.page_h - h as f32).max(0.0);
        self.scroll = self.scroll.clamp(0.0, most);
        let Some(laid) = page.laid() else { return };
        let mut surface = Surface { pixels, width, height, clip };
        let inside = (clip.0.max(x), clip.1.max(y), (clip.0 + clip.2).min(x + w) - clip.0.max(x), (clip.1 + clip.3).min(y + h) - clip.1.max(y));
        if inside.2 <= 0 || inside.3 <= 0 {
            return;
        }
        surface.clip = inside;
        self.painter.draw(ty, laid, x, y, self.scroll, &mut surface, &self.pictures, true);
        // The caret, in the field being typed in.
        if let Some(f) = self.focus
            && let Some((cx, cy, ch)) = caret_place(page, ty, f, self.caret)
        {
            let (px, py) = ((x as f32 + cx + 0.5) as i32, (y as f32 + cy - self.scroll + 0.5) as i32);
            let thick = ((dpr + 0.5) as i32).max(1);
            for yy in py.max(inside.1)..(py + (ch + 0.5) as i32).min(inside.1 + inside.3) {
                for xx in px.max(inside.0)..(px + thick).min(inside.0 + inside.2) {
                    pixels_set(surface.pixels, width, xx, yy, 0x000000);
                }
            }
        }
    }

    /// Scrolls by `dy` device pixels (down for more than 0); true if it moved.
    pub fn scroll_by(&mut self, dy: f32) -> bool {
        let most = (self.page_h - self.view.1).max(0.0);
        let to = (self.scroll + dy).clamp(0.0, most);
        if to != self.scroll {
            self.scroll = to;
            self.dirty = true;
            return true;
        }
        false
    }

    /// A click at (x, y) in the page's panel (device pixels from its top left).
    pub fn click(&mut self, x: f32, y: f32) -> Clicked {
        let Some(page) = self.page.as_mut() else { return Clicked::Nothing };
        let hit = page.at(x, y + self.scroll);
        match hit {
            Hit::Link(url, fragment) => {
                log!("Chat Computer kernel: site: link to {url}");
                if url == page.url && fragment.is_some() {
                    if let Some(top) = fragment.as_deref().and_then(|f| page.fragment_top(f)) {
                        self.scroll = top;
                        self.dirty = true;
                    }
                    return Clicked::Changed;
                }
                self.go(Request { url, body: None }, fragment, true);
                Clicked::Going
            }
            Hit::Fragment(f) => {
                if let Some(top) = page.fragment_top(&f) {
                    self.scroll = top;
                    self.dirty = true;
                }
                Clicked::Changed
            }
            Hit::Field(n) => {
                self.focus = Some(n);
                self.caret = page.value(n).len();
                self.dirty = true;
                Clicked::Changed
            }
            Hit::Check(n) => {
                page.toggle(n);
                self.dirty = true;
                Clicked::Changed
            }
            Hit::Choice(n) => {
                // (The next option, round again after the last: there's no list to choose from yet.)
                let (options, chosen) = page.options(n);
                if !options.is_empty() {
                    page.choose(n, (chosen + 1) % options.len());
                    self.dirty = true;
                }
                Clicked::Changed
            }
            Hit::Submit(form, button) => match page.submit(form, Some(button)) {
                Some(r) => {
                    log!("Chat Computer kernel: site: form sent to {} ({})", r.url, if r.body.is_some() { "POST" } else { "GET" });
                    self.go(r, None, true);
                    Clicked::Going
                }
                None => Clicked::Nothing,
            },
            Hit::Nothing => {
                if self.focus.take().is_some() {
                    self.dirty = true;
                    return Clicked::Changed;
                }
                Clicked::Nothing
            }
        }
    }

    /// A key typed while a field has the caret: true if the field took it (letters, Backspace, the arrows, Enter
    /// sending its form, Tab to the next field, Esc leaving it).
    pub fn key(&mut self, p: crate::keyboard::Press) -> bool {
        use crate::keyboard::Key;
        let Some(f) = self.focus else { return false };
        let Some(page) = self.page.as_mut() else { return false };
        let mut value = page.value(f);
        let at = self.caret.min(value.len());
        let before = |s: &str, at: usize| s[..at].chars().next_back().map_or(at, |c| at - c.len_utf8());
        let after = |s: &str, at: usize| s[at..].chars().next().map_or(at, |c| at + c.len_utf8());
        match p.key {
            Key::Char(c) if !p.ctrl && !p.alt && !p.logo => {
                if value.len() < 8192 {
                    value.insert(at, c);
                    self.caret = at + c.len_utf8();
                    page.set_value(f, &value);
                }
            }
            Key::Backspace if at > 0 => {
                let from = before(&value, at);
                value.replace_range(from..at, "");
                self.caret = from;
                page.set_value(f, &value);
            }
            Key::Delete if at < value.len() => {
                let to = after(&value, at);
                value.replace_range(at..to, "");
                page.set_value(f, &value);
            }
            Key::Left => self.caret = before(&value, at),
            Key::Right => self.caret = after(&value, at),
            Key::Home => self.caret = 0,
            Key::End => self.caret = value.len(),
            Key::Tab => {
                self.focus = page.next_field(Some(f), false);
                self.caret = self.focus.map_or(0, |n| page.value(n).len());
            }
            Key::Escape => self.focus = None,
            Key::Enter => {
                if page.doc.name(f) == "textarea" {
                    value.insert(at, '\n');
                    self.caret = at + 1;
                    page.set_value(f, &value);
                } else if let Some(form) = page.form_of(f)
                    && let Some(r) = page.submit(form, None)
                {
                    log!("Chat Computer kernel: site: form sent to {} ({})", r.url, if r.body.is_some() { "POST" } else { "GET" });
                    self.go(r, None, true);
                }
            }
            _ => return false,
        }
        self.dirty = true;
        true
    }

    /// The first field to type in (Tab with none chosen).
    pub fn first_field(&mut self) -> bool {
        let Some(page) = self.page.as_ref() else { return false };
        self.focus = page.next_field(None, false);
        self.caret = self.focus.map_or(0, |n| page.value(n).len());
        self.dirty = true;
        self.focus.is_some()
    }

}

/// A picture's bytes came: decoded (at most PICTURE_SIDE a side, while the page's pictures take less than
/// PICTURES_BYTES), its size given to the page.
fn picture_came(page: &mut Page, pictures: &mut Pictures, bytes_used: &mut usize, i: usize, bytes: &[u8]) {
    let Some((w, h)) = picture::size(bytes) else {
        page.failed(i);
        return;
    };
    let left = PICTURES_BYTES.saturating_sub(*bytes_used) as u64;
    match picture::decode_within(bytes, PICTURE_SIDE, PICTURE_SIDE, [255, 255, 255], left.min(256 << 20)) {
        Ok(p) => {
            *bytes_used += p.rgb.len();
            if pictures.0.len() <= i {
                pictures.0.resize(i + 1, None);
            }
            pictures.0[i] = Some((p.rgb, p.width, p.height));
            page.picture_came(i, w, h);
        }
        Err(_) => page.failed(i),
    }
}

/// Where a field's caret goes (in the page: x, y, and its height), from the words drawn in it.
fn caret_place(page: &Page, ty: &Type, field: usize, caret: usize) -> Option<(f32, f32, f32)> {
    let laid = page.laid()?;
    let value = page.value(field);
    let password = page.doc.element(field).and_then(|e| e.attr("type")).is_some_and(|t| t.eq_ignore_ascii_case("password"));
    // The field's own words (its value, or its placeholder while it's empty).
    let words = laid.items.iter().find(|i| i.node == Some(field) && matches!(i.paint, browser::layout::Paint::Text { .. }));
    if let Some(browser::layout::Paint::Text { x, y, face, size, line, .. }) = words.map(|i| &i.paint) {
        let before = if value.is_empty() {
            String::new()
        } else if password {
            "\u{2022}".repeat(value[..caret.min(value.len())].chars().count())
        } else {
            String::from(&value[..caret.min(value.len())])
        };
        let w = ty.width_in(&before, *face, (*size * 256.0) as i32) as f32 / 256.0;
        return Some((x + w, *y + (line - size * 1.2) / 2.0, size * 1.2));
    }
    let r = page.place_of(field)?;
    Some((r.x + 4.0, r.y + 3.0, (r.h - 6.0).max(4.0)))
}

fn pixels_set(pixels: &mut [u32], width: usize, x: i32, y: i32, rgb: u32) {
    if x >= 0 && y >= 0 {
        let i = y as usize * width + x as usize;
        if i < pixels.len() {
            pixels[i] = rgb;
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[allow(dead_code)]
fn now_seconds() -> Option<i64> {
    clock::now()
}
