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
            (Picture, Some("/big.png".into()), None),
            (Picture, Some("/lazy.png".into()), None),
        ]
    );
}

fn scripted(html: &str) -> Page {
    Page::with_scripts(Url::parse("https://example.com/a/page").unwrap(), html, 0.0, (800.0, 600.0, 1.0), 1)
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
    use text::Face;
    assert_eq!(
        [face("sans"), face("serif"), face("default"), face("webfont"), face("code")],
        [Face::Sans, Face::Bold, Face::Regular, Face::SansBold, Face::Mono]
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
