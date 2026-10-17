//! Time: the local APIC timer, measured against a clock of known speed (the ACPI power-management
//! timer, or the old PIT when there is none), ticking `HZ` times a second.

use crate::acpi::{Acpi, PmTimer};
use crate::apic;
use crate::cpu::{inb, inl, outb};
use crate::idt::TIMER_VECTOR;
use core::sync::atomic::{AtomicU64, Ordering};

pub const HZ: u64 = 100;
const PM_TIMER_HZ: u64 = 3_579_545;
const PIT_HZ: u64 = 1_193_182;
const MEASURE_MS: u64 = 50;

static TICKS: AtomicU64 = AtomicU64::new(0);

pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

/// Time since the timer started, in milliseconds.
pub fn uptime_ms() -> u64 {
    TICKS.load(Ordering::Relaxed) * 1000 / HZ
}

fn wait_pm(timer: PmTimer, ms: u64) {
    let mask: u64 = if timer.wide { 0xFFFF_FFFF } else { 0xFF_FFFF };
    let target = PM_TIMER_HZ * ms / 1000;
    let mut last = unsafe { inl(timer.port) } as u64 & mask;
    let mut passed = 0;
    while passed < target {
        let now = unsafe { inl(timer.port) } as u64 & mask;
        passed += now.wrapping_sub(last) & mask;
        last = now;
    }
}

fn wait_pit(ms: u64) {
    let count = (PIT_HZ * ms / 1000).min(0xFFFF) as u16;
    unsafe {
        let gate = inb(0x61) & 0xFC; // speaker off
        outb(0x61, gate | 1); // channel 2 gate on
        outb(0x43, 0xB0); // channel 2, both bytes, count down once
        outb(0x42, count as u8);
        outb(0x42, (count >> 8) as u8);
        outb(0x61, gate); // restart the count
        outb(0x61, gate | 1);
        while inb(0x61) & 0x20 == 0 {}
    }
}

/// Measures the APIC timer and starts it. Returns its speed in counts per second (after ÷16).
pub fn init(acpi: &Acpi) -> (u64, &'static str) {
    apic::write(apic::TIMER_DIVIDE, 0x3); // ÷16
    apic::write(apic::LVT_TIMER, apic::MASKED);
    apic::write(apic::TIMER_INITIAL, u32::MAX);
    let source = match acpi.pm_timer {
        Some(t) => {
            wait_pm(t, MEASURE_MS);
            "the ACPI power-management timer"
        }
        None => {
            wait_pit(MEASURE_MS);
            "the PIT"
        }
    };
    let counted = (u32::MAX - apic::read(apic::TIMER_CURRENT)) as u64;
    apic::write(apic::TIMER_INITIAL, 0);
    let per_second = counted * 1000 / MEASURE_MS;
    apic::write(apic::LVT_TIMER, TIMER_VECTOR as u32 | 1 << 17); // periodic
    apic::write(apic::TIMER_DIVIDE, 0x3);
    apic::write(apic::TIMER_INITIAL, (per_second / HZ).max(1) as u32);
    (per_second, source)
}
