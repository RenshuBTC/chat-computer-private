//! NVMe SSDs (the NVM Express base specification): the standard interface of every PC's SSD that plugs into an M.2
//! socket's PCI Express lanes, so one driver covers them all. Chat Computer uses them only a little: a few blocks of
//! its own memory file on the one its start-up partition is on, read and written now and then, and Windows' files
//! read, on that one and the others; so this driver keeps it simple: one queue of commands (and the administration
//! queue to set it up), one 4 KiB page a command, and the answers waited for (polled) rather than interrupting.
//!
//! The controller is reset first: the firmware's driver used it until the start-up stage ended the firmware's
//! services, and a reset is the specification's way to take a controller over. Before the PC restarts, it's told to
//! shut down (so what it has cached is written).

use crate::dma::Page;
use crate::paging::{self, Cache};
use crate::{pci, time};
use alloc::format;
use alloc::string::String;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

// Registers.
const CAP: u64 = 0x00; // 64
const CC: u64 = 0x14;
const CSTS: u64 = 0x1C;
const AQA: u64 = 0x24;
const ASQ: u64 = 0x28; // 64
const ACQ: u64 = 0x30; // 64
const DOORBELLS: u64 = 0x1000;

const CC_ENABLE: u32 = 1 << 0;
/// Submission queue entries of 64 bytes (2^6), completion entries of 16 (2^4); 4 KiB pages (MPS 0); the NVM
/// command set (CSS 0).
const CC_QUEUE_SIZES: u32 = 6 << 16 | 4 << 20;
const CC_SHUTDOWN_NORMAL: u32 = 1 << 14;
const CSTS_READY: u32 = 1 << 0;
const CSTS_FATAL: u32 = 1 << 1;
const CSTS_SHUTDOWN_MASK: u32 = 3 << 2;
const CSTS_SHUTDOWN_DONE: u32 = 2 << 2;

// Commands: the administration queue's, and reading and writing (the NVM command set's).
const ADMIN_CREATE_SQ: u8 = 0x01;
const ADMIN_CREATE_CQ: u8 = 0x05;
const ADMIN_IDENTIFY: u8 = 0x06;
const IO_FLUSH: u8 = 0x00;
const IO_WRITE: u8 = 0x01;
const IO_READ: u8 = 0x02;

/// Entries in each queue (a page of submissions holds 64).
const ENTRIES: u16 = 64;
/// How long one command may take before it's given up.
const COMMAND_MS: u64 = 10_000;

/// A queue pair: submissions (64 bytes each) and completions (16 bytes each), each a page.
struct Queue {
    id: u16,
    sq: Page,
    cq: Page,
    tail: u16,
    head: u16,
    /// The phase the next completion will have (the controller flips it each time round the queue).
    phase: bool,
    next_id: u16,
}

pub struct Nvme {
    pub at: pci::Address,
    regs: u64,
    stride: u64,
    timeout_ms: u64,
    admin: Queue,
    io: Queue,
    /// The namespace used: its id, its block size (bytes) and its size (blocks).
    pub namespace: u32,
    pub block_size: u32,
    pub blocks: u64,
    /// The model, as the drive says it (for the log).
    pub model: String,
    /// A page for data going to and from the drive.
    data: Page,
    /// Switched on with this driver's queues (so switched off again if it's dropped: only a drive that didn't
    /// start is).
    enabled: bool,
}

impl Drop for Nvme {
    fn drop(&mut self) {
        if self.enabled {
            // (Off, so it no longer uses this driver's memory, which is given back.)
            self.w32(CC, self.r32(CC) & !CC_ENABLE);
            self.wait(|d| d.r32(CSTS) & CSTS_READY == 0);
        }
    }
}

unsafe impl Send for Nvme {}

