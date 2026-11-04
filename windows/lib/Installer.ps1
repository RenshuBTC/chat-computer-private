# The installer, on the PC's own disk (no USB stick). Ubuntu's installer files, the answers file and
# Chat Computer itself go into a temporary FAT32 partition labelled CHATCOMP, at the end of the free space left
# for Chat Computer. The PC starts it once through a power-on entry, "Chat Computer setup", whose boot loader
# (Ubuntu's signed shim and GRUB) sits on the EFI partition in \EFI\chatcomputer-setup. When Chat Computer first
# starts, it deletes that partition and grows into its space (linux/firstboot/chatcomputer-reclaim-installer).
# ARCHITECTURE.md, "Windows setup -> installer contract".
# ASCII-only file (see Common.ps1).

$Script:InstallerLabel = 'CHATCOMP'
$Script:InstallerBytes = [int64]6 * 1073741824     # Ubuntu's installer (about 3 GB), Chat Computer and room to spare
$Script:SetupLoaderDir = 'chatcomputer-setup'       # \EFI\chatcomputer-setup on the EFI partition
$Script:SetupEntryName = 'Chat Computer setup'
$Script:MiB = [int64]1048576

function Update-GrubConfig {
    # Turns the installer's boot menu into "Install Chat Computer": it starts by itself (the "autoinstall" word)
    # after 5 seconds. The newer-kernel entry comes first because new PCs often need it for Wi-Fi and
    # graphics; the original kernel stays as a second choice.
    # Once this installer has installed Chat Computer (finish.sh leaves /EFI/chatcomputer/installed-<id> on the PC's
    # EFI partition), the menu offers "Start Chat Computer" instead, so it never installs twice.
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$InstallId
    )
    if ($InstallId -notmatch '^[0-9a-f]{8,64}$') { throw 'Bad install id' }
    $t = $Text -replace "`r`n", "`n"
    $t2 = [regex]::Replace($t, '(?m)^set timeout=\d+[ \t]*$', 'set timeout=5')
    if ($t2 -eq $t -and $t -notmatch '(?m)^set timeout=5[ \t]*$') { throw 'Unexpected boot menu: no timeout line.' }
    $t = $t2

    $gaPattern = '(?s)menuentry "Try or Install Ubuntu Server" \{(.*?)\n\}'
    $hwePattern = '(?s)menuentry "Ubuntu Server with the HWE kernel" \{(.*?)\n\}'
    $ga = [regex]::Match($t, $gaPattern)
    $hwe = [regex]::Match($t, $hwePattern)
    if (-not $ga.Success -or -not $hwe.Success) { throw 'Unexpected boot menu: Ubuntu''s install entries were not found.' }

    # "subiquity-storage-version=2" makes the installer see free space next to existing partitions;
    # with its default (version 1) a disk that already has partitions offers no free space at all.
    $addAuto = {
        param([string]$Body)
        $b = [regex]::Replace($Body, '(?m)^(\s*linux\s+\S+)(\s+)', '$1 autoinstall subiquity-storage-version=2$2', 1)
        if ($b -eq $Body) { throw 'Unexpected boot menu: no kernel line.' }
        return $b
    }
    $hweBody = & $addAuto $hwe.Groups[1].Value
    $gaBody = & $addAuto $ga.Groups[1].Value
    $install = 'menuentry "Install Chat Computer" {' + $hweBody + "`n}`n" +
               'menuentry "Install Chat Computer (older kernel)" {' + $gaBody + "`n}"
    # Starting the installed system: the installer's GRUB reads the installed boot menu (the small grub.cfg
    # that Ubuntu keeps next to its boot loader on the EFI partition points to it). Chainloading the
    # installed shim instead doesn't work with Secure Boot on: GRUB 2.12 runs it without a file system,
    # so it can't find the rest of the boot loader.
    $marker = '/EFI/chatcomputer/installed-' + $InstallId
    $start = @(
        '# Chat Computer: if this installer has already installed Chat Computer on this PC, start it instead of installing again.',
        ('if search --no-floppy --file --set=chatcomputer_esp ' + $marker + '; then'),
        'menuentry "Start Chat Computer (already installed)" --id chatcomputer-start {',
        ("`tsearch --no-floppy --file --set=root " + $marker),
        "`tconfigfile /EFI/ubuntu/grub.cfg",
        '}',
        'set default=chatcomputer-start',
        'else'
    ) -join "`n"
    $newEntries = $start + "`n" + $install + "`nfi"

    # Replace the two original entries (in either order) with the new pair, at the first one's place.
    $blocks = @($ga, $hwe) | Sort-Object Index
    $firstEnd = $blocks[0].Index + $blocks[0].Length
    $before = $t.Substring(0, $blocks[0].Index)
    $between = $t.Substring($firstEnd, $blocks[1].Index - $firstEnd).Trim("`n")
    $after = $t.Substring($blocks[1].Index + $blocks[1].Length)
    $result = $before + $newEntries
    if ($between) { $result += "`n" + $between }
    $result += $after
    if (([regex]::Matches($result, ' autoinstall ')).Count -ne 2) { throw 'Boot menu change did not apply cleanly.' }
    return $result
}

