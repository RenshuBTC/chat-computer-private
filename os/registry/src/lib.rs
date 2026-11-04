//! Windows' registry, read: a hive file (as Windows keeps them in Windows\System32\config: SYSTEM, SOFTWARE...),
//! its keys and their values, never written. Chat Computer reads what Windows says about the PC: which drive has which
//! letter (SYSTEM's MountedDevices), and more later.
//!
//! A hive is a 4 KiB header ("regf", with the root key's place), then bins ("hbin") of cells, each its size (negative
//! while it's in use) then what it holds: a key ("nk": its name, its subkeys' list, its values' list), a value ("vk":
//! its name, type and data, or where the data is), lists of subkeys ("lf", "lh" and "li", and "ri", a list of
//! those), a list of values, and data (in pieces, "db", past 16,344 bytes). As the format's descriptions by the libyal
//! project and by Maxim Suhanov describe it. Cells are read as they're needed, through `Source`, so a hive of tens of
//! megabytes isn't read whole to find a key. Every offset and size is checked before it's used, so a damaged hive
//! gives an error, never a crash; lists that point back at themselves are cut off.

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Where the hive's bytes come from: `buf.len()` bytes at byte `offset` of the file.
pub trait Source {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    Read,
    /// Not a hive (no "regf", or a version that isn't 1).
    NotAHive,
    /// Something in it doesn't add up.
    Damaged,
    NotFound,
}

/// Value types.
pub const REG_NONE: u32 = 0;
pub const REG_SZ: u32 = 1;
pub const REG_EXPAND_SZ: u32 = 2;
pub const REG_BINARY: u32 = 3;
pub const REG_DWORD: u32 = 4;
pub const REG_MULTI_SZ: u32 = 7;
pub const REG_QWORD: u32 = 11;

/// The header's size: the bins come after it, and cells' offsets count from there.
const BASE: u64 = 4096;
/// The biggest cell read whole (a value's data past 16,344 bytes is in pieces), and the biggest value's data put
/// together.
const MOST_CELL: u32 = 1 << 20;
const MOST_DATA: u32 = 16 << 20;
/// Data at most this long is in one cell; longer, in pieces ("db"), from version 1.4.
const ONE_CELL: u32 = 16_344;
/// The most entries a list of subkeys can have (each cell holds at most a megabyte of them).
const MOST_ENTRIES: usize = 1 << 17;

/// A hive, its header read.
pub struct Hive {
    /// Where the root key is, and how many bytes of bins there are.
    root: u32,
    bins: u32,
    minor: u32,
}

/// A key.
#[derive(Clone, Debug)]
pub struct Key {
    pub name: String,
    /// When it last changed (a FILETIME: 100 ns since 1601).
    pub modified: u64,
    pub subkey_count: u32,
    pub value_count: u32,
    offset: u32,
    subkeys: u32,
    values: u32,
}

/// A value: its name ("" for a key's default value), its type (REG_SZ...) and its data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value {
    pub name: String,
    pub kind: u32,
    pub data: Vec<u8>,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

/// A name as the hive keeps it: one byte a character (Latin-1) if `compressed`, else UTF-16.
fn name_of(b: &[u8], compressed: bool) -> String {
    if compressed {
        b.iter().map(|&c| c as char).collect()
    } else {
        let units = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
        char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect()
    }
}

/// Two names the same, as Windows compares them: whatever their case.
fn same_name(a: &str, b: &str) -> bool {
    a.chars().flat_map(char::to_uppercase).eq(b.chars().flat_map(char::to_uppercase))
}

/// A string value's text (REG_SZ, REG_EXPAND_SZ: UTF-16 up to its first NUL).
pub fn text(data: &[u8]) -> String {
    let units = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0);
    char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect()
}

/// A REG_MULTI_SZ value's strings.
pub fn texts(data: &[u8]) -> Vec<String> {
    let units: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    units
        .split(|&u| u == 0)
        .filter(|s| !s.is_empty())
        .map(|s| char::decode_utf16(s.iter().copied()).map(|c| c.unwrap_or('\u{FFFD}')).collect())
        .collect()
}

