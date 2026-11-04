//! The processor's other cores. A PC starts with one core running (the first, which runs the firmware, then Chat
//! Computer); the others wait. Chat Computer starts each the way every x86 core starts: an INIT message resets it to
//! wait, then a start-up message names a page below 1 MiB, where it starts in 16-bit real mode. The code there
//! (below) switches it to protected mode, then to 64-bit mode with page tables made for the switch (that page at its
//! own address, and the kernel's half of the address space), then into the kernel, on a stack of its own, where it
//! takes the kernel's own page tables, the first core's settings (its control registers: caches, SSE's and AVX's
//! registers; its page attribute table) and a segment table and TSS of its own.
//!
//! What they do: the model's arithmetic. Each big step of it is some number of jobs (rows of a matrix product, heads
//! of attention), which every core takes from, one at a time, the first core included, as the app's threads do
//! (model/src/threads.rs): `Cores` is the pool the kernel runs the model with. Between steps the others wait a moment
//! for the next one, then sleep until the first core's message (`idt::WAKE_VECTOR`) wakes them. Only the first core
//! runs everything else; the others' interrupts stay off but while they sleep, and no device sends them any.
//!
//! A core other than the first that faults or panics stops, and says so in the log; the first one then stops too,
//! saying which core stopped and why on the screen (`check`).

use crate::acpi::Acpi;
use crate::sync::Static;
use crate::{apic, cpu, gdt, idt, memory, paging, time};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use bootinfo::{memtype, BootInfo, PAGE_SIZE, PHYS_OFFSET};
use core::arch::{asm, global_asm};
use core::fmt::{self, Write};
use core::mem::offset_of;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// The most cores used, the first included (the rest of a bigger processor's stay waiting).
const MOST: usize = paging::MOST_CORES;
/// Each other core's stack: 128 KiB (the model's arithmetic needs little), with nothing mapped below it.
const STACK_PAGES: u64 = 32;
/// How long a core waits for the next step before it sleeps, in microseconds (steps come a few apart while the model
/// works; between its tokens, the first core draws and does the rest).
const AWAKE_US: u64 = 2_000;

// The start: copied to the page below 1 MiB. Its data at `cores_launch` is `Launch`'s.
global_asm!(
    r#"
    .section .rodata.cores_trampoline, "a"
    .global cores_trampoline
    .global cores_trampoline_end
    .global cores_launch
    .p2align 4
cores_trampoline:
    .code16
    cli
    cld
    movw %cs, %ax
    movw %ax, %ds
    # Take this start if it's on offer (1, to 2: taken). A core that starts late, finding it taken or withdrawn,
    # stops here.
    movl $1, %eax
    movl $2, %ecx
    lock cmpxchgl %ecx, (cores_launch - cores_trampoline)
    jne cores_lost
    # This page's address: the code segment's, times 16.
    xorl %ebx, %ebx
    movw %cs, %bx
    shll $4, %ebx
    # The segment table's address and protected mode's code's, as this page's address plus theirs in it.
    leal (cores_gdt - cores_trampoline)(%ebx), %eax
    movl %eax, (cores_gdt_pointer + 2 - cores_trampoline)
    leal (cores_start32 - cores_trampoline)(%ebx), %eax
    movl %eax, (cores_far32 - cores_trampoline)
    lgdtl (cores_gdt_pointer - cores_trampoline)
    # Protected mode.
    movl %cr0, %eax
    orl $1, %eax
    movl %eax, %cr0
    ljmpl *(cores_far32 - cores_trampoline)
cores_lost:
    cli
    hlt
    jmp cores_lost

    .code32
cores_start32:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw %ax, %fs
    movw %ax, %gs
    # PAE, which 64-bit mode's page tables need; those made for the switch.
    movl %cr4, %eax
    orl $0x20, %eax
    movl %eax, %cr4
    movl (cores_launch + 4 - cores_trampoline)(%ebx), %eax
    movl %eax, %cr3
    # Long mode, and no-execute where the kernel's page tables use it (EFER).
    movl $0xC0000080, %ecx
    rdmsr
    orl (cores_launch + 8 - cores_trampoline)(%ebx), %eax
    wrmsr
    # Paging on (so long mode), and the caches, which INIT left off (CD and NW).
    movl %cr0, %eax
    andl $0x9FFFFFFF, %eax
    orl $0x80000000, %eax
    movl %eax, %cr0
    leal (cores_start64 - cores_trampoline)(%ebx), %eax
    movl %eax, (cores_far64 - cores_trampoline)(%ebx)
    ljmpl *(cores_far64 - cores_trampoline)(%ebx)

    .code64
cores_start64:
    movl %ebx, %ebx
    movq (cores_launch + 16 - cores_trampoline)(%rbx), %rsp
    movq (cores_launch + 24 - cores_trampoline)(%rbx), %rdi
    movq (cores_launch + 32 - cores_trampoline)(%rbx), %rsi
    movq (cores_launch + 40 - cores_trampoline)(%rbx), %rax
    jmpq *%rax

    .p2align 3
cores_launch:
    .long 0, 0, 0, 0
    .quad 0, 0, 0, 0
    .p2align 3
cores_gdt:
    .quad 0
    .quad 0x00CF9A000000FFFF
    .quad 0x00CF92000000FFFF
    .quad 0x00AF9A000000FFFF
cores_gdt_pointer:
    .word 31
    .long 0
cores_far32:
    .long 0
    .word 0x08
cores_far64:
    .long 0
    .word 0x18
cores_trampoline_end:
"#,
    options(att_syntax)
);

