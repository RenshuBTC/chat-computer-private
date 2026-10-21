//! Realtek's PCI Express network chips: the RTL8111/8168 family (gigabit, from the 8111E of 2010 to the 8111H and
//! 8111FP of today: the most common wired chip on PCs' motherboards, AMD ones especially) and the RTL8125 family
//! (2.5 Gb/s, including Killer's E3000). Realtek publishes no datasheet for these, so this is written from the one it
//! published for the RTL8111B/8168B (the registers and descriptors are the same) and from what the open-source
//! drivers that run them agree on: Linux's r8169, Realtek's own r8125, FreeBSD's re and iPXE's. None of it has been
//! tried on a real chip yet (QEMU has no model of them), and the log says so.
//!
//! Every chip of the family works the same way at heart: two rings of 16-byte descriptors, one of empty buffers it
//! fills with the frames that arrive, one of frames to send, each descriptor handed to the chip by its OWN bit, the
//! last marked "end of ring". Which chip it is comes from the version in its transmit configuration register (the
//! "XID"), not from its PCI id. What differs from chip to chip is the setting up: the 8125 and 8111G and later have
//! a microcontroller to talk the chip out of the state a firmware's or Windows' driver left it in (Linux's
//! sequence, followed here), and the 8125 keeps its interrupt registers and "send now" register elsewhere.
//!
//! Left out so far: the PCI Express PHY's tuning tables, the PHY firmware patches and the PHY parameter writes Linux
//! applies to each chip (Linux works without the firmware; its maintainer says it is for compatibility on some
//! systems; FreeBSD's driver makes none of the writes on the 8111E to 8111H), checksum offloads, and the DASH
//! management variants' handshake with their management processor.

use super::Card;
use crate::dma::Page;
use crate::paging::{self, Cache};
use crate::{pci, time};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

pub const VENDOR: u16 = 0x10EC;

/// PCI ids of the family (the chip itself is told by its XID).
const DEVICES: &[u16] = &[
    0x8168, // RTL8111/8168/8411
    0x8161, // RTL8111/8168/8411
    0x2502, // Killer E2500
    0x2600, // Killer E2600
    0x8125, // RTL8125
    0x3000, // Killer E3000 (an RTL8125)
    0x8162, // RTL8168KB (an RTL8125 inside, Realtek's driver says)
    0x8136, // RTL810x: only the ones with an 8111G's or 8111H's insides and a 100 Mb/s PHY
];

/// Whether it's one of these (and an Ethernet controller).
pub fn ours(f: &pci::Function) -> bool {
    f.vendor == VENDOR && f.class == 0x02 && f.subclass == 0x00 && DEVICES.contains(&f.device)
}

/// The chips, by XID (bits 31:20 of TxConfig without the writable 25:24): mask, value, Linux's version number for
/// the chip (the setting-up below goes by it), and its name. In Linux's order: the first match counts.
const XIDS: &[(u16, u16, u8, &str)] = &[
    (0x7CF, 0x681, 66, "RTL8125BP"),
    (0x7CF, 0x708, 65, "RTL8125CP"),
    (0x7CF, 0x68B, 64, "RTL9151A"),
    (0x7CF, 0x68A, 64, "RTL8125K"),
    (0x7CF, 0x689, 64, "RTL8125D"),
    (0x7CF, 0x688, 64, "RTL8125D"),
    (0x7CF, 0x641, 63, "RTL8125B"),
    (0x7CF, 0x640, 63, "RTL8125B"), // the first stepping (Realtek's driver knows it; Linux no longer does)
    (0x7CF, 0x609, 61, "RTL8125A"),
    (0x7CF, 0x608, 61, "RTL8125A"), // the first stepping
    (0x7CF, 0x54B, 52, "RTL8168FP"),
    (0x7CF, 0x54A, 52, "RTL8117"),
    (0x7CF, 0x502, 51, "RTL8111EP"),
    (0x7CF, 0x541, 46, "RTL8111H"),
    (0x7CF, 0x6C0, 46, "RTL8168M"),
    (0x7CF, 0x5C8, 44, "RTL8411B"),
    (0x7CF, 0x509, 42, "RTL8111GU"),
    (0x7CF, 0x4C0, 40, "RTL8111G"),
    (0x7C8, 0x488, 38, "RTL8411"),
    (0x7CF, 0x481, 36, "RTL8111F"),
    (0x7CF, 0x480, 35, "RTL8111F"),
    (0x7C8, 0x2C8, 34, "RTL8111E-VL"),
    (0x7CF, 0x2C1, 32, "RTL8111E"),
    (0x7C8, 0x2C0, 33, "RTL8111E"),
];

