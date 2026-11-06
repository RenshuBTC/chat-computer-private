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
use crate::srcset;
use crate::style::{self, Sheets, Styles};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
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
/// At most this many web fonts fetched for a page.
const MOST_FONTS: usize = 64;
/// The most frames (<iframe>s with documents of their own) a page shows at once, how many it may have had in all
/// (scripts put them in and take them out), and how deep frames go in frames.
const MOST_FRAMES: usize = 16;
const MOST_FRAMES_EVER: usize = 64;
const MOST_FRAME_DEPTH: u8 = 3;
/// Where the numbers the page gives its frames' requests and WebSockets start (its own scripts' are far below).
const FRAME_IDS: u32 = 1 << 30;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Sheet,
    Picture,
    Script,
    /// A module a module script imports (its text goes to the scripts' module loader).
    Module,
    /// A web font an @font-face rule names (TrueType, WOFF or WOFF2).
    Font,
    /// A frame's document (an <iframe>'s src): its HTML, or a picture or text, shown in its box (`frame_came`).
    Frame,
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
    /// A picture's own size as CSS sizes it, where it isn't simply its pixels' (an SVG's).
    pub own: Option<picture::Intrinsic>,
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
    /// A web font that's come (kept in text::loaded: its number).
    Font(u16),
    /// A frame's document that's come (its frame's page now).
    Document,
    Failed,
}

/// Where a style sheet comes from, in the page's order.
enum Source {
    Inline(String),
    /// A <link rel=stylesheet>'s, or an @import's: the resource.
    Linked(usize),
    /// A <style> in a shadow tree (its rules for that tree): its text, and the tree's shadow root.
    Scoped(String, NodeId),
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

/// What an <iframe> shows: a document from an address, one written into it (its srcdoc, or a data: address's), or
/// none (about:blank).
#[derive(Clone, Debug, PartialEq)]
enum FrameSource {
    Address(Url),
    Written(String),
    Blank,
}

/// One of the page's <iframe>s: its document, a page of its own, laid out in the frame's box and drawn there.
struct Frame {
    node: NodeId,
    source: FrameSource,
    /// The resource its document is fetched as (Kind::Frame), once it's asked for (for what it shows now).
    resource: Option<usize>,
    page: Option<Box<Page>>,
    /// Its content box as last laid out (device pixels, the page's coordinates), and whether it's shown (laid out,
    /// of some size): a frame that isn't (display: none) isn't fetched.
    rect: Rect,
    shown: bool,
    /// The page's time (ms) when its document came: its own clock starts there.
    started: f64,
    /// Its <iframe>'s load event has been fired, for the document it shows now.
    loaded: bool,
    /// Its <iframe> isn't in the page any more (kept, so its number isn't another's).
    gone: bool,
}

/// What an <object> or <embed> shows, by its type (or, where it gives none, its address's ending): a document (HTML,
/// or an SVG drawn as one) in a frame of its own, a picture, or neither (what a plugin would have shown: an
/// <object>'s fallback, its children, shows instead, as in Chrome, which has no plugins).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Embedded {
    Document,
    Picture,
    Neither,
}

pub(crate) fn embedded(e: &crate::dom::Element) -> Embedded {
    let address = match e.name.as_str() {
        "object" => e.attr("data"),
        "embed" => e.attr("src"),
        _ => None,
    };
    let Some(address) = address.map(str::trim).filter(|a| !a.is_empty()) else { return Embedded::Neither };
    let lower = address.to_ascii_lowercase();
    let ty = match e.attr("type").map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty()) {
        Some(t) => t,
        None if lower.starts_with("data:") => String::from(lower[5..].split([',', ';']).next().unwrap_or("")),
        None => {
            let path = lower.split(['?', '#']).next().unwrap_or("");
            let ending = path.rsplit('/').next().unwrap_or("").rsplit_once('.').map_or("", |(_, x)| x);
            String::from(match ending {
                "html" | "htm" | "shtml" | "xhtml" | "xht" => "text/html",
                "svg" => "image/svg+xml",
                "png" | "jpg" | "jpeg" | "gif" | "webp" => "image/png",
                _ => "",
            })
        }
    };
    let ty = ty.split(';').next().unwrap_or("").trim();
    if ty.contains("html") || ty.contains("svg") || ty == "text/xml" || ty == "application/xml" || ty == "text/plain" {
        Embedded::Document
    } else if matches!(ty, "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp" | "image/apng") {
        Embedded::Picture
    } else {
        Embedded::Neither
    }
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
    /// The densities of the pictures elements show (their srcset's candidates'), where they aren't 1.
    densities: Vec<(NodeId, f32)>,
    /// Pictures chosen, then not before they were fetched (marked failed, unless they're chosen again).
    dropped: Vec<usize>,
    /// The window pictures are chosen for (CSS pixels wide and high, device pixels to one).
    view: (f32, f32, f32),
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
    /// Its web fonts (its sheets' @font-face rules), as they've come.
    web_fonts: Arc<Vec<style::WebFont>>,
    /// Each web font asked for: its face, the resource it's fetched as, and its number once it's come.
    font_loads: Vec<(crate::css::FontFace, usize, Option<u16>)>,
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
    /// Its <svg> elements' pictures: (element, its text's hash, the resource, and what the page's style sheets gave
    /// it (svg_look's, hashed): the colour for currentColor, and its fills and strokes).
    inline_svgs: Vec<(NodeId, u64, usize, u64)>,
    /// Its <canvas> elements' pictures (their pixels are its scripts'): (element, resource).
    canvases: Vec<(NodeId, usize)>,
    /// Pictures written into it (data: addresses), by their address's hash: their resources.
    data_pictures: Vec<(u64, usize)>,
    /// The <img>s whose load (or error) event has been fired, for the picture they show.
    img_fired: alloc::collections::BTreeSet<(NodeId, usize)>,
    /// Its scripts drew on a canvas: the page is to be drawn again (not laid out).
    pub repaint: bool,
    /// The pictures its styles show (backgrounds' and masks' url()s): each address as written, and its resource.
    style_pictures: BTreeMap<String, usize>,
    /// How far down the view is scrolled (CSS pixels), for what's fixed to the window.
    view_scroll: f32,
    /// The page's time (ms since it began), as last given (run, animation_frame).
    clock_ms: f64,
    /// Whether its CSS animations are drawn as they run (at its time), or as they rest (some time after it has come:
    /// for a system that can't draw it again often enough).
    pub animate: bool,
    /// Its own layout is out of date (as against its frames': theirs changing only puts them in their places again).
    relayout: bool,
    /// Its <iframe>s, each with its own document.
    frames: Vec<Frame>,
    /// A frame's document changed: laid out again in its place.
    frames_changed: bool,
    /// What's laid out, before its frames' documents are put in their places (kept while it has frames).
    own: Option<Laid>,
    /// The resources its frames' documents ask for, fetched as the page's own (the system knows only the page's
    /// numbers): the page's number for each, its frame and the frame's own number, and back.
    frame_resources: BTreeMap<usize, (usize, usize)>,
    frame_numbers: BTreeMap<(usize, usize), usize>,
    /// Its frames' scripts' requests and WebSockets, by the page's numbers for them: their frames and their own.
    frame_requests: BTreeMap<u32, (usize, u32)>,
    frame_sockets: BTreeMap<u32, (usize, u32)>,
    next_frame_id: u32,
    /// How deep in frames the page is (0: in a window of its own), and the addresses of the pages it's in, outermost
    /// first (for a document's say on which sites may show it in a frame).
    depth: u8,
    ancestors: Vec<Url>,
    /// When its clock began (ms since 1970).
    epoch: f64,
}

/// The page's pictures with its canvases' among them (for the painter).
pub struct WithCanvases<'a> {
    page: &'a Page,
    inner: &'a dyn crate::paint::Shown,
}

impl crate::paint::Shown for WithCanvases<'_> {
    fn picture(&self, n: usize) -> Option<(&[u8], u32, u32)> {
        match self.page.canvas_of(n) {
            Some(c) => Some((&c.rgb, c.w, c.h)),
            None if self.page.is_canvas(n) => None,
            None => self.inner.picture(n),
        }
    }
    fn alpha(&self, n: usize) -> Option<&[u8]> {
        match self.page.canvas_of(n) {
            Some(c) => Some(&c.alpha),
            None => self.inner.alpha(n),
        }
    }
    fn version(&self, n: usize) -> u64 {
        match self.page.canvas_of(n) {
            Some(c) => c.version,
            None => self.inner.version(n),
        }
    }
}

/// The pictures' sizes, and those the styles show by their addresses, for the layout.
struct Known<'a> {
    resources: &'a [Resource],
    images: &'a BTreeMap<String, usize>,
}

impl Sizes for Known<'_> {
    fn size(&self, n: usize) -> Option<(u32, u32)> {
        match self.resources.get(n).map(|r| &r.state) {
            Some(State::Picture(w, h)) => Some((*w, *h)),
            _ => None,
        }
    }

    fn intrinsic(&self, n: usize) -> Option<picture::Intrinsic> {
        let (w, h) = self.size(n)?;
        Some(self.resources[n].own.unwrap_or(picture::Intrinsic::sized(w, h)))
    }

    fn image(&self, url: &str) -> Option<usize> {
        self.images.get(url).copied()
    }
}


impl Page {
    /// A page from its address and HTML (already decoded to text), its scripts not run.
    pub fn new(url: Url, html_text: &str) -> Page {
        Page::make(url, html_text, false, 0.0, (1024.0, 768.0, 1.0), 0)
    }

    /// A page whose scripts run: its clock starting at `epoch` (ms since 1970), its view `view` (CSS pixels wide
    /// and high, device pixels to one). `seed`: the system's random numbers, for crypto.getRandomValues.
    pub fn with_scripts(url: Url, html_text: &str, epoch: f64, view: (f32, f32, f32), seed: u64) -> Page {
        crate::script::seed(seed);
        Page::make(url, html_text, true, epoch, view, 0)
    }

    /// A frame's document: a page `depth` frames deep.
    fn make_in_frame(url: Url, html_text: &str, scripting: bool, epoch: f64, view: (f32, f32, f32), depth: u8) -> Page {
        Page::make(url, html_text, scripting, epoch, view, depth)
    }