// In the kernel, on the core's own stack (rdi: its number; rsi: the kernel's page tables): the kernel's own page
// tables, the first core's control registers (caches on; SSE's registers, which any of the kernel's code may use, on:
// so before any of it runs), then `core_main`.
global_asm!(
    r#"
    .section .text.cores_entry, "ax"
    .global cores_entry
    .p2align 4
cores_entry:
    movq %rsi, %cr3
    movq {cr0}(%rip), %rax
    movq %rax, %cr0
    movq {cr4}(%rip), %rax
    movq %rax, %cr4
    xorl %ebp, %ebp
    call {main}
    ud2
"#,
    cr0 = sym CORE_CR0,
    cr4 = sym CORE_CR4,
    main = sym core_main,
    options(att_syntax)
);

unsafe extern "C" {
    static cores_trampoline: u8;
    static cores_trampoline_end: u8;
    static cores_launch: u8;
    fn cores_entry();
}

/// What the start's code reads, at `cores_launch` in its page.
#[repr(C)]
struct Launch {
    /// 0: nothing on offer; 1: on offer; 2: taken (by the core that started); 3: withdrawn (none did in time).
    claim: AtomicU32,
    /// The page tables for the switch (below 4 GiB, as 32-bit code loads them).
    tables: u32,
    /// The bits to set in EFER.
    efer: u32,
    _unused: u32,
    stack: u64,
    core: u64,
    kernel_tables: u64,
    entry: u64,
}

const _: () = assert!(
    offset_of!(Launch, tables) == 4
        && offset_of!(Launch, efer) == 8
        && offset_of!(Launch, stack) == 16
        && offset_of!(Launch, core) == 24
        && offset_of!(Launch, kernel_tables) == 32
        && offset_of!(Launch, entry) == 40
);

const OFFERED: u32 = 1;
const WITHDRAWN: u32 = 3;

/// The first core's settings, for the others: CR0 (caches on), CR4 (what's safe to copy), XCR0, the page attribute
/// table, and MXCSR (how SSE's and AVX's arithmetic rounds: the same everywhere, for the same answers).
static CORE_CR0: AtomicU64 = AtomicU64::new(0);
static CORE_CR4: AtomicU64 = AtomicU64::new(0);
static CORE_XCR0: AtomicU64 = AtomicU64::new(0);
static CORE_PAT: AtomicU64 = AtomicU64::new(0);
static CORE_MXCSR: AtomicU32 = AtomicU32::new(0x1F80);

