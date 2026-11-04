# Walks through the whole setup conversation on a pretend PC (no Windows needed).
#   pwsh -NoProfile -File windows/tests/Flow-Test.ps1 -Scenario NAME -OutDir DIR
# The pretend PC: a 1 TB NVMe drive with Windows (C:), a 2 TB data drive, Wi-Fi "Home 5G", AirPods paired,
# Fast Startup on. Windows' own commands are replaced by stand-ins that record what they were asked to do
# and keep track of the disks' partitions, the power-on menu and the EFI partition (a folder).
# Scenarios:
#   plan, shrink, disk         install (plan only; making room on C:; on the separate disk)
#   blocked                    install on a PC like the one setup was first tried on: one disk, C: can't shrink
#                              at first because of restore points, then the change journal, then $Mft
#   gap63                      install into 63 GB already empty after C: (made by hand), $Mft in the way of more
#   stuck                      install when Windows' page file stops C: from shrinking at all: stops, undoes
#   nospace                    install when C: hasn't the free space: stops before changing anything
#   nodocs                     install when the recovery password can't be saved in Documents (shown instead)
#   no-entry, install-fails    install, when the PC won't take the power-on entry / when copying fails
#   leftover                   install, while the installer of an earlier try is still there
#   bitlocker, bitlocker-fails install on an encrypted C: (Chat Computer gets its own key) / when a later step fails
#   bitlocker-stop             stop at the recovery-key question
#   startup-windows, startup-once
#   remove, remove-keep        remove Chat Computer (confirmed / not confirmed)
#   remove-again               remove Chat Computer, keeping the downloads to install it again
#   remove-both                remove Chat Computer before it has cleaned up its installer
#   remove-unfinished          remove an installer that never installed
param(
    [Parameter(Mandatory = $true)][ValidateSet('plan', 'shrink', 'disk', 'no-entry', 'install-fails', 'leftover', 'bitlocker', 'bitlocker-fails', 'bitlocker-stop',
        'startup-windows', 'startup-once', 'remove', 'remove-keep', 'remove-again', 'remove-both', 'remove-unfinished',
        'blocked', 'gap63', 'stuck', 'nospace', 'nodocs')][string]$Scenario,
    [Parameter(Mandatory = $true)][string]$OutDir
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0
$Here = $PSScriptRoot
$Win = Split-Path -Parent $Here
$Repo = Split-Path -Parent $Win
foreach ($lib in @('Common', 'Timezones', 'Autoinstall', 'Disks', 'Network', 'Ubuntu', 'Installer', 'Checks', 'Startup', 'Flow')) {
    . ([IO.Path]::Combine($Win, 'lib', $lib + '.ps1'))
}

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$Script:Calls = New-Object System.Collections.ArrayList
function Add-Call([string]$What) { [void]$Script:Calls.Add($What) }
$Script:MockGB = [int64]1073741824
$Script:MockMB = [int64]1048576
$Script:MockTests = $Here
$Script:SetupId = '{5e7e0a00-0000-4000-8000-00000000000a}'     # the "Chat Computer setup" entry bcdedit /copy makes

# ------------------------------------------------------------------ the pretend PC
$env:firmware_type = 'UEFI'
$env:USERNAME = 'sammy'
$env:USERPROFILE = (Join-Path $OutDir 'Users/sammy')
$Script:BitLocker = ($Scenario -in @('bitlocker', 'bitlocker-fails', 'bitlocker-stop'))
# Chat Computer installed (and its installer cleaned up, except in remove-both); or only an installer, never used.
$Script:Installed = $Scenario -in @('remove', 'remove-keep', 'remove-again', 'remove-both', 'startup-windows', 'startup-once')
# A PC with just the one disk, like most laptops (no "Where should Chat Computer go?" question then).
$Script:OneDisk = $Scenario -in @('blocked', 'gap63', 'stuck', 'nospace')
$Script:InstallerLeft = $Scenario -in @('remove-both', 'remove-unfinished', 'leftover')
$Script:Disk0Size = 277 * $Script:MockMB + 951 * $Script:MockGB + 2 * $Script:MockMB
$Script:Disks = @(
    [pscustomobject]@{ Number = 0; FriendlyName = 'Samsung SSD 990 PRO 1TB'; Model = 'Samsung SSD 990 PRO 1TB'; SerialNumber = '0025_3881_0123_4567.'
        Size = $Script:Disk0Size; BusType = 'NVMe'; IsBoot = $true; IsSystem = $true; OperationalStatus = 'Online'; PartitionStyle = 'GPT'; LargestFreeExtent = [int64]0 },
    [pscustomobject]@{ Number = 1; FriendlyName = 'WDC WD20EZAZ-00GGJB0'; Model = 'WDC WD20EZAZ-00GGJB0'; SerialNumber = 'WD-WX12A3456789'
        Size = 2000 * $Script:MockGB; BusType = 'SATA'; IsBoot = $false; IsSystem = $false; OperationalStatus = 'Online'; PartitionStyle = 'GPT'; LargestFreeExtent = [int64]0 })
if ($Script:OneDisk) { $Script:Disks = @($Script:Disks[0]) }

# The partitions, in bytes. Disk 0: EFI, reserved, C: (950 GB before Chat Computer), then Chat Computer's space, then
# Windows' recovery partition. Disk 1: reserved, D: (data).
$Script:Parts = New-Object System.Collections.ArrayList
$Script:TypeEsp = '{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}'
$Script:TypeData = '{ebd0a0a2-b9e5-4433-87c0-68b6b72699c7}'
$Script:TypeMsr = '{e3c9e316-0b5c-4db8-817d-f92df00215ae}'
function Add-MockPartition {
    param([int]$Disk, [int]$Number, [int64]$Offset, [int64]$Size, [string]$Type, [string]$GptType, [string]$Guid,
          [bool]$IsSystem = $false, [bool]$IsBoot = $false, $DriveLetter = [char]0, [string]$VolumePath = '')
    foreach ($p in $Script:Parts) {
        if ($p.DiskNumber -eq $Disk -and $Offset -lt $p.Offset + $p.Size -and $p.Offset -lt $Offset + $Size) {
            throw ('The pretend disk ' + $Disk + ' has no room there: partition ' + $p.PartitionNumber + ' is in the way.')
        }
    }
    $p = [pscustomobject]@{ DiskNumber = $Disk; PartitionNumber = $Number; Offset = $Offset; Size = $Size; Type = $Type
        GptType = $GptType; Guid = $Guid; IsSystem = $IsSystem; IsBoot = $IsBoot; DriveLetter = $DriveLetter; VolumePath = $VolumePath }
    [void]$Script:Parts.Add($p)
    return $p
}
$cOff = 277 * $Script:MockMB
$cGb = 950
if ($Script:Installed) { $cGb = 870 }
if ($Script:InstallerLeft) { $cGb = 864 }
if ($Scenario -eq 'gap63') { $cGb = 887 }      # 63 GB already empty after C:
$null = Add-MockPartition 0 1 (1 * $Script:MockMB) (260 * $Script:MockMB) 'System' $Script:TypeEsp '{0a000000-0000-0000-0000-000000000001}' $true
$null = Add-MockPartition 0 2 (261 * $Script:MockMB) (16 * $Script:MockMB) 'Reserved' $Script:TypeMsr '{0a000000-0000-0000-0000-000000000002}'
$null = Add-MockPartition 0 3 $cOff ($cGb * $Script:MockGB) 'Basic' $Script:TypeData '{0a000000-0000-0000-0000-000000000003}' $false $true 'C'
$null = Add-MockPartition 0 4 ($cOff + 950 * $Script:MockGB) (1 * $Script:MockGB) 'Recovery' '{de94bba4-06d1-4d40-a16a-bfd50179d6ac}' '{0a000000-0000-0000-0000-000000000004}'
if ($Script:Installed) {
    $null = Add-MockPartition 0 5 ($cOff + $cGb * $Script:MockGB) (80 * $Script:MockGB) 'Unknown' '{0fc63daf-8483-4772-8e79-3d69d8477de4}' '{3F2A9C1E-5B7D-4E8F-9A0B-1C2D3E4F5A6B}'
}
# The installer of an earlier run: the last 6 GB of the 86 GB taken from C:, with its mark.
$Script:LeftoverVolume = Join-Path $OutDir 'leftover-installer'
if ($Script:InstallerLeft) {
    $n = 5
    if ($Script:Installed) { $n = 6 }
    $null = Add-MockPartition 0 $n ($cOff + 944 * $Script:MockGB) (6 * $Script:MockGB) 'Basic' $Script:TypeData '{6a000000-0000-0000-0000-000000000006}' `
        $false $false ([char]0) $Script:LeftoverVolume
    New-Item -ItemType Directory -Path (Join-Path $Script:LeftoverVolume 'chatcomputer') -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $Script:LeftoverVolume 'chatcomputer/installer-0123456789abcdef01234567'), "id=0123456789abcdef01234567`n")
}
if (-not $Script:OneDisk) {
    $null = Add-MockPartition 1 1 (1 * $Script:MockMB) (16 * $Script:MockMB) 'Reserved' $Script:TypeMsr '{1a000000-0000-0000-0000-000000000001}'
    $null = Add-MockPartition 1 2 (17 * $Script:MockMB) (1999 * $Script:MockGB) 'Basic' $Script:TypeData '{1a000000-0000-0000-0000-000000000002}' $false $false 'D'
}
function Get-MockC { return ($Script:Parts | Where-Object { [string]$_.DriveLetter -eq 'C' } | Select-Object -First 1) }

function Confirm-SecureBootUEFI { return $true }
function Get-BitLockerVolume {
    $status = 'FullyDecrypted'
    $prot = @()
    if ($Script:BitLocker) {
        $status = 'FullyEncrypted'
        $prot = @([pscustomobject]@{ KeyProtectorType = 'RecoveryPassword'; RecoveryPassword = '111111-222222-333333-444444-555555-666666-777777-888888' })
    }
    $on = 'Off'
    if ($Scenario -eq 'remove') { $on = 'On' }
    return [pscustomobject]@{ VolumeStatus = $status; KeyProtector = $prot; ProtectionStatus = $on }
}
# BitLocker's key protectors on C: (id -> type; 3 = recovery password), through its WMI class. In 'remove',
# Chat Computer's own key is there, with setup's record of it.
$env:CHATCOMPUTER_STATE_DIR = Join-Path $OutDir 'ProgramData/ChatComputer'
$Script:Protectors = [ordered]@{ '{0b000000-0000-4000-8000-000000000001}' = 3 }
$Script:OurKeyId = '{2b000000-0000-4000-8000-0000000000c1}'
$Script:OurKey = '123456-234567-345678-456789-567890-678901-789012-890123'
if ($Scenario -eq 'remove') {
    $Script:Protectors[$Script:OurKeyId] = 3
    New-Item -ItemType Directory -Path $env:CHATCOMPUTER_STATE_DIR -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $env:CHATCOMPUTER_STATE_DIR 'bitlocker-key-id'), $Script:OurKeyId + "`n")
}
function Get-CimInstance { param([string]$Namespace, [string]$ClassName, [string]$Filter) return [pscustomobject]@{ DriveLetter = 'C:' } }
function Invoke-CimMethod {
    param($InputObject, [string]$MethodName, [hashtable]$Arguments)
    $id = ''
    if ($Arguments -and $Arguments.ContainsKey('VolumeKeyProtectorID')) { $id = [string]$Arguments['VolumeKeyProtectorID'] }
    switch ($MethodName) {
        'ProtectKeyWithNumericalPassword' {
            Add-Call 'BitLocker add key'
            $Script:Protectors[$Script:OurKeyId] = 3
            return [pscustomobject]@{ ReturnValue = 0; VolumeKeyProtectorID = $Script:OurKeyId }
        }
        'GetKeyProtectorNumericalPassword' { return [pscustomobject]@{ ReturnValue = 0; NumericalPassword = $Script:OurKey } }
        'GetKeyProtectorType' {
            if ($Script:Protectors.Contains($id)) { return [pscustomobject]@{ ReturnValue = 0; KeyProtectorType = $Script:Protectors[$id] } }
            return [pscustomobject]@{ ReturnValue = 2150694920; KeyProtectorType = 0 }     # 0x80310008: no such key
        }
        'DeleteKeyProtector' {
            Add-Call ('BitLocker delete key ' + $id)
            $Script:Protectors.Remove($id)
            return [pscustomobject]@{ ReturnValue = 0 }
        }
    }
    throw ('The pretend BitLocker has no ' + $MethodName)
}
function Get-ItemProperty { return [pscustomobject]@{ HiberbootEnabled = 1; PEFirmwareType = 2 } }
function Disable-FastStartup { Add-Call 'Disable-FastStartup' }
function Get-PnpDevice { return @([pscustomobject]@{ FriendlyName = 'Sam''s AirPods Pro' }, [pscustomobject]@{ FriendlyName = 'Sam''s AirPods Pro' }) }
function Get-NetAdapter { return @([pscustomobject]@{ Status = 'Up'; PhysicalMediaType = 'Native 802.11' }) }
function netsh.exe {
    $a = @($args)
    if ($a[0] -eq 'wlan' -and $a[1] -eq 'show') {
        return @('There is 1 interface on the system:', '    Name                   : Wi-Fi', '    SSID                   : Home 5G',
                 '    BSSID                  : 12:34:56:78:9a:bc', '    Profile                : Home 5G')
    }
    if ($a[0] -eq 'wlan' -and $a[1] -eq 'export') {
        $folder = ($a | Where-Object { $_ -like 'folder=*' } | Select-Object -First 1).Substring(7)
        $xml = '<?xml version="1.0"?><WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1"><name>Home 5G</name>' +
               '<SSIDConfig><SSID><name>Home 5G</name></SSID></SSIDConfig><MSM><security><authEncryption><authentication>WPA2PSK</authentication>' +
               '<encryption>AES</encryption></authEncryption><sharedKey><keyType>passPhrase</keyType><protected>false</protected>' +
               '<keyMaterial>correct horse</keyMaterial></sharedKey></security></MSM></WLANProfile>'
        [IO.File]::WriteAllText((Join-Path $folder 'Wi-Fi-Home 5G.xml'), $xml)
        return 'Interface profile "Home 5G" is saved.'
    }
}
function Get-Disk {
    param([int]$Number = -1)
    foreach ($d in $Script:Disks) {
        $free = Get-FreeExtents -Partitions @($Script:Parts | Where-Object { $_.DiskNumber -eq $d.Number }) -DiskSize ([int64]$d.Size)
        $d.LargestFreeExtent = [int64]0
        if ($free.Count -gt 0) { $d.LargestFreeExtent = [int64]$free[0].Size }
    }
    if ($Number -ge 0) { return ($Script:Disks | Where-Object { $_.Number -eq $Number }) }
    return $Script:Disks
}
function Update-Disk { param([int]$Number) Add-Call ('Update-Disk ' + $Number) }
function Get-Partition {
    param([Parameter(ValueFromPipeline = $true)]$InputObject, [string]$DriveLetter, [int]$DiskNumber = -1, [int]$PartitionNumber = -1)
    process {
        $all = @($Script:Parts | Sort-Object DiskNumber, PartitionNumber)
        if ($InputObject) {
            return @($all | Where-Object { $_.DiskNumber -eq $InputObject.DiskNumber -and $_.PartitionNumber -eq $InputObject.PartitionNumber })
        }
        if ($DriveLetter) {
            $hit = @($all | Where-Object { [string]$_.DriveLetter -eq $DriveLetter })
            if ($hit.Count -eq 0) { throw ('No partition has drive letter ' + $DriveLetter) }
            return $hit[0]
        }
        if ($DiskNumber -ge 0) { $all = @($all | Where-Object { $_.DiskNumber -eq $DiskNumber }) }
        if ($PartitionNumber -ge 0) {
            $all = @($all | Where-Object { $_.PartitionNumber -eq $PartitionNumber })
            if ($all.Count -eq 0) { throw ('No partition ' + $PartitionNumber + ' on disk ' + $DiskNumber) }
        }
        return $all
    }
}
# Files Windows can't move while it runs, by how far into C: they reach (C: can't shrink below that), as on
# the PC setup was first tried on: a restore point at the very end, the change journal 2 GB before it (Windows
# starts a new journal 37 GB from the end after the first reset, then far down), and $Mft's bitmap 63 GB in.
$Script:Unmovable = [ordered]@{}
$Script:JournalSpots = New-Object System.Collections.Queue
$Script:Svi = '\System Volume Information\{9db92f71-c6a1-11f1-ae98-00155d606220}{3808876b-c176-4e48-b7ae-04046e6cc752}::$DATA'
$Script:Journal = '\$Extend\$UsnJrnl:$J:$DATA'
switch ($Scenario) {
    'blocked' {
        $Script:Unmovable[$Script:Svi] = 950 * $Script:MockGB
        $Script:Unmovable[$Script:Journal] = 948 * $Script:MockGB
        $Script:Unmovable['\$Mft::$BITMAP'] = 887 * $Script:MockGB
        $Script:JournalSpots.Enqueue(913 * $Script:MockGB)
        $Script:JournalSpots.Enqueue(200 * $Script:MockGB)
    }
    'gap63' { $Script:Unmovable['\$Mft::$BITMAP'] = 887 * $Script:MockGB }
    'stuck' { $Script:Unmovable['\pagefile.sys'] = 950 * $Script:MockGB }
}
$Script:LastBlocker = $null
function Get-PartitionSupportedSize {
    # C: can shrink to 300 GB (or to the last file Windows can't move), and grow up to the next partition.
    # Like Windows, working that out writes event 259 naming that last file.
    $c = Get-MockC
    $next = @($Script:Parts | Where-Object { $_.DiskNumber -eq $c.DiskNumber -and $_.Offset -gt $c.Offset } | Sort-Object Offset)
    $end = $Script:Disk0Size - $Script:MockMB
    if ($next.Count -gt 0) { $end = $next[0].Offset }
    $min = 300 * $Script:MockGB
    $Script:LastBlocker = $null
    foreach ($k in $Script:Unmovable.Keys) {
        if ([int64]$Script:Unmovable[$k] -gt $min) { $min = [int64]$Script:Unmovable[$k]; $Script:LastBlocker = $k }
    }
    return [pscustomobject]@{ SizeMin = $min; SizeMax = $end - $c.Offset }
}
function Get-WinEvent {
    param([hashtable]$FilterHashtable, [int]$MaxEvents)
    if (-not $Script:LastBlocker) { throw 'No events were found that match the specified selection criteria.' }
    return [pscustomobject]@{ Id = 259; ProviderName = 'Microsoft-Windows-Defrag'; TimeCreated = (Get-Date)
        Properties = @([pscustomobject]@{ Value = 'Windows (C:)' }, [pscustomobject]@{ Value = '\\?\Volume{cb9ec628-0753-4fba-a216-523bf2d0eb7c}' },
                       [pscustomobject]@{ Value = $Script:LastBlocker }, [pscustomobject]@{ Value = '0x1d175df3' }, [pscustomobject]@{ Value = '0x18d4dda5' })
        Message = ('A volume shrink analysis was initiated on volume Windows (C:). ... The last unmovable file appears to be: ' + $Script:LastBlocker) }
}
function vssadmin.exe {
    Add-Call ('vssadmin ' + ($args -join ' '))
    $gone = @($Script:Unmovable.Keys | Where-Object { $_ -like '\System Volume Information\*' })
    foreach ($k in $gone) { $Script:Unmovable.Remove($k) }
    $global:LASTEXITCODE = 0
    if ($gone.Count -eq 0) { $global:LASTEXITCODE = 1; return 'No items found that satisfy the query.' }
}
function fsutil.exe {
    Add-Call ('fsutil ' + ($args -join ' '))
    if ($Script:Unmovable.Contains($Script:Journal)) {
        if ($Script:JournalSpots.Count -gt 0) { $Script:Unmovable[$Script:Journal] = $Script:JournalSpots.Dequeue() }
        else { $Script:Unmovable.Remove($Script:Journal) }
    }
    $global:LASTEXITCODE = 0
}
function powercfg.exe {
    Add-Call ('powercfg ' + ($args -join ' '))
    if ($Script:Unmovable.Contains('\hiberfil.sys')) { $Script:Unmovable.Remove('\hiberfil.sys') }
    $global:LASTEXITCODE = 0
}
function Get-Volume {
    param([Parameter(ValueFromPipeline = $true)]$InputObject, [string]$DriveLetter)
    process {
        if ($DriveLetter -eq 'C') {
            $freeGb = 400
            if ($Scenario -eq 'nospace') { $freeGb = 50 }
            return [pscustomobject]@{ Size = (Get-MockC).Size; SizeRemaining = $freeGb * $Script:MockGB }
        }
        if ($InputObject) {
            if ([string]$InputObject.DriveLetter -notmatch '^[A-Z]$') { return }
            return [pscustomobject]@{ Size = $InputObject.Size; SizeRemaining = [int64]($InputObject.Size / 2); FileSystemLabel = 'Data'; FileSystem = 'NTFS' }
        }
        # Every volume: Windows, and any CHATCOMP installer partition.
        $vols = @([pscustomobject]@{ FileSystemLabel = 'Windows'; FileSystem = 'NTFS'; Path = (Join-Path $OutDir 'nothing-here'); DiskNumber = 0; PartitionNumber = 3 })
        foreach ($p in @($Script:Parts | Where-Object { $_.VolumePath })) {
            $vols += [pscustomobject]@{ FileSystemLabel = 'CHATCOMP'; FileSystem = 'FAT32'; Path = $p.VolumePath; DiskNumber = $p.DiskNumber; PartitionNumber = $p.PartitionNumber }
        }
        return $vols
    }
}
function Resize-Partition {
    param([string]$DriveLetter, [int64]$Size)
    Add-Call ('Resize-Partition ' + $DriveLetter + ' ' + ($Size / $Script:MockGB))
    $supported = Get-PartitionSupportedSize
    if ($Size -gt $supported.SizeMax -or $Size -lt $supported.SizeMin) { throw 'Size Not Supported' }
    (Get-MockC).Size = $Size
}
function Remove-Partition {
    param([int]$DiskNumber, [int]$PartitionNumber)
    Add-Call ('Remove-Partition ' + $DiskNumber + ' ' + $PartitionNumber)
    $p = @($Script:Parts | Where-Object { $_.DiskNumber -eq $DiskNumber -and $_.PartitionNumber -eq $PartitionNumber })
    if ($p.Count -ne 1) { throw 'No such partition' }
    $Script:Parts.Remove($p[0])
}
function Clear-Disk {
    param([int]$Number)
    Add-Call ('Clear-Disk ' + $Number)
    foreach ($p in @($Script:Parts | Where-Object { $_.DiskNumber -eq $Number })) { $Script:Parts.Remove($p) }
    (Get-Disk -Number $Number).PartitionStyle = 'RAW'
}
function Initialize-Disk {
    param([int]$Number, [string]$PartitionStyle)
    Add-Call ('Initialize-Disk ' + $Number + ' ' + $PartitionStyle)
    (Get-Disk -Number $Number).PartitionStyle = $PartitionStyle
}
function Get-TimeZone { return [pscustomobject]@{ Id = 'Singapore Standard Time' } }
function Get-LocalUser { return [pscustomobject]@{ FullName = 'Sam Lee' } }
function Get-UbuntuIso {
    Add-Call 'Get-UbuntuIso'
    $iso = Join-Path $OutDir 'ubuntu-24.04.5-live-server-amd64.iso'
    [IO.File]::WriteAllBytes($iso, [byte[]]@())
    if ($env:OS -eq 'Windows_NT') {
        # A sparse file, as on Linux, or every scenario would take 4 GB of the test machine's disk.
        & (Join-Path $env:WINDIR 'System32\fsutil.exe') sparse setflag $iso | Out-Null
    }
    $fs = [IO.File]::OpenWrite($iso)
    try { $fs.SetLength([int64]4080486400) } finally { $fs.Close() }      # as big as the real one, without the bytes
    return $iso
}
# The installer partition setup makes, as a folder (its real steps are tested in Run-Tests.ps1).
$Script:InstallerRoot = Join-Path $OutDir 'installer'
function New-InstallerPartition {
    param([int]$DiskNumber, [int64]$Offset, [int64]$Size)
    Add-Call ('New-InstallerPartition ' + $DiskNumber + ' ' + ($Offset / $Script:MockMB) + ' ' + ($Size / $Script:MockMB))
    $n = [int](1 + (@($Script:Parts | Where-Object { $_.DiskNumber -eq $DiskNumber } | ForEach-Object { $_.PartitionNumber }) + 0 | Measure-Object -Maximum).Maximum)
    $null = Add-MockPartition $DiskNumber $n $Offset $Size 'Basic' $Script:TypeData '{7a000000-0000-0000-0000-000000000007}'
    New-Item -ItemType Directory -Path $Script:InstallerRoot -Force | Out-Null
    return [pscustomobject]@{ Root = $Script:InstallerRoot; Letter = 'I'; DiskNumber = $DiskNumber; PartitionNumber = $n; Offset = $Offset; Size = $Size }
}
function Close-InstallerPartition { param($Installer) Add-Call 'Close-InstallerPartition' }
function Copy-IsoToInstaller {
    param([string]$IsoPath, [string]$Root)
    Add-Call 'Copy-IsoToInstaller'
    if ($Scenario -eq 'install-fails') { throw 'The installer files did not copy correctly.' }
    New-Item -ItemType Directory -Path ([IO.Path]::Combine($Root, '.disk')), ([IO.Path]::Combine($Root, 'boot', 'grub')),
        ([IO.Path]::Combine($Root, 'EFI', 'boot')) -Force | Out-Null
    [IO.File]::WriteAllText([IO.Path]::Combine($Root, '.disk', 'info'), 'Ubuntu-Server 24.04.5 LTS')
    foreach ($f in @('bootx64.efi', 'grubx64.efi', 'mmx64.efi')) { [IO.File]::WriteAllText([IO.Path]::Combine($Root, 'EFI', 'boot', $f), $f) }
    Copy-Item -LiteralPath (Join-Path $Script:MockTests 'data/ubuntu-24.04.5-grub.cfg') -Destination ([IO.Path]::Combine($Root, 'boot', 'grub', 'grub.cfg'))
}
function Invoke-Robocopy {
    # As robocopy /XD does: a folder named alone is left out wherever it is; one given with its path, only that
    # one. /XF: files matching a pattern are left out.
    param([string]$From, [string]$To, [string[]]$ExcludeDirs = @(), [string[]]$ExcludeFiles = @())
    Add-Call ('Invoke-Robocopy ' + (Split-Path -Leaf $From))
    $names = @($ExcludeDirs | Where-Object { -not (Split-Path -Parent $_) })
    $paths = @($ExcludeDirs | Where-Object { Split-Path -Parent $_ } |
        ForEach-Object { [IO.Path]::GetFullPath(($_ -replace '\\', [IO.Path]::DirectorySeparatorChar)).TrimEnd([IO.Path]::DirectorySeparatorChar) })
    $copy = {
        param([string]$Src, [string]$Dst)
        New-Item -ItemType Directory -Path $Dst -Force | Out-Null
        foreach ($item in Get-ChildItem -LiteralPath $Src -Force) {
            if ($item.PSIsContainer) {
                if ($names -contains $item.Name -or $paths -contains $item.FullName.TrimEnd([IO.Path]::DirectorySeparatorChar)) { continue }
                & $copy $item.FullName (Join-Path $Dst $item.Name)
            } elseif (-not @($ExcludeFiles | Where-Object { $item.Name -like $_ }).Count) {
                Copy-Item -LiteralPath $item.FullName -Destination (Join-Path $Dst $item.Name) -Force
            }
        }
    }
    & $copy $From $To
}
function shutdown.exe { Add-Call ('shutdown ' + ($args -join ' ')) }
function Suspend-BitLocker { param([string]$MountPoint, [int]$RebootCount) Add-Call ('Suspend-BitLocker ' + $MountPoint + ' ' + $RebootCount) }

