#!/bin/sh
# Builds Chat Computer's own operating system: the kernel first, then the start-up stage that carries it.
# The result is one file, out/esp/EFI/BOOT/BOOTX64.EFI: the path a PC's firmware starts from a disk's
# EFI system partition when nothing else is set up.
set -eu
cd "$(dirname "$0")"

# The kernel uses the processor's floating-point and vector registers (its model's arithmetic needs them), so it's
# built for a target that has them (x86_64-unknown-none has none), as a program of its own: at a fixed address in
# the top 2 GiB (kernel/kernel.ld), with no red zone (an interrupt pushes onto the stack it comes on), linked by
# the toolchain's own linker, in a folder of its own (the flags are its alone).
lld="$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld"
RUSTFLAGS="-C relocation-model=static -C code-model=kernel -C no-redzone=yes -C linker=$lld -C linker-flavor=ld.lld" \
    cargo build --release -p chatcomputer-kernel --target x86_64-unknown-linux-gnu --target-dir target/kernel
CHATCOMPUTER_KERNEL_ELF="$PWD/target/kernel/x86_64-unknown-linux-gnu/release/chatcomputer-kernel" \
    cargo build --release -p chatcomputer-boot --target x86_64-unknown-uefi

mkdir -p out/esp/EFI/BOOT
cp target/x86_64-unknown-uefi/release/chatcomputer-boot.efi out/esp/EFI/BOOT/BOOTX64.EFI
echo "Built out/esp/EFI/BOOT/BOOTX64.EFI ($(wc -c < out/esp/EFI/BOOT/BOOTX64.EFI) bytes)"