/// The cores in use (the first included), and each one's interrupt controller id (the first's at 0).
static COUNT: AtomicUsize = AtomicUsize::new(1);
static IDS: [AtomicU32; MOST] = [const { AtomicU32::new(0) }; MOST];
/// Cores that have reached the kernel, and those of them that didn't join (their registers aren't the first
/// core's); whether any has been sent its start.
static ALIVE: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicUsize = AtomicUsize::new(0);
static STARTING: AtomicBool = AtomicBool::new(false);

/// One step's jobs (on the first core's stack, which waits until every core is done with them): the function, how
/// many, which is next, how many are left.
struct Job {
    f: *const (dyn Fn(usize) + Sync),
    total: usize,
    next: AtomicUsize,
    left: AtomicUsize,
}

/// A new step for the others: its job (null between steps), counted on.
static CURRENT: AtomicPtr<Job> = AtomicPtr::new(core::ptr::null_mut());
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Cores looking at the current job: until none is, it stays where it is.
static INSIDE: AtomicUsize = AtomicUsize::new(0);
/// Cores asleep, one bit each.
static ASLEEP: AtomicU64 = AtomicU64::new(0);
/// Jobs of steps shared among the cores, and how many of them the others did (since last said: `shared`).
static SHARED_JOBS: AtomicU64 = AtomicU64::new(0);
static OTHERS_DID: AtomicU64 = AtomicU64::new(0);

/// A core other than the first stopped, and why.
static STOPPED: AtomicBool = AtomicBool::new(false);
static STOP_CLAIMED: AtomicBool = AtomicBool::new(false);
static STOP_TEXT: Static<[u8; 480]> = Static::new([0; 480]);
static STOP_LEN: AtomicUsize = AtomicUsize::new(0);

/// How many cores are in use, the first included.
pub fn count() -> usize {
    COUNT.load(Ordering::Acquire)
}

/// Whether this is the first core (the only one, until the others start).
pub fn is_first() -> bool {
    !STARTING.load(Ordering::Acquire) || apic::live_id() == apic::id()
}

/// Which core this is: 0 for the first.
fn this_core() -> usize {
    let id = apic::live_id();
    if id == apic::id() {
        return 0;
    }
    (1..MOST).find(|&c| IDS[c].load(Ordering::Relaxed) == id).unwrap_or(MOST)
}

/// A free page below 1 MiB (the kernel gives out none of that first MiB: memory.rs), not page 0: first among what
/// the firmware left free, then among what it used while it was in charge (free now, and in use by no core: they
/// have all been reset by then).
fn low_page(info: &BootInfo) -> Option<u64> {
    let map = unsafe { info.memory_map() };
    let kinds = [memtype::CONVENTIONAL, memtype::BOOT_SERVICES_DATA, memtype::BOOT_SERVICES_CODE];
    kinds.iter().find_map(|&kind| {
        map.iter().filter(|r| r.kind == kind).find_map(|r| {
            let start = r.start.max(0x1000).div_ceil(PAGE_SIZE) * PAGE_SIZE;
            (start + PAGE_SIZE <= r.end().min(0xA_0000)).then_some(start)
        })
    })
}

const PRESENT: u64 = 1;
const WRITABLE: u64 = 1 << 1;
const HUGE: u64 = 1 << 7;
const ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;

