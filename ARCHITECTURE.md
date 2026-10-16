# ClaudeOS architecture (internal contract)

Goal: **hardware → thin Linux drivers (invisible) → Claude (user's subscription, over the internet) → the user (AirPods +
screen + phone)**. ClaudeOS is a conversational shell: you talk (AirPods) or type (the Claude app on the phone is the
keyboard), and both are one conversation.
Power on → boot menu ("ClaudeOS" default, "Windows") → straight into the full-screen ClaudeOS conversation.
No login screen, no desktop environment, no display manager, no browser UI. Windows stays installed untouched.
On an Apple silicon Mac (M1, M2, M3) it is the same: a second system next to macOS, on Fedora Asahi Remix instead
of Ubuntu, chosen with the Mac's own start-up volume instead of a boot menu (see "Mac" at the end).

## Repository layout

```
ClaudeOS/
  claudeos/                 Python package: the ClaudeOS runtime (brain, tools, voice, screen)
  linux/                    The thin Linux base (installed into the target system)
    install-claudeos.sh     Runs INSIDE the target (curtin in-target) or later by hand; idempotent
    system/                 Files copied into the target root (/etc, /usr/local/...)
    session/                tty1 autologin -> startx -> session script -> ClaudeOS
    grub/                   /etc/grub.d/07_claudeos (boot menu entries)
    root-helper/claudeos-root   The only privileged entry point ClaudeOS may use (via sudo -n)
    tests/
  windows/                  One-time setup tool run on Windows (PowerShell)
    "ClaudeOS Setup.cmd"    Double-click entry point (self-elevates)
    setup.ps1, lib/*.ps1
    autoinstall.template.yaml
    tests/
  mac/                      One-time setup run in macOS (bash 3.2, in Terminal)
    setup.sh                Install, send files, sign in again, choose what starts, remove
    manifest.sh             What setup downloads, pinned with SHA-256: Asahi installer, Fedora's image
    installer_data.json     The Asahi installer's OS list: ClaudeOS (Fedora Asahi Remix 44 Minimal)
    drive-installer.exp     Answers the Asahi installer's questions (expect)
    esp/                    ClaudeOS's first start on the Mac: grub-firststart.cfg, firstboot.sh
    claudeos.icns           Icon in the Mac's startup options (make_icon.py)
  "ClaudeOS Setup.command"  Double-click entry point for Mac setup (runs mac/setup.sh)
  requirements-linux.txt    Pinned Python deps for the Linux runtime
  tests/                    Python tests for the runtime (+ mock Claude API)
```

## Base system

* Ubuntu Server 24.04 LTS (amd64), installed unattended by the official installer (subiquity autoinstall).
* Installed **alongside Windows** in free disk space (`storage.layout.mode: use_gap`) or on a separate whole disk
  the user picked (matched by serial). Never touches existing partitions. Reuses the existing EFI System Partition.
* Packages are the minimum for: display (Xorg, no desktop), Wi-Fi/Ethernet (NetworkManager), Bluetooth + audio
  (BlueZ, PipeWire, WirePlumber, libspa-0.2-bluetooth), web rendering for ClaudeOS's own screen (WebKitGTK 4.1 via
  python3-gi + gir1.2-webkit2-4.1, GStreamer codecs), screen control (xdotool, wmctrl), Windows files (ntfs-3g,
  cryptsetup-bin for BitLocker), boot control (efibootmgr), fonts, Python 3.12 venv.
* On a Mac: Fedora Asahi Remix 44 Minimal (aarch64, 16K pages), installed by the official Asahi installer, and the
  same set by Fedora's names (`install-claudeos.sh --distro fedora`, chosen from `/etc/os-release`), plus
  `asahi-bless` (start-up volume) and `exfatprogs` (the drive macOS shares). See "Mac".

## Installed paths and users