# ------------------------------------------------------------------ where the installer partition goes
function Get-FreeExtents {
    # The free areas of a disk, from its partitions (Offset and Size in bytes), largest first, in whole
    # MiBs. The first and last MiB are left alone: the partition tables live there.
    param([AllowEmptyCollection()][object[]]$Partitions = @(), [Parameter(Mandatory = $true)][int64]$DiskSize)
    $parts = @($Partitions | Sort-Object { [int64](Get-Prop $_ 'Offset') })
    $raw = @()
    $pos = $Script:MiB
    foreach ($p in $parts) {
        $start = [int64](Get-Prop $p 'Offset')
        if ($start -gt $pos) { $raw += , @($pos, $start) }
        $end = $start + [int64](Get-Prop $p 'Size')
        if ($end -gt $pos) { $pos = $end }
    }
    $last = $DiskSize - $Script:MiB
    if ($last -gt $pos) { $raw += , @($pos, $last) }
    $extents = @()
    foreach ($r in $raw) {
        $s = [int64][Math]::Ceiling([double]$r[0] / $Script:MiB) * $Script:MiB
        $e = [int64][Math]::Floor([double]$r[1] / $Script:MiB) * $Script:MiB
        if ($e - $s -ge $Script:MiB) { $extents += [pscustomobject]@{ Offset = $s; Size = $e - $s } }
    }
    return , @($extents | Sort-Object -Property Size -Descending)
}

function Get-InstallerPlacement {
    # The installer partition goes at the end of the free area meant for Chat Computer. The installer then makes
    # Chat Computer's partition in the rest, right before it, which Chat Computer grows into it later.
    param([Parameter(Mandatory = $true)]$Extent, [int64]$InstallerBytes = $Script:InstallerBytes)
    $end = [int64]$Extent.Offset + [int64]$Extent.Size
    $offset = [int64][Math]::Floor([double]($end - $InstallerBytes) / $Script:MiB) * $Script:MiB
    return [pscustomobject]@{ Offset = $offset; Size = $end - $offset; ChatComputerBytes = $offset - [int64]$Extent.Offset }
}

