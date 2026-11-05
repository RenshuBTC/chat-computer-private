//! A page as Chat Computer's own system shows it: its address and HTML read, the style sheets, pictures and scripts
//! it asks for (the kernel fetches them and hands them in as they come), laid out for the panel's width and ready to
//! draw; its scripts run in their order (script/: the page's own JavaScript engine), with its events (the click, the
//! typing, the page loading), its timers, and what its scripts ask of the network; where a click lands (a link, a
//! field, a button), what's typed into its fields, and its forms sent (the address, and for a POST its body).

use crate::boxes::Tree;
use crate::css::{self, Sheet, Viewport};
use crate::dom::{Data, Document, NodeId};
use crate::html;
use crate::layout::{Item, Laid, Layout, Paint, Rect, Sizes};
use crate::script::{EventInit, Picker, Script, ScriptKind};
use crate::style::{self, Sheets, Styles};
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};
use text::Type;
use web::Url;

/// The browser's own style sheet, and what's added to it in quirks mode (the HTML standard's rendering section).
const UA: &str = include_str!("ua.css");
const QUIRKS: &str = "table { font-weight: initial; font-style: initial; font-size: initial; line-height: initial; \
                      white-space: initial; text-align: initial; }";

/// The most style sheets and pictures one page may ask for.
const MOST_SHEETS: usize = 64;
const MOST_PICTURES: usize = 400;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Sheet,
    Picture,
    Script,
    /// A module a module script imports (its text goes to the scripts' module loader).
    Module,
}

/// Something the page asks for.
#[derive(Clone, Debug)]
pub struct Resource {
    pub kind: Kind,
    /// Where it is (for a picture written into the page, `data:`, the bytes are in `inline` instead).
    pub url: Option<Url>,
    pub inline: Option<Vec<u8>>,
    /// Asked for (the kernel has started fetching it), come, or failed.
    pub asked: bool,
    pub state: State,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Waiting,
    /// A style sheet's text.
    Css(String),
    /// A picture's size in pixels (the kernel keeps its pixels).
    Picture(u32, u32),
    /// A script's text (taken when it runs).
    Script(String),
    Failed,
}

/// Where a style sheet comes from, in the page's order.
enum Source {
    Inline(String),
    /// A <link rel=stylesheet>'s, or an @import's: the resource.
    Linked(usize),
}

/// When a page's script runs.
#[derive(Clone, Copy, PartialEq, Debug)]
enum When {
    /// In the page's order, before the page is read (a plain <script>).
    InOrder,
    /// In the page's order, once the page is read (defer).
    Deferred,
    /// As soon as it has come (async, and those scripts put in the page).
    Soon,
}

/// One of the page's scripts.
struct PageScript {
    node: NodeId,
    /// Its text, for one in the page; its resource, for one with an address.
    text: Option<String>,
    resource: Option<usize>,
    /// A module script's load (the scripts' module loader fetches its modules), and whether it has an address.
    module: Option<(u32, bool)>,
    when: When,
    done: bool,
}

/// What a click lands on.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    /// A link, to this address (and the place on that page its #fragment names).
    Link(Url, Option<String>),
    /// A link to a place on this page (its #fragment).
    Fragment(String),
    /// A field to type in (a text input, a text area).
    Field(NodeId),
    /// A checkbox or radio button (clicking changes it).
    Check(NodeId),
    /// A drop-down list (clicking chooses the next option).
    Choice(NodeId),
    /// A button that sends its form (the form, the button).
    Submit(NodeId, NodeId),
    Nothing,
}

/// A form, sent.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub url: Url,
    /// POST's body (application/x-www-form-urlencoded); None for a GET.
    pub body: Option<Vec<u8>>,
}

pub struct Page {
    pub url: Url,
    pub doc: Document,
    pub resources: Vec<Resource>,
    sources: Vec<Source>,
    /// Each <img> (and the like) element's picture: (element, resource).
    picture_of: Vec<(NodeId, usize)>,
    /// What's laid out now, and for what width.
    laid: Option<Laid>,
    laid_for: (f32, f32, f32),
    /// The styles worked out (kept while the sheets and the width stay the same: typing in a field doesn't change
    /// them).
    styles: Option<Arc<Styles>>,
    /// Something changed that needs the page laid out again.
    pub changed: bool,
    ua: Arc<Sheet>,
    /// The page's own style sheets, parsed (again when they change, or the view does: their @media rules), and the
    /// view they're parsed for.
    author: Arc<Vec<Sheet>>,
    sheets_for: (f32, f32),
    sheets_stale: bool,
    /// What its scripts lay the page out from on the spot needs giving them again.
    inputs_stale: bool,
    /// The page's scripting (None where its scripts don't run), and its scripts in the page's order.
    script: Option<Box<Script>>,
    scripts: Vec<PageScript>,
    /// Imported style sheets: (the sheet, a sheet it imports), in order.
    imports_of: Vec<(usize, usize)>,
    /// DOMContentLoaded and load have been fired.
    dom_loaded: bool,
    load_fired: bool,
    /// Where a script sends the browser (a link it followed, a form it sent), and whether it asked to go back.
    pub navigation: Option<Request>,
    pub go_back: bool,
    /// Where a script scrolled the page to (CSS pixels down), for the system to scroll.
    pub scrolled: Option<f32>,
    /// The page's scale (device pixels to a CSS pixel), as last laid out.
    dpr: f32,
    /// The modules being fetched for its scripts: their resources, and their keys in the module loader's map.
    module_keys: Vec<(usize, String)>,
    /// Its <svg> elements' pictures: (element, its text's hash, the resource, and the colour it was given for
    /// currentColor).
    inline_svgs: Vec<(NodeId, u64, usize, Option<u32>)>,
}

/// The pictures' sizes, for the layout.
struct Known<'a>(&'a [Resource]);

impl Sizes for Known<'_> {
    fn size(&self, n: usize) -> Option<(u32, u32)> {
        match self.0.get(n).map(|r| &r.state) {
            Some(State::Picture(w, h)) => Some((*w, *h)),
            _ => None,
        }
    }
}


impl Page {
    /// A page from its address and HTML (already decoded to text), its scripts not run.
    pub fn new(url: Url, html_text: &str) -> Page {
        Page::make(url, html_text, false, 0.0, (1024.0, 768.0, 1.0))
    }

    /// A page whose scripts run: its clock starting at `epoch` (ms since 1970), its view `view` (CSS pixels wide
    /// and high, device pixels to one). `seed`: the system's random numbers, for crypto.getRandomValues.
    pub fn with_scripts(url: Url, html_text: &str, epoch: f64, view: (f32, f32, f32), seed: u64) -> Page {
        crate::script::seed(seed);
        Page::make(url, html_text, true, epoch, view)
    }

