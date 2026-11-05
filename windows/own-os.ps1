<#
Chat Computer's own system (double-click "Chat Computer Own System.cmd").

Starts Chat Computer's own operating system (os/ in this folder: its own start-up stage and kernel, written from
scratch) on this PC, without a USB stick:
  1. checks this PC (read-only) and saves a list of its parts, for writing Chat Computer's drivers,
  2. signs Chat Computer's start-up file with a key made on this PC (the key never leaves it): the one the PC's
     Secure Boot already trusts, where there is one (ClaudeOS's, made by this before the name changed, too),
  3. says which model it answers with there: the one Chat Computer's app got ready on this PC (free, run on the PC
     itself, so nothing you type leaves it),
  4. puts the signed file, its settings, its memory file (where it keeps the conversation across restarts; kept
     when this runs again) and its log file (read back when this next runs, to last-run.txt in its work folder, and
     said in a line) on the partition the PC starts from (\EFI\chatcomputer-own), next to Windows' own files,
     and keeps "Chat Computer (own system)" in the power-on menu (the PC's boot menu), to choose whenever you like,
     taking ClaudeOS's old entries out of it (its own system's, its installers', and "Ubuntu" when that's the
     Ubuntu-based ClaudeOS: its mark is on the start-up partition), with Windows first; their files stay,
  5. restarts into it when you press Enter (closing the window stays in Windows). With no model on the PC yet, it
     starts all the same and says how to get one.
Windows' permission, then Enter: nothing else is asked. With Secure Boot on and no key the PC trusts (it reuses
the one approved for ClaudeOS), the key needs approving once in the PC's setup screen: it shows the steps (the
certificate is at the top of the start-up partition, the first file the setup screen lists), and Enter restarts
there. From then on, Enter here or "Chat Computer (own system)" in the boot menu starts it. The restart after it
is Windows.

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
        Write-Host ('Windows didn''t give Chat Computer Own System permission to make changes, so nothing was changed. (' +
                    $_.Exception.Message + ')')
        exit 1
    }
    exit 0
}

$WorkDir = Join-Path $env:LOCALAPPDATA 'ChatComputer\own-os'
New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
$LogPath = Join-Path $WorkDir 'own-os.log'
Initialize-SetupLog $LogPath
Use-LongTempPath
if (-not $Unattended) { $null = Disable-ConsoleQuickEdit }
Unblock-SetupFiles -Root $RepoRoot

$exitCode = 0
try {
    try { $host.UI.RawUI.WindowTitle = 'Chat Computer Own System' } catch { $null = $_ }
    Write-Line ''
    Write-Host 'Chat Computer''s own system' -ForegroundColor White
    Write-Line 'This starts Chat Computer''s own operating system on this PC. Windows isn''t changed: a few small files go'
    Write-Line 'on the partition the PC starts from, next to Windows'' own files.'
    # (A double-click starts it, with the newest files in this folder, every time: taking it off is -Remove.)
    if ($Remove) {
        Remove-OwnSystem -Unattended:$Unattended -WorkDir $WorkDir
    } else {
        Start-OwnSystem -RepoRoot $RepoRoot -WorkDir $WorkDir -Unattended:$Unattended -NoRestart:$NoRestart -Out $Out
    }
} catch {
    Write-SetupLog ('FAILED: ' + $_.Exception.ToString())
    Write-Host ''
    Write-Warn ('Stopped: ' + (Protect-SetupText (Get-ErrorText $_)))
    Write-Line ('Details are in ' + $LogPath)
    $exitCode = 1
}
if ($Relaunched) {
    Read-Host 'Press Enter to close this window' | Out-Null
}
exit $exitCode
