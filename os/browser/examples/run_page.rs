//! A page with its scripts run, on this computer: `run_page PAGE.html [ADDRESS] [FILES] [--ms N] [--html] [--ppm OUT
//! WIDTH]` reads the page as Chat Computer's own system does, runs its scripts (and its timers, for N ms of the
//! page's time: 2000 if not said), and prints what its scripts printed; with --html, the page as its scripts left it.
//! Its style sheets, pictures and scripts, and what its scripts fetch, are read from FILES (a folder) by their
//! address's path (or as files.txt there names them), when it's given. With --ppm it's drawn WIDTH CSS pixels wide (and
//! its window is that wide for its scripts too; 1280 if not said), 900 tall (--window H: H tall).

use browser::page::{Kind, Page, State};
use browser::paint::{Painter, Shown, Surface};
use browser::script::Pixels;
use std::sync::Arc;
use std::io::Write;
use text::Type;
use web::Url;

struct Pictures(Vec<Option<Arc<Pixels>>>);

impl Shown for Pictures {
    fn picture(&self, n: usize) -> Option<(&[u8], u32, u32)> {
        self.0.get(n)?.as_ref().map(|p| (p.rgb.as_slice(), p.width, p.height))
    }
    fn alpha(&self, n: usize) -> Option<&[u8]> {
        self.0.get(n)?.as_ref()?.alpha.as_deref()
    }
}

/// The bytes at an address, from the folder.
fn read(files: &Option<std::path::PathBuf>, index: &std::collections::HashMap<String, std::path::PathBuf>, u: &str) -> Option<Vec<u8>> {
    let dir = files.as_ref()?;
    if let Some(f) = index.get(u) {
        return std::fs::read(f).ok();
    }
    let path = u.split("://").nth(1).and_then(|r| r.find('/').map(|i| &r[i..])).unwrap_or("/");
    let path = path.split(['?', '#']).next().unwrap_or("/").trim_start_matches('/');
    std::fs::read(dir.join(path)).ok()
}

fn main() {
    // (On a thread with a big stack: a page's scripts can go deep, as on Chat Computer's own system, where they run on
    // a stack of their own.)
    std::thread::Builder::new().stack_size(std::env::var("RUN_PAGE_STACK_KB").ok().and_then(|v| v.parse::<usize>().ok()).map_or(256 << 20, |k| k << 10)).spawn(run).expect("a thread").join().ok();
}