impl Nvme {
    fn r32(&self, reg: u64) -> u32 {
        unsafe { read_volatile((self.regs + reg) as *const u32) }
    }
    fn w32(&self, reg: u64, v: u32) {
        unsafe { write_volatile((self.regs + reg) as *mut u32, v) }
    }
    fn r64(&self, reg: u64) -> u64 {
        self.r32(reg) as u64 | (self.r32(reg + 4) as u64) << 32
    }
    fn w64(&self, reg: u64, v: u64) {
        self.w32(reg, v as u32);
        self.w32(reg + 4, (v >> 32) as u32);
    }

    /// Waits up to the controller's own timeout for `done`.
    fn wait(&self, done: impl Fn(&Self) -> bool) -> bool {
        let end = time::now_us() + self.timeout_ms * 1000;
        while time::now_us() < end {
            if done(self) {
                return true;
            }
            time::delay_us(100);
        }
        done(self)
    }

    fn doorbell(&self, queue: u16, completion: bool) -> u64 {
        DOORBELLS + (2 * queue as u64 + completion as u64) * (4 << self.stride)
    }

    /// Sends one command on `queue` (the admin queue if `admin`), waits for its completion. Ok: the command's
    /// result (its first dword); Err: its status, or that it never completed.
    fn command(&mut self, admin: bool, mut entry: [u32; 16]) -> Result<u32, String> {
        let q = if admin { &mut self.admin } else { &mut self.io };
        let id = q.next_id;
        q.next_id = q.next_id.wrapping_add(1);
        entry[0] = (entry[0] & 0xFFFF) | (id as u32) << 16;
        let slot = q.sq.at::<u32>(q.tail as usize * 64);
        for (i, dword) in entry.iter().enumerate() {
            unsafe { write_volatile(slot.add(i), *dword) };
        }
        q.tail = (q.tail + 1) % ENTRIES;
        let (qid, tail) = (q.id, q.tail);
        fence(Ordering::SeqCst);
        let bell = self.doorbell(qid, false);
        self.w32(bell, tail as u32);
        let end = time::now_us() + COMMAND_MS * 1000;
        loop {
            let q = if admin { &mut self.admin } else { &mut self.io };
            let done = q.cq.at::<u32>(q.head as usize * 16);
            let dw3 = unsafe { read_volatile(done.add(3)) };
            if (dw3 & 1 << 16 != 0) == q.phase {
                fence(Ordering::SeqCst);
                let result = unsafe { read_volatile(done) };
                q.head = (q.head + 1) % ENTRIES;
                if q.head == 0 {
                    q.phase = !q.phase;
                }
                let (qid, head) = (q.id, q.head);
                let bell = self.doorbell(qid, true);
                self.w32(bell, head as u32);
                let status = (dw3 >> 17) & 0x7FFF;
                if dw3 & 0xFFFF != id as u32 {
                    return Err(format!("an answer to another command ({:#x})", dw3 & 0xFFFF));
                }
                return if status == 0 { Ok(result) } else { Err(format!("status {status:#x}")) };
            }
            if time::now_us() > end {
                return Err(String::from("no answer"));
            }
            if self.r32(CSTS) & CSTS_FATAL != 0 {
                return Err(String::from("the controller stopped with a fatal error"));
            }
            core::hint::spin_loop();
        }
    }

