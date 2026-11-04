//! Intel's network chips with "advanced" descriptors: the igb family (Intel's datasheets for the 82576, 82580, I350
//! and I210/I211) and the I225 and I226 (2.5 Gb/s; Intel publishes no datasheet for them, so their differences
//! come from the open-source drivers that agree on them: Linux's and FreeBSD's igc, DPDK's and Zephyr's). These are
//! the I211 and I210 on many motherboards (AMD ones especially), the I225-V and I226-V on most recent ones that
//! have Intel's 2.5 Gb/s port, the I350 and 82580 on workstation and server boards, and the 82576, which QEMU gives
//! virtual machines ("igb"). Only their copper (RJ45) versions. So far only the 82576 has been tried, in a virtual
//! machine; the log says when another is used.
//!
//! The chip works from two rings of 16-byte descriptors in memory Chat Computer gives it: one of empty 2 KB buffers
//! it fills with the frames that arrive (writing over each descriptor to say so), one of frames to send. Chat
//! Computer moves each ring's tail on to hand descriptors over; the chip moves the head as it works through them.
//!
//! Its PHY (the part that talks to the cable) is reached through the MDIC register, which the chip's firmware may use
//! too: the two take turns by a semaphore (SWSM and SW_FW_SYNC).

use super::Card;
use crate::dma::Page;
use crate::paging::{self, Cache};
use crate::{pci, time};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

pub const VENDOR: u16 = 0x8086;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    /// The 82576: virtual machines' igb, and older server cards.
    I82576,
    /// The 82580 and I350 (workstation and server boards).
    I82580,
    /// The I210 and I211 (motherboards).
    I210,
    /// The I225 and I226, 2.5 Gb/s (motherboards; Linux's igc driver).
    I225,
}

/// Device ids (copper versions only), their names and kinds.
const DEVICES: &[(u16, &str, Family)] = &[
    (0x10C9, "82576", Family::I82576),
    (0x10E8, "82576", Family::I82576),
    (0x1526, "82576", Family::I82576),
    (0x150A, "82576NS", Family::I82576),
    (0x150E, "82580", Family::I82580),
    (0x1516, "82580", Family::I82580),
    (0x1521, "I350", Family::I82580),
    (0x1533, "I210", Family::I210),
    (0x157B, "I210", Family::I210),
    (0x1539, "I211", Family::I210),
    (0x15F3, "I225-V", Family::I225),
    (0x15F2, "I225-LM", Family::I225),
    (0x15F8, "I225-I", Family::I225),
    (0x15F7, "I220-V", Family::I225),
    (0x0D9F, "I225-IT", Family::I225),
    (0x5502, "I225-LMvP", Family::I225),
    (0x3100, "I225-K", Family::I225),
    (0x3101, "I225-K2 (Killer E3100X)", Family::I225),
    (0x15FD, "I225 (blank NVM)", Family::I225),
    (0x125C, "I226-V", Family::I225),
    (0x125B, "I226-LM", Family::I225),
    (0x125D, "I226-IT", Family::I225),
    (0x5503, "I226-LMvP", Family::I225),
    (0x3102, "I226-K", Family::I225),
    (0x125E, "I221-V", Family::I225),
    (0x125F, "I226 (blank NVM)", Family::I225),
];

/// The I225s (not the I226, I221 or I220) whose first revisions can't take frames at 2.5 Gb/s with the shortest gap
/// between them (FreeBSD's igc).
const I225_IDS: &[u16] = &[0x15F2, 0x15F3, 0x3100, 0x15F8, 0x3101, 0x5502, 0x0D9F, 0x15FD];
/// The I210s (not the I211), whose packet buffers keep their sizes through a reset.
const I210_IDS: &[u16] = &[0x1533, 0x157B];

/// The chip's name, if it's one of these (and an Ethernet controller).
pub fn model(f: &pci::Function) -> Option<&'static str> {
    if f.vendor != VENDOR || f.class != 0x02 || f.subclass != 0x00 {
        return None;
    }
    DEVICES.iter().find(|d| d.0 == f.device).map(|d| d.1)
}

