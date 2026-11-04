//! A page's words, as the main Chat Computer reads a page (chatcomputer/brain/builtin.py `_page_text`): its title,
//! then its text, without its scripts, styles, pictures or tags; paragraphs, headings, list items and table rows on
//! lines of their own; a link's words kept (not where it leads); entities (&amp;, &#8217;...) as their characters;
//! spaces run together. The bytes are read as UTF-8 unless the response or the page says Latin-1 (or Windows-1252).

use alloc::string::String;
use alloc::vec::Vec;

/// A page's title (empty if it has none) and words.
pub struct Page {
    pub title: String,
    pub text: String,
}

/// What a response's bytes say, as words: a page (HTML) as its words, plain text and the like as they are. `kind` is
/// the response's Content-Type. Err: what it is, for one that isn't text.
pub fn words(bytes: &[u8], kind: &str) -> Result<Page, String> {
    let kind_l = kind.to_ascii_lowercase();
    let mime = kind_l.split(';').next().unwrap_or("").trim();
    let text = decode(bytes, &kind_l);
    let html = mime.contains("html") || (mime.is_empty() && text.trim_start().starts_with('<'));
    if html {
        return Ok(from_html(&text));
    }
    let textual = mime.is_empty() || mime.starts_with("text/") || mime.contains("json") || mime.contains("xml")
        || mime.contains("javascript");
    if !textual {
        let what = match mime {
            "application/pdf" => String::from("a PDF"),
            m if m.starts_with("image/") => String::from("a picture"),
            m if m.starts_with("video/") => String::from("a video"),
            m if m.starts_with("audio/") => String::from("a sound"),
            m => alloc::format!("a file of another kind ({m})"),
        };
        return Err(what);
    }
    Ok(Page { title: String::new(), text: tidy_lines(&text) })
}

/// The bytes as text: Latin-1 / Windows-1252 if the Content-Type (`kind`, in lower case) or a <meta> near the start
/// says so, else UTF-8 (anything that isn't, as U+FFFD).
fn decode(bytes: &[u8], kind: &str) -> String {
    let head = &bytes[..bytes.len().min(2048)];
    let head = String::from_utf8_lossy(head).to_ascii_lowercase();
    let says = |s: &str| ["iso-8859-1", "windows-1252", "latin1", "latin-1", "us-ascii", "cp1252"].iter().any(|c| s.contains(&alloc::format!("charset={c}")) || s.contains(&alloc::format!("charset=\"{c}\"")));
    if says(kind) || (!kind.contains("charset=") && says(&head)) {
        return bytes.iter().map(|&b| windows_1252(b)).collect();
    }
    String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)).into_owned()
}

/// A byte of Windows-1252 (Latin-1, with printable characters where Latin-1 has controls, 0x80 to 0x9F).
fn windows_1252(b: u8) -> char {
    const HIGH: [u16; 32] = [
        0x20AC, 0xFFFD, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0xFFFD,
        0x017D, 0xFFFD, 0xFFFD, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A,
        0x0153, 0xFFFD, 0x017E, 0x0178,
    ];
    match b {
        0x80..=0x9F => char::from_u32(HIGH[(b - 0x80) as usize] as u32).unwrap_or('\u{FFFD}'),
        _ => b as char,
    }
}

/// Elements whose insides are dropped whole.
const DROPPED: [&str; 9] = ["script", "style", "noscript", "svg", "template", "iframe", "object", "canvas", "math"];
/// Elements that start and end a line of their own.
const BLOCKS: [&str; 32] = [
    "p", "div", "li", "h1", "h2", "h3", "h4", "h5", "h6", "tr", "section", "article", "header", "footer", "nav", "aside",
    "main", "ul", "ol", "table", "blockquote", "pre", "dl", "dt", "dd", "figure", "figcaption", "form", "address",
    "details", "summary", "caption",
];

