//! What the program asks of the computer's own system, each system its own way (no library in between): how much
//! memory it has, how much room is left on a disk, and hard-to-guess bytes (for TLS).

use std::path::Path;

/// The computer's memory, in bytes (0 if it can't be told).
pub fn memory_bytes() -> u64 {
    imp::memory_bytes()
}

/// The room left on the disk `dir` is on, for this program, in bytes.
pub fn free_bytes(dir: &Path) -> Option<u64> {
    imp::free_bytes(dir)
}

/// Hard-to-guess bytes, from the system's own source of them.
pub fn random(buf: &mut [u8]) {
    imp::random(buf)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::CString;
    use std::io::Read;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub fn memory_bytes() -> u64 {
        let info = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        info.lines()
            .find_map(|l| l.strip_prefix("MemTotal:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
            .map_or(0, |kb| kb * 1024)
    }

    // (struct statvfs, as glibc and musl have it on 64-bit Linux.)
    #[repr(C)]
    #[derive(Default)]
    struct StatVfs {
        f_bsize: u64,
        f_frsize: u64,
        f_blocks: u64,
        f_bfree: u64,
        f_bavail: u64,
        f_files: u64,
        f_ffree: u64,
        f_favail: u64,
        f_fsid: u64,
        f_flag: u64,
        f_namemax: u64,
        spare: [i32; 6],
    }

    unsafe extern "C" {
        fn statvfs(path: *const core::ffi::c_char, buf: *mut StatVfs) -> i32;
    }

    pub fn free_bytes(dir: &Path) -> Option<u64> {
        let c = CString::new(dir.as_os_str().as_bytes()).ok()?;
        let mut s = StatVfs::default();
        (unsafe { statvfs(c.as_ptr(), &mut s) } == 0).then(|| s.f_bavail * s.f_frsize)
    }

    pub fn random(buf: &mut [u8]) {
        std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf)).expect("the system's random bytes");
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::CString;
    use std::io::Read;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    unsafe extern "C" {
        fn sysctlbyname(name: *const core::ffi::c_char, out: *mut core::ffi::c_void, len: *mut usize,
                        new: *const core::ffi::c_void, new_len: usize) -> i32;
        fn statvfs(path: *const core::ffi::c_char, buf: *mut StatVfs) -> i32;
    }

    pub fn memory_bytes() -> u64 {
        let mut v = 0u64;
        let mut len = core::mem::size_of::<u64>();
        let ok = unsafe { sysctlbyname(c"hw.memsize".as_ptr(), (&mut v as *mut u64).cast(), &mut len, core::ptr::null(), 0) };
        if ok == 0 { v } else { 0 }
    }

    // (struct statvfs as macOS has it: its block counts are 32 bits.)
    #[repr(C)]
    #[derive(Default)]
    struct StatVfs {
        f_bsize: u64,
        f_frsize: u64,
        f_blocks: u32,
        f_bfree: u32,
        f_bavail: u32,
        f_files: u32,
        f_ffree: u32,
        f_favail: u32,
        f_fsid: u64,
        f_flag: u64,
        f_namemax: u64,
    }

    pub fn free_bytes(dir: &Path) -> Option<u64> {
        let c = CString::new(dir.as_os_str().as_bytes()).ok()?;
        let mut s = StatVfs::default();
        (unsafe { statvfs(c.as_ptr(), &mut s) } == 0).then(|| s.f_bavail as u64 * s.f_frsize)
    }

    pub fn random(buf: &mut [u8]) {
        std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf)).expect("the system's random bytes");
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
        fn GetDiskFreeSpaceExW(dir: *const u16, free_to_caller: *mut u64, total: *mut u64, total_free: *mut u64) -> i32;
    }

    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(algorithm: *mut core::ffi::c_void, buf: *mut u8, len: u32, flags: u32) -> i32;
    }

    pub fn memory_bytes() -> u64 {
        let mut s = MemoryStatusEx {
            length: core::mem::size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        if unsafe { GlobalMemoryStatusEx(&mut s) } != 0 { s.total_phys } else { 0 }
    }

    pub fn free_bytes(dir: &Path) -> Option<u64> {
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        let (mut free, mut total, mut all_free) = (0u64, 0u64, 0u64);
        (unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut all_free) } != 0).then_some(free)
    }

    pub fn random(buf: &mut [u8]) {
        const SYSTEM_PREFERRED: u32 = 2;
        for chunk in buf.chunks_mut(1 << 20) {
            let ok = unsafe { BCryptGenRandom(core::ptr::null_mut(), chunk.as_mut_ptr(), chunk.len() as u32, SYSTEM_PREFERRED) };
            assert!(ok >= 0, "the system's random bytes");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_computer_says_what_it_has() {
        assert!(memory_bytes() > 256 << 20);
        let free = free_bytes(Path::new(".")).unwrap();
        assert!(free > 0);
        let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
        random(&mut a);
        random(&mut b);
        assert_ne!(a, b);
    }
}
