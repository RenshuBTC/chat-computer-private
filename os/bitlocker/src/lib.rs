//! BitLocker, read: a drive Windows encrypts (as it does by itself on many new PCs: "device encryption"), unlocked
//! with its recovery key (the 48 digits Microsoft keeps in the person's account), its password, or the clear key a
//! drive keeps while its encryption waits to be switched on. Then its sectors are decrypted as Windows' own would be,
//! for the NTFS reader to read.
//!
//! At the volume's start, where NTFS's boot sector was, BitLocker's own header says where the three copies of its
//! metadata are (each checked by its CRC). The metadata lists the volume's "key protectors": each a way of unwrapping
//! its volume master key (VMK), such as a key stretched from the recovery key's digits by a million rounds of SHA-256,
//! or the computer's TPM, which only Windows' own start-up can ask. The VMK unwraps the full volume encryption key
//! (FVEK), with AES-CCM, whose tag shows the key was right; and the metadata's own SHA-256, sealed with the VMK too,
//! shows it's as Windows wrote it. Sectors are AES-XTS (Windows 10 and later), or AES-CBC (7
//! and 8), each sector with its own tweak or IV from where it is on the drive. NTFS's first sectors are kept, encrypted,
//! elsewhere, and the metadata's own places read as zeros.
//!
//! As the libyal project's description of the format and cryptsetup's BitLocker support describe it. Windows 7's
//! "Elephant diffuser", Vista's version of the format, and drives part way through being encrypted or decrypted
//! (including ones encrypted as they're written to) aren't read. Nothing is ever written; the keys are kept only in memory, and overwritten when they're dropped.

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;

use crypto::aes_tables::Aes;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crypto::{Hash, Sha256};

/// Where the volume's bytes come from: `buf.len()` bytes at byte `offset` of it (multiples of 512).
pub trait Disk {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Not a volume BitLocker encrypts.
    NotBitLocker,
    Read,
    /// Something in the metadata doesn't add up.
    Damaged,
    /// A kind this doesn't read.
    Unsupported(&'static str),
    /// No protector of that kind on this volume.
    NoSuchProtector,
    /// The key (or password) doesn't unlock it.
    WrongKey,
    /// What was given isn't a recovery key: 48 digits, in 8 groups of 6, each group a multiple of 11.
    NotARecoveryKey,
}

/// A way the volume's master key is kept.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protection {
    /// In the clear: the drive's encryption isn't switched on yet (or is suspended).
    ClearKey,
    Tpm,
    StartupKey,
    TpmAndPin,
    RecoveryKey,
    SmartCard,
    Password,
    Other(u16),
}

fn protection(v: u16) -> Protection {
    match v {
        0x0000 => Protection::ClearKey,
        0x0100 => Protection::Tpm,
        0x0200 => Protection::StartupKey,
        0x0500 => Protection::TpmAndPin,
        0x0800 => Protection::RecoveryKey,
        0x1000 => Protection::SmartCard,
        0x2000 => Protection::Password,
        other => Protection::Other(other),
    }
}

/// A key sealed with AES-CCM: its 12-byte nonce, its 16-byte tag, and the sealed bytes.
#[derive(Clone)]
struct Sealed {
    nonce: [u8; 12],
    tag: [u8; 16],
    data: Vec<u8>,
}

#[derive(Clone)]
struct Protector {
    /// Its ID (a GUID).
    id: [u8; 16],
    kind: Protection,
    /// The salt its key is stretched with (a password's or a recovery key's).
    salt: Option<[u8; 16]>,
    /// The VMK, sealed with the protector's key.
    vmk: Option<Sealed>,
    /// A clear key's key, in the clear.
    clear: Option<Vec<u8>>,
}

/// Sector encryption methods.
const AES_CBC_128_DIFFUSER: u16 = 0x8000;
const AES_CBC_256_DIFFUSER: u16 = 0x8001;
const AES_CBC_128: u16 = 0x8002;
const AES_CBC_256: u16 = 0x8003;
const AES_XTS_128: u16 = 0x8004;
const AES_XTS_256: u16 = 0x8005;