/// A page's title and words, from its HTML.
pub fn from_html(html: &str) -> Page {
    let b = html.as_bytes();
    let mut out = String::with_capacity(html.len() / 3);
    let mut title = String::new();
    let mut i = 0;
    let mut in_title = false;
    let mut pre = 0u32;
    while i < b.len() {
        if b[i] != b'<' {
            let end = b[i..].iter().position(|&c| c == b'<').map_or(b.len(), |n| i + n);
            let raw = &html[i..end];
            if in_title {
                title.push_str(raw);
            } else {
                push_text(&mut out, raw, pre > 0);
            }
            i = end;
            continue;
        }
        // A comment, a doctype, a processing instruction, or CDATA.
        if html[i..].starts_with("<!--") {
            i = html[i + 4..].find("-->").map_or(b.len(), |n| i + 4 + n + 3);
            continue;
        }
        if html[i..].starts_with("<!") || html[i..].starts_with("<?") {
            i = html[i..].find('>').map_or(b.len(), |n| i + n + 1);
            continue;
        }
        // A tag: its name, whether it closes, and where it ends (a quoted ">" inside it doesn't end it).
        let start = i;
        let closing = b.get(i + 1) == Some(&b'/');
        let name_at = i + 1 + closing as usize;
        let name_len = b[name_at..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'-').count();
        if name_len == 0 {
            // ("<" not starting a tag is a "<".)
            push_text(&mut out, "<", pre > 0);
            i += 1;
            continue;
        }
        let name = html[name_at..name_at + name_len].to_ascii_lowercase();
        let end = tag_end(b, name_at + name_len);
        i = end;
        if !closing && DROPPED.contains(&name.as_str()) {
            // Everything up to its end tag, whatever is inside.
            let close = alloc::format!("</{name}");
            let rest = html[i..].to_ascii_lowercase();
            i = rest.find(&close).map_or(b.len(), |n| tag_end(b, i + n + close.len()));
            continue;
        }
        match name.as_str() {
            "title" => in_title = !closing && title.is_empty(),
            "head" if closing => in_title = false,
            "br" => push_break(&mut out, 1),
            "hr" => push_break(&mut out, 2),
            "pre" => {
                pre = if closing { pre.saturating_sub(1) } else { pre + 1 };
                push_break(&mut out, 1);
            }
            "li" if !closing => {
                push_break(&mut out, 1);
                out.push_str("- ");
            }
            "td" | "th" if !closing => {
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push_str(" | ");
                }
            }
            "img" => {
                // A picture's description, where it has one.
                if let Some(alt) = attribute(&html[start..end], "alt").filter(|a| !a.trim().is_empty()) {
                    push_text(&mut out, &alloc::format!(" [{}] ", alt.trim()), false);
                }
            }
            n if n.len() == 2 && n.starts_with('h') && n.as_bytes()[1].is_ascii_digit() => {
                push_break(&mut out, 2);
                if !closing {
                    for _ in 0..(n.as_bytes()[1] - b'0').min(4) {
                        out.push('#');
                    }
                    out.push(' ');
                }
            }
            n if BLOCKS.contains(&n) => push_break(&mut out, if matches!(n, "p" | "table" | "blockquote" | "ul" | "ol") { 2 } else { 1 }),
            _ => {}
        }
    }
    let title = collapse(&entities(&title));
    Page { title, text: tidy_lines(&out) }
}

/// Where the tag whose name ends at `from` ends: just past its ">", a ">" inside quotes not counting.
fn tag_end(b: &[u8], from: usize) -> usize {
    let mut quote = 0u8;
    let mut i = from;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// An attribute's value in a tag's text (`<img src=... alt="...">`), its entities read.
pub fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name) {
        let at = from + at;
        from = at + name.len();
        let before = lower.as_bytes().get(at.wrapping_sub(1)).copied().unwrap_or(b' ');
        if !(before.is_ascii_whitespace() || before == b'"' || before == b'\'') {
            continue;
        }
        let rest = lower[from..].trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        let value_at = tag.len() - rest.len() + 1;
        let value = tag[value_at..].trim_start();
        let v = match value.as_bytes().first() {
            Some(&q) if q == b'"' || q == b'\'' => value[1..].split(q as char).next().unwrap_or(""),
            _ => value.split(|c: char| c.is_ascii_whitespace() || c == '>').next().unwrap_or(""),
        };
        return Some(entities(v));
    }
    None
}

