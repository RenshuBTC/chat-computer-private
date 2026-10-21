//! Keeping the conversation across restarts, in Chat Computer's memory file (`\EFI\chatcomputer-own\memory.bin` on
//! the start-up partition; its format: the remember crate). The start-up stage says which drive and partition it
//! came from; the drive's driver (storage) and the FAT reader (fat) find the file's blocks; every block written is
//! first read and its tags checked, so nothing but the file is ever written. Windows' setup makes the file, and puts
//! its id in the settings; without either, nothing is kept and the log says why.
//!
//! The drive is used from the main loop only, with nothing locked while it works (a save takes a moment), and told to
//! shut down (write what it has cached) before the PC restarts.

use crate::storage::{self, Disk, FileBlocks};
use crate::sync::SpinLock;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use bootinfo::StartDisk;

const PATH: [&str; 3] = ["EFI", "chatcomputer-own", "memory.bin"];

struct Kept {
    file: FileBlocks,
    memory: remember::Memory,
}

static KEPT: SpinLock<Option<Kept>> = SpinLock::new(None);
/// Why nothing is kept, if it isn't (for the computer's status).
static WHY_NOT: SpinLock<Option<String>> = SpinLock::new(None);

/// The memory file's sectors, for the remember crate: the file's blocks on the drive.
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

/// Finds the memory file on the start-up drive (storage::open_start has started it). Returns what to log.
pub fn init(start: &StartDisk, id: Option<u32>) -> String {
    let result = (|| -> Result<(Kept, String), String> {
        let id = id.ok_or_else(|| String::from("the settings have no memory id (set up again on Windows to make one)"))?;
        let described = match storage::start_said() {
            Some(Ok(d)) => d,
            Some(Err(e)) => return Err(format!("the start-up drive: {e}")),
            None => return Err(String::from("the start-up drive wasn't started")),
        };
        storage::with_start(|disk| {
            let file = storage::find_file(disk, start.partition_start, start.partition_blocks, &PATH)
                .map_err(|e| format!("{described}: {e}"))?;
            let store = FileStore { disk, file: &file };
            let memory = remember::Memory::new(&store, id).map_err(|e| format!("the memory file: {e:?}"))?;
            let note = format!("{described}; the memory file holds {} KB", memory.capacity() / 1024);
            Ok((Kept { file, memory }, note))
        })
        .unwrap_or_else(|| Err(String::from("the start-up drive is busy")))
    })();
    match result {
        Ok((kept, note)) => {
            *KEPT.lock() = Some(kept);
            format!("memory: {note}")
        }
        Err(why) => {
            let log = format!("memory: nothing kept across restarts: {why}");
            *WHY_NOT.lock() = Some(why);
            log
        }
    }
}

/// Runs `f` with the memory file and the drive, nothing locked meanwhile (None if there's no memory file).
fn with<T>(f: impl FnOnce(&mut Kept, &mut dyn Disk) -> T) -> Option<T> {
    let mut kept = KEPT.lock().take()?;
    let result = storage::with_start(|disk| f(&mut kept, disk));
    *KEPT.lock() = Some(kept);
    result
}

/// The most that can be kept, in bytes (0 if nothing can).
pub fn capacity() -> usize {
    KEPT.lock().as_ref().map_or(0, |k| k.memory.capacity())
}

/// What was kept before the last restart, if anything.
pub fn load() -> Option<Vec<u8>> {
    with(|k, disk| {
        let mut store = FileStore { disk, file: &k.file };
        match k.memory.load(&mut store) {
            Ok(Some((_, sequence, data))) => {
                crate::log!("Chat Computer kernel: memory: copy {sequence} read ({} bytes)", data.len());
                Some(data)
            }
            Ok(None) => None,
            Err(e) => {
                crate::log!("Chat Computer kernel: memory: couldn't be read: {e:?}");
                None
            }
        }
    })
    .flatten()
}

/// Keeps `data` (replacing what was kept).
pub fn save(data: &[u8]) -> Result<(), String> {
    with(|k, disk| {
        let mut store = FileStore { disk, file: &k.file };
        k.memory.save(&mut store, data).map_err(|e| format!("{e:?}"))
    })
    .unwrap_or_else(|| Err(String::from("there's no memory file")))
}

/// Before the PC restarts: the drive writes what it has cached.
pub fn shut_down() {
    storage::shut_down();
}

/// For the computer's status.
pub fn status() -> String {
    if let Some(k) = KEPT.lock().as_ref() {
        let drive = match storage::start_said() {
            Some(Ok(d)) => d,
            _ => String::from("start-up drive"),
        };
        return format!(
            "The conversation is kept across restarts, in Chat Computer's memory file on the start-up partition \
             ({} KB), on the {drive}",
            k.memory.capacity() / 1024,
        );
    }
    match WHY_NOT.lock().as_ref() {
        Some(why) => format!("The conversation isn't kept across restarts: {why}"),
        None => String::from("The conversation isn't kept across restarts"),
    }
}
