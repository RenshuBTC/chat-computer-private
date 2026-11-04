//! Fields: bits of an operation region (or reached through an index and a data register, or a bank), read and written
//! in units of their access width, the bits around a field kept, set or cleared as its update rule says. And buffer
//! fields: bits of a buffer.

use crate::convert::copy;
use crate::{Aml, Error, Host, Object, PciAddress, Space, Value};
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

/// Where a field's bits are.
pub(crate) enum FieldAt {
    Region(usize),
    /// An index register (written with the byte offset) and a data register (then read or written).
    Index { index: usize, data: usize },
    /// A region reached once its bank register has a value.
    Bank { region: usize, bank: usize, value: u64 },
}

pub(crate) struct Field {
    pub at: FieldAt,
    /// Its first bit (from the region's start) and how many bits.
    pub bit: u64,
    pub bits: u64,
    /// The access type: 0 any, 1 byte, 2 word, 3 dword, 4 qword, 5 buffer.
    pub access: u8,
    /// The update rule: 0 preserve, 1 write as ones, 2 write as zeros.
    pub update: u8,
    /// Read and written holding the firmware's global lock (the Lock rule).
    pub lock: bool,
}

/// The most bytes a field is read or written as (a field bigger than any region is damaged).
const MOST_FIELD: u64 = 1 << 16;

impl Field {
    /// The bytes each access reads or writes: as its access type says; for "any", as ACPICA chooses (the narrowest
    /// width that reaches the whole field in one access, else the one that takes fewest, never past the end of its
    /// region, `region` bytes long).
    fn width(&self, region: Option<u64>) -> u64 {
        match self.access {
            1 => 1,
            2 => 2,
            3 => 4,
            4 => 8,
            _ => {
                let (start, end) = (self.bit, self.bit + self.bits.max(1));
                let end_byte = end.div_ceil(8);
                let (mut best, mut least) = (1u64, u64::MAX);
                for w in [1u64, 2, 4, 8] {
                    if region.is_some_and(|len| end_byte.next_multiple_of(w) > len) {
                        break;
                    }
                    let accesses = end.div_ceil(w * 8) - start / (w * 8);
                    if accesses <= 1 {
                        return w;
                    }
                    if accesses < least {
                        (best, least) = (w, accesses);
                    }
                }
                best
            }
        }
    }
}

impl Aml {
    /// A field's value: an integer if it fits in one, else a buffer.
    pub(crate) fn read_field(&mut self, node: usize, host: &mut dyn Host) -> Result<Value, Error> {
        let (bytes, bits, any) = match self.nodes[node].object.clone() {
            Object::BufferField { buffer, bit, bits, any } => {
                let b = buffer.borrow();
                if bit + bits > b.len() * 8 {
                    return Err(Error::Index);
                }
                (extract(&b, bit as u64, bits as u64), bits as u64, any)
            }
            Object::Field(f) => (self.locked(f.lock, host, |a, host| a.field_bytes(&f, host))?, f.bits, false),
            _ => return Err(Error::Type("a field")),
        };
        if bits <= self.int_bytes() as u64 * 8 && !any {
            Ok(Value::Integer(bytes.iter().rev().fold(0u64, |n, &x| n << 8 | x as u64)))
        } else {
            Ok(Value::buffer(bytes))
        }
    }

    /// Writes a value into a field: an integer's or a buffer's bits (cut, or padded with zeros, to its size).
    pub(crate) fn write_field(&mut self, node: usize, v: &Value, host: &mut dyn Host) -> Result<(), Error> {
        let mut bytes = match v {
            Value::Integer(n) => n.to_le_bytes().to_vec(),
            other => self.to_buffer(other)?,
        };
        match self.nodes[node].object.clone() {
            Object::BufferField { buffer, bit, bits, .. } => {
                bytes.resize(bits.div_ceil(8), 0);
                let mut b = buffer.borrow_mut();
                if bit + bits > b.len() * 8 {
                    return Err(Error::Index);
                }
                insert(&mut b, bit as u64, bits as u64, &bytes);
                Ok(())
            }
            Object::Field(f) => {
                bytes.resize(f.bits.div_ceil(8) as usize, 0);
                self.locked(f.lock, host, |a, host| a.write_field_bytes(&f, &bytes, host))
            }
            _ => Err(Error::Type("a field")),
        }
    }

    /// Runs `run` holding the firmware's global lock, if `lock` says to (a field with the Lock rule).
    fn locked<T>(&mut self, lock: bool, host: &mut dyn Host, run: impl FnOnce(&mut Self, &mut dyn Host) -> Result<T, Error>) -> Result<T, Error> {
        if !lock {
            return run(self, host);
        }
        if self.global_lock == 0 && !host.acquire_global_lock(0xFFFF) {
            return Err(Error::TooLong);
        }
        self.global_lock += 1;
        let r = run(self, host);
        self.global_lock -= 1;
        if self.global_lock == 0 {
            host.release_global_lock();
        }
        r
    }

