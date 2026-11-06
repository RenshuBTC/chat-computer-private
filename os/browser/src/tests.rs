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
            (Picture, Some("/big.png".into()), None),
            (Picture, Some("/lazy.png".into()), None),
        ]
    );
}

fn scripted(html: &str) -> Page {
    Page::with_scripts(Url::parse("https://example.com/a/page").unwrap(), html, 0.0, (800.0, 600.0, 1.0), 1)
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
    let shadow = l.items.iter().position(|i| matches!(i.paint, Paint::Shadow { .. })).expect("a shadow");
    let background = l.items.iter().position(|i| matches!(i.paint, Paint::Rect { color: RED, .. })).expect("the background");
    assert!(shadow < background, "the shadow first, the box over it");
    let Paint::Shadow { x, y, w, h, radius, dx, dy, blur, spread, color } = l.items[shadow].paint else { unreachable!() };
    assert_eq!((x, y, w, h, radius), (20.0, 20.0, 100.0, 50.0, [12.0; 4]));
    assert_eq!((dx, dy, blur, spread), (0.0, 4.0, 32.0, 2.0));
    assert_eq!(color >> 24, 26, "its colour's alpha, 0.1");
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
