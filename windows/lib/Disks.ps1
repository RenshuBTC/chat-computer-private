# Where ClaudeOS goes: free space next to Windows, or a separate internal disk.
#
# Safety rules (also in ARCHITECTURE.md):
# * The Ubuntu installer is only ever told to use FREE space ("use_gap"). It never reformats a disk.
# * Any space-making happens here, on Windows, after the person has confirmed it:
#   shrinking C: (like Disk Management's "Shrink Volume"), or erasing a separate disk they typed
#   the number of. The disk Windows runs from and USB drives can never be erased.
# * The installer installs onto the disk it starts from: setup puts the installer partition on the disk
#   ClaudeOS goes on, at the end of ClaudeOS's free space (Installer.ps1), and the answers file says
#   "install-media: true". So the installer can't pick another disk.
# ASCII-only file (see Common.ps1).

$Script:GB = [int64]1073741824
$Script:MinInstallBytes = 40 * $Script:GB          # smallest gap ClaudeOS will install into
$Script:DefaultInstallBytes = 64 * $Script:GB      # what setup suggests
$Script:MinWindowsFreeBytes = 25 * $Script:GB      # Windows keeps at least this much free space

function Get-DiskLabel {
    param([Parameter(Mandatory = $true)]$Disk)
    $name = [string](Get-Prop $Disk 'FriendlyName')
    if (-not $name) { $name = [string](Get-Prop $Disk 'Model') }
    return ('Disk ' + $Disk.Number + ': ' + $name.Trim() + ' (' + (Format-Size $Disk.Size) + ')')
}

function Get-DiskVolumesText {
    param([Parameter(Mandatory = $true)][int]$Number)
    $lines = @()
    try {
        foreach ($p in @(Get-Partition -DiskNumber $Number -ErrorAction Stop)) {
            $vol = $null
            try { $vol = $p | Get-Volume -ErrorAction Stop } catch { $vol = $null }
            $letter = [string](Get-Prop $p 'DriveLetter')
            $what = ''
            if ($letter -and $letter -ne [char]0) { $what = $letter + ': ' }
            if ($vol) {
                $label = [string](Get-Prop $vol 'FileSystemLabel')
                $fs = [string](Get-Prop $vol 'FileSystem')
                $used = [double]$vol.Size - [double]$vol.SizeRemaining
                $what += ($label + ' ' + $fs + ', ' + (Format-Size $used) + ' used').Trim()
            } else {
                $what += ([string](Get-Prop $p 'Type'))
            }
            $lines += ('    ' + $what + ' (' + (Format-Size $p.Size) + ')')
        }
    } catch {
        $lines += '    (could not read the partitions)'
    }
    return ($lines -join "`n")
}

function Get-SystemDiskPlan {
    # The disk Windows runs from: how much free space it has, and how far C: can shrink.
    $c = Get-Partition -DriveLetter C -ErrorAction Stop
    $disk = Get-Disk -Number $c.DiskNumber -ErrorAction Stop
    $supported = Get-PartitionSupportedSize -DriveLetter C -ErrorAction Stop
    $vol = Get-Volume -DriveLetter C -ErrorAction Stop
    $free = [int64](Get-Prop $disk 'LargestFreeExtent')
    $maxShrinkByLayout = [int64]$c.Size - [int64]$supported.SizeMin
    $maxShrinkByFreeSpace = [int64]$vol.SizeRemaining - $Script:MinWindowsFreeBytes
    $maxShrink = [Math]::Max([int64]0, [Math]::Min($maxShrinkByLayout, $maxShrinkByFreeSpace))
    return [pscustomobject]@{
        Disk          = $disk
        Partition     = $c
        FreeBytes     = $free
        MaxShrink     = $maxShrink
        WindowsFree   = [int64]$vol.SizeRemaining
        PartitionStyle = [string](Get-Prop $disk 'PartitionStyle')
    }
}

function Get-OtherDiskCandidates {
    # Internal disks other than the one Windows runs from, big enough for ClaudeOS.
    $all = @(Get-Disk -ErrorAction Stop)
    return , @($all | Where-Object {
            -not $_.IsBoot -and -not $_.IsSystem -and
            ([string](Get-Prop $_ 'BusType')) -notin @('USB', 'SD', 'MMC', 'File Backed Virtual', 'Virtual') -and
            [int64]$_.Size -ge $Script:MinInstallBytes -and
            ([string](Get-Prop $_ 'OperationalStatus')) -eq 'Online'
        })
}

function Invoke-ShrinkWindows {
    param([Parameter(Mandatory = $true)][int64]$ShrinkBytes)
    $c = Get-Partition -DriveLetter C -ErrorAction Stop
    $newSize = [int64]$c.Size - $ShrinkBytes
    Write-SetupLog ('Shrinking C: from ' + $c.Size + ' to ' + $newSize)
    Resize-Partition -DriveLetter C -Size $newSize -ErrorAction Stop
}

function Invoke-EraseDisk {
    # Erases a separate disk the person chose and typed the number of. Never the Windows disk.
    param([Parameter(Mandatory = $true)][int]$Number)
    $disk = Get-Disk -Number $Number -ErrorAction Stop
    if ($disk.IsBoot -or $disk.IsSystem) { throw 'Refusing to erase the disk Windows runs from.' }
    if (([string](Get-Prop $disk 'BusType')) -in @('USB', 'SD', 'MMC')) { throw 'Refusing to erase a USB or SD drive here.' }
    Write-SetupLog ('Erasing disk ' + $Number)
    Clear-Disk -Number $Number -RemoveData -RemoveOEM -Confirm:$false -ErrorAction Stop
    $disk = Get-Disk -Number $Number
    if (([string](Get-Prop $disk 'PartitionStyle')) -eq 'RAW') {
        Initialize-Disk -Number $Number -PartitionStyle GPT -ErrorAction Stop
    }
}
