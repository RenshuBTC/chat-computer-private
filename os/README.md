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
Computer's. Claude can search the web and read web pages (Anthropic runs those tools, so no browser is needed here),
and act on the computer through tools: see how it's doing, make the text bigger or smaller, set timers, read your
files on Windows' drive, and restart it. The PC's
UEFI firmware starts it. So far it has only run in a virtual PC (QEMU with OVMF, the
open-source UEFI firmware). It reaches the internet through the wired network chips most PCs have (Intel's
I219, I210, I211, I225 and I226, Realtek's RTL8111/8168 and RTL8125: written to the makers' datasheets and the
open-source drivers, not yet tried on a real PC), a USB network adapter, an Android phone sharing its connection
over USB, or a virtual machine's network card; other network chips need their drivers first (step 6, below). It
keeps the conversation across restarts, in a file on the PC's start-up partition that it writes through its own
NVMe and SATA drivers.

* `boot/` is the start-up stage, a UEFI program (`\EFI\BOOT\BOOTX64.EFI`) that carries the kernel inside
  it.
  * It asks the firmware for the screen, the monitor's description (EDID), the memory map, where the
    ACPI tables are and the time (the PC's clock), and reads Chat Computer's settings file
    (`\EFI\chatcomputer-own\settings.txt`) from the partition it was started from.
  * It notes which drive and partition that is, from the firmware's description of where it was started from
    (its device path): the PCI path to the NVMe or SATA controller, the NVMe namespace or the SATA port, and
    where the partition starts and ends. The kernel finds the drive again from that.
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
    gives drivers short waits that work with interrupts off, and counts the time since start-up when it runs at
    one speed in every power state (an "invariant" one, as processors of the last 15 years or so have), since
    ticks are lost while interrupts are off;
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
    * mice, read as each one's report descriptor says (`hid/`), or by the boot protocol if it can't be used. Their
      wheels scroll the conversation (`mouse.rs`); there is no pointer on the screen yet. A wireless receiver's
      keyboard and mouse both work;
    * network adapters: the configuration that offers a standard way to carry Ethernet, its bulk endpoints, and the
      way's own framing (`usb/net.rs`);
    * devices plugged in and out at any time. Unplugging a hub lets go of everything behind it.
  * the network cards there are drivers for (`netcard/`), and Chat Computer's own network stack on the first
    (`net/`):
    * virtio's (virtual machines');
    * Intel's gigabit cards with the e1000 interface: the 82540EM that QEMU, VirtualBox and VMware give virtual
      machines, QEMU's 82574L, and the I217, I218 and I219 built into Intel's chipsets (`e1000.rs`);
    * Intel's chips with "advanced" descriptors: the I210 and I211 (many AMD motherboards), the I225 and I226
      (2.5 Gb/s, most recent motherboards with an Intel port), the I350 and 82580, and the 82576 QEMU gives virtual
      machines (`igb.rs`). Intel publishes no datasheet for the I225 and I226, so their part follows what Linux's and
      FreeBSD's drivers agree on;
    * Realtek's RTL8111/8168 (8111E to 8111H and 8111FP) and RTL8125 (2.5 Gb/s, Killer's E3000 too): the most common
      wired chips on motherboards. Realtek publishes no datasheet for them either: this follows the RTL8168B's and
      what Linux's, Realtek's own, FreeBSD's and iPXE's drivers do (`realtek.rs`).

    Of these, only the virtual machines' have run so far; the others haven't been tried on a real PC, and the log
    says so when one starts. Without a card, a USB network adapter plugged in at any time takes over (USB CDC ECM and
    NCM, the USB standard's, and RNDIS, which most Android phones use to share their connection over USB);
  * the drive the PC started from (`storage/`): an NVMe SSD (`nvme.rs`), or a SATA drive on an AHCI controller
    (`ahci.rs`), the kinds nearly every PC starts from. On it, Chat Computer's memory file
    (`\EFI\chatcomputer-own\memory.bin`, which Windows' setup makes), found through Chat Computer's own FAT reader
    (`fat/`). The conversation and Claude's notes are kept there (`keep.rs`, in the format `remember/` defines, as
    JSON: `{"notes": [...], "messages": [...]}`) after every answer, and read back at start-up, so a restart carries
    on the same conversation. Only the memory file's own blocks are
    ever written, each read and checked first: never the file system, Windows' files or any other partition.
    Before a restart, the drive is told to write what it has cached;
  * Windows' drive, C:, read and never changed (`files.rs`, by Chat Computer's own NTFS reader, `ntfs/`): of the
    start-up drive's basic data partitions, the NTFS one with Windows' folders. Claude reads the person's files there
    through its tools (below). Links are followed on C: (a junction such as "My Documents", a symbolic link);
    OneDrive's files are read when they're downloaded to the PC, and said to be online only when they aren't. Word,
    Excel and PowerPoint files are read as their words (`office/`, through Chat Computer's own ZIP and DEFLATE
    reader, `zip/`); pictures and PDFs go to Claude whole, as the API takes them, for Claude to see. A drive
    BitLocker encrypts (many new laptops have Windows' "device encryption" on) is read decrypted (`bitlocker/`):
    at once if BitLocker keeps its key in the clear (suspended, or not switched on yet), else once the person types
    its recovery key in the message box (48 digits, from their Microsoft account at aka.ms/myrecoverykey). The key
    is taken out of the message there and never sent to Claude, shown or kept (so it's typed again after a restart);
    a wrong one is refused, naming the key ID that unlocks the drive. Not yet: a drive Windows is still encrypting
    as it's written to, files Windows compressed itself ("Compact OS", mostly its own), other drives (D:), and
    Office's files from before 2007;
  * the sound controller (`sound/hda.rs`): HD Audio, Intel's High Definition Audio, which nearly every PC's chipset
    has, Intel's and AMD's alike. It's reset and its codecs found on its link; the first codec with an audio function
    that has outputs is used (a graphics card's controller has only its screens' digital ones). Each codec
    describes its widgets and how they connect, so the way from an output converter (a DAC) to each output pin the
    PC's maker marked as speakers, headphones or a line out is found by following those connections, through
    mixers and selectors, as Linux's generic driver does; on the way, the right connections are chosen and the
    amplifiers unmuted at 0 dB (a mixer's other inputs, such as a microphone's loop back, muted), the pins switched on
    with their external amplifiers (EAPD), and built-in speakers left off while headphones are plugged in. A sound
    plays whole from memory on the first output stream, 48 kHz in 16 bits, and the stream stops after it. Commands
    go through the command and response rings, one at a time; nothing interrupts. Only QEMU's controllers and codecs
    have played it so far;
  * Claude, with the API key from the settings and Mozilla's certificate authorities (`assistant.rs`), and the
    tools Claude may use on this computer:
    * `computer_status`: what the computer is and how it's doing (processor, screen, firmware, Secure Boot,
      memory, how long it has been running, the date and time, the network, the keyboards and mice, the sound and the
      timers set) and what the own system can't do yet (`system.rs`);
    * `set_text_size`: the text 50% to 300% of the size chosen for this monitor;
    * `reminders`: timers, reminders and alarms, as the main Chat Computer has them (`schedule.rs`): for a time from
      now, or for a time of day when the settings give the local time's offset. When one is due, a line says so at
      the end of the conversation and its chime plays (`chime/`, the main Chat Computer's: two bell notes), again
      every 4 seconds for a minute (an alarm every 6 for ten minutes) until a key is pressed; a reminder rings once.
      They count the kernel's own time, so a clock that's out doesn't move them, and last until the PC restarts;
    * `set_volume`: how loud the chimes are, 0 to 100%;
    * `list_files`, `read_file` and `find_files`: a folder's files and folders on Windows' drive (with their sizes and
      when they changed; Windows' hidden system files, such as desktop.ini, left out as File Explorer leaves them
      out), a file's text (UTF-8, or UTF-16 as Notepad saves it, 60,000 bytes at a time, from any point), and the
      files and folders whose names have some words in them, from the person's folders (Users) down by default,
      hidden folders such as AppData passed by. A Word, Excel or PowerPoint file (or OpenDocument's) comes back as
      its words: a document's paragraphs with Markdown's headings, lists and tables; a workbook's sheets as CSV with
      each row's number and each column's letter, dates as dates, formulas after their values; a presentation's
      slides with their notes. A picture (JPEG, PNG, GIF or WebP, up to 7.4 MB and 8,000 pixels each way) or a PDF
      (up to 20 MB) goes to Claude whole, in the tool's result, so Claude sees it (a PDF's pages as words and as
      pictures). The newest of them are sent with each question, up to 28 MB and 20 of them; older ones are taken out,
      a line saying each was there, and the memory file keeps the conversation without them. A ZIP archive's files are
      listed. Anything else is said to be the kind it is;
    * `show_picture`: a photo, a screenshot or a drawing on Windows' drive (JPEG, PNG or GIF), shown to the person in
      the conversation where Claude's answer is, as wide as the column at most (and two thirds of its height), its
      corners rounded, turned the way the camera says (`picture/`). The last 12 shown keep their pixels;
    * `restart_computer`: a restart a few seconds after Claude's answer, which goes back to Windows. It uses the
      reset register the firmware names (ACPI's FADT), else the chipset's reset control (port 0xCF9), else the
      keyboard controller's reset line (`power.rs`);
    * `turn_off_computer`: the PC turned off a few seconds after Claude's answer, by ACPI's S5 ("soft off"). S5's
      sleep type is read from the firmware's DSDT, where it is plain data, and written with "sleep enable" to the
      PM1 control registers (or a hardware-reduced PC's sleep control register). Other systems also run the
      firmware's own preparation for it (AML, which Chat Computer doesn't run yet); nearly all PCs turn off without
      it, and if one doesn't, the screen says so;
    * `start_new_conversation`: when the person asks to start over, the conversation ends after Claude's answer. It
      is no longer sent or kept, and stays on the screen above a line until the next restart;
    * `remember` and `forget`: Claude's notes about the person (their name, what they like), kept in the memory file
      with the conversation and shown to Claude in every conversation after, new ones and restarts included. The
      screen says when Claude notes or forgets something. At most 40 notes, 8,000 characters in all.

    And the tools Anthropic runs itself: web search (`web_search`) and reading a web page (`web_fetch`). While Claude
    uses one, the screen says so by the thinking dot ("Searching the web"). If an organization has switched them off
    for its keys, the API refuses them, and Chat Computer asks again without them.

    Claude may use tools (or have its answer paused by the API, during a long search) up to 8 times for one message.
    Its answers go back into the conversation whole, thinking blocks, search results and citations exactly as they
    came, as the API asks.

    Every request asks for the API's prompt caching. When the next message (or a tool's result) comes within 5
    minutes, the conversation so far is read from Anthropic's cache, for a twentieth of the price. The system prompt
    changes at most once a day (it gives the date, not the time) so that the cache keeps matching. At most about
    600 KB of the conversation is sent; in a longer one the oldest questions are left out, a third at a time. They
    stay on the screen and in the memory file.

  Then the screen is the conversation (`chat.rs`), drawn in memory and copied to the screen where it changed.
  Before the first message it says what the PC is, how long Chat Computer has been running (by the kernel's own
  timer), which keyboards it found and whether Claude can be reached. Type, press Enter, and the message goes
  to Claude: the name looked up, a connection made, the certificate checked, the conversation so far sent, and
  the answer shown as it streams in, with a pulsing dot while Claude thinks. Claude's answers are Markdown, laid
  out as the main screen shows them (`rich.rs`): headings, bold, italics, lists and tasks, code in the code font,
  quotes, tables and rules. Esc stops an answer; Page Up and
  Page Down, or a mouse's wheel, scroll. Shift, Caps Lock, Backspace, Delete, the arrows, Home, End and key repeat
  work, in the US layout. Nothing typed or answered is written to the log.
* `net/` is Chat Computer's own network stack: Ethernet, ARP, IPv4, ICMP, UDP, DHCP, DNS and TCP (with
  retransmission timed as RFC 6298 says, slow start, fast retransmit and out-of-order segments kept). Tested on
  its own with lost, repeated and reordered packets.
* `crypto/` is Chat Computer's own cryptography: SHA-256, SHA-384 and SHA-512, HMAC, HKDF, AES-GCM,
  ChaCha20-Poly1305, X25519, ECDSA on P-256 and P-384, RSA (PKCS #1 v1.5 and PSS), and a generator of random
  bytes for keys, seeded from the processor. Tested against the standards' own test vectors and OpenSSL's.
* `tls/` is Chat Computer's own TLS 1.3 client, with its own certificate checks (chains, names, dates, what
  each certificate may be used for, name constraints) against Mozilla's certificate authorities. Tested against
  OpenSSL's server with every cipher and kind of key, and against the chain the real api.anthropic.com sends.
* `claude/` talks to Claude's Messages API: the request (Opus 5.5, effort "max", the tools, and the web tools
  Anthropic runs), HTTP/1.1, JSON and the streamed answer's events, put back together into whole content blocks
  (thinking, words and their citations, tool calls, web searches and their results) to send back.
