# The PC's power-on menu: the installer's one-time entry, what starts when the PC turns on (ClaudeOS or
# Windows), starting ClaudeOS once, and removing ClaudeOS completely. ASCII-only file (see Common.ps1).
#
# The PC's power-on menu is its UEFI boot entries; bcdedit reads and changes them. Its output is in
# Windows' language, so only values that are never translated are read from it: {identifiers} and
# \EFI\ paths. ClaudeOS's entry is the one that starts \EFI\ubuntu\ (Ubuntu's boot loader).

$Script:GptEsp = '{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}'
$Script:GptLinuxData = '{0fc63daf-8483-4772-8e79-3d69d8477de4}'

function ConvertFrom-BcdFirmware {
    # Turns `bcdedit /enum firmware` text into entries: Id ({guid} or {bootmgr}) and Path (\EFI\...).
    param([AllowEmptyString()][string]$Text)
    $entries = @()
    foreach ($block in [regex]::Split(($Text -replace "`r", ''), "\n[ \t]*\n")) {
        $id = [regex]::Match($block, '\{[0-9A-Za-z-]+\}')
        if (-not $id.Success) { continue }
        $path = [regex]::Match($block, '(?i)\\EFI\\[^\s]+')
        $entries += [pscustomobject]@{ Id = $id.Value; Path = $(if ($path.Success) { $path.Value } else { '' }) }
    }
    return , $entries
}

function Get-FirmwareBootEntries {
    $out = & bcdedit.exe /enum firmware 2>&1
    if ($LASTEXITCODE -ne 0) { throw ('Windows could not read the power-on menu: ' + (($out | Out-String).Trim())) }
    return (ConvertFrom-BcdFirmware (($out | ForEach-Object { [string]$_ }) -join "`n"))
}

function Get-ClaudeOSBootEntries {
    return , @(Get-FirmwareBootEntries | Where-Object { $_.Path -match '(?i)^\\EFI\\ubuntu\\' -and $_.Id -ne '{fwbootmgr}' })
}

function Invoke-Bcdedit {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    Write-SetupLog ('bcdedit ' + ($Arguments -join ' '))
    $out = & bcdedit.exe @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) { throw ('Windows could not change the power-on menu: ' + (($out | Out-String).Trim())) }
}

function Set-StartsFirst {
    # Puts one entry first in the power-on menu; the others keep their places.
    param([Parameter(Mandatory = $true)][string]$Id)
    Invoke-Bcdedit @('/set', '{fwbootmgr}', 'displayorder', $Id, '/addfirst')
}

# ------------------------------------------------------------------ the installer's power-on entry
function Get-SetupBootEntries {
    # "ClaudeOS setup": the entries that start \EFI\claudeos-setup\ (the installer's boot loader).
    return , @(Get-FirmwareBootEntries | Where-Object { $_.Path -match ('(?i)^\\EFI\\' + $Script:SetupLoaderDir + '\\') -and $_.Id -ne '{fwbootmgr}' })
}

