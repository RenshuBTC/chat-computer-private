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

You type to Chat Computer on it, and its own model answers, on the PC itself: an open model of Qwen's, run by Chat
Computer's own engine (`model/`), from the file Chat Computer's app gets ready on Windows, read into memory as it
starts. There's no account, no internet needed and nothing to pay, and nothing you type leaves the PC. The answer
streams onto a screen that looks like Chat Computer's. The model acts on the computer through tools: it sees how it's
doing, makes the text bigger or smaller, sets timers, reads your files on Windows' drive and USB drives, shows you a
picture, and restarts it. The PC's UEFI firmware starts it. So far it has only run in a virtual PC (QEMU with OVMF,
the open-source UEFI firmware). It has its own network stack, with drivers for the wired network chips most PCs have
(Intel's I219, I210, I211, I225 and I226, Realtek's RTL8111/8168 and RTL8125: written to the makers' datasheets and
the open-source drivers, not yet tried on a real PC), USB network adapters, Android phones sharing their connection
over USB, and virtual machines' network cards; the model doesn't use it yet (the web comes next). It keeps the
conversation across restarts, in a file on the PC's start-up partition that it writes through its own NVMe and SATA
drivers.

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
  * the processor's floating-point and vector registers (SSE, and AVX where it has them) switched on for the
    model's arithmetic, the kernel built to use them (`build.sh`), and saved and restored around every interrupt;
  * a record of free memory, one bit per 4 KiB;
  * its own page tables:
    * all RAM at `0xFFFF800000000000`;
    * the kernel's code, read-only data and data, each with only the access it needs;
    * nothing at address 0, so a null pointer faults;
    * device memory uncached, and the framebuffer write-combined;
    * room for the heap to grow into, at `0xFFFFE00000000000`, RAM mapped there as it's needed;
  * a heap: 16 to 256 MiB to start with, then growing as it's needed into its own range of addresses (in 2 MiB pages
    where whole blocks of RAM are free), so the model's file, gigabytes, is one piece in memory however scattered the
    RAM behind it, while 64 MiB is always left for everything else;
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
  * the firmware's own programs (ACPI Machine Language, in its DSDT and SSDTs), run by Chat Computer's own
    interpreter (`aml/`) the way ACPICA (Linux's interpreter) runs them, once the PC is in ACPI mode, as every
    system puts it: the embedded controller's driver (its ports from its device's resources), the firmware told it's
    there (`_REG`), and the devices' `_INI`s. Then the firmware's events are watched, checked 20 times a second: the
    power button (pressing it turns the PC off, with the conversation kept), the general-purpose events the
    firmware's `_Lxx` and `_Exx` methods handle, and the embedded controller's queries (its `_Qxx` methods).
    Batteries (`_BIX` or `_BIF`, and `_BST`), AC adapters (`_PSR`), lids (`_LID`) and thermal zones (`_TMP`,
    `_CRT`) are read when the firmware says they changed, and every so often anyway. They're in the computer's
    status for the model, and a note says when the PC is plugged in or out, or its battery is low. A nearly empty
    battery turns it off a minute later unless it's plugged in, and so does reaching the firmware's critical
    temperature, at once (`firmware.rs`). A method that runs too long is given up, and an embedded controller
    that doesn't answer is left alone for a while. `firmware-programs = off` in the settings file turns all of it
    off, for a PC whose firmware does something odd;
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
    * drives (`usb/storage.rs`): USB sticks, external SSDs and hard drives, and card readers (many laptops' SD card
      slot is one), by USB's mass storage class: SCSI's commands (INQUIRY, TEST UNIT READY, REQUEST SENSE, READ
      CAPACITY, READ), over its bulk-only transport, 64 KiB at a time, with its reset recovery when something goes
      wrong. Each logical unit (a card reader's slot) is asked about once a second whether a medium is in it, so a
      card put in is read at once, and one taken out forgotten. Nothing is ever written to them;
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
    (`fat/`). The conversation and the model's notes are kept there (`keep.rs`, in the format `remember/` defines,
    as JSON: `{"notes": [...], "chat": [{"you": ..., "at": ...}, {"said": ...}, ...]}`: each question, when it was
    asked, and its answer's words) after every answer, and read back at start-up, so a restart carries on the same
    conversation (one kept when the own system talked to Claude is read too). Only the memory file's own blocks are
    ever written, each read and checked first: never the file system, Windows' files or any other partition.
    Before a restart, the drive is told to write what it has cached. The PC's other drives are started too, to be
    read: every NVMe controller's first namespace, and every SATA port with a disk on it (an optical drive left
    alone); each told before a restart too (an SSD counts being switched off without being told);
  * Windows' drives and USB drives, read and never changed (`files.rs`, by Chat Computer's own readers of NTFS,
    `ntfs/`, exFAT, `exfat/`, and FAT, `fat/`): every volume in the basic data partitions of every drive (GPT, or an
    MBR, its extended partition's logical drives included; or a whole stick or card with no partition table), with
    the letter Windows gives it: C:, Windows' own (the one with Windows' folders), and the others as Windows' registry
    names them (SYSTEM's MountedDevices, read by `registry/` from C:, by each partition's GUID, or its disk's
    signature and where it starts; a USB stick by its serial number); a drive Windows hasn't seen gets the next
    letter no drive has, as Windows would give it. USB drives come and go: a stick plugged in or a card put in gets
    its letters (the other drives keep theirs), and a line in the conversation says so, as it does when it's gone.
    The partition Chat Computer starts from, and its own folder (`\EFI\chatcomputer-own`, with its settings and
    memory file) wherever it is, are never read. The computer's status lists the drives, with their names. The model
    reads the person's files there through its tools (below), and its own file is read from there too. Links are
    followed within a drive (a junction such as "My Documents", a symbolic link);
    OneDrive's files are read when they're downloaded to the PC, and said to be online only when they aren't. Word,
    Excel and PowerPoint files are read as their words (`office/`, through Chat Computer's own ZIP and DEFLATE
    reader, `zip/`); a picture or a PDF is said to be one (the model reads words only), and a picture can be shown to
    the person. A drive
    BitLocker encrypts (many new laptops have Windows' "device encryption" on) is read decrypted (`bitlocker/`):
    at once if BitLocker keeps its key in the clear (suspended, or not switched on yet), else once the person types
    its recovery key in the message box (48 digits, from their Microsoft account at aka.ms/myrecoverykey). The key
    is taken out of the message there and never given to the model, shown or kept (so it's typed again after a
    restart);
    a wrong one is refused, naming the key ID that unlocks the drive (each locked drive is tried; a USB drive's
    "BitLocker To Go" too). Not yet: a drive Windows is still encrypting as it's written to, files Windows compressed
    itself ("Compact OS", mostly its own), and Office's files from before 2007;
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
  * Chat Computer's own model (`assistant.rs`): the one Chat Computer's app got ready on Windows
    (`Users\NAME\AppData\Local\ai.chatcomputer.desktop\models\MODEL.ccm`, on Windows' drive or any other with
    people's folders: of those that fit in memory, the biggest that answers quickly enough on one processor core, at
    most 3.6 GB, which is Qwen3.5 4B; else the smallest), or the one the settings name (`model = ...`). Its file is read
    into memory a part at a time (the screen says how far), and its refusal direction beside it (`MODEL.refusal`,
    which the engine found when it got the model ready), so its refusals are taken out as it runs, as on Windows.
    Chat Computer's own engine (`model/`, the same code as in the app) runs it on one processor core, with the fastest
    arithmetic the processor has (AVX2, or AVX-VNNI). It works a step at a time from the main loop: part of what it's
    asked read, or one token written, a few hundredths of a second each (as the processor allows), so the screen and
    the keyboard go on meanwhile. Before the first question, it reads its instructions (the system prompt, with its
    tools) ahead, and keeps that reading for every conversation after. Each message it's given starts with when it
    was sent; it doesn't think before answering (on one core, thinking would take minutes). It's given as much of the
    conversation as the memory holds the reading of (up to 16,384 tokens): in a longer one, the oldest questions are
    left out, a third at a time; they stay on the screen and in the memory file. Its tools:
    * `computer_status`: what the computer is and how it's doing (processor, screen, firmware, Secure Boot,
      memory, how long it has been running, the date and time, the network, the keyboards and mice, the sound, the
      timers set, the drives and the model itself) and what the own system can't do yet (`system.rs`);
    * `set_text_size`: the text 50% to 300% of the size chosen for this monitor;
    * `reminders`: timers, reminders and alarms, as the main Chat Computer has them (`schedule.rs`): for a time from
      now, or for a time of day when the settings give the local time's offset. When one is due, a line says so at
      the end of the conversation and its chime plays (`chime/`, the main Chat Computer's: two bell notes), again
      every 4 seconds for a minute (an alarm every 6 for ten minutes) until a key is pressed; a reminder rings once.
      They count the kernel's own time, so a clock that's out doesn't move them, and last until the PC restarts;
    * `set_volume`: how loud the chimes are, 0 to 100%;
    * `list_files`, `read_file` and `find_files`: a folder's files and folders on one of the drives (C:, D:..., USB
      sticks and memory cards included)
      (with their sizes and when they changed; Windows' hidden system files, such as desktop.ini, left out as File
      Explorer leaves them out), a file's text (UTF-8, or UTF-16 as Notepad saves it, 60,000 bytes at a time, from any
      point), and the files and folders whose names have some words in them, from the person's folders (Users) down by
      default, hidden folders such as AppData passed by. A Word, Excel or PowerPoint file (or OpenDocument's) comes
      back as its words: a document's paragraphs with Markdown's headings, lists and tables; a workbook's sheets as
      CSV with each row's number and each column's letter, dates as dates, formulas after their values; a
      presentation's slides with their notes. A ZIP archive's files are listed. A picture (with its size in pixels)
      or a PDF is said to be one, as is anything else of a kind it can't read;
    * `show_picture`: a photo, a screenshot or a drawing on one of Windows' drives (JPEG, PNG or GIF), shown to the
      person in the conversation where the model's answer is, as wide as the column at most (and two thirds of its
      height), its corners rounded, turned the way the camera says (`picture/`). The last 12 shown keep their pixels;
    * `restart_computer`: a restart a few seconds after the model's answer, which goes back to Windows. It uses the
      reset register the firmware names (ACPI's FADT), else the chipset's reset control (port 0xCF9), else the
      keyboard controller's reset line (`power.rs`);
    * `turn_off_computer`: the PC turned off a few seconds after the model's answer, by ACPI's S5 ("soft off"). First
      the firmware's own preparation for it runs (its `_TTS` and `_PTS` methods). S5's sleep type, which the
      firmware's `\_S5` gives (or the DSDT's bytes, where it is plain data, when its programs aren't run), is written
      with "sleep enable" to the PM1 control registers (or a hardware-reduced PC's sleep control register). If the
      PC doesn't turn off, the screen says so;
    * `start_new_conversation`: when the person asks to start over, the conversation ends after the model's answer.
      It is no longer given to the model or kept, and stays on the screen above a line until the next restart;
    * `remember` and `forget`: the model's notes about the person (their name, what they like), kept in the memory
      file with the conversation and in its instructions in every conversation after, new ones and restarts included
      (each conversation's instructions stay as they began, so they're read once). The screen says when the model
      notes or forgets something. At most 40 notes, 8,000 characters in all.

    The model may use tools up to 8 times for one message. A test build answers from a script in place of the model
    (`CHATCOMPUTER_TEST=chat`, by `check.py --chat` and the drives' checks), so all that's around it is checked
    without one, and logs what the model would have been given.

  Then the screen is the conversation (`chat.rs`), drawn in memory and copied to the screen where it changed.
  Before the first message it says what the PC is, how long Chat Computer has been running (by the kernel's own
  timer), which keyboards it found and how its model is getting ready. Type, press Enter, and the model reads the
  message and answers, its words shown as they're written, with a pulsing dot until the first (and, when it's slow,
  what it's doing: reading your message, or a file). Its answers are Markdown, laid
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
  each certificate may be used for, name constraints) against Mozilla's certificate authorities: the engine gets its
  models ready over it, on Windows and Linux. Tested against OpenSSL's server with every cipher and kind of key, and
  against a real site's chain.
* `model/` is Chat Computer's own model engine: Qwen's open models on the PC's own processor (see its README.md).
  The kernel runs it without the standard library; the same code serves the model on Windows and Linux.
* `webengine/` is the same engine in a web page, for chatcomputer.ai's chat: `model/` as WebAssembly (its 128-bit
  SIMD, one thread), which gets Qwen3.5 0.8B ready in the visitor's browser (downloaded from Qwen's repository at
  the pinned revision, checked and converted as it comes, kept by the browser) and answers as the engine's program
  does, token for token with its plain arithmetic. `scripts/build_web_engine.py` builds it into `docs/demo/engine/`.