/// The metadata's state when the whole volume is encrypted (not part way through).
const STATE_NORMAL: u16 = 0x0004;
/// The identifier in the header of a volume encrypted throughout, and of one encrypted as it's written to ("encrypt on
/// write", as Windows' device encryption starts: what hasn't been written since is still in the clear, so the volume
/// can't be read as one, and isn't).
const NORMAL: [u8; 16] = [0x3b, 0xd6, 0x67, 0x49, 0x29, 0x2e, 0xd8, 0x4a, 0x83, 0x99, 0xf6, 0xa3, 0x39, 0xe3, 0xd0, 0x01];
const ENCRYPT_ON_WRITE: [u8; 16] = [0x3b, 0x4d, 0xa8, 0x92, 0x80, 0xdd, 0x0e, 0x4d, 0x9e, 0x4e, 0xb1, 0xe3, 0x28, 0x4e, 0xae, 0xd8];
/// Each copy of the metadata takes 64 KiB.
const METADATA_SIZE: u64 = 64 * 1024;

/// A BitLocker volume, its metadata read, still locked.
#[derive(Clone)]
pub struct Volume {
    /// Its sectors' size: 512 or 4096 (what each tweak or IV is for).
    pub sector_size: u32,
    /// Its size, as its metadata says.
    pub size: u64,
    pub method: u16,
    /// BitLocker To Go's layout (a removable drive's).
    pub to_go: bool,
    /// What Windows calls the volume ("DESKTOP-ABC C: 21/10/2026").
    pub description: String,
    metadata: [u64; 3],
    /// The metadata's SHA-256, and the same sealed with the VMK (to check, once it's opened, that the metadata is as
    /// Windows wrote it).
    metadata_hash: [u8; 32],
    validation: Sealed,
    /// Where NTFS's first sectors are kept, and how many bytes.
    header_offset: u64,
    header_size: u64,
    protectors: Vec<Protector>,
    fvek: Option<Sealed>,
}