// Registers (offsets in the first memory region). The rings' are queue 0's.
const CTRL: u64 = 0x0000;
const STATUS: u64 = 0x0008;
const EECD: u64 = 0x0010;
const CTRL_EXT: u64 = 0x0018;
const MDIC: u64 = 0x0020;
const VET: u64 = 0x0038;
const RCTL: u64 = 0x0100;
const TCTL: u64 = 0x0400;
const TIPG: u64 = 0x0410;
const MDICNFG: u64 = 0x0E04;
const PHPM: u64 = 0x0E14; // the PHY's power management (82580 and later)
const EEER: u64 = 0x0E30; // Energy Efficient Ethernet (I225)
const IPCNFG: u64 = 0x0E38; // (I225)
const EEMNGCTL: u64 = 0x1010;
const IVAR0: u64 = 0x1700;
const RXPBS: u64 = 0x2404; // the receive packet buffer's size (I210, I225)
const TXPBS: u64 = 0x3404; // the transmit packet buffer's (I210, I225)
const DTXMXPKTSZ_I210: u64 = 0x355C; // the largest frame sent
const TQAVCTRL_I210: u64 = 0x3570; // the send side's mode (bit 0: the time-sensitive "Qav" mode)
const RFCTL: u64 = 0x5008;
const MTA: u64 = 0x5200;
const RAL0: u64 = 0x5400;
const RAH0: u64 = 0x5404;
const VMOLR0: u64 = 0x5AD0;
const WUC: u64 = 0x5800;
const MANC: u64 = 0x5820;
const SWSM: u64 = 0x5B50;
const SW_FW_SYNC: u64 = 0x5B5C;
const RDBAL: u64 = 0xC000;
const RDBAH: u64 = 0xC004;
const RDLEN: u64 = 0xC008;
const SRRCTL: u64 = 0xC00C;
const RDH: u64 = 0xC010;
const RDT: u64 = 0xC018;
const RXDCTL: u64 = 0xC028;
const TDBAL: u64 = 0xE000;
const TDBAH: u64 = 0xE004;
const TDLEN: u64 = 0xE008;
const TDH: u64 = 0xE010;
const TDT: u64 = 0xE018;
const TXDCTL: u64 = 0xE028;
const EEARBC_I210: u64 = 0x12024;
const EEMNGCTL_I210: u64 = 0x12030;
const INVM_DATA: u64 = 0x12120; // 64 words of the I210's and I211's one-time-programmable memory

/// The interrupt cause (read to clear), mask-set and mask-clear registers: the igb family has them where the e1000
/// family does; the I225 and I226 elsewhere.
fn interrupt_registers(family: Family) -> (u64, u64, u64) {
    match family {
        Family::I225 => (0x1500, 0x1508, 0x150C),
        _ => (0x00C0, 0x00D0, 0x00D8),
    }
}

const CTRL_GIO_MASTER_DISABLE: u32 = 1 << 2;
const CTRL_SLU: u32 = 1 << 6;
const CTRL_FRCSPD: u32 = 1 << 11;
const CTRL_FRCDPLX: u32 = 1 << 12;
const CTRL_RST: u32 = 1 << 26;
const CTRL_PHY_RST: u32 = 1 << 31;
const STATUS_LU: u32 = 1 << 1;
const STATUS_SPEED_2500: u32 = 1 << 22; // (I225)
const STATUS_FUNC_SHIFT: u32 = 2;
const TIPG_IPGT: u32 = 0x3FF;
const STATUS_GIO_MASTER_ENABLE: u32 = 1 << 19;
const STATUS_DEV_RST_SET: u32 = 1 << 20;
const EECD_AUTO_RD: u32 = 1 << 9;
const EECD_FLASH_DETECTED_I210: u32 = 1 << 19;
/// Port 0's "configuration done" in EEMNGCTL (the others follow): the chip has set its PHY up from its NVM.
const CFG_DONE_PORT_0: u32 = 1 << 18;
const CTRL_EXT_SDLPE: u32 = 1 << 18;
const CTRL_EXT_PHYPDEN: u32 = 1 << 20;
const CTRL_EXT_DRV_LOAD: u32 = 1 << 28;
const MDICNFG_EXT_MDIO: u32 = 1 << 31;
// PHPM: on the 82580 and later, low-power link-up in D0 and "go link disconnect"; on the I225, the same disconnect,
// the switches that leave out 1000 and 2500 Mb/s, and "PHY reset complete".
const PHPM_D0_LPLU: u32 = 1 << 1;
const PHPM_GO_LINKD: u32 = 1 << 5;
const PHPM_DIS_1000: u32 = 1 << 6;
const PHPM_RST_COMPL: u32 = 1 << 8;
const PHPM_DIS_2500: u32 = 1 << 11;
const EEER_LPI: u32 = 7 << 16;
const IPCNFG_EEE: u32 = 7 << 2;
const RFCTL_IPV6_EX_DIS: u32 = 1 << 16;
const MANC_BLK_PHY_RST_ON_IDE: u32 = 1 << 18;
const RAH_AV: u32 = 1 << 31;
const VMOLR_AUPE: u32 = 1 << 24;
const VMOLR_BAM: u32 = 1 << 27;
const IVAR_VALID: u32 = 0x80;
const SWSM_SMBI: u32 = 1 << 0;
const SWSM_SWESMBI: u32 = 1 << 1;

const RCTL_EN: u32 = 1 << 1;
const RCTL_BAM: u32 = 1 << 15; // broadcasts too
const RCTL_SECRC: u32 = 1 << 26; // the chip takes the Ethernet checksum off
/// Off: storing bad frames, every unicast or multicast frame, long frames, loopback, the multicast table's offset,
/// VLAN filtering.
const RCTL_OFF: u32 = 1 << 2 | 1 << 3 | 1 << 4 | 1 << 5 | 3 << 6 | 3 << 12 | 1 << 18;
const TCTL_EN: u32 = 1 << 1;
const TCTL_PSP: u32 = 1 << 3; // short frames padded to 64 bytes
const TCTL_CT_MASK: u32 = 0xFF << 4;
const TCTL_CT: u32 = 0x0F << 4; // retries after a collision (half duplex only)
const TCTL_RTLC: u32 = 1 << 24;