* `font/` is Chat Computer's own TrueType reader and anti-aliased rasterizer. Coverage is exact and uses
  integer arithmetic only. It reads variable fonts too (fvar, avar and gvar: deltas, intermediate regions and
  inferred deltas; HVAR for the advances), so the bold comes from the same font file as the regular. And it reads
  a font's kerning (GPOS's pair adjustments, by pairs of glyphs and by classes, varying with the weight through
  GDEF's item variation store). It's tested against fontTools' reading of the same font, at the default weight and
  at others, and against HarfBuzz: every glyph's advance at three weights, and the kerning of every pair of 152
  characters at four (`font/reference.py`).
* `text/` sets lines of text in those fonts: which font has each character (the fallback fonts for what Lora and
  the code font lack), how wide lines are, where they wrap, and drawing them, with the kerning a browser applies
  (within a run of one font and face). Tested against where HarfBuzz puts each letter, at the regular weight and
  the bold (`text/reference.py`).
* `markdown/` reads the Markdown the model writes into blocks and styled words, by the same rules as the main
  screen's renderer (`chatcomputer/ui/assets/md.js`). Its test runs md.js on each case (`markdown/gen/`) and
  checks the two read it the same way.
* `fat/` is Chat Computer's own reader of FAT12, FAT16 and FAT32 (the file system of every PC's start-up
  partition, and of most USB sticks and memory cards up to 32 GB), long names included, and 8.3 names in small
  letters as Windows keeps them: a file's blocks (for the memory file), folders' lists (names, sizes, attributes,
  times) and files' bytes anywhere, the FAT read a few sectors at a time. Tested against volumes made by `mkfs.vfat`
  and filled by mtools: FAT12, FAT16 and FAT32, every file and folder as the tools put them there, fragmented files,
  and every byte of the first sector, the FAT and the root folder changed in turn.
