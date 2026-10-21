//! Intel's gigabit network cards with the e1000 interface (Intel's manuals for the 8254x and 8257x families): the
//! 82540EM and 82545EM that QEMU, VirtualBox and VMware give virtual machines, the 82574L (QEMU's "e1000e"), and on
//! real PCs the 8257x chips and the I217, I218 and I219 built into Intel's chipsets. Those last three have only
//! been tried in a virtual machine's likeness so far: their hardware asks for more care (their PHY can be left
//! asleep by the firmware), so the log says when one is used.
//!
//! The card works from two rings of 16-byte descriptors in memory Chat Computer gives it, in the "legacy" format
//! every one of these cards has: one of empty 2 KB buffers it fills with the frames that arrive (and marks done),
//! one of frames to send. Chat Computer moves each ring's tail on to hand descriptors over; the card moves the
//! head as it works through them.

use super::Card;
use crate::dma::Page;
use crate::paging::{self, Cache};
use crate::{pci, time};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

pub const VENDOR: u16 = 0x8086;

/// The kinds of card, as far as the driver cares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    /// 8254x: PCI cards (and virtual machines' e1000).
    Pci,
    /// 8257x: PCI Express cards (QEMU's e1000e is the 82574L).
    Express,
    /// I217, I218, I219: built into Intel's chipsets (PCH).
    Chipset,
}

/// Device ids, their names and kinds.
const DEVICES: &[(u16, &str, Family)] = &[
    (0x100E, "82540EM", Family::Pci),
    (0x100F, "82545EM", Family::Pci),
    (0x1015, "82540EM (LOM)", Family::Pci),
    (0x1026, "82545GM", Family::Pci),
    (0x1076, "82541GI", Family::Pci),
    (0x107C, "82541PI", Family::Pci),
    (0x105E, "82571EB", Family::Express),
    (0x107D, "82572EI", Family::Express),
    (0x108B, "82573V", Family::Express),
    (0x108C, "82573E", Family::Express),
    (0x109A, "82573L", Family::Express),
    (0x10D3, "82574L", Family::Express),
    (0x10F6, "82574LA", Family::Express),
    (0x150C, "82583V", Family::Express),
    (0x153A, "I217-LM", Family::Chipset),
    (0x153B, "I217-V", Family::Chipset),
    (0x155A, "I218-LM", Family::Chipset),
    (0x1559, "I218-V", Family::Chipset),
    (0x15A0, "I218-LM", Family::Chipset),
    (0x15A1, "I218-V", Family::Chipset),
    (0x15A2, "I218-LM", Family::Chipset),
    (0x15A3, "I218-V", Family::Chipset),
    (0x156F, "I219-LM", Family::Chipset),
    (0x1570, "I219-V", Family::Chipset),
    (0x15B7, "I219-LM", Family::Chipset),
    (0x15B8, "I219-V", Family::Chipset),
    (0x15B9, "I219-LM", Family::Chipset),
    (0x15D7, "I219-LM", Family::Chipset),
    (0x15D8, "I219-V", Family::Chipset),
    (0x15E3, "I219-LM", Family::Chipset),
    (0x15D6, "I219-V", Family::Chipset),
    (0x15BD, "I219-LM", Family::Chipset),
    (0x15BE, "I219-V", Family::Chipset),
    (0x15BB, "I219-LM", Family::Chipset),
    (0x15BC, "I219-V", Family::Chipset),
    (0x15DF, "I219-LM", Family::Chipset),
    (0x15E0, "I219-V", Family::Chipset),
    (0x15E1, "I219-LM", Family::Chipset),
    (0x15E2, "I219-V", Family::Chipset),
    (0x0D4E, "I219-LM", Family::Chipset),
    (0x0D4F, "I219-V", Family::Chipset),
    (0x0D4C, "I219-LM", Family::Chipset),
    (0x0D4D, "I219-V", Family::Chipset),
    (0x0D53, "I219-LM", Family::Chipset),
    (0x0D55, "I219-V", Family::Chipset),
    (0x15FB, "I219-LM", Family::Chipset),
    (0x15FC, "I219-V", Family::Chipset),
    (0x15F9, "I219-LM", Family::Chipset),
    (0x15FA, "I219-V", Family::Chipset),
    (0x15F4, "I219-LM", Family::Chipset),
    (0x15F5, "I219-V", Family::Chipset),
    (0x1A1E, "I219-LM", Family::Chipset),
    (0x1A1F, "I219-V", Family::Chipset),
    (0x1A1C, "I219-LM", Family::Chipset),
    (0x1A1D, "I219-V", Family::Chipset),
];