/// Receive buffers of 2 KB (in KB), one buffer to a frame, "advanced" descriptors.
const SRRCTL_BSIZEPKT_2K: u32 = 2;
const SRRCTL_BSIZEHDR_256: u32 = 4 << 8; // (only used when headers are split, which they aren't)
const SRRCTL_DESCTYPE_ADV_ONEBUF: u32 = 1 << 25;
/// A queue's switch, and its thresholds: fetch descriptors when fewer than 8 are on hand, in bursts of at least 8
/// (receiving) or 1 (sending), and write each one back as soon as it's done, so nothing waits for more. (Linux's
/// igb writes back one at a time where the 82576's erratum keeps it from batching; FreeBSD's igc writes sent
/// descriptors back at once with a threshold of 0.)
const DCTL_ENABLE: u32 = 1 << 25;
const RXDCTL_THRESHOLDS: u32 = 8 | 8 << 8 | 1 << 16;
fn txdctl_thresholds(family: Family) -> u32 {
    if family == Family::I225 { 8 | 1 << 8 } else { 8 | 1 << 8 | 1 << 16 }
}

// Interrupt causes: a descriptor written back (a frame arrived), link status changed, receive ring nearly empty,
// overrun, sequence error.
const INT_LSC: u32 = 1 << 2;
const INT_RXSEQ: u32 = 1 << 3;
const INT_RXDMT0: u32 = 1 << 4;
const INT_RXO: u32 = 1 << 6;
const INT_RXDW: u32 = 1 << 7;

// The MDIC register: the PHY's registers, one at a time.
const MDIC_OP_WRITE: u32 = 1 << 26;
const MDIC_OP_READ: u32 = 1 << 27;
const MDIC_READY: u32 = 1 << 28;
const MDIC_ERROR: u32 = 1 << 30;
// The PHY's registers (IEEE 802.3, clause 22) and the bits used.
const PHY_CONTROL: u32 = 0;
const PHY_ID1: u32 = 2;
const PHY_AUTONEG_ADV: u32 = 4;
const PHY_1000T_CTRL: u32 = 9;
const PHY_MMD_CONTROL: u32 = 13;
const PHY_MMD_DATA: u32 = 14;
const PHY_PAGE_SELECT: u32 = 0x16;
const MII_CR_RESTART_AUTO_NEG: u16 = 1 << 9;
const MII_CR_POWER_DOWN: u16 = 1 << 11;
const MII_CR_AUTO_NEG_EN: u16 = 1 << 12;
const MII_CR_LOOPBACK: u16 = 1 << 14;
/// Advertised: 10 and 100 Mb/s, half and full duplex (IEEE 802.3 selector); 1000 Mb/s full duplex; and on the I225,
/// 2500 Mb/s (the multi-gigabit advertisement: MMD 7, register 0x20, bit 7).
const ADVERTISE_10_100: u16 = 0x01E1;
const ADVERTISE_1000_FULL: u16 = 1 << 9;
const MMD_AUTONEG: u16 = 7;
const MMD_MULTIGIG_CTRL: u16 = 0x20;
const ADVERTISE_2500_FULL: u16 = 1 << 7;
/// The semaphore bit for port n's PHY in SW_FW_SYNC (firmware's are 16 bits higher).
const SWFW_PHY: [u32; 4] = [0x02, 0x04, 0x20, 0x40];

// The I210's and I211's PLL erratum (Intel's I210 specification update): after a reset the PHY's clock may not have
// started, which shows as 0xFF in the PHY's register 14 on page 0xFC. Linux's igb works around it the same way.
const PLL_FREQ_PAGE: u16 = 0xFC;
const PLL_FREQ_REG: u32 = 0x0E;
const PLL_UNCONFIGURED: u16 = 0xFF;
const PLL_TRIES: usize = 5;
const INVM_AUTOLOAD: u32 = 0x0A;
const INVM_DEFAULT_AUTOLOAD: u16 = 0x202F;
const INVM_PLL_WORKAROUND: u16 = 0x0010;

// Descriptor bits: advanced receive write-back (status at byte 8), advanced transmit data descriptor.
const RX_DD: u32 = 1 << 0;
const RX_EOP: u32 = 1 << 1;
/// The receive errors that make a frame bad: on the igb family, a CRC, symbol, sequence or carrier-extension error, or
/// "RX data error" (Linux's igb); on the I225 and I226, RX data error (Linux's and FreeBSD's igc). Checksum errors
/// aren't among them: the network stack checks every checksum itself (and the I225 sets one wrongly on some frames).
const RX_FRAME_ERRORS: u32 = 1 << 24 | 1 << 25 | 1 << 26 | 1 << 28 | 1 << 31;
const RX_FRAME_ERRORS_I225: u32 = 1 << 31;
const TX_DTYP_DATA: u32 = 0x3 << 20;
const TX_EOP: u32 = 1 << 24;
const TX_IFCS: u32 = 1 << 25;
const TX_RS: u32 = 1 << 27;
const TX_DEXT: u32 = 1 << 29;
const TX_PAYLEN_SHIFT: u32 = 14;
const TX_DD: u32 = 1 << 0;

