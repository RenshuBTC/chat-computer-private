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

/// Use no more than `l` (for tests of the plainer arithmetic, and to compare). A level the processor hasn't
/// (AVX-VNNI where it has only AVX-512 VNNI) is the next one down that it has.
pub fn at_most(l: Level) {
    let mut want = level().min(l);
    while !supports(want) {
        want = match want {
            Level::Avx512Vnni => Level::AvxVnni,
            Level::AvxVnni => Level::Avx2,
            _ => Level::Plain,
        };
    }
    LEVEL.store(want as u8, Ordering::Relaxed);
}

/// Never use AVX2 (for tests of the plain arithmetic).
pub fn without_avx2() {
    at_most(Level::Plain);
}

/// Whether the processor (and the system) can run a level's instructions, each level on its own: a processor with
/// AVX-512 VNNI may not have AVX-VNNI (Intel's servers before Sapphire Rapids), nor the other way round (Alder
/// Lake). For tests that try every level the processor has.
pub fn supports(l: Level) -> bool {
    let (avx2, avx_vnni, avx512_vnni) = features();
    match l {
        Level::Plain => true,
        Level::Avx2 => avx2,
        Level::AvxVnni => avx2 && avx_vnni,
        Level::Avx512Vnni => avx2 && avx512_vnni,
    }
}

fn check() -> Level {
    match features() {
        (false, _, _) => Level::Plain,
        (true, true, _) => Level::AvxVnni,
        (true, false, true) => Level::Avx512Vnni,
        (true, false, false) => Level::Avx2,
    }
}

/// AVX2 (with FMA and F16C), AVX-VNNI, and AVX-512 VNNI (with VL) the processor has and the system saves the
/// registers of.
#[cfg(target_arch = "x86_64")]
fn features() -> (bool, bool, bool) {
    use core::arch::x86_64::{__cpuid, __cpuid_count};
    unsafe {
        let leaf1 = __cpuid(1);
        let fma = leaf1.ecx & (1 << 12) != 0;
        let osxsave = leaf1.ecx & (1 << 27) != 0;
        let avx = leaf1.ecx & (1 << 28) != 0;
        let f16c = leaf1.ecx & (1 << 29) != 0;
        if !(fma && osxsave && avx && f16c) {
            return (false, false, false);
        }
        // The system saves the SSE and AVX registers (XCR0 bits 1 and 2).
        let xcr0 = xcr0();
        if xcr0 & 6 != 6 || __cpuid(0).eax < 7 {
            return (false, false, false);
        }
        let leaf7 = __cpuid_count(7, 0);
        if leaf7.ebx & (1 << 5) == 0 {
            return (false, false, false);
        }
        // AVX-VNNI: leaf 7, sub-leaf 1, EAX bit 4.
        let avx_vnni = leaf7.eax >= 1 && __cpuid_count(7, 1).eax & (1 << 4) != 0;
        // AVX-512 VNNI (ECX bit 11) with AVX-512 F (EBX 16) and VL (EBX 31), and the system saving the AVX-512
        // registers (XCR0 bits 5 to 7).
        let avx512 = leaf7.ebx & (1 << 16) != 0 && leaf7.ebx & (1 << 31) != 0 && leaf7.ecx & (1 << 11) != 0;
        (true, avx_vnni, avx512 && xcr0 & 0xE6 == 0xE6)
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "xsave")]
unsafe fn xcr0() -> u64 {
    unsafe { core::arch::x86_64::_xgetbv(0) }
}

#[cfg(not(target_arch = "x86_64"))]
fn features() -> (bool, bool, bool) {
    (false, false, false)
}
