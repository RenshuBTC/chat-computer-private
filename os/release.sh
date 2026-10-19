#!/bin/sh
# Builds Chat Computer's own system and puts the result in release/, where "Chat Computer Own System" on Windows
# takes it from (Windows has no Rust compiler). The build is reproducible: CI rebuilds it from this source
# and checks it is the same file, byte for byte.
set -eu
cd "$(dirname "$0")"
./build.sh
mkdir -p release
cp out/esp/EFI/BOOT/BOOTX64.EFI release/chatcomputer.efi
(cd release && sha256sum chatcomputer.efi > chatcomputer.efi.sha256)
cat release/chatcomputer.efi.sha256
