<#
ClaudeOS Setup (run on Windows; double-click "ClaudeOS Setup.cmd").

Installs ClaudeOS next to Windows, without a USB stick:
  1. checks this PC (read-only),
  2. asks where ClaudeOS should go and makes room there (only after you confirm),
  3. signs in to Claude with your subscription,
  4. downloads the official Ubuntu Server 24.04 installer and checks it,
  5. writes the installer, ClaudeOS and your settings to a temporary partition at the end of that room,
     and adds "ClaudeOS setup" to the power-on menu for the next restart.
Then the PC restarts into the installer; ClaudeOS installs itself, becomes the first choice on the
power-on menu, with Windows as the second, and takes over the installer's space when it first starts.

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
foreach ($lib in @('Common', 'Timezones', 'Autoinstall', 'Disks', 'Network', 'ClaudeSignIn', 'Ubuntu', 'Installer', 'Checks', 'Startup', 'Flow')) {
    . ([IO.Path]::Combine($Here, 'lib', $lib + '.ps1'))
}

if (-not (Test-IsAdministrator)) {
    # Changing disks needs administrator rights; ask Windows for them and start again.
    $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + $PSCommandPath + '"'))
    if ($IsoPath) { $argList += @('-IsoPath', ('"' + $IsoPath + '"')) }
    if ($PlanOnly) { $argList += '-PlanOnly' }
    $argList += '-Relaunched'
    Start-Process -FilePath 'powershell.exe' -ArgumentList $argList -Verb RunAs
    exit 0
}

$WorkDir = Join-Path $env:LOCALAPPDATA 'ClaudeOS\setup'
New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
$LogPath = Join-Path $WorkDir 'setup.log'
Initialize-SetupLog $LogPath
Use-LongTempPath                     # C:\Users\Ren Shu\..., not C:\Users\RENSHU~1\... (see Remove-Folder)
$null = Disable-ConsoleQuickEdit     # a click in this window can't pause setup
Unblock-SetupFiles -Root $RepoRoot   # no "publisher could not be verified" warning next time

$exitCode = 0
try {
    Invoke-ClaudeOSSetup -Here $Here -RepoRoot $RepoRoot -WorkDir $WorkDir -IsoPath $IsoPath -PlanOnly:$PlanOnly
} catch {
    Write-SetupLog ('FAILED: ' + $_.Exception.ToString())
    Write-Host ''
    Write-Warn ('Setup stopped: ' + (Protect-SetupText $_.Exception.Message))
    Write-Line ('Details are in ' + $LogPath)
    $exitCode = 1
}
if ($Relaunched) {
    # This window was opened by setup itself; keep it open so the last words can be read.
    Read-Host 'Press Enter to close this window' | Out-Null
}
exit $exitCode
