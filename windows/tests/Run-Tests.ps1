# Unit tests for ClaudeOS Setup's plain functions. Runs on any PowerShell (Windows 5.1, or pwsh on Linux).
#   pwsh -NoProfile -File windows/tests/Run-Tests.ps1 [-OutDir DIR]
# With -OutDir, also writes sample generated files there for the Python tests to validate.
param([string]$OutDir)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0
$Here = $PSScriptRoot
$Win = Split-Path -Parent $Here
foreach ($lib in @('Common', 'Timezones', 'Autoinstall', 'Disks', 'Network', 'ClaudeSignIn', 'Ubuntu', 'Installer', 'Checks', 'Startup', 'OwnOs')) {
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
Assert-Equal 'rensh' (ConvertTo-LinuxUserName 'rensh') 'plain name'
Assert-Equal 'renshu' (ConvertTo-LinuxUserName 'Ren Shu') 'spaces and capitals'
Assert-Equal 'jose' (ConvertTo-LinuxUserName ('Jos' + [char]0x00E9)) 'accent removed'
Assert-Equal 'u123abc' (ConvertTo-LinuxUserName '123abc') 'leading digit'
Assert-Equal 'root2' (ConvertTo-LinuxUserName 'root') 'reserved'
Assert-Equal 'admin2' (ConvertTo-LinuxUserName 'Admin') 'reserved after lowercasing'
Assert-Equal 'me' (ConvertTo-LinuxUserName '') 'empty'
Assert-Equal 'me' (ConvertTo-LinuxUserName ([string][char]0x674E + [char]0x660E)) 'only non-latin letters'
Assert-True ((ConvertTo-LinuxUserName ('a' * 80)).Length -le 32) 'long names are cut'
Assert-True (Test-LinuxUserName 'rensh') 'valid name accepted'
Assert-True (-not (Test-LinuxUserName 'Rensh')) 'capitals rejected'
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
Assert-Equal (80 * $GiB) $place.ClaudeOSBytes 'installer: ClaudeOS gets the rest, right before it'
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

# ------------------------------------------------------------------ Claude token
$tok = 'sk-ant-oat01-' + ('Ab9_-' * 20)
Assert-True (Test-ClaudeToken $tok) 'token accepted'
Assert-Equal $tok (ConvertTo-CleanToken ('  "' + $tok.Substring(0, 50) + "`r`n" + $tok.Substring(50) + '" ')) 'wrapped paste cleaned'
Assert-True (-not (Test-ClaudeToken 'sk-ant-oat01-short')) 'short token rejected'
Assert-True (-not (Test-ClaudeToken 'hello')) 'junk rejected'

# ------------------------------------------------------------------ a sign-in code pasted over several lines
# Claude Code wraps the code with real line breaks; the rest of a paste is already waiting in the console.
$origReadSecret = ${function:Read-Secret}
$origReadSecretLine = ${function:Read-SecretLine}
$origInputWaiting = ${function:Test-InputWaiting}
$Script:PasteLines = @()
$Script:PasteIndex = 0
${function:Read-Secret} = { param($Prompt) $Script:PasteIndex = 1; return $Script:PasteLines[0] }
${function:Read-SecretLine} = { $v = $Script:PasteLines[$Script:PasteIndex]; $Script:PasteIndex++; return $v }
${function:Test-InputWaiting} = { return ($Script:PasteIndex -lt $Script:PasteLines.Count) }
$Script:PasteLines = @($tok.Substring(0, 70), $tok.Substring(70))
Assert-Equal $tok (ConvertTo-CleanToken (Read-PastedSecret 'Paste' -SettleMs 1)) 'code pasted over two lines is read whole'
$Script:PasteLines = @($tok.Substring(0, 40), $tok.Substring(40, 40), $tok.Substring(80), '')
Assert-Equal $tok (ConvertTo-CleanToken (Read-PastedSecret 'Paste' -SettleMs 1)) 'three lines, then the Enter after them'
$Script:PasteLines = @($tok)
Assert-Equal $tok (ConvertTo-CleanToken (Read-PastedSecret 'Paste' -SettleMs 1)) 'code on one line'
${function:Read-Secret} = $origReadSecret
${function:Read-SecretLine} = $origReadSecretLine
${function:Test-InputWaiting} = $origInputWaiting

# A sign-in code pasted instead of the browser sign-in: taken whole, asked again after a bad paste (Ctrl+V can
# give one stray character), and Enter goes back.
$origPasted = ${function:Read-PastedSecret}
$Script:Pasted = New-Object System.Collections.Queue
${function:Read-PastedSecret} = { param($Prompt, $SettleMs) return $Script:Pasted.Dequeue() }
$Script:Pasted.Enqueue('  ' + $tok.Substring(0, 60) + "`r`n" + $tok.Substring(60) + ' ')
Assert-Equal $tok (Read-SignInCode -ClaudeExe 'C:\x\claude.exe') 'code: a wrapped paste is read whole'
$Script:Pasted.Enqueue([string][char]22)
$Script:Pasted.Enqueue($tok)
Assert-Equal $tok (Read-SignInCode -ClaudeExe 'C:\x\claude.exe') 'code: a bad paste is asked again'
$Script:Pasted.Enqueue('')
Assert-True ($null -eq (Read-SignInCode -ClaudeExe 'C:\x\claude.exe')) 'code: Enter goes back'
foreach ($i in 1..3) { $Script:Pasted.Enqueue('hello') }
Assert-True ($null -eq (Read-SignInCode -ClaudeExe 'C:\x\claude.exe')) 'code: three bad pastes go back'
Assert-Equal 0 $Script:Pasted.Count 'code: every answer used'

# The sign-in window's .cmd: Windows Terminal when it's there, the person's home folder, a throwaway
# settings folder, "claude auth login"; ASCII with Windows line endings.
$cmd = New-SignInWindowScript -ClaudeExe '%LOCALAPPDATA%\ClaudeOS\setup\claude\claude.exe' -ConfigDir '%TEMP%\claudeos-signin-1\config'
Assert-True ($cmd.Contains('start "" wt.exe -d "%~dp0." cmd /c signin.cmd here')) 'window: reopens in Windows Terminal'
Assert-True ($cmd.Contains("cd /d `"%USERPROFILE%`"`r`n")) 'window: runs from the home folder'
Assert-True ($cmd.Contains('set "CLAUDE_CONFIG_DIR=%TEMP%\claudeos-signin-1\config"')) 'window: throwaway settings folder'
Assert-True ($cmd.Contains('"%LOCALAPPDATA%\ClaudeOS\setup\claude\claude.exe" auth login --claudeai')) 'window: claude auth login'
Assert-True ($cmd.Contains('if exist "%TEMP%\claudeos-signin-1\config\.credentials.json" (')) 'window: says whether it worked'
Assert-True (-not ($cmd -replace "`r`n", '').Contains("`n") -and $cmd -notmatch '[^\x00-\x7F]') 'window: ASCII, Windows line endings'
# Paths in it go through the variables, so a user folder with other letters works.
$saved = @{}
foreach ($v in 'TEMP', 'LOCALAPPDATA', 'USERPROFILE') { $saved[$v] = [Environment]::GetEnvironmentVariable($v) }
[Environment]::SetEnvironmentVariable('TEMP', 'C:\Users\Ren Shu\AppData\Local\Temp')
[Environment]::SetEnvironmentVariable('LOCALAPPDATA', 'C:\Users\Ren Shu\AppData\Local')
[Environment]::SetEnvironmentVariable('USERPROFILE', 'C:\Users\Ren Shu')
Assert-Equal '%TEMP%\claudeos-signin-1\config' (ConvertTo-CmdPath 'C:\Users\Ren Shu\AppData\Local\Temp\claudeos-signin-1\config') 'cmd path: temp'
Assert-Equal '%LOCALAPPDATA%\ClaudeOS\setup\claude\claude.exe' (ConvertTo-CmdPath 'c:\users\ren shu\appdata\local\ClaudeOS\setup\claude\claude.exe') 'cmd path: local app data, any case'
Assert-Equal 'D:\Tools\claude.exe' (ConvertTo-CmdPath 'D:\Tools\claude.exe') 'cmd path: elsewhere unchanged'
Assert-Equal '%USERPROFILE%\TempX\a' (ConvertTo-CmdPath 'C:\Users\Ren Shu\TempX\a') 'cmd path: only whole folder names'
foreach ($v in @($saved.Keys)) { [Environment]::SetEnvironmentVariable($v, $saved[$v]) }

# Waiting for the sign-in: the saved sign-in is returned; none within the time: $null.
$signDir = Join-Path ([IO.Path]::GetTempPath()) ('signin-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $signDir 'config') -Force | Out-Null
$credFile = Join-Path (Join-Path $signDir 'config') '.credentials.json'
$fullLogin = '{"claudeAiOauth":{"accessToken":"at","refreshToken":"rt","expiresAt":1}}'
Assert-True ($null -eq (Wait-ClaudeSignIn -File $credFile -Seconds 1 -PollMs 100)) 'wait: nothing saved, nothing returned'
[IO.File]::WriteAllText($credFile, '{"claudeAiOauth":{"accessT')       # still being written
Assert-True ($null -eq (Wait-ClaudeSignIn -File $credFile -Seconds 1 -PollMs 100)) 'wait: half a file is not a sign-in'
[IO.File]::WriteAllText($credFile, $fullLogin)
Assert-Equal $fullLogin (Wait-ClaudeSignIn -File $credFile -Seconds 5 -PollMs 100) 'wait: the saved sign-in'
Remove-Item -LiteralPath $signDir -Recurse -Force

# The whole sign-in, with the window standing in: one approval, nothing pasted; if the window can't open,
# the person picks "Paste a sign-in code instead".
$savedTemp = $env:TEMP
$env:TEMP = [IO.Path]::GetTempPath().TrimEnd('\', '/')
$origStart = ${function:Start-SignInWindow}
$origChoice = ${function:Read-Choice}
$Script:Opened = @()
${function:Start-SignInWindow} = {
    param($ClaudeExe, $Folder)
    $Script:Opened += $Folder
    [IO.File]::WriteAllText((Join-Path (Join-Path $Folder 'config') '.credentials.json'), '{"claudeAiOauth":{"accessToken":"at","refreshToken":"rt"}}')
    return $true
}
$r = Invoke-ClaudeBrowserSignIn -ClaudeExe 'C:\x\claude.exe' -WaitSeconds 5
Assert-True ((Test-ClaudeLogin $r.Login) -and $null -eq $r.Token) 'sign-in: the full sign-in, no code'
Assert-True ($Script:Opened.Count -eq 1 -and -not (Test-Path -LiteralPath (Join-Path $Script:Opened[0] 'config'))) 'sign-in: one window; the sign-in it saved is deleted'
Remove-Item -LiteralPath $Script:Opened[0] -Recurse -Force
${function:Start-SignInWindow} = { param($ClaudeExe, $Folder) return $false }
${function:Read-Choice} = { param($Question, $Options, $Default) return 1 }
$Script:Pasted.Enqueue($tok)
$r = Invoke-ClaudeBrowserSignIn -ClaudeExe 'C:\x\claude.exe' -WaitSeconds 1
Assert-True ($null -eq $r.Login -and $r.Token -eq $tok) 'sign-in: no window, a pasted code instead'
${function:Read-Choice} = { param($Question, $Options, $Default) return 2 }
Assert-Throws { Invoke-ClaudeBrowserSignIn -ClaudeExe 'C:\x\claude.exe' -WaitSeconds 1 } 'sign-in: stopping stops setup'
${function:Start-SignInWindow} = $origStart
${function:Read-Choice} = $origChoice
${function:Read-PastedSecret} = $origPasted
$env:TEMP = $savedTemp

# The Claude program comes out of the checked download into a folder of that download's own: an earlier setup's
# program (another version) is neither run nor kept.
$cp = Join-Path ([IO.Path]::GetTempPath()) ('claudeos-cp-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $cp 'claude') -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $cp 'claude/claude.exe'), 'an older version')
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$fakeWheel = Join-Path $cp 'fake.whl'
$zip = [IO.Compression.ZipFile]::Open($fakeWheel, [IO.Compression.ZipArchiveMode]::Create)
try {
    $w = New-Object IO.StreamWriter(($zip.CreateEntry('claude_agent_sdk/_bundled/claude.exe')).Open())
    try { $w.Write('this version') } finally { $w.Dispose() }
} finally {
    $zip.Dispose()
}
$savedWheel = $Script:ClaudeWheel
$Script:ClaudeWheel = [pscustomobject]@{ Name = 'fake.whl'; Url = 'https://example.invalid/fake.whl'; Sha256 = (Get-FileSha256 $fakeWheel)
    Exe = 'claude_agent_sdk/_bundled/claude.exe' }
$exe = Get-ClaudeProgram -WorkDir $cp
Assert-Equal 'this version' ([IO.File]::ReadAllText($exe)) 'claude program: the one in this download, not an earlier one'
Assert-True (-not (Test-Path -LiteralPath (Join-Path $cp 'claude')) -and -not (Test-Path -LiteralPath ($exe + '.part'))) 'claude program: the earlier one deleted, nothing half-written'
Assert-Equal $exe (Get-ClaudeProgram -WorkDir $cp) 'claude program: taken out once'
$Script:ClaudeWheel = $savedWheel
Remove-Folder $cp

# ------------------------------------------------------------------ folders, and short (8.3) folder names
# Remove-Folder deletes a folder with everything in it, quietly; a missing folder is fine.
$rf = Join-Path ([IO.Path]::GetTempPath()) ('claudeos-rf-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $rf 'a/b') -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $rf 'a/b/x.json'), '{}')
$out = @(Remove-Folder $rf)
Assert-True (-not (Test-Path -LiteralPath $rf) -and $out.Count -eq 0) 'remove folder: gone, and nothing written to the output'
Remove-Folder $rf
Assert-True (-not (Test-Path -LiteralPath $rf)) 'remove folder: a missing folder is fine'
# Sign-in folders an earlier, stopped run left behind are deleted; other folders stay.
$tr = Join-Path ([IO.Path]::GetTempPath()) ('claudeos-tr-' + [guid]::NewGuid().ToString('N'))
foreach ($d in 'claudeos-signin-1/config', 'claudeos-signin-2', 'keep-me') { New-Item -ItemType Directory -Path (Join-Path $tr $d) -Force | Out-Null }
[IO.File]::WriteAllText((Join-Path $tr 'claudeos-signin-1/config/.credentials.json'), '{}')
Remove-OldSignInFolders -Root $tr
Assert-True (-not (Test-Path -LiteralPath (Join-Path $tr 'claudeos-signin-1')) -and -not (Test-Path -LiteralPath (Join-Path $tr 'claudeos-signin-2')) -and
             (Test-Path -LiteralPath (Join-Path $tr 'keep-me'))) 'old sign-in folders: deleted, others kept'
Remove-Folder $tr
Assert-Equal 'C:\Users\Ren Shu\x' (Get-LongPath 'C:\Users\Ren Shu\x') 'long path: no short names, unchanged'
Assert-True ([string]::IsNullOrEmpty((Get-LongPath $null))) 'long path: nothing in, nothing out'
if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
    # The PC that needed this: user folder "Ren Shu", so %TEMP% is C:\Users\RENSHU~1\AppData\Local\Temp, and setup
    # runs in a folder on C:. A folder with a space gets a short name like that (where the drive makes them).
    $base = Get-LongPath ([Environment]::GetFolderPath('LocalApplicationData'))    # on C:, like the user folder
    $long = Join-Path $base ('ClaudeOS short ' + [guid]::NewGuid().ToString('N').Substring(0, 8))
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
            # The sign-in, start to finish, with %TEMP% spelled the short way: setup carries on.
            $env:TEMP = $short
            ${function:Start-SignInWindow} = {
                param($ClaudeExe, $Folder)
                $Script:Opened += $Folder
                [IO.File]::WriteAllText((Join-Path (Join-Path $Folder 'config') '.credentials.json'), '{"claudeAiOauth":{"accessToken":"at","refreshToken":"rt"}}')
                return $true
            }
            $Script:Opened = @()
            $r = $null
            try { $r = Invoke-ClaudeBrowserSignIn -ClaudeExe 'C:\x\claude.exe' -WaitSeconds 5 } catch { Write-Host ('short path: sign-in failed: ' + $_.Exception.Message) }
            Assert-True ($null -ne $r -and (Test-ClaudeLogin $r.Login)) 'short path: the sign-in comes through'
            Assert-True ($Script:Opened.Count -eq 1 -and -not [IO.Directory]::Exists((Join-Path $Script:Opened[0] 'config'))) 'short path: the sign-in it saved is deleted'
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
            ${function:Start-SignInWindow} = $origStart
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

# Only real answers reach the log, so a stray piece of a code typed at a yes/no question never does.
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
Assert-True (-not (Protect-SetupText ('x ' + $tok + ' y')).Contains('Ab9_')) 'token hidden in logs'
# The full sign-in (Claude Code's .credentials.json).
Assert-True (Test-ClaudeLogin '{"claudeAiOauth":{"accessToken":"abc","refreshToken":"def","expiresAt":1}}') 'full sign-in accepted'
Assert-True (-not (Test-ClaudeLogin '{"claudeAiOauth":{"accessToken":"abc"}}')) 'sign-in without refresh token rejected'
Assert-True (-not (Test-ClaudeLogin '{"claudeAiOauth":{"accessToken":"abc","refreshToken":5}}')) 'non-text token rejected'
Assert-True (-not (Test-ClaudeLogin '{"other":1}')) 'other json rejected'
Assert-True (-not (Test-ClaudeLogin 'not json')) 'junk sign-in rejected'
Assert-True (-not (Test-ClaudeLogin '')) 'empty sign-in rejected'

# ------------------------------------------------------------------ time zones and settings
Assert-Equal 'Asia/Singapore' (Get-IanaTimeZone 'Singapore Standard Time') 'singapore'
Assert-Equal 'Europe/London' (Get-IanaTimeZone 'GMT Standard Time') 'london'
# The current names: Ubuntu 24.04 has no Asia/Calcutta or Europe/Kiev (they're in tzdata-legacy, not installed).
Assert-Equal 'Asia/Kolkata' (Get-IanaTimeZone 'India Standard Time') 'india'
Assert-Equal 'Europe/Kyiv' (Get-IanaTimeZone 'FLE Standard Time') 'kyiv'
Assert-Equal 'Etc/UTC' (Get-IanaTimeZone 'Nowhere Standard Time') 'unknown zone'
$settings = New-SettingsJson -UserName 'Ren "R" Shu' -WindowsUser 'rensh' -TimeZone 'Asia/Singapore'
$parsed = $settings | ConvertFrom-Json
Assert-Equal 'Ren "R" Shu' $parsed.user_name 'settings json user'
Assert-Equal 'rensh' $parsed.windows_user 'settings json windows user'

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
# Up to 24.04.3 the installer made the account only at the first start, too late for ClaudeOS's last install
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
Assert-Equal 'Install ClaudeOS' $installEntry.Groups[1].Value 'first install entry name'
Assert-True ($installEntry.Groups[2].Value -match 'hwe-vmlinuz autoinstall subiquity-storage-version=2\s+---') 'first install entry: newer kernel, autoinstall and storage v2 before ---'
Assert-True ($grubOut.Contains('menuentry "Install ClaudeOS (older kernel)"')) 'second entry'
Assert-True ($grubOut.Contains("fwsetup")) 'firmware entry kept'
Assert-True (-not $grubOut.Contains('Try or Install Ubuntu Server')) 'old entry replaced'
# If the installer starts again after installing, it starts the installed ClaudeOS instead of installing again.
Assert-True ($grubOut.Contains('if search --no-floppy --file --set=claudeos_esp /EFI/claudeos/installed-0123456789abcdef01234567; then')) 'checks for its own install'
$startAt = $grubOut.IndexOf('menuentry "Start ClaudeOS')
Assert-True ($startAt -ge 0 -and $startAt -lt $grubOut.IndexOf('menuentry "Install ClaudeOS"')) 'start entry comes first'
Assert-True ($grubOut.Contains("`tconfigfile /EFI/ubuntu/grub.cfg`n}")) 'reads the installed boot menu'
Assert-True (-not $grubOut.Contains('chainloader /EFI/ubuntu')) 'no chainloading (fails under Secure Boot)'
Assert-True ($grubOut -match '(?s)set default=claudeos-start\nelse\nmenuentry "Install ClaudeOS" \{.*\n\}\nfi\n') 'install entries only when not installed'
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
Assert-Equal 1 $ub.Count 'bcdedit: one ClaudeOS entry'
Assert-Equal '{9f62d3e2-5c1a-11ef-a1b2-806e6f6e6963}' $ub[0].Id 'bcdedit: its identifier'
Assert-Equal '\EFI\Microsoft\Boot\bootmgfw.efi' ($entries | Where-Object { $_.Id -eq '{bootmgr}' }).Path 'bcdedit: Windows Boot Manager path'
# German Windows: the labels are translated, the values are not.
$bcdGerman = ($bcdEnglish -replace 'identifier  ', 'Bezeichner  ' -replace 'description ', 'Beschreibung' -replace 'path    ', 'Pfad    ' -replace 'Firmware Application', 'Firmwareanwendung') -replace "`n", "`r`n"
$de = ConvertFrom-BcdFirmware $bcdGerman
$deUb = @($de | Where-Object { $_.Path -match '(?i)^\\EFI\\ubuntu\\' })
Assert-Equal '{9f62d3e2-5c1a-11ef-a1b2-806e6f6e6963}' $deUb[0].Id 'bcdedit: German output'
Assert-Equal 0 (ConvertFrom-BcdFirmware '').Count 'bcdedit: empty output'

$markerDir = Join-Path ([IO.Path]::GetTempPath()) ('claudeos-marker-' + [guid]::NewGuid())
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
$yaml = New-AutoinstallYaml -Template $template -Account 'rensh' -RealName ('Ren Shu "' + [char]0x00E9 + '"') -PasswordHash $hash `
    -TimeZone 'Asia/Singapore' -InstallId '0123456789abcdef01234567' `
    -WifiSsid 'Home "5G"' -WifiPassword 'p@ss w0rd"\' -WifiAuth 'WPA2PSK'
Assert-True (-not ($yaml -match '\{\{')) 'all fields filled'
Assert-True ($yaml.Contains('mode: use_gap')) 'free space only'
Assert-True ($yaml -match '(?m)^\s+match:\s*\n\s+install-media: true\s*$') 'the disk the installer is on'
Assert-True (-not $yaml.Contains('reformat_disk')) 'never reformats'
Assert-Throws { New-AutoinstallYaml -Template $template -Account 'Bad Name' -PasswordHash $hash -TimeZone 'Etc/UTC' -InstallId '0123456789abcdef' } 'bad user name refused'
Assert-Throws { New-AutoinstallYaml -Template 'nothing' -Account 'rensh' -PasswordHash $hash -TimeZone 'Etc/UTC' -InstallId '0123456789abcdef' } 'template without fields refused'
$wiredOnly = New-AutoinstallYaml -Template $template -Account 'rensh' -PasswordHash $hash -TimeZone 'Etc/UTC' -InstallId '0123456789abcdef'
# A name as a work PC may have it: the account is made with useradd, which refuses a colon.
$work = New-AutoinstallYaml -Template $template -Account 'rensh' -RealName ("Shu, Ren (IT: Ops)`t= x") -PasswordHash $hash -TimeZone 'Etc/UTC' -InstallId '0123456789abcdef'
Assert-True ($work.Contains('realname: "Shu Ren (IT Ops) x"')) 'name without : , = or tabs, as the installer''s form takes it'
Assert-True ((New-AutoinstallYaml -Template $template -Account 'rensh' -RealName ('a' * 200) -PasswordHash $hash -TimeZone 'Etc/UTC' -InstallId '0123456789abcdef').Contains(('realname: "' + ('a' * 160) + '"'))) 'name at most 160 characters'
# A name and a Wi-Fi network with characters past the first 65,536 (checked by the Python tests).
$wide = [char]::ConvertFromUtf32(0x20BB7) + [char]::ConvertFromUtf32(0x1F600)
$wideYaml = New-AutoinstallYaml -Template $template -Account 'rensh' -RealName ('Ren ' + $wide) -PasswordHash $hash `
    -TimeZone 'Asia/Singapore' -InstallId '0123456789abcdef01234567' -WifiSsid ([char]::ConvertFromUtf32(0x1F3E0) + ' Home') -WifiPassword 'pw'
$wideSettings = New-SettingsJson -UserName ('Ren ' + $wide) -WindowsUser 'rensh' -TimeZone 'Asia/Singapore'

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
Assert-Equal '/copy {bootmgr} /d ClaudeOS setup' $Script:BcdCalls[0] 'setup entry: a copy of Windows Boot Manager''s'
Assert-Equal ('/set ' + $Script:BcdNewId + ' path \EFI\claudeos-setup\shimx64.efi') $Script:BcdCalls[1] 'setup entry: starts the installer''s boot loader'
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
# Removing: every entry that starts \EFI\claudeos-setup\, and nothing when there is none.
$Script:Bcd = $bcdEnglish
$Script:BcdCalls.Clear()
Remove-SetupBootEntries
Assert-Equal 0 $Script:BcdCalls.Count 'no setup entry: nothing deleted'
$Script:Bcd = $bcdEnglish + (New-BcdBlock '{aaaaaaaa-0000-4000-8000-000000000001}' '\EFI\claudeos-setup\shimx64.efi') +
    (New-BcdBlock '{aaaaaaaa-0000-4000-8000-000000000002}' '\EFI\CLAUDEOS-SETUP\shimx64.efi')
Remove-SetupBootEntries
Assert-Equal '/delete {aaaaaaaa-0000-4000-8000-000000000001}|/delete {aaaaaaaa-0000-4000-8000-000000000002}' ($Script:BcdCalls -join '|') 'two setup entries: both deleted, one at a time'
Assert-Equal 1 (Get-ClaudeOSBootEntries).Count 'ClaudeOS''s own entry is not a setup entry'
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
Assert-True ($Script:PartCalls -contains 'Format-Volume FAT32 CLAUDEOS') 'installer partition: FAT32, labelled CLAUDEOS'
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
$tmpInst = Join-Path ([IO.Path]::GetTempPath()) ('claudeos-installer-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $tmpInst 'inst/EFI/boot'), (Join-Path $tmpInst 'esp/EFI/Microsoft') -Force | Out-Null
foreach ($f in @('bootx64.efi', 'grubx64.efi', 'mmx64.efi')) { [IO.File]::WriteAllText((Join-Path $tmpInst ('inst/EFI/boot/' + $f)), $f) }
Write-InstallerMark -Root (Join-Path $tmpInst 'inst') -InstallId '0123456789abcdef01234567'
Assert-Equal "id=0123456789abcdef01234567`n" ([IO.File]::ReadAllText((Join-Path $tmpInst 'inst/claudeos/installer-0123456789abcdef01234567'))) 'installer mark'
Assert-Throws { Write-InstallerMark -Root (Join-Path $tmpInst 'inst') -InstallId '../../x' } 'installer mark: bad id refused'
Install-SetupLoader -InstallerRoot (Join-Path $tmpInst 'inst') -EspRoot (Join-Path $tmpInst 'esp')
Assert-Equal 'bootx64.efi' ([IO.File]::ReadAllText((Join-Path $tmpInst 'esp/EFI/claudeos-setup/shimx64.efi'))) 'boot loader: shim, under the name its GRUB expects'
Assert-Equal 'grubx64.efi' ([IO.File]::ReadAllText((Join-Path $tmpInst 'esp/EFI/claudeos-setup/grubx64.efi'))) 'boot loader: GRUB next to it'
Assert-True (Test-Path -LiteralPath (Join-Path $tmpInst 'esp/EFI/claudeos-setup/mmx64.efi')) 'boot loader: MokManager too'
Remove-Item -LiteralPath (Join-Path $tmpInst 'inst/EFI/boot/grubx64.efi')
Assert-Throws { Install-SetupLoader -InstallerRoot (Join-Path $tmpInst 'inst') -EspRoot (Join-Path $tmpInst 'esp') } 'boot loader: missing files are an error'
Remove-Item -LiteralPath $tmpInst -Recurse -Force

# ------------------------------------------------------------------ ClaudeOS's own BitLocker key
$tmpState = Join-Path ([IO.Path]::GetTempPath()) ('claudeos-state-' + [guid]::NewGuid().ToString('N'))
$env:CLAUDEOS_STATE_DIR = $tmpState
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
$k = New-ClaudeOSBitLockerKey
Assert-Equal '{2b000000-0000-4000-8000-0000000000c1}' $k.Id 'BitLocker key: its id'
Assert-Equal $Script:BlPassword $k.Password 'BitLocker key: its recovery password'
Assert-Equal "{2b000000-0000-4000-8000-0000000000c1}`n" ([IO.File]::ReadAllText((Join-Path $tmpState 'bitlocker-key-id'))) 'BitLocker key: id recorded'
# Only ever deletes a recovery password: if the recorded id names another kind of key, nothing is deleted.
$Script:BlType = 1
[void]$Script:BlCalls.Clear()
Assert-Throws { Remove-ClaudeOSBitLockerKey } 'BitLocker key: refuses to delete a key that is not a recovery password'
Assert-True ($Script:BlCalls -notcontains 'DeleteKeyProtector') 'BitLocker key: nothing deleted then'
$Script:BlType = 3
Assert-True (Remove-ClaudeOSBitLockerKey) 'BitLocker key: deleted'
Assert-True (-not (Test-Path -LiteralPath (Join-Path $tmpState 'bitlocker-key-id'))) 'BitLocker key: record gone'
Assert-True (-not (Remove-ClaudeOSBitLockerKey)) 'BitLocker key: nothing to delete without a record'
# A key BitLocker shows in an unexpected form is not kept: it's deleted again and setup says so.
$Script:BlPassword = 'not-a-key'
[void]$Script:BlCalls.Clear()
Assert-Throws { New-ClaudeOSBitLockerKey } 'BitLocker key: an unexpected form is an error'
Assert-True ($Script:BlCalls -contains 'DeleteKeyProtector') 'BitLocker key: the half-made key is deleted'
Remove-Item -LiteralPath $tmpState -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item Function:\Get-CimInstance, Function:\Invoke-CimMethod
$env:CLAUDEOS_STATE_DIR = $null

# ------------------------------------------------------------------ ClaudeOS's own system
$repo = Split-Path -Parent $Win
$ownRelease = [IO.Path]::Combine($repo, 'os', 'release', 'claudeos.efi')
Assert-Equal $ownRelease (Get-OwnRelease -RepoRoot $repo) 'own system: the release file, with the right checksum'
Assert-True (-not (Test-PeSigned $ownRelease)) 'own system: the release file is not signed yet'
$ownTmp = [IO.Path]::Combine([IO.Path]::GetTempPath(), 'claudeos-own-test-' + [guid]::NewGuid().ToString('N'))
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
    Copy-Item -LiteralPath $ownRelease -Destination (Join-Path $fakeRelease 'claudeos.efi')
    [IO.File]::WriteAllText((Join-Path $fakeRelease 'claudeos.efi.sha256'), ('0' * 64) + '  claudeos.efi')
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
Assert-Equal '/copy {bootmgr} /d ClaudeOS (own system)' $Script:BcdCalls[0] 'own entry: a copy of Windows Boot Manager''s'
Assert-Equal ('/set ' + $Script:BcdNewId + ' path \EFI\claudeos-own\claudeos.efi') $Script:BcdCalls[1] 'own entry: starts ClaudeOS''s own system'
Assert-Equal ('/set {fwbootmgr} displayorder ' + $Script:BcdNewId + ' /addlast') $Script:BcdCalls[2] 'own entry: last in the menu'
$Script:Bcd = $bcdEnglish + (New-BcdBlock '{bbbbbbbb-0000-4000-8000-000000000001}' '\EFI\ClaudeOS-Own\claudeos.efi')
$Script:BcdCalls.Clear()
Assert-Equal '{bbbbbbbb-0000-4000-8000-000000000001}' (Get-OwnBootEntry) 'own entry: the one already there'
Assert-Equal 0 $Script:BcdCalls.Count 'own entry: nothing added when it is there'
Assert-Equal 0 (Get-SetupBootEntries).Count 'own entry: not the installer''s entry'
Assert-Equal 1 (Get-ClaudeOSBootEntries).Count 'own entry: not Linux ClaudeOS''s entry'
Remove-Item Function:\bcdedit.exe

# ------------------------------------------------------------------ files from a disc image
# On a real PC, grub.cfg copied from the mounted Ubuntu image kept the image's read-only mark, and setup
# couldn't patch it ("Access to the path 'S:\boot\grub\grub.cfg' is denied"). tests/win_ci/iso.ps1 checks
# the real thing on Windows; these check the parts.
$roDir = Join-Path ([IO.Path]::GetTempPath()) ('claudeos-ro-' + [guid]::NewGuid().ToString('N'))
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
