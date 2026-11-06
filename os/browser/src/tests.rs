//! The engine on small pages: where things are laid out, and what clicks and forms do.

use crate::layout::{Laid, Paint};
use crate::page::{Hit, Page, Request};
use text::Type;
use web::Url;

fn page(html: &str) -> Page {
    Page::new(Url::parse("https://example.com/a/page").unwrap(), html)
}

/// A page laid out 600 pixels wide (its pictures all 100 × 50).
fn laid(p: &mut Page) -> &Laid {
    while let Some((i, _, _, _)) = p.next_wanted() {
        p.picture_came(i, 100, 50);
    }
    let ty = Type::builtin();
    p.lay_out(&ty, 600.0, 400.0, 1.0)
}

/// Where the words `text` are drawn (x, y).
fn words(l: &Laid, text: &str) -> (f32, f32) {
    l.items
        .iter()
        .find_map(|i| match &i.paint {
            Paint::Text { x, y, text: t, .. } if t.contains(text) => Some((*x, *y)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{text:?} isn't drawn: {:?}", l.items.iter().map(|i| &i.paint).collect::<Vec<_>>()))
}

/// The rectangles drawn in a colour (x, y, w, h).
fn rects(l: &Laid, color: u32) -> Vec<(f32, f32, f32, f32)> {
    l.items
        .iter()
        .filter_map(|i| match &i.paint {
            Paint::Rect { x, y, w, h, color: c, .. } if *c == color => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .collect()
}

const RED: u32 = 0xFFFF_0000;
const GREEN: u32 = 0xFF00_8000;
const BLUE: u32 = 0xFF00_00FF;

#[test]
fn blocks_one_under_another_their_margins_collapsing() {
    let mut p = page("<body style='margin:0'><div style='margin-bottom:30px;height:10px;background:red'></div>\n\n\
                      <div style='margin-top:20px;height:10px;background:blue'></div></body>");
    let l = laid(&mut p);
    assert_eq!(rects(l, RED), [(0.0, 0.0, 600.0, 10.0)]);
    assert_eq!(rects(l, BLUE), [(0.0, 40.0, 600.0, 10.0)], "30px and 20px collapse to 30px, and spaces between blocks are nothing");
}

#[test]
fn a_block_s_auto_margins_centre_it_and_max_width_holds_it() {
    let mut p = page("<body style='margin:0'><div style='max-width:200px;margin:0 auto;height:5px;background:red'></div></body>");
    let l = laid(&mut p);
    assert_eq!(rects(l, RED), [(200.0, 0.0, 200.0, 5.0)]);
}

#[test]
fn words_wrap_beside_a_float_and_go_under_it_after() {
    let mut p = page(
        "<body style='margin:0;font-size:16px'><div style='float:left;width:100px;height:50px;background:red'></div>\
         <p style='margin:0'>Beside the float. Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod \
         tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation \
         ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate \
         velit esse cillum dolore eu fugiat nulla pariatur. Last</p></body>",
    );
    let l = laid(&mut p);
    assert_eq!(rects(l, RED), [(0.0, 0.0, 100.0, 50.0)]);
    let (x, y) = words(l, "Beside");
    assert!(x >= 100.0 && y == 0.0, "the first line is beside the float: {x}, {y}");
    let (x, y) = words(l, "Last");
    assert!(x < 100.0 && y >= 50.0, "a line below the float starts at the left: {x}, {y}");
}

#[test]
fn a_float_among_words_goes_at_the_line_s_side() {
    let mut p = page("<body style='margin:0'><p style='margin:0'>Text <img src='/a.png' style='float:right'> after.</p></body>");
    let l = laid(&mut p);
    let pic = l.items.iter().find_map(|i| match i.paint {
        Paint::Picture { x, y, w, h, .. } => Some((x, y, w, h)),
        _ => None,
    });
    assert_eq!(pic, Some((500.0, 0.0, 100.0, 50.0)));
    assert_eq!(words(l, "Text").1, 0.0);
}

#[test]
fn clear_goes_below_the_floats() {
    let mut p = page("<body style='margin:0'><div style='float:left;width:50px;height:80px'></div>\
                      <div style='clear:both;height:5px;background:red'></div></body>");
    let l = laid(&mut p);
    assert_eq!(rects(l, RED), [(0.0, 80.0, 600.0, 5.0)]);
}

#[test]
fn flex_items_grow_and_keep_their_gaps() {
    let mut p = page("<body style='margin:0'><div style='display:flex;gap:30px'>\n\
                      <div style='flex:1;height:5px;background:red'></div>\n\
                      <div style='flex:2;height:5px;background:blue'></div></div></body>");
    let l = laid(&mut p);
    assert_eq!(rects(l, RED), [(0.0, 0.0, 190.0, 5.0)]);
    assert_eq!(rects(l, BLUE), [(220.0, 0.0, 380.0, 5.0)]);
}

#[test]
fn flex_justify_and_align_centre() {
    let mut p = page("<body style='margin:0'><div style='display:flex;justify-content:center;align-items:center;height:100px'>\
                      <div style='width:100px;height:20px;background:red'></div></div></body>");
    let l = laid(&mut p);
    assert_eq!(rects(l, RED), [(250.0, 40.0, 100.0, 20.0)]);
}

#[test]
fn grid_columns_of_lengths_and_fractions() {
    let mut p = page("<body style='margin:0'><div style='display:grid;grid-template-columns:100px 1fr 1fr;column-gap:10px'>\
                      <div style='height:5px;background:red'></div><div style='height:5px;background:blue'></div>\
                      <div style='height:5px;background:green'></div></div></body>");
    let l = laid(&mut p);
    assert_eq!(rects(l, RED), [(0.0, 0.0, 100.0, 5.0)]);
    assert_eq!(rects(l, BLUE), [(110.0, 0.0, 240.0, 5.0)]);
    assert_eq!(rects(l, GREEN), [(360.0, 0.0, 240.0, 5.0)]);
}

#[test]
fn a_table_s_columns_and_a_cell_across_two() {
    let mut p = page("<body style='margin:0'><table style='border-spacing:0;width:400px'>\
                      <tr><td colspan=2 style='background:red'>Across</td></tr>\
                      <tr><td style='width:100px;background:blue'>A</td><td style='background:green'>B</td></tr></table></body>");
    let l = laid(&mut p);
    let red = rects(l, RED);
    let (blue, green) = (rects(l, BLUE), rects(l, GREEN));
    assert_eq!(red.len(), 1);
    assert_eq!((red[0].0, red[0].2), (0.0, 400.0), "{red:?}");
    assert_eq!((blue[0].0 + blue[0].2, green[0].0 + green[0].2), (green[0].0, 400.0), "{blue:?} {green:?}");
}

#[test]
fn a_table_in_center_is_centred_and_its_cells_words_aren_t_in_quirks_mode() {
    let mut p = page("<html><body style='margin:0'><center><table width='50%' style='border-spacing:0'><tr>\
                      <td style='background:red'>Left</td></tr></table></center></body></html>");
    assert!(p.doc.quirks, "no doctype: quirks mode");
    let l = laid(&mut p);
    assert_eq!(rects(l, RED)[0].0, 150.0);
    assert!((words(l, "Left").0 - 151.0).abs() < 0.5, "the cell's words at its left (after its padding): {:?}", words(l, "Left"));
}

#[test]
fn centred_words() {
    let mut p = page("<!doctype html><body style='margin:0'><p style='text-align:center;margin:0'>Hi</p></body>");
    assert!(!p.doc.quirks);
    let l = laid(&mut p);
    let (x, _) = words(l, "Hi");
    assert!(x > 280.0 && x < 300.0, "{x}");
}

#[test]
fn a_click_on_a_link_and_a_fragment() {
    let mut p = page("<body style='margin:0'><a href='../b?x=1#top' style='display:block;height:40px'>Link</a>\
                      <a href='#here' style='display:block;height:40px'>Here</a><p id=here>There</p></body>");
    laid(&mut p);
    assert_eq!(p.at(10.0, 10.0), Hit::Link(Url::parse("https://example.com/b?x=1").unwrap(), Some("top".into())));
    assert_eq!(p.at(10.0, 50.0), Hit::Fragment("here".into()));
    assert!(p.fragment_top("here").is_some_and(|y| y >= 80.0));
}

#[test]
fn a_form_sent_by_get_and_by_post() {
    let mut p = page("<body><form action='/search'><input name=q value='x'><input type=checkbox name=c checked>\
                      <input type=hidden name=h value='1'><select name=s><option>One<option selected value=2>Two</select>\
                      <button name=go value=yes>Go</button></form>\
                      <form method=post action='https://other.org/in'><input name=user><textarea name=t>Hi</textarea></form></body>");
    laid(&mut p);
    let forms: Vec<usize> = p.doc.descendants(0).filter(|&n| p.doc.name(n) == "form").collect();
    let q = p.doc.descendants(0).find(|&n| p.doc.element(n).is_some_and(|e| e.attr("name") == Some("q"))).unwrap();
    p.set_value(q, "sunny day & more");
    let r = p.submit(forms[0], None).unwrap();
    assert_eq!(r, Request { url: Url::parse("https://example.com/search?q=sunny+day+%26+more&c=on&h=1&s=2").unwrap(), body: None });
    let user = p.doc.descendants(0).find(|&n| p.doc.element(n).is_some_and(|e| e.attr("name") == Some("user"))).unwrap();
    p.set_value(user, "Sam");
    let r = p.submit(forms[1], None).unwrap();
    assert_eq!(r.url, Url::parse("https://other.org/in").unwrap());
    assert_eq!(r.body.as_deref(), Some(&b"user=Sam&t=Hi"[..]));
}

#[test]
fn the_persons_form_is_sent_after_its_submit_event() {
    // A page with scripts: its submit event first. One that cancels it keeps the page (sending what it must itself);
    // one that doesn't lets the form go, with what its handler changed.
    let mut p = scripted(
        "<body><form id=a action='/note' method=post><input name=n value='hi'><button>Add</button></form>\
         <form id=b action='/find'><input name=q value='x'></form><script>\
         document.getElementById('a').addEventListener('submit', function (e) { e.preventDefault(); \
           console.log('added ' + e.target.n.value + ' by ' + (e.submitter ? e.submitter.textContent : 'none')); });\
         document.getElementById('b').onsubmit = function (e) { e.target.q.value = 'y'; console.log('finding'); };\
         var a = document.forms[0]; console.log([a === document.getElementById('a'), a instanceof HTMLFormElement, \
           'n' in a, 'm' in a, a.elements.n === a.n, a.n.form === a, a.method, a.action, a.nodeName, typeof a.submit].join(' '));\
         </script></body>",
    );
    p.run(0.0);
    assert_eq!(p.take_console(), ["true true true false true true post https://example.com/note FORM function"]);
    let id = |p: &Page, id: &str| p.doc.descendants(0).find(|&n| p.doc.element(n).is_some_and(|e| e.attr("id") == Some(id))).unwrap();
    let (a, b) = (id(&p, "a"), id(&p, "b"));
    let button = p.doc.descendants(a).find(|&n| p.doc.name(n) == "button").unwrap();
    assert_eq!(p.person_submits(a, Some(button)), None, "its script cancelled it");
    assert_eq!(p.take_console(), ["added hi by Add"]);
    let r = p.person_submits(b, None).expect("nothing cancelled it");
    assert_eq!(r, Request { url: Url::parse("https://example.com/find?q=y").unwrap(), body: None });
    assert_eq!(p.take_console(), ["finding"]);
    // A page without scripts: the form at once.
    let mut p = page("<body><form action='/find'><input name=q value='z'></form></body>");
    let f = p.doc.descendants(0).find(|&n| p.doc.name(n) == "form").unwrap();
    assert_eq!(p.person_submits(f, None).map(|r| r.url), Some(Url::parse("https://example.com/find?q=z").unwrap()));
}

#[test]
fn a_page_that_sends_itself_on() {
    let p = page("<head><meta http-equiv='refresh' content='0; url=/next'></head><body>Moved</body>");
    assert_eq!(p.refresh(), Some(Url::parse("https://example.com/next").unwrap()));
    let p = page("<head><meta http-equiv='refresh' content='60; url=/later'></head>");
    assert_eq!(p.refresh(), None, "a refresh a minute later isn't followed");
}

#[test]
fn style_sheets_and_pictures_asked_for_in_order_and_a_picture_written_in_the_page() {
    let mut p = page("<head><link rel=stylesheet href='/s.css'><link rel='alternate stylesheet' href='/no.css'>\
                      <link rel=stylesheet href='/print.css' media=print></head>\
                      <body><img src='data:image/gif;base64,R0lGODlhAQABAAAAACw='><img srcset='/big.png 2x, /small.png 1x'>\
                      <img data-src='/lazy.png'></body>");
    let mut asked = Vec::new();
    while let Some((_, kind, url, inline)) = p.next_wanted() {
        asked.push((kind, url.map(|u| u.path), inline.map(|b| b.len())));
    }
    use crate::page::Kind::{Picture, Sheet};
    assert_eq!(
        asked,
        [
            (Sheet, Some("/s.css".into()), None),
            (Picture, None, Some(14)),
            (Picture, Some("/small.png".into()), None),
            (Picture, Some("/lazy.png".into()), None),
        ]
    );
}

#[test]
fn srcsets_read_as_html_reads_them_and_a_candidate_picked_as_chrome_picks_it() {
    use crate::css::Viewport;
    use crate::srcset::{Candidate, parse, pick, source_size};
    let c = |url: &str, width: Option<f32>, density: Option<f32>| Candidate { url: url.into(), width, density };
    assert_eq!(parse("a.png 1x, b.png 2x"), [c("a.png", None, Some(1.0)), c("b.png", None, Some(2.0))]);
    assert_eq!(parse(" a.jpg 400w,b.jpg  800w "), [c("a.jpg", Some(400.0), None), c("b.jpg", Some(800.0), None)]);
    // An address with commas in it; one with no descriptor (1x); ones whose descriptors don't make sense, left out.
    assert_eq!(parse("data:image/png;base64,AAA= 2x"), [c("data:image/png;base64,AAA=", None, Some(2.0))]);
    assert_eq!(parse("a.png, b.png 1.5x"), [c("a.png", None, None), c("b.png", None, Some(1.5))]);
    assert_eq!(parse("a.png 1x 2x, b.png 100w 2x, c.png 0w, d.png (x, y), e.png 2x"), [c("e.png", None, Some(2.0))]);
    assert_eq!(parse("a.png 100w 50h, b.png 50h"), [c("a.png", Some(100.0), None)]);

    // The least dense at least as dense as the screen; between two, the nearer by their geometric mean.
    let x = || parse("s.jpg 1x, m.jpg 1.5x, l.jpg 2x");
    assert_eq!(pick(x(), 0.0, 1.0), Some(("s.jpg".into(), 1.0)));
    assert_eq!(pick(x(), 0.0, 1.6), Some(("m.jpg".into(), 1.5)));
    assert_eq!(pick(x(), 0.0, 1.8), Some(("l.jpg".into(), 2.0)));
    assert_eq!(pick(x(), 0.0, 3.0), Some(("l.jpg".into(), 2.0)));
    // Widths: each one's density its width over the picture's size (500px here).
    let w = || parse("a.jpg 400w, b.jpg 800w, c.jpg 1600w");
    assert_eq!(pick(w(), 500.0, 1.0), Some(("b.jpg".into(), 1.6)));
    assert_eq!(pick(w(), 500.0, 2.0), Some(("b.jpg".into(), 1.6)));
    assert_eq!(pick(w(), 500.0, 2.5), Some(("c.jpg".into(), 3.2)));

    // sizes: the first whose media condition matches, else 100vw.
    let v = Viewport { width: 1000.0, height: 800.0 };
    assert_eq!(source_size(Some("(max-width: 600px) 100vw, 50vw"), v), 500.0);
    assert_eq!(source_size(Some("(min-width: 900px) calc(30vw - 2em), 100vw"), v), 268.0);
    assert_eq!(source_size(Some("bogus, 300px"), v), 300.0);
    assert_eq!(source_size(Some("(max-width: 500px) 50vw"), v), 1000.0);
    assert_eq!(source_size(None, v), 1000.0);
}

#[test]
fn a_picture_s_source_chosen_by_its_media_and_type_and_a_srcset_s_density_sizes_its_picture() {
    let html = "<body style='margin:0'><picture><source srcset='/a.avif' type='image/avif'>\
                <source srcset='/narrow.webp' media='(max-width: 400px)' type='image/webp'>\
                <source srcset='/wide.webp 1x, /wide2.webp 2x' type='image/webp'><img src='/fallback.jpg'></picture>\
                <img srcset='/hi.png 2x' src='/lo.png'><img srcset='/w400.png 400w, /w800.png 800w' sizes='200px'></body>";
    let ty = Type::builtin();
    for (w, dpr, want) in [
        (600.0, 1.0, ["/wide.webp", "/lo.png", "/w400.png"]),
        (300.0, 1.0, ["/narrow.webp", "/lo.png", "/w400.png"]),
        (1200.0, 2.0, ["/wide2.webp", "/hi.png", "/w400.png"]),
    ] {
        let mut p = page(html);
        p.lay_out(&ty, w, 800.0, dpr);
        let mut asked = Vec::new();
        while let Some((i, _, url, _)) = p.next_wanted() {
            asked.push(url.map(|u| u.path).unwrap_or_default());
            p.picture_came(i, 100, 50);
        }
        // (In the order they were first chosen: those chosen for the window the page was made for, first.)
        asked.sort();
        let mut want = want;
        want.sort();
        assert_eq!(asked, want, "{w} pixels wide, {dpr} to a CSS pixel");
        // Each picture's size in CSS pixels: its own over its density (the 400w one's is 2, as the picture's 200px).
        let l = p.lay_out(&ty, w, 800.0, dpr);
        let pics: Vec<(f32, f32)> = l.items.iter().filter_map(|i| if let Paint::Picture { w, h, .. } = i.paint { Some((w / dpr, h / dpr)) } else { None }).collect();
        let first = if dpr == 2.0 { (50.0, 25.0) } else { (100.0, 50.0) };
        assert_eq!(pics, [first, first, (50.0, 25.0)], "{w} pixels wide, {dpr} to a CSS pixel");
    }
    // A window made narrower before the pictures were fetched: the narrow one's fetched, not the wide one.
    let mut p = page(html);
    p.lay_out(&ty, 600.0, 800.0, 1.0);
    p.lay_out(&ty, 300.0, 800.0, 1.0);
    let mut asked = Vec::new();
    while let Some((_, _, url, _)) = p.next_wanted() {
        asked.push(url.map(|u| u.path).unwrap_or_default());
    }
    asked.sort();
    assert_eq!(asked, ["/lo.png", "/narrow.webp", "/w400.png"]);
}

#[test]
fn an_svg_with_only_a_ratio_is_sized_as_css_says_in_a_background_and_in_an_img() {
    let mut p = page("<body style='margin:0'><div style='width:100px;height:40px;background:url(/i.svg) no-repeat'></div>\
                      <div style='width:200px;height:30px;background:url(/i.svg) 0 0/50px no-repeat'></div>\
                      <div style='width:200px'><img src='/w.svg'></div><img src='/none.svg' style='display:block'></body>");
    let ty = Type::builtin();
    p.lay_out(&ty, 600.0, 400.0, 1.0);
    for _ in 0..2 {
        while let Some((i, _, url, _)) = p.next_wanted() {
            p.picture_came(i, 300, 150);
            let ratio = if url.is_some_and(|u| u.path == "/none.svg") { None } else { Some(2.0) };
            p.picture_own(i, picture::Intrinsic { width: None, height: None, ratio });
        }
        p.lay_out(&ty, 600.0, 400.0, 1.0);
    }
    let l = p.lay_out(&ty, 600.0, 400.0, 1.0);
    let tiles: Vec<(f32, f32)> = l.items.iter().filter_map(|i| if let Paint::Tiles { tile, .. } = &i.paint { Some((tile.w, tile.h)) } else { None }).collect();
    // A ratio alone: as big as fits the box (as contain); a width given: the height from the ratio.
    assert_eq!(tiles, [(80.0, 40.0), (50.0, 25.0)]);
    // In an <img>: as wide as there's room for, in its ratio; with neither a size nor a ratio, 300 by 150.
    let pics: Vec<(f32, f32)> = l.items.iter().filter_map(|i| if let Paint::Picture { w, h, .. } = i.paint { Some((w, h)) } else { None }).collect();
    assert_eq!(pics, [(200.0, 100.0), (300.0, 150.0)]);
}

#[test]
fn an_icon_s_use_of_a_sprite_elsewhere_in_the_page_carries_what_it_uses() {
    let mut p = page("<body><svg style='display:none'><defs><linearGradient id='g'><stop offset='0' stop-color='red'/></linearGradient></defs>\
                      <symbol id='star' viewBox='0 0 10 10'><path d='M0 0h10v10z' fill='url(#g)'/></symbol></svg>\
                      <svg width='20' height='20'><use href='#star'/></svg><svg><use href='#nowhere'/></svg></body>");
    let mut texts = Vec::new();
    while let Some((_, _, _, inline)) = p.next_wanted() {
        if let Some(b) = inline {
            texts.push(String::from_utf8(b).unwrap());
        }
    }
    // The icon: the symbol it uses, and the gradient the symbol uses, put in a <defs> of its own.
    let icon = texts.iter().find(|t| t.contains("#star")).unwrap();
    assert!(icon.contains("<defs><symbol id=\"star\""), "{icon}");
    assert!(icon.to_ascii_lowercase().contains("<lineargradient id=\"g\""), "{icon}");
    // What isn't in the page: nothing put in.
    let lost = texts.iter().find(|t| t.contains("#nowhere")).unwrap();
    assert!(!lost.contains("<defs>"), "{lost}");
}

#[test]
fn timing_functions_and_an_animation_s_progress_as_css_has_them() {
    use crate::animate::{Easing, Timing};
    use crate::css::tokens;
    let e = |v: &str| Easing::parse(&tokens(v)).unwrap();
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
    assert!(near(e("linear").at(0.3), 0.3));
    assert!(near(e("ease").at(0.5), 0.8024));
    assert!(near(e("cubic-bezier(0, 1, 1, 0)").at(0.5), 0.5));
    assert_eq!([0.0, 0.49, 0.5, 0.99, 1.0].map(|p| e("steps(2, jump-end)").at(p)), [0.0, 0.0, 0.5, 0.5, 1.0]);
    assert_eq!([0.0, 0.5, 1.0].map(|p| e("steps(2, start)").at(p)), [0.5, 1.0, 1.0]);
    assert_eq!([0.0, 0.5, 1.0].map(|p| e("steps(3, jump-none)").at(p)), [0.0, 0.5, 1.0]);
    assert!(near(e("linear(0, 0.25 75%, 1)").at(0.375), 0.125) && near(e("linear(0, 0.25 75%, 1)").at(0.875), 0.625));
    let t = |delay: f32, count: f32, direction: u8, fill: u8, paused: bool| Timing { duration: 4.0, delay, count, direction, fill, paused };
    // Running: a second into a 4s animation; delayed, with no fill backwards: no effect yet.
    assert_eq!(t(0.0, 1.0, 0, 0, false).progress(1.0), Some(0.25));
    assert_eq!(t(2.0, 1.0, 0, 0, false).progress(1.0), None);
    assert_eq!(t(2.0, 1.0, 0, 2, false).progress(1.0), Some(0.0));
    // Over: its end with fill forwards (two and a half iterations alternating: halfway, going back); else none.
    assert_eq!(t(0.0, 1.0, 0, 1, false).progress(9.0), Some(1.0));
    assert_eq!(t(0.0, 1.0, 0, 0, false).progress(9.0), None);
    assert_eq!(t(0.0, 2.5, 2, 1, false).progress(99.0), Some(0.5));
    assert_eq!(t(0.0, 2.0, 2, 1, false).progress(99.0), Some(0.0));
    // Paused, at a negative delay: as far in as the delay says, whatever the page's time.
    assert_eq!(t(-1.0, 1.0, 0, 0, true).progress(30.0), Some(0.25));
    assert_eq!(t(-1.0, f32::INFINITY, 1, 0, false).progress(2.0), Some(0.25));
}

#[test]
fn values_between_keyframes_mixed_part_by_part() {
    use crate::animate::interpolate;
    use crate::css::tokens;
    let mix = |a: &str, b: &str, p: f32| interpolate(&tokens(a), &tokens(b), p).map(|t| crate::css::serialize(&t));
    assert_eq!(mix("0", "1", 0.25).as_deref(), Some("0.25"));
    assert_eq!(mix("10px 20px", "30px 40px", 0.5).as_deref(), Some("20px 30px"));
    assert_eq!(mix("rotate(0deg) translateX(10px)", "rotate(90deg) translateX(30px)", 0.5).as_deref(), Some("rotate(45deg) translatex(20px)"));
    assert_eq!(mix("none", "translate(10px, 20px) scale(3)", 0.5).as_deref(), Some("translate(5px, 10px) scale(2)"));
    assert_eq!(mix("brightness(0%)", "none", 0.5).as_deref(), Some("brightness(0.5)"));
    assert_eq!(mix("circle(50% at 50% 50%)", "circle(20% at 20% 20%)", 0.5).as_deref(), Some("circle(35% at 35% 35%)"));
    assert_eq!(mix("red", "rgba(0, 0, 255, 0)", 0.5).as_deref(), Some("rgba(255,0,0,0.5)"));
    assert_eq!(mix("path('M0 0 L10 10')", "path('M10 10 L30 30')", 0.5).as_deref(), Some("path(\"M5 5 L20 20\")"));
    // Different units: calc(); what can't be mixed: None (changed halfway by the caller).
    assert!(mix("10px", "50%", 0.5).is_some_and(|v| v.starts_with("calc(")));
    assert_eq!(mix("block", "none", 0.5), None);
    assert_eq!(mix("rotate(10deg)", "scale(2)", 0.5), None);
}

#[test]
fn an_animation_drawn_at_the_page_s_time() {
    let html = "<style>@keyframes fade { from { opacity: 0 } to { opacity: 1 } } \
                @keyframes slide { to { transform: translateX(100px) } }</style>\
                <div id=a style='animation: fade 4s linear'>a</div>\
                <div id=b style='animation: fade 4s linear -1s paused'>b</div>\
                <div id=c style='animation: slide 2s linear forwards'>c</div>\
                <div id=d style='animation: fade 4s linear; opacity: 0.2 !important'>d</div>";
    let ty = Type::builtin();
    let mut p = page(html);
    p.run(1000.0);
    p.lay_out(&ty, 600.0, 400.0, 1.0);
    let style = |p: &Page, id: &str| {
        let n = p.doc.descendants(0).find(|&n| p.doc.element(n).and_then(|e| e.id()) == Some(id)).unwrap();
        p.style_of(n).unwrap()
    };
    // A second in: a quarter of the way; the paused one where its delay put it; important declarations stand.
    assert!((style(&p, "a").opacity - 0.25).abs() < 1e-3);
    assert!((style(&p, "b").opacity - 0.25).abs() < 1e-3);
    assert!((style(&p, "d").opacity - 0.2).abs() < 1e-3);
    assert!(p.animations_running());
    // Later: over; the one that fills forwards where it ended, the others as they are.
    p.run(5000.0);
    p.lay_out(&ty, 600.0, 400.0, 1.0);
    assert_eq!(style(&p, "a").opacity, 1.0);
    assert!(style(&p, "c").transform.is_some());
    assert!((style(&p, "b").opacity - 0.25).abs() < 1e-3);
    assert!(!p.animations_running());
}

fn scripted(html: &str) -> Page {
    Page::with_scripts(Url::parse("https://example.com/a/page").unwrap(), html, 0.0, (800.0, 600.0, 1.0), 1)
}

#[test]
fn a_script_s_animation_runs_on_the_page_s_clock_and_holds_its_end() {
    let mut p = scripted(
        "<div id=a style='opacity:0'>x</div><div id=b>y</div><script>\
         var an = a.animate([{opacity: 0}, {opacity: 1}], {duration: 1000, fill: 'forwards'});\
         var log = [];\
         an.finished.then(function (x) { log.push('finished ' + (x === an)); });\
         an.onfinish = function () { log.push('onfinish ' + an.playState); };\
         var bn = b.animate({transform: ['translateX(0px)', 'translateX(100px)']}, 1000);\
         bn.pause(); bn.currentTime = 250;\
         setTimeout(function () { console.log(log.join(', '), an.playState); }, 1250);\
         console.log(an.playState, document.getAnimations().length, typeof document.timeline.currentTime);</script>",
    );
    let ty = Type::builtin();
    let style = |p: &Page, id: &str| {
        let n = p.doc.descendants(0).find(|&n| p.doc.element(n).and_then(|e| e.id()) == Some(id)).unwrap();
        p.style_of(n).unwrap()
    };
    p.run(0.0);
    assert_eq!(p.take_console(), ["running 2 number"]);
    p.run(500.0);
    p.lay_out(&ty, 600.0, 400.0, 1.0);
    assert!((style(&p, "a").opacity - 0.5).abs() < 1e-3, "halfway");
    assert!(p.animations_running());
    // The paused one: a quarter of the way, held there.
    assert_eq!(style(&p, "b").translate, (crate::values::Len::px(25.0), crate::values::Len::ZERO));
    p.run(1200.0);
    p.lay_out(&ty, 600.0, 400.0, 1.0);
    assert_eq!(style(&p, "a").opacity, 1.0, "its end held (fill: forwards), though its own style says 0");
    p.run(1300.0);
    assert_eq!(p.take_console(), ["onfinish finished, finished true finished"]);
}

#[test]
fn weak_maps_and_maps_keep_working_as_the_collector_frees_and_entries_go() {
    // (YouTube's scripts: a WeakMap whose keys mostly went once the collector ran, which stopped the engine.)
    let mut p = scripted(
        "<body><script>var w = new WeakMap(), s = new WeakSet(), kept = [];\
         for (var i = 0; i < 5000; i++) { var k = {i: i}; w.set(k, i); s.add(k); if (i % 100 === 0) kept.push(k); }\
         var junk = []; for (var j = 0; j < 300000; j++) junk.push({j: j}); junk = null;\
         for (var j = 0; j < 300000; j++) ({j: j});\
         var ok = kept.every(function (k) { return w.get(k) === k.i && w.has(k) && s.has(k); });\
         var fresh = {}; w.set(fresh, 'new'); w.delete(kept[0]);\
         var m = new Map(); for (var i = 0; i < 100; i++) m.set('k' + i, i); for (var i = 0; i < 100; i++) m.delete('k' + i);\
         m.set('k5', 'again'); m.set('other', 1);\
         console.log(ok, w.get(fresh), w.has(kept[0]), w.get(kept[1]), m.get('k5'), m.get('k6'), m.size, m.has('other'));</script></body>",
    );
    p.run(0.0);
    assert_eq!(p.take_console(), ["true new false 100 again undefined 2 true"]);
}

#[test]
fn a_first_blocks_top_margin_collapses_through_white_space_and_empty_blocks_without_being_lost() {
    // (YouTube's page under its masthead: its margin-top came after an empty block, and was lost.)
    for (inside, top) in [
        ("\n  <div class=m>x</div>", 56.0),
        ("\n<div></div>\n<div class=m>x</div>", 56.0),
        ("<div style='margin-bottom: 30px'></div><div class=m>x</div>", 56.0),
        ("<div style='margin-bottom: 70px'></div> <div class=m>x</div>", 70.0),
    ] {
        let mut p = page(&format!(
            "<style>body {{ margin: 0 }} #app {{ position: absolute; top: 0; left: 0; width: 400px }} .m {{ margin-top: 56px; \
             height: 10px; background: #123456 }}</style><body><div id=app><div>{inside}</div></div></body>"
        ));
        let l = laid(&mut p);
        let r = rects(l, 0xFF12_3456);
        assert_eq!(r.first().map(|r| r.1), Some(top), "{inside:?}");
    }
}

#[test]
fn the_bodys_overflow_is_the_pages_so_a_body_of_no_height_doesnt_hide_it() {
    // (YouTube's body: overflow-y: scroll, its app positioned in it, so the body itself has no height.)
    let mut p = page(
        "<style>body { margin: 0; overflow-y: scroll } #app { position: absolute; top: 0; left: 0; width: 200px; height: 100px; \
         background: #123456 } #in { overflow: hidden; height: 0 } #in div { height: 50px; background: #654321 }</style>\
         <body><div id=app></div><div id=in><div></div></div></body>",
    );
    let l = laid(&mut p);
    let clip_of = |color: u32| {
        l.items.iter().find(|i| matches!(i.paint, Paint::Rect { color: c, .. } if c == color)).map(|i| (i.clip.w, i.clip.h))
    };
    let (w, h) = clip_of(0xFF12_3456).expect("the app's box");
    assert!(w >= 200.0 && h >= 100.0, "the app is drawn whole, not clipped by the body: {w}x{h}");
    let (w, h) = clip_of(0xFF65_4321).expect("the box in the one of no height");
    assert!(w * h < 1.0, "a box of no height that hides its overflow still hides it: {w}x{h}");
}

#[test]
fn custom_elements_in_an_import_or_a_clone_are_upgraded_before_it_comes_back() {
    // (Polymer stamps its templates with importNode, then sets the properties of the custom elements in the copy:
    // YouTube's dom-if needed its constructor run by then. In a template's own contents, nothing is upgraded. What
    // innerHTML makes is upgraded before it returns, as Chrome does, in the page or not.)
    let mut p = scripted(
        "<body><template id=t><div><x-box></x-box></div></template><script>\
         class XBox extends HTMLElement { constructor() { super(); this.made = true; } get v() { return this.made ? 'ok' : 'none'; } }\
         customElements.define('x-box', XBox);\
         var t = document.getElementById('t');\
         var a = document.importNode(t.content, true).querySelector('x-box');\
         var b = t.content.cloneNode(true).querySelector('x-box');\
         var c = document.createElement('div'); c.appendChild(document.createElement('x-box')); var d = c.cloneNode(true).firstChild;\
         var e = document.createElement('div'); e.innerHTML = '<x-box></x-box>'; var f = e.firstChild;\
         console.log(a instanceof XBox, a.v, b instanceof XBox, b.made, d instanceof XBox, d.v, f instanceof XBox, f.made);</script></body>",
    );
    p.run(0.0);
    assert_eq!(p.take_console(), ["true ok false undefined true ok true true"]);
}

#[test]
fn a_custom_element_is_constructed_once() {
    // (Made by createElement or its own constructor, it isn't constructed again when it's put in the page: YouTube's
    // page manager hid its watch page before attaching it, and a second construction forgot that.)
    let mut p = scripted(
        "<body><script>var made = 0;\
         class XEl extends HTMLElement { constructor() { super(); made++; } connectedCallback() { console.log('connected', made); } }\
         customElements.define('x-el', XEl);\
         var a = document.createElement('x-el'); a.hidden = true; document.body.appendChild(a);\
         var b = new XEl(); document.body.appendChild(b);\
         console.log(made, a.hidden);</script></body>",
    );
    p.run(0.0);
    assert_eq!(p.take_console(), ["connected 1", "connected 2", "2 true"]);
}

#[test]
fn a_popover_shows_only_once_shown() {
    let mut p = scripted(
        "<body style='margin:0'><div id=a popover style='width:50px;height:20px'>tip</div><p>text</p><script>\
         var a = document.getElementById('a'); a.addEventListener('toggle', function (e) { console.log(e.newState); });\
         </script></body>",
    );
    p.run(0.0);
    let ty = Type::builtin();
    let a = p.doc.descendants(0).find(|&n| p.doc.element(n).is_some_and(|e| e.attr("id") == Some("a"))).unwrap();
    let laid = p.lay_out(&ty, 400.0, 300.0, 1.0);
    assert!(laid.boxes.iter().all(|(n, _)| *n != a), "a popover isn't shown until a script shows it");
    p.run_text("a.showPopover(); console.log(a.matches(':popover-open'));");
    let laid = p.lay_out(&ty, 400.0, 300.0, 1.0);
    let r = laid.boxes.iter().find(|(n, _)| *n == a).map(|(_, r)| *r).expect("shown");
    // (In the middle of the window, over the page.)
    assert!((r.x + r.w / 2.0 - 200.0).abs() < 2.0 && (r.y + r.h / 2.0 - 150.0).abs() < 2.0, "{r:?}");
    p.run_text("a.togglePopover();");
    assert_eq!(p.take_console(), ["open", "true", "closed"]);
}

#[test]
fn scripts_change_the_page() {
    let mut p = scripted(
        "<body><p id=a>x</p><script>document.getElementById('a').textContent = 'y';\
         setTimeout(function () { document.body.appendChild(document.createElement('hr')); }, 10);\
         console.log(document.querySelectorAll('p').length, location.pathname);</script></body>",
    );
    p.run(0.0);
    let a = p.doc.find("p").unwrap();
    assert_eq!(p.doc.text_of(a), "y");
    assert!(p.doc.find("hr").is_none());
    assert_eq!(p.take_console(), ["1 /a/page"]);
    p.run(20.0);
    assert!(p.doc.find("hr").is_some(), "the timer ran");
}

#[test]
fn scripts_see_clicks_and_can_cancel_them() {
    let mut p = scripted(
        "<body><a id=l href=/x>link</a><script>var n = 0; document.getElementById('l').addEventListener('click', \
         function (e) { n++; e.preventDefault(); document.title = 'clicked ' + n; });</script></body>",
    );
    p.run(0.0);
    let a = p.doc.find("a").unwrap();
    assert!(!p.click(Some(a), 5.0, 5.0), "the script cancelled the click");
    assert_eq!(p.title(), "clicked 1");
}

#[test]
fn scripts_run_in_order_as_they_come() {
    let mut p = scripted("<head><script src=/one.js></script><script>console.log('two')</script></head><body></body>");
    let (i, kind, url, _) = p.next_wanted().unwrap();
    assert_eq!(kind, crate::page::Kind::Script);
    assert_eq!(url.unwrap().path, "/one.js");
    p.run(0.0);
    assert!(p.take_console().is_empty(), "the second waits for the first");
    p.script_came(i, String::from("console.log('one')"));
    p.run(1.0);
    assert_eq!(p.take_console(), ["one", "two"]);
}

#[test]
fn scripts_fetch_through_the_page() {
    let mut p = scripted("<body><script>fetch('/d.json').then(function (r) { return r.json(); }).then(function (j) { console.log('got ' + j.a); });</script></body>");
    p.run(0.0);
    let reqs = p.take_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].1, "https://example.com/d.json");
    let r = crate::script::Response { status: 200, status_text: String::new(), url: reqs[0].1.clone(), headers: Vec::new(), body: b"{\"a\": 5}".to_vec() };
    p.answer(reqs[0].0, Ok(r));
    assert_eq!(p.take_console(), ["got 5"]);
}

#[test]
fn web_sockets_open_send_receive_and_close_through_the_page() {
    use crate::script::{SocketEvent, SocketOp};
    let mut p = scripted(
        "<body><script>\
         try { new WebSocket('ftp://example.com/'); } catch (e) { console.log(e.name + ' for ftp'); }\
         try { new WebSocket('wss://example.com/#x'); } catch (e) { console.log(e.name + ' for a fragment'); }\
         try { new WebSocket('/s', ['a', 'a']); } catch (e) { console.log(e.name + ' for a protocol twice'); }\
         try { new WebSocket('/s', 'a b'); } catch (e) { console.log(e.name + ' for a space'); }\
         var ws = new WebSocket('/chat?room=1', ['chat', 'v2']);\
         console.log(ws.url + ' ' + ws.readyState + ' ' + WebSocket.OPEN + ' ' + ws.CLOSED + ' ' + ws.binaryType);\
         try { ws.send('early'); } catch (e) { console.log(e.name + ' while connecting'); }\
         ws.binaryType = 'nonsense'; ws.binaryType = 'arraybuffer';\
         ws.onopen = function () { console.log('open ' + ws.protocol + ' ' + ws.readyState); ws.send('hi'); ws.send(new Uint8Array([1, 2, 3])); \
           console.log('buffered ' + ws.bufferedAmount); };\
         ws.addEventListener('message', function (e) { console.log('message ' + (typeof e.data === 'string' ? e.data : \
           Array.prototype.join.call(new Uint8Array(e.data), ',')) + ' from ' + e.origin + ' ' + (e instanceof MessageEvent)); });\
         ws.onerror = function () { console.log('error'); };\
         ws.onclose = function (e) { console.log('close ' + e.code + ' ' + e.reason + ' ' + e.wasClean + ' ' + ws.readyState + ' ' + (e instanceof CloseEvent)); };\
         var lost = new WebSocket('wss://nowhere.example/');\
         lost.onerror = function () { console.log('lost: error ' + lost.readyState); };\
         lost.onclose = function (e) { console.log('lost: close ' + e.code + ' ' + e.wasClean); };\
         setTimeout(function () { try { ws.close(1001); } catch (e) { console.log(e.name + ' for 1001'); } ws.close(1000, 'bye'); \
           console.log('closing ' + ws.readyState); ws.send('late'); console.log('buffered ' + ws.bufferedAmount); }, 50);\
         </script></body>",
    );
    p.run(0.0);
    assert_eq!(
        p.take_console(),
        [
            "SyntaxError for ftp",
            "SyntaxError for a fragment",
            "SyntaxError for a protocol twice",
            "SyntaxError for a space",
            "wss://example.com/chat?room=1 0 1 3 blob",
            "InvalidStateError while connecting",
        ]
    );
    let ops = p.take_socket_ops();
    assert_eq!(ops.len(), 2);
    let SocketOp::Open { id, url, protocols } = ops[0].clone() else { panic!("an open first: {ops:?}") };
    assert_eq!((url.as_str(), protocols), ("wss://example.com/chat?room=1", vec![String::from("chat"), String::from("v2")]));
    let SocketOp::Open { id: lost, .. } = ops[1].clone() else { panic!("then the other") };
    p.socket_event(id, SocketEvent::Open(String::from("v2")));
    assert_eq!(p.take_console(), ["open v2 1", "buffered 5"]);
    assert_eq!(
        p.take_socket_ops(),
        [SocketOp::Send { id, binary: false, data: b"hi".to_vec() }, SocketOp::Send { id, binary: true, data: vec![1, 2, 3] }]
    );
    p.socket_event(id, SocketEvent::Text(String::from("hello")));
    p.socket_event(id, SocketEvent::Binary(vec![9, 8]));
    assert_eq!(p.take_console(), ["message hello from wss://example.com true", "message 9,8 from wss://example.com true"]);
    p.socket_event(lost, SocketEvent::Closed(1006, String::new(), false));
    assert_eq!(p.take_console(), ["lost: error 3", "lost: close 1006 false"]);
    // Its script closes it: the closing handshake asked of the system; then it's closed, cleanly.
    p.run(60.0);
    assert_eq!(p.take_console(), ["InvalidAccessError for 1001", "closing 2", "buffered 4"]);
    assert_eq!(p.take_socket_ops(), [SocketOp::Close { id, code: Some(1000), reason: String::from("bye") }]);
    p.socket_event(id, SocketEvent::Text(String::from("after closing")));
    p.socket_event(id, SocketEvent::Closed(1000, String::from("bye"), true));
    assert_eq!(p.take_console(), ["close 1000 bye true 3 true"]);
}

#[test]
fn module_scripts_import_what_the_page_fetches() {
    let mut p = scripted(
        "<head><script type=importmap>{\"imports\": {\"lib\": \"/m/lib.js\"}}</script>\
         <script type=module>import { n, add } from 'lib'; add(); console.log('module sees ' + n + ' at ' + import.meta.url);</script>\
         <script nomodule>console.log('nomodule ran')</script>\
         <script>console.log('classic first')</script></head><body></body>",
    );
    let (i, kind, url, _) = p.next_wanted().unwrap();
    assert_eq!(kind, crate::page::Kind::Module);
    assert_eq!(url.unwrap().path, "/m/lib.js");
    p.run(0.0);
    assert_eq!(p.take_console(), ["classic first"], "the module waits for what it imports");
    p.script_came(i, String::from("export let n = 1; export function add() { n += 1; }"));
    p.run(1.0);
    assert_eq!(p.take_console(), ["module sees 2 at https://example.com/a/page"]);
}

#[test]
fn scripts_see_where_boxes_are_at_once() {
    let mut p = scripted(
        "<head><style>body { margin: 8px } .box { width: 120px; height: 30px; padding: 5px; border: 2px solid red }\
         .gone { display: none }</style></head><body><div id=a class=box>x</div><script>\
         var a = document.getElementById('a');\
         var b = document.createElement('div'); b.style.height = '20px'; document.body.appendChild(b);\
         console.log(a.offsetWidth + ' ' + a.offsetHeight + ' ' + b.getBoundingClientRect().top);\
         a.className = 'gone';\
         console.log(a.offsetWidth + ' ' + getComputedStyle(a).display + ' ' + b.getBoundingClientRect().top);\
         </script></body>",
    );
    p.run(0.0);
    assert_eq!(p.take_console(), ["134 44 52", "0 none 8"], "laid out on the spot, before the page itself is");
}

#[test]
fn a_family_list_gets_the_kind_of_font_it_asks_for() {
    let mut p = page(
        "<p style=\"font-family: -apple-system, 'Segoe UI', Roboto, sans-serif\">sans</p>\
         <p style=\"font-family: 'Unknown Web Font', Georgia, serif\"><b>serif</b></p><p>default</p>\
         <p style=\"font-family: 'Proxima Nova', sans-serif\"><b>webfont</b></p><code>code</code>",
    );
    let l = laid(&mut p);
    let face = |text: &str| {
        l.items
            .iter()
            .find_map(|i| match &i.paint {
                Paint::Text { text: t, face, .. } if t.contains(text) => Some(*face),
                _ => None,
            })
            .unwrap()
    };
    use text::{Face, Family};
    assert_eq!(
        [face("sans"), face("serif"), face("default"), face("webfont"), face("code")],
        [
            Face::Web(Family::Sans, false, false),
            Face::Web(Family::Serif, true, false),
            Face::Web(Family::Serif, false, false),
            Face::Web(Family::Sans, true, false),
            Face::Web(Family::Mono, false, false)
        ]
    );
}

#[test]
fn an_svg_in_the_page_is_a_picture_drawn_from_its_text() {
    let mut p = page(
        "<body style='margin:0; color: #00ff00'><svg viewBox='0 0 24 12' width='48'><path fill='currentColor' d='M0 0h24v12H0z'/></svg>\
         <svg width='10' height='10'><svg width='5' height='5'></svg></svg></body>",
    );
    // (Laid out once: its colour is known, and its picture made again in it.)
    let ty = Type::builtin();
    p.lay_out(&ty, 600.0, 400.0, 1.0);
    let mut svgs = Vec::new();
    while let Some((_, kind, url, inline)) = p.next_wanted() {
        assert_eq!((kind, url), (crate::page::Kind::Picture, None));
        svgs.push(String::from_utf8(inline.unwrap()).unwrap());
    }
    assert!(svgs.iter().any(|s| s.starts_with("<svg color=\"#00ff00\" viewbox=\"0 0 24 12\" width=\"48\">")), "{svgs:?}");
    assert_eq!(svgs.iter().filter(|s| s.contains("width=\"5\"")).count(), 1, "the inner one is part of the outer one's picture");
    let l = p.lay_out(&ty, 600.0, 400.0, 1.0);
    let pics: Vec<(f32, f32)> = l.items.iter().filter_map(|i| if let Paint::Picture { w, h, .. } = i.paint { Some((w, h)) } else { None }).collect();
    assert_eq!(pics, [(48.0, 24.0), (10.0, 10.0)]);
}

#[test]
fn a_canvas_shows_what_its_scripts_draw() {
    let mut p = scripted(
        "<body style='margin:0'><canvas id=c width=40 height=20></canvas><script>\
         var c = document.getElementById('c'), x = c.getContext('2d');\
         x.fillStyle = 'red'; x.fillRect(0, 0, 10, 10);\
         x.fillStyle = 'rgba(0, 0, 255, 0.5)'; x.fillRect(5, 0, 10, 10);\
         x.beginPath(); x.arc(30, 10, 5, 0, Math.PI * 2); x.fillStyle = '#0f0'; x.fill();\
         var d = x.getImageData(0, 0, 40, 20).data, at = function (px, py) { var i = (py * 40 + px) * 4; return [d[i], d[i+1], d[i+2], d[i+3]].join(','); };\
         console.log(at(2, 2), at(7, 2), at(12, 2), at(30, 10), at(20, 15), x.getContext === undefined, c.getContext('2d') === x, c.getContext('webgl'));\
         console.log(x.fillStyle, x.isPointInPath(30, 10), x.isPointInPath(20, 10), c.toDataURL().slice(0, 22));\
         </script></body>",
    );
    p.run(0.0);
    assert_eq!(
        p.take_console(),
        ["255,0,0,255 128,0,128,255 0,0,255,128 0,255,0,255 0,0,0,0 true true null", "#00ff00 true false data:image/png;base64,"]
    );
    // The page shows it: its picture, the canvas's pixels.
    let ty = Type::builtin();
    let l = p.lay_out(&ty, 600.0, 400.0, 1.0);
    let n = l.items.iter().find_map(|i| if let Paint::Picture { n, w, h, .. } = i.paint { Some((n, w, h)) } else { None });
    let (n, w, h) = n.expect("the canvas is a picture");
    assert_eq!((w, h), (40.0, 20.0));
    let shown = p.shown(&crate::paint::NoPictures);
    use crate::paint::Shown;
    let (rgb, pw, ph) = shown.picture(n).expect("its pixels");
    assert_eq!((pw, ph, &rgb[..3]), (40, 20, &[255u8, 0, 0][..]));
    assert_eq!(shown.alpha(n).map(|a| a[2 * 40 + 2]), Some(255));
}

#[test]
fn a_canvas_s_paths_paints_and_pixels() {
    let mut p = scripted(
        "<canvas id=c width=100 height=50></canvas><canvas id=o width=4 height=4></canvas><script>\
         var c = document.getElementById('c'), x = c.getContext('2d');\
         var px = function (a, b) { var d = x.getImageData(a, b, 1, 1).data; return d[0] + ',' + d[1] + ',' + d[2] + ',' + d[3]; };\
         var g = x.createLinearGradient(0, 0, 100, 0); g.addColorStop(0, '#000'); g.addColorStop(1, '#fff');\
         x.fillStyle = g; x.fillRect(0, 0, 100, 10);\
         var grad = px(50, 5);\
         x.save(); x.beginPath(); x.rect(0, 20, 50, 30); x.clip(); x.fillStyle = 'blue'; x.fillRect(0, 20, 100, 30); x.restore();\
         var clipped = px(25, 30) + ' ' + px(75, 30);\
         x.globalCompositeOperation = 'destination-out'; x.fillRect(0, 0, 10, 10); x.globalCompositeOperation = 'source-over';\
         var cleared = px(5, 5);\
         x.translate(60, 30); x.scale(2, 2); x.fillStyle = 'lime'; x.fillRect(0, 0, 5, 5); x.setTransform(1, 0, 0, 1, 0, 0);\
         var moved = px(65, 35) + ' ' + px(71, 35);\
         var o = document.getElementById('o').getContext('2d'); o.fillStyle = '#123456'; o.fillRect(0, 0, 4, 4);\
         x.drawImage(document.getElementById('o'), 90, 40); var drawn = px(91, 41);\
         var id = x.createImageData(2, 1); id.data.set([1, 2, 3, 255, 4, 5, 6, 128]); x.putImageData(id, 0, 49);\
         var put = px(0, 49) + ' ' + px(1, 49);\
         x.font = '20px serif'; var m = x.measureText('Hello');\
         console.log(grad, '|', clipped, '|', cleared, '|', moved, '|', drawn, '|', put, '|', m.width > 30 && m.width < 70, x.font);\
         c.width = 100; console.log(px(25, 30), x.fillStyle, x.getTransform().e);\
         try { x.arc(0, 0, -1, 0, 1); } catch (e) { console.log(e.name); }\
         x.lineWidth = -5; x.globalAlpha = 2; x.lineCap = 'nope'; console.log(x.lineWidth, x.globalAlpha, x.lineCap);\
         </script>",
    );
    p.run(0.0);
    assert_eq!(
        p.take_console(),
        [
            "129,129,129,255 | 0,0,255,255 0,0,0,0 | 0,0,0,0 | 0,255,0,255 0,0,0,0 | 18,52,86,255 | 1,2,3,255 4,5,6,128 | true 20px serif",
            "0,0,0,0 #000000 0",
            "IndexSizeError",
            "1 1 butt"
        ]
    );
}

#[test]
fn a_shadow_tree_is_shown_in_its_host_with_its_own_styles_and_slots() {
    let mut p = scripted(
        "<head><style>.t { color: #ff0000 } x-card { display: block }</style></head><body style='margin:0'>\
         <x-card><span slot=role>Role</span>Bio</x-card><p class=t>page</p><script>\
         customElements.define('x-card', class extends HTMLElement { constructor() { super();\
           this.attachShadow({ mode: 'open' }).innerHTML = '<style>:host { background: #0000ff } .t { color: #00ff00 }\
           .r { color: #00ffff } ::slotted(span) { font-weight: bold }</style>\
           <div class=t>Title</div><div class=r><slot name=role>none</slot></div><div><slot>no bio</slot></div>'; } });\
         </script></body>",
    );
    p.run(0.0);
    let l = laid(&mut p);
    let color_of = |text: &str| {
        l.items
            .iter()
            .find_map(|i| match &i.paint {
                Paint::Text { text: t, color, .. } if t.contains(text) => Some(*color & 0xFF_FFFF),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text} isn't drawn"))
    };
    // The tree's own rule for .t, not the page's; what's slotted inherits from its slot's parent.
    assert_eq!([color_of("Title"), color_of("Role"), color_of("Bio"), color_of("page")], [0x00FF00, 0x00FFFF, 0x000000, 0xFF0000]);
    // :host styles the host.
    assert!(!rects(l, 0xFF0000FF).is_empty(), "the host's background");
    assert!(!l.items.iter().any(|i| matches!(&i.paint, Paint::Text { text, .. } if text.contains("none") || text.contains("no bio"))), "slots given something don't show their own");
}

#[test]
fn css_in_js_rules_templates_and_tree_walkers() {
    let mut p = scripted(
        "<body><p class=a>x</p><script>\
         var tag = document.createElement('style'); document.head.appendChild(tag);\
         tag.sheet.insertRule('.a { color: #0000ff }', tag.sheet.cssRules.length);\
         tag.sheet.insertRule('.b { color: red }', 0); tag.sheet.deleteRule(0);\
         console.log(tag.sheet.cssRules.length, tag.sheet.cssRules[0].selectorText, document.styleSheets[0] === tag.sheet);\
         var t = document.createElement('template'); t.innerHTML = '<b><?mark></b><i>i</i>';\
         var w = document.createTreeWalker(document, 129); w.currentNode = t.content; var seen = [];\
         var n; while ((n = w.nextNode())) seen.push(n.nodeName + (n.data ? ':' + n.data : ''));\
         console.log(t.content.childNodes.length, seen.join(' '));\
         </script></body>",
    );
    p.run(0.0);
    assert_eq!(p.take_console(), ["1 .a true", "2 B #comment:?mark I"]);
    let l = laid(&mut p);
    let blue = l.items.iter().any(|i| matches!(&i.paint, Paint::Text { text, color, .. } if text == "x" && color & 0xFF_FFFF == 0x0000FF));
    assert!(blue, "the rule a script inserted styles the page");
}

#[test]
fn dashed_dotted_and_double_borders_are_drawn_as_their_pieces() {
    // A 100 × 50 box (its border inside it): 2-pixel dashes 6 long, 3-pixel round dots, a 6-pixel double line.
    let mut p = page(
        "<body style='margin:0'><div style='box-sizing:border-box;width:100px;height:50px;border:2px dashed red'></div>\
         <div style='box-sizing:border-box;width:100px;height:50px;border:3px dotted blue'></div>\
         <div style='box-sizing:border-box;width:100px;height:50px;border:6px double green'></div></body>",
    );
    let l = laid(&mut p);
    let dashes: Vec<_> = rects(l, RED).into_iter().filter(|r| r.1 == 0.0 && r.3 == 2.0).collect();
    // (Along the top, 100 long: 8 dashes of 6, a whole one at each end, the gaps between them the same.)
    assert_eq!(dashes.len(), 8, "{dashes:?}");
    assert_eq!((dashes[0].0, dashes[0].2), (0.0, 6.0));
    assert!((dashes[7].0 + dashes[7].2 - 100.0).abs() < 0.01, "{dashes:?}");
    let dots: Vec<_> = l
        .items
        .iter()
        .filter_map(|i| match &i.paint {
            Paint::Rect { y, w, h, color, radius, .. } if *color == BLUE && *y == 50.0 => Some((*w, *h, *radius)),
            _ => None,
        })
        .collect();
    assert!(dots.len() > 10 && dots.iter().all(|d| *d == (3.0, 3.0, [1.5; 4])), "{dots:?}");
    // The double line's top: two lines 2 pixels thick, 2 apart.
    let lines: Vec<_> = rects(l, GREEN).into_iter().filter(|r| r.2 == 100.0).collect();
    assert!(lines.iter().any(|r| r.1 == 100.0 && r.3 == 2.0) && lines.iter().any(|r| r.1 == 104.0 && r.3 == 2.0), "{lines:?}");
}

#[test]
fn tab_goes_through_links_buttons_and_fields_shown() {
    // In the page's order: what Tab goes to, that's shown (not one in a hidden box, a hidden field, a disabled
    // button, or one with tabindex -1); a div a script made focusable; then none after the last.
    let mut p = page(
        "<body><a href='/a'>A link</a> <a>no address</a><div style='display:none'><a href='/hidden'>Hidden</a></div>\
         <input type=hidden name=h><input name=q><button disabled>Off</button><button>On</button>\
         <span tabindex=-1>skipped</span><div tabindex=0>Made focusable</div><select><option>One</select>\
         <input type=checkbox><textarea></textarea></body>",
    );
    laid(&mut p);
    let mut order = Vec::new();
    let mut at = None;
    while let Some(n) = p.next_focusable(at, false) {
        let e = p.doc.element(n).unwrap();
        order.push(format!("{}{}", e.name, e.attr("type").map(|t| format!("[{t}]")).unwrap_or_default()));
        at = Some(n);
        assert!(order.len() < 20);
    }
    assert_eq!(order, ["a", "input", "button", "div", "select", "input[checkbox]", "textarea"]);
    // Back from the first: none; from none, the last.
    let first = p.next_focusable(None, false);
    assert_eq!(p.next_focusable(first, true), None);
    assert_eq!(p.next_focusable(None, true).map(|n| p.doc.name(n).to_string()), Some("textarea".to_string()));
    // A link's box: round its words.
    let b = p.box_of(first.unwrap()).unwrap();
    assert!(b.w > 10.0 && b.h > 5.0, "{b:?}");
    assert!(p.is_text_field(p.next_focusable(first, false).unwrap()));
}

/// The border box (CSS pixels) of the element with id `id`, as the page is laid out `w` wide.
fn box_of(p: &mut Page, id: &str, w: f32) -> crate::layout::Rect {
    let ty = Type::builtin();
    let n = p.doc.descendants(0).find(|&n| p.doc.element(n).is_some_and(|e| e.attr("id") == Some(id))).expect("the element");
    let laid = p.lay_out(&ty, w, 600.0, 1.0);
    laid.boxes.iter().find(|(b, _)| *b == n).map(|(_, r)| *r).expect("its box")
}

#[test]
fn rem_is_the_root_elements_font_size() {
    // (YouTube's, and many sites': html { font-size: 62.5% }, so 1.6rem is 16px, not 25.6.)
    let mut p = page("<html style='font-size:62.5%'><body style='margin:0'><div id=a style='width:10rem;height:2.4rem'></div></body></html>");
    let r = box_of(&mut p, "a", 800.0);
    assert_eq!((r.w, r.h), (100.0, 24.0));
}

#[test]
fn a_stretched_flex_item_is_as_high_as_its_line_before_its_insides_are_laid_out() {
    // (A row 40px high: what stretches is 40 high, and a child 100% high of it is too, as in Chrome; YouTube's
    // search box.)
    let mut p = page(
        "<body style='margin:0'><div style='display:flex;height:40px;width:600px'><div id=w style='width:100%'>\
         <div id=c style='display:flex;height:100%'><div id=tall style='height:60px;width:10px'></div></div></div></div></body>",
    );
    assert_eq!(box_of(&mut p, "w", 800.0).h, 40.0);
    assert_eq!(box_of(&mut p, "c", 800.0).h, 40.0);
}

#[test]
fn fields_are_sized_as_chrome_sizes_them() {
    // A text field a line of its line height high however wide it's made (no shape to keep, as a picture has), 20
    // characters wide by default; a button's box-sizing border-box; Chrome's numbers.
    let mut p = page(
        "<body style='margin:0'><input id=a style='width:100%;font-size:16px;line-height:22px;padding:1px 0;border:0'>\
         <br><input id=b><br><button id=c style='width:64px'>Go</button><br><textarea id=d></textarea></body>",
    );
    let a = box_of(&mut p, "a", 600.0);
    assert_eq!((a.w, a.h), (600.0, 24.0));
    let b = box_of(&mut p, "b", 600.0);
    assert!((b.w - 185.0).abs() < 2.0 && b.h == 21.0, "{b:?}");
    assert_eq!(box_of(&mut p, "c", 600.0).w, 64.0);
    let d = box_of(&mut p, "d", 600.0);
    assert!((d.w - 182.0).abs() < 2.0 && d.h == 36.0, "{d:?}");
}

#[test]
fn normal_line_height_is_the_fonts_own_as_chrome_works_it_out() {
    // (Times New Roman's metric twin at 16px: 18px lines; Arial's 18 too, at 13.333px 15; monospace's medium is
    // 13px, 15px lines, unless it's "monospace, monospace".)
    let mut p = page(
        "<body style='margin:0'><div id=a>serif</div><div id=b style='font-family:Arial'>sans</div>\
         <div id=c style='font-family:Arial;font-size:13.333px'>small</div><pre id=d style='margin:0'>code</pre>\
         <div id=e style='font-family:monospace, monospace'>code</div></body>",
    );
    assert_eq!(box_of(&mut p, "a", 600.0).h, 18.0);
    assert_eq!(box_of(&mut p, "b", 600.0).h, 18.0);
    assert_eq!(box_of(&mut p, "c", 600.0).h, 15.0);
    assert_eq!(box_of(&mut p, "d", 600.0).h, 15.0);
    assert_eq!(box_of(&mut p, "e", 600.0).h, 18.0);
}

#[test]
fn a_flex_or_grid_item_keeps_its_childrens_margins_inside_it() {
    // (A flex or grid item's insides are a formatting context of their own: a child's margins don't collapse through
    // it, as YouTube's feed has them, its nudge's 36px under it.)
    let mut p = page(
        "<body style='margin:0'><div id=f style='display:flex;flex-wrap:wrap'><div id=item>\
         <div id=inner style='height:100px;margin:10px 0 36px'></div></div></div>\
         <div id=g style='display:grid'><div><div style='height:50px;margin:20px 0 30px'></div></div></div>\
         <div id=plain><div style='height:50px;margin:20px 0 30px'></div></div></body>",
    );
    assert_eq!(box_of(&mut p, "item", 600.0).h, 146.0);
    assert_eq!(box_of(&mut p, "inner", 600.0).y, 10.0);
    assert_eq!(box_of(&mut p, "f", 600.0).h, 146.0);
    assert_eq!(box_of(&mut p, "g", 600.0).h, 100.0);
    // (A plain block's child's margins still collapse through it.)
    let plain = box_of(&mut p, "plain", 600.0);
    assert_eq!((plain.y, plain.h), (266.0, 50.0));
}

#[test]
fn each_corner_rounded_its_own_and_scaled_down_to_fit() {
    // (Four values: top left, top right, bottom right, bottom left; a longhand for one corner; and radii too big for
    // their sides all scaled down by the same factor, as CSS says: 40 + 40 on a side of 50 is 25 + 25.)
    let mut p = page(
        "<body style='margin:0'><div style='width:100px;height:50px;background:red;border-radius:1px 2px 3px 4px;border-top-right-radius:5px'></div>\
         <div style='width:100px;height:50px;background:blue;border-radius:40px'></div></body>",
    );
    let l = laid(&mut p);
    let radius_of = |color: u32| {
        l.items.iter().find_map(|i| match &i.paint {
            Paint::Rect { color: c, radius, .. } if *c == color => Some(*radius),
            _ => None,
        })
    };
    assert_eq!(radius_of(RED), Some([1.0, 5.0, 3.0, 4.0]));
    assert_eq!(radius_of(BLUE), Some([25.0; 4]));
}

#[test]
fn a_box_shadow_is_drawn_under_its_box() {
    let mut p = page(
        "<body style='margin:0'><div style='margin:20px;width:100px;height:50px;background:red;border-radius:12px;\
         box-shadow:0 4px 32px 2px rgba(0, 0, 0, 0.1), 0 1px 2px blue'></div></body>",
    );
    let l = laid(&mut p);
    let shadows: Vec<usize> = l.items.iter().enumerate().filter(|(_, i)| matches!(i.paint, Paint::Shadow { .. })).map(|(k, _)| k).collect();
    let background = l.items.iter().position(|i| matches!(i.paint, Paint::Rect { color: RED, .. })).expect("the background");
    assert_eq!(shadows.len(), 2, "both shadows");
    assert!(shadows.iter().all(|&k| k < background), "the shadows first, the box over them");
    // (The first listed is drawn last: on top.)
    let Paint::Shadow { x, y, w, h, radius, dx, dy, blur, spread, color, inset } = l.items[shadows[1]].paint else { unreachable!() };
    assert_eq!((x, y, w, h, radius), (20.0, 20.0, 100.0, 50.0, [12.0; 4]));
    assert_eq!((dx, dy, blur, spread, inset), (0.0, 4.0, 32.0, 2.0, false));
    assert_eq!(color >> 24, 26, "its colour's alpha, 0.1");
    let Paint::Shadow { blur, color, .. } = l.items[shadows[0]].paint else { unreachable!() };
    assert_eq!((blur, color), (2.0, BLUE));
}

#[test]
fn an_inset_shadow_is_drawn_inside_over_the_background() {
    let mut p = page("<body style='margin:0'><div style='width:100px;height:40px;background:red;border:2px solid;box-shadow:inset 0 -2px 0 blue'></div></body>");
    let l = laid(&mut p);
    let background = l.items.iter().position(|i| matches!(i.paint, Paint::Rect { color: RED, .. })).expect("the background");
    let inset = l.items.iter().position(|i| matches!(i.paint, Paint::Shadow { inset: true, .. })).expect("the inset shadow");
    assert!(background < inset);
    let Paint::Shadow { x, y, w, h, dy, .. } = l.items[inset].paint else { unreachable!() };
    assert_eq!((x, y, w, h, dy), (2.0, 2.0, 100.0, 40.0, -2.0), "within the padding box");
}

#[test]
fn a_border_of_one_colour_with_a_side_missing_keeps_its_rounded_corners() {
    // (YouTube's search box: rounded on the left, no border on the right.)
    let mut p = page(
        "<body style='margin:0'><div style='box-sizing:border-box;width:200px;height:40px;border:1px solid red;\
         border-right:0;border-radius:40px 0 0 40px'></div></body>",
    );
    let l = laid(&mut p);
    let borders: Vec<_> = l.items.iter().filter_map(|i| match &i.paint {
        Paint::Border { widths, color, radius, .. } => Some((*widths, *color, *radius)),
        _ => None,
    }).collect();
    assert_eq!(borders, [([1.0, 0.0, 1.0, 1.0], RED, [20.0, 0.0, 0.0, 20.0])]);
}

#[test]
fn a_buttons_words_are_in_the_middle_of_its_height() {
    let mut p = page(
        "<body style='margin:0'><div><button style='padding:0;border:0;background:red'>One</button></div>\
         <div><button style='padding:0;border:0;background:blue;height:40px'>Two</button></div></body>",
    );
    let l = laid(&mut p);
    let (one, two) = (rects(l, RED)[0], rects(l, BLUE)[0]);
    let (y1, y2) = (words(l, "One").1, words(l, "Two").1);
    assert_eq!(two.3, 40.0);
    // (Moved down by whole pixels, as Chrome draws it: 12.5 is 12.)
    assert_eq!(y2 - two.1, y1 - one.1 + crate::num::floor((40.0 - one.3) / 2.0));
}

#[test]
fn an_svg_is_filled_as_the_page_s_style_sheets_say() {
    // (YouTube's icons: fill: currentcolor on the box round the <svg>, the colour blue; and a rule for one path.)
    let mut p = page(
        "<html><head><style>.x { fill: #ff0000 } .round { fill: currentcolor; color: #065fd4 }</style></head>\
         <body><div class=round><svg viewBox='0 0 24 24' width=24><path d='M0 0h24v24H0z'/>\
         <path class=x fill=green d='M0 0h2v2H0z'/><path fill=none d='M0 0h4v4H0z'/></svg></div>\
         <svg width=10 fill=yellow><path d='M0 0h1v1H0z'/></svg></body></html>",
    );
    let ty = Type::builtin();
    p.lay_out(&ty, 600.0, 400.0, 1.0);
    let mut svgs = Vec::new();
    while let Some((_, _, _, inline)) = p.next_wanted() {
        svgs.push(String::from_utf8(inline.unwrap()).unwrap());
    }
    let icon = svgs.iter().find(|s| s.contains("width=\"24\"")).expect("the icon's picture");
    assert!(icon.starts_with("<svg color=\"#065fd4\" viewbox=\"0 0 24 24\" width=\"24\" fill=\"currentColor\">"), "{icon}");
    assert!(icon.contains("<path class=\"x\" d=\"M0 0h2v2H0z\" fill=\"#ff0000\">"), "{icon}");
    assert!(icon.contains("<path fill=\"none\" d=\"M0 0h4v4H0z\">"), "{icon}");
    // (An <svg> the page's style sheets say nothing of: as its own text says.)
    assert!(svgs.iter().any(|s| s.contains("width=\"10\" fill=\"yellow\"")), "{svgs:?}");
}

#[test]
fn custom_properties_in_a_cycle_are_invalid_and_order_doesnt_matter() {
    // (Wikipedia's icons: --size: var(--size, 1rem) on the element itself is a cycle, so var() there takes its
    // fallback, 1rem, not what the element inherits.)
    let mut p = page(
        "<html><head><style>:root { --m: .875rem } body { margin: 0 }\
         .i { --m: var(--m, 1rem); width: calc(var(--m, 1rem) + 4px); height: 5px }\
         .j { --a: var(--b); --b: 10px; width: var(--a); height: 5px }\
         .k { --p: var(--q, 3px); --q: var(--p); width: var(--p, 7px); height: 5px }\
         .l { --z: 30px } .l > div { width: var(--z, 1px); height: 5px }\
         .n { --z: initial; width: var(--z, 9px); height: 5px }</style></head>\
         <body><div class=i id=i></div><div class=j id=j></div><div class=k id=k></div>\
         <div class=l><div id=l></div></div><div class=n id=n></div></body></html>",
    );
    assert_eq!(box_of(&mut p, "i", 600.0).w, 20.0);
    assert_eq!(box_of(&mut p, "j", 600.0).w, 10.0);
    assert_eq!(box_of(&mut p, "k", 600.0).w, 7.0);
    assert_eq!(box_of(&mut p, "l", 600.0).w, 30.0);
    assert_eq!(box_of(&mut p, "n", 600.0).w, 9.0);
}

#[test]
fn grid_tracks_sized_as_css_says_and_areas_across_rows() {
    // (Wikipedia's page: a 59.25rem-at-most column and a min-content one, a column at the end across three rows that
    // doesn't make the title's row taller; Chrome's numbers.)
    let mut p = page(
        "<html><head><style>body { margin: 0 }\
         .w { display: grid; grid-template: min-content min-content min-content 1fr / minmax(0, 59.25rem) min-content;\
         grid-template-areas: 'tcx .' 'title end' 'tool end' 'content end'; column-gap: 24px; width: 1012px }\
         .t { grid-area: title; height: 40px } .o { grid-area: tool; height: 33px } .c { grid-area: content; height: 300px }\
         .e { grid-area: end; width: 196px; margin-top: 44.8px; height: 80px }\
         .g2 { display: grid; grid-template-columns: 100px 1fr 2fr; gap: 10px; width: 600px }\
         .g4 { display: grid; grid-template-columns: repeat(3, 1fr); gap: 5px; width: 400px }</style></head>\
         <body><div class=w><div class=t id=t></div><div class=o id=o></div><div class=c id=c></div><div class=e id=e></div></div>\
         <div class=g2 id=g2><div id=a>a</div><div id=b>b</div><div id=c2>c</div><div>d</div><div id=e2 style='grid-column: 2 / 4'>e</div></div>\
         <div class=g4><div id=tall style='grid-row: span 2; height: 50px'>t</div><div id=b4>b</div><div>c</div><div>d</div>\
         <div id=e4 style='grid-column: span 2'>e</div><div id=f4>f</div></div></body></html>",
    );
    let (t, o, c, e) = (box_of(&mut p, "t", 1100.0), box_of(&mut p, "o", 1100.0), box_of(&mut p, "c", 1100.0), box_of(&mut p, "e", 1100.0));
    assert_eq!((t.x, t.y, t.w, t.h), (0.0, 0.0, 792.0, 40.0));
    assert_eq!((o.y, o.h), (40.0, 33.0));
    assert_eq!(c.y, 73.0);
    assert_eq!((e.x, e.y, e.w), (816.0, 44.8, 196.0));
    // (Fractions of what's left after the fixed column and the gaps: 160 and 320.)
    let (a, b, c2, e2) = (box_of(&mut p, "a", 1100.0), box_of(&mut p, "b", 1100.0), box_of(&mut p, "c2", 1100.0), box_of(&mut p, "e2", 1100.0));
    assert_eq!((a.w, b.x, b.w, c2.x, c2.w), (100.0, 110.0, 160.0, 280.0, 320.0));
    assert_eq!((e2.x, e2.w), (110.0, 490.0));
    // (An item two rows tall; one two columns wide that doesn't fit after the item before it goes on to the next
    // row's start, as Chrome places it, and the next after it.)
    let (tall, b4, e4, f4) = (box_of(&mut p, "tall", 1100.0), box_of(&mut p, "b4", 1100.0), box_of(&mut p, "e4", 1100.0), box_of(&mut p, "f4", 1100.0));
    assert_eq!(tall.h, 50.0);
    assert!((b4.x - 135.0).abs() < 0.01, "{b4:?}");
    assert!(e4.x == 0.0 && (e4.w - 265.0).abs() < 0.01, "{e4:?}");
    assert!((f4.x - 270.0).abs() < 0.01 && f4.y == e4.y, "{f4:?}");
}

#[test]
fn a_positioned_box_among_words_is_where_it_would_have_been() {
    // (One that was inline: where it comes on its line; one that was a block: under the line, if the line has
    // something before it.)
    let mut p = page(
        "<html><head><style>body { margin: 0; font: 16px Arial; line-height: 30px } p { margin: 0; width: 300px }\
         .a { position: absolute; width: 10px; height: 10px }</style></head><body>\
         <p>Hello <span id=s class=a></span>world</p>\
         <div style='width:300px'>Text then <div id=d class=a></div> more</div>\
         <div style='width:300px'><div id=f class=a></div>first</div></body></html>",
    );
    let s = box_of(&mut p, "s", 800.0);
    assert!(s.x > 30.0 && s.y == 0.0, "{s:?}");
    let d = box_of(&mut p, "d", 800.0);
    assert_eq!((d.x, d.y), (0.0, 60.0));
    let f = box_of(&mut p, "f", 800.0);
    assert_eq!((f.x, f.y), (0.0, 60.0));
}

#[test]
fn an_inline_flex_button_sits_on_its_line_as_chrome_puts_it() {
    // (Wikipedia's menu button: a 32px inline-flex label, its icon centred in its min-height; the line no taller
    // than it, as its baseline is made up from its icon's bottom; no space after it counted in its box's width.)
    let mut p = page(
        "<html><head><style>body { margin: 0; font: 16px Arial }\
         .l { display: inline-flex; box-sizing: border-box; min-width: 32px; min-height: 32px; border: 1px solid;\
         justify-content: center; align-items: center } .i { display: block; width: 20px; height: 20px }\
         .d { position: relative; display: inline-block } .c { position: absolute; width: 100%; height: 100%; margin: 0 }\
         </style></head><body><div id=a><label class=l><span id=i class=i></span></label></div>\
         <div><div id=d class=d>\n <input type=checkbox id=c class=c>\n <label class=l><span class=i></span></label>\n</div></div></body></html>",
    );
    assert_eq!(box_of(&mut p, "a", 800.0).h, 32.0);
    let i = box_of(&mut p, "i", 800.0);
    assert_eq!((i.x, i.y), (6.0, 6.0));
    let d = box_of(&mut p, "d", 800.0);
    assert_eq!((d.y, d.w, d.h), (32.0, 32.0, 32.0));
    let c = box_of(&mut p, "c", 800.0);
    assert_eq!((c.x, c.y, c.w), (0.0, 32.0, 32.0));
}

#[test]
fn font_face_rules_are_read_from_a_sheets_text() {
    // (Comments, a data: address with its ";", formats, a range of weights, unicode-range as Google Fonts writes it,
    // and one inside @media.)
    let faces = crate::css::font_faces(
        "/* @font-face { font-family: no } */ @font-face { font-family: 'My Font'; src: url(data:font/woff2;base64,AAAA) format('woff2'),\
         url(\"/f.woff\") format(\"woff\"); font-weight: 300 700; font-style: italic; unicode-range: U+0000-00FF, U+1E00-1EFF, U+4??; }\
         @media print { @font-face { font-family: B; src: url(b.ttf) } }",
    );
    assert_eq!(faces.len(), 2);
    let f = &faces[0];
    assert_eq!(f.family, "my font");
    assert_eq!(f.sources, [(String::from("data:font/woff2;base64,AAAA"), String::from("woff2")), (String::from("/f.woff"), String::from("woff"))]);
    assert_eq!((f.weight, f.italic), ((300, 700), true));
    assert_eq!(f.ranges, [(0, 0xFF), (0x1E00, 0x1EFF), (0x400, 0x4FF)]);
    assert!(f.covers('a' as u32) && !f.covers(0x500));
    assert_eq!((faces[1].family.as_str(), faces[1].weight, faces[1].italic), ("b", (400, 400), false));
}

#[test]
fn a_font_is_chosen_by_family_style_and_nearest_weight() {
    use crate::style::{WebFont, pick_font};
    let face = |w: (u16, u16), italic: bool, ranges: Vec<(u32, u32)>| WebFont {
        face: crate::css::FontFace { family: String::from("f"), sources: Vec::new(), weight: w, italic, ranges, stretch: (100, 100) },
        loaded: None,
    };
    let fonts = [face((300, 300), false, vec![]), face((700, 700), false, vec![]), face((400, 400), true, vec![]), face((400, 400), false, vec![(0x400, 0x4FF)]), face((400, 400), false, vec![(0, 0xFF)])];
    let names = [String::from("other"), String::from("f")];
    // (400: the Latin one of the two for it; 500: up to 500, else lighter; 600: heavier; italic: the italic one.)
    assert_eq!(pick_font(&fonts, &names, 400, false, 100), Some((4, false, false)));
    assert_eq!(pick_font(&fonts, &names, 500, false, 100).map(|p| p.0), Some(4));
    assert_eq!(pick_font(&fonts, &names, 600, false, 100), Some((1, false, false)));
    assert_eq!(pick_font(&fonts, &names, 200, false, 100).map(|p| p.0), Some(0));
    assert_eq!(pick_font(&fonts, &names, 400, true, 100), Some((2, false, false)));
    // (Bold from a face that isn't: made up. Italic with none in the family: leaned.)
    let regular = [face((400, 400), false, vec![])];
    assert_eq!(pick_font(&regular, &names, 700, true, 100), Some((0, true, true)));
    assert_eq!(pick_font(&regular, &[String::from("other")], 400, false, 100), None);
}

#[test]
fn a_web_font_is_fetched_and_the_words_set_in_it() {
    let mut p = page(
        "<html><head><style>@font-face { font-family: Mine; src: url(/mine.woff2) format('woff2'), url(/mine.ttf) format('truetype') }\
         body { margin: 0; font-size: 16px } span { font-family: Mine, sans-serif; display: inline-block }</style></head>\
         <body><div><span id=m>Hamburgefonstiv</span></div><div><span id=c style='font-family: \"Courier New\"'>Hamburgefonstiv</span></div></body></html>",
    );
    let ty = Type::builtin();
    let before = box_of(&mut p, "m", 600.0).w;
    let mut asked = Vec::new();
    while let Some((i, kind, url, _)) = p.next_wanted() {
        if kind == crate::page::Kind::Font {
            asked.push((i, url.map(|u| u.to_string())));
        }
    }
    // (Its first address in a format that's read.)
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].1.as_deref(), Some("https://example.com/mine.woff2"));
    // It comes (the monospace web font, as a site might send Courier New's twin): the words are set in it.
    p.font_came(asked[0].0, include_bytes!("../../fonts/ChatComputerWebMono-Regular.ttf"));
    let after = box_of(&mut p, "m", 600.0).w;
    assert!(after > before, "{before} {after}");
    assert_eq!(after, box_of(&mut p, "c", 600.0).w);
    // A font that can't be read: the words stay as they were.
    let mut q = page("<html><head><style>@font-face { font-family: Bad; src: url(/bad.ttf) } body { margin: 0 } span { font-family: Bad, sans-serif; display: inline-block }</style></head><body><span id=b>Words</span></body></html>");
    let w = box_of(&mut q, "b", 600.0).w;
    while let Some((i, kind, _, _)) = q.next_wanted() {
        if kind == crate::page::Kind::Font {
            q.font_came(i, b"not a font");
        }
    }
    assert_eq!(box_of(&mut q, "b", 600.0).w, w);
    let _ = &ty;
}

#[test]
fn an_element_with_an_id_is_a_global_and_custom_properties_are_worked_out() {
    // (HTML's named properties: main, a form by its name; a global of the scripts' own or the window's comes first.
    // getComputedStyle's custom property: its value worked out, var() replaced; invalid in a cycle.)
    let mut p = scripted(
        "<html><head><style>#t { --a: 10px; --b: var(--a) var(--a); --c: var(--c, 1px) }</style></head><body>\
         <main id=main></main><form name=f></form><div id=alert></div><div id=t></div><script>\
         var mine = 1;\
         console.log([typeof main, main.tagName, typeof f, typeof alert, 'main' in window, typeof nothere, mine]);\
         var cs = getComputedStyle(document.getElementById('t'));\
         console.log(['[' + cs.getPropertyValue('--a') + ']', '[' + cs.getPropertyValue('--b') + ']', '[' + cs.getPropertyValue('--c') + ']']);\
         </script></body></html>",
    );
    p.run(0.0);
    assert_eq!(p.take_console(), [r#"["object", "MAIN", "object", "function", true, "undefined", 1]"#, r#"["[10px]", "[10px 10px]", "[]"]"#]);
}

/// A page laid out `w` × `h` and drawn (its pictures all 2 × 2: red, green; blue, white), its pixels (0xRRGGBB).
fn drawn(p: &mut Page, w: usize, h: usize) -> Vec<u32> {
    struct Checker;
    impl crate::paint::Shown for Checker {
        fn picture(&self, _n: usize) -> Option<(&[u8], u32, u32)> {
            Some((&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255], 2, 2))
        }
    }
    let ty = Type::builtin();
    // (Laid out, what that asks for come, and laid out again: a background's pictures are asked for by its style.)
    for _ in 0..2 {
        while let Some((i, _, _, _)) = p.next_wanted() {
            p.picture_came(i, 2, 2);
        }
        p.lay_out(&ty, w as f32, h as f32, 1.0);
    }
    let laid = p.laid().expect("laid out");
    let mut px = vec![0u32; w * h];
    let mut surface = crate::paint::Surface { pixels: &mut px, width: w, height: h, clip: (0, 0, w as i32, h as i32) };
    crate::paint::Painter::new().draw(&ty, laid, 0, 0, 0.0, &mut surface, &Checker, true);
    px
}

#[test]
fn a_background_picture_is_placed_and_repeated_as_its_properties_say() {
    // A 2 × 2 picture made 10 × 10, from the box's centre, along the x axis only.
    let mut p = page("<body style='margin:0'><div style='width:60px;height:30px;background:url(c.png) repeat-x center / 10px 10px'></div></body>");
    let px = drawn(&mut p, 60, 30);
    let at = |x: usize, y: usize| px[y * 60 + x];
    // (Centred: a tile's top left at (25, 10); its top left red, top right green, smoothly between them.)
    assert_eq!(at(26, 11), 0xFF0000);
    assert_eq!(at(34, 11), 0x00FF00);
    assert_eq!(at(36, 11), 0xFF0000, "repeated across");
    assert_eq!(at(4, 11), 0x00FF00, "and to the left, from where the tiles fall");
    assert_eq!(at(26, 2), 0xFFFFFF, "not down");
}

#[test]
fn gradients_go_as_css_says() {
    let mut p = page("<body style='margin:0'><div style='width:100px;height:20px;background:linear-gradient(to right, black, white)'></div>\
                      <div style='width:40px;height:40px;background:radial-gradient(circle closest-side, red, red 50%, blue 50%)'></div>\
                      <div style='width:40px;height:40px;background:conic-gradient(red 25%, blue 25%)'></div></body>");
    let px = drawn(&mut p, 100, 100);
    let at = |x: usize, y: usize| px[y * 100 + x];
    assert!(at(2, 10) & 0xFF < 0x10 && at(97, 10) & 0xFF > 0xF0, "black to white");
    assert_eq!(at(20, 40), 0xFF0000, "the radial one's middle");
    assert_eq!(at(20, 22), 0x0000FF, "past half its radius");
    assert_eq!(at(30, 66), 0xFF0000, "the conic one: its first quarter round from the top");
    assert_eq!(at(10, 90), 0x0000FF, "and on round");
}

#[test]
fn a_transform_turns_what_is_drawn() {
    // A 40 × 10 bar turned a quarter about its middle: 10 wide and 40 tall.
    let mut p = page("<body style='margin:0'><div style='margin:30px 0 0 10px;width:40px;height:10px;background:#00f;transform:rotate(90deg)'></div></body>");
    let px = drawn(&mut p, 80, 80);
    let at = |x: usize, y: usize| px[y * 80 + x];
    assert_eq!(at(30, 20), 0x0000FF, "above where it was");
    assert_eq!(at(30, 50), 0x0000FF, "and below");
    assert_eq!(at(12, 35), 0xFFFFFF, "its ends are gone from where they were");
}

#[test]
fn opacity_makes_a_group_see_through_as_one() {
    // Two overlapping boxes in a half see-through one: where they overlap, the top one's colour at half, not both.
    let mut p = page("<body style='margin:0'><div style='opacity:.5'><div style='height:20px;background:#f00'></div>\
                      <div style='height:20px;margin-top:-10px;background:#00f'></div></div></body>");
    let px = drawn(&mut p, 20, 40);
    let c = px[15 * 20 + 5];
    let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    assert!((0x78..=0x88).contains(&r) && (0x78..=0x88).contains(&g) && b > 0xF0, "white with blue over it at half (no red under it): {c:06x}");
}

#[test]
fn rounded_clipping_cuts_what_is_inside_to_the_corners() {
    let mut p = page("<body style='margin:0'><div style='width:40px;height:40px;border-radius:20px;overflow:hidden'>\
                      <div style='height:40px;background:#f00'></div></div></body>");
    let px = drawn(&mut p, 40, 40);
    assert_eq!(px[0], 0xFFFFFF, "the corner is left out");
    assert_eq!(px[20 * 40 + 20], 0xFF0000);
}

#[test]
fn positioned_boxes_go_in_their_stacking_context_by_z_index() {
    // The second box's z-index puts it under the first, inside its context; the context's sibling after it covers it.
    let mut p = page("<body style='margin:0'><div style='position:relative;z-index:1'>\
                      <div style='position:absolute;width:20px;height:20px;background:#f00;z-index:2'></div>\
                      <div style='position:absolute;width:20px;height:20px;background:#0f0;z-index:1'></div></div>\
                      <div style='position:relative;z-index:0;width:10px;height:10px;background:#00f'></div></body>");
    let px = drawn(&mut p, 20, 20);
    assert_eq!(px[15 * 20 + 15], 0xFF0000, "the higher z-index on top, in its context");
    assert_eq!(px[5 * 20 + 5], 0xFF0000, "the context (z-index 1) over its sibling (0)");
}

#[test]
fn a_mask_and_a_clip_path_show_only_their_shapes() {
    let mut p = page("<body style='margin:0'><div style='width:40px;height:20px;background:#f00;mask-image:linear-gradient(to right, black 50%, transparent 50%)'></div>\
                      <div style='width:40px;height:40px;background:#00f;clip-path:circle(10px)'></div></body>");
    let px = drawn(&mut p, 40, 60);
    assert_eq!(px[10 * 40 + 10], 0xFF0000);
    assert_eq!(px[10 * 40 + 30], 0xFFFFFF, "masked away");
    assert_eq!(px[40 * 40 + 20], 0x0000FF, "the circle's middle");
    assert_eq!(px[22 * 40 + 2], 0xFFFFFF, "outside it");
}

#[test]
fn borders_of_different_colours_meet_on_the_diagonal() {
    // CSS's arrow: a box of no size, its top border coloured and its sides' transparent.
    let mut p = page("<body style='margin:0'><div style='width:0;height:0;border-left:10px solid transparent;border-right:10px solid transparent;border-top:10px solid #000'></div></body>");
    let px = drawn(&mut p, 20, 10);
    assert_eq!(px[1 * 20 + 10], 0x000000, "the arrow's top middle");
    assert_eq!(px[8 * 20 + 1], 0xFFFFFF, "its sides are cut away on the diagonals");
    assert_eq!(px[8 * 20 + 10] & 0xFF, 0, "its point");
}

#[test]
fn a_line_too_long_ends_in_an_ellipsis_and_a_clamp_keeps_its_lines() {
    let mut p = page("<body style='margin:0'><div style='width:100px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis'>a long line of words to cut</div>\
                      <div id=c style='width:100px;display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2;overflow:hidden'>one two three four five six seven eight nine ten eleven twelve</div></body>");
    let l = laid(&mut p);
    let ells = l.items.iter().filter(|i| matches!(&i.paint, Paint::Text { text, .. } if text == "\u{2026}")).count();
    assert_eq!(ells, 2, "both end in one");
    assert!(!l.items.iter().any(|i| matches!(&i.paint, Paint::Text { text, .. } if text.contains("twelve"))), "the third line isn't drawn");
    let (_, y) = words(l, "one");
    let line = l.items.iter().filter_map(|i| if let Paint::Text { y: ty, .. } = i.paint { Some(ty) } else { None }).fold(0.0f32, f32::max);
    assert!(line - y < 30.0, "two lines: {y} to {line}");
}

#[test]
fn an_animation_leaves_its_box_as_its_last_keyframe_says() {
    let mut p = page("<style>@keyframes in { from { opacity: 0 } to { opacity: 1; background: #00f } }</style>\
                      <body style='margin:0'><div style='opacity:0;height:10px;animation:in 1s ease forwards'></div>\
                      <div style='height:10px;background:#f00;animation:in 1s'></div></body>");
    // (Two seconds in: both over.)
    p.run(2000.0);
    let l = laid(&mut p);
    assert_eq!(rects(l, BLUE).len(), 1, "forwards: its last frame stays");
    assert_eq!(rects(l, RED).len(), 1, "without: it's as it was once it's over");
}

#[test]
fn a_sticky_box_stays_in_view_and_a_fixed_one_with_the_window() {
    use crate::layout::Scroll;
    let s = Scroll::Sticky { top: Some(0.0), bottom: None, y: 100.0, h: 20.0, lo: 0.0, hi: 500.0, view: 300.0 };
    assert_eq!(s.shift(50.0), 0.0, "not yet at the top");
    assert_eq!(s.shift(150.0), 50.0, "held at the top");
    assert_eq!(s.shift(600.0), 380.0, "until its containing block's end");
    assert_eq!(Scroll::Fixed.shift(123.0), 123.0);
}

#[test]
fn an_absolutely_positioned_pseudo_element_is_drawn() {
    let mut p = page("<style>a { position: relative; display: inline-flex; height: 20px } a::after { position: absolute; bottom: 0; left: 0; width: 100%; height: 2px; background: red; content: '' }</style>\
                      <body style='margin:0'><a>tab</a></body>");
    let l = laid(&mut p);
    let r = rects(l, RED);
    assert_eq!(r.len(), 1, "its line under the tab");
    assert_eq!((r[0].1, r[0].3), (18.0, 2.0));
}

#[test]
fn a_font_is_chosen_by_its_width_before_its_style_and_weight() {
    use crate::style::{WebFont, pick_font};
    let face = |stretch: (u16, u16), weight: u16| WebFont {
        face: crate::css::FontFace { family: String::from("f"), sources: Vec::new(), weight: (weight, weight), italic: false, ranges: Vec::new(), stretch },
        loaded: None,
    };
    let fonts = [face((200, 200), 400), face((100, 100), 700), face((75, 75), 400)];
    let names = [String::from("f")];
    assert_eq!(pick_font(&fonts, &names, 400, false, 100).map(|p| p.0), Some(1), "normal width first, whatever its weight");
    assert_eq!(pick_font(&fonts, &names, 400, false, 87).map(|p| p.0), Some(2), "narrower than normal: the nearest narrower");
    assert_eq!(pick_font(&fonts, &names, 400, false, 150).map(|p| p.0), Some(0), "wider: the nearest wider");
    let faces = crate::css::font_faces("@font-face { font-family: f; src: url(a.woff); font-stretch: 75% 125% }");
    assert_eq!(faces[0].stretch, (75, 125));
}

/// What a page asks for, given: each frame's document from `docs` (by its address's path), each picture 100 × 50.
fn fetch_with_frames(p: &mut Page, docs: &[(&str, &str, &[(String, String)])]) -> Vec<(usize, crate::page::Kind, String)> {
    let mut asked = Vec::new();
    let ty = Type::builtin();
    for _ in 0..6 {
        p.lay_out(&ty, 600.0, 400.0, 1.0);
        while let Some((i, kind, url, _)) = p.next_wanted() {
            let path = url.as_ref().map(|u| u.path.clone()).unwrap_or_default();
            asked.push((i, kind, path.clone()));
            match kind {
                crate::page::Kind::Frame => match docs.iter().find(|(d, _, _)| *d == path) {
                    Some((_, html, headers)) => p.frame_came(i, url.unwrap(), html.as_bytes(), "text/html; charset=utf-8", headers),
                    None => p.failed(i),
                },
                crate::page::Kind::Picture => p.picture_came(i, 100, 50),
                _ => p.failed(i),
            }
        }
    }
    asked
}

#[test]
fn a_frame_s_document_is_laid_out_in_its_box_and_drawn_there() {
    let mut p = page("<body style='margin:0'><p style='margin:0;height:20px'>before</p><iframe src='inner.html'></iframe></body>");
    let asked = fetch_with_frames(&mut p, &[("/a/inner.html", "<body style='margin:0;background:#00f'><img src='pic.png'><p style='margin:0'>inside</p></body>", &[])]);
    assert!(asked.iter().any(|(_, k, path)| *k == crate::page::Kind::Frame && path == "/a/inner.html"), "its document fetched: {asked:?}");
    // (Its picture fetched as the page's own: a number of the page's.)
    let (pic, _, _) = asked.iter().find(|(_, k, path)| *k == crate::page::Kind::Picture && path == "/a/pic.png").expect("its picture asked for");
    let ty = Type::builtin();
    let l = p.lay_out(&ty, 600.0, 400.0, 1.0);
    // The frame: 300 × 150 inside its 2px border, under the paragraph; its document's background fills it.
    assert_eq!(rects(l, BLUE), vec![(2.0, 22.0, 300.0, 150.0)]);
    let (x, y) = words(l, "inside");
    assert!(x == 2.0 && y >= 72.0 && y < 100.0, "its words where its document puts them (under its picture), moved to the frame: {x}, {y}");
    let drawn = l.items.iter().find_map(|i| if let Paint::Picture { n, x, y, .. } = i.paint { Some((n, x, y)) } else { None });
    assert_eq!(drawn, Some((*pic, 2.0, 22.0)), "its picture by the page's number for it, in the frame");
}

#[test]
fn a_frame_written_into_its_element_and_one_that_says_no() {
    let refuse: [(String, String); 1] = [(String::from("x-frame-options"), String::from("DENY"))];
    let others: [(String, String); 1] = [(String::from("content-security-policy"), String::from("default-src 'self'; frame-ancestors https://*.example.com"))];
    let mut p = page("<body style='margin:0'><iframe srcdoc='<p>written</p>'></iframe><iframe src='/no.html'></iframe><iframe src='/csp.html'></iframe></body>");
    fetch_with_frames(&mut p, &[("/no.html", "<p>secret</p>", &refuse), ("/csp.html", "<p>allowed</p>", &others)]);
    let ty = Type::builtin();
    let l = p.lay_out(&ty, 1200.0, 400.0, 1.0);
    let (x, _) = words(l, "written");
    assert!(x > 2.0 && x < 20.0, "in the first frame: {x}");
    assert!(!l.items.iter().any(|i| matches!(&i.paint, Paint::Text { text, .. } if text.contains("secret"))), "a document that says DENY isn't shown");
    words(l, "example.com refused to connect.");
    // (www.example.com isn't *.example.com's: example.com itself isn't a subdomain of it.)
    assert!(!l.items.iter().any(|i| matches!(&i.paint, Paint::Text { text, .. } if text.contains("allowed"))), "frame-ancestors that don't name the page");
    let mut q = Page::new(Url::parse("https://www.example.com/").unwrap(), "<iframe src='/csp.html'></iframe>");
    fetch_with_frames(&mut q, &[("/csp.html", "<p>allowed</p>", &others)]);
    words(q.lay_out(&ty, 600.0, 400.0, 1.0), "allowed");
}

#[test]
fn a_frame_s_scripts_run_its_requests_numbered_as_the_page_s_and_its_load_fired() {
    let mut p = scripted("<iframe id=f src='inner.html' onload='console.log(\"frame loaded\")'></iframe>");
    let ty = Type::builtin();
    p.lay_out(&ty, 800.0, 600.0, 1.0);
    let (i, kind, _, _) = p.next_wanted().expect("its document");
    assert_eq!(kind, crate::page::Kind::Frame);
    p.frame_came(i, Url::parse("https://example.com/a/inner.html").unwrap(), b"<script>fetch('/data').then(r => r.text()).then(t => console.log('got ' + t + ' ' + innerWidth))</script>", "text/html", &[]);
    p.run(10.0);
    let reqs = p.take_requests();
    assert_eq!(reqs.len(), 1, "its fetch");
    assert!(reqs[0].0 >= 1 << 30, "numbered apart from the page's own");
    p.answer(reqs[0].0, Ok(crate::script::Response { status: 200, status_text: String::new(), url: String::from("https://example.com/data"), headers: Vec::new(), body: b"hi".to_vec() }));
    for t in [20.0, 30.0, 40.0] {
        p.run(t);
    }
    let console = p.take_console();
    assert!(console.iter().any(|l| l == "frame loaded"), "the <iframe>'s load: {console:?}");
    assert!(console.iter().any(|l| l.ends_with("got hi 300")), "its script, in a window as wide as the frame: {console:?}");
}

#[test]
fn three_d_borders_are_shaded_as_chrome_shades_them() {
    let mut p = page("<body style='margin:0'><div style='border:3px inset black;height:10px'></div><div style='border:3px inset;height:10px'></div>\
                      <div style='border:3px outset #4080c0;height:10px'></div><div style='color:red;border-style:solid;border-width:2px;height:10px'></div></body>");
    let l = laid(&mut p);
    let sided: Vec<[u32; 4]> = l.items.iter().filter_map(|i| if let Paint::SidedBorder { colors, .. } = i.paint { Some(colors) } else { None }).collect();
    assert_eq!(sided[0], [0xFF00_0000, 0xFF54_5454, 0xFF54_5454, 0xFF00_0000], "black: its darker sides black, its lighter ones lightened");
    assert_eq!(sided[1], [0xFF9A_9A9A, 0xFFEE_EEEE, 0xFFEE_EEEE, 0xFF9A_9A9A], "currentcolor: from Chrome's light grey");
    assert_eq!(sided[2], [0xFF40_80C0, 0xFF24_486C, 0xFF24_486C, 0xFF40_80C0], "a colour: darkened where it's shaded");
    let red = l.items.iter().any(|i| matches!(i.paint, Paint::Border { color: 0xFFFF_0000, .. }));
    assert!(red, "a border's colour is the text's where it isn't said");
}

#[test]
fn a_box_aligned_to_the_line_s_top_is_at_its_top_beside_a_tall_picture() {
    let mut p = page("<body style='margin:0'><div id=a style='display:inline-block;vertical-align:top;width:40px;height:30px;margin:10px'></div><img src='p.png' style='width:300px;height:150px'>\
                      <div id=b style='display:inline-block;vertical-align:bottom;width:40px;height:30px'></div></body>");
    let id = |p: &Page, name: &str| p.doc.descendants(0).find(|&n| p.doc.element(n).is_some_and(|e| e.id() == Some(name))).unwrap();
    let (a, b) = (id(&p, "a"), id(&p, "b"));
    let l = laid(&mut p);
    let place = |n: usize| l.boxes.iter().find(|(k, _)| *k == n).map(|(_, r)| (r.x, r.y)).unwrap();
    assert_eq!(place(a).1, 10.0, "at the line's top (and its margin)");
    assert!((place(b).1 + 30.0 - 154.0).abs() < 4.0, "at its bottom: {:?}", place(b));
}

#[test]
fn a_border_image_is_cut_in_nine_and_drawn_round_the_box() {
    let mut p = page("<body style='margin:0'><div style='width:100px;height:60px;border:30px solid red;border-image:url(nine.png) 20 round'></div></body>");
    // (Its picture is asked for once its styles are worked out.)
    laid(&mut p);
    let l = laid(&mut p);
    let slices: Vec<(f32, f32, f32, f32, [f32; 4], bool)> = l
        .items
        .iter()
        .filter_map(|i| if let Paint::Slice { x, y, w, h, src, repeat, .. } = i.paint { Some((x, y, w, h, src, repeat.0 || repeat.1)) } else { None })
        .collect();
    // (The picture is 100 × 50 here: its slices are 20 of it in from each side, its middle not filled.)
    assert_eq!(slices.len(), 8, "four corners and four sides: {slices:?}");
    assert_eq!((slices[0].0, slices[0].1, slices[0].2, slices[0].3), (0.0, 0.0, 30.0, 30.0), "its top left corner");
    assert_eq!(slices[0].4, [0.0, 0.0, 0.2, 0.4]);
    assert!(slices[1].5, "its top side in tiles (round)");
    assert!(!l.items.iter().any(|i| matches!(i.paint, Paint::Border { .. } | Paint::SidedBorder { .. })), "the border's own style isn't drawn");
}

#[test]
fn ex_and_ch_are_the_font_s_own() {
    let mut p = page("<body style='margin:0;font:20px Arial'><div id=a style='width:10ch;height:10ex'></div><div id=b style='font-family:monospace;width:10ch'></div></body>");
    let id = |p: &Page, name: &str| p.doc.descendants(0).find(|&n| p.doc.element(n).is_some_and(|e| e.id() == Some(name))).unwrap();
    let (a, b) = (id(&p, "a"), id(&p, "b"));
    let l = laid(&mut p);
    let rect = |n: usize| l.boxes.iter().find(|(k, _)| *k == n).map(|(_, r)| (r.w, r.h)).unwrap();
    let (w, h) = rect(a);
    assert!((w - 20.0 * 10.0 * 1139.0 / 2048.0).abs() < 0.01 && (h - 20.0 * 10.0 * 1062.0 / 2048.0).abs() < 0.01, "Arial's 0 and x-height: {w} {h}");
    let (w, _) = rect(b);
    assert!((w - 20.0 * 10.0 * 1229.0 / 2048.0).abs() < 0.01, "Courier New's 0 (its size the body's: monospace's 13px is only for the size that isn't said): {w}");
}

#[test]
fn a_block_s_first_letter_takes_its_own_style_with_the_punctuation_round_it() {
    let mut p = page("<style>p::first-letter { color: red; font-size: 32px } div::first-letter { color: red }</style>\
                      <body style='margin:0'><p style='font-size:16px'>\u{201C}Hello,\u{201D} she said.</p><div><div></div>No first letter here</div></body>");
    let l = laid(&mut p);
    let texts: Vec<(String, f32, u32)> = l.items.iter().filter_map(|i| if let Paint::Text { text, size, color, .. } = &i.paint { Some((text.clone(), *size, *color)) } else { None }).collect();
    assert_eq!(texts[0], (String::from("\u{201C}H"), 32.0, RED), "the opening quote and the letter: {texts:?}");
    assert!(texts[1].0.starts_with("ello") && texts[1].2 != RED, "then the rest in the paragraph's own style: {texts:?}");
    assert!(texts.iter().any(|(t, _, c)| t.starts_with("No first") && *c != RED), "a block before the words: they aren't on the first line");
    assert!(crate::first_letter::punctuation(')') && !crate::first_letter::punctuation('a') && !crate::first_letter::punctuation('-'));
    assert_eq!(crate::first_letter::length(" (T)est", false), 4, "the space, the brackets and the letter");
    assert_eq!(crate::first_letter::length("\nThe", true), 0, "a kept line break first: the first line has no letter");
}

#[test]
fn an_outline_is_drawn_round_the_border_box_over_what_comes_after() {
    let mut p = page("<body style='margin:0'><div style='margin:10px;width:20px;height:20px;outline:2px solid red;outline-offset:3px'></div>\
                      <div style='margin:-30px 0 0 0;height:50px;background:blue'></div><div style='outline:2px solid red;outline-offset:inset;width:10px;height:10px'></div></body>");
    let l = laid(&mut p);
    let rings: Vec<(usize, f32, f32, f32, f32)> = l.items.iter().enumerate().filter_map(|(k, i)| if let Paint::Border { x, y, w, h, color: RED, .. } = i.paint { Some((k, x, y, w, h)) } else { None }).collect();
    assert_eq!((rings[0].1, rings[0].2, rings[0].3, rings[0].4), (5.0, 5.0, 30.0, 30.0), "outside its border box by its offset and width");
    let blue = l.items.iter().position(|i| matches!(i.paint, Paint::Rect { color: BLUE, .. })).unwrap();
    assert!(rings[0].0 > blue, "over the block after it");
    assert_eq!((rings[1].3, rings[1].4), (10.0, 10.0), "inset: inside its edge");
}