# ------------------------------------------------------------------ Windows-only steps
function New-InstallerPartition {
    # Makes the temporary FAT32 partition, kept out of File Explorer, and opens it at a drive letter for setup.
    # If that doesn't work out, the new partition is deleted again.
    param(
        [Parameter(Mandatory = $true)][int]$DiskNumber,
        [Parameter(Mandatory = $true)][int64]$Offset,
        [Parameter(Mandatory = $true)][int64]$Size
    )
    Write-SetupLog ('Installer partition: disk ' + $DiskNumber + ', offset ' + $Offset + ', ' + $Size + ' bytes')
    $part = New-Partition -DiskNumber $DiskNumber -Offset $Offset -Size $Size -ErrorAction Stop
    $inst = [pscustomobject]@{ Root = ''; Letter = ''; DiskNumber = $DiskNumber; PartitionNumber = [int]$part.PartitionNumber
        Offset = [int64]$part.Offset; Size = [int64]$part.Size }
    try {
        Set-Partition -DiskNumber $DiskNumber -PartitionNumber $inst.PartitionNumber -NoDefaultDriveLetter $true -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
        Format-Volume -Partition $part -FileSystem FAT32 -NewFileSystemLabel $Script:InstallerLabel -Confirm:$false -Force -ErrorAction Stop | Out-Null
        $letter = Get-FreeDriveLetter
        Add-PartitionAccessPath -DiskNumber $DiskNumber -PartitionNumber $inst.PartitionNumber -AccessPath ($letter + ':\') -ErrorAction Stop
    } catch {
        $why = $_.Exception.Message
        try { Remove-InstallerPartition $inst } catch { Write-SetupLog ('Deleting the unfinished installer partition: ' + $_.Exception.Message) }
        throw ('Windows could not prepare the installer partition: ' + $why)
    }
    $inst.Letter = $letter
    $inst.Root = $letter + ':\'
    return $inst
}

function Remove-InstallerPartition {
    # Deletes the installer partition this run of setup made, after checking that it's still that one.
    param([Parameter(Mandatory = $true)]$Installer)
    $p = Get-Partition -DiskNumber $Installer.DiskNumber -PartitionNumber $Installer.PartitionNumber -ErrorAction Stop |
        Select-Object -First 1
    if (-not $p -or [int64]$p.Offset -ne [int64]$Installer.Offset -or [int64]$p.Size -ne [int64]$Installer.Size) {
        throw 'The installer partition is not where setup made it, so setup left it alone.'
    }
    Remove-Partition -DiskNumber $Installer.DiskNumber -PartitionNumber $Installer.PartitionNumber -Confirm:$false -ErrorAction Stop
}

function Write-InstallerMark {
    # chatcomputer\installer-<id>: tells "Remove Chat Computer" (and, later, Chat Computer itself) that this partition is
    # Chat Computer's installer. Written first, so even a half-made installer can be found and removed.
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)][string]$InstallId)
    if ($InstallId -notmatch '^[0-9a-f]{8,64}$') { throw 'Bad install id' }
    $dir = Join-Path $Root 'chatcomputer'
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    Write-TextFileLf -Path (Join-Path $dir ('installer-' + $InstallId)) -Text ("id=" + $InstallId + "`n")
}

