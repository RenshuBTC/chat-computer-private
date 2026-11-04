//! Time: the local APIC timer, ticking `HZ` times a second. Its speed differs from PC to PC, so it is
//! measured against a clock of known speed: the ACPI power-management timer, else the HPET, else the
//! old PIT. A PC may lack any of them or have one switched off, so every wait gives up after a while,
//! and a measurement that makes no sense is thrown away.
//!
//! The processor's time-stamp counter is measured at the same time. It gives drivers short waits and
//! microsecond time that work even while interrupts are off, and when it runs at one speed whatever the processor
//! does (an "invariant" one, as processors of the last 15 years or so have), it counts the time since start-up too:
//! ticks are lost while interrupts are off (as they are while a lock is held), and the time-stamp counter loses none.

use crate::acpi::{Acpi, PmTimer};
use crate::apic;
use crate::cpu::{cpuid, inb, inl, outb, rdtsc};
use crate::idt::TIMER_VECTOR;
use crate::paging::{self, Cache};
use core::ops::RangeInclusive;
use core::sync::atomic::{AtomicU64, Ordering};

pub const HZ: u64 = 100;
const PM_TIMER_HZ: u64 = 3_579_545;
const PIT_HZ: u64 = 1_193_182;
const MEASURE_MS: u64 = 50;
/// The most times one wait reads its clock: several times what 50 ms of a working clock takes, and
/// a few seconds (a read takes about a microsecond) if the clock never moves.
const MAX_READS: u32 = 5_000_000;
/// APIC timer speeds (counts a second, after ÷16) that make sense. Below this a tick would be under
/// 1,000 counts; above it the timer would run at more than 8 GHz.
const PLAUSIBLE: RangeInclusive<u64> = 100_000..=500_000_000;
/// What the timer is assumed to count when nothing could measure it: a 100 MHz clock, ÷16. This is a
/// guess, and the log says so.
const ASSUMED: u64 = 6_250_000;

/// Time-stamp counts a microsecond that make sense: 100 MHz to 10 GHz.
const PLAUSIBLE_TSC: RangeInclusive<u64> = 100..=10_000;
/// What a time-stamp counter is assumed to count when nothing could measure it (and the log says so).
const ASSUMED_TSC_PER_US: u64 = 2_000;

static TICKS: AtomicU64 = AtomicU64::new(0);
static TSC_PER_US: AtomicU64 = AtomicU64::new(0);
/// The time-stamp counter's counts a millisecond as measured (0: not measured).
static MEASURED_TSC_PER_MS: AtomicU64 = AtomicU64::new(0);
/// When the time-stamp counter counts the uptime: its counts a millisecond, and its count when the timer started
/// (0: the uptime is counted in ticks).
static UPTIME_TSC_PER_MS: AtomicU64 = AtomicU64::new(0);
static UPTIME_TSC_START: AtomicU64 = AtomicU64::new(0);

pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

/// Time since the timer started, in milliseconds.
pub fn uptime_ms() -> u64 {
    match UPTIME_TSC_PER_MS.load(Ordering::Relaxed) {
        0 => TICKS.load(Ordering::Relaxed) * 1000 / HZ,
        per_ms => rdtsc().wrapping_sub(UPTIME_TSC_START.load(Ordering::Relaxed)) / per_ms,
    }
}

/// Whether the time-stamp counter runs at one speed in every power state (CPUID 0x80000007, EDX bit 8).
fn invariant_tsc() -> bool {
    cpuid(0x8000_0000, 0).0 >= 0x8000_0007 && cpuid(0x8000_0007, 0).3 & (1 << 8) != 0
}

/// What counts the time since start-up.
pub fn uptime_by() -> &'static str {
    if UPTIME_TSC_PER_MS.load(Ordering::Relaxed) != 0 { "the time-stamp counter" } else { "the timer's ticks" }
}

fn tsc_per_us() -> u64 {
    match TSC_PER_US.load(Ordering::Relaxed) {
        0 => ASSUMED_TSC_PER_US,
        n => n,
    }
}

/// Microseconds since the processor started counting (some time before Chat Computer did). Works with
/// interrupts off.
pub fn now_us() -> u64 {
    rdtsc() / tsc_per_us()
}

/// Waits `us` microseconds without sleeping (for drivers' short waits; works with interrupts off).
pub fn delay_us(us: u64) {
    let end = rdtsc().saturating_add(us.saturating_mul(tsc_per_us()));
    while rdtsc() < end {
        core::hint::spin_loop();
    }
}

pub fn delay_ms(ms: u64) {
    delay_us(ms * 1000);
}

/// Waits `ms` milliseconds by the ACPI power-management timer. False if it stopped moving.
fn wait_pm(timer: PmTimer, ms: u64) -> bool {
    let mask: u64 = if timer.wide { 0xFFFF_FFFF } else { 0xFF_FFFF };
    let target = PM_TIMER_HZ * ms / 1000;
    let mut last = unsafe { inl(timer.port) } as u64 & mask;
    let mut passed = 0;
    for _ in 0..MAX_READS {
        let now = unsafe { inl(timer.port) } as u64 & mask;
        passed += now.wrapping_sub(last) & mask;
        last = now;
        if passed >= target {
            return true;
        }
    }
    false
}

