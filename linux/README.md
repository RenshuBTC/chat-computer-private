# Chat Computer — the thin Linux base

This directory is the **invisible plumbing** that turns a stock Ubuntu Server
24.04 install into Chat Computer: power on → GRUB menu (*Chat Computer* default, *Windows*)
→ straight into one full-screen voice session. No desktop, no display manager,
no login prompt, no browser, no SSH. Windows stays installed, untouched.

See `../ARCHITECTURE.md` for the binding contract. This README is the operator's
quick reference.

## What gets installed, and why

`install-chatcomputer.sh` (run by the autoinstaller via
`curtin in-target -- bash .../install-chatcomputer.sh --user <user> --from-installer`)
sets up:

| Area | What | Why |
|---|---|---|
| Display | `xserver-xorg-core`, `xserver-xorg-legacy`, `xinit`, `x11-xserver-utils`, `openbox`, `xdotool`, `wmctrl`, `unclutter-xfixes` | Xorg with **no desktop**; rootless X started from the tty1 autologin; openbox gives borderless, maximized windows and no root menu; a mouse's arrow hidden once the mouse is still for 3 s. |
| Chat Computer screen | `python3-gi`, `python3-gi-cairo`, `gir1.2-webkit2-4.1`, `gir1.2-gtk-3.0`, `gstreamer1.0-plugins-good/-bad`, `gstreamer1.0-libav` | GTK 3 + WebKitGTK 4.1 render the Chat Computer UI and web media; cairo draws the onscreen keyboard. |
| Audio + Bluetooth | `pipewire`, `pipewire-pulse`, `pipewire-alsa`, `wireplumber`, `libspa-0.2-bluetooth`, `bluez`, `dbus-user-session`, `bolt` | AirPods over PipeWire; WirePlumber auto-switches to the headset mic profile when Chat Computer listens; mSBC wideband speech enabled. `bolt` lets in Thunderbolt devices, such as an Apple Studio Display's camera, microphones and speakers. |
| Networking | `network-manager`, `wpasupplicant` | NM manages **all** devices (netplan renderer = NetworkManager); no admin needed. |
| Windows files | `ntfs-3g`, `cryptsetup-bin`, `efibootmgr` | Mount the Windows partition (rw if clean, ro if dirty, never touching the hiberfile); a BitLocker drive is opened with Chat Computer's own key (cryptsetup's BITLK, the key on stdin); UEFI BootNext to switch to Windows. |
| Voice I/O | `libportaudio2`, `espeak-ng` | PortAudio for capture; espeak-ng as a TTS fallback. |
| Fonts | `fonts-noto-core`, `fonts-noto-cjk`, `fonts-noto-color-emoji`, `fonts-dejavu-core` | The characters Chat Computer's own font lacks (CJK, Greek, Cyrillic) and emoji. Chat Computer brings Atkinson Hyperlegible Next and Mono itself (`chatcomputer/ui/assets/fonts`). |
| Security | `sudo`, `ufw`, `unattended-upgrades`, `ca-certificates`, `curl` | Root helper reached via `sudo -n`; firewall default-deny inbound; security-only auto-updates; TLS trust. |
| Boot UX | `plymouth`, `plymouth-label` | Boot splash: "Chat Computer" in black on white (a Plymouth script theme). |
| Recovery | `xterm` | The recovery screen shown if Chat Computer keeps failing to start. |

GPU drivers: `ubuntu-drivers install` runs **only if** an NVIDIA GPU is detected
(`lspci`); AMD/Intel use the in-kernel driver. The step is guarded so a failure
never aborts the install.