    fn make(url: Url, html_text: &str, scripting: bool, epoch: f64, view: (f32, f32, f32)) -> Page {
        let doc = html::parse_with(html_text, scripting);
        let mut p = Page {
            url,
            doc,
            resources: Vec::new(),
            sources: Vec::new(),
            picture_of: Vec::new(),
            laid: None,
            laid_for: (0.0, 0.0, 0.0),
            styles: None,
            changed: true,
            ua: Arc::new(Sheet::default()),
            author: Arc::new(Vec::new()),
            sheets_for: (-1.0, -1.0),
            sheets_stale: true,
            inputs_stale: true,
            script: None,
            scripts: Vec::new(),
            imports_of: Vec::new(),
            dom_loaded: false,
            load_fired: false,
            navigation: None,
            go_back: false,
            scrolled: None,
            dpr: view.2,
            module_keys: Vec::new(),
            inline_svgs: Vec::new(),
        };
        if scripting {
            p.script = Some(Box::new(Script::new(&p.url.to_string(), epoch, view)));
        }
        // <base href> changes where its links go.
        if let Some(b) = p.doc.find("base").and_then(|b| p.doc.element(b)).and_then(|e| e.attr("href")).map(String::from)
            && let Ok(u) = p.url.join(&b)
        {
            p.url = u;
        }
        p.scan();
        if p.script.is_some() {
            p.find_scripts();
        }
        p
    }

    /// The page's title (its <title>), or its address.
    pub fn title(&self) -> String {
        let t = self.doc.find("title").map(|t| collapse(&self.doc.text_of(t))).unwrap_or_default();
        if t.is_empty() { self.url.host.clone() } else { t }
    }

    /// A <meta http-equiv=refresh> sending the page on at once (within 5 seconds) elsewhere: where.
    pub fn refresh(&self) -> Option<Url> {
        for n in self.doc.descendants(0) {
            let Some(e) = self.doc.element(n) else { continue };
            if e.name != "meta" || !e.attr("http-equiv").is_some_and(|h| h.eq_ignore_ascii_case("refresh")) {
                continue;
            }
            let content = e.attr("content")?;
            let (secs, rest) = content.split_once([';', ',']).unwrap_or((content, ""));
            if secs.trim().parse::<f32>().map_or(true, |s| s > 5.0) {
                continue;
            }
            let rest = rest.trim();
            let target = rest.strip_prefix("url=").or_else(|| rest.strip_prefix("URL=")).or_else(|| rest.strip_prefix("Url=")).unwrap_or(rest);
            let target = target.trim().trim_matches(['\'', '"']);
            if target.is_empty() {
                continue;
            }
            return self.url.join(target.split('#').next().unwrap_or("")).ok();
        }
        None
    }

    /// The style sheets (<style>, <link rel=stylesheet>) and pictures (<img>, <input type=image>, <video poster>)
    /// the page has, in order (again after a script changes the page: what's been fetched kept).
    fn scan(&mut self) {
        self.sources.clear();
        self.picture_of.clear();
        self.sheets_stale = true;
        self.inputs_stale = true;
        let nodes: Vec<NodeId> = self.doc.descendants(0).collect();
        for n in nodes {
            let Some(e) = self.doc.element(n) else { continue };
            match e.name.as_str() {
                "style" => {
                    let media = e.attr("media").map(String::from);
                    let css = self.doc.text_of(n);
                    let css = match media {
                        Some(m) if !m.trim().is_empty() && !m.trim().eq_ignore_ascii_case("all") => format!("@media {m} {{\n{css}\n}}"),
                        _ => css,
                    };
                    self.sources.push(Source::Inline(css));
                }
                "link" => {
                    let rel = e.attr("rel").unwrap_or("").to_ascii_lowercase();
                    let is_sheet = rel.split_ascii_whitespace().any(|r| r == "stylesheet") && !rel.split_ascii_whitespace().any(|r| r == "alternate");
                    let media_ok = e.attr("media").is_none_or(|m| !m.to_ascii_lowercase().contains("print") || m.to_ascii_lowercase().contains("screen"));
                    if !is_sheet || !media_ok || e.has_attr("disabled") {
                        continue;
                    }
                    let Some(href) = e.attr("href") else { continue };
                    if self.resources.iter().filter(|r| r.kind == Kind::Sheet).count() >= MOST_SHEETS {
                        continue;
                    }
                    if let Ok(u) = self.url.join(href.trim()) {
                        let media = e.attr("media").map(String::from);
                        let r = match self.resources.iter().position(|r| r.kind == Kind::Sheet && r.url.as_ref() == Some(&u)) {
                            Some(r) => r,
                            None => self.add(Kind::Sheet, Some(u), None),
                        };
                        // (Its imports first.)
                        let imports: Vec<usize> = self.imports_of.iter().filter(|(s, _)| *s == r).map(|(_, i)| *i).collect();
                        for i in imports {
                            self.sources.push(Source::Linked(i));
                        }
                        // (A media attribute other than all: the sheet's rules inside it, as @media would put them.)
                        if let Some(m) = media.filter(|m| !m.trim().is_empty() && !m.trim().eq_ignore_ascii_case("all")) {
                            self.sources.push(Source::Inline(format!("@media {m} {{")));
                            self.sources.push(Source::Linked(r));
                            self.sources.push(Source::Inline(String::from("}")));
                        } else {
                            self.sources.push(Source::Linked(r));
                        }
                    }
                }
                "svg" => {
                    // A picture drawn from its own text (unless it's inside another: that one's the picture).
                    let mut inside = false;
                    let mut p = self.doc.parent(n);
                    while let Some(a) = p {
                        if self.doc.name(a) == "svg" {
                            inside = true;
                            break;
                        }
                        p = self.doc.parent(a);
                    }
                    if inside || self.picture_of.len() >= MOST_PICTURES {
                        continue;
                    }
                    let mut text = self.doc.outer_html(n);
                    // (currentColor: the colour the page gives the element, unless it says its own.)
                    let color = self.color_of(n);
                    if !e.has_attr("color")
                        && let Some(c) = color
                    {
                        text = format!("<svg color=\"#{c:06x}\"{}", &text[4..]);
                    }
                    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
                    let r = match self.inline_svgs.iter().find(|(k, h, _, _)| *k == n && *h == hash) {
                        Some((_, _, r, _)) => *r,
                        None => {
                            // (Its size is known from its text at once: the page is laid out right before it's drawn.)
                            let size = picture::size(text.as_bytes());
                            let r = self.add(Kind::Picture, None, Some(text.into_bytes()));
                            if let Some((w, h)) = size {
                                self.resources[r].state = State::Picture(w, h);
                            }
                            // (The one it had before: not wanted now.)
                            for (_, _, old, _) in self.inline_svgs.iter().filter(|(k, _, _, _)| *k == n) {
                                self.resources[*old].asked = true;
                                self.resources[*old].inline = None;
                            }
                            self.inline_svgs.retain(|(k, _, _, _)| *k != n);
                            self.inline_svgs.push((n, hash, r, color));
                            r
                        }
                    };
                    self.picture_of.push((n, r));
                }
                "img" | "input" | "video" => {
                    if e.name == "input" && !e.attr("type").is_some_and(|t| t.eq_ignore_ascii_case("image")) {
                        continue;
                    }
                    let address = match e.name.as_str() {
                        "video" => e.attr("poster").map(String::from),
                        _ => picture_address(e.attr("src"), e.attr("srcset"), e.attr("data-src")),
                    };
                    let Some(address) = address else { continue };
                    if self.picture_of.len() >= MOST_PICTURES {
                        continue;
                    }
                    let r = if let Some(bytes) = data_url(&address) {
                        Some(self.add(Kind::Picture, None, Some(bytes)))
                    } else if let Ok(u) = self.url.join(address.trim().split('#').next().unwrap_or("")) {
                        // (The same picture twice on a page: fetched once.)
                        match self.resources.iter().position(|r| r.kind == Kind::Picture && r.url.as_ref() == Some(&u)) {
                            Some(i) => Some(i),
                            None => Some(self.add(Kind::Picture, Some(u), None)),
                        }
                    } else {
                        None
                    };
                    if let Some(r) = r {
                        self.picture_of.push((n, r));
                    }
                }
                _ => {}
            }
        }
    }

