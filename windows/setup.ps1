<#
Chat Computer Setup (run on Windows; double-click "Chat Computer Setup.cmd").

Installs Chat Computer next to Windows, without a USB stick:
  1. checks this PC (read-only),
  2. asks where Chat Computer should go and makes room there (only after you confirm),
  3. downloads the official Ubuntu Server 24.04 installer and checks it,
  4. writes the installer, Chat Computer and your settings to a temporary partition at the end of that room,
     and adds "Chat Computer setup" to the power-on menu for the next restart.
Then the PC restarts into the installer; Chat Computer installs itself, becomes the first choice on the
power-on menu, with Windows as the second, and takes over the installer's space when it first starts. There it
downloads the open model it thinks with (Qwen's, chosen for this PC's memory) and runs it on the PC itself: no
account, no sign-in, nothing to pay.

  -PlanOnly   go through every question and check, show what would happen, change nothing.
  -IsoPath    use an Ubuntu 24.04 server installer you already downloaded.

ASCII-only file (see lib\Common.ps1).
#>
[CmdletBinding()]
param(
    [string]$IsoPath,
    [switch]$PlanOnly,
    [switch]$Relaunched
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$Here = $PSScriptRoot
$RepoRoot = Split-Path -Parent $Here
foreach ($lib in @('Common', 'Timezones', 'Autoinstall', 'Disks', 'Network', 'Ubuntu', 'Installer', 'Checks', 'Startup', 'Flow')) {
    . ([IO.Path]::Combine($Here, 'lib', $lib + '.ps1'))
}

if (-not (Test-IsAdministrator)) {
    # Changing disks needs administrator rights; ask Windows for them and start again.
    $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + $PSCommandPath + '"'))
    if ($IsoPath) { $argList += @('-IsoPath', ('"' + $IsoPath + '"')) }
    if ($PlanOnly) { $argList += '-PlanOnly' }
    $argList += '-Relaunched'
    try {
        Start-Process -FilePath 'powershell.exe' -ArgumentList $argList -Verb RunAs -ErrorAction Stop
    } catch {
        # The person said no to Windows' question (or it couldn't be asked).
        Write-Host ('Windows didn''t give Chat Computer Setup permission to make changes, so nothing was changed. (' +
                    $_.Exception.Message + ')')
        exit 1
    }
    exit 0
}

$WorkDir = Join-Path $env:LOCALAPPDATA 'ChatComputer\setup'
New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
$LogPath = Join-Path $WorkDir 'setup.log'
Initialize-SetupLog $LogPath
Use-LongTempPath                     # C:\Users\Sam Lee\..., not C:\Users\SAMLEE~1\... (see Remove-Folder)
$null = Disable-ConsoleQuickEdit     # a click in this window can't pause setup
Unblock-SetupFiles -Root $RepoRoot   # no "publisher could not be verified" warning next time

$exitCode = 0
try {
    Invoke-ChatComputerSetup -Here $Here -RepoRoot $RepoRoot -WorkDir $WorkDir -IsoPath $IsoPath -PlanOnly:$PlanOnly
} catch {
    Write-SetupLog ('FAILED: ' + $_.Exception.ToString())
    Write-Host ''
    # Messages from the install step already begin "Setup stopped:".
    $why = Protect-SetupText (Get-ErrorText $_)
    if ($why -notmatch '^Setup stopped') { $why = 'Setup stopped: ' + $why }
    Write-Warn $why
    Write-Line ('Details are in ' + $LogPath)
    $exitCode = 1
}
if ($Relaunched) {
    # This window was opened by setup itself; keep it open so the last words can be read.
    Read-Host 'Press Enter to close this window' | Out-Null
}
exit $exitCode