impl Hive {
    /// Reads a hive's header (checked by its checksum).
    pub fn open(src: &mut impl Source) -> Result<Hive, Error> {
        let mut base = [0u8; 512];
        if !src.read(0, &mut base) {
            return Err(Error::Read);
        }
        if &base[..4] != b"regf" || u32_at(&base, 0x14) != Some(1) {
            return Err(Error::NotAHive);
        }
        // The first 508 bytes XORed as 32-bit numbers (0 and all ones kept from).
        let mut sum = base[..508].chunks_exact(4).fold(0u32, |x, c| x ^ u32::from_le_bytes(c.try_into().unwrap()));
        sum = match sum {
            0 => 1,
            0xFFFF_FFFF => 0xFFFF_FFFE,
            s => s,
        };
        if u32_at(&base, 0x1FC) != Some(sum) {
            return Err(Error::Damaged);
        }
        let hive = Hive { root: u32_at(&base, 0x24).unwrap(), bins: u32_at(&base, 0x28).unwrap(), minor: u32_at(&base, 0x18).unwrap() };
        let mut bin = [0u8; 4];
        if !src.read(BASE, &mut bin) {
            return Err(Error::Read);
        }
        if &bin != b"hbin" || hive.bins < 4096 || hive.bins % 4096 != 0 {
            return Err(Error::Damaged);
        }
        Ok(hive)
    }

    /// A cell's contents (without its size), checked: in use, inside the bins, of a sensible size.
    fn cell(&self, src: &mut impl Source, offset: u32) -> Result<Vec<u8>, Error> {
        if offset % 8 != 0 || offset < 32 || offset as u64 + 4 > self.bins as u64 {
            return Err(Error::Damaged);
        }
        let mut size = [0u8; 4];
        if !src.read(BASE + offset as u64, &mut size) {
            return Err(Error::Read);
        }
        let size = i32::from_le_bytes(size);
        if size >= 0 {
            return Err(Error::Damaged); // (free)
        }
        let size = size.unsigned_abs();
        if !(8..=MOST_CELL).contains(&size) || offset as u64 + size as u64 > self.bins as u64 {
            return Err(Error::Damaged);
        }
        let mut data = vec![0u8; size as usize - 4];
        if !src.read(BASE + offset as u64 + 4, &mut data) {
            return Err(Error::Read);
        }
        Ok(data)
    }

    fn key_at(&self, src: &mut impl Source, offset: u32) -> Result<Key, Error> {
        let c = self.cell(src, offset)?;
        if c.get(..2) != Some(b"nk") {
            return Err(Error::Damaged);
        }
        let flags = u16_at(&c, 0x02).ok_or(Error::Damaged)?;
        let len = u16_at(&c, 0x48).ok_or(Error::Damaged)? as usize;
        let name = c.get(0x4C..0x4C + len).ok_or(Error::Damaged)?;
        Ok(Key {
            name: name_of(name, flags & 0x20 != 0),
            modified: u64_at(&c, 0x04).ok_or(Error::Damaged)?,
            subkey_count: u32_at(&c, 0x14).ok_or(Error::Damaged)?,
            value_count: u32_at(&c, 0x24).ok_or(Error::Damaged)?,
            offset,
            subkeys: u32_at(&c, 0x1C).ok_or(Error::Damaged)?,
            values: u32_at(&c, 0x28).ok_or(Error::Damaged)?,
        })
    }

    /// The hive's root key.
    pub fn root(&self, src: &mut impl Source) -> Result<Key, Error> {
        self.key_at(src, self.root)
    }