    fn add(&mut self, kind: Kind, url: Option<Url>, inline: Option<Vec<u8>>) -> usize {
        self.resources.push(Resource { kind, url, inline, asked: false, state: State::Waiting });
        self.resources.len() - 1
    }

    /// The next thing to fetch (marked asked): style sheets first, then pictures in the page's order.
    pub fn next_wanted(&mut self) -> Option<(usize, Kind, Option<Url>, Option<Vec<u8>>)> {
        let i = self
            .resources
            .iter()
            .position(|r| !r.asked && r.kind == Kind::Sheet)
            .or_else(|| self.resources.iter().position(|r| !r.asked))?;
        let r = &mut self.resources[i];
        r.asked = true;
        Some((i, r.kind, r.url.clone(), r.inline.take()))
    }

    /// Whether the style sheets have all come (or failed): the page can be shown as it's meant to look.
    pub fn sheets_done(&self) -> bool {
        self.resources.iter().all(|r| r.kind != Kind::Sheet || matches!(r.state, State::Css(_) | State::Failed))
    }

    /// A style sheet came: its text (its @imports asked for in turn).
    pub fn sheet_came(&mut self, i: usize, css_text: String) {
        let Some(r) = self.resources.get(i) else { return };
        let base = r.url.clone().unwrap_or_else(|| self.url.clone());
        // Its @imports, before it.
        let imports = css::sheet(&css_text, Viewport { width: 1000.0, height: 800.0 }).imports;
        let mut before = Vec::new();
        for href in imports {
            if self.resources.iter().filter(|r| r.kind == Kind::Sheet).count() >= MOST_SHEETS {
                break;
            }
            if let Ok(u) = base.join(&href) {
                before.push(self.add(Kind::Sheet, Some(u), None));
            }
        }
        if !before.is_empty()
            && let Some(at) = self.sources.iter().position(|s| matches!(s, Source::Linked(k) if *k == i))
        {
            for (k, r) in before.into_iter().enumerate() {
                self.sources.insert(at + k, Source::Linked(r));
                self.imports_of.push((i, r));
            }
        }
        // (url()s inside it are relative to it: made whole now, as the page's own are relative to the page.)
        let css_text = absolute_urls(&css_text, &base);
        self.resources[i].state = State::Css(css_text);
        self.sheets_changed();
        self.changed = true;
    }

    /// A picture came (decoded by the kernel): its size in pixels.
    pub fn picture_came(&mut self, i: usize, w: u32, h: u32) {
        if let Some(r) = self.resources.get_mut(i) {
            r.state = State::Picture(w, h);
            self.changed = true;
            self.inputs_stale = true;
        }
    }

    pub fn failed(&mut self, i: usize) {
        if let Some(r) = self.resources.get_mut(i) {
            if r.kind == Kind::Script {
                r.state = State::Failed;
                return;
            }
            if r.kind == Kind::Module {
                r.state = State::Failed;
                let key = self.module_keys.iter().find(|(k, _)| *k == i).map(|(_, key)| key.clone()).unwrap_or_default();
                self.with_script(|s| {
                    s.module_came(&key, None);
                    s.checkpoint();
                });
                return;
            }
            let was_sheet = r.kind == Kind::Sheet;
            r.state = State::Failed;
            if was_sheet {
                self.sheets_changed();
            }
            self.changed = true;
        }
    }

    /// Lays the page out for a panel `w` by `h` device pixels (`dpr` device pixels a CSS pixel), if it isn't laid out
    /// for that already or something changed.
    pub fn lay_out(&mut self, ty: &Type, w: f32, h: f32, dpr: f32) -> &Laid {
        let size_changed = self.laid_for != (w, h, dpr);
        if size_changed {
            self.styles = None;
        }
        if self.laid.is_none() || self.changed || size_changed {
            let view = Viewport { width: w / dpr, height: h / dpr };
            if self.styles.is_none() {
                self.parse_sheets(view);
                self.styles = Some(Arc::new(style::compute(&self.doc, &Sheets { ua: &self.ua, author: &self.author }, view, dpr)));
                // (An <svg> made before the colour it's drawn in was known: made again.)
                if self.inline_svgs.iter().any(|(n, _, _, c)| self.color_of(*n) != *c) {
                    self.scan();
                    self.sheets_stale = false;
                }
            }
            let styles = self.styles.as_ref().expect("worked out above");
            let mut tree = Tree::build(&self.doc, styles, &mut Picker { of: &self.picture_of });
            let laid = Layout::new(&mut tree, ty, &Known(&self.resources), w, h, dpr).run();
            self.laid = Some(laid);
            self.laid_for = (w, h, dpr);
            self.changed = false;
            self.dpr = dpr;
            self.share_layout(w, h, dpr);
        }
        self.laid.as_ref().expect("laid out above")
    }

    /// The page's style sheets parsed for a view (again only if they or the view changed).
    fn parse_sheets(&mut self, view: Viewport) {
        if !self.sheets_stale && self.sheets_for == (view.width, view.height) {
            return;
        }
        self.ua = Arc::new(if self.doc.quirks { css::sheet(&format!("{UA}\n{QUIRKS}"), view) } else { css::sheet(UA, view) });
        let mut text = String::new();
        let mut author: Vec<Sheet> = Vec::new();
        // (Sheets one after another, in one piece where a link's media wrapped it.)
        for s in &self.sources {
            match s {
                Source::Inline(css) => text.push_str(css),
                Source::Linked(r) => {
                    if let Some(State::Css(css)) = self.resources.get(*r).map(|r| &r.state) {
                        text.push_str(css);
                    }
                }
            }
            text.push('\n');
            if brackets_closed(&text) {
                author.push(css::sheet(&text, view));
                text.clear();
            }
        }
        if !text.trim().is_empty() {
            author.push(css::sheet(&text, view));
        }
        self.author = Arc::new(author);
        self.sheets_for = (view.width, view.height);
        self.sheets_stale = false;
        self.inputs_stale = true;
    }

