# Chat Computer for Mac: setup

What a person runs in macOS to put Chat Computer next to macOS on a Mac with Apple silicon (M1, M2 or M3, not
M3 Ultra). Chat Computer's Linux base there is Fedora Asahi Remix, installed by the Asahi installer, which setup
downloads, checks and runs. How to use it is in `../README.md` ("Chat Computer on a Mac"); how it works, in
`../ARCHITECTURE.md` ("Mac").

| File | What it is |
|---|---|
| `setup.sh` | Chat Computer Setup for Mac (bash 3.2, in Terminal): install, send more files, choose what starts, remove |
| `manifest.sh` | What setup downloads, pinned with SHA-256: the Asahi installer (v0.9.2) and Fedora Asahi Remix 44 Minimal; and that image's kernel and file system ids |
| `installer_data.json` | The Asahi installer's list of systems: Chat Computer, with Fedora's own partitions and Chat Computer's icon |
| `drive-installer.exp` | Answers the Asahi installer's questions (expect); the person types only their Mac's password |
| `esp/grub-firststart.cfg` | Put in place of Fedora's `EFI/fedora/grub.cfg` for one start: runs `firstboot.sh` instead of Fedora's first-start questions |
| `esp/firstboot.sh` | Chat Computer's first start on the Mac: Fedora's SSH and cockpit masked, the account, Wi-Fi, Chat Computer itself, then Fedora's usual start back |
| `chatcomputer.icns`, `make_icon.py` | The icon in the Mac's startup options |

Updating the pinned versions: change `manifest.sh`, `installer_data.json` (the package name and Fedora's
partitions, from the image's own `.json` next to it on fedora-asahi-remix.org) and the kernel and ids together
(read from the image's `boot.img`: `loader/entries/*.conf`, `grub2/grub.cfg`). If the installer changes its
questions, `drive-installer.exp` and `../tests/mac_fake_installer.py` change with it.

Tests: `../tests/test_mac_setup.py` (a pretend Mac; run it with macOS's bash 3.2 through
`CHATCOMPUTER_TEST_BASH`) and `../tests/mac_ci/run.sh` (on GitHub's Macs, `../.github/workflows/mac.yml`).
Test hooks in `setup.sh`: `CHATCOMPUTER_MAC_ROOT`, `CHATCOMPUTER_SETUP_WORK`, `CHATCOMPUTER_PLAN_ONLY`,
`CHATCOMPUTER_MANIFEST`, `CHATCOMPUTER_ENGINE_CLI`, `CHATCOMPUTER_EXPECT`, `CHATCOMPUTER_INSTALLER_CMD`.