    /// Where a key's subkeys are: from its list, or its lists' list ("ri").
    fn subkey_offsets(&self, src: &mut impl Source, key: &Key) -> Result<Vec<u32>, Error> {
        if key.subkey_count == 0 || key.subkeys == 0xFFFF_FFFF {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let list = self.cell(src, key.subkeys)?;
        match list.get(..2) {
            Some(b"ri") => {
                let n = u16_at(&list, 2).ok_or(Error::Damaged)? as usize;
                for i in 0..n {
                    let sub = u32_at(&list, 4 + 4 * i).ok_or(Error::Damaged)?;
                    let inner = self.cell(src, sub)?;
                    if inner.get(..2) == Some(b"ri") {
                        return Err(Error::Damaged); // (a list of lists of lists isn't made)
                    }
                    Self::list_entries(&inner, &mut out)?;
                    // Checked here too: 65,535 lists of 65,535 entries each would be 17 GB before the check after the loop.
                    if out.len() > MOST_ENTRIES {
                        return Err(Error::Damaged);
                    }
                }
            }
            _ => Self::list_entries(&list, &mut out)?,
        }
        if out.len() > MOST_ENTRIES {
            return Err(Error::Damaged);
        }
        Ok(out)
    }

    /// A subkey list's entries: "li" (offsets), "lf" and "lh" (an offset and a hint of the name each).
    fn list_entries(list: &[u8], out: &mut Vec<u32>) -> Result<(), Error> {
        let count = u16_at(list, 2).ok_or(Error::Damaged)? as usize;
        let step = match list.get(..2) {
            Some(b"li") => 4,
            Some(b"lf" | b"lh") => 8,
            _ => return Err(Error::Damaged),
        };
        for i in 0..count {
            out.push(u32_at(list, 4 + step * i).ok_or(Error::Damaged)?);
        }
        Ok(())
    }

    /// A key's subkeys, in the hive's order (by name, as Windows keeps them).
    pub fn subkeys(&self, src: &mut impl Source, key: &Key) -> Result<Vec<Key>, Error> {
        let offsets = self.subkey_offsets(src, key)?;
        let mut out = Vec::with_capacity(offsets.len());
        for o in offsets {
            if o == key.offset {
                return Err(Error::Damaged);
            }
            out.push(self.key_at(src, o)?);
        }
        Ok(out)
    }

    /// A key's subkey of this name (whatever its case).
    pub fn subkey(&self, src: &mut impl Source, key: &Key, name: &str) -> Result<Key, Error> {
        for o in self.subkey_offsets(src, key)? {
            let k = self.key_at(src, o)?;
            if same_name(&k.name, name) {
                return Ok(k);
            }
        }
        Err(Error::NotFound)
    }

    /// The key at a path from the root ("ControlSet001\Control\TimeZoneInformation").
    pub fn key(&self, src: &mut impl Source, path: &str) -> Result<Key, Error> {
        let mut key = self.root(src)?;
        for part in path.split(['\\', '/']).filter(|p| !p.is_empty()) {
            key = self.subkey(src, &key, part)?;
        }
        Ok(key)
    }

    /// A key's values, in the hive's order.
    pub fn values(&self, src: &mut impl Source, key: &Key) -> Result<Vec<Value>, Error> {
        if key.value_count == 0 || key.values == 0xFFFF_FFFF {
            return Ok(Vec::new());
        }
        let list = self.cell(src, key.values)?;
        let count = key.value_count as usize;
        if count * 4 > list.len() {
            return Err(Error::Damaged);
        }
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            out.push(self.value_at(src, u32_at(&list, 4 * i).unwrap())?);
        }
        Ok(out)
    }

    /// A key's value of this name (whatever its case; "" for its default).
    pub fn value(&self, src: &mut impl Source, key: &Key, name: &str) -> Result<Value, Error> {
        self.values(src, key)?.into_iter().find(|v| same_name(&v.name, name)).ok_or(Error::NotFound)
    }

    fn value_at(&self, src: &mut impl Source, offset: u32) -> Result<Value, Error> {
        let c = self.cell(src, offset)?;
        if c.get(..2) != Some(b"vk") {
            return Err(Error::Damaged);
        }
        let len = u16_at(&c, 0x02).ok_or(Error::Damaged)? as usize;
        let size = u32_at(&c, 0x04).ok_or(Error::Damaged)?;
        let at = u32_at(&c, 0x08).ok_or(Error::Damaged)?;
        let kind = u32_at(&c, 0x0C).ok_or(Error::Damaged)?;
        let flags = u16_at(&c, 0x10).ok_or(Error::Damaged)?;
        let name = name_of(c.get(0x14..0x14 + len).ok_or(Error::Damaged)?, flags & 1 != 0);
        let data = if size & 0x8000_0000 != 0 {
            // Up to 4 bytes, kept where the data's offset would be.
            let n = (size & 0x7FFF_FFFF) as usize;
            if n > 4 {
                return Err(Error::Damaged);
            }
            c[0x08..0x08 + n].to_vec()
        } else if size == 0 {
            Vec::new()
        } else if size > MOST_DATA {
            return Err(Error::Damaged);
        } else {
            let mut d = self.cell(src, at)?;
            if d.len() >= size as usize {
                d.truncate(size as usize);
                d
            } else if size > ONE_CELL && self.minor >= 4 && d.starts_with(b"db") {
                // (Windows keeps data this long in pieces; hivex, in one cell, as above.)
                self.big_data(src, &d, size)?
            } else {
                return Err(Error::Damaged);
            }
        };
        Ok(Value { name, kind, data })
    }

