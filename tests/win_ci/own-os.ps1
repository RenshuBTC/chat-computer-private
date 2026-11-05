# "Chat Computer Own System" on real Windows (GitHub's Windows machines). Signs Chat Computer's start-up file the way it
# is signed on a PC (a key made by Windows, Windows' own signing), and leaves the signed file and the key's
# certificate in -Out, where the Linux job starts it in a virtual PC with Secure Boot enforced. On a machine
# that starts in UEFI mode it then runs the whole thing unattended (without restarting): the start-up partition
# (with the memory file, kept when it runs again) and the power-on menu, and removing it again. The test key is
# deleted at the end.
# ASCII-only file (see windows\lib\Common.ps1).
param([Parameter(Mandatory = $true)][string]$Out)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0
$Repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$Win = Join-Path $Repo 'windows'
foreach ($lib in @('Common', 'Checks', 'Startup', 'OwnOs')) { . ([IO.Path]::Combine($Win, 'lib', $lib + '.ps1')) }
New-Item -ItemType Directory -Path $Out -Force | Out-Null
Initialize-SetupLog (Join-Path $Out 'own-os-test.log')

$failures = New-Object System.Collections.ArrayList
function Check([bool]$Ok, [string]$What) {
    if ($Ok) { Write-Host ('ok   ' + $What) } else { Write-Host ('FAIL ' + $What) -ForegroundColor Red; [void]$failures.Add($What) }
}

$fw = Get-FirmwareType
$sb = Get-SecureBootState
Write-Host ('Firmware: ' + $fw + ', Secure Boot: ' + $sb + ', PowerShell ' + $PSVersionTable.PSVersion)

# 1. Signing, as on a PC.
$release = Get-OwnRelease -RepoRoot $Repo
$hadKey = $null -ne (Get-OwnKey)
$cert = Get-OwnKey
if (-not $cert) { $cert = New-OwnKey }
Check ($cert.HasPrivateKey) 'the key has its private half'
$signed = Join-Path $Out $Script:OwnFile
New-OwnSignedFile -Release $release -Destination $signed -Cert $cert
[IO.File]::WriteAllBytes((Join-Path $Out $Script:OwnCert), [byte[]]$cert.RawData)
Check (Test-PeSigned $signed) 'the start-up file is signed'
$sig = Get-AuthenticodeSignature -FilePath $signed
Check ($null -ne $sig.SignerCertificate -and $sig.SignerCertificate.Thumbprint -eq $cert.Thumbprint) 'signed by the key made on this PC'
Write-Host ('Signature: ' + $sig.Status + ' (' + $sig.StatusMessage + ')')

# 2. The whole thing, unattended, without restarting, when this machine starts in UEFI mode.
if ($fw -eq 'UEFI') {
    $own = Join-Path $Win 'own-os.ps1'
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $own -Unattended -NoRestart
    Check ($LASTEXITCODE -eq 0) 'own-os.ps1 ran'
    $entries = Get-OwnBootEntries
    Check ($entries.Count -eq 1) 'one "Chat Computer (own system)" entry in the power-on menu'
    (& bcdedit.exe /enum firmware) | Out-File -FilePath (Join-Path $Out 'bcdedit-after.txt') -Encoding utf8
    $esp = Get-WindowsEsp
    $m = Mount-EspPartition $esp
    $memoryId = $null
    try {
        $dir = [IO.Path]::Combine($m.Root, 'EFI', $Script:OwnDir)
        $file = Join-Path $dir $Script:OwnFile
        Check (Test-Path -LiteralPath $file) 'the signed file is on the start-up partition'
        if (Test-Path -LiteralPath $file) { Check (Test-PeSigned $file) 'and it is signed' }
        Check (Test-Path -LiteralPath (Join-Path $dir $Script:OwnCert)) 'the certificate is next to it'
        Check (Test-Path -LiteralPath (Join-Path $m.Root $Script:OwnCert)) 'and at the partition''s top, for the setup screen'
        Check (Test-Path -LiteralPath ([IO.Path]::Combine($m.Root, 'EFI', 'Microsoft', 'Boot', 'bootmgfw.efi'))) 'Windows'' own start-up file is still there'
        # The memory file (the conversation, kept across restarts): there, with the settings' id in its tags, unless
        # the partition is short of room.
        $memoryId = Get-OwnSettingsMemoryId ([IO.File]::ReadAllText((Join-Path $dir $Script:OwnSettings)))
        $free = Get-OwnFreeSpace $m.Root
        Write-Host ('Memory id: ' + $memoryId + '; free on the start-up partition: ' + $free + ' bytes')
        if ($null -ne $memoryId) {
            Check (Test-OwnMemoryFile -Path (Join-Path $dir $Script:OwnMemory) -Id $memoryId) 'the memory file is there, with the settings'' id in its tags'
        } else {
            Check ($null -ne $free -and $free - 1MB -lt $Script:OwnEspRoom) 'no memory file only when the start-up partition is short of room'
        }
    } finally {
        Dismount-EspPartition $m
    }
    if ($null -ne $memoryId) {
        # Set up again: the same memory file stays (with the conversation in it).
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $own -Unattended -NoRestart
        Check ($LASTEXITCODE -eq 0) 'own-os.ps1 ran again'
        $m = Mount-EspPartition $esp
        try {
            $dir = [IO.Path]::Combine($m.Root, 'EFI', $Script:OwnDir)
            Check ((Get-OwnSettingsMemoryId ([IO.File]::ReadAllText((Join-Path $dir $Script:OwnSettings)))) -eq $memoryId) 'set up again, the memory file''s id is the same'
            Check (Test-OwnMemoryFile -Path (Join-Path $dir $Script:OwnMemory) -Id $memoryId) 'and the memory file is still that one'
        } finally {
            Dismount-EspPartition $m
        }
        Check ((Get-OwnBootEntries).Count -eq 1) 'still one entry in the power-on menu'
    }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $own -Remove -Unattended
    Check ($LASTEXITCODE -eq 0) 'own-os.ps1 -Remove ran'
    Check ((Get-OwnBootEntries).Count -eq 0) 'the entry is gone'
    Check ($null -eq (Get-OwnKey)) 'the key is gone'
    $m = Mount-EspPartition $esp
    try {
        Check (-not (Test-Path -LiteralPath ([IO.Path]::Combine($m.Root, 'EFI', $Script:OwnDir)))) 'its folder is gone from the start-up partition'
        Check (-not (Test-Path -LiteralPath (Join-Path $m.Root $Script:OwnCert))) 'and the certificate at its top'
    } finally {
        Dismount-EspPartition $m
    }
    $log = Join-Path $env:LOCALAPPDATA 'ChatComputer\own-os\own-os.log'
    if (Test-Path -LiteralPath $log) { Copy-Item -LiteralPath $log -Destination $Out }
} else {
    Write-Host 'This machine does not start in UEFI mode: the start-up partition and power-on menu parts are skipped.'
}

if (-not $hadKey) {
    $left = Get-OwnKey
    if ($left) { Remove-Item -LiteralPath ('Cert:\CurrentUser\My\' + $left.Thumbprint) -DeleteKey }
}
Write-Host ('' + $failures.Count + ' failed')
if ($failures.Count -gt 0) { exit 1 }
exit 0