/// Waits `ms` milliseconds by the HPET, whose registers are at `phys`. False if it isn't usable.
fn wait_hpet(phys: u64, ms: u64) -> bool {
    const PERIOD: u64 = 0x04; // the capabilities' upper half: femtoseconds a count
    const CONFIG: u64 = 0x10; // bit 0: the counter runs
    const COUNTER: u64 = 0xF0; // its lower 32 bits
    let regs = paging::map_device(phys, 0x100, Cache::Uncached);
    let read = |reg: u64| unsafe { core::ptr::read_volatile((regs + reg) as *const u32) };
    let write = |reg: u64, v: u32| unsafe { core::ptr::write_volatile((regs + reg) as *mut u32, v) };
    let period_fs = read(PERIOD) as u64;
    if period_fs == 0 || period_fs > 100_000_000 {
        return false; // the HPET specification allows at most 100 ns a count
    }
    let config = read(CONFIG);
    if config & 1 == 0 {
        write(CONFIG, config | 1);
    }
    let target = ms * 1_000_000_000_000 / period_fs;
    let start = read(COUNTER);
    let mut ok = false;
    for _ in 0..MAX_READS {
        if read(COUNTER).wrapping_sub(start) as u64 >= target {
            ok = true;
            break;
        }
    }
    if config & 1 == 0 {
        write(CONFIG, config); // as it was found
    }
    ok
}

/// Waits `ms` milliseconds by the PIT's channel 2. False if it never finished counting.
fn wait_pit(ms: u64) -> bool {
    let count = (PIT_HZ * ms / 1000).min(0xFFFF) as u16;
    unsafe {
        let gate = inb(0x61) & 0xFC; // speaker off
        outb(0x61, gate | 1); // channel 2 gate on
        outb(0x43, 0xB0); // channel 2, both bytes, count down once
        outb(0x42, count as u8);
        outb(0x42, (count >> 8) as u8);
        outb(0x61, gate); // restart the count
        outb(0x61, gate | 1);
        for _ in 0..MAX_READS {
            if inb(0x61) & 0x20 != 0 {
                return true;
            }
        }
    }
    false
}

/// The APIC timer's counts a second while `wait` waits `MEASURE_MS`, if `wait` worked and the answer
/// makes sense. The time-stamp counter is measured too, and kept if it makes sense.
fn measure(wait: impl FnOnce() -> bool) -> Option<u64> {
    apic::write(apic::TIMER_INITIAL, u32::MAX); // (re)starts the count down
    let tsc = rdtsc();
    let finished = wait();
    let tsc = rdtsc().wrapping_sub(tsc);
    let left = apic::read(apic::TIMER_CURRENT);
    apic::write(apic::TIMER_INITIAL, 0); // stops it
    let per_second = (u32::MAX - left) as u64 * 1000 / MEASURE_MS;
    let tsc_per_us = tsc / (MEASURE_MS * 1000);
    if finished && PLAUSIBLE_TSC.contains(&tsc_per_us) {
        TSC_PER_US.store(tsc_per_us, Ordering::Relaxed);
        MEASURED_TSC_PER_MS.store(tsc / MEASURE_MS, Ordering::Relaxed);
    }
    (finished && PLAUSIBLE.contains(&per_second)).then_some(per_second)
}

/// The time-stamp counter's speed in MHz, and whether it was measured (else it is assumed).
pub fn tsc_mhz() -> (u64, bool) {
    let n = TSC_PER_US.load(Ordering::Relaxed);
    (tsc_per_us(), n != 0)
}

/// The timer's speed when nothing could measure it. Intel processors that state their crystal clock
/// (CPUID leaf 0x15) feed the APIC timer from it, as Linux assumes too; otherwise it's a guess.
fn unmeasured() -> (u64, &'static str) {
    if cpuid(0, 0).0 >= 0x15 {
        let crystal_hz = cpuid(0x15, 0).2 as u64;
        if PLAUSIBLE.contains(&(crystal_hz / 16)) {
            return (crystal_hz / 16, "taken from the processor's crystal clock (no clock to measure it against)");
        }
    }
    (ASSUMED, "assumed (no clock to measure it against)")
}

/// Measures the APIC timer and starts it. Returns its speed in counts a second (after ÷16), and how
/// that speed was found.
pub fn init(acpi: &Acpi) -> (u64, &'static str) {
    apic::write(apic::LVT_TIMER, apic::MASKED);
    apic::write(apic::TIMER_DIVIDE, 0x3); // ÷16
    let mut found = None;
    if let Some(t) = acpi.pm_timer {
        found = measure(|| wait_pm(t, MEASURE_MS)).map(|s| (s, "measured against the ACPI power-management timer"));
    }
    if found.is_none()
        && let Some(hpet) = acpi.hpet
    {
        found = measure(|| wait_hpet(hpet, MEASURE_MS)).map(|s| (s, "measured against the HPET"));
    }
    if found.is_none() {
        found = measure(|| wait_pit(MEASURE_MS)).map(|s| (s, "measured against the PIT"));
    }
    let (per_second, how) = found.unwrap_or_else(unmeasured);
    apic::write(apic::LVT_TIMER, TIMER_VECTOR as u32 | 1 << 17); // periodic
    apic::write(apic::TIMER_INITIAL, (per_second / HZ) as u32);
    let tsc_per_ms = MEASURED_TSC_PER_MS.load(Ordering::Relaxed);
    if tsc_per_ms != 0 && invariant_tsc() {
        UPTIME_TSC_START.store(rdtsc(), Ordering::Relaxed);
        UPTIME_TSC_PER_MS.store(tsc_per_ms, Ordering::Relaxed);
    }
    (per_second, how)
}
