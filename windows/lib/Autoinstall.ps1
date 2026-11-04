# Builds the answers for the Ubuntu installer (autoinstall.yaml) from what setup learned on Windows.
# Everything here is plain computation so it can be tested anywhere PowerShell runs.
# ASCII-only file (see Common.ps1).

$Script:ReservedLinuxNames = @(
    'root', 'daemon', 'bin', 'sys', 'sync', 'games', 'man', 'lp', 'mail', 'news', 'uucp', 'proxy', 'www-data',
    'backup', 'list', 'irc', 'gnats', 'nobody', 'adm', 'tty', 'disk', 'kmem', 'dialout', 'fax', 'voice', 'cdrom',
    'floppy', 'tape', 'sudo', 'audio', 'dip', 'operator', 'src', 'shadow', 'utmp', 'video', 'sasl', 'plugdev',
    'staff', 'users', 'nogroup', 'netplan', 'ftn', 'mysql', 'tac-plus', 'alias', 'qmail', 'qmaild', 'qmails',
    'qmailr', 'qmailq', 'qmaill', 'qmailp', 'asterisk', 'vpopmail', 'vchkpw', 'slurm', 'hacluster', 'haclient',
    'ceph', 'opensrf', 'libvirt-qemu', 'admin', 'bind', 'crontab', 'cupsys', 'dhcp', 'dnsmasq', 'ftp', 'fuse',
    'gdm', 'input', 'kvm', 'lpadmin', 'messagebus', 'netdev', 'render', 'scanner', 'sgx', 'ssh', 'sshd',
    'ssl-cert', 'syslog', 'bluetooth', 'pulse', 'pipewire', 'chatcomputer', 'systemd-network', 'systemd-resolve',
    'systemd-timesync', 'polkitd', 'tcpdump', 'tss', 'uuidd', 'usbmux', 'rtkit', 'avahi', 'colord', 'geoclue')

function ConvertTo-LinuxUserName {
    # A valid Ubuntu user name ([a-z_][a-z0-9_-]*, at most 32 characters, not reserved) from a
    # Windows user name such as "Sam Lee" or "sammy".
    param([AllowNull()][AllowEmptyString()][string]$WindowsName)
    if ($null -eq $WindowsName) { $WindowsName = '' }
    $decomposed = $WindowsName.Normalize([Text.NormalizationForm]::FormD)
    $sb = New-Object System.Text.StringBuilder
    foreach ($ch in $decomposed.ToCharArray()) {
        $cat = [Globalization.CharUnicodeInfo]::GetUnicodeCategory($ch)
        if ($cat -ne [Globalization.UnicodeCategory]::NonSpacingMark) { [void]$sb.Append($ch) }
    }
    $name = $sb.ToString().ToLowerInvariant()
    $name = [regex]::Replace($name, '[^a-z0-9_-]', '')
    $name = $name.TrimStart('-')
    if ($name -eq '') { $name = 'me' }
    if ($name -match '^[0-9]') { $name = 'u' + $name }
    if ($name.Length -gt 30) { $name = $name.Substring(0, 30) }
    $candidate = $name
    $n = 1
    while ($Script:ReservedLinuxNames -contains $candidate) {
        $n++
        $candidate = $name + $n
    }
    return $candidate
}

function Test-LinuxUserName {
    param([string]$Name)
    if ([string]::IsNullOrEmpty($Name)) { return $false }
    if ($Name.Length -gt 32) { return $false }
    if ($Script:ReservedLinuxNames -contains $Name) { return $false }
    return ($Name -cmatch '^[a-z_][a-z0-9_-]*$')
}

# ------------------------------------------------------------------ SHA-512 crypt ($6$)
# The password format Ubuntu uses in /etc/shadow (glibc "SHA-crypt" by Ulrich Drepper).
$Script:CryptAlphabet = './0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz'

function Get-Sha512Digest {
    param([byte[][]]$Parts)
    $sha = [System.Security.Cryptography.SHA512]::Create()
    try {
        foreach ($p in $Parts) {
            if ($p.Length -gt 0) { [void]$sha.TransformBlock($p, 0, $p.Length, $null, 0) }
        }
        [void]$sha.TransformFinalBlock((New-Object byte[] 0), 0, 0)
        return , $sha.Hash
    } finally {
        $sha.Dispose()
    }
}

function Get-RepeatedBytes {
    # The first $Length bytes of $Block repeated.
    param([byte[]]$Block, [int]$Length)
    $out = New-Object byte[] $Length
    for ($i = 0; $i -lt $Length; $i++) { $out[$i] = $Block[$i % $Block.Length] }
    return , $out
}

