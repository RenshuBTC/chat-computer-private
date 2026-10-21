//! Virtio's network card (virtio 1.x over PCI, the OASIS specification): the card QEMU, KVM and most virtual
//! machines and clouds give a computer.
//!
//! The card works from two queues in memory Chat Computer gives it: one of empty buffers it fills with the
//! frames that arrive, one of frames to send. Each queue is a table of buffers (descriptors), a ring where
//! Chat Computer offers them (the driver area) and a ring where the card hands them back (the device area).
//! Each frame starts with a 12-byte virtio header, all zeros here: no checksums done by the card, no
//! segmentation.

use super::Card;
use crate::dma::Page;
use crate::paging::{self, Cache};
use crate::pci;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

pub const VENDOR: u16 = 0x1AF4;
/// Transitional (0x1000) and modern (0x1041) network cards.
pub const DEVICES: [u16; 2] = [0x1000, 0x1041];

const CAP_VENDOR: u8 = 0x09;
const CFG_COMMON: u8 = 1;
const CFG_NOTIFY: u8 = 2;
const CFG_DEVICE: u8 = 4;

// The common configuration.
const DEVICE_FEATURE_SELECT: u64 = 0x00;
const DEVICE_FEATURE: u64 = 0x04;
const DRIVER_FEATURE_SELECT: u64 = 0x08;
const DRIVER_FEATURE: u64 = 0x0C;
const CONFIG_MSIX: u64 = 0x10;
const STATUS: u64 = 0x14;
const QUEUE_SELECT: u64 = 0x16;
const QUEUE_SIZE: u64 = 0x18;
const QUEUE_MSIX: u64 = 0x1A;
const QUEUE_ENABLE: u64 = 0x1C;
const QUEUE_NOTIFY_OFF: u64 = 0x1E;
const QUEUE_DESC: u64 = 0x20;
const QUEUE_DRIVER: u64 = 0x28;
const QUEUE_DEVICE: u64 = 0x30;

const ACKNOWLEDGE: u8 = 1;
const DRIVER: u8 = 2;
const DRIVER_OK: u8 = 4;
const FEATURES_OK: u8 = 8;
const FAILED: u8 = 128;

const F_MAC: u64 = 1 << 5;
const F_STATUS: u64 = 1 << 16;
const F_VERSION_1: u64 = 1 << 32;

const NO_VECTOR: u16 = 0xFFFF;
const DESC_WRITE: u16 = 2;

const HEADER: usize = 12;
const BUF: usize = 2048;
/// Buffers per queue (fewer if the card offers fewer).
const QUEUE_MAX: u16 = 128;

fn r8(a: u64) -> u8 {
    unsafe { read_volatile(a as *const u8) }
}
fn r16(a: u64) -> u16 {
    unsafe { read_volatile(a as *const u16) }
}
fn r32(a: u64) -> u32 {
    unsafe { read_volatile(a as *const u32) }
}
fn w8(a: u64, v: u8) {
    unsafe { write_volatile(a as *mut u8, v) }
}
fn w16(a: u64, v: u16) {
    unsafe { write_volatile(a as *mut u16, v) }
}
fn w32(a: u64, v: u32) {
    unsafe { write_volatile(a as *mut u32, v) }
}
fn w64(a: u64, v: u64) {
    w32(a, v as u32);
    w32(a + 4, (v >> 32) as u32);
}

/// One queue: its table, its two rings, and its buffers (two to a page).
struct Queue {
    size: u16,
    desc: Page,
    avail: Page,
    used: Page,
    buffers: Vec<Page>,
    notify: u64,
    index: u16,
    /// How many have been offered (it counts on past the ring's size).
    offered: u16,
    /// How many handed back have been looked at.
    seen: u16,
    /// Buffers free to send from (the sending queue).
    free: Vec<u16>,
}

impl Queue {
    fn buffer(&self, id: u16) -> (u64, *mut u8) {
        let page = &self.buffers[id as usize / 2];
        let offset = (id as usize % 2) * BUF;
        (page.phys + offset as u64, page.at::<u8>(offset))
    }

    fn set_desc(&self, id: u16, phys: u64, len: u32, flags: u16) {
        let d = self.desc.at::<u8>(16 * id as usize) as u64;
        unsafe {
            write_volatile(d as *mut u64, phys);
            write_volatile((d + 8) as *mut u32, len);
            write_volatile((d + 12) as *mut u16, flags);
            write_volatile((d + 14) as *mut u16, 0);
        }
    }

