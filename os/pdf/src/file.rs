//! A PDF file's objects (ISO 32000-1, 7.5): where each is, from its cross-reference sections (tables, or, since PDF
//! 1.5, streams), newest first along their /Prev chain, and objects packed in object streams; or, where those can't be
//! read, from looking through the file for "n g obj" (as readers do with damaged files), which also finds an object
//! the sections put in the wrong place or leave out. Objects are read as they're asked for, object streams decoded
//! once each. An encrypted file's strings and streams are decrypted as they're read, if it opens without a password
//! (crypt.rs).

use crate::crypt::Crypt;
use crate::filter;
use crate::lex::{Dict, Lexer, Obj};
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug)]
enum Entry {
    /// An object written as it is, at this byte.
    At(usize),
    /// An object in an object stream: the stream's object, and which of its objects.
    Packed(u32, u32),
}

/// An object stream, decoded: its objects' numbers and where each starts.
struct Packed {
    data: Vec<u8>,
    objects: Vec<(u32, usize)>,
}

pub struct File<'a> {
    pub data: &'a [u8],
    entries: BTreeMap<u32, Entry>,
    pub trailer: Dict,
    packed: RefCell<BTreeMap<u32, Option<Rc<Packed>>>>,
    /// Bytes decoded so far (all of a file's streams together are held to a most).
    decoded: Cell<usize>,
    /// How many objects are being read inside one another now (a file whose objects lead back to themselves stops).
    depth: Cell<u32>,
    /// Where each "n g obj" and each "endstream" is in the file, once looked for.
    found: RefCell<Option<Rc<Found>>>,
    /// An encrypted file's key, if it opens without a password; and which object each stream read is (by where its
    /// data starts), for its key.
    crypt: Option<Crypt>,
    locked: bool,
    streams: RefCell<BTreeMap<usize, (u32, u16)>>,
}

/// All of a file's streams decode to at most this, together.
const MOST_DECODED: usize = 256 << 20;
/// The most cross-reference sections followed.
const MOST_SECTIONS: usize = 128;

