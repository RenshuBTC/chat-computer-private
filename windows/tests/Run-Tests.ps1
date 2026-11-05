# Unit tests for Chat Computer Setup's plain functions. Runs on any PowerShell (Windows 5.1, or pwsh on Linux).
#   pwsh -NoProfile -File windows/tests/Run-Tests.ps1 [-OutDir DIR]
# With -OutDir, also writes sample generated files there for the Python tests to validate.
param([string]$OutDir)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0
$Here = $PSScriptRoot
$Win = Split-Path -Parent $Here
foreach ($lib in @('Common', 'Timezones', 'Autoinstall', 'Disks', 'Network', 'Ubuntu', 'Installer', 'Checks', 'Startup', 'OwnOs')) {
    . (Join-Path $Win ('lib/' + $lib + '.ps1'))
}

$Script:Failures = 0
$Script:Count = 0
function Assert-Equal($Expected, $Actual, [string]$What) {
    $Script:Count++
    if ($Expected -cne $Actual) {
        $Script:Failures++
        Write-Host ('FAIL ' + $What + "`n  expected: " + $Expected + "`n  actual:   " + $Actual) -ForegroundColor Red
    }
}
function Assert-True($Condition, [string]$What) {
    $Script:Count++
    if (-not $Condition) { $Script:Failures++; Write-Host ('FAIL ' + $What) -ForegroundColor Red }
}
function Assert-Throws([scriptblock]$Block, [string]$What) {
    $Script:Count++
    try { & $Block; $Script:Failures++; Write-Host ('FAIL (no error) ' + $What) -ForegroundColor Red } catch { $null = $_ }
}

# ------------------------------------------------------------------ user names
Assert-Equal 'sammy' (ConvertTo-LinuxUserName 'sammy') 'plain name'
Assert-Equal 'samlee' (ConvertTo-LinuxUserName 'Sam Lee') 'spaces and capitals'
Assert-Equal 'jose' (ConvertTo-LinuxUserName ('Jos' + [char]0x00E9)) 'accent removed'
Assert-Equal 'u123abc' (ConvertTo-LinuxUserName '123abc') 'leading digit'
Assert-Equal 'root2' (ConvertTo-LinuxUserName 'root') 'reserved'
Assert-Equal 'admin2' (ConvertTo-LinuxUserName 'Admin') 'reserved after lowercasing'
Assert-Equal 'me' (ConvertTo-LinuxUserName '') 'empty'
Assert-Equal 'me' (ConvertTo-LinuxUserName ([string][char]0x674E + [char]0x660E)) 'only non-latin letters'
Assert-True ((ConvertTo-LinuxUserName ('a' * 80)).Length -le 32) 'long names are cut'
Assert-True (Test-LinuxUserName 'sammy') 'valid name accepted'
Assert-True (-not (Test-LinuxUserName 'Sammy')) 'capitals rejected'
Assert-True (-not (Test-LinuxUserName 'sudo')) 'reserved rejected'

# ------------------------------------------------------------------ passwords
# Test vector from the SHA-crypt specification (Drepper).
Assert-Equal '$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/O817G3uBnIFNjnQJuesI68u4OTLiBFdcbYEdFCoEOfaS35inz1' `
    (Get-Sha512Crypt -Password 'Hello world!' -Salt 'saltstring') 'SHA-512 crypt test vector'
$h = Get-Sha512Crypt -Password 'abc'
Assert-True ($h -match '^\$6\$[./0-9A-Za-z]{16}\$[./0-9A-Za-z]{86}$') 'random salt hash format'
$r = New-RecoveryPassword
Assert-True ($r -match '^[a-z2-9]{4}(-[a-z2-9]{4}){3}$') 'recovery password format'
Assert-True ((New-InstallId) -match '^[0-9a-f]{24}$') 'install id format'

# ------------------------------------------------------------------ YAML strings
Assert-Equal '"plain"' (ConvertTo-YamlString 'plain') 'yaml plain'
Assert-Equal '"a\"b\\c"' (ConvertTo-YamlString 'a"b\c') 'yaml quote and backslash'
Assert-Equal '"x\ny"' (ConvertTo-YamlString ("x`ny")) 'yaml newline'
Assert-Equal '""' (ConvertTo-YamlString $null) 'yaml null'
Assert-Equal ('"caf' + [char]0x00E9 + '"') (ConvertTo-YamlString ('caf' + [char]0x00E9)) 'yaml unicode kept'
# A character past the first 65,536 (an emoji; some Chinese names have one) stays whole: YAML reads an escaped
# pair of halves as two broken characters.
$emoji = [char]::ConvertFromUtf32(0x1F3E0)
Assert-Equal ('"' + $emoji + ' Home"') (ConvertTo-YamlString ($emoji + ' Home')) 'yaml: a character past the first 65,536 stays whole'
Assert-Equal '"a\ufffdb"' (ConvertTo-YamlString ('a' + [char]0xD83C + 'b')) 'yaml: half a pair on its own'
Assert-Equal '"a\ufffd"' (ConvertTo-YamlString ('a' + [char]0xDFE0)) 'yaml: the second half on its own'
Assert-Equal '"x\u2028y"' (ConvertTo-YamlString ('x' + [char]0x2028 + 'y')) 'yaml: a line separator stays on the line'

# ------------------------------------------------------------------ where the installer partition goes
$MiB = [int64]1048576
$GiB = [int64]1073741824
# Windows' usual layout once C: has given 86 GB: EFI, reserved, C:, (the free space), recovery.
$parts = @(
    [pscustomobject]@{ Offset = 277 * $MiB; Size = 864 * $GiB },
    [pscustomobject]@{ Offset = 1 * $MiB; Size = 260 * $MiB },
    [pscustomobject]@{ Offset = 277 * $MiB + 950 * $GiB; Size = 1 * $GiB },
    [pscustomobject]@{ Offset = 261 * $MiB; Size = 16 * $MiB })
$ext = Get-FreeExtents -Partitions $parts -DiskSize (277 * $MiB + 951 * $GiB + 2 * $MiB)
Assert-Equal 2 $ext.Count 'free space: the space after C:, and one MiB at the end'
Assert-Equal (277 * $MiB + 864 * $GiB) $ext[0].Offset 'free space: the largest first, right after C:'
Assert-Equal (86 * $GiB) $ext[0].Size 'free space: its size'
$place = Get-InstallerPlacement -Extent $ext[0]
Assert-Equal (277 * $MiB + 944 * $GiB) $place.Offset 'installer: at the end of the free space'
Assert-Equal (6 * $GiB) $place.Size 'installer: 6 GB'
Assert-Equal (80 * $GiB) $place.ChatComputerBytes 'installer: Chat Computer gets the rest, right before it'
# Partitions that don't start or end on a whole MiB: the free space is rounded inward.
$ext = Get-FreeExtents -Partitions @([pscustomobject]@{ Offset = 1 * $MiB; Size = 100 * $MiB + 512 }) -DiskSize (10 * $GiB + 4096)
Assert-Equal (102 * $MiB) $ext[0].Offset 'free space starts on a whole MiB'
Assert-Equal (10239 * $MiB) ($ext[0].Offset + $ext[0].Size) 'free space ends on a whole MiB, clear of the last MiB'
$place = Get-InstallerPlacement -Extent $ext[0]
Assert-Equal 0 ($place.Offset % $MiB) 'installer starts on a whole MiB'
Assert-Equal (10239 * $MiB) ($place.Offset + $place.Size) 'installer ends where the free space ends'
# A disk that was just erased, and a full one.
$ext = Get-FreeExtents -Partitions @() -DiskSize (2000 * $GiB)
Assert-Equal 1 $ext.Count 'erased disk: one free area'
Assert-Equal $MiB $ext[0].Offset 'erased disk: after the first MiB'
Assert-Equal (2000 * $GiB - 2 * $MiB) $ext[0].Size 'erased disk: up to the last MiB'
Assert-Equal 0 (Get-FreeExtents -Partitions @([pscustomobject]@{ Offset = $MiB; Size = 10 * $GiB - 2 * $MiB }) -DiskSize (10 * $GiB)).Count 'full disk: no free space'

# ------------------------------------------------------------------ Wi-Fi
$wpa2 = @'
<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
  <name>Home "5G"</name>
  <SSIDConfig><SSID><hex>486F6D65</hex><name>Home "5G"</name></SSID></SSIDConfig>
  <connectionType>ESS</connectionType>
  <MSM><security>
    <authEncryption><authentication>WPA2PSK</authentication><encryption>AES</encryption><useOneX>false</useOneX></authEncryption>
    <sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>p@ss w0rd"\</keyMaterial></sharedKey>
  </security></MSM>
</WLANProfile>
'@
$w = ConvertFrom-WlanProfileXml $wpa2
Assert-Equal 'Home "5G"' $w.Ssid 'wifi ssid'
Assert-Equal 'p@ss w0rd"\' $w.Password 'wifi password'
Assert-True $w.Usable 'wpa2 usable'
$ent = $wpa2.Replace('<authentication>WPA2PSK</authentication>', '<authentication>WPA2</authentication>').Replace('<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>p@ss w0rd"\</keyMaterial></sharedKey>', '')
Assert-True (-not (ConvertFrom-WlanProfileXml $ent).Usable) 'enterprise not usable'
$prot = $wpa2.Replace('<protected>false</protected>', '<protected>true</protected>')
Assert-True (-not (ConvertFrom-WlanProfileXml $prot).Usable) 'encrypted key not usable'
$open = $wpa2.Replace('<authentication>WPA2PSK</authentication>', '<authentication>open</authentication>').Replace('<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>p@ss w0rd"\</keyMaterial></sharedKey>', '')
Assert-True (ConvertFrom-WlanProfileXml $open).Usable 'open network usable'
$net = New-NetworkYaml -WifiSsid 'Home "5G"' -WifiPassword 'secret' -WifiAuth 'WPA3SAE'
Assert-True ($net.Contains('key-management: sae')) 'wpa3 uses sae'
Assert-True (-not (New-NetworkYaml).Contains('wifis')) 'no wifi section without a network'

# ------------------------------------------------------------------ secrets in the log
$key = 'sk-ant-api03-' + ('Ab9_-' * 20)
Assert-Equal 'key sk-ant-(hidden) end' (Protect-SetupText ('key ' + $key + ' end')) 'a key is never logged'