/// The page tables for the switch: the first 2 MiB at their own addresses (the start's page among them), and the
/// kernel's half of the address space as the kernel's own tables have it. The top table below 4 GiB.
fn switch_tables() -> Option<u64> {
    let below = 1 << 32;
    let (top, middle, low) =
        (memory::alloc_zeroed_below(below)?, memory::alloc_zeroed_below(below)?, memory::alloc_zeroed_below(below)?);
    let table = |p: u64| (PHYS_OFFSET + p) as *mut u64;
    unsafe {
        *table(low) = PRESENT | WRITABLE | HUGE;
        *table(middle) = low | PRESENT | WRITABLE;
        *table(top) = middle | PRESENT | WRITABLE;
        let kernel = (PHYS_OFFSET + (cpu::cr3() & ADDRESS)) as *const u64;
        for i in 256..512 {
            *table(top).add(i) = *kernel.add(i);
        }
    }
    Some(top)
}

fn xcr0() -> u64 {
    let (lo, hi): (u32, u32);
    unsafe { asm!("xgetbv", in("ecx") 0u32, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) };
    (hi as u64) << 32 | lo as u64
}

/// Waits up to `us` microseconds for `done`; whether it came.
fn wait_for(us: u64, done: impl Fn() -> bool) -> bool {
    let until = time::now_us() + us;
    loop {
        if done() {
            return true;
        }
        if time::now_us() > until {
            return false;
        }
        check();
        core::hint::spin_loop();
    }
}

