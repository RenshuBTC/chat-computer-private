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
    airtouch/               Hands seen by the screen's camera: camera, model reader + numpy engine, hand
                            tracking, gestures, keyboard layout, pointer (its own process)
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
  os/                       ClaudeOS's own operating system, from scratch (Rust, no outside code); see os/README.md
    boot/                   UEFI start-up stage (BOOTX64.EFI) that carries the kernel
    kernel/                 The kernel: memory, interrupts, timer, PCI, USB (xHCI, hubs, keyboards), PS/2
                            keyboards, the first screen with a line to type on
    font/                   Own TrueType reader and rasterizer
    build.sh, check.py      Build; start in QEMU/OVMF and check the screen (also with Secure Boot enforced,
                            and typing on PS/2 and USB keyboards)
    release/claudeos.efi    The reproducible build Windows uses (CI checks it matches the source)
  windows/own-os.ps1        "ClaudeOS Own System.cmd": signs os/release with a key Windows makes on the PC
                            (CurrentUser\My, not exportable), copies it to \EFI\claudeos-own on Windows' EFI
                            partition with ClaudeOS.cer, adds "ClaudeOS (own system)" last in the power-on
                            menu and sets it as bootsequence (once); with Secure Boot on and the key not in db,
                            guides approving it (db Append) or turning Secure Boot off, then shutdown /r /fw
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
| Where updates come from | `/etc/claudeos/update-source` (root, 0644): the folder ClaudeOS Setup ran from, as a path below the top of Windows' drive (`Users/<you>/Downloads/ClaudeOS`); see "Updating ClaudeOS" |
| The version before an update | `/opt/claudeos/app.prev` (`app.failed` after an undo); `/var/lib/claudeos/update-pending` while a new version hasn't yet run 45 s |
| Logs, readable from Windows | `C:\Users\<you>\AppData\Local\ClaudeOS\from-claudeos\` (`claudeos.log.txt`, `session.log.txt`, `info.json`), see "Runtime" |

`settings.json` keys written by the Windows setup (all optional): `user_name` (display name), `windows_user`
(Windows profile folder name, e.g. `rensh`), `timezone` (IANA), `locale`.

### ClaudeOS exit codes (session supervisor contract)

* `0` normal exit → session script restarts ClaudeOS after 1 s.
* `3` restart requested (after an update) → restart immediately.
* other → crash; restart with backoff (1, 2, 5 s). 3 crashes within 120 s → if the version is a new update
  (`/var/lib/claudeos/update-pending` exists), `sudo -n claudeos-root update-undo` puts the version before back
  and ClaudeOS starts again at once; otherwise a recovery screen (full-screen `xterm` with the last log lines
  and `claudeos doctor` output; Enter retries). If it can't be shown, the session waits a minute before trying
  again rather than restarting in a tight loop.

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
| `apt-install PKG...` | `apt-get install -y --no-remove` (an install never removes anything) of validated package names (`^[a-z0-9][a-z0-9+.-]{0,61}[a-z0-9+]$`), each one apt knows by exactly that name (`apt-cache pkgnames`): apt-get reads `name-` as "remove name", and a name that isn't a package's as a pattern |
| `apt-remove PKG...` | the same validation, each one installed (`dpkg-query`) and not one of ClaudeOS's own packages (`/etc/claudeos/base-packages`, written by install-claudeos.sh); refused when the removal would also take packages that weren't named (they're listed in the error, from `apt-get -s remove`), so no removal takes ClaudeOS's screen, voice or network with it |
| `system-update` | `apt-get update && apt-get -y upgrade` |
| `boot-macos-next` | Mac: macOS for the next start only (`asahi-bless --set-boot-macos --next`; if macOS was renamed, the one start-up volume that isn't ClaudeOS) |
| `boot-default macos\|claudeos` | Mac: the start-up volume (`asahi-bless --set-boot-macos`, or `--set-boot ClaudeOS`) |
| `bt-import-windows-keys` | Copy Bluetooth Classic link keys (e.g. AirPods) from the Windows SYSTEM registry hive into BlueZ, restart bluetooth |
| `set-timezone ZONE` | validated against /usr/share/zoneinfo |
| `wipe-install-secrets` | Remove `/claudeos/secrets` from a ClaudeOS installer (FAT32 labelled `CLAUDEOS`) still on the PC or plugged in |
| `update-app` | Update ClaudeOS from the folder in `/etc/claudeos/update-source` (see "Updating ClaudeOS"). `{"ok":true,"changed":N}` |
| `update-done` | The new version started well: remove `/var/lib/claudeos/update-pending` |
| `update-undo` | Put `/opt/claudeos/app.prev` back (the new version goes to `app.failed`) and remove the marker |

On Fedora (a Mac), `apt-install`, `apt-remove` and `system-update` use `dnf` (`install -y
--setopt=install_weak_deps=False`, `remove -y --setopt=clean_requirements_on_remove=False` after `rpm -q` and
`rpm -e --test` show the packages are installed and nothing else needs them, `upgrade -y --refresh`). ClaudeOS asks the user (spoken yes/no) before calling any verb except `status`, `update-done`
(called after a new version has run 45 s) and `update-undo` (called by the session); `update-app` is asked on the
screen.

### Updating ClaudeOS

ClaudeOS Setup writes `/claudeos/update-source` on the installer: the folder it runs from, as a path below the
top of Windows' drive with each name spelled as the drive stores it (only when that folder is on Windows' own
drive). `finish.sh` copies it to `/etc/claudeos/update-source`, root's, so nothing ClaudeOS runs can point updates
elsewhere. A newer ClaudeOS put in that folder (downloaded again over the old one) is noticed when ClaudeOS
starts (`claudeos/update.py`: the files of `claudeos/` and `linux/session/` compared with the installed ones) and
offered on the screen, answered by typing or saying yes or no. Yes runs `claudeos-root update-app`:

1. refused until ClaudeOS has finished installing (`/var/lib/claudeos/base-installed`): until then the first
   start's finisher runs `install-claudeos.sh` as root, and that runs ClaudeOS's own code;
2. the recorded path must be relative, without `.` or `..` parts or control characters, and lead (after
   `realpath`) into Windows' drive, to a folder with `claudeos/__init__.py` and `requirements-linux.txt`;
3. `requirements-linux.txt` must be the installed one: a version that needs other Python packages is
   installed with ClaudeOS Setup;
4. only `claudeos/` and `linux/session/` are taken, the parts that run as the person. Nothing root runs (this
   helper, the Bluetooth importer, `linux/firstboot`, the system's settings) changes in an update, and root
   never runs ClaudeOS's own code once installed (`install-claudeos.sh` checks the import path with
   `importlib.util.find_spec`, which finds the package without running it);
5. refused when those parts hold anything but plain files and folders (links could lead out of the folder) or
   a Python start-up hook (`sitecustomize.py`, `usercustomize.py`, `.pth`), checked before the copy and again
   on the copy;
6. `/opt/claudeos/app.next` is a copy of the installed app with those two parts replaced (compiled files left
   out), made root's, folders 0755, files 0644, `#!` files 0755; then the installed version becomes `app.prev`,
   `app.next` becomes `app`, and `/var/lib/claudeos/update-pending` is written.