    /// Offers buffer `id` to the card.
    fn offer(&mut self, id: u16) {
        let ring = self.avail.at::<u16>(4 + 2 * (self.offered % self.size) as usize);
        unsafe { write_volatile(ring, id) };
        self.offered = self.offered.wrapping_add(1);
        fence(Ordering::SeqCst);
        unsafe { write_volatile(self.avail.at::<u16>(2), self.offered) };
        fence(Ordering::SeqCst);
    }

    fn kick(&self) {
        w16(self.notify, self.index);
    }

    /// The next buffer the card has handed back: its id and how much it wrote.
    fn take(&mut self) -> Option<(u16, u32)> {
        fence(Ordering::SeqCst);
        let handed = unsafe { read_volatile(self.used.at::<u16>(2)) };
        if handed == self.seen {
            return None;
        }
        let at = 4 + 8 * (self.seen % self.size) as usize;
        let (id, len) = unsafe { (read_volatile(self.used.at::<u32>(at)), read_volatile(self.used.at::<u32>(at + 4))) };
        self.seen = self.seen.wrapping_add(1);
        (id < self.size as u32).then_some((id as u16, len))
    }
}

pub struct Virtio {
    at: pci::Address,
    device: u16,
    common: u64,
    device_cfg: Option<u64>,
    features: u64,
    mac: [u8; 6],
    rx: Queue,
    tx: Queue,
    pub signal: pci::Signal,
}

unsafe impl Send for Virtio {}

/// Where a capability's region is, mapped: (address, length).
fn region(f: &pci::Function, cap: u16) -> Option<(u64, u32)> {
    let bar = f.at.read8(cap + 4);
    let offset = f.at.read32(cap + 8);
    let length = f.at.read32(cap + 12);
    if bar > 5 || length == 0 {
        return None;
    }
    let b = f.at.memory_bar(bar)?;
    if offset as u64 + length as u64 > b.size {
        return None;
    }
    Some((paging::map_device(b.phys + offset as u64, length as u64, Cache::Uncached), length))
}