/// Starts the other cores the firmware lists (enabled), up to `MOST` in all, or `limit` (the settings'). Says how it
/// went. On the first core, once its interrupt controller and timer are set up, before the firmware's memory is
/// reused.
pub fn start(acpi: &Acpi, info: &BootInfo, limit: Option<usize>) -> String {
    let first = apic::id();
    let x2 = apic::x2apic();
    let others: Vec<u32> = acpi.core_ids.iter().copied().filter(|&id| id != first && (x2 || id < 0xFF)).collect();
    let listed = acpi.core_ids.len().max(1);
    let most = limit.unwrap_or(MOST).clamp(1, MOST);
    if others.is_empty() {
        return format!("processor cores: 1 ({} listed)", listed);
    }
    if most == 1 {
        return format!("processor cores: 1 of the {listed} listed (the settings say so)");
    }
    // Every other core reset, wherever the firmware left it (some may be left so, if there are more than MOST).
    STARTING.store(true, Ordering::Release);
    for &id in &others {
        if !apic::send(id, apic::INIT) {
            crate::log!("Chat Computer kernel: core with interrupt controller {id} didn't take its INIT message");
        }
    }
    time::delay_ms(10);

    let Some(page) = low_page(info) else {
        return format!("processor cores: 1 of {listed} (no free page below 1 MiB to start the others from)");
    };
    let Some(tables) = switch_tables() else {
        return format!("processor cores: 1 of {listed} (no memory for the page tables to start the others with)");
    };
    let code = unsafe {
        let start = &raw const cores_trampoline;
        let len = (&raw const cores_trampoline_end).offset_from(start) as usize;
        core::slice::from_raw_parts(start, len)
    };
    let at = (&raw const cores_launch as usize) - (&raw const cores_trampoline as usize);
    unsafe { core::ptr::copy_nonoverlapping(code.as_ptr(), (PHYS_OFFSET + page) as *mut u8, code.len()) };
    let launch = unsafe { &mut *(((PHYS_OFFSET + page) as usize + at) as *mut Launch) };

    // The first core's settings, for the others.
    const CD: u64 = 1 << 30;
    const NW: u64 = 1 << 29;
    const TS: u64 = 1 << 3;
    const EM: u64 = 1 << 2;
    const MP: u64 = 1 << 1;
    CORE_CR0.store((cpu::cr0() & !(CD | NW | TS | EM)) | MP, Ordering::Relaxed);
    // PAE, machine checks, global pages, SSE (FXSAVE and its exceptions), FSGSBASE, XSAVE: as the first core has them.
    const CR4_COPIED: u64 = 1 << 5 | 1 << 6 | 1 << 7 | 1 << 9 | 1 << 10 | 1 << 16 | 1 << 18;
    let cr4 = cpu::cr4() & CR4_COPIED;
    CORE_CR4.store(cr4, Ordering::Relaxed);
    CORE_XCR0.store(if cr4 & 1 << 18 != 0 { xcr0() } else { 0 }, Ordering::Relaxed);
    const IA32_PAT: u32 = 0x277;
    CORE_PAT.store(unsafe { cpu::rdmsr(IA32_PAT) }, Ordering::Relaxed);
    let mut mxcsr = 0u32;
    unsafe { asm!("stmxcsr [{}]", in(reg) &mut mxcsr, options(nostack, preserves_flags)) };
    CORE_MXCSR.store(mxcsr, Ordering::Relaxed);
    const EFER: u32 = 0xC000_0080;
    const LME: u64 = 1 << 8;
    const NXE: u64 = 1 << 11;
    let efer = (unsafe { cpu::rdmsr(EFER) } & NXE) | LME;

    launch.tables = tables as u32;
    launch.efer = efer as u32;
    launch.kernel_tables = cpu::cr3() & ADDRESS;
    launch.entry = cores_entry as *const () as u64;
    let vector = apic::STARTUP | (page >> 12) as u32;
    let (mut started, mut stacks, mut not_started, mut refused) = (0usize, 0usize, 0usize, 0usize);
    let mut stuck = false;
    for &id in &others {
        let core = 1 + started;
        if core >= most {
            break;
        }
        if stacks < core {
            launch.stack = paging::map_core_stack(core, STACK_PAGES);
            stacks = core;
        }
        launch.core = core as u64;
        IDS[core].store(id, Ordering::Relaxed);
        let (alive, refusals) = (ALIVE.load(Ordering::Acquire), REFUSED.load(Ordering::Acquire));
        launch.claim.store(OFFERED, Ordering::Release);
        // The start-up message; again if the first wasn't taken, as Intel and AMD say.
        apic::send(id, vector);
        if !wait_for(1_000, || launch.claim.load(Ordering::Acquire) != OFFERED) {
            apic::send(id, vector);
            wait_for(100_000, || launch.claim.load(Ordering::Acquire) != OFFERED);
        }
        if launch.claim.compare_exchange(OFFERED, WITHDRAWN, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            not_started += 1;
            crate::log!("Chat Computer kernel: core with interrupt controller {id} didn't start");
            continue;
        }
        // Taken: it's on its way to the kernel.
        if !wait_for(500_000, || ALIVE.load(Ordering::Acquire) > alive) {
            // (Still using the start's page, maybe: no other core is started from it.)
            crate::log!("Chat Computer kernel: core with interrupt controller {id} started but didn't reach the kernel");
            not_started += 1;
            stuck = true;
            break;
        }
        if REFUSED.load(Ordering::Acquire) > refusals {
            // (Halted for good, using nothing of the start's.)
            crate::log!("Chat Computer kernel: core with interrupt controller {id} has registers the first core's don't match: not used");
            refused += 1;
            continue;
        }
        started += 1;
        COUNT.store(1 + started, Ordering::Release);
    }
    let used = 1 + started;
    let mut said = format!("processor cores: {used} in use");
    if used < listed {
        said += &format!(" of the {listed} listed");
        if not_started > 0 {
            said += &format!(" ({not_started} didn't start{})", if stuck { ", the last of them stuck" } else { "" });
        }
        if refused > 0 {
            said += &format!(" ({refused} unlike the first)");
        }
        if used == most && limit.is_some() {
            said += " (as many as the settings say)";
        }
    }
    said += &format!(", started from the page at {page:#x}");
    said
}

