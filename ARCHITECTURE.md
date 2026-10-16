# ClaudeOS architecture (internal contract)

Goal: **hardware → thin Linux drivers (invisible) → Claude (user's subscription, over the internet) → the user (AirPods +
screen + phone)**. ClaudeOS is a conversational shell: you talk (AirPods) or type (the Claude app on the phone is the
keyboard), and both are one conversation.
Power on → boot menu ("ClaudeOS" default, "Windows") → straight into the full-screen ClaudeOS conversation.
No login screen, no desktop environment, no display manager, no browser UI. Windows stays installed untouched.
On a Mac, ClaudeOS is a full-screen app on macOS instead of a second system (see "Mac" at the end).

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
  mac/                      ClaudeOS for Mac: an app; macOS stays as it is
    setup.sh                Install/update, sign in again, open at login, remove (bash 3.2, run in Terminal)
    requirements-mac.txt    Pinned Python deps for the Mac runtime
    launcher/               The app's main program (C; built with Zig for arm64 and x86_64)
    ClaudeOS.icns           App icon (make_icon.py)
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
| `bt-import-windows-keys` | Copy Bluetooth Classic link keys (e.g. AirPods) from the Windows SYSTEM registry hive into BlueZ, restart bluetooth |
| `set-timezone ZONE` | validated against /usr/share/zoneinfo |
| `wipe-install-secrets` | Remove `/claudeos/secrets` from a ClaudeOS installer (FAT32 labelled `CLAUDEOS`) still on the PC or plugged in |

ClaudeOS asks the user (spoken yes/no) before calling any verb except `status`.

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

