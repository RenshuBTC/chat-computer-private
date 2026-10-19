# Chat Computer's own system (os/ in this folder) on this PC: signing its start-up file with a key made on this
# PC, putting it on the PC's start-up (EFI) partition next to Windows' own files, its entry in the power-on
# menu, and getting past Secure Boot. ASCII-only file (see Common.ps1).

$Script:OwnDir = 'chatcomputer-own'              # \EFI\chatcomputer-own on the EFI partition
$Script:OwnEntryName = 'Chat Computer (own system)'
$Script:OwnKeyName = 'Chat Computer Secure Boot key'  # the certificate's friendly name in Windows' certificate store
$Script:OwnFile = 'chatcomputer.efi'
$Script:OwnCert = 'ChatComputer.cer'

function Get-OwnSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $stream = [IO.File]::OpenRead($Path)
        try { return ([BitConverter]::ToString($sha.ComputeHash($stream)) -replace '-', '').ToLowerInvariant() }
        finally { $stream.Dispose() }
    } finally {
        $sha.Dispose()
    }
}

function Get-OwnRelease {
    # Chat Computer's own system as built from os/ (os\release\chatcomputer.efi), checked against its SHA-256.
    param([Parameter(Mandatory = $true)][string]$RepoRoot)
    $file = [IO.Path]::Combine($RepoRoot, 'os', 'release', $Script:OwnFile)
    $sumFile = $file + '.sha256'
    if (-not (Test-Path -LiteralPath $file) -or -not (Test-Path -LiteralPath $sumFile)) {
        throw ('Chat Computer''s own system is not in this folder (' + $file + '). Download the whole Chat Computer folder again.')
    }
    $expected = ((([IO.File]::ReadAllText($sumFile)).Trim() -split '\s+')[0]).ToLowerInvariant()
    $actual = Get-OwnSha256 $file
    Write-SetupLog ('Chat Computer''s own system: ' + $file + ', SHA-256 ' + $actual)
    if ($actual -ne $expected) {
        throw 'Chat Computer''s own system in this folder is damaged (its checksum is wrong). Download the Chat Computer folder again.'
    }
    return $file
}

function Test-PeSigned {
    # Whether a PE file (an .exe, or a UEFI program like Chat Computer's start-up file) carries a signature: its
    # security directory points at data inside the file.
    param([Parameter(Mandatory = $true)][string]$Path)
    $b = [IO.File]::ReadAllBytes($Path)
    if ($b.Length -lt 0x40 -or $b[0] -ne 0x4D -or $b[1] -ne 0x5A) { return $false }
    $pe = [BitConverter]::ToInt32($b, 0x3C)
    if ($pe -lt 0 -or $pe + 24 + 152 -gt $b.Length) { return $false }
    if ($b[$pe] -ne 0x50 -or $b[$pe + 1] -ne 0x45) { return $false }
    $opt = $pe + 24
    $dirs = $opt + 96                                                      # PE32
    if ([BitConverter]::ToUInt16($b, $opt) -eq 0x20B) { $dirs = $opt + 112 } # PE32+ (64-bit)
    $at = [int64][BitConverter]::ToUInt32($b, $dirs + 32)                   # directory 4: the signatures
    $size = [int64][BitConverter]::ToUInt32($b, $dirs + 36)
    return ($at -gt 0 -and $size -gt 0 -and $at + $size -le $b.Length)
}

function Test-BytesContain {
    # Whether $Bytes has $Part somewhere in it, byte for byte.
    param([AllowNull()][byte[]]$Bytes, [AllowNull()][byte[]]$Part)
    if (-not $Bytes -or -not $Part -or $Part.Length -gt $Bytes.Length) { return $false }
    # As text with a dash after every byte, a match can only start at a byte's start.
    return ([BitConverter]::ToString($Bytes)).Contains([BitConverter]::ToString($Part))
}

# ------------------------------------------------------------------ the key
function Get-OwnKey {
    # The certificate (with its private key) made for signing Chat Computer on this PC, or $null.
    $all = @(Get-ChildItem -Path 'Cert:\CurrentUser\My' -ErrorAction SilentlyContinue |
            Where-Object { $_.FriendlyName -eq $Script:OwnKeyName -and $_.HasPrivateKey })
    if ($all.Count -eq 0) { return $null }
    return ($all | Sort-Object NotBefore -Descending | Select-Object -First 1)
}

