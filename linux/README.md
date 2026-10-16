# ClaudeOS — the thin Linux base

This directory is the **invisible plumbing** that turns a stock Ubuntu Server
24.04 install into ClaudeOS: power on → GRUB menu (*ClaudeOS* default, *Windows*)
→ straight into one full-screen voice session. No desktop, no display manager,
no login prompt, no browser, no SSH. Windows stays installed, untouched.

See `../ARCHITECTURE.md` for the binding contract. This README is the operator's
quick reference.

## What gets installed, and why

`install-claudeos.sh` (run by the autoinstaller via
`curtin in-target -- bash .../install-claudeos.sh --user <user> --from-installer`)
sets up:

| Area | What | Why |
|---|---|---|
| Display | `xserver-xorg-core`, `xserver-xorg-legacy`, `xinit`, `x11-xserver-utils`, `openbox`, `xdotool`, `wmctrl`, `unclutter-xfixes` | Xorg with **no desktop**; rootless X started from the tty1 autologin; openbox gives borderless, maximized windows and no root menu; cursor hidden when idle. |
| ClaudeOS screen | `python3-gi`, `gir1.2-webkit2-4.1`, `gir1.2-gtk-3.0`, `gstreamer1.0-plugins-good/-bad`, `gstreamer1.0-libav` | pywebview + WebKitGTK 4.1 render the ClaudeOS UI and web media. |
| Audio + Bluetooth | `pipewire`, `pipewire-pulse`, `pipewire-alsa`, `wireplumber`, `libspa-0.2-bluetooth`, `bluez`, `dbus-user-session` | AirPods over PipeWire; WirePlumber auto-switches to the headset mic profile when ClaudeOS listens; mSBC wideband speech enabled. |
| Networking | `network-manager`, `wpasupplicant` | NM manages **all** devices (netplan renderer = NetworkManager); no admin needed. |
| Windows files | `ntfs-3g`, `cryptsetup-bin`, `efibootmgr` | Mount the Windows partition (rw if clean, ro if dirty, never touching the hiberfile); a BitLocker drive is opened with ClaudeOS's own key (cryptsetup's BITLK, the key on stdin); UEFI BootNext to switch to Windows. |
| Voice I/O | `libportaudio2`, `espeak-ng` | PortAudio for capture; espeak-ng as a TTS fallback. |
| Fonts | `fonts-noto-core`, `fonts-noto-color-emoji`, `fonts-dejavu-core` | UI text + emoji. |
| Security | `sudo`, `ufw`, `unattended-upgrades`, `ca-certificates`, `curl` | Root helper reached via `sudo -n`; firewall default-deny inbound; security-only auto-updates; TLS trust. |
| Boot UX | `plymouth`, `plymouth-label` | Boot splash: "ClaudeOS" in black on white (a Plymouth script theme). |
| Recovery | `xterm` | The recovery screen shown if ClaudeOS keeps failing to start. |

GPU drivers: `ubuntu-drivers install` runs **only if** an NVIDIA GPU is detected
(`lspci`); AMD/Intel use the in-kernel driver. The step is guarded so a failure
never aborts the install.