/// Descriptors per ring (1 KB: the chip's rings are multiples of 128 bytes), and the size of each buffer.
const RING: usize = 64;
const BUF: usize = 2048;
/// Frames sent are at least this long (without their checksum): Ethernet's shortest. (The chip pads shorter ones
/// itself, but only from 17 bytes up.)
const SHORTEST: usize = 60;

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

pub struct Igb {
    at: pci::Address,
    device: u16,
    name: &'static str,
    regs: u64,
    /// The interrupt cause register.
    icr: u64,
    mac: [u8; 6],
    /// This port's PHY semaphore bit, and the PHY's address on its management bus.
    phy_lock: u32,
    phy_address: u32,
    /// Whether a semaphore left set has been cleared already (that's done once).
    semaphore_cleared: Cell<bool>,
    /// The receive errors that make a frame bad, on this chip.
    rx_errors: u32,
    /// A first-revision I225: its gap between frames follows the link's speed.
    gap_workaround: bool,
    rx: Ring,
    tx: Ring,
    /// The next receive descriptor to look at.
    rx_next: usize,
    /// The next transmit descriptor to fill, and the oldest not yet known to be sent.
    tx_tail: usize,
    tx_clean: usize,
    pub signal: pci::Signal,
}

unsafe impl Send for Igb {}

impl Igb {
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

    // ------------------------------------------------------------------------------------------- the semaphore

    /// Software's bit of the hardware semaphore: reading SWSM sets SMBI, so it's ours if it was clear.
    fn smbi(&self) -> bool {
        for _ in 0..2000 {
            if self.r(SWSM) & SWSM_SMBI == 0 {
                return true;
            }
            time::delay_us(50);
        }
        false
    }

    /// The hardware semaphore: software's bit, then the bit that keeps the firmware out (SWESMBI, ours if it stays
    /// set).
    fn semaphore(&self) -> bool {
        // (A semaphore left set, by a firmware's driver say, is cleared once, as Linux and FreeBSD do for the I210 and
        // I225; clearing it again would take it from whoever holds it.)
        if !self.smbi() {
            if self.semaphore_cleared.replace(true) {
                return false;
            }
            self.semaphore_off();
            if !self.smbi() {
                return false;
            }
        }
        for _ in 0..2000 {
            self.w(SWSM, self.r(SWSM) | SWSM_SWESMBI);
            if self.r(SWSM) & SWSM_SWESMBI != 0 {
                return true;
            }
            time::delay_us(50);
        }
        self.semaphore_off();
        false
    }

    fn semaphore_off(&self) {
        self.w(SWSM, self.r(SWSM) & !(SWSM_SMBI | SWSM_SWESMBI));
    }

    /// Takes a resource (`mask` in SW_FW_SYNC) from the firmware's sharing, waiting up to a second if it's using it.
    fn acquire(&self, mask: u32) -> bool {
        for _ in 0..200 {
            if !self.semaphore() {
                return false;
            }
            let sync = self.r(SW_FW_SYNC);
            if sync & (mask | mask << 16) == 0 {
                self.w(SW_FW_SYNC, sync | mask);
                self.semaphore_off();
                return true;
            }
            self.semaphore_off();
            time::delay_ms(5);
        }
        false
    }

    fn release(&self, mask: u32) {
        let got = self.semaphore();
        self.w(SW_FW_SYNC, self.r(SW_FW_SYNC) & !mask);
        if got {
            self.semaphore_off();
        }
    }

    // ------------------------------------------------------------------------------------------------ the PHY

    /// Runs `f` with the PHY taken from the firmware's sharing (None if it couldn't be taken).
    fn with_phy<T>(&self, f: impl FnOnce(&Self) -> T) -> Option<T> {
        if !self.acquire(self.phy_lock) {
            return None;
        }
        let result = f(self);
        self.release(self.phy_lock);
        Some(result)
    }

    /// One MDIC transaction: the register's value (a read), or that it worked (a write).
    fn mdic(&self, command: u32) -> Option<u16> {
        self.w(MDIC, command | self.phy_address << 21);
        for _ in 0..1920 {
            time::delay_us(50);
            let m = self.r(MDIC);
            if m & MDIC_READY != 0 {
                return (m & MDIC_ERROR == 0).then_some(m as u16);
            }
        }
        None
    }

    /// A PHY register (the PHY must be held: `with_phy`).
    fn mdio_read(&self, reg: u32) -> Option<u16> {
        self.mdic(reg << 16 | MDIC_OP_READ)
    }

    fn mdio_write(&self, reg: u32, value: u16) -> bool {
        self.mdic(value as u32 | reg << 16 | MDIC_OP_WRITE).is_some()
    }

    fn phy_read(&self, reg: u32) -> Option<u16> {
        self.with_phy(|s| s.mdio_read(reg)).flatten()
    }

    fn phy_write(&self, reg: u32, value: u16) -> bool {
        self.with_phy(|s| s.mdio_write(reg, value)) == Some(true)
    }

