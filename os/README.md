# Chat Computer's own operating system

This folder is Chat Computer's own operating system, written from scratch. It covers every piece of software
from the moment the PC's firmware hands over to the screen. There is no Linux, no GRUB and no code from
other projects. It will replace the thin Linux base in `linux/` (today's Chat Computer runs on Ubuntu or
Fedora). Until it can do everything that base does, the two live side by side.

The aim: Chat Computer as deep as it can go while still working on everybody's PC or laptop, installed with a
download. The firmware stays the PC maker's on every PC (see "What Chat Computer owns" below). From the first
program the firmware starts, everything is Chat Computer's own. On a PC whose parts the own kernel can't drive
yet, Chat Computer's start-up program will start the Linux-based Chat Computer instead, so the same download works
everywhere, and PCs move to the own kernel as its drivers grow (see "Reaching everybody's PC").

## What it does today

You type to Claude on it: Opus 5.5 at its highest effort, with your own Claude API key, through Chat Computer's own
network stack, its own encryption and its own TLS 1.3, and the answer streams onto a screen that looks like Chat
Computer's. The PC's UEFI firmware starts it. So far it has only run in a virtual PC (QEMU with OVMF, the
open-source UEFI firmware), where it reaches the internet through the virtual network card; on a real PC it
needs a driver for that PC's network chip first (step 6, below).

