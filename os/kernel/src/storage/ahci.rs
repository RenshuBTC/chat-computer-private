//! SATA drives through AHCI (Intel's Serial ATA AHCI specification 1.3.1): the standard interface of every PC's SATA
//! controller, so one driver covers them all. As with NVMe, Chat Computer uses its drives only a little (the start-up
//! one for its memory file; the others, Windows' other drives, read): one command at a time, one 4 KiB page a
//! command, its end waited for.
//!
//! The controller is taken from the firmware first where it asks for that (the BIOS/OS hand-off), then only the
//! drive's own port is touched: stopped, given this driver's command list, started again.

use crate::dma::Page;
use crate::paging::{self, Cache};
use crate::{pci, time};
use alloc::format;
use alloc::string::String;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

// The controller's registers.
const CAP2: u64 = 0x24;
const GHC: u64 = 0x04;
const PI: u64 = 0x0C;
const BOHC: u64 = 0x28;
const GHC_AHCI_ENABLE: u32 = 1 << 31;
const CAP2_HANDOFF: u32 = 1 << 0;
const BOHC_BIOS_OWNED: u32 = 1 << 0;
const BOHC_OS_OWNED: u32 = 1 << 1;
const BOHC_BIOS_BUSY: u32 = 1 << 4;

// A port's registers (0x100 + 0x80 a port).
const PX_CLB: u64 = 0x00; // 64: the command list
const PX_FB: u64 = 0x08; // 64: where received FISes go
const PX_IS: u64 = 0x10;
const PX_CMD: u64 = 0x18;
const PX_TFD: u64 = 0x20;
const PX_SSTS: u64 = 0x28;
const PX_SIG: u64 = 0x24;
const PX_SERR: u64 = 0x30;
const PX_CI: u64 = 0x38;
/// What's on a port, by the signature it sent: a disk (ATA), or not known yet (others are optical drives, port
/// multipliers and enclosures).
const SIG_ATA: u32 = 0x0000_0101;
const SIG_UNKNOWN: u32 = 0xFFFF_FFFF;
const CMD_START: u32 = 1 << 0;
const CMD_SPIN_UP: u32 = 1 << 1;
const CMD_POWER_ON: u32 = 1 << 2;
const CMD_FIS_RECEIVE: u32 = 1 << 4;
const CMD_FIS_RUNNING: u32 = 1 << 14;
const CMD_LIST_RUNNING: u32 = 1 << 15;
const TFD_ERROR: u32 = 1 << 0;
const TFD_DRQ: u32 = 1 << 3;
const TFD_BUSY: u32 = 1 << 7;
const IS_TASK_FILE_ERROR: u32 = 1 << 30;

// ATA commands.
const ATA_IDENTIFY: u8 = 0xEC;
const ATA_READ_DMA_EXT: u8 = 0x25;
const ATA_WRITE_DMA_EXT: u8 = 0x35;
const ATA_FLUSH_CACHE_EXT: u8 = 0xEA;

/// In the command page: the command list (32 headers, 1 KiB), the received FISes (256 bytes), then the one command
/// table (its FIS, then one PRD entry at 0x80).
const LIST: usize = 0;
const RECEIVED: usize = 1024;
const TABLE: usize = 2048;

const COMMAND_MS: u64 = 10_000;

pub struct Ahci {
    pub at: pci::Address,
    regs: u64,
    pub port: u32,
    commands: Page,
    data: Page,
    pub block_size: u32,
    pub blocks: u64,
    pub model: String,
    /// Its port given this driver's memory (so stopped again if it's dropped: only a drive that didn't start is).
    configured: bool,
}

unsafe impl Send for Ahci {}

impl Drop for Ahci {
    fn drop(&mut self) {
        if self.configured {
            // (Stopped, so the port no longer uses this driver's memory, which is given back.)
            self.pw(PX_CMD, self.pr(PX_CMD) & !(CMD_START | CMD_FIS_RECEIVE));
            self.wait(500, |d| d.pr(PX_CMD) & (CMD_LIST_RUNNING | CMD_FIS_RUNNING) == 0);
        }
    }
}

