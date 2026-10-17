# ClaudeOS's own operating system

This folder is ClaudeOS's own operating system, written from scratch. It covers every piece of software
from the moment the PC's firmware hands over to the screen. There is no Linux, no GRUB and no code from
other projects. It will replace the thin Linux base in `linux/` (today's ClaudeOS runs on Ubuntu or
Fedora). Until it can do everything that base does, the two live side by side.

## What it does today

The PC's UEFI firmware starts it. So far it has only run in a virtual PC: QEMU with OVMF, the
open-source UEFI firmware.

* `boot/` is the start-up stage, a UEFI program (`\EFI\BOOT\BOOTX64.EFI`) that carries the kernel inside
  it.
  * It asks the firmware for the screen, the monitor's description (EDID), the memory map and where the
    ACPI tables are.
  * It copies the kernel into memory and builds the kernel's first page tables.
  * It ends the firmware's services (ExitBootServices) and jumps to the kernel. Nothing in the firmware is
    called after that.
  * Its UEFI definitions are ClaudeOS's own, written from the specification.
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
    makes no sense is thrown away.

  Then it keeps the screen's clock current.
* `font/` is ClaudeOS's own TrueType reader and anti-aliased rasterizer. Coverage is exact and uses
  integer arithmetic only. It's tested against fontTools' reading of the same font.
* `bootinfo/` defines what the start-up stage hands the kernel, once for both.
* `shared/mem.rs` holds ClaudeOS's own `memcpy`, `memmove`, `memset` and `memcmp`.

The screen follows ClaudeOS's look:
* black text on white;
* Atkinson Hyperlegible Next;
* text sized by the same 0.3° x-height rule as `claudeos/ui/layout.py`, from the monitor's real size;
* a column at most 30 em wide.

## Starting it on a PC

On a Windows PC, double-click `windows\ClaudeOS Own System.cmd` (`windows/own-os.ps1`). No USB stick is
needed.

1. It checks the PC (read-only) and saves a list of its parts in `%LOCALAPPDATA%\ClaudeOS\own-os\pc.txt`, so
   drivers can be written for exactly those chips.
2. It signs `release/claudeos.efi` with a key that Windows makes on that PC and won't let be copied off.
3. After you confirm, it puts the signed file in `\EFI\claudeos-own` on the partition the PC starts from,
   next to Windows' own files.
4. It adds "ClaudeOS (own system)" to the end of the power-on menu and starts it once, at the next restart.
   The restart after that starts Windows.

With Secure Boot on, the PC first has to trust ClaudeOS's key. That takes one approval in the PC's setup
screen: Secure Boot, then Key Management, then Authorized Signatures (db), then Append `ClaudeOS.cer`. You
can turn Secure Boot off there instead. The script shows the steps and restarts into the setup screen.

To go back to Windows, hold the power button until the PC turns off. ClaudeOS doesn't write to any disk yet.
Running the script again with `-Remove` (or choosing Remove) takes the file and the entry off again, and
deletes the key.

`release/claudeos.efi` is committed because Windows has no Rust compiler. The build is reproducible, and CI
rebuilds it and checks that it's the same file, byte for byte. `./release.sh` updates it.

## Building and trying it

You need Rust (`rustup`). The exact compiler version is pinned in `rust-toolchain.toml`, and rustup
fetches it. To try it you also need QEMU, OVMF and Pillow (`apt install qemu-system-x86 ovmf
python3-pil`).