* `font/` is Chat Computer's own TrueType reader and anti-aliased rasterizer. Coverage is exact and uses
  integer arithmetic only. It reads variable fonts too (fvar, avar and gvar: deltas, intermediate regions and
  inferred deltas; HVAR for the advances), so the bold comes from the same font file as the regular. And it reads
  a font's kerning (GPOS's pair adjustments, by pairs of glyphs and by classes, varying with the weight through
  GDEF's item variation store). It's tested against fontTools' reading of the same font, at the default weight and
  at others, and against HarfBuzz: every glyph's advance at three weights, and the kerning of every pair of 152
  characters at four (`font/reference.py`).
* `text/` sets lines of text in those fonts: which font has each character (the fallback fonts for what Atkinson
  Hyperlegible lacks), how wide lines are, where they wrap, and drawing them, with the kerning a browser applies
  (within a run of one font and face). Tested against where HarfBuzz puts each letter, at the regular weight and
  the bold (`text/reference.py`).
* `markdown/` reads the Markdown Claude writes into blocks and styled words, by the same rules as the main
  screen's renderer (`chatcomputer/ui/assets/md.js`). Its test runs md.js on each case (`markdown/gen/`) and
  checks the two read it the same way.
* `fat/` is Chat Computer's own reader of FAT12, FAT16 and FAT32 (the file system of every PC's start-up
  partition), long names included: it finds a file's blocks. Tested against volumes made by `mkfs.vfat` and
  filled by mtools, fragmented files among them.