    /// Starts the controller at `f` and finds namespace `namespace` (or the first one, if 0).
    pub fn start(f: &pci::Function, namespace: u32) -> Result<Nvme, String> {
        f.at.power_on();
        let bar = f.at.memory_bar(0).ok_or_else(|| String::from("its registers aren't in memory"))?;
        if bar.size < 0x2000 {
            return Err(String::from("its register region is too small"));
        }
        let regs = paging::map_device(bar.phys, bar.size.min(0x4000), Cache::Uncached);
        f.at.enable(true);
        let new_queue = |id: u16| -> Option<Queue> {
            Some(Queue { id, sq: Page::new(true)?, cq: Page::new(true)?, tail: 0, head: 0, phase: true, next_id: 0 })
        };
        let (Some(admin), Some(io), Some(data)) = (new_queue(0), new_queue(1), Page::new(true)) else {
            return Err(String::from("out of memory"));
        };
        let mut d = Nvme {
            at: f.at,
            regs,
            stride: 0,
            timeout_ms: 0,
            admin,
            io,
            namespace: 0,
            block_size: 0,
            blocks: 0,
            model: String::new(),
            data,
            enabled: false,
        };
        let cap = d.r64(CAP);
        if cap == !0 {
            return Err(String::from("it doesn't answer"));
        }
        d.stride = (cap >> 32) & 0xF;
        d.timeout_ms = (((cap >> 24) & 0xFF).max(1)) * 500;
        if (cap >> 48) & 0xF != 0 {
            return Err(String::from("it doesn't take 4 KiB pages"));
        }
        if (cap >> 37) & 1 == 0 {
            return Err(String::from("it doesn't have the NVM command set"));
        }
        if (cap & 0xFFFF) + 1 < ENTRIES as u64 {
            return Err(String::from("its queues are too short"));
        }

        // Off (a controller just switched on must say it's ready before it's switched off), then on with this
        // driver's queues.
        let cc = d.r32(CC);
        if cc & CC_ENABLE != 0 {
            d.wait(|d| d.r32(CSTS) & (CSTS_READY | CSTS_FATAL) != 0);
            d.w32(CC, cc & !CC_ENABLE);
        }
        if !d.wait(|d| d.r32(CSTS) & CSTS_READY == 0) {
            return Err(String::from("it didn't switch off"));
        }
        d.w32(AQA, ((ENTRIES - 1) as u32) << 16 | (ENTRIES - 1) as u32);
        d.w64(ASQ, d.admin.sq.phys);
        d.w64(ACQ, d.admin.cq.phys);
        d.w32(CC, CC_QUEUE_SIZES | CC_ENABLE);
        d.enabled = true;
        if !d.wait(|d| d.r32(CSTS) & (CSTS_READY | CSTS_FATAL) != 0) || d.r32(CSTS) & CSTS_FATAL != 0 {
            return Err(String::from("it didn't start"));
        }

        // What it is (the controller's identity: its model at bytes 24-63).
        let page = d.data.phys;
        d.command(true, identify(0, 1, page)).map_err(|e| format!("it didn't say what it is: {e}"))?;
        let model: alloc::vec::Vec<u8> = (24..64).map(|i| unsafe { read_volatile(d.data.at::<u8>(i)) }).collect();
        d.model = String::from(String::from_utf8_lossy(&model).trim());

        // The namespace: the one asked for, else the first of its active ones.
        d.namespace = if namespace != 0 {
            namespace
        } else {
            d.command(true, identify(0, 2, page)).map_err(|e| format!("it didn't list its namespaces: {e}"))?;
            match unsafe { read_volatile(d.data.at::<u32>(0)) } {
                0 => return Err(String::from("it has no namespace")),
                n => n,
            }
        };
        d.command(true, identify(d.namespace, 0, page)).map_err(|e| format!("it didn't describe namespace {}: {e}", d.namespace))?;
        d.blocks = unsafe { read_volatile(d.data.at::<u64>(0)) };
        let format = unsafe { read_volatile(d.data.at::<u8>(26)) } & 0xF;
        let lba_format = unsafe { read_volatile(d.data.at::<u32>(128 + 4 * format as usize)) };
        let (metadata, shift) = (lba_format & 0xFFFF, (lba_format >> 16) & 0xFF);
        if metadata != 0 || !(9..=12).contains(&shift) || d.blocks == 0 {
            return Err(format!("its namespace {} has blocks this driver can't use", d.namespace));
        }
        d.block_size = 1 << shift;

        // The I/O queue pair: completions first, then the submissions that go to them (no interrupts).
        let cq = d.io.cq.phys;
        let mut create_cq = [0u32; 16];
        create_cq[0] = ADMIN_CREATE_CQ as u32;
        create_cq[6] = cq as u32;
        create_cq[7] = (cq >> 32) as u32;
        create_cq[10] = ((ENTRIES - 1) as u32) << 16 | 1;
        create_cq[11] = 1; // physically contiguous, no interrupts
        d.command(true, create_cq).map_err(|e| format!("it didn't make a completion queue: {e}"))?;
        let sq = d.io.sq.phys;
        let mut create_sq = [0u32; 16];
        create_sq[0] = ADMIN_CREATE_SQ as u32;
        create_sq[6] = sq as u32;
        create_sq[7] = (sq >> 32) as u32;
        create_sq[10] = ((ENTRIES - 1) as u32) << 16 | 1;
        create_sq[11] = 1 << 16 | 1; // to completion queue 1, physically contiguous
        d.command(true, create_sq).map_err(|e| format!("it didn't make a submission queue: {e}"))?;
        Ok(d)
    }