function Get-Sha512Crypt {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Password,
        [string]$Salt
    )
    if ([string]::IsNullOrEmpty($Salt)) { $Salt = New-CryptSalt }
    if ($Salt.Length -gt 16) { $Salt = $Salt.Substring(0, 16) }
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    $pw = $utf8.GetBytes($Password)
    $s = $utf8.GetBytes($Salt)
    $pl = $pw.Length

    $b = Get-Sha512Digest @($pw, $s, $pw)

    $parts = New-Object System.Collections.Generic.List[byte[]]
    $parts.Add($pw); $parts.Add($s)
    $cnt = $pl
    while ($cnt -gt 64) { $parts.Add($b); $cnt -= 64 }
    $parts.Add((Get-RepeatedBytes $b $cnt))
    $cnt = $pl
    while ($cnt -gt 0) {
        if (($cnt -band 1) -ne 0) { $parts.Add($b) } else { $parts.Add($pw) }
        $cnt = $cnt -shr 1
    }
    $a = Get-Sha512Digest $parts.ToArray()

    $dpParts = New-Object System.Collections.Generic.List[byte[]]
    for ($i = 0; $i -lt $pl; $i++) { $dpParts.Add($pw) }
    $dp = Get-Sha512Digest $dpParts.ToArray()
    $p = Get-RepeatedBytes $dp $pl

    $dsParts = New-Object System.Collections.Generic.List[byte[]]
    for ($i = 0; $i -lt (16 + [int]$a[0]); $i++) { $dsParts.Add($s) }
    $ds = Get-Sha512Digest $dsParts.ToArray()
    $sBytes = Get-RepeatedBytes $ds $s.Length

    $c = $a
    for ($r = 0; $r -lt 5000; $r++) {
        $round = New-Object System.Collections.Generic.List[byte[]]
        if (($r -band 1) -ne 0) { $round.Add($p) } else { $round.Add($c) }
        if (($r % 3) -ne 0) { $round.Add($sBytes) }
        if (($r % 7) -ne 0) { $round.Add($p) }
        if (($r -band 1) -ne 0) { $round.Add($c) } else { $round.Add($p) }
        $c = Get-Sha512Digest $round.ToArray()
    }

    $order = @(
        @(0, 21, 42), @(22, 43, 1), @(44, 2, 23), @(3, 24, 45), @(25, 46, 4), @(47, 5, 26), @(6, 27, 48),
        @(28, 49, 7), @(50, 8, 29), @(9, 30, 51), @(31, 52, 10), @(53, 11, 32), @(12, 33, 54), @(34, 55, 13),
        @(56, 14, 35), @(15, 36, 57), @(37, 58, 16), @(59, 17, 38), @(18, 39, 60), @(40, 61, 19), @(62, 20, 41))
    $sb = New-Object System.Text.StringBuilder
    foreach ($g in $order) {
        $w = ([int]$c[$g[0]] -shl 16) -bor ([int]$c[$g[1]] -shl 8) -bor [int]$c[$g[2]]
        for ($k = 0; $k -lt 4; $k++) { [void]$sb.Append($Script:CryptAlphabet[$w -band 0x3f]); $w = $w -shr 6 }
    }
    $w = [int]$c[63]
    for ($k = 0; $k -lt 2; $k++) { [void]$sb.Append($Script:CryptAlphabet[$w -band 0x3f]); $w = $w -shr 6 }
    return ('$6$' + $Salt + '$' + $sb.ToString())
}

function Get-RandomBytes {
    param([int]$Count)
    $bytes = New-Object byte[] $Count
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    return , $bytes
}

function New-CryptSalt {
    $bytes = Get-RandomBytes 16
    $sb = New-Object System.Text.StringBuilder
    foreach ($x in $bytes) { [void]$sb.Append($Script:CryptAlphabet[$x % 64]) }
    return $sb.ToString()
}

function New-RecoveryPassword {
    # Four groups of four easy-to-read characters (about 80 bits), e.g. "kq7m-3vtz-p9xa-hc4r".
    $alphabet = 'abcdefghjkmnpqrstuvwxyz23456789'
    $groups = @()
    for ($g = 0; $g -lt 4; $g++) {
        $sb = New-Object System.Text.StringBuilder
        foreach ($x in (Get-RandomBytes 4)) {
            # 248 is the largest multiple of 31 below 256; retrying above it would remove bias,
            # the tiny bias left here does not matter for a one-off recovery password.
            [void]$sb.Append($alphabet[$x % $alphabet.Length])
        }
        $groups += $sb.ToString()
    }
    return ($groups -join '-')
}

function New-InstallId {
    $bytes = Get-RandomBytes 12
    return (($bytes | ForEach-Object { $_.ToString('x2') }) -join '')
}

# ------------------------------------------------------------------ the answers file
function ConvertTo-YamlBlock {
    # Indents a YAML fragment so it sits under a key at the given depth.
    param([string]$Yaml, [int]$Indent)
    $pad = ' ' * $Indent
    $lines = ($Yaml -replace "`r`n", "`n").TrimEnd("`n").Split("`n")
    return (($lines | ForEach-Object { if ($_ -eq '') { '' } else { $pad + $_ } }) -join "`n")
}