    /// The sheets changed (or which there are): parsed again when next needed.
    fn sheets_changed(&mut self) {
        self.sheets_stale = true;
        self.inputs_stale = true;
        self.styles = None;
    }

    /// Where each node's box is, the styles, and the view's size, for the page's scripts (getBoundingClientRect,
    /// offsetWidth, getComputedStyle...).
    fn share_layout(&mut self, w: f32, h: f32, dpr: f32) {
        let (Some(laid), Some(s)) = (&self.laid, self.script.as_mut()) else { return };
        let scale = if dpr > 0.0 { dpr } else { 1.0 };
        let host = s.host();
        host.rects = crate::script::rects_of(laid, &self.doc, w, dpr);
        host.view = (w / scale, h / scale, scale);
        host.styles = self.styles.clone();
        host.laid_version = self.doc.version;
    }

    /// What the page's scripts need to lay it out on the spot (its sheets, parsed for its view, and its pictures),
    /// given to them again if it changed.
    fn share_inputs(&mut self) {
        let Some(s) = self.script.as_mut() else { return };
        let h = s.host();
        h.forced_layouts = 0;
        if !self.inputs_stale && h.layout_inputs.is_some() {
            return;
        }
        let (w, ht, _) = h.view;
        self.parse_sheets(Viewport { width: w, height: ht });
        let sizes = self.resources.iter().map(|r| if let State::Picture(w, h) = r.state { Some((w, h)) } else { None }).collect();
        let inputs = crate::script::LayoutInputs { ua: self.ua.clone(), author: self.author.clone(), picture_of: self.picture_of.clone(), sizes };
        if let Some(s) = self.script.as_mut() {
            s.host().layout_inputs = Some(alloc::rc::Rc::new(inputs));
        }
        self.inputs_stale = false;
    }

    // ---------------------------------------------------------------------------------------------- scripts

    /// The page's scripts (those its parser found), in order.
    fn find_scripts(&mut self) {
        let nodes: Vec<NodeId> = self.doc.descendants(0).filter(|&n| self.doc.name(n) == "script").collect();
        for n in nodes {
            self.add_script(n, false);
        }
    }

    /// A <script> to run: in the page's order, deferred, or as soon as it comes. (A module script is deferred
    /// unless it's async; an import map is taken in at once.)
    fn add_script(&mut self, n: NodeId, inserted: bool) {
        let Some(e) = self.doc.element(n) else { return };
        let kind = crate::script::script_kind(e.attr("type"));
        if self.scripts.iter().any(|s| s.node == n) || kind == ScriptKind::Data || (kind == ScriptKind::Classic && e.has_attr("nomodule")) {
            return;
        }
        let src = e.attr("src").map(|s| String::from(s.trim()));
        if kind == ScriptKind::ImportMap {
            let text = self.doc.text_of(n);
            if let Some(s) = self.script.as_mut() {
                s.host().started.push(n);
                s.add_import_map(&text);
            }
            return;
        }
        if kind == ScriptKind::Module {
            let when = if inserted || e.has_attr("async") { When::Soon } else { When::Deferred };
            let address = match &src {
                Some(s) => match self.url.join(s) {
                    Ok(u) => Some(u.to_string()),
                    Err(_) => return,
                },
                None => None,
            };
            let text = if address.is_none() { Some(self.doc.text_of(n)) } else { None };
            let Some(s) = self.script.as_mut() else { return };
            s.host().started.push(n);
            let id = s.load_module(address.as_deref(), text.as_deref());
            self.scripts.push(PageScript { node: n, text: None, resource: None, module: Some((id, address.is_some())), when, done: false });
            self.modules_wanted();
            return;
        }
        let when = match &src {
            _ if inserted || e.has_attr("async") && src.is_some() => When::Soon,
            Some(_) if e.has_attr("defer") => When::Deferred,
            _ => When::InOrder,
        };
        let (text, resource) = match src {
            Some(s) => match self.url.join(&s) {
                Ok(u) => (None, Some(self.add(Kind::Script, Some(u), None))),
                Err(_) => return,
            },
            None => (Some(self.doc.text_of(n)), None),
        };
        if let Some(s) = self.script.as_mut() {
            s.host().started.push(n);
        }
        self.scripts.push(PageScript { node: n, text, resource, module: None, when, done: false });
    }

    /// A script came: its text, to run in its turn (a module's, to the module loader).
    pub fn script_came(&mut self, i: usize, text: String) {
        if let Some(r) = self.resources.get_mut(i) {
            if r.kind == Kind::Module {
                // (Its text is the module loader's now.)
                r.state = State::Script(String::new());
                let key = self.module_keys.iter().find(|(k, _)| *k == i).map(|(_, key)| key.clone()).unwrap_or_default();
                self.with_script(|s| {
                    s.module_came(&key, Some(&text));
                    s.checkpoint();
                });
                return;
            }
            r.state = State::Script(text);
        }
    }

    /// An element's colour (RGB) as the page's styles give it, if they've been worked out.
    fn color_of(&self, n: NodeId) -> Option<u32> {
        self.styles.as_ref().and_then(|st| st.of.get(n).cloned().flatten()).map(|st| st.color & 0x00FF_FFFF)
    }

    /// The modules the module loader wants fetched, asked for.
    fn modules_wanted(&mut self) {
        let Some(s) = self.script.as_mut() else { return };
        let wanted = s.modules_wanted();
        for key in wanted {
            match Url::parse(js::module::split_key(&key).0) {
                Ok(url) => {
                    let i = self.add(Kind::Module, Some(url), None);
                    self.module_keys.push((i, key));
                }
                Err(_) => {
                    if let Some(s) = self.script.as_mut() {
                        s.module_came(&key, None);
                    }
                }
            }
        }
    }

    /// Whether the page runs scripts.
    pub fn scripting(&self) -> bool {
        self.script.is_some()
    }

    /// Runs something with the page's scripting (its tree lent to it), then takes in what it changed.
    fn with_script<T>(&mut self, f: impl FnOnce(&mut Script) -> T) -> Option<T> {
        self.share_inputs();
        let mut s = self.script.take()?;
        s.lend(&mut self.doc);
        let r = f(&mut s);
        s.give_back(&mut self.doc);
        self.script = Some(s);
        self.took_effect();
        Some(r)
    }

    /// What scripts changed: the page laid out again, new style sheets, pictures and scripts asked for, where they
    /// sent the browser.
    fn took_effect(&mut self) {
        let Some(s) = self.script.as_mut() else { return };
        let h = s.host();
        let mutated = core::mem::take(&mut h.mutated);
        let rescan = core::mem::take(&mut h.rescan);
        let new_scripts = core::mem::take(&mut h.new_scripts);
        let navigate = h.navigate.take();
        let submit = h.submit.take();
        let back = h.history_go.take();
        let title = core::mem::take(&mut h.title_changed);
        let scroll = h.scroll.1;
        let url = h.url.href();
        if mutated || title {
            self.styles = None;
            self.changed = true;
        }
        if rescan {
            self.scan();
            self.sheets_changed();
            self.changed = true;
        }
        for n in new_scripts {
            self.add_script(n, true);
        }
        self.modules_wanted();
        if let Some(u) = navigate.and_then(|u| u.to_web()) {
            self.navigation = Some(Request { url: u, body: None });
        }
        if let Some((form, submitter)) = submit {
            self.navigation = self.submit(form, submitter);
        }
        if back.is_some() {
            self.go_back = true;
        }
        if (scroll - self.scrolled_known()).abs() > 0.5 {
            self.scrolled = Some(scroll);
        }
        // (history.pushState: the page's address as its scripts set it.)
        if let Ok(u) = Url::parse(&url)
            && u.host == self.url.host
        {
            self.url = u;
        }
    }

