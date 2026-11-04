//! The kernel's log, kept on the start-up partition in `\EFI\chatcomputer-own\log.bin`, for Windows' setup to read
//! back the next time it runs (windows/lib/OwnOs.ps1): most PCs have no serial port, so without it what the kernel
//! found on a PC, how far its start got and why it stopped would be seen by no one. The file is made by the setup as
//! the memory file is (the remember crate's format: every chunk tagged, two copies written in turn, so a whole one is
//! always there), with an id of its own (`log-id` in the settings), and only ever changed in place.
//!
//! What's saved is the log as kept in memory (serial.rs): its start, and its latest part. It's saved once the
//! start-up drive is found, before each of the start's riskier steps, when the kernel is ready, every few seconds
//! when there's more, before the PC turns off or restarts, and when the kernel stops (if the drive isn't in the
//! middle of something then).

use crate::storage::{self, Disk, FileBlocks};
use crate::sync::SpinLock;
use crate::{heap, serial, time};
use alloc::format;
use alloc::string::String;
use bootinfo::StartDisk;

const PATH: [&str; 3] = ["EFI", "chatcomputer-own", "log.bin"];
/// How often the main loop saves it, when there's more.
const EVERY_MS: u64 = 10_000;

struct Journal {
    file: FileBlocks,
    memory: remember::Memory,
    /// How much of the log had been written at the last save, and when that was.
    saved: u64,
    at: u64,
}

static JOURNAL: SpinLock<Option<Journal>> = SpinLock::new(None);

/// The log file's sectors, for the remember crate.
struct FileStore<'a> {
    disk: &'a mut dyn Disk,
    file: &'a FileBlocks,
}

impl remember::Store for FileStore<'_> {
    fn sector_size(&self) -> usize {
        self.file.block_size as usize
    }
    fn sectors(&self) -> usize {
        self.file.blocks.len()
    }
    fn read(&mut self, sector: usize, buf: &mut [u8]) -> bool {
        self.file.blocks.get(sector).is_some_and(|&lba| self.disk.read(lba, buf).is_ok())
    }
    fn write(&mut self, sector: usize, buf: &[u8]) -> bool {
        self.file.blocks.get(sector).is_some_and(|&lba| self.disk.write(lba, buf).is_ok())
    }
    fn flush(&mut self) -> bool {
        self.disk.flush().is_ok()
    }
}

/// Finds the log file on the start-up drive (storage::open_start has started it). Returns what to log (saved with
/// the rest at the next `save`).
pub fn init(start: &StartDisk, id: Option<u32>) -> String {
    let Some(id) = id else {
        return String::from("log: not kept on the start-up partition (the settings have no log id: set up again on Windows)");
    };
    let found = storage::with_start(|disk| {
        let file = storage::find_file(disk, start.partition_start, start.partition_blocks, &PATH)?;
        let store = FileStore { disk, file: &file };
        let memory = remember::Memory::new(&store, id).map_err(|e| format!("its file: {e:?}"))?;
        Ok::<_, String>((file, memory))
    });
    match found {
        Some(Ok((file, memory))) => {
            let room = memory.capacity() / 1024;
            *JOURNAL.lock() = Some(Journal { file, memory, saved: 0, at: 0 });
            format!("log: kept on the start-up partition too, for Windows to read back (up to {room} KB)")
        }
        Some(Err(why)) => format!("log: not kept on the start-up partition: {why}"),
        None => String::from("log: not kept on the start-up partition (the start-up drive isn't started)"),
    }
}

/// Saves the log if there's more of it than was saved. Whether it's saved now (or there was nothing new).
pub fn save() -> bool {
    let Some(mut j) = JOURNAL.lock().take() else { return false };
    let Some((log, written)) = serial::kept(true) else {
        *JOURNAL.lock() = Some(j);
        return false;
    };
    let mut ok = true;
    if written != j.saved {
        let result = storage::with_start(|disk| {
            let mut store = FileStore { disk, file: &j.file };
            j.memory.save(&mut store, &log[..log.len().min(j.memory.capacity())])
        });
        ok = matches!(result, Some(Ok(())));
        if ok {
            j.saved = written;
        }
    }
    j.at = time::uptime_ms();
    *JOURNAL.lock() = Some(j);
    ok
}

/// From the main loop: saves the log every few seconds, when there's more.
pub fn work() {
    let due = JOURNAL.lock().as_ref().is_some_and(|j| time::uptime_ms() >= j.at + EVERY_MS);
    if due && serial::written() != JOURNAL.lock().as_ref().map_or(0, |j| j.saved) {
        save();
    }
}

/// When the kernel stops: the log saved, with its last words, if nothing it needs is in the middle of being used
/// (never waits: the stop may have come from inside any of it).
pub fn save_on_stop() {
    if heap::busy() {
        return;
    }
    let Some(mut guard) = JOURNAL.try_lock() else { return };
    let Some(j) = guard.as_mut() else { return };
    let Some((log, _)) = serial::kept(false) else { return };
    let (memory, file) = (&j.memory, &j.file);
    let _ = storage::try_with_start(|disk| {
        let mut store = FileStore { disk, file };
        let saved = memory.save(&mut store, &log[..log.len().min(memory.capacity())]);
        let _ = disk.flush();
        saved
    });
}