* `boot/` is the start-up stage, a UEFI program (`\EFI\BOOT\BOOTX64.EFI`) that carries the kernel inside
  it.
  * It asks the firmware for the screen, the monitor's description (EDID), the memory map, where the
    ACPI tables are and the time (the PC's clock), and reads Chat Computer's settings file
    (`\EFI\chatcomputer-own\settings.txt`) from the partition it was started from.
  * It copies the kernel into memory and builds the kernel's first page tables.
  * It ends the firmware's services (ExitBootServices) and jumps to the kernel. Nothing in the firmware is
    called after that.
  * Its UEFI definitions are Chat Computer's own, written from the specification.
* `kernel/` is the kernel. It sets itself up in this order:
  * its log, on the serial port;
  * its own segment table and task state segment, with separate stacks for double faults, NMIs and
    machine checks;
  * its own interrupt table, with all 256 entries. A fault is written to the log, with every register,
    and shown on the screen;
  * a record of free memory, one bit per 4 KiB;
  * its own page tables:
    * all RAM at `0xFFFF800000000000`;
    * the kernel's code, read-only data and data, each with only the access it needs;
    * nothing at address 0, so a null pointer faults;
    * device memory uncached, and the framebuffer write-combined;
  * a heap;
  * the first screen, early, so that if anything after it stops the kernel, the screen says so;
  * the ACPI tables: processor cores, interrupt controllers and timers;
  * the local APIC. The old 8259 controllers and all I/O APIC inputs are masked;
  * a timer ticking 100 times a second. Its speed is measured against the ACPI power-management timer,
    else the HPET, else the old PIT. Every wait gives up after a few seconds, and a measurement that
    makes no sense is thrown away. The processor's time-stamp counter is measured at the same time; it
    gives drivers short waits that work with interrupts off;
  * IOMMUs (Intel VT-d, AMD-Vi) the firmware left on are switched off. Chat Computer doesn't use them yet, and
    a firmware that protects itself from devices this way would otherwise keep them from Chat Computer's
    memory, and drop their interrupts;
  * the PCI devices, through the PCI Express window (ACPI's MCFG table), else the old ports. Drivers
    have devices interrupt by message: MSI-X, else MSI;
  * the USB controllers (xHCI). Each is taken from the firmware the official way (USB legacy support
    hand-off, which also stops the firmware making a USB keyboard look like a PS/2 one), then stopped
    and reset before the memory the firmware used is reused. On Intel's 2012-2015 chipsets, the
    sockets are moved over from the older controller next to it;
  * the PS/2 keyboard controller, which most laptops' built-in keyboards use;
  * then, with interrupts on, the USB controllers start. Interrupts come by MSI-X or MSI. A
    controller that can do neither is checked for work every time the kernel wakes. What is plugged in
    is found and set up:
    * USB 2 ports are reset; USB 3 ports enable themselves (a warm reset if one doesn't);
    * hubs, up to five in a row. This includes the USB 2 side of USB 3 hubs, where keyboards appear, and
      the "transaction translator" fields that slow devices behind fast hubs need;
    * keyboards, in the boot protocol every keyboard supports, with their Caps Lock light;
    * devices plugged in and out at any time. Unplugging a hub lets go of everything behind it.
  * the network cards there are drivers for (so far virtio's, the virtual machines' card: its rings of
    buffers, interrupts by MSI-X), and Chat Computer's own network stack on the first (`net/`);
  * Claude, with the API key from the settings and Mozilla's certificate authorities (`assistant.rs`).

  Then the screen is the conversation (`chat.rs`), drawn in memory and copied to the screen where it changed.
  Before the first message it says what the PC is, how long Chat Computer has been running (by the kernel's own
  timer), which keyboards it found and whether Claude can be reached. Type, press Enter, and the message goes
  to Claude: the name looked up, a connection made, the certificate checked, the conversation so far sent, and
  the answer shown as it streams in, with a pulsing dot while Claude thinks. Esc stops an answer; Page Up and
  Page Down scroll. Shift, Caps Lock, Backspace, Delete, the arrows, Home, End and key repeat work, in the US
  layout. Nothing typed or answered is written to the log.
* `net/` is Chat Computer's own network stack: Ethernet, ARP, IPv4, ICMP, UDP, DHCP, DNS and TCP (with
  retransmission timed as RFC 6298 says, slow start, fast retransmit and out-of-order segments kept). Tested on
  its own with lost, repeated and reordered packets.
* `crypto/` is Chat Computer's own cryptography: SHA-256, SHA-384 and SHA-512, HMAC, HKDF, AES-GCM,
  ChaCha20-Poly1305, X25519, ECDSA on P-256 and P-384, RSA (PKCS #1 v1.5 and PSS), and a generator of random
  bytes for keys, seeded from the processor. Tested against the standards' own test vectors and OpenSSL's.
* `tls/` is Chat Computer's own TLS 1.3 client, with its own certificate checks (chains, names, dates, what
  each certificate may be used for, name constraints) against Mozilla's certificate authorities. Tested against
  OpenSSL's server with every cipher and kind of key, and against the chain the real api.anthropic.com sends.
* `claude/` talks to Claude's Messages API: the request (Opus 5.5, effort "max"), HTTP/1.1, JSON and the
  streamed answer's events.
* `font/` is Chat Computer's own TrueType reader and anti-aliased rasterizer. Coverage is exact and uses
  integer arithmetic only. It's tested against fontTools' reading of the same font.
* `bootinfo/` defines what the start-up stage hands the kernel, once for both.
* `shared/mem.rs` holds Chat Computer's own `memcpy`, `memmove`, `memset` and `memcmp`.

The screen follows Chat Computer's look (`chatcomputer/ui/assets/theme.css` and `chat.css`):
* black text on the warm off-white background, your messages in light grey bubbles on the right, the
  message box a white card with a soft shadow at the bottom;
* Atkinson Hyperlegible Next;
* text sized by the same 0.3° x-height rule as `chatcomputer/ui/layout.py`, from the monitor's real size;
* a column at most 30 em wide.

Claude is asked to write plain text there, as the screen doesn't show Markdown yet (marks it uses anyway, like
`**`, are taken out).

## Starting it on a PC

On a Windows PC, double-click `windows\Chat Computer Own System.cmd` (`windows/own-os.ps1`). No USB stick is
needed.

1. It checks the PC (read-only) and saves a list of its parts in `%LOCALAPPDATA%\ChatComputer\own-os\pc.txt`, so
   drivers can be written for exactly those chips.
2. It signs `release/chatcomputer.efi` with a key that Windows makes on that PC and won't let be copied off.
3. It asks for your Claude API key (paste it; it isn't shown or logged). Make a key just for this at
   console.anthropic.com, so you can switch it off there any time.
4. After you confirm, it puts the signed file in `\EFI\chatcomputer-own` on the partition the PC starts from,
   next to Windows' own files, with `settings.txt`: the key, and how far the PC's clock is from UTC (Windows
   keeps that clock in local time). The key is kept there as it is, so anyone who can read that partition can
   read it.
5. It adds "Chat Computer (own system)" to the end of the power-on menu and starts it once, at the next restart.
   The restart after that starts Windows.

With Secure Boot on, the PC first has to trust Chat Computer's key. That takes one approval in the PC's setup
screen: Secure Boot, then Key Management, then Authorized Signatures (db), then Append `ChatComputer.cer`. You
can turn Secure Boot off there instead. The script shows the steps and restarts into the setup screen.

Any USB keyboard works, plugged into the PC or into a hub (a monitor's or a dock's), and so do most
laptops' built-in keyboards. A keyboard that only connects by Bluetooth needs its cable for now:
Bluetooth comes at step 10.

Claude needs the internet, and so far the own kernel has a driver only for virtual machines' network card. On
a real PC it shows what it found, and says Claude can't be reached yet, until its network chip's driver comes
(step 6). The list of parts in `pc.txt` says which chip that is.

To go back to Windows, hold the power button until the PC turns off. Chat Computer doesn't write to any disk yet.
Running the script again with `-Remove` (or choosing Remove) takes the file and the entry off again, and
deletes the key.

`release/chatcomputer.efi` is committed because Windows has no Rust compiler. The build is reproducible, and CI
rebuilds it and checks that it's the same file, byte for byte. `./release.sh` updates it.

## Building and trying it

You need Rust (`rustup`). The exact compiler version is pinned in `rust-toolchain.toml`, and rustup
fetches it. To try it you also need QEMU, OVMF and Pillow (`apt install qemu-system-x86 ovmf
python3-pil`).

```
./build.sh                       # builds out/esp/EFI/BOOT/BOOTX64.EFI (one file, about 750 KB)
cargo test -p font -p net -p crypto -p tls -p claude   # the font engine, network stack, cryptography, TLS
                                 # (needs openssl) and Claude API client
cargo run -p claude --example ask   # Chat Computer's TLS and API code against the real api.anthropic.com
./check.py                       # starts it in a virtual PC; checks the screen and the clock
./check.py --cpu qemu64 --memory 4096 --size 1024x768
./check.py --crash page-fault    # a kernel that faults on purpose: checks it stops cleanly
./check.py --crash panic
./check.py --keyboard            # typing on a PS/2 keyboard, then USB keyboards on a port and behind a
                                 # hub (MSI-X, MSI, no interrupts), with Intel's and AMD's IOMMUs
                                 # present, plugged in and out
./check.py --network            # the virtual network card, DHCP, DNS and a TCP conversation
./check.py --claude              # typing to a stand-in for Claude's API (HTTPS, with a test certificate
                                 # authority): the request, the streamed answer on the screen, a failure
./check.py --secure-boot         # signed with a test key the virtual PC's firmware trusts, Secure Boot
                                 # enforced; and the same build unsigned must be refused
./release.sh                     # updates release/chatcomputer.efi
```

For `--secure-boot` you also need `sbsigntool` and `virt-firmware` (`pipx install virt-firmware`).

`check.py` leaves pictures of the screen and the kernel's log in `out/`. To watch it in a window:

```
cp /usr/share/OVMF/OVMF_VARS_4M.fd /tmp/vars.fd
qemu-system-x86_64 -machine q35 -m 512 -serial stdio \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,file=/tmp/vars.fd \
  -drive format=raw,file=fat:rw:out/esp
```

## What Chat Computer owns, and what no operating system can own on a PC

Chat Computer writes every piece of software that runs on the processor's main cores after the firmware hands
over. On any standard PC, some software stays outside. The PC this is first aimed at has an AMD Ryzen 7
7800X3D; on it, that software is:

* **The processor's security processor.** This is AMD's Platform Security Processor, or Intel's
  Management Engine on Intel PCs: a separate small processor running its maker's firmware. On AMD it
  starts first, and the main cores are never activated without it
  ([Wikipedia](https://en.wikipedia.org/wiki/AMD_Platform_Security_Processor)).
* **The motherboard's firmware (UEFI).**
  * It starts Chat Computer.
  * It leaves System Management Mode code behind, which can pause any operating system and which the OS
    cannot disable ([Wikipedia](https://en.wikipedia.org/wiki/System_Management_Mode)).
  * Its ACPI tables include small power-management programs (AML) that an OS has to run for things like
    sleep. Chat Computer reads the tables but doesn't run those programs yet.
* **CPU microcode**, from the processor's maker.
* **Firmware inside devices**: the SSD, graphics, network and Bluetooth chips, and the AirPods. Some chips
  need the OS to load their maker's firmware into them each time they power on
  ([Wikipedia](https://en.wikipedia.org/wiki/Binary_blob)). Chat Computer would carry and load that firmware,
  not write it.

The firmware can't be replaced on everybody's PC, so Chat Computer doesn't try:

* Many PCs, most laptops among them, refuse firmware their maker didn't sign. Intel's Boot Guard has the
  maker fuse its key into the chipset ([Phoronix](https://www.phoronix.com/news/Intel-Boot-Guard-Kills-Coreboot)).
  AMD's Platform Secure Boot ties the processor to the maker's firmware key
  ([ServeTheHome](https://www.servethehome.com/lenovo-vendor-locking-ryzen-based-systems-with-amd-psb/)).
* Replacement firmware such as coreboot exists only for the boards someone has adapted it to, and it
  still needs Intel's or AMD's own closed parts.
* Replacing firmware can leave a PC unable to start, and often needs a clip on the chip. That can't be
  a download.

Secure Boot, today, adds no one else's code to the chain. Either Chat Computer's own key is approved once in
the motherboard's settings (added to the firmware's allowed list, db), or Secure Boot is off. That is
too hard for most people, so the plan for everybody's PC is the first step Linux distributions use (see
"Reaching everybody's PC"). It puts one small program that isn't Chat Computer's own, shim, at the start of
the chain. The approved-key way stays, for anyone who wants none.

Some things are made by others but are not software between the hardware and Chat Computer:

* **The Rust compiler** runs where Chat Computer is built, not on the PC. It does build a small part of Rust's
  own library into Chat Computer (`core` and `alloc`, for things like formatting numbers and collections on
  the heap). Chat Computer replaces pieces of it with its own where that matters; `memcpy` and its relatives
  are already Chat Computer's own.
* **Data**, such as the font (Atkinson Hyperlegible Next, by the Braille Institute, under the SIL Open
  Font License), and the list of certificate authorities Chat Computer trusts, which is Mozilla's
  (`tls/roots.txt`). Chat Computer ignores the hinting programs inside the font. Later, speech models' weights
  will be data of this kind too.
* **Claude itself**, which runs in Anthropic's cloud.

## Roadmap

The steps come in the order Chat Computer needs them. Each is tested in a virtual PC first, then on the real
PC.

1. ✓ The start-up stage, the kernel's core, interrupts and timer, the font engine and the first screen.
2. The real PC: start from its own start-up partition (no USB stick) and show the first screen on its
   monitor, with Secure Boot on and Chat Computer's own key approved. Ready to try: see "Starting it on a PC".
3. ✓ Keyboard: USB (the xHCI controllers, hubs and keyboards) and PS/2 (laptops' built-in keyboards).
   Checked in virtual PCs; on the real PC with step 2.
4. All processor cores, programs kept apart from each other and from the kernel, and a scheduler.
5. Storage: NVMe and SATA, and Chat Computer's own file system for settings, memory and logs.
6. Network: ✓ IPv4, ARP, DHCP, DNS and TCP, and the virtual machines' card (virtio). To do: the internet
   over a USB cable (a phone's tethering, USB Ethernet adapters: standard, so one driver covers most), then the
   common wired chips (Realtek's and Intel's), then IPv6.
7. ✓ Encryption: SHA-2, HMAC and HKDF, X25519, AES-GCM, ChaCha20-Poly1305, ECDSA and RSA signatures,
   certificate checks against Mozilla's authorities, and TLS 1.3.
8. ✓ Claude: HTTPS and the conversation itself, streamed, with an API key (billed per use). Anthropic says
   subscription sign-in is for Claude Code and its own apps
   ([legal and compliance](https://code.claude.com/docs/en/legal-and-compliance)), and the own kernel doesn't
   run the Claude Code program (a Linux program), so it uses an API key.
9. The conversation screen: ✓ the conversation, wrapping and scrolling. To do: kerning, and the formatting
   Claude's answers use (Markdown).
10. Sound and AirPods: Bluetooth (HCI over USB, L2CAP, A2DP and HFP, SBC and mSBC).
11. Speech: Chat Computer's own speech recognition and voice, running the models itself.
12. Power: shut down, restart and sleep; on laptops the battery and the lid. These need an AML
    interpreter.
13. Updates: signed, with a way back.
14. Later: Wi-Fi (Intel's and MediaTek's chips, which most laptops have), touchpads, graphics
    acceleration, and showing websites.

## Reaching everybody's PC

Three things stand between the steps above and a download that works on any PC or laptop:

1. **Secure Boot with one confirmation.** PCs ship with Secure Boot on, and their firmware then only
   starts programs signed with a key it trusts, which Chat Computer's own key isn't. The plan is the route
   Linux distributions take. Shim, a small program Microsoft signs, starts Chat Computer's own start-up
   program once the person has confirmed Chat Computer's key, once, on a blue screen at the next restart.
   Setup on Windows asks for that confirmation, so there is nothing to find in the PC's settings.
   (Microsoft could later sign Chat Computer's own start-up program directly. That needs a registered
   organization with an EV code-signing certificate, and Microsoft may still require the shim route:
   [Microsoft's requirements](https://learn.microsoft.com/windows-hardware/drivers/dashboard/file-signing-reqs).)
2. **One download installs both.** Chat Computer's start-up program starts the own kernel when the kernel
   can drive everything the PC needs (screen, keyboard, storage and the internet), and the Linux-based
   Chat Computer otherwise. Both look and work the same to the person.
3. **Drivers, in order of how many PCs each one covers.** Standard interfaces first, where one driver
   covers nearly every PC: USB (done), PS/2 keyboards (done), NVMe and SATA, the internet over USB, HD
   Audio and USB audio, and laptops' power (ACPI). Then the most common chips: Realtek's and Intel's
   wired network, then Intel's and MediaTek's Wi-Fi, the hardest single piece.
