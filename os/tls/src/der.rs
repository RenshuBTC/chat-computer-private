//! Reading DER (X.690), the encoding certificates are in: tag, length, value, nested. Only what certificates use:
//! one-byte tags, definite lengths up to 4 bytes long. Everything is bounds-checked; damaged input gives None.

#[derive(Clone, Copy, Debug)]
pub struct Tlv<'a> {
    pub tag: u8,
    pub value: &'a [u8],
    /// The tag, the length and the value: what a signature covers.
    pub whole: &'a [u8],
}

impl<'a> Tlv<'a> {
    pub fn reader(&self) -> Der<'a> {
        Der::new(self.value)
    }
}

pub const BOOLEAN: u8 = 0x01;
pub const INTEGER: u8 = 0x02;
pub const BIT_STRING: u8 = 0x03;
pub const OCTET_STRING: u8 = 0x04;
pub const OID: u8 = 0x06;
pub const SEQUENCE: u8 = 0x30;
pub const SET: u8 = 0x31;
pub const UTC_TIME: u8 = 0x17;
pub const GENERALIZED_TIME: u8 = 0x18;

#[derive(Clone)]
pub struct Der<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Der<'a> {
    pub fn new(data: &'a [u8]) -> Der<'a> {
        Der { data, at: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.at >= self.data.len()
    }

    pub fn peek_tag(&self) -> Option<u8> {
        self.data.get(self.at).copied()
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Tlv<'a>> {
        let start = self.at;
        let tag = *self.data.get(self.at)?;
        if tag & 0x1F == 0x1F {
            return None; // (multi-byte tags: certificates don't use them)
        }
        let first = *self.data.get(self.at + 1)? as usize;
        let mut at = self.at + 2;
        let len = if first < 0x80 {
            first
        } else {
            let n = first & 0x7F;
            if n == 0 || n > 4 {
                return None;
            }
            let mut l = 0usize;
            for _ in 0..n {
                l = (l << 8) | *self.data.get(at)? as usize;
                at += 1;
            }
            l
        };
        let end = at.checked_add(len)?;
        let value = self.data.get(at..end)?;
        self.at = end;
        Some(Tlv { tag, value, whole: &self.data[start..end] })
    }

    pub fn expect(&mut self, tag: u8) -> Option<Tlv<'a>> {
        let t = self.next()?;
        (t.tag == tag).then_some(t)
    }

    /// The next item if it has `tag`; else None, and nothing read.
    pub fn optional(&mut self, tag: u8) -> Option<Tlv<'a>> {
        if self.peek_tag()? == tag { self.next() } else { None }
    }
}

/// An INTEGER's value without the leading zero DER adds to keep positive numbers positive.
pub fn unsigned(v: &[u8]) -> &[u8] {
    let mut v = v;
    while v.len() > 1 && v[0] == 0 {
        v = &v[1..];
    }
    v
}

/// A BIT STRING's bytes (with no unused bits: keys and signatures are whole bytes).
pub fn bits(v: &[u8]) -> Option<&[u8]> {
    match v.split_first() {
        Some((0, rest)) => Some(rest),
        _ => None,
    }
}

/// Seconds since 1970 for a calendar date and time (UTC).
pub fn seconds(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
    // Days from 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + hour * 3600 + minute * 60 + second
}

/// A UTCTime ("YYMMDDHHMMSSZ") or GeneralizedTime ("YYYYMMDDHHMMSSZ"), in seconds since 1970.
pub fn time(t: &Tlv) -> Option<i64> {
    let s = t.value;
    let digits = |r: core::ops::Range<usize>| -> Option<i64> {
        let mut v = 0i64;
        for &c in s.get(r)? {
            if !c.is_ascii_digit() {
                return None;
            }
            v = v * 10 + (c - b'0') as i64;
        }
        Some(v)
    };
    let (year, rest) = match t.tag {
        UTC_TIME if s.len() == 13 => {
            let yy = digits(0..2)?;
            (if yy < 50 { 2000 + yy } else { 1900 + yy }, 2)
        }
        GENERALIZED_TIME if s.len() == 15 => (digits(0..4)?, 4),
        _ => return None,
    };
    if s[s.len() - 1] != b'Z' {
        return None;
    }
    let (mo, d, h, mi, se) =
        (digits(rest..rest + 2)?, digits(rest + 2..rest + 4)?, digits(rest + 4..rest + 6)?, digits(rest + 6..rest + 8)?, digits(rest + 8..rest + 10)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    Some(seconds(year, mo, d, h, mi, se))
}
