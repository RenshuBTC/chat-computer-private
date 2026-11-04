# What real Windows says about the things Chat Computer Setup relies on when it makes room.
# Run by .github/workflows/windows.yml on GitHub's Windows machines (Windows PowerShell 5.1, as an
# administrator). Saves what it saw into -Out; prints a short summary. Changes only that throwaway machine.
# ASCII-only file (Windows PowerShell 5.1 reads files without a BOM as ANSI).
param([Parameter(Mandatory = $true)][string]$Out)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Path $Out -Force | Out-Null
function Save([string]$Name, $Value) { ($Value | Out-String -Width 400) | Set-Content -LiteralPath (Join-Path $Out $Name) -Encoding UTF8 }
function Run([string]$Name, [string]$Command) {
    # A Windows command line through cmd, its output and exit code saved.
    $text = & cmd.exe /c ($Command + ' 2>&1')
    $code = $LASTEXITCODE
    Save $Name (@('> ' + $Command, ('exit code ' + $code)) + @($text))
    return $code
}
Save 'psversion.txt' $PSVersionTable
Save 'os.txt' (Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, OSLanguage)

function Save-Shrink([string]$Name) {
    # How far C: can shrink, and the newest "volume shrink analysis" event (ID 259) with its fields.
    $t0 = Get-Date
    $c = Get-Partition -DriveLetter C
    $s = Get-PartitionSupportedSize -DriveLetter C
    $v = Get-Volume -DriveLetter C
    $lines = @(('C: size ' + $c.Size + ', can shrink to ' + $s.SizeMin + ' (' + [Math]::Round(($c.Size - $s.SizeMin) / 1GB, 1) +
                ' GB), free ' + $v.SizeRemaining), ('asked at ' + $t0.ToString('o')))
    $ev = @(Get-WinEvent -FilterHashtable @{ LogName = 'Application'; Id = 259 } -MaxEvents 2 -ErrorAction SilentlyContinue)
    foreach ($e in $ev) {
        $lines += ('event ' + $e.Id + ' from ' + $e.ProviderName + ' at ' + $e.TimeCreated.ToString('o'))
        $i = 0
        foreach ($p in $e.Properties) { $lines += ('  [' + $i + '] ' + [string]$p.Value); $i++ }
        $lines += ('  message: ' + ($e.Message -replace "`r?`n", ' | '))
    }
    if ($ev.Count -gt 0) { $ev[0].ToXml() | Set-Content -LiteralPath (Join-Path $Out ($Name + '.xml')) -Encoding UTF8 }
    $lines | Set-Content -LiteralPath (Join-Path $Out ($Name + '.txt')) -Encoding UTF8
    Write-Host ($lines -join "`n")
}

# 1. Shrink analysis as it is.
Save-Shrink 'shrink-1'
# 2. With a shadow copy (Windows Server can make one by command; a PC's restore points are the same thing).
Run 'vss-create.txt' 'vssadmin create shadow /for=C:' | Out-Null
Run 'vss-list.txt' 'vssadmin list shadows /for=C:' | Out-Null
Save-Shrink 'shrink-2-shadow'
Run 'vss-delete.txt' 'vssadmin delete shadows /for=C: /quiet' | Out-Null
Run 'vss-delete-again.txt' 'vssadmin delete shadows /for=C: /quiet' | Out-Null
Save-Shrink 'shrink-3-noshadow'
# 3. The change journal: reset it, and see whether (and how fast) Windows starts a new one.
Run 'usn-before.txt' 'fsutil usn queryjournal C:' | Out-Null
Run 'usn-delete.txt' 'fsutil usn deletejournal /n C:' | Out-Null
Run 'usn-after-0s.txt' 'fsutil usn queryjournal C:' | Out-Null
Start-Sleep -Seconds 20
Run 'usn-after-20s.txt' 'fsutil usn queryjournal C:' | Out-Null
Save-Shrink 'shrink-4-nojournal'
# 4. Hibernation.
Run 'powercfg-a.txt' 'powercfg /a' | Out-Null
Run 'powercfg-hibernate-off.txt' 'powercfg /hibernate off' | Out-Null
# 5. A shrink by command, as setup does it (1 GB), then back.
$c = Get-Partition -DriveLetter C
try {
    Resize-Partition -DriveLetter C -Size ($c.Size - 1GB) -ErrorAction Stop
    $after = (Get-Partition -DriveLetter C).Size
    Resize-Partition -DriveLetter C -Size $c.Size -ErrorAction Stop
    Save 'resize.txt' ('shrank by ' + ($c.Size - $after) + ' bytes and grew back')
} catch { Save 'resize.txt' ('resize failed: ' + $_.Exception.Message) }
# 6. The file mark that makes Windows warn "The publisher could not be verified", and taking it off.
$f = Join-Path $Out 'marked.cmd'
Set-Content -LiteralPath $f -Value '@echo off' -Encoding ASCII
Set-Content -LiteralPath $f -Stream Zone.Identifier -Value "[ZoneTransfer]`r`nZoneId=3" -Encoding ASCII
$before = @(Get-Item -LiteralPath $f -Stream * | ForEach-Object { $_.Stream })
Unblock-File -LiteralPath $f
$afterStreams = @(Get-Item -LiteralPath $f -Stream * | ForEach-Object { $_.Stream })
Save 'unblock.txt' ('streams before: ' + ($before -join ', ') + "`nstreams after: " + ($afterStreams -join ', '))
Remove-Item -LiteralPath $f -Force
# 7. Programs setup opens windows with.
Save 'programs.txt' (@('explorer.exe: ' + (Test-Path -LiteralPath (Join-Path $env:WINDIR 'explorer.exe')),
                      'wt.exe: ' + [string](Get-Command wt.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
                      'TEMP: ' + [IO.Path]::GetTempPath()))
# 8. Setup's own helpers on real Windows.
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
foreach ($lib in @('Common', 'Timezones', 'Autoinstall', 'Disks', 'Network', 'Ubuntu', 'Installer', 'Checks', 'Startup', 'Flow')) {
    . ([IO.Path]::Combine($repo, 'windows', 'lib', $lib + '.ps1'))
}
# What stops C: from shrinking, as setup reads it.
$plan = Get-SystemDiskPlan
$b = Get-ShrinkBlocker -Since $plan.AskedAt
Save 'setup-plan.txt' (@(('gap after C: ' + $plan.GapAfterC + ', can shrink ' + $plan.MaxShrink + ' (layout ' + $plan.MaxShrinkByLayout +
                          ', free space ' + $plan.MaxShrinkByFreeSpace + ')'), ('in the way: ' + $b.File + ' (' + $b.Kind + ')')))
# QuickEdit off, in a console window of its own (the runner's steps have none).
$qe = Join-Path $Out 'quickedit.txt'
$child = "Set-StrictMode -Version 2.0; . '" + (Join-Path $repo 'windows\lib\Common.ps1') + "'; " +
         "`$r = Disable-ConsoleQuickEdit; Set-Content -LiteralPath '" + $qe + "' -Value ('Disable-ConsoleQuickEdit: ' + `$r)"
Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-Command', $child) -Wait -WindowStyle Minimized
if (-not (Test-Path -LiteralPath $qe)) { Save 'quickedit.txt' 'the console window did not run' }
Write-Host 'probe done'