ClaudeOS then exits with code 3 and the session starts the new version. After it has run 45 s, ClaudeOS calls
`update-done`; if it keeps crashing before that, the session calls `update-undo`. Claude can read the update
folder but not change it: the permission gate denies edits there and in `/opt/claudeos`, shell commands that
change something and name it or `cd` into it, and moving, copying into or trashing it with ClaudeOS's own file
tools.

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
* **What's on it.** The official Ubuntu Server 24.04.x ISO's files, 24.04.4 or newer (`MinPointRelease`: from
  there on its installer makes the account before `late-commands`, which `finish.sh` needs; up to 24.04.3 that
  was left to cloud-init at the first start) (copied from the ISO as Windows mounts it;
  its links, like `ubuntu -> .`, aren't folders there and aren't copied), plus:
  * `/claudeos/installer-<install-id>` (`id=<install-id>`): the mark, written first, so even a half-made
    installer can be found by "Remove ClaudeOS" and, later, by ClaudeOS;
  * `/autoinstall.yaml` (generated from `windows/autoinstall.template.yaml`). The installer's GRUB menu
    becomes "Install ClaudeOS" (HWE kernel, default, 5 s) and "Install ClaudeOS (older kernel)", both with
    `autoinstall subiquity-storage-version=2` on the kernel command line: unattended, and able to see free
    space next to existing partitions (the installer's default storage version 1 sees none on a disk with
    partitions);
  * `/claudeos/app/`: a copy of this repo, without `windows/`, `mac/`, `.venv` or caches;
  * `/claudeos/secrets/claude_login.json`: the full sign-in from `claude auth login --claudeai` (Claude Code's
    `.credentials.json`), which setup picks up by itself; or, when the person pasted a code instead,
    `/claudeos/secrets/claude_token`, a one-year token from `claude setup-token`;
  * `/claudeos/settings.json`: initial settings (keys above);
  * `/claudeos/update-source`: where setup runs from, for updates (see "Updating ClaudeOS");
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
  (only if it's still the one it made, same offset and size), and C:'s space (back to the size C: had); it says
  what it couldn't. Fast Startup is turned off last, just before the PC is told to start the installer.
* **Making room on C:** (`Invoke-MakeRoom`, after "Go ahead"). ClaudeOS's space is the empty space right after
  C:, grown by shrinking C:: 70 GiB wanted (64 for ClaudeOS, 6 for the installer), 46 at least. Windows can
  only shrink a volume up to the last file it can't move while it runs, and `Get-PartitionSupportedSize`
  names that file in Application event 259 (Microsoft-Windows-Defrag; field 2 is the file, the same in every
  language; checked on Windows Server 2025). Setup shrinks as far as Windows allows, never leaving C: less
  than 25 GiB free, then deals with the file in the way and shrinks again, up to 12 rounds: the change journal
  (`\$Extend\$UsnJrnl`) is reset whenever it's in the way, which is harmless (Windows starts a new one at
  once, possibly near the end again, hence the rounds); restore points (`\System Volume Information\{...}
  {3808876b-c176-4e48-b7ae-04046e6cc752}`, shadow copies) are deleted and hibernation is turned off only when
  the minimum can't be reached otherwise. The page file and the file system's own records (`\$Mft`...) can't
  be moved while Windows runs: below the minimum, setup stops, undoes, and says what to do. The PC this was
  built for had 272 GiB free on C:, which could shrink by nothing: a restore point, then the journal twice,
  then `$Mft::$BITMAP` were in the way, and the rounds above made 63 GiB.
