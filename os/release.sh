#!/bin/sh
# Builds ClaudeOS's own system and puts the result in release/, where "ClaudeOS Own System" on Windows
# takes it from (Windows has no Rust compiler). The build is reproducible: CI rebuilds it from this source
# and checks it is the same file, byte for byte.
set -eu
cd "$(dirname "$0")"
./build.sh
mkdir -p release
cp out/esp/EFI/BOOT/BOOTX64.EFI release/claudeos.efi
(cd release && sha256sum claudeos.efi > claudeos.efi.sha256)
cat release/claudeos.efi.sha256
