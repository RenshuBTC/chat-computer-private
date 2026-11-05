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