/// The unlocked volume's sector key.
pub struct Key {
    method: u16,
    data: Aes,
    /// XTS's tweak key.
    tweak: Option<Aes>,
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

/// Whether the boot sector says BitLocker encrypts the volume ("-FVE-FS-", or To Go's "MSWIN4.1" with its
/// identifier).
pub fn is_bitlocker(boot: &[u8]) -> bool {
    boot.get(3..11) == Some(b"-FVE-FS-") || (boot.get(3..11) == Some(b"MSWIN4.1") && boot.get(424..440).is_some_and(|g| g.iter().any(|&b| b != 0)))
}

/// Reads a BitLocker volume's header and a good copy of its metadata.
pub fn open(disk: &mut impl Disk) -> Result<Volume, Error> {
    let mut boot = vec![0u8; 512];
    if !disk.read(0, &mut boot) {
        return Err(Error::Read);
    }
    let (to_go, at) = match &boot[3..11] {
        b"-FVE-FS-" => (false, 160),
        b"MSWIN4.1" => (true, 424),
        _ => return Err(Error::NotBitLocker),
    };
    if boot[..3] == [0xEB, 0x52, 0x90] {
        return Err(Error::Unsupported("Windows Vista's BitLocker"));
    }
    match &boot[at..at + 16] {
        g if g == NORMAL => {}
        g if g == ENCRYPT_ON_WRITE => return Err(Error::Unsupported("a drive Windows is still encrypting as it's written to")),
        _ => return Err(Error::Unsupported("a kind of BitLocker volume this doesn't know")),
    }
    let sector_size = match u16_at(&boot, 11).ok_or(Error::Damaged)? {
        0 => 512,
        s @ (512 | 4096) => s as u32,
        _ => return Err(Error::Damaged),
    };
    let offsets = [u64_at(&boot, at + 16), u64_at(&boot, at + 24), u64_at(&boot, at + 32)];
    // The first copy of the metadata whose CRC is right.
    let mut block = vec![0u8; METADATA_SIZE as usize];
    let mut good = None;
    for offset in offsets.into_iter().flatten() {
        if offset % 512 != 0 || !disk.read(offset, &mut block) {
            continue;
        }
        if &block[0..8] != b"-FVE-FS-" || u16_at(&block, 10) != Some(2) {
            continue;
        }
        let size = (u16_at(&block, 8).unwrap_or(0) as usize) << 4;
        if size < 112 || size + 88 > block.len() {
            continue;
        }
        let (vsize, version) = (u16_at(&block, size).unwrap_or(0), u16_at(&block, size + 2).unwrap_or(99));
        if vsize < 8 || version > 2 || u32_at(&block, size + 4) != Some(crc32(&block[..size])) {
            continue;
        }
        good = Some(size);
        break;
    }
    let size = good.ok_or(Error::Damaged)?;
    // After the metadata and its CRC: an entry (80 bytes) with the metadata's SHA-256, sealed with the VMK.
    let after = &block[size..];
    if u16_at(after, 0).unwrap_or(0) < 88 || u16_at(after, 8) != Some(80) || u16_at(after, 10) != Some(0) || u16_at(after, 12) != Some(5) {
        return Err(Error::Damaged);
    }
    let validation = sealed(&after[8..88]).ok_or(Error::Damaged)?;
    let metadata_hash = sha256(&block[..size]);
    let b = &block[..size];
    let (state, next_state) = (u16_at(b, 12).ok_or(Error::Damaged)?, u16_at(b, 14).ok_or(Error::Damaged)?);
    if state != STATE_NORMAL || next_state != STATE_NORMAL {
        return Err(Error::Unsupported("a drive part way through being encrypted or decrypted"));
    }
    let volume_size = u64_at(b, 16).ok_or(Error::Damaged)?;
    let metadata = [u64_at(b, 32).ok_or(Error::Damaged)?, u64_at(b, 40).ok_or(Error::Damaged)?, u64_at(b, 48).ok_or(Error::Damaged)?];
    let method = u16_at(b, 64 + 36).ok_or(Error::Damaged)?;
    let entries_size = (u32_at(b, 64).ok_or(Error::Damaged)? as usize).saturating_sub(48);
    let entries = b.get(112..112 + entries_size).ok_or(Error::Damaged)?;
    let mut v = Volume {
        sector_size,
        size: volume_size,
        method,
        to_go,
        description: String::new(),
        metadata,
        metadata_hash,
        validation,
        header_offset: 0,
        header_size: 0,
        protectors: Vec::new(),
        fvek: None,
    };
    // The entries: each its size, type, value type and version, then what it holds.
    let mut at = 0;
    while at + 8 <= entries.len() {
        let len = u16_at(entries, at).unwrap_or(0) as usize;
        if len == 0 {
            break;
        }
        if len < 8 || at + len > entries.len() {
            return Err(Error::Damaged);
        }
        let e = &entries[at..at + len];
        match u16_at(e, 2).unwrap_or(0) {
            // A key protector: its id (16), when it changed (8), 2 bytes, its kind (2), then its own entries.
            0x0002 if len >= 36 => {
                let id = e[8..24].try_into().unwrap();
                let mut p = Protector { id, kind: protection(u16_at(e, 34).unwrap_or(0xFFFF)), salt: None, vmk: None, clear: None };
                let mut k = 36;
                while k + 8 <= e.len() {
                    let n = u16_at(e, k).unwrap_or(0) as usize;
                    if n < 8 || k + n > e.len() {
                        break;
                    }
                    let inner = &e[k..k + n];
                    match u16_at(inner, 4).unwrap_or(0) {
                        // A stretched key: its method (4), then the salt.
                        0x0003 if n >= 28 => p.salt = inner[12..28].try_into().ok(),
                        0x0005 => p.vmk = sealed(inner),
                        // A key in the clear: its method (4), then the key.
                        0x0001 if n > 12 => p.clear = Some(inner[12..].to_vec()),
                        _ => {}
                    }
                    k += n;
                }
                v.protectors.push(p);
            }
            0x0003 if v.fvek.is_none() => v.fvek = sealed(e),
            0x0007 => v.description = utf16(&e[8..]),
            // Where NTFS's first sectors are: their offset and size.
            0x000F if len >= 24 => {
                v.header_offset = u64_at(e, 8).unwrap_or(0);
                v.header_size = u64_at(e, 16).unwrap_or(0);
            }
            _ => {}
        }
        at += len;
    }
    // (The first sectors kept elsewhere: NTFS's first 8 KiB, or a FAT drive's first sectors and its tables, which
    // can be many megabytes.)
    let header_end = v.header_offset.checked_add(v.header_size);
    if v.fvek.is_none() || v.header_size == 0 || (v.header_offset | v.header_size) % 512 != 0 || header_end.is_none() {
        return Err(Error::Damaged);
    }
    Ok(v)
}

/// An AES-CCM sealed key entry: the entry's header (8), the nonce (12), the tag (16), then the sealed key.
fn sealed(e: &[u8]) -> Option<Sealed> {
    Some(Sealed { nonce: e.get(8..20)?.try_into().ok()?, tag: e.get(20..36)?.try_into().ok()?, data: e.get(36..)?.to_vec() })
}

fn utf16(b: &[u8]) -> String {
    let units = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0);
    char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect()
}