# The PC's power-on menu: (Chat Computer,) Windows, a network boot entry, and what setup adds.
$Script:BcdBase = ([IO.File]::ReadAllText((Join-Path $Script:MockTests 'data/bcdedit-firmware.txt')) -replace "`r", '').TrimEnd("`n")
if (-not $Script:Installed) {
    $Script:BcdBase = $Script:BcdBase -replace '\nFirmware Application \(101fffff\)\n-+\nidentifier\s+\{9f62d3e2[^\n]*\n[^\n]*\n[^\n]*\n[^\n]*\n', "`n"
}
$Script:BcdAdded = [ordered]@{}
if ($Script:InstallerLeft) { $Script:BcdAdded[$Script:SetupId] = '\EFI\chatcomputer-setup\shimx64.efi' }
function bcdedit.exe {
    $a = @($args)
    $global:LASTEXITCODE = 0
    if ($a[0] -eq '/enum') {
        $text = $Script:BcdBase
        foreach ($id in $Script:BcdAdded.Keys) {
            $text += "`n`nFirmware Application (101fffff)`n-------------------------------`nidentifier              " + $id +
                     "`ndevice                  partition=\Device\HarddiskVolume1`npath                    " + $Script:BcdAdded[$id] +
                     "`ndescription             Chat Computer setup"
        }
        return ($text -split "`n")
    }
    Add-Call ('bcdedit ' + ($a -join ' '))
    if ($a[0] -eq '/copy') {
        if ($Scenario -eq 'no-entry') { $global:LASTEXITCODE = 1; return 'The boot configuration data store could not be opened.' }
        $Script:BcdAdded[$Script:SetupId] = '\EFI\Microsoft\Boot\bootmgfw.efi'
        return ('The entry was successfully copied to ' + $Script:SetupId + '.')
    }
    if ($a[0] -eq '/set' -and $a[2] -eq 'path' -and $Script:BcdAdded.Contains($a[1])) { $Script:BcdAdded[$a[1]] = $a[3] }
    if ($a[0] -eq '/delete' -and $Script:BcdAdded.Contains($a[1])) { $Script:BcdAdded.Remove($a[1]) }
}