```
./build.sh                       # builds out/esp/EFI/BOOT/BOOTX64.EFI (one file, about 240 KB)
cargo test -p font               # the font engine's tests
./check.py                       # starts it in a virtual PC; checks the screen and the clock
./check.py --cpu qemu64 --memory 4096 --size 1024x768
./check.py --crash page-fault    # a kernel that faults on purpose: checks it stops cleanly
./check.py --crash panic
./check.py --secure-boot         # signed with a test key the virtual PC's firmware trusts, Secure Boot
                                 # enforced; and the same build unsigned must be refused
./release.sh                     # updates release/claudeos.efi
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

## What ClaudeOS owns, and what no operating system can own on a PC

ClaudeOS writes every piece of software that runs on the processor's main cores after the firmware hands
over. On any standard PC, some software stays outside. The PC this is first aimed at has an AMD Ryzen 7
7800X3D; on it, that software is:

* **The processor's security processor.** This is AMD's Platform Security Processor, or Intel's
  Management Engine on Intel PCs: a separate small processor running its maker's firmware. On AMD it
  starts first, and the main cores are never activated without it
  ([Wikipedia](https://en.wikipedia.org/wiki/AMD_Platform_Security_Processor)).
* **The motherboard's firmware (UEFI).**
  * It starts ClaudeOS.
  * It leaves System Management Mode code behind, which can pause any operating system and which the OS
    cannot disable ([Wikipedia](https://en.wikipedia.org/wiki/System_Management_Mode)).
  * Its ACPI tables include small power-management programs (AML) that an OS has to run for things like
    sleep. ClaudeOS reads the tables but doesn't run those programs yet.
* **CPU microcode**, from the processor's maker.
* **Firmware inside devices**: the SSD, graphics, network and Bluetooth chips, and the AirPods. Some chips
  need the OS to load their maker's firmware into them each time they power on
  ([Wikipedia](https://en.wikipedia.org/wiki/Binary_blob)). ClaudeOS would carry and load that firmware,
  not write it.

Secure Boot adds no one else's code to the chain. Either ClaudeOS's own key is approved once in the
motherboard's settings (added to the firmware's allowed list, db), or Secure Boot is off. Owning the
layers above would take hardware whose firmware can be replaced.

Some things are made by others but are not software between the hardware and ClaudeOS:

* **The Rust compiler** runs where ClaudeOS is built, not on the PC. It does build a small part of Rust's
  own library into ClaudeOS (`core` and `alloc`, for things like formatting numbers and collections on
  the heap). ClaudeOS replaces pieces of it with its own where that matters; `memcpy` and its relatives
  are already ClaudeOS's own.
* **Data**, such as the font (Atkinson Hyperlegible Next, by the Braille Institute, under the SIL Open
  Font License). ClaudeOS ignores the hinting programs inside the font. Later, speech models' weights
  will be data of this kind too.
* **Claude itself**, which runs in Anthropic's cloud.

## Roadmap

The steps come in the order ClaudeOS needs them. Each is tested in a virtual PC first, then on the real
PC.

1. ✓ The start-up stage, the kernel's core, interrupts and timer, the font engine and the first screen.
2. The real PC: start from its own start-up partition (no USB stick) and show the first screen on its
   monitor, with Secure Boot on and ClaudeOS's own key approved. Ready to try: see "Starting it on a PC".
3. Keyboard: the USB host controller (xHCI) and USB keyboards.
4. All processor cores, programs kept apart from each other and from the kernel, and a scheduler.
5. Storage: NVMe, and ClaudeOS's own file system for settings, memory and logs.
6. Network: the PC's wired network chip, then IPv4 and IPv6, DHCP, DNS and TCP.
7. Encryption: SHA-2, HMAC and HKDF, X25519, AES-GCM, ChaCha20-Poly1305, certificate checks, and
   TLS 1.3.
8. Claude: HTTPS and the conversation itself, streamed. There's a decision to make at this step.
   Anthropic says subscription sign-in is for Claude Code and its own apps
   ([legal and compliance](https://code.claude.com/docs/en/legal-and-compliance)). So either ClaudeOS
   runs the unmodified Claude Code program, which means it must run Linux programs, or it uses an API
   key, billed per use.
9. The conversation screen: kerning, scrolling, and the formatting Claude's answers use.
10. Sound and AirPods: Bluetooth (HCI over USB, L2CAP, A2DP and HFP, SBC and mSBC).
11. Speech: ClaudeOS's own speech recognition and voice, running the models itself.
12. Power: shut down, restart and sleep. Sleep needs an AML interpreter.
13. Updates: signed, with a way back.
14. Later: Wi-Fi, graphics acceleration, and showing websites.