/// The ports of the controller at `f` with a drive connected (by their status: a device there, talking; and not an
/// optical drive's, or anything else that says it isn't a disk), as bits.
pub fn ports_in_use(f: &pci::Function) -> u32 {
    let Some(bar) = f.at.memory_bar(5).filter(|b| b.size >= 0x180) else { return 0 };
    f.at.enable(true);
    let regs = paging::map_device(bar.phys, bar.size.min(0x2000), Cache::Uncached);
    let r = |reg: u64| unsafe { read_volatile((regs + reg) as *const u32) };
    let ports = r(PI);
    (0..32u32)
        .filter(|&p| ports & 1 << p != 0 && bar.size >= 0x100 + 0x80 * (p as u64 + 1))
        .filter(|&p| r(0x100 + 0x80 * p as u64 + PX_SSTS) & 0xF == 3)
        .filter(|&p| matches!(r(0x100 + 0x80 * p as u64 + PX_SIG), SIG_ATA | SIG_UNKNOWN))
        .fold(0, |bits, p| bits | 1 << p)
}

impl Ahci {
    fn r(&self, reg: u64) -> u32 {
        unsafe { read_volatile((self.regs + reg) as *const u32) }
    }
    fn w(&self, reg: u64, v: u32) {
        unsafe { write_volatile((self.regs + reg) as *mut u32, v) }
    }
    fn port_reg(&self, reg: u64) -> u64 {
        0x100 + 0x80 * self.port as u64 + reg
    }
    fn pr(&self, reg: u64) -> u32 {
        self.r(self.port_reg(reg))
    }
    fn pw(&self, reg: u64, v: u32) {
        self.w(self.port_reg(reg), v)
    }

    fn wait(&self, ms: u64, done: impl Fn(&Self) -> bool) -> bool {
        let end = time::now_us() + ms * 1000;
        while time::now_us() < end {
            if done(self) {
                return true;
            }
            time::delay_us(50);
        }
        done(self)
    }

