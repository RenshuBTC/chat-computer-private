#!/bin/sh
# Chat Computer's own system in a virtual PC (QEMU, UEFI firmware from OVMF), with Meta's Muse Spark answering:
# the key for Meta's Model API goes into the settings file the kernel reads at start-up, and the kernel itself sends
# the conversation to api.meta.ai over its own network stack and TLS 1.3.
#
#     META_API_KEY=... ./try-muse-spark.sh [1280x720]
#
# Needs qemu-system-x86_64 and OVMF (Debian/Ubuntu: apt install qemu-system-x86 ovmf). It uses the system as built
# here, else release/chatcomputer.efi (built from this source by release.sh), else builds it (with the Rust toolchain
# that rust-toolchain.toml names). Type a message and press Enter; Windows+H talks, when a speech model is on a drive
# (see README.md).
set -eu
cd "$(dirname "$0")"
: "${META_API_KEY:?set META_API_KEY to a key from https://dev.meta.ai}"
size="${1:-1280x720}"
efi=out/esp/EFI/BOOT/BOOTX64.EFI
[ -f "$efi" ] || efi=release/chatcomputer.efi
[ -f "$efi" ] || { ./build.sh; efi=out/esp/EFI/BOOT/BOOTX64.EFI; }
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/esp/EFI/BOOT" "$work/esp/EFI/chatcomputer-own"
cp "$efi" "$work/esp/EFI/BOOT/BOOTX64.EFI"
printf 'meta-api-key = %s\r\n' "$META_API_KEY" > "$work/esp/EFI/chatcomputer-own/settings.txt"
code=/usr/share/OVMF/OVMF_CODE_4M.fd
vars=/usr/share/OVMF/OVMF_VARS_4M.fd
[ -f "$code" ] || { code=/usr/share/OVMF/OVMF_CODE.fd; vars=/usr/share/OVMF/OVMF_VARS.fd; }
cp "$vars" "$work/vars.fd"
qemu-system-x86_64 -machine q35 -cpu max -m 2048 -smp 2 \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$code" \
    -drive "if=pflash,format=raw,unit=1,file=$work/vars.fd" \
    -drive "format=raw,file=fat:rw:$work/esp" \
    -vga none -device "VGA,edid=on,xres=${size%x*},yres=${size#*x}" \
    -netdev user,id=n0 -device virtio-net-pci,netdev=n0