// Registers (offsets in the memory region), with their widths.
const MAC0: u64 = 0x00; // 32: the address's first four bytes (written whole, with the configuration unlocked)
const MAC4: u64 = 0x04; // 32: its last two
const MAR0: u64 = 0x08; // 32 + 32: the multicast filter
const TX_RING: u64 = 0x20; // 32 + 32: the send ring's address, low then high
const INT_CFG0_8125: u64 = 0x34; // 8
const CHIP_CMD: u64 = 0x37; // 8
const TX_POLL_8168: u64 = 0x38; // 8: "send now" (the 8125 has its interrupt mask here)
const IMR_8125: u64 = 0x38; // 32
const ISR_8125: u64 = 0x3C; // 32
const IMR_8168: u64 = 0x3C; // 16
const ISR_8168: u64 = 0x3E; // 16
const TX_CONFIG: u64 = 0x40; // 32
const RX_CONFIG: u64 = 0x44; // 32
const CFG9346: u64 = 0x50; // 8: unlocks the configuration registers and the address
const CONFIG1: u64 = 0x52;
const CONFIG2: u64 = 0x53;
const CONFIG3: u64 = 0x54;
const CONFIG5: u64 = 0x56;
const PHYAR: u64 = 0x60; // 32: the PHY's registers, before the 8111G
const PHY_STATUS: u64 = 0x6C; // 8 (16 on the 8125)
const ERIDR: u64 = 0x70; // 32: the extended registers' window (8111E and later)
const ERIAR: u64 = 0x74;
const INT_CFG1_8125: u64 = 0x7A; // 16
const TX_POLL_8125: u64 = 0x90; // 16
const MAC_OCP: u64 = 0xB0; // 32: the microcontroller's registers (8111G and later, 8125)
const PHY_OCP: u64 = 0xB8; // 32: the PHY's registers (8111G and later, 8125)
const MCU: u64 = 0xD3; // 8
const RX_MAX_SIZE: u64 = 0xDA; // 16
const CPLUS_CMD: u64 = 0xE0; // 16, and only written whole
const INTR_MITIGATE: u64 = 0xE2; // 16
const RX_RING: u64 = 0xE4; // 32 + 32
const MAX_TX_PACKET: u64 = 0xEC; // 8 (8168)
const MISC: u64 = 0xF0; // 32
const SW_PATCH_8125_382: u64 = 0x382; // 16 (Linux: 0x221B)
const SW_PATCH_8125_1880: u64 = 0x1880; // 16
const MAC0_BKP_8125: u64 = 0x19E0; // 8 x 6: the 8125's factory address
const RSS_CTRL_8125: u64 = 0x4500; // 32
const Q_NUM_CTRL_8125: u64 = 0x4800; // 16

const CMD_STOP_REQ: u8 = 1 << 7;
const CMD_RESET: u8 = 1 << 4;
const CMD_RX_ENABLE: u8 = 1 << 3;
const CMD_TX_ENABLE: u8 = 1 << 2;
const MCU_NOW_IS_OOB: u8 = 1 << 7;
const MCU_TX_EMPTY: u8 = 1 << 5;
const MCU_RX_EMPTY: u8 = 1 << 4;
const MCU_LINK_LIST_READY: u8 = 1 << 1;
const MISC_RXDV_GATED: u32 = 1 << 19;
const TXCFG_EMPTY: u32 = 1 << 11;
const TXCFG_AUTO_FIFO: u32 = 1 << 7;
/// TxConfig: the standard gap between frames (3) and unlimited DMA bursts (7).
const TXCFG: u32 = 3 << 24 | 7 << 8;
/// RxConfig: accept broadcasts (AB) and frames to this address (APM).
const RX_ACCEPT: u32 = 1 << 3 | 1 << 1;
const LOCK: u8 = 0x00;
const UNLOCK: u8 = 0xC0;
const PHY_LINK: u16 = 1 << 1;

// Interrupts: received, receive error, sent, send error, link changed.
const INT_MASK: u32 = 1 << 0 | 1 << 1 | 1 << 2 | 1 << 3 | 1 << 5;

// Descriptor bits (the first word, "opts1").
const OWN: u32 = 1 << 31;
const EOR: u32 = 1 << 30;
const FS: u32 = 1 << 29;
const LS: u32 = 1 << 28;
const RX_RES: u32 = 1 << 21;
const RX_LENGTH: u32 = 0x3FFF;

// The PHY's registers (IEEE 802.3, clause 22) and the bits used; on the chips with the PHY's registers in the
// microcontroller's space, standard register n is at 0xA400 + 2n.
const BMCR: u32 = 0;
const ANAR: u32 = 4;
const GBCR: u32 = 9; // 1000BASE-T control
const PAGE: u32 = 0x1F; // (PHYAR chips: the PHY's register page)
const BMCR_RESET: u16 = 1 << 15;
const BMCR_LOOPBACK: u16 = 1 << 14;
const BMCR_AN_ENABLE: u16 = 1 << 12;
const BMCR_POWER_DOWN: u16 = 1 << 11;
const BMCR_ISOLATE: u16 = 1 << 10;
const BMCR_AN_RESTART: u16 = 1 << 9;
/// What a driver before (or a firmware) may have left set in the PHY's control register: a reset under way, powered
/// down, cut off from the chip, looped back.
const BMCR_LEFT: u16 = BMCR_RESET | BMCR_POWER_DOWN | BMCR_ISOLATE | BMCR_LOOPBACK;
const ADVERTISE_10_100: u16 = 0x01E1;
const ADVERTISE_1000_FULL: u16 = 1 << 9;
const OCP_STANDARD: u16 = 0xA400;
const OCP_ALDPS: u16 = 0xA430; // bit 2: the PHY's link-down power saving, which Linux switches off
const OCP_ADVERTISE_2500: u16 = 0xA5D4; // bit 7
/// Energy Efficient Ethernet offered: bits 1 and 2 at 100 and 1000 Mb/s; on the 8125, bit 0 of the other at 2.5 Gb/s
/// (the clause 45 registers' copies Linux's Realtek PHY driver uses).
const OCP_EEE_ADVERTISE: u16 = 0xA5D0;
const OCP_EEE_ADVERTISE_2500: u16 = 0xA6D4;

