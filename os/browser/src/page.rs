//! A page as Chat Computer's own system shows it: its address and HTML read, the style sheets and pictures it asks
//! for (the kernel fetches them and hands them in as they come), laid out for the panel's width and ready to draw;
//! where a click lands (a link, a field, a button), what's typed into its fields, and its forms sent (the address,
//! and for a POST its body).

use crate::boxes::{Pictures, Tree};
use crate::css::{self, Sheet, Viewport};
use crate::dom::{Data, Document, NodeId};
use crate::html;
use crate::layout::{Item, Laid, Layout, Paint, Rect, Sizes};
use crate::style::{self, Sheets, Styles};
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
    Failed,
}

/// Where a style sheet comes from, in the page's order.
enum Source {
    Inline(String),
    /// A <link rel=stylesheet>'s, or an @import's: the resource.
    Linked(usize),
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
    styles: Option<Styles>,
    /// Something changed that needs the page laid out again.
    pub changed: bool,
    ua: Sheet,
    ua_for: f32,
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

/// A page's pictures by element, for the box tree.
struct Picker<'a> {
    of: &'a [(NodeId, usize)],
}

impl Pictures for Picker<'_> {
    fn picture(&mut self, _doc: &Document, node: NodeId) -> Option<usize> {
        self.of.iter().find(|(n, _)| *n == node).map(|(_, r)| *r)
    }
}

impl Page {
    /// A page from its address and HTML (already decoded to text).
    pub fn new(url: Url, html_text: &str) -> Page {
        let doc = html::parse(html_text);
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
            ua: Sheet::default(),
            ua_for: -1.0,
        };
        // <base href> changes where its links go.
        if let Some(b) = p.doc.find("base").and_then(|b| p.doc.element(b)).and_then(|e| e.attr("href")).map(String::from)
            && let Ok(u) = p.url.join(&b)
        {
            p.url = u;
        }
        p.find_resources();
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
    /// the page has, in order.
    fn find_resources(&mut self) {
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
                        let r = self.add(Kind::Sheet, Some(u), None);
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
            }
        }
        // (url()s inside it are relative to it: made whole now, as the page's own are relative to the page.)
        let css_text = absolute_urls(&css_text, &base);
        self.resources[i].state = State::Css(css_text);
        self.styles = None;
        self.changed = true;
    }

    /// A picture came (decoded by the kernel): its size in pixels.
    pub fn picture_came(&mut self, i: usize, w: u32, h: u32) {
        if let Some(r) = self.resources.get_mut(i) {
            r.state = State::Picture(w, h);
            self.changed = true;
        }
    }

    pub fn failed(&mut self, i: usize) {
        if let Some(r) = self.resources.get_mut(i) {
            let was_sheet = r.kind == Kind::Sheet;
            r.state = State::Failed;
            if was_sheet {
                self.styles = None;
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
                if self.ua_for != view.width {
                    self.ua = if self.doc.quirks { css::sheet(&format!("{UA}\n{QUIRKS}"), view) } else { css::sheet(UA, view) };
                    self.ua_for = view.width;
                }
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
                self.styles = Some(style::compute(&self.doc, &Sheets { ua: &self.ua, author: &author }, view, dpr));
            }
            let styles = self.styles.as_ref().expect("worked out above");
            let mut tree = Tree::build(&self.doc, styles, &mut Picker { of: &self.picture_of });
            let laid = Layout::new(&mut tree, ty, &Known(&self.resources), w, h, dpr).run();
            self.laid = Some(laid);
            self.laid_for = (w, h, dpr);
            self.changed = false;
        }
        self.laid.as_ref().expect("laid out above")
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
        let Some(laid) = &self.laid else { return Hit::Nothing };
        let Some(node) = laid.items.iter().rev().find(|i| i.node.is_some() && i.clip.contains(x, y) && bounds(i).contains(x, y)).and_then(|i| i.node)
        else {
            return Hit::Nothing;
        };
        self.hit_from(node)
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