The Python runtime lives in `/opt/claudeos/venv` (`--system-site-packages`, so
it can import the distro's `python3-gi`/WebKit typelibs) and installs
`../requirements-linux.txt` (provided by the runtime engineer). ClaudeOS itself
isn't pip-installed: `claudeos-app.pth` in the venv's site-packages puts
`/opt/claudeos/app` on the import path, so `python -m claudeos` works from any
folder (the session starts it from the home folder). The installer checks this
and stops if it doesn't work.

### Flags

- `--user NAME` (required) — the ClaudeOS user (created by subiquity's identity
  section during install, before late-commands run).
- `--from-installer` — running in the target chroot: only `systemctl enable`
  (never start), no interactive prompts.
- `--dry-run` — print every action, change nothing.
- `--skip-models` — don't download voice models (done on first boot as the user).

## Layout

```
install-claudeos.sh        Idempotent installer (apt, venv, autologin, configs, services, GRUB, firewall).
system/                    Files copied verbatim into the target root (/etc, templated @CLAUDEOS_USER@).
session/                   claudeos-session (exit-code supervisor + recovery), openbox config, bash_profile, xinitrc.
grub/07_claudeos           Boot menu: "ClaudeOS" (default) + "Windows" (chainload bootmgfw.efi).
root-helper/claudeos-root  The ONLY privileged entry point (sudo -n), strict arg validation, JSON out.
root-helper/bt-import-windows-keys.py  Copy Bluetooth Classic link keys from the Windows SYSTEM hive into BlueZ.
firstboot/                 The first start after the installer (the installer's finish.sh enables both):
                           claudeos-finish-install: finishes install-claudeos.sh if the installer couldn't;
                           claudeos-reclaim-installer: deletes the installer's temporary partition, grows
                           ClaudeOS into its space, removes \EFI\claudeos-setup and its power-on entry.
tests/                     Test suite (see below).
```

## The session, in one breath

tty1 autologin → `~/.bash_profile` execs `startx` (tty1, no DISPLAY) → `~/.xinitrc`
execs `session/claudeos-session`, which disables DPMS/blanking, starts openbox +
`unclutter`, then **supervises** `python -m claudeos run`:

- exit `0` → restart after 1 s
- exit `3` → restart immediately (e.g. after an update)
- other → crash; backoff 1, 2, 5 s; **3 crashes in 120 s → recovery screen**
  (last log lines + `claudeos doctor`, Enter retries).

Logs: `~/ClaudeOS/logs/session.log` (size-rotated, keeps `.1`/`.2`).
Escape hatch: **Ctrl+Alt+Backspace** restarts ClaudeOS (ends X → getty respawns
→ startx again).

## Root helper verbs

`sudo -n /usr/local/libexec/claudeos/claudeos-root <verb>` →
`status`, `boot-windows-next`, `mount-windows`, `apt-install PKG…`,
`apt-remove PKG…`, `system-update`, `bt-import-windows-keys`,
`set-timezone ZONE`, `wipe-install-secrets`. Every verb validates its arguments,
emits one JSON object on stdout, and never interpolates caller input into a
shell. ClaudeOS asks the user (spoken yes/no) before any verb except `status`.

## Re-running / repair

The installer is **idempotent** — safe to run again by hand on the target:

```
sudo bash /opt/claudeos/app/linux/install-claudeos.sh --user <user>
```

Without `--from-installer` it also starts/enables services. Use `--dry-run`
first to preview. Re-running fixes drifted configs, re-validates the sudoers
file, re-applies the BlueZ block (exactly once), and regenerates GRUB.

## Recovery

- **Boot loops / ClaudeOS won't start:** the recovery screen appears
  automatically after 3 crashes in 120 s; it shows the last log and
  `claudeos doctor`. Press Enter to retry.
- **Get a shell:** switch to another VT (Ctrl+Alt+F2…F6) — only tty1 autologs
  into ClaudeOS; the others keep a normal password login.
- **Boot Windows:** pick *Windows* in the GRUB menu, or let ClaudeOS use UEFI
  BootNext (`boot-windows-next`).
- **AirPods won't reconnect after a Windows↔ClaudeOS switch:** run
  `bt-import-windows-keys` (copies the pairing from Windows). As a last resort
  an admin can set `ControllerMode = bredr` in `/etc/bluetooth/main.conf`
  (see `system/etc/bluetooth/main.conf.d/90-claudeos-bredr-fallback.conf.disabled`).
- **BitLocker-encrypted Windows partition:** ClaudeOS Setup gives ClaudeOS a
  recovery password of its own for it, which the installer puts in
  `/etc/claudeos/bitlocker-recovery-key` (root, 0600). Without that file (or if
  the key was deleted in Windows), ClaudeOS doesn't mount Windows; any recovery
  password for that drive placed there works too.

## Tests

```
sudo bash tests/run.sh          # full suite incl. the real chroot install (needs root + debootstrap)
bash tests/run.sh               # everything except the chroot test (auto-skips it)
```

`run.sh` shellchecks every script, syntax-checks the Python, and runs:
`test_grub_07_claudeos.sh`, `test_root_helper.sh`, `test_session_supervisor.sh`,
`test_finish_install.sh`, `test_reclaim_installer.sh` (a pretend disk in a fake `/sys`)
(under Xvfb), `test_bt_import.sh` (synthetic SYSTEM hive → BlueZ `info`), and
`test_install_chroot.sh` (debootstrap a noble chroot, install for real, verify,
prove idempotency). It finds or builds a Python with `regipy` for the
hive-parsing tests.
