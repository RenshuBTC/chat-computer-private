# Chat Computer architecture (internal contract)

Goal: **hardware → thin Linux drivers (invisible) → Chat Computer's own model (one of Qwen's open models, run on the
computer itself by Chat Computer's own engine: free, and nothing leaves the computer) → the user (touchscreen +
keyboard + AirPods)**. Chat Computer is a conversational shell: you touch the screen as an iPad's, type or talk
(AirPods), and it's all one conversation.
Power on → boot menu ("Chat Computer" default, "Windows") → straight into the full-screen Chat Computer conversation.
No login screen, no desktop environment, no display manager, no browser UI. Windows stays installed untouched.
On an Apple silicon Mac (M1, M2, M3) it is the same: a second system next to macOS, on Fedora Asahi Remix instead
of Ubuntu, chosen with the Mac's own start-up volume instead of a boot menu (see "Mac" at the end).

## Repository layout

```
chat-computer/
  chatcomputer/             Python package: the Chat Computer runtime (brain, tools, voice, screen)
    airtouch/               Hands seen by the screen's camera: camera, model reader + numpy engine, hand
                            tracking, gestures, keyboard layout, pointer (its own process)
  linux/                    The thin Linux base (installed into the target system)
    install-chatcomputer.sh  Runs INSIDE the target (curtin in-target) or later by hand; idempotent
    system/                 Files copied into the target root (/etc, /usr/local/...)
    session/                tty1 autologin -> startx -> session script -> Chat Computer
    grub/                   /etc/grub.d/07_chatcomputer (boot menu entries)
    root-helper/chatcomputer-root  The only privileged entry point Chat Computer may use (via sudo -n)
    tests/
  windows/                  One-time setup tool run on Windows (PowerShell)
    "Chat Computer Setup.cmd"  Double-click entry point (self-elevates)
    setup.ps1, lib/*.ps1
    autoinstall.template.yaml
    tests/
  mac/                      One-time setup run in macOS (bash 3.2, in Terminal)
    setup.sh                Install, send files, sign in again, choose what starts, remove
    manifest.sh             What setup downloads, pinned with SHA-256: Asahi installer, Fedora's image
    installer_data.json     The Asahi installer's OS list: Chat Computer (Fedora Asahi Remix 44 Minimal)
    drive-installer.exp     Answers the Asahi installer's questions (expect)
    esp/                    Chat Computer's first start on the Mac: grub-firststart.cfg, firstboot.sh
    chatcomputer.icns       Icon in the Mac's startup options (make_icon.py)
  "Chat Computer Setup.command"  Double-click entry point for Mac setup (runs mac/setup.sh)
  os/                       Chat Computer's own operating system, from scratch (Rust, no outside code); see os/README.md
    boot/                   UEFI start-up stage (BOOTX64.EFI) that carries the kernel
    kernel/                 The kernel: memory, interrupts, timer, PCI, USB (xHCI, hubs, keyboards, mice (their
                            wheels scroll: mouse.rs), network adapters: CDC ECM and NCM, RNDIS), PS/2 keyboards,
                            network cards (netcard/: virtio,
                            Intel's e1000 and igb families and I225/I226, Realtek's RTL8111/8168 and
                            RTL8125), the drive the PC started from (storage/: NVMe, SATA by AHCI), the
                            settings (the Claude API key, the memory file's id) and the clock from the
                            start-up stage, Claude (assistant.rs) with tools on the computer (its status:
                            system.rs; the text size; a restart, or turning off by ACPI S5: power.rs; a new conversation;
                            notes about the person: remember and forget), the conversation and the notes kept across
                            restarts (keep.rs, as JSON {"notes": [...], "messages": [...]}; at most ~600 KB of the
                            conversation sent), and the conversation screen (chat.rs, drawn in memory: draw.rs;
                            Claude's answers laid out from their Markdown: rich.rs)
    net/                    Own network stack: Ethernet, ARP, IPv4, ICMP, UDP, DHCP, DNS, TCP
    crypto/                 Own cryptography: SHA-2, HMAC, HKDF, AES-GCM, ChaCha20-Poly1305, X25519, ECDSA, RSA
    tls/                    Own TLS 1.3 client and certificate checks, against Mozilla's authorities (roots.txt)
    claude/                 Claude's Messages API, streamed: the request (Opus 5.5, effort max, automatic prompt
                            caching), HTTP/1.1, JSON
    model/                  Chat Computer's own model engine (no outside code; its core needs no standard library):
                            Qwen's open models (3.5, 3.6, 3.8; Apache 2.0) on the PC's own processor, AVX2 and VNNI;
                            the model's file format, its converter from Qwen's published files, the tokenizer, each
                            model's chat template as code, tool calls, sessions that read only what's new; the
                            program chat-computer-model (serve: OpenAI's chat completions API on 127.0.0.1, for
                            this computer's programs and Chat Computer's own pages only; convert), built for Linux
                            by release.sh into chatcomputer/brain/bin (x86-64 and ARM, static, reproducible: CI
                            checks them). See its README.md. In place of Claude, everywhere: the Linux runtime
                            thinks with it now; the apps and the own system next
    font/                   Own TrueType reader and rasterizer, variable fonts too (fvar/avar/gvar: the bold, from the same file)
    fonts/                  Fallback fonts for what Atkinson Hyperlegible lacks (arrows, maths, Greek, Cyrillic, emoji,
                            box drawing), cut and merged from Noto fonts by build.py
    markdown/               Claude's Markdown read by md.js's rules (tested against md.js itself: markdown/gen/gen.js)
    fat/                    Own FAT12/16/32 reader: a file's blocks on the start-up partition
    hid/                    USB HID report descriptors: where a mouse's buttons, movement and wheel are in its reports
    remember/               The memory file's format (memory.bin): 512-byte chunks tagged "CCMEMORY" + file id
                            (u32 LE) + chunk number (u32 LE), 496 data bytes each; two halves written in turn, each
                            headed "CCMEMDAT" + version + sequence + length + FNV-1a 64 (header written last); a
                            sector is written only after all its tags are read back and match
    build.sh, check.py      Build; start in QEMU/OVMF and check the screen (also with Secure Boot enforced,
                            typing on PS/2 and USB keyboards, USB mice's wheels, the network, a conversation with a
                            stand-in for Claude's API over HTTPS, and the conversation kept across a restart on NVMe
                            and SATA)
    release/chatcomputer.efi  The reproducible build Windows uses (CI checks it matches the source)
  windows/own-os.ps1        "Chat Computer Own System.cmd": signs os/release with a key Windows makes on the PC
                            (CurrentUser\My, not exportable), asks for a Claude API key (hidden, never logged),
                            copies it to \EFI\chatcomputer-own on Windows' EFI partition with ChatComputer.cer,
                            settings.txt (the key, the clock's offset from UTC, memory-id = <8 hex digits>) and
                            memory.bin (made at full size, 4 MiB, else 1 MiB, only if 16 MiB stay free; kept when
                            its first and last tags carry the settings' id), adds "Chat Computer (own system)" last in the power-on
                            menu and sets it as bootsequence (once); with Secure Boot on and the key not in db,
                            guides approving it (db Append) or turning Secure Boot off, then shutdown /r /fw
  requirements-linux.txt    Pinned Python deps for the Linux runtime
  requirements-dev.txt      What its tests need besides those (pytest, the stand-in model server's aiohttp); the runtime's
                            tests run as .github/workflows/python.yml does: a venv with the system's python3-gi and
                            python3-cairo, both requirements files, then pytest --ignore=tests/test_rendering.py
  tests/                    Python tests for the runtime (+ a stand-in for the model's server: tests/mock_model.py; a
                            tiny model with random weights for the real engine: tests/data/tiny-model.ccm)
  docs/                     The browser demo (GitHub Pages): Chat Computer's own screen with docs/demo/host.js standing
                            in for the computer; scripts/build_demo.py copies the screen, tests/test_demo.py checks
  docs/get/                 chatcomputer.ai/get: the apps to download (as Annotated's page offers its extension)
  apps/chrome/              The Chrome extension: the website in Chrome's side panel (?app=chrome), its sites in
                            real Chrome tabs; scripts/build_apps.py zips it into docs/get, tests/test_apps.py checks
  apps/desktop/             The Windows and Mac app (Tauri 2): src-tauri/ (Rust: the window, the site's own view
                            laid over the page's frame, "/" file search, the Mac's 127.0.0.1 page server,
                            capabilities/ giving only the app's own page its commands), frontend/ (made by
                            build_apps.py desktop: the website's page, with Gemma's and Heart's code), smoke/ (a site for
                            CI's real start-up pictures); .github/workflows/apps.yml builds both on GitHub;
                            tests/test_desktop.py checks it, driving a Linux build with WebDriver where there is one
  apps/android/             The Android app: chatcomputer.ai full screen in Chrome (a Trusted Web Activity, Google's
                            androidbrowserhelper; docs/.well-known/assetlinks.json vouches for its key), Gradle
  apps/ios/                 The iPhone app: chatcomputer.ai in a WKWebView (SwiftUI), other sites in Safari's view;
                            XcodeGen's project.yml, built for the simulator by .github/workflows/ios.yml
```

## The Windows and Mac app