/// Descriptors per ring, and the size of each buffer (a frame and its checksum must fit in one).
const RING: usize = 64;
const BUF: usize = 2048;

struct Ring {
    desc: Page,
    buffers: Vec<Page>,
}

impl Ring {
    fn new() -> Option<Ring> {
        let desc = Page::new(true)?; // (page-aligned; the chip wants 256 bytes)
        let mut buffers = Vec::new();
        for _ in 0..RING / 2 {
            buffers.push(Page::new(true)?);
        }
        Some(Ring { desc, buffers })
    }

    fn buffer(&self, i: usize) -> (u64, *mut u8) {
        let page = &self.buffers[i / 2];
        let offset = (i % 2) * BUF;
        (page.phys + offset as u64, page.at::<u8>(offset))
    }

    fn desc(&self, i: usize) -> *mut u8 {
        self.desc.at::<u8>(16 * i)
    }
}

pub struct Realtek {
    at: pci::Address,
    device: u16,
    name: &'static str,
    /// Linux's version number for the chip.
    version: u8,
    regs: u64,
    mac: [u8; 6],
    rx: Ring,
    tx: Ring,
    rx_next: usize,
    tx_tail: usize,
    tx_clean: usize,
    pub signal: pci::Signal,
}

unsafe impl Send for Realtek {}

impl Realtek {
    fn is_8125(&self) -> bool {
        self.version >= 61
    }

    /// The 8111G and later (and the 8125): PHY and microcontroller registers through the OCP windows.
    fn has_ocp(&self) -> bool {
        self.version >= 40
    }

    fn r8(&self, reg: u64) -> u8 {
        unsafe { read_volatile((self.regs + reg) as *const u8) }
    }
    fn r16(&self, reg: u64) -> u16 {
        unsafe { read_volatile((self.regs + reg) as *const u16) }
    }
    fn r32(&self, reg: u64) -> u32 {
        unsafe { read_volatile((self.regs + reg) as *const u32) }
    }
    fn w8(&self, reg: u64, v: u8) {
        unsafe { write_volatile((self.regs + reg) as *mut u8, v) }
    }
    fn w16(&self, reg: u64, v: u16) {
        unsafe { write_volatile((self.regs + reg) as *mut u16, v) }
    }
    fn w32(&self, reg: u64, v: u32) {
        unsafe { write_volatile((self.regs + reg) as *mut u32, v) }
    }

    /// Checks `done` every `us` microseconds, `times` times.
    fn poll(&self, us: u64, times: u32, done: impl Fn(&Self) -> bool) -> bool {
        for _ in 0..times {
            if done(self) {
                return true;
            }
            time::delay_us(us);
        }
        done(self)
    }

    // ------------------------------------------------------------------------------------------- the windows

    fn mac_ocp_read(&self, addr: u16) -> u16 {
        self.w32(MAC_OCP, (addr as u32) << 15);
        self.r32(MAC_OCP) as u16
    }

    fn mac_ocp_write(&self, addr: u16, value: u16) {
        self.w32(MAC_OCP, 1 << 31 | (addr as u32) << 15 | value as u32);
    }

    /// A microcontroller register: `mask` cleared, `set` set.
    fn mac_ocp_modify(&self, addr: u16, mask: u16, set: u16) {
        let v = self.mac_ocp_read(addr);
        self.mac_ocp_write(addr, (v & !mask) | set);
    }

    fn phy_ocp_read(&self, addr: u16) -> Option<u16> {
        self.w32(PHY_OCP, (addr as u32) << 15);
        self.poll(25, 10, |s| s.r32(PHY_OCP) & 1 << 31 != 0).then(|| self.r32(PHY_OCP) as u16)
    }

    fn phy_ocp_write(&self, addr: u16, value: u16) -> bool {
        self.w32(PHY_OCP, 1 << 31 | (addr as u32) << 15 | value as u32);
        self.poll(25, 10, |s| s.r32(PHY_OCP) & 1 << 31 == 0)
    }

    /// A standard PHY register: through the OCP window, or (before the 8111G) PHYAR, which wants 20 µs more after the
    /// chip says it's done ("according to hardware specs", Linux says).
    fn phy_read(&self, reg: u32) -> Option<u16> {
        if self.has_ocp() {
            return self.phy_ocp_read(OCP_STANDARD + 2 * reg as u16);
        }
        self.w32(PHYAR, (reg & 0x1F) << 16);
        let done = self.poll(25, 20, |s| s.r32(PHYAR) & 1 << 31 != 0);
        time::delay_us(20);
        done.then(|| self.r32(PHYAR) as u16)
    }

