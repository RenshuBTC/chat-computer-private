//! The kernel's interrupt table: where the processor goes on a fault, a timer tick, or any other
//! interrupt. All 256 entries lead to one small stub each, which saves the registers and calls
//! `interrupt_dispatch`. The kernel is built to use the processor's floating-point and vector registers (the
//! model's arithmetic needs them, and the compiler uses them anywhere), so the stub saves those too (FXSAVE: the
//! x87 and SSE registers; a handler, built for SSE2 alone, never touches the upper halves of AVX's).

use crate::gdt;
use crate::sync::Static;
use crate::{apic, time};
use core::arch::{asm, global_asm};
use core::mem::size_of;
use core::sync::atomic::{AtomicUsize, Ordering};

pub const TIMER_VECTOR: u8 = 0x30;
pub const APIC_ERROR_VECTOR: u8 = 0x31;
/// The first core's message to the others that there's work for them (cores.rs): nothing to do but acknowledge it.
pub const WAKE_VECTOR: u8 = 0x32;
pub const SPURIOUS_VECTOR: u8 = 0xFF;
/// Vectors for devices' interrupt messages, each with its driver's handler.
pub const DEVICE_VECTORS: core::ops::Range<u8> = 0x40..0x50;
pub const USB_VECTOR: u8 = 0x40;
pub const NET_VECTOR: u8 = 0x41;

static DEVICE_HANDLERS: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];

/// Has interrupts at `vector` (one of `DEVICE_VECTORS`) call `handler`, with interrupts off.
pub fn set_device_handler(vector: u8, handler: fn()) {
    assert!(DEVICE_VECTORS.contains(&vector));
    DEVICE_HANDLERS[(vector - DEVICE_VECTORS.start) as usize].store(handler as usize, Ordering::Release);
}
/// The old 8259 interrupt controllers are moved to 0x20-0x2F and switched off; these two can still
/// arrive ("spurious" interrupts) and need no answer.
const PIC_SPURIOUS: [u64; 2] = [0x27, 0x2F];

// One 16-byte stub per vector. The processor pushes an error code for some exceptions; for the others
// the stub pushes 0, so the saved frame always has the same shape.
global_asm!(
    r#"
    .section .text.interrupts, "ax"
    .global interrupt_stubs
    .p2align 4
interrupt_stubs:
    .set vector, 0
    .rept 256
        .p2align 4
        .if (vector == 8) || (vector == 10) || (vector == 11) || (vector == 12) || (vector == 13) || (vector == 14) || (vector == 17) || (vector == 21) || (vector == 29) || (vector == 30)
        .else
            pushq $0
        .endif
        pushq $vector
        jmp interrupt_common
        .set vector, vector + 1
    .endr

interrupt_common:
    pushq %rax
    pushq %rbx
    pushq %rcx
    pushq %rdx
    pushq %rsi
    pushq %rdi
    pushq %rbp
    pushq %r8
    pushq %r9
    pushq %r10
    pushq %r11
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    subq $512, %rsp
    fxsave64 (%rsp)
    leaq 512(%rsp), %rdi
    cld
    call interrupt_dispatch
    fxrstor64 (%rsp)
    addq $512, %rsp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %r11
    popq %r10
    popq %r9
    popq %r8
    popq %rbp
    popq %rdi
    popq %rsi
    popq %rdx
    popq %rcx
    popq %rbx
    popq %rax
    addq $16, %rsp
    iretq
"#,
    options(att_syntax)
);

unsafe extern "C" {
    static interrupt_stubs: u8;
}

