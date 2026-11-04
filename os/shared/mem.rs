// Chat Computer's own memcpy, memmove, memset, memcmp and bcmp, which the compiler calls for copying and
// comparing memory. Used by both the start-up stage and the kernel (each includes this file).
//
// They use the processor's string instructions, so the compiler can't turn them back into calls to
// themselves. The direction flag is clear on entry, as the calling conventions require.

use core::arch::asm;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    unsafe {
        asm!("rep movsb", inout("rcx") n => _, inout("rdi") dest => _, inout("rsi") src => _,
             options(nostack, preserves_flags));
    }
    dest
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if n == 0 {
        return dest;
    }
    if (dest as usize).wrapping_sub(src as usize) >= n {
        // `dest` is below `src`, or past its end: copying forwards never overwrites unread bytes.
        unsafe { memcpy(dest, src, n) };
    } else {
        unsafe {
            asm!("std", "rep movsb", "cld", inout("rcx") n => _, inout("rdi") dest.add(n - 1) => _,
                 inout("rsi") src.add(n - 1) => _, options(nostack));
        }
    }
    dest
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dest: *mut u8, value: i32, n: usize) -> *mut u8 {
    unsafe {
        asm!("rep stosb", inout("rcx") n => _, inout("rdi") dest => _, in("al") value as u8,
             options(nostack, preserves_flags));
    }
    dest
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        // Volatile reads keep the compiler from replacing this loop with a call to memcmp.
        let (x, y) = unsafe { (a.add(i).read_volatile(), b.add(i).read_volatile()) };
        if x != y {
            return x as i32 - y as i32;
        }
        i += 1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    unsafe { memcmp(a, b, n) }
}