    fn phy_write(&self, reg: u32, value: u16) -> bool {
        if self.has_ocp() {
            return self.phy_ocp_write(OCP_STANDARD + 2 * reg as u16, value);
        }
        self.w32(PHYAR, 1 << 31 | (reg & 0x1F) << 16 | value as u32);
        let done = self.poll(25, 20, |s| s.r32(PHYAR) & 1 << 31 == 0);
        time::delay_us(20);
        done
    }

    fn eri_read(&self, addr: u16) -> Option<u32> {
        self.w32(ERIAR, 0xF << 12 | addr as u32);
        self.poll(100, 100, |s| s.r32(ERIAR) & 1 << 31 != 0).then(|| self.r32(ERIDR))
    }

    /// An extended register: `value`'s bytes that `bytes` (a 4-bit mask) picks.
    fn eri_write(&self, addr: u16, bytes: u32, value: u32) -> bool {
        self.w32(ERIDR, value);
        self.w32(ERIAR, 1 << 31 | bytes << 12 | addr as u32);
        self.poll(100, 100, |s| s.r32(ERIAR) & 1 << 31 == 0)
    }

    fn link_list_ready(&self) -> bool {
        self.poll(100, 42, |s| s.r8(MCU) & MCU_LINK_LIST_READY != 0)
    }

    fn fifos_empty(&self) -> bool {
        self.poll(100, 42, |s| s.r8(MCU) & (MCU_TX_EMPTY | MCU_RX_EMPTY) == MCU_TX_EMPTY | MCU_RX_EMPTY)
    }

    // ------------------------------------------------------------------------------------------------- start

    /// RxConfig without the "accept" bits: Linux's for each chip (writing all of it also clears bit 24, which a
    /// warm restart from Windows can leave set, with garbage received after it, iPXE found).
    fn rx_config_base(&self) -> u32 {
        match self.version {
            61 => 0x4000_0700,
            v if v > 61 => 0x4000_0F00,
            40..=52 => 0x0000_CF00,
            34..=38 => 0x0000_C700,
            _ => 0x0000_8700,
        }
    }

    fn mask_and_ack(&self) {
        if self.is_8125() {
            self.w32(IMR_8125, 0);
            self.w32(ISR_8125, !0);
        } else {
            self.w16(IMR_8168, 0);
            self.w16(ISR_8168, !0);
        }
        let _ = self.r8(CHIP_CMD);
    }

    /// Talks the chip's microcontroller out of what a firmware's or Windows' driver left (Linux's "hw_init"): no
    /// frames in (the RXDV gate) and the FIFOs drained, sending and receiving off, out of management mode, and the
    /// microcontroller's handshake. False if it didn't answer as it should (which isn't fatal).
    fn hw_init(&self) -> bool {
        let mut ok = true;
        self.w32(MISC, self.r32(MISC) | MISC_RXDV_GATED);
        time::delay_ms(2);
        if self.version == 61 {
            ok &= self.fifos_empty();
        } else if self.is_8125() {
            self.w8(CHIP_CMD, self.r8(CHIP_CMD) | CMD_STOP_REQ);
            ok &= self.fifos_empty();
            ok &= self.poll(100, 42, |s| s.r16(INTR_MITIGATE) & 0x0103 == 0x0103);
        } else {
            ok &= self.poll(100, 42, |s| s.r32(TX_CONFIG) & TXCFG_EMPTY != 0);
            ok &= self.fifos_empty();
        }
        self.w8(CHIP_CMD, self.r8(CHIP_CMD) & !(CMD_TX_ENABLE | CMD_RX_ENABLE));
        time::delay_ms(1);
        self.w8(MCU, self.r8(MCU) & !MCU_NOW_IS_OOB);
        self.mac_ocp_modify(0xE8DE, 1 << 14, 0);
        ok &= self.link_list_ready();
        if self.is_8125() {
            self.mac_ocp_write(0xC0AA, 0x07D0);
            self.mac_ocp_write(0xC0A6, 0x0150);
            self.mac_ocp_write(0xC01E, 0x5555);
        } else {
            self.mac_ocp_modify(0xE8DE, 0, 1 << 15);
        }
        ok &= self.link_list_ready();
        ok
    }

    /// The chips before the 8111G: sending stopped before the reset (Linux's).
    fn stop_old(&self) {
        self.w8(CHIP_CMD, self.r8(CHIP_CMD) | CMD_STOP_REQ);
        if self.version >= 34 {
            self.poll(100, 666, |s| s.r32(TX_CONFIG) & TXCFG_EMPTY != 0);
        } else {
            time::delay_us(100);
        }
    }