/// A GUID as Windows writes it: "64311DEA-4587-4029-924A-BA299647998E" (its first three parts stored little-endian).
fn guid(g: &[u8; 16]) -> String {
    let mut out = String::new();
    let order = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
    for (i, &k) in order.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        let hex = b"0123456789ABCDEF";
        out.push(hex[(g[k] >> 4) as usize] as char);
        out.push(hex[(g[k] & 15) as usize] as char);
    }
    out
}

/// CRC-32 (IEEE), as BitLocker checks its metadata with.
fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// AES-CCM (RFC 3610: a 16-byte tag, 3 bytes of length, no associated data), opened: the plain bytes, if the tag
/// shows they're what was sealed with this key.
fn open_ccm(key: &[u8], s: &Sealed) -> Option<Vec<u8>> {
    let aes = Aes::new(key)?;
    let counter = |i: u32| -> [u8; 16] {
        let mut a = [0u8; 16];
        a[0] = 2; // L − 1
        a[1..13].copy_from_slice(&s.nonce);
        a[13..].copy_from_slice(&i.to_be_bytes()[1..]);
        aes.encrypt(&mut a);
        a
    };
    let mut plain = s.data.clone();
    for (i, chunk) in plain.chunks_mut(16).enumerate() {
        let k = counter(i as u32 + 1);
        for (b, x) in chunk.iter_mut().zip(k) {
            *b ^= x;
        }
    }
    // The CBC-MAC of the plain bytes, after the first block (its flags: a 16-byte tag, 3 bytes of length).
    let mut x = [0u8; 16];
    x[0] = 0x3A;
    x[1..13].copy_from_slice(&s.nonce);
    x[13..].copy_from_slice(&(plain.len() as u32).to_be_bytes()[1..]);
    aes.encrypt(&mut x);
    for chunk in plain.chunks(16) {
        for (a, b) in x.iter_mut().zip(chunk) {
            *a ^= b;
        }
        aes.encrypt(&mut x);
    }
    let s0 = counter(0);
    let tag: [u8; 16] = core::array::from_fn(|i| x[i] ^ s0[i]);
    // (Compared without stopping at the first difference.)
    let differ = tag.iter().zip(&s.tag).fold(0u8, |d, (a, b)| d | (a ^ b));
    if differ != 0 {
        wipe(&mut plain);
        return None;
    }
    Some(plain)
}

/// A key opened from its sealed entry: the entry's size (which must be the sealed bytes'), its type, value type
/// and version, its method (4), then the key.
fn key_in(plain: &[u8], sealed_len: usize) -> Option<&[u8]> {
    if u16_at(plain, 0)? as usize != sealed_len || plain.len() < 12 {
        return None;
    }
    Some(&plain[12..])
}