| What | Path / value |
|---|---|
| Runtime code (root-owned, read-only to the user) | `/opt/claudeos/app` (copy of this repo) |
| Python venv (`--system-site-packages`, for python3-gi) | `/opt/claudeos/venv`; `claudeos-app.pth` in its site-packages puts `/opt/claudeos/app` on the import path |
| Start command | `/opt/claudeos/venv/bin/python -m claudeos run` with `CLAUDEOS_ENGINE=linux` |
| Model download command (run as the user) | `/opt/claudeos/venv/bin/python -m claudeos models --download` |
| Health check command | `/opt/claudeos/venv/bin/python -m claudeos doctor` (prints JSON, exit 0 if usable) |
| User data (owned by the user) | `/home/<user>/ClaudeOS/` |
| Claude subscription token | `/home/<user>/ClaudeOS/secrets/claude_token` (mode 0600, single line `sk-ant-oat01-...`) |
| Initial settings from Windows setup | `/home/<user>/ClaudeOS/settings.json` (JSON; keys below) |
| Voice models | `/home/<user>/ClaudeOS/models/` (layout owned by `claudeos.voice.models`) |
| Windows partition | `/mnt/windows` (rw when NTFS is clean, else ro). A BitLocker drive is opened with ClaudeOS's own recovery password (`/etc/claudeos/bitlocker-recovery-key`, from setup) as `/dev/mapper/claudeos-windows` |
| Root helper | `/usr/local/libexec/claudeos/claudeos-root`, sudoers: `<user> ALL=(root) NOPASSWD: /usr/local/libexec/claudeos/claudeos-root` |

`settings.json` keys written by the Windows setup (all optional): `user_name` (display name), `windows_user`
(Windows profile folder name, e.g. `rensh`), `timezone` (IANA), `locale`.

### ClaudeOS exit codes (session supervisor contract)

* `0` normal exit → session script restarts ClaudeOS after 1 s.
* `3` restart requested (e.g. after an update) → restart immediately.
* other → crash; restart with backoff (1, 2, 5 s). 3 crashes within 120 s → show a recovery screen
  (full-screen `xterm` with the last log lines and `claudeos doctor` output; Enter retries). If it can't be
  shown, the session waits a minute before trying again rather than restarting in a tight loop.

### Drawing pages