* `exfat/` is Chat Computer's own reader of exFAT (USB sticks bigger than 32 GB, SD cards, drives shared with
  Macs): the boot sector, the FAT (or a file's clusters in one run, as exFAT keeps most), folders' entry sets (names
  in several entries, times with their offset from UTC), the volume's name, and its up-case table, by which names
  are compared as Windows compares them. Tested against volumes made by exfatprogs' `mkfs.exfat` and filled by
  exfat-fuse (`exfat/tests/make.py`), with 4 KiB and 32 KiB clusters: every file's bytes against a hash of what was
  written, and damaged volumes.
* `ntfs/` is Chat Computer's own reader of NTFS, the file system of Windows' drive, which it never changes: the master
  file table's records (put back together from their update sequences, attributes kept in them or in runs of clusters,
  and in other records, through an attribute list, for a file in hundreds of pieces), folders' indexes (B-trees of
  names, compared in capitals as the volume's own table makes them, as Windows compares them), long and 8.3 names,
  hard links, sparse files, compressed files (LZNT1), the volume's name, and reparse points: links (where they go),
  and placeholders, such as OneDrive's (whether the file's bytes are on the PC). Encrypted files (EFS) and files
  Windows compressed itself are recognized, not read; a volume BitLocker encrypts is read through `bitlocker/`. Every
  number read from the disk is checked first, so a damaged volume gives an error, not a crash. Tested against volumes
  made by Linux's `mkntfs` and filled by ntfs-3g (`ntfs/tests/make.py`), with 512-byte and 4 KiB sectors, 4 KiB and 64
  KiB clusters: every file's bytes, against a hash of what ntfs-3g wrote, and every byte of the volume's first records
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
* `registry/` reads Windows' registry: a hive file (as Windows keeps them in Windows\System32\config), its keys
  and their values, cell by cell as they're needed (a hive of tens of megabytes isn't read whole): keys' lists of
  subkeys of every kind, values of every type, data in pieces past 16 KB, names whatever their case. From SYSTEM's
  MountedDevices, which drive has which letter. Tested against a hive hivex wrote (`registry/tests/make.py`) and one
  with data in pieces as Windows keeps it, every key and value as hivex reads them back, and every byte read changed
  in turn.
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
  first frame. Shrunk to fit by averaging, in integers only, and in the memory it's given (the kernel gives half its
  heap's biggest free piece: a bigger picture is said to be too big). It writes JPEGs too: baseline, 4:2:0, the standard's tables, exactly as libjpeg writes them (its colour arithmetic, its
  rounding as it halves the colour, its edges and dummy blocks, its accurate integer DCT). Tested against Pillow's
  (libjpeg-turbo's) decoding of pictures it wrote (within one level a sample) and PNGs written by its test, every
  depth and colour type (exactly), damaged pictures, and Pillow's JPEGs of the same pixels, which the writer's must
  match byte for byte, at every size and quality tried.
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
* `aml/` is Chat Computer's own interpreter of ACPI Machine Language, the byte code a PC's firmware describes its
  devices and their power with (the ACPI specification, 6.5): the namespace of devices, methods, data, operation
  regions and their fields (memory, I/O ports, PCI configuration space, the embedded controller, CMOS), mutexes and
  events, every operator, the implicit conversions as ACPICA does them, 32-bit integers for old tables, `_OSI`
  answered as Windows answers it (firmware is written and tested for Windows), the firmware's global lock, and
  resource templates (I/O ports, memory, interrupts, GPIO pins, I2C devices). Where ACPICA (Linux's interpreter) has
  rules of its own, it keeps them: start-up in its order (`_REG`, `\_INI` and `\_SB._INI`, then each device's
  `_STA` and `_INI`), names in packages settled once every table is loaded, and what's an error (a device defined
  twice, a local read before anything was stored in it, a method's missing value where one is wanted). A table with
  an error loses only what fails, and the rest loads where the table puts it. Every byte is checked before it's
  used, and a loop waiting for hardware that never answers is given up after a while: a damaged table gives an
  error, never a crash or a hang. Tested against ACPICA's own `acpiexec` (Intel's reference interpreter): test
  tables (`tests/*.asl`, compiled by `iasl`) whose every method must give the same in both, run in CI; real PCs'
  tables from the linuxhw/ACPI collection (`tests/corpus.py`, set up as acpiexec sets itself up): of 59,468 objects
  on 144 PCs (79 laptops, 65 desktops, 7 of them AMD AM5 boards), every one the same but 20, 8 on two laptops whose
  batteries are on I2C, which acpiexec simulates in its own way, and 12 on two whose tables have an error inside a
  device, after which ACPICA files what follows inside that device; and damaged tables at random (`examples/fuzz.rs`),
  which must never make it panic.
* `bootinfo/` defines what the start-up stage hands the kernel, once for both.
* `shared/mem.rs` holds Chat Computer's own `memcpy`, `memmove`, `memset` and `memcmp`.

The screen follows Chat Computer's look (`chatcomputer/ui/assets/theme.css` and `chat.css`):
* light words on the dark background, in the tones of Claude's dark mode, your messages in bubbles on the
  right, the message box a card lifted off the middle with a soft shadow at the bottom;
* Lora (its regular and bold), and Atkinson Hyperlegible Mono for code. What they lack (arrows, check marks, maths,
  Greek, emoji in black and white, box drawing in code) comes from two
  fallback fonts made from Google's Noto fonts (`fonts/`, built by `fonts/build.py`), as a browser would take it
  from the system's fonts. Chinese, Japanese and Korean show as boxes still;
* text sized as the Claude app's (16 px at 100%), scaled as `chatcomputer/ui/layout.py` does, from the
  monitor's real size;
* the model's words up to 54 em across (865 px at 100%, as the Claude app's) in a 57.75 em conversation, and a 60 em
  message box.