/// A key stretched from a hash (of a password, or of a recovery key's 16 bytes) and a salt: a million rounds of
/// SHA-256 over the hash before, the first hash, the salt and the round's number.
fn stretch(first: [u8; 32], salt: &[u8; 16]) -> [u8; 32] {
    let mut data = [0u8; 88];
    data[32..64].copy_from_slice(&first);
    data[64..80].copy_from_slice(salt);
    for round in 0..0x10_0000u64 {
        data[80..88].copy_from_slice(&round.to_le_bytes());
        let mut h = Sha256::new();
        h.update(&data);
        h.finish_into(&mut data[..32]);
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&data[..32]);
    wipe(&mut data);
    key
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    let mut out = [0u8; 32];
    h.finish_into(&mut out);
    out
}

/// A recovery key's 16 bytes: its 48 digits (dashes and spaces between them don't matter), in 8 groups of 6, each a
/// multiple of 11 below 720,896, whose eleventh is two of the bytes (little-endian).
pub fn recovery_key_bytes(typed: &str) -> Result<[u8; 16], Error> {
    let mut digits = [0u8; 48];
    let mut n = 0;
    let mut ok = true;
    for b in typed.bytes().filter(|b| !matches!(b, b'-' | b' ')) {
        if n == 48 || !b.is_ascii_digit() {
            ok = false;
            break;
        }
        digits[n] = b - b'0';
        n += 1;
    }
    ok &= n == 48;
    let mut key = [0u8; 16];
    for (i, group) in digits.chunks(6).enumerate() {
        let v = group.iter().fold(0u32, |v, &d| v * 10 + d as u32);
        ok &= v % 11 == 0 && v / 11 <= 0xFFFF;
        key[2 * i..2 * i + 2].copy_from_slice(&((v / 11) as u16).to_le_bytes());
    }
    wipe(&mut digits);
    if !ok {
        wipe(&mut key);
        return Err(Error::NotARecoveryKey);
    }
    Ok(key)
}

/// Overwrites secret bytes (a key, or what one is made from) once they're done with.
fn wipe(b: &mut [u8]) {
    for x in b.iter_mut() {
        unsafe { core::ptr::write_volatile(x, 0) };
    }
}

/// A recovery key in `text`, if there's one in it: 8 groups of 6 digits, with a dash or a space between them (or
/// nothing), not part of a longer number, each group as a recovery key's are.
pub fn find_recovery_key(text: &str) -> Option<&str> {
    let b = text.as_bytes();
    for i in 0..b.len() {
        if !b[i].is_ascii_digit() || (i > 0 && b[i - 1].is_ascii_digit()) {
            continue;
        }
        let mut at = i;
        let mut groups = 0;
        while groups < 8 {
            if groups > 0 && matches!(b.get(at), Some(b'-' | b' ')) {
                at += 1;
            }
            if !b.get(at..at + 6).is_some_and(|g| g.iter().all(u8::is_ascii_digit)) {
                break;
            }
            at += 6;
            groups += 1;
        }
        if groups == 8 && !b.get(at).is_some_and(u8::is_ascii_digit) && recovery_key_bytes(&text[i..at]).is_ok() {
            return Some(&text[i..at]);
        }
    }
    None
}

impl Volume {
    /// The volume's key protectors, by kind.
    pub fn protections(&self) -> Vec<Protection> {
        self.protectors.iter().map(|p| p.kind).collect()
    }

    /// Its recovery keys' IDs, as Windows shows them ("64311DEA-4587-4029-924A-BA299647998E"): Windows' own recovery
    /// screen, and the Microsoft account's page that keeps the keys, show the first 8 digits, to find the right key by.
    pub fn recovery_key_ids(&self) -> Vec<String> {
        self.protectors.iter().filter(|p| p.kind == Protection::RecoveryKey).map(|p| guid(&p.id)).collect()
    }

    /// With the clear key a volume keeps while its encryption waits to be switched on (or is suspended).
    pub fn unlock_clear(&self) -> Result<Key, Error> {
        let p = self.protectors.iter().find(|p| p.kind == Protection::ClearKey).ok_or(Error::NoSuchProtector)?;
        let clear = p.clear.as_ref().ok_or(Error::Damaged)?;
        self.with_protector(p, clear)
    }