    /// Starts port `port` of the controller at `f`.
    pub fn start(f: &pci::Function, port: u32) -> Result<Ahci, String> {
        f.at.power_on();
        let bar = f.at.memory_bar(5).ok_or_else(|| String::from("its registers aren't in memory"))?;
        if port >= 32 || bar.size < 0x100 + 0x80 * (port as u64 + 1) {
            return Err(format!("it has no port {port}"));
        }
        let regs = paging::map_device(bar.phys, bar.size.min(0x2000), Cache::Uncached);
        f.at.enable(true);
        let (Some(commands), Some(data)) = (Page::new(true), Page::new(true)) else {
            return Err(String::from("out of memory"));
        };
        let mut d = Ahci { at: f.at, regs, port, commands, data, block_size: 512, blocks: 0, model: String::new(), configured: false };

        // The firmware hands the controller over, where it says it takes part in that; then AHCI mode.
        if d.r(CAP2) & CAP2_HANDOFF != 0 && d.r(BOHC) & BOHC_BIOS_OWNED != 0 {
            d.w(BOHC, d.r(BOHC) | BOHC_OS_OWNED);
            d.wait(25, |d| d.r(BOHC) & BOHC_BIOS_OWNED == 0);
            if d.r(BOHC) & BOHC_BIOS_BUSY != 0 {
                d.wait(2000, |d| d.r(BOHC) & BOHC_BIOS_BUSY == 0);
            }
        }
        d.w(GHC, d.r(GHC) | GHC_AHCI_ENABLE);
        if d.r(PI) & 1 << port == 0 {
            return Err(format!("it has no port {port}"));
        }

        // The port stopped (its command list, then its receiving), given this driver's memory, and started again.
        d.pw(PX_CMD, d.pr(PX_CMD) & !CMD_START);
        if !d.wait(500, |d| d.pr(PX_CMD) & CMD_LIST_RUNNING == 0) {
            return Err(String::from("its port didn't stop"));
        }
        d.pw(PX_CMD, d.pr(PX_CMD) & !CMD_FIS_RECEIVE);
        if !d.wait(500, |d| d.pr(PX_CMD) & CMD_FIS_RUNNING == 0) {
            return Err(String::from("its port didn't stop receiving"));
        }
        let base = d.commands.phys;
        d.pw(PX_CLB, (base + LIST as u64) as u32);
        d.pw(PX_CLB + 4, ((base + LIST as u64) >> 32) as u32);
        d.pw(PX_FB, (base + RECEIVED as u64) as u32);
        d.pw(PX_FB + 4, ((base + RECEIVED as u64) >> 32) as u32);
        d.configured = true;
        d.pw(PX_SERR, !0);
        d.pw(PX_IS, !0);
        d.pw(PX_CMD, d.pr(PX_CMD) | CMD_FIS_RECEIVE | CMD_SPIN_UP | CMD_POWER_ON);
        if d.pr(PX_SSTS) & 0xF != 3 {
            return Err(format!("nothing is connected to port {port}"));
        }
        if !d.wait(1000, |d| d.pr(PX_TFD) & (TFD_BUSY | TFD_DRQ) == 0) {
            return Err(String::from("its drive stayed busy"));
        }
        d.pw(PX_CMD, d.pr(PX_CMD) | CMD_START);

        // What it is (ATA IDENTIFY DEVICE: 256 words).
        d.command(ATA_IDENTIFY, 0, 0, 512, false).map_err(|e| format!("its drive didn't say what it is: {e}"))?;
        let word = |i: usize| unsafe { read_volatile(d.data.at::<u16>(2 * i)) };
        if word(83) & 1 << 10 == 0 {
            return Err(String::from("its drive doesn't take 48-bit block numbers"));
        }
        d.blocks = (0..4).fold(0u64, |n, i| n | (word(100 + i) as u64) << (16 * i));
        // A logical sector bigger than 512 bytes says so (word 106 bit 12; its size, in words, in 117-118).
        if word(106) & 0xC000 == 0x4000 && word(106) & 1 << 12 != 0 {
            d.block_size = 2 * (word(117) as u32 | (word(118) as u32) << 16);
        }
        if !matches!(d.block_size, 512 | 1024 | 2048 | 4096) || d.blocks == 0 {
            return Err(String::from("its drive's blocks aren't ones this driver can use"));
        }
        let mut model = alloc::vec::Vec::new();
        for i in 27..47 {
            let w = word(i);
            model.push((w >> 8) as u8);
            model.push(w as u8);
        }
        d.model = String::from(String::from_utf8_lossy(&model).trim());
        Ok(d)
    }