    /// Sets `bits` in register `reg` of MMD `device` (IEEE 802.3 clause 45, through clause 22's registers 13 and 14).
    fn mmd_set(&self, device: u16, reg: u16, bits: u16) -> bool {
        self.mdio_write(PHY_MMD_CONTROL, device)
            && self.mdio_write(PHY_MMD_DATA, reg)
            && self.mdio_write(PHY_MMD_CONTROL, device | 0x4000)
            && self.mdio_read(PHY_MMD_DATA).is_some_and(|v| self.mdio_write(PHY_MMD_DATA, v | bits))
            && self.mdio_write(PHY_MMD_CONTROL, 0)
    }

    /// The PHY awake and negotiating with the other end: every speed it has offered (up to 1000 Mb/s, or 2500 on the
    /// I225), no loopback.
    fn phy_up(&self, family: Family) -> bool {
        self.with_phy(|s| {
            let Some(control) = s.mdio_read(PHY_CONTROL) else { return false };
            s.mdio_write(PHY_AUTONEG_ADV, ADVERTISE_10_100)
                && s.mdio_write(PHY_1000T_CTRL, ADVERTISE_1000_FULL)
                && (family != Family::I225 || s.mmd_set(MMD_AUTONEG, MMD_MULTIGIG_CTRL, ADVERTISE_2500_FULL))
                && s.mdio_write(
                    PHY_CONTROL,
                    (control & !(MII_CR_POWER_DOWN | MII_CR_LOOPBACK)) | MII_CR_AUTO_NEG_EN | MII_CR_RESTART_AUTO_NEG,
                )
        }) == Some(true)
    }

    /// The I225's PHY reset (Linux's igc does one before using the PHY: firmware has been seen to leave it on another
    /// register page, where even its id reads wrong). Then wait for "reset complete". (Not when the firmware keeps the
    /// PHY: the caller checks.)
    fn phy_reset_i225(&self) -> bool {
        self.with_phy(|s| {
            s.w(CTRL, s.r(CTRL) | CTRL_PHY_RST);
            time::delay_us(100);
            s.w(CTRL, s.r(CTRL) & !CTRL_PHY_RST);
            time::delay_us(100);
            let done = (0..10_000).any(|_| {
                time::delay_us(1);
                s.r(PHPM) & PHPM_RST_COMPL != 0
            });
            time::delay_us(100);
            done
        }) == Some(true)
    }