    /// Data in pieces ("db": how many, and where the list of them is), each piece's first 16,344 bytes.
    fn big_data(&self, src: &mut impl Source, db: &[u8], size: u32) -> Result<Vec<u8>, Error> {
        let count = u16_at(db, 2).ok_or(Error::Damaged)? as usize;
        let list = self.cell(src, u32_at(db, 4).ok_or(Error::Damaged)?)?;
        let mut out = Vec::with_capacity(size as usize);
        for i in 0..count {
            let piece = self.cell(src, u32_at(&list, 4 * i).ok_or(Error::Damaged)?)?;
            let want = (size as usize - out.len()).min(ONE_CELL as usize).min(piece.len());
            out.extend_from_slice(&piece[..want]);
            if out.len() == size as usize {
                break;
            }
        }
        if out.len() != size as usize {
            return Err(Error::Damaged);
        }
        Ok(out)
    }
}

/// What SYSTEM's MountedDevices says a volume is, to know which has which letter: a GPT partition by its unique
/// GUID ("DMIO:ID:" and the GUID's 16 bytes), or an MBR partition by its disk's signature and where it starts (bytes);
/// or another kind of volume by its device's name, in UTF-16 (a USB stick's:
/// "_??_USBSTOR#Disk&Ven_SanDisk&Prod_Ultra&Rev_1.00#<its serial number>&0#{53f56307-...}").
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Mounted {
    Gpt([u8; 16]),
    Mbr { signature: u32, offset: u64 },
    Device(String),
}

impl Mounted {
    /// Whether this names the volume of a USB drive's logical unit `lun` whose serial number is `serial` (USB sticks
    /// are named by their device, not their partitions).
    pub fn is_usb(&self, serial: &str, lun: u8) -> bool {
        let Mounted::Device(name) = self else { return false };
        let name = name.to_ascii_uppercase();
        !serial.is_empty()
            && (name.starts_with("_??_USBSTOR#") || name.starts_with("\\??\\USBSTOR#"))
            && name.contains(&alloc::format!("#{}&{lun}#", serial.to_ascii_uppercase()))
    }
}

/// The drive letters in SYSTEM's MountedDevices ("\DosDevices\D:"), each with the volume it's for.
pub fn drive_letters(hive: &Hive, src: &mut impl Source) -> Result<Vec<(char, Mounted)>, Error> {
    let key = hive.key(src, "MountedDevices")?;
    let mut out = Vec::new();
    for v in hive.values(src, &key)? {
        let Some(letter) = v.name.strip_prefix("\\DosDevices\\").and_then(|l| l.strip_suffix(':')) else { continue };
        let Some(letter) = letter.chars().next().filter(|c| letter.len() == 1 && c.is_ascii_alphabetic()) else { continue };
        let d = &v.data;
        let mounted = if d.len() == 24 && d.starts_with(b"DMIO:ID:") {
            Mounted::Gpt(d[8..24].try_into().unwrap())
        } else if d.len() == 12 {
            Mounted::Mbr { signature: u32_at(d, 0).unwrap(), offset: u64_at(d, 4).unwrap() }
        } else {
            let units = d.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
            Mounted::Device(char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect::<String>().trim_end_matches('\0').into())
        };
        out.push((letter.to_ascii_uppercase(), mounted));
    }
    out.sort_by_key(|(l, _)| *l);
    Ok(out)
}