/// The registers as they were when the interrupt came, in the order the stub saved them.
#[repr(C)]
pub struct Frame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Gate {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

const _: () = assert!(size_of::<Gate>() == 16);

static IDT: Static<[Gate; 256]> = Static::new(
    [Gate { offset_low: 0, selector: 0, ist: 0, attributes: 0, offset_mid: 0, offset_high: 0, reserved: 0 }; 256],
);

#[repr(C, packed)]
struct Pointer {
    limit: u16,
    base: u64,
}

pub fn init() {
    let stubs = &raw const interrupt_stubs as u64;
    let table = unsafe { &mut *IDT.get() };
    for (vector, gate) in table.iter_mut().enumerate() {
        let handler = stubs + 16 * vector as u64;
        let ist = match vector {
            2 => gdt::NMI_STACK,
            8 => gdt::DOUBLE_FAULT_STACK,
            18 => gdt::MACHINE_CHECK_STACK,
            _ => 0,
        };
        *gate = Gate {
            offset_low: handler as u16,
            selector: gdt::KERNEL_CODE,
            ist,
            attributes: 0x8E, // present, ring 0, interrupt gate (interrupts stay off inside the handler)
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        };
    }
    load();
}

/// Has this core use the interrupt table (every core uses the same one).
pub fn load() {
    let pointer = Pointer { limit: (size_of::<[Gate; 256]>() - 1) as u16, base: IDT.get() as u64 };
    unsafe { asm!("lidt [{}]", in(reg) &pointer, options(readonly, nostack, preserves_flags)) };
}

const EXCEPTIONS: [&str; 32] = [
    "Divide error",
    "Debug",
    "Non-maskable interrupt",
    "Breakpoint",
    "Overflow",
    "Bound range exceeded",
    "Invalid opcode",
    "Device not available",
    "Double fault",
    "Coprocessor segment overrun",
    "Invalid TSS",
    "Segment not present",
    "Stack-segment fault",
    "General protection fault",
    "Page fault",
    "Reserved (15)",
    "x87 floating-point error",
    "Alignment check",
    "Machine check",
    "SIMD floating-point error",
    "Virtualization exception",
    "Control protection fault",
    "Reserved (22)",
    "Reserved (23)",
    "Reserved (24)",
    "Reserved (25)",
    "Reserved (26)",
    "Reserved (27)",
    "Hypervisor injection",
    "VMM communication",
    "Security exception",
    "Reserved (31)",
];

#[unsafe(no_mangle)]
extern "C" fn interrupt_dispatch(frame: &mut Frame) {
    let vector = frame.vector;
    if vector == 2 {
        nmi(frame);
    } else if vector < 32 {
        exception(frame);
    } else if vector == TIMER_VECTOR as u64 {
        time::tick();
        apic::end_of_interrupt();
    } else if vector == WAKE_VECTOR as u64 {
        apic::end_of_interrupt();
    } else if vector == APIC_ERROR_VECTOR as u64 {
        crate::log!("Chat Computer kernel: the interrupt controller reported error {:#x}", apic::error_status());
        apic::end_of_interrupt();
    } else if vector == SPURIOUS_VECTOR as u64 || PIC_SPURIOUS.contains(&vector) {
        // Nothing to do, and nothing to acknowledge.
    } else if (DEVICE_VECTORS.start as u64..DEVICE_VECTORS.end as u64).contains(&vector) {
        let handler = DEVICE_HANDLERS[(vector - DEVICE_VECTORS.start as u64) as usize].load(Ordering::Acquire);
        if handler != 0 {
            let handler: fn() = unsafe { core::mem::transmute(handler) };
            handler();
        }
        apic::end_of_interrupt();
    } else {
        crate::log!("Chat Computer kernel: unexpected interrupt {vector:#x}");
        apic::end_of_interrupt();
    }
}

/// NMIs said so far (more than a few aren't each said).
static NMIS: AtomicUsize = AtomicUsize::new(0);

/// A non-maskable interrupt: from the chipset (a memory or bus error it noticed, a watchdog), or a button some PCs
/// have. Nothing of Chat Computer's sends one, and it's not a fault of the code it interrupted: said in the log (on
/// its own stack, never waiting for a lock: it can come in the middle of anything), with why, as far as the old
/// NMI status port (0x61) says, and the work goes on.
fn nmi(f: &Frame) {
    let n = NMIS.fetch_add(1, Ordering::Relaxed);
    if n < 8 {
        let status = unsafe { crate::cpu::inb(0x61) };
        let why = match (status & 0x80 != 0, status & 0x40 != 0) {
            (true, true) => "a memory or bus error (SERR) and an I/O channel check",
            (true, false) => "a memory or bus error (SERR)",
            (false, true) => "an I/O channel check",
            (false, false) => "no reason the chipset's status port gives",
        };
        crate::serial::write_anyway(format_args!(
            "Chat Computer kernel: a non-maskable interrupt (NMI): {why} (port 0x61: {status:#04x}), at {:#x}; going on{}\n",
            f.rip,
            if n == 7 { " (more won't be said)" } else { "" }
        ));
    }
}

/// What a machine check's error banks say (MCi_STATUS with its valid bit, and MCi_ADDR where it's valid), for the
/// log: the processor found a hardware error (memory, cache, bus).
fn machine_check_banks() {
    const IA32_MCG_CAP: u32 = 0x179;
    const IA32_MCG_STATUS: u32 = 0x17A;
    let cpu_has = crate::cpu::cpuid(1, 0).3;
    // (Machine checks and their architecture: CPUID 1 EDX bits 7 and 14.)
    if cpu_has & (1 << 7) == 0 || cpu_has & (1 << 14) == 0 {
        return;
    }
    let banks = unsafe { crate::cpu::rdmsr(IA32_MCG_CAP) } & 0xFF;
    let status = unsafe { crate::cpu::rdmsr(IA32_MCG_STATUS) };
    crate::serial::write_anyway(format_args!("Chat Computer kernel: machine check: MCG_STATUS {status:#x}, {banks} banks\n"));
    for i in 0..banks.min(32) as u32 {
        let bank = unsafe { crate::cpu::rdmsr(0x401 + 4 * i) };
        if bank & (1 << 63) == 0 {
            continue;
        }
        let address = if bank & (1 << 58) != 0 { unsafe { crate::cpu::rdmsr(0x402 + 4 * i) } } else { 0 };
        crate::serial::write_anyway(format_args!(
            "Chat Computer kernel: machine check: bank {i}: status {bank:#018x}, address {address:#x}\n"
        ));
    }
}

fn exception(f: &Frame) -> ! {
    if f.vector == 18 {
        machine_check_banks();
    }
    // (Running past the end of the kernel's stack faults just past it, where nothing is mapped (paging.rs), and that
    // fault can't be taken on the same stack, so it comes as a double fault, on a stack of its own: said as what it is.)
    let name = if matches!(f.vector, 8 | 14) && crate::paging::past_stack(crate::cpu::cr2()) {
        "The kernel's stack ran out"
    } else {
        EXCEPTIONS[f.vector as usize]
    };
    if !crate::cores::is_first() {
        // (Another core's: it stops, and says why; the first core shows it, and stops too.)
        crate::cores::stop_here(format_args!(
            "{name} (vector {}, error {:#x}, at {:#x}, address {:#x}, stack {:#x})",
            f.vector, f.error, f.rip, crate::cpu::cr2(), f.rsp
        ));
    }
    crate::serial::write_anyway(format_args!(
        "\nChat Computer kernel stopped: {name} (vector {}, error {:#x})\n\
         rip {:#018x}  cs {:#x}  rflags {:#x}  rsp {:#018x}  ss {:#x}\n\
         rax {:#018x}  rbx {:#018x}  rcx {:#018x}  rdx {:#018x}\n\
         rsi {:#018x}  rdi {:#018x}  rbp {:#018x}  r8  {:#018x}\n\
         r9  {:#018x}  r10 {:#018x}  r11 {:#018x}  r12 {:#018x}\n\
         r13 {:#018x}  r14 {:#018x}  r15 {:#018x}  cr2 {:#018x}\n",
        f.vector, f.error, f.rip, f.cs, f.rflags, f.rsp, f.ss, f.rax, f.rbx, f.rcx, f.rdx, f.rsi, f.rdi,
        f.rbp, f.r8, f.r9, f.r10, f.r11, f.r12, f.r13, f.r14, f.r15,
        crate::cpu::cr2()
    ));
    crate::chat::show_stop(
        name,
        format_args!("vector {}, error {:#x}, at {:#x}, address {:#x}.", f.vector, f.error, f.rip, crate::cpu::cr2()),
    );
    crate::journal::save_on_stop();
    crate::cpu::stop()
}