* `ntfs/` is Chat Computer's own reader of NTFS, the file system of Windows' drive, which it never changes: the
  master file table's records (put back together from their update sequences, attributes kept in them or in runs
  of clusters, and in other records, through an attribute list, for a file in hundreds of pieces), folders' indexes
  (B-trees of names, compared in capitals as the volume's own table makes them, as Windows compares them), long and
  8.3 names, hard links, sparse files, compressed files (LZNT1), and reparse points: links (where they go), and
  placeholders, such as OneDrive's (whether the file's bytes are on the PC). Encrypted files (EFS) and files Windows
  compressed itself are recognized, not read; a volume BitLocker encrypts is read through `bitlocker/`. Every number read from the disk is
  checked first, so a damaged volume gives an error, not a crash. Tested against volumes made by Linux's
  `mkntfs` and filled by ntfs-3g (`ntfs/tests/make.py`), with 512-byte and 4 KiB sectors, 4 KiB and 64 KiB
  clusters: every file's bytes, against a hash of what ntfs-3g wrote, and every byte of the volume's first records
  changed in turn.
* `bitlocker/` reads a drive BitLocker encrypts, never writing it: its metadata (the first of its three copies
  whose CRC is right), its key protectors, the volume master key opened with the recovery key (stretched by a
  million rounds of SHA-256) or a password, or taken from a clear key, and then the full volume encryption key
  (AES-CCM); the metadata's own hash, sealed with the master key, must match. Sectors are decrypted with AES-XTS
  (Windows 10 and 11) or AES-CBC (7 and 8), NTFS's first sectors read from where BitLocker keeps them and its
  metadata's places read as zeros. Windows 7's Elephant diffuser, Vista's format and drives still being encrypted
  are recognized, not read. Its own AES uses tables (the crypto crate's computes its S-box in constant time, for
  TLS, and only encrypts). Tested against the volumes Windows encrypted that cryptsetup tests with (downloaded at a
  fixed commit and checked): 17 of them, unlocked by recovery key, password, second recovery key and clear key,
  each decrypting to exactly the SHA-256 cryptsetup's decryption gives, To Go's FAT volumes and 4 KiB sectors
  among them; reads of every size anywhere; wrong keys, damaged metadata copies, and metadata changed but its CRCs
  made right. `bitlocker/tests/encrypt.py` makes volumes for the virtual PCs' check.