/// The kernel, on a core other than the first, with its control registers set as the first core's (`cores_entry`).
extern "C" fn core_main(core: usize) -> ! {
    // (Interrupts are off, as INIT and the start left them.)
    idt::load();
    gdt::init_core();
    // AVX's registers (and AVX-512's) as the first core has them: if this core has them all (it's the same processor,
    // but a fault here would be the end of it), else it isn't used.
    let xcr0 = CORE_XCR0.load(Ordering::Relaxed);
    if xcr0 != 0 {
        let (a, _, _, d) = cpu::cpuid(0xD, 0);
        let has = (d as u64) << 32 | a as u64;
        if xcr0 & !has != 0 {
            ALIVE.fetch_add(1, Ordering::AcqRel);
            REFUSED.fetch_add(1, Ordering::AcqRel);
            cpu::stop();
        }
        unsafe {
            asm!("xsetbv", in("ecx") 0u32, in("eax") xcr0 as u32, in("edx") (xcr0 >> 32) as u32,
                 options(nomem, nostack, preserves_flags))
        };
    }
    let mxcsr = CORE_MXCSR.load(Ordering::Relaxed);
    unsafe { asm!("ldmxcsr [{}]", in(reg) &mxcsr, options(nostack, preserves_flags, readonly)) };
    const IA32_PAT: u32 = 0x277;
    unsafe { cpu::wrmsr(IA32_PAT, CORE_PAT.load(Ordering::Relaxed)) };
    apic::init_core();
    IDS[core.min(MOST - 1)].store(apic::live_id(), Ordering::Relaxed);
    ALIVE.fetch_add(1, Ordering::AcqRel);
    serve(core)
}