    /// Reads or writes `count` blocks at block `lba` through the data page (4 KiB at most).
    fn transfer(&mut self, write: bool, lba: u64, count: u32) -> Result<(), String> {
        if count == 0 || count * self.block_size > 4096 {
            return Err(String::from("more than a page at once"));
        }
        if lba.checked_add(count as u64).is_none_or(|end| end > self.blocks) {
            return Err(String::from("past the end of the drive"));
        }
        let mut e = [0u32; 16];
        e[0] = if write { IO_WRITE } else { IO_READ } as u32;
        e[1] = self.namespace;
        e[6] = self.data.phys as u32;
        e[7] = (self.data.phys >> 32) as u32;
        e[10] = lba as u32;
        e[11] = (lba >> 32) as u32;
        e[12] = count - 1;
        self.command(false, e).map(|_| ())
    }

}

impl super::Disk for Nvme {
    fn block_size(&self) -> u32 {
        self.block_size
    }

    fn blocks(&self) -> u64 {
        self.blocks
    }

    fn read(&mut self, lba: u64, out: &mut [u8]) -> Result<(), String> {
        self.transfer(false, lba, out.len() as u32 / self.block_size)?;
        for (i, b) in out.iter_mut().enumerate() {
            *b = unsafe { read_volatile(self.data.at::<u8>(i)) };
        }
        Ok(())
    }

    fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), String> {
        for (i, b) in data.iter().enumerate() {
            unsafe { write_volatile(self.data.at::<u8>(i), *b) };
        }
        self.transfer(true, lba, data.len() as u32 / self.block_size)
    }

    fn flush(&mut self) -> Result<(), String> {
        let mut flush = [0u32; 16];
        flush[0] = IO_FLUSH as u32;
        flush[1] = self.namespace;
        self.command(false, flush).map(|_| ())
    }

    /// Tells the controller the PC is about to restart, so it writes what it has cached, and waits for it to say done.
    fn shut_down(&mut self) -> bool {
        self.w32(CC, (self.r32(CC) & !(3 << 14)) | CC_SHUTDOWN_NORMAL);
        self.wait(|d| d.r32(CSTS) & CSTS_SHUTDOWN_MASK == CSTS_SHUTDOWN_DONE)
    }

    fn describe(&self) -> String {
        format!("NVMe SSD \u{201C}{}\u{201D} at {}, namespace {}", self.model, self.at, self.namespace)
    }
}

/// An Identify command: what `cns` says (1: the controller; 0: namespace `namespace`; 2: the active namespaces' ids),
/// into the page at `page`.
fn identify(namespace: u32, cns: u32, page: u64) -> [u32; 16] {
    let mut e = [0u32; 16];
    e[0] = ADMIN_IDENTIFY as u32;
    e[1] = namespace;
    e[6] = page as u32;
    e[7] = (page >> 32) as u32;
    e[10] = cns;
    e
}