# The EFI partition, as a folder: Windows' boot manager; once installed, Ubuntu's boot loader (also in the
# fallback slot) and Chat Computer's mark; while an installer is left, its boot loader.
$Script:Esp = Join-Path $OutDir 'esp'
New-Item -ItemType Directory -Path (Join-Path $Script:Esp 'EFI/Microsoft/Boot') -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $Script:Esp 'EFI/Microsoft/Boot/bootmgfw.efi'), 'Windows Boot Manager')
if ($Script:Installed) {
    foreach ($d in @('EFI/ubuntu', 'EFI/chatcomputer', 'EFI/BOOT')) { New-Item -ItemType Directory -Path (Join-Path $Script:Esp $d) -Force | Out-Null }
    [IO.File]::WriteAllText((Join-Path $Script:Esp 'EFI/ubuntu/shimx64.efi'), 'shim')
    [IO.File]::WriteAllText((Join-Path $Script:Esp 'EFI/ubuntu/grubx64.efi'), 'grub')
    [IO.File]::WriteAllText((Join-Path $Script:Esp 'EFI/BOOT/BOOTX64.EFI'), 'shim')
    [IO.File]::WriteAllText((Join-Path $Script:Esp 'EFI/BOOT/fbx64.efi'), 'fallback')
    [IO.File]::WriteAllText((Join-Path $Script:Esp 'EFI/chatcomputer/installed-0123456789abcdef01234567'),
        "id=0123456789abcdef01234567`nroot=3f2a9c1e-5b7d-4e8f-9a0b-1c2d3e4f5a6b`n")
}
if ($Script:InstallerLeft) {
    New-Item -ItemType Directory -Path (Join-Path $Script:Esp 'EFI/chatcomputer-setup') -Force | Out-Null
    foreach ($f in @('shimx64.efi', 'grubx64.efi', 'mmx64.efi')) { [IO.File]::WriteAllText((Join-Path $Script:Esp ('EFI/chatcomputer-setup/' + $f)), $f) }
}
if ($Script:Installed -or $Script:InstallerLeft) {
    $w = New-Item -ItemType Directory -Path (Join-Path $OutDir 'work') -Force
    [IO.File]::WriteAllText((Join-Path $w.FullName 'ubuntu-24.04.5-live-server-amd64.iso'), 'iso')
    # What an earlier setup (when Chat Computer thought with Claude) also left: Claude's engine and a sign-in copy.
    [IO.File]::WriteAllText((Join-Path $w.FullName 'claude_agent_sdk-0.2.167-py3-none-win_amd64.whl'), 'whl')
    $engine = New-Item -ItemType Directory -Path (Join-Path $w.FullName 'claude-139597261e6d') -Force
    [IO.File]::WriteAllText((Join-Path $engine.FullName 'claude.exe'), 'exe')
    $u = New-Item -ItemType Directory -Path (Join-Path $OutDir 'Users/sammy/ChatComputer') -Force
    [IO.File]::WriteAllText((Join-Path $u.FullName 'claude_token'), 'sk-ant-oat01-old')
    # In remove-both, the person also keeps a file of their own in that folder.
    if ($Scenario -eq 'remove-both') { [IO.File]::WriteAllText((Join-Path $u.FullName 'notes.txt'), 'mine') }
    $docs = New-Item -ItemType Directory -Path (Join-Path $OutDir 'Documents') -Force
    [IO.File]::WriteAllText((Join-Path $docs.FullName 'Chat Computer recovery password.txt'), 'Password: x')
}
# In 'bitlocker-fails', opening the EFI partition fails once, after Chat Computer's BitLocker key was made.
$Script:EspFailed = $false
function Mount-EspPartition {
    param($Partition)
    Add-Call ('Mount-EspPartition ' + $Partition.DiskNumber + ' ' + $Partition.PartitionNumber)
    if ($Scenario -eq 'bitlocker-fails' -and $Script:Protectors.Contains($Script:OurKeyId) -and -not $Script:EspFailed) {
        $Script:EspFailed = $true
        throw 'Windows could not open its EFI partition: access denied.'
    }
    return [pscustomobject]@{ Root = $Script:Esp; Letter = 'S'; How = 'test'; Partition = $Partition }
}
function Dismount-EspPartition { param($Mount) Add-Call 'Dismount-EspPartition' }

