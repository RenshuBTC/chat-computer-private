//! A page drawn on this computer, to look at: `render PAGE.html WIDTH OUT.ppm [ADDRESS] [FILES]` lays the page out
//! WIDTH CSS pixels wide (at 1 device pixel each) as Chat Computer's own system does, and writes it as a PPM picture
//! (as tall as the page, at most 6000 pixels). Its style sheets and pictures are read from FILES (a folder), by
//! their address's path, when it's given; else only what's in the page shows.

use browser::page::{Kind, Page, State};
use browser::paint::{Painter, Shown, Surface};
use std::io::Write;
use text::Type;
use web::Url;

struct Pictures(Vec<Option<(Vec<u8>, u32, u32)>>);

impl Shown for Pictures {
    fn picture(&self, n: usize) -> Option<(&[u8], u32, u32)> {
        self.0.get(n)?.as_ref().map(|(p, w, h)| (p.as_slice(), *w, *h))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: render PAGE.html WIDTH OUT.ppm [ADDRESS] [FILES]");
        std::process::exit(2);
    }
    let html = std::fs::read(&args[1]).expect("the page");
    let width: f32 = args[2].parse().expect("a width");
    let address = args.get(4).map(String::as_str).unwrap_or("https://example.com/");
    let files = args.get(5).map(std::path::PathBuf::from);
    // (A folder from a mirror of the page: files.txt says which file has each address's bytes.)
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
    let mut page = Page::new(Url::parse(address).expect("an address"), &text);
    let mut pictures = Pictures(Vec::new());
    while let Some((i, kind, url, inline)) = page.next_wanted() {
        let bytes = match (inline, &url, &files) {
            (Some(b), _, _) => Some(b),
            (None, Some(u), Some(dir)) => match index.get(&u.to_string()) {
                Some(f) => std::fs::read(f).ok(),
                None => {
                    let path = u.path.split('?').next().unwrap_or("/").trim_start_matches('/');
                    std::fs::read(dir.join(path)).ok()
                }
            },
            _ => None,
        };
        match (kind, bytes) {
            (Kind::Sheet, Some(b)) => page.sheet_came(i, String::from_utf8_lossy(&b).into_owned()),
            (Kind::Picture, Some(b)) => match picture::decode(&b, 4000, 4000, [255, 255, 255]) {
                Ok(p) => {
                    let (w, h) = picture::shown_size(&b).unwrap_or((p.width, p.height));
                    page.picture_came(i, w, h);
                    if let Some(own) = picture::intrinsic(&b) {
                        page.picture_own(i, own);
                    }
                    if pictures.0.len() <= i {
                        pictures.0.resize(i + 1, None);
                    }
                    pictures.0[i] = Some((p.rgb, p.width, p.height));
                }
                Err(_) => page.failed(i),
            },
            _ => page.failed(i),
        }
    }
    let ty = Type::builtin();
    let laid = page.lay_out(&ty, width, 900.0, 1.0);
    let laid_ms = started.elapsed().as_millis();
    let w = width as usize;
    let h = (laid.height.ceil() as usize).clamp(1, 6000);
    let mut pixels = vec![0u32; w * h];
    let mut surface = Surface { pixels: &mut pixels, width: w, height: h, clip: (0, 0, w as i32, h as i32) };
    let mut painter = Painter::new();
    let laid = page.laid().expect("laid out");
    if std::env::var_os("RENDER_ITEMS").is_some() {
        for i in &laid.items {
            eprintln!("{:?} node {:?} clip {:?}", i.paint, i.node, i.clip);
        }
    }
    painter.draw(&ty, laid, 0, 0, 0.0, &mut surface, &pictures, true);
    let mut out = std::fs::File::create(&args[3]).expect("the picture");
    write!(out, "P6\n{w} {h}\n255\n").unwrap();
    let mut bytes = Vec::with_capacity(w * h * 3);
    for p in &pixels {
        bytes.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    out.write_all(&bytes).unwrap();
    eprintln!(
        "{}: {} items, {}x{} px, laid out in {laid_ms} ms; title {:?}; sheets {}, pictures {}",
        args[1],
        laid.items.len(),
        w,
        laid.height as usize,
        page.title(),
        page.resources.iter().filter(|r| r.kind == Kind::Sheet && matches!(r.state, State::Css(_))).count(),
        page.resources.iter().filter(|r| r.kind == Kind::Picture && matches!(r.state, State::Picture(..))).count(),
    );
}
