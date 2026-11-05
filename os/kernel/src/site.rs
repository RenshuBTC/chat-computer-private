//! Websites, in the middle of the screen: a site's address typed in the message box ("google.com", "open
//! en.wikipedia.org/wiki/Penang") opens the site there, as the website and the app show one, in a rounded frame with
//! its name and a close button (x) above it, the conversation put away until it's closed. The page is fetched over
//! Chat Computer's own TCP and TLS 1.3 (web.rs), read and laid out by its own browser engine (os/browser: HTML, CSS,
//! the layout, and its scripts run by its own JavaScript engine), its style sheets, pictures and scripts fetched as it
//! asks for them, a few at a time, and drawn (browser::paint). Its scripts get its clicks and keys first (and can
//! cancel what they'd do), their timers run as the main loop goes round, what they fetch is fetched here, and their
//! WebSockets are kept open here (web::WebSocket) for as long as the page is shown. It
//! scrolls with the mouse's wheel and the keys; a click follows a link, ticks a box, or puts the caret in a field to
//! type in; Enter in a field sends its form (GET or POST, with the site's cookies).

use crate::web::Fetch;
use crate::{clock, time};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use browser::page::{Kind, Page, State};
use browser::paint::{Painter, Shown, Surface};
use browser::script::{Pixels, SocketEvent, SocketOp};
pub use browser::address::name_of;
use browser::{Hit, Request};
use text::Type;
use web::Url;

/// The most fetches for a page's style sheets, pictures and scripts at once, and for what its scripts fetch.
const AT_ONCE: usize = 4;
const SCRIPT_FETCHES: usize = 6;
/// The most WebSockets a page keeps open at once.
const MOST_SOCKETS: usize = 16;
/// The most lines of a page's console logged.
const CONSOLE_LOGGED: usize = 200;
/// The most <meta refresh>es followed in a row.
const MOST_REFRESHES: u32 = 3;
/// A picture's pixels are kept at most this big (in device pixels, each side), and at most this much memory a page.
const PICTURE_SIDE: u32 = 2400;
const PICTURES_BYTES: usize = 160 << 20;
/// How often a page's animation frames are drawn, at most (ms: 60 a second).
const FRAME_MS: u64 = 16;
/// The most sites whose scripts' storage (localStorage) is kept while Chat Computer runs.
const MOST_STORED: usize = 64;

/// Sites' scripts' storage (localStorage), by origin: kept from page to page, while Chat Computer runs (an app's,
/// across restarts too, with the app: assistant.rs keeps it).
static STORED: crate::sync::SpinLock<BTreeMap<String, BTreeMap<String, String>>> = crate::sync::SpinLock::new(BTreeMap::new());

/// Where the apps the model makes are, as pages: each at a host of its own under this name (which isn't one on the
/// internet), so each has its own storage.
const APPS_HOST: &str = "apps.chatcomputer";

/// An app's name as part of its address: lower case letters and digits, words joined by hyphens.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = String::from(out.trim_end_matches('-'));
    if out.is_empty() { String::from("app") } else { out }
}

/// An app's address, and its origin (where its storage is kept).
pub fn app_url(name: &str) -> Url {
    Url { host: format!("{}.{APPS_HOST}", slug(name)), port: 443, path: String::from("/") }
}

pub fn app_origin(name: &str) -> String {
    format!("https://{}.{APPS_HOST}", slug(name))
}

fn is_app(url: &Url) -> bool {
    url.host.ends_with(&format!(".{APPS_HOST}"))
}

/// What a site's (or an app's) scripts keep in localStorage now.
pub fn storage_of(origin: &str) -> Option<BTreeMap<String, String>> {
    STORED.lock().get(origin).cloned()
}

/// What an app kept before the last restart, back in its storage.
pub fn restore_storage(origin: &str, data: BTreeMap<String, String>) {
    STORED.lock().insert(String::from(origin), data);
}
/// The most pages kept to go back to.
const MOST_BACK: usize = 50;
/// While a page loads, it's laid out again at most this often (each style sheet or picture that comes changes it).
const RELAYOUT_MS: u64 = 250;
/// The stack a page is laid out on (Wikipedia's longest pages take about a twentieth of it).
const LAYOUT_STACK: usize = 4 << 20;