function Close-InstallerPartition {
    # Takes its drive letter away again; the installer partition stays, out of sight, until Chat Computer starts.
    param([Parameter(Mandatory = $true)]$Installer)
    try {
        Remove-PartitionAccessPath -DiskNumber $Installer.DiskNumber -PartitionNumber $Installer.PartitionNumber `
            -AccessPath ($Installer.Letter + ':\') -ErrorAction Stop
    } catch {
        Write-SetupLog ('Could not close the installer partition: ' + $_.Exception.Message)
    }
}

function Invoke-Robocopy {
    param([string]$From, [string]$To, [string[]]$ExcludeDirs = @(), [string[]]$ExcludeFiles = @())
    # /A-:R: files on a disc image (the Ubuntu installer) are marked read-only, robocopy copies that mark,
    # and setup has to change one of the copies (grub.cfg). The copies are left without it.
    $robocopyArgs = @($From, $To, '/E', '/A-:R', '/R:2', '/W:2', '/NFL', '/NDL', '/NJH', '/NJS', '/NP')
    if ($ExcludeDirs.Count -gt 0) { $robocopyArgs += '/XD'; $robocopyArgs += $ExcludeDirs }
    if ($ExcludeFiles.Count -gt 0) { $robocopyArgs += '/XF'; $robocopyArgs += $ExcludeFiles }
    & robocopy.exe @robocopyArgs | Out-Null
    # Robocopy exit codes below 8 mean success.
    if ($LASTEXITCODE -ge 8) { throw ('Copying files failed (robocopy code ' + $LASTEXITCODE + ').') }
}

function Copy-IsoToInstaller {
    param([Parameter(Mandatory = $true)][string]$IsoPath, [Parameter(Mandatory = $true)][string]$Root)
    $img = Mount-DiskImage -ImagePath $IsoPath -PassThru -ErrorAction Stop
    try {
        $vol = $img | Get-Volume
        $src = ([string]$vol.DriveLetter) + ':\'
        Write-Line 'Copying the Ubuntu installer (a few minutes)...'
        Invoke-Robocopy -From $src -To $Root
    } finally {
        Dismount-DiskImage -ImagePath $IsoPath | Out-Null
    }
    $info = [IO.Path]::Combine($Root, '.disk', 'info')
    if (-not (Test-Path -LiteralPath $info)) { throw 'The installer files did not copy correctly.' }
}

function ConvertTo-UpdateSource {
    # Where a folder is on Windows' own drive, the way Chat Computer finds it there: the path below the top of the
    # drive, with forward slashes ("Users/Sam/Downloads/ChatComputer"). Empty for a folder on another drive, or with
    # a part Chat Computer won't follow.
    param([Parameter(Mandatory = $true)][string]$FullPath, [AllowEmptyString()][string]$SystemDrive = '')
    if (-not $SystemDrive) { return '' }
    $prefix = $SystemDrive.TrimEnd('\') + '\'
    $path = $FullPath.TrimEnd('\')
    if (-not $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { return '' }
    $parts = @($path.Substring($prefix.Length).Split('\') | Where-Object { $_ -ne '' })
    if ($parts.Count -eq 0) { return '' }
    foreach ($p in $parts) {
        if ($p -eq '.' -or $p -eq '..' -or $p -match '[\x00-\x1f\x7f/]') { return '' }
    }
    return ($parts -join '/')
}

function Get-TrueCasePath {
    # A folder's full path with each name written as the drive stores it. Windows doesn't mind the case of a
    # name; Linux, which reads the same drive for Chat Computer's updates, does. The path as given when a part
    # can't be looked up.
    param([Parameter(Mandatory = $true)][string]$Path)
    $full = [IO.Path]::GetFullPath($Path).TrimEnd('\')
    $root = [IO.Path]::GetPathRoot($full)
    if (-not $root -or -not [IO.Directory]::Exists($full)) { return $full }
    $out = $root.ToUpperInvariant()
    foreach ($part in $full.Substring($root.Length).Split('\')) {
        if (-not $part) { continue }
        $match = @([IO.Directory]::GetFileSystemEntries($out, $part))
        if ($match.Count -ne 1) { return $full }
        $out = [IO.Path]::Combine($out, [IO.Path]::GetFileName($match[0]))
    }
    return $out
}

function Get-UpdateSource {
    # Where this setup runs from, on Windows' drive (ConvertTo-UpdateSource): a newer Chat Computer put there updates
    # Chat Computer (chatcomputer/update.py). Empty when setup isn't on Windows' own drive.
    param([Parameter(Mandatory = $true)][string]$RepoRoot)
    if (-not $env:SystemDrive) { return '' }
    try { $full = Get-TrueCasePath -Path $RepoRoot } catch { return '' }
    return ConvertTo-UpdateSource -FullPath $full -SystemDrive $env:SystemDrive
}

function Write-InstallerPayload {
    # Chat Computer itself, its BitLocker key (if Windows' drive is encrypted), the settings, the answers file and
    # the boot menu change.
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$AutoinstallYaml,
        [Parameter(Mandatory = $true)][string]$SettingsJson,
        [Parameter(Mandatory = $true)][string]$InstallId,
        [AllowNull()]$BitLockerKey = $null
    )
    if ($InstallId -notmatch '^[0-9a-f]{8,64}$') { throw 'Bad install id' }
    $root = $Root
    $payload = Join-Path $root 'chatcomputer'
    $app = Join-Path $payload 'app'
    New-Item -ItemType Directory -Path $app -Force | Out-Null
    Write-Line 'Copying Chat Computer...'
    # The PC needs neither setup tool (Windows', this one, nor the Mac's), nor the website, the apps and their
    # checks, nor anything built from the own system's source here (gigabytes, where it was built).
    $skipDirs = @('.venv', '.git', '.pytest_cache', '__pycache__', 'node_modules', (Join-Path $RepoRoot 'windows'),
        (Join-Path $RepoRoot 'mac'), (Join-Path $RepoRoot 'docs'), (Join-Path $RepoRoot 'apps'), (Join-Path $RepoRoot 'evals'),
        (Join-Path $RepoRoot 'os\target'), (Join-Path $RepoRoot 'os\out'))
    Invoke-Robocopy -From $RepoRoot -To $app -ExcludeDirs $skipDirs -ExcludeFiles @('*.pyc', 'Chat Computer Setup.command')
    foreach ($script in @('guard.sh', 'network.sh', 'finish.sh')) {
        $text = [IO.File]::ReadAllText([IO.Path]::Combine((Split-Path -Parent $PSScriptRoot), 'installer', $script))
        Write-TextFileLf -Path (Join-Path $payload $script) -Text $text
    }
    $secrets = Join-Path $payload 'secrets'
    New-Item -ItemType Directory -Path $secrets -Force | Out-Null
    if ($BitLockerKey) {
        # Chat Computer's own recovery password for C: (finish.sh puts it where Chat Computer opens Windows' drive with it).
        Write-TextFileLf -Path (Join-Path $secrets 'bitlocker-key') -Text ([string]$BitLockerKey.Password + "`n")
    }
    Write-TextFileLf -Path (Join-Path $payload 'settings.json') -Text $SettingsJson
    # Where this setup runs from: a newer Chat Computer put in this folder updates Chat Computer (finish.sh).
    $updateSource = Get-UpdateSource -RepoRoot $RepoRoot
    $updateFile = Join-Path $payload 'update-source'
    if ($updateSource) { Write-TextFileLf -Path $updateFile -Text ($updateSource + "`n") }
    elseif (Test-Path -LiteralPath $updateFile) { Remove-Item -LiteralPath $updateFile -Force }
    Write-TextFileLf -Path (Join-Path $root 'autoinstall.yaml') -Text $AutoinstallYaml
    $grub = [IO.Path]::Combine($root, 'boot', 'grub', 'grub.cfg')
    $patched = Update-GrubConfig -Text ([IO.File]::ReadAllText($grub)) -InstallId $InstallId
    Write-TextFileLf -Path $grub -Text $patched
}