/// The card's name, if it's one of these (and an Ethernet controller, not something else of Intel's).
pub fn model(f: &pci::Function) -> Option<&'static str> {
    if f.vendor != VENDOR || f.class != 0x02 || f.subclass != 0x00 {
        return None;
    }
    DEVICES.iter().find(|d| d.0 == f.device).map(|d| d.1)
}

// Registers (offsets in the first memory region).
const CTRL: u64 = 0x0000;
const STATUS: u64 = 0x0008;
const EERD: u64 = 0x0014;
const ICR: u64 = 0x00C0;
const IMS: u64 = 0x00D0;
const IMC: u64 = 0x00D8;
const RCTL: u64 = 0x0100;
const TCTL: u64 = 0x0400;
const TIPG: u64 = 0x0410;
const RDBAL: u64 = 0x2800;
const RDBAH: u64 = 0x2804;
const RDLEN: u64 = 0x2808;
const RDH: u64 = 0x2810;
const RDT: u64 = 0x2818;
const TDBAL: u64 = 0x3800;
const TDBAH: u64 = 0x3804;
const TDLEN: u64 = 0x3808;
const TDH: u64 = 0x3810;
const TDT: u64 = 0x3818;
const MTA: u64 = 0x5200;
const RAL0: u64 = 0x5400;
const RAH0: u64 = 0x5404;

const CTRL_LRST: u32 = 1 << 3;
const CTRL_ASDE: u32 = 1 << 5;
const CTRL_SLU: u32 = 1 << 6;
const CTRL_ILOS: u32 = 1 << 7;
const CTRL_FRCSPD: u32 = 1 << 11;
const CTRL_FRCDPLX: u32 = 1 << 12;
const CTRL_RST: u32 = 1 << 26;
const CTRL_VME: u32 = 1 << 30;
const CTRL_PHY_RST: u32 = 1 << 31;
const STATUS_LU: u32 = 1 << 1;
const RAH_AV: u32 = 1 << 31;

const RCTL_EN: u32 = 1 << 1;
const RCTL_BAM: u32 = 1 << 15; // broadcasts too
const RCTL_SECRC: u32 = 1 << 26; // the card takes the Ethernet checksum off
const TCTL_EN: u32 = 1 << 1;
const TCTL_PSP: u32 = 1 << 3; // short frames padded to 64 bytes
const TCTL_CT: u32 = 0x0F << 4;
const TCTL_COLD: u32 = 0x3F << 12;
const TCTL_RTLC: u32 = 1 << 24;

// Interrupt causes: link status changed, a frame arrived (and the timer after it), receive ring nearly empty,
// overrun.
const INT_LSC: u32 = 1 << 2;
const INT_RXDMT0: u32 = 1 << 4;
const INT_RXO: u32 = 1 << 6;
const INT_RXT0: u32 = 1 << 7;

// Descriptor bits.
const RX_DD: u8 = 1 << 0;
const RX_EOP: u8 = 1 << 1;
const TX_EOP: u8 = 1 << 0;
const TX_IFCS: u8 = 1 << 1;
const TX_RS: u8 = 1 << 3;
const TX_DD: u8 = 1 << 0;

/// Descriptors per ring (a 1 KB ring, 128 bytes being the card's unit), and the size of each buffer.
const RING: usize = 64;
const BUF: usize = 2048;

/// One ring: the descriptors (one page), the buffers (two to a page).
struct Ring {
    desc: Page,
    buffers: Vec<Page>,
}

