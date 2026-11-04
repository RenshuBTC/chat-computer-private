use crate::http::{self, Response};
use crate::page::{self, entities, from_html};
use crate::search;
use crate::url::{query_escape, unescape, Url};
use crate::MOST_BYTES;

#[test]
fn addresses() {
    let u = Url::parse("https://Example.com/a b?q=1#part").unwrap();
    assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("example.com", 443, "/a%20b?q=1"));
    assert_eq!(u.to_string(), "https://example.com/a%20b?q=1");
    assert_eq!(Url::parse("example.com").unwrap().path, "/");
    assert_eq!(Url::parse("example.com?x=1").unwrap().path, "/?x=1");
    let u = Url::parse("http://x.org:8443/p").unwrap();
    assert_eq!((u.port, u.to_string().as_str()), (8443, "https://x.org:8443/p"));
    assert_eq!(Url::parse("//cdn.example.net/f.js").unwrap().host, "cdn.example.net");
    assert_eq!(Url::parse("localhost:8472/x").unwrap().port, 8472);
    assert_eq!(Url::parse("https://www.example.com./").unwrap().host, "www.example.com");
    assert_eq!(Url::parse("https://example.com/caf\u{e9}").unwrap().path, "/caf%C3%A9");
    assert_eq!(Url::parse("https://example.com/%7Euser").unwrap().path, "/%7Euser");
    for bad in ["", "ftp://x.org/f", "file:///C:/x", "javascript:alert(1)", "mailto:a@b.com", "https://user:pw@x.com/",
        "https://[::1]/", "https://ex\u{e4}mple.com/", "https://a..b/", "https://x.org:0/", "https://x.org:99999/"]
    {
        assert!(Url::parse(bad).is_err(), "{bad} wasn't refused");
    }
}

#[test]
fn links_joined() {
    let base = Url::parse("https://a.com/dir/page.html?x=1").unwrap();
    let j = |l: &str| base.join(l).unwrap().to_string();
    assert_eq!(j("other.html"), "https://a.com/dir/other.html");
    assert_eq!(j("../up.html"), "https://a.com/up.html");
    assert_eq!(j("./same/x"), "https://a.com/dir/same/x");
    assert_eq!(j("/root?y=2"), "https://a.com/root?y=2");
    assert_eq!(j("?q=2"), "https://a.com/dir/page.html?q=2");
    assert_eq!(j("https://b.com/x"), "https://b.com/x");
    assert_eq!(j("//c.com/y"), "https://c.com/y");
    assert_eq!(j("../../../../top"), "https://a.com/top");
    assert!(base.join("javascript:void(0)").is_err());
}

#[test]
fn queries() {
    assert_eq!(query_escape("caf\u{e9} & cream-1.5~"), "caf%C3%A9+%26+cream-1.5~");
    assert_eq!(unescape("https%3A%2F%2Fexample.com%2Fa%20b+c"), "https://example.com/a b c");
    assert_eq!(unescape("100%"), "100%");
    assert_eq!(unescape("%E2%9C%93%zz"), "\u{2713}%zz");
    assert_eq!(search::address("rust no_std"), "https://html.duckduckgo.com/html/?q=rust+no_std");
}

#[test]
fn request() {
    let r = String::from_utf8(http::get(&Url::parse("https://example.com/a?b=1").unwrap())).unwrap();
    assert!(r.starts_with("GET /a?b=1 HTTP/1.1\r\nHost: example.com\r\nUser-Agent: "), "{r}");
    assert!(r.contains("\r\nConnection: close\r\n") && r.contains("\r\nAccept-Encoding: gzip, deflate\r\n"), "{r}");
    assert!(r.ends_with("\r\n\r\n"));
    let r = String::from_utf8(http::get(&Url::parse("https://localhost:8472/").unwrap())).unwrap();
    assert!(r.contains("\r\nHost: localhost:8472\r\n"), "{r}");
}

fn fed(pieces: &[&[u8]]) -> Response {
    let mut r = Response::new();
    for p in pieces {
        r.feed(p);
    }
    r
}

