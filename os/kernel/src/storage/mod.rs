//! Storage: the drive Chat Computer's start-up partition is on, and Chat Computer's memory file in that partition
//! (`\EFI\chatcomputer-own\memory.bin`), where the conversation is kept across restarts.
//!
//! The start-up stage says which drive and partition it was started from (the firmware's device path for it: the
//! PCI devices on the way to the controller, the NVMe namespace or SATA port, the partition's first block). The
//! partition is the PC's EFI system partition, which Windows' own start-up files are on too, so nothing in it is
//! changed but the memory file's own bytes, in place: Windows' setup made the file at its full size, every 512 bytes
//! of it marked with a tag (`memory::TAG`) that names the file and the place in it, and every write here first reads
//! the sector it's for and checks its tags. Its file system's structures (FAT) are read, never written (fat crate).

pub mod ahci;
pub mod nvme;

use crate::pci;
use crate::sync::SpinLock;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use bootinfo::StartDisk;

/// What a drive's driver does: whole blocks, a page (4 KiB) at most at a time.
pub trait Disk: Send {
    /// Bytes in a block (its logical sector): 512 or 4096, mostly.
    fn block_size(&self) -> u32;
    fn blocks(&self) -> u64;
    fn read(&mut self, lba: u64, out: &mut [u8]) -> Result<(), String>;
    fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), String>;
    /// Makes sure what was written is on the drive, not just in its cache.
    fn flush(&mut self) -> Result<(), String>;
    /// Before the PC restarts: whatever the drive has cached written.
    fn shut_down(&mut self) -> bool;
    fn describe(&self) -> String;
}

/// The PCI device at the end of the start-up stage's path: from the root's bus, through each bridge's secondary bus.
fn follow(d: &StartDisk) -> Result<pci::Function, String> {
    if d.pci_root != 0 {
        return Err(format!("its controller is under PCI root {} (only the first is known)", d.pci_root));
    }
    let path = &d.path[..(d.path_len as usize).min(d.path.len())];
    let mut bus = 0u8;
    let mut found = None;
    for (i, (device, function)) in path.iter().map(|p| (p[0], p[1])).enumerate() {
        let f = pci::at(bus, device, function).ok_or_else(|| format!("nothing at {bus:02x}:{device:02x}.{function}"))?;
        if i + 1 < path.len() {
            if f.class != 0x06 || f.subclass != 0x04 {
                return Err(format!("{} on the way to the drive isn't a bridge", f.at));
            }
            bus = f.at.read8(0x19);
        }
        found = Some(f);
    }
    found.ok_or_else(|| String::from("the start-up stage gave no PCI path"))
}

/// The start-up drive, once started: the memory file is on it (keep.rs), and Windows' drive usually is too
/// (files.rs). Used from the main loop only, taken out of its lock while it works (a read or save takes a moment).
static START: SpinLock<Option<Box<dyn Disk>>> = SpinLock::new(None);
/// What it is (for the computer's status), or why it couldn't be started.
static START_SAID: SpinLock<Option<Result<String, String>>> = SpinLock::new(None);

/// Starts the drive the start-up partition is on. Says which, or why it couldn't, for the log.
pub fn open_start(d: &StartDisk) -> Result<String, String> {
    let result = if d.kind == bootinfo::DISK_UNKNOWN {
        Err(String::from("the start-up partition isn't on an NVMe or SATA drive"))
    } else {
        start_disk(d).map(|disk| {
            let said = disk.describe();
            *START.lock() = Some(disk);
            said
        })
    };
    *START_SAID.lock() = Some(result.clone());
    result
}

/// What the start-up drive is, or why it isn't started (None: not tried yet).
pub fn start_said() -> Option<Result<String, String>> {
    START_SAID.lock().clone()
}

/// Runs `f` with the start-up drive, nothing locked meanwhile (None if it isn't started).
pub fn with_start<T>(f: impl FnOnce(&mut dyn Disk) -> T) -> Option<T> {
    let mut disk = START.lock().take()?;
    let result = f(disk.as_mut());
    *START.lock() = Some(disk);
    Some(result)
}

