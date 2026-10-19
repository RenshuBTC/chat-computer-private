# Where Chat Computer goes: free space next to Windows, or a separate internal disk.
#
# Safety rules (also in ARCHITECTURE.md):
# * The Ubuntu installer is only ever told to use FREE space ("use_gap"). It never reformats a disk.
# * Any space-making happens here, on Windows, after the person has confirmed it:
#   shrinking C: (like Disk Management's "Shrink Volume"), or erasing a separate disk they typed
#   the number of. The disk Windows runs from and USB drives can never be erased.
# * The installer installs onto the disk it starts from: setup puts the installer partition on the disk
#   Chat Computer goes on, at the end of Chat Computer's free space (Installer.ps1), and the answers file says
#   "install-media: true". So the installer can't pick another disk.
# ASCII-only file (see Common.ps1).

$Script:GB = [int64]1073741824
$Script:MinInstallBytes = 40 * $Script:GB          # smallest gap Chat Computer will install into
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
        # A disk without partitions makes Get-Partition report an error; it has nothing to list.
        $parts = @(Get-Partition -DiskNumber $Number -ErrorAction SilentlyContinue)
        if ($parts.Count -eq 0) { $lines += '    (no partitions)' }
        foreach ($p in $parts) {
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
    # The disk Windows runs from: how much free space it has, how far C: can shrink, and how much empty space
    # there already is right after C: (where shrinking C: adds to it).
    # MaxShrink is how much C: can give now. Two things limit it: Windows keeps MinWindowsFreeBytes free
    # (MaxShrinkByFreeSpace), and C: can't shrink past the last file Windows can't move while it runs
    # (MaxShrinkByLayout; Get-ShrinkBlocker says which file that is).
    $c = Get-Partition -DriveLetter C -ErrorAction Stop
    $disk = Get-Disk -Number $c.DiskNumber -ErrorAction Stop
    $asked = (Get-Date).AddSeconds(-2)
    $supported = Get-PartitionSupportedSize -DriveLetter C -ErrorAction Stop
    $vol = Get-Volume -DriveLetter C -ErrorAction Stop
    $free = [int64](Get-Prop $disk 'LargestFreeExtent')
    $maxShrinkByLayout = [Math]::Max([int64]0, [int64]$c.Size - [int64]$supported.SizeMin)
    $maxShrinkByFreeSpace = [Math]::Max([int64]0, [int64]$vol.SizeRemaining - $Script:MinWindowsFreeBytes)
    $maxShrink = [Math]::Min($maxShrinkByLayout, $maxShrinkByFreeSpace)
    $gap = [int64]0
    try {
        $extents = Get-FreeExtents -Partitions @(Get-Partition -DiskNumber $c.DiskNumber -ErrorAction Stop) -DiskSize ([int64]$disk.Size)
        $cEnd = [int64]$c.Offset + [int64]$c.Size
        $after = @($extents | Where-Object { [int64]$_.Offset -ge $cEnd -and [int64]$_.Offset -lt $cEnd + $Script:MiB })
        if ($after.Count -gt 0) { $gap = [int64]$after[0].Size }
    } catch {
        Write-SetupLog ('Reading the empty space after C: ' + $_.Exception.Message)
    }
    Write-SetupLog ('C: ' + $c.Size + ' bytes, ' + $vol.SizeRemaining + ' free, can shrink to ' + $supported.SizeMin +
                    '; empty right after it ' + $gap + ', largest empty space on its disk ' + $free)
    return [pscustomobject]@{
        Disk                 = $disk
        Partition            = $c
        FreeBytes            = $free
        GapAfterC            = $gap
        MaxShrink            = $maxShrink
        MaxShrinkByLayout    = $maxShrinkByLayout
        MaxShrinkByFreeSpace = $maxShrinkByFreeSpace
        WindowsFree          = [int64]$vol.SizeRemaining
        PartitionStyle       = [string](Get-Prop $disk 'PartitionStyle')
        AskedAt              = $asked
    }
}

# ------------------------------------------------------------------ what stops C: from shrinking
# When Windows works out how far a drive can shrink (Get-PartitionSupportedSize does), it writes event 259 to
# the Application log naming the last file it can't move; C: can't shrink past that file. Microsoft's
# "Shrink a basic volume" says to check that event when a shrink stops short.
$Script:ShadowCopyFile = '{3808876b-c176-4e48-b7ae-04046e6cc752}'