function New-OwnKey {
    # A key pair for signing Chat Computer, made by Windows on this PC. The private half can't be exported: it never
    # leaves this PC. UEFI firmware doesn't check dates, so the 30 years are only for Windows' sake.
    $cert = New-SelfSignedCertificate -Subject ('CN=Chat Computer Secure Boot key (' + $env:COMPUTERNAME + ')') `
        -FriendlyName $Script:OwnKeyName -Type CodeSigningCert -KeyAlgorithm RSA -KeyLength 2048 -HashAlgorithm SHA256 `
        -KeyExportPolicy NonExportable -CertStoreLocation 'Cert:\CurrentUser\My' -NotAfter (Get-Date).AddYears(30)
    Write-SetupLog ('Made the key for signing Chat Computer: ' + $cert.Thumbprint)
    return $cert
}

function New-OwnSignedFile {
    # Copies Chat Computer's start-up file to $Destination and signs it (Authenticode, SHA-256), as UEFI Secure Boot
    # checks it. Windows doesn't trust a key only this PC made, so the signing reports "UnknownError"; what
    # matters is that the signature is in the file.
    param(
        [Parameter(Mandatory = $true)][string]$Release,
        [Parameter(Mandatory = $true)][string]$Destination,
        [Parameter(Mandatory = $true)]$Cert
    )
    Copy-Item -LiteralPath $Release -Destination $Destination -Force
    $sig = Set-AuthenticodeSignature -FilePath $Destination -Certificate $Cert -HashAlgorithm SHA256 -IncludeChain All -ErrorAction Stop
    Write-SetupLog ('Signing: ' + [string]$sig.Status + ' ' + [string]$sig.StatusMessage)
    if (-not (Test-PeSigned $Destination)) {
        # Some Windows versions only sign files whose name they know; the file is the same either way.
        $tmp = $Destination + '.exe'
        Copy-Item -LiteralPath $Release -Destination $tmp -Force
        $sig = Set-AuthenticodeSignature -FilePath $tmp -Certificate $Cert -HashAlgorithm SHA256 -IncludeChain All -ErrorAction Stop
        Write-SetupLog ('Signing as .exe: ' + [string]$sig.Status + ' ' + [string]$sig.StatusMessage)
        Move-Item -LiteralPath $tmp -Destination $Destination -Force
    }
    if (-not (Test-PeSigned $Destination)) { throw 'Windows would not sign Chat Computer''s start-up file.' }
}

function Test-OwnKeyApproved {
    # $true when the firmware's list of allowed signers (db) holds the key's certificate, $false when it doesn't,
    # $null when the list can't be read.
    param([Parameter(Mandatory = $true)]$Cert)
    try {
        $db = Get-SecureBootUEFI -Name db -ErrorAction Stop
    } catch {
        Write-SetupLog ('Reading the firmware''s db: ' + $_.Exception.Message)
        return $null
    }
    return (Test-BytesContain ([byte[]](Get-Prop $db 'Bytes')) ([byte[]]$Cert.RawData))
}

# ------------------------------------------------------------------ the start-up partition and the power-on menu
function Get-OwnBootEntries {
    return , @(Get-FirmwareBootEntries | Where-Object { $_.Path -match ('(?i)^\\EFI\\' + $Script:OwnDir + '\\') -and $_.Id -ne '{fwbootmgr}' })
}

