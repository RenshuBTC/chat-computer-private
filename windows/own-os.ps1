<#
ClaudeOS's own system (double-click "ClaudeOS Own System.cmd").

Starts ClaudeOS's own operating system (os/ in this folder: its own start-up stage and kernel, written from
scratch) on this PC, without a USB stick:
  1. checks this PC (read-only) and saves a list of its parts, for writing ClaudeOS's drivers,
  2. signs ClaudeOS's start-up file with a key made on this PC (the key never leaves it),
  3. after you confirm, puts the signed file on the partition the PC starts from (\EFI\claudeos-own), next
     to Windows' own files, and adds "ClaudeOS (own system)" to the end of the power-on menu, started once.
With Secure Boot on, the PC must first trust ClaudeOS's key: one approval in the PC's setup screen (or turn
Secure Boot off there). Then the PC restarts into ClaudeOS's own system; the restart after that is Windows.

  -Remove       take it off this PC again
  -Unattended   answer yes to everything (for tests); add -NoRestart so nothing restarts
  -Out DIR      also keep the signed file and the key's certificate in DIR (for tests)

ASCII-only file (see lib\Common.ps1).
#>
[CmdletBinding()]
param(
    [switch]$Remove,
    [switch]$Unattended,
    [switch]$NoRestart,
    [string]$Out,
    [switch]$Relaunched
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$Here = $PSScriptRoot
$RepoRoot = Split-Path -Parent $Here
foreach ($lib in @('Common', 'Checks', 'Startup', 'OwnOs')) {
    . ([IO.Path]::Combine($Here, 'lib', $lib + '.ps1'))
}

if (-not (Test-IsAdministrator)) {
    # The start-up partition and the power-on menu need administrator rights; ask Windows and start again.
    $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + $PSCommandPath + '"'))
    if ($Remove) { $argList += '-Remove' }
    $argList += '-Relaunched'
    try {
        Start-Process -FilePath 'powershell.exe' -ArgumentList $argList -Verb RunAs -ErrorAction Stop
    } catch {
        # The person said no to Windows' question (or it couldn't be asked).
        Write-Host ('Windows didn''t give ClaudeOS Own System permission to make changes, so nothing was changed. (' +
                    $_.Exception.Message + ')')
        exit 1
    }
    exit 0
}

$WorkDir = Join-Path $env:LOCALAPPDATA 'ClaudeOS\own-os'
New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
$LogPath = Join-Path $WorkDir 'own-os.log'
Initialize-SetupLog $LogPath
Use-LongTempPath
if (-not $Unattended) { $null = Disable-ConsoleQuickEdit }
Unblock-SetupFiles -Root $RepoRoot

$exitCode = 0
try {
    try { $host.UI.RawUI.WindowTitle = 'ClaudeOS Own System' } catch { $null = $_ }
    Write-Line ''
    Write-Host 'ClaudeOS''s own system' -ForegroundColor White
    Write-Line 'This starts ClaudeOS''s own operating system on this PC: its own start-up and kernel, written from'
    Write-Line 'scratch, with nothing from Linux or other projects. So far it shows one screen: what it found on this'
    Write-Line 'PC, and how long it has been running. No USB stick is needed, and Windows isn''t changed: one small file'
    Write-Line 'goes on the partition the PC starts from, next to Windows'' own files.'
    $mode = 0
    if ($Remove) {
        $mode = 1
    } elseif ((Get-OwnBootEntries).Count -gt 0 -and -not $Unattended) {
        $mode = Read-Choice 'What would you like to do?' @(
            'Start ClaudeOS''s own system (at the next restart, once)',
            'Remove it from this PC') 0
    }
    if ($mode -eq 1) {
        Remove-OwnSystem -Unattended:$Unattended
    } else {
        Start-OwnSystem -RepoRoot $RepoRoot -WorkDir $WorkDir -Unattended:$Unattended -NoRestart:$NoRestart -Out $Out
    }
} catch {
    Write-SetupLog ('FAILED: ' + $_.Exception.ToString())
    Write-Host ''
    Write-Warn ('Stopped: ' + (Protect-SetupText $_.Exception.Message))
    Write-Line ('Details are in ' + $LogPath)
    $exitCode = 1
}
if ($Relaunched) {
    Read-Host 'Press Enter to close this window' | Out-Null
}
exit $exitCode