function ConvertFrom-ShrinkEvent {
    # The file an event 259 names, from its fields (the same in every Windows language): the one that is a
    # path on the drive (\...), as opposed to the drive's name or its \\?\Volume{...} path. Falls back to the
    # English message.
    param([Parameter(Mandatory = $true)]$Event)
    foreach ($p in @(Get-Prop $Event 'Properties')) {
        $v = ([string](Get-Prop $p 'Value')).Trim()
        if ($v -match '^\\[^\\?]' -and $v -notmatch '^\\\\') { return $v }
    }
    $m = [regex]::Match([string](Get-Prop $Event 'Message'), 'unmovable file appears to be:\s*([^\r\n]+)')
    if ($m.Success) { return $m.Groups[1].Value.Trim() }
    return $null
}

function Get-ShrinkBlockerKind {
    # What kind of file stops the shrink, and so what can be done about it.
    #   restorepoints  System Restore's restore points (shadow copies): deleting them frees the space
    #   journal        the drive's change journal: resetting it is harmless (Windows starts a new one)
    #   hibernation    hiberfil.sys: turning hibernation off deletes it
    #   pagefile       pagefile.sys or swapfile.sys: only movable after a restart without a page file
    #   ntfs           the file system's own records ($Mft, $LogFile...): not movable while Windows runs
    #   other, unknown anything else, or no event
    param([AllowNull()][AllowEmptyString()][string]$File)
    if (-not $File) { return 'unknown' }
    if ($File -match '(?i)^\\System Volume Information\\' -and $File.ToLowerInvariant().Contains($Script:ShadowCopyFile)) { return 'restorepoints' }
    if ($File -match '(?i)^\\\$Extend\\\$UsnJrnl') { return 'journal' }
    if ($File -match '(?i)^\\hiberfil\.sys') { return 'hibernation' }
    if ($File -match '(?i)^\\(pagefile|swapfile)\.sys') { return 'pagefile' }
    if ($File -match '^\\\$') { return 'ntfs' }
    return 'other'
}

function Get-ShrinkBlocker {
    # The newest event 259 written since $Since (by the Get-PartitionSupportedSize behind Get-SystemDiskPlan):
    # the file in the way and its kind, or $null when Windows wrote none.
    param([Parameter(Mandatory = $true)][datetime]$Since)
    $ev = $null
    try {
        $ev = Get-WinEvent -FilterHashtable @{ LogName = 'Application'; Id = 259; StartTime = $Since } -MaxEvents 1 -ErrorAction Stop |
            Select-Object -First 1
    } catch {
        return $null
    }
    if (-not $ev) { return $null }
    $file = ConvertFrom-ShrinkEvent $ev
    $kind = Get-ShrinkBlockerKind $file
    Write-SetupLog ('In the way of shrinking C: ' + $file + ' (' + $kind + ')')
    return [pscustomobject]@{ File = $file; Kind = $kind }
}

function Invoke-WindowsTool {
    # Runs one of Windows' own command-line tools; logs what it said. Returns its exit code.
    param([Parameter(Mandatory = $true)][string]$Tool, [string[]]$Arguments = @())
    $text = & $Tool @Arguments 2>&1
    $code = $LASTEXITCODE
    Write-SetupLog ($Tool + ' ' + ($Arguments -join ' ') + ' -> exit ' + $code + ': ' + ((@($text) | ForEach-Object { [string]$_ }) -join ' / '))
    return $code
}

function Remove-RestorePoints {
    # Deletes the shadow copies on C: (System Restore's restore points are shadow copies). Microsoft's
    # KB5005357 gives this command; Windows makes new restore points by itself later.
    $null = Invoke-WindowsTool 'vssadmin.exe' @('delete', 'shadows', '/for=C:', '/quiet')
}

function Reset-ChangeJournal {
    # Deletes C:'s change journal (NTFS's running list of changed files). Windows starts a new one straight away;
    # programs that read it (Windows Search) look through the drive once more.
    $null = Invoke-WindowsTool 'fsutil.exe' @('usn', 'deletejournal', '/n', 'C:')
}

function Disable-Hibernation {
    # Turns hibernation off, which deletes hiberfil.sys. (Fast Startup, which needs it, is off anyway.)
    $null = Invoke-WindowsTool 'powercfg.exe' @('/hibernate', 'off')
}

