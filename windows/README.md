# Chat Computer Setup (Windows)

`Chat Computer Setup.cmd` is what you double-click. The steps are in the main `README.md`; this file is for
anyone looking under the hood.

| File | What it does |
|---|---|
| `Chat Computer Setup.cmd` | Asks Windows for administrator rights, then runs `setup.ps1` (Windows PowerShell 5.1). |
| `setup.ps1` | Starts the setup and keeps a log in `%LOCALAPPDATA%\ChatComputer\setup\setup.log`. The log never contains the Wi-Fi password or the recovery password. Turns QuickEdit off in its window (a click there would otherwise pause setup until a key is pressed) and takes the downloaded-file mark off its own files (so Windows warns "the publisher could not be verified" only the first time). Switches: `-PlanOnly` (change nothing, show the plan), `-IsoPath` (use a downloaded Ubuntu image). |
| `lib\Flow.ps1` | The conversation. On a PC without Chat Computer: install, asking only what it must (a recovery-key check when C: is encrypted, where Chat Computer goes when there is a second disk, a Wi-Fi network when Windows knows none), then one list of everything that will change and "Go ahead?". With Chat Computer (or an earlier try's installer) on the PC: a menu to choose what starts when the PC turns on, or remove Chat Computer. Nothing to sign in to: Chat Computer downloads the open model it thinks with when it first starts. |
| `lib\Startup.ps1` | The power-on menu through `bcdedit` (only untranslated values are read from its output: `{identifiers}` and `\EFI\` paths): the installer's one-time entry "Chat Computer setup" (a copy of Windows Boot Manager's entry that starts `\EFI\chatcomputer-setup\shimx64.efi`), Chat Computer or Windows first, Chat Computer once (`bootsequence`). Removing Chat Computer: finds its mark on an EFI partition (`\EFI\chatcomputer\installed-<id>`, which names Chat Computer's partition by its GPT GUID) and any installer partition with its mark, pauses BitLocker for one restart, puts Windows first and deletes Chat Computer's and the installer's entries, deletes `\EFI\ubuntu`, `\EFI\chatcomputer` and `\EFI\chatcomputer-setup` (and puts Windows' boot manager back in `\EFI\BOOT\BOOTX64.EFI` if Ubuntu's took that slot), deletes exactly the partitions the marks name after checking them, grows C: into the space when it is next to it, deletes the BitLocker key setup gave Chat Computer, and deletes setup's downloads (and the sign-in copies an earlier version, which thought with Claude, left). Nothing changes before you type REMOVE. |
| `lib\Checks.ps1` | Checks: UEFI, Secure Boot, BitLocker (shows the recovery key), Fast Startup (turned off last, after "Go ahead"), AirPods. Chat Computer's own BitLocker key: when C: is encrypted, a recovery password made for Chat Computer (through BitLocker's WMI class `Win32_EncryptableVolume`, which Windows Home has too), its id kept in `%ProgramData%\ChatComputer\bitlocker-key-id` so removing Chat Computer deletes exactly that key; the person's own keys are never touched. |
| `lib\Disks.ps1` | Where Chat Computer goes: empty space after C: made by shrinking C:, or a separate disk erased. Making room works the way one does it by hand: shrink C: as far as Windows allows (never below 25 GB free), and when a file Windows can't move is in the way, find out which from event 259 in the Application log (which `Get-PartitionSupportedSize` writes; Microsoft's "Shrink a basic volume" points to it), deal with it, and shrink again. The change journal is reset whenever it's in the way (`fsutil usn deletejournal /n C:`; Windows starts a new one at once); restore points are deleted (`vssadmin delete shadows /for=C: /quiet`, Microsoft's KB5005357) and hibernation turned off only when the 40 GB minimum isn't reached otherwise. The page file and the file system's own records can't be moved while Windows runs: setup stops and says what to do. |
| `lib\Network.ps1` | Cable or Wi-Fi for the installer. Reads the connected network's saved password. |
| `lib\Ubuntu.ps1` | Downloads Ubuntu Server 24.04 and checks it against known and published SHA-256 fingerprints. |
| `lib\Installer.ps1` | The installer, on the PC's own disk: a temporary 6 GB FAT32 partition `CHATCOMP` at the end of Chat Computer's space with the Ubuntu installer, Chat Computer and its mark (`chatcomputer\installer-<id>`); the boot loader that starts it (`\EFI\chatcomputer-setup`, Ubuntu's signed shim and GRUB); and the boot menu change to "Install Chat Computer" (adds `autoinstall`), or "Start Chat Computer" once this installer has installed it. |
| `lib\Autoinstall.ps1` | Builds `autoinstall.yaml` from `autoinstall.template.yaml`, including the SHA-512 password hash. |
| `installer\guard.sh` | Runs in the installer before anything is written. If this installer has already installed Chat Computer on this PC, it restarts into Chat Computer instead. |
| `installer\network.sh` | Runs in the installer before it reads the network settings: puts the Wi-Fi card's Linux name in them. |
| `installer\finish.sh` | Runs in the installer at the end. Copies Chat Computer and its settings (and Chat Computer's BitLocker key, to `/etc/chatcomputer/bitlocker-recovery-key`, root only), leaves the mark `\EFI\chatcomputer\installed-<id>` (`id=` and `root=` Chat Computer's partition GUID), sets up the first start (finishing the install if needed, and deleting the installer partition so Chat Computer gets its space), then runs `linux/install-chatcomputer.sh`. |

## Disk safety

The installer is only ever told to use **free space** (`storage.layout.mode: use_gap`). It never
reformats a disk.

Making that space happens on Windows, and only after you answer "Go ahead":
* shrinking C:, the same as Disk Management's "Shrink volume" (in steps, clearing what Windows can't move
  out of the way as described under `lib\Disks.ps1`), or
* erasing a separate internal disk whose number you typed.

The disk Windows runs from and USB drives are refused as erase targets.

The installer installs onto the disk it starts from (`match: {install-media: true}`): setup puts the
installer partition on the disk Chat Computer goes on, at the end of Chat Computer's free space, so the free space
the installer uses is the space set aside for Chat Computer, right before it. When Chat Computer first starts, it
deletes the installer partition and grows into it.

The order after "Go ahead" is: the power-on entry (if the PC won't take one, nothing else changes), the
space, the installer partition, its files, Chat Computer's BitLocker key (when C: is encrypted) and the boot
loader, Fast Startup off, and last, telling the PC to start the installer once. If a step fails, setup
takes back what it did, as far as it can (C: grows back to the size it had; deleted restore points stay
deleted), and says what's left.

## Tests

```
pytest windows/tests            # needs pwsh; uses PSScriptAnalyzer and subiquity's schema if found
pwsh -File windows/tests/Run-Tests.ps1
pwsh -File windows/tests/Flow-Test.ps1 -Scenario shrink -OutDir /tmp/flow
```

The flow test walks the whole conversation on a pretend PC: a 1 TB NVMe drive with Windows, a data
disk and Wi-Fi. The stand-ins for Windows' commands keep track of the disks' partitions, the power-on menu
and the EFI partition, so the tests check where the installer goes, the order of the steps, and what
removing and undoing leave behind. Its "blocked" scenario is the PC setup was first tried on: 272 GB free,
yet C: couldn't shrink at all, because of a restore point, then the change journal (twice), then $Mft.

`.github/workflows/windows.yml` runs these tests on GitHub's Windows machines under Windows PowerShell
5.1, and `tests/win_ci/probe.ps1` asks real Windows what setup relies on: event 259's fields, `vssadmin`,
`fsutil usn`, `powercfg`, shrinking C:, QuickEdit.