    /// The chip's own address: the 8125's factory copy, or (8111F and later) the extended registers' copy, else the
    /// one in its address registers; else a made-up one.
    fn read_mac(&self) -> ([u8; 6], bool) {
        let valid = |m: &[u8; 6]| *m != [0; 6] && m[0] & 1 == 0;
        let mut m = [0u8; 6];
        if self.is_8125() {
            for (i, b) in m.iter_mut().enumerate() {
                *b = self.r8(MAC0_BKP_8125 + i as u64);
            }
        } else if self.version >= 35
            && let (Some(low), Some(high)) = (self.eri_read(0xE0), self.eri_read(0xE4))
        {
            m = [low as u8, (low >> 8) as u8, (low >> 16) as u8, (low >> 24) as u8, high as u8, (high >> 8) as u8];
        }
        if valid(&m) {
            return (m, true);
        }
        let (low, high) = (self.r32(MAC0), self.r32(MAC4));
        m = [low as u8, (low >> 8) as u8, (low >> 16) as u8, (low >> 24) as u8, high as u8, (high >> 8) as u8];
        if valid(&m) {
            return (m, true);
        }
        let r = crate::random::u64().to_le_bytes();
        ([0x02, r[1], r[2], r[3], r[4], r[5]], false)
    }

    /// The PHY: awake (a driver before may have powered it down, and a restart without a power cut leaves it so; then
    /// a moment, as "internal PHY's from RTL8168h up may not be instantly ready", Linux says), its link-down power
    /// saving off (Linux), reset, Energy Efficient Ethernet not offered, then negotiating with the other end, offering
    /// every speed it has (1000 Mb/s unless it's a 100 Mb/s PHY; 2500 on the 8125). False if it didn't answer or
    /// didn't finish its reset (it's set going all the same).
    fn phy_up(&self, gigabit: bool) -> bool {
        if !self.has_ocp() {
            self.phy_write(PAGE, 0);
        }
        let Some(control) = self.phy_read(BMCR) else { return false };
        self.phy_write(BMCR, control & !BMCR_LEFT);
        time::delay_ms(20);
        if self.has_ocp()
            && let Some(v) = self.phy_ocp_read(OCP_ALDPS)
        {
            self.phy_ocp_write(OCP_ALDPS, v & !(1 << 2));
        }
        self.phy_write(BMCR, BMCR_RESET | BMCR_AN_ENABLE);
        let reset = (0..60).any(|_| {
            time::delay_ms(10);
            self.phy_read(BMCR).is_some_and(|v| v & BMCR_RESET == 0)
        });
        self.eee_off();
        let ok = self.phy_write(ANAR, ADVERTISE_10_100)
            && self.phy_write(GBCR, if gigabit { ADVERTISE_1000_FULL } else { 0 })
            && (!self.is_8125() || self.phy_ocp_read(OCP_ADVERTISE_2500).is_some_and(|v| self.phy_ocp_write(OCP_ADVERTISE_2500, v | 1 << 7)));
        let control = self.phy_read(BMCR).unwrap_or(0);
        let going = self.phy_write(BMCR, (control & !BMCR_LEFT) | BMCR_AN_ENABLE | BMCR_AN_RESTART);
        reset && ok && going
    }

    /// Energy Efficient Ethernet off, as Realtek's own driver leaves it (Linux leaves it off on the 8111H, and at
    /// 2.5 Gb/s on the 8125A): with some switches it drops the link or stalls. The PHY doesn't offer it (8111G and
    /// later), and the chip never asks for the line's low-power idle when sending (Linux's register for each chip).
    fn eee_off(&self) {
        if self.has_ocp()
            && let Some(v) = self.phy_ocp_read(OCP_EEE_ADVERTISE)
        {
            self.phy_ocp_write(OCP_EEE_ADVERTISE, v & !0x0006);
        }
        if self.is_8125()
            && let Some(v) = self.phy_ocp_read(OCP_EEE_ADVERTISE_2500)
        {
            self.phy_ocp_write(OCP_EEE_ADVERTISE_2500, v & !0x0001);
        }
        match self.version {
            34..=52 => {
                if let Some(v) = self.eri_read(0x1B0) {
                    self.eri_write(0x1B0, 0xF, v & !0x0003);
                }
            }
            61 => {
                self.mac_ocp_modify(0xE040, 0x0003, 0);
                self.mac_ocp_modify(0xEB62, 0x0006, 0);
            }
            _ if self.is_8125() => self.mac_ocp_modify(0xE040, 0x0003, 0),
            _ => {}
        }
    }