    /// With the recovery key (48 digits).
    pub fn unlock_recovery(&self, typed: &str) -> Result<Key, Error> {
        let mut bytes = recovery_key_bytes(typed)?;
        let first = sha256(&bytes);
        wipe(&mut bytes);
        self.unlock_stretched(Protection::RecoveryKey, first)
    }

    /// With the password a person set (for a drive that isn't Windows' own: a USB stick's, say).
    pub fn unlock_password(&self, password: &str) -> Result<Key, Error> {
        let mut utf16: Vec<u8> = password.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let mut once = sha256(&utf16);
        wipe(&mut utf16);
        let first = sha256(&once);
        wipe(&mut once);
        self.unlock_stretched(Protection::Password, first)
    }

    fn unlock_stretched(&self, kind: Protection, mut first: [u8; 32]) -> Result<Key, Error> {
        let mut result = Err(Error::NoSuchProtector);
        // (A volume can have more than one recovery key: each is tried.)
        for p in self.protectors.iter().filter(|p| p.kind == kind) {
            result = Err(Error::WrongKey);
            let Some(salt) = p.salt else { continue };
            let mut key = stretch(first, &salt);
            result = self.with_protector(p, &key);
            wipe(&mut key);
            if result.as_ref().err() != Some(&Error::WrongKey) {
                break;
            }
        }
        wipe(&mut first);
        result
    }

    /// The VMK opened with a protector's key; then the FVEK with the VMK; then the sector key. (What's opened on the
    /// way is overwritten once it's done with.)
    fn with_protector(&self, p: &Protector, key: &[u8]) -> Result<Key, Error> {
        let sealed = p.vmk.as_ref().ok_or(Error::Damaged)?;
        let mut plain = open_ccm(key, sealed).ok_or(Error::WrongKey)?;
        let result = key_in(&plain, sealed.data.len()).ok_or(Error::Damaged).and_then(|vmk| self.with_vmk(vmk));
        wipe(&mut plain);
        result
    }

    fn with_vmk(&self, vmk: &[u8]) -> Result<Key, Error> {
        // The metadata as Windows wrote it: its hash, sealed with the VMK (its role 0, type 1, the hash's kind 0x2005).
        let check = open_ccm(vmk, &self.validation).ok_or(Error::Damaged)?;
        if (u16_at(&check, 2), u16_at(&check, 4), u16_at(&check, 8)) != (Some(0), Some(1), Some(0x2005)) || check.get(12..44) != Some(&self.metadata_hash[..]) {
            return Err(Error::Damaged);
        }
        let sealed = self.fvek.as_ref().ok_or(Error::Damaged)?;
        let mut plain = open_ccm(vmk, sealed).ok_or(Error::Damaged)?;
        let result = key_in(&plain, sealed.data.len()).ok_or(Error::Damaged).and_then(|fvek| self.sector_key(fvek));
        wipe(&mut plain);
        result
    }

    fn sector_key(&self, fvek: &[u8]) -> Result<Key, Error> {
        let aes = |k: &[u8]| Aes::new(k).ok_or(Error::Damaged);
        Ok(match self.method {
            AES_XTS_128 if fvek.len() >= 32 => Key { method: self.method, data: aes(&fvek[..16])?, tweak: Some(aes(&fvek[16..32])?) },
            AES_XTS_256 if fvek.len() >= 64 => Key { method: self.method, data: aes(&fvek[..32])?, tweak: Some(aes(&fvek[32..64])?) },
            AES_CBC_128 if fvek.len() >= 16 => Key { method: self.method, data: aes(&fvek[..16])?, tweak: None },
            AES_CBC_256 if fvek.len() >= 32 => Key { method: self.method, data: aes(&fvek[..32])?, tweak: None },
            AES_CBC_128_DIFFUSER | AES_CBC_256_DIFFUSER => return Err(Error::Unsupported("Windows 7's Elephant diffuser")),
            AES_XTS_128 | AES_XTS_256 | AES_CBC_128 | AES_CBC_256 => return Err(Error::Damaged),
            _ => return Err(Error::Unsupported("an encryption method BitLocker didn't use before")),
        })
    }