WebKit draws with the graphics chip when the system offers one for drawing (a `/dev/dri/renderD*` device).
Without one (a virtual machine, or graphics the kernel can't drive yet) its GPU drawing crashes and leaves a
blank screen, so ClaudeOS sets `WEBKIT_SKIA_ENABLE_CPU_RENDERING=1` and `WEBKIT_DISABLE_DMABUF_RENDERER=1`
before the window opens and draws with the processor. With NVIDIA's own driver it keeps the graphics chip but
sets `WEBKIT_DISABLE_DMABUF_RENDERER=1`, a common workaround for blank WebKit windows on that driver. If a page's
renderer process stops anyway, ClaudeOS switches to the processor, loads the page again and puts the
conversation back as it was.

### Session

tty1 autologin (systemd getty override) → `~/.bash_profile` execs `startx` only on tty1 with no DISPLAY →
`~/.xinitrc` execs `/opt/claudeos/app/linux/session/claudeos-session` which: disables screen blanking/DPMS,
sets a black root window, starts `openbox` (no decorations, all windows maximized/fullscreen),
`unclutter-xfixes` (hide idle cursor), then supervises ClaudeOS per the exit-code contract
(`python -P -m claudeos run`: `-P` keeps the home folder off the import path, so a `~/json.py` can't replace
part of Python). agetty runs with `--noissue` and the user has an empty `~/.hushlogin`, so no Ubuntu banner or
welcome text shows between the splash and ClaudeOS.
PipeWire/WirePlumber run as systemd user services (dbus-user-session). Logs: `~/ClaudeOS/logs/`.

### Root helper verbs (stdout = one JSON object, exit code 0 = success)

| Verb | Effect |
|---|---|
| `status` | `{"windows_mounted":bool,"windows_rw":bool,"windows_bitlocker":bool,"windows_boot_entry":bool,"efi":bool}` |
| `boot-windows-next` | One-time boot into Windows on next reboot: `efibootmgr --bootnext <Windows Boot Manager>`; fallback `grub-reboot "Windows"` |
| `boot-default windows\|claudeos` | What starts when the PC turns on: that system's UEFI entry first in `BootOrder` (Windows Boot Manager, or `ubuntu` for ClaudeOS), the other entries kept in order; nothing removed |
| `mount-windows` | (Re)mount the Windows partition at /mnt/windows (rw if clean else ro; never `remove_hiberfile`) |
| `apt-install PKG...` | `apt-get install -y` validated package names (`^[a-z0-9][a-z0-9+.-]{1,62}$`) |
| `apt-remove PKG...` | same validation |
| `system-update` | `apt-get update && apt-get -y upgrade` |
| `boot-macos-next` | Mac: macOS for the next start only (`asahi-bless --set-boot-macos --next`; if macOS was renamed, the one start-up volume that isn't ClaudeOS) |
| `boot-default macos\|claudeos` | Mac: the start-up volume (`asahi-bless --set-boot-macos`, or `--set-boot ClaudeOS`) |
| `bt-import-windows-keys` | Copy Bluetooth Classic link keys (e.g. AirPods) from the Windows SYSTEM registry hive into BlueZ, restart bluetooth |
| `set-timezone ZONE` | validated against /usr/share/zoneinfo |
| `wipe-install-secrets` | Remove `/claudeos/secrets` from a ClaudeOS installer (FAT32 labelled `CLAUDEOS`) still on the PC or plugged in |

On Fedora (a Mac), `apt-install`, `apt-remove` and `system-update` use `dnf` (`install -y
--setopt=install_weak_deps=False`, `remove -y`, `upgrade -y --refresh`). ClaudeOS asks the user (spoken yes/no) before calling any verb except `status`.

### Boot menu

`/etc/grub.d/07_claudeos` generates, before the stock entries: `ClaudeOS` (default) and `Windows`
(chainload `/EFI/Microsoft/Boot/bootmgfw.efi`). `GRUB_DEFAULT=saved`, saved default = `ClaudeOS`, timeout 3 s,
menu visible. Stock Ubuntu entries remain under "Advanced options" for recovery. os-prober disabled (our entry
replaces it). "Switch to Windows" from ClaudeOS uses UEFI BootNext (BitLocker-friendly), not the GRUB chainload.
The ClaudeOS entry boots `/boot/vmlinuz` and `/boot/initrd.img` (the links Ubuntu 24.04 keeps up to date; older
setups' `/vmlinuz` links at the root also work). Without a kernel link it adds neither entry, so GRUB's
fallback, the first entry, is Ubuntu's own (this same system), never Windows.

The boot splash is a Plymouth script theme (`/usr/share/plymouth/themes/claudeos`, chosen with the
`default.plymouth` alternative): a white screen with "ClaudeOS" in black. `systemd-networkd` and its
wait-online service are turned off, since NetworkManager owns the network; otherwise every start would wait
two minutes for a network networkd doesn't manage.

## Windows setup → installer contract

No USB stick: setup puts the installer on the PC's own disk (`windows/lib/Installer.ps1`).

* **Where.** The space ClaudeOS gets (shrunk from C:, a free area, or a separate disk the person erased) also
  holds its installer at first: a FAT32 partition labelled `CLAUDEOS`, 6 GiB (`InstallerBytes`), GPT basic
  data type, without a drive letter, at the *end* of the largest free area on that disk (`Get-FreeExtents`,
  `Get-InstallerPlacement`: whole MiBs, the first and last MiB of the disk left alone). ClaudeOS installs into
  the rest, right before it. Setup refuses ISOs bigger than 5 GiB.
* **What's on it.** The official Ubuntu Server 24.04.x ISO's files (copied from the ISO as Windows mounts it;
  its links, like `ubuntu -> .`, aren't folders there and aren't copied), plus:
  * `/claudeos/installer-<install-id>` (`id=<install-id>`): the mark, written first, so even a half-made
    installer can be found by "Remove ClaudeOS" and, later, by ClaudeOS;
  * `/autoinstall.yaml` (generated from `windows/autoinstall.template.yaml`). The installer's GRUB menu
    becomes "Install ClaudeOS" (HWE kernel, default, 5 s) and "Install ClaudeOS (older kernel)", both with
    `autoinstall subiquity-storage-version=2` on the kernel command line: unattended, and able to see free
    space next to existing partitions (the installer's default storage version 1 sees none on a disk with
    partitions);
  * `/claudeos/app/`: a copy of this repo, without `windows/`, `mac/`, `.venv` or caches;
  * `/claudeos/secrets/claude_token`: the subscription token the user pasted after `claude setup-token`;
  * `/claudeos/settings.json`: initial settings (keys above);
  * `/claudeos/guard.sh`, `network.sh`, `finish.sh`: copied from `windows/installer/`, with LF line endings.
* **How the PC starts it.** The ISO's signed boot loader goes to `\EFI\claudeos-setup\` on the EFI partition
  Windows starts from: `EFI/boot/bootx64.efi` (shim, signed by Microsoft's third-party UEFI CA) as
  `shimx64.efi`, with `grubx64.efi` (signed by Canonical) and `mmx64.efi`. A power-on entry "ClaudeOS setup"
  starts it: `bcdedit /copy {bootmgr} /d "ClaudeOS setup"` (so it points at the same EFI partition; the new
  `{id}` is found by comparing `bcdedit /enum firmware` before and after, never by reading Windows' language),
  `/set {id} path \EFI\claudeos-setup\shimx64.efi`, `/set {fwbootmgr} displayorder {id} /addlast`; and,
  once everything is ready, `/set {fwbootmgr} bootsequence {id}` (UEFI `BootNext`: the next start only). That
  GRUB's built-in configuration finds no `/boot/grub` on the EFI partition, so it looks for the partition with
  `/.disk/info`, the installer partition, and reads its menu from there. casper finds the live system on that
  partition too. Tested in QEMU with OVMF, Secure Boot and Microsoft's keys.
* **Order, and taking it back.** Nothing changes before "Go ahead". Then the power-on entry comes first: if the
  PC won't take one, setup stops with nothing else changed (and deletes a half-made copy). If any later step
  fails, setup takes back what it did (`Undo-InstallSteps`): the entry, the boot loader, the installer partition
  (only if it's still the one it made, same offset and size), and C:'s space; it says what it couldn't.

Storage is always `layout: {name: direct, mode: use_gap, match: {install-media: true}}`: the largest free
area on the disk the installer is on, which is ClaudeOS's space right before the installer partition. The
installer never reformats a disk. Shrinking C: or erasing a separate disk happens on Windows, after "Go
ahead". (subiquity only leaves out disks in use when a match names `size` or `ssd`; `install-media` picks the
disk holding the partition mounted at `/cdrom`.)

* The installer's boot menu checks first: if `/EFI/claudeos/installed-<install-id>` exists on any partition
  (GRUB `search --file`), this installer already installed ClaudeOS here, and the menu offers only
  "Start ClaudeOS (already installed)", which reads the installed system's boot menu
  (`configfile /EFI/ubuntu/grub.cfg` on the EFI partition). It doesn't chainload the installed shim: with
  Secure Boot on, GRUB 2.12 starts it without a file system and it can't find the rest of the boot loader
  (tested). Otherwise the menu offers the two install entries.
* `early-commands`: `sh /cdrom/claudeos/guard.sh <install-id>`, the second check for the same marker, in
  case the installer starts anyway. The installer has no `efibootmgr`, so the guard writes UEFI `BootNext`
  through efivarfs (the entry described `ubuntu`) and restarts; if it can't, it stops the installer with a
  message, and nothing is written.
* `late-commands`: `sh /cdrom/claudeos/finish.sh <user> <install-id>`. It:
  1. copies the app to `/target/opt/claudeos/app` (root-owned, files 0644, `#!` files 0755);
  2. copies the token (0600) and settings into `/target/home/<user>/ClaudeOS/`, owned by the user, and the
     full sign-in for the phone, if setup made one (`secrets/claude_login.json`), to
     `~/ClaudeOS/.engine/.credentials.json` (0600; `.engine` 0700), where Claude Code reads and refreshes it;
  3. writes the marker `/EFI/claudeos/installed-<install-id>` to the ESP: `id=<install-id>` and
     `root=<PARTUUID>` (ClaudeOS's root partition, from `findmnt` + `blkid`), which "Remove ClaudeOS" in
     ClaudeOS Setup uses to delete exactly that partition;
  4. sets up the first start: `/etc/claudeos/install-user`, `/etc/claudeos/install-id`, and two services from
     `linux/firstboot`, enabled by their `multi-user.target.wants` links:
     * `claudeos-finish-install.service`, the safety net: until `/var/lib/claudeos/base-installed` exists, it
       runs before `getty@tty1`, retries `install-claudeos.sh` with growing waits (30 s doubling to 5 min,
       `dpkg --configure -a` before each try), then reboots;
     * `claudeos-reclaim-installer.service`, once (`/var/lib/claudeos/installer-reclaimed`): finds the
       installer partition (vfat, label `CLAUDEOS`, `claudeos/installer-<this install's id>`, neither the root
       partition nor an EFI partition), overwrites the sign-in it carried in place, deletes it (`sfdisk
       --delete`), grows ClaudeOS's partition and file system into the space when it sits right after it
       (`growpart`, `resize2fs`; up to 2 MiB between them counts as right after), deletes
       `\EFI\claudeos-setup` from every EFI partition and the power-on entries that start it
       (`efibootmgr -B`). If deleting fails it tries again at the next start;
  5. runs `curtin in-target -- bash /opt/claudeos/app/linux/install-claudeos.sh --user <user> --from-installer`,
     up to three tries. If all fail (usually no internet), it still exits 0, so the Ubuntu install ends
     normally and the safety net finishes the job on the first start.

"Remove ClaudeOS from this PC" (Windows) also deletes installer partitions with a mark, the "ClaudeOS setup"
entries and `\EFI\claudeos-setup`, so it cleans up an install that never finished too.

`install-claudeos.sh` stops with exit status 1 at the first essential failure (apt packages, the venv,
pip), before later steps depend on it (for example handing networking to NetworkManager). On success it
writes `/var/lib/claudeos/base-installed` and removes the safety net's link.

Re-signing in later: ClaudeOS Setup's "Sign ClaudeOS in to Claude again" writes
`C:\Users\<you>\ClaudeOS\claude_token` and, for the phone, `claude_login.json`. ClaudeOS imports them at
start, before the file index runs, and deletes the Windows copies. Any file named `claude_token`, and the
engine's `.credentials.json`, are unreadable to Claude's tools.

First start: if the voice models are missing, ClaudeOS downloads them in the background (about 600 MB),
retrying with backoff, then starts listening.

## Runtime (claudeos package)

* Brain: Claude Agent SDK (`claude-agent-sdk`, bundles the official Claude Code binary) with the user's
  subscription and an isolated `CLAUDE_CONFIG_DIR` (`~/ClaudeOS/.engine`). Two sign-ins:
  * the one-year token from `claude setup-token`, passed as `CLAUDE_CODE_OAUTH_TOKEN`. It can only make model
    requests;
  * the full sign-in from `claude auth login` (`.engine/.credentials.json`, refreshed by Claude Code itself),
    used when present and `phone_access` is on, because Remote Control needs it. In this mode the token
    variables are blanked and `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` is not set (Remote Control needs
    Claude Code's feature flags); telemetry and error reporting stay off. If it fails with an auth error,
    the brain switches to the token for the rest of the run and asks the last question again.
* Phone (the Claude app, Claude Code's Remote Control): after connecting with the full sign-in, the brain sends
  the SDK control request `{"subtype": "remote_control", "enabled": true, "name": "ClaudeOS (<host>)"}`, the
  same request Claude Code's own hosts send for `/remote-control`. The Python SDK has no public call for it,
  so it goes through `client._query._send_control_request`. The bridge session id from the response is
  kept and offered as `reattach_session_id` next time; a failure becomes a one-time notice on screen.
  One reader task consumes everything the engine emits. A turn that starts without a local question (a
  user message with a human or missing origin, or output with no turn open) is a phone turn: shown as
  the user's words like anything said here, the reply is not spoken, and voice questions are not asked. Turns the engine injects itself
  (task notifications and the like) belong to the PC. Speaking at the PC interrupts a phone turn. A phone
  turn silent for 15 minutes is closed. Local questions carry the "[time · screen: ...]" prefix; the system
  prompt tells Claude that a message without it came from the phone.
* ClaudeOS tools are in-process MCP tools (screen, files, computer control, apps, system, memory, self-made tools).
* Every risky action goes through the permission gate → spoken confirmation ("Should I go ahead?"). During a
  phone turn the confirm callback returns "phone": the action is denied with a message telling Claude to ask
  in its reply. A "yes" typed on the phone (`classify_answer`) then allows exactly that action (keyed by tool
  and input; for Bash, the command) once within 10 minutes; "no" drops the question.
* Screen: one full-screen GTK 3 window (`claudeos/ui/gtk_app.py`) with two WebKitGTK 4.1 views in a stack:
  the conversation (`ui/assets/chat.html`: only the user's words and Claude's words) and the content layer (a
  real browser with a persistent profile in `~/ClaudeOS/web`, plus a GTK caption strip over it). No buttons.
* Look: plain. Black text on white (white on black in dark mode) in the system's standard font (CSS
  `sans-serif` and GTK's default, which is Noto Sans on the Ubuntu base). The user's words are grey and Claude's
  are black, flush left in one column in the middle of the screen, newest at the bottom. No bundled fonts and
  no imitation of any app.
* Size and width: `ui/layout.py` scales the usual desktop size (16 px text, a column at most 48 rem wide) for
  the monitor's physical size (EDID via GDK) and the viewing distance (estimated from the screen size, or set
  by voice), the way display scaling does. Websites get the same zoom. Recomputed when the window, monitor or
  settings change. Settings: `text_size` (0 = auto), `text_scale`, `viewing_distance_cm`, `screen_inches`
  (0 = reported), `web_zoom` (0 = auto).
* Voice: PipeWire/AirPods, Silero VAD, faster-whisper (local), Kokoro TTS (local), barge-in.

## Mac

On a Mac with Apple silicon, ClaudeOS is a second system next to macOS, as on a PC. Only Asahi Linux can start
another system on these Macs, so ClaudeOS's Linux base there is Fedora Asahi Remix (the Asahi project's
flagship distribution), installed by the Asahi installer. That sets the Macs it works on: M1, M2, and M3, M3 Pro,
M3 Max (not M3 Ultra, M4, M5; on M3 for now without graphics acceleration, brightness or other screens, per
Asahi's feature tables). The runtime (`claudeos/`) is the same as on a PC; `claudeos/platform/linux.py` has
`MacPlatform` (chosen when `/proc/device-tree/compatible` starts with `apple,`).

| What | Path / value |
|---|---|
| Image | Fedora Asahi Remix 44 Minimal, unmodified (`mac/manifest.sh`: the package's URL, SHA-256, kernel, file system ids) |
| Installer | Asahi installer v0.9.2 (`cdn.asahilinux.org`, SHA-256 pinned), run locally: `REPO_BASE` is setup's work folder (`os/<package>`), `installer_data.json` is ClaudeOS's (one OS, Fedora's partitions exactly: EFI 500 MiB FAT32 with Fedora's volume id, Boot 1 GiB, Root expanding), `DISTRO=ClaudeOS` |
| Partitions | macOS's container (shrunk), the start-up volume's container (`ClaudeOS`, 2.5 GB, made by the installer), EFI (`EFI - CLAUD`), Boot, Root, then the ClaudeOS drive (exFAT, `ClaudeOS`) in the space left |
| The drive macOS shares | `~/Mac` in ClaudeOS: fstab `PARTUUID=<id> exfat uid,gid,umask=077,nofail`; the id (GPT partition GUID, diskutil's `DiskUUID`) is in `/etc/claudeos/mac-files-partuuid` |
| Sign-in left by "Sign in again" | `.claudeos/` on the ClaudeOS drive; imported at start and deleted, like `C:\Users\<you>\ClaudeOS` on a PC |

### Setup (`mac/setup.sh`, in macOS)

Menu: Install ClaudeOS on this Mac, Send more files to ClaudeOS, Sign ClaudeOS in to Claude again, Choose
what starts when the Mac turns on, Remove ClaudeOS from this Mac. Written for macOS's bash 3.2.

Install:

1. Checks: Apple silicon (`sysctl hw.optional.arm64`, so a Rosetta Terminal counts), the chip
   (`machdep.cpu.brand_string`: M1, M2, M3; M3 Ultra, M4, M5 refused with the reason), not a virtual Mac
   (`kern.hv_vmm_present`, allowed with `CLAUDEOS_PLAN_ONLY=1`), macOS 13.5 or newer, an admin account, no
   ClaudeOS already on the disk (`find_install`), and the internet. On a Mac mini, Studio or Pro it says screens
   must use the HDMI port.
2. The folders to bring (Desktop, Documents, Downloads, Pictures, Movies, Music; sizes from `du -A`, so files
   kept only in iCloud count).
3. The space, as the installer works it out: macOS keeps what it uses plus 38 GB, never less than diskutil's
   `MinimumSizePreferred`, plus 1 GB. ClaudeOS gets 60 GB when there's room (at least 30 GB); the ClaudeOS
   drive gets the folders plus 10 % plus 10 GB. Plan-only stops after this.
4. Sign-in: the official Claude program from the `claude-agent-sdk` macOS wheel (pinned), `claude setup-token`
   in a throwaway config dir, the pasted token read with bracketed paste (it can span lines); optionally
   `claude auth login --claudeai` for the phone.
5. Wi-Fi: the network in use (`networksetup`, `ipconfig getsummary`), its password typed or taken from the
   keychain (`security find-generic-password -D 'AirPort network password'`).
6. "Go ahead?", then: download and check the installer and the image; add `claudeos.icns` to the image's zip
   with the installer's own Python (its new SHA-256 is noted next to it, so a second run doesn't download it
   again; if the icon can't be added, the copy of `installer_data.json` loses its `icon` line); run the
   installer under `expect` (`mac/drive-installer.exp`), which answers each question a second after it
   appears (the installer clears typed-ahead input before each one) and lets the person type their password
   to it. Exit codes: 2 resize failed, 3 not enough space, 4 a question it can't answer, 5 macOS too old, 6
   failed. The partitions it made come from its `installer.log`.
7. The ClaudeOS drive: `diskutil addPartition <Root> ExFAT ClaudeOS 0` (the space left), Spotlight off; the
   folders copied with `rsync -rt` and `COPYFILE_DISABLE=1`, without `.DS_Store`, `*.photoslibrary` or
   extended attributes (no `._` files).
8. The first start, on ClaudeOS's EFI partition (`claudeos/`): `app.tar.gz` (`claudeos/`, `linux/`, the
   requirements and docs; `--no-xattrs`), `firstboot.sh`, `setup.env` (base64 values: account, name,
   recovery password, time zone, install id, the drive's id, Wi-Fi name and password), `secrets/` (token,
   phone sign-in), `settings.json`; Fedora's `EFI/fedora/grub.cfg` kept as `claudeos/fedora-grub.cfg` and
   replaced by `esp/grub-firststart.cfg` with the kernel and ids filled in.
9. The recovery password to `~/Documents/ClaudeOS recovery password.txt`; the downloads deleted; the one step
   Apple requires (choosing ClaudeOS in the startup options and finishing the installer's second step in
   1TR), shown to be photographed; shutdown.

Remove: copies the drive's files to `~/ClaudeOS files` first if asked, typed REMOVE, `bless --setBoot` on
macOS's volume (`--stdinpass`), `diskutil apfs deleteContainer` on the start-up volume's container, `diskutil
eraseVolume free free` on the EFI, Boot, Root and drive partitions, then `diskutil apfs resizeContainer <macOS's>
0`. Choose what starts: `bless --setBoot --device` on ClaudeOS's or macOS's volume. Sign in again: the token
and phone sign-in to the drive's `.claudeos/`.

### First start (`mac/esp/firstboot.sh`)

GRUB starts Fedora's kernel with `systemd.run="/bin/sh /boot/efi/claudeos/firstboot.sh"` (single-quoted, so
it's one argument) and `systemd.run_success_action=reboot`, `systemd.run_failure_action=reboot`, instead of
Fedora's first-start questions. firstboot.sh (POSIX sh, as root, SELinux enforcing): hostname `claudeos`;
the account (`useradd --groups wheel`, its password the recovery password, through `chpasswd`); time zone and
`LANG`; a NetworkManager keyfile for the Wi-Fi (0600); ClaudeOS to `/opt/claudeos/app` (root-owned); the token,
phone sign-in and settings into `~/ClaudeOS`; `/etc/claudeos/install-user`, `install-id`,
`mac-files-partuuid`; `claudeos-finish-install.service` enabled; `restorecon` on all of it. Only then: Fedora's
first-start questions off (`/etc/reconfigSys`, `initial-setup.service`), Fedora's `grub.cfg` back, and the
sign-in copies deleted from the EFI partition (FAT: readable by anyone who starts the Mac). On failure it counts
tries in `claudeos/tries`; at the third it puts Fedora's start back, so the Mac never loops.

The next start is Fedora's usual one: `claudeos-finish-install.service` runs `install-claudeos.sh --user <user>
--from-installer` with the internet until it works (as on a PC), then restarts. On Fedora that script installs
with `dnf --setopt=install_weak_deps=False`, WirePlumber 0.5 settings (SPA-JSON), firewalld without the ssh and
cockpit services, `dnf5-automatic` for security updates, no SSH server, the Plymouth theme
(`plymouth-set-default-theme -R`), GRUB without its menu (`GRUB_TIMEOUT=0`, `quiet rhgb`), the drive in fstab,
and `restorecon` at the end.

### Switching

* "Switch to macOS": `power(switch_to_macos)` → root helper `boot-macos-next`, then `systemctl reboot`. The
  next start after that is ClaudeOS again.
* `start_by_default` (claudeos | other): root helper `boot-default claudeos|macos`.
* From macOS: setup's "Choose what starts", or the startup options (hold the power button).

### Tests

* `tests/test_mac_setup.py`: setup on a pretend Mac (stand-ins for `diskutil`, `plutil` and the rest in
  `tests/mac_pretend.py`), the real `drive-installer.exp` against `tests/mac_fake_installer.py` (the v0.9.2
  installer's questions, colours and terminal handling), first start against a pretend Fedora, and the pinned
  values agreeing with each other. Run with macOS's bash 3.2 (`CLAUDEOS_TEST_BASH`).
* `tests/mac_ci/run.sh` on GitHub's Macs (`.github/workflows/mac.yml`): plan-only on the real disk; a disk
  image laid out with the installer's own commands for finding, the drive, copying, the first start and
  removing; the real downloads; the real installer until it turns down a virtual Mac.
* `tests/test_mac.py`: `MacPlatform` and the runtime on a Mac; `linux/tests/test_install_dryrun.sh`: the Fedora
  install steps.

Not yet tried on a real Mac. Unknowns: AirPods' microphone (Bluetooth HFP) under Asahi; AirPods paired in
ClaudeOS may need pairing again in macOS (the same Mac, a new key); ClaudeOS's Python wheels and the Claude
program on 16K pages.
