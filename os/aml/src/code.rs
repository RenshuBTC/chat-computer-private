//! Reading the byte code: its bytes, package lengths, and names.

use crate::Error;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

/// A place in a table's code, and where the part being read ends.
pub(crate) struct Cursor {
    pub code: Rc<[u8]>,
    pub pos: usize,
    pub end: usize,
}

/// A name as the code writes it: from the root (\), or up from the current scope (^ each), then its segments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Name {
    pub root: bool,
    pub up: usize,
    pub segs: Vec<[u8; 4]>,
}

impl Name {
    /// Whether it's a single segment with no prefix, which is searched for up the scopes.
    pub fn searched(&self) -> bool {
        !self.root && self.up == 0 && self.segs.len() == 1
    }

    pub fn text(&self) -> String {
        let mut s = String::new();
        if self.root {
            s.push('\\');
        }
        for _ in 0..self.up {
            s.push('^');
        }
        let segs: Vec<String> = self.segs.iter().map(|g| String::from_utf8_lossy(g).into_owned()).collect();
        s + &segs.join(".")
    }
}

/// Whether a byte starts a name.
pub(crate) fn starts_name(b: u8) -> bool {
    matches!(b, b'\\' | b'^' | 0x2E | 0x2F | b'A'..=b'Z' | b'_')
}

impl Cursor {
    pub fn new(code: Rc<[u8]>, pos: usize, end: usize) -> Cursor {
        Cursor { code, pos, end }
    }

    fn fail<T>(&self) -> Result<T, Error> {
        Err(Error::Parse(self.pos))
    }

    pub fn byte(&mut self) -> Result<u8, Error> {
        if self.pos >= self.end {
            return self.fail();
        }
        let b = self.code[self.pos];
        self.pos += 1;
        Ok(b)
    }

    pub fn peek(&self) -> Option<u8> {
        (self.pos < self.end).then(|| self.code[self.pos])
    }

    pub fn word(&mut self) -> Result<u16, Error> {
        Ok(self.byte()? as u16 | (self.byte()? as u16) << 8)
    }

    pub fn dword(&mut self) -> Result<u32, Error> {
        Ok(self.word()? as u32 | (self.word()? as u32) << 16)
    }

    pub fn qword(&mut self) -> Result<u64, Error> {
        Ok(self.dword()? as u64 | (self.dword()? as u64) << 32)
    }

    /// A string constant: ASCII to its NUL.
    pub fn string(&mut self) -> Result<String, Error> {
        let mut s = String::new();
        loop {
            match self.byte()? {
                0 => return Ok(s),
                b => s.push(b as char),
            }
        }
    }

    /// A package length's value: one to four bytes.
    pub fn pkg_length(&mut self) -> Result<usize, Error> {
        let lead = self.byte()?;
        let count = (lead >> 6) as usize;
        if count == 0 {
            return Ok((lead & 0x3F) as usize);
        }
        let mut len = (lead & 0x0F) as usize;
        for i in 0..count {
            len |= (self.byte()? as usize) << (4 + 8 * i);
        }
        Ok(len)
    }

    /// A package length, as where the package ends (it counts from its own first byte).
    pub fn pkg_end(&mut self) -> Result<usize, Error> {
        let start = self.pos;
        let len = self.pkg_length()?;
        let end = start.checked_add(len).ok_or(Error::Parse(start))?;
        if end > self.end || end < self.pos {
            return Err(Error::Parse(start));
        }
        Ok(end)
    }

    /// A name segment: four characters, A-Z, 0-9 and _ (not a digit first).
    pub fn seg(&mut self) -> Result<[u8; 4], Error> {
        let at = self.pos;
        let mut s = [0u8; 4];
        for (i, b) in s.iter_mut().enumerate() {
            *b = self.byte()?;
            let fine = matches!(*b, b'A'..=b'Z' | b'_') || (i > 0 && b.is_ascii_digit());
            if !fine {
                return Err(Error::Parse(at));
            }
        }
        Ok(s)
    }

    /// A name: a prefix, then nothing (the null name), one segment, two (0x2E) or a count of them (0x2F).
    pub fn name(&mut self) -> Result<Name, Error> {
        let mut n = Name { root: false, up: 0, segs: Vec::new() };
        match self.peek() {
            Some(b'\\') => {
                self.pos += 1;
                n.root = true;
            }
            Some(b'^') => {
                while self.peek() == Some(b'^') {
                    self.pos += 1;
                    n.up += 1;
                }
            }
            _ => {}
        }
        match self.peek() {
            Some(0x00) => self.pos += 1,
            Some(0x2E) => {
                self.pos += 1;
                n.segs.push(self.seg()?);
                n.segs.push(self.seg()?);
            }
            Some(0x2F) => {
                self.pos += 1;
                let count = self.byte()?;
                for _ in 0..count {
                    n.segs.push(self.seg()?);
                }
            }
            _ => n.segs.push(self.seg()?),
        }
        Ok(n)
    }
}