    /// A field's bits, as bytes (little-endian, the last cut to its size).
    fn field_bytes(&mut self, f: &Field, host: &mut dyn Host) -> Result<Vec<u8>, Error> {
        let w = f.width(self.region_length(f));
        let first = f.bit / 8 / w * w;
        let end = (f.bit + f.bits).div_ceil(8);
        if f.bits == 0 || end - first > MOST_FIELD {
            return Err(Error::Index);
        }
        let mut raw = Vec::with_capacity((end - first + w) as usize);
        let mut at = first;
        while at < end {
            let unit = self.read_unit(f, at, w, host)?;
            raw.extend_from_slice(&unit.to_le_bytes()[..w as usize]);
            at += w;
        }
        Ok(extract(&raw, f.bit - first * 8, f.bits))
    }

    fn write_field_bytes(&mut self, f: &Field, bytes: &[u8], host: &mut dyn Host) -> Result<(), Error> {
        let w = f.width(self.region_length(f));
        let first = f.bit / 8 / w * w;
        let end = (f.bit + f.bits).div_ceil(8);
        if f.bits == 0 || end - first > MOST_FIELD {
            return Err(Error::Index);
        }
        let mut at = first;
        while at < end {
            // This unit's bits that are the field's, and the rest, kept, set or cleared.
            let lo = (f.bit.max(at * 8) - at * 8) as u32;
            let hi = ((f.bit + f.bits).min((at + w) * 8) - at * 8) as u32;
            let mask = if hi - lo >= 64 { u64::MAX } else { ((1u64 << (hi - lo)) - 1) << lo };
            let unit_bits = (w * 8) as u32;
            let full = if unit_bits >= 64 { u64::MAX } else { (1u64 << unit_bits) - 1 };
            // The field's bits for this unit.
            let from = at * 8 + lo as u64 - f.bit;
            let part = extract(bytes, from, (hi - lo) as u64);
            let part = part.iter().rev().fold(0u64, |n, &x| n << 8 | x as u64) << lo;
            let rest = if mask & full == full {
                0
            } else {
                match f.update {
                    1 => full & !mask,
                    2 => 0,
                    _ => self.read_unit(f, at, w, host)? & !mask,
                }
            };
            self.write_unit(f, at, w, (rest | part) & full, host)?;
            at += w;
        }
        Ok(())
    }

    /// One access unit of `w` bytes at byte `at` of a field's region (or through its index or bank).
    fn read_unit(&mut self, f: &Field, at: u64, w: u64, host: &mut dyn Host) -> Result<u64, Error> {
        match f.at {
            FieldAt::Region(r) => self.region_read(r, at, w, host),
            FieldAt::Bank { region, bank, value } => {
                self.store_node(bank, &Value::Integer(value), host)?;
                self.region_read(region, at, w, host)
            }
            FieldAt::Index { index, data } => {
                self.store_node(index, &Value::Integer(at), host)?;
                let v = self.read_field(data, host)?;
                self.to_integer(&v)
            }
        }
    }

    fn write_unit(&mut self, f: &Field, at: u64, w: u64, value: u64, host: &mut dyn Host) -> Result<(), Error> {
        match f.at {
            FieldAt::Region(r) => self.region_write(r, at, w, value, host),
            FieldAt::Bank { region, bank, value: b } => {
                self.store_node(bank, &Value::Integer(b), host)?;
                self.region_write(region, at, w, value, host)
            }
            FieldAt::Index { index, data } => {
                self.store_node(index, &Value::Integer(at), host)?;
                self.store_node(data, &Value::Integer(value), host)
            }
        }
    }

    /// The length of the region a field's bits are in (None for an index field's).
    fn region_length(&self, f: &Field) -> Option<u64> {
        let r = match f.at {
            FieldAt::Region(r) | FieldAt::Bank { region: r, .. } => r,
            FieldAt::Index { .. } => return None,
        };
        self.region(r).ok().map(|r| r.length)
    }

    fn region(&self, r: usize) -> Result<crate::Region, Error> {
        match &self.nodes[r].object {
            Object::Region(region) => Ok(region.clone()),
            _ => Err(Error::Type("an operation region")),
        }
    }

    fn region_read(&mut self, r: usize, at: u64, w: u64, host: &mut dyn Host) -> Result<u64, Error> {
        let region = self.region(r)?;
        if at + w > region.length {
            return Err(Error::Index);
        }
        if let Some(table) = &region.table {
            let b = table.get(at as usize..(at + w) as usize).ok_or(Error::Index)?;
            return Ok(b.iter().rev().fold(0u64, |n, &x| n << 8 | x as u64));
        }
        match region.space {
            Space::Pci => {
                let pci = self.pci_address(r, host);
                host.read_pci(pci, (region.offset + at) as u16, (w * 8) as u8).ok_or(Error::Region(Space::Pci))
            }
            space => host.read(space, region.offset.wrapping_add(at), (w * 8) as u8).ok_or(Error::Region(space)),
        }
    }