The website's page (docs/index.html and docs/demo/, copied by `scripts/build_apps.py desktop`) is bundled into the
app, so the app's own page (the webview `main`) never runs code from elsewhere: even Gemma's Transformers.js, Heart's
kokoro-js (the warmer voice) and ONNX Runtime's WebAssembly come with it (`demo/lm/`, the website's versions, from
npm), and only the models are downloaded. host.js sees the app (Tauri's `window.__TAURI__`, `SHELL.kind == "app"`) and then:

* A site goes in the app's own browser view, the webview `site` (`show_site`), laid over the frame chat.js draws,
  moved with it (`place_site`), put out of sight while the conversation has the middle or a sheet or a held message
  is over it (`hide_site`, `unhide_site`), and closed with its x (`close_site`). Any site shows there, as on a
  Chat Computer (`framed()` takes every web address). It's the same view each time, so sign-ins and the way back
  stay. A link meant for a new window opens there instead; only http(s), about: and blob: are followed; downloads
  go to Downloads and are listed in Actions; its title and where it is come back to the page (`chatcomputerApp`,
  which the app runs in the page with `eval`). On a Mac it says it's Safari (it is Safari's engine). A video is
  YouTube's own page; turning it down, pausing it, and Page Up and Down are fixed scripts the app runs in the
  site's page (`site_media`), never anything the page sends.
* "/" finds the computer's files by name (`search_files`: Desktop, Documents, Downloads, Pictures, Music, Videos
  and OneDrive, skipping hidden folders, programs' own folders and app bundles; at most 8, best first), and the
  one chosen opens with its own app (`open_file`), only if a search found it and it's a document, picture, music,
  video, archive or folder, never a program or a script. Connected Claude has the same (`find_files`,
  `open_file`); the website's made-up messages, files and earlier conversations aren't in the app.
