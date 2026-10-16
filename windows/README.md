# ClaudeOS Setup (Windows)

`ClaudeOS Setup.cmd` is what you double-click. The steps are in the main `README.md`; this file is for
anyone looking under the hood.

| File | What it does |
|---|---|
| `ClaudeOS Setup.cmd` | Asks Windows for administrator rights, then runs `setup.ps1` (Windows PowerShell 5.1). |
| `setup.ps1` | Starts the setup and keeps a log in `%LOCALAPPDATA%\ClaudeOS\setup\setup.log`. The log never contains the Claude sign-in, the Wi-Fi password or the recovery password. Switches: `-PlanOnly` (change nothing, show the plan), `-IsoPath` (use a downloaded Ubuntu image). |
| `lib\Flow.ps1` | The conversation: install, "sign in to Claude again", what starts when the PC turns on, or remove ClaudeOS. |
| `lib\Startup.ps1` | The power-on menu through `bcdedit` (only untranslated values are read from its output: `{identifiers}` and `\EFI\` paths): ClaudeOS or Windows first, ClaudeOS once (`bootsequence`). Removing ClaudeOS: finds its mark on an EFI partition (`\EFI\claudeos\installed-<id>`, which names ClaudeOS's partition by its GPT GUID), pauses BitLocker for one restart, puts Windows first and deletes ClaudeOS's entries, deletes `\EFI\ubuntu` and `\EFI\claudeos` (and puts Windows' boot manager back in `\EFI\BOOT\BOOTX64.EFI` if Ubuntu's took that slot), deletes exactly the partition the mark names after checking it is a Linux partition, grows C: into the space when it is next to it, and deletes setup's downloads and sign-in copies. Nothing changes before you type REMOVE. |
| `lib\Checks.ps1` | Read-only checks: UEFI, Secure Boot, BitLocker (shows the recovery key), Fast Startup, AirPods. |
| `lib\Disks.ps1` | Where ClaudeOS goes, and how the installer recognises that disk on Linux. |
| `lib\Network.ps1` | Cable or Wi-Fi for the installer. Reads the connected network's saved password. |
| `lib\ClaudeSignIn.ps1` | Downloads the official Claude program from Anthropic's `claude-agent-sdk` package on PyPI (checked against a fixed SHA-256) and runs `claude setup-token`. For the Claude app on the phone it also runs `claude auth login` in a throwaway settings folder and takes the full sign-in it saves (`.credentials.json`). |
| `lib\Ubuntu.ps1` | Downloads Ubuntu Server 24.04 and checks it against known and published SHA-256 fingerprints. |
| `lib\Usb.ps1` | Formats the stick FAT32 as `CLAUDEOS`, copies the installer and ClaudeOS, and changes the boot menu to "Install ClaudeOS" (adds `autoinstall`), or "Start ClaudeOS" once this stick has installed it. |
| `lib\Autoinstall.ps1` | Builds `autoinstall.yaml` from `autoinstall.template.yaml`, including the SHA-512 password hash. |
| `usb\guard.sh` | Runs in the installer before anything is written. If this stick has already installed ClaudeOS on this PC, it restarts into ClaudeOS instead. |
| `usb\finish.sh` | Runs in the installer at the end. Copies ClaudeOS, the sign-in and settings, leaves the mark `\EFI\claudeos\installed-<id>` (`id=` and `root=` ClaudeOS's partition GUID), then runs `linux/install-claudeos.sh`. |

## Disk safety

The installer is only ever told to use **free space** (`storage.layout.mode: use_gap`). It never
reformats a disk.

Making that space happens on Windows, and only after you answer "Go ahead":
* shrinking C:, the same as Disk Management's "Shrink volume", or
* erasing a separate internal disk whose number you typed.

The disk Windows runs from and USB drives are refused as erase targets.

The installer finds the target disk with rules tried in order:
1. its serial number;
2. its model, if no other disk has the same one;
3. "the only disk with room", only when that's true.

Every rule skips disks in use, so the installer's own USB stick is never a target. If no rule matches,
the installer stops before changing anything.

## Tests

```
pytest windows/tests            # needs pwsh; uses PSScriptAnalyzer and subiquity's schema if found
pwsh -File windows/tests/Run-Tests.ps1
pwsh -File windows/tests/Flow-Test.ps1 -Scenario shrink -OutDir /tmp/flow
```

The flow test walks the whole conversation on a pretend PC: a 1 TB NVMe drive with Windows, a data
disk, a USB stick and Wi-Fi.