# The answers, in order. The first picks what to do: 1 install, 2 what starts, 3 remove.
$Script:Answers = New-Object System.Collections.Queue
# No menu unless Chat Computer (or an earlier installer) is already there.
$scripted = switch ($Scenario) {
    'plan' { @('1') }                                       # next to Windows (the pretend PC also has disk 1)
    'shrink' { @('1', 'y', 'n') }                           # next to Windows, go ahead, restart later
    'disk' { @('2', '1', 'y', 'y') }                        # the separate disk, type 1, go ahead, restart
    'no-entry' { @('1', 'y') }
    'install-fails' { @('1', 'y') }
    'leftover' { @('1') }                                   # (menu) install
    'bitlocker' { @('y', '1', 'y', 'n') }                   # recovery key saved, then as in shrink
    'bitlocker-fails' { @('y', '1', 'y') }
    'bitlocker-stop' { @('n') }                             # recovery key not saved
    'blocked' { @('y', 'n') }                               # one disk: go ahead, restart later
    'gap63' { @('y', 'n') }
    'stuck' { @('y') }
    'nospace' { @() }
    'nodocs' { @('1', 'y', 'n') }
    'startup-windows' { @('2', '2') }                        # Windows
    'startup-once' { @('2', '3', 'n') }                      # Chat Computer once, restart later
    'remove' { @('3', 'REMOVE', 'n') }                       # downloads deleted too
    'remove-keep' { @('3', 'no') }
    'remove-again' { @('3', 'REMOVE', 'y') }                 # the download kept, to install again
    'remove-both' { @('3', 'REMOVE', 'n') }
    'remove-unfinished' { @('3', 'REMOVE', 'n') }
}
foreach ($a in $scripted) { $Script:Answers.Enqueue($a) }
function Read-Host {
    param([Parameter(Position = 0)][string]$Prompt, [switch]$AsSecureString)
    if ($Script:Answers.Count -eq 0) { throw ('No scripted answer for: ' + $Prompt) }
    $a = [string]$Script:Answers.Dequeue()
    Add-Call ('Q ' + $Prompt + ' -> ' + $a)
    return $a
}