* **Signing in** (`windows/lib/ClaudeSignIn.ps1`). `claude auth login --claudeai` runs in a window of its own,
  opened through `explorer.exe` so it runs as the person without administrator rights, in Windows Terminal when
  `wt.exe` is there, from the home folder, with a throwaway `CLAUDE_CONFIG_DIR` under `%TEMP%`. Setup polls for
  the `.credentials.json` it saves, takes it, and deletes the folder. Run from setup's administrator console
  instead, the same program showed nothing at all on a real PC (twice), while it worked at once in an ordinary
  Windows Terminal window. Fallback: a code from `claude setup-token`, run in an ordinary window and pasted
  into setup with a right-click.

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
  4. sets up the first start: `/etc/claudeos/install-user`, `/etc/claudeos/install-id`, `/etc/claudeos/update-source`
     (when setup wrote one; root's, 0644), and two services from `linux/firstboot`, enabled by their
     `multi-user.target.wants` links:
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
entries and `\EFI\claudeos-setup`, so it cleans up an install that never finished too. Right after the typed
REMOVE it asks whether to keep the downloads (the Ubuntu ISO and the Agent SDK wheel, about 4 GB; default no):
kept, installing again finds them by their SHA-256 and doesn't download them again.

Tested end to end in QEMU (OVMF with Secure Boot and Microsoft's keys, an NVMe disk laid out as setup leaves a
PC): the "ClaudeOS setup" entry starts the installer from its partition, the install finishes, ClaudeOS starts,
and its first start deletes the installer partition, grows ClaudeOS's partition into it, and removes
`\EFI\claudeos-setup` and the entry; Windows' partition is untouched.

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
  subscription and an isolated `CLAUDE_CONFIG_DIR` (`~/ClaudeOS/.engine`). Two sign-ins (Windows setup gives
  the full one; a pasted code gives the token):
  * the one-year token from `claude setup-token`, passed as `CLAUDE_CODE_OAUTH_TOKEN`. It can only make model
    requests;
  * the full sign-in from `claude auth login` (`.engine/.credentials.json`, refreshed by Claude Code itself),
    used when present and `phone_access` is on, because Remote Control needs it. In this mode the token
    variables are blanked and `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` is not set (Remote Control needs
    Claude Code's feature flags); telemetry and error reporting stay off, which Claude Code allows Remote
    Control with on a paid plan (Pro, Max, Team, Enterprise) once it knows the account's organization. If it
    fails with an auth error, the brain switches to the token for the rest of the run and asks the last
    question again.
* Phone (the Claude app, Claude Code's Remote Control): after connecting with the full sign-in, the brain sends
  the SDK control request `{"subtype": "remote_control", "enabled": true, "name": "ClaudeOS (<host>)"}`, the
  same request Claude Code's own hosts send for `/remote-control`. The Python SDK has no public call for it,
  so it goes through `client._query._send_control_request`. The bridge session id from the response is
  kept and offered as `reattach_session_id` next time. A refused request is tried again after 20 s and 60 s:
  on the engine's first start it looks the account up in the background and refuses Remote Control until it
  has ("Unable to determine your organization"). Each refusal is logged with the engine's last lines of error
  output (kept by the brain, sign-ins blanked); the last one becomes a one-time notice on screen.
  One reader task consumes everything the engine emits. A turn that starts without a local question (a
  user message with a human or missing origin, or output with no turn open) is a phone turn: shown as
  the user's words like anything said here, the reply is not spoken, and voice questions are not asked. Turns the engine injects itself
  (task notifications and the like) belong to the PC. Speaking at the PC interrupts a phone turn. A phone
  turn silent for 15 minutes is closed. Local questions carry the "[time · screen: ...]" prefix; the system
  prompt tells Claude that a message without it came from the phone.
* ClaudeOS tools are in-process MCP tools (screen, files, computer control, apps, system, memory, self-made tools).
* Typed or said: what's typed on this PC's keyboard is answered in writing only (`Brain.ask(typed=True)`, and the
  controller doesn't speak that turn), like what's typed on the phone; only what's said out loud gets a spoken
  answer. Its note ends with "typed" (`[time · screen: ... · typed]`), and the system prompt tells Claude to
  answer that like a chat message, without an account of what it's about to do. Questions about a typed request are shown, not said, and end with "(Type yes or no.)", as they do
  whenever the microphone isn't listening. Things ClaudeOS says unasked (the greeting, "not signed in") are said
  only when sound plays on headphones; on the PC's or the monitor's speakers they are only shown.
* At the start (not in tests or `--fake`): a new version of ClaudeOS to update to, then AirPods, one question at
  a time. AirPods: nothing when sound already plays on headphones; AirPods already paired here are connected if
  they're out of their case; otherwise "Connect your AirPods?" on the screen. Yes copies Windows' pairing
  (`bt-import-windows-keys`) and connects, or else looks for AirPods in pairing mode for a minute and pairs
  them. The update comes first when Windows' drive is already mounted (yes restarts ClaudeOS); while it is still
  on its way, the AirPods come first.
* Logs on Windows' drive (`claudeos/logcopy.py`): ClaudeOS's own drive can't be opened from Windows, so the end
  of `claudeos.log` and `session.log` (1.5 MB each, CRLF line endings, anything like a sign-in, password or key
  blanked) and `info.json` (the phone link's state and why, voice, microphone, AirTouch) are copied to
  `C:\Users\<you>\AppData\Local\ClaudeOS\from-claudeos\`, only when ClaudeOS Setup's folder is there: a
  minute after the start, every 10 minutes, just before a restart or shutdown, and when ClaudeOS stops. The log
  also says, at the start, which graphics chips and drivers there are and the screens' sizes, and once a minute
  while typing, how long typed letters took to reach the screen and how often the window was busy for more
  than a tenth of a second.
* Every risky action goes through the permission gate → spoken confirmation ("Should I go ahead?"). During a
  phone turn the confirm callback returns "phone": the action is denied with a message telling Claude to ask
  in its reply. A "yes" typed on the phone (`classify_answer`) then allows exactly that action (keyed by tool
  and input; for Bash, the command) once within 10 minutes; "no" drops the question.
* A shell command runs without a question only when every part of it is known to only read
  (`classify_bash`): anything the gate can't analyse with certainty (several lines, substitutions, unusual
  redirections, options that write files or run programs, like `sed`'s `w`/`e` or `sort -o`) is asked about.
  `git` is always asked about: a folder's own git settings can make even `git status` run a program.
* ClaudeOS's private folders (`secrets/`, the engine's `.engine/` with the full sign-in, the browser's `web/`
  with its logins) can't be read, searched inside, copied, moved, trashed or shown on screen by ClaudeOS's tools
  or Claude Code's file tools, nor can other programs' settings (`/proc/*/environ`). Shell commands that name
  them, reach them with wildcards or `cd`, or search through a folder holding them (`grep -r`, `rg`) are
  blocked, even ones that would otherwise be asked about. The limit: a command the gate can't analyse is asked
  about, and once the person says yes it runs with their rights, which include reading those folders. Closing
  that too would need commands to run in a sandbox that hides the folders.
* Changing a file the computer runs commands from (dotfiles in the home folder, `~/.config/autostart`,
  `~/.local/bin` and the like) is asked about. The content layer doesn't let pages opened from files read other
  files with scripts (WebKit's file-access setting stays off), so a downloaded `.html` file can't read the
  sign-in.
* Screen: one full-screen GTK 3 window (`claudeos/ui/gtk_app.py`) with two WebKitGTK 4.1 views in a stack:
  the conversation (`ui/assets/chat.html`: only the user's words and Claude's words) and the content layer (a
  real browser with a persistent profile in `~/ClaudeOS/web`, plus a GTK caption strip over it). No buttons.
* Look: plain. Black text (#000000) on white (#FFFFFF) and nothing else: no dark mode, no grey text. Full
  black on full white is the most contrast a screen can show (21:1). Dimming belongs to the screen's
  brightness, because a grey background would cost contrast. The font is Atkinson Hyperlegible Next, with
  Atkinson Hyperlegible Mono for code. They ship in `ui/assets/fonts` (SIL OFL 1.1, from google/fonts; see
  its README), and `theme.css` loads them with `@font-face`. Pages written elsewhere get the fonts' file
  address (`render._theme_css`). `gtk_app.register_fonts` hands the folder to fontconfig (FcConfigAppFontAddDir)
  for the GTK captions. Characters the fonts lack (CJK, Greek, Cyrillic, emoji) come from the system's fonts
  (Noto). The user's words are in italics and Claude's are upright, flush left in one column in the middle of
  the screen, newest at the bottom. No imitation of any app.
* Size and width: text is sized so that the x-height covers 0.3 degrees of view. That is the top of the
  normal range of critical print size (0.15-0.3 degrees; Legge & Bigelow 2011), above which reading gets no
  faster until about 2 degrees. With Atkinson's x-height (0.496 em), that is 23.9 px at 100%. The column is
  at most 30 rem wide, about 70 characters a line (WCAG 1.4.8 AAA allows 80). `ui/layout.py` scales this for
  the monitor's physical size (EDID via GDK) and the viewing distance (estimated from the screen size, or set
  by voice), the way display scaling does. Websites get the display-scaling zoom, not the bigger text.
  Everything is recomputed when the window, monitor or settings change. Settings: `text_size` (0 = auto), `text_scale`, `viewing_distance_cm`, `screen_inches`
  (0 = reported), `web_zoom` (0 = auto).
* Voice: PipeWire/AirPods, Silero VAD, faster-whisper (local), Kokoro TTS (local), barge-in.
* AirTouch (`claudeos/airtouch`): the hands, seen by the screen's camera, in a process of its own at nice 5
  (`worker.py`). All of it is ClaudeOS's own code except the two networks: Google MediaPipe's palm detector
  and hand landmark model ("lite", Apache 2.0), downloaded with the voice models and checked against the
  SHA-256 that MediaPipe's own build pins.
  * Camera (`camera.py`): Video4Linux2 through ioctl, with mmap buffers. YUYV when the camera gives it at
    15 fps or more, otherwise MJPEG (the Huffman tables are added when a camera leaves them out). The Apple
    Studio Display's camera is chosen first; a camera plugged in later is found within 5 s.
  * Networks: `tflite.py` reads the model files (FlatBuffers); `nn.py` runs them in numpy (NHWC float32),
    within about 1e-6 of LiteRT. Tracking (`hands.py`) is MediaPipe's published method: palm detection on
    the letterboxed picture, landmarks on a rotated crop, then the joints place the next crop. Up to two
    hands, each keeping its number; a palm detected inside a hand already followed is that hand. With the
    same models the joints are within 0.5 px of MediaPipe's.
  * Pacing: with no hand, palm detection runs only when the picture changed (at most 4 times a second) and
    every 2 s; hands are followed with half a processor core (0.8 with the keyboard up), at 8 pictures a
    second when they rest; with one hand followed, palm detection looks for the other twice a second.
  * Gestures (`gestures.py`), from the joints in 3D: a hand's size is wrist to middle knuckle; which way the
    palm faces comes from the knuckles seen from the wrist and which hand it is (the landmark network says).
    Positions are absolute: the working space is 5 hand sizes across the screen, placed so the first dot
    starts where the pointer was, and moved along when a hand passes its edge; dots are smoothed with the
    1-euro filter. A dot per index finger; a pinch (two pictures running) taps where the dot was held while
    the fingers met; pinching and moving scrolls; an open palm towards the camera held 0.6 s is "stop"; a
    thumb held up or down is yes or no. Open fingers, then all four curled within 0.8 s with the hand still,
    is a beckon: palm up, the keyboard comes up; palm down, it goes away. With the keyboard up the working
    space is the keyboard, a row per 0.2 hand sizes and a key per 0.21 (a real keyboard's 19 mm), or, when
    both hands are in view as it comes up, whatever puts each index finger on its home key (F, J), within
    0.15 to 0.35. A hand's four fingers are on four keys side by side along its knuckles (which don't move
    when a finger taps); only the index finger's is drawn. A finger bending 18 degrees past where it rests,
    6 more than the hand's other fingers, types the key it was on a tenth of a second before.
  * Keyboard (`airtouch/keyboard.py`): the iPad's (iPadOS, 11-inch iPads), every key's edges and every row's
    measured from the three pictures of it in Apple's iPad User Guide ("Type with the onscreen keyboard on
    iPad" and two more pages; 587 by 180 pixels of keys, the pictures agreeing within 1.4 px). It's scaled as
    a whole, as big as fits 42% of the screen's height, in the middle. As in the guide: a double tap on
    Shift is Caps Lock, a double tap on Space a full stop and a space. The .?123 page has the letter keys'
    second characters in their places (not in Apple's pictures, so not measured); the microphone key types
    the next thing said (`Controller.dictate_until`), and the emoji key has a page of emoji.
  * Screen: the dots and the keyboard (`ui/airtouch_layer.py`) are drawn with cairo on a layer over
    everything that lets clicks through, black on white; the keys' symbols are drawn there, not taken from
    fonts. The keyboard is drawn once into a picture of its own, so dots moving over it redraw only their
    squares; pages get the room above it. Keys are typed with xdotool, so they go wherever typing goes: the
    conversation, or a page's box.
  * Pointer: moved with XTEST. Its arrow is hidden with XFIXES while AirTouch moves it, and comes back when
    something else moves the pointer (a mouse, or a click Claude made); AirTouch then leaves the pointer
    alone until it has been still for 3 s.
* Keyboard and mouse: USB ones just work. The arrow shows when the mouse moves (unclutter hides it after 3 s
  still). A Bluetooth keyboard (`bluetoothctl info`: Icon input-keyboard) pairs through bluetoothctl's own
  agent as KeyboardDisplay, answering its questions: the passkey to type on the keyboard (or a PIN made up
  for an older one) is shown and spoken. Other devices pair as NoInputNoOutput.
* Speed (`tests/test_speed.py`, and the listener tests in `tests/test_voice.py`):
  * Start: the window comes first. `run_app` shows it, then makes the controller on the asyncio thread, since
    loading the Agent SDK takes most of a second (the permission gate imports the SDK only when the engine
    first asks, so the window module doesn't wait for it). The voice models load side by side and speech
    recognition gets a first run on silence right away (its first run is the slowest). The file index's first
    crawl waits 10 s.
  * Listening: an utterance ends after 0.7 s of silence (0.45 s for a yes/no), but its final transcription
    starts after 0.3 s and is used if no more speech came. Partial transcriptions run only while someone is
    talking, at most every 0.9 s or 1.5 times the last transcription's duration, whichever is longer, so the
    recognizer is free when the utterance ends.
  * Speaking: Kokoro uses one thread per core (onnxruntime's choice); a long first sentence is spoken from its
    first comma, `;` or `:` after 20 characters, once 50 have arrived. Whether sound plays on headphones (for
    barge-in) is asked on every 32 ms frame while ClaudeOS talks: it's answered from a check (wpctl, about a
    quarter of a frame) made in the background every 5 s, not by running wpctl on the event loop each frame.
  * The file index reads documents' text (pypdf, python-docx, openpyxl, python-pptx: pure Python) in a process
    of its own, `TextReader`, at nice 10. In ClaudeOS's own process they held the interpreter long enough for
    speech to skip (6 to 11 underflows in 6 s of speech under PipeWire; none since, and none from walking
    folders). A document a reader hangs on is given up after 60 s, and stopping ClaudeOS doesn't wait for one.
  * Tools: `system_status` asks its six parts side by side. Without a Bluetooth adapter (`/sys/class/bluetooth`
    empty or missing) bluetoothd may not run, and each bluetoothctl command waited 8 s for it: the Bluetooth
    calls now say so at once. `look_at_screen` makes the JPEG straight from the grab (a 4K screen: 0.65 s with a
    PNG in between, 0.17 s without), of the whole screen (mss's monitor 0), so click positions match with more
    than one monitor. `show_file` renders PDF pages as JPEG at quality 90 (30 pages: 7.4 s as PNG, 0.6 s), into
    a folder kept only when complete, and drops pages of PDFs not shown for two weeks. Trashed files are found
    by the name they were given (GLib's `name.2.ext`, send2trash's `name 1.ext`) rather than by reading every
    `.trashinfo` (2 ms instead of 40 ms with 2,000 things in the trash), and the newest wins a tie within the
    second, so undo restores the right one.
  * The conversation history shown at the start comes from the last 256 KB of `transcript.jsonl` (it keeps
    every conversation).
  * A request doesn't wait for an internet check: a recent result, or an older "yes" (checked again in the
    background), is used; only an older "no" is checked first.
  * The conversation page hides messages that scrolled off one by one as words stream in (measuring before it
    changes anything), and looks at all of them again only when there may be room (a new size, a shorter
    message). Captions over websites change once per 40 ms flush, not per streamed word.
  * Boot: `claudeos-windows-mount.service` waits for the disks itself (`udevadm settle` in `ExecStartPre`)
    instead of pulling in `systemd-udev-settle.service`, which comes before `sysinit.target` and so held up the
    whole start. If Windows' drive (or a Mac's shared drive) arrives after ClaudeOS has started, ClaudeOS takes
    it in then: a sign-in left there, its files in the search, and the engine starts again when idle to learn
    where they are (at the start the engine waits up to 10 s for the drive).

## Mac

On a Mac with Apple silicon, ClaudeOS is a second system next to macOS, as on a PC. Only Asahi Linux can start
another system on these Macs, so ClaudeOS's Linux base there is Fedora Asahi Remix (the Asahi project's
flagship distribution), installed by the Asahi installer. That sets the Macs it works on: M1, M2, and M3, M3 Pro,
M3 Max (not M3 Ultra, M4, M5; on M3 for now without graphics acceleration, brightness or other screens, per
Asahi's feature tables). The runtime (`claudeos/`) is the same as on a PC; `claudeos/platform/linux.py` has
`MacPlatform` (chosen when `/proc/device-tree/compatible` starts with `apple,`).

macOS's own files can't be read from Linux, unlike Windows' on a PC: Apple silicon encrypts every internal APFS
volume ("Encrypted at rest", FileVault or not), and the disk controller decrypts a request only when it carries
the volume's encryption details, which macOS sets up with the Secure Enclave. Linux's driver leaves those fields
empty (`aes_iv` in `drivers/nvme/host/apple.c`), and Asahi has no Secure Enclave driver yet; apfs-fuse lists
hardware-encrypted volumes as unsupported. So setup copies folders onto an exFAT
drive both systems open.

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

GRUB starts Fedora's kernel with `"systemd.run=/bin/sh /boot/efi/claudeos/firstboot.sh"`,
`systemd.unit=kernel-command-line.target`, `systemd.run_success_action=reboot` and
`systemd.run_failure_action=reboot`, instead of Fedora's first-start questions. Two details matter, both found
in an emulated Mac: the whole argument is in double quotes (GRUB passes it on in double quotes, which systemd
removes; quotes inside it would reach systemd as `\"`, which it doesn't read as quotes), and `systemd.unit` names
the target outright (systemd-run-generator's own `default.target` loses to the one Fedora's image has in /etc). firstboot.sh (POSIX sh, as root, SELinux enforcing): hostname `claudeos`;
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
* An emulated Mac (QEMU `virt`, `-cpu neoverse-n1` for 16K pages, UEFI), its disk built from the real Fedora
  package with setup's own `write_first_start`: Fedora's 16K kernel starts, the first start runs and restarts,
  `claudeos-finish-install` installs everything (dnf, then pip on Python 3.14) and restarts, and ClaudeOS's
  screen comes up, with SELinux enforcing and no denials. On 16K pages the Claude program runs and GTK,
  WebKit, numpy, onnxruntime, CTranslate2 and the voice packages load. It also found what the pretend tests
  couldn't: the first-start quoting, the default target, Fedora Asahi Remix's os-release ID, kokoro-onnx's
  Python bound, and GTK's cairo typelib (in `gobject-introspection`, only a weak dependency). (QEMU 8.2's
  `-cpu max` crashes this kernel early: an emulator problem, not a Mac one.)

Not yet tried on a real Mac. Unknowns: AirPods' microphone (Bluetooth HFP) under Asahi; AirPods paired in
ClaudeOS may need pairing again in macOS (the same Mac, a new key); the Mac's own graphics (the emulated Mac
used a virtual one).