function New-NetworkYaml {
    # Netplan for the installer: any wired port by DHCP, plus the Wi-Fi network if one is given.
    param([string]$WifiSsid, [string]$WifiPassword, [string]$WifiAuth = 'WPA2PSK')
    $y = "version: 2`n" +
         "ethernets:`n" +
         "  any-wired:`n" +
         "    match: {name: " + (ConvertTo-YamlString 'en*') + "}`n" +
         "    dhcp4: true`n" +
         "    optional: true`n" +
         "  any-wired-eth:`n" +
         "    match: {name: " + (ConvertTo-YamlString 'eth*') + "}`n" +
         "    dhcp4: true`n" +
         "    optional: true`n"
    if (-not [string]::IsNullOrEmpty($WifiSsid)) {
        $ap = '{}'
        if (-not [string]::IsNullOrEmpty($WifiPassword)) {
            if ($WifiAuth -eq 'WPA3SAE') {
                $ap = '{auth: {key-management: sae, password: ' + (ConvertTo-YamlString $WifiPassword) + '}}'
            } else {
                $ap = '{password: ' + (ConvertTo-YamlString $WifiPassword) + '}'
            }
        }
        $y += "wifis:`n" +
              "  any-wifi:`n" +
              "    match: {name: " + (ConvertTo-YamlString 'wl*') + "}`n" +
              "    dhcp4: true`n" +
              "    optional: true`n" +
              "    access-points:`n" +
              "      " + (ConvertTo-YamlString $WifiSsid) + ": " + $ap + "`n"
    }
    return $y
}

function New-AutoinstallYaml {
    param(
        [Parameter(Mandatory = $true)][string]$Template,
        [Parameter(Mandatory = $true)][string]$Account,
        [string]$RealName = '',
        [Parameter(Mandatory = $true)][string]$PasswordHash,
        [Parameter(Mandatory = $true)][string]$TimeZone,
        [Parameter(Mandatory = $true)][string]$InstallId,
        [string]$WifiSsid = '',
        [string]$WifiPassword = '',
        [string]$WifiAuth = 'WPA2PSK'
    )
    if (-not (Test-LinuxUserName $Account)) { throw ('Not a valid Linux user name: ' + $Account) }
    if ($InstallId -notmatch '^[0-9a-f]{8,64}$') { throw 'Bad install id' }
    if ($PasswordHash -notmatch '^\$6\$[./0-9A-Za-z]{1,16}\$[./0-9A-Za-z]{86}$') { throw 'Bad password hash' }
    # The name as the installer's own form would take it: without : , or = (the account is made with useradd,
    # which refuses a colon) and at most 160 characters. "Lee, Sam" becomes "Lee Sam".
    $name = ([regex]::Replace([string]$RealName, '[:,=\p{Cc}]', ' ') -replace '\s+', ' ').Trim()
    if ($name.Length -gt 160) { $name = $name.Substring(0, 160).TrimEnd() }
    $values = @{
        '{{USERNAME}}'      = (ConvertTo-YamlString $Account)
        '{{REALNAME}}'      = (ConvertTo-YamlString $name)
        '{{PASSWORD_HASH}}' = (ConvertTo-YamlString $PasswordHash)
        '{{TIMEZONE}}'      = (ConvertTo-YamlString $TimeZone)
        '{{INSTALL_ID}}'    = (ConvertTo-YamlString $InstallId)
        '{{NETWORK}}'       = (ConvertTo-YamlBlock (New-NetworkYaml -WifiSsid $WifiSsid -WifiPassword $WifiPassword -WifiAuth $WifiAuth) 4)
    }
    $text = $Template -replace "`r`n", "`n"
    foreach ($k in $values.Keys) {
        if (-not $text.Contains($k)) { throw ('Template is missing ' + $k) }
        $text = $text.Replace($k, $values[$k])
    }
    if ($text -match '\{\{[A-Z_]+\}\}') { throw ('Template has an unfilled field: ' + $Matches[0]) }
    return $text
}

function Get-IanaTimeZone {
    param([string]$WindowsId)
    if ($WindowsId -and $Script:WindowsToIana.ContainsKey($WindowsId)) { return $Script:WindowsToIana[$WindowsId] }
    return 'Etc/UTC'
}

function New-SettingsJson {
    # ~/ChatComputer/settings.json on the new system (keys in ARCHITECTURE.md).
    param([string]$UserName, [string]$WindowsUser, [string]$TimeZone, [string]$Locale = 'en_US.UTF-8')
    $pairs = @(
        ('  "user_name": ' + (ConvertTo-YamlString $UserName)),
        ('  "windows_user": ' + (ConvertTo-YamlString $WindowsUser)),
        ('  "timezone": ' + (ConvertTo-YamlString $TimeZone)),
        ('  "locale": ' + (ConvertTo-YamlString $Locale)))
    return ("{`n" + ($pairs -join ",`n") + "`n}`n")
}