# ------------------------------------------------------------------ folders, and short (8.3) folder names
# Remove-Folder deletes a folder with everything in it, quietly; a missing folder is fine.
$rf = Join-Path ([IO.Path]::GetTempPath()) ('chatcomputer-rf-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $rf 'a/b') -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $rf 'a/b/x.json'), '{}')
$out = @(Remove-Folder $rf)
Assert-True (-not (Test-Path -LiteralPath $rf) -and $out.Count -eq 0) 'remove folder: gone, and nothing written to the output'
Remove-Folder $rf
Assert-True (-not (Test-Path -LiteralPath $rf)) 'remove folder: a missing folder is fine'
# Sign-in folders an earlier setup's stopped run left behind are deleted; other folders stay.
$tr = Join-Path ([IO.Path]::GetTempPath()) ('chatcomputer-tr-' + [guid]::NewGuid().ToString('N'))
foreach ($d in 'chatcomputer-signin-1/config', 'chatcomputer-signin-2', 'keep-me') { New-Item -ItemType Directory -Path (Join-Path $tr $d) -Force | Out-Null }
[IO.File]::WriteAllText((Join-Path $tr 'chatcomputer-signin-1/config/.credentials.json'), '{}')
Remove-OldSignInFolders -Root $tr
Assert-True (-not (Test-Path -LiteralPath (Join-Path $tr 'chatcomputer-signin-1')) -and -not (Test-Path -LiteralPath (Join-Path $tr 'chatcomputer-signin-2')) -and
             (Test-Path -LiteralPath (Join-Path $tr 'keep-me'))) 'old sign-in folders: deleted, others kept'
Remove-Folder $tr
Assert-Equal 'C:\Users\Sam Lee\x' (Get-LongPath 'C:\Users\Sam Lee\x') 'long path: no short names, unchanged'
Assert-True ([string]::IsNullOrEmpty((Get-LongPath $null))) 'long path: nothing in, nothing out'
if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
    # The PC that needed this: user folder "Sam Lee", so %TEMP% is C:\Users\SAMLEE~1\AppData\Local\Temp, and setup
    # runs in a folder on C:. A folder with a space gets a short name like that (where the drive makes them).
    $base = Get-LongPath ([Environment]::GetFolderPath('LocalApplicationData'))    # on C:, like the user folder
    $long = Join-Path $base ('Chat Computer short ' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Path $long -Force | Out-Null
    $short = $null
    try { $short = (New-Object -ComObject Scripting.FileSystemObject).GetFolder($long).ShortPath } catch { $short = $null }
    if ($short -and $short.Contains('~')) {
        Assert-Equal $long.ToLowerInvariant() (Get-LongPath $short).ToLowerInvariant() 'long path: short names spelled out'
        $savedEnvTemp = $env:TEMP
        Push-Location (Split-Path -Qualifier $long)
        try {
            # What Windows PowerShell 5.1 does with Remove-Item -Recurse there (logged, not judged).
            New-Item -ItemType Directory -Path (Join-Path $long 'probe') -Force | Out-Null
            [IO.File]::WriteAllText((Join-Path $long 'probe/x.json'), '{}')
            try { Remove-Item -LiteralPath (Join-Path $short 'probe') -Recurse -Force -ErrorAction SilentlyContinue; Write-Host 'short path: Remove-Item -Recurse worked here' }
            catch { Write-Host ('short path: Remove-Item -Recurse failed here: ' + $_.Exception.Message) }
            # Setup spells %TEMP% out at the start.
            $savedTmp = $env:TMP
            $env:TMP = $short
            Use-LongTempPath
            Assert-Equal $long.ToLowerInvariant() $env:TEMP.ToLowerInvariant() 'short path: %TEMP% spelled out'
            Assert-Equal $long.ToLowerInvariant() $env:TMP.ToLowerInvariant() 'short path: %TMP% spelled out'
            $env:TMP = $savedTmp
        } finally {
            Pop-Location
            $env:TEMP = $savedEnvTemp
        }
    } else {
        Write-Host 'short path: this drive makes no short names; skipped'
    }
    Remove-Folder $long
}

# ------------------------------------------------------------------ what stops C: from shrinking
# Event 259 as real Windows writes it (fields from GitHub's Windows Server 2025, and the PC that needed this).
$ev = [pscustomobject]@{
    Properties = @([pscustomobject]@{ Value = 'Windows (C:)' }, [pscustomobject]@{ Value = '\\?\Volume{6f146b24-4f0e-4ba2-97d1-29ea630e5753}' },
                   [pscustomobject]@{ Value = '\$Mft::$BITMAP' }, [pscustomobject]@{ Value = '0x24fa0b2' }, [pscustomobject]@{ Value = '-S--S' })
    Message    = 'A volume shrink analysis was initiated on volume Windows (C:). ...'
}
Assert-Equal '\$Mft::$BITMAP' (ConvertFrom-ShrinkEvent $ev) 'event 259: the file, from its fields'
$evText = [pscustomobject]@{ Message = "Diagnostic details:`r`n - The last unmovable file appears to be: \`$Extend\`$UsnJrnl:`$J:`$DATA`r`n - The last cluster" }
Assert-Equal '\$Extend\$UsnJrnl:$J:$DATA' (ConvertFrom-ShrinkEvent $evText) 'event 259: the file, from the English message'
$svi = '\System Volume Information\{9db92f71-c6a1-11f1-ae98-00155d606220}{3808876b-c176-4e48-b7ae-04046e6cc752}::$DATA'
Assert-Equal 'restorepoints' (Get-ShrinkBlockerKind $svi) 'kind: restore points'
Assert-Equal 'journal' (Get-ShrinkBlockerKind '\$Extend\$UsnJrnl:$J:$DATA') 'kind: change journal'
Assert-Equal 'hibernation' (Get-ShrinkBlockerKind '\hiberfil.sys') 'kind: hibernation'
Assert-Equal 'pagefile' (Get-ShrinkBlockerKind '\pagefile.sys') 'kind: page file'
Assert-Equal 'pagefile' (Get-ShrinkBlockerKind '\swapfile.sys') 'kind: swap file'
Assert-Equal 'ntfs' (Get-ShrinkBlockerKind '\$Mft::$BITMAP') 'kind: file system records'
Assert-Equal 'other' (Get-ShrinkBlockerKind '\Windows\System32\x.dll') 'kind: other'
Assert-Equal 'other' (Get-ShrinkBlockerKind '\System Volume Information\tracking.log') 'kind: other System Volume Information file'
Assert-Equal 'unknown' (Get-ShrinkBlockerKind '') 'kind: no event'
Assert-True ((Get-RoomAdvice -Blocker ([pscustomobject]@{ Kind = 'space'; File = $null }) -Made 0 -NeedBytes 46GB) -match 'free some up') 'advice: free space'
Assert-True ((Get-RoomAdvice -Blocker ([pscustomobject]@{ Kind = 'pagefile'; File = '\pagefile.sys' }) -Made 10GB -NeedBytes 46GB) -match 'Virtual memory') 'advice: page file'
Assert-True ((Get-RoomAdvice -Blocker ([pscustomobject]@{ Kind = 'ntfs'; File = '\$Mft::$BITMAP' }) -Made 10GB -NeedBytes 46GB) -match 'separate disk') 'advice: file system records'
Assert-True ((Get-RoomAdvice -Blocker $null -Made 10GB -NeedBytes 46GB) -match '10 GB of room') 'advice: says what was made'

# Only real answers reach the log, so a stray piece of a key typed at a yes/no question never does.
$Script:SetupLogFile = [IO.Path]::GetTempFileName()
$Script:Answers = New-Object System.Collections.Queue
function Read-Host { param([Parameter(Position = 0)]$Prompt, [switch]$AsSecureString) return $Script:Answers.Dequeue() }
$Script:Answers.Enqueue('Ab9_-Ab9_-Ab9_-tail-of-a-code')
$Script:Answers.Enqueue('y')
Assert-True (Read-YesNo 'Go ahead?') 'yes after a stray line'
$Script:Answers.Enqueue('Ab9_-tail-of-a-code')
$Script:Answers.Enqueue('2')
Assert-Equal 1 (Read-Choice 'What would you like to do?' @('One', 'Two')) 'choice after a stray line'
$Script:Answers.Enqueue('Ab9_-tail-of-a-code')
Assert-True (-not (Read-ConfirmWord 'Type REMOVE' 'REMOVE')) 'confirm word not matched'
$logText = [IO.File]::ReadAllText($Script:SetupLogFile)
Assert-True ($logText -notmatch 'tail-of-a-code') 'stray text is not logged'
Assert-True ($logText -match 'ahead\? -> y' -and $logText -match 'do\? -> 2') 'real answers are logged'
Remove-Item Function:\Read-Host
Remove-Item -LiteralPath $Script:SetupLogFile -Force
$Script:SetupLogFile = $null
Assert-True (-not (Protect-SetupText ('x ' + $key + ' y')).Contains('Ab9_')) 'key hidden in logs'

# ------------------------------------------------------------------ time zones and settings
Assert-Equal 'Asia/Singapore' (Get-IanaTimeZone 'Singapore Standard Time') 'singapore'
Assert-Equal 'Europe/London' (Get-IanaTimeZone 'GMT Standard Time') 'london'
# The current names: Ubuntu 24.04 has no Asia/Calcutta or Europe/Kiev (they're in tzdata-legacy, not installed).
Assert-Equal 'Asia/Kolkata' (Get-IanaTimeZone 'India Standard Time') 'india'
Assert-Equal 'Europe/Kyiv' (Get-IanaTimeZone 'FLE Standard Time') 'kyiv'
Assert-Equal 'Etc/UTC' (Get-IanaTimeZone 'Nowhere Standard Time') 'unknown zone'
$settings = New-SettingsJson -UserName 'Sam "R" Lee' -WindowsUser 'sammy' -TimeZone 'Asia/Singapore'
$parsed = $settings | ConvertFrom-Json
Assert-Equal 'Sam "R" Lee' $parsed.user_name 'settings json user'
Assert-Equal 'sammy' $parsed.windows_user 'settings json windows user'

# ------------------------------------------------------------------ Ubuntu downloads
$sums = ConvertFrom-Sha256Sums ([IO.File]::ReadAllText((Join-Path $Here 'data/SHA256SUMS')))
$iso = Select-ServerIso $sums
Assert-Equal 'ubuntu-24.04.5-live-server-amd64.iso' $iso.Name 'newest server iso'
Assert-Equal '97f3d7ffb032c3eb3b23d2c8be9cc76e60c2c1f2c0146ba5ba9fe01cafae0fd8' $iso.Sha256 'iso hash'
$bad = @{}
foreach ($k in $sums.Keys) { $bad[$k] = $sums[$k] }
$bad['ubuntu-24.04.5-live-server-amd64.iso'] = ('0' * 64)
Assert-Throws { Select-ServerIso $bad } 'changed fingerprint is refused'
Assert-Equal 6 (Get-PointRelease 'ubuntu-24.04.6-live-server-amd64.iso') 'future point release recognised'
Assert-Equal -1 (Get-PointRelease 'ubuntu-24.04.5-desktop-amd64.iso') 'desktop image ignored'
# Up to 24.04.3 the installer made the account only at the first start, too late for Chat Computer's last install
# step: such an installer is refused, from the download site and with -IsoPath.
$old = @{ 'ubuntu-24.04.3-live-server-amd64.iso' = $sums['ubuntu-24.04.3-live-server-amd64.iso'] }
Assert-Throws { Select-ServerIso $old } 'an installer older than 24.04.4 is refused'
$why = ''
try {
    $null = Get-UbuntuIso -WorkDir ([IO.Path]::GetTempPath()) -IsoPath (Join-Path ([IO.Path]::GetTempPath()) 'ubuntu-24.04.3-live-server-amd64.iso')
} catch {
    $why = $_.Exception.Message
}
Assert-True ($why -match '24\.04\.4 or newer') 'a 24.04.3 installer given with -IsoPath is refused, saying why'

# ------------------------------------------------------------------ boot menu
$grubIn = [IO.File]::ReadAllText((Join-Path $Here 'data/ubuntu-24.04.5-grub.cfg'))
$grubOut = Update-GrubConfig -Text $grubIn -InstallId '0123456789abcdef01234567'
Assert-True ($grubOut -match '(?m)^set timeout=5$') 'timeout 5'
Assert-Equal 2 ([regex]::Matches($grubOut, ' autoinstall ')).Count 'two install entries'
$installEntry = [regex]::Match($grubOut, '(?s)menuentry "(Install[^"]+)" \{(.*?)\n\}')
Assert-Equal 'Install Chat Computer' $installEntry.Groups[1].Value 'first install entry name'
Assert-True ($installEntry.Groups[2].Value -match 'hwe-vmlinuz autoinstall subiquity-storage-version=2\s+---') 'first install entry: newer kernel, autoinstall and storage v2 before ---'
Assert-True ($grubOut.Contains('menuentry "Install Chat Computer (older kernel)"')) 'second entry'
Assert-True ($grubOut.Contains("fwsetup")) 'firmware entry kept'
Assert-True (-not $grubOut.Contains('Try or Install Ubuntu Server')) 'old entry replaced'
# If the installer starts again after installing, it starts the installed Chat Computer instead of installing again.
Assert-True ($grubOut.Contains('if search --no-floppy --file --set=chatcomputer_esp /EFI/chatcomputer/installed-0123456789abcdef01234567; then')) 'checks for its own install'
$startAt = $grubOut.IndexOf('menuentry "Start Chat Computer')
Assert-True ($startAt -ge 0 -and $startAt -lt $grubOut.IndexOf('menuentry "Install Chat Computer"')) 'start entry comes first'
Assert-True ($grubOut.Contains("`tconfigfile /EFI/ubuntu/grub.cfg`n}")) 'reads the installed boot menu'
Assert-True (-not $grubOut.Contains('chainloader /EFI/ubuntu')) 'no chainloading (fails under Secure Boot)'
Assert-True ($grubOut -match '(?s)set default=chatcomputer-start\nelse\nmenuentry "Install Chat Computer" \{.*\n\}\nfi\n') 'install entries only when not installed'
Assert-Throws { Update-GrubConfig -Text 'set timeout=30' -InstallId '0123456789abcdef' } 'unexpected menu refused'
Assert-Throws { Update-GrubConfig -Text $grubIn -InstallId 'x; reboot' } 'bad install id refused'

# ------------------------------------------------------------------ the power-on menu (bcdedit)
$bcdEnglish = @'
Firmware Boot Manager
---------------------
identifier              {fwbootmgr}
displayorder            {9f62d3e2-5c1a-11ef-a1b2-806e6f6e6963}
                        {bootmgr}
                        {9f62d3e3-5c1a-11ef-a1b2-806e6f6e6963}
timeout                 0

Windows Boot Manager
--------------------
identifier              {bootmgr}
device                  partition=\Device\HarddiskVolume1
path                    \EFI\Microsoft\Boot\bootmgfw.efi
description             Windows Boot Manager
locale                  en-US

Firmware Application (101fffff)
-------------------------------
identifier              {9f62d3e2-5c1a-11ef-a1b2-806e6f6e6963}
device                  partition=\Device\HarddiskVolume1
path                    \EFI\ubuntu\shimx64.efi
description             ubuntu

Firmware Application (101fffff)
-------------------------------
identifier              {9f62d3e3-5c1a-11ef-a1b2-806e6f6e6963}
description             UEFI: PXE IPv4 Intel(R) Ethernet
'@
$entries = ConvertFrom-BcdFirmware $bcdEnglish
Assert-Equal 4 $entries.Count 'bcdedit: four blocks'
$ub = @($entries | Where-Object { $_.Path -match '(?i)^\\EFI\\ubuntu\\' })
Assert-Equal 1 $ub.Count 'bcdedit: one Chat Computer entry'
Assert-Equal '{9f62d3e2-5c1a-11ef-a1b2-806e6f6e6963}' $ub[0].Id 'bcdedit: its identifier'
Assert-Equal '\EFI\Microsoft\Boot\bootmgfw.efi' ($entries | Where-Object { $_.Id -eq '{bootmgr}' }).Path 'bcdedit: Windows Boot Manager path'
# German Windows: the labels are translated, the values are not.
$bcdGerman = ($bcdEnglish -replace 'identifier  ', 'Bezeichner  ' -replace 'description ', 'Beschreibung' -replace 'path    ', 'Pfad    ' -replace 'Firmware Application', 'Firmwareanwendung') -replace "`n", "`r`n"
$de = ConvertFrom-BcdFirmware $bcdGerman
$deUb = @($de | Where-Object { $_.Path -match '(?i)^\\EFI\\ubuntu\\' })
Assert-Equal '{9f62d3e2-5c1a-11ef-a1b2-806e6f6e6963}' $deUb[0].Id 'bcdedit: German output'
Assert-Equal 0 (ConvertFrom-BcdFirmware '').Count 'bcdedit: empty output'

$markerDir = Join-Path ([IO.Path]::GetTempPath()) ('chatcomputer-marker-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $markerDir | Out-Null
[IO.File]::WriteAllText((Join-Path $markerDir 'new'), "id=0123456789abcdef01234567`nroot=3F2A9C1E-5B7D-4E8F-9A0B-1C2D3E4F5A6B`n")
[IO.File]::WriteAllText((Join-Path $markerDir 'old'), "0123456789abcdef01234567`n")
$mk = Read-InstallMarker (Join-Path $markerDir 'new')
Assert-Equal '0123456789abcdef01234567' $mk.Id 'marker: id'
Assert-Equal '3f2a9c1e-5b7d-4e8f-9a0b-1c2d3e4f5a6b' $mk.Root 'marker: partition'
$mk = Read-InstallMarker (Join-Path $markerDir 'old')
Assert-Equal '0123456789abcdef01234567' $mk.Id 'older marker: id'
Assert-Equal '' $mk.Root 'older marker: no partition'
Remove-Item -LiteralPath $markerDir -Recurse -Force
Assert-Equal (Format-Guid '{3F2A9C1E-5B7D-4E8F-9A0B-1C2D3E4F5A6B}') '3f2a9c1e-5b7d-4e8f-9a0b-1c2d3e4f5a6b' 'partition GUIDs compare without braces or case'

# ------------------------------------------------------------------ the answers file
$template = [IO.File]::ReadAllText((Join-Path $Win 'autoinstall.template.yaml'))
$hash = Get-Sha512Crypt -Password 'kq7m-3vtz-p9xa-hc4r' -Salt 'abcdefghijklmnop'
$yaml = New-AutoinstallYaml -Template $template -Account 'sammy' -RealName ('Sam Lee "' + [char]0x00E9 + '"') -PasswordHash $hash `
    -TimeZone 'Asia/Singapore' -InstallId '0123456789abcdef01234567' `
    -WifiSsid 'Home "5G"' -WifiPassword 'p@ss w0rd"\' -WifiAuth 'WPA2PSK'
Assert-True (-not ($yaml -match '\{\{')) 'all fields filled'
Assert-True ($yaml.Contains('mode: use_gap')) 'free space only'
Assert-True ($yaml -match '(?m)^\s+match:\s*\n\s+install-media: true\s*$') 'the disk the installer is on'
Assert-True (-not $yaml.Contains('reformat_disk')) 'never reformats'
Assert-Throws { New-AutoinstallYaml -Template $template -Account 'Bad Name' -PasswordHash $hash -TimeZone 'Etc/UTC' -InstallId '0123456789abcdef' } 'bad user name refused'
Assert-Throws { New-AutoinstallYaml -Template 'nothing' -Account 'sammy' -PasswordHash $hash -TimeZone 'Etc/UTC' -InstallId '0123456789abcdef' } 'template without fields refused'
$wiredOnly = New-AutoinstallYaml -Template $template -Account 'sammy' -PasswordHash $hash -TimeZone 'Etc/UTC' -InstallId '0123456789abcdef'
# A name as a work PC may have it: the account is made with useradd, which refuses a colon.
$work = New-AutoinstallYaml -Template $template -Account 'sammy' -RealName ("Lee, Sam (IT: Ops)`t= x") -PasswordHash $hash -TimeZone 'Etc/UTC' -InstallId '0123456789abcdef'
Assert-True ($work.Contains('realname: "Lee Sam (IT Ops) x"')) 'name without : , = or tabs, as the installer''s form takes it'
Assert-True ((New-AutoinstallYaml -Template $template -Account 'sammy' -RealName ('a' * 200) -PasswordHash $hash -TimeZone 'Etc/UTC' -InstallId '0123456789abcdef').Contains(('realname: "' + ('a' * 160) + '"'))) 'name at most 160 characters'
# A name and a Wi-Fi network with characters past the first 65,536 (checked by the Python tests).
$wide = [char]::ConvertFromUtf32(0x20BB7) + [char]::ConvertFromUtf32(0x1F600)
$wideYaml = New-AutoinstallYaml -Template $template -Account 'sammy' -RealName ('Sam ' + $wide) -PasswordHash $hash `
    -TimeZone 'Asia/Singapore' -InstallId '0123456789abcdef01234567' -WifiSsid ([char]::ConvertFromUtf32(0x1F3E0) + ' Home') -WifiPassword 'pw'
$wideSettings = New-SettingsJson -UserName ('Sam ' + $wide) -WindowsUser 'sammy' -TimeZone 'Asia/Singapore'

# ------------------------------------------------------------------ the installer's power-on entry
# bcdedit stand-in: a menu that /copy adds to, and calls that can be made to fail.
$Script:Bcd = ''
$Script:BcdCalls = New-Object System.Collections.ArrayList
$Script:BcdFail = ''
$Script:BcdNewId = '{11111111-2222-4333-8444-555555555555}'
$Script:BcdCopyText = ''
function bcdedit.exe {
    $a = @($args)
    $global:LASTEXITCODE = 0
    if ($a[0] -eq '/enum') { return ($Script:Bcd -split "`n") }
    [void]$Script:BcdCalls.Add($a -join ' ')
    if ($Script:BcdFail -and ($a -join ' ') -like $Script:BcdFail) { $global:LASTEXITCODE = 1; return 'The parameter is incorrect.' }
    if ($a[0] -eq '/copy') {
        $Script:Bcd += "`r`n`r`nFirmware Application (101fffff)`r`n-------------------------------`r`nidentifier              " + $Script:BcdNewId +
                       "`r`npath                    \EFI\Microsoft\Boot\bootmgfw.efi"
        return $Script:BcdCopyText
    }
}
function New-BcdBlock([string]$Id, [string]$Path) {
    return ("`n`nFirmware Application (101fffff)`n-------------------------------`nidentifier              " + $Id + "`npath                    " + $Path)
}
$Script:Bcd = $bcdEnglish
$Script:BcdCopyText = 'The entry was successfully copied to ' + $Script:BcdNewId + '.'
$id = New-SetupBootEntry
Assert-Equal $Script:BcdNewId $id 'setup entry: the new one'
Assert-Equal '/copy {bootmgr} /d Chat Computer setup' $Script:BcdCalls[0] 'setup entry: a copy of Windows Boot Manager''s'
Assert-Equal ('/set ' + $Script:BcdNewId + ' path \EFI\chatcomputer-setup\shimx64.efi') $Script:BcdCalls[1] 'setup entry: starts the installer''s boot loader'
Assert-Equal ('/set {fwbootmgr} displayorder ' + $Script:BcdNewId + ' /addlast') $Script:BcdCalls[2] 'setup entry: last in the menu'
# In another language, and with a message that names no entry: found by comparing the menu before and after.
$Script:Bcd = $bcdGerman
$Script:BcdCopyText = 'Der Eintrag wurde erfolgreich kopiert.'
$Script:BcdCalls.Clear()
Assert-Equal $Script:BcdNewId (New-SetupBootEntry) 'setup entry: found in German too'
# If it can't be pointed at the installer, it is deleted again rather than left as a second Windows entry.
$Script:Bcd = $bcdEnglish
$Script:BcdCalls.Clear()
$Script:BcdFail = '/set * path *'
Assert-Throws { New-SetupBootEntry } 'setup entry: a failed change is an error'
Assert-Equal ('/delete ' + $Script:BcdNewId) $Script:BcdCalls[$Script:BcdCalls.Count - 1] 'setup entry: the half-made entry is deleted'
$Script:BcdFail = '/copy*'
$Script:Bcd = $bcdEnglish
$Script:BcdCalls.Clear()
Assert-Throws { New-SetupBootEntry } 'setup entry: a PC that won''t take one is an error'
Assert-Equal 1 $Script:BcdCalls.Count 'setup entry: nothing else is tried'
$Script:BcdFail = ''
# Removing: every entry that starts \EFI\chatcomputer-setup\, and nothing when there is none.
$Script:Bcd = $bcdEnglish
$Script:BcdCalls.Clear()
Remove-SetupBootEntries
Assert-Equal 0 $Script:BcdCalls.Count 'no setup entry: nothing deleted'
$Script:Bcd = $bcdEnglish + (New-BcdBlock '{aaaaaaaa-0000-4000-8000-000000000001}' '\EFI\chatcomputer-setup\shimx64.efi') +
    (New-BcdBlock '{aaaaaaaa-0000-4000-8000-000000000002}' '\EFI\CHATCOMPUTER-SETUP\shimx64.efi')
Remove-SetupBootEntries
Assert-Equal '/delete {aaaaaaaa-0000-4000-8000-000000000001}|/delete {aaaaaaaa-0000-4000-8000-000000000002}' ($Script:BcdCalls -join '|') 'two setup entries: both deleted, one at a time'
Assert-Equal 1 (Get-ChatComputerBootEntries).Count 'Chat Computer''s own entry is not a setup entry'
Remove-Item Function:\bcdedit.exe

# ------------------------------------------------------------------ the installer partition
$Script:PartCalls = New-Object System.Collections.ArrayList
$Script:FormatFails = $false
$Script:PartAt = [int64]0
function New-Partition {
    param([int]$DiskNumber, [int64]$Offset, [int64]$Size)
    [void]$Script:PartCalls.Add('New-Partition ' + $DiskNumber)
    $Script:PartAt = $Offset
    return [pscustomobject]@{ DiskNumber = $DiskNumber; PartitionNumber = 5; Offset = $Offset; Size = $Size }
}
function Set-Partition { [void]$Script:PartCalls.Add('Set-Partition ' + ($args -join ' ')) }
function Format-Volume {
    param($Partition, [string]$FileSystem, [string]$NewFileSystemLabel)
    [void]$Script:PartCalls.Add('Format-Volume ' + $FileSystem + ' ' + $NewFileSystemLabel)
    if ($Script:FormatFails) { throw 'The format failed.' }
}
function Get-FreeDriveLetter { return 'T' }
function Add-PartitionAccessPath { param([int]$DiskNumber, [int]$PartitionNumber, [string]$AccessPath) [void]$Script:PartCalls.Add('Add-PartitionAccessPath ' + $AccessPath) }
function Start-Sleep { }
function Get-Partition {
    param([int]$DiskNumber, [int]$PartitionNumber)
    return [pscustomobject]@{ DiskNumber = $DiskNumber; PartitionNumber = $PartitionNumber; Offset = $Script:PartAt; Size = 6 * $GiB }
}
function Remove-Partition { param([int]$DiskNumber, [int]$PartitionNumber) [void]$Script:PartCalls.Add('Remove-Partition ' + $DiskNumber + ' ' + $PartitionNumber) }
$inst = New-InstallerPartition -DiskNumber 0 -Offset (966933 * $MiB) -Size (6 * $GiB)
Assert-Equal 'T:\' $inst.Root 'installer partition: opened at a drive letter'
Assert-Equal 5 $inst.PartitionNumber 'installer partition: its number'
Assert-True ($Script:PartCalls -contains 'Format-Volume FAT32 CHATCOMP') 'installer partition: FAT32, labelled CHATCOMP'
Assert-True (@($Script:PartCalls | Where-Object { $_ -like 'Set-Partition*NoDefaultDriveLetter*' }).Count -eq 1) 'installer partition: kept out of File Explorer'
$Script:PartCalls.Clear()
$Script:FormatFails = $true
Assert-Throws { New-InstallerPartition -DiskNumber 0 -Offset (966933 * $MiB) -Size (6 * $GiB) } 'installer partition: a failed format is an error'
Assert-True ($Script:PartCalls -contains 'Remove-Partition 0 5') 'installer partition: deleted again when it can''t be prepared'
$Script:FormatFails = $false
# Deleting it later: only if the partition at that number is still the one setup made.
$Script:PartCalls.Clear()
$Script:PartAt = 12345 * $MiB
Assert-Throws { Remove-InstallerPartition $inst } 'installer partition: another partition at that number is refused'
Assert-Equal 0 @($Script:PartCalls | Where-Object { $_ -like 'Remove-Partition*' }).Count 'installer partition: and left alone'
$Script:PartAt = 966933 * $MiB
Remove-InstallerPartition $inst
Assert-True ($Script:PartCalls -contains 'Remove-Partition 0 5') 'installer partition: deleted when it is the one setup made'
foreach ($f in @('New-Partition', 'Set-Partition', 'Format-Volume', 'Get-FreeDriveLetter', 'Add-PartitionAccessPath', 'Start-Sleep', 'Get-Partition', 'Remove-Partition')) {
    Remove-Item ('Function:\' + $f)
}
. (Join-Path $Win 'lib/Startup.ps1')    # Get-FreeDriveLetter again

# Its mark, and the boot loader it puts on the EFI partition.
$tmpInst = Join-Path ([IO.Path]::GetTempPath()) ('chatcomputer-installer-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $tmpInst 'inst/EFI/boot'), (Join-Path $tmpInst 'esp/EFI/Microsoft') -Force | Out-Null
foreach ($f in @('bootx64.efi', 'grubx64.efi', 'mmx64.efi')) { [IO.File]::WriteAllText((Join-Path $tmpInst ('inst/EFI/boot/' + $f)), $f) }
Write-InstallerMark -Root (Join-Path $tmpInst 'inst') -InstallId '0123456789abcdef01234567'
Assert-Equal "id=0123456789abcdef01234567`n" ([IO.File]::ReadAllText((Join-Path $tmpInst 'inst/chatcomputer/installer-0123456789abcdef01234567'))) 'installer mark'
Assert-Throws { Write-InstallerMark -Root (Join-Path $tmpInst 'inst') -InstallId '../../x' } 'installer mark: bad id refused'
Install-SetupLoader -InstallerRoot (Join-Path $tmpInst 'inst') -EspRoot (Join-Path $tmpInst 'esp')
Assert-Equal 'bootx64.efi' ([IO.File]::ReadAllText((Join-Path $tmpInst 'esp/EFI/chatcomputer-setup/shimx64.efi'))) 'boot loader: shim, under the name its GRUB expects'
Assert-Equal 'grubx64.efi' ([IO.File]::ReadAllText((Join-Path $tmpInst 'esp/EFI/chatcomputer-setup/grubx64.efi'))) 'boot loader: GRUB next to it'
Assert-True (Test-Path -LiteralPath (Join-Path $tmpInst 'esp/EFI/chatcomputer-setup/mmx64.efi')) 'boot loader: MokManager too'
Remove-Item -LiteralPath (Join-Path $tmpInst 'inst/EFI/boot/grubx64.efi')
Assert-Throws { Install-SetupLoader -InstallerRoot (Join-Path $tmpInst 'inst') -EspRoot (Join-Path $tmpInst 'esp') } 'boot loader: missing files are an error'
Remove-Item -LiteralPath $tmpInst -Recurse -Force

# ------------------------------------------------------------------ Chat Computer's own BitLocker key
$tmpState = Join-Path ([IO.Path]::GetTempPath()) ('chatcomputer-state-' + [guid]::NewGuid().ToString('N'))
$env:CHATCOMPUTER_STATE_DIR = $tmpState
$Script:BlCalls = New-Object System.Collections.ArrayList
$Script:BlType = 3
$Script:BlPassword = '123456-234567-345678-456789-567890-678901-789012-890123'
function Get-CimInstance { return [pscustomobject]@{ DriveLetter = 'C:' } }
function Invoke-CimMethod {
    param($InputObject, [string]$MethodName, [hashtable]$Arguments)
    [void]$Script:BlCalls.Add($MethodName)
    switch ($MethodName) {
        'ProtectKeyWithNumericalPassword' { return [pscustomobject]@{ ReturnValue = 0; VolumeKeyProtectorID = '{2b000000-0000-4000-8000-0000000000c1}' } }
        'GetKeyProtectorNumericalPassword' { return [pscustomobject]@{ ReturnValue = 0; NumericalPassword = $Script:BlPassword } }
        'GetKeyProtectorType' { return [pscustomobject]@{ ReturnValue = 0; KeyProtectorType = $Script:BlType } }
        'DeleteKeyProtector' { return [pscustomobject]@{ ReturnValue = 0 } }
    }
}
$k = New-ChatComputerBitLockerKey
Assert-Equal '{2b000000-0000-4000-8000-0000000000c1}' $k.Id 'BitLocker key: its id'
Assert-Equal $Script:BlPassword $k.Password 'BitLocker key: its recovery password'
Assert-Equal "{2b000000-0000-4000-8000-0000000000c1}`n" ([IO.File]::ReadAllText((Join-Path $tmpState 'bitlocker-key-id'))) 'BitLocker key: id recorded'
# Only ever deletes a recovery password: if the recorded id names another kind of key, nothing is deleted.
$Script:BlType = 1
[void]$Script:BlCalls.Clear()
Assert-Throws { Remove-ChatComputerBitLockerKey } 'BitLocker key: refuses to delete a key that is not a recovery password'
Assert-True ($Script:BlCalls -notcontains 'DeleteKeyProtector') 'BitLocker key: nothing deleted then'
$Script:BlType = 3
Assert-True (Remove-ChatComputerBitLockerKey) 'BitLocker key: deleted'
Assert-True (-not (Test-Path -LiteralPath (Join-Path $tmpState 'bitlocker-key-id'))) 'BitLocker key: record gone'
Assert-True (-not (Remove-ChatComputerBitLockerKey)) 'BitLocker key: nothing to delete without a record'
# A key BitLocker shows in an unexpected form is not kept: it's deleted again and setup says so.
$Script:BlPassword = 'not-a-key'
[void]$Script:BlCalls.Clear()
Assert-Throws { New-ChatComputerBitLockerKey } 'BitLocker key: an unexpected form is an error'
Assert-True ($Script:BlCalls -contains 'DeleteKeyProtector') 'BitLocker key: the half-made key is deleted'
Remove-Item -LiteralPath $tmpState -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item Function:\Get-CimInstance, Function:\Invoke-CimMethod
$env:CHATCOMPUTER_STATE_DIR = $null

# ------------------------------------------------------------------ Chat Computer's own system
$repo = Split-Path -Parent $Win
$ownRelease = [IO.Path]::Combine($repo, 'os', 'release', 'chatcomputer.efi')
Assert-Equal $ownRelease (Get-OwnRelease -RepoRoot $repo) 'own system: the release file, with the right checksum'
Assert-True (-not (Test-PeSigned $ownRelease)) 'own system: the release file is not signed yet'
$ownTmp = [IO.Path]::Combine([IO.Path]::GetTempPath(), 'chatcomputer-own-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $ownTmp | Out-Null
try {
    # A copy whose signature directory points at bytes at its end, as signing leaves it.
    $b = [IO.File]::ReadAllBytes($ownRelease)
    $dir = [BitConverter]::ToInt32($b, 0x3C) + 24 + 112 + 32
    $withSig = New-Object byte[] ($b.Length + 16)
    [Array]::Copy($b, $withSig, $b.Length)
    [Array]::Copy([BitConverter]::GetBytes([uint32]$b.Length), 0, $withSig, $dir, 4)
    [Array]::Copy([BitConverter]::GetBytes([uint32]16), 0, $withSig, $dir + 4, 4)
    $fake = Join-Path $ownTmp 'signed.efi'
    [IO.File]::WriteAllBytes($fake, $withSig)
    Assert-True (Test-PeSigned $fake) 'own system: a file with a signature counts as signed'
    [Array]::Copy([BitConverter]::GetBytes([uint32]1000), 0, $withSig, $dir + 4, 4)
    [IO.File]::WriteAllBytes($fake, $withSig)
    Assert-True (-not (Test-PeSigned $fake)) 'own system: a signature reaching past the end of the file does not'
    [IO.File]::WriteAllBytes($fake, [byte[]](1, 2, 3))
    Assert-True (-not (Test-PeSigned $fake)) 'own system: a file that is not a program is not signed'
    # A damaged or missing file is refused.
    $fakeRepo = Join-Path $ownTmp 'repo'
    $fakeRelease = [IO.Path]::Combine($fakeRepo, 'os', 'release')
    New-Item -ItemType Directory -Path $fakeRelease -Force | Out-Null
    Copy-Item -LiteralPath $ownRelease -Destination (Join-Path $fakeRelease 'chatcomputer.efi')
    [IO.File]::WriteAllText((Join-Path $fakeRelease 'chatcomputer.efi.sha256'), ('0' * 64) + '  chatcomputer.efi')
    Assert-Throws { Get-OwnRelease -RepoRoot $fakeRepo } 'own system: a file that does not match its checksum is refused'
    Assert-Throws { Get-OwnRelease -RepoRoot (Join-Path $ownTmp 'nothing') } 'own system: a missing file is refused'
} finally {
    Remove-Item -LiteralPath $ownTmp -Recurse -Force
}
# Finding the key's certificate in the firmware's list of allowed signers.
Assert-True (Test-BytesContain ([byte[]](0, 1, 2, 3, 4)) ([byte[]](2, 3))) 'own system: bytes found'
Assert-True (-not (Test-BytesContain ([byte[]](0, 1, 2, 3, 4)) ([byte[]](3, 2)))) 'own system: the same bytes in another order are not'
Assert-True (-not (Test-BytesContain ([byte[]](0x12, 0x34)) ([byte[]](0x23)))) 'own system: half of one byte and half of the next are not a byte'
Assert-True (-not (Test-BytesContain $null ([byte[]](1)))) 'own system: nothing to look in'
Assert-True (-not (Test-BytesContain ([byte[]](1)) ([byte[]](1, 2)))) 'own system: longer than what it is looked for in'
# Its power-on entry (with the bcdedit stand-in from above).
$Script:BcdCalls = New-Object System.Collections.ArrayList
$Script:BcdFail = ''
function bcdedit.exe {
    $a = @($args)
    $global:LASTEXITCODE = 0
    if ($a[0] -eq '/enum') { return ($Script:Bcd -split "`n") }
    [void]$Script:BcdCalls.Add($a -join ' ')
    if ($a[0] -eq '/copy') {
        $Script:Bcd += (New-BcdBlock $Script:BcdNewId '\EFI\Microsoft\Boot\bootmgfw.efi')
        return ('The entry was successfully copied to ' + $Script:BcdNewId + '.')
    }
}
$Script:Bcd = $bcdEnglish
Assert-Equal $Script:BcdNewId (Get-OwnBootEntry) 'own entry: made when it is not there'
Assert-Equal '/copy {bootmgr} /d Chat Computer (own system)' $Script:BcdCalls[0] 'own entry: a copy of Windows Boot Manager''s'
Assert-Equal ('/set ' + $Script:BcdNewId + ' path \EFI\chatcomputer-own\chatcomputer.efi') $Script:BcdCalls[1] 'own entry: starts Chat Computer''s own system'
Assert-Equal ('/set {fwbootmgr} displayorder ' + $Script:BcdNewId + ' /addlast') $Script:BcdCalls[2] 'own entry: last in the menu'
$Script:Bcd = $bcdEnglish + (New-BcdBlock '{bbbbbbbb-0000-4000-8000-000000000001}' '\EFI\ChatComputer-Own\chatcomputer.efi')
$Script:BcdCalls.Clear()
Assert-Equal '{bbbbbbbb-0000-4000-8000-000000000001}' (Get-OwnBootEntry) 'own entry: the one already there'
Assert-Equal 0 $Script:BcdCalls.Count 'own entry: nothing added when it is there'
Assert-Equal 0 (Get-SetupBootEntries).Count 'own entry: not the installer''s entry'
Assert-Equal 1 (Get-ChatComputerBootEntries).Count 'own entry: not Linux Chat Computer''s entry'
Remove-Item Function:\bcdedit.exe
# The key it signs with: one the PC's Secure Boot already trusts (ClaudeOS's, made before the name changed, too: a PC
# that approved it starts Chat Computer with nothing to approve), else Chat Computer's own, else a new one.
$kNew = [pscustomobject]@{ FriendlyName = 'Chat Computer Secure Boot key'; Thumbprint = 'NEW' }
$kOld = [pscustomobject]@{ FriendlyName = 'ClaudeOS Secure Boot key'; Thumbprint = 'OLD' }
Assert-Equal 'OLD' (Select-OwnKey -Keys @($kNew, $kOld) -Trusted { param($k) $k.Thumbprint -eq 'OLD' }).Thumbprint 'own key: the one Secure Boot trusts, ClaudeOS''s too'
Assert-Equal 'NEW' (Select-OwnKey -Keys @($kOld, $kNew) -Trusted { param($k) $false }).Thumbprint 'own key: else Chat Computer''s own'
Assert-Equal 'NEW' (Select-OwnKey -Keys @($kOld, $kNew)).Thumbprint 'own key: Secure Boot off: Chat Computer''s own'
Assert-Equal $null (Select-OwnKey -Keys @($kOld)) 'own key: only an untrusted ClaudeOS key: a new one is made'
Assert-Equal $null (Select-OwnKey -Keys @()) 'own key: none yet'
Assert-True (Wait-OwnCountdown 10 -Unattended) 'own restart: no countdown to wait through, unattended'
# Its model: the one Chat Computer's app got ready, picked as the kernel picks (os/kernel/src/assistant.rs).
function New-FakeModel([string]$Name, [int64]$Length) { [pscustomobject]@{ Name = $Name; Length = $Length; FullName = ('C:\m\' + $Name) } }
Assert-Equal 'Qwen3.5 4B' (Get-OwnModelLabel 'qwen3.5-4b.ccm') 'own model: its name'
Assert-Equal 'Qwen3.6 35B-A3B' (Get-OwnModelLabel 'qwen3.6-35b-a3b.ccm') 'own model: a mixture of experts'' name'
$picked = Select-OwnModel @((New-FakeModel 'qwen3.5-9b.ccm' 5600000000), (New-FakeModel 'qwen3.5-4b.ccm' 2700000000), (New-FakeModel 'qwen3.5-0.8b.ccm' 600000000))
Assert-Equal 'C:\m\qwen3.5-4b.ccm' $picked.Path 'own model: the biggest quick enough on one processor core'
Assert-Equal 'Qwen3.5 4B' $picked.Label 'own model: its name, picked'
Assert-Equal ([int64]2700000000) $picked.Bytes 'own model: its size'
Assert-Equal 'C:\m\qwen3.5-9b.ccm' (Select-OwnModel @((New-FakeModel 'qwen3.8-27b.ccm' 16000000000), (New-FakeModel 'qwen3.5-9b.ccm' 5600000000))).Path 'own model: else the smallest'
Assert-Equal $null (Select-OwnModel @()) 'own model: none'
Assert-Equal $null (Find-OwnModel -Dir (Join-Path ([IO.Path]::GetTempPath()) ('no-models-' + [guid]::NewGuid()))) 'own model: none where there''s no folder'
$modelDir = New-Item -ItemType Directory -Path (Join-Path ([IO.Path]::GetTempPath()) ('models-' + [guid]::NewGuid())) -Force
[IO.File]::WriteAllBytes((Join-Path $modelDir.FullName 'qwen3.5-0.8b.ccm'), (New-Object byte[] 3))
[IO.File]::WriteAllBytes((Join-Path $modelDir.FullName 'qwen3.5-0.8b.refusal'), (New-Object byte[] 9))
[IO.File]::WriteAllBytes((Join-Path $modelDir.FullName 'qwen3.5-4b.ccm'), (New-Object byte[] 5))
Assert-Equal 'Qwen3.5 4B' (Find-OwnModel -Dir $modelDir.FullName).Label 'own model: found in the app''s folder, the biggest'
Remove-Item -LiteralPath $modelDir.FullName -Recurse -Force
# Its settings: the clock, a model named by hand, as the kernel reads them (os/kernel/src/settings.rs).
$ownText = New-OwnSettingsText -Model '' -ClockOffsetMinutes 480
Assert-True ($ownText -match '(?m)^clock-offset-minutes = 480\r$') 'own settings: the clock offset'
Assert-True ($ownText -notmatch '(?m)^model') 'own settings: no model named unless one was'
Assert-True ($ownText -notmatch 'claude') 'own settings: no Claude API key'
Assert-Equal '' (Get-OwnSettingsModel $ownText) 'own settings: no model read back'
$named = 'D:\Models\my model.ccm'
$withModel = New-OwnSettingsText -Model $named -ClockOffsetMinutes -300
Assert-True ($withModel -match ('(?m)^model = ' + [regex]::Escape($named) + '\r$')) 'own settings: a model named, on a line of its own'
Assert-True ($withModel -match '(?m)^clock-offset-minutes = -300\r$') 'own settings: a clock behind UTC'
Assert-Equal $named (Get-OwnSettingsModel $withModel) 'own settings: the model read back, spaces and all'
Assert-Equal '' (Get-OwnSettingsModel "claude-api-key = sk-ant-api03-old`r`nclock-offset-minutes = 0`r`n") 'own settings: an older setup''s key isn''t a model'
Assert-Equal $ownText ([Text.Encoding]::ASCII.GetString([Text.Encoding]::ASCII.GetBytes($ownText))) 'own settings: plain ASCII'
Assert-True ((Get-ClockOffsetMinutes) -is [int]) 'own settings: the clock offset is a number of minutes'
Assert-True ($ownText -notmatch 'memory-id') 'own settings: no memory id without a memory file'
Assert-Equal $null (Get-OwnSettingsMemoryId $ownText) 'own settings: no memory id read back'
$withMemory = New-OwnSettingsText -Model $named -ClockOffsetMinutes 0 -MemoryId ([uint32]0x0A1B2C3D)
Assert-True ($withMemory -match '(?m)^memory-id = 0a1b2c3d\r$') 'own settings: the memory id, in hex as the kernel reads it'
Assert-Equal ([uint32]0x0A1B2C3D) (Get-OwnSettingsMemoryId $withMemory) 'own settings: the memory id read back'
Assert-Equal ([uint32]4294967295) (Get-OwnSettingsMemoryId (New-OwnSettingsText -Model '' -ClockOffsetMinutes 0 -MemoryId ([uint32]::MaxValue))) 'own settings: the biggest memory id'
Assert-Equal $null (Get-OwnSettingsMemoryId "memory-id = 0`r`n") 'own settings: a memory id of 0 is none'
Assert-Equal $named (Get-OwnSettingsModel $withMemory) 'own settings: the model read back next to a memory id'
# Meta's Model API (Muse Spark answers when its key is there): a key and a model, put there by hand or pasted when setup
# asked, written as the kernel reads them and read back, kept when setup runs again.
$withMeta = New-OwnSettingsText -Model '' -ClockOffsetMinutes 0 -MetaKey 'mk-Test_key.123' -MetaModel 'muse-spark-1.3'
Assert-True ($withMeta -match '(?m)^meta-api-key = mk-Test_key\.123\r$') 'own settings: Meta''s key, on a line of its own'
Assert-True ($withMeta -match '(?m)^meta-model = muse-spark-1\.3\r$') 'own settings: Meta''s model, on a line of its own'
Assert-Equal 'mk-Test_key.123' (Get-OwnSettingsValue $withMeta 'meta-api-key') 'own settings: Meta''s key read back'
Assert-Equal 'muse-spark-1.3' (Get-OwnSettingsValue $withMeta 'meta-model') 'own settings: Meta''s model read back'
Assert-Equal '' (Get-OwnSettingsValue $ownText 'meta-api-key') 'own settings: no key read back unless one was given'
Assert-True ($ownText -notmatch 'meta') 'own settings: no Meta lines unless given'
Assert-True (Test-OwnMetaKey 'mk-Test_key.123') 'Meta key: one that can be'
Assert-True (-not (Test-OwnMetaKey 'two words')) 'Meta key: no spaces'
Assert-True (-not (Test-OwnMetaKey '')) 'Meta key: not empty'
Assert-True (-not (Test-OwnMetaKey ('x' * 513))) 'Meta key: at most 512 characters'
Assert-True (-not (Test-OwnMetaKey ('caf' + [char]0x00E9 + 'keyvalue'))) 'Meta key: ASCII only'

# Its memory file, as Chat Computer's own system reads it (os/remember: 512-byte chunks, each tagged with "CCMEMORY",
# the file's id and the chunk's number).
Assert-Equal '43-43-4D-45-4D-4F-52-59-FE-CA-ED-5E-03-00-00-00' ([BitConverter]::ToString((New-OwnMemoryTag -Id ([uint32]0x5EEDCAFE) -Index 3))) 'own memory: a chunk''s tag'
$memBytes = New-OwnMemoryBytes -Id ([uint32]0x5EEDCAFE) -Bytes 8192
Assert-Equal 8192 $memBytes.Length 'own memory: whole chunks'
Assert-Equal '43-43-4D-45-4D-4F-52-59-FE-CA-ED-5E-0F-00-00-00' ([BitConverter]::ToString($memBytes, 15 * 512, 16)) 'own memory: the last chunk''s tag'
Assert-Equal 0 (@($memBytes[16..511] | Where-Object { $_ -ne 0 })).Count 'own memory: nothing kept yet'
Assert-Equal 1000 (@(1..1000 | ForEach-Object { New-OwnMemoryId } | Where-Object { $_ -ne 0 } | Sort-Object -Unique)).Count 'own memory: random ids, never 0'
$memTmp = Join-Path ([IO.Path]::GetTempPath()) ('chatcomputer-memory-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $memTmp | Out-Null
try {
    $memFile = Join-Path $memTmp 'memory.bin'
    $memRoot = [IO.Path]::GetPathRoot($memTmp)
    # A first setup: a new file, at the biggest size.
    $memId = Install-OwnMemory -Dir $memTmp -Root $memRoot -Id $null
    Assert-True ($memId -ne 0) 'own memory: made'
    Assert-Equal (4 * 1024 * 1024) ((Get-Item -LiteralPath $memFile).Length) 'own memory: 4 MB'
    Assert-True (Test-OwnMemoryFile -Path $memFile -Id $memId) 'own memory: its tags'
    Assert-True (-not (Test-OwnMemoryFile -Path $memFile -Id ([uint32]($memId -bxor 1)))) 'own memory: another id''s file is not this one'
    Assert-True (-not (Test-OwnMemoryFile -Path $memFile -Id $null)) 'own memory: no id, no file'
    # Setting up again: the file stays, with what it keeps.
    $stream = [IO.File]::OpenWrite($memFile)
    $stream.Position = 20
    $stream.WriteByte(0x42)
    $stream.Dispose()
    Assert-Equal $memId (Install-OwnMemory -Dir $memTmp -Root $memRoot -Id $memId) 'own memory: set up again, the same file'
    Assert-Equal 0x42 ([IO.File]::ReadAllBytes($memFile)[20]) 'own memory: what it keeps stays'
    # Settings whose id isn't the file's (or no settings): a new file, with a new id.
    $newId = Install-OwnMemory -Dir $memTmp -Root $memRoot -Id ([uint32]($memId -bxor 1))
    Assert-True ($newId -ne 0 -and $newId -ne $memId) 'own memory: a new file when the id is not the file''s'
    Assert-True (Test-OwnMemoryFile -Path $memFile -Id $newId) 'own memory: the new file''s tags'
    Assert-Equal 0 ([IO.File]::ReadAllBytes($memFile)[20]) 'own memory: the new file keeps nothing yet'
    # Something added at its end, or a file cut mid-chunk, is not the memory file.
    $grown = [IO.File]::ReadAllBytes($memFile) + (New-Object byte[] 512)
    [IO.File]::WriteAllBytes($memFile, [byte[]]$grown)
    Assert-True (-not (Test-OwnMemoryFile -Path $memFile -Id $newId)) 'own memory: a chunk added at its end that is not its own'
    [IO.File]::WriteAllBytes($memFile, [byte[]]($grown[0..4999]))
    Assert-True (-not (Test-OwnMemoryFile -Path $memFile -Id $newId)) 'own memory: a file cut mid-chunk is not it'
    # Too little room on the partition: the smaller size, or none.
    $roomBefore = $Script:OwnEspRoom
    $free = Get-OwnFreeSpace $memRoot
    if ($null -ne $free) {
        Remove-Item -LiteralPath $memFile
        $Script:OwnEspRoom = $free - 2MB
        $smallId = Install-OwnMemory -Dir $memTmp -Root $memRoot -Id $null
        Assert-True ($smallId -ne 0) 'own memory: made smaller when there is less room'
        Assert-Equal (1024 * 1024) ((Get-Item -LiteralPath $memFile).Length) 'own memory: 1 MB'
        $Script:OwnEspRoom = $free + 1GB
        Assert-Equal ([uint32]0) (Install-OwnMemory -Dir $memTmp -Root $memRoot -Id ([uint32]($smallId -bxor 1))) 'own memory: none when there is no room'
        Assert-True (-not (Test-Path -LiteralPath $memFile)) 'own memory: no file left behind'
    } else {
        Write-Host 'own memory: free space unknown here, room not tested'
    }
    $Script:OwnEspRoom = $roomBefore
    # Chat Computer's own system's file: refused, saying so, where it doesn't fit (in place of the one there).
    if ($null -ne $free) {
        $sys = Join-Path $memTmp 'chatcomputer.efi'
        Assert-OwnRoom -Root $memRoot -Path $sys -Size 4MB
        $said = ''
        try { Assert-OwnRoom -Root $memRoot -Path $sys -Size ($free + 1GB) } catch { $said = $_.Exception.Message }
        Assert-True ($said -like 'The partition this PC starts from has * MB free, and Chat Computer''s own system needs * MB there.') ('own system: no room said: ' + $said)
        [IO.File]::WriteAllBytes($sys, (New-Object byte[] 4096))
        $said = ''
        try { Assert-OwnRoom -Root $memRoot -Path $sys -Size ($free - 1MB + 4096) } catch { $said = $_.Exception.Message }
        Assert-Equal '' $said 'own system: the one there counts as room'
    }
} finally {
    Remove-Item -LiteralPath $memTmp -Recurse -Force
}

# Its log file (os/kernel/src/journal.rs), made as the memory file is with an id of its own (log-id in the settings),
# and read back here the way Chat Computer's own system keeps it (os/remember): two halves written in turn, each
# starting with a header ("CCMEMDAT", version 1, a sequence number, the data's length and its FNV-1a hash) that the
# data follows, across the 496 bytes after each 512-byte chunk's tag. What's read is the newest whole copy.
function Get-TestFnv1a([byte[]]$Data) {
    # The data's FNV-1a hash (64 bits, little-endian), worked in two 32-bit halves so that nothing passes 64 bits.
    $hi = [uint64]3421674724
    $lo = [uint64]2216829733
    $mask = [uint64]4294967295
    foreach ($b in $Data) {
        $lo = $lo -bxor [uint64]$b
        $t = $lo * [uint64]435
        $hi = ($hi * [uint64]435 + $lo * [uint64]256 + ($t -shr 32)) -band $mask
        $lo = $t -band $mask
    }
    return , [byte[]]([BitConverter]::GetBytes([uint32]$lo) + [BitConverter]::GetBytes([uint32]$hi))
}
function Add-TestKept([byte[]]$File, [int]$Which, [uint64]$Sequence, [byte[]]$Data, [int64]$Length = -1) {
    # A copy of $Data kept in half $Which (0 or 1) of $File, as os/remember writes one. $Length: what its header says
    # the data's length is, if not the data's own.
    if ($Length -lt 0) { $Length = $Data.Length }
    $all = New-Object byte[] (32 + $Data.Length)
    [Array]::Copy([Text.Encoding]::ASCII.GetBytes('CCMEMDAT'), 0, $all, 0, 8)
    [Array]::Copy([BitConverter]::GetBytes([uint32]1), 0, $all, 8, 4)
    [Array]::Copy([BitConverter]::GetBytes($Sequence), 0, $all, 12, 8)
    [Array]::Copy([BitConverter]::GetBytes([uint32]$Length), 0, $all, 20, 4)
    [Array]::Copy((Get-TestFnv1a $Data), 0, $all, 24, 8)
    [Array]::Copy($Data, 0, $all, 32, $Data.Length)
    $firstChunk = $Which * [int][Math]::Floor($File.Length / 1024)
    for ($k = 0; $k * 496 -lt $all.Length; $k++) {
        [Array]::Copy($all, $k * 496, $File, ($firstChunk + $k) * 512 + 16, [Math]::Min(496, $all.Length - $k * 496))
    }
}
function Get-TestKeptText([byte[]]$File) {
    # What Read-OwnKept finds in $File, as text ('(none)' if it finds nothing).
    $keptBytes = Read-OwnKept $File
    if ($null -eq $keptBytes) { return '(none)' }
    return [Text.Encoding]::UTF8.GetString($keptBytes)
}
function ConvertTo-TestBytes([string]$Text) { return , [byte[]][Text.Encoding]::UTF8.GetBytes($Text) }
Assert-Equal '0B-BD-AA-80-46-D8-30-A4' ([BitConverter]::ToString((Get-TestFnv1a (ConvertTo-TestBytes 'hello')))) 'own log: the copies made here carry os/remember''s hash (FNV-1a of "hello")'
$logBlank = New-OwnMemoryBytes -Id ([uint32]0x10C0FFEE) -Bytes 512KB
$second = $logBlank.Length / 2 + 16               # the second half's header: after its first chunk's tag
Assert-Equal '(none)' (Get-TestKeptText $logBlank) 'own log: a new file keeps nothing'
Assert-Equal '(none)' (Get-TestKeptText (New-Object byte[] 1536)) 'own log: nothing in a file too small for two halves'
$kept = $logBlank.Clone()
Add-TestKept $kept 0 1 (ConvertTo-TestBytes "Chat Computer kernel: started`n")
Assert-True ((Read-OwnKept $kept) -is [byte[]]) 'own log: read back as bytes'
Assert-Equal "Chat Computer kernel: started`n" (Get-TestKeptText $kept) 'own log: one copy'
$kept = $logBlank.Clone()
Add-TestKept $kept 1 1 (ConvertTo-TestBytes 'only in the second half')
Assert-Equal 'only in the second half' (Get-TestKeptText $kept) 'own log: a copy in the second half, its header in that half''s first chunk'
# Two copies: the one with the higher sequence number, in either half.
$kept = $logBlank.Clone()
Add-TestKept $kept 0 7 (ConvertTo-TestBytes 'older')
Add-TestKept $kept 1 8 (ConvertTo-TestBytes 'newer')
Assert-Equal 'newer' (Get-TestKeptText $kept) 'own log: two copies, the newer in the second half'
$kept = $logBlank.Clone()
Add-TestKept $kept 0 4294967296 (ConvertTo-TestBytes 'newer')
Add-TestKept $kept 1 4294967295 (ConvertTo-TestBytes 'older')
Assert-Equal 'newer' (Get-TestKeptText $kept) 'own log: two copies, the newer in the first half (sequence numbers past 32 bits)'
# A half whose header isn't one (its magic gone wrong, another version) holds no copy, whatever its sequence number.
$kept = $logBlank.Clone()
Add-TestKept $kept 0 3 (ConvertTo-TestBytes 'whole')
Add-TestKept $kept 1 4 (ConvertTo-TestBytes 'damaged')
$kept[$second] = [byte][char]'X'
Assert-Equal 'whole' (Get-TestKeptText $kept) 'own log: a half with a bad magic is no copy'
$kept[$second] = [byte][char]'C'
$kept[$second + 8] = 2
Assert-Equal 'whole' (Get-TestKeptText $kept) 'own log: nor is one of another version'
# Data across several chunks: each chunk's 496 bytes after its tag, in order.
$longLog = (1..60 | ForEach-Object { 'Chat Computer kernel: line ' + $_ + ' of a long start' }) -join "`n"
$kept = $logBlank.Clone()
Add-TestKept $kept 1 2 (ConvertTo-TestBytes $longLog)
Assert-Equal $longLog (Get-TestKeptText $kept) 'own log: data across several chunks, whole and in order'
# A half of 8 chunks holds 8 x 496 - 32 = 3936 bytes: a copy that fills it is read, a header saying more is no copy.
$small = New-OwnMemoryBytes -Id ([uint32]0x10C0FFEE) -Bytes 8192
$kept = $small.Clone()
Add-TestKept $kept 1 1 (ConvertTo-TestBytes ('x' * 3936))
Assert-Equal ('x' * 3936) (Get-TestKeptText $kept) 'own log: a copy that fills its half, to the file''s last chunk'
$kept = $small.Clone()
Add-TestKept $kept 0 1 (ConvertTo-TestBytes 'whole')
Add-TestKept $kept 1 2 (ConvertTo-TestBytes ('x' * 3936)) -Length 3937
Assert-Equal 'whole' (Get-TestKeptText $kept) 'own log: a length more than a half holds is no copy (the other half''s is read)'
$kept = $small.Clone()
Add-TestKept $kept 0 1 (ConvertTo-TestBytes 'x') -Length 100000
Assert-Equal '(none)' (Get-TestKeptText $kept) 'own log: nor is it when it''s the only one'

# The settings name the log file's id too, and keep a line put there by hand for the most processor cores to use, as
# the kernel reads them (os/kernel/src/settings.rs).
$withLog = New-OwnSettingsText -Model '' -ClockOffsetMinutes 60 -MemoryId ([uint32]0x0A1B2C3D) -LogId ([uint32]0x10C0FFEE) -Cores '2'
Assert-True ($withLog -match '(?m)^log-id = 10c0ffee\r$') 'own settings: the log id, in hex as the kernel reads it'
Assert-True ($withLog -match '(?m)^cores = 2\r$') 'own settings: the most cores, on a line of its own'
Assert-Equal ([uint32]0x10C0FFEE) (Get-OwnSettingsMemoryId $withLog -Name 'log-id') 'own settings: the log id read back'
Assert-Equal ([uint32]0x0A1B2C3D) (Get-OwnSettingsMemoryId $withLog) 'own settings: the memory id read back next to it, not the log''s'
Assert-Equal '2' (Get-OwnSettingsCores $withLog) 'own settings: the most cores read back'
Assert-Equal $null (Get-OwnSettingsMemoryId "log-id = 10c0ffee`r`n") 'own settings: a log id is no memory id'
Assert-Equal $null (Get-OwnSettingsMemoryId $ownText -Name 'log-id') 'own settings: no log id read back without a log file'
Assert-Equal $null (Get-OwnSettingsMemoryId "log-id = 0`r`n" -Name 'log-id') 'own settings: a log id of 0 is none'
Assert-True ($ownText -notmatch '(?m)^(log-id|cores)') 'own settings: no log id or most cores unless given'
Assert-Equal '' (Get-OwnSettingsCores $ownText) 'own settings: no most cores read back'
Assert-Equal '' (Get-OwnSettingsCores $null) 'own settings: no settings, no most cores'
Assert-Equal '7' (Get-OwnSettingsCores "cores = 007`r`n") 'own settings: the most cores, as a number'
# What the kernel doesn't take as a number of cores isn't kept: 0, or anything else that isn't a number.
foreach ($junk in @('cores = 0', 'cores = all', 'cores = -1', 'cores = 1.5', 'cores = 2 # core 3 hangs', 'cores =', 'cores')) {
    Assert-Equal '' (Get-OwnSettingsCores ($junk + "`r`n")) ('own settings: not a number of cores: ' + $junk)
}
# The kernel reads a setting's name in capitals or not, and its value in quotes or not: written so by hand, kept too.
Assert-Equal '2' (Get-OwnSettingsCores "Cores = 2`r`n") 'own settings: the most cores, its name in capitals'
Assert-Equal '2' (Get-OwnSettingsCores "cores = `"2`"`r`n") 'own settings: the most cores, in quotes'
# Set up again, the settings are written anew: the most cores stays, a 0 doesn't come back.
$again = New-OwnSettingsText -Model '' -ClockOffsetMinutes 0 -Cores (Get-OwnSettingsCores "# by hand`r`ncores = 1`r`n")
Assert-Equal '1' (Get-OwnSettingsCores $again) 'own settings: the most cores kept when set up again'
Assert-True ((New-OwnSettingsText -Model '' -ClockOffsetMinutes 0 -Cores (Get-OwnSettingsCores "cores = 0`r`n")) -notmatch 'cores') 'own settings: cores = 0 not written back'

# The log file on the start-up partition (a folder here): made; kept when the settings name it; a new one when they
# name another; none when there's no room.
$logTmp = Join-Path ([IO.Path]::GetTempPath()) ('chatcomputer-log-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $logTmp | Out-Null
$logRoomBefore = $Script:OwnEspRoom
try {
    $logFile = Join-Path $logTmp 'log.bin'
    $logRoot = [IO.Path]::GetPathRoot($logTmp)
    $logId = Install-OwnLog -Dir $logTmp -Root $logRoot -Id $null
    Assert-True ($logId -is [uint32] -and $logId -ne 0) 'own log: made'
    Assert-Equal (512 * 1024) ((Get-Item -LiteralPath $logFile).Length) 'own log: 512 KB'
    Assert-True (Test-OwnMemoryFile -Path $logFile -Id $logId) 'own log: every chunk tagged with its id'
    Assert-Equal '(none)' (Get-TestKeptText ([IO.File]::ReadAllBytes($logFile))) 'own log: nothing kept in it yet'
    # Set up again with its id in the settings: the same file stays, with the last run's log in it.
    $logBytes = [IO.File]::ReadAllBytes($logFile)
    Add-TestKept $logBytes 0 1 (ConvertTo-TestBytes "Chat Computer: ready`n")
    [IO.File]::WriteAllBytes($logFile, $logBytes)
    Assert-Equal $logId (Install-OwnLog -Dir $logTmp -Root $logRoot -Id $logId) 'own log: set up again, the same file'
    Assert-Equal "Chat Computer: ready`n" (Get-TestKeptText ([IO.File]::ReadAllBytes($logFile))) 'own log: the last run''s log in it stays'
    # Settings whose log id isn't the file's (an older setup's file, say): a new file, with a new id.
    $newLogId = Install-OwnLog -Dir $logTmp -Root $logRoot -Id ([uint32]($logId -bxor 1))
    Assert-True ($newLogId -ne 0 -and $newLogId -ne $logId) 'own log: a new file when the id is not the file''s'
    Assert-True (Test-OwnMemoryFile -Path $logFile -Id $newLogId) 'own log: the new file''s tags'
    Assert-Equal '(none)' (Get-TestKeptText ([IO.File]::ReadAllBytes($logFile))) 'own log: the new file keeps nothing yet'
    $logFree = Get-OwnFreeSpace $logRoot
    if ($null -ne $logFree) {
        $Script:OwnEspRoom = $logFree + 1GB
        Assert-Equal ([uint32]0) (Install-OwnLog -Dir $logTmp -Root $logRoot -Id ([uint32]($newLogId -bxor 1))) 'own log: none when there is no room'
        Assert-True (-not (Test-Path -LiteralPath $logFile)) 'own log: no file left behind'
    } else {
        Write-Host 'own log: free space unknown here, room not tested'
    }
} finally {
    $Script:OwnEspRoom = $logRoomBefore
    Remove-Item -LiteralPath $logTmp -Recurse -Force
}

# What's said about the last run, from its log (os/kernel/src/serial.rs, with plain line ends): when, by its clock's
# line; why it stopped, if it did; else, for a start that never got ready, its last step; else that it was ready.
$clockLine = 'Chat Computer kernel: clock: 4 November 2026, 10:10 UTC (local time, offset from the settings)'
$started = "Chat Computer kernel: started`n" + $clockLine + "`nChat Computer kernel: next: the network`n"
$panic = $started + "Chat Computer: ready`n`nChat Computer kernel stopped: panicked at kernel/src/cores.rs:581:9:`nindex out of bounds: the len is 2 but the index is 5`n"
$said = Get-OwnLastRunSummary $panic
Assert-True ($said -clike 'The last time Chat Computer''s own system ran (4 November 2026, 10:10 UTC*') ('last run: when, by its clock: ' + $said)
Assert-True ($said -clike '*, it stopped: panicked at kernel/src/cores.rs:581:9: index out of bounds: the len is 2 but the index is 5') ('last run: a panic, its place and, from the next line, its message: ' + $said)
$fault = $started + "Chat Computer: ready`n`nChat Computer kernel stopped: Page fault (vector 14, error 0x2)`nrip 0xffffffff8005cf07  cs 0x8  rflags 0x282`n"
$said = Get-OwnLastRunSummary $fault
Assert-True ($said -clike '*, it stopped: Page fault (vector 14, error 0x2)') ('last run: a fault, on one line (not the registers after it): ' + $said)
# Another processor core's stop (os/kernel/src/cores.rs) says which core: the first core may stop after it (saying so
# in its own words), or go on without it.
$coreStop = $started + "Chat Computer: ready`n`nChat Computer kernel: processor core 3 stopped: panicked at kernel/src/model.rs:40:5:`nattempt to divide by zero`n" +
    "`nChat Computer kernel stopped: panicked at kernel/src/cores.rs:600:9:`nprocessor core 3 stopped: panicked at kernel/src/model.rs:40:5:`nattempt to divide by zero`n"
$said = Get-OwnLastRunSummary $coreStop
Assert-True ($said -clike '*processor core 3 stopped: panicked at kernel/src/model.rs:40:5: attempt to divide by zero') ('last run: another core''s stop, which core and why: ' + $said)
$coreFault = $started + "`nChat Computer kernel: processor core 1 stopped: General protection fault (vector 13, error 0x0, at 0xffffffff80012345, address 0x0, stack 0xfffff07ffffbff70)`nChat Computer: ready`n"
$said = Get-OwnLastRunSummary $coreFault
Assert-True ($said -clike '*processor core 1 stopped: General protection fault (vector 13, error 0x0, *') ('last run: another core''s fault, the rest going on to be ready: ' + $said)
$said = Get-OwnLastRunSummary $started
Assert-True ($said -clike '*ran (4 November 2026, 10:10 UTC*, it didn''t finish starting. The last thing in its log: Chat Computer kernel: next: the network') ('last run: a start that never got ready, and its last step: ' + $said)
Assert-Equal 'The last time Chat Computer''s own system ran, it didn''t finish starting. The last thing in its log: Chat Computer kernel: started' `
    (Get-OwnLastRunSummary "Chat Computer kernel: started`n") 'last run: with no clock line, no when'
$clean = ($started + "Chat Computer: ready`nChat Computer kernel: memory: the conversation kept (812 bytes)`nChat Computer kernel: turning off`n") -replace "`n", "`r`n"
$said = Get-OwnLastRunSummary $clean
Assert-True ($said -clike '*ran (4 November 2026, 10:10 UTC*, it started, and nothing went wrong that it noticed.') ('last run: ready, and nothing stopped (Windows line ends too): ' + $said)
# Only the kernel's own words for a stop are one: "stopped: " in a line for another reason (a USB drive's own name,
# here) isn't.
$innocent = $started + "Chat Computer kernel: USB drive `"Stopped: Pro 128`": its medium was taken out`nChat Computer: ready`n"
$said = Get-OwnLastRunSummary $innocent
Assert-True ($said -clike '*, it started, and nothing went wrong that it noticed.') ('last run: "stopped: " in another line is no stop: ' + $said)
Assert-Equal $null (Get-OwnLastRunSummary '') 'last run: nothing to say about an empty log'
Assert-Equal $null (Get-OwnLastRunSummary "`n`r`n  `n") 'last run: nor about one of empty lines'

# Reading it back (the start-up partition a folder here, Get-WindowsEsp and its mounting stood in for): saved in the
# work folder with Windows' line ends, and said in a line. No log, one the settings don't name, or anything going
# wrong on the way: nothing to say, and never an error that would stop the setup (the partition closed again).
$Script:FakeEsp = Join-Path ([IO.Path]::GetTempPath()) ('chatcomputer-esp-' + [guid]::NewGuid().ToString('N'))
$Script:FakeEspFails = $false
$Script:FakeEspClosed = 0
function Get-WindowsEsp { return [pscustomobject]@{ IsSystem = $true } }
function Mount-EspPartition {
    param($Partition)
    if ($Script:FakeEspFails) { throw 'Windows could not open its EFI partition.' }
    return [pscustomobject]@{ Root = $Script:FakeEsp }
}
function Dismount-EspPartition { param($Mount) $Script:FakeEspClosed++ }
$runWork = Join-Path $Script:FakeEsp 'work'
$runDir = [IO.Path]::Combine($Script:FakeEsp, 'EFI', 'chatcomputer-own')
$runSettings = Join-Path $runDir 'settings.txt'
New-Item -ItemType Directory -Path $runWork, $runDir -Force | Out-Null
try {
    Assert-Equal $null (Read-OwnLastRun -WorkDir $runWork) 'last run: nothing when there are no settings or log there'
    $runText = "Chat Computer kernel: started`nChat Computer kernel: files: Caf" + [char]0x00E9 + " (D:)`nChat Computer: ready`n"
    $runFile = New-OwnMemoryBytes -Id ([uint32]0x10C0FFEE) -Bytes 8192
    Add-TestKept $runFile 1 5 (ConvertTo-TestBytes $runText)
    [IO.File]::WriteAllBytes((Join-Path $runDir 'log.bin'), $runFile)
    [IO.File]::WriteAllText($runSettings, (New-OwnSettingsText -Model '' -ClockOffsetMinutes 0 -LogId ([uint32]0x10C0FFEE)))
    $Script:FakeEspClosed = 0
    $run = Read-OwnLastRun -WorkDir $runWork
    Assert-True ([string](Get-Prop $run 'Summary') -clike '*, it started, and nothing went wrong that it noticed.') 'last run: said in a line'
    Assert-Equal (Join-Path $runWork 'last-run.txt') (Get-Prop $run 'Path') 'last run: saved in the work folder'
    Assert-Equal ($runText -replace "`n", "`r`n") ([IO.File]::ReadAllText((Join-Path $runWork 'last-run.txt'))) 'last run: its whole log, as UTF-8 with Windows'' line ends'
    Assert-Equal 1 $Script:FakeEspClosed 'last run: the start-up partition closed again'
    [IO.File]::WriteAllText($runSettings, (New-OwnSettingsText -Model '' -ClockOffsetMinutes 0 -LogId ([uint32]0x10C0FFEF)))
    Assert-Equal $null (Read-OwnLastRun -WorkDir $runWork) 'last run: nothing from a log file the settings don''t name'
    [IO.File]::WriteAllText($runSettings, (New-OwnSettingsText -Model '' -ClockOffsetMinutes 0 -MemoryId ([uint32]0x0A1B2C3D)))
    Assert-Equal $null (Read-OwnLastRun -WorkDir $runWork) 'last run: nothing when the settings name no log file (an older setup''s)'
    $Script:FakeEspFails = $true
    Assert-Equal $null (Read-OwnLastRun -WorkDir $runWork) 'last run: nothing when the start-up partition can''t be opened'
    $Script:FakeEspFails = $false
    [IO.File]::WriteAllText($runSettings, (New-OwnSettingsText -Model '' -ClockOffsetMinutes 0 -LogId ([uint32]0x10C0FFEE)))
    Assert-Equal $null (Read-OwnLastRun -WorkDir (Join-Path $runWork 'not-there')) 'last run: nothing when it can''t be saved'
    # The settings there now (for a key for Meta's Model API kept there): read, or '' when they can't be.
    [IO.File]::WriteAllText($runSettings, (New-OwnSettingsText -Model '' -ClockOffsetMinutes 0 -MetaKey 'mk-Test_key.123'))
    $Script:FakeEspClosed = 0
    Assert-Equal 'mk-Test_key.123' (Get-OwnSettingsValue (Read-OwnSettings) 'meta-api-key') 'settings now: Meta''s key read from the start-up partition'
    Assert-Equal 1 $Script:FakeEspClosed 'settings now: the start-up partition closed again'
    $Script:FakeEspFails = $true
    Assert-Equal '' (Read-OwnSettings) 'settings now: nothing when the start-up partition can''t be opened'
    $Script:FakeEspFails = $false
    # Reading failing part-way (the settings can't be read: here, a folder of that name).
    Remove-Item -LiteralPath $runSettings
    New-Item -ItemType Directory -Path $runSettings | Out-Null
    $Script:FakeEspClosed = 0
    Assert-Equal $null (Read-OwnLastRun -WorkDir $runWork) 'last run: nothing when reading fails part-way'
    Assert-Equal 1 $Script:FakeEspClosed 'last run: the start-up partition closed again then too'
    Assert-Equal '' (Read-OwnSettings) 'settings now: nothing when the settings aren''t a file'
} finally {
    Remove-Item -LiteralPath $Script:FakeEsp -Recurse -Force
    Remove-Item Function:\Get-WindowsEsp, Function:\Mount-EspPartition, Function:\Dismount-EspPartition
    . (Join-Path $Win 'lib/Startup.ps1')    # the real ones again
}

# ------------------------------------------------------------------ files from a disc image
# On a real PC, grub.cfg copied from the mounted Ubuntu image kept the image's read-only mark, and setup
# couldn't patch it ("Access to the path 'S:\boot\grub\grub.cfg' is denied"). tests/win_ci/iso.ps1 checks
# the real thing on Windows; these check the parts.
$roDir = Join-Path ([IO.Path]::GetTempPath()) ('chatcomputer-ro-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $roDir -Force | Out-Null
try {
    $roFile = Join-Path $roDir 'grub.cfg'
    [IO.File]::WriteAllText($roFile, 'from the disc')
    [IO.File]::SetAttributes($roFile, ([IO.File]::GetAttributes($roFile) -bor [IO.FileAttributes]::ReadOnly))
    Assert-True (([IO.File]::GetAttributes($roFile) -band [IO.FileAttributes]::ReadOnly) -ne 0) 'read-only file: marked'
    Write-TextFileLf -Path $roFile -Text "patched`r`n"
    Assert-Equal "patched`n" ([IO.File]::ReadAllText($roFile)) 'read-only file: rewritten'
    Assert-True (([IO.File]::GetAttributes($roFile) -band [IO.FileAttributes]::ReadOnly) -eq 0) 'read-only file: mark taken off'
} finally {
    Get-ChildItem -LiteralPath $roDir -Recurse -File | ForEach-Object { $_.IsReadOnly = $false }
    Remove-Item -LiteralPath $roDir -Recurse -Force
}
$Script:RobocopyArgs = @()
function robocopy.exe { $Script:RobocopyArgs = @($args); $global:LASTEXITCODE = 1 }
Invoke-Robocopy -From 'E:\' -To 'S:\'
Assert-True ($Script:RobocopyArgs -contains '/A-:R') 'robocopy takes the read-only mark off the copies'
$global:LASTEXITCODE = 8
function robocopy.exe { $global:LASTEXITCODE = 8 }
Assert-Throws { Invoke-Robocopy -From 'E:\' -To 'S:\' } 'robocopy failing (code 8 and up) stops setup'
Remove-Item Function:\robocopy.exe
$global:LASTEXITCODE = 0

# What went wrong, in plain words: a failed .NET call's own error, not PowerShell's wrapping of it.
try { [IO.File]::ReadAllText((Join-Path $roDir 'not-there.txt')) | Out-Null } catch { $plain = Get-ErrorText $_ }
Assert-True ($plain -notmatch 'Exception calling') ('error text unwrapped: ' + $plain)
Assert-True ($plain -match 'not-there') ('error text names the file: ' + $plain)
try { throw 'Setup stopped: something.' } catch { $own = Get-ErrorText $_ }
Assert-Equal 'Setup stopped: something.' $own 'error text: a thrown message as it is'

# ------------------------------------------------------------------ where Chat Computer updates itself from
Assert-Equal 'Users/Sam Lee/Downloads/ChatComputer/ChatComputer' (ConvertTo-UpdateSource -FullPath 'C:\Users\Sam Lee\Downloads\ChatComputer\ChatComputer' -SystemDrive 'C:') 'update folder below C:'
Assert-Equal 'Users/x/ChatComputer' (ConvertTo-UpdateSource -FullPath 'c:\Users\x\ChatComputer\' -SystemDrive 'C:') 'update folder: drive letter case, last backslash'
Assert-Equal '' (ConvertTo-UpdateSource -FullPath 'D:\ChatComputer' -SystemDrive 'C:') 'update folder on another drive: none'
Assert-Equal '' (ConvertTo-UpdateSource -FullPath '\\server\share\ChatComputer' -SystemDrive 'C:') 'update folder on a network share: none'
Assert-Equal '' (ConvertTo-UpdateSource -FullPath 'C:\' -SystemDrive 'C:') 'the top of the drive: none'
Assert-Equal '' (ConvertTo-UpdateSource -FullPath 'C:\Users\..\Windows' -SystemDrive 'C:') 'update folder with ..: none'
Assert-Equal '' (ConvertTo-UpdateSource -FullPath ('C:\Users\a' + [char]10 + 'b') -SystemDrive 'C:') 'update folder with a line break: none'
Assert-Equal '' (ConvertTo-UpdateSource -FullPath 'C:\Users\x' -SystemDrive '') 'not Windows: none'
if ($env:SystemDrive) {
    # On Windows: the names as the drive stores them, whatever case the path was typed in.
    $caseRoot = Join-Path ([IO.Path]::GetTempPath()) ('ChatComputer-Case-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    $made = New-Item -ItemType Directory -Path (Join-Path $caseRoot 'MiXeD Folder') -Force
    try {
        # (The temporary folder's path can have short 8.3 names in it, which this spells out in full.)
        $typed = Join-Path $caseRoot.ToLowerInvariant() 'mixed folder'
        $found = Get-TrueCasePath -Path $typed
        Assert-Equal (Get-TrueCasePath -Path $made.FullName) $found 'a folder named as the drive stores it, however typed'
        Assert-True ($found.EndsWith('\' + (Split-Path -Leaf $caseRoot) + '\MiXeD Folder')) ('the names keep their case: ' + $found)
    } finally {
        Remove-Item -LiteralPath $caseRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

if ($OutDir) {
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
    Write-TextFileLf -Path (Join-Path $OutDir 'autoinstall-wifi.yaml') -Text $yaml
    Write-TextFileLf -Path (Join-Path $OutDir 'autoinstall-wired.yaml') -Text $wiredOnly
    Write-TextFileLf -Path (Join-Path $OutDir 'grub.cfg') -Text $grubOut
    Write-TextFileLf -Path (Join-Path $OutDir 'settings.json') -Text $settings
    Write-TextFileLf -Path (Join-Path $OutDir 'autoinstall-wide.yaml') -Text $wideYaml
    Write-TextFileLf -Path (Join-Path $OutDir 'settings-wide.json') -Text $wideSettings
    $vectors = @()
    foreach ($pw in @('', 'a', 'kq7m-3vtz-p9xa-hc4r', ('long ' * 30), ('p' + [char]0x00E4 + 'ss'))) {
        foreach ($salt in @('x', 'saltstring', 'abcdefghijklmnop')) {
            $vectors += [pscustomobject]@{ password = $pw; salt = $salt; hash = (Get-Sha512Crypt -Password $pw -Salt $salt) }
        }
    }
    $json = $vectors | ConvertTo-Json -Depth 3
    Write-TextFileLf -Path (Join-Path $OutDir 'crypt-vectors.json') -Text $json
}

Write-Host ('' + ($Script:Count - $Script:Failures) + '/' + $Script:Count + ' checks passed')
if ($Script:Failures -gt 0) { exit 1 }
exit 0