/// A core other than the first: the jobs of each step, as they come.
fn serve(core: usize) -> ! {
    let bit = 1u64 << (core % 64);
    let mut seen = GENERATION.load(Ordering::Acquire);
    loop {
        let mut since = time::now_us();
        while GENERATION.load(Ordering::Acquire) == seen {
            if time::now_us() < since + AWAKE_US {
                core::hint::spin_loop();
                continue;
            }
            // Asleep until woken (`wake`): said first, then looked again, so a step posted meanwhile isn't missed
            // (its message then arrives as this core sleeps, or before, and wakes it at once).
            ASLEEP.fetch_or(bit, Ordering::SeqCst);
            if GENERATION.load(Ordering::SeqCst) == seen {
                unsafe { asm!("sti", "hlt", "cli", options(nostack)) };
            }
            ASLEEP.fetch_and(!bit, Ordering::SeqCst);
            since = time::now_us();
        }
        seen = GENERATION.load(Ordering::Acquire);
        INSIDE.fetch_add(1, Ordering::SeqCst);
        let job = CURRENT.load(Ordering::SeqCst);
        if !job.is_null() {
            let did = work(unsafe { &*job });
            if did > 0 {
                OTHERS_DID.fetch_add(did, Ordering::Relaxed);
            }
        }
        INSIDE.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Does the step's jobs until there are none left to take; how many.
fn work(job: &Job) -> u64 {
    let mut did = 0;
    loop {
        let i = job.next.fetch_add(1, Ordering::Relaxed);
        if i >= job.total {
            return did;
        }
        // (The first core waits for every job before its function goes.)
        unsafe { (*job.f)(i) };
        job.left.fetch_sub(1, Ordering::Release);
        did += 1;
    }
}

/// For the log, after the model's answer: how much of its arithmetic the other cores did since last said.
pub fn shared() -> String {
    let (all, others) = (SHARED_JOBS.swap(0, Ordering::Relaxed), OTHERS_DID.swap(0, Ordering::Relaxed));
    if count() <= 1 || all == 0 {
        return String::new();
    }
    format!("; on {} processor cores, the others doing {}% of the jobs shared", count(), others * 100 / all)
}

/// Before the PC turns off or restarts: the other cores reset (INIT), waiting as the firmware expects them.
pub fn park() {
    for core in 1..count().min(MOST) {
        apic::send(IDS[core].load(Ordering::Relaxed), apic::INIT);
    }
    COUNT.store(1, Ordering::Release);
}

/// Wakes the cores in `asleep`.
fn wake(asleep: u64) {
    for core in 1..count().min(MOST) {
        if asleep & 1 << (core % 64) != 0 {
            apic::send(IDS[core].load(Ordering::Relaxed), apic::FIXED | idt::WAKE_VECTOR as u32);
        }
    }
}

/// Every core: the pool the model's engine runs with. Only the first core runs it.
pub struct Cores;

impl model::Pool for Cores {
    fn run(&self, jobs: usize, f: &(dyn Fn(usize) + Sync)) {
        if jobs <= 1 || count() <= 1 {
            for i in 0..jobs {
                f(i);
            }
            return;
        }
        // (The function's lifetime is the caller's: it's only used until every job is done, below, and no core can
        // start it again after that, every index being taken.)
        let f: *const (dyn Fn(usize) + Sync) = unsafe { core::mem::transmute(f) };
        let job = Job { f, total: jobs, next: AtomicUsize::new(0), left: AtomicUsize::new(jobs) };
        SHARED_JOBS.fetch_add(jobs as u64, Ordering::Relaxed);
        CURRENT.store(&job as *const Job as *mut Job, Ordering::SeqCst);
        GENERATION.fetch_add(1, Ordering::SeqCst);
        let asleep = ASLEEP.load(Ordering::SeqCst);
        if asleep != 0 {
            wake(asleep);
        }
        work(&job);
        let mut since = 0;
        while job.left.load(Ordering::Acquire) != 0 {
            check();
            core::hint::spin_loop();
            // (A job takes milliseconds: one not done in a minute is one a core is stuck on.)
            if since == 0 {
                since = time::now_us();
            } else if time::now_us() > since + 60_000_000 {
                panic!("a processor core has been on one step of the model's arithmetic for a minute: {} of its {} jobs \
                        not done", job.left.load(Ordering::Relaxed), jobs);
            }
        }
        // No core looks at it after this (one that looked before is done with it once it's no longer inside).
        CURRENT.store(core::ptr::null_mut(), Ordering::SeqCst);
        while INSIDE.load(Ordering::SeqCst) != 0 {
            check();
            core::hint::spin_loop();
        }
    }
}

/// On the first core: if another core stopped, stops too, saying which and why.
pub fn check() {
    if STOPPED.load(Ordering::Acquire) {
        let len = STOP_LEN.load(Ordering::Relaxed);
        let text: &[u8; 480] = unsafe { &*STOP_TEXT.get() };
        panic!("{}", core::str::from_utf8(&text[..len]).unwrap_or("another processor core stopped"));
    }
}

/// Stops a core other than the first (a fault or a panic on it), saying why: in the log, and for the first core to
/// show (`check`).
pub fn stop_here(why: fmt::Arguments) -> ! {
    cpu::disable_interrupts();
    let core = this_core();
    crate::serial::write_anyway(format_args!("\nChat Computer kernel: processor core {core} stopped: {why}\n"));
    if !STOP_CLAIMED.swap(true, Ordering::AcqRel) {
        let mut text = Text { buf: unsafe { &mut *STOP_TEXT.get() }, len: 0 };
        let _ = write!(text, "processor core {core} stopped: {why}");
        STOP_LEN.store(text.len, Ordering::Relaxed);
        STOPPED.store(true, Ordering::Release);
    }
    cpu::stop()
}

/// Text written into a fixed buffer, as much as fits (cut where a character ends).
struct Text {
    buf: &'static mut [u8; 480],
    len: usize,
}

impl Write for Text {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = self.buf.len() - self.len;
        let mut n = s.len().min(room);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// For the computer's status.
pub fn status() -> String {
    match count() {
        1 => String::from("Processor cores in use: 1"),
        n => format!("Processor cores in use: {n} (the model's arithmetic is shared among them)"),
    }
}

/// A test (CHATCOMPUTER_TEST=core): a core other than the first panics, in the middle of a step's jobs.
pub fn test_stop() {
    use model::Pool;
    Cores.run(4096, &|_| {
        if !is_first() {
            panic!("a test of what happens when a core other than the first stops");
        }
        time::delay_us(10);
    });
}
