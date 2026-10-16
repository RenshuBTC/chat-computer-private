# ClaudeOS for Mac: setup

What a person runs in macOS to put ClaudeOS next to macOS on a Mac with Apple silicon (M1, M2 or M3, not
M3 Ultra). ClaudeOS's Linux base there is Fedora Asahi Remix, installed by the Asahi installer, which setup
downloads, checks and runs. How to use it is in `../README.md` ("ClaudeOS on a Mac"); how it works, in
`../ARCHITECTURE.md` ("Mac").

| File | What it is |
|---|---|
| `setup.sh` | ClaudeOS Setup for Mac (bash 3.2, in Terminal): install, send more files, sign in again, choose what starts, remove |
| `manifest.sh` | What setup downloads, pinned with SHA-256: the Asahi installer (v0.9.2), Fedora Asahi Remix 44 Minimal, the Claude program for signing in; and that image's kernel and file system ids |
| `installer_data.json` | The Asahi installer's list of systems: ClaudeOS, with Fedora's own partitions and ClaudeOS's icon |
| `drive-installer.exp` | Answers the Asahi installer's questions (expect); the person types only their Mac's password |
| `esp/grub-firststart.cfg` | Put in place of Fedora's `EFI/fedora/grub.cfg` for one start: runs `firstboot.sh` instead of Fedora's first-start questions |
| `esp/firstboot.sh` | ClaudeOS's first start on the Mac: the account, Wi-Fi, ClaudeOS itself, then Fedora's usual start back |
| `claudeos.icns`, `make_icon.py` | The icon in the Mac's startup options |

Updating the pinned versions: change `manifest.sh`, `installer_data.json` (the package name and Fedora's
partitions, from the image's own `.json` next to it on fedora-asahi-remix.org) and the kernel and ids together
(read from the image's `boot.img`: `loader/entries/*.conf`, `grub2/grub.cfg`). If the installer changes its
questions, `drive-installer.exp` and `../tests/mac_fake_installer.py` change with it.

Tests: `../tests/test_mac_setup.py` (a pretend Mac; run it with macOS's bash 3.2 through
`CLAUDEOS_TEST_BASH`) and `../tests/mac_ci/run.sh` (on GitHub's Macs, `../.github/workflows/mac.yml`).
Test hooks in `setup.sh`: `CLAUDEOS_MAC_ROOT`, `CLAUDEOS_SETUP_WORK`, `CLAUDEOS_PLAN_ONLY`,
`CLAUDEOS_MANIFEST`, `CLAUDEOS_ENGINE_CLI`, `CLAUDEOS_EXPECT`, `CLAUDEOS_INSTALLER_CMD`.