    /// One ATA command through slot 0: `bytes` to or from the data page (0: none). Waits for it to finish.
    fn command(&mut self, ata: u8, lba: u64, count: u32, bytes: u32, write: bool) -> Result<(), String> {
        if bytes > 4096 {
            return Err(String::from("more than a page at once"));
        }
        if !self.wait(COMMAND_MS, |d| d.pr(PX_TFD) & (TFD_BUSY | TFD_DRQ) == 0) {
            return Err(String::from("the drive is busy"));
        }
        let table = self.commands.phys + TABLE as u64;
        unsafe {
            // The command header: a 5-dword FIS, written to the drive or not, one PRD entry (or none).
            let header = self.commands.at::<u32>(LIST);
            let prds = if bytes > 0 { 1 } else { 0 };
            write_volatile(header, 5 | (write as u32) << 6 | prds << 16);
            write_volatile(header.add(1), 0);
            write_volatile(header.add(2), table as u32);
            write_volatile(header.add(3), (table >> 32) as u32);
            // The command FIS (Register, host to device), then the PRD entry.
            let fis = self.commands.at::<u8>(TABLE);
            for i in 0..64 {
                write_volatile(fis.add(i), 0);
            }
            let set = |i: usize, v: u8| write_volatile(fis.add(i), v);
            set(0, 0x27);
            set(1, 0x80); // a command
            set(2, ata);
            set(4, lba as u8);
            set(5, (lba >> 8) as u8);
            set(6, (lba >> 16) as u8);
            set(7, 0x40); // LBA addressing
            set(8, (lba >> 24) as u8);
            set(9, (lba >> 32) as u8);
            set(10, (lba >> 40) as u8);
            set(12, count as u8);
            set(13, (count >> 8) as u8);
            let prd = self.commands.at::<u32>(TABLE + 0x80);
            write_volatile(prd, self.data.phys as u32);
            write_volatile(prd.add(1), (self.data.phys >> 32) as u32);
            write_volatile(prd.add(2), 0);
            write_volatile(prd.add(3), bytes.saturating_sub(1));
        }
        fence(Ordering::SeqCst);
        self.pw(PX_IS, !0);
        self.pw(PX_CI, 1);
        let done = self.wait(COMMAND_MS, |d| d.pr(PX_CI) & 1 == 0 || d.pr(PX_IS) & IS_TASK_FILE_ERROR != 0);
        fence(Ordering::SeqCst);
        if self.pr(PX_IS) & IS_TASK_FILE_ERROR != 0 || self.pr(PX_TFD) & TFD_ERROR != 0 {
            let tfd = self.pr(PX_TFD);
            // The port stops itself after an error: started again for the next command.
            self.pw(PX_CMD, self.pr(PX_CMD) & !CMD_START);
            self.wait(500, |d| d.pr(PX_CMD) & CMD_LIST_RUNNING == 0);
            self.pw(PX_SERR, !0);
            self.pw(PX_IS, !0);
            self.pw(PX_CMD, self.pr(PX_CMD) | CMD_START);
            return Err(format!("the drive reported an error ({:#x})", tfd >> 8 & 0xFF));
        }
        if !done {
            return Err(String::from("no answer"));
        }
        Ok(())
    }

    fn check(&self, lba: u64, len: usize) -> Result<u32, String> {
        let count = len as u32 / self.block_size;
        if count == 0 || len > 4096 || !(len as u32).is_multiple_of(self.block_size) {
            return Err(String::from("not a whole number of blocks, or more than a page"));
        }
        if lba.checked_add(count as u64).is_none_or(|end| end > self.blocks) {
            return Err(String::from("past the end of the drive"));
        }
        Ok(count)
    }
}

impl super::Disk for Ahci {
    fn block_size(&self) -> u32 {
        self.block_size
    }

    fn blocks(&self) -> u64 {
        self.blocks
    }

    fn read(&mut self, lba: u64, out: &mut [u8]) -> Result<(), String> {
        let count = self.check(lba, out.len())?;
        self.command(ATA_READ_DMA_EXT, lba, count, out.len() as u32, false)?;
        for (i, b) in out.iter_mut().enumerate() {
            *b = unsafe { read_volatile(self.data.at::<u8>(i)) };
        }
        Ok(())
    }

    fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), String> {
        let count = self.check(lba, data.len())?;
        for (i, b) in data.iter().enumerate() {
            unsafe { write_volatile(self.data.at::<u8>(i), *b) };
        }
        self.command(ATA_WRITE_DMA_EXT, lba, count, data.len() as u32, true)
    }

    fn flush(&mut self) -> Result<(), String> {
        self.command(ATA_FLUSH_CACHE_EXT, 0, 0, 0, false)
    }

    fn shut_down(&mut self) -> bool {
        self.command(ATA_FLUSH_CACHE_EXT, 0, 0, 0, false).is_ok()
    }

    fn describe(&self) -> String {
        format!("SATA drive \u{201C}{}\u{201D} on port {} of the controller at {}", self.model, self.port, self.at)
    }
}
