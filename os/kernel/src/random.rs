//! Hard-to-guess numbers, from the processor: RDSEED or RDRAND where it has them (every PC processor since
//! about 2013), mixed with the time-stamp counter's jitter. Used directly for network port numbers and the first
//! numbers of connections; encryption keys come from a generator seeded here (`fill`).

use crate::cpu;
use crate::sync::SpinLock;
use core::arch::asm;
use crypto::drbg::Drbg;

/// The generator for keys, and how many times it has been used since it was last given fresh bytes.
static KEYS: SpinLock<Option<(Drbg, u32)>> = SpinLock::new(None);

/// 512 bits from the processor.
fn seed() -> [u8; 64] {
    let mut s = [0u8; 64];
    for chunk in s.chunks_mut(8) {
        chunk.copy_from_slice(&u64().to_le_bytes());
    }
    s
}

/// Fills `out` with bytes for keys: from a ChaCha20 generator (crypto::drbg, whose own key changes after every
/// use) seeded with 512 bits from the processor, and given 512 more every 16 uses.
pub fn fill(out: &mut [u8]) {
    let mut guard = KEYS.lock();
    let (drbg, uses) = guard.get_or_insert_with(|| (Drbg::new(&seed()), 0));
    if *uses >= 16 {
        drbg.reseed(&seed());
        *uses = 0;
    }
    *uses += 1;
    drbg.fill(out);
}

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
