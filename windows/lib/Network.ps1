# The installer needs the internet (to download Chat Computer's parts). This finds out whether a cable is
# plugged in and which Wi-Fi network Windows is using, so the installer can join the same one.
# ASCII-only file (see Common.ps1).

function ConvertFrom-WlanProfileXml {
    # Reads a profile exported by "netsh wlan export profile key=clear".
    # Returns SSID, authentication, password (if any) and whether the installer can use it.
    param([Parameter(Mandatory = $true)][string]$Xml)
    $doc = New-Object System.Xml.XmlDocument
    $doc.LoadXml($Xml)
    $ns = New-Object System.Xml.XmlNamespaceManager($doc.NameTable)
    $ns.AddNamespace('w', 'http://www.microsoft.com/networking/WLAN/profile/v1')
    $ssidNode = $doc.SelectSingleNode('/w:WLANProfile/w:SSIDConfig/w:SSID/w:name', $ns)
    $authNode = $doc.SelectSingleNode('/w:WLANProfile/w:MSM/w:security/w:authEncryption/w:authentication', $ns)
    $keyNode = $doc.SelectSingleNode('/w:WLANProfile/w:MSM/w:security/w:sharedKey/w:keyMaterial', $ns)
    $protNode = $doc.SelectSingleNode('/w:WLANProfile/w:MSM/w:security/w:sharedKey/w:protected', $ns)
    $ssid = $null
    if ($ssidNode) { $ssid = $ssidNode.InnerText }
    $auth = 'open'
    if ($authNode) { $auth = $authNode.InnerText }
    $key = $null
    if ($keyNode -and -not ($protNode -and $protNode.InnerText -eq 'true')) { $key = $keyNode.InnerText }
    # Personal (password) networks work; company networks that sign in with a user name don't.
    $usable = $auth -in @('open', 'WPAPSK', 'WPA2PSK', 'WPA3SAE')
    if ($auth -ne 'open' -and [string]::IsNullOrEmpty($key)) { $usable = $false }
    return [pscustomobject]@{ Ssid = $ssid; Authentication = $auth; Password = $key; Usable = $usable }
}

function Get-ConnectedSsid {
    # The network name Windows is connected to right now, if any.
    $out = & netsh.exe wlan show interfaces 2>$null
    foreach ($line in @($out)) {
        if ($line -match '^\s*SSID\s*:\s*(.+?)\s*$') { return $Matches[1] }
    }
    try {
        $p = Get-NetConnectionProfile -ErrorAction Stop | Where-Object { ([string](Get-Prop $_ 'InterfaceAlias')) -match 'Wi-?Fi|WLAN|Wireless' } | Select-Object -First 1
        if ($p) { return [string]$p.Name }
    } catch {
        $null = $_
    }
    return $null
}

function Get-WifiForInstaller {
    # Exports Windows' saved Wi-Fi profiles to a private temp folder, finds the connected network,
    # reads its password, and deletes the export. Returns $null if there's no usable Wi-Fi.
    $ssid = Get-ConnectedSsid
    if (-not $ssid) { return $null }
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('chatcomputer-wifi-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp -Force | Out-Null
    try {
        & netsh.exe wlan export profile key=clear folder="$tmp" 2>$null | Out-Null
        foreach ($f in @(Get-ChildItem -LiteralPath $tmp -Filter '*.xml' -ErrorAction SilentlyContinue)) {
            try {
                $info = ConvertFrom-WlanProfileXml ([IO.File]::ReadAllText($f.FullName))
                if ($info.Ssid -eq $ssid) { return $info }
            } catch {
                Write-SetupLog ('Could not read a Wi-Fi profile: ' + $_.Exception.Message)
            }
        }
        return [pscustomobject]@{ Ssid = $ssid; Authentication = 'unknown'; Password = $null; Usable = $false }
    } finally {
        Remove-Folder $tmp    # it holds Wi-Fi passwords in plain text
    }
}

function Test-WiredConnection {
    try {
        $up = @(Get-NetAdapter -Physical -ErrorAction Stop | Where-Object {
                $_.Status -eq 'Up' -and ([string](Get-Prop $_ 'PhysicalMediaType')) -match '802\.3'
            })
        return ($up.Count -gt 0)
    } catch {
        return $false
    }
}
