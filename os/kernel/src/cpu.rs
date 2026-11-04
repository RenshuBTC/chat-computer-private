//! Direct use of the processor: I/O ports, model-specific registers, control registers, CPUID,
//! interrupts on and off, halting.
#![allow(dead_code)]

use core::arch::asm;

pub unsafe fn outb(port: u16, v: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack, preserves_flags)) }
}
pub unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    unsafe { asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack, preserves_flags)) };
    v
}
pub unsafe fn outw(port: u16, v: u16) {
    unsafe { asm!("out dx, ax", in("dx") port, in("ax") v, options(nomem, nostack, preserves_flags)) }
}
pub unsafe fn inw(port: u16) -> u16 {
    let v: u16;
    unsafe { asm!("in ax, dx", out("ax") v, in("dx") port, options(nomem, nostack, preserves_flags)) };
    v
}
pub unsafe fn outl(port: u16, v: u32) {
    unsafe { asm!("out dx, eax", in("dx") port, in("eax") v, options(nomem, nostack, preserves_flags)) }
}
pub unsafe fn inl(port: u16) -> u32 {
    let v: u32;
    unsafe { asm!("in eax, dx", out("eax") v, in("dx") port, options(nomem, nostack, preserves_flags)) };
    v
}

pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    unsafe { asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) };
    (hi as u64) << 32 | lo as u64
}
pub unsafe fn wrmsr(msr: u32, v: u64) {
    unsafe {
        asm!("wrmsr", in("ecx") msr, in("eax") v as u32, in("edx") (v >> 32) as u32, options(nostack, preserves_flags))
    }
}

pub fn cr2() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr2", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}
pub fn cr3() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}
pub unsafe fn set_cr3(v: u64) {
    unsafe { asm!("mov cr3, {}", in(reg) v, options(nostack, preserves_flags)) }
}
pub fn cr0() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr0", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}
pub fn cr4() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr4", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}
pub unsafe fn set_cr4(v: u64) {
    unsafe { asm!("mov cr4, {}", in(reg) v, options(nostack, preserves_flags)) }
}

/// The processor's time-stamp counter: counts at a steady rate on any processor from the last 15 years.
pub fn rdtsc() -> u64 {
    let (lo, hi): (u32, u32);
    unsafe { asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) };
    (hi as u64) << 32 | lo as u64
}

pub fn invlpg(addr: u64) {
    unsafe { asm!("invlpg [{}]", in(reg) addr, options(nostack, preserves_flags)) }
}
pub fn wbinvd() {
    unsafe { asm!("wbinvd", options(nostack, preserves_flags)) }
}
/// Writes the cache line holding `addr` back to memory and drops it from the caches: for a device that may read
/// memory, or write it, without the caches knowing (a sound controller set not to snoop).
pub fn flush_line(addr: u64) {
    unsafe { asm!("clflush [{}]", in(reg) addr, options(nostack, preserves_flags)) }
}
/// Every load and store before it done before any after it (clflush included).
pub fn memory_fence() {
    unsafe { asm!("mfence", options(nostack, preserves_flags)) }
}

/// CPUID leaf `leaf`, sub-leaf `sub`: (eax, ebx, ecx, edx).
pub fn cpuid(leaf: u32, sub: u32) -> (u32, u32, u32, u32) {
    let (a, b, c, d): (u32, u64, u32, u32);
    unsafe {
        asm!("mov {t}, rbx", "cpuid", "xchg {t}, rbx", t = out(reg) b, inout("eax") leaf => a,
             inout("ecx") sub => c, out("edx") d, options(nomem, nostack, preserves_flags));
    }
    (a, b as u32, c, d)
}

pub fn has_no_execute() -> bool {
    cpuid(0x8000_0000, 0).0 >= 0x8000_0001 && cpuid(0x8000_0001, 0).3 & (1 << 20) != 0
}
pub fn has_gib_pages() -> bool {
    cpuid(0x8000_0000, 0).0 >= 0x8000_0001 && cpuid(0x8000_0001, 0).3 & (1 << 26) != 0
}

/// The processor's own name, e.g. "AMD Ryzen 7 7800X3D 8-Core Processor" (48 bytes, space-padded).
pub fn brand() -> [u8; 48] {
    let mut out = [b' '; 48];
    if cpuid(0x8000_0000, 0).0 < 0x8000_0004 {
        return out;
    }
    for (i, leaf) in (0x8000_0002u32..=0x8000_0004).enumerate() {
        let (a, b, c, d) = cpuid(leaf, 0);
        for (j, r) in [a, b, c, d].iter().enumerate() {
            out[i * 16 + j * 4..i * 16 + j * 4 + 4].copy_from_slice(&r.to_le_bytes());
        }
    }
    for b in out.iter_mut() {
        if *b == 0 {
            *b = b' ';
        }
    }
    out
}

/// The processor's wider vector registers (AVX's, and AVX-512's where it has them), switched on, for the model's
/// arithmetic (the firmware leaves SSE's on): the system says it saves them (CR4.OSXSAVE), and which (XCR0: x87, SSE
/// and AVX's; AVX-512's masks and upper registers too, so the engine can use AVX-512's VNNI, as on AMD's Zen 4, which
/// has no AVX-VNNI). Nothing saves them but SSE's: an interrupt's handler, built for SSE2 alone, never touches them,
/// and nothing else runs between the model's steps. Whether AVX is on, and whether AVX-512's registers are.
pub fn enable_avx() -> (bool, bool) {
    let (_, _, ecx, _) = cpuid(1, 0);
    let (xsave, avx) = (ecx & (1 << 26) != 0, ecx & (1 << 28) != 0);
    if !(xsave && avx) || cpuid(0, 0).0 < 0xD {
        return (false, false);
    }
    unsafe { set_cr4(cr4() | (1 << 18)) };
    let supported = cpuid(0xD, 0).0 as u64;
    if supported & 0b111 != 0b111 {
        return (false, false);
    }
    // AVX-512's opmask registers, its registers' upper halves, and its 16 more (XCR0 bits 5 to 7): all or none.
    const AVX512: u64 = 0b1110_0000;
    let avx512 = supported & AVX512 == AVX512;
    let xcr0 = 0b111 | if avx512 { AVX512 } else { 0 };
    unsafe { asm!("xsetbv", in("ecx") 0u32, in("eax") xcr0 as u32, in("edx") 0u32, options(nomem, nostack, preserves_flags)) };
    (true, avx512)
}

pub fn interrupts_enabled() -> bool {
    let flags: u64;
    unsafe { asm!("pushfq", "pop {}", out(reg) flags, options(nomem, preserves_flags)) };
    flags & (1 << 9) != 0
}
// Not `nomem`: interrupt handlers read and write memory, so the compiler must not move memory accesses
// across turning interrupts on or off, or across waiting for one.
pub fn enable_interrupts() {
    unsafe { asm!("sti", options(nostack)) }
}
pub fn disable_interrupts() {
    unsafe { asm!("cli", options(nostack)) }
}

/// Sleeps until the next interrupt.
pub fn wait_for_interrupt() {
    unsafe { asm!("hlt", options(nostack, preserves_flags)) }
}

pub fn stop() -> ! {
    loop {
        unsafe { asm!("cli", "hlt", options(nomem, nostack)) };
    }
}
