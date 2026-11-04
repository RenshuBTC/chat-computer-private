//! What the processor can do, for the arithmetic to use the fastest instructions there are and the system has
//! switched on (the processor's XCR0 says which registers the system saves):
//!
//! - AVX2 with FMA and F16C (most PCs since 2013): 32 products of bytes at a time;
//! - and AVX-VNNI (Intel since 2021's Alder Lake) or AVX-512 VNNI (Intel's servers and Ice Lake, AMD since Zen 4):
//!   the same in one instruction instead of two, and with no need to fold signs.

use core::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Plain = 1,
    Avx2 = 2,
    /// AVX2, and VNNI's byte dot products in 256-bit registers (as AVX-VNNI encodes them).
    AvxVnni = 3,
    /// AVX2, and VNNI's byte dot products in 256-bit registers (as AVX-512 VNNI encodes them).
    Avx512Vnni = 4,
}

/// 0: not checked yet; else the level's number.
static LEVEL: AtomicU8 = AtomicU8::new(0);

#[inline]
pub fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        1 => Level::Plain,
        2 => Level::Avx2,
        3 => Level::AvxVnni,
        4 => Level::Avx512Vnni,
        _ => {
            let l = check();
            LEVEL.store(l as u8, Ordering::Relaxed);
            l
        }
    }
}

#[inline]
pub fn avx2() -> bool {
    level() >= Level::Avx2
}

#[inline]
pub fn vnni() -> bool {
    level() >= Level::AvxVnni
}

/// Use no more than `l` (for tests of the plainer arithmetic, and to compare).
pub fn at_most(l: Level) {
    let now = level();
    LEVEL.store(now.min(l) as u8, Ordering::Relaxed);
}

/// Never use AVX2 (for tests of the plain arithmetic).
pub fn without_avx2() {
    at_most(Level::Plain);
}

#[cfg(target_arch = "x86_64")]
fn check() -> Level {
    use core::arch::x86_64::{__cpuid, __cpuid_count};
    unsafe {
        let leaf1 = __cpuid(1);
        let fma = leaf1.ecx & (1 << 12) != 0;
        let osxsave = leaf1.ecx & (1 << 27) != 0;
        let avx = leaf1.ecx & (1 << 28) != 0;
        let f16c = leaf1.ecx & (1 << 29) != 0;
        if !(fma && osxsave && avx && f16c) {
            return Level::Plain;
        }
        // The system saves the SSE and AVX registers (XCR0 bits 1 and 2).
        let xcr0 = xcr0();
        if xcr0 & 6 != 6 || __cpuid(0).eax < 7 {
            return Level::Plain;
        }
        let leaf7 = __cpuid_count(7, 0);
        if leaf7.ebx & (1 << 5) == 0 {
            return Level::Plain;
        }
        // AVX-VNNI: leaf 7, sub-leaf 1, EAX bit 4.
        if leaf7.eax >= 1 && __cpuid_count(7, 1).eax & (1 << 4) != 0 {
            return Level::AvxVnni;
        }
        // AVX-512 VNNI (ECX bit 11) with AVX-512 F (EBX 16) and VL (EBX 31), and the system saving the AVX-512
        // registers (XCR0 bits 5 to 7).
        let avx512 = leaf7.ebx & (1 << 16) != 0 && leaf7.ebx & (1 << 31) != 0 && leaf7.ecx & (1 << 11) != 0;
        if avx512 && xcr0 & 0xE6 == 0xE6 {
            return Level::Avx512Vnni;
        }
        Level::Avx2
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "xsave")]
unsafe fn xcr0() -> u64 {
    unsafe { core::arch::x86_64::_xgetbv(0) }
}

#[cfg(not(target_arch = "x86_64"))]
fn check() -> Level {
    Level::Plain
}