Italics are the regular or bold glyphs slanted 12° (as browsers do for a font without its italic), which keeps
the italic font file out.

## Starting it on a PC

On a Windows PC, double-click `windows\Chat Computer Own System.cmd` (`windows/own-os.ps1`). No USB stick is
needed.

1. It checks the PC (read-only) and saves a list of its parts in `%LOCALAPPDATA%\ChatComputer\own-os\pc.txt`, so
   drivers can be written for exactly those chips.
2. It signs `release/chatcomputer.efi` with a key that Windows makes on that PC and won't let be copied off.
3. It says which model the own system will answer with: the one Chat Computer's app got ready on the PC (open the
   app once first; it gets its model by itself).
4. After you confirm, it puts the signed file in `\EFI\chatcomputer-own` on the partition the PC starts from,
   next to Windows' own files, with `settings.txt` (how far the PC's clock is from UTC, as Windows keeps that
   clock in local time, and the memory file's id; a model's file named there by hand, `model = ...`, is kept) and
   `memory.bin`, the memory file where Chat Computer keeps the conversation across restarts (4 MB, or 1 MB if the
   partition is short of room; running setup again keeps it, with the conversation in it). The conversation is kept
   there as it is, so anyone who can read that partition can read it.
5. It adds "Chat Computer (own system)" to the end of the power-on menu and starts it once, at the next restart.
   The restart after that starts Windows.