    fn scrolled_known(&self) -> f32 {
        self.scrolled.unwrap_or(0.0)
    }

    /// Tells the page's scripts where the view is scrolled to (CSS pixels down).
    pub fn set_scroll(&mut self, y: f32) {
        if let Some(s) = self.script.as_mut() {
            s.host().scroll.1 = y;
        }
        self.scrolled = Some(y);
    }

    /// Moves the page's scripts on (from the system's loop, `now` ms since the page began): the scripts that have
    /// come run in their turn, DOMContentLoaded and load fired, timers and tasks due run. True if anything ran.
    pub fn run(&mut self, now: f64) -> bool {
        if self.script.is_none() {
            return false;
        }
        let mut ran = false;
        // The page's scripts, in their order.
        loop {
            let next = self.next_script();
            let Some(k) = next else { break };
            if let Some((id, has_src)) = self.scripts[k].module {
                self.scripts[k].done = true;
                let node = self.scripts[k].node;
                let fetched = self.with_script(|s| {
                    s.set_now(now);
                    s.run_module(id, "a module")
                });
                let event = if fetched == Some(false) { "error" } else { "load" };
                if has_src || event == "error" {
                    self.with_script(|s| s.fire(Some(node), &EventInit::new(event, "Event", false, false)));
                }
                ran = true;
                continue;
            }
            let (node, text) = {
                let ps = &mut self.scripts[k];
                ps.done = true;
                let text = match ps.resource {
                    Some(r) => match &mut self.resources[r].state {
                        State::Script(t) => Some(core::mem::take(t)),
                        _ => None,
                    },
                    None => ps.text.take(),
                };
                (ps.node, text)
            };
            if let Some(code) = text {
                let name = String::from(if self.scripts[k].resource.is_some() { "a script" } else { "an inline script" });
                self.with_script(|s| {
                    s.set_now(now);
                    s.run(&code, &name, Some(node));
                });
                // (A script with an address fires load on its element after it runs.)
                if self.scripts[k].resource.is_some() {
                    self.with_script(|s| s.fire(Some(node), &EventInit::new("load", "Event", false, false)));
                }
            } else if self.scripts[k].resource.is_some() {
                self.with_script(|s| s.fire(Some(node), &EventInit::new("error", "Event", false, false)));
            }
            ran = true;
        }
        let in_order_done = self.scripts.iter().all(|s| s.done || s.when == When::Soon);
        if in_order_done && !self.dom_loaded {
            self.dom_loaded = true;
            self.with_script(|s| {
                s.set_now(now);
                s.fire(Some(0), &EventInit::new("readystatechange", "Event", false, false));
                s.fire(Some(0), &EventInit::new("DOMContentLoaded", "Event", true, false))
            });
            ran = true;
        }
        if self.dom_loaded && !self.load_fired && self.resources.iter().all(|r| r.asked && r.state != State::Waiting) && self.scripts.iter().all(|s| s.done) {
            self.load_fired = true;
            self.with_script(|s| {
                s.set_now(now);
                s.host().loaded = true;
                s.fire(Some(0), &EventInit::new("readystatechange", "Event", false, false));
                s.fire(None, &EventInit::new("load", "Event", false, false));
                s.fire(None, &EventInit::new("pageshow", "PageTransitionEvent", false, false))
            });
            ran = true;
        }
        if self.with_script(|s| s.tick(now)).unwrap_or(false) {
            ran = true;
        }
        ran
    }

    /// The next script to run: the first in-order one if it's ready (none, if it isn't); then deferred ones; any
    /// that runs as soon as it comes, once it has.
    fn next_script(&self) -> Option<usize> {
        let ready = |s: &PageScript| match (s.module, s.resource) {
            (Some((id, _)), _) => self.script.as_ref().is_some_and(|sc| sc.module_ready(id)),
            (None, Some(r)) => matches!(self.resources[r].state, State::Script(_) | State::Failed),
            (None, None) => true,
        };
        for (k, s) in self.scripts.iter().enumerate() {
            if s.done || s.when != When::InOrder {
                continue;
            }
            return if ready(s) { Some(k) } else { None }.or_else(|| self.soon_script(&ready));
        }
        for (k, s) in self.scripts.iter().enumerate() {
            if s.done || s.when != When::Deferred {
                continue;
            }
            return if ready(s) { Some(k) } else { None }.or_else(|| self.soon_script(&ready));
        }
        self.soon_script(&ready)
    }

    fn soon_script(&self, ready: &dyn Fn(&PageScript) -> bool) -> Option<usize> {
        self.scripts.iter().position(|s| !s.done && s.when == When::Soon && ready(s))
    }

    /// Runs requestAnimationFrame's callbacks (before the page is drawn): true if any ran.
    pub fn animation_frame(&mut self, now: f64) -> bool {
        self.with_script(|s| s.animation_frame(now)).unwrap_or(false)
    }

    /// When the page's scripts next want to run (ms since the page began), if they do.
    pub fn next_due(&mut self) -> Option<f64> {
        self.script.as_mut().and_then(|s| s.next_due())
    }

    /// What the page's scripts printed (console.log, and the exceptions nothing caught), since last asked.
    pub fn take_console(&mut self) -> Vec<String> {
        self.script.as_mut().map(|s| s.take_console()).unwrap_or_default()
    }

    /// What the page's scripts asked of the network (fetch, XMLHttpRequest), to send: (its number, address, method,
    /// headers, body). Each is answered with `answer`.
    pub fn take_requests(&mut self) -> Vec<(u32, String, String, Vec<(String, String)>, Option<Vec<u8>>)> {
        let Some(s) = self.script.as_mut() else { return Vec::new() };
        let h = s.host();
        let reqs = core::mem::take(&mut h.requests);
        let out = reqs.iter().map(|r| (r.id, r.url.clone(), r.method.clone(), r.headers.clone(), r.body.clone())).collect();
        h.waiting.extend(reqs);
        out
    }

    /// The network's answer to a script's request.
    pub fn answer(&mut self, id: u32, response: Result<crate::script::Response, String>) {
        self.with_script(|s| s.answer(id, response));
    }