/// The page's pictures, decoded (by their resource's place): each one's colours, size, and alpha (if it has
/// see-through parts); shared with the page's scripts (for drawing on its canvases).
struct Pictures(Vec<Option<Arc<Pixels>>>);

impl Shown for Pictures {
    fn picture(&self, n: usize) -> Option<(&[u8], u32, u32)> {
        self.0.get(n)?.as_ref().map(|p| (p.rgb.as_slice(), p.width, p.height))
    }
    fn alpha(&self, n: usize) -> Option<&[u8]> {
        self.0.get(n)?.as_ref()?.alpha.as_deref()
    }
}

/// Where a page's WebSocket may go: its address (ws: and wss: are reached as https: is: Chat Computer speaks to the
/// web only over TLS), if it may be opened: an app's only to the sites the person allowed it (wss:, on port 443); a
/// page has at most MOST_SOCKETS at once.
fn socket_allowed(address: &str, app: Option<&str>, allowed: &[String], pending: &[String], open: usize) -> Result<Url, String> {
    let rest = address.strip_prefix("wss:").or_else(|| address.strip_prefix("ws:")).ok_or_else(|| String::from("it isn't a WebSocket's address"))?;
    let u = Url::parse(&format!("https:{rest}")).map_err(String::from)?;
    if open >= MOST_SOCKETS {
        return Err(format!("the page has {MOST_SOCKETS} open already"));
    }
    if let Some(app) = app {
        let secure = address.starts_with("wss:") && u.port == 443;
        if !secure || !allowed.iter().any(|h| h.eq_ignore_ascii_case(&u.host)) {
            return Err(if secure && pending.iter().any(|h| h.eq_ignore_ascii_case(&u.host)) {
                format!("the person hasn't allowed the {app} app to reach {} yet", u.host)
            } else {
                format!("the {app} app may reach only the sites the person allowed it, over wss:")
            });
        }
    }
    Ok(u)
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
    /// When the page came (uptime ms: its scripts' clock starts there), and the screen's scale.
    started: u64,
    dpr: f32,
    /// What its scripts fetch: (the request's number, the fetch).
    script_fetches: Vec<(u32, Fetch)>,
    console_logged: usize,
    /// Whether "has come" has been logged for the page.
    come_logged: bool,
    /// When its scripts' last animation frame was asked to be drawn (uptime ms).
    frame_at: u64,
    /// Its pages' scripts' sessionStorage, by origin (for as long as the site is open).
    session: BTreeMap<String, BTreeMap<String, String>>,
    /// The app it shows, if it's one the model made (its name), and the sites it reaches: those it says it needs and
    /// the person allowed (none else, and no cookies); and those it needs that the person hasn't allowed yet.
    app: Option<String>,
    app_allowed: Vec<String>,
    app_pending: Vec<String>,
    /// What its app did that the person should see (in the Actions sidebar), not yet taken.
    actions: Vec<String>,
    /// Its page's scripts' WebSockets: (their number for it, the connection).
    sockets: Vec<(u32, crate::web::WebSocket)>,
}

impl Site {
    /// Opens a site: its page asked for.
    pub fn open(url: Url) -> Site {
        log!("Chat Computer kernel: site: opening {url}");
        let mut s = Site::blank(url.clone());
        s.go(Request { url, body: None }, None, false);
        s
    }

    /// A page made here, not fetched: a file on this PC shown as one (files::view), at `url` (its links, if it has
    /// any, go to the web from there).
    pub fn local(url: Url, html: &str) -> Site {
        let mut s = Site::blank(url.clone());
        s.page_came(crate::web::Page { url, kind: String::from("text/html; charset=utf-8"), bytes: html.as_bytes().to_vec(), cut: false, status: 200, headers: Vec::new() }, None);
        s
    }