* The greeting has the first name of the computer's account (`about`), unless one was given ("call me ...").
* A link clicked in the page opens in the middle; the page sent elsewhere is taken back (on Windows the main
  frame's navigation is stopped; on a Mac and Linux, where frames ask too, the page is loaded again) and the
  address opened in the middle.

Only the app's own page can ask for these: `build.rs` makes each command a permission, `capabilities/` give them
to the webview `main` alone (never to a window, which would include the site's view), and each command checks the
webview asking is `main`. A site's page gets Tauri's script too, but every command it asks for is refused (the
CI's start-up picture shows a test site saying so). On Windows and Linux the page comes from Tauri's own scheme;
on a Mac, whose web view doesn't count that scheme as secure (and so gives it no WebGPU, microphone or cache),
from the app's own server at 127.0.0.1 (`loopback.rs`: ports 42851 to 42853, the first free one, so the page's
storage stays put; only GET, only to that name, only the bundled files), with a capability for exactly those
addresses.

## Base system

* Ubuntu Server 24.04 LTS (amd64), installed unattended by the official installer (subiquity autoinstall).
* Installed **alongside Windows** in free disk space (`storage.layout.mode: use_gap`) or on a separate whole disk
  the user picked (matched by serial). Never touches existing partitions. Reuses the existing EFI System Partition.
* Packages are the minimum for: display (Xorg, no desktop), Wi-Fi/Ethernet (NetworkManager), Bluetooth + audio
  (BlueZ, PipeWire, WirePlumber, libspa-0.2-bluetooth), web rendering for Chat Computer's own screen (WebKitGTK 4.1 via
  python3-gi + gir1.2-webkit2-4.1, GStreamer codecs), screen control (xdotool, wmctrl), Windows files (ntfs-3g,
  cryptsetup-bin for BitLocker), boot control (efibootmgr), fonts, Python 3.12 venv.
* On a Mac: Fedora Asahi Remix 44 Minimal (aarch64, 16K pages), installed by the official Asahi installer, and the
  same set by Fedora's names (`install-chatcomputer.sh --distro fedora`, chosen from `/etc/os-release`), plus
  `asahi-bless` (start-up volume) and `exfatprogs` (the drive macOS shares). See "Mac".

## Installed paths and users

| What | Path / value |
|---|---|
| Runtime code (root-owned, read-only to the user) | `/opt/chatcomputer/app` (copy of this repo) |
| Python venv (`--system-site-packages`, for python3-gi) | `/opt/chatcomputer/venv`; `chatcomputer-app.pth` in its site-packages puts `/opt/chatcomputer/app` on the import path |
| Start command | `/opt/chatcomputer/venv/bin/python -m chatcomputer run` with `CHATCOMPUTER_ENGINE=linux` |
| Model download command (run as the user) | `/opt/chatcomputer/venv/bin/python -m chatcomputer models --download` |
| Health check command | `/opt/chatcomputer/venv/bin/python -m chatcomputer doctor` (prints JSON, exit 0 if usable) |
| User data (owned by the user) | `/home/<user>/ChatComputer/` |
| The model (Chat Computer's own file of it) | `/home/<user>/ChatComputer/models/llm/<name>.ccm`, made from Qwen's files when first needed (`chatcomputer/brain/models.py`); the system prompt's reading kept in `models/llm/cache/` |
| The model's engine | `/opt/chatcomputer/app/chatcomputer/brain/bin/chat-computer-model-<machine>` (built reproducibly from `os/model`; run from a copy in `models/llm/` when an update left it not runnable); serves `127.0.0.1:8765` |
| Initial settings from Windows setup | `/home/<user>/ChatComputer/settings.json` (JSON; keys below) |
| Voice models | `/home/<user>/ChatComputer/models/` (layout owned by `chatcomputer.voice.models`) |
| Windows partition | `/mnt/windows` (rw when NTFS is clean, else ro). A BitLocker drive is opened with Chat Computer's own recovery password (`/etc/chatcomputer/bitlocker-recovery-key`, from setup) as `/dev/mapper/chatcomputer-windows` |
| Root helper | `/usr/local/libexec/chatcomputer/chatcomputer-root`, sudoers: `<user> ALL=(root) NOPASSWD: /usr/local/libexec/chatcomputer/chatcomputer-root` |
| Where updates come from | `/etc/chatcomputer/update-source` (root, 0644): the folder Chat Computer Setup ran from, as a path below the top of Windows' drive (`Users/<you>/Downloads/ChatComputer`); see "Updating Chat Computer" |
| The version before an update | `/opt/chatcomputer/app.prev` (`app.failed` after an undo); `/var/lib/chatcomputer/update-pending` while a new version hasn't yet run 45 s |
| Logs, readable from Windows | `C:\Users\<you>\AppData\Local\ChatComputer\from-chatcomputer\` (`chatcomputer.log.txt`, `session.log.txt`, `info.json`), see "Runtime" |

`settings.json` keys written by the Windows setup (all optional): `user_name` (display name), `windows_user`
(Windows profile folder name, e.g. `rensh`), `timezone` (IANA), `locale`.

### Chat Computer exit codes (session supervisor contract)

* `0` normal exit → session script restarts Chat Computer after 1 s.
* `3` restart requested (after an update) → restart immediately.
* other → crash; restart with backoff (1, 2, 5 s). 3 crashes within 120 s → if the version is a new update
  (`/var/lib/chatcomputer/update-pending` exists), `sudo -n chatcomputer-root update-undo` puts the version before back
  and Chat Computer starts again at once; otherwise a recovery screen (full-screen `xterm` with the last log lines
  and `chatcomputer doctor` output; Enter retries). If it can't be shown, the session waits a minute before trying
  again rather than restarting in a tight loop.

### Drawing pages

WebKit draws with the graphics chip when the system offers one for drawing (a `/dev/dri/renderD*` device).
Without one (a virtual machine, or graphics the kernel can't drive yet) its GPU drawing crashes and leaves a
blank screen, so Chat Computer sets `WEBKIT_SKIA_ENABLE_CPU_RENDERING=1` and `WEBKIT_DISABLE_DMABUF_RENDERER=1`
before the window opens and draws with the processor. With NVIDIA's own driver it keeps the graphics chip but
sets `WEBKIT_DISABLE_DMABUF_RENDERER=1`, a common workaround for blank WebKit windows on that driver. If a page's
renderer process stops anyway, Chat Computer switches to the processor, loads the page again and puts the
conversation back as it was.

### Session

tty1 autologin (systemd getty override) → `~/.bash_profile` execs `startx` only on tty1 with no DISPLAY →
`~/.xinitrc` execs `/opt/chatcomputer/app/linux/session/chatcomputer-session` which: disables screen blanking/DPMS,
sets a black root window, starts `openbox` (no decorations, all windows maximized/fullscreen),
`unclutter-xfixes` (hide idle cursor), then supervises Chat Computer per the exit-code contract
(`python -P -m chatcomputer run`: `-P` keeps the home folder off the import path, so a `~/json.py` can't replace
part of Python). agetty runs with `--noissue` and the user has an empty `~/.hushlogin`, so no Ubuntu banner or
welcome text shows between the splash and Chat Computer.
PipeWire/WirePlumber run as systemd user services (dbus-user-session). Logs: `~/ChatComputer/logs/`.

### Root helper verbs (stdout = one JSON object, exit code 0 = success)

| Verb | Effect |
|---|---|
| `status` | `{"windows_mounted":bool,"windows_rw":bool,"windows_bitlocker":bool,"windows_boot_entry":bool,"efi":bool}` |
| `boot-windows-next` | One-time boot into Windows on next reboot: `efibootmgr --bootnext <Windows Boot Manager>`; fallback `grub-reboot "Windows"` |
| `boot-default windows\|chatcomputer` | What starts when the PC turns on: that system's UEFI entry first in `BootOrder` (Windows Boot Manager, or `ubuntu` for Chat Computer), the other entries kept in order; nothing removed |
| `mount-windows` | (Re)mount the Windows partition at /mnt/windows (rw if clean else ro; never `remove_hiberfile`) |
| `apt-install PKG...` | `apt-get install -y --no-remove` (an install never removes anything) of validated package names (`^[a-z0-9][a-z0-9+.-]{0,61}[a-z0-9+]$`), each one apt knows by exactly that name (`apt-cache pkgnames`): apt-get reads `name-` as "remove name", and a name that isn't a package's as a pattern |
| `apt-remove PKG...` | the same validation, each one installed (`dpkg-query`) and not one of Chat Computer's own packages (`/etc/chatcomputer/base-packages`, written by install-chatcomputer.sh); refused when the removal would also take packages that weren't named (they're listed in the error, from `apt-get -s remove`), so no removal takes Chat Computer's screen, voice or network with it |
| `system-update` | `apt-get update && apt-get -y upgrade` |
| `boot-macos-next` | Mac: macOS for the next start only (`asahi-bless --set-boot-macos --next`; if macOS was renamed, the one start-up volume that isn't Chat Computer) |
| `boot-default macos\|chatcomputer` | Mac: the start-up volume (`asahi-bless --set-boot-macos`, or `--set-boot Chat Computer`) |
| `bt-import-windows-keys` | Copy Bluetooth Classic link keys (e.g. AirPods) from the Windows SYSTEM registry hive into BlueZ, restart bluetooth |
| `set-timezone ZONE` | validated against /usr/share/zoneinfo |
| `wipe-install-secrets` | Remove `/chatcomputer/secrets` from a Chat Computer installer (FAT32 labelled `CHATCOMP`) still on the PC or plugged in |
| `update-app` | Update Chat Computer from the folder in `/etc/chatcomputer/update-source` (see "Updating Chat Computer"). `{"ok":true,"changed":N}` |
| `update-done` | The new version started well: remove `/var/lib/chatcomputer/update-pending` |
| `update-undo` | Put `/opt/chatcomputer/app.prev` back (the new version goes to `app.failed`) and remove the marker |

On Fedora (a Mac), `apt-install`, `apt-remove` and `system-update` use `dnf` (`install -y
--setopt=install_weak_deps=False`, `remove -y --setopt=clean_requirements_on_remove=False` after `rpm -q` and
`rpm -e --test` show the packages are installed and nothing else needs them, `upgrade -y --refresh`). Chat Computer asks the user (spoken yes/no) before calling any verb except `status`, `update-done`
(called after a new version has run 45 s) and `update-undo` (called by the session); `update-app` is asked on the
screen.

### Updating Chat Computer

Chat Computer Setup writes `/chatcomputer/update-source` on the installer: the folder it runs from, as a path below the
top of Windows' drive with each name spelled as the drive stores it (only when that folder is on Windows' own
drive). `finish.sh` copies it to `/etc/chatcomputer/update-source`, root's, so nothing Chat Computer runs can point updates
elsewhere. A newer Chat Computer put in that folder (downloaded again over the old one) is noticed when Chat Computer
starts (`chatcomputer/update.py`: the files of `chatcomputer/` and `linux/session/` compared with the installed ones) and
offered on the screen, answered by typing or saying yes or no. Yes runs `chatcomputer-root update-app`:

1. refused until Chat Computer has finished installing (`/var/lib/chatcomputer/base-installed`): until then the first
   start's finisher runs `install-chatcomputer.sh` as root, and that runs Chat Computer's own code;
2. the recorded path must be relative, without `.` or `..` parts or control characters, and lead (after
   `realpath`) into Windows' drive, to a folder with `chatcomputer/__init__.py` and `requirements-linux.txt`;
3. `requirements-linux.txt` must be the installed one: a version that needs other Python packages is
   installed with Chat Computer Setup;
4. only `chatcomputer/` and `linux/session/` are taken, the parts that run as the person. Nothing root runs (this
   helper, the Bluetooth importer, `linux/firstboot`, the system's settings) changes in an update, and root
   never runs Chat Computer's own code once installed (`install-chatcomputer.sh` checks the import path with
   `importlib.util.find_spec`, which finds the package without running it);
5. refused when those parts hold anything but plain files and folders (links could lead out of the folder) or
   a Python start-up hook (`sitecustomize.py`, `usercustomize.py`, `.pth`), checked before the copy and again
   on the copy;
6. `/opt/chatcomputer/app.next` is a copy of the installed app with those two parts replaced (compiled files left
   out), made root's, folders 0755, files 0644, `#!` files 0755; then the installed version becomes `app.prev`,
   `app.next` becomes `app`, and `/var/lib/chatcomputer/update-pending` is written.

Chat Computer then exits with code 3 and the session starts the new version. After it has run 45 s, Chat Computer calls
`update-done`; if it keeps crashing before that, the session calls `update-undo`. Claude can read the update
folder but not change it: the permission gate denies edits there and in `/opt/chatcomputer`, shell commands that
change something and name it or `cd` into it, and moving, copying into or trashing it with Chat Computer's own file
tools.

### Boot menu

`/etc/grub.d/07_chatcomputer` generates, before the stock entries: `Chat Computer` (default) and `Windows`
(chainload `/EFI/Microsoft/Boot/bootmgfw.efi`). `GRUB_DEFAULT=saved`, saved default = `Chat Computer`, timeout 3 s,
menu visible. Stock Ubuntu entries remain under "Advanced options" for recovery. os-prober disabled (our entry
replaces it). "Switch to Windows" from Chat Computer uses UEFI BootNext (BitLocker-friendly), not the GRUB chainload.
The Chat Computer entry boots `/boot/vmlinuz` and `/boot/initrd.img` (the links Ubuntu 24.04 keeps up to date; older
setups' `/vmlinuz` links at the root also work). Without a kernel link it adds neither entry, so GRUB's
fallback, the first entry, is Ubuntu's own (this same system), never Windows.

The boot splash is a Plymouth script theme (`/usr/share/plymouth/themes/chatcomputer`, chosen with the
`default.plymouth` alternative): a white screen with "Chat Computer" in black. `systemd-networkd` and its
wait-online service are turned off, since NetworkManager owns the network; otherwise every start would wait
two minutes for a network networkd doesn't manage.

## Windows setup → installer contract

No USB stick: setup puts the installer on the PC's own disk (`windows/lib/Installer.ps1`).

* **Where.** The space Chat Computer gets (shrunk from C:, a free area, or a separate disk the person erased) also
  holds its installer at first: a FAT32 partition labelled `CHATCOMP`, 6 GiB (`InstallerBytes`), GPT basic
  data type, without a drive letter, at the *end* of the largest free area on that disk (`Get-FreeExtents`,
  `Get-InstallerPlacement`: whole MiBs, the first and last MiB of the disk left alone). Chat Computer installs into
  the rest, right before it. Setup refuses ISOs bigger than 5 GiB.
* **What's on it.** The official Ubuntu Server 24.04.x ISO's files, 24.04.4 or newer (`MinPointRelease`: from
  there on its installer makes the account before `late-commands`, which `finish.sh` needs; up to 24.04.3 that
  was left to cloud-init at the first start) (copied from the ISO as Windows mounts it;
  its links, like `ubuntu -> .`, aren't folders there and aren't copied), plus:
  * `/chatcomputer/installer-<install-id>` (`id=<install-id>`): the mark, written first, so even a half-made
    installer can be found by "Remove Chat Computer" and, later, by Chat Computer;
  * `/autoinstall.yaml` (generated from `windows/autoinstall.template.yaml`). The installer's GRUB menu
    becomes "Install Chat Computer" (HWE kernel, default, 5 s) and "Install Chat Computer (older kernel)", both with
    `autoinstall subiquity-storage-version=2` on the kernel command line: unattended, and able to see free
    space next to existing partitions (the installer's default storage version 1 sees none on a disk with
    partitions);
  * `/chatcomputer/app/`: a copy of this repo, without `windows/`, `mac/`, `.venv` or caches;
  * `/chatcomputer/secrets/claude_login.json`: the full sign-in from `claude auth login --claudeai` (Claude Code's
    `.credentials.json`), which setup picks up by itself; or, when the person pasted a code instead,
    `/chatcomputer/secrets/claude_token`, a one-year token from `claude setup-token`;
  * `/chatcomputer/settings.json`: initial settings (keys above);
  * `/chatcomputer/update-source`: where setup runs from, for updates (see "Updating Chat Computer");
  * `/chatcomputer/guard.sh`, `network.sh`, `finish.sh`: copied from `windows/installer/`, with LF line endings.
* **How the PC starts it.** The ISO's signed boot loader goes to `\EFI\chatcomputer-setup\` on the EFI partition
  Windows starts from: `EFI/boot/bootx64.efi` (shim, signed by Microsoft's third-party UEFI CA) as
  `shimx64.efi`, with `grubx64.efi` (signed by Canonical) and `mmx64.efi`. A power-on entry "Chat Computer setup"
  starts it: `bcdedit /copy {bootmgr} /d "Chat Computer setup"` (so it points at the same EFI partition; the new
  `{id}` is found by comparing `bcdedit /enum firmware` before and after, never by reading Windows' language),
  `/set {id} path \EFI\chatcomputer-setup\shimx64.efi`, `/set {fwbootmgr} displayorder {id} /addlast`; and,
  once everything is ready, `/set {fwbootmgr} bootsequence {id}` (UEFI `BootNext`: the next start only). That
  GRUB's built-in configuration finds no `/boot/grub` on the EFI partition, so it looks for the partition with
  `/.disk/info`, the installer partition, and reads its menu from there. casper finds the live system on that
  partition too. Tested in QEMU with OVMF, Secure Boot and Microsoft's keys.
* **Order, and taking it back.** Nothing changes before "Go ahead". Then the power-on entry comes first: if the
  PC won't take one, setup stops with nothing else changed (and deletes a half-made copy). If any later step
  fails, setup takes back what it did (`Undo-InstallSteps`): the entry, the boot loader, the installer partition
  (only if it's still the one it made, same offset and size), and C:'s space (back to the size C: had); it says
  what it couldn't. Fast Startup is turned off last, just before the PC is told to start the installer.
* **Making room on C:** (`Invoke-MakeRoom`, after "Go ahead"). Chat Computer's space is the empty space right after
  C:, grown by shrinking C:: 70 GiB wanted (64 for Chat Computer, 6 for the installer), 46 at least. Windows can
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
area on the disk the installer is on, which is Chat Computer's space right before the installer partition. The
installer never reformats a disk. Shrinking C: or erasing a separate disk happens on Windows, after "Go
ahead". (subiquity only leaves out disks in use when a match names `size` or `ssd`; `install-media` picks the
disk holding the partition mounted at `/cdrom`.)

* The installer's boot menu checks first: if `/EFI/chatcomputer/installed-<install-id>` exists on any partition
  (GRUB `search --file`), this installer already installed Chat Computer here, and the menu offers only
  "Start Chat Computer (already installed)", which reads the installed system's boot menu
  (`configfile /EFI/ubuntu/grub.cfg` on the EFI partition). It doesn't chainload the installed shim: with
  Secure Boot on, GRUB 2.12 starts it without a file system and it can't find the rest of the boot loader
  (tested). Otherwise the menu offers the two install entries.
* `early-commands`: `sh /cdrom/chatcomputer/guard.sh <install-id>`, the second check for the same marker, in
  case the installer starts anyway. The installer has no `efibootmgr`, so the guard writes UEFI `BootNext`
  through efivarfs (the entry described `ubuntu`) and restarts; if it can't, it stops the installer with a
  message, and nothing is written.
* `late-commands`: `sh /cdrom/chatcomputer/finish.sh <user> <install-id>`. It:
  1. copies the app to `/target/opt/chatcomputer/app` (root-owned, files 0644, `#!` files 0755);
  2. copies the token (0600) and settings into `/target/home/<user>/ChatComputer/`, owned by the user, and the
     full sign-in for the phone, if setup made one (`secrets/claude_login.json`), to
     `~/ChatComputer/.engine/.credentials.json` (0600; `.engine` 0700), where Claude Code reads and refreshes it;
  3. writes the marker `/EFI/chatcomputer/installed-<install-id>` to the ESP: `id=<install-id>` and
     `root=<PARTUUID>` (Chat Computer's root partition, from `findmnt` + `blkid`), which "Remove Chat Computer" in
     Chat Computer Setup uses to delete exactly that partition;
  4. sets up the first start: `/etc/chatcomputer/install-user`, `/etc/chatcomputer/install-id`, `/etc/chatcomputer/update-source`
     (when setup wrote one; root's, 0644), and two services from `linux/firstboot`, enabled by their
     `multi-user.target.wants` links:
     * `chatcomputer-finish-install.service`, the safety net: until `/var/lib/chatcomputer/base-installed` exists, it
       runs before `getty@tty1`, retries `install-chatcomputer.sh` with growing waits (30 s doubling to 5 min,
       `dpkg --configure -a` before each try), then reboots;
     * `chatcomputer-reclaim-installer.service`, once (`/var/lib/chatcomputer/installer-reclaimed`): finds the
       installer partition (vfat, label `CHATCOMP`, `chatcomputer/installer-<this install's id>`, neither the root
       partition nor an EFI partition), overwrites the sign-in it carried in place, deletes it (`sfdisk
       --delete`), grows Chat Computer's partition and file system into the space when it sits right after it
       (`growpart`, `resize2fs`; up to 2 MiB between them counts as right after), deletes
       `\EFI\chatcomputer-setup` from every EFI partition and the power-on entries that start it
       (`efibootmgr -B`). If deleting fails it tries again at the next start;
  5. runs `curtin in-target -- bash /opt/chatcomputer/app/linux/install-chatcomputer.sh --user <user> --from-installer`,
     up to three tries. If all fail (usually no internet), it still exits 0, so the Ubuntu install ends
     normally and the safety net finishes the job on the first start.

"Remove Chat Computer from this PC" (Windows) also deletes installer partitions with a mark, the "Chat Computer setup"
entries and `\EFI\chatcomputer-setup`, so it cleans up an install that never finished too. Right after the typed
REMOVE it asks whether to keep the downloads (the Ubuntu ISO and the Agent SDK wheel, about 4 GB; default no):
kept, installing again finds them by their SHA-256 and doesn't download them again.

Tested end to end in QEMU (OVMF with Secure Boot and Microsoft's keys, an NVMe disk laid out as setup leaves a
PC): the "Chat Computer setup" entry starts the installer from its partition, the install finishes, Chat Computer starts,
and its first start deletes the installer partition, grows Chat Computer's partition into it, and removes
`\EFI\chatcomputer-setup` and the entry; Windows' partition is untouched.

`install-chatcomputer.sh` stops with exit status 1 at the first essential failure (apt packages, the venv,
pip), before later steps depend on it (for example handing networking to NetworkManager). On success it
writes `/var/lib/chatcomputer/base-installed` and removes the safety net's link.

Re-signing in later: Chat Computer Setup's "Sign Chat Computer in to Claude again" writes
`C:\Users\<you>\ChatComputer\claude_token` and, for the phone, `claude_login.json`. Chat Computer imports them at
start, before the file index runs, and deletes the Windows copies. Any file named `claude_token`, and the
engine's `.credentials.json`, are unreadable to Claude's tools.

First start: if the voice models are missing, Chat Computer downloads them in the background (about 600 MB),
retrying with backoff, then starts listening.

## Runtime (chatcomputer package)

* Model: one of Qwen's open models (Apache 2.0), as big as the computer's memory runs well
  (`brain/models.py` `pick`: 48 GB or more Qwen3.8-27B, 32 GB Qwen3.6-35B-A3B, 16 GB Qwen3.5-9B, 8 GB Qwen3.5-4B,
  less Qwen3.5-0.8B; the `model` setting can name one instead of `auto`) and its disk has room to get ready (the
  download and the converted file, with 4 GB to spare; one already there is kept). While the chosen one is got
  ready, the biggest one already there that the memory runs answers (`ModelService.serving`). `brain/models.json` pins each to one
  revision of Qwen's repository on Hugging Face with every file's size and SHA-256 (`scripts/model_manifest.py`
  writes it). On first need (`ModelService.prepare_in_background`) the files are downloaded (a download that
  stops carries on with a Range request), each checked against its SHA-256, converted by the engine
  (`chat-computer-model convert ... --bits 4` or 8) into `models/llm/<name>.ccm`, and the downloads removed. Until
  it's ready the controller answers what it can by itself (Wi-Fi, volume, switching systems) and says how far
  along the model is.
* The model's server (`brain/server.py`): Chat Computer's own engine, `os/model` (no outside code: the model's
  file format, the tokenizer, each model's chat template as code, int8 kernels for AVX2 and VNNI), started as
  `chat-computer-model serve <file> --port 8765 --cache models/llm/cache` in its own process group. It speaks
  OpenAI's chat completions API on 127.0.0.1 only, answers only requests whose Host is 127.0.0.1 or localhost and
  whose Origin, if any, is Chat Computer's own (chatcomputer.ai, the desktop app's), keeps what it has read of a
  conversation so each request reads only what's new, and keeps the reading of the first message (the system
  prompt and the tools: about 10,000 tokens) on disk across restarts.
* Brain (`brain/agent.py`): its own agent loop: the model writes (streamed: words, thinking, tool calls), each
  tool call goes through the permission gate, runs, and its result goes back, until the model answers without
  calling a tool (at most 60 calls a turn). Tools: Chat Computer's own (`chatcomputer/tools`, named
  `mcp__chatcomputer__<name>` for the gate and Actions) and the everyday ones in `brain/builtin.py` (Bash, Read,
  Write, Edit, Glob, Grep, WebFetch, WebSearch: the names and fields Claude Code's tools had, so the gate's rules
  apply as they did). Thinking follows the `effort` setting (off, low, medium, high, max). Each conversation is
  `~/ChatComputer/.engine/conversations/<id>.json`, its system prompt fixed when it starts, so it carries on
  after a restart and the model's server reuses what it read. Speaking over it stops the turn
  ("[Request interrupted by user]" is kept, as Claude Code kept it). The model reads text only: a tool's picture
  reaches it as a line saying there was one.
* Chat Computer tools are Python functions in the same process (screen, files, computer control, apps, system, memory,
  self-made tools), given to the model with their JSON schemas.
* The computer's everyday abilities, as Windows and macOS have them, through Linux's own services and never the
  root helper (`chatcomputer/platform/linux.py`; `tests/test_machine.py`):
  * USB drives and memory cards (`drives`): udisks2 opens a drive when it's plugged in and ejects it (powering a
    USB disk off when nothing on it is open), for the person at the computer without an administrator.
  * Printing (`printer`): CUPS finds printers on the Wi-Fi or USB by themselves (driverless: IPP Everywhere,
    AirPrint, ipp-usb). PDFs, photos and text go as they are; anything else becomes a PDF first: Word, Excel and
    PowerPoint files with LibreOffice when it's installed (`software install libreoffice-nogui`; the error says
    so when it isn't), Markdown and HTML as Chat Computer shows them, printed by WebKit to a PDF without a dialog
    (`GtkScreen.to_pdf`, in a view of its own that's never shown, on the paper the locale uses). `save_pdf` keeps
    such a PDF, of a file or of what's open.
  * The screen (`display`): brightness through logind (the laptop's backlight), else ddcutil (a monitor that
    takes DDC/CI); the night light as XRandR gamma ramps scaled by a black-body white point (`platform/gamma.py`),
    now or every evening; the keyboard layout (`setxkbmap`, checked against `evdev.lst`, applied again to a
    keyboard plugged in later).
  * Timers, reminders and alarms (`reminders`, `chatcomputer/schedule.py`): kept in `schedule.json`, rung by the
    controller with a chime and speech, shown as missed when the computer was off; the lock screen (`lock_screen`,
    `chatcomputer/lock.py`: a PIN kept as a PBKDF2 hash, wrong guesses slowed).
  * Screenshots (`screenshot`): the whole screen, or just what's open (`Screen.content_rect`), as a PNG in
    Pictures/Screenshots or the folder they name; undoable like any new file.
  * Backups (`backup`): their folders onto a USB drive or external disk, into "Chat Computer backup/<date
    time>", every file read back and compared; `backups.json` remembers when, where and how much, for `status`.
  * VPNs (`vpn`): NetworkManager's WireGuard and OpenVPN connections: listed, turned on and off, and added from
    the provider's `.conf` or `.ovpn` file (OpenVPN's needs `network-manager-openvpn`).
  * Updates (`software check`): what's waiting, from `apt list --upgradable` (dnf's cache on a Mac), without an
    administrator or a download; security updates install by themselves (unattended-upgrades).
  * The internet's speed (`wifi speed`): Cloudflare's speed test (speed.cloudflare.com): four round trips on an
    open connection for the delay, up to 25 MB or 8 s down, 4 MB up.
  * `system_status` also says the kind of computer (SMBIOS chassis type, or the Mac's model) and its battery
    (none on a desktop), what's keeping it busy (each program's processes together, measured over half a second,
    with the names people know them by), and the camera, with the programs that have it open.
* Typed or said: what's typed on this PC's keyboard is answered in writing only (`Brain.ask(typed=True)`, and the
  controller doesn't speak that turn); only what's said out loud gets a spoken
  answer. Its note ends with "typed" (`[time · screen: ... · typed]`), and the system prompt tells the model to
  answer that like a chat message, without an account of what it's about to do. Questions about a typed request are shown, not said, and end with "(Type yes or no.)", as they do
  whenever the microphone isn't listening. Things Chat Computer says unasked (the greeting, the model getting ready) are said
  only when sound plays on headphones; on the PC's or the monitor's speakers they are only shown.
* At the start (not in tests or `--fake`): a new version of Chat Computer to update to, then AirPods. AirPods are
  connected without a question (`Controller._airpods_quietly`, started first): nothing when sound already plays
  on headphones; else AirPods paired here, or else Windows' pairing copied over silently (`bt-import-windows-keys`,
  which also brings Windows' name for each device from `BTHPORT\Parameters\Devices\<mac>\Name` and marks it
  trusted, and restarts bluetoothd only when something changed). If they don't answer (in their case), a notice
  says to take them out, and they're tried every 3 s for 2 minutes. Only when no AirPods are known anywhere is
  "Connect your AirPods?" asked; yes looks for AirPods in pairing mode for a minute and pairs them. The model's
  `bluetooth` tool has `airpods`, the same in one step ("connect my AirPods"). The update comes first when
  Windows' drive is already mounted (yes restarts Chat Computer); while it is still on its way, the AirPods come
  first.
* Actions (`chatcomputer/done.py`, `~/ChatComputer/done.jsonl`): the brain reports each tool call when its result
  comes back (`BrainEvents.on_tool`, with the call's id); one that worked becomes a past-tense line ("Searched youtube.com for “lofi”", "Saved notes.txt"), unless
  it only looked at something (read_screen, status) or Chat Computer's own tool said in words that it failed
  ("Failed: ...", "I couldn't ..."). Reading and looking steps are kept but not listed. Each line keeps the
  message that asked for it: its id on the screen and in `transcript.jsonl` (written with the same id and
  time), its words and time. The conversation page lists them at the right (`CO.doneList`, `CO.done`); a click
  scrolls to that message, or, if it isn't on the page any more, asks for it (`goto` message →
  `Controller.go_back_to` → `History.earlier`: the transcript from that message to where the page starts, at
  most 300 messages) and scrolls there (`CO.earlier`). `recent_actions` reads the same list (words to look for,
  days back) for the model.
* Revert, on an item in Actions (beside its pin, under the pointer; swiped left on a touchscreen): what that action
  changed is taken back, its last change first, and that is listed as an action of its own ("Reverted “…”"),
  which can be reverted in turn (the action is back, and can be reverted again). Every change that can be taken
  back is a record in the journal (`chatcomputer/journal.py`, `~/ChatComputer/journal.jsonl`): files as
  before (`file_change`: the copy in `~/ChatComputer/undo` of what was overwritten, both ends of a move, where a
  trashed file came from), and other changes (`change`) with how to put them back, the tool call that does
  (`{"type": "tool", "tool": ..., "input": ...}`: the Wi-Fi's network, or the Wi-Fi on or off; a Bluetooth device
  paired, connected or disconnected; the volume, mute, where sound plays and which microphone listens; the
  brightness and the night light; a timer, reminder or alarm set (cancelled); a VPN on or off; the time zone;
  Chat Computer starting first again; Chat Computer's settings; programs installed or removed) or what was
  remembered (`memory_forget`, `memory_restore`). Each record names the
  tool call that made it: the model's calls run one at a time, and the permission gate sets `Journal.call` to each
  one's id (`tool_use_id`) just before it runs; a revert names itself
  (`journal.CALL`). Each line in `done.jsonl` keeps its call's records (`changes`), and a revert also the line
  it reverted (`reverts`). Taking a change back never destroys anything: a file it would overwrite is copied
  first, one it would remove goes to the trash; what it does is recorded too, marked as taking that record back
  (`by_undo`, `inverse_of`), and an `undo` record says the change was taken back (`undoes`), or, for a change
  that took another back, that the other is back (`redoes`): `journal.undone_ids` reads which are taken back
  now. The page gets `revert` on a line with changes not taken back (that the journal still has: past 5 MB it's
  cut to its last 5,000 records) and `reverted` on one with all taken back (by Revert, or by "undo that":
  `undo_last_change` only walks back changes not made by undoing), and the click posts `revert`
  (`Controller.revert`), the line saying "Reverting…" until `CO.doneStates` comes back. What couldn't be taken
  back (a file no longer there or something else in its place, a setting the computer refused) is said in a
  notice, with why. Not revertible:
  what only looked, opened a page or site, or can't be put back (a Wi-Fi network forgotten, power, printing).
  On the website and in the app (`docs/demo/host.js`), the volume, the text's size, a timer still open and the
  demo's saved file can be reverted the same way; the app keeps the marks with its Actions.
* Logs on Windows' drive (`chatcomputer/logcopy.py`): Chat Computer's own drive can't be opened from Windows, so the end
  of `chatcomputer.log` and `session.log` (1.5 MB each, CRLF line endings, anything like a sign-in, password or key
  blanked) and `info.json` (voice, microphone, AirTouch) are copied to
  `C:\Users\<you>\AppData\Local\ChatComputer\from-chatcomputer\`, only when Chat Computer Setup's folder is there: a
  minute after the start, every 10 minutes, just before a restart or shutdown, and when Chat Computer stops. The log
  also says, at the start, which graphics chips and drivers there are and the screens' sizes, and once a minute
  while typing, how long typed letters took to reach the screen (measured by the page: key to two frames later)
  and how often the window was busy for more than a tenth of a second. `session.log` says each screen's refresh
  rate and the X server's driver, cursor and input lines.
* Every risky action goes through the permission gate → spoken confirmation ("Should I go ahead?").
* A shell command runs without a question only when every part of it is known to only read
  (`classify_bash`): anything the gate can't analyse with certainty (several lines, substitutions, unusual
  redirections, options that write files or run programs, like `sed`'s `w`/`e` or `sort -o`) is asked about.
  `git` is always asked about: a folder's own git settings can make even `git status` run a program.
* Chat Computer's private folders (`secrets/` with the accounts' sign-ins, `.engine/` with the conversations, the
  browser's `web/` with its logins) can't be read, searched inside, copied, moved, trashed or shown on screen by
  Chat Computer's tools or the file tools, nor can other programs' settings (`/proc/*/environ`). Shell commands that name
  them, reach them with wildcards or `cd`, or search through a folder holding them (`grep -r`, `rg`) are
  blocked, even ones that would otherwise be asked about. The limit: a command the gate can't analyse is asked
  about, and once the person says yes it runs with their rights, which include reading those folders. Closing
  that too would need commands to run in a sandbox that hides the folders.
* Changing a file the computer runs commands from (dotfiles in the home folder, `~/.config/autostart`,
  `~/.local/bin` and the like) is asked about. The content layer doesn't let pages opened from files read other
  files with scripts (WebKit's file-access setting stays off), so a downloaded `.html` file can't read the
  sign-in.
* Screen: one full-screen GTK 3 window (`chatcomputer/ui/gtk_app.py`), a `Gtk.Overlay`: the conversation page
  (`ui/assets/chat.html`) fills it, and the content layer (a real browser with a persistent profile in
  `~/ChatComputer/web`) is laid over the space the page leaves for it ("hole", reported by chat.js): the middle,
  between the two sidebar cards, from the top down to the message box, inside a rounded frame (`#frame`) under
  its bar (`#frame-bar`: the name, `CO.mode("content", title)` then `CO.title` as the page's title comes, and a
  close button that posts `close_content`). The conversation (`#column`) is hidden while something is open: the
  screen is only ever three parts side by side, Messages, the middle and Actions. To start with, the middle fills
  the screen (`body.full`: no sidebars or switches, `--left`/`--panel` 0, the frame or the conversation and the
  message box across the width); Escape brings the sidebars in, and puts them away again: chat.js has it when
  the conversation page has the keys, the window
  (`_on_key`, calling `CO.toggleFull()`) when what's open has them, except a full-screen video's Escape, which
  leaves full screen. A website gets the keys when it's shown (typing and the keys that scroll are its own) and a
  page of Chat Computer's own (`native_uris`) leaves them in the message box; Page Up and Down there, and the
  arrow keys, Home and End while it's empty, scroll what's open (`scroll_open`, `__CO.scroll`). A question typed
  here and answered in words alone, with something open, brings the conversation back to show the answer
  (`Controller._show_words`: not when the turn used a tool in `SCREEN_TOOLS`, which work on what's open; what's
  open is put away, still playing, as `show_conversation` does). The browser demo does the same for every answer
  (`wordsOnly`, `putAway` in host.js: nothing done to what's open during the turn). While "/"
  searches, its list (`#results`) is in front: the frame ends above it, since the window lays the content layer
  over the page (in the browser demo, which draws what's open in the page itself, `CO.openInPage`, the list just
  overlaps it). The sites Messages reads have views of their own (kept in a `Gtk.OffscreenWindow`,
  moved into the hole when shown). The AirTouch layer (a `DrawingArea` with an empty input shape, so clicks go
  through) is on top. A conversation starts with the word logo alone in the middle (`#wordmark`: Atkinson
  Hyperlegible Next at 800, the words alone: there's no icon anywhere; shown while `body.intro`, which the page
  has from its first moment; after a second it fades away over 0.5 s, at once if something's said first) and
  then Chat Computer's first words come in as its answers do, as if just typed (`greet()` in chat.js: an
  `.ai.hello` message the page adds itself, not part of the transcript, a moment's thinking and then a few
  letters at a time through `CO.ai`, so they fade in; none if something was said first): `hello, world`, lower
  case with its comma as in Kernighan's C tutorial and K&R, alone for now (as the person asked: `GREETING` in
  chat.js is off, and with it the look-up of the weather for it, `CO.greets`; the website's host and the runtime
  look nothing up). With it on: under it the rest of the same greeting, a line going on from it ("and ...": one greeting, not two), as the Claude app's are: in the manner of
  its own (as claude.ai's code has them: each for a time of day, from 6, 12, 17 and 21 o'clock, some for a day of
  the week, "coffee and computer time?" only where a model answers), said on from hello, world ("and good evening
  to you, Ren"), with the name from `CO.settings` (on a Chat Computer `user_name`'s first word; on the website the
  name given with "call me ..."), and without it where there's none ("and good
  evening to you", "and greetings to you, whoever you are"); and, half the time where there's a place, one for
  the place and its weather now from `CO.here` ({place, code, temp, day}), from the world to the city: "and
  hello, Singapore! How was your day, Ren?", "and to you in rainy Singapore, Ren: stay dry". The place is where they said they live (`home_place`, or the
  website's "I live in ..."), else the time zone's city; its weather comes from Open-Meteo (`here.py` on a Chat
  Computer, every half hour; the website's host, at most once every half hour), and the greeting is chosen once
  the moment's thinking is over, so a weather looked up meanwhile can be in it. Never the one shown last time
  (`localStorage`, where there is one); again when the conversation is cleared. The empty
  message box (white, `--paper`, with a soft shadow) shows its placeholder, "Type anything", as the person asked
  (dimmed, `--dim`; gone once something is typed, `body.typed`), the cursor is in
  the box from the start, and its round send button is `--dim` until there's something to send, then black. No buttons beyond the sidebars' items,
  the sidebars' switches and what's open's close button: settings, the microphone (sound tool, `input`) and the
  model are changed by asking.
* The sidebars' switches (`.toggle` in chat.html): Messages' at the top left, Actions' at the top right; each
  icon is the screen with its sidebar marked, filled while it shows. Off, a sidebar leaves a 3.45 rem margin
  (`--rail`) with its switch. The page posts `panels`; the controller keeps them in `settings.left_panel` and
  `right_panel` (also set by asking; the old `bottom_panel` is dropped when settings load) and sends them back at
  the start (`CO.panels`). Only where the sidebars fit (`body.panel`), and not while the middle fills the
  screen.
* Sidebars: Messages at the left (`chatcomputer/messages`), Actions at the right (`done.py`), each a card
  (2 px `--bubble` border, 1.1 rem corners, 0.75 rem from the screen's edges and from the frame), each item
  pressable, and pinnable (`pins.py`, `~/ChatComputer/pins.json`: pinned items first, newest pin first). Actions
  shows the date and time (`#clock`, each minute) and each item's time at the right of its words, on their first line (a pinned one from another day: its day), as a message's is; under the pointer, the pin and Revert take the time's place.
* Messages (`chatcomputer/messages`): every account's messages in one list, newest first, kept in
  `~/ChatComputer/messages.db` (SQLite: `store.py`); accounts and their sign-ins in `secrets/accounts.json` (0600,
  the model's tools can't read it; `accounts.public()` is what the screen and the model get). Email over IMAP
  (`mail.py`: app passwords; Outlook.com through Microsoft's sign-in, OAuth 2 code + PKCE as a public client,
  XOAUTH2, `msauth.py`, which needs `settings.outlook_client_id` from an Entra app registration): read-only
  (`SELECT` read-only, `BODY.PEEK`), Gmail's Primary tab only (`X-GM-RAW "category:primary"`), new mail every
  `mail_check_minutes`. Telegram with Telethon (`telegram.py`: the person's own api_id and api_hash, phone, code,
  2FA password; chats and groups, not channels). WhatsApp and social sites (`web.py`, `assets/sources.js`): their
  own inbox pages in views with their own profile in `~/ChatComputer/sources`, read in an isolated script world
  (WhatsApp stays open; the others are looked at every 10 minutes). Sign-in forms (`CO.accountForm`) send what's
  typed straight to the Inbox, never to the model, and come back with what went wrong and what was typed (passwords
  aside). An email opened shows as the conversation with its sender (`render.thread_html`: the opened one and the
  latest few read in full over one connection, `Mailbox.read_many`).
* A web address typed or said alone (`answers.typed_address`: "youtube.com", "open bbc.co.uk", a whole link;
  a known ending or a country's two letters, but not those more often a file's, as notes.md) opens its site at
  once (`Controller._open_address`: `show_url`, Actions), without a turn of the model's; the next turn's note of the
  screen says what's open. The website does the same (`address()` in host.js), opening in a new tab the sites
  that won't show inside another page.
* "/" in the message box (`slash_search.py`): the words after it search the file index (`files_index.py`) as
  they're typed; matches show in a list above the box (matching letters bold), the arrow keys choose and Enter
  opens one between the sidebars (`render.file_view`); the last line asks Chat Computer to look instead.
* Touchscreen (`ui/touch.py`, `Gdk.event_handler_set`): every touch passes through `TouchGestures` before GTK
  delivers it. The iPad's system gestures (Apple's iPad User Guide): up from the bottom edge is Home
  (`close_content`); along the bottom or in from the sides, back and forward in the open view; down from the top
  centre, Messages (`CO.showMessages`); three fingers pinched closed copy (twice: cut), open paste, swiped left and
  right undo and redo (WebKit's editing commands on the focused view); four or five fingers pinched or swiped up
  are Home, swiped sideways back and forward. A touch starting at an edge is held back (its events copied) until
  it's clearly a swipe or not, then taken or replayed to GTK; a third finger cancels the first two on the page
  (a copy of each one's last event, as TOUCH_CANCEL). The onscreen keyboard (AirTouch's, the iPad's layout) comes
  up for a finger on a box to type in (chat.js and content.js post `keyboard`), unless a key on a real keyboard
  (not XTEST, which is Chat Computer typing) was pressed in the last 5 minutes; such a key puts it away. Its touches
  never reach the page: the key under the finger shows pressed and is typed on lift (`on_keyboard_action`, the
  controller types it with xdotool). In the conversation page (chat.js): pinch for the text size (`text_scale`,
  kept), hold a message to select it, double and triple taps for a word and a paragraph, hold the box and move to
  move the cursor, swipe a sidebar item left for Pin. The session maps each touchscreen (XI2 DirectTouch) to the
  primary output (`xinput map-to-output`) and hides the arrow on touch. On a touchscreen alone (`(hover: none)
  and (pointer: coarse)`: no mouse) the box takes the focus only when it's touched, Edit is pressed or a real key
  is typed, and after sending it lets go of it (`doneTyping`, posting `keyboard` `show: false`), so an onscreen
  keyboard goes down as in ChatGPT's app; what's typed in is at least 16px (phones' browsers zoom in on smaller
  boxes). The keyboard never covers the box: Safari on an iPhone lays its keyboard over the page without making
  the page smaller (Chrome on Android makes it smaller, `interactive-widget=resizes-content`), so `fitKeyboard`
  watches the visual viewport (what's on the screen) and lifts the box (`--kb`, its `bottom`) by however far its
  bottom is below it, and gives the conversation that much more room at its end. A phone (`body.phone`: that, on a screen under 600 CSS px across) is laid out as the Claude app is there
  (the website's: docs/demo/host.js sizes the text at an eighteenth of the screen's width, 16 to 24 px, from
  `screen.width`, so every browser on a phone gets the same): the box nearly the screen's width in two rows (a
  grid: the words; then the send arrow), nothing zooms
  (the viewport's `maximum-scale=1, user-scalable=no`, `touch-action: pan-x pan-y`, Safari's `gesture*` events
  and two-finger moves prevented), the conversation's own selecting and callout off (`user-select: none`), and
  holding a message opens `#hold-menu` over `#hold-shade` (the screen dimmed by 20% black, not blurred; the menu
  frosted): Copy, Send to (`navigator.share`, where there is one), Select text (`#reader`: the message alone over
  the screen, selectable as usual), and Edit for yours or Read aloud for Chat Computer's.
* Look: plain. Black text (#000000) on a warm off-white (`--ground: #F8F5F0`, white 8% dimmer with a faint warm
  tint) and nothing else: no dark mode, no grey text. Chosen from the research on long screen reading, as the user
  asked (theme.css cites it): dark on light reads best because the screen is bright; full white is rated glarier;
  19.3:1 instead of 21:1 changes no reading measure; very warm screens tired people most. The screen's own
  brightness matters far more than the colour, so dimming stays the screen's (`display`, the night light). The font is Atkinson Hyperlegible Next, with
  Atkinson Hyperlegible Mono for code. They ship in `ui/assets/fonts` (SIL OFL 1.1, from google/fonts; see
  its README), and `theme.css` loads them with `@font-face`. Pages written elsewhere get the fonts' file
  address (`render._theme_css`). `gtk_app.register_fonts` hands the folder to fontconfig (FcConfigAppFontAddDir)
  for the onscreen keyboard. Characters the fonts lack (CJK, Greek, Cyrillic, emoji) come from the system's fonts
  (Noto). Laid out like the Claude app, as the user asked: their words on the right in a light grey bubble
  (`--bubble: #E7E4DD`; black on it is 16.5:1; lines are `--line: #DDD9D1`), Chat Computer's flush left in one
  column, from the top of the screen down. Its words fade in as they come, each new piece from nothing to
  black in 400 ms at an even pace (the Claude app's timing, measured there), and a new message rises 10px into
  place. Under each message a row of what can be done with it, as in the Claude app: under Chat Computer's, copy, read
  aloud (`speak`, through the Speaker), good and bad (kept in `feedback.jsonl` with the question, for finding what
  to improve) and ask again, with when; under theirs, when, ask again, edit and copy. Below is a message box (a real `<textarea>`: chat.js sends on Enter, Shift+Enter
  for a new line; no placeholder, since that would be grey text) with nothing under it; while asleep, a line
  above it says how to wake Chat Computer. On a wide screen Messages runs down the left (with its buttons to
  connect email, chat apps and social media) and the right sidebar down the right, each 17.5 rem (the Claude app's
  sidebar's 280 px) against its edge of the screen from top to bottom, a thin line between it and the middle; the
  right one is two halves of the same height, Actions above and Chats below (each task's chat, newest first, the
  one on the screen marked; + starts a new one: `chats.py`). The column and the box centre in the rest.
* Size and width: the Claude app's, measured from it on a 1920 x 1080 screen at 100% display scaling: 16 px
  words on lines 22 px apart, a 924 px conversation with Chat Computer's words up to 865 px (about 120 characters) and
  yours in bubbles up to 85% of it, a 962 x 46 px message box 39 px above the bottom. Atkinson's letters at 16 px
  are as big as the Claude app's (an x-height of 8 px). `ui/layout.py` scales this for the monitor's physical
  size (EDID via GDK) and the viewing distance (estimated from the screen size, or set by voice), the way
  display scaling makes the Claude app bigger. Websites get the display-scaling zoom, not the bigger text.
  Everything is recomputed when the window, monitor or settings change. Settings: `text_size` (0 = auto), `text_scale`, `viewing_distance_cm`, `screen_inches`
  (0 = reported), `web_zoom` (0 = auto).
* Voice: PipeWire/AirPods, Silero VAD, faster-whisper (local), Kokoro TTS (local), barge-in.
* Input, in the order the person uses it: the touchscreen, a keyboard, the AirPods. AirTouch is off unless asked for (`airtouch: false`; `Controller._settings_moved_on` turned it off once
  for those who had it on).
* AirTouch (`chatcomputer/airtouch`): the hands, seen by the screen's camera, in a process of its own at nice 5
  (`worker.py`). All of it is Chat Computer's own code except the two networks: Google MediaPipe's palm detector
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
    something else moves the pointer (a mouse, or a click the model made); AirTouch then leaves the pointer
    alone until it has been still for 3 s.
* Keyboard and mouse: USB ones just work. The arrow shows when the mouse moves (unclutter hides it after 3 s
  still). Clicks reach the page: AirTouch's layer over the window (a GtkDrawingArea in a GtkOverlay) took them
  all, pass-through or not (GTK 3.24), until it was given an empty input shape. Keys go to the page too: on the
  conversation, into the message box (the page puts the focus back there after a click elsewhere); GTK keeps
  only the media keys and Ctrl+Shift+Q. `linux/session/chatcomputer-refresh` sets each connected screen to the
  fastest rate xrandr lists at its current size when X starts (X picks the screen's preferred mode, usually
  60 Hz; Windows keeps the 120/144 Hz chosen there), unless `~/ChatComputer/keep-refresh-rate` exists. A Bluetooth keyboard (`bluetoothctl info`: Icon input-keyboard) pairs through bluetoothctl's own
  agent as KeyboardDisplay, answering its questions: the passkey to type on the keyboard (or a PIN made up
  for an older one) is shown and spoken. Other devices pair as NoInputNoOutput.
* Speed (`tests/test_speed.py`, and the listener tests in `tests/test_voice.py`):
  * Start: the window comes first. `run_app` shows it, then makes the controller on the asyncio thread (the
    window module needs only the gate's `is_private`, which doesn't load the brain). The voice models load side by side and speech
    recognition gets a first run on silence right away (its first run is the slowest). The file index's first
    crawl waits 10 s.
  * Listening: an utterance ends after 0.7 s of silence (0.45 s for a yes/no), but its final transcription
    starts after 0.3 s and is used if no more speech came. Partial transcriptions run only while someone is
    talking, at most every 0.9 s or 1.5 times the last transcription's duration, whichever is longer, so the
    recognizer is free when the utterance ends.
  * Speaking: Kokoro uses one thread per core (onnxruntime's choice); a long first sentence is spoken from its
    first comma, `;` or `:` after 20 characters, once 50 have arrived. Whether sound plays on headphones (for
    barge-in) is asked on every 32 ms frame while Chat Computer talks: it's answered from a check (wpctl, about a
    quarter of a frame) made in the background every 5 s, not by running wpctl on the event loop each frame.
  * The file index reads documents' text (pypdf, python-docx, openpyxl, python-pptx: pure Python) in a process
    of its own, `TextReader`, at nice 10. In Chat Computer's own process they held the interpreter long enough for
    speech to skip (6 to 11 underflows in 6 s of speech under PipeWire; none since, and none from walking
    folders). A document a reader hangs on is given up after 60 s, and stopping Chat Computer doesn't wait for one.
  * Tools: `system_status` asks its eight parts side by side. Without a Bluetooth adapter (`/sys/class/bluetooth`
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
  * A request doesn't wait for the internet at all: the model is on the computer.
  * The model reads the system prompt and the tools (about 10,000 tokens) once: the engine keeps that reading
    on disk, so a restart picks it up in a second or two instead of reading it again (minutes on a small
    processor), and each request of a conversation reads only what's new since the last.
  * The conversation page scrolls like the Claude app's: what you send glides to the top of the screen and the
    answer comes in under it, so nothing new is ever below the screen (chat.js leaves room under the latest
    exchange, `#room`: the screen's height less what's under your message); an answer longer than the screen is
    followed down while you're at the end; scrolled up, it stays put until a new message. Chat Computer's words stream in
    as Markdown drawn incrementally: blocks that ended at a blank line
    (outside code) are drawn once, and only the block still being written is drawn again each frame. Captions
    over websites change once per 40 ms flush, not per streamed word.
  * Website steps wait for the page instead of a fixed pause (`Screen.settle`): a page that starts loading is
    waited for, then until nothing on it changed for 0.15 s (content.js's MutationObserver), at most 1.5 s.
    A fixed pause was too short for a new page (the model read the old one and had to look again) and too long
    for a menu.
  * Typing on the conversation is the page's own text box: no trip through Python per key.
  * Boot: `chatcomputer-windows-mount.service` waits for the disks itself (`udevadm settle` in `ExecStartPre`)
    instead of pulling in `systemd-udev-settle.service`, which comes before `sysinit.target` and so held up the
    whole start. If Windows' drive (or a Mac's shared drive) arrives after Chat Computer has started, Chat Computer takes
    it in then: its files in the search, and the next conversation's system prompt says where they are (at the
    start the first conversation waits up to 10 s for the drive).

## Mac

On a Mac with Apple silicon, Chat Computer is a second system next to macOS, as on a PC. Only Asahi Linux can start
another system on these Macs, so Chat Computer's Linux base there is Fedora Asahi Remix (the Asahi project's
flagship distribution), installed by the Asahi installer. That sets the Macs it works on: M1, M2, and M3, M3 Pro,
M3 Max (not M3 Ultra, M4, M5; on M3 for now without graphics acceleration, brightness or other screens, per
Asahi's feature tables). The runtime (`chatcomputer/`) is the same as on a PC; `chatcomputer/platform/linux.py` has
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
| Installer | Asahi installer v0.9.2 (`cdn.asahilinux.org`, SHA-256 pinned), run locally: `REPO_BASE` is setup's work folder (`os/<package>`), `installer_data.json` is Chat Computer's (one OS, Fedora's partitions exactly: EFI 500 MiB FAT32 with Fedora's volume id, Boot 1 GiB, Root expanding), `DISTRO=Chat Computer` |
| Partitions | macOS's container (shrunk), the start-up volume's container (`Chat Computer`, 2.5 GB, made by the installer), EFI (`EFI - CHAT`), Boot, Root, then the Chat Computer drive (exFAT, `Chat Computer`) in the space left |
| The drive macOS shares | `~/Mac` in Chat Computer: fstab `PARTUUID=<id> exfat uid,gid,umask=077,nofail`; the id (GPT partition GUID, diskutil's `DiskUUID`) is in `/etc/chatcomputer/mac-files-partuuid` |
| Sign-in left by "Sign in again" | `.chatcomputer/` on the Chat Computer drive; imported at start and deleted, like `C:\Users\<you>\ChatComputer` on a PC |

### Setup (`mac/setup.sh`, in macOS)

Menu: Install Chat Computer on this Mac, Send more files to Chat Computer, Sign Chat Computer in to Claude again, Choose
what starts when the Mac turns on, Remove Chat Computer from this Mac. Written for macOS's bash 3.2.

Install:

1. Checks: Apple silicon (`sysctl hw.optional.arm64`, so a Rosetta Terminal counts), the chip
   (`machdep.cpu.brand_string`: M1, M2, M3; M3 Ultra, M4, M5 refused with the reason), not a virtual Mac
   (`kern.hv_vmm_present`, allowed with `CHATCOMPUTER_PLAN_ONLY=1`), macOS 13.5 or newer, an admin account, no
   Chat Computer already on the disk (`find_install`), and the internet. On a Mac mini, Studio or Pro it says screens
   must use the HDMI port.
2. The folders to bring (Desktop, Documents, Downloads, Pictures, Movies, Music; sizes from `du -A`, so files
   kept only in iCloud count).
3. The space, as the installer works it out: macOS keeps what it uses plus 38 GB, never less than diskutil's
   `MinimumSizePreferred`, plus 1 GB. Chat Computer gets 60 GB when there's room (at least 30 GB); the Chat Computer
   drive gets the folders plus 10 % plus 10 GB. Plan-only stops after this.
4. Sign-in: the official Claude program from the `claude-agent-sdk` macOS wheel (pinned), `claude setup-token`
   in a throwaway config dir, the pasted token read with bracketed paste (it can span lines); optionally
   `claude auth login --claudeai` for the phone.
5. Wi-Fi: the network in use (`networksetup`, `ipconfig getsummary`), its password typed or taken from the
   keychain (`security find-generic-password -D 'AirPort network password'`).
6. "Go ahead?", then: download and check the installer and the image; add `chatcomputer.icns` to the image's zip
   with the installer's own Python (its new SHA-256 is noted next to it, so a second run doesn't download it
   again; if the icon can't be added, the copy of `installer_data.json` loses its `icon` line); run the
   installer under `expect` (`mac/drive-installer.exp`), which answers each question a second after it
   appears (the installer clears typed-ahead input before each one) and lets the person type their password
   to it. Exit codes: 2 resize failed, 3 not enough space, 4 a question it can't answer, 5 macOS too old, 6
   failed. The partitions it made come from its `installer.log`.
7. The Chat Computer drive: `diskutil addPartition <Root> ExFAT Chat Computer 0` (the space left), Spotlight off; the
   folders copied with `rsync -rt` and `COPYFILE_DISABLE=1`, without `.DS_Store`, `*.photoslibrary` or
   extended attributes (no `._` files).
8. The first start, on Chat Computer's EFI partition (`chatcomputer/`): `app.tar.gz` (`chatcomputer/`, `linux/`, the
   requirements and docs; `--no-xattrs`), `firstboot.sh`, `setup.env` (base64 values: account, name,
   recovery password, time zone, install id, the drive's id, Wi-Fi name and password), `secrets/` (token,
   phone sign-in), `settings.json`; Fedora's `EFI/fedora/grub.cfg` kept as `chatcomputer/fedora-grub.cfg` and
   replaced by `esp/grub-firststart.cfg` with the kernel and ids filled in.
9. The recovery password to `~/Documents/Chat Computer recovery password.txt`; the downloads deleted; the one step
   Apple requires (choosing Chat Computer in the startup options and finishing the installer's second step in
   1TR), shown to be photographed; shutdown.

Remove: copies the drive's files to `~/Chat Computer files` first if asked, typed REMOVE, `bless --setBoot` on
macOS's volume (`--stdinpass`), `diskutil apfs deleteContainer` on the start-up volume's container, `diskutil
eraseVolume free free` on the EFI, Boot, Root and drive partitions, then `diskutil apfs resizeContainer <macOS's>
0`. Choose what starts: `bless --setBoot --device` on Chat Computer's or macOS's volume. Sign in again: the token
and phone sign-in to the drive's `.chatcomputer/`.

### First start (`mac/esp/firstboot.sh`)

GRUB starts Fedora's kernel with `"systemd.run=/bin/sh /boot/efi/chatcomputer/firstboot.sh"`,
`systemd.unit=kernel-command-line.target`, `systemd.run_success_action=reboot` and
`systemd.run_failure_action=reboot`, instead of Fedora's first-start questions. Two details matter, both found
in an emulated Mac: the whole argument is in double quotes (GRUB passes it on in double quotes, which systemd
removes; quotes inside it would reach systemd as `\"`, which it doesn't read as quotes), and `systemd.unit` names
the target outright (systemd-run-generator's own `default.target` loses to the one Fedora's image has in /etc). firstboot.sh (POSIX sh, as root, SELinux enforcing): hostname `chatcomputer`;
the account (`useradd --groups wheel`, its password the recovery password, through `chpasswd`); time zone and
`LANG`; a NetworkManager keyfile for the Wi-Fi (0600); Chat Computer to `/opt/chatcomputer/app` (root-owned); the token,
phone sign-in and settings into `~/ChatComputer`; `/etc/chatcomputer/install-user`, `install-id`,
`mac-files-partuuid`; `chatcomputer-finish-install.service` enabled; `restorecon` on all of it. Only then: Fedora's
first-start questions off (`/etc/reconfigSys`, `initial-setup.service`), Fedora's `grub.cfg` back, and the
sign-in copies deleted from the EFI partition (FAT: readable by anyone who starts the Mac). On failure it counts
tries in `chatcomputer/tries`; at the third it puts Fedora's start back, so the Mac never loops.

The next start is Fedora's usual one: `chatcomputer-finish-install.service` runs `install-chatcomputer.sh --user <user>
--from-installer` with the internet until it works (as on a PC), then restarts. On Fedora that script installs
with `dnf --setopt=install_weak_deps=False`, WirePlumber 0.5 settings (SPA-JSON), firewalld without the ssh and
cockpit services, `dnf5-automatic` for security updates, no SSH server, the Plymouth theme
(`plymouth-set-default-theme -R`), GRUB without its menu (`GRUB_TIMEOUT=0`, `quiet rhgb`), the drive in fstab,
and `restorecon` at the end.

### Switching

* "Switch to macOS": `power(switch_to_macos)` → root helper `boot-macos-next`, then `systemctl reboot`. The
  next start after that is Chat Computer again.
* `start_by_default` (chatcomputer | other): root helper `boot-default chatcomputer|macos`.
* From macOS: setup's "Choose what starts", or the startup options (hold the power button).

### Tests

* `tests/test_mac_setup.py`: setup on a pretend Mac (stand-ins for `diskutil`, `plutil` and the rest in
  `tests/mac_pretend.py`), the real `drive-installer.exp` against `tests/mac_fake_installer.py` (the v0.9.2
  installer's questions, colours and terminal handling), first start against a pretend Fedora, and the pinned
  values agreeing with each other. Run with macOS's bash 3.2 (`CHATCOMPUTER_TEST_BASH`).
* `tests/mac_ci/run.sh` on GitHub's Macs (`.github/workflows/mac.yml`): plan-only on the real disk; a disk
  image laid out with the installer's own commands for finding, the drive, copying, the first start and
  removing; the real downloads; the real installer until it turns down a virtual Mac.
* `tests/test_mac.py`: `MacPlatform` and the runtime on a Mac; `linux/tests/test_install_dryrun.sh`: the Fedora
  install steps.
* An emulated Mac (QEMU `virt`, `-cpu neoverse-n1` for 16K pages, UEFI), its disk built from the real Fedora
  package with setup's own `write_first_start`: Fedora's 16K kernel starts, the first start runs and restarts,
  `chatcomputer-finish-install` installs everything (dnf, then pip on Python 3.14) and restarts, and Chat Computer's
  screen comes up, with SELinux enforcing and no denials. On 16K pages the Claude program runs and GTK,
  WebKit, numpy, onnxruntime, CTranslate2 and the voice packages load. It also found what the pretend tests
  couldn't: the first-start quoting, the default target, Fedora Asahi Remix's os-release ID, kokoro-onnx's
  Python bound, and GTK's cairo typelib (in `gobject-introspection`, only a weak dependency). (QEMU 8.2's
  `-cpu max` crashes this kernel early: an emulator problem, not a Mac one.)

Not yet tried on a real Mac. Unknowns: AirPods' microphone (Bluetooth HFP) under Asahi; AirPods paired in
Chat Computer may need pairing again in macOS (the same Mac, a new key); the Mac's own graphics (the emulated Mac
used a virtual one).