function Get-OwnBootEntry {
    # Chat Computer's own system's entry in the power-on menu, made if it isn't there. Returns its {id}.
    $entries = Get-OwnBootEntries
    if ($entries.Count -gt 0) { return $entries[0].Id }
    return (New-FirmwareBootEntry -Name $Script:OwnEntryName -Path ('\EFI\' + $Script:OwnDir + '\' + $Script:OwnFile))
}

function Install-OwnFiles {
    # The signed start-up file and the key's certificate, in \EFI\chatcomputer-own on the partition Windows starts from.
    param([Parameter(Mandatory = $true)][string]$Signed, [Parameter(Mandatory = $true)][string]$Certificate)
    $esp = Get-WindowsEsp
    if (-not $esp) { throw 'Windows could not find the partition this PC starts from.' }
    $m = Mount-EspPartition $esp
    try {
        $dir = [IO.Path]::Combine($m.Root, 'EFI', $Script:OwnDir)
        if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
        $file = Join-Path $dir $Script:OwnFile
        Copy-Item -LiteralPath $Signed -Destination $file -Force
        Copy-Item -LiteralPath $Certificate -Destination (Join-Path $dir $Script:OwnCert) -Force
        if ((Get-OwnSha256 $file) -ne (Get-OwnSha256 $Signed)) { throw 'The copy on the start-up partition is not the same as the original.' }
        Write-SetupLog ('Copied to ' + $dir)
    } finally {
        Dismount-EspPartition $m
    }
}

function Remove-OwnFiles {
    $esp = Get-WindowsEsp
    if (-not $esp) { return }
    $m = Mount-EspPartition $esp
    try {
        $dir = [IO.Path]::Combine($m.Root, 'EFI', $Script:OwnDir)
        if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
    } finally {
        Dismount-EspPartition $m
    }
}

# ------------------------------------------------------------------ this PC's parts
function Get-MonitorText {
    param($Codes)
    if (-not $Codes) { return '' }
    return ((@($Codes) | Where-Object { $_ -gt 0 } | ForEach-Object { [char][int]$_ }) -join '').Trim()
}

function Save-PcParts {
    # A list of this PC's parts (motherboard, firmware, graphics, monitors, and every device Windows sees, with
    # its hardware id), so Chat Computer's drivers can be written for exactly these chips. Read-only.
    param([Parameter(Mandatory = $true)][string]$Path, [AllowNull()]$SecureBoot)
    $lines = New-Object System.Collections.ArrayList
    [void]$lines.Add('This PC''s parts, as Windows sees them (' + (Get-Date -Format 's') + ').')
    try {
        $b = Get-CimInstance -ClassName Win32_BaseBoard -ErrorAction Stop | Select-Object -First 1
        [void]$lines.Add('Motherboard: ' + [string](Get-Prop $b 'Manufacturer') + ' ' + [string](Get-Prop $b 'Product') + ' ' + [string](Get-Prop $b 'Version'))
    } catch { Write-SetupLog ('Motherboard: ' + $_.Exception.Message) }
    try {
        $f = Get-CimInstance -ClassName Win32_BIOS -ErrorAction Stop | Select-Object -First 1
        [void]$lines.Add('Firmware: ' + [string](Get-Prop $f 'Manufacturer') + ' ' + [string](Get-Prop $f 'SMBIOSBIOSVersion') + ' (' + [string](Get-Prop $f 'ReleaseDate') + ')')
    } catch { Write-SetupLog ('Firmware: ' + $_.Exception.Message) }
    try {
        $c = Get-CimInstance -ClassName Win32_ComputerSystem -ErrorAction Stop | Select-Object -First 1
        [void]$lines.Add('PC: ' + [string](Get-Prop $c 'Manufacturer') + ' ' + [string](Get-Prop $c 'Model') + ', ' + [string](Get-Prop $c 'TotalPhysicalMemory') + ' bytes of memory')
    } catch { Write-SetupLog ('PC: ' + $_.Exception.Message) }
    try {
        foreach ($p in @(Get-CimInstance -ClassName Win32_Processor -ErrorAction Stop)) { [void]$lines.Add('Processor: ' + [string](Get-Prop $p 'Name')) }
    } catch { Write-SetupLog ('Processor: ' + $_.Exception.Message) }
    try {
        foreach ($v in @(Get-CimInstance -ClassName Win32_VideoController -ErrorAction Stop)) {
            [void]$lines.Add('Graphics: ' + [string](Get-Prop $v 'Name') + ', ' + [string](Get-Prop $v 'CurrentHorizontalResolution') + 'x' +
                [string](Get-Prop $v 'CurrentVerticalResolution') + ', ' + [string](Get-Prop $v 'PNPDeviceID'))
        }
    } catch { Write-SetupLog ('Graphics: ' + $_.Exception.Message) }
    try {
        foreach ($m in @(Get-CimInstance -Namespace 'root\wmi' -ClassName WmiMonitorID -ErrorAction Stop)) {
            [void]$lines.Add('Monitor: ' + (Get-MonitorText (Get-Prop $m 'ManufacturerName')) + ' ' + (Get-MonitorText (Get-Prop $m 'UserFriendlyName')))
        }
        foreach ($m in @(Get-CimInstance -Namespace 'root\wmi' -ClassName WmiMonitorBasicDisplayParams -ErrorAction Stop)) {
            [void]$lines.Add('Monitor size: ' + [string](Get-Prop $m 'MaxHorizontalImageSize') + ' x ' + [string](Get-Prop $m 'MaxVerticalImageSize') + ' cm')
        }
    } catch { Write-SetupLog ('Monitors: ' + $_.Exception.Message) }
    [void]$lines.Add('Secure Boot: ' + $(if ($SecureBoot -eq $true) { 'on' } elseif ($SecureBoot -eq $false) { 'off' } else { 'unknown' }))
    [void]$lines.Add('')
    [void]$lines.Add('Devices (class | name | hardware id):')
    try {
        foreach ($d in @(Get-PnpDevice -PresentOnly -ErrorAction Stop | Sort-Object Class, FriendlyName)) {
            [void]$lines.Add('  ' + [string](Get-Prop $d 'Class') + ' | ' + [string](Get-Prop $d 'FriendlyName') + ' | ' + [string](Get-Prop $d 'InstanceId'))
        }
    } catch { Write-SetupLog ('Devices: ' + $_.Exception.Message) }
    [IO.File]::WriteAllLines($Path, [string[]]$lines.ToArray())
    return $Path
}

# ------------------------------------------------------------------ what the person reads
function Write-ApproveKeySteps {
    param([string]$Maker)
    Write-Line 'In the PC''s setup screen (names differ a little between makers; these are the usual ones):'
    Write-Line '  1. If it opens in a simple view, switch to the advanced one (click "Advanced Mode"). Don''t press'
    Write-Line '     keys you don''t know there: on some boards a single key loads the default settings.'
    if ($Maker -match '(?i)gigabyte') {
        Write-Line '  2. Open the Boot tab, then "Secure Boot".'
        Write-Line '  3. Set "Secure Boot Mode" to Custom. Then open "Expert Key Management".'
    } else {
        Write-Line '  2. Find "Secure Boot". It is usually under Boot, or under Security.'
        Write-Line '  3. If there is a "Secure Boot Mode" setting, set it to Custom. Then open "Key Management"'
        Write-Line '     (on some boards "Expert Key Management").'
    }
    Write-Line '  4. Choose "Authorized Signatures (db)", then "Append".'
    Write-Line '     Only Append: "Update", "Set", "Delete" or "Clear" would replace or remove Microsoft''s keys, which'
    Write-Line '     Windows needs.'
    Write-Line '  5. When it asks where to load from, choose a file (if it asks whether to load the factory default,'
    Write-Line '     answer No). Pick the drive it lists, then the folders EFI and chatcomputer-own, then ChatComputer.cer.'
    Write-Line '  6. If it asks what kind of file it is, choose "Public Key Certificate". Confirm with Yes.'
    Write-Line '  7. Choose Save & Exit (Save Changes and Reset). The PC restarts into Chat Computer''s own system.'
    Write-Line 'Leave "Secure Boot Mode" on Custom afterwards: switching back to Standard puts only the factory keys back.'
    Write-Line 'If a step isn''t there, leave without saving: Windows starts as before, and you can choose to turn'
    Write-Line 'Secure Boot off instead. If Windows starts instead of Chat Computer after step 7, double-click'
    Write-Line '"Chat Computer Own System" again: it sees the key is approved and restarts straight into Chat Computer.'
    Write-Line 'If anything goes wrong, "Restore Factory Keys" (or "Install Default Secure Boot Keys") in the same'
    Write-Line 'place puts Microsoft''s keys back, and Windows starts as before.'
}

function Write-SecureBootOffSteps {
    param([string]$Maker)
    Write-Line 'In the PC''s setup screen (names differ a little between makers; these are the usual ones):'
    Write-Line '  1. If it opens in a simple view, switch to the advanced one (click "Advanced Mode"). Don''t press'
    Write-Line '     keys you don''t know there: on some boards a single key loads the default settings.'
    if ($Maker -match '(?i)gigabyte') {
        Write-Line '  2. Open the Boot tab, then "Secure Boot".'
    } else {
        Write-Line '  2. Find "Secure Boot". It is usually under Boot, or under Security.'
    }
    Write-Line '  3. Set Secure Boot to Disabled. (On ASUS boards: set "OS Type" to "Other OS".)'
    Write-Line '  4. Choose Save & Exit (Save Changes and Reset). The PC restarts into Chat Computer''s own system.'
    Write-Line 'To turn Secure Boot on again later, change the same setting back. If Windows starts instead of'
    Write-Line 'Chat Computer, double-click "Chat Computer Own System" again.'
}

# ------------------------------------------------------------------ the two things it does
function Confirm-Own {
    param([Parameter(Mandatory = $true)][string]$Question, [bool]$Default = $true, [switch]$Unattended)
    if ($Unattended) { Write-SetupLog ('Q: ' + $Question + ' -> (unattended) ' + $Default); return $Default }
    return (Read-YesNo $Question $Default)
}

function Get-BitLockerOn {
    try {
        $v = Get-BitLockerVolume -MountPoint 'C:' -ErrorAction Stop
        return ([string](Get-Prop $v 'ProtectionStatus') -eq 'On')
    } catch {
        Write-SetupLog ('BitLocker: ' + $_.Exception.Message)
        return $false
    }
}

function Start-OwnSystem {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$WorkDir,
        [switch]$Unattended,
        [switch]$NoRestart,
        [string]$Out
    )
    Write-Step 'Checking this PC'
    if ((Get-FirmwareType) -eq 'Legacy') {
        throw 'This PC starts in the old BIOS mode, and Chat Computer needs UEFI. Nothing was changed.'
    }
    Write-Good 'Starts in UEFI mode.'
    $release = Get-OwnRelease -RepoRoot $RepoRoot
    $sb = Get-SecureBootState
    if ($sb -eq $true) { Write-Line 'Secure Boot is on.' } elseif ($sb -eq $false) { Write-Line 'Secure Boot is off.' }
    $parts = Save-PcParts -Path (Join-Path $WorkDir 'pc.txt') -SecureBoot $sb
    Write-Line ('A list of this PC''s parts is in ' + $parts + ', so Chat Computer''s drivers can be written for them.')

    Write-Step 'Signing Chat Computer with this PC''s own key'
    $cert = Get-OwnKey
    if ($cert) {
        Write-Line 'Using the key made on this PC for signing Chat Computer.'
    } else {
        $cert = New-OwnKey
        Write-Good 'Made a key for signing Chat Computer. Windows keeps it on this PC and won''t let it be copied off.'
    }
    $signed = Join-Path $WorkDir $Script:OwnFile
    $cer = Join-Path $WorkDir $Script:OwnCert
    New-OwnSignedFile -Release $release -Destination $signed -Cert $cert
    [IO.File]::WriteAllBytes($cer, [byte[]]$cert.RawData)
    Write-Good 'Signed.'
    if ($Out) {
        if (-not (Test-Path -LiteralPath $Out)) { New-Item -ItemType Directory -Path $Out -Force | Out-Null }
        Copy-Item -LiteralPath $signed -Destination $Out -Force
        Copy-Item -LiteralPath $cer -Destination $Out -Force
    }

    $approved = $null
    if ($sb -eq $true) { $approved = Test-OwnKeyApproved -Cert $cert }
    $setupScreen = ($sb -eq $true -and $approved -ne $true)
    $bitlocker = $setupScreen -and (Get-BitLockerOn)
    if ($approved -eq $true) { Write-Good 'This PC already trusts Chat Computer''s key.' }

    Write-Step 'What changes'
    Write-Line '  * Chat Computer''s start-up file (signed with this PC''s key) and the key''s certificate go in \EFI\chatcomputer-own'
    Write-Line '    on the partition this PC starts from (about 250 KB). Windows'' own files there stay as they are.'
    Write-Line ('  * "' + $Script:OwnEntryName + '" goes at the end of the power-on menu and starts once, at the next restart.')
    Write-Line '    The restart after that starts Windows, as now.'
    if ($bitlocker) {
        Write-Line '  * BitLocker pauses for two restarts, so the Secure Boot change doesn''t make it ask for its recovery key.'
    }
    if (-not (Confirm-Own 'Go ahead?' $true -Unattended:$Unattended)) {
        Write-Line 'Nothing was changed.'
        return
    }
    Install-OwnFiles -Signed $signed -Certificate $cer
    $id = Get-OwnBootEntry
    Invoke-Bcdedit @('/set', '{fwbootmgr}', 'bootsequence', $id)
    Write-Good ('Chat Computer''s own system is on the start-up partition, and "' + $Script:OwnEntryName + '" starts at the next restart, once.')

    if (-not $setupScreen) {
        if ($NoRestart) { return }
        if (Confirm-Own 'Restart into Chat Computer''s own system now?' $true -Unattended:$Unattended) {
            Write-Line 'Restarting in 5 seconds. To come back to Windows, hold the power button until the PC turns off, then turn it on.'
            & shutdown.exe /r /t 5
        } else {
            Write-Line 'It starts at the next restart.'
        }
        return
    }

    Write-Step 'Secure Boot'
    Write-Line 'This PC starts only software signed with a key it trusts, and it doesn''t know Chat Computer''s key yet.'
    $pick = 0
    if (-not $Unattended) {
        $pick = Read-Choice 'How should it start Chat Computer?' @(
            'Approve Chat Computer''s key, once, in the PC''s setup screen (Secure Boot stays on)',
            'Turn Secure Boot off in the PC''s setup screen (one setting; some games'' anti-cheat needs it on)') 0
    }
    if ($bitlocker) {
        try {
            Suspend-BitLocker -MountPoint 'C:' -RebootCount 2 -ErrorAction Stop | Out-Null
            Write-Good 'BitLocker is paused for two restarts.'
        } catch {
            Write-SetupLog ('Suspend-BitLocker: ' + $_.Exception.Message)
            Write-Warn 'BitLocker could not be paused, so Windows may ask for its recovery key once after the change.'
        }
    }
    $maker = ''
    try { $maker = [string](Get-Prop (Get-CimInstance -ClassName Win32_BaseBoard -ErrorAction Stop | Select-Object -First 1) 'Manufacturer') } catch { $null = $_ }
    Write-Line ''
    if ($pick -eq 0) { Write-ApproveKeySteps -Maker $maker } else { Write-SecureBootOffSteps -Maker $maker }
    Write-Line ''
    Write-Line 'Take a photo of these steps: the setup screen comes next, and this window won''t be there.'
    if ($NoRestart) { return }
    if (Confirm-Own 'Restart into the PC''s setup screen now?' $true -Unattended:$Unattended) {
        $out = & shutdown.exe /r /fw /t 5 2>&1
        if ($LASTEXITCODE -ne 0) {
            Write-SetupLog ('shutdown /fw: ' + (($out | Out-String).Trim()))
            Write-Line 'Windows can''t open the setup screen on this PC by itself. Restart, and press the setup key as the PC'
            Write-Line 'starts (often Del or F2; the PC''s first screen usually says which).'
        }
    } else {
        Write-Line 'Whenever you''re ready, run this again, or restart and press the setup key as the PC starts (often Del or F2).'
    }
}

function Remove-OwnSystem {
    param([switch]$Unattended)
    Write-Step 'Remove Chat Computer''s own system from this PC'
    $entries = Get-OwnBootEntries
    $cert = Get-OwnKey
    Write-Line ('This takes "' + $Script:OwnEntryName + '" out of the power-on menu and deletes its files from the start-up')
    Write-Line 'partition. Windows stays as it is.'
    if ($cert) { Write-Line 'It also deletes the key made on this PC for signing Chat Computer.' }
    if (-not (Confirm-Own 'Go ahead?' $true -Unattended:$Unattended)) {
        Write-Line 'Nothing was changed.'
        return
    }
    foreach ($e in $entries) { Invoke-Bcdedit @('/delete', $e.Id) }
    Remove-OwnFiles
    if ($cert) {
        Remove-Item -LiteralPath ('Cert:\CurrentUser\My\' + $cert.Thumbprint) -DeleteKey -ErrorAction Stop
        Write-SetupLog ('Deleted the key ' + $cert.Thumbprint)
    }
    Write-Good 'Done: Chat Computer''s own system is off this PC.'
    if ($cert) {
        Write-Line 'If you approved its key in the PC''s setup screen, it is still listed there, but nothing can be signed'
        Write-Line 'with it any more. To take it off that list too, use "Restore Factory Keys" there.'
    }
}