* `zip/` is Chat Computer's own reader of ZIP archives (ZIP64 too) and DEFLATE, and zlib's wrapping of it (for
  PNG, later): what Word, Excel and PowerPoint files are made of. Its decoder works as zlib's "puff" does, with a
  table for short codes; what comes out is capped, so a "zip bomb" stops. Tested against zlib at every level and
  strategy, Python's zipfile and Info-ZIP's zip (data descriptors, ZIP64, code page 437 names), and damaged streams.
* `office/` reads the words in Word, Excel and PowerPoint files (Office Open XML) and OpenDocument's, through just
  enough XML of its own: a document's paragraphs (headings and lists by their styles, tables, text boxes once,
  tracked deletions and field codes left out, footnotes after), a workbook's cells (shared strings, number formats
  that make dates, times and percentages, both of Excel's date systems, formulas), a presentation's slides in order
  with their notes. Tested against documents made by python-docx, openpyxl and python-pptx, the same saved by
  LibreOffice, and documents written by hand with what programs write less often (`office/tests/make.py`).
* `picture/` decodes pictures for the screen: JPEG (baseline and progressive, any sampling, restart markers,
  Adobe's CMYK, EXIF's orientation) as libjpeg decodes it (its accurate integer inverse DCT, its "fancy" upsampling,
  its YCbCr arithmetic), a big one at a half, a quarter or an eighth of its size when that's enough, so a phone's
  48-megapixel photo needs a few megabytes; PNG (every colour type and depth, transparency, interlacing); and GIF's
  first frame. Shrunk to fit by averaging, in integers only. Tested against Pillow's (libjpeg-turbo's) decoding of
  pictures it wrote (within one level a sample) and PNGs written by its test, every depth and colour type
  (exactly), and damaged pictures.