function New-SetupBootEntry {
    # Adds "ClaudeOS setup" to the PC's power-on menu, starting \EFI\claudeos-setup\shimx64.efi. Returns its {id}.
    return (New-FirmwareBootEntry -Name $Script:SetupEntryName -Path ('\EFI\' + $Script:SetupLoaderDir + '\shimx64.efi'))
}

function New-FirmwareBootEntry {
    # Adds an entry called $Name to the end of the PC's power-on menu: a copy of Windows Boot Manager's entry, so
    # it points at the same EFI partition, that starts $Path (\EFI\...) instead. Returns its {id}.
    # The new entry is found by comparing the menu before and after, so nothing in Windows' language is read.
    param([Parameter(Mandatory = $true)][string]$Name, [Parameter(Mandatory = $true)][string]$Path)
    $before = @(Get-FirmwareBootEntries | ForEach-Object { $_.Id })
    Write-SetupLog ('bcdedit /copy {bootmgr} /d "' + $Name + '"')
    $out = & bcdedit.exe /copy '{bootmgr}' /d $Name 2>&1
    if ($LASTEXITCODE -ne 0) { throw ('Windows could not add "' + $Name + '" to the power-on menu: ' + (($out | Out-String).Trim())) }
    $after = @(Get-FirmwareBootEntries | ForEach-Object { $_.Id })
    $new = @($after | Where-Object { $before -notcontains $_ })
    $id = $null
    if ($new.Count -eq 1) {
        $id = $new[0]
    } else {
        $m = [regex]::Match((($out | ForEach-Object { [string]$_ }) -join ' '), '\{[0-9A-Fa-f-]{36}\}')
        if ($m.Success -and ($after -contains $m.Value)) { $id = $m.Value }
    }
    if (-not $id) { throw 'Windows added an entry to the power-on menu, but setup could not tell which one it is.' }
    try {
        Invoke-Bcdedit @('/set', $id, 'path', $Path)
        # Last in the menu (it's started once, through bootsequence), but there for the boot-menu key.
        Invoke-Bcdedit @('/set', '{fwbootmgr}', 'displayorder', $id, '/addlast')
    } catch {
        # Don't leave a half-made entry behind (until its path is set, it's a second Windows Boot Manager).
        $null = & bcdedit.exe /delete $id 2>&1
        throw
    }
    return $id
}

function Remove-SetupBootEntries {
    # Into a variable first: these functions return their list as one object, and "foreach ($e in Get-...)"
    # would go round once with the whole list.
    $entries = Get-SetupBootEntries
    foreach ($e in $entries) { Invoke-Bcdedit @('/delete', $e.Id) }
}

function Invoke-StartupChoice {
    param([switch]$PlanOnly)
    Write-Step 'What starts when the PC turns on'
    $entries = Get-ClaudeOSBootEntries
    if ($entries.Count -eq 0) {
        Write-Warn 'ClaudeOS is not in this PC''s power-on menu, so Windows starts. Nothing was changed.'
        return
    }
    $claude = $entries[0].Id
    $pick = Read-Choice 'What should start when the PC turns on?' @(
        'ClaudeOS (Windows stays one restart away: in ClaudeOS, say "switch to Windows")',
        'Windows (ClaudeOS stays installed; run this again to start it)',
        'Restart into ClaudeOS now, just this once') 0
    if ($PlanOnly) { Write-Line '(Plan only: nothing was changed.)'; return }
    if ($pick -eq 0) {
        Set-StartsFirst $claude
        Write-Good 'ClaudeOS now starts when the PC turns on.'
    } elseif ($pick -eq 1) {
        Set-StartsFirst '{bootmgr}'
        Write-Good 'Windows now starts when the PC turns on. ClaudeOS is still installed.'
        Write-Line 'To use ClaudeOS, run ClaudeOS Setup again and choose to restart into it, or pick it in the PC''s boot menu.'
        Write-Line 'To delete it completely, run ClaudeOS Setup again and choose "Remove ClaudeOS from this PC".'
    } else {
        Invoke-Bcdedit @('/set', '{fwbootmgr}', 'bootsequence', $claude)
        Write-Good 'The PC starts ClaudeOS next time, once. After that, it starts what it started before.'
        if (Read-YesNo 'Restart now?' $true) { & shutdown.exe /r /t 5 }
    }
}

# ------------------------------------------------------------------ removing ClaudeOS
function Get-FreeDriveLetter {
    $used = @(Get-PSDrive -PSProvider FileSystem -ErrorAction SilentlyContinue | ForEach-Object { [string]$_.Name })
    foreach ($l in @('S', 'R', 'Q', 'P', 'O', 'N', 'M', 'L', 'T', 'U', 'V', 'W', 'Y', 'Z')) {
        if ($used -notcontains $l) { return $l }
    }
    throw 'There is no free drive letter to look at the EFI partition with.'
}

function Mount-EspPartition {
    # Makes an EFI partition readable at a drive letter. Returns where, and how to undo it.
    param([Parameter(Mandatory = $true)]$Partition)
    $letter = Get-FreeDriveLetter
    if ([bool](Get-Prop $Partition 'IsSystem')) {
        $out = & mountvol.exe ($letter + ':') /s 2>&1
        if ($LASTEXITCODE -ne 0) { throw ('Windows could not open its EFI partition: ' + (($out | Out-String).Trim())) }
        $how = 'mountvol'
    } else {
        Add-PartitionAccessPath -DiskNumber $Partition.DiskNumber -PartitionNumber $Partition.PartitionNumber -AccessPath ($letter + ':\') -ErrorAction Stop
        $how = 'accesspath'
    }
    return [pscustomobject]@{ Root = ($letter + ':\'); Letter = $letter; How = $how; Partition = $Partition }
}

function Dismount-EspPartition {
    param([Parameter(Mandatory = $true)]$Mount)
    try {
        if ($Mount.How -eq 'mountvol') {
            $null = & mountvol.exe ($Mount.Letter + ':') /d 2>&1
        } else {
            Remove-PartitionAccessPath -DiskNumber $Mount.Partition.DiskNumber -PartitionNumber $Mount.Partition.PartitionNumber `
                -AccessPath ($Mount.Letter + ':\') -ErrorAction Stop
        }
    } catch {
        Write-SetupLog ('Could not close the EFI partition: ' + $_.Exception.Message)
    }
}

function Format-Guid {
    param([AllowNull()][string]$Guid)
    return ([string]$Guid).Trim().Trim('{', '}').ToLowerInvariant()
}

function Read-InstallMarker {
    # The file finish.sh leaves on the EFI partition: id=<install id> and root=<partition GUID>.
    param([Parameter(Mandatory = $true)][string]$Path)
    $info = @{ Id = ''; Root = '' }
    foreach ($line in [IO.File]::ReadAllLines($Path)) {
        if ($line -match '^id=([0-9a-f]+)\s*$') { $info.Id = $Matches[1] }
        elseif ($line -match '^root=([0-9A-Fa-f-]{36})\s*$') { $info.Root = $Matches[1].ToLowerInvariant() }
        elseif ($line -match '^([0-9a-f]+)\s*$' -and -not $info.Id) { $info.Id = $Matches[1] }
    }
    return [pscustomobject]$info
}

function Find-ClaudeOSInstall {
    # Looks on every EFI partition for ClaudeOS's marker, then finds ClaudeOS's own partition.
    $all = @(Get-Partition -ErrorAction Stop)
    foreach ($esp in @($all | Where-Object { (Format-Guid (Get-Prop $_ 'GptType')) -eq (Format-Guid $Script:GptEsp) })) {
        $m = Mount-EspPartition $esp
        $marker = $null
        $hasWindows = $false
        try {
            $dir = [IO.Path]::Combine($m.Root, 'EFI', 'claudeos')
            if (Test-Path -LiteralPath $dir) {
                $marker = Get-ChildItem -LiteralPath $dir -Filter 'installed-*' -ErrorAction SilentlyContinue | Select-Object -First 1
                if ($marker) { $marker = Read-InstallMarker $marker.FullName }
            }
            $hasWindows = Test-Path -LiteralPath ([IO.Path]::Combine($m.Root, 'EFI', 'Microsoft'))
        } finally {
            Dismount-EspPartition $m
        }
        if (-not $marker) { continue }
        $root = $null
        if ($marker.Root) {
            $root = $all | Where-Object { (Format-Guid (Get-Prop $_ 'Guid')) -eq $marker.Root } | Select-Object -First 1
        } else {
            # Installed before the marker named the partition: accept the one Linux partition on that disk.
            $linux = @($all | Where-Object { $_.DiskNumber -eq $esp.DiskNumber -and (Format-Guid (Get-Prop $_ 'GptType')) -eq (Format-Guid $Script:GptLinuxData) })
            if ($linux.Count -eq 1) { $root = $linux[0] }
        }
        return [pscustomobject]@{ Esp = $esp; EspHasWindows = $hasWindows; Id = $marker.Id; Root = $root }
    }
    return $null
}

function Clear-ClaudeOSBootFiles {
    # Deletes ClaudeOS's boot files from an EFI partition Windows shares. If Ubuntu's boot loader had
    # taken the PC's fallback slot (\EFI\BOOT\BOOTX64.EFI), Windows' boot manager goes back there.
    param([Parameter(Mandatory = $true)][string]$Root)
    $efi = [IO.Path]::Combine($Root, 'EFI')
    $fallback = [IO.Path]::Combine($efi, 'BOOT', 'BOOTX64.EFI')
    $shim = [IO.Path]::Combine($efi, 'ubuntu', 'shimx64.efi')
    $winboot = [IO.Path]::Combine($efi, 'Microsoft', 'Boot', 'bootmgfw.efi')
    if ((Test-Path -LiteralPath $fallback) -and (Test-Path -LiteralPath $shim) -and
        (Get-FileSha256 $fallback) -eq (Get-FileSha256 $shim)) {
        if (Test-Path -LiteralPath $winboot) {
            Copy-Item -LiteralPath $winboot -Destination $fallback -Force
        } else {
            Remove-Item -LiteralPath $fallback -Force
        }
        foreach ($f in @('fbx64.efi', 'mmx64.efi', 'BOOTX64.CSV')) {
            $p = [IO.Path]::Combine($efi, 'BOOT', $f)
            if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Force }
        }
    }
    foreach ($d in @('ubuntu', 'claudeos', $Script:SetupLoaderDir)) {
        $p = [IO.Path]::Combine($efi, $d)
        if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Recurse -Force }
    }
}

function Get-WindowsEsp {
    # The EFI partition Windows started from.
    return (Get-Partition -ErrorAction Stop | Where-Object { [bool](Get-Prop $_ 'IsSystem') } | Select-Object -First 1)
}

function Clear-SetupLoader {
    # Deletes \EFI\claudeos-setup (the installer's boot loader) from Windows' EFI partition, if it's there.
    $esp = Get-WindowsEsp
    if (-not $esp) { return }
    $m = Mount-EspPartition $esp
    try {
        $p = [IO.Path]::Combine($m.Root, 'EFI', $Script:SetupLoaderDir)
        if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Recurse -Force }
    } finally {
        Dismount-EspPartition $m
    }
}

function Read-ConfirmWord {
    param([Parameter(Mandatory = $true)][string]$Prompt, [Parameter(Mandatory = $true)][string]$Word)
    $answer = Read-Host $Prompt
    $match = ([string]$answer).Trim() -eq $Word
    Write-SetupLog ('Q: ' + $Prompt + ' -> ' + $(if ($match) { $Word } else { '(something else)' }))
    return $match
}

function Remove-ClaudeOS {
    param([Parameter(Mandatory = $true)][string]$WorkDir, [string]$DocumentsDir, [switch]$PlanOnly)
    Write-Step 'Remove ClaudeOS from this PC'
    $found = Find-ClaudeOSInstall
    # An installer that hasn't finished (or that ClaudeOS hasn't cleaned up yet) goes too.
    $installers = Find-InstallerPartitions
    $setupEntries = Get-SetupBootEntries
    if (-not $found -and $installers.Count -eq 0 -and $setupEntries.Count -eq 0) {
        Write-Line 'ClaudeOS is not installed on this PC (no EFI partition has its mark). Nothing was changed.'
        return
    }
    $root = $null
    if ($found) {
        $root = $found.Root
        if (-not $root) {
            throw 'ClaudeOS''s mark is here, but setup could not tell which partition is ClaudeOS''s. Nothing was changed.'
        }
        if ((Format-Guid (Get-Prop $root 'GptType')) -ne (Format-Guid $Script:GptLinuxData) -or [bool](Get-Prop $root 'IsSystem') -or
            [bool](Get-Prop $root 'IsBoot') -or ([string](Get-Prop $root 'DriveLetter')) -match '^[A-Za-z]$') {
            throw 'The partition ClaudeOS''s mark names does not look like ClaudeOS''s. Nothing was changed.'
        }
    }
    foreach ($i in $installers) {
        if ([bool](Get-Prop $i 'IsSystem') -or [bool](Get-Prop $i 'IsBoot')) {
            throw 'A partition that looks like ClaudeOS''s installer is one Windows needs. Nothing was changed.'
        }
    }
    $espToo = $found -and (-not $found.EspHasWindows) -and $found.Esp.DiskNumber -eq $root.DiskNumber
    $freed = [int64]0
    if ($root) {
        Write-Line ('This deletes ClaudeOS and everything saved in it: its partition on disk ' + $root.DiskNumber + ' (' +
                    (Format-Size $root.Size) + ').')
        $freed += [int64]$root.Size
    }
    $lead = 'This deletes ClaudeOS''s installer, left from an install that didn''t finish'
    if ($root) { $lead = 'It also deletes ClaudeOS''s installer' }
    foreach ($i in $installers) {
        Write-Line ($lead + ': its partition on disk ' + $i.DiskNumber + ' (' + (Format-Size $i.Size) + ').')
        $freed += [int64]$i.Size
    }
    if (-not $root -and $installers.Count -eq 0) {
        Write-Line 'It takes "ClaudeOS setup" out of the power-on menu (left from an install that didn''t finish).'
    }
    $hasKey = Test-Path -LiteralPath (Join-Path (Get-ClaudeOSStateDir) 'bitlocker-key-id')
    if ($hasKey) { Write-Line 'It deletes the BitLocker key setup made for ClaudeOS. Your own keys stay.' }
    Write-Line 'Windows, your apps and your files stay as they are. Windows starts when the PC turns on.'
    if ($root) { Write-Line 'Anything you saved only inside ClaudeOS is lost. Files you kept in your Windows folders stay.' }
    if (-not (Read-ConfirmWord 'Type REMOVE to remove ClaudeOS (anything else keeps it)' 'REMOVE')) {
        Write-Line 'Kept ClaudeOS. Nothing was changed.'
        return
    }
    if ($PlanOnly) { Write-Line '(Plan only: nothing was changed.)'; return }

    # 1. Windows first in the power-on menu, and ClaudeOS's entries (and the installer's) out of it.
    try {
        $bl = Get-BitLockerVolume -MountPoint 'C:' -ErrorAction Stop
        if ([string](Get-Prop $bl 'ProtectionStatus') -eq 'On') {
            # Changing the power-on menu can make BitLocker ask for its recovery key; pause it for one restart.
            Suspend-BitLocker -MountPoint 'C:' -RebootCount 1 -ErrorAction Stop | Out-Null
            Write-Line 'BitLocker is paused until the next restart, so Windows starts normally after this change.'
        }
    } catch {
        Write-SetupLog ('BitLocker check: ' + $_.Exception.Message)
    }
    Set-StartsFirst '{bootmgr}'
    $claudeEntries = Get-ClaudeOSBootEntries
    foreach ($e in $claudeEntries) { Invoke-Bcdedit @('/delete', $e.Id) }
    Remove-SetupBootEntries
    Write-Good 'Windows starts when the PC turns on; ClaudeOS is out of the power-on menu.'

    # 2. ClaudeOS's boot files on the EFI partition Windows uses (or that whole partition, if it was ClaudeOS's
    #    own) and the installer's, then ClaudeOS's partitions. Windows already starts by itself from here on.
    $diskNumbers = @()
    try {
        if ($found) {
            if ($espToo) {
                Remove-Partition -DiskNumber $found.Esp.DiskNumber -PartitionNumber $found.Esp.PartitionNumber -Confirm:$false -ErrorAction Stop
            } else {
                $m = Mount-EspPartition $found.Esp
                try { Clear-ClaudeOSBootFiles -Root $m.Root } finally { Dismount-EspPartition $m }
            }
        }
        Clear-SetupLoader
        if ($root) {
            Remove-Partition -DiskNumber $root.DiskNumber -PartitionNumber $root.PartitionNumber -Confirm:$false -ErrorAction Stop
            $diskNumbers += $root.DiskNumber
        }
        foreach ($i in $installers) {
            Remove-Partition -DiskNumber $i.DiskNumber -PartitionNumber $i.PartitionNumber -Confirm:$false -ErrorAction Stop
            $diskNumbers += $i.DiskNumber
        }
    } catch {
        throw ('Windows starts normally, but ClaudeOS''s files could not all be deleted (' + $_.Exception.Message +
               '). Run setup again to finish, or delete the partition in Disk Management.')
    }
    if ($root) { Write-Good 'ClaudeOS is deleted.' } else { Write-Good 'The installer is deleted.' }

    # ClaudeOS's BitLocker key: nothing can use it now.
    if ($hasKey) {
        try {
            if (Remove-ClaudeOSBitLockerKey) { Write-Good 'ClaudeOS''s BitLocker key is deleted.' }
        } catch {
            Write-SetupLog ('BitLocker key: ' + $_.Exception.Message)
            Write-Warn ('Setup could not delete the BitLocker key it made for ClaudeOS (' + $_.Exception.Message + '). ' +
                        'Run setup again to try once more.')
        }
    }

    # 3. Its space back to C: when it sits right after C:.
    $grown = $false
    try {
        foreach ($n in @($diskNumbers | Select-Object -Unique)) { Update-Disk -Number $n -ErrorAction SilentlyContinue }
        $c = Get-Partition -DriveLetter C -ErrorAction Stop
        if ($diskNumbers -contains $c.DiskNumber) {
            $max = [int64](Get-PartitionSupportedSize -DriveLetter C -ErrorAction Stop).SizeMax
            if ($max -gt [int64]$c.Size + 1GB) {
                Resize-Partition -DriveLetter C -Size $max -ErrorAction Stop
                Write-Good ('C: has its space back: it is now ' + (Format-Size $max) + '.')
                $grown = $true
            }
        }
    } catch {
        Write-SetupLog ('Growing C: ' + $_.Exception.Message)
    }
    if (-not $grown -and $freed -gt 0) {
        Write-Line ('Its ' + (Format-Size $freed) + ' are free space now. Windows'' Disk Management can turn them into a ' +
                    'drive, or add them to C:.')
    }

    # 4. What setup left on Windows: downloads, sign-in copies and the recovery password note (setup's log stays).
    if ($WorkDir -and (Test-Path -LiteralPath $WorkDir)) {
        Get-ChildItem -LiteralPath $WorkDir -Force | Where-Object { $_.Name -ne 'setup.log' } |
            Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    }
    # C:\Users\<you>\ClaudeOS: the sign-in copies, and the folder only when nothing else is in it (a folder of that
    # name may hold something of the person's own, or this very ClaudeOS folder).
    $signin = Join-Path $env:USERPROFILE 'ClaudeOS'
    if ([IO.Directory]::Exists($signin)) {
        try {
            foreach ($f in @('claude_login.json', 'claude_token')) { [IO.File]::Delete((Join-Path $signin $f)) }
            if (@([IO.Directory]::GetFileSystemEntries($signin)).Count -eq 0) { [IO.Directory]::Delete($signin, $false) }
        } catch {
            Write-SetupLog ('Deleting the sign-in copies: ' + $_.Exception.Message)
        }
    }
    Remove-OldSignInFolders
    $docs = $DocumentsDir
    if (-not $docs) { $docs = [Environment]::GetFolderPath('MyDocuments') }
    $note = Join-Path $docs 'ClaudeOS recovery password.txt'
    if (Test-Path -LiteralPath $note) { Remove-Item -LiteralPath $note -Force }
    Write-Good 'Done. ClaudeOS is removed and this PC is back to just Windows.'
}