The Python runtime lives in `/opt/chatcomputer/venv` (`--system-site-packages`, so
it can import the distro's `python3-gi`/WebKit typelibs) and installs
`../requirements-linux.txt` (provided by the runtime engineer). Chat Computer itself
isn't pip-installed: `chatcomputer-app.pth` in the venv's site-packages puts
`/opt/chatcomputer/app` on the import path, so `python -m chatcomputer` works from any
folder (the session starts it from the home folder). The installer checks this
and stops if it doesn't work.

### Flags

- `--user NAME` (required) — the Chat Computer user (created by subiquity's identity
  section during install, before late-commands run: so in Ubuntu 24.04.4 and
  later, the only ones Chat Computer Setup uses; up to 24.04.3 cloud-init made it at
  the first start).
- `--from-installer` — running in the target chroot: only `systemctl enable`
  (never start), no interactive prompts.
- `--dry-run` — print every action, change nothing.
- `--skip-models` — don't download voice models (done on first boot as the user).
- `--distro ubuntu|fedora` — which system it installs on (default: from `/etc/os-release`).

### On a Mac: Fedora Asahi Remix

On an Apple silicon Mac the same script runs on Fedora Asahi Remix 44 Minimal (aarch64), started by
`chatcomputer-finish-install.service` after Chat Computer's first start (`../mac/esp/firstboot.sh`). The differences:

| Area | Fedora | Why |
|---|---|---|
| Packages | the same set by Fedora's names (`xorg-x11-server-Xorg`, `python3-gobject`, `webkit2gtk4.1`, `pipewire-pulseaudio`, ...), `dnf install --setopt=install_weak_deps=False` | Nothing beyond what Chat Computer needs; the Asahi image already has the Mac's own parts. All 41 names checked against Fedora 44's aarch64 repositories; `gobject-introspection` is listed because GTK needs its cairo typelib and weak dependencies are off. |
| The Mac | `asahi-bless`, `exfatprogs` | "Switch to macOS" chooses the start-up volume; the drive macOS shares is exFAT. |
| Audio | `51-chatcomputer-bluetooth.conf` (WirePlumber 0.5, SPA-JSON) | Fedora's WirePlumber reads SPA-JSON, not Lua. |
| Files | fstab line for the Chat Computer drive at `~/Mac` (`PARTUUID` from `/etc/chatcomputer/mac-files-partuuid`) | Folders sent over from macOS. No Windows mount, no netplan, no Xwrapper (logind gives Xorg the devices). |
| Security | firewalld without `ssh` and `cockpit` (`firewall-offline-cmd`), `dnf5-automatic.timer` with `upgrade_type = security`, `sshd` off | Same contract as ufw/unattended-upgrades on Ubuntu. |
| Boot | `GRUB_TIMEOUT=0`, hidden menu, `quiet rhgb`, `plymouth-set-default-theme chatcomputer -R` | The Mac picks the system itself (startup options), so GRUB shows nothing. |
| SELinux | `restorecon -RF` over everything written | Fedora enforces SELinux labels. |
| Python | Fedora 44's Python is 3.14; `kokoro-onnx` (the voice) is installed after the rest with `--ignore-requires-python` | It declares Python older than 3.14 but is plain Python; Chat Computer's whole test suite passes on 3.14, speaking and hearing included. Every other package has ARM wheels for 3.14. |
| Which system | `ID`/`ID_LIKE` in `/etc/os-release` (Fedora Asahi Remix is `ID=fedora-asahi-remix`, `ID_LIKE=fedora`) | |

## Layout

```
install-chatcomputer.sh    Idempotent installer (apt, venv, autologin, configs, services, GRUB, firewall).
system/                    Files copied verbatim into the target root (/etc, templated @CHATCOMPUTER_USER@).
session/                   chatcomputer-session (exit-code supervisor + recovery), openbox config, bash_profile, xinitrc.
grub/07_chatcomputer       Boot menu: "Chat Computer" (default) + "Windows" (chainload bootmgfw.efi).
root-helper/chatcomputer-root  The ONLY privileged entry point (sudo -n), strict arg validation, JSON out.
root-helper/bt-import-windows-keys.py  Copy Bluetooth Classic link keys from the Windows SYSTEM hive into BlueZ.
firstboot/                 The first start after the installer (the installer's finish.sh enables both):
                           chatcomputer-finish-install: finishes install-chatcomputer.sh if the installer couldn't;
                           chatcomputer-reclaim-installer: deletes the installer's temporary partition, grows
                           Chat Computer into its space, removes \EFI\chatcomputer-setup and its power-on entry.
tests/                     Test suite (see below).
```

## The session, in one breath

tty1 autologin → `~/.bash_profile` execs `startx` (tty1, no DISPLAY) → `~/.xinitrc`
execs `session/chatcomputer-session`, which disables DPMS/blanking, starts openbox +
`unclutter`, then **supervises** `python -m chatcomputer run`:

- exit `0` → restart after 1 s
- exit `3` → restart immediately (e.g. after an update)
- other → crash; backoff 1, 2, 5 s; **3 crashes in 120 s → recovery screen**
  (last log lines + `chatcomputer doctor`, Enter retries).

Logs: `~/ChatComputer/logs/session.log` (size-rotated, keeps `.1`/`.2`).
Escape hatch: **Ctrl+Alt+Backspace** restarts Chat Computer (ends X → getty respawns
→ startx again).

## Root helper verbs

`sudo -n /usr/local/libexec/chatcomputer/chatcomputer-root <verb>` →
`status`, `boot-windows-next`, `boot-default windows|chatcomputer`, `mount-windows`, `apt-install PKG…`,
`apt-remove PKG…`, `system-update`, `bt-import-windows-keys`,
`set-timezone ZONE`, `wipe-install-secrets`, `update-app`, `update-done`, `update-undo`; on a Mac also
`boot-macos-next` and `boot-default macos|chatcomputer` (asahi-bless), and the package verbs use dnf. Every verb
validates its arguments, emits one JSON object on stdout, and never interpolates caller input into a
shell. Chat Computer asks the user (yes/no) before any verb except `status`, `update-done` and `update-undo` (see
"Updating Chat Computer" in ARCHITECTURE.md).

## Re-running / repair

The installer is **idempotent** — safe to run again by hand on the target:

```
sudo bash /opt/chatcomputer/app/linux/install-chatcomputer.sh --user <user>
```

Without `--from-installer` it also starts/enables services. Use `--dry-run`
first to preview. Re-running fixes drifted configs, re-validates the sudoers
file, re-applies the BlueZ block (exactly once), and regenerates GRUB.

## Recovery

- **Boot loops / Chat Computer won't start:** the recovery screen appears
  automatically after 3 crashes in 120 s; it shows the last log and
  `chatcomputer doctor`. Press Enter to retry.
- **Get a shell:** switch to another VT (Ctrl+Alt+F2…F6) — only tty1 autologs
  into Chat Computer; the others keep a normal password login.
- **Boot Windows:** pick *Windows* in the GRUB menu, or let Chat Computer use UEFI
  BootNext (`boot-windows-next`).
- **AirPods won't reconnect after a Windows↔Chat Computer switch:** run
  `bt-import-windows-keys` (copies the pairing from Windows). As a last resort
  an admin can set `ControllerMode = bredr` in `/etc/bluetooth/main.conf`
  (see `system/etc/bluetooth/main.conf.d/90-chatcomputer-bredr-fallback.conf.disabled`).
- **BitLocker-encrypted Windows partition:** Chat Computer Setup gives Chat Computer a
  recovery password of its own for it, which the installer puts in
  `/etc/chatcomputer/bitlocker-recovery-key` (root, 0600). Without that file (or if
  the key was deleted in Windows), Chat Computer doesn't mount Windows; any recovery
  password for that drive placed there works too.

## Tests

```
sudo bash tests/run.sh          # full suite incl. the real chroot install (needs root + debootstrap)
bash tests/run.sh               # everything except the chroot test (auto-skips it)
```

`run.sh` shellchecks every script, syntax-checks the Python, and runs:
`test_grub_07_chatcomputer.sh`, `test_root_helper.sh`, `test_session_supervisor.sh`,
`test_finish_install.sh`, `test_reclaim_installer.sh` (a pretend disk in a fake `/sys`),
`test_install_dryrun.sh` (`--dry-run` for Ubuntu and for Fedora)
(under Xvfb), `test_bt_import.sh` (synthetic SYSTEM hive → BlueZ `info`), and
`test_install_chroot.sh` (debootstrap a noble chroot, install for real, verify,
prove idempotency). It finds or builds a Python with `regipy` for the
hive-parsing tests.
