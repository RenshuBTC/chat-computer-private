//! The kernel's own segment table (GDT) and task state segment (TSS). In 64-bit mode segments do
//! little, but the processor still needs them: a code segment, a data segment, and a TSS whose
//! separate stacks catch the faults that can't run on a broken stack. Each processor core has a table and a TSS of
//! its own (a core's TSS is marked busy while it uses it), with stacks of its own.

use crate::sync::Static;
use alloc::alloc::{alloc_zeroed, Layout};
use alloc::boxed::Box;
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

const STACK_SIZE: usize = 16 * 1024;

#[repr(C, align(16))]
struct Stack([u8; STACK_SIZE]);

const EMPTY_TSS: Tss = Tss { reserved0: 0, rsp: [0; 3], reserved1: 0, ist: [0; 7], reserved2: 0, reserved3: 0, iomap_base: 104 };

static TSS: Static<Tss> = Static::new(EMPTY_TSS);
static STACKS: Static<[Stack; 3]> = Static::new([const { Stack([0; STACK_SIZE]) }; 3]);
static GDT: Static<[u64; 7]> = Static::new([0; 7]);

#[repr(C, packed)]
struct Pointer {
    limit: u16,
    base: u64,
}

/// The first core's tables (its stacks are the kernel's own memory, there being no heap yet).
pub fn init() {
    unsafe { load(GDT.get(), TSS.get(), STACKS.get() as u64) }
}

/// The tables of a core other than the first: on the heap, for good.
pub fn init_core() {
    let gdt: *mut [u64; 7] = Box::leak(Box::new([0u64; 7]));
    let tss: *mut Tss = Box::leak(Box::new(EMPTY_TSS));
    let stacks = unsafe { alloc_zeroed(Layout::from_size_align(3 * STACK_SIZE, 4096).expect("the stacks' layout")) };
    assert!(!stacks.is_null(), "no memory for a processor core's own stacks");
    unsafe { load(gdt, tss, stacks as u64) }
}

/// Fills in the table `gdt` with the TSS `tss`, whose stacks for faults are the three from `stacks` up, and has this
/// core use them.
unsafe fn load(gdt: *mut [u64; 7], tss: *mut Tss, stacks: u64) {
    unsafe {
        let mut ist = [0u64; 7];
        for (i, top) in ist.iter_mut().take(3).enumerate() {
            *top = stacks + (i as u64 + 1) * STACK_SIZE as u64; // stacks grow down
        }
        (*tss).ist = ist;

        let base = tss as u64;
        let limit = (size_of::<Tss>() - 1) as u64;
        let tss_low = (limit & 0xFFFF)
            | (base & 0xFF_FFFF) << 16
            | 0x89 << 40 // present, 64-bit TSS (available)
            | ((limit >> 16) & 0xF) << 48
            | ((base >> 24) & 0xFF) << 56;
        *gdt = [
            0,
            0x00AF_9A00_0000_FFFF, // kernel code: 64-bit, ring 0
            0x00CF_9200_0000_FFFF, // kernel data
            0x00CF_F200_0000_FFFF, // user data (ring 3), for later
            0x00AF_FA00_0000_FFFF, // user code (ring 3, 64-bit), for later
            tss_low,
            base >> 32,
        ];

        let pointer = Pointer { limit: (size_of::<[u64; 7]>() - 1) as u16, base: gdt as u64 };
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