impl<'a> File<'a> {
    pub fn open(data: &'a [u8]) -> File<'a> {
        let mut f = File {
            data,
            entries: BTreeMap::new(),
            trailer: Dict::default(),
            packed: RefCell::new(BTreeMap::new()),
            decoded: Cell::new(0),
            depth: Cell::new(0),
            found: RefCell::new(None),
            crypt: None,
            locked: false,
            streams: RefCell::new(BTreeMap::new()),
        };
        let read = startxref(data).is_some_and(|at| f.sections(at));
        if read {
            f.unlock();
        }
        if !read || f.trailer.get("Root").is_none() || f.get_root().is_none() {
            f.entries.clear();
            f.trailer = Dict::default();
            f.crypt = None;
            f.locked = false;
            f.packed.borrow_mut().clear();
            f.scan();
        }
        f
    }

    /// Whether it's encrypted and doesn't open without a password.
    pub fn locked(&self) -> bool {
        self.locked
    }

    /// An encrypted file's key worked out, if it opens without a password (else it's locked).
    fn unlock(&mut self) {
        let Some(e) = self.trailer.get("Encrypt").cloned() else { return };
        if e == Obj::Null {
            return;
        }
        let own = match e {
            Obj::Ref(n, _) => Some(n),
            _ => None,
        };
        let id = match self.trailer.get("ID").map(|i| self.resolve(i)) {
            Some(Obj::Array(a)) => a.first().map(|i| self.resolve(i)).and_then(|i| i.string().map(<[u8]>::to_vec)).unwrap_or_default(),
            _ => Vec::new(),
        };
        let crypt = match self.resolve(&e) {
            Obj::Dict(d) => Crypt::open(&d, &id, own, &|o| self.resolve(o)),
            _ => None,
        };
        self.locked = crypt.is_none();
        self.crypt = crypt;
        // (Object streams read before were read without the key.)
        self.packed.borrow_mut().clear();
    }

    fn get_root(&self) -> Option<Dict> {
        self.resolve(self.trailer.get("Root")?).dict().cloned()
    }

    /// The cross-reference sections from `at` back along /Prev. Whether any was read.
    fn sections(&mut self, mut at: usize) -> bool {
        let mut seen = Vec::new();
        let mut any = false;
        while seen.len() < MOST_SECTIONS && !seen.contains(&at) {
            seen.push(at);
            let Some((entries, trailer)) = self.section(at) else { break };
            any = true;
            for (n, e) in entries {
                self.entries.entry(n).or_insert(e);
            }
            // A file updated with both kinds has a stream of the newer objects' places beside its table.
            if let Some(stm) = trailer.get("XRefStm").and_then(Obj::int)
                && let Some((entries, _)) = self.section(stm as usize)
            {
                for (n, e) in entries {
                    self.entries.entry(n).or_insert(e);
                }
            }
            for (k, v) in &trailer.0 {
                if !self.trailer.0.iter().any(|(kk, _)| kk == k) {
                    self.trailer.0.push((k.clone(), v.clone()));
                }
            }
            match trailer.get("Prev").and_then(Obj::int) {
                Some(p) if p > 0 => at = p as usize,
                _ => break,
            }
        }
        any
    }

    /// One cross-reference section: a table and its trailer, or a stream (whose dictionary is the trailer).
    fn section(&self, at: usize) -> Option<(Vec<(u32, Entry)>, Dict)> {
        let mut lex = Lexer::new(self.data, at.min(self.data.len()));
        if lex.keyword(b"xref") {
            let mut out = Vec::new();
            loop {
                if lex.keyword(b"trailer") {
                    let trailer = lex.object()?.dict()?.clone();
                    return Some((out, trailer));
                }
                let first = lex.unsigned()?;
                let count = lex.unsigned()?;
                for i in 0..count.min(10_000_000) {
                    let offset = lex.unsigned()?;
                    let _generation = lex.unsigned()?;
                    let in_use = lex.keyword(b"n");
                    if !in_use && !lex.keyword(b"f") {
                        return None;
                    }
                    if in_use
                        && offset > 0
                        && let Ok(n) = u32::try_from(first.saturating_add(i))
                    {
                        out.push((n, Entry::At(offset as usize)));
                    }
                }
            }
        }
        // A cross-reference stream.
        let (_, _, obj) = self.object_at(at)?;
        let Obj::Stream(dict, range) = &obj else { return None };
        if dict.get("Type").and_then(Obj::name) != Some(b"XRef") {
            return None;
        }
        let data = self.decode(dict, range.clone())?;
        let w: Vec<usize> = dict.get("W")?.array()?.iter().map(|x| x.int().unwrap_or(0).clamp(0, 8) as usize).collect();
        if w.len() < 3 {
            return None;
        }
        let size = dict.get("Size").and_then(Obj::int).unwrap_or(0).max(0) as u64;
        let index: Vec<u64> = match dict.get("Index").and_then(Obj::array) {
            Some(a) => a.iter().map(|x| x.int().unwrap_or(0).max(0) as u64).collect(),
            None => alloc::vec![0, size],
        };
        let width = w[0] + w[1] + w[2];
        if width == 0 {
            return None;
        }
        let field = |row: &[u8], from: usize, len: usize| row[from..from + len].iter().fold(0u64, |v, &b| v << 8 | b as u64);
        let mut out = Vec::new();
        let mut rows = data.chunks_exact(width);
        for pair in index.chunks(2) {
            let (first, count) = (pair[0], pair.get(1).copied().unwrap_or(0));
            for i in 0..count {
                let Some(row) = rows.next() else { break };
                let kind = if w[0] == 0 { 1 } else { field(row, 0, w[0]) };
                let (a, b) = (field(row, w[0], w[1]), field(row, w[0] + w[1], w[2]));
                let Ok(n) = u32::try_from(first.saturating_add(i)) else { break };
                match kind {
                    1 if a > 0 => out.push((n, Entry::At(a as usize))),
                    2 => out.push((n, Entry::Packed(a as u32, b as u32))),
                    _ => {}
                }
            }
        }
        Some((out, dict.clone()))
    }

    /// Every "n g obj" in the file (the last of each number counting, as the newest), the trailer's dictionaries'
    /// last /Root, or else the catalog found among the objects; and the objects packed in object streams.
    fn scan(&mut self) {
        let d = self.data;
        let found = self.found();
        for (&n, &at) in found.objects.iter() {
            self.entries.insert(n, Entry::At(at));
        }
        // The trailer: the last "trailer" with a /Root.
        let mut i = 0;
        while let Some(found) = find(&d[i..], b"trailer") {
            let at = i + found + 7;
            i = at;
            let mut lex = Lexer::new(d, at);
            if let Some(Obj::Dict(t)) = lex.object()
                && t.get("Root").is_some()
            {
                self.trailer = t;
            }
        }
        let numbers: Vec<u32> = self.entries.keys().copied().collect();
        let mut catalog = None;
        let mut streams: Vec<u32> = Vec::new();
        for n in numbers {
            let Some(o) = self.get(n) else { continue };
            match o.dict().and_then(|d| d.get("Type")).and_then(Obj::name) {
                Some(b"Catalog") => catalog = Some(n),
                Some(b"ObjStm") => streams.push(n),
                Some(b"XRef") if self.trailer.get("Root").is_none() => {
                    if let Some(dict) = o.dict() {
                        self.trailer = dict.clone();
                    }
                }
                _ => {}
            }
        }
        // (The key, now the trailer's known, for the object streams.)
        self.unlock();
        for s in streams {
            if let Some(p) = self.packed(s) {
                for (k, &(n, _)) in p.objects.iter().enumerate() {
                    if !self.entries.contains_key(&n) {
                        self.entries.insert(n, Entry::Packed(s, k as u32));
                        // (The catalog may be one of them.)
                        if catalog.is_none() && self.get(n).and_then(|o| o.dict().and_then(|d| d.get("Type")).and_then(Obj::name).map(|t| t == b"Catalog")) == Some(true) {
                            catalog = Some(n);
                        }
                    }
                }
            }
        }
        if (self.trailer.get("Root").is_none() || self.get_root().is_none())
            && let Some(c) = catalog
        {
            self.trailer.0.retain(|(k, _)| k != b"Root");
            self.trailer.0.push((b"Root".to_vec(), Obj::Ref(c, 0)));
        }
    }

    /// Object number `n`, if the file has it.
    pub fn get(&self, n: u32) -> Option<Obj> {
        if self.depth.get() >= 16 {
            return None;
        }
        self.depth.set(self.depth.get() + 1);
        let found = self.get_inner(n);
        self.depth.set(self.depth.get() - 1);
        found
    }

    fn get_inner(&self, n: u32) -> Option<Obj> {
        let Some(&entry) = self.entries.get(&n) else { return self.rescued(n, None) };
        match entry {
            Entry::At(at) => self.object_at(at).filter(|(num, ..)| *num == n).map(|(n, g, o)| self.decrypted(n, g, o)).or_else(|| self.rescued(n, Some(at))),
            Entry::Packed(stream, index) => {
                let p = self.packed(stream)?;
                let &(num, start) = p.objects.get(index as usize)?;
                if num != n {
                    return None;
                }
                Lexer::new(&p.data, start).object()
            }
        }
    }

    /// Object `n` where looking through the file finds it, when its cross-reference entry (at `tried`, if it has one)
    /// was wrong or missing.
    fn rescued(&self, n: u32, tried: Option<usize>) -> Option<Obj> {
        let at = *self.found().objects.get(&n)?;
        if Some(at) == tried {
            return None;
        }
        self.object_at(at).filter(|(num, ..)| *num == n).map(|(n, g, o)| self.decrypted(n, g, o))
    }

    /// An object written as it is (not in an object stream), its strings decrypted if the file is encrypted.
    fn decrypted(&self, n: u32, g: u16, mut o: Obj) -> Obj {
        if let Some(c) = &self.crypt {
            c.strings_in(n, g, &mut o);
        }
        o
    }

    /// Where each "n g obj" and "endstream" is, looked for once.
    fn found(&self) -> Rc<Found> {
        if let Some(f) = self.found.borrow().as_ref() {
            return f.clone();
        }
        let f = Rc::new(objects_in(self.data));
        *self.found.borrow_mut() = Some(f.clone());
        f
    }

    /// The numbers of the objects the file has.
    pub fn numbers(&self) -> Vec<u32> {
        self.entries.keys().copied().collect()
    }

    /// The object "n g obj ... endobj" at byte `at`: its number, generation and value (a stream with where its data
    /// is).
    fn object_at(&self, at: usize) -> Option<(u32, u16, Obj)> {
        let mut lex = Lexer::new(self.data, at.min(self.data.len()));
        let n = u32::try_from(lex.unsigned()?).ok()?;
        let g = lex.unsigned()?.min(u16::MAX as u64) as u16;
        if !lex.keyword(b"obj") {
            return None;
        }
        let value = lex.object()?;
        let Obj::Dict(dict) = value else { return Some((n, g, value)) };
        if !lex.keyword(b"stream") {
            return Some((n, g, Obj::Dict(dict)));
        }
        // Its data: after the end of "stream"'s line, as long as /Length says if "endstream" is there, else up to the
        // next "endstream", or the next object, or the file's end (a file cut short). (Where those are is looked for
        // once, so a file of many such streams is read in the time one is.)
        let mut start = lex.pos;
        if self.data.get(start) == Some(&b'\r') {
            start += 1;
        }
        if self.data.get(start) == Some(&b'\n') {
            start += 1;
        }
        let said = match dict.get("Length") {
            Some(Obj::Ref(r, _)) => self.get(*r).and_then(|o| o.int()),
            Some(o) => o.int(),
            None => None,
        };
        let fits = |len: usize| {
            let mut l = Lexer::new(self.data, start.saturating_add(len).min(self.data.len()));
            l.keyword(b"endstream")
        };
        let end = match said {
            Some(len) if len >= 0 && (len as usize) <= self.data.len() - start.min(self.data.len()) && fits(len as usize) => {
                start + len as usize
            }
            _ => {
                let from = start.min(self.data.len());
                let found = self.found();
                let after = |list: &[usize], at: usize| list.get(list.partition_point(|&p| p <= at)).copied();
                let stream_end = after(&found.ends, from.saturating_sub(1)).unwrap_or(self.data.len());
                let next = after(&found.starts, at).unwrap_or(self.data.len());
                let mut end = stream_end.min(next).max(from);
                while end > from && matches!(self.data[end - 1], b'\r' | b'\n') {
                    end -= 1;
                }
                end
            }
        };
        if self.crypt.is_some() {
            self.streams.borrow_mut().insert(start, (n, g));
        }
        Some((n, g, Obj::Stream(dict, start..end)))
    }

    /// An object stream (object `n`), decoded, its table of contents read (once).
    fn packed(&self, n: u32) -> Option<Rc<Packed>> {
        if let Some(p) = self.packed.borrow().get(&n) {
            return p.clone();
        }
        // (Marked as being read first: one that leads back to itself isn't read again.)
        self.packed.borrow_mut().insert(n, None);
        let made = (|| {
            let (_, _, obj) = match *self.entries.get(&n)? {
                Entry::At(at) => self.object_at(at)?,
                Entry::Packed(..) => return None,
            };
            let Obj::Stream(dict, range) = obj else { return None };
            let data = self.decode(&dict, range)?;
            let count = dict.get("N").and_then(Obj::int)?.clamp(0, 1_000_000) as usize;
            let first = dict.get("First").and_then(Obj::int)?.max(0) as usize;
            let mut lex = Lexer::new(&data, 0);
            let mut objects = Vec::with_capacity(count.min(4096));
            for _ in 0..count {
                let (Some(num), Some(off)) = (lex.unsigned(), lex.unsigned()) else { break };
                objects.push((num as u32, first.saturating_add(off as usize)));
            }
            Some(Rc::new(Packed { data, objects }))
        })();
        self.packed.borrow_mut().insert(n, made.clone());
        made
    }

    /// The object itself, a reference followed (a few in a row at most).
    pub fn resolve(&self, o: &Obj) -> Obj {
        let mut cur = o.clone();
        for _ in 0..8 {
            match cur {
                Obj::Ref(n, _) => cur = self.get(n).unwrap_or(Obj::Null),
                other => return other,
            }
        }
        Obj::Null
    }

    /// A dictionary's entry, a reference followed.
    pub fn get_in(&self, d: &Dict, key: &str) -> Obj {
        d.get(key).map_or(Obj::Null, |o| self.resolve(o))
    }

    /// A stream's data, its filters undone.
    pub fn stream(&self, o: &Obj) -> Option<Vec<u8>> {
        match self.resolve(o) {
            Obj::Stream(dict, range) => self.decode(&dict, range),
            _ => None,
        }
    }

    /// A picture's stream: its dictionary, and its data, its filters undone but for a picture's own compression
    /// (its last filter's, if it's DCT, JPX, CCITT or JBIG2), which is given with its parameters.
    pub fn image_stream(&self, o: &Obj) -> Option<(Dict, Vec<u8>, Option<filter::Codec>)> {
        let Obj::Stream(dict, range) = self.resolve(o) else { return None };
        let raw = self.data.get(range.clone())?;
        if self.decoded.get() > MOST_DECODED {
            return None;
        }
        let decrypted;
        let raw = match &self.crypt {
            Some(c) if self.encrypted(c, &dict) => {
                let (n, g) = *self.streams.borrow().get(&range.start)?;
                decrypted = c.stream(n, g, raw);
                &decrypted[..]
            }
            _ => raw,
        };
        let (out, codec) = filter::decode_image(&dict, raw, &|o| self.resolve(o))?;
        self.decoded.set(self.decoded.get() + out.len());
        Some((dict, out, codec))
    }

    fn decode(&self, dict: &Dict, range: core::ops::Range<usize>) -> Option<Vec<u8>> {
        let raw = self.data.get(range.clone())?;
        if self.decoded.get() > MOST_DECODED {
            return None;
        }
        let decrypted;
        let raw = match &self.crypt {
            Some(c) if self.encrypted(c, dict) => {
                let (n, g) = *self.streams.borrow().get(&range.start)?;
                decrypted = c.stream(n, g, raw);
                &decrypted[..]
            }
            _ => raw,
        };
        let out = filter::decode(dict, raw, &|o| self.resolve(o))?;
        self.decoded.set(self.decoded.get() + out.len());
        Some(out)
    }
}

impl File<'_> {
    /// Whether a stream is encrypted: all are but cross-reference streams, metadata where the file says it isn't, and
    /// those whose first filter is the Identity crypt filter.
    fn encrypted(&self, c: &Crypt, dict: &Dict) -> bool {
        match self.get_in(dict, "Type").name() {
            Some(b"XRef") => return false,
            Some(b"Metadata") if !c.metadata => return false,
            _ => {}
        }
        let first = match self.get_in(dict, "Filter") {
            Obj::Array(a) => a.first().map(|f| self.resolve(f)),
            f => Some(f),
        };
        if first.as_ref().and_then(Obj::name) == Some(b"Crypt") {
            let parms = match self.get_in(dict, "DecodeParms") {
                Obj::Array(a) => a.first().map(|p| self.resolve(p)),
                p => Some(p),
            };
            let name = parms.as_ref().and_then(|p| p.dict()).and_then(|p| p.get("Name")).and_then(Obj::name);
            return !matches!(name, None | Some(b"Identity"));
        }
        true
    }
}