    /// A word of the I210's or I211's one-time-programmable memory, if it holds one for `address`.
    fn invm_word(&self, address: u32) -> Option<u16> {
        let mut i = 0;
        while i < 64 {
            let record = self.r(INVM_DATA + 4 * i);
            match record & 7 {
                0 => return None, // the end
                1 if (record >> 9) & 0x7F == address => return Some((record >> 16) as u16),
                2 => i += 1, // a register to load: one more word
                4 => i += 8, // a key: eight more
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// The I210's and I211's PLL erratum, as Intel describes it: while the PHY says its clock isn't set up, reset the
    /// PHY, and take the chip through a power cycle (D3 and back) with the "PLL workaround" bit added to the word the
    /// chip loads at start-up. Only for chips without a flash memory (all I211s, some I210s).
    fn pll_workaround(&self) -> bool {
        let wuc = self.r(WUC);
        let mdicnfg = self.r(MDICNFG);
        self.w(MDICNFG, mdicnfg & !MDICNFG_EXT_MDIO);
        let autoload = self.invm_word(INVM_AUTOLOAD).unwrap_or(INVM_DEFAULT_AUTOLOAD);
        let power = self.at.power_control();
        // (Where the chip's registers are, in case the power cycle loses it.)
        let bars = [self.at.read32(0x10), self.at.read32(0x14)];
        let mut ok = false;
        self.phy_write(PHY_PAGE_SELECT, PLL_FREQ_PAGE);
        for _ in 0..PLL_TRIES {
            if self.phy_read(PLL_FREQ_REG).is_some_and(|v| v & PLL_UNCONFIGURED != PLL_UNCONFIGURED) {
                ok = true;
                break;
            }
            self.w(CTRL, self.r(CTRL) | CTRL_PHY_RST);
            self.w(CTRL_EXT, self.r(CTRL_EXT) | CTRL_EXT_PHYPDEN | CTRL_EXT_SDLPE);
            self.w(WUC, 0);
            self.w(EEARBC_I210, INVM_AUTOLOAD << 4 | ((autoload | INVM_PLL_WORKAROUND) as u32) << 16);
            match power {
                Some(pmcsr) => {
                    let state = self.at.read16(pmcsr);
                    self.at.write16(pmcsr, state | 3); // D3
                    time::delay_ms(2);
                    self.at.write16(pmcsr, state & !3); // D0
                    time::delay_ms(10);
                    if [self.at.read32(0x10), self.at.read32(0x14)] != bars {
                        self.at.write32(0x10, bars[0]);
                        self.at.write32(0x14, bars[1]);
                    }
                    self.at.enable(true);
                }
                None => time::delay_ms(2),
            }
            if self.r(CTRL) & CTRL_PHY_RST != 0 {
                self.w(CTRL, self.r(CTRL) & !CTRL_PHY_RST);
                time::delay_ms(1);
            }
            self.w(EEARBC_I210, INVM_AUTOLOAD << 4 | (autoload as u32) << 16);
            self.w(WUC, wuc);
        }
        self.phy_write(PHY_PAGE_SELECT, 0);
        self.w(MDICNFG, mdicnfg);
        ok
    }

    // ------------------------------------------------------------------------------------------------- start

    pub fn start(f: &pci::Function, vector: u8) -> Result<Igb, String> {
        let Some(&(_, name, family)) = DEVICES.iter().find(|d| d.0 == f.device) else {
            return Err(String::from("it isn't a chip this driver knows"));
        };
        f.at.power_on();
        let bar = f.at.memory_bar(0).ok_or_else(|| String::from("its registers aren't in memory"))?;
        if bar.size < 0x20000 {
            return Err(String::from("its register region is too small"));
        }
        let regs = paging::map_device(bar.phys, 0x20000, Cache::Uncached);
        f.at.enable(true);
        let (rx, tx) = match (Ring::new(), Ring::new()) {
            (Some(rx), Some(tx)) => (rx, tx),
            _ => return Err(String::from("out of memory")),
        };
        let (icr, ims, imc) = interrupt_registers(family);
        let mut card = Igb {
            at: f.at,
            device: f.device,
            name,
            regs,
            icr,
            mac: [0; 6],
            phy_lock: SWFW_PHY[0],
            // (The I225's internal PHY answers at 0, as Linux's and FreeBSD's igc use it; the igb family's at 1.)
            phy_address: if family == Family::I225 { 0 } else { 1 },
            semaphore_cleared: Cell::new(false),
            rx_errors: if family == Family::I225 { RX_FRAME_ERRORS_I225 } else { RX_FRAME_ERRORS },
            gap_workaround: false,
            rx,
            tx,
            rx_next: 0,
            tx_tail: 0,
            tx_clean: 0,
            signal: pci::Signal::None,
        };
        let port = match family {
            Family::I82576 | Family::I82580 => (card.r(STATUS) >> STATUS_FUNC_SHIFT) & 3,
            _ => 0,
        };
        card.phy_lock = SWFW_PHY[port as usize];
        let mut notes = String::new();

        // Quiet: the chip stops taking part in the PCI Express bus's transfers (so a reset can't leave one half
        // done), no interrupts, receiving and sending off (a firmware's driver may have left them on); then a reset
        // (its settings come back from its NVM).
        card.w(CTRL, card.r(CTRL) | CTRL_GIO_MASTER_DISABLE);
        card.wait(80, |s| s.r(STATUS) & STATUS_GIO_MASTER_ENABLE == 0);
        card.w(imc, !0);
        card.w(RCTL, 0);
        card.w(TCTL, TCTL_PSP);
        let _ = card.r(STATUS);
        time::delay_ms(10);
        card.w(CTRL, card.r(CTRL) | CTRL_RST);
        time::delay_ms(5);
        if !card.wait(200, |s| s.r(CTRL) & CTRL_RST == 0) {
            return Err(String::from("it didn't come back from a reset"));
        }
        card.wait(10, |s| s.r(EECD) & EECD_AUTO_RD != 0); // (a chip without an NVM never says so)
        if matches!(family, Family::I82580 | Family::I210) {
            card.w(STATUS, STATUS_DEV_RST_SET);
        }
        card.w(imc, !0);
        let _ = card.r(icr);
        if family != Family::I225 {
            // The PHY set up from the NVM (a chip without one never says so).
            let manageability = if family == Family::I210 { EEMNGCTL_I210 } else { EEMNGCTL };
            card.wait(100, |s| s.r(manageability) & CFG_DONE_PORT_0 << port != 0);
        }
        if family == Family::I210 && card.r(EECD) & EECD_FLASH_DETECTED_I210 == 0 && !card.pll_workaround() {
            notes.push_str("; its PHY's clock didn't start (Intel's I210 erratum)");
        }

        // A driver is here now (the chip's firmware, if it has one, leaves the port to it).
        card.w(CTRL_EXT, card.r(CTRL_EXT) | CTRL_EXT_DRV_LOAD);
        if I210_IDS.contains(&f.device) {
            // The I210's packet buffers keep their sizes through a reset (only a power cut resets them): a system
            // before that changed them, for its time-sensitive queues say, would leave the chip unable to pass traffic
            // (iPXE). Their defaults, and the send side's plain mode, as Linux's igb sets them.
            card.w(RXPBS, 0xA2);
            card.w(TXPBS, 0x0400_0014);
            card.w(DTXMXPKTSZ_I210, 0x98);
            card.w(TQAVCTRL_I210, card.r(TQAVCTRL_I210) & !1);
        }
        if family == Family::I225 {
            // The packet buffers split as Intel's I225/I226 manual says (Linux's igc), and Energy Efficient Ethernet
            // off: the I226-V can hang with it on (DPDK), and Linux and FreeBSD switch it off too.
            card.w(RXPBS, 0xA2);
            card.w(TXPBS, 0x0400_0014);
            card.w(IPCNFG, card.r(IPCNFG) & !IPCNFG_EEE);
            card.w(EEER, card.r(EEER) & !EEER_LPI);
            card.w(VET, 0x8100);
            // The first I225s (PCI revision 0 and 1) can't take frames sent with the shortest gap at 2.5 Gb/s:
            // Intel's workaround is to send with a 15-byte gap at that speed (FreeBSD; set as the link comes up).
            card.gap_workaround = I225_IDS.contains(&f.device) && f.at.read8(0x08) < 2;
            // PCI Express power saving the I225 and I226 get wrong (L1.2): off (Linux, FreeBSD). The I225 can enter it
            // when it shouldn't, so both kinds go; the I226 is only too slow coming out of the ASPM kind.
            f.at.disable_l1_2(name.starts_with("I225") || name.starts_with("I220"));
        }

        // Its address: in the first receive-address register, from its NVM, since the reset.
        let (ral, rah) = (card.r(RAL0), card.r(RAH0));
        card.mac = [ral as u8, (ral >> 8) as u8, (ral >> 16) as u8, (ral >> 24) as u8, rah as u8, (rah >> 8) as u8];
        if card.mac == [0; 6] || card.mac == [0xFF; 6] || card.mac[0] & 1 != 0 {
            let r = crate::random::u64().to_le_bytes();
            card.mac = [0x02, r[1], r[2], r[3], r[4], r[5]];
            notes.push_str("; no address in its NVM, so a made-up one");
        }
        let m = card.mac;
        card.w(RAL0, u32::from_le_bytes([m[0], m[1], m[2], m[3]]));
        let _ = card.r(STATUS); // (two writes in a row could be merged on the way, which some chips get wrong)
        card.w(RAH0, u32::from_le_bytes([m[4], m[5], 0, 0]) | RAH_AV);
        for i in 0..128 {
            card.w(MTA + 4 * i, 0); // no multicast groups
        }

        // The link: up, at whatever speed the other end agrees to. The 82580 and later can be told to keep the link
        // down, or slow, to save power (a firmware may have left them so).
        let ctrl = card.r(CTRL);
        card.w(CTRL, (ctrl | CTRL_SLU) & !(CTRL_FRCSPD | CTRL_FRCDPLX));
        match family {
            Family::I82576 => {}
            Family::I225 => card.w(PHPM, card.r(PHPM) & !(PHPM_GO_LINKD | PHPM_DIS_1000 | PHPM_DIS_2500)),
            _ => card.w(PHPM, card.r(PHPM) & !(PHPM_GO_LINKD | PHPM_D0_LPLU)),
        }
        if card.r(MANC) & MANC_BLK_PHY_RST_ON_IDE != 0 {
            // A management firmware (Intel's AMT, a server's BMC) keeps the PHY: its link is already up, and
            // negotiating again would drop the firmware's own connection (Linux and FreeBSD leave it too).
            notes.push_str("; its management firmware keeps the PHY, so the link is as the firmware set it");
        } else {
            if family == Family::I225 {
                if !card.phy_reset_i225() {
                    notes.push_str("; its PHY didn't finish a reset");
                }
                if !card.phy_read(PHY_ID1).is_some_and(|id| id != 0 && id != 0xFFFF) {
                    notes.push_str("; its PHY didn't say who it is");
                }
            }
            if !card.phy_up(family) {
                notes.push_str("; its PHY didn't answer");
            }
        }

        // Receiving: queue 0 off while it's set up, then every descriptor gets an empty buffer; the chip may fill all
        // but one. Frames for this address and broadcasts. (On the igb family, the first pool, all there is without
        // virtual machines' queues, takes frames without a VLAN tag, and broadcasts.)
        card.w(RCTL, card.r(RCTL) & !RCTL_EN);
        card.w(RXDCTL, 0);
        if family == Family::I225 {
            card.w(RFCTL, card.r(RFCTL) | RFCTL_IPV6_EX_DIS); // (an erratum, as Linux's igc says)
        } else {
            card.w(VMOLR0, card.r(VMOLR0) | VMOLR_AUPE | VMOLR_BAM);
        }
        card.w(RDBAL, card.rx.desc.phys as u32);
        card.w(RDBAH, (card.rx.desc.phys >> 32) as u32);
        card.w(RDLEN, (RING * 16) as u32);
        card.w(RDH, 0);
        card.w(RDT, 0);
        card.w(SRRCTL, SRRCTL_BSIZEPKT_2K | SRRCTL_BSIZEHDR_256 | SRRCTL_DESCTYPE_ADV_ONEBUF);
        for i in 0..RING {
            card.give_rx(i);
        }
        // (The queue says it's on before its tail may move: Intel's I210 datasheet, 4.5.9. No other driver waits for
        // it, so one that doesn't say so goes on anyway.)
        card.w(RXDCTL, RXDCTL_THRESHOLDS | DCTL_ENABLE);
        if !card.wait(10, |s| s.r(RXDCTL) & DCTL_ENABLE != 0) {
            notes.push_str("; its receive queue didn't say it started");
        }
        card.w(RDT, (RING - 1) as u32);
        card.w(RCTL, (card.r(RCTL) & !RCTL_OFF) | RCTL_EN | RCTL_BAM | RCTL_SECRC);

        // Sending.
        card.w(TXDCTL, 0);
        card.w(TDBAL, card.tx.desc.phys as u32);
        card.w(TDBAH, (card.tx.desc.phys >> 32) as u32);
        card.w(TDLEN, (RING * 16) as u32);
        card.w(TDH, 0);
        card.w(TDT, 0);
        card.w(TXDCTL, txdctl_thresholds(family) | DCTL_ENABLE);
        if !card.wait(10, |s| s.r(TXDCTL) & DCTL_ENABLE != 0) {
            notes.push_str("; its send queue didn't say it started");
        }
        card.w(TCTL, (card.r(TCTL) & !TCTL_CT_MASK) | TCTL_EN | TCTL_PSP | TCTL_CT | TCTL_RTLC);

        // Interrupts by MSI (one message for everything); the queues' causes go to it (IVAR's first entry: receive,
        // then send). The main loop, woken by them, looks at the rings. Reading ICR (in `receive`) lets the next come.
        card.w(IVAR0, (card.r(IVAR0) & !0xFFFF) | IVAR_VALID | IVAR_VALID << 8);
        card.signal = f.at.signal_msi(vector, crate::apic::id());
        let _ = card.r(icr);
        card.w(ims, INT_LSC | INT_RXSEQ | INT_RXDMT0 | INT_RXO | INT_RXDW);
        if family != Family::I82576 || !notes.is_empty() {
            crate::log!(
                "Chat Computer kernel: network: the {name} hasn't been tried on a real PC yet{notes}; if it doesn't \
                 connect, that's why"
            );
        }
        Ok(card)
    }

    /// Hands receive descriptor `i` to the chip, with its empty buffer.
    fn give_rx(&self, i: usize) {
        let (phys, _) = self.rx.buffer(i);
        let d = self.rx.desc(i);
        unsafe {
            write_volatile(d as *mut u64, phys);
            write_volatile(d.add(8) as *mut u64, 0); // (clears "done")
        }
    }
}

impl Card for Igb {
    fn describe(&self) -> String {
        format!(
            "Intel {} network chip {:04x}:{:04x} at {}, address {}, interrupts {}",
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
        let status = self.r(STATUS);
        if self.gap_workaround && status & STATUS_LU != 0 {
            // A first-revision I225's gap between frames: 15 bytes at 2.5 Gb/s, the standard 12 below (FreeBSD).
            let gap = if status & STATUS_SPEED_2500 != 0 { 0x0B } else { 0x08 };
            let tipg = self.r(TIPG);
            if tipg & TIPG_IPGT != gap {
                self.w(TIPG, (tipg & !TIPG_IPGT) | gap);
            }
        }
        status & STATUS_LU != 0
    }

    fn send(&mut self, frame: &[u8]) -> bool {
        if frame.is_empty() || frame.len() > BUF {
            return false;
        }
        // Descriptors the chip is done with.
        while self.tx_clean != self.tx_tail {
            let status = unsafe { read_volatile(self.tx.desc(self.tx_clean).add(12) as *const u32) };
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
        let len = frame.len().max(SHORTEST);
        unsafe {
            core::ptr::copy_nonoverlapping(frame.as_ptr(), at, frame.len());
            core::ptr::write_bytes(at.add(frame.len()), 0, len - frame.len());
            write_volatile(d as *mut u64, phys);
            write_volatile(d.add(12) as *mut u32, (len as u32) << TX_PAYLEN_SHIFT);
            write_volatile(d.add(8) as *mut u32, len as u32 | TX_DTYP_DATA | TX_DEXT | TX_EOP | TX_IFCS | TX_RS);
        }
        fence(Ordering::SeqCst);
        self.tx_tail = next;
        self.w(TDT, next as u32);
        true
    }

    fn receive(&mut self, took: &mut dyn FnMut(&[u8])) -> usize {
        let _ = self.r(self.icr); // (clears the causes, so the next frame interrupts again)
        let mut n = 0;
        // At most a ring's worth at a time, good frames or bad, so a flood can't keep this here.
        for _ in 0..RING {
            let d = self.rx.desc(self.rx_next);
            let status = unsafe { read_volatile(d.add(8) as *const u32) };
            if status & RX_DD == 0 {
                break;
            }
            fence(Ordering::SeqCst);
            let len = unsafe { read_volatile(d.add(12) as *const u16) } as usize;
            if status & RX_EOP != 0 && status & self.rx_errors == 0 && len > 0 && len <= BUF {
                let (_, at) = self.rx.buffer(self.rx_next);
                took(unsafe { core::slice::from_raw_parts(at, len) });
                n += 1;
            }
            // The descriptor, empty again (the chip wrote over its buffer's address), goes back to the chip.
            self.give_rx(self.rx_next);
            fence(Ordering::SeqCst);
            self.w(RDT, self.rx_next as u32);
            self.rx_next = (self.rx_next + 1) % RING;
        }
        n
    }
}