    /// The 8125's common setting up (Linux's "hw_start_8125_common", written to the microcontroller's registers;
    /// Realtek documents none of it). The parts that matter most: no RSS and one queue, the descriptor format this
    /// driver uses (MAC OCP 0xEB58 bit 0 off), and frames let in (the RXDV gate off).
    fn start_8125_common(&self) {
        let b = self.version == 63;
        self.w8(CONFIG3, self.r8(CONFIG3) & !(1 << 1));
        self.w16(SW_PATCH_8125_382, 0x221B);
        self.w32(RSS_CTRL_8125, 0);
        self.w16(Q_NUM_CTRL_8125, 0);
        self.mac_ocp_modify(0xD40A, 0x0010, 0);
        self.w8(CONFIG1, self.r8(CONFIG1) & !0x10);
        self.mac_ocp_write(0xC140, 0xFFFF);
        self.mac_ocp_write(0xC142, 0xFFFF);
        self.mac_ocp_modify(0xD3E2, 0x0FFF, 0x03A9);
        self.mac_ocp_modify(0xD3E4, 0x00FF, 0x0000);
        self.mac_ocp_modify(0xE860, 0x0000, 0x0080);
        self.mac_ocp_modify(0xEB58, 0x0001, 0x0000);
        self.mac_ocp_modify(0xE614, 0x0700, if b { 0x0200 } else { 0x0300 });
        if b {
            self.mac_ocp_modify(0xE63E, 0x0C30, 0x0000);
        } else {
            self.mac_ocp_modify(0xE63E, 0x0C30, 0x0020);
        }
        self.mac_ocp_modify(0xC0B4, 0x0000, 0x000C);
        self.mac_ocp_modify(0xEB6A, 0x00FF, 0x0033);
        self.mac_ocp_modify(0xEB50, 0x03E0, 0x0040);
        self.mac_ocp_modify(0xE056, 0x00F0, 0x0000);
        self.mac_ocp_modify(0xE040, 0x1000, 0x0000);
        self.mac_ocp_modify(0xEA1C, 0x0003, 0x0001);
        self.mac_ocp_modify(0xEA1C, 0x0004, 0x0000);
        self.mac_ocp_modify(0xE0C0, 0x4F0F, 0x4403);
        self.mac_ocp_modify(0xE052, 0x0080, 0x0068);
        self.mac_ocp_modify(0xD430, 0x0FFF, 0x047F);
        self.mac_ocp_modify(0xEA1C, 0x0004, 0x0000);
        self.mac_ocp_modify(0xEB54, 0x0000, 0x0001);
        time::delay_us(1);
        self.mac_ocp_modify(0xEB54, 0x0001, 0x0000);
        self.w16(SW_PATCH_8125_1880, self.r16(SW_PATCH_8125_1880) & !0x0030);
        self.mac_ocp_write(0xE098, 0xC302);
        self.poll(1000, 10, |s| s.mac_ocp_read(0xE00E) & 1 << 13 == 0);
        self.w32(MISC, self.r32(MISC) & !MISC_RXDV_GATED);
    }