function Invoke-MakeRoom {
    # Makes the empty space right after C: at least $NeedBytes big, and up to $WantBytes, as one would by hand:
    # shrink C: as far as Windows allows (never below MinWindowsFreeBytes free); when a file Windows can't move
    # is in the way, deal with it if possible and try again. The change journal is reset whenever it is in the
    # way (harmless); restore points are deleted and hibernation turned off only when the minimum can't be
    # reached otherwise. Returns what it did and the space it made: Gap, ShrunkBy, Blocker (what stopped it
    # short of $WantBytes, if anything), ClearedRestorePoints, HibernationOff.
    param([Parameter(Mandatory = $true)][int64]$NeedBytes, [Parameter(Mandatory = $true)][int64]$WantBytes, [int]$Rounds = 12)
    $result = [pscustomobject]@{ Gap = [int64]0; ShrunkBy = [int64]0; Blocker = $null; ClearedRestorePoints = $false
        HibernationOff = $false; JournalResets = 0 }
    for ($round = 1; $round -le $Rounds; $round++) {
        $plan = Get-SystemDiskPlan
        $gap = [int64]$plan.GapAfterC
        $result.Gap = $gap
        if ($gap -ge $WantBytes) { $result.Blocker = $null; break }
        $take = [int64]([Math]::Floor([double]([Math]::Min([int64]$plan.MaxShrink, $WantBytes - $gap)) / $Script:MiB)) * $Script:MiB
        if ($take -ge $Script:GB) {
            Write-Line ('Making room: C: gives ' + (Format-Size $take) + '. Windows moves files out of the way first, which can take a while...')
            Invoke-ShrinkWindows -ShrinkBytes $take
            $result.ShrunkBy += $take
            continue
        }
        if ([int64]$plan.MaxShrinkByFreeSpace -lt $Script:GB) {
            # Windows' own free space is what's left; nothing to clear.
            $result.Blocker = [pscustomobject]@{ File = $null; Kind = 'space' }
            break
        }
        $b = Get-ShrinkBlocker -Since $plan.AskedAt
        if (-not $b) { $b = [pscustomobject]@{ File = $null; Kind = 'unknown' } }
        $result.Blocker = $b
        if ($b.Kind -eq 'journal' -and $result.JournalResets -lt 4) {
            Write-Line 'Making room: resetting the drive''s change journal (Windows starts a new one by itself)...'
            Reset-ChangeJournal
            $result.JournalResets++
            continue
        }
        if ($gap -ge $NeedBytes) { break }     # enough for Chat Computer; the rest isn't worth deleting anything for
        if ($b.Kind -eq 'restorepoints' -and -not $result.ClearedRestorePoints) {
            Write-Line 'Making room: deleting your restore points (Windows makes new ones by itself)...'
            Remove-RestorePoints
            $result.ClearedRestorePoints = $true
            continue
        }
        if ($b.Kind -eq 'hibernation' -and -not $result.HibernationOff) {
            Write-Line 'Making room: turning hibernation off...'
            Disable-Hibernation
            $result.HibernationOff = $true
            continue
        }
        break
    }
    $result.Gap = [int64](Get-SystemDiskPlan).GapAfterC
    if ($result.Gap -ge $WantBytes) { $result.Blocker = $null }
    return $result
}

function Get-RoomAdvice {
    # What the person can do when Windows can't make enough room on C:, by what is in the way.
    param($Blocker, [int64]$Made, [int64]$NeedBytes)
    $have = 'Windows can''t make room for Chat Computer next to it right now.'
    if ($Made -ge $Script:GB) { $have = 'Windows could make ' + (Format-Size $Made) + ' of room next to it; Chat Computer needs ' + (Format-Size $NeedBytes) + '.' }
    $kind = 'unknown'
    $file = ''
    if ($Blocker) { $kind = [string]$Blocker.Kind; $file = [string]$Blocker.File }
    switch ($kind) {
        'space' { return ($have + ' C: needs more free space: free some up (Settings, System, Storage), then run setup again.') }
        'pagefile' { return ($have + ' Windows'' page file (' + $file.TrimStart('\') + ') is in the way, and Windows can only move it ' +
                             'after a restart without one. Turn it off for now: Settings, System, About, Advanced system settings, ' +
                             'Performance Settings, Advanced, Virtual memory: Change, untick "Automatically manage", choose "No paging ' +
                             'file", Set, OK. Restart, run setup again, then turn it back on the same way.') }
        'ntfs' { return ($have + ' The file system''s own records (' + $file.TrimStart('\') + ') are in the way, and Windows can''t ' +
                         'move them while it runs. Chat Computer can go on a separate disk instead, or a partition tool that works while ' +
                         'Windows is off can make the room.') }
        default {
            if ($file) { return ($have + ' A file Windows can''t move while it runs is in the way: ' + $file + '.') }
            return ($have + ' Something Windows can''t move while it runs is in the way.')
        }
    }
}

function Get-OtherDiskCandidates {
    # Internal disks other than the one Windows runs from, big enough for Chat Computer.
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