    fn make(url: Url, html_text: &str, scripting: bool, epoch: f64, view: (f32, f32, f32), depth: u8) -> Page {
        // (An XHTML document, .xht or .xhtml, is XML: its CDATA sections are just their text, as in its style sheets'
        // "<![CDATA[ ... ]]>".)
        let path = url.path.to_ascii_lowercase();
        let unwrapped;
        let html_text = if (path.ends_with(".xht") || path.ends_with(".xhtml")) && html_text.contains("<![CDATA[") {
            unwrapped = html_text.replace("<![CDATA[", "").replace("]]>", "");
            unwrapped.as_str()
        } else {
            html_text
        };
        let doc = html::parse_with(html_text, scripting);
        let mut p = Page {
            url,
            doc,
            resources: Vec::new(),
            sources: Vec::new(),
            picture_of: Vec::new(),
            densities: Vec::new(),
            dropped: Vec::new(),
            view,
            laid: None,
            laid_for: (0.0, 0.0, 0.0),
            styles: None,
            changed: true,
            ua: Arc::new(Sheet::default()),
            author: Arc::new(Vec::new()),
            web_fonts: Arc::new(Vec::new()),
            font_loads: Vec::new(),
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
            canvases: Vec::new(),
            data_pictures: Vec::new(),
            img_fired: alloc::collections::BTreeSet::new(),
            repaint: false,
            style_pictures: BTreeMap::new(),
            view_scroll: 0.0,
            clock_ms: 0.0,
            animate: true,
            relayout: true,
            frames: Vec::new(),
            frames_changed: false,
            own: None,
            frame_resources: BTreeMap::new(),
            frame_numbers: BTreeMap::new(),
            frame_requests: BTreeMap::new(),
            frame_sockets: BTreeMap::new(),
            next_frame_id: FRAME_IDS,
            depth,
            ancestors: Vec::new(),
            epoch,
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
    /// the page has, in order (again after a script changes the page: what's been fetched kept). A picture chosen
    /// before and not now (another of a srcset's, for another window), not yet asked for: not fetched.
    fn scan(&mut self) {
        let before: Vec<usize> = self.picture_of.iter().map(|&(_, r)| r).collect();
        self.scan_nodes();
        for r in before {
            if self.picture_of.iter().any(|&(_, k)| k == r) || self.style_pictures.values().any(|&k| k == r) {
                continue;
            }
            let res = &mut self.resources[r];
            if res.kind == Kind::Picture && res.url.is_some() && !res.asked {
                res.asked = true;
                res.state = State::Failed;
                self.dropped.push(r);
            }
        }
    }

    fn scan_nodes(&mut self) {
        // (The page's elements by id, made when an <svg> first needs them.)
        let mut ids: Option<BTreeMap<String, NodeId>> = None;
        let mut iframes: Vec<NodeId> = Vec::new();
        self.sources.clear();
        self.picture_of.clear();
        self.densities.clear();
        self.sheets_stale = true;
        self.inputs_stale = true;
        // The page's nodes in order, and its shadow trees' (each after its host), with the tree each is in.
        let mut nodes: Vec<(NodeId, Option<NodeId>)> = Vec::new();
        let mut stack: Vec<(NodeId, Option<NodeId>)> = vec![(0, None)];
        while let Some((n, tree)) = stack.pop() {
            nodes.push((n, tree));
            if let Some(root) = self.doc.shadow_of(n) {
                for &c in self.doc.children(root).iter().rev() {
                    stack.push((c, Some(root)));
                }
            }
            for &c in self.doc.children(n).iter().rev() {
                stack.push((c, tree));
            }
        }
        for (n, tree) in nodes {
            let Some(e) = self.doc.element(n) else { continue };
            match e.name.as_str() {
                "style" => {
                    let media = e.attr("media").map(String::from);
                    let mut css = self.doc.text_of(n);
                    // (Its rules as its scripts changed them, if they did, since its text was last set.)
                    if let Some(s) = self.script.as_mut()
                        && let Some((src, rules)) = s.host().cssom.get(&n)
                        && *src == css
                    {
                        css = rules.join("\n");
                    }
                    let css = match media {
                        Some(m) if !m.trim().is_empty() && !m.trim().eq_ignore_ascii_case("all") => format!("@media {m} {{\n{css}\n}}"),
                        _ => css,
                    };
                    match tree {
                        Some(root) => self.sources.push(Source::Scoped(css, root)),
                        None => self.sources.push(Source::Inline(css)),
                    }
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
                    // (Its fills and strokes as the page's style sheets give them, where they say other than its text.)
                    let (color, paints) = self.svg_look(n);
                    let mut text = self.doc.outer_html_setting(n, &|m| paints.iter().filter(|p| p.0 == m).map(|p| (p.1, p.2.clone())).collect());
                    // (What it refers to that's elsewhere in the page, as an icon's <use> does a sprite's <symbol>:
                    // put in it, in a <defs>, as it's drawn on its own.)
                    let outside = self.svg_outside_refs(n, &mut ids);
                    if !outside.is_empty()
                        && let Some(end) = tag_end(&text)
                    {
                        let mut defs = String::from("<defs>");
                        for t in outside {
                            defs.push_str(&self.doc.outer_html(t));
                        }
                        defs.push_str("</defs>");
                        text.insert_str(end + 1, &defs);
                    }
                    // (currentColor: the colour the page gives the element, unless it says its own.)
                    if !e.has_attr("color")
                        && let Some(c) = color
                    {
                        text = format!("<svg color=\"#{c:06x}\"{}", &text[4..]);
                    }
                    let look = look_hash(color, &paints);
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
                            self.inline_svgs.push((n, hash, r, look));
                            r
                        }
                    };
                    self.picture_of.push((n, r));
                }
                "canvas" => {
                    // A picture its scripts draw (its size from its width and height).
                    if self.picture_of.len() >= MOST_PICTURES {
                        continue;
                    }
                    let size = |name: &str, d: u32| e.attr(name).and_then(non_negative).unwrap_or(d);
                    let (w, h) = (size("width", 300), size("height", 150));
                    let r = match self.canvases.iter().find(|(k, _)| *k == n) {
                        Some((_, r)) => *r,
                        None => {
                            let r = self.add(Kind::Picture, None, None);
                            self.resources[r].asked = true;
                            self.canvases.push((n, r));
                            r
                        }
                    };
                    self.resources[r].state = State::Picture(w, h);
                    self.picture_of.push((n, r));
                }
                "img" | "input" | "video" => {
                    if e.name == "input" && !e.attr("type").is_some_and(|t| t.eq_ignore_ascii_case("image")) {
                        continue;
                    }
                    self.picture_element(n);
                }
                "iframe" => iframes.push(n),
                "object" | "embed" => match embedded(e) {
                    Embedded::Document => iframes.push(n),
                    Embedded::Picture => self.picture_element(n),
                    Embedded::Neither => {}
                },
                _ => {}
            }
        }
        self.frames_found(&iframes);
        // Pictures its scripts made that aren't in it (new Image(), its src set): fetched too.
        let made: Vec<NodeId> = self.script.as_mut().map(|s| s.host().images.clone()).unwrap_or_default();
        for n in made {
            if !self.doc.connected(n) {
                self.picture_element(n);
            }
        }
    }

    /// The page's <iframe>s, in its order: each one's frame (a new one's made; one whose src or srcdoc changed shows
    /// that now), and those no longer in the page let go. (None in a page that's in frames already as deep as they
    /// go, nor in a <template>'s contents.)
    fn frames_found(&mut self, iframes: &[NodeId]) {
        let mut seen: Vec<usize> = Vec::new();
        for &n in iframes {
            if self.depth >= MOST_FRAME_DEPTH || self.in_template(n) {
                continue;
            }
            let Some(e) = self.doc.element(n) else { continue };
            // (An <object>'s document is its data's, an <embed>'s its src's.)
            let (doc, src) = match e.name.as_str() {
                "object" => (None, e.attr("data")),
                "embed" => (None, e.attr("src")),
                _ => (e.attr("srcdoc"), e.attr("src")),
            };
            let source = match (doc, src.map(str::trim).filter(|s| !s.is_empty())) {
                (Some(doc), _) => FrameSource::Written(String::from(doc)),
                (None, None) => FrameSource::Blank,
                (None, Some(src)) if src.eq_ignore_ascii_case("about:blank") || src.get(..12).is_some_and(|a| a.eq_ignore_ascii_case("about:blank#")) => FrameSource::Blank,
                (None, Some(src)) if src.get(..5).is_some_and(|d| d.eq_ignore_ascii_case("data:")) => match data_document(src) {
                    Some(html) => FrameSource::Written(html),
                    None => FrameSource::Blank,
                },
                (None, Some(src)) => match self.url.join(src) {
                    Ok(u) => FrameSource::Address(u),
                    Err(_) => FrameSource::Blank,
                },
            };
            match self.frames.iter().position(|f| f.node == n && !f.gone) {
                Some(k) => {
                    seen.push(k);
                    if self.frames[k].source != source {
                        self.frames[k].source = source;
                        self.reset_frame(k);
                    }
                }
                None if self.frames.len() < MOST_FRAMES_EVER && self.frames.iter().filter(|f| !f.gone).count() < MOST_FRAMES => {
                    self.frames.push(Frame { node: n, source, resource: None, page: None, rect: Rect::default(), shown: false, started: 0.0, loaded: false, gone: false });
                    seen.push(self.frames.len() - 1);
                }
                None => {}
            }
        }
        for k in 0..self.frames.len() {
            if !self.frames[k].gone && !seen.contains(&k) {
                self.frames[k].gone = true;
                self.reset_frame(k);
            }
        }
    }

    fn in_template(&self, n: NodeId) -> bool {
        let mut at = self.doc.parent(n);
        while let Some(p) = at {
            if self.doc.name(p) == "template" {
                return true;
            }
            at = self.doc.parent(p);
        }
        false
    }

    /// A frame's document let go (it shows another now, or none): what was asked for it, and of it, forgotten.
    fn reset_frame(&mut self, k: usize) {
        let f = &mut self.frames[k];
        f.page = None;
        f.resource = None;
        f.loaded = false;
        self.frame_resources.retain(|_, (g, _)| *g != k);
        self.frame_numbers.retain(|(g, _), _| *g != k);
        self.frame_requests.retain(|_, (g, _)| *g != k);
        self.frame_sockets.retain(|_, (g, _)| *g != k);
        self.frames_changed = true;
        self.changed = true;
        self.want_frame(k);
    }