* `chime/` makes the chimes of timers and reminders: bell notes, each a tone with its second and third harmonics,
  rising in 5 ms and dying away, the same as the main Chat Computer's (`chatcomputer/voice/chime.py`), at any
  sample rate. Made with sine recurrences (each sample from the two before), as the kernel's floating point is
  done in software. Tested against the main Chat Computer's formula worked out by numpy.
* `hid/` reads USB HID report descriptors: the description each mouse gives of its own reports (how many
  buttons, how fine its movement, whether it has a wheel, and where each is), by the HID specification and the way
  Linux reads the usages some devices declare out of order. Tested with QEMU's mouse and tablet, a wireless
  receiver's and a high-resolution wheel's layouts, descriptors cut short, and random ones.
* `remember/` is the memory file's format: 512-byte chunks, each tagged with the file's id (a random number setup
  chose) and the chunk's place, so nothing but this file is ever written; two copies written in turn, each with a
  hash, so a restart in the middle of a write leaves the last whole copy. Tested with writes cut short and with
  blocks that aren't the file's.
* `bootinfo/` defines what the start-up stage hands the kernel, once for both.
* `shared/mem.rs` holds Chat Computer's own `memcpy`, `memmove`, `memset` and `memcmp`.

The screen follows Chat Computer's look (`chatcomputer/ui/assets/theme.css` and `chat.css`):
* black text on the warm off-white background, your messages in light grey bubbles on the right, the
  message box a white card with a soft shadow at the bottom;
* Atkinson Hyperlegible Next (its regular and bold), and Atkinson Hyperlegible Mono for code. What they lack
  (arrows, check marks, maths, Greek and Cyrillic, emoji in black and white, box drawing in code) comes from two
  fallback fonts made from Google's Noto fonts (`fonts/`, built by `fonts/build.py`), as a browser would take it
  from the system's fonts. Chinese, Japanese and Korean show as boxes still;
* text sized by the same 0.3° x-height rule as `chatcomputer/ui/layout.py`, from the monitor's real size;
* a column at most 30 em wide.

Italics are the regular or bold glyphs slanted 12° (as browsers do for a font without its italic), which keeps
the italic font file out.

## Starting it on a PC

On a Windows PC, double-click `windows\Chat Computer Own System.cmd` (`windows/own-os.ps1`). No USB stick is
needed.

1. It checks the PC (read-only) and saves a list of its parts in `%LOCALAPPDATA%\ChatComputer\own-os\pc.txt`, so
   drivers can be written for exactly those chips.