    pub fn start(f: &pci::Function, vector: u8) -> Result<Realtek, String> {
        f.at.power_on();
        let bar = (0..6).find_map(|i| f.at.memory_bar(i)).ok_or_else(|| String::from("its registers aren't in memory"))?;
        if bar.size < 0x100 {
            return Err(String::from("its register region is too small"));
        }
        let regs = paging::map_device(bar.phys, bar.size.min(0x10000), Cache::Uncached);
        // Its registers on; its use of memory (bus mastering) only once it has been reset, below, so nothing a system
        // before left running can write into memory that's now Chat Computer's (Linux's order too).
        f.at.enable(false);
        let tx_config = unsafe { read_volatile((regs + TX_CONFIG) as *const u32) };
        if tx_config == !0 {
            return Err(String::from("it doesn't answer"));
        }
        let xid = ((tx_config >> 20) & 0xFCF) as u16;
        let Some(&(_, _, mut version, mut name)) = XIDS.iter().find(|&&(mask, value, _, _)| xid & mask == value) else {
            return Err(format!("its version ({xid:#05x}) isn't one this driver knows yet"));
        };
        // A 100 Mb/s chip on the RTL810x's id is an 8111GU's or 8111H's insides with a 100 Mb/s PHY (Linux).
        let gigabit = f.device != 0x8136;
        if !gigabit {
            (version, name) = match version {
                42 => (42, "RTL8106eus"),
                46 => (46, "RTL8107e"),
                _ => return Err(format!("it's a 100 Mb/s RTL810x (version {xid:#05x}), which this driver doesn't know yet")),
            };
        }
        if version >= 61 && bar.size < 0x7000 {
            return Err(String::from("its register region is too small for an RTL8125"));
        }
        let (rx, tx) = match (Ring::new(), Ring::new()) {
            (Some(rx), Some(tx)) => (rx, tx),
            _ => return Err(String::from("out of memory")),
        };
        let mut card = Realtek {
            at: f.at,
            device: f.device,
            name,
            version,
            regs,
            mac: [0; 6],
            rx,
            tx,
            rx_next: 0,
            tx_tail: 0,
            tx_clean: 0,
            signal: pci::Signal::None,
        };
        let mut notes = String::new();

        // The PCI Express link's power saving off: Linux switches L1 off on these chips because of "random device
        // stop working problems as well as full system hangs".
        f.at.disable_aspm();

        // Quiet: frames no longer accepted, no interrupts; the microcontroller's handshake (or, on older chips,
        // sending stopped); then a reset, which empties the FIFOs and takes the chip back to its rings' starts (and
        // keeps its address and multicast filter).
        card.w32(RX_CONFIG, card.rx_config_base());
        card.mask_and_ack();
        if card.has_ocp() {
            if !card.hw_init() {
                notes.push_str("; its microcontroller didn't answer as expected");
            }
            if (51..=52).contains(&version) || version == 66 {
                notes.push_str("; it has a DASH management processor, which this driver doesn't talk to yet");
            }
        } else {
            card.stop_old();
        }
        card.w8(CHIP_CMD, CMD_RESET);
        if !card.poll(100, 100, |s| s.r8(CHIP_CMD) & CMD_RESET == 0) {
            return Err(String::from("it didn't come back from a reset"));
        }
        f.at.enable(true);

        // Its address, written back (another system may have left another one in its address registers).
        let (mac, known) = card.read_mac();
        card.mac = mac;
        if !known {
            notes.push_str("; no address of its own found, so a made-up one");
        }
        card.w8(CFG9346, UNLOCK);
        card.w32(MAC4, u16::from_le_bytes([mac[4], mac[5]]) as u32);
        let _ = card.r32(MAC4);
        card.w32(MAC0, u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]));
        let _ = card.r32(MAC0);
        card.w8(CFG9346, LOCK);

        // The PHY.
        if !card.phy_up(gigabit) {
            notes.push_str("; its PHY didn't answer");
        }

        // The rings: every receive descriptor gets an empty buffer; the last of each ring is marked.
        for i in 0..RING {
            card.give_rx(i);
        }
        unsafe { write_volatile(card.tx.desc(RING - 1) as *mut u32, EOR) };

        // Setting up (Linux's "hw_start"), with the configuration registers unlocked.
        card.w8(CFG9346, UNLOCK);
        // The chip's own power saving off (ASPM and CLKREQ).
        if card.is_8125() || version == 46 {
            card.mac_ocp_modify(0xE092, 0x00FF, 0x0000);
        }
        card.w8(CONFIG2, card.r8(CONFIG2) & !(1 << 7));
        card.w8(CONFIG5, card.r8(CONFIG5) & !(1 << 0));
        // Not "ready for L2/L3" (a PCI Express power state's handshake): "an issue when PCI reset occurs during L2/L3
        // state", Linux says, on the 8411 and the 8111G and later. (The 8125's common setting up does it too.)
        if version == 38 || (40..=52).contains(&version) {
            card.w8(CONFIG3, card.r8(CONFIG3) & !(1 << 1));
        }
        // C+ command: only "normal mode" and the interrupt timer kept (no VLAN stripping, no checksums, no 64-bit DAC
        // bit, which iPXE saw lock some cards up).
        card.w16(CPLUS_CMD, card.r16(CPLUS_CMD) & 0x2003);
        if card.is_8125() {
            card.w8(INT_CFG0_8125, 0); // the interrupt registers this driver uses (not Realtek's "v2" ones)
            // Interrupt moderation off.
            let end = if version == 63 { 0xA80 } else { 0xB00 };
            for reg in (0xA00..end).step_by(4) {
                card.w32(reg, 0);
            }
            if version == 63 {
                card.w16(INT_CFG1_8125, 0);
            }
            card.start_8125_common();
            card.mac_ocp_modify(0xC0AC, 0x0000, 0x1F80); // (leaving L1 when there is traffic)
        } else {
            card.w8(MAX_TX_PACKET, if version >= 34 { 0x27 } else { 0x3F });
            // The FIFOs' sizes, within which the chips from the 8111E-VL on manage them by themselves (TxConfig's
            // automatic FIFO, set below): Linux's values for each chip ("exact meaning of FIFO values isn't known",
            // it says).
            match version {
                34..=38 => {
                    card.eri_write(0xC8, 0xF, 0x0010_0002);
                    card.eri_write(0xE8, 0xF, 0x0010_0006);
                }
                40..=52 => {
                    card.eri_write(0xC8, 0xF, 0x0008_0002);
                    card.eri_write(0xE8, 0xF, 0x0010_0006);
                }
                _ => {}
            }
            if version >= 34 {
                // The packet filter reset (Linux), and out of management mode.
                card.eri_write(0xDC, 0xF, card.eri_read(0xDC).unwrap_or(0) & !1);
                card.eri_write(0xDC, 0xF, card.eri_read(0xDC).unwrap_or(0) | 1);
                card.w8(MCU, card.r8(MCU) & !MCU_NOW_IS_OOB);
            }
            if card.has_ocp() {
                card.w32(MISC, card.r32(MISC) & !MISC_RXDV_GATED);
                card.mac_ocp_modify(0xC0AC, 0x0000, 0x1F80);
            }
            card.w16(INTR_MITIGATE, 0);
        }
        card.w16(RX_MAX_SIZE, BUF as u16);
        // The rings' addresses, high halves first (Linux: some boards need it).
        card.w32(TX_RING + 4, (card.tx.desc.phys >> 32) as u32);
        card.w32(TX_RING, card.tx.desc.phys as u32);
        card.w32(RX_RING + 4, (card.rx.desc.phys >> 32) as u32);
        card.w32(RX_RING, card.rx.desc.phys as u32);
        card.w8(CFG9346, LOCK);
        let _ = card.r8(CHIP_CMD);
        // Sending and receiving on first: "the transmit configuration register can only be changed after having set
        // TE" (the RTL8168B's datasheet).
        card.w8(CHIP_CMD, CMD_TX_ENABLE | CMD_RX_ENABLE);
        card.w32(RX_CONFIG, card.rx_config_base());
        let auto_fifo = if (34..=52).contains(&version) { TXCFG_AUTO_FIFO } else { 0 };
        card.w32(TX_CONFIG, TXCFG | auto_fifo);
        card.w32(MAR0 + 4, 0); // no multicast groups
        card.w32(MAR0, 0);
        card.w32(RX_CONFIG, card.rx_config_base() | RX_ACCEPT);

        // Interrupts by MSI; the main loop, woken by them, looks at the rings. Acknowledging what happened (in
        // `receive`) lets the next one come.
        card.signal = f.at.signal_msi(vector, crate::apic::id());
        card.mask_and_ack();
        if card.is_8125() {
            card.w32(IMR_8125, INT_MASK);
        } else {
            card.w16(IMR_8168, INT_MASK as u16);
        }
        crate::log!(
            "Chat Computer kernel: network: the {name} (version {xid:#05x}) hasn't been tried on a real PC yet{notes}; \
             if it doesn't connect, that's why"
        );
        Ok(card)
    }

    /// Hands receive descriptor `i` back to the chip, with its empty buffer.
    fn give_rx(&self, i: usize) {
        let (phys, _) = self.rx.buffer(i);
        let d = self.rx.desc(i);
        let eor = if i == RING - 1 { EOR } else { 0 };
        unsafe {
            write_volatile(d.add(8) as *mut u64, phys);
            write_volatile(d.add(4) as *mut u32, 0);
            fence(Ordering::SeqCst);
            write_volatile(d as *mut u32, OWN | eor | BUF as u32);
        }
    }

    /// Tells the chip there's something to send.
    fn doorbell(&self) {
        if self.is_8125() {
            self.w16(TX_POLL_8125, 1);
        } else {
            self.w8(TX_POLL_8168, 1 << 6);
        }
    }

    /// Moves past the sent frames (the chip gave their descriptors back).
    fn reap(&mut self) {
        while self.tx_clean != self.tx_tail {
            let opts1 = unsafe { read_volatile(self.tx.desc(self.tx_clean) as *const u32) };
            if opts1 & OWN != 0 {
                break;
            }
            self.tx_clean = (self.tx_clean + 1) % RING;
        }
    }
}

