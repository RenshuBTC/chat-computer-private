# What Chat Computer Setup downloads on a Mac, pinned and checked against SHA-256 fingerprints. Sourced by setup.sh.
#
# Chat Computer on a Mac is built at its first start on Fedora Asahi Remix's Minimal edition, which the Asahi
# installer puts next to macOS. FEDORA_KERNEL and the file system ids below are that image's own (read from
# its boot.img: loader/entries/*.conf and grub2/grub.cfg); the first start uses them to start that kernel
# (esp/grub-firststart.cfg), and installer_data.json repeats the image's partitions. Change them together.

# The Asahi installer (AsahiLinux/asahi-installer, MIT), the same build alx.sh uses.
ASAHI_INSTALLER_VERSION="v0.9.2"
ASAHI_INSTALLER_URL="https://cdn.asahilinux.org/installer/installer-v0.9.2.tar.gz"
ASAHI_INSTALLER_SHA256="484d999e53b4aef27d17c4478e49567e96842f8a6cb35c073a0df5bf0bbe8276"

# Fedora Asahi Remix 44 Minimal, built 4 September 2026 (its current release; supports M1, M2 and M3).
FEDORA_PACKAGE="fedora-44-minimal-202609041600.zip"
FEDORA_PACKAGE_URL="https://fedora-asahi-remix.org/os/fedora-44-minimal-202609041600.zip"
FEDORA_PACKAGE_SHA256="610a953148a9409c10f9e5c1353426729b77c4ac2a6991c213ec7a1281720dd1"
FEDORA_PACKAGE_BYTES=1117242057
FEDORA_KERNEL="7.1.6-400.asahi.fc44.aarch64+16k"
FEDORA_BOOT_UUID="65e9d770-9bc9-4b3f-9877-10347fb284e5"
FEDORA_ROOT_UUID="9db8788b-4718-404e-a6d0-1c9694d398fd"