2. It signs `release/chatcomputer.efi` with a key that Windows makes on that PC and won't let be copied off.
3. It asks for your Claude API key (paste it; it isn't shown or logged). Make a key just for this at
   console.anthropic.com, so you can switch it off there any time.
4. After you confirm, it puts the signed file in `\EFI\chatcomputer-own` on the partition the PC starts from,
   next to Windows' own files, with `settings.txt` (the key, how far the PC's clock is from UTC, as Windows
   keeps that clock in local time, and the memory file's id) and `memory.bin`, the memory file where Chat
   Computer keeps the conversation across restarts (4 MB, or 1 MB if the partition is short of room; running
   setup again keeps it, with the conversation in it). The key and the conversation are kept there as they are,
   so anyone who can read that partition can read them.
5. It adds "Chat Computer (own system)" to the end of the power-on menu and starts it once, at the next restart.
   The restart after that starts Windows.

With Secure Boot on, the PC first has to trust Chat Computer's key. That takes one approval in the PC's setup
screen: Secure Boot, then Key Management, then Authorized Signatures (db), then Append `ChatComputer.cer`. You
can turn Secure Boot off there instead. The script shows the steps and restarts into the setup screen.

Any USB keyboard works, plugged into the PC or into a hub (a monitor's or a dock's), and so do most
laptops' built-in keyboards. A keyboard that only connects by Bluetooth needs its cable for now:
Bluetooth comes at step 10.

Claude needs the internet. Of real PCs' network chips, the own kernel so far has drivers for the wired ones most
desktops and many laptops have: Intel's I219 (built into Intel's chipsets), I210, I211, I225 and I226, and Realtek's
RTL8111/8168 and RTL8125. They are written to the makers' datasheets and the open-source drivers, but none has been
tried on a real PC yet. On any PC, a USB network adapter (one that follows the USB standard) or an Android phone
sharing its connection over USB ("USB tethering") works too. Otherwise it shows what it found and says Claude can't
be reached yet, until that network chip's driver comes (step 6); the list of parts in `pc.txt` says which chip that
is.

To go back to Windows, hold the power button until the PC turns off, or ask Claude to restart it. The only thing
Chat Computer writes on any disk is its memory file, in place. Running the script again with `-Remove` (or choosing
Remove) takes the files (the conversation with them) and the entry off again, and deletes the key.

`release/chatcomputer.efi` is committed because Windows has no Rust compiler. The build is reproducible, and CI
rebuilds it and checks that it's the same file, byte for byte. `./release.sh` updates it.

## Building and trying it

You need Rust (`rustup`). The exact compiler version is pinned in `rust-toolchain.toml`, and rustup
fetches it. To try it you also need QEMU, OVMF and Pillow (`apt install qemu-system-x86 ovmf
python3-pil`).