/// Before the PC restarts or turns off: the start-up drive writes what it has cached. (Not if it's in use: a crash
/// in the middle of a save.)
pub fn shut_down() {
    if let Some(mut guard) = START.try_lock()
        && let Some(disk) = guard.as_mut()
    {
        let _ = disk.flush();
        disk.shut_down();
    }
}

/// The drive the start-up partition is on, started (or why not).
fn start_disk(d: &StartDisk) -> Result<Box<dyn Disk>, String> {
    let f = follow(d)?;
    match d.kind {
        bootinfo::DISK_NVME if (f.class, f.subclass, f.interface) == (0x01, 0x08, 0x02) => {
            Ok(Box::new(nvme::Nvme::start(&f, d.nvme_namespace)?))
        }
        bootinfo::DISK_SATA if (f.class, f.subclass, f.interface) == (0x01, 0x06, 0x01) => {
            if d.sata_multiplier != 0xFFFF && d.sata_multiplier != 0 {
                return Err(String::from("the drive is behind a port multiplier"));
            }
            Ok(Box::new(ahci::Ahci::start(&f, d.sata_port as u32)?))
        }
        bootinfo::DISK_NVME | bootinfo::DISK_SATA => Err(format!("the device at {} isn't the controller expected", f.at)),
        _ => Err(String::from("it's on a kind of drive Chat Computer has no driver for yet (USB, say)")),
    }
}

/// The start-up partition's sectors, for the FAT reader: the partition's first block, and the drive.
pub struct Partition<'a> {
    pub disk: &'a mut dyn Disk,
    pub start: u64,
    pub blocks: u64,
}

impl fat::Sectors for Partition<'_> {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> bool {
        // FAT's sectors are the drive's blocks (checked in `open`).
        if buf.len() as u32 != self.disk.block_size() || sector >= self.blocks {
            return false;
        }
        self.disk.read(self.start + sector, buf).is_ok()
    }
}

/// A file in the partition: which of the drive's blocks hold it, in order.
pub struct FileBlocks {
    pub blocks: Vec<u64>,
    pub block_size: u32,
}

/// Finds `path` in the start-up partition (a FAT volume whose sectors are the drive's blocks).
pub fn find_file(disk: &mut dyn Disk, start: u64, blocks: u64, path: &[&str]) -> Result<FileBlocks, String> {
    let block_size = disk.block_size();
    if blocks == 0 || start.checked_add(blocks).is_none_or(|end| end > disk.blocks()) {
        return Err(String::from("the start-up stage's partition isn't on this drive"));
    }
    let mut p = Partition { disk, start, blocks };
    // The FAT reader reads its first sector as 512 bytes; the drive reads whole blocks.
    let mut first = vec![0u8; block_size as usize];
    if !fat::Sectors::read(&mut p, 0, &mut first) {
        return Err(String::from("the partition's first block couldn't be read"));
    }
    let sector_size = u16::from_le_bytes([first[11], first[12]]) as u32;
    if sector_size != block_size {
        return Err(format!("its file system's sectors ({sector_size} bytes) aren't the drive's blocks ({block_size})"));
    }
    let mut sectors = BlockSized(&mut p);
    let v = fat::open(&mut sectors).map_err(|e| format!("the partition's file system: {e:?}"))?;
    let entry = v.find(&mut sectors, path).map_err(|e| format!("{}: {e:?}", path.join("\\")))?;
    if entry.folder {
        return Err(format!("{} is a folder", path.join("\\")));
    }
    let file = v.file_sectors(&mut sectors, &entry).map_err(|e| format!("{}: {e:?}", path.join("\\")))?;
    Ok(FileBlocks { blocks: file.iter().map(|s| start + s).collect(), block_size })
}

/// The FAT reader asks for the first sector as 512 bytes before it knows the sector size: given the start of the
/// whole block.
struct BlockSized<'a, 'b>(&'a mut Partition<'b>);

impl fat::Sectors for BlockSized<'_, '_> {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> bool {
        let size = self.0.disk.block_size() as usize;
        if buf.len() == size {
            return self.0.read(sector, buf);
        }
        let mut whole = vec![0u8; size];
        if buf.len() > size || !self.0.read(sector, &mut whole) {
            return false;
        }
        buf.copy_from_slice(&whole[..buf.len()]);
        true
    }
}
