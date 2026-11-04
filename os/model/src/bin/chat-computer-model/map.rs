//! The model's file, mapped into memory rather than read: the system reads each part when it's first used and can
//! share it with its file cache, so a model as big as most of the memory still starts at once.

use std::fs::File;

/// The file's bytes, for as long as the program runs.
pub fn map(path: &str) -> Result<&'static [u8], String> {
    let f = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let len = f.metadata().map_err(|e| e.to_string())?.len() as usize;
    if len == 0 {
        return Err(format!("{path} is empty"));
    }
    let p = sys::map(&f, len)?;
    Ok(unsafe { std::slice::from_raw_parts(p, len) })
}

#[cfg(unix)]
mod sys {
    use core::ffi::c_void;
    use std::fs::File;
    use std::os::fd::AsRawFd;

    unsafe extern "C" {
        fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut c_void;
    }
    const PROT_READ: i32 = 1;
    const MAP_PRIVATE: i32 = 2;

    pub fn map(f: &File, len: usize) -> Result<*const u8, String> {
        let p = unsafe { mmap(core::ptr::null_mut(), len, PROT_READ, MAP_PRIVATE, f.as_raw_fd(), 0) };
        if p as isize == -1 || p.is_null() {
            return Err(format!("couldn't map the model's file: {}", std::io::Error::last_os_error()));
        }
        // (The mapping stays after the file is closed.)
        Ok(p as *const u8)
    }
}

#[cfg(windows)]
mod sys {
    use core::ffi::c_void;
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;

    unsafe extern "system" {
        fn CreateFileMappingW(file: *mut c_void, attributes: *mut c_void, protect: u32, high: u32, low: u32, name: *const u16) -> *mut c_void;
        fn MapViewOfFile(mapping: *mut c_void, access: u32, high: u32, low: u32, bytes: usize) -> *mut c_void;
    }
    const PAGE_READONLY: u32 = 0x02;
    const FILE_MAP_READ: u32 = 0x04;

    pub fn map(f: &File, len: usize) -> Result<*const u8, String> {
        let m = unsafe { CreateFileMappingW(f.as_raw_handle() as *mut c_void, core::ptr::null_mut(), PAGE_READONLY, 0, 0, core::ptr::null()) };
        if m.is_null() {
            return Err(format!("couldn't map the model's file: {}", std::io::Error::last_os_error()));
        }
        let p = unsafe { MapViewOfFile(m, FILE_MAP_READ, 0, 0, len) };
        if p.is_null() {
            return Err(format!("couldn't map the model's file: {}", std::io::Error::last_os_error()));
        }
        // (The view keeps the mapping, and the mapping the file, for as long as the program runs.)
        Ok(p as *const u8)
    }
}