```
./build.sh                       # builds out/esp/EFI/BOOT/BOOTX64.EFI (one file, about 2.8 MB)
cargo test -p font -p text -p net -p crypto -p tls -p claude -p fat -p ntfs -p zip -p office -p picture -p bitlocker -p remember -p markdown -p chime -p hid
                                 # the font engine and text setting, network stack, cryptography, TLS (needs
                                 # openssl), Claude API client, FAT reader (needs dosfstools and mtools), NTFS
                                 # reader (needs xz), ZIP, Office and picture readers (need python3 and Pillow),
                                 # BitLocker reader (needs curl, xz and python3-cryptography; BITLOCKER_IMAGES
                                 # names the test volumes' archive if it's here already), memory file, Markdown
                                 # reader, chimes and HID descriptors
cargo run -p claude --example ask   # Chat Computer's TLS and API code against the real api.anthropic.com
./check.py                       # starts it in a virtual PC; checks the screen and the clock
./check.py --cpu qemu64 --memory 4096 --size 1024x768
./check.py --crash page-fault    # a kernel that faults on purpose: checks it stops cleanly
./check.py --crash panic
./check.py --keyboard            # typing on a PS/2 keyboard, then USB keyboards on a port and behind a
                                 # hub (MSI-X, MSI, no interrupts), with Intel's and AMD's IOMMUs
                                 # present, plugged in and out
./check.py --network            # each kind of network card (virtio, Intel's e1000e, e1000 and igb, a USB
                                 # adapter by CDC ECM, plugged in and out, and by RNDIS): DHCP, DNS and a TCP
                                 # conversation (--card igb: just one of them)
./check.py --claude              # typing to a stand-in for Claude's API (HTTPS, with a test certificate
                                 # authority): the request, the streamed answer on the screen, Claude's tools
                                 # (the computer's status, a bigger text size, a restart), a web search with a
                                 # paused answer, the web tools refused, a failure
./check.py --remember            # the conversation and Claude's notes kept across a restart, on an NVMe SSD and
                                 # a SATA drive laid out like a PC's, then a new conversation started (the note
                                 # still shown to Claude), then the PC turned off;
                                 # nothing else on the disk changed (needs dosfstools, mtools and gdisk)
./check.py --sound               # a timer set by a stand-in for Claude, its chime recorded from each of QEMU's
                                 # HD Audio controllers and codecs and checked against the chime's own samples
./check.py --files               # Windows' drive read by Claude's tools (a stand-in for Claude asks): a folder
                                 # listed, text files read (UTF-8 and UTF-16, a long one in two parts), links
                                 # followed, OneDrive's files on the PC and online only, Word, Excel and
                                 # PowerPoint files as words, a photo and a PDF sent whole, a search, and a
                                 # photo shown on the screen (needs xz and gdisk)
./check.py --bitlocker           # the same drive encrypted with BitLocker: locked; a wrong recovery key typed
                                 # refused, the right one unlocking it, its files read; neither key reaching
                                 # Claude or the disk; and read at once with BitLocker's key in the clear (needs
                                 # python3-cryptography too)
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
5. Storage: ✓ NVMe and SATA (the drive the PC started from), ✓ the conversation kept across restarts in a file on
   the start-up partition, ✓ Windows' drive read (NTFS), for Claude to read the person's files. Checked in virtual
   PCs. ✓ Word, Excel and PowerPoint files' words, and pictures and PDFs for Claude to see. ✓ A drive BitLocker
   encrypts, with its recovery key. To do: a drive still being encrypted as it's written to, unlocking with the
   PC's TPM (for Chat Computer's own start-up, measured), files Windows compressed itself (XPRESS and LZX), other
   drives, Office's files from before 2007, Chat Computer's own file system for memory and logs, and keeping them
   encrypted.
6. Network: ✓ IPv4, ARP, DHCP, DNS and TCP; virtual machines' cards (virtio, e1000, e1000e, igb); the internet over
   a USB cable (USB Ethernet adapters and Android phones' tethering: CDC ECM, NCM and RNDIS); written, not yet tried
   on a real PC: Intel's I219, I210, I211, I225 and I226, and Realtek's RTL8111/8168 and RTL8125. To do: trying those
   on real PCs, Realtek's PCI Express PHY tuning and PHY firmware, then IPv6.
7. ✓ Encryption: SHA-2, HMAC and HKDF, X25519, AES-GCM, ChaCha20-Poly1305, ECDSA and RSA signatures,
   certificate checks against Mozilla's authorities, and TLS 1.3.
8. ✓ Claude: HTTPS and the conversation itself, streamed, with an API key (billed per use). Anthropic says
   subscription sign-in is for Claude Code and its own apps
   ([legal and compliance](https://code.claude.com/docs/en/legal-and-compliance)), and the own kernel doesn't
   run the Claude Code program (a Linux program), so it uses an API key.
9. The conversation screen: ✓ the conversation, wrapping and scrolling, ✓ the formatting Claude's answers use
   (Markdown, with bold from the variable font), ✓ kerning, as a browser sets the same font, ✓ pictures in the
   conversation (JPEG, PNG, GIF). To do: accents
   placed over letters typed as two characters (GPOS's mark positioning), and Chinese, Japanese and Korean.
10. Sound and AirPods: ✓ HD Audio's outputs, for the chimes of timers and reminders (checked with QEMU's
    controllers and codecs). To do: trying it on real PCs, microphones, USB audio, then Bluetooth (HCI over USB,
    L2CAP, A2DP and HFP, SBC and mSBC) for AirPods.
11. Speech: Chat Computer's own speech recognition and voice, running the models itself.
12. Power: ✓ restart, ✓ turning off (ACPI's S5, without the firmware's AML). To do: sleep; on laptops the battery
    and the lid. These need an AML interpreter.
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
   covers nearly every PC: USB (done), PS/2 keyboards (done), NVMe and SATA (done), the internet over USB (done), HD
   Audio and USB audio, and laptops' power (ACPI). Then the most common chips: Realtek's and Intel's
   wired network (written; to be tried on real PCs), then Intel's and MediaTek's Wi-Fi, the hardest single piece.