    /// A click on the page (where it landed, CSS pixels in the view): the mouse's events to the element there,
    /// and whether its default action is to happen (no script cancelled it).
    pub fn click(&mut self, node: Option<NodeId>, x: f32, y: f32) -> bool {
        let Some(target) = node.map(|n| self.element_at(n)) else { return true };
        use crate::script::EvValue;
        let mouse = |ty: &str, buttons: f64| {
            EventInit::new(ty, "PointerEvent", true, true)
                .with("clientX", EvValue::Num(x as f64))
                .with("clientY", EvValue::Num(y as f64))
                .with("screenX", EvValue::Num(x as f64))
                .with("screenY", EvValue::Num(y as f64))
                .with("buttons", EvValue::Num(buttons))
                .with("detail", EvValue::Num(1.0))
        };
        let r = self.with_script(|s| {
            s.fire(Some(target), &mouse("pointerdown", 1.0));
            s.fire(Some(target), &mouse("mousedown", 1.0));
            s.fire(Some(target), &mouse("pointerup", 0.0));
            s.fire(Some(target), &mouse("mouseup", 0.0));
            s.fire(Some(target), &mouse("click", 0.0))
        });
        r.unwrap_or(true)
    }

    /// The element a node is (a text's parent element).
    fn element_at(&self, n: NodeId) -> NodeId {
        if self.doc.element(n).is_some() { n } else { self.doc.parent_element(n).unwrap_or(n) }
    }

    /// A key pressed (or let go) in the page: its events to the element with the focus (or the body), and whether
    /// its default action is to happen.
    pub fn key(&mut self, focus: Option<NodeId>, key: &str, code: &str, down: bool, mods: (bool, bool, bool)) -> bool {
        if self.script.is_none() {
            return true;
        }
        use crate::script::EvValue;
        let target = focus.or_else(|| self.doc.find("body"));
        let key_code = key_code(key);
        let init = |ty: &str| {
            EventInit::new(ty, "KeyboardEvent", true, true)
                .with("key", EvValue::Str(String::from(key)))
                .with("code", EvValue::Str(String::from(code)))
                .with("keyCode", EvValue::Num(key_code))
                .with("which", EvValue::Num(key_code))
                .with("ctrlKey", EvValue::Bool(mods.0))
                .with("shiftKey", EvValue::Bool(mods.1))
                .with("altKey", EvValue::Bool(mods.2))
        };
        let r = self.with_script(|s| {
            if down {
                let ok = s.fire(target, &init("keydown"));
                if ok && key.chars().count() == 1 {
                    s.fire(target, &init("keypress"));
                }
                ok
            } else {
                s.fire(target, &init("keyup"))
            }
        });
        r.unwrap_or(true)
    }

    /// A field's value changed by typing: its input event (and change, when it's done).
    pub fn typed(&mut self, node: NodeId, done: bool) {
        use crate::script::EvValue;
        self.with_script(|s| {
            s.fire(Some(node), &EventInit::new("input", "InputEvent", true, false).with("inputType", EvValue::Str(String::from("insertText"))));
            if done {
                s.fire(Some(node), &EventInit::new("change", "Event", true, false));
            }
        });
    }

    /// The focus moved to a field (or away from all).
    pub fn focused(&mut self, node: Option<NodeId>) {
        self.with_script(|s| {
            let old = s.host().focus;
            if old == node {
                return;
            }
            s.host().focus = node;
            if let Some(o) = old {
                s.fire(Some(o), &EventInit::new("blur", "FocusEvent", false, false));
                s.fire(Some(o), &EventInit::new("focusout", "FocusEvent", true, false));
            }
            if let Some(n) = node {
                s.fire(Some(n), &EventInit::new("focus", "FocusEvent", false, false));
                s.fire(Some(n), &EventInit::new("focusin", "FocusEvent", true, false));
            }
        });
    }

    /// The window's size changed: its resize event.
    pub fn resized(&mut self) {
        self.inputs_stale = true;
        self.with_script(|s| s.fire(None, &EventInit::new("resize", "UIEvent", false, false)));
    }

    /// The page scrolled (by the person): its scroll event.
    pub fn scrolled_by_user(&mut self, y: f32) {
        self.set_scroll(y);
        self.scrolled = None;
        self.with_script(|s| {
            s.fire(Some(0), &EventInit::new("scroll", "Event", true, false));
        });
    }

    /// What's laid out now (if it has been).
    pub fn laid(&self) -> Option<&Laid> {
        self.laid.as_ref()
    }

    /// The picture resource drawn for an item's `n`.
    pub fn resource(&self, n: usize) -> Option<&Resource> {
        self.resources.get(n)
    }

    /// What's at `x`, `y` (device pixels from the page's top left).
    pub fn at(&self, x: f32, y: f32) -> Hit {
        match self.node_at(x, y) {
            Some(node) => self.hit_from(node),
            None => Hit::Nothing,
        }
    }

    /// The node drawn at `x`, `y` (device pixels from the page's top left), on top.
    pub fn node_at(&self, x: f32, y: f32) -> Option<NodeId> {
        let laid = self.laid.as_ref()?;
        laid.items.iter().rev().find(|i| i.node.is_some() && i.clip.contains(x, y) && bounds(i).contains(x, y)).and_then(|i| i.node)
    }

    /// Device pixels to a CSS pixel, as the page was last laid out.
    pub fn scale(&self) -> f32 {
        if self.dpr > 0.0 { self.dpr } else { 1.0 }
    }

    /// What a click on `node` (or what it's in) does.
    fn hit_from(&self, node: NodeId) -> Hit {
        let mut at = Some(node);
        let mut steps = 0;
        while let Some(n) = at {
            steps += 1;
            if steps > 200 {
                break;
            }
            if let Some(e) = self.doc.element(n) {
                match e.name.as_str() {
                    "a" | "area" => {
                        if let Some(href) = e.attr("href") {
                            let href = href.trim();
                            if href.starts_with("javascript:") {
                                return Hit::Nothing;
                            }
                            let (to, fragment) = match href.split_once('#') {
                                Some((to, f)) => (to, Some(String::from(f))),
                                None => (href, None),
                            };
                            if to.is_empty() {
                                return Hit::Fragment(fragment.unwrap_or_default());
                            }
                            if let Ok(u) = self.url.join(to) {
                                return Hit::Link(u, fragment.filter(|f| !f.is_empty()));
                            }
                        }
                    }
                    "input" => {
                        let kind = e.attr("type").unwrap_or("text").to_ascii_lowercase();
                        if e.has_attr("disabled") {
                            return Hit::Nothing;
                        }
                        return match kind.as_str() {
                            "submit" | "image" => self.form_of(n).map_or(Hit::Nothing, |f| Hit::Submit(f, n)),
                            "checkbox" | "radio" => Hit::Check(n),
                            "button" | "reset" | "file" | "hidden" | "range" | "color" => Hit::Nothing,
                            _ => Hit::Field(n),
                        };
                    }
                    "textarea" => return if e.has_attr("disabled") { Hit::Nothing } else { Hit::Field(n) },
                    "select" => return Hit::Choice(n),
                    "button" => {
                        let kind = e.attr("type").unwrap_or("submit").to_ascii_lowercase();
                        if kind == "submit" && !e.has_attr("disabled") {
                            return self.form_of(n).map_or(Hit::Nothing, |f| Hit::Submit(f, n));
                        }
                        return Hit::Nothing;
                    }
                    "label" => {
                        // A label's field: the one it names, else the one inside it.
                        let target = e.attr("for").and_then(|id| self.by_id(id)).or_else(|| {
                            self.doc.descendants(n).find(|&d| matches!(self.doc.name(d), "input" | "textarea" | "select"))
                        });
                        if let Some(t) = target
                            && t != n
                        {
                            return match self.hit_from(t) {
                                Hit::Submit(..) => Hit::Nothing,
                                h => h,
                            };
                        }
                    }
                    _ => {}
                }
            }
            at = self.doc.parent(n);
        }
        Hit::Nothing
    }