With Secure Boot on, the PC first has to trust Chat Computer's key. That takes one approval in the PC's setup
screen: Secure Boot, then Key Management, then Authorized Signatures (db), then Append `ChatComputer.cer`. You
can turn Secure Boot off there instead. The script shows the steps and restarts into the setup screen.

Any USB keyboard works, plugged into the PC or into a hub (a monitor's or a dock's), and so do most
laptops' built-in keyboards. A keyboard that only connects by Bluetooth needs its cable for now:
Bluetooth comes at step 10.

The model needs no internet. Starting up, it reads the model's file from Windows' drive into memory, which takes a
moment (longer if BitLocker encrypts the drive: then type its recovery key when it asks). It runs on one processor
core for now, so it answers slowly (how fast hasn't been measured on a real PC yet); using every core comes next
(step 4).

To go back to Windows, press the power button (the PC turns off; hold it if a press doesn't), or ask Chat Computer to
restart it. The only thing
Chat Computer writes on any disk is its memory file, in place. Running the script again with `-Remove` (or choosing
Remove) takes the files (the conversation with them) and the entry off again, and deletes the key.

`release/chatcomputer.efi` is committed because Windows has no Rust compiler. The build is reproducible, and CI
rebuilds it and checks that it's the same file, byte for byte. `./release.sh` updates it.

## Building and trying it

You need Rust (`rustup`). The exact compiler version is pinned in `rust-toolchain.toml`, and rustup
fetches it. To try it you also need QEMU, OVMF and Pillow (`apt install qemu-system-x86 ovmf
python3-pil`).

```
./build.sh                       # builds out/esp/EFI/BOOT/BOOTX64.EFI (one file, about 3.4 MB)
cargo test -p font -p text -p net -p crypto -p tls -p fat -p exfat -p ntfs -p zip -p office -p picture -p bitlocker -p registry -p remember -p markdown -p chime -p hid -p aml
                                 # the font engine and text setting, network stack, cryptography, TLS (needs
                                 # openssl), FAT reader (needs dosfstools and mtools), exFAT
                                 # and NTFS readers (need xz), ZIP, Office and picture readers (need python3 and
                                 # Pillow),
                                 # BitLocker reader (needs curl, xz and python3-cryptography; BITLOCKER_IMAGES
                                 # names the test volumes' archive if it's here already), registry reader (needs
                                 # xz), memory file, Markdown reader, chimes, HID descriptors and the AML
                                 # interpreter (needs acpica-tools: iasl and acpiexec)
cargo test -p model --features std # the model engine (small models of random weights)
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
./check.py --model               # the own model: a tiny one of random weights found where the app keeps its
                                 # models, read, its instructions read ahead, a question answered as the engine
                                 # on Linux answers it; named by the settings; a file that isn't a model
./check.py --chat                # typing to a test's script in place of the model: what it's given each round,
                                 # its tools (the computer's status, a bigger text size, a timer, a restart), the
                                 # answer on the screen (Markdown), a failure, a mouse's wheel scrolling
./check.py --remember            # the conversation and the model's notes kept across a restart, on an NVMe SSD
                                 # and a SATA drive laid out like a PC's, then a new conversation started (the
                                 # note still in the model's instructions), then the PC turned off;
                                 # nothing else on the disk changed (needs dosfstools, mtools and gdisk)
./check.py --sound               # a timer set by a test build, its chime recorded from each of QEMU's
                                 # HD Audio controllers and codecs and checked against the chime's own samples
./check.py --files               # Windows' drive read by the model's tools (a test's script asks): a folder
                                 # listed, text files read (UTF-8 and UTF-16, a long one in two parts), links
                                 # followed, OneDrive's files on the PC and online only, Word, Excel and
                                 # PowerPoint files as words, a search, and a photo shown on the screen (needs
                                 # xz and gdisk)
./check.py --drives              # the PC's other drives: a second NVMe SSD (GPT) and two SATA drives (MBR and
                                 # GPT), read by the letters Windows' registry gives them, one Windows hasn't seen
                                 # given the next free letter (needs ntfs-3g's mkntfs too)
./check.py --usb                 # USB drives as they come and go: a FAT32 stick Windows' registry names, a card
                                 # reader's two slots (a FAT16 card with no partition table; an exFAT card put in
                                 # later), a stick of 4 KiB blocks with NTFS plugged in later, then a stick
                                 # unplugged and a card taken out; Chat Computer's own folder never read
./check.py --power               # the firmware's own programs: a battery, an AC adapter and a lid added to the
                                 # virtual PC's tables (aml/tests/power.asl), read and followed as they change
                                 # (unplugged, the battery low, the lid closed), a processor plugged in (an event
                                 # QEMU's firmware handles), then the power button: the PC turns off, the
                                 # firmware's _TTS and _PTS run first (needs iasl, from acpica-tools)
./check.py --bitlocker           # the same drive encrypted with BitLocker: locked; a wrong recovery key typed
                                 # refused, the right one unlocking it, its files read; neither key reaching
                                 # the model or the disk; and read at once with BitLocker's key in the clear (needs
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
  * Its ACPI tables include small power-management programs (AML) that an OS has to run for the battery, the
    power button and sleep. Chat Computer runs them with its own interpreter, but they're the firmware's code.
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
* **Data**, such as the font (Lora, by Cyreal, under the SIL Open Font License), and the list of certificate authorities Chat Computer trusts, which is Mozilla's
  (`tls/roots.txt`). Chat Computer ignores the hinting programs inside the font. Later, speech models' weights
  will be data of this kind too, as the model's weights already are (Qwen's, published by Alibaba under the Apache
  2.0 licence; Chat Computer's own engine runs them).

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
   the start-up partition, ✓ Windows' drive read (NTFS), for the model to read the person's files (and its own).
   Checked in virtual PCs. ✓ Word, Excel and PowerPoint files' words. ✓ A drive BitLocker
   encrypts, with its recovery key. ✓ The PC's other drives (NVMe and SATA), by Windows' letters. ✓ USB drives
   (sticks, external disks, card readers) as they come and go, and exFAT and FAT besides NTFS. To do: USB 3 hubs'
   own side (for USB 3 drives behind one), UAS (USB-attached SCSI, faster), memory card
   readers on PCI Express (Realtek's, in many laptops), a drive still being encrypted as it's written to, unlocking with the PC's TPM (for Chat Computer's own start-up,
   measured), files Windows compressed itself (XPRESS and LZX), Office's files from before 2007, Chat Computer's own
   file system for memory and logs, and keeping them encrypted.
6. Network: ✓ IPv4, ARP, DHCP, DNS and TCP; virtual machines' cards (virtio, e1000, e1000e, igb); the internet over
   a USB cable (USB Ethernet adapters and Android phones' tethering: CDC ECM, NCM and RNDIS); written, not yet tried
   on a real PC: Intel's I219, I210, I211, I225 and I226, and Realtek's RTL8111/8168 and RTL8125. To do: trying those
   on real PCs, Realtek's PCI Express PHY tuning and PHY firmware, then IPv6.
7. ✓ Encryption: SHA-2, HMAC and HKDF, X25519, AES-GCM, ChaCha20-Poly1305, ECDSA and RSA signatures,
   certificate checks against Mozilla's authorities, and TLS 1.3.
8. ✓ Chat Computer's own model engine (model/, see its README.md): Qwen's open models on the PC's own processor,
   checked against the reference implementation, with nothing from elsewhere, so Chat Computer needs no account and
   no internet to think. ✓ The kernel runs it: the model's file read from Windows' drive into memory, the
   processor's vector registers (AVX) kept across interrupts, the answer a step at a time beside the screen and the
   keyboard; checked in a virtual PC against the engine on Linux. To do: every core (step 4), the web (looking
   things up over its own HTTPS), and pictures and PDFs read.
9. The conversation screen: ✓ the conversation, wrapping and scrolling, ✓ the formatting the model's answers use
   (Markdown, with bold from the variable font), ✓ kerning, as a browser sets the same font, ✓ pictures in the
   conversation (JPEG, PNG, GIF). To do: accents
   placed over letters typed as two characters (GPOS's mark positioning), and Chinese, Japanese and Korean.
10. Sound and AirPods: ✓ HD Audio's outputs, for the chimes of timers and reminders (checked with QEMU's
    controllers and codecs). To do: trying it on real PCs, microphones, USB audio, then Bluetooth (HCI over USB,
    L2CAP, A2DP and HFP, SBC and mSBC) for AirPods.
11. Speech: Chat Computer's own speech recognition and voice, running the models itself.
12. Power: ✓ restart, ✓ turning off (ACPI's S5, with the firmware's _TTS and _PTS), ✓ the firmware's own programs
    (AML) run by Chat Computer's own interpreter: ✓ the power button, ✓ laptops' batteries, AC adapters and lids, ✓
    the firmware's events (GPEs, the embedded controller's queries), ✓ its critical temperature. Checked against
    ACPICA on 144 real PCs' tables and in a virtual PC; to be tried on real PCs. To do: sleep (S3) and waking, the
    screen's brightness (_BCM), and the touchpads and keyboards laptops have on I2C (described only in the
    firmware's tables).
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
   covers nearly every PC: USB (done), PS/2 keyboards (done), NVMe and SATA (done), the internet over USB (done),
   laptops' power (ACPI: done, to be tried on real laptops), HD Audio and USB audio. Then the most common chips: Realtek's and Intel's
   wired network (written; to be tried on real PCs), then Intel's and MediaTek's Wi-Fi, the hardest single piece.