On a Mac, ClaudeOS runs as an app on macOS that takes over the whole Mac from login ("whole-Mac mode"), not as
a second system: on Apple silicon only Asahi Linux can start another system, not yet on M4/M5, and Linux there
can't open the Mac's files (encrypted with keys only that Mac's macOS can use). The runtime (`claudeos/`) is the
same; the platform layer is `claudeos/platform/mac.py` (engine name `mac`, chosen on `sys.platform ==
"darwin"`) and the screen is `claudeos/ui/mac_app.py`.

| What | Path / value |
|---|---|
| App | `~/Applications/ClaudeOS.app`, a copy of `mac/ClaudeOS.app`: `Info.plist` (from `mac/app`; id `com.claudeos.app`, reasons for the microphone, Bluetooth, Apple events and Location, `NSAllowsArbitraryLoadsInWebContent`), `Contents/MacOS/ClaudeOS` (universal, from `mac/launcher`), the icon. Signed ad hoc when it's built (rcodesign), so it is identical on every Mac and nothing is signed there (`codesign` needs Apple's developer tools for `codesign_allocate`). |
| Launch settings | `~/ClaudeOS/launch.conf`: `python=` (the venv's python) and `libpython=` (the shared library), written by setup |
| Runtime code | `~/ClaudeOS/app` (`claudeos/`, `mac/`, docs) |
| Python | uv 0.12.20 (PyPI wheel, fixed SHA-256) installs CPython 3.12 into `~/ClaudeOS/python`; venv `~/ClaudeOS/runtime` with `claudeos-app.pth` → `~/ClaudeOS/app`. Packages are wheels only (`--only-binary :all:`). uv runs `install_name_tool` after downloading Python; setup gives it a do-nothing one first on `PATH`, because on a Mac without the developer tools the real one only opens an install window. |
| User data | `~/ClaudeOS/` with the same layout as on Linux (token, settings, models, logs, memory, notes) |
| Engine sign-in | `CLAUDE_CONFIG_DIR=~/ClaudeOS/.engine`; on macOS Claude Code keeps the full sign-in in the login Keychain, so `config.keychain_login` asks `claude auth status --json` (`authMethod` `claude.ai`) instead of looking for `.credentials.json`. |
| Opening at login | `~/Library/LaunchAgents/com.claudeos.start.plist`: `/usr/bin/open -a <app>`, `RunAtLoad`, no `KeepAlive` (quitting is respected) |
| Website data | WebKit's default data store for `com.claudeos.app`; the conversation page uses a non-persistent store |

Requirements: Apple silicon with macOS 14+ (onnxruntime 1.31.0 publishes Apple silicon wheels for macOS 14+
only), or Intel with macOS 13+ (onnxruntime 1.23.2, the last release with Intel Mac wheels, via a
`platform_machine` marker). Both sets were checked with `uv pip compile --python-platform ... --only-binary :all:`.

### The app's main program

`mac/launcher/launcher.c` reads `~/ClaudeOS/launch.conf` (or `$CLAUDEOS_HOME/launch.conf`), sends
stdout/stderr to `~/ClaudeOS/logs/app.log`, removes `PYTHONHOME`, `PYTHONPATH`, `PYTHONSTARTUP` and
`PYTHONINSPECT`, sets `PYTHONEXECUTABLE` to the venv's python and `CLAUDEOS_HOME`, changes to the home folder,
`dlopen`s the shared `libpython3.12.dylib` that python-build-standalone ships (`RTLD_GLOBAL`, so extension
modules resolve against it) and calls `Py_BytesMain` with `<venv python> -P -m claudeos run` (`-P`: the home
folder isn't put on the import path, so a `~/json.py` can't replace part of Python; the PC's session uses `-P`
too). CPython's `getpath.py` takes
`PYTHONEXECUTABLE` as `sys.executable` and the place to look for `pyvenv.cfg`, so the venv applies.
`claudeos.__main__` removes `PYTHONEXECUTABLE` again before anything starts other programs. Running inside the
bundle's own process means macOS attributes the Dock icon, the menu name and permission requests (TCC) to
ClaudeOS.app; a script that started a separate Python would be attributed to that Python, which has no
usage descriptions. On failure the launcher shows an alert through `/usr/bin/osascript`. It uses only
libSystem; `mac/launcher/build.sh` builds it with Zig (`aarch64-macos.11.0`, `x86_64-macos.11.0`), joins the
two into a universal program (`universal.py`) and signs the app with rcodesign 0.29.0. The build is
reproducible (`mac/launcher/BUILT-WITH.txt`), so the app, and the permissions macOS remembers for its signature,
stay the same across updates unless the launcher changes.

### Screen and switching

* One borderless window the size of the screen (`NSWindow`), the conversation in a `WKWebView` and the
  content layer in a second one (`content.js` injected in the `WKContentWorld` "claudeos", messages through
  the `claudeosPage` handler), captions as `NSTextField`s. While ClaudeOS is in front, the presentation
  options `HideDock | HideMenuBar | DisableProcessSwitching` leave nothing of macOS on screen (Force Quit still
  works), and an `NSProcessInfo` activity (`IdleDisplaySleepDisabled | UserInitiated`) keeps the display on:
  when it goes dark macOS locks, and nobody can talk past a lock screen. Hiding ends the activity. Pages use
  the Mac's system font (`system-ui`).
* WebKit's decision and completion handlers (navigation response policy, downloads, camera requests) reach
  Python as blocks without a signature PyObjC can call; `claudeos/ui/mac_blocks.py` calls their invoke
  function directly. (Found on GitHub's Macs: ClaudeOS quit at its first page on Apple silicon, and no page
  finished loading on Intel.)
* "Switch to macOS" is `power(switch_to_macos)`: no restart. The platform calls `on_step_aside`; the
  controller goes to sleep (wake word only) and the screen hides the app. Command-H does the same through
  `applicationDidHide:`. Coming back: the wake word, an AirPods press (`MPRemoteCommandCenter`), or
  activating the app (Dock), which wakes the controller. It unhides and asks to be active; since macOS 14 that
  can be refused while another app is in use, so if it isn't active 0.4 s later it opens its own bundle with
  `NSWorkspace openApplicationAtURL` (activates: YES), the way the Dock does.
* `start_by_default` (claudeos | other) writes or removes the LaunchAgent (`MacPlatform.set_default_os`).
* Quitting (Command-Q, logging out, a restart or shutdown): `applicationShouldTerminate:` answers
  `NSTerminateLater`, stops ClaudeOS on its loop, flushes the logs, then `replyToApplicationShouldTerminate:YES`;
  macOS ends the process. A thread's timer replies after 10 s if stopping hangs (while macOS waits, the run loop
  runs only in the modal-panel mode, where `callLater` timers don't fire). If pages keep failing to draw, ClaudeOS quits
  this way and opens itself again.
* Sound: PortAudio reads the Mac's devices only when it starts, so every 2 s ClaudeOS reads Core Audio's default
  input and output device ids (`AudioObjectGetPropertyData`, through ctypes); when they change (AirPods connect
  or go back in their case), it closes the microphone and speaker, restarts PortAudio and opens the microphone
  again, all under one lock (`audio.PORTAUDIO_LOCK`) that every stream open and close also takes.
* Permissions are asked for when first needed: Accessibility with `AXIsProcessTrustedWithOptions` (prompt, once
  per run), Location with `CLLocationManager.requestWhenInUseAuthorization` on the main thread the first time
  Wi-Fi names come back hidden.
* Files: the index skips `~/Library` (other apps' data; reading it makes macOS ask about "data from other
  apps" every run) and `~/Applications`, and doesn't walk into packages (`.app`, `.photoslibrary`, ...). iCloud
  Drive is its own root; files kept only in iCloud (`SF_DATALESS`) are listed but not read. Trashing uses
  `NSFileManager trashItemAtURL:resultingItemURL:` so undo knows where the item went; if macOS won't let
  ClaudeOS take it out of the Trash, it says how to use Put Back.
* Platform: Wi-Fi through `networksetup` and Core WLAN; Bluetooth and sound devices from `system_profiler -json`,
  connecting with IOBluetooth; volume through AppleScript; sleep with `pmset`, shutdown/restart through
  System Events; clicks and keys for websites with Quartz `CGEvent` (needs Accessibility, and only while
  ClaudeOS is the app in front, so events never land in another app; otherwise through the page); software through
  Homebrew when it's installed.

### Setup (`mac/setup.sh`)

Menu: Install (or Update), Sign ClaudeOS in to Claude again, Choose whether ClaudeOS opens when you log in,
Remove ClaudeOS from this Mac. Install checks the Mac (`sysctl hw.optional.arm64` so Rosetta Terminals count
as Apple silicon), asks "Go ahead?", then quits a running ClaudeOS, gets uv, copies the app, makes the
runtime, writes `settings.json` (name from `id -F`, time zone from `/etc/localtime`), signs in
(`claude setup-token` in a throwaway config dir, then the pasted token → `secrets/claude_token` 0600;
optionally `claude auth login --claudeai` with `CLAUDE_CONFIG_DIR=~/ClaudeOS/.engine`), downloads the voice,
copies the app and writes `launch.conf`, and asks about opening at login. Claude Code wraps the token with real
line breaks, so a copy can hold two or more lines: setup reads it with the terminal in non-canonical mode and
bracketed paste on (`ESC[?2004h`), so line breaks inside the paste don't end it and the Enter after it does; in a
terminal without paste marks, a line break followed within half a second by more text counts as part of the paste
(Windows setup reads the lines already waiting in the console for the same reason). Only
recognised answers are written to the setup log, and anything shaped like a token is blanked out there. Remove (typed REMOVE): quits ClaudeOS, runs `claude auth logout` for
`~/ClaudeOS/.engine` (which removes that Keychain item), removes the LaunchAgent, the app, WebKit's data for
`com.claudeos.app` and `~/ClaudeOS` (optionally keeping memory, notes and conversations).

Test hooks: `CLAUDEOS_MAC_ROOT` (home folder), `CLAUDEOS_UV`, `CLAUDEOS_ENGINE_CLI`, `CLAUDEOS_SKIP_MODELS`.
Tests: `tests/test_mac_setup.py` (setup against stand-ins for macOS's commands), `tests/test_mac_launcher.py`
(the launcher built for Linux, plus a check of the universal program's Mach-O headers and signature), `tests/test_mac.py`. None of
this has run on a real Mac yet.