function Install-SetupLoader {
    # Ubuntu's signed boot loader from the installer files (shim, which Microsoft signs, then GRUB, which
    # Canonical signs) into \EFI\chatcomputer-setup on the EFI partition, where "Chat Computer setup" starts it. GRUB
    # then finds the installer partition by itself: its built-in start looks for the disk with /.disk/info.
    param([Parameter(Mandatory = $true)][string]$InstallerRoot, [Parameter(Mandatory = $true)][string]$EspRoot)
    $src = [IO.Path]::Combine($InstallerRoot, 'EFI', 'boot')
    $dest = [IO.Path]::Combine($EspRoot, 'EFI', $Script:SetupLoaderDir)
    New-Item -ItemType Directory -Path $dest -Force | Out-Null
    foreach ($pair in @(@('bootx64.efi', 'shimx64.efi'), @('grubx64.efi', 'grubx64.efi'), @('mmx64.efi', 'mmx64.efi'))) {
        $from = [IO.Path]::Combine($src, $pair[0])
        if (-not (Test-Path -LiteralPath $from)) { throw ('The Ubuntu installer has no ' + $pair[0] + '.') }
        Copy-Item -LiteralPath $from -Destination ([IO.Path]::Combine($dest, $pair[1])) -Force -ErrorAction Stop
    }
}

function Find-InstallerPartitions {
    # Chat Computer installer partitions on this PC (left from an install that didn't finish, or not yet cleaned
    # up by Chat Computer): FAT32 labelled CHATCOMP with a chatcomputer\installer-<id> mark.
    $found = @()
    foreach ($vol in @(Get-Volume -ErrorAction SilentlyContinue | Where-Object {
                ([string](Get-Prop $_ 'FileSystemLabel')) -eq $Script:InstallerLabel -and ([string](Get-Prop $_ 'FileSystem')) -match '^FAT' })) {
        $path = [string](Get-Prop $vol 'Path')
        if (-not $path) { continue }
        $marks = @()
        try { $marks = @(Get-ChildItem -LiteralPath ([IO.Path]::Combine($path, 'chatcomputer')) -Filter 'installer-*' -ErrorAction Stop) } catch { $marks = @() }
        if ($marks.Count -eq 0) { continue }
        $part = $vol | Get-Partition -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($part) { $found += $part }
    }
    return , $found
}