/// Text from between tags: entities read, spaces run together (unless it's preformatted).
fn push_text(out: &mut String, raw: &str, pre: bool) {
    let text = entities(raw);
    if pre {
        out.push_str(&text);
        return;
    }
    for c in text.chars() {
        let space = c.is_whitespace();
        if space {
            if !out.is_empty() && !out.ends_with([' ', '\n']) {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
}

/// A line break (or a blank line, `n` = 2) unless there's one already.
fn push_break(out: &mut String, n: usize) {
    while out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() {
        return;
    }
    let have = out.chars().rev().take_while(|&c| c == '\n').count();
    for _ in have..n {
        out.push('\n');
    }
}

/// Spaces at lines' ends taken off, at most one blank line in a row, none at the start or end.
fn tidy_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.split('\n') {
        let line = line.trim_end_matches(['\r', ' ', '\t']);
        if line.trim().is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(line);
    }
    out
}

/// Spaces (and line breaks) run together, the ends trimmed.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Entities read: &name; (the common ones), &#123; and &#x7B;. One that isn't known stays as it is.
pub fn entities(s: &str) -> String {
    if !s.contains('&') {
        return String::from(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = Some(rest[1..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '#')).map_or(rest.len(), |n| n + 1));
        let read = end.and_then(|e| {
            let name = &rest[1..e];
            let c = if let Some(num) = name.strip_prefix('#') {
                let n = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => num.parse::<u32>().ok(),
                };
                n.map(|n| match n {
                    0x80..=0x9F => windows_1252(n as u8),
                    _ => char::from_u32(n).unwrap_or('\u{FFFD}'),
                })
            } else {
                named(name)
            }?;
            // (The ";" after it, if there is one.)
            let used = if rest.as_bytes().get(e) == Some(&b';') { e + 1 } else { e };
            Some((c, used))
        });
        match read {
            Some((c, used)) => {
                if c != '\u{AD}' && c != '\u{200B}' {
                    out.push(c);
                }
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The named entities pages use most.
fn named(name: &str) -> Option<char> {
    Some(match name {
        "amp" | "AMP" => '&',
        "lt" | "LT" => '<',
        "gt" | "GT" => '>',
        "quot" | "QUOT" => '"',
        "apos" => '\'',
        "nbsp" | "ensp" | "emsp" | "thinsp" | "numsp" => ' ',
        "shy" => '\u{AD}',
        "zwnj" | "zwj" | "ZeroWidthSpace" => '\u{200B}',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "lsquo" => '‘',
        "rsquo" => '’',
        "sbquo" => '‚',
        "ldquo" => '“',
        "rdquo" => '”',
        "bdquo" => '„',
        "laquo" => '«',
        "raquo" => '»',
        "lsaquo" => '‹',
        "rsaquo" => '›',
        "bull" => '•',
        "middot" => '·',
        "deg" => '°',
        "plusmn" => '±',
        "times" => '×',
        "divide" => '÷',
        "minus" => '−',
        "euro" => '€',
        "pound" => '£',
        "yen" => '¥',
        "cent" => '¢',
        "sect" => '§',
        "para" => '¶',
        "frac12" => '½',
        "frac14" => '¼',
        "frac34" => '¾',
        "sup1" => '¹',
        "sup2" => '²',
        "sup3" => '³',
        "micro" => 'µ',
        "iexcl" => '¡',
        "iquest" => '¿',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        "harr" => '↔',
        "le" => '≤',
        "ge" => '≥',
        "ne" => '≠',
        "asymp" => '≈',
        "infin" => '∞',
        "check" => '✓',
        "dagger" => '†',
        "Dagger" => '‡',
        "permil" => '‰',
        "prime" => '′',
        "Prime" => '″',
        "ordf" => 'ª',
        "ordm" => 'º',
        "acute" => '´',
        "cedil" => '¸',
        "uml" => '¨',
        "macr" => '¯',
        "not" => '¬',
        "brvbar" => '¦',
        "curren" => '¤',
        "szlig" => 'ß',
        "Agrave" => 'À', "Aacute" => 'Á', "Acirc" => 'Â', "Atilde" => 'Ã', "Auml" => 'Ä', "Aring" => 'Å', "AElig" => 'Æ',
        "Ccedil" => 'Ç', "Egrave" => 'È', "Eacute" => 'É', "Ecirc" => 'Ê', "Euml" => 'Ë', "Igrave" => 'Ì', "Iacute" => 'Í',
        "Icirc" => 'Î', "Iuml" => 'Ï', "ETH" => 'Ð', "Ntilde" => 'Ñ', "Ograve" => 'Ò', "Oacute" => 'Ó', "Ocirc" => 'Ô',
        "Otilde" => 'Õ', "Ouml" => 'Ö', "Oslash" => 'Ø', "Ugrave" => 'Ù', "Uacute" => 'Ú', "Ucirc" => 'Û', "Uuml" => 'Ü',
        "Yacute" => 'Ý', "THORN" => 'Þ',
        "agrave" => 'à', "aacute" => 'á', "acirc" => 'â', "atilde" => 'ã', "auml" => 'ä', "aring" => 'å', "aelig" => 'æ',
        "ccedil" => 'ç', "egrave" => 'è', "eacute" => 'é', "ecirc" => 'ê', "euml" => 'ë', "igrave" => 'ì', "iacute" => 'í',
        "icirc" => 'î', "iuml" => 'ï', "eth" => 'ð', "ntilde" => 'ñ', "ograve" => 'ò', "oacute" => 'ó', "ocirc" => 'ô',
        "otilde" => 'õ', "ouml" => 'ö', "oslash" => 'ø', "ugrave" => 'ù', "uacute" => 'ú', "ucirc" => 'û', "uuml" => 'ü',
        "yacute" => 'ý', "thorn" => 'þ', "yuml" => 'ÿ', "OElig" => 'Œ', "oelig" => 'œ', "Scaron" => 'Š', "scaron" => 'š',
        "Yuml" => 'Ÿ', "fnof" => 'ƒ',
        "alpha" => 'α', "beta" => 'β', "gamma" => 'γ', "delta" => 'δ', "epsilon" => 'ε', "theta" => 'θ', "lambda" => 'λ',
        "mu" => 'μ', "pi" => 'π', "sigma" => 'σ', "omega" => 'ω', "Delta" => 'Δ', "Sigma" => 'Σ', "Omega" => 'Ω',
        _ => return None,
    })
}
