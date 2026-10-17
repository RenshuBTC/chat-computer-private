/* Where the kernel's parts go in memory. The top 2 GiB of the address space, one part per kind of
   access, each starting on its own page so each can have its own permissions:
   code (read, run), read-only data (read), data (read, write). */
ENTRY(kernel_entry)

PHDRS {
    code   PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data   PT_LOAD FLAGS(6);
}

SECTIONS {
    . = 0xFFFFFFFF80000000;

    .text : ALIGN(4K) {
        *(.text .text.*)
    } :code

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    } :rodata
    .eh_frame_hdr : { *(.eh_frame_hdr) } :rodata
    .eh_frame : { *(.eh_frame) } :rodata

    .data : ALIGN(4K) {
        *(.data .data.*)
        *(.got .got.*)
    } :data

    .bss : ALIGN(4K) {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    /DISCARD/ : {
        *(.comment)
        *(.note .note.*)
    }
}