impl Virtio {
    pub fn start(f: &pci::Function, vector: u8) -> Result<Virtio, String> {
        let (mut common, mut notify, mut device_cfg) = (None, None, None);
        for cap in f.at.capabilities(CAP_VENDOR) {
            match f.at.read8(cap + 3) {
                CFG_COMMON if common.is_none() => common = region(f, cap),
                CFG_NOTIFY if notify.is_none() => notify = region(f, cap).map(|r| (r.0, f.at.read32(cap + 16))),
                CFG_DEVICE if device_cfg.is_none() => device_cfg = region(f, cap).map(|r| r.0),
                _ => {}
            }
        }
        let (Some((common, _)), Some((notify, multiplier))) = (common, notify) else {
            return Err(String::from("it has no virtio 1.0 registers (only the old interface, which Chat Computer doesn't use)"));
        };
        f.at.enable(true);
        // Reset, then say hello.
        w8(common + STATUS, 0);
        for _ in 0..1000 {
            if r8(common + STATUS) == 0 {
                break;
            }
            crate::time::delay_us(100);
        }
        w8(common + STATUS, ACKNOWLEDGE);
        w8(common + STATUS, ACKNOWLEDGE | DRIVER);
        w32(common + DEVICE_FEATURE_SELECT, 0);
        let low = r32(common + DEVICE_FEATURE) as u64;
        w32(common + DEVICE_FEATURE_SELECT, 1);
        let offered = low | (r32(common + DEVICE_FEATURE) as u64) << 32;
        if offered & F_VERSION_1 == 0 {
            w8(common + STATUS, FAILED);
            return Err(String::from("it isn't a virtio 1.0 card"));
        }
        let features = offered & (F_VERSION_1 | F_MAC | F_STATUS);
        w32(common + DRIVER_FEATURE_SELECT, 0);
        w32(common + DRIVER_FEATURE, features as u32);
        w32(common + DRIVER_FEATURE_SELECT, 1);
        w32(common + DRIVER_FEATURE, (features >> 32) as u32);
        w8(common + STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK);
        if r8(common + STATUS) & FEATURES_OK == 0 {
            w8(common + STATUS, FAILED);
            return Err(String::from("it didn't accept the features asked for"));
        }
        // Interrupts by message, for frames that arrive (not for frames sent: those are looked at when sending).
        let signal = f.at.signal_messages(vector, crate::apic::id());
        w16(common + CONFIG_MSIX, NO_VECTOR);
        let mut queues = Vec::new();
        for index in 0..2u16 {
            w16(common + QUEUE_SELECT, index);
            let max = r16(common + QUEUE_SIZE);
            if max == 0 {
                w8(common + STATUS, FAILED);
                return Err(format!("it has no queue {index}"));
            }
            // A power of two, at most QUEUE_MAX (the card's size can be any number up to 32768).
            let mut size = 1u16;
            while size * 2 <= max.min(QUEUE_MAX) {
                size *= 2;
            }
            w16(common + QUEUE_SIZE, size);
            let page = || Page::new(false).ok_or_else(|| String::from("out of memory"));
            let (desc, avail, used) = (page()?, page()?, page()?);
            let mut buffers = Vec::new();
            for _ in 0..size.div_ceil(2) {
                buffers.push(page()?);
            }
            let notify_at = notify + r16(common + QUEUE_NOTIFY_OFF) as u64 * multiplier as u64;
            w64(common + QUEUE_DESC, desc.phys);
            w64(common + QUEUE_DRIVER, avail.phys);
            w64(common + QUEUE_DEVICE, used.phys);
            let wanted = if index == 0 && signal == pci::Signal::MsiX { 0 } else { NO_VECTOR };
            w16(common + QUEUE_MSIX, wanted);
            w16(common + QUEUE_ENABLE, 1);
            queues.push(Queue { size, desc, avail, used, buffers, notify: notify_at, index, offered: 0, seen: 0, free: Vec::new() });
        }
        let tx = queues.pop().unwrap();
        let mut rx = queues.pop().unwrap();
        let mut mac = [0u8; 6];
        if features & F_MAC != 0
            && let Some(cfg) = device_cfg
        {
            for (i, b) in mac.iter_mut().enumerate() {
                *b = r8(cfg + i as u64);
            }
        } else {
            // No address of its own: a made-up one, marked as locally given.
            let r = crate::random::u64().to_le_bytes();
            mac = [0x02, r[1], r[2], r[3], r[4], r[5]];
        }
        let mut tx = tx;
        for id in 0..rx.size {
            let (phys, _) = rx.buffer(id);
            rx.set_desc(id, phys, BUF as u32, DESC_WRITE);
            rx.offer(id);
        }
        tx.free = (0..tx.size).rev().collect();
        w8(common + STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK | DRIVER_OK);
        rx.kick();
        Ok(Virtio { at: f.at, device: f.device, common, device_cfg, features, mac, rx, tx, signal })
    }
}

impl Card for Virtio {
    fn describe(&self) -> String {
        format!(
            "virtio network card {:04x}:{:04x} at {}, address {}, interrupts {}",
            VENDOR,
            self.device,
            self.at,
            net::Mac(self.mac),
            match self.signal {
                pci::Signal::MsiX => "by MSI-X",
                _ => "off (it is checked for frames instead)",
            }
        )
    }

    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn link_up(&self) -> bool {
        match self.device_cfg {
            Some(cfg) if self.features & F_STATUS != 0 => r16(cfg + 6) & 1 != 0,
            _ => true,
        }
    }

    fn send(&mut self, frame: &[u8]) -> bool {
        if frame.len() + HEADER > BUF || r8(self.common + STATUS) & FAILED != 0 {
            return false;
        }
        while let Some((id, _)) = self.tx.take() {
            self.tx.free.push(id);
        }
        let Some(id) = self.tx.free.pop() else { return false };
        let (phys, at) = self.tx.buffer(id);
        unsafe {
            core::ptr::write_bytes(at, 0, HEADER);
            core::ptr::copy_nonoverlapping(frame.as_ptr(), at.add(HEADER), frame.len());
        }
        self.tx.set_desc(id, phys, (HEADER + frame.len()) as u32, 0);
        self.tx.offer(id);
        self.tx.kick();
        true
    }

    fn receive(&mut self, took: &mut dyn FnMut(&[u8])) -> usize {
        let mut n = 0;
        let mut offered = false;
        while let Some((id, len)) = self.rx.take() {
            let len = (len as usize).min(BUF);
            if len > HEADER {
                let (_, at) = self.rx.buffer(id);
                let frame = unsafe { core::slice::from_raw_parts(at.add(HEADER), len - HEADER) };
                took(frame);
                n += 1;
            }
            self.rx.offer(id); // the buffer, empty again
            offered = true;
            if n >= 64 {
                break;
            }
        }
        if offered {
            self.rx.kick();
        }
        n
    }
}
