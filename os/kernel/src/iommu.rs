//! IOMMUs: the parts between devices and memory that can limit which memory each device reaches
//! (Intel's VT-d, AMD's AMD-Vi). Chat Computer doesn't use them yet. Some firmware leaves one switched on
//! with its own settings, to keep devices away from memory while it starts the PC ("pre-boot DMA
//! protection", common on laptops with Thunderbolt). Then devices couldn't reach the memory Chat Computer
//! gives them, and an Intel unit left remapping interrupts would drop their interrupt messages. So
//! whatever the firmware left on is switched off here, before any driver starts.

use crate::acpi::Acpi;
use crate::paging::{self, Cache};
use crate::time;
use core::ptr::{read_volatile, write_volatile};

// Intel VT-d registers.
const GCMD: u64 = 0x18;
const GSTS: u64 = 0x1C;
const PMEN: u64 = 0x64;
const TRANSLATION: u32 = 1 << 31;
const INTERRUPT_REMAPPING: u32 = 1 << 25;
/// The command bits that take effect once when written (root table pointer, fault log, write-buffer
/// flush, interrupt table pointer): written as 0 so they do nothing.
const ONE_SHOT: u32 = 1 << 30 | 1 << 29 | 1 << 27 | 1 << 24;
const PROTECTED_MEMORY: u32 = 1 << 31;
const PROTECTED_STATUS: u32 = 1 << 0;

// AMD-Vi's control register.
const AMD_CONTROL: u64 = 0x18;
const AMD_ENABLE: u32 = 1 << 0;

fn read32(at: u64) -> u32 {
    unsafe { read_volatile(at as *const u32) }
}

fn write32(at: u64, v: u32) {
    unsafe { write_volatile(at as *mut u32, v) }
}

fn wait_until(ms: u64, done: impl Fn() -> bool) -> bool {
    let end = time::now_us() + ms * 1000;
    while !done() {
        if time::now_us() > end {
            return false;
        }
        time::delay_us(10);
    }
    true
}

/// Switches off what the firmware left on. Returns how many IOMMUs there are and how many were on.
pub fn take_over(acpi: &Acpi) -> (usize, usize) {
    let mut on = 0;
    for &base in &acpi.intel_iommus[..acpi.intel_iommu_count] {
        let regs = paging::map_device(base, 0x1000, Cache::Uncached);
        let status = read32(regs + GSTS);
        let pmen = read32(regs + PMEN);
        let mut was_on = false;
        if status & (TRANSLATION | INTERRUPT_REMAPPING) != 0 {
            was_on = true;
            write32(regs + GCMD, status & !ONE_SHOT & !(TRANSLATION | INTERRUPT_REMAPPING));
            if !wait_until(100, || read32(regs + GSTS) & (TRANSLATION | INTERRUPT_REMAPPING) == 0) {
                crate::log!("Chat Computer kernel: IOMMU at {base:#x} didn't switch off");
            }
        }
        if pmen & PROTECTED_MEMORY != 0 {
            was_on = true;
            write32(regs + PMEN, 0);
            if !wait_until(100, || read32(regs + PMEN) & PROTECTED_STATUS == 0) {
                crate::log!("Chat Computer kernel: IOMMU at {base:#x} kept its protected memory");
            }
        }
        if was_on {
            crate::log!("Chat Computer kernel: Intel IOMMU at {base:#x}: the firmware left it on; switched off");
            on += 1;
        }
    }
    for &base in &acpi.amd_iommus[..acpi.amd_iommu_count] {
        let regs = paging::map_device(base, 0x4000, Cache::Uncached);
        let control = read32(regs + AMD_CONTROL);
        if control & AMD_ENABLE != 0 {
            write32(regs + AMD_CONTROL, control & !AMD_ENABLE);
            crate::log!("Chat Computer kernel: AMD IOMMU at {base:#x}: the firmware left it on; switched off");
            on += 1;
        }
    }
    (acpi.intel_iommu_count + acpi.amd_iommu_count, on)
}