    /// An app the model made: its page shown as a site's is, at its own address, reaching only the sites it needs
    /// that the person allowed (`sites`, `allowed`).
    pub fn app(name: &str, html: &str, sites: &[String], allowed: &[String]) -> Site {
        let mut s = Site::blank(app_url(name));
        s.app = Some(String::from(name));
        s.app_allowed = sites.iter().filter(|h| allowed.contains(h)).cloned().collect();
        s.app_pending = sites.iter().filter(|h| !allowed.contains(h)).cloned().collect();
        s.page_came(crate::web::Page { url: app_url(name), kind: String::from("text/html; charset=utf-8"), bytes: html.as_bytes().to_vec(), cut: false, status: 200, headers: Vec::new() }, None);
        s
    }

    fn blank(url: Url) -> Site {
        Site {
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
            started: 0,
            dpr: 1.0,
            script_fetches: Vec::new(),
            console_logged: 0,
            come_logged: false,
            frame_at: 0,
            session: BTreeMap::new(),
            app: None,
            app_allowed: Vec::new(),
            app_pending: Vec::new(),
            actions: Vec::new(),
            sockets: Vec::new(),
        }
    }

    /// The app it shows, if it's one the model made: its name.
    pub fn app_name(&self) -> Option<&str> {
        self.app.as_deref()
    }

    /// The sites the app shown needs that the person hasn't allowed it yet.
    pub fn app_pending(&self) -> &[String] {
        if self.app.is_some() { &self.app_pending } else { &[] }
    }

