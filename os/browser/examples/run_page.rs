//! A page with its scripts run, on this computer: `run_page PAGE.html [ADDRESS] [FILES] [--ms N] [--html] [--ppm OUT
//! WIDTH]` reads the page as Chat Computer's own system does, runs its scripts (and its timers, for N ms of the
//! page's time: 2000 if not said), and prints what its scripts printed; with --html, the page as its scripts left it.
//! Its style sheets, pictures and scripts, and what its scripts fetch, are read from FILES (a folder) by their
//! address's path (or as files.txt there names them), when it's given. With --ppm it's drawn WIDTH CSS pixels wide.

use browser::page::{Kind, Page, State};
use browser::paint::{Painter, Shown, Surface};
use std::io::Write;
use text::Type;
use web::Url;

struct Pictures(Vec<Option<(Vec<u8>, u32, u32, Option<Vec<u8>>)>>);

impl Shown for Pictures {
    fn picture(&self, n: usize) -> Option<(&[u8], u32, u32)> {
        self.0.get(n)?.as_ref().map(|(p, w, h, _)| (p.as_slice(), *w, *h))
    }
    fn alpha(&self, n: usize) -> Option<&[u8]> {
        self.0.get(n)?.as_ref()?.3.as_deref()
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
                "--ms" => skip = 1,
                "--ppm" => skip = 2,
                "--html" => {}
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
    let mut page = Page::with_scripts(Url::parse(address).expect("an address"), &text, epoch, (1280.0, 800.0, 1.0), 0x1234_5678);
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
            page.lay_out(&ty, 1280.0, 800.0, 1.0);
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
            page.lay_out(&ty, 1280.0, 800.0, 1.0);
        }
    }
    eprintln!("({:.0} ms, {} ms of the page's time)", started.elapsed().as_secs_f64() * 1000.0, now as u64);
    if flag("--html").is_some() {
        println!("{}", page.doc.outer_html(0));
    }
    if let Some(i) = flag("--ppm") {
        let out = &args[i + 1];
        let width: f32 = args[i + 2].parse().expect("a width");
        // (Laid out at this width, and what that asks for fetched: an <svg>'s picture made in its colours.)
        page.lay_out(&ty, width, 900.0, 1.0);
        fetch(&mut page, &mut pictures, &files, &index);
        let laid = page.lay_out(&ty, width, 900.0, 1.0);
        let w = width as usize;
        let h = (laid.height.ceil() as usize).clamp(1, 6000);
        let mut pixels = vec![0u32; w * h];
        let mut surface = Surface { pixels: &mut pixels, width: w, height: h, clip: (0, 0, w as i32, h as i32) };
        let mut painter = Painter::new();
        let laid = page.laid().expect("laid out");
        painter.draw(&ty, laid, 0, 0, 0.0, &mut surface, &pictures, true);
        let mut f = std::fs::File::create(out).expect("the picture's file");
        write!(f, "P6\n{w} {h}\n255\n").ok();
        let mut bytes = Vec::with_capacity(w * h * 3);
        for p in &pixels {
            bytes.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
        }
        f.write_all(&bytes).ok();
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
                (Kind::Picture, Some(b)) => match picture::decode_alpha(&b, 4000, 4000, u64::MAX) {
                    Ok((p, alpha)) => {
                        let (w, h) = picture::size(&b).unwrap_or((p.width, p.height));
                        page.picture_came(i, w, h);
                        if pictures.0.len() <= i {
                            pictures.0.resize(i + 1, None);
                        }
                        pictures.0[i] = Some((p.rgb, p.width, p.height, alpha));
                    }
                    Err(_) => page.failed(i),
                },
                _ => page.failed(i),
            }
        }
    }
}