    /// Reads the unlocked volume as Windows sees it: `buf.len()` bytes from byte `offset`, each sector decrypted where
    /// it really is (NTFS's first ones from where they're kept; the metadata's places as zeros). Whole sectors are read
    /// from the drive as many at once as are alike, and decrypted in place.
    pub fn read(&self, key: &Key, disk: &mut impl Disk, offset: u64, buf: &mut [u8]) -> bool {
        let ss = self.sector_size as usize;
        let mut done = 0usize;
        while done < buf.len() {
            let pos = offset + done as u64;
            let within = (pos % ss as u64) as usize;
            let start = pos - within as u64;
            let (real, run) = self.run(start);
            let left = buf.len() - done;
            if within != 0 || left < ss {
                // Part of a sector.
                let n = (ss - within).min(left);
                match real {
                    None => buf[done..done + n].fill(0),
                    Some(r) => {
                        let mut sector = vec![0u8; ss];
                        if !disk.read(r, &mut sector) {
                            return false;
                        }
                        self.decrypt(key, r, &mut sector);
                        buf[done..done + n].copy_from_slice(&sector[within..within + n]);
                    }
                }
                done += n;
                continue;
            }
            let sectors = (run / ss as u64).clamp(1, (left / ss) as u64) as usize;
            let out = &mut buf[done..done + sectors * ss];
            match real {
                None => out.fill(0),
                Some(r) => {
                    if !disk.read(r, out) {
                        return false;
                    }
                    for (i, sector) in out.chunks_exact_mut(ss).enumerate() {
                        self.decrypt(key, r + (i * ss) as u64, sector);
                    }
                }
            }
            done += sectors * ss;
        }
        true
    }

    /// Where the volume's sector at `start` really is on the drive (None: it reads as zeros, being where the metadata
    /// or NTFS's first sectors are kept), and how many bytes from it on are alike.
    fn run(&self, start: u64) -> (Option<u64>, u64) {
        let ss = self.sector_size as u64;
        if start < self.header_size {
            return (Some(self.header_offset + start), self.header_size - start);
        }
        let kept = self.metadata.iter().map(|&m| (m, m.saturating_add(METADATA_SIZE)));
        let mut next = u64::MAX;
        for (from, to) in kept.chain([(self.header_offset, self.header_offset + self.header_size)]) {
            if start < to && start + ss > from {
                return (None, to - start);
            }
            if from > start {
                next = next.min(from);
            }
        }
        (Some(start), next - start)
    }

    /// A sector (as many bytes as the volume's sectors) decrypted where it is on the drive (`at`, in bytes).
    fn decrypt(&self, key: &Key, at: u64, sector: &mut [u8]) {
        match (key.method, &key.tweak) {
            (AES_XTS_128 | AES_XTS_256, Some(tweak_key)) => {
                // XTS: the tweak is the sector's number encrypted; each block's is the one before's times x in
                // GF(2^128).
                let mut t = [0u8; 16];
                t[..8].copy_from_slice(&(at / self.sector_size as u64).to_le_bytes());
                tweak_key.encrypt(&mut t);
                for block in sector.chunks_exact_mut(16) {
                    let mut b: [u8; 16] = core::array::from_fn(|i| block[i] ^ t[i]);
                    key.data.decrypt(&mut b);
                    for i in 0..16 {
                        block[i] = b[i] ^ t[i];
                    }
                    let carry = t[15] >> 7;
                    for i in (1..16).rev() {
                        t[i] = t[i] << 1 | t[i - 1] >> 7;
                    }
                    t[0] = t[0] << 1 ^ if carry != 0 { 0x87 } else { 0 };
                }
            }
            _ => {
                // CBC: the IV is the sector's offset in bytes, encrypted.
                let mut iv = [0u8; 16];
                iv[..8].copy_from_slice(&at.to_le_bytes());
                key.data.encrypt(&mut iv);
                for block in sector.chunks_exact_mut(16) {
                    let cipher: [u8; 16] = block.try_into().unwrap();
                    let mut b = cipher;
                    key.data.decrypt(&mut b);
                    for i in 0..16 {
                        block[i] = b[i] ^ iv[i];
                    }
                    iv = cipher;
                }
            }
        }
    }
}
