//! Hard-to-guess numbers, from the processor: RDSEED or RDRAND where it has them (every PC processor since
//! about 2013), mixed with the time-stamp counter's jitter. Used for network port numbers and the first numbers of
//! connections, and for the model's choices among its likeliest words.

use crate::cpu;
use core::arch::asm;

fn has_rdrand() -> bool {
    cpu::cpuid(1, 0).2 & (1 << 30) != 0
}

fn has_rdseed() -> bool {
    cpu::cpuid(0, 0).0 >= 7 && cpu::cpuid(7, 0).1 & (1 << 18) != 0
}

fn rdrand() -> Option<u64> {
    for _ in 0..10 {
        let (v, ok): (u64, u8);
        unsafe { asm!("rdrand {}", "setc {}", out(reg) v, out(reg_byte) ok, options(nomem, nostack)) };
        if ok != 0 {
            return Some(v);
        }
    }
    None
}

fn rdseed() -> Option<u64> {
    for _ in 0..100 {
        let (v, ok): (u64, u8);
        unsafe { asm!("rdseed {}", "setc {}", out(reg) v, out(reg_byte) ok, options(nomem, nostack)) };
        if ok != 0 {
            return Some(v);
        }
        core::hint::spin_loop();
    }
    None
}

/// How the numbers are made here, for the log.
pub fn source() -> &'static str {
    if has_rdseed() {
        "the processor's RDSEED"
    } else if has_rdrand() {
        "the processor's RDRAND"
    } else {
        "timing jitter only (this processor has no random number instruction)"
    }
}

/// `buf` filled with hard-to-guess bytes (TLS's keys and nonces).
pub fn fill(buf: &mut [u8]) {
    for chunk in buf.chunks_mut(8) {
        let v = u64().to_le_bytes();
        chunk.copy_from_slice(&v[..chunk.len()]);
    }
}

/// 64 hard-to-guess bits.
pub fn u64() -> u64 {
    let mut x = jitter();
    if has_rdseed()
        && let Some(v) = rdseed()
    {
        x ^= v;
    } else if has_rdrand()
        && let Some(v) = rdrand()
    {
        x ^= v;
    }
    x
}

/// The time-stamp counter's low bits over many short, uneven waits, folded together.
fn jitter() -> u64 {
    let mut x: u64 = cpu::rdtsc();
    for i in 0..64u64 {
        let t0 = cpu::rdtsc();
        for _ in 0..(i % 7) * 3 {
            core::hint::spin_loop();
        }
        let d = cpu::rdtsc().wrapping_sub(t0);
        x = (x.rotate_left(7) ^ d).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    x
}