Initialize-SetupLog (Join-Path $OutDir 'setup.log')
$result = [ordered]@{ scenario = $Scenario; ok = $false; error = $null }
$docsDir = (New-Item -ItemType Directory -Path (Join-Path $OutDir 'Documents') -Force).FullName
if ($Scenario -eq 'nodocs') { $docsDir = Join-Path $docsDir 'not-there' }      # a folder that doesn't exist
try {
    Invoke-ChatComputerSetup -Here $Win -RepoRoot $Repo -WorkDir (Join-Path $OutDir 'work') -PlanOnly:($Scenario -eq 'plan') `
        -DocumentsDir $docsDir
    $result.ok = $true
} catch {
    $result.error = $_.Exception.Message
}
$result.calls = @($Script:Calls)
$result.answers_left = $Script:Answers.Count
$result.boot_entries = $Script:BcdAdded
$result.bitlocker_keys = @($Script:Protectors.Keys)
$result.partitions = @($Script:Parts | Sort-Object DiskNumber, Offset | ForEach-Object {
        [ordered]@{ disk = $_.DiskNumber; number = $_.PartitionNumber; start_mib = $_.Offset / $Script:MockMB; size_mib = $_.Size / $Script:MockMB; letter = ([string]$_.DriveLetter).Trim([char]0) } })
[IO.File]::WriteAllText((Join-Path $OutDir 'flow.json'), ($result | ConvertTo-Json -Depth 5))