    fn by_id(&self, id: &str) -> Option<NodeId> {
        self.doc.descendants(0).find(|&n| self.doc.element(n).is_some_and(|e| e.id() == Some(id)))
    }

    /// The form a field or button belongs to: its form attribute's, else the one it's in.
    pub fn form_of(&self, node: NodeId) -> Option<NodeId> {
        if let Some(f) = self.doc.element(node).and_then(|e| e.attr("form")).and_then(|id| self.by_id(id))
            && self.doc.name(f) == "form"
        {
            return Some(f);
        }
        self.doc.ancestor_named(node, "form")
    }

    /// What's in a field now.
    pub fn value(&self, node: NodeId) -> String {
        let Some(e) = self.doc.element(node) else { return String::new() };
        match e.attr("value") {
            Some(v) => String::from(v),
            None if e.name == "textarea" => self.doc.text_of(node),
            None => String::new(),
        }
    }

    /// Puts words in a field (as typed).
    pub fn set_value(&mut self, node: NodeId, value: &str) {
        if let Some(e) = self.doc.element_mut(node) {
            e.set_attr("value", value);
            self.changed = true;
        }
    }

    /// A checkbox ticked or not, a radio button chosen (the others of its group not).
    pub fn toggle(&mut self, node: NodeId) {
        let Some(e) = self.doc.element(node) else { return };
        let radio = e.attr("type").is_some_and(|t| t.eq_ignore_ascii_case("radio"));
        if radio {
            let name = String::from(e.attr("name").unwrap_or(""));
            let form = self.form_of(node);
            let group: Vec<NodeId> = self
                .doc
                .descendants(0)
                .filter(|&n| {
                    n != node
                        && self.doc.element(n).is_some_and(|x| {
                            x.name == "input" && x.attr("type").is_some_and(|t| t.eq_ignore_ascii_case("radio")) && x.attr("name") == Some(name.as_str())
                        })
                        && self.form_of(n) == form
                })
                .collect();
            for n in group {
                if let Some(x) = self.doc.element_mut(n) {
                    x.remove_attr("checked");
                }
            }
            if let Some(x) = self.doc.element_mut(node) {
                x.set_attr("checked", "");
            }
        } else if let Some(x) = self.doc.element_mut(node) {
            if x.has_attr("checked") {
                x.remove_attr("checked");
            } else {
                x.set_attr("checked", "");
            }
        }
        self.changed = true;
    }

    /// A drop-down list's options (their words), and which is chosen.
    pub fn options(&self, select: NodeId) -> (Vec<String>, usize) {
        let opts: Vec<NodeId> = self.doc.descendants(select).filter(|&d| self.doc.name(d) == "option").collect();
        let chosen = opts.iter().position(|&o| self.doc.element(o).is_some_and(|e| e.has_attr("selected"))).unwrap_or(0);
        (opts.iter().map(|&o| collapse(&self.doc.text_of(o))).collect(), chosen)
    }

    /// Chooses a drop-down list's `k`th option.
    pub fn choose(&mut self, select: NodeId, k: usize) {
        let opts: Vec<NodeId> = self.doc.descendants(select).filter(|&d| self.doc.name(d) == "option").collect();
        for (i, o) in opts.iter().enumerate() {
            if let Some(e) = self.doc.element_mut(*o) {
                if i == k {
                    e.set_attr("selected", "");
                } else {
                    e.remove_attr("selected");
                }
            }
        }
        self.changed = true;
    }

    /// The fields a page asks to be typed in first (autofocus), else none.
    pub fn autofocus(&self) -> Option<NodeId> {
        self.doc.descendants(0).find(|&n| self.doc.element(n).is_some_and(|e| matches!(e.name.as_str(), "input" | "textarea") && e.has_attr("autofocus")))
    }

    /// The field after (or before) `from` in the page's order, for Tab.
    pub fn next_field(&self, from: Option<NodeId>, back: bool) -> Option<NodeId> {
        let fields: Vec<NodeId> = self
            .doc
            .descendants(0)
            .filter(|&n| {
                self.doc.element(n).is_some_and(|e| {
                    (e.name == "textarea" || (e.name == "input" && !matches!(e.attr("type").map(|t| t.to_ascii_lowercase()).as_deref(), Some("hidden" | "submit" | "button" | "reset" | "checkbox" | "radio" | "image" | "file" | "range" | "color"))))
                        && !e.has_attr("disabled")
                })
            })
            .collect();
        if fields.is_empty() {
            return None;
        }
        let at = from.and_then(|f| fields.iter().position(|&n| n == f));
        Some(match (at, back) {
            (None, false) => fields[0],
            (None, true) => fields[fields.len() - 1],
            (Some(i), false) => fields[(i + 1) % fields.len()],
            (Some(i), true) => fields[(i + fields.len() - 1) % fields.len()],
        })
    }

    /// Where a field is laid out (its box), for its caret and to scroll it into view.
    pub fn place_of(&self, node: NodeId) -> Option<Rect> {
        let laid = self.laid.as_ref()?;
        let mut r: Option<Rect> = None;
        for i in laid.items.iter().filter(|i| i.node == Some(node)) {
            let b = bounds(i);
            r = Some(match r {
                None => b,
                Some(a) => {
                    let x = a.x.min(b.x);
                    let y = a.y.min(b.y);
                    Rect::new(x, y, a.right().max(b.right()) - x, a.bottom().max(b.bottom()) - y)
                }
            });
        }
        r
    }

    /// Where the page's #fragment is (its element's top), to scroll to.
    pub fn fragment_top(&self, fragment: &str) -> Option<f32> {
        let id = web::url::unescape(fragment);
        let node = self.by_id(&id).or_else(|| {
            self.doc.descendants(0).find(|&n| self.doc.element(n).is_some_and(|e| e.name == "a" && e.attr("name") == Some(id.as_str())))
        })?;
        let laid = self.laid.as_ref()?;
        // (Its own items, else the first item of something inside it.)
        let inside: Vec<NodeId> = self.doc.descendants(node).collect();
        laid.items.iter().find(|i| i.node == Some(node) || i.node.is_some_and(|n| inside.contains(&n))).map(|i| bounds(i).y)
    }