    /// A frame that's shown, its document asked for: fetched (an address's), or made at once (one written into it).
    fn want_frame(&mut self, k: usize) {
        let f = &self.frames[k];
        if f.gone || !f.shown || f.page.is_some() || f.resource.is_some() {
            return;
        }
        match f.source.clone() {
            FrameSource::Address(u) => {
                let r = self.add(Kind::Frame, Some(u), None);
                self.frames[k].resource = Some(r);
            }
            FrameSource::Written(html) => {
                let base = self.url.clone();
                self.open_frame(k, base, &html);
            }
            FrameSource::Blank => {}
        }
    }

    /// A frame's document, made: a page of its own, as wide and high as the frame's box, its scripts run if the
    /// page's are (and the frame's sandbox, if it has one, allows them).
    fn open_frame(&mut self, k: usize, url: Url, html: &str) {
        let dpr = if self.dpr > 0.0 { self.dpr } else { 1.0 };
        let rect = self.frames[k].rect;
        let view = ((rect.w / dpr).max(1.0), (rect.h / dpr).max(1.0), dpr);
        let sandboxed = self.doc.element(self.frames[k].node).and_then(|e| e.attr("sandbox")).is_some_and(|s| !s.split_ascii_whitespace().any(|t| t.eq_ignore_ascii_case("allow-scripts")));
        let scripting = self.script.is_some() && !sandboxed;
        let mut p = Page::make_in_frame(url, html, scripting, self.epoch + self.clock_ms, view, self.depth + 1);
        // (The frame's marginwidth and marginheight: its document's body's margins, unless the body says its own.)
        let margins = self.doc.element(self.frames[k].node).map(|e| (e.attr("marginwidth").map(String::from), e.attr("marginheight").map(String::from)));
        if let (Some((mw, mh)), Some(body)) = (margins, p.doc.find("body")) {
            for (name, own, value) in [("marginwidth", "leftmargin", mw), ("marginheight", "topmargin", mh)] {
                if let Some(v) = value
                    && let Some(e) = p.doc.element_mut(body)
                    && !e.has_attr(name)
                    && !e.has_attr(own)
                {
                    e.set_attr(name, &v);
                }
            }
        }
        p.animate = self.animate;
        p.ancestors = self.ancestors.clone();
        p.ancestors.push(self.url.clone());
        let f = &mut self.frames[k];
        f.page = Some(Box::new(p));
        f.started = self.clock_ms;
        f.loaded = false;
        self.frames_changed = true;
        self.changed = true;
    }

    /// The frame an <iframe> element's document is shown in, if it has one.
    fn frame_of(&self, node: NodeId) -> Option<usize> {
        self.frames.iter().position(|f| f.node == node && !f.gone)
    }

    /// Something done to a frame's document: the page laid out again (its frames in their places) if it changed it,
    /// drawn again if it drew on a canvas.
    fn with_frame<T>(&mut self, k: usize, go: impl FnOnce(&mut Page) -> T) -> Option<T> {
        let p = self.frames.get_mut(k)?.page.as_mut()?;
        let r = go(p);
        let (changed, repaint) = (p.changed, core::mem::take(&mut p.repaint));
        if changed {
            self.frames_changed = true;
            self.changed = true;
        }
        if repaint {
            self.repaint = true;
        }
        Some(r)
    }

    /// The frame (and its own number) one of the page's resources was fetched for.
    fn frame_resource(&self, i: usize) -> Option<(usize, usize)> {
        self.frame_resources.get(&i).copied()
    }

    /// The page's number for one of a frame's resources (a picture it shows that the page hasn't fetched for it, a
    /// canvas's: numbered now).
    fn frame_number(&mut self, k: usize, n: usize) -> usize {
        if let Some(&i) = self.frame_numbers.get(&(k, n)) {
            return i;
        }
        let (kind, state, own) = self.frames[k].page.as_ref().and_then(|p| p.resources.get(n)).map_or((Kind::Picture, State::Failed, None), |r| {
            (r.kind, if matches!(r.state, State::Picture(..)) { r.state.clone() } else { State::Failed }, r.own)
        });
        let i = self.add(kind, None, None);
        self.resources[i].asked = true;
        self.resources[i].state = state;
        self.resources[i].own = own;
        self.frame_resources.insert(i, (k, n));
        self.frame_numbers.insert((k, n), i);
        i
    }

    /// A frame's document came (its address, after any redirects; its bytes and their type; and the response's
    /// headers): shown in its frame, unless it says it isn't to be shown in this page (X-Frame-Options, CSP's
    /// frame-ancestors), when what Chrome says in its place is.
    pub fn frame_came(&mut self, i: usize, url: Url, bytes: &[u8], kind: &str, headers: &[(String, String)]) {
        if let Some((k, n)) = self.frame_resource(i) {
            self.resources[i].state = State::Document;
            self.with_frame(k, |p| p.frame_came(n, url, bytes, kind, headers));
            return;
        }
        let Some(r) = self.resources.get_mut(i) else { return };
        r.state = State::Document;
        let Some(k) = self.frames.iter().position(|f| f.resource == Some(i) && !f.gone) else { return };
        let html = if self.may_frame(&url, headers) {
            frame_document(bytes, kind, &url.to_string())
        } else {
            format!(
                "<!DOCTYPE html><style>html {{ height: 100%; }} body {{ margin: 0; height: 100%; display: flex; align-items: center; justify-content: center; font: 15px sans-serif; color: #5f6368; }}</style><p>{} refused to connect.</p>",
                escape_html(&url.host)
            )
        };
        self.open_frame(k, url, &html);
    }