fn run() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|a| a == name);
    let positional: Vec<&String> = {
        let mut out = Vec::new();
        let mut skip = 0;
        for a in args.iter().skip(1) {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            match a.as_str() {
                "--ms" | "--why" | "--eval" | "--before" | "--window" | "--scroll" => skip = 1,
                "--ppm" => skip = 2,
                "--html" | "--items" | "--boxes" => {}
                _ => out.push(a),
            }
        }
        out
    };
    if positional.is_empty() {
        eprintln!("usage: run_page PAGE.html [ADDRESS] [FILES] [--ms N] [--html] [--ppm OUT WIDTH]");
        std::process::exit(2);
    }
    let html = std::fs::read(positional[0]).expect("the page");
    let address = positional.get(1).map(|s| s.as_str()).unwrap_or("https://example.com/");
    let files = positional.get(2).map(std::path::PathBuf::from);
    let total_ms: f64 = flag("--ms").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(2000.0);
    let mut index = std::collections::HashMap::new();
    if let Some(dir) = &files
        && let Ok(list) = std::fs::read_to_string(dir.join("files.txt"))
    {
        for line in list.lines() {
            if let Some((u, f)) = line.split_once('\t') {
                index.insert(u.to_string(), dir.join(f));
            }
        }
    }
    let text = web::page::decode(&html, "text/html");
    let started = std::time::Instant::now();
    let epoch = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
    // (The window's size: as wide as the picture asked for, if one is.)
    let width: f32 = flag("--ppm").and_then(|i| args.get(i + 2)).and_then(|v| v.parse().ok()).unwrap_or(1280.0);
    // (Its window's height: 900 if not said; --window H says.)
    let height: f32 = flag("--window").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(900.0);
    let mut page = Page::with_scripts(Url::parse(address).expect("an address"), &text, epoch, (width, height, 1.0), 0x1234_5678);
    // (A script of the tool's own, run before the page's: to watch what they do.)
    if let Some(i) = flag("--before") {
        page.run_text(&args[i + 1]);
    }
    let mut pictures = Pictures(Vec::new());
    let ty = Type::builtin();
    let mut now = 0.0;
    let mut laid_once = false;
    loop {
        // What the page asks for.
        fetch(&mut page, &mut pictures, &files, &index);
        // What its scripts fetch.
        for (id, u, method, _headers, _body) in page.take_requests() {
            let r = match read(&files, &index, &u) {
                Some(b) => Ok(browser::script::Response { status: 200, status_text: String::from("OK"), url: u.clone(), headers: vec![(String::from("content-type"), String::from(if u.ends_with(".json") { "application/json" } else { "text/plain" }))], body: b }),
                None => Ok(browser::script::Response { status: 404, status_text: String::from("Not Found"), url: u.clone(), headers: Vec::new(), body: Vec::new() }),
            };
            eprintln!("(fetch {method} {u})");
            page.answer(id, r);
        }
        page.run(now);
        if !laid_once && page.sheets_done() {
            page.lay_out(&ty, width, height, 1.0);
            laid_once = true;
        }
        page.animation_frame(now);
        for line in page.take_console() {
            println!("console: {line}");
        }
        if let Some(n) = page.navigation.take() {
            println!("(the page goes to {})", n.url);
        }
        // On to the next timer (or 16 ms on).
        let next = page.next_due().unwrap_or(f64::INFINITY);
        let step = if next.is_finite() && next > now { next } else { now + 16.0 };
        if step > total_ms {
            break;
        }
        now = step;
        if page.changed {
            page.lay_out(&ty, width, height, 1.0);
        }
    }
    eprintln!("({:.0} ms, {} ms of the page's time; its scripts {} deep)", started.elapsed().as_secs_f64() * 1000.0, now as u64, page.script_depth());
    if flag("--items").is_some() {
        // What's drawn, item by item (text and rectangles): where, and in what colour.
        let laid = page.lay_out(&ty, width, height, 1.0);
        println!("(laid out: {} items, {} px tall, background {:?})", laid.items.len(), laid.height, laid.background);
        for it in laid.items.iter().take(400) {
            match &it.paint {
                browser::layout::Paint::Text { x, y, text, color, size, .. } => println!("text {x:.0},{y:.0} {size}px {color:?} {:?} clip {:?}", text.chars().take(60).collect::<String>(), it.clip),
                browser::layout::Paint::Rect { x, y, w, h, color, .. } => println!("rect {x:.0},{y:.0} {w:.0}x{h:.0} {color:?}"),
                browser::layout::Paint::Picture { x, y, w, h, n, .. } => println!("picture {n} {x:.0},{y:.0} {w:.0}x{h:.0}"),
                _ => {}
            }
        }
    }
    if let Some(i) = flag("--why") {
        // Where some words are drawn from: each element they're in, outwards, with its box.
        let want = args[i + 1].clone();
        let laid = page.lay_out(&ty, width, height, 1.0);
        let found = laid.items.iter().find(|it| matches!(&it.paint, browser::layout::Paint::Text { text, .. } if text.contains(want.as_str()))).map(|it| it.node);
        let boxes = laid.boxes.clone();
        if let Some(node) = found {
            let mut at = node;
            while let Some(n) = at {
                let rect = boxes.iter().filter(|(b, _)| *b == n).map(|(_, r)| *r).collect::<Vec<_>>();
                let d = &page.doc;
                let e = d.element(n);
                let desc = e.map(|e| format!("<{} id={:?} class={:?} style={:?}>", e.name, e.attr("id").unwrap_or(""), e.attr("class").unwrap_or("").chars().take(60).collect::<String>(), e.attr("style").unwrap_or("").chars().take(80).collect::<String>())).unwrap_or_else(|| String::from("(not an element)"));
                let st = page.style_of(n).map(|s| format!("display {:?} position {:?} margin {:?} clip {} families {:?} web_face {:?}", s.display, s.position, s.margin, s.clip, s.families, s.web_face)).unwrap_or_default();
                println!("why: {desc} {rect:?} {st}");
                at = d.parent(n);
            }
        }
    }
    if let Some(i) = flag("--eval") {
        // A script's value once the page has run (as Chromium's would be asked for): logged.
        page.run_text(&format!("try {{ console.log('eval: ' + String({})); }} catch (e) {{ console.log('eval threw: ' + e); }}", args[i + 1]));
        for line in page.take_console() {
            println!("{line}");
        }
    }
    if flag("--boxes").is_some() {
        // Every element with an id: its border box (CSS pixels), as chrome_boxes.py prints Chromium's, to compare.
        let laid = page.lay_out(&ty, width, height, 1.0);
        let boxes = laid.boxes.clone();
        let d = &page.doc;
        for n in 0..d.nodes.len() {
            let Some(e) = d.element(n) else { continue };
            let Some(id) = e.attr("id").filter(|i| !i.is_empty()) else { continue };
            let r = boxes.iter().find(|(b, _)| *b == n).map(|(_, r)| *r);
            match r {
                Some(r) => println!("{}#{}: x {:.1} y {:.1} w {:.1} h {:.1}", e.name, id, r.x, r.y, r.w, r.h),
                None => println!("{}#{}: x 0.0 y 0.0 w 0.0 h 0.0", e.name, id),
            }
        }
    }
    if flag("--html").is_some() {
        println!("{}", page.doc.outer_html(0));
    }
    if let Some(i) = flag("--ppm") {
        let out = &args[i + 1];
        // (Laid out again, and what that asks for fetched: an <svg>'s picture made in its colours.)
        page.lay_out(&ty, width, height, 1.0);
        fetch(&mut page, &mut pictures, &files, &index);
        let laid = page.lay_out(&ty, width, height, 1.0);
        let w = width as usize;
        // (The whole page, up to 6000 pixels; with --window, the window's first screen, as a test's picture is.)
        let h = if flag("--window").is_some() { height as usize } else { (laid.height.ceil() as usize).clamp(1, 6000) };
        let mut pixels = vec![0u32; w * h];
        let mut surface = Surface { pixels: &mut pixels, width: w, height: h, clip: (0, 0, w as i32, h as i32) };
        let mut painter = Painter::new();
        // (--scroll Y: the window scrolled Y CSS pixels down.)
        let scroll: f32 = flag("--scroll").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        if scroll > 0.0 {
            page.scrolled_by_user(scroll);
        }
        let laid = page.laid().expect("laid out");
        painter.draw(&ty, laid, 0, 0, scroll, &mut surface, &page.shown(&pictures), true);
        let mut f = std::fs::File::create(out).expect("the picture's file");
        write!(f, "P6\n{w} {h}\n255\n").ok();
        let mut bytes = Vec::with_capacity(w * h * 3);
        for p in &pixels {
            bytes.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
        }
        f.write_all(&bytes).ok();
    }
    if std::env::var("RUN_PAGE_RESOURCES").is_ok() {
        for (i, r) in page.resources.iter().enumerate() {
            let state = match &r.state {
                State::Css(c) => format!("css {} bytes", c.len()),
                State::Picture(w, h) => format!("picture {w}x{h}"),
                State::Font(id) => format!("font {id}"),
                State::Failed => String::from("failed"),
                State::Waiting => String::from("waiting"),
                State::Script(_) => String::from("script"),
            };
            eprintln!("resource {i}: {:?} {} {state}", r.kind, r.url.as_ref().map_or(String::from("(inline)"), |u| u.to_string()));
        }
    }
    let _ = State::Waiting;
}