#[test]
fn responses() {
    // A length, in pieces that split the head.
    let r = fed(&[b"HTTP/1.1 200 OK\r\nContent-Ty", b"pe: text/html\r\nContent-Length: 5\r\n\r\nhel", b"lo and more"]);
    assert_eq!((r.status(), r.done(), r.body()), (Some(200), true, &b"hello"[..]));
    assert_eq!(r.header("content-type"), Some("text/html"));
    // Chunks, split anywhere, with an extension and trailers.
    let r = fed(&[b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4;x=1\r\nWiki\r\n5\r", b"\npedia\r\n0\r\nX: y\r\n\r\n"]);
    assert_eq!((r.done(), r.body()), (true, &b"Wikipedia"[..]));
    // Until the connection closes; and a body that should have had more.
    let mut r = fed(&[b"HTTP/1.0 200 OK\r\n\r\nall of it"]);
    assert!(!r.done());
    r.closed();
    assert_eq!((r.done(), r.broken(), r.body()), (true, false, &b"all of it"[..]));
    let mut r = fed(&[b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort"]);
    r.closed();
    assert!(r.broken());
    // An interim response, then the real one; a redirect's Location; something that isn't HTTP.
    let r = fed(&[b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 301 Moved\r\nLocation: /new\r\nContent-Length: 0\r\n\r\n"]);
    assert_eq!((r.status(), r.header("location"), r.done()), (Some(301), Some("/new"), true));
    assert!(fed(&[b"SSH-2.0-OpenSSH\r\n\r\n"]).broken());
    // A body past the most kept: cut there, and done.
    let mut r = fed(&[b"HTTP/1.1 200 OK\r\n\r\n"]);
    let big = vec![b'x'; 1 << 20];
    for _ in 0..4 {
        r.feed(&big);
    }
    assert_eq!((r.done(), r.cut(), r.body().len()), (true, true, MOST_BYTES));
}

#[test]
fn compressed() {
    const GZIP: [u8; 53] = [
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0xb3, 0x29, 0xb0, 0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0xd7,
        0x51, 0x48, 0xaf, 0xca, 0x2c, 0x50, 0x50, 0x4b, 0xcc, 0x2d, 0xb0, 0x56, 0x28, 0xc9, 0x48, 0x55, 0x28, 0x4f, 0x4d,
        0xd2, 0xb3, 0xd1, 0x2f, 0xb0, 0x03, 0x00, 0x4f, 0xff, 0x23, 0x30, 0x21, 0x00, 0x00, 0x00,
    ];
    let mut head = b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 53\r\n\r\n".to_vec();
    head.extend_from_slice(&GZIP);
    let r = fed(&[&head]);
    assert_eq!(r.page().unwrap(), b"<p>Hello, gzip &amp; the web.</p>");
    let r = fed(&[b"HTTP/1.1 200 OK\r\nContent-Encoding: br\r\nContent-Length: 2\r\n\r\nxx"]);
    assert!(r.page().is_err());
    let r = fed(&[b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 4\r\n\r\nnope"]);
    assert!(r.page().is_err());
}

#[test]
fn page_words() {
    let html = "<!DOCTYPE html><html><head><title>The  Example &amp; Co</title><style>p{color:red}</style>\
        <script>var a = '<p>not this</p>';</script></head><body><!-- not this either -->\
        <h1>Welcome</h1><p>It&#8217;s a <b>small</b>   page&nbsp;here.<br>Second line &#x2014; ok.</p>\
        <ul><li>One</li><li>Two &lt;three&gt;</li></ul><table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>\
        <img src=x.png alt=\"A cat\"><img src=y.png alt=\"\"><pre>  keep\n  this</pre><noscript>no</noscript>\
        <p>Price: 5 &lt; 6 &unknown; &amp</p></body></html>";
    let p = from_html(html);
    assert_eq!(p.title, "The Example & Co");
    assert_eq!(
        p.text,
        "# Welcome\n\nIt\u{2019}s a small page here.\nSecond line \u{2014} ok.\n\n- One\n- Two <three>\n\na | b\nc | d\n\n\
         [A cat]\n  keep\n  this\n\nPrice: 5 < 6 &unknown; &"
    );
    assert_eq!(entities("&#65;&#x42;&#150;&eacute;&zwnj;x"), "AB\u{2013}\u{e9}x");
}

#[test]
fn kinds_and_character_sets() {
    let p = page::words(b"just text\r\n\r\n\r\nmore  ", "text/plain").unwrap();
    assert_eq!((p.title.as_str(), p.text.as_str()), ("", "just text\n\nmore"));
    assert_eq!(page::words(b"%PDF-1.7", "application/pdf").err().unwrap(), "a PDF");
    assert_eq!(page::words(b"\x89PNG", "image/png").err().unwrap(), "a picture");
    // Latin-1 (Windows-1252), said by the response or by the page.
    let p = page::words(b"caf\xe9 \x93quoted\x94", "text/plain; charset=ISO-8859-1").unwrap();
    assert_eq!(p.text, "caf\u{e9} \u{201c}quoted\u{201d}");
    let p = page::words(b"<meta charset=\"windows-1252\"><p>na\xefve</p>", "text/html").unwrap();
    assert_eq!(p.text, "na\u{ef}ve");
    // UTF-8 (with its mark), and an HTML page served with no type.
    let p = page::words("\u{feff}<p>caf\u{e9}</p>".as_bytes(), "").unwrap();
    assert_eq!(p.text, "caf\u{e9}");
}

#[test]
fn search_results() {
    let page = r#"<html><body>
        <div class="result results_links results_links_deep result--ad">
          <h2 class="result__title"><a rel="nofollow" class="result__a" href="https://duckduckgo.com/y.js?ad_domain=shop.com&amp;u3=x">Buy now</a></h2>
          <a class="result__snippet" href="https://duckduckgo.com/y.js?x">An ad.</a></div>
        <div class="result results_links results_links_deep web-result">
          <h2 class="result__title"><a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2Flearn&amp;rut=abc">Learn <b>Rust</b> - Rust Programming Language</a></h2>
          <div class="result__extras"><a class="result__url" href="//duckduckgo.com/l/?uddg=x">www.rust-lang.org/learn</a></div>
          <a class="result__snippet" href="//duckduckgo.com/l/?uddg=x">A language empowering <b>everyone</b> to build reliable &amp; efficient software.</a>
        </div>
        <div class="result results_links web-result">
          <h2 class="result__title"><a class="result__a" href="https://doc.rust-lang.org/book/">The Book</a></h2>
        </div></body></html>"#;
    let found = search::results(page);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].title, "Learn Rust - Rust Programming Language");
    assert_eq!(found[0].url, "https://www.rust-lang.org/learn");
    assert_eq!(found[0].snippet, "A language empowering everyone to build reliable & efficient software.");
    assert_eq!((found[1].url.as_str(), found[1].snippet.as_str()), ("https://doc.rust-lang.org/book/", ""));
    assert_eq!(
        search::listed(&found),
        "1. Learn Rust - Rust Programming Language\n   https://www.rust-lang.org/learn\n   A language empowering everyone to \
         build reliable & efficient software.\n2. The Book\n   https://doc.rust-lang.org/book/"
    );
    assert!(search::results("<html>no results here</html>").is_empty());
}