impl Card for Realtek {
    fn describe(&self) -> String {
        format!(
            "Realtek {} network chip {:04x}:{:04x} at {}, address {}, interrupts {}",
            self.name,
            VENDOR,
            self.device,
            self.at,
            net::Mac(self.mac),
            match self.signal {
                pci::Signal::Msi => "by MSI",
                _ => "off (it is checked for frames instead)",
            }
        )
    }

    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn link_up(&self) -> bool {
        let status = if self.is_8125() { self.r16(PHY_STATUS) } else { self.r8(PHY_STATUS) as u16 };
        status & PHY_LINK != 0
    }

    fn send(&mut self, frame: &[u8]) -> bool {
        if frame.is_empty() || frame.len() > BUF {
            return false;
        }
        self.reap();
        let next = (self.tx_tail + 1) % RING;
        if next == self.tx_clean {
            return false; // all in use
        }
        // Short frames padded to 60 bytes here: the 8125 and the 8111E-VL need it (Linux), and it costs nothing.
        let len = frame.len().max(60);
        let (phys, at) = self.tx.buffer(self.tx_tail);
        let d = self.tx.desc(self.tx_tail);
        let eor = if self.tx_tail == RING - 1 { EOR } else { 0 };
        unsafe {
            core::ptr::copy_nonoverlapping(frame.as_ptr(), at, frame.len());
            core::ptr::write_bytes(at.add(frame.len()), 0, len - frame.len());
            write_volatile(d.add(8) as *mut u64, phys);
            write_volatile(d.add(4) as *mut u32, 0);
            fence(Ordering::SeqCst);
            write_volatile(d as *mut u32, OWN | eor | FS | LS | len as u32);
        }
        fence(Ordering::SeqCst);
        self.tx_tail = next;
        self.doorbell();
        true
    }

    fn receive(&mut self, took: &mut dyn FnMut(&[u8])) -> usize {
        // What happened, acknowledged (so the next event interrupts again).
        if self.is_8125() {
            let status = self.r32(ISR_8125);
            if status != 0 && status != !0 {
                self.w32(ISR_8125, status);
            }
        } else {
            let status = self.r16(ISR_8168);
            if status != 0 && status != !0 {
                self.w16(ISR_8168, status);
            }
        }
        // A "send now" can be missed when frames go close together (Linux, FreeBSD): asked again while any wait.
        self.reap();
        if self.tx_clean != self.tx_tail {
            self.doorbell();
        }
        let mut n = 0;
        // At most a ring's worth at a time, good frames or bad, so a flood can't keep this here.
        for _ in 0..RING {
            let d = self.rx.desc(self.rx_next);
            let opts1 = unsafe { read_volatile(d as *const u32) };
            if opts1 & OWN != 0 {
                break;
            }
            fence(Ordering::SeqCst);
            // Whole frames only (one descriptor each), without errors; the length includes the Ethernet checksum.
            let len = (opts1 & RX_LENGTH) as usize;
            if opts1 & (FS | LS) == FS | LS && opts1 & RX_RES == 0 && len > 4 && len <= BUF {
                let (_, at) = self.rx.buffer(self.rx_next);
                took(unsafe { core::slice::from_raw_parts(at, len - 4) });
                n += 1;
            }
            self.give_rx(self.rx_next);
            self.rx_next = (self.rx_next + 1) % RING;
        }
        n
    }
}
