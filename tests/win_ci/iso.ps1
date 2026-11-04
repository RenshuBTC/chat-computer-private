# Setup's copy of the Ubuntu installer, from a real disc image, on a real Windows machine (run.ps1 runs it).
# Files on a mounted disc image are marked read-only, and robocopy copies that mark. On a real PC, setup then
# couldn't patch grub.cfg on the copy ("Access to the path 'S:\boot\grub\grub.cfg' is denied"). This makes a
# small ISO with Windows' own image maker (IMAPI2), runs setup's Copy-IsoToInstaller on it, and patches
# grub.cfg the way Write-InstallerPayload does. Mounting needs administrator rights, which CI machines have.
# Writes what it did into -Out. Exits non-zero when anything failed. ASCII-only file.
param([Parameter(Mandatory = $true)][string]$Out)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Path $Out -Force | Out-Null
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
foreach ($lib in @('Common', 'Installer')) {
    . (Join-Path $repo ('windows\lib\' + $lib + '.ps1'))
}
$log = New-Object System.Collections.ArrayList
function Note([string]$Text) { [void]$log.Add($Text); Write-Host $Text }

Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
public static class IsoImageFile {
    // Saves the image IMAPI2 made (a COM stream) to a file.
    public static void Save(object image, string path) {
        IStream stream = (IStream)image;
        byte[] buffer = new byte[1 << 16];
        IntPtr read = Marshal.AllocHGlobal(sizeof(int));
        try {
            using (FileStream file = File.Create(path)) {
                while (true) {
                    stream.Read(buffer, buffer.Length, read);
                    int n = Marshal.ReadInt32(read);
                    if (n <= 0) break;
                    file.Write(buffer, 0, n);
                }
            }
        } finally {
            Marshal.FreeHGlobal(read);
        }
    }
}
'@

$work = Join-Path ([IO.Path]::GetTempPath()) ('chatcomputer-iso-' + [guid]::NewGuid().ToString('N'))
$src = Join-Path $work 'src'
$iso = Join-Path $work 'test.iso'
$dest = Join-Path $work 'installer'
$failed = $false
try {
    # What Copy-IsoToInstaller and Write-InstallerPayload look for: .disk\info and boot\grub\grub.cfg.
    New-Item -ItemType Directory -Path (Join-Path $src '.disk') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $src 'boot\grub') -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $src '.disk\info'), 'Ubuntu-Server 24.04.5 LTS "Noble Numbat" - Release amd64')
    $grubText = "set timeout=30`n`nmenuentry `"Try or Install Ubuntu Server`" {`n`tset gfxpayload=keep`n" +
                "`tlinux`t/casper/vmlinuz  ---`n`tinitrd`t/casper/initrd`n}`n" +
                "menuentry `"Ubuntu Server with the HWE kernel`" {`n`tset gfxpayload=keep`n" +
                "`tlinux`t/casper/hwe-vmlinuz  ---`n`tinitrd`t/casper/hwe-initrd`n}`n"
    [IO.File]::WriteAllText((Join-Path $src 'boot\grub\grub.cfg'), $grubText)

    $fsi = New-Object -ComObject IMAPI2FS.MsftFileSystemImage
    $fsi.FileSystemsToCreate = 3   # ISO 9660 and Joliet, like a downloaded installer image
    $fsi.VolumeName = 'CHATCOMP_TEST'
    $fsi.Root.AddTree($src, $false)
    [IsoImageFile]::Save($fsi.CreateResultImage().ImageStream, $iso)
    Note ('Made a test image: ' + (Get-Item -LiteralPath $iso).Length + ' bytes')

    # The premise: the mounted image's files are read-only.
    $img = Mount-DiskImage -ImagePath $iso -PassThru
    try {
        $letter = [string]($img | Get-Volume).DriveLetter
        $onDisc = (Get-Item -LiteralPath ($letter + ':\boot\grub\grub.cfg')).IsReadOnly
        Note ('On the mounted image, grub.cfg is read-only: ' + $onDisc)
    } finally {
        Dismount-DiskImage -ImagePath $iso | Out-Null
    }

    # Setup's own copy, then its patch.
    Copy-IsoToInstaller -IsoPath $iso -Root $dest
    $grub = Join-Path $dest 'boot\grub\grub.cfg'
    $readOnly = (Get-Item -LiteralPath $grub).IsReadOnly
    Note ('After setup''s copy, grub.cfg is read-only: ' + $readOnly)
    if ($readOnly) { $failed = $true; Note 'FAIL: the copy kept the read-only mark' }
    $patched = Update-GrubConfig -Text ([IO.File]::ReadAllText($grub)) -InstallId '0123456789abcdef01234567'
    Write-TextFileLf -Path $grub -Text $patched
    if ([IO.File]::ReadAllText($grub) -ne ($patched -replace "`r`n", "`n")) { $failed = $true; Note 'FAIL: grub.cfg was not patched' }
    else { Note 'grub.cfg patched on the copy, as setup does.' }
} catch {
    $failed = $true
    Note ('FAIL: ' + (Get-ErrorText $_))
} finally {
    if (Test-Path -LiteralPath $work) {
        Get-ChildItem -LiteralPath $work -Recurse -File -Force | ForEach-Object { $_.IsReadOnly = $false }
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }
}
$log | Out-File -FilePath (Join-Path $Out 'iso.txt') -Encoding utf8
if ($failed) { exit 1 }
exit 0