    /// A form sent (by `submitter`, the button clicked, if any): where to, and for a POST, what.
    pub fn submit(&self, form: NodeId, submitter: Option<NodeId>) -> Option<Request> {
        let f = self.doc.element(form)?;
        let action = f.attr("action").map(str::trim).filter(|a| !a.is_empty());
        let mut url = match action {
            Some(a) => self.url.join(a.split('#').next().unwrap_or("")).ok()?,
            None => self.url.clone(),
        };
        let post = f.attr("method").is_some_and(|m| m.eq_ignore_ascii_case("post"));
        let mut pairs: Vec<(String, String)> = Vec::new();
        for n in self.doc.descendants(0) {
            let Some(e) = self.doc.element(n) else { continue };
            if !matches!(e.name.as_str(), "input" | "select" | "textarea" | "button") || self.form_of(n) != Some(form) || e.has_attr("disabled") {
                continue;
            }
            let Some(name) = e.attr("name").filter(|s| !s.is_empty()) else { continue };
            let name = String::from(name);
            match e.name.as_str() {
                "input" => {
                    let kind = e.attr("type").unwrap_or("text").to_ascii_lowercase();
                    match kind.as_str() {
                        "checkbox" | "radio" => {
                            if e.has_attr("checked") {
                                pairs.push((name, String::from(e.attr("value").unwrap_or("on"))));
                            }
                        }
                        "submit" | "button" | "reset" | "image" => {
                            if Some(n) == submitter && kind != "reset" && kind != "button" {
                                if kind == "image" {
                                    pairs.push((format!("{name}.x"), String::from("0")));
                                    pairs.push((format!("{name}.y"), String::from("0")));
                                } else {
                                    pairs.push((name, String::from(e.attr("value").unwrap_or("Submit"))));
                                }
                            }
                        }
                        "file" => pairs.push((name, String::new())),
                        _ => pairs.push((name, self.value(n))),
                    }
                }
                "button" => {
                    if Some(n) == submitter {
                        pairs.push((name, String::from(e.attr("value").unwrap_or(""))));
                    }
                }
                "select" => {
                    let opts: Vec<NodeId> = self.doc.descendants(n).filter(|&d| self.doc.name(d) == "option").collect();
                    let chosen = opts.iter().copied().find(|&o| self.doc.element(o).is_some_and(|x| x.has_attr("selected"))).or(opts.first().copied());
                    if let Some(o) = chosen {
                        let v = self.doc.element(o).and_then(|x| x.attr("value")).map(String::from).unwrap_or_else(|| collapse(&self.doc.text_of(o)));
                        pairs.push((name, v));
                    }
                }
                _ => pairs.push((name, self.value(n))),
            }
        }
        let encoded = pairs.iter().map(|(k, v)| format!("{}={}", form_escape(k), form_escape(v))).collect::<Vec<_>>().join("&");
        if post {
            Some(Request { url, body: Some(encoded.into_bytes()) })
        } else {
            let path = url.path.split('?').next().unwrap_or("/");
            url.path = format!("{path}?{encoded}");
            Some(Request { url, body: None })
        }
    }
}

/// The old keyCode of a key (by its name), as pages' scripts read it.
fn key_code(key: &str) -> f64 {
    let c = match key {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" => 13,
        "Shift" => 16,
        "Control" => 17,
        "Alt" => 18,
        "Escape" => 27,
        " " => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Delete" => 46,
        k if k.chars().count() == 1 => {
            let ch = k.chars().next().unwrap_or(' ');
            if ch.is_ascii_alphabetic() { ch.to_ascii_uppercase() as u32 } else if ch.is_ascii_digit() { ch as u32 } else { 0 }
        }
        _ => 0,
    };
    c as f64
}

/// An item's rectangle.
pub fn bounds(i: &Item) -> Rect {
    match &i.paint {
        Paint::Rect { x, y, w, h, .. } | Paint::Border { x, y, w, h, .. } | Paint::Gradient { x, y, w, h, .. } | Paint::Picture { x, y, w, h, .. } => {
            Rect::new(*x, *y, *w, *h)
        }
        Paint::Text { x, y, w, line, .. } => Rect::new(*x, *y, *w, *line),
    }
}

/// Spaces run together, and trimmed.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a style sheet's text so far has closed all its braces (a link's media wraps it in @media {...}).
fn brackets_closed(css: &str) -> bool {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut chars = css.chars().peekable();
    let mut comment = false;
    while let Some(c) = chars.next() {
        if comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                comment = false;
            }
            continue;
        }
        match quote {
            Some(q) => {
                if c == '\\' {
                    chars.next();
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    comment = true;
                }
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            },
        }
    }
    depth <= 0
}

/// The picture an <img> shows: its src, else the first of its srcset, else data-src (pages that load their
/// pictures with a script put them there); a src that's only a placeholder gives way to the others.
fn picture_address(src: Option<&str>, srcset: Option<&str>, data_src: Option<&str>) -> Option<String> {
    let placeholder = |s: &str| s.trim().is_empty() || (s.starts_with("data:") && s.len() < 200);
    if let Some(s) = src
        && !placeholder(s)
    {
        return Some(String::from(s.trim()));
    }
    if let Some(set) = srcset {
        // (Each candidate: an address, then its width or density.)
        if let Some(first) = set.split(',').map(str::trim).find(|c| !c.is_empty()) {
            let address = first.split_ascii_whitespace().next().unwrap_or("");
            if !address.is_empty() {
                return Some(String::from(address));
            }
        }
    }
    if let Some(s) = data_src
        && !s.trim().is_empty()
    {
        return Some(String::from(s.trim()));
    }
    src.map(|s| String::from(s.trim())).filter(|s| !s.is_empty())
}

/// A data: URL's bytes (base64 or percent-encoded).
pub fn data_url(s: &str) -> Option<Vec<u8>> {
    let rest = s.trim().strip_prefix("data:")?;
    let (head, data) = rest.split_once(',')?;
    if head.ends_with(";base64") {
        base64(data)
    } else {
        Some(web::url::unescape(data).into_bytes())
    }
}

fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            b'%' => continue,
            _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// A form's words, escaped as forms send them (spaces as +).
fn form_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A style sheet's url()s made whole against where it came from.
fn absolute_urls(css: &str, base: &Url) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(at) = rest.find("url(") {
        out.push_str(&rest[..at + 4]);
        rest = &rest[at + 4..];
        let end = rest.find(')').unwrap_or(rest.len());
        let inner = rest[..end].trim();
        let (q, address) = match inner.chars().next() {
            Some(c @ ('"' | '\'')) => (Some(c), inner.trim_matches(c)),
            _ => (None, inner),
        };
        let whole = if address.starts_with("data:") { None } else { base.join(address).ok().map(|u| u.to_string()) };
        match (whole, q) {
            (Some(w), _) => {
                out.push('"');
                out.push_str(&w);
                out.push('"');
            }
            (None, _) => out.push_str(&rest[..end]),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[allow(dead_code)]
fn unused(_: Data) {
    let _ = vec![0u8];
}