    fn region_write(&mut self, r: usize, at: u64, w: u64, value: u64, host: &mut dyn Host) -> Result<(), Error> {
        let region = self.region(r)?;
        if at + w > region.length {
            return Err(Error::Index);
        }
        if region.table.is_some() {
            return Err(Error::Region(Space::Table));
        }
        let fine = match region.space {
            Space::Pci => {
                let pci = self.pci_address(r, host);
                host.write_pci(pci, (region.offset + at) as u16, (w * 8) as u8, value)
            }
            space => host.write(space, region.offset.wrapping_add(at), (w * 8) as u8, value),
        };
        if fine { Ok(()) } else { Err(Error::Region(region.space)) }
    }

    /// Where a PCI_Config region is: its device's _ADR (device, function); the bus under the PCI root bridge it's
    /// under (its _BBN, and its _SEG), through each PCI bridge on the way (its secondary bus number).
    fn pci_address(&mut self, region: usize, host: &mut dyn Host) -> PciAddress {
        let device = self.nodes[region].parent;
        // The devices from the root bridge down to the region's.
        let mut way = Vec::new();
        let mut n = device;
        let mut root = None;
        for _ in 0..32 {
            if self.is_root_bridge(n, host) {
                root = Some(n);
                break;
            }
            way.push(n);
            if n == crate::ROOT {
                break;
            }
            n = self.nodes[n].parent;
        }
        let (mut bus, segment) = match root {
            Some(r) => (self.child_integer(r, "_BBN", host).unwrap_or(0) as u8, self.child_integer(r, "_SEG", host).unwrap_or(0) as u16),
            None => (0, 0),
        };
        way.reverse();
        let mut at = PciAddress { segment, bus, device: 0, function: 0 };
        if way.is_empty() {
            // The region is the root bridge's own (its device's _ADR, usually 0: the host bridge).
            let adr = root.and_then(|r| self.child_integer(r, "_ADR", host)).unwrap_or(0);
            return PciAddress { segment, bus, device: (adr >> 16) as u8, function: adr as u8 };
        }
        for (i, &d) in way.iter().enumerate() {
            let adr = self.child_integer(d, "_ADR", host).unwrap_or(0);
            at = PciAddress { segment, bus, device: (adr >> 16) as u8, function: adr as u8 };
            if i + 1 < way.len() {
                // A bridge on the way: the devices under it are on its secondary bus.
                bus = host.read_pci(at, 0x19, 8).unwrap_or(0) as u8;
            }
        }
        at
    }

    fn is_root_bridge(&mut self, node: usize, host: &mut dyn Host) -> bool {
        if !self.is_device(node) {
            return false;
        }
        let ids = self.child_ids(node, host);
        ids.iter().any(|i| i == "PNP0A03" || i == "PNP0A08")
    }

    /// Creates a buffer field over a buffer's bits (`any`: by CreateField).
    pub(crate) fn buffer_field(&self, buffer: Rc<RefCell<Vec<u8>>>, bit: u64, bits: u64, any: bool) -> Result<Object, Error> {
        if bits == 0 || bit.checked_add(bits).is_none_or(|end| end > buffer.borrow().len() as u64 * 8) {
            return Err(Error::Index);
        }
        Ok(Object::BufferField { buffer, bit: bit as usize, bits: bits as usize, any })
    }

    /// Stores into a named object (a field, or data), as Store does.
    pub(crate) fn store_node(&mut self, node: usize, v: &Value, host: &mut dyn Host) -> Result<(), Error> {
        match self.nodes[node].object.clone() {
            Object::Field(_) | Object::BufferField { .. } => self.write_field(node, v, host),
            Object::Data(old) => {
                let new = match old {
                    Value::Package(_) | Value::Uninitialized | Value::Reference(_) | Value::Object(_) => copy(v),
                    old => self.convert_like(v, &old)?,
                };
                self.nodes[node].object = Object::Data(new);
                Ok(())
            }
            Object::Alias(n) => self.store_node(n, v, host),
            _ => Err(Error::Type("something a value can be stored in")),
        }
    }
}

/// `bits` bits of `bytes` from bit `from`, as bytes (little-endian, the last cut to its size).
pub(crate) fn extract(bytes: &[u8], from: u64, bits: u64) -> Vec<u8> {
    let mut out = vec![0u8; bits.div_ceil(8) as usize];
    for i in 0..bits {
        let b = from + i;
        if bytes.get((b / 8) as usize).is_some_and(|x| x >> (b % 8) & 1 != 0) {
            out[(i / 8) as usize] |= 1 << (i % 8);
        }
    }
    out
}

/// Puts `bits` bits of `value` into `bytes` at bit `at`.
pub(crate) fn insert(bytes: &mut [u8], at: u64, bits: u64, value: &[u8]) {
    for i in 0..bits {
        let set = value.get((i / 8) as usize).is_some_and(|x| x >> (i % 8) & 1 != 0);
        let b = at + i;
        if let Some(byte) = bytes.get_mut((b / 8) as usize) {
            if set {
                *byte |= 1 << (b % 8);
            } else {
                *byte &= !(1 << (b % 8));
            }
        }
    }
}