/// What the page asks for (its sheets, pictures, scripts and modules), read from the folder.
fn fetch(page: &mut Page, pictures: &mut Pictures, files: &Option<std::path::PathBuf>, index: &std::collections::HashMap<String, std::path::PathBuf>) {
    {
        while let Some((i, kind, url, inline)) = page.next_wanted() {
            let bytes = match (inline, &url) {
                (Some(b), _) => Some(b),
                (None, Some(u)) => read(files, index, &u.to_string()),
                _ => None,
            };
            match (kind, bytes) {
                (Kind::Sheet, Some(b)) => page.sheet_came(i, String::from_utf8_lossy(&b).into_owned()),
                (Kind::Script | Kind::Module, Some(b)) => page.script_came(i, String::from_utf8_lossy(&b).into_owned()),
                (Kind::Font, Some(b)) => page.font_came(i, &b),
                (Kind::Picture, Some(b)) => match picture::decode_alpha(&b, 4000, 4000, u64::MAX) {
                    Ok((p, alpha)) => {
                        let (w, h) = picture::size(&b).unwrap_or((p.width, p.height));
                        let px = Arc::new(Pixels { rgb: p.rgb, alpha, width: p.width, height: p.height, natural: (w, h) });
                        page.picture_came(i, w, h);
                        page.picture_pixels(i, px.clone());
                        if pictures.0.len() <= i {
                            pictures.0.resize(i + 1, None);
                        }
                        pictures.0[i] = Some(px);
                    }
                    Err(_) => page.failed(i),
                },
                _ => page.failed(i),
            }
        }
    }
}
