# Getting the official Ubuntu Server 24.04 LTS installer and checking it is genuine.
# Chat Computer runs on top of Ubuntu's drivers for Wi-Fi, Bluetooth, sound and the screen.
# ASCII-only file (see Common.ps1).

$Script:UbuntuMirrors = @('https://releases.ubuntu.com/24.04/', 'https://cdimage.ubuntu.com/releases/24.04/release/')

# 24.04.4 or newer: from there on the installer makes the person's account before its last step, which is
# Chat Computer's (finish.sh hands Chat Computer's folder to that account). Up to 24.04.3 the account was only made at
# the first start, so that step failed and the install stopped half done.
$Script:MinPointRelease = 4

# Fingerprints of releases checked when this setup was made. A file with one of these names must
# match exactly; newer point releases are checked against Ubuntu's published SHA256SUMS.
$Script:KnownUbuntuIsos = @{
    'ubuntu-24.04.5-live-server-amd64.iso' = '97f3d7ffb032c3eb3b23d2c8be9cc76e60c2c1f2c0146ba5ba9fe01cafae0fd8'
    'ubuntu-24.04.4-live-server-amd64.iso' = 'e907d92eeec9df64163a7e454cbc8d7755e8ddc7ed42f99dbc80c40f1a138433'
}

function ConvertFrom-Sha256Sums {
    # "hash *name" lines -> @{ name = hash }
    param([Parameter(Mandatory = $true)][string]$Text)
    $map = @{}
    foreach ($line in ($Text -split "`r?`n")) {
        $m = [regex]::Match($line, '^([0-9a-fA-F]{64})\s+\*?(\S+)\s*$')
        if ($m.Success) { $map[$m.Groups[2].Value] = $m.Groups[1].Value.ToLowerInvariant() }
    }
    return $map
}

function Get-PointRelease {
    # "ubuntu-24.04.5-live-server-amd64.iso" -> 5 ; "ubuntu-24.04-live-server-amd64.iso" -> 0
    param([string]$Name)
    $m = [regex]::Match($Name, '^ubuntu-24\.04(?:\.(\d+))?-live-server-amd64\.iso$')
    if (-not $m.Success) { return -1 }
    if ($m.Groups[1].Success) { return [int]$m.Groups[1].Value }
    return 0
}

function Select-ServerIso {
    # The newest 24.04 server installer listed in SHA256SUMS.
    param([Parameter(Mandatory = $true)][hashtable]$Sums)
    $best = $null
    $bestN = -1
    foreach ($name in $Sums.Keys) {
        $n = Get-PointRelease $name
        if ($n -gt $bestN) { $best = $name; $bestN = $n }
    }
    if (-not $best) { throw 'No Ubuntu 24.04 server installer is listed.' }
    if ($bestN -lt $Script:MinPointRelease) {
        throw ('Ubuntu''s download site offers ' + $best + ', but Chat Computer needs 24.04.' + $Script:MinPointRelease + ' or newer.')
    }
    $hash = $Sums[$best]
    if ($Script:KnownUbuntuIsos.ContainsKey($best) -and $Script:KnownUbuntuIsos[$best] -ne $hash) {
        throw ('The published fingerprint for ' + $best + ' does not match the known one. Stopping to be safe.')
    }
    return [pscustomobject]@{ Name = $best; Sha256 = $hash }
}

function Get-UbuntuIso {
    # Returns the path of a checked installer image, downloading it (about 4 GB) if needed.
    param([Parameter(Mandatory = $true)][string]$WorkDir, [string]$IsoPath)
    if ($IsoPath) {
        $name = Split-Path -Leaf $IsoPath
        $n = Get-PointRelease $name
        if ($n -ge 0 -and $n -lt $Script:MinPointRelease) {
            $version = '24.04'
            if ($n -gt 0) { $version += '.' + $n }
            throw ('That is the Ubuntu ' + $version + ' installer; Chat Computer needs 24.04.' + $Script:MinPointRelease + ' or newer. ' +
                   'Run setup without -IsoPath and it downloads the current one.')
        }
        Write-Line ('Checking ' + $name + '...')
        $hash = Get-FileSha256 $IsoPath
        if ($Script:KnownUbuntuIsos.ContainsKey($name) -and $Script:KnownUbuntuIsos[$name] -eq $hash) { return $IsoPath }
        if ($Script:KnownUbuntuIsos.Values -contains $hash) { return $IsoPath }
        throw ('That file is not a known Ubuntu 24.04 server installer (fingerprint ' + $hash + ').')
    }
    $sums = $null
    $base = $null
    foreach ($m in $Script:UbuntuMirrors) {
        try {
            [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
            $resp = Invoke-WebRequest -Uri ($m + 'SHA256SUMS') -UseBasicParsing -ErrorAction Stop
            $content = $resp.Content
            if ($content -is [byte[]]) { $content = [Text.Encoding]::ASCII.GetString($content) }
            $sums = ConvertFrom-Sha256Sums ([string]$content)
            $base = $m
            break
        } catch {
            Write-SetupLog ('Could not read ' + $m + 'SHA256SUMS: ' + $_.Exception.Message)
        }
    }
    if (-not $sums) { throw 'Could not reach Ubuntu''s download site. Check the internet connection and try again.' }
    $iso = Select-ServerIso $sums
    $dest = Join-Path $WorkDir $iso.Name
    if ((Test-Path -LiteralPath $dest) -and (Get-FileSha256 $dest) -eq $iso.Sha256) {
        Write-Good ('Already downloaded and checked: ' + $iso.Name)
        return $dest
    }
    Write-Line ('Downloading ' + $iso.Name + ' from Ubuntu (about 4 GB; this can take a while)...')
    Invoke-Download -Url ($base + $iso.Name) -Destination $dest -Description 'Ubuntu installer'
    Write-Line 'Checking the download...'
    $hash = Get-FileSha256 $dest
    if ($hash -ne $iso.Sha256) {
        Remove-Item -LiteralPath $dest -Force -ErrorAction SilentlyContinue
        throw 'The Ubuntu download was damaged or changed (fingerprint mismatch). It was deleted; please run setup again.'
    }
    Write-Good 'Ubuntu installer downloaded and checked.'
    return $dest
}
