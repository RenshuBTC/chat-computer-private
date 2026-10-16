# ClaudeOS Setup (Windows)

`ClaudeOS Setup.cmd` is what you double-click. The steps are in the main `README.md`; this file is for
anyone looking under the hood.

| File | What it does |
|---|---|
| `ClaudeOS Setup.cmd` | Asks Windows for administrator rights, then runs `setup.ps1` (Windows PowerShell 5.1). |
| `setup.ps1` | Starts the setup and keeps a log in `%LOCALAPPDATA%\ClaudeOS\setup\setup.log`. The log never contains the Claude sign-in, the Wi-Fi password or the recovery password. Switches: `-PlanOnly` (change nothing, show the plan), `-IsoPath` (use a downloaded Ubuntu image). |
| `lib\Flow.ps1` | The conversation: install, "sign in to Claude again", what starts when the PC turns on, or remove ClaudeOS. |
| `lib\Startup.ps1` | The power-on menu through `bcdedit` (only untranslated values are read from its output: `{identifiers}` and `\EFI\` paths): the installer's one-time entry "ClaudeOS setup" (a copy of Windows Boot Manager's entry that starts `\EFI\claudeos-setup\shimx64.efi`), ClaudeOS or Windows first, ClaudeOS once (`bootsequence`). Removing ClaudeOS: finds its mark on an EFI partition (`\EFI\claudeos\installed-<id>`, which names ClaudeOS's partition by its GPT GUID) and any installer partition with its mark, pauses BitLocker for one restart, puts Windows first and deletes ClaudeOS's and the installer's entries, deletes `\EFI\ubuntu`, `\EFI\claudeos` and `\EFI\claudeos-setup` (and puts Windows' boot manager back in `\EFI\BOOT\BOOTX64.EFI` if Ubuntu's took that slot), deletes exactly the partitions the marks name after checking them, grows C: into the space when it is next to it, and deletes setup's downloads and sign-in copies. Nothing changes before you type REMOVE. |
| `lib\Checks.ps1` | Read-only checks: UEFI, Secure Boot, BitLocker (shows the recovery key), Fast Startup, AirPods. |
| `lib\Disks.ps1` | Where ClaudeOS goes: free space, C: shrunk, or a separate disk erased. |
| `lib\Network.ps1` | Cable or Wi-Fi for the installer. Reads the connected network's saved password. |
| `lib\ClaudeSignIn.ps1` | Downloads the official Claude program from Anthropic's `claude-agent-sdk` package on PyPI (checked against a fixed SHA-256) and runs `claude setup-token`. For the Claude app on the phone it also runs `claude auth login` in a throwaway settings folder and takes the full sign-in it saves (`.credentials.json`). |
| `lib\Ubuntu.ps1` | Downloads Ubuntu Server 24.04 and checks it against known and published SHA-256 fingerprints. |
| `lib\Installer.ps1` | The installer, on the PC's own disk: a temporary 6 GB FAT32 partition `CLAUDEOS` at the end of ClaudeOS's space with the Ubuntu installer, ClaudeOS and its mark (`claudeos\installer-<id>`); the boot loader that starts it (`\EFI\claudeos-setup`, Ubuntu's signed shim and GRUB); and the boot menu change to "Install ClaudeOS" (adds `autoinstall`), or "Start ClaudeOS" once this installer has installed it. |
| `lib\Autoinstall.ps1` | Builds `autoinstall.yaml` from `autoinstall.template.yaml`, including the SHA-512 password hash. |
| `installer\guard.sh` | Runs in the installer before anything is written. If this installer has already installed ClaudeOS on this PC, it restarts into ClaudeOS instead. |
| `installer\network.sh` | Runs in the installer before it reads the network settings: puts the Wi-Fi card's Linux name in them. |
| `installer\finish.sh` | Runs in the installer at the end. Copies ClaudeOS, the sign-in and settings, leaves the mark `\EFI\claudeos\installed-<id>` (`id=` and `root=` ClaudeOS's partition GUID), sets up the first start (finishing the install if needed, and deleting the installer partition so ClaudeOS gets its space), then runs `linux/install-claudeos.sh`. |

## Disk safety

The installer is only ever told to use **free space** (`storage.layout.mode: use_gap`). It never
reformats a disk.

Making that space happens on Windows, and only after you answer "Go ahead":
* shrinking C:, the same as Disk Management's "Shrink volume", or
* erasing a separate internal disk whose number you typed.

The disk Windows runs from and USB drives are refused as erase targets.

The installer installs onto the disk it starts from (`match: {install-media: true}`): setup puts the
installer partition on the disk ClaudeOS goes on, at the end of ClaudeOS's free space, so the free space
the installer uses is the space set aside for ClaudeOS, right before it. When ClaudeOS first starts, it
deletes the installer partition and grows into it.

The order after "Go ahead" is: the power-on entry (if the PC won't take one, nothing else changes), the
space, the installer partition, its files and boot loader, and last, telling the PC to start it once. If
a step fails, setup takes back what it did, as far as it can, and says what's left.

## Tests

```
pytest windows/tests            # needs pwsh; uses PSScriptAnalyzer and subiquity's schema if found
pwsh -File windows/tests/Run-Tests.ps1
pwsh -File windows/tests/Flow-Test.ps1 -Scenario shrink -OutDir /tmp/flow
```

The flow test walks the whole conversation on a pretend PC: a 1 TB NVMe drive with Windows, a data
disk and Wi-Fi. The stand-ins for Windows' commands keep track of the disks' partitions, the power-on menu
and the EFI partition, so the tests check where the installer goes, the order of the steps, and what
removing and undoing leave behind.