    /// What its app did that the person should see, for the Actions sidebar.
    pub fn take_actions(&mut self) -> Vec<String> {
        core::mem::take(&mut self.actions)
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
                match (&self.app, t.is_empty()) {
                    (Some(app), true) => app.clone(),
                    (_, true) => name_of(&self.url),
                    _ => t,
                }
            }
            _ => self.app.clone().unwrap_or_else(|| name_of(&self.url)),
        }
    }

    /// Whether a page is coming.
    pub fn loading(&self) -> bool {
        self.loading.is_some() || !self.fetching.is_empty()
    }

    /// Goes to another page (a link, a form sent, an address typed), keeping this one to go back to.
    pub fn go(&mut self, r: Request, fragment: Option<String>, keep: bool) {
        // (Away from an app: a site, which can reach the internet.)
        if !is_app(&r.url) {
            self.app = None;
        }
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
        for (_, mut f) in self.script_fetches.drain(..) {
            f.stop();
        }
        for (_, mut w) in self.sockets.drain(..) {
            w.stop();
        }
    }

    /// The page's clock: ms since it came.
    fn page_now(&self) -> f64 {
        time::uptime_ms().saturating_sub(self.started) as f64
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
                        let accept = match kind {
                            Kind::Sheet => web::http::ACCEPT_CSS,
                            Kind::Script | Kind::Module => web::http::ACCEPT_SCRIPT,
                            Kind::Picture => web::http::ACCEPT_PICTURE,
                        };
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
                    (Ok(got), Some(Kind::Script | Kind::Module)) => {
                        let js = web::page::decode(&got.bytes, &got.kind);
                        page.script_came(i, js);
                    }
                    _ => page.failed(i),
                }
            }
            // Its scripts: those that have come run in their turn, their timers, what they fetch.
            if page.scripting() {
                let now = time::uptime_ms().saturating_sub(self.started) as f64;
                if page.run(now) {
                    changed = true;
                }
                for (id, address, method, headers, body) in page.take_requests() {
                    // An app reaches only the sites the person allowed it, over HTTPS, without cookies.
                    if let Some(app) = &self.app {
                        let to = Url::parse(&address).ok().filter(|u| u.port == 443 && address.starts_with("https:"));
                        match to {
                            Some(u) if self.app_allowed.iter().any(|h| h.eq_ignore_ascii_case(&u.host)) && self.script_fetches.len() < 64 => {
                                let what = format!("The {app} app fetched from {}", u.host);
                                if !self.actions.contains(&what) {
                                    log!("Chat Computer kernel: site: {what}");
                                    self.actions.push(what);
                                }
                                self.script_fetches.push((id, Fetch::app(u, &method, headers, body, self.app_allowed.clone())));
                            }
                            Some(u) if self.app_pending.iter().any(|h| h.eq_ignore_ascii_case(&u.host)) => {
                                page.answer(id, Err(format!("the person hasn't allowed the {app} app to reach {} yet", u.host)));
                            }
                            Some(u) => page.answer(id, Err(format!("the {app} app may reach only the sites the person allowed it, not {}", u.host))),
                            None => page.answer(id, Err(format!("the {app} app may reach only sites over HTTPS that the person allowed it"))),
                        }
                        continue;
                    }
                    match Url::parse(&address) {
                        Ok(u) if self.script_fetches.len() < 64 => {
                            self.script_fetches.push((id, Fetch::script(u, &method, headers, body, Some(page.url.clone()))));
                        }
                        _ => page.answer(id, Err(String::from("this address can't be fetched here"))),
                    }
                }
                // Its WebSockets: opened as it asks (an app's only to the sites the person allowed it, without
                // cookies), its messages sent, closed; what comes of each told to the page.
                for op in page.take_socket_ops() {
                    match op {
                        SocketOp::Open { id, url, protocols } => {
                            let origin = page.origin();
                            match socket_allowed(&url, self.app.as_deref(), &self.app_allowed, &self.app_pending, self.sockets.len()) {
                                Ok(u) => {
                                    if let Some(app) = &self.app {
                                        let what = format!("The {app} app connected to {}", u.host);
                                        if !self.actions.contains(&what) {
                                            log!("Chat Computer kernel: site: {what}");
                                            self.actions.push(what);
                                        }
                                    }
                                    let cookies = self.app.is_none();
                                    self.sockets.push((id, crate::web::WebSocket::open(u, protocols, origin, cookies)));
                                }
                                Err(why) => {
                                    log!("Chat Computer kernel: site: a WebSocket to {url} wasn't opened: {why}");
                                    page.socket_event(id, SocketEvent::Closed(1006, String::new(), false));
                                    changed = true;
                                }
                            }
                        }
                        SocketOp::Send { id, binary, data } => {
                            if let Some((_, w)) = self.sockets.iter_mut().find(|(i, _)| *i == id) {
                                w.send(binary, &data);
                            }
                        }
                        SocketOp::Close { id, code, reason } => {
                            if let Some((_, w)) = self.sockets.iter_mut().find(|(i, _)| *i == id) {
                                w.close(code, &reason);
                            }
                        }
                    }
                }
                for (id, w) in self.sockets.iter_mut() {
                    for ev in w.step() {
                        let ev = match ev {
                            crate::web::WsEvent::Open(p) => SocketEvent::Open(p),
                            crate::web::WsEvent::Text(t) => SocketEvent::Text(t),
                            crate::web::WsEvent::Binary(b) => SocketEvent::Binary(b),
                            crate::web::WsEvent::Closed(code, reason, clean) => SocketEvent::Closed(code, reason, clean),
                        };
                        page.socket_event(*id, ev);
                        changed = true;
                    }
                }
                self.sockets.retain(|(_, w)| !w.is_closed());
                let mut k = 0;
                let mut active = 0;
                while k < self.script_fetches.len() {
                    if active >= SCRIPT_FETCHES {
                        break;
                    }
                    active += 1;
                    let Some(result) = self.script_fetches[k].1.step() else {
                        k += 1;
                        continue;
                    };
                    let (id, _) = self.script_fetches.remove(k);
                    let answer = result.map(|got| browser::script::Response {
                        status: got.status,
                        status_text: String::new(),
                        url: format!("{}", got.url),
                        headers: got.headers.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.clone())).collect(),
                        body: got.bytes,
                    });
                    page.answer(id, answer);
                    // (What came may have set cookies its scripts see.)
                    let url = page.url.clone();
                    page.set_cookies(&crate::web::script_cookies(&url));
                    changed = true;
                }
                // What its scripts set: cookies, and storage, kept for the site.
                let url = page.url.clone();
                for c in page.take_cookies_set() {
                    crate::web::script_set_cookie(&url, &c);
                }
                if let Some((local, session)) = page.take_storage() {
                    let origin = page.origin();
                    {
                        let mut stored = STORED.lock();
                        // (The oldest site's let go when there are many; never an app's.)
                        if !stored.contains_key(&origin) && stored.len() >= MOST_STORED
                            && let Some(k) = stored.keys().find(|k| !k.ends_with(&format!(".{APPS_HOST}"))).cloned()
                        {
                            stored.remove(&k);
                        }
                        stored.insert(origin.clone(), local);
                    }
                    self.session.insert(origin, session);
                    if is_app(&page.url) {
                        crate::assistant::app_data_changed();
                    }
                }
                for line in page.take_console() {
                    if self.console_logged < CONSOLE_LOGGED {
                        self.console_logged += 1;
                        let line: String = line.chars().take(300).collect();
                        log!("Chat Computer kernel: site: console: {line}");
                    }
                }
                if let Some(y) = page.scrolled.take() {
                    self.scroll = y * page.scale();
                    changed = true;
                }
                if page.go_back {
                    page.go_back = false;
                    self.go_back();
                    return true;
                }
                if let Some(r) = page.navigation.take() {
                    log!("Chat Computer kernel: site: the page's script goes to {}", r.url);
                    self.go(r, None, true);
                    return true;
                }
            }
            if page.changed {
                self.dirty = true;
            }
            // (Its scripts drew on a canvas, or want an animation frame: drawn again, at most FRAME_MS apart.)
            if core::mem::take(&mut page.repaint) {
                self.dirty = true;
            }
            let now = time::uptime_ms();
            if page.wants_frame() && now.saturating_sub(self.frame_at) >= FRAME_MS {
                self.frame_at = now;
                self.dirty = true;
            }
            if self.fetching.is_empty() && page.resources.iter().all(|r| r.asked) && changed && !self.come_logged {
                self.come_logged = true;
                let sheets = page.resources.iter().filter(|r| r.kind == Kind::Sheet && matches!(r.state, State::Css(_))).count();
                let pics = page.resources.iter().enumerate().filter(|(i, r)| r.kind == Kind::Picture && matches!(r.state, State::Picture(..)) && !page.is_canvas(*i)).count();
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
        // (Pages run their scripts; what's shown in a page's place here, a picture or a file, doesn't.)
        let scripting = kind.contains("html") || kind.is_empty();
        self.started = time::uptime_ms();
        self.console_logged = 0;
        self.come_logged = false;
        let page = if scripting {
            let epoch = crate::clock::now_ms().unwrap_or(0) as f64;
            let dpr = if self.dpr > 0.0 { self.dpr } else { 1.0 };
            let (w, h) = if self.view.0 > 0.0 { (self.view.0 / dpr, self.view.1 / dpr) } else { (1280.0, 800.0) };
            let mut p = Page::with_scripts(got.url.clone(), &html, epoch, (w, h, dpr), crate::random::u64());
            // (What its scripts kept before, on this site, and its cookies they may see.)
            let origin = p.origin();
            let local = STORED.lock().get(&origin).cloned().unwrap_or_default();
            p.set_storage(local, self.session.get(&origin).cloned().unwrap_or_default());
            p.set_cookies(&crate::web::script_cookies(&got.url));
            p
        } else {
            Page::new(got.url.clone(), &html)
        };
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
        self.dpr = dpr;
        let page_now = self.page_now();
        let Some(page) = self.page.as_mut() else { return };
        // (Its scripts' animation frames, before it's drawn.)
        page.animation_frame(page_now);
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
        self.painter.draw(ty, laid, x, y, self.scroll, &mut surface, &page.shown(&self.pictures), true);
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
            if let Some(p) = self.page.as_mut() {
                let s = p.scale();
                p.scrolled_by_user(to / s);
            }
            return true;
        }
        false
    }

    /// A key with no field to type in: to the page's scripts (on its body). True if they cancelled what it'd do.
    pub fn page_key(&mut self, p: crate::keyboard::Press) -> bool {
        let Some(page) = self.page.as_mut() else { return false };
        if !page.scripting() {
            return false;
        }
        let (key, code) = key_names(p);
        let ok = page.key(None, &key, &code, true, (p.ctrl, false, p.alt));
        page.key(None, &key, &code, false, (p.ctrl, false, p.alt));
        if page.changed {
            self.dirty = true;
        }
        !ok
    }

    /// A click at (x, y) in the page's panel (device pixels from its top left).
    pub fn click(&mut self, x: f32, y: f32) -> Clicked {
        let Some(page) = self.page.as_mut() else { return Clicked::Nothing };
        // The page's scripts first: a click they cancel does nothing more.
        if page.scripting() {
            let node = page.node_at(x, y + self.scroll);
            let s = page.scale();
            let proceed = page.click(node, x / s, y / s);
            if page.changed {
                self.dirty = true;
            }
            if !proceed {
                return Clicked::Changed;
            }
        }
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
                page.focused(Some(n));
                self.dirty = true;
                Clicked::Changed
            }
            Hit::Check(n) => {
                page.toggle(n);
                page.typed(n, true);
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
                    page.focused(None);
                    self.dirty = true;
                    return Clicked::Changed;
                }
                if page.changed {
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
        // (The page's scripts see the key first; one they cancel isn't typed.)
        if page.scripting() {
            let (key, code) = key_names(p);
            let ok = page.key(Some(f), &key, &code, true, (p.ctrl, false, p.alt));
            page.key(Some(f), &key, &code, false, (p.ctrl, false, p.alt));
            if !ok {
                self.dirty = true;
                return true;
            }
        }
        let before_value = page.value(f);
        let mut value = before_value.clone();
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
        if let Some(page) = self.page.as_mut()
            && page.value(f) != before_value
        {
            page.typed(f, false);
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

/// A key's name and code, as a page's scripts see them (KeyboardEvent's key and code).
fn key_names(p: crate::keyboard::Press) -> (String, String) {
    use crate::keyboard::Key;
    match p.key {
        Key::Char(c) => {
            let code = if c.is_ascii_alphabetic() {
                format!("Key{}", c.to_ascii_uppercase())
            } else if c.is_ascii_digit() {
                format!("Digit{c}")
            } else if c == ' ' {
                String::from("Space")
            } else {
                String::new()
            };
            (String::from(c), code)
        }
        Key::Function(n) => (format!("F{n}"), format!("F{n}")),
        k => {
            let name = match k {
                Key::Enter => "Enter",
                Key::Backspace => "Backspace",
                Key::Delete => "Delete",
                Key::Tab => "Tab",
                Key::Escape => "Escape",
                Key::Left => "ArrowLeft",
                Key::Right => "ArrowRight",
                Key::Up => "ArrowUp",
                Key::Down => "ArrowDown",
                Key::Home => "Home",
                Key::End => "End",
                Key::PageUp => "PageUp",
                Key::PageDown => "PageDown",
                _ => "",
            };
            (String::from(name), String::from(name))
        }
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
    match picture::decode_alpha(bytes, PICTURE_SIDE, PICTURE_SIDE, left.min(256 << 20)) {
        Ok((p, alpha)) => {
            *bytes_used += p.rgb.len() + alpha.as_ref().map_or(0, |a| a.len());
            if pictures.0.len() <= i {
                pictures.0.resize(i + 1, None);
            }
            let px = Arc::new(Pixels { rgb: p.rgb, alpha, width: p.width, height: p.height, natural: (w, h) });
            pictures.0[i] = Some(px.clone());
            page.picture_came(i, w, h);
            page.picture_pixels(i, px);
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

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[allow(dead_code)]
fn now_seconds() -> Option<i64> {
    clock::now()
}
