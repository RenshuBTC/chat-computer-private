# Signing in to Claude with the person's subscription.
#
# Uses the official Claude Code program, taken from Anthropic's claude-agent-sdk package on PyPI
# (the same version ClaudeOS runs on Linux). The download is checked against a fixed SHA-256, then
# "claude setup-token" opens the browser to sign in and prints a long-lived token, which the person
# pastes back here. The token is never shown, logged or sent anywhere except onto the installer partition.
# For the Claude app on the phone (Claude Code's Remote Control), that token isn't enough: it needs a
# full sign-in, which "claude auth login" saves as .credentials.json in a throwaway settings folder.
# That file goes onto the installer partition the same way, and is never shown or logged either.
# ASCII-only file (see Common.ps1).

$Script:ClaudeWheel = [pscustomobject]@{
    Name   = 'claude_agent_sdk-0.2.167-py3-none-win_amd64.whl'
    Url    = 'https://files.pythonhosted.org/packages/06/1d/ef41f5a385ad12ebb93427c847b10b6d00510e2be9a94e52fc2269aab704/claude_agent_sdk-0.2.167-py3-none-win_amd64.whl'
    Sha256 = '139597261e6d7329c3a99fbadcbb6e7835f40bda77fa7d34db53888beef777b8'
    Exe    = 'claude_agent_sdk/_bundled/claude.exe'
}

function Test-ClaudeToken {
    # A Claude subscription token from "claude setup-token" (sk-ant-oat01-...) or an API key.
    param([AllowNull()][string]$Token)
    if ([string]::IsNullOrEmpty($Token)) { return $false }
    return ($Token -cmatch '^sk-ant-(oat01|api03)-[A-Za-z0-9_\-]{20,}$')
}

function ConvertTo-CleanToken {
    # People paste with spaces, line breaks (long tokens wrap in the console) or quotes.
    param([AllowNull()][string]$Text)
    if ($null -eq $Text) { return '' }
    $t = [regex]::Replace($Text, '[\s"''`]', '')
    $m = [regex]::Match($t, 'sk-ant-(oat01|api03)-[A-Za-z0-9_\-]+')
    if ($m.Success) { return $m.Value }
    return $t
}

function Invoke-Download {
    # Downloads a file, preferring BITS (resumable, shows progress), falling back to .NET.
    param([Parameter(Mandatory = $true)][string]$Url, [Parameter(Mandatory = $true)][string]$Destination,
          [string]$Description = 'Downloading')
    $partial = $Destination + '.part'
    $ok = $false
    try {
        Import-Module BitsTransfer -ErrorAction Stop
        Start-BitsTransfer -Source $Url -Destination $partial -Description $Description -ErrorAction Stop
        $ok = $true
    } catch {
        Write-SetupLog ('BITS download failed, trying again another way: ' + $_.Exception.Message)
    }
    if (-not $ok) {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        $wc = New-Object System.Net.WebClient
        try { $wc.DownloadFile($Url, $partial) } finally { $wc.Dispose() }
    }
    Move-Item -LiteralPath $partial -Destination $Destination -Force
}

function Get-FileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-ClaudeProgram {
    # Returns the path of claude.exe, downloading and checking it if needed.
    param([Parameter(Mandatory = $true)][string]$WorkDir)
    $wheel = Join-Path $WorkDir $Script:ClaudeWheel.Name
    $exeDir = Join-Path $WorkDir 'claude'
    $exe = Join-Path $exeDir 'claude.exe'
    if (-not (Test-Path -LiteralPath $wheel) -or (Get-FileSha256 $wheel) -ne $Script:ClaudeWheel.Sha256) {
        Write-Line 'Downloading the official Claude program (about 110 MB)...'
        Invoke-Download -Url $Script:ClaudeWheel.Url -Destination $wheel -Description 'Claude program'
    }
    $hash = Get-FileSha256 $wheel
    if ($hash -ne $Script:ClaudeWheel.Sha256) {
        Remove-Item -LiteralPath $wheel -Force -ErrorAction SilentlyContinue
        throw ('The Claude download did not match its known fingerprint (got ' + $hash + '). Nothing was run.')
    }
    if (-not (Test-Path -LiteralPath $exe)) {
        New-Item -ItemType Directory -Path $exeDir -Force | Out-Null
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $zip = [IO.Compression.ZipFile]::OpenRead($wheel)
        try {
            $entry = $zip.Entries | Where-Object { $_.FullName -eq $Script:ClaudeWheel.Exe } | Select-Object -First 1
            if (-not $entry) { throw 'claude.exe was not found inside the download.' }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $exe, $true)
        } finally {
            $zip.Dispose()
        }
    }
    return $exe
}