/// Where a file's objects and its streams' ends are: each "n g obj" (the last of each number, as the newest), every
/// one's start in order, and every "endstream", in order.
pub(crate) struct Found {
    objects: BTreeMap<u32, usize>,
    starts: Vec<usize>,
    ends: Vec<usize>,
}

fn objects_in(d: &[u8]) -> Found {
    let mut ends = Vec::new();
    let mut i = 0;
    while let Some(at) = find(&d[i..], b"endstream") {
        ends.push(i + at);
        i += at + 9;
    }
    let mut out = BTreeMap::new();
    let mut starts = Vec::new();
    let mut i = 0;
    while let Some(found) = find(&d[i..], b"obj") {
        let at = i + found;
        i = at + 3;
        if d.get(at + 3).is_some_and(|&b| !(crate::lex::is_white(b) || crate::lex::is_delim(b))) {
            continue;
        }
        // Back over "n g ".
        let mut j = at;
        let mut numbers = [0u64; 2];
        let mut ok = true;
        for slot in (0..2).rev() {
            while j > 0 && crate::lex::is_white(d[j - 1]) {
                j -= 1;
            }
            let digits_end = j;
            while j > 0 && d[j - 1].is_ascii_digit() {
                j -= 1;
            }
            // (Not digits: "endobj" and the like.)
            if j == digits_end || digits_end - j > 10 {
                ok = false;
                break;
            }
            numbers[slot] = core::str::from_utf8(&d[j..digits_end]).ok().and_then(|s| s.parse().ok()).unwrap_or(u64::MAX);
        }
        if ok && (j == 0 || crate::lex::is_white(d[j - 1]) || crate::lex::is_delim(d[j - 1])) && numbers[0] <= u32::MAX as u64 {
            out.insert(numbers[0] as u32, j);
            starts.push(j);
        }
    }
    Found { objects: out, starts, ends }
}

/// Where "startxref"'s number points (the newest cross-reference section).
fn startxref(d: &[u8]) -> Option<usize> {
    let tail = d.len().saturating_sub(4096);
    let at = tail + rfind(&d[tail..], b"startxref")?;
    let mut lex = Lexer::new(d, at + 9);
    lex.unsigned().map(|n| n as usize).filter(|&n| n < d.len())
}

pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let first = needle[0];
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        match hay[i..hay.len() - needle.len() + 1].iter().position(|&b| b == first) {
            Some(p) => {
                i += p;
                if &hay[i..i + needle.len()] == needle {
                    return Some(i);
                }
                i += 1;
            }
            None => return None,
        }
    }
    None
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    (0..=hay.len().checked_sub(needle.len())?).rev().find(|&i| &hay[i..i + needle.len()] == needle)
}
