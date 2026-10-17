#!/bin/sh
# Builds ClaudeOS's own operating system: the kernel first, then the start-up stage that carries it.
# The result is one file, out/esp/EFI/BOOT/BOOTX64.EFI: the path a PC's firmware starts from a disk's
# EFI system partition when nothing else is set up.
set -eu
cd "$(dirname "$0")"

cargo build --release -p claudeos-kernel --target x86_64-unknown-none
CLAUDEOS_KERNEL_ELF="$PWD/target/x86_64-unknown-none/release/claudeos-kernel" \
    cargo build --release -p claudeos-boot --target x86_64-unknown-uefi

mkdir -p out/esp/EFI/BOOT
cp target/x86_64-unknown-uefi/release/claudeos-boot.efi out/esp/EFI/BOOT/BOOTX64.EFI
echo "Built out/esp/EFI/BOOT/BOOTX64.EFI ($(wc -c < out/esp/EFI/BOOT/BOOTX64.EFI) bytes)"