function Invoke-ClaudeSignIn {
    # Runs "claude setup-token" in this window, then asks for the token it printed.
    param([Parameter(Mandatory = $true)][string]$ClaudeExe)
    $config = Join-Path ([IO.Path]::GetTempPath()) ('claudeos-signin-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $config -Force | Out-Null
    $oldConfig = $env:CLAUDE_CONFIG_DIR
    try {
        # A throwaway settings folder, so any Claude Code you already use is left alone.
        $env:CLAUDE_CONFIG_DIR = $config
        Write-Line ''
        Write-Line 'Your browser will open to sign in to Claude. Use the account with your Claude subscription.'
        Write-Line 'When it finishes, this window shows a long code starting with sk-ant-. It may run over two'
        Write-Line 'lines: copy all of it (select it with the mouse and press Enter, or Ctrl+C).'
        Write-Line ''
        & $ClaudeExe setup-token
        Write-SetupLog ('claude setup-token exit code ' + $LASTEXITCODE)
    } finally {
        $env:CLAUDE_CONFIG_DIR = $oldConfig
        Remove-Item -LiteralPath $config -Recurse -Force -ErrorAction SilentlyContinue
    }
    for ($try = 1; $try -le 3; $try++) {
        $raw = Read-PastedSecret 'Paste the code here (it stays hidden), then press Enter'
        $token = ConvertTo-CleanToken $raw
        if (Test-ClaudeToken $token) {
            Write-Good 'Got it. Claude is signed in for ClaudeOS.'
            return $token
        }
        Write-Warn 'That does not look like the code. It starts with sk-ant- and is about 100 characters long.'
    }
    throw 'No valid Claude sign-in code was entered.'
}

function Test-ClaudeLogin {
    # The contents of Claude Code's .credentials.json after "claude auth login": a claude.ai sign-in.
    param([AllowNull()][string]$Json)
    if ([string]::IsNullOrWhiteSpace($Json)) { return $false }
    try { $data = $Json | ConvertFrom-Json -ErrorAction Stop } catch { return $false }
    if ($null -eq $data -or $null -eq $data.PSObject.Properties['claudeAiOauth']) { return $false }
    $oauth = $data.claudeAiOauth
    if ($null -eq $oauth) { return $false }
    foreach ($key in @('accessToken', 'refreshToken')) {
        $prop = $oauth.PSObject.Properties[$key]
        if ($null -eq $prop -or -not ($prop.Value -is [string]) -or [string]::IsNullOrEmpty($prop.Value)) { return $false }
    }
    return $true
}

function Invoke-ClaudeLogin {
    # Runs "claude auth login" in a throwaway settings folder and returns the full sign-in it saved, which
    # the Claude app on the phone needs to reach ClaudeOS. Returns $null if it didn't work.
    param([Parameter(Mandatory = $true)][string]$ClaudeExe)
    for ($try = 1; $try -le 2; $try++) {
        $config = Join-Path ([IO.Path]::GetTempPath()) ('claudeos-login-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $config -Force | Out-Null
        $oldConfig = $env:CLAUDE_CONFIG_DIR
        $json = $null
        try {
            $env:CLAUDE_CONFIG_DIR = $config
            Write-Line ''
            Write-Line 'Your browser opens once more. Approve it with the same Claude account.'
            Write-Line 'If the browser shows a code instead, paste it here when asked.'
            Write-Line ''
            & $ClaudeExe auth login --claudeai
            Write-SetupLog ('claude auth login exit code ' + $LASTEXITCODE)
            $file = Join-Path $config '.credentials.json'
            if (Test-Path -LiteralPath $file) { $json = [IO.File]::ReadAllText($file) }
        } finally {
            $env:CLAUDE_CONFIG_DIR = $oldConfig
            Remove-Item -LiteralPath $config -Recurse -Force -ErrorAction SilentlyContinue
        }
        if (Test-ClaudeLogin $json) {
            Write-Good 'Done. You can type to ClaudeOS in the Claude app on your phone.'
            return $json
        }
        Write-Warn 'That sign-in did not finish.'
        if ($try -lt 2 -and -not (Read-YesNo 'Try again?' $true)) { break }
    }
    Write-Warn 'Skipping the phone. ClaudeOS works without it; to add it later, run ClaudeOS Setup again and choose "Sign ClaudeOS in to Claude again".'
    return $null
}