impl Ring {
    fn new() -> Option<Ring> {
        let desc = Page::new(true)?;
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

pub struct E1000 {
    at: pci::Address,
    device: u16,
    name: &'static str,
    family: Family,
    regs: u64,
    mac: [u8; 6],
    rx: Ring,
    tx: Ring,
    /// The next receive descriptor to look at.
    rx_next: usize,
    /// The next transmit descriptor to fill, and the oldest not yet known to be sent.
    tx_tail: usize,
    tx_clean: usize,
    pub signal: pci::Signal,
}

unsafe impl Send for E1000 {}

impl E1000 {
    fn r(&self, reg: u64) -> u32 {
        unsafe { read_volatile((self.regs + reg) as *const u32) }
    }

    fn w(&self, reg: u64, v: u32) {
        unsafe { write_volatile((self.regs + reg) as *mut u32, v) }
    }

    /// Waits up to `ms` for `done`.
    fn wait(&self, ms: u64, done: impl Fn(&Self) -> bool) -> bool {
        for _ in 0..ms * 10 {
            if done(self) {
                return true;
            }
            time::delay_us(100);
        }
        done(self)
    }

    /// One 16-bit word of the card's settings memory (NVM), through EERD: the older cards put the address at bit
    /// 8 and say done at bit 4; the newer at bit 2 and bit 1.
    fn nvm_word(&self, word: u16) -> Option<u16> {
        let (shift, done) = if self.family == Family::Pci { (8, 1 << 4) } else { (2, 1 << 1) };
        self.w(EERD, (word as u32) << shift | 1);
        self.wait(10, |s| s.r(EERD) & done != 0).then(|| (self.r(EERD) >> 16) as u16)
    }

    pub fn start(f: &pci::Function, vector: u8) -> Result<E1000, String> {
        let Some(&(_, name, family)) = DEVICES.iter().find(|d| d.0 == f.device) else {
            return Err(String::from("it isn't a card this driver knows"));
        };
        f.at.power_on();
        let bar = f.at.memory_bar(0).ok_or_else(|| String::from("its registers aren't in memory"))?;
        if bar.size < 0x6000 {
            return Err(String::from("its register region is too small"));
        }
        let regs = paging::map_device(bar.phys, bar.size.min(128 * 1024), Cache::Uncached);
        f.at.enable(true);
        let (rx, tx) = match (Ring::new(), Ring::new()) {
            (Some(rx), Some(tx)) => (rx, tx),
            _ => return Err(String::from("out of memory")),
        };
        let mut card =
            E1000 { at: f.at, device: f.device, name, family, regs, mac: [0; 6], rx, tx, rx_next: 0, tx_tail: 0, tx_clean: 0, signal: pci::Signal::None };

        // Quiet: no interrupts, receiving and sending off (a firmware's network driver may have left them on), then
        // a reset of the card (its settings come back from its NVM).
        card.w(IMC, !0);
        card.w(RCTL, 0);
        card.w(TCTL, TCTL_PSP);
        let _ = card.r(STATUS);
        time::delay_ms(10);
        card.w(CTRL, card.r(CTRL) | CTRL_RST);
        time::delay_ms(5);
        if !card.wait(200, |s| s.r(CTRL) & CTRL_RST == 0) {
            return Err(String::from("it didn't come back from a reset"));
        }
        time::delay_ms(10);
        card.w(IMC, !0);
        let _ = card.r(ICR);

        // The link: up, at whatever speed the other end agrees to.
        let ctrl = card.r(CTRL);
        card.w(CTRL, (ctrl | CTRL_SLU | CTRL_ASDE) & !(CTRL_LRST | CTRL_ILOS | CTRL_PHY_RST | CTRL_VME | CTRL_FRCSPD | CTRL_FRCDPLX));

        // Its address: in the first receive-address register once the NVM is read, else from the NVM itself.
        let (ral, rah) = (card.r(RAL0), card.r(RAH0));
        if rah & RAH_AV != 0 && (ral != 0 || rah & 0xFFFF != 0) {
            card.mac = [ral as u8, (ral >> 8) as u8, (ral >> 16) as u8, (ral >> 24) as u8, rah as u8, (rah >> 8) as u8];
        } else if let (Some(a), Some(b), Some(c)) = (card.nvm_word(0), card.nvm_word(1), card.nvm_word(2)) {
            card.mac = [a as u8, (a >> 8) as u8, b as u8, (b >> 8) as u8, c as u8, (c >> 8) as u8];
        } else {
            let r = crate::random::u64().to_le_bytes();
            card.mac = [0x02, r[1], r[2], r[3], r[4], r[5]];
        }
        let m = card.mac;
        card.w(RAL0, u32::from_le_bytes([m[0], m[1], m[2], m[3]]));
        card.w(RAH0, u32::from_le_bytes([m[4], m[5], 0, 0]) | RAH_AV);
        for i in 0..128 {
            card.w(MTA + 4 * i, 0); // no multicast groups
        }

        // Receiving: every descriptor gets an empty buffer; the card may fill all but one.
        for i in 0..RING {
            let (phys, _) = card.rx.buffer(i);
            let d = card.rx.desc(i);
            unsafe {
                write_volatile(d as *mut u64, phys);
                write_volatile(d.add(8) as *mut u64, 0);
            }
        }
        card.w(RDBAL, card.rx.desc.phys as u32);
        card.w(RDBAH, (card.rx.desc.phys >> 32) as u32);
        card.w(RDLEN, (RING * 16) as u32);
        card.w(RDH, 0);
        card.w(RDT, (RING - 1) as u32);
        card.w(RCTL, RCTL_EN | RCTL_BAM | RCTL_SECRC); // 2 KB buffers (BSIZE 0)

        // Sending.
        card.w(TDBAL, card.tx.desc.phys as u32);
        card.w(TDBAH, (card.tx.desc.phys >> 32) as u32);
        card.w(TDLEN, (RING * 16) as u32);
        card.w(TDH, 0);
        card.w(TDT, 0);
        card.w(TIPG, if family == Family::Pci { 0x0060_200A } else { 0x0060_2008 });
        card.w(TCTL, TCTL_EN | TCTL_PSP | TCTL_CT | TCTL_COLD | TCTL_RTLC);

        // Interrupts by MSI (one message for everything) where the card can; the main loop, woken by them, looks
        // at the rings. Reading ICR (in `receive`) lets the next one come.
        card.signal = f.at.signal_msi(vector, crate::apic::id());
        card.w(IMS, INT_LSC | INT_RXDMT0 | INT_RXO | INT_RXT0);
        if family == Family::Chipset {
            crate::log!(
                "Chat Computer kernel: network: the {name} hasn't been tried on a real PC yet; if it doesn't connect, \
                 that's why"
            );
        }
        Ok(card)
    }
}

impl Card for E1000 {
    fn describe(&self) -> String {
        format!(
            "Intel {} network card {:04x}:{:04x} at {}, address {}, interrupts {}",
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
        self.r(STATUS) & STATUS_LU != 0
    }

    fn send(&mut self, frame: &[u8]) -> bool {
        if frame.is_empty() || frame.len() > BUF {
            return false;
        }
        // Descriptors the card is done with.
        while self.tx_clean != self.tx_tail {
            let status = unsafe { read_volatile(self.tx.desc(self.tx_clean).add(12)) };
            if status & TX_DD == 0 {
                break;
            }
            self.tx_clean = (self.tx_clean + 1) % RING;
        }
        let next = (self.tx_tail + 1) % RING;
        if next == self.tx_clean {
            return false; // all in use
        }
        let (phys, at) = self.tx.buffer(self.tx_tail);
        let d = self.tx.desc(self.tx_tail);
        unsafe {
            core::ptr::copy_nonoverlapping(frame.as_ptr(), at, frame.len());
            write_volatile(d as *mut u64, phys);
            write_volatile(d.add(8) as *mut u16, frame.len() as u16);
            write_volatile(d.add(10), 0u8); // no checksum offset
            write_volatile(d.add(11), TX_EOP | TX_IFCS | TX_RS);
            write_volatile(d.add(12), 0u8);
            write_volatile(d.add(13), 0u8);
            write_volatile(d.add(14) as *mut u16, 0);
        }
        fence(Ordering::SeqCst);
        self.tx_tail = next;
        self.w(TDT, next as u32);
        true
    }

    fn receive(&mut self, took: &mut dyn FnMut(&[u8])) -> usize {
        let _ = self.r(ICR); // (clears the causes, so the next frame interrupts again)
        let mut n = 0;
        loop {
            let d = self.rx.desc(self.rx_next);
            let status = unsafe { read_volatile(d.add(12)) };
            if status & RX_DD == 0 {
                break;
            }
            fence(Ordering::SeqCst);
            let len = unsafe { read_volatile(d.add(8) as *const u16) } as usize;
            let errors = unsafe { read_volatile(d.add(13)) };
            if status & RX_EOP != 0 && errors == 0 && len > 0 && len <= BUF {
                let (_, at) = self.rx.buffer(self.rx_next);
                took(unsafe { core::slice::from_raw_parts(at, len) });
                n += 1;
            }
            // The descriptor, empty again, goes back to the card.
            unsafe {
                write_volatile(d.add(8) as *mut u64, 0);
            }
            fence(Ordering::SeqCst);
            self.w(RDT, self.rx_next as u32);
            self.rx_next = (self.rx_next + 1) % RING;
            if n >= RING {
                break;
            }
        }
        n
    }
}