    /// Whether a document from `framed` may be shown in this page (and in those it's in): its CSP's frame-ancestors
    /// say which pages may show it, else its X-Frame-Options (DENY: none; SAMEORIGIN: only its own site's), as
    /// Chrome reads them.
    fn may_frame(&self, framed: &Url, headers: &[(String, String)]) -> bool {
        let ancestors: Vec<&Url> = self.ancestors.iter().chain(core::iter::once(&self.url)).collect();
        let mut said = false;
        for (name, value) in headers {
            if !name.eq_ignore_ascii_case("content-security-policy") {
                continue;
            }
            for policy in value.split(',') {
                let Some(sources) = policy.split(';').find_map(|d| {
                    let mut words = d.split_ascii_whitespace();
                    words.next().filter(|w| w.eq_ignore_ascii_case("frame-ancestors")).map(|_| words.collect::<Vec<&str>>())
                }) else {
                    continue;
                };
                said = true;
                if !ancestors.iter().all(|a| sources.iter().any(|s| frame_source_matches(s, a, framed))) {
                    return false;
                }
            }
        }
        if said {
            return true;
        }
        let mut options: Vec<String> = Vec::new();
        for (_, value) in headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case("x-frame-options")) {
            for v in value.split(',') {
                let v = v.trim().to_ascii_lowercase();
                if !options.contains(&v) {
                    options.push(v);
                }
            }
        }
        if options.len() > 1 && options.iter().any(|o| matches!(o.as_str(), "deny" | "sameorigin" | "allowall")) {
            return false;
        }
        match options.first().map(String::as_str) {
            Some("deny") => false,
            Some("sameorigin") => ancestors.iter().all(|a| a.host == framed.host && a.port == framed.port),
            _ => true,
        }
    }

    /// An <img> (or the like)'s picture: the resource it shows (fetched once for all that show it).
    fn picture_element(&mut self, n: NodeId) {
        let Some(e) = self.doc.element(n) else { return };
        let chosen = match e.name.as_str() {
            "video" => e.attr("poster").map(|p| (String::from(p), 1.0)),
            "object" => e.attr("data").map(|p| (String::from(p), 1.0)),
            "embed" => e.attr("src").map(|p| (String::from(p), 1.0)),
            _ => self.choose_picture(n),
        };
        let Some((address, density)) = chosen else { return };
        if self.picture_of.len() >= MOST_PICTURES {
            return;
        }
        let r = if address.trim_start().get(..5).is_some_and(|d| d.eq_ignore_ascii_case("data:")) {
            // (A picture written into the page: made once, however often the page is read again.)
            let hash = address.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
            match self.data_pictures.iter().find(|(h, _)| *h == hash) {
                Some((_, r)) => Some(*r),
                None => data_url(&address).map(|bytes| {
                    let r = self.add(Kind::Picture, None, Some(bytes));
                    self.data_pictures.push((hash, r));
                    r
                }),
            }
        } else if let Ok(u) = self.url.join(address.trim().split('#').next().unwrap_or("")) {
            // (The same picture twice on a page: fetched once. One chosen before, then not, and now again: fetched.)
            match self.resources.iter().position(|r| r.kind == Kind::Picture && r.url.as_ref() == Some(&u)) {
                Some(i) => {
                    if let Some(k) = self.dropped.iter().position(|&d| d == i) {
                        self.dropped.swap_remove(k);
                        self.resources[i].asked = false;
                        self.resources[i].state = State::Waiting;
                    }
                    Some(i)
                }
                None => Some(self.add(Kind::Picture, Some(u), None)),
            }
        } else {
            None
        };
        if let Some(r) = r {
            self.picture_of.push((n, r));
            if density != 1.0 {
                self.densities.push((n, density));
            }
        }
    }

    /// The picture an <img> shows, and its density: from the first <source> of its <picture> that suits the window
    /// and is of a type that's read, else from its own srcset (its src the 1x candidate, where that has no 1x and no
    /// widths), as the screen's density picks among them (srcset.rs); else its src. A src that's only a placeholder
    /// (pages that load their pictures with a script put the real one in data-srcset or data-src) gives way to those.
    fn choose_picture(&self, n: NodeId) -> Option<(String, f32)> {
        let e = self.doc.element(n)?;
        let view = Viewport { width: self.view.0, height: self.view.1 };
        let dpr = if self.view.2 > 0.0 { self.view.2 } else { 1.0 };
        if e.name == "img"
            && let Some(p) = self.doc.parent(n)
            && self.doc.name(p) == "picture"
        {
            for &c in self.doc.children(p) {
                if c == n {
                    break;
                }
                let Some(s) = self.doc.element(c).filter(|s| s.name == "source") else { continue };
                let Some(set) = s.attr("srcset").filter(|v| !v.trim().is_empty()) else { continue };
                if s.attr("media").is_some_and(|m| !css::media_matches(&css::tokens(m), view)) {
                    continue;
                }
                if s.attr("type").is_some_and(|t| !t.trim().is_empty() && !srcset::type_supported(t)) {
                    continue;
                }
                let candidates = srcset::parse(set);
                if candidates.is_empty() {
                    continue;
                }
                return srcset::pick(candidates, srcset::source_size(s.attr("sizes"), view), dpr);
            }
        }
        let placeholder = |s: &str| s.trim().is_empty() || (s.trim_start().starts_with("data:") && s.len() < 200);
        let src = e.attr("src").filter(|s| !s.trim().is_empty());
        let mut candidates = e.attr("srcset").map(srcset::parse).unwrap_or_default();
        if candidates.is_empty() && src.is_none_or(placeholder) {
            candidates = e.attr("data-srcset").map(srcset::parse).unwrap_or_default();
            if candidates.is_empty()
                && let Some(d) = e.attr("data-src").filter(|d| !d.trim().is_empty())
            {
                return Some((String::from(d.trim()), 1.0));
            }
        }
        if candidates.is_empty() {
            return src.map(|s| (String::from(s.trim()), 1.0));
        }
        if let Some(s) = src.filter(|s| !placeholder(s))
            && !candidates.iter().any(|c| c.width.is_some() || c.density.is_none_or(|d| d == 1.0))
        {
            candidates.push(srcset::Candidate { url: String::from(s.trim()), width: None, density: Some(1.0) });
        }
        srcset::pick(candidates, srcset::source_size(e.attr("sizes"), view), dpr)
    }

    fn add(&mut self, kind: Kind, url: Option<Url>, inline: Option<Vec<u8>>) -> usize {
        self.resources.push(Resource { kind, url, inline, asked: false, state: State::Waiting, own: None });
        self.resources.len() - 1
    }

    /// The next thing to fetch (marked asked): style sheets first, then pictures in the page's order.
    /// Then what its frames' documents ask for, numbered as the page's own.
    pub fn next_wanted(&mut self) -> Option<(usize, Kind, Option<Url>, Option<Vec<u8>>)> {
        let own = self.resources.iter().position(|r| !r.asked && r.kind == Kind::Sheet).or_else(|| self.resources.iter().position(|r| !r.asked));
        if let Some(i) = own {
            let r = &mut self.resources[i];
            r.asked = true;
            return Some((i, r.kind, r.url.clone(), r.inline.take()));
        }
        for k in 0..self.frames.len() {
            let Some((n, kind, url, inline)) = self.frames[k].page.as_mut().and_then(|p| p.next_wanted()) else { continue };
            // (Its address isn't the page's: the page's own pictures and sheets aren't found among its frames'.)
            let i = self.add(kind, None, None);
            self.resources[i].asked = true;
            self.frame_resources.insert(i, (k, n));
            self.frame_numbers.insert((k, n), i);
            return Some((i, kind, url, inline));
        }
        None
    }

    /// Whether the style sheets have all come (or failed): the page can be shown as it's meant to look.
    pub fn sheets_done(&self) -> bool {
        self.resources.iter().enumerate().all(|(i, r)| r.kind != Kind::Sheet || matches!(r.state, State::Css(_) | State::Failed) || self.frame_resources.contains_key(&i))
    }

    /// A style sheet came: its text (its @imports asked for in turn).
    pub fn sheet_came(&mut self, i: usize, css_text: String) {
        if let Some((k, n)) = self.frame_resource(i) {
            self.resources[i].state = State::Css(String::new());
            self.with_frame(k, |p| p.sheet_came(n, css_text));
            return;
        }
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
        self.dirty();
    }

    /// The web fonts the page's styles want that haven't been asked for: asked for, from the first of each one's
    /// addresses in a format that's read (WOFF2, WOFF, TrueType or OpenType; or a data: address's bytes).
    fn fetch_fonts(&mut self) {
        let Some(styles) = self.styles.clone() else { return };
        for &i in &styles.wanted_fonts {
            let Some(font) = self.web_fonts.get(i) else { continue };
            if self.font_loads.iter().any(|(f, _, _)| *f == font.face) || self.font_loads.len() >= MOST_FONTS {
                continue;
            }
            let face = font.face.clone();
            let source = face.sources.iter().find(|(_, format)| matches!(format.as_str(), "" | "woff2" | "woff" | "truetype" | "opentype" | "woff2-variations" | "woff-variations" | "truetype-variations" | "opentype-variations"));
            let r = match source {
                Some((address, _)) if address.trim_start().get(..5).is_some_and(|d| d.eq_ignore_ascii_case("data:")) => {
                    data_url(address).map(|bytes| self.add(Kind::Font, None, Some(bytes)))
                }
                Some((address, _)) => self.url.join(address.trim()).ok().map(|u| self.add(Kind::Font, Some(u), None)),
                None => None,
            };
            // (One that can't be fetched is noted all the same: it isn't asked for again.)
            self.font_loads.push((face, r.unwrap_or(usize::MAX), None));
        }
    }

    /// The pictures the page's styles show (backgrounds' and masks' url()s) that haven't been asked for: asked for
    /// (each address once; a data: address's bytes as they are).
    fn fetch_style_pictures(&mut self) {
        let Some(styles) = self.styles.clone() else { return };
        let mut wanted: Vec<String> = Vec::new();
        let all = styles.of.iter().flatten().chain(styles.before.values()).chain(styles.after.values());
        for st in all {
            let border = st.border_image.as_ref().and_then(|b| b.source.as_ref());
            for img in [&st.bg, &st.mask].into_iter().flatten().flat_map(|l| l.images.iter().flatten()).chain(border) {
                if let crate::effects::Image::Url(u, _) = img
                    && !u.starts_with('#')
                    && !self.style_pictures.contains_key(u)
                    && !wanted.contains(u)
                {
                    wanted.push(u.clone());
                }
            }
        }
        for address in wanted {
            if self.style_pictures.len() >= MOST_PICTURES {
                break;
            }
            let r = if address.trim_start().get(..5).is_some_and(|d| d.eq_ignore_ascii_case("data:")) {
                let hash = address.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
                match self.data_pictures.iter().find(|(h, _)| *h == hash) {
                    Some((_, r)) => Some(*r),
                    None => data_url(&address).map(|bytes| {
                        let r = self.add(Kind::Picture, None, Some(bytes));
                        self.data_pictures.push((hash, r));
                        r
                    }),
                }
            } else if let Ok(u) = self.url.join(address.trim().split('#').next().unwrap_or("")) {
                match self.resources.iter().position(|r| r.kind == Kind::Picture && r.url.as_ref() == Some(&u)) {
                    Some(i) => Some(i),
                    None => Some(self.add(Kind::Picture, Some(u), None)),
                }
            } else {
                None
            };
            // (One that can't be fetched is noted all the same: it isn't asked for again.)
            self.style_pictures.insert(address, r.unwrap_or(usize::MAX));
        }
    }

    /// A web font came: kept (text::loaded), and the page laid out again in it.
    pub fn font_came(&mut self, i: usize, bytes: &[u8]) {
        if let Some((k, n)) = self.frame_resource(i) {
            self.resources[i].state = State::Font(0);
            self.with_frame(k, |p| p.font_came(n, bytes));
            return;
        }
        let id = text::loaded::load(bytes);
        if let Some(r) = self.resources.get_mut(i) {
            r.state = id.map_or(State::Failed, State::Font);
        }
        let Some(k) = self.font_loads.iter().position(|(_, r, _)| *r == i) else { return };
        self.font_loads[k].2 = id;
        if id.is_some() {
            let face = self.font_loads[k].0.clone();
            let mut fonts = (*self.web_fonts).clone();
            for f in fonts.iter_mut().filter(|f| f.face == face) {
                f.loaded = id;
            }
            self.web_fonts = Arc::new(fonts);
            self.styles = None;
            self.dirty();
            self.inputs_stale = true;
        }
    }

    /// A picture came (decoded by the kernel): its size in pixels.
    pub fn picture_came(&mut self, i: usize, w: u32, h: u32) {
        if let Some((k, n)) = self.frame_resource(i) {
            self.resources[i].state = State::Picture(w, h);
            self.with_frame(k, |p| p.picture_came(n, w, h));
            return;
        }
        if let Some(r) = self.resources.get_mut(i) {
            r.state = State::Picture(w, h);
            self.dirty();
            self.inputs_stale = true;
        }
    }

    /// A picture's own size as CSS sizes it (an SVG's: its width and height only where they're lengths, its ratio
    /// perhaps from its viewBox), after picture_came.
    pub fn picture_own(&mut self, i: usize, own: picture::Intrinsic) {
        if let Some((k, n)) = self.frame_resource(i) {
            self.resources[i].own = Some(own);
            self.with_frame(k, |p| p.picture_own(n, own));
            return;
        }
        if let Some(r) = self.resources.get_mut(i)
            && r.own != Some(own)
        {
            r.own = Some(own);
            self.dirty();
            self.inputs_stale = true;
        }
    }

    pub fn failed(&mut self, i: usize) {
        if let Some((k, n)) = self.frame_resource(i) {
            self.resources[i].state = State::Failed;
            self.with_frame(k, |p| p.failed(n));
            return;
        }
        if let Some(r) = self.resources.get_mut(i) {
            if r.kind == Kind::Frame {
                // (A frame whose document couldn't be had shows nothing; its <iframe>'s load event is fired all the
                // same, as Chrome fires it for the error it shows.)
                r.state = State::Failed;
                return;
            }
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
            self.dirty();
        }
    }

    /// Lays the page out for a panel `w` by `h` device pixels (`dpr` device pixels a CSS pixel), if it isn't laid out
    /// for that already or something changed.
    pub fn lay_out(&mut self, ty: &Type, w: f32, h: f32, dpr: f32) -> &Laid {
        // (Pictures chosen again for another window: their srcsets' sizes and their sources' media may say others.)
        if dpr > 0.0 && (w / dpr, h / dpr, dpr) != self.view {
            self.view = (w / dpr, h / dpr, dpr);
            self.scan();
        }
        let size_changed = self.laid_for != (w, h, dpr);
        if size_changed {
            self.styles = None;
        }
        let fresh = self.laid.is_none() || self.relayout || size_changed;
        if fresh {
            let view = Viewport { width: w / dpr, height: h / dpr };
            if self.styles.is_none() {
                self.parse_sheets(view);
                let scripted = self.scripted_animations();
                self.styles = Some(Arc::new(style::compute(&self.doc, &Sheets { ua: &self.ua, author: &self.author, fonts: &self.web_fonts, now: self.animate.then_some((self.clock_ms / 1000.0) as f32), scripted: &scripted }, view, dpr)));
                self.fetch_fonts();
                self.fetch_style_pictures();
                // (An <svg> made before the colours it's drawn in were known: made again.)
                if self.inline_svgs.iter().any(|(n, _, _, look)| {
                    let (color, paints) = self.svg_look(*n);
                    look_hash(color, &paints) != *look
                }) {
                    self.scan();
                    self.sheets_stale = false;
                }
            }
            let styles = self.styles.as_ref().expect("worked out above");
            let mut tree = Tree::build(&self.doc, styles, &mut Picker { of: &self.picture_of, densities: &self.densities });
            let laid = Layout::new(&mut tree, ty, &Known { resources: &self.resources, images: &self.style_pictures }, w, h, dpr).run();
            // (With frames, kept as it is: their documents are put in their places in a copy.)
            if laid.items.iter().any(|i| matches!(i.paint, Paint::Frame { .. })) {
                self.own = Some(laid);
            } else {
                self.own = None;
                self.laid = Some(laid);
                for f in self.frames.iter_mut() {
                    f.shown = false;
                }
            }
            self.laid_for = (w, h, dpr);
            self.relayout = false;
            self.dpr = dpr;
            self.share_layout(w, h, dpr);
        }
        if self.own.is_some() && (fresh || self.frames_changed) {
            self.compose(ty, dpr);
        }
        self.frames_changed = false;
        self.changed = false;
        self.laid.as_ref().expect("laid out above")
    }

    /// What's laid out, with its frames' documents in their places: each laid out as wide and high as its frame's
    /// box (at the page's scale), what it draws moved there and cut to the box (its corners rounded, if the frame's
    /// are), over its background where it gives one (else what's under the frame shows through, as in Chrome).
    fn compose(&mut self, ty: &Type, dpr: f32) {
        let Some(own) = self.own.take() else { return };
        for f in self.frames.iter_mut() {
            f.shown = false;
        }
        let mut items: Vec<Item> = Vec::with_capacity(own.items.len() + 64);
        for it in &own.items {
            let Paint::Frame { x, y, w, h, radius } = it.paint else {
                items.push(it.clone());
                continue;
            };
            // (Its place, for clicks: what's drawn in it is its document's.)
            items.push(Item { paint: Paint::Rect { x, y, w, h, color: 0, radius }, clip: it.clip, node: it.node });
            let Some(k) = it.node.and_then(|n| self.frame_of(n)) else { continue };
            let rect = Rect::new(x, y, w, h);
            self.frames[k].rect = rect;
            self.frames[k].shown = w >= 1.0 && h >= 1.0;
            if !self.frames[k].shown {
                continue;
            }
            self.want_frame(k);
            let animate = self.animate;
            let Some(child) = self.frames[k].page.as_mut() else { continue };
            if child.animate != animate {
                child.animate = animate;
                child.restyle();
            }
            let laid = child.lay_out(ty, w, h, dpr);
            let background = laid.given.then_some(laid.background);
            let inner = laid.items.clone();
            items.push(Item { paint: Paint::Begin(Box::new(crate::layout::Group { clip: Some((rect, radius)), ..crate::layout::Group::none() })), clip: it.clip, node: None });
            if let Some(color) = background {
                items.push(Item { paint: Paint::Rect { x, y, w, h, color, radius: [0.0; 4] }, clip: it.clip, node: None });
            }
            let mut depth = 0usize;
            for mut c in inner {
                let top = depth == 0;
                match c.paint {
                    Paint::Begin(_) => depth += 1,
                    Paint::End => depth = depth.saturating_sub(1),
                    _ => {}
                }
                moved(&mut c.paint, x, y, &mut |n| self.frame_number(k, n));
                if c.clip.w < 1.0e6 {
                    c.clip.x += x;
                    c.clip.y += y;
                }
                // (Cut to the frame: those not in a group of the document's own are in the page's coordinates.)
                if top {
                    c.clip = if c.clip.w < 1.0e6 { c.clip.intersect(&rect) } else { rect };
                }
                c.node = None;
                items.push(c);
            }
            for _ in 0..depth {
                items.push(Item::end_of_group());
            }
            items.push(Item::end_of_group());
        }
        self.laid = Some(Laid { items, width: own.width, height: own.height, background: own.background, given: own.given, boxes: own.boxes.clone() });
        self.own = Some(own);
    }

    /// The page's style sheets parsed for a view (again only if they or the view changed).
    fn parse_sheets(&mut self, view: Viewport) {
        if !self.sheets_stale && self.sheets_for == (view.width, view.height) {
            return;
        }
        self.ua = Arc::new(if self.doc.quirks { css::sheet(&format!("{UA}\n{QUIRKS}"), view) } else { css::sheet(UA, view) });
        let mut text = String::new();
        let mut author: Vec<Sheet> = Vec::new();
        let mut scoped: Vec<Sheet> = Vec::new();
        // (Sheets one after another, in one piece where a link's media wrapped it.)
        for s in &self.sources {
            match s {
                Source::Inline(css) => text.push_str(css),
                Source::Linked(r) => {
                    if let Some(State::Css(css)) = self.resources.get(*r).map(|r| &r.state) {
                        text.push_str(css);
                    }
                }
                Source::Scoped(css, root) => {
                    let mut sheet = css::sheet(css, view);
                    sheet.scope = Some(*root);
                    scoped.push(sheet);
                    continue;
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
        author.extend(scoped);
        // Its web fonts: each @font-face once, with its number if it has come.
        let mut fonts: Vec<style::WebFont> = Vec::new();
        for face in author.iter().flat_map(|s| s.fonts.iter()) {
            if fonts.iter().any(|f| f.face == *face) {
                continue;
            }
            let loaded = self.font_loads.iter().find(|(f, _, _)| f == face).and_then(|(_, _, id)| *id);
            fonts.push(style::WebFont { face: face.clone(), loaded });
        }
        self.web_fonts = Arc::new(fonts);
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
        let laid = if self.own.is_some() { &self.own } else { &self.laid };
        let (Some(laid), Some(s)) = (laid, self.script.as_mut()) else { return };
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
        let own = self.resources.iter().map(|r| r.own).collect();
        let scripted = self.scripted_animations();
        let inputs = crate::script::LayoutInputs { ua: self.ua.clone(), author: self.author.clone(), fonts: self.web_fonts.clone(), picture_of: self.picture_of.clone(), densities: self.densities.clone(), sizes, own, now: self.animate.then_some((self.clock_ms / 1000.0) as f32), scripted };
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
        if let Some((k, n)) = self.frame_resource(i) {
            self.resources[i].state = State::Script(String::new());
            self.with_frame(k, |p| p.script_came(n, text));
            return;
        }
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

    /// What the page's style sheets say an <svg> is drawn with, beyond its own text: currentColor's colour, and the
    /// fills and strokes they give it (or it inherits from the page round it, where it says none of its own) and the
    /// elements inside it (each element's, as an attribute to set on it).
    /// The elements an <svg> refers to (by href="#id", and url(#id) in its attributes) that are elsewhere in the page,
    /// and those they refer to in turn (a sprite's <symbol>s, gradients in a hidden <svg>); `ids`: the page's elements
    /// by id, made the first time they're needed.
    fn svg_outside_refs(&self, n: NodeId, ids: &mut Option<BTreeMap<String, NodeId>>) -> Vec<NodeId> {
        let refs_in = |root: NodeId, out: &mut Vec<String>| {
            for m in self.doc.descendants(root) {
                let Some(e) = self.doc.element(m) else { continue };
                for (name, value) in &e.attrs {
                    let v = value.trim();
                    if (name == "href" || name == "xlink:href") && v.len() > 1 && v.starts_with('#') {
                        out.push(String::from(&v[1..]));
                    }
                    let mut rest = v;
                    while let Some(at) = rest.find("url(") {
                        let inner = &rest[at + 4..];
                        let close = inner.find(')').unwrap_or(inner.len());
                        let id = inner[..close].trim().trim_matches(['\'', '"']);
                        if let Some(id) = id.strip_prefix('#').filter(|i| !i.is_empty()) {
                            out.push(String::from(id));
                        }
                        rest = &inner[close..];
                    }
                }
            }
        };
        let mut wanted = Vec::new();
        refs_in(n, &mut wanted);
        if wanted.is_empty() {
            return Vec::new();
        }
        let inside = |id: &str, root: NodeId| self.doc.descendants(root).any(|m| self.doc.element(m).and_then(|e| e.id()) == Some(id));
        let ids = ids.get_or_insert_with(|| {
            let mut map = BTreeMap::new();
            for m in self.doc.descendants(0) {
                if let Some(id) = self.doc.element(m).and_then(|e| e.id()) {
                    map.entry(String::from(id)).or_insert(m);
                }
            }
            map
        });
        let mut found: Vec<NodeId> = Vec::new();
        let mut k = 0;
        while k < wanted.len() && found.len() < 64 {
            let id = wanted[k].clone();
            k += 1;
            if inside(&id, n) || found.iter().any(|&f| inside(&id, f)) {
                continue;
            }
            let Some(&t) = ids.get(&id) else { continue };
            // (Not something it's inside, nor what holds it.)
            if self.doc.descendants(t).any(|m| m == n) {
                continue;
            }
            found.push(t);
            refs_in(t, &mut wanted);
        }
        found
    }

    fn svg_look(&self, n: NodeId) -> (Option<u32>, Vec<(NodeId, &'static str, String)>) {
        let color = self.color_of(n);
        let mut paints = Vec::new();
        let Some(styles) = self.styles.as_ref() else { return (color, paints) };
        let style = |m: NodeId| styles.of.get(m).cloned().flatten();
        for m in self.doc.descendants(n) {
            let (Some(e), Some(s)) = (self.doc.element(m), style(m)) else { continue };
            let parent = self.doc.parent(m).and_then(style);
            for (name, mine, theirs) in [("fill", s.fill, parent.as_ref().and_then(|p| p.fill)), ("stroke", s.stroke, parent.as_ref().and_then(|p| p.stroke))] {
                let Some(mine) = mine else { continue };
                // (Said for this element itself, or, on the <svg>, inherited from round it with nothing of its own.)
                if Some(mine) != theirs || (m == n && !e.has_attr(name)) {
                    paints.push((m, name, mine.svg()));
                }
            }
        }
        (color, paints)
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
        if core::mem::take(&mut h.canvas_drawn) {
            self.repaint = true;
        }
        let scroll = h.scroll.1;
        let url = h.url.href();
        if mutated || title {
            self.styles = None;
            self.dirty();
        }
        if rescan {
            self.scan();
            self.sheets_changed();
            self.dirty();
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
        self.view_scroll = y;
        self.scrolled = Some(y);
    }

    /// Moves the page's scripts on (from the system's loop, `now` ms since the page began): the scripts that have
    /// come run in their turn, DOMContentLoaded and load fired, timers and tasks due run. True if anything ran.
    /// Runs `source` as a classic script in the page, what it logs going to the console (for tools and tests).
    pub fn run_text(&mut self, source: &str) {
        self.with_script(|s| s.run(source, "eval", None));
    }

    pub fn run(&mut self, now: f64) -> bool {
        self.tick(now);
        let mut ran = self.run_frames(now);
        if self.script.is_none() {
            return ran;
        }
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
        // Pictures that have come (or couldn't be had): their <img>s' load (or error) events.
        let mut due = Vec::new();
        for &(n, r) in &self.picture_of {
            if self.img_fired.contains(&(n, r)) || !matches!(self.doc.name(n), "img" | "input") {
                continue;
            }
            match self.resources[r].state {
                State::Picture(w, h) => due.push((n, r, Some((w, h)))),
                State::Failed => due.push((n, r, None)),
                _ => {}
            }
        }
        for (n, r, size) in due {
            self.img_fired.insert((n, r));
            self.with_script(|s| {
                s.set_now(now);
                s.host().settled.insert(n, size);
                s.fire(Some(n), &EventInit::new(if size.is_some() { "load" } else { "error" }, "Event", false, false))
            });
            ran = true;
        }
        if self.dom_loaded && !self.load_fired && self.resources.iter().all(|r| r.asked && r.state != State::Waiting) && self.scripts.iter().all(|s| s.done) && self.frames_finished() {
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

    /// Its frames' documents moved on (each at its own time): their scripts run, their timers; where their scripts
    /// send them, they go (in their frames); each that has come, its <iframe>'s load event. True if anything ran.
    fn run_frames(&mut self, now: f64) -> bool {
        let mut ran = false;
        for k in 0..self.frames.len() {
            if self.frames[k].gone {
                continue;
            }
            let started = self.frames[k].started;
            if self.with_frame(k, |p| p.run(now - started)) == Some(true) {
                ran = true;
            }
            let went = self.frames[k].page.as_mut().and_then(|p| {
                p.go_back = false;
                p.scrolled = None;
                p.navigation.take()
            });
            if let Some(r) = went {
                self.frames[k].source = FrameSource::Address(r.url);
                self.reset_frame(k);
                ran = true;
            }
            // (Its load event: once its document has, or once it couldn't be had; at once for one that shows none.)
            let f = &self.frames[k];
            let failed = f.resource.is_some_and(|r| self.resources.get(r).is_some_and(|r| r.state == State::Failed));
            let done = match &f.source {
                FrameSource::Blank => true,
                _ => failed || f.page.as_ref().is_some_and(|p| p.finished()),
            };
            if done && !f.loaded {
                let node = f.node;
                self.frames[k].loaded = true;
                self.with_script(|s| {
                    s.set_now(now);
                    s.fire(Some(node), &EventInit::new("load", "Event", false, false))
                });
                ran = true;
            }
        }
        ran
    }

    /// Whether the page has come, all it asked for with it: its load event fired (its resources and its frames' all
    /// in, for a page whose scripts don't run).
    fn finished(&self) -> bool {
        if self.script.is_some() {
            return self.load_fired;
        }
        self.resources.iter().all(|r| r.asked && r.state != State::Waiting) && self.frames_finished()
    }

    /// Whether its frames that are shown have all come (or couldn't be had): the page's load event waits for them,
    /// as for its pictures. (Before it's first laid out, it isn't known which are shown.)
    fn frames_finished(&self) -> bool {
        if self.frames.iter().all(|f| f.gone) {
            return true;
        }
        if self.laid.is_none() {
            return false;
        }
        self.frames.iter().all(|f| f.gone || !f.shown || f.loaded)
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

    /// The cookies its scripts see (document.cookie): the system's for the page's address, the HttpOnly ones left
    /// out ("a=1; b=2").
    pub fn set_cookies(&mut self, cookies: &str) {
        if let Some(s) = self.script.as_mut() {
            s.host().cookies = cookies
                .split(';')
                .filter(|c| !c.trim().is_empty())
                .map(|c| match c.split_once('=') {
                    Some((k, v)) => (String::from(k.trim()), String::from(v.trim())),
                    None => (String::new(), String::from(c.trim())),
                })
                .collect();
        }
    }

    /// The cookies its scripts set since last asked (as they wrote them), for the system's cookies.
    pub fn take_cookies_set(&mut self) -> Vec<String> {
        self.script.as_mut().map(|s| core::mem::take(&mut s.host().cookies_set)).unwrap_or_default()
    }

    /// Its scripts' storage (localStorage, sessionStorage), as kept for its origin.
    pub fn set_storage(&mut self, local: BTreeMap<String, String>, session: BTreeMap<String, String>) {
        if let Some(s) = self.script.as_mut() {
            let h = s.host();
            h.local = local;
            h.session = session;
        }
    }

    /// Its scripts' storage, if they changed it since last asked (for the system to keep for the origin).
    pub fn take_storage(&mut self) -> Option<(BTreeMap<String, String>, BTreeMap<String, String>)> {
        let h = self.script.as_mut()?.host();
        if !core::mem::take(&mut h.storage_changed) {
            return None;
        }
        Some((h.local.clone(), h.session.clone()))
    }

    /// The page's origin, as its scripts have it (where its storage is kept).
    pub fn origin(&self) -> String {
        if self.url.port == 443 { format!("https://{}", self.url.host) } else { format!("https://{}:{}", self.url.host, self.url.port) }
    }

    /// A picture's pixels, decoded (for its scripts to draw on canvases).
    pub fn picture_pixels(&mut self, i: usize, px: Arc<crate::script::Pixels>) {
        if let Some((k, n)) = self.frame_resource(i) {
            self.with_frame(k, |p| p.picture_pixels(n, px));
            return;
        }
        if let Some(s) = self.script.as_mut() {
            let h = s.host();
            if h.pixels.len() <= i {
                h.pixels.resize(i + 1, None);
            }
            h.pixels[i] = Some(px);
        }
    }

    /// The page's pictures for the painter: `pictures` (those fetched), with its canvases'.
    pub fn shown<'a>(&'a self, pictures: &'a dyn crate::paint::Shown) -> WithCanvases<'a> {
        WithCanvases { page: self, inner: pictures }
    }

    /// Whether a resource is a canvas's picture (drawn by the page's scripts, not fetched).
    pub fn is_canvas(&self, r: usize) -> bool {
        if let Some((k, n)) = self.frame_resource(r) {
            return self.frames[k].page.as_ref().is_some_and(|p| p.is_canvas(n));
        }
        self.canvases.iter().any(|(_, k)| *k == r)
    }

    /// A canvas's pixels, by its resource (if its scripts have drawn on it).
    fn canvas_of(&self, r: usize) -> Option<&crate::script::Canvas> {
        if let Some((k, n)) = self.frame_resource(r) {
            return self.frames[k].page.as_ref()?.canvas_of(n);
        }
        let (node, _) = self.canvases.iter().find(|(_, k)| *k == r)?;
        let h = self.script.as_ref()?.vm.host.as_ref()?.downcast_ref::<crate::script::Host>()?;
        h.canvases.iter().find(|c| c.node == *node && !c.alpha.is_empty())
    }

    /// Whether the page is to be drawn again soon: its scripts asked for an animation frame (requestAnimationFrame),
    /// or its CSS animations are running.
    pub fn wants_frame(&mut self) -> bool {
        self.script.as_mut().is_some_and(|s| !s.host().frames.is_empty()) || self.animations_running() || self.frames.iter_mut().any(|f| f.page.as_mut().is_some_and(|p| p.wants_frame()))
    }

    /// Whether any of its CSS animations is running at its time (drawn as they run): each frame different.
    pub fn animations_running(&self) -> bool {
        if !self.animate {
            return false;
        }
        let Some(styles) = self.styles.as_ref() else { return false };
        let now = (self.clock_ms / 1000.0) as f32;
        // (Only those whose keyframes the page has: one it doesn't do anything.)
        let named: alloc::collections::BTreeSet<&str> = core::iter::once(&*self.ua).chain(self.author.iter()).flat_map(|sh| sh.keyframes.iter().map(|k| k.name.as_str())).collect();
        let scripts = self.script.as_ref().and_then(|s| s.vm.host.as_ref()?.downcast_ref::<crate::script::Host>()).is_some_and(|h| h.animations.iter().any(|a| a.running(self.clock_ms)));
        let frames = self.frames.iter().any(|f| f.page.as_ref().is_some_and(|p| p.animations_running()));
        scripts
            || frames
            || (!named.is_empty()
                && styles.of.iter().flatten().chain(styles.before.values()).chain(styles.after.values()).any(|s| {
                    s.animations.as_ref().is_some_and(|a| (0..a.names.len()).any(|i| named.contains(a.names[i].as_str()) && a.timing(i).running(now)))
                }))
    }

    /// Its scripts' animations (element.animate) as its styles take them, where each is at its time (or, if its
    /// animations aren't drawn as they run, where each ends).
    fn scripted_animations(&mut self) -> Vec<style::ScriptedAnimation> {
        let now = if self.animate { self.clock_ms } else { f64::INFINITY };
        let Some(s) = self.script.as_mut() else { return Vec::new() };
        s.host()
            .animations
            .iter()
            .filter(|a| a.state != crate::script::animations::State::Idle)
            .map(|a| {
                let progress = if now.is_finite() { a.progress(now) } else { a.progress(a.start + a.end() / a.rate.abs().max(1e-6)) };
                style::ScriptedAnimation { node: a.node, keyframes: a.keyframes(), progress }
            })
            .collect()
    }

    /// Its styles worked out again, and it laid out again (as after `animate` is changed).
    pub fn restyle(&mut self) {
        self.styles = None;
        self.dirty();
        for f in self.frames.iter_mut() {
            if let Some(p) = f.page.as_mut() {
                p.restyle();
            }
        }
    }

    /// Something of its own changed: laid out again.
    fn dirty(&mut self) {
        self.changed = true;
        self.relayout = true;
    }

    /// The page's time moves on (ms since it began): its styles worked out again if its animations are running.
    fn tick(&mut self, now: f64) {
        if now != self.clock_ms {
            let running = self.animations_running();
            self.clock_ms = now;
            if running {
                self.styles = None;
                self.dirty();
            }
        }
    }

    /// Runs requestAnimationFrame's callbacks (before the page is drawn): true if any ran.
    pub fn animation_frame(&mut self, now: f64) -> bool {
        self.tick(now);
        let mut ran = self.with_script(|s| s.animation_frame(now)).unwrap_or(false);
        for k in 0..self.frames.len() {
            let started = self.frames[k].started;
            if self.with_frame(k, |p| p.animation_frame(now - started)) == Some(true) {
                ran = true;
            }
        }
        ran
    }

    /// When the page's scripts next want to run (ms since the page began), if they do (its frames' documents'
    /// too).
    pub fn next_due(&mut self) -> Option<f64> {
        let mut due = self.script.as_mut().and_then(|s| s.next_due());
        for f in self.frames.iter_mut() {
            if let Some(d) = f.page.as_mut().and_then(|p| p.next_due()) {
                let d = d + f.started;
                due = Some(due.map_or(d, |e: f64| e.min(d)));
            }
        }
        due
    }

    /// An element's style, as the page's last layout worked it out.
    pub fn style_of(&self, n: NodeId) -> Option<Arc<crate::style::Style>> {
        self.styles.as_ref()?.of.get(n).cloned().flatten()
    }

    /// How deep its scripts have gone in calls between the engine and its built-in functions (each takes room on the
    /// stack they run on).
    pub fn script_depth(&self) -> usize {
        self.script.as_ref().map_or(0, |s| s.vm.deepest)
    }

    /// What the page's scripts printed (console.log, and the exceptions nothing caught), since last asked.
    pub fn take_console(&mut self) -> Vec<String> {
        let mut out = self.script.as_mut().map(|s| s.take_console()).unwrap_or_default();
        for f in self.frames.iter_mut() {
            if let Some(p) = f.page.as_mut() {
                out.extend(p.take_console().into_iter().map(|l| format!("(in a frame, {}) {l}", p.url.host)));
            }
        }
        out
    }

    /// What the page's scripts asked of the network (fetch, XMLHttpRequest), to send: (its number, address, method,
    /// headers, body). Each is answered with `answer`.
    /// Its frames' documents' scripts' requests among them, numbered as the page's.
    pub fn take_requests(&mut self) -> Vec<(u32, String, String, Vec<(String, String)>, Option<Vec<u8>>)> {
        let mut out: Vec<(u32, String, String, Vec<(String, String)>, Option<Vec<u8>>)> = Vec::new();
        if let Some(s) = self.script.as_mut() {
            let h = s.host();
            let reqs = core::mem::take(&mut h.requests);
            out.extend(reqs.iter().map(|r| (r.id, r.url.clone(), r.method.clone(), r.headers.clone(), r.body.clone())));
            h.waiting.extend(reqs);
        }
        for k in 0..self.frames.len() {
            let Some(p) = self.frames[k].page.as_mut() else { continue };
            for (id, url, method, headers, body) in p.take_requests() {
                let mine = self.next_frame_id;
                self.next_frame_id = self.next_frame_id.wrapping_add(1).max(FRAME_IDS);
                self.frame_requests.insert(mine, (k, id));
                out.push((mine, url, method, headers, body));
            }
        }
        out
    }

    /// The network's answer to a script's request.
    pub fn answer(&mut self, id: u32, response: Result<crate::script::Response, String>) {
        if id >= FRAME_IDS {
            if let Some((k, n)) = self.frame_requests.remove(&id) {
                self.with_frame(k, |p| p.answer(n, response));
            }
            return;
        }
        self.with_script(|s| s.answer(id, response));
    }

    /// What the page's scripts asked of their WebSockets (to open one, to send a message, to close it), for the
    /// system to do. What comes of each is told with `socket_event`. (Its frames' documents' among them, numbered as
    /// the page's.)
    pub fn take_socket_ops(&mut self) -> Vec<crate::script::SocketOp> {
        use crate::script::SocketOp;
        let mut out = self.script.as_mut().map(|s| s.take_socket_ops()).unwrap_or_default();
        for k in 0..self.frames.len() {
            let Some(p) = self.frames[k].page.as_mut() else { continue };
            for op in p.take_socket_ops() {
                let theirs = match &op {
                    SocketOp::Open { id, .. } | SocketOp::Send { id, .. } | SocketOp::Close { id, .. } => *id,
                };
                let mine = match self.frame_sockets.iter().find(|(_, v)| **v == (k, theirs)).map(|(m, _)| *m) {
                    Some(m) => m,
                    None => {
                        let m = self.next_frame_id;
                        self.next_frame_id = self.next_frame_id.wrapping_add(1).max(FRAME_IDS);
                        self.frame_sockets.insert(m, (k, theirs));
                        m
                    }
                };
                out.push(match op {
                    SocketOp::Open { url, protocols, .. } => SocketOp::Open { id: mine, url, protocols },
                    SocketOp::Send { binary, data, .. } => SocketOp::Send { id: mine, binary, data },
                    SocketOp::Close { code, reason, .. } => SocketOp::Close { id: mine, code, reason },
                });
            }
        }
        out
    }

    /// What came of one of the page's WebSockets: it opened, a message came, it closed.
    pub fn socket_event(&mut self, id: u32, ev: crate::script::SocketEvent) {
        if id >= FRAME_IDS {
            if let Some(&(k, n)) = self.frame_sockets.get(&id) {
                if matches!(ev, crate::script::SocketEvent::Closed(..)) {
                    self.frame_sockets.remove(&id);
                }
                self.with_frame(k, |p| p.socket_event(n, ev));
            }
            return;
        }
        self.with_script(|s| s.socket_event(id, ev));
    }

    /// A click on the page (where it landed, CSS pixels in the view): the mouse's events to the element there,
    /// and whether its default action is to happen (no script cancelled it).
    pub fn click(&mut self, node: Option<NodeId>, x: f32, y: f32) -> bool {
        // (One in a frame is its document's: where it lands there, in its own view.)
        if let Some(k) = node.and_then(|n| self.frame_of(n)).filter(|&k| self.frames[k].page.is_some()) {
            let s = self.scale();
            let r = self.frames[k].rect;
            let (fx, fy) = (x - r.x / s, y + self.view_scroll - r.y / s);
            return self.with_frame(k, |p| {
                let target = p.node_at(fx * s, fy * s);
                p.click(target, fx, fy)
            })
            .unwrap_or(true);
        }
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
            // (In a frame: what's there in its document. A link there is followed in the window, as one whose
            // target is _top; its fields aren't typed in yet.)
            Some(node) if self.frame_of(node).is_some_and(|k| self.frames[k].page.is_some()) => {
                let f = &self.frames[self.frame_of(node).expect("found above")];
                match f.page.as_ref().map(|p| p.at(x - f.rect.x, y - f.rect.y)) {
                    Some(Hit::Link(u, fragment)) => Hit::Link(u, fragment),
                    _ => Hit::Nothing,
                }
            }
            Some(node) => self.hit_from(node),
            None => Hit::Nothing,
        }
    }

    /// The node drawn at `x`, `y` (device pixels from the page's top left), on top.
    pub fn node_at(&self, x: f32, y: f32) -> Option<NodeId> {
        let laid = self.laid.as_ref()?;
        // (Each item's point: the one clicked, through the transforms of the groups it's in.)
        let points = crate::paint::points_in_items(&laid.items, x, y, self.view_scroll * self.scale());
        laid.items.iter().zip(points.iter()).rev().find(|(i, p)| i.node.is_some() && p.is_some_and(|(px, py)| i.clip.contains(px, py) && bounds(i).contains(px, py))).and_then(|(i, _)| i.node)
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
            self.dirty();
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
        self.dirty();
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
        self.dirty();
    }

    /// The fields a page asks to be typed in first (autofocus), else none.
    pub fn autofocus(&self) -> Option<NodeId> {
        self.doc.descendants(0).find(|&n| self.doc.element(n).is_some_and(|e| matches!(e.name.as_str(), "input" | "textarea") && e.has_attr("autofocus")))
    }

    /// Whether Tab goes to an element: a link, a button, a field of any kind (but a hidden one), a list to choose
    /// from, a <summary>, or one a script gave a tabindex of 0 or more (none of them disabled; tabindex -1 takes
    /// any out).
    pub fn focusable(&self, n: NodeId) -> bool {
        let Some(e) = self.doc.element(n) else { return false };
        if e.has_attr("disabled") {
            return false;
        }
        if let Some(i) = e.attr("tabindex").and_then(|t| t.trim().parse::<i32>().ok()) {
            return i >= 0;
        }
        match e.name.as_str() {
            "a" | "area" => e.has_attr("href"),
            "button" | "select" | "textarea" | "summary" => true,
            "input" => !e.attr("type").is_some_and(|t| t.eq_ignore_ascii_case("hidden")),
            _ => false,
        }
    }

    /// Whether an element is a field to type in (a text field of any kind, or a textarea).
    pub fn is_text_field(&self, n: NodeId) -> bool {
        self.doc.element(n).is_some_and(|e| {
            e.name == "textarea"
                || (e.name == "input"
                    && !matches!(e.attr("type").map(|t| t.to_ascii_lowercase()).as_deref(), Some("hidden" | "submit" | "button" | "reset" | "checkbox" | "radio" | "image" | "file" | "range" | "color")))
        })
    }

    /// The element Tab goes to next after `from` (with none, the first), or before it (`back`), in the page's order:
    /// those Tab goes to (`focusable`) that are laid out (shown); None past the last (or before the first).
    pub fn next_focusable(&self, from: Option<NodeId>, back: bool) -> Option<NodeId> {
        let laid = self.laid.as_ref()?;
        // What's shown: each laid-out item's node and the elements it's in.
        let mut shown = vec![false; self.doc.nodes.len()];
        for i in &laid.items {
            let mut at = i.node;
            while let Some(n) = at {
                if n >= shown.len() || shown[n] {
                    break;
                }
                shown[n] = true;
                at = self.doc.parent(n);
            }
        }
        let order: Vec<NodeId> = self.doc.descendants(0).filter(|&n| shown.get(n) == Some(&true) && self.focusable(n)).collect();
        match (from.and_then(|f| order.iter().position(|&n| n == f)), back) {
            (None, false) => order.first().copied(),
            (None, true) => order.last().copied(),
            (Some(i), false) => order.get(i + 1).copied(),
            (Some(i), true) => i.checked_sub(1).map(|j| order[j]),
        }
    }

    /// Where an element and all that's in it are laid out (one box round them all), for a ring round it when Tab
    /// chose it, and to scroll it into view.
    pub fn box_of(&self, node: NodeId) -> Option<Rect> {
        let laid = self.laid.as_ref()?;
        let mut inside = vec![false; self.doc.nodes.len()];
        for n in self.doc.descendants(node) {
            if let Some(x) = inside.get_mut(n) {
                *x = true;
            }
        }
        if let Some(x) = inside.get_mut(node) {
            *x = true;
        }
        let mut r: Option<Rect> = None;
        for i in laid.items.iter().filter(|i| i.node.is_some_and(|n| inside.get(n) == Some(&true))) {
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

    /// The element the page's scripts have given the focus (focus()), if any.
    pub fn script_focus(&mut self) -> Option<NodeId> {
        self.script.as_mut().and_then(|s| s.host().focus)
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

    /// The person sends a form (`submitter` clicked, or Enter in one of its fields): on a page with scripts, its
    /// submit event first (a script that cancels it sends what it must itself, and the page stays); then where it
    /// goes, if it goes (as `submit`).
    pub fn person_submits(&mut self, form: NodeId, submitter: Option<NodeId>) -> Option<Request> {
        if self.script.is_none() {
            return self.submit(form, submitter);
        }
        self.with_script(|s| s.submit_form(form, submitter));
        self.navigation.take()
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

/// A frame's document as HTML, from what came (`kind`: its type; `address`: where it came from): HTML as it is; an
/// SVG as it's drawn as a document of its own (filling the frame where it has no size of its own); a picture as
/// Chrome shows one by itself; text as it does text; anything else, nothing.
fn frame_document(bytes: &[u8], kind: &str, address: &str) -> String {
    let kind = kind.to_ascii_lowercase();
    let mime = kind.split(';').next().unwrap_or("").trim();
    if mime.contains("svg") {
        let (w, h, _) = picture::svg::intrinsic(bytes).unwrap_or((None, None, None));
        let text = web::page::decode(bytes, &kind);
        return format!(
            "<!DOCTYPE html><style>html, body {{ margin: 0; overflow: hidden; }} body > svg {{ display: block;{}{} }}</style>{text}",
            if w.is_none() { " width: 100vw;" } else { "" },
            if h.is_none() { " height: 100vh;" } else { "" }
        );
    }
    if mime.is_empty() || mime.contains("html") || mime.contains("xml") {
        return web::page::decode(bytes, &kind);
    }
    if mime.starts_with("image/") {
        return format!("<!DOCTYPE html><body style=\"margin: 0\"><img src=\"{}\"></body>", escape_html(address));
    }
    if mime.starts_with("text/") {
        return format!("<!DOCTYPE html><pre style=\"word-wrap: break-word; white-space: pre-wrap;\">{}</pre>", escape_html(&web::page::decode(bytes, &kind)));
    }
    String::new()
}

/// A data: address's document (an <iframe>'s src), as HTML (its type text/plain if it says none, as data: has it).
fn data_document(src: &str) -> Option<String> {
    let src = src.trim();
    let head = src.get(5..)?.split(',').next()?.to_ascii_lowercase();
    let bytes = data_url(src)?;
    let kind = if head.split(';').next().unwrap_or("").trim().is_empty() { format!("text/plain{head}") } else { head.replace(";base64", "") };
    Some(frame_document(&bytes, &kind, src))
}

/// Text with what HTML would read as markup escaped.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Whether a page at `at` is one a source in a CSP frame-ancestors directive names (`framed`: the address of the
/// document that says it, for 'self'), as CSP matches sources: *, 'self', a scheme, or a host (*.example.com for
/// its subdomains) with a port and a path, if it gives them.
fn frame_source_matches(source: &str, at: &Url, framed: &Url) -> bool {
    let s = source.trim().to_ascii_lowercase();
    match s.as_str() {
        "*" | "https:" | "http:" => return true,
        "'self'" => return at.host == framed.host && at.port == framed.port,
        _ => {}
    }
    if s.starts_with('\'') {
        return false;
    }
    let (scheme, rest) = match s.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, s.as_str()),
    };
    if scheme.is_some_and(|sc| sc != "https" && sc != "http") || (scheme.is_none() && rest.ends_with(':')) {
        return false;
    }
    let (place, path) = match rest.find('/') {
        Some(i) => (&rest[..i], Some(&rest[i..])),
        None => (rest, None),
    };
    let (host, port) = match place.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (place, None),
    };
    let host_ok = match host.strip_prefix("*.") {
        Some(base) => at.host.ends_with(&format!(".{base}")),
        None => host == "*" || at.host == host,
    };
    let port_ok = match port {
        Some("*") => true,
        Some(p) => p.parse::<u16>().is_ok_and(|p| p == at.port || (p == 80 && at.port == 443)),
        None => at.port == 443,
    };
    let here = at.path.split('?').next().unwrap_or("/");
    let path_ok = match path {
        None => true,
        Some(p) if p.ends_with('/') => here.starts_with(p),
        Some(p) => here == p,
    };
    host_ok && port_ok && path_ok
}

/// One of a frame's document's items moved by (`dx`, `dy`) into the page's coordinates (its groups' transforms,
/// clips, masks and backdrops with it), its pictures numbered as the page's (`picture`); none fixed to the window
/// (the frame's document doesn't scroll in it here).
fn moved(paint: &mut Paint, dx: f32, dy: f32, picture: &mut dyn FnMut(usize) -> usize) {
    match paint {
        Paint::Rect { x, y, .. } | Paint::Border { x, y, .. } | Paint::SidedBorder { x, y, .. } | Paint::Text { x, y, .. } | Paint::Shadow { x, y, .. } | Paint::Frame { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
        Paint::Picture { x, y, n, at, .. } => {
            *x += dx;
            *y += dy;
            at.x += dx;
            at.y += dy;
            *n = picture(*n);
        }
        Paint::Tiles { x, y, tile, image, .. } | Paint::Slice { x, y, tile, image, .. } => {
            *x += dx;
            *y += dy;
            tile.x += dx;
            tile.y += dy;
            if let crate::layout::Tile::Picture(n) = image {
                *n = picture(*n);
            }
        }
        Paint::Begin(g) => {
            // (A transform from its coordinates to those round it, both moved: the same transform moved.)
            if let Some(t) = g.transform.as_mut() {
                t[4] += dx - t[0] * dx - t[2] * dy;
                t[5] += dy - t[1] * dx - t[3] * dy;
            }
            if let Some((r, _)) = g.clip.as_mut() {
                r.x += dx;
                r.y += dy;
            }
            if let Some((polygons, _)) = g.clip_path.as_mut() {
                for p in polygons.iter_mut() {
                    for pt in p.iter_mut() {
                        pt.0 += dx;
                        pt.1 += dy;
                    }
                }
            }
            if let Some(layers) = g.mask.as_mut() {
                for l in layers.iter_mut() {
                    for p in l.paints.iter_mut() {
                        moved(p, dx, dy, picture);
                    }
                }
            }
            if let Some((_, r, _)) = g.backdrop.as_mut() {
                r.x += dx;
                r.y += dy;
            }
            g.scroll = None;
        }
        Paint::End => {}
    }
}

/// An item's rectangle.
pub fn bounds(i: &Item) -> Rect {
    match &i.paint {
        Paint::Rect { x, y, w, h, .. } | Paint::Border { x, y, w, h, .. } | Paint::SidedBorder { x, y, w, h, .. } | Paint::Tiles { x, y, w, h, .. } | Paint::Picture { x, y, w, h, .. } | Paint::Frame { x, y, w, h, .. } | Paint::Slice { x, y, w, h, .. } => {
            Rect::new(*x, *y, *w, *h)
        }
        Paint::Text { x, y, w, line, .. } => Rect::new(*x, *y, *w, *line),
        Paint::Shadow { x, y, w, h, .. } => Rect::new(*x, *y, *w, *h),
        Paint::Begin(_) | Paint::End => Rect::default(),
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

/// Where an element's start tag ends in its HTML (its ">"; quotes minded).
fn tag_end(html: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in html.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// A data: URL's bytes (base64 or percent-encoded).
/// HTML's rules for a non-negative integer (a canvas's width and height).
/// What svg_look found, hashed (to tell when it's changed).
fn look_hash(color: Option<u32>, paints: &[(NodeId, &'static str, String)]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |b: u8| h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    for b in color.map_or(u64::MAX, u64::from).to_le_bytes() {
        eat(b);
    }
    for (n, name, value) in paints {
        for b in (*n as u64).to_le_bytes().into_iter().chain(name.bytes()).chain(value.bytes()) {
            eat(b);
        }
        eat(0);
    }
    h
}

fn non_negative(s: &str) -> Option<u32> {
    let s = s.trim_start_matches([' ', '\t', '\n', '\x0c', '\r']);
    let s = s.strip_prefix('+').unwrap_or(s);
    let digits = &s[..s.bytes().take_while(u8::is_ascii_digit).count()];
    digits.parse::<u64>().ok().filter(|v| *v <= i32::MAX as u64).map(|v| v as u32)
}

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
