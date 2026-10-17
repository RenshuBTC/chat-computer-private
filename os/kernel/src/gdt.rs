//! The kernel's own segment table (GDT) and task state segment (TSS). In 64-bit mode segments do
//! little, but the processor still needs them: a code segment, a data segment, and a TSS whose
//! separate stacks catch the faults that can't run on a broken stack.

use crate::sync::Static;
use core::arch::asm;
use core::mem::size_of;

pub const KERNEL_CODE: u16 = 0x08;
pub const KERNEL_DATA: u16 = 0x10;
const TSS_SELECTOR: u16 = 0x28;

/// Stack numbers in the TSS (1-based, as the interrupt table refers to them).
pub const DOUBLE_FAULT_STACK: u8 = 1;
pub const NMI_STACK: u8 = 2;
pub const MACHINE_CHECK_STACK: u8 = 3;

#[repr(C, packed(4))]
struct Tss {
    reserved0: u32,
    rsp: [u64; 3],
    reserved1: u64,
    ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    iomap_base: u16,
}

const _: () = assert!(size_of::<Tss>() == 104);

#[repr(C, align(16))]
struct Stack([u8; 16 * 1024]);

static TSS: Static<Tss> =
    Static::new(Tss { reserved0: 0, rsp: [0; 3], reserved1: 0, ist: [0; 7], reserved2: 0, reserved3: 0, iomap_base: 104 });
static STACKS: Static<[Stack; 3]> = Static::new([const { Stack([0; 16 * 1024]) }; 3]);
static GDT: Static<[u64; 7]> = Static::new([0; 7]);

#[repr(C, packed)]
struct Pointer {
    limit: u16,
    base: u64,
}

pub fn init() {
    unsafe {
        let stacks = STACKS.get() as u64;
        let mut ist = [0u64; 7];
        for i in 0..3 {
            ist[i] = stacks + (i as u64 + 1) * size_of::<Stack>() as u64; // stacks grow down
        }
        (*TSS.get()).ist = ist;

        let base = TSS.get() as u64;
        let limit = (size_of::<Tss>() - 1) as u64;
        let tss_low = (limit & 0xFFFF)
            | (base & 0xFF_FFFF) << 16
            | 0x89 << 40 // present, 64-bit TSS (available)
            | ((limit >> 16) & 0xF) << 48
            | ((base >> 24) & 0xFF) << 56;
        *GDT.get() = [
            0,
            0x00AF_9A00_0000_FFFF, // kernel code: 64-bit, ring 0
            0x00CF_9200_0000_FFFF, // kernel data
            0x00CF_F200_0000_FFFF, // user data (ring 3), for later
            0x00AF_FA00_0000_FFFF, // user code (ring 3, 64-bit), for later
            tss_low,
            base >> 32,
        ];

        let pointer = Pointer { limit: (size_of::<[u64; 7]>() - 1) as u16, base: GDT.get() as u64 };
        asm!("lgdt [{}]", in(reg) &pointer, options(readonly, nostack, preserves_flags));
        // Reload the code segment with a far return to the next instruction.
        asm!(
            "push {sel}",
            "lea {tmp}, [rip + 55f]",
            "push {tmp}",
            "retfq",
            "55:",
            sel = in(reg) KERNEL_CODE as u64,
            tmp = lateout(reg) _,
            options(preserves_flags)
        );
        asm!(
            "mov ds, {d:x}",
            "mov es, {d:x}",
            "mov ss, {d:x}",
            "mov fs, {z:x}",
            "mov gs, {z:x}",
            d = in(reg) KERNEL_DATA,
            z = in(reg) 0u16,
            options(nostack, preserves_flags)
        );
        asm!("ltr {:x}", in(reg) TSS_SELECTOR, options(nostack, preserves_flags));
    }
}
