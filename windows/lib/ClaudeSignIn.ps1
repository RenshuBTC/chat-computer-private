# Signing in to Claude with the person's subscription.
#
# Uses the official Claude Code program, taken from Anthropic's claude-agent-sdk package on PyPI
# (the same version ClaudeOS runs on Linux). The download is checked against a fixed SHA-256.
#
# The sign-in is one approval in the browser, with nothing to copy: "claude auth login" runs in a window of
# its own and saves a full sign-in (.credentials.json) in a throwaway settings folder, which setup picks up.
# That window is opened as the person, without administrator rights: run from setup's administrator window,
# the program can hang without showing anything (seen on a real PC, where the same program worked in an
# ordinary window). The full sign-in is also what the Claude app on the phone needs (Remote Control), and
# ClaudeOS uses it for everything. If it doesn't come through, a one-year sign-in code made with "claude
# setup-token" can be pasted instead. Sign-ins are never shown, logged or sent anywhere except onto the
# installer partition (or, for "Sign in again", the person's own ClaudeOS folder).
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

function ConvertTo-CmdPath {
    # A path as a .cmd file should spell it: through %LOCALAPPDATA%, %TEMP% or %USERPROFILE% when it is inside
    # one of them, so a user folder with letters outside ASCII still works (a .cmd file is read in the console's
    # code page).
    param([Parameter(Mandatory = $true)][string]$Path)
    foreach ($name in @('TEMP', 'LOCALAPPDATA', 'USERPROFILE')) {
        $base = [Environment]::GetEnvironmentVariable($name)
        if (-not $base) { continue }
        $base = $base.TrimEnd('\')
        if ($Path.Length -gt $base.Length -and $Path.Substring(0, $base.Length + 1) -ieq ($base + '\')) {
            return ('%' + $name + '%' + $Path.Substring($base.Length))
        }
    }
    return $Path
}

function New-SignInWindowScript {
    # The .cmd that runs the sign-in in its own window. When Windows Terminal is there, the window reopens itself
    # in it (where Claude's sign-in program is known to work); it runs from the person's home folder, with a
    # throwaway settings folder, so any Claude Code they already use is left alone. Paths are given as a .cmd
    # file spells them (ConvertTo-CmdPath).
    param([Parameter(Mandatory = $true)][string]$ClaudeExe, [Parameter(Mandatory = $true)][string]$ConfigDir)
    $lines = @(
        '@echo off',
        'rem ClaudeOS Setup''s sign-in window (opened without administrator rights).',
        'title Sign in to Claude',
        'if not "%~1"=="here" (',
        '  where wt.exe >nul 2>nul && (',
        '    start "" wt.exe -d "%~dp0." cmd /c signin.cmd here',
        '    exit /b 0',
        '  )',
        ')',
        'cd /d "%USERPROFILE%"',
        ('set "CLAUDE_CONFIG_DIR=' + $ConfigDir + '"'),
        'echo Your browser opens on Claude''s sign-in page. Sign in with the account that has your Claude',
        'echo subscription, and approve. ClaudeOS Setup carries on by itself.',
        'echo If the browser shows a code instead, paste it here: Ctrl+V or a right-click.',
        'echo.',
        ('"' + $ClaudeExe + '" auth login --claudeai'),
        ('if exist "' + $ConfigDir + '\.credentials.json" ('),
        '  echo.',
        '  echo Signed in. This window closes in a moment.',
        '  timeout /t 4 >nul',
        ') else (',
        '  echo.',
        '  echo The sign-in did not finish. Close this window: ClaudeOS Setup offers to try again.',
        '  pause',
        ')'
    )
    return (($lines -join "`r`n") + "`r`n")
}

function Start-SignInWindow {
    # Writes the sign-in window's .cmd into $Folder and opens it as the person signed in to Windows, without
    # administrator rights: File Explorer opens it, and File Explorer runs without them. $true if it was opened.
    param([Parameter(Mandatory = $true)][string]$ClaudeExe, [Parameter(Mandatory = $true)][string]$Folder)
    $script = Join-Path $Folder 'signin.cmd'
    $text = New-SignInWindowScript -ClaudeExe (ConvertTo-CmdPath $ClaudeExe) -ConfigDir (ConvertTo-CmdPath (Join-Path $Folder 'config'))
    [IO.File]::WriteAllText($script, $text, [Text.Encoding]::ASCII)
    try {
        Start-Process -FilePath (Join-Path $env:WINDIR 'explorer.exe') -ArgumentList ('"' + $script + '"') -ErrorAction Stop
        return $true
    } catch {
        Write-SetupLog ('Opening the sign-in window: ' + $_.Exception.Message)
        return $false
    }
}

function Wait-ClaudeSignIn {
    # Waits until the sign-in window has saved a full sign-in in $File, and returns it. Enter, or $Seconds
    # passing, stops the wait ($null).
    param([Parameter(Mandatory = $true)][string]$File, [int]$Seconds = 900, [int]$PollMs = 500)
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        if (Test-Path -LiteralPath $File) {
            $text = $null
            try { $text = [IO.File]::ReadAllText($File) } catch { $text = $null }    # still being written
            if (Test-ClaudeLogin $text) { return $text }
        }
        if (Test-InputWaiting) {
            try { $key = [Console]::ReadKey($true) } catch { $key = $null }
            if ($key -and $key.Key -eq [ConsoleKey]::Enter) { return $null }
        }
        Start-Sleep -Milliseconds $PollMs
    }
    return $null
}

function Read-SignInCode {
    # A one-year sign-in code, made with "claude setup-token" in an ordinary window and pasted here. $null if
    # the person pressed Enter (or gave up after three tries).
    param([Parameter(Mandatory = $true)][string]$ClaudeExe)
    Write-Line ''
    Write-Line 'To make a sign-in code: open PowerShell (not as administrator), run the line below and sign in. It shows'
    Write-Line 'a long code that starts with sk-ant-: copy it, then paste it here with a RIGHT-CLICK (choose Paste if a'
    Write-Line 'menu opens). It stays hidden; then press Enter.'
    Write-Line ('  & "' + $ClaudeExe + '" setup-token')
    for ($try = 1; $try -le 3; $try++) {
        $raw = Read-PastedSecret 'Your code (right-click to paste), or Enter to go back'
        if ([string]::IsNullOrWhiteSpace($raw)) { return $null }
        $token = ConvertTo-CleanToken $raw
        if (Test-ClaudeToken $token) {
            Write-Good 'Got it. Claude is signed in for ClaudeOS.'
            return $token
        }
        Write-Warn 'That does not look like a sign-in code (it starts with sk-ant- and is about 100 characters long).'
    }
    return $null
}

function Invoke-ClaudeBrowserSignIn {
    # Signs in to Claude: the sign-in window, then the browser, then setup picks the sign-in up by itself.
    # Returns @{ Login = the full sign-in (or $null); Token = a pasted sign-in code (or $null) }: one of them.
    # The sign-in window's settings folder (the sign-in itself) is deleted as soon as setup has it; its .cmd
    # stays in %TEMP% (nothing secret in it), because the window may still be reading it.
    param([Parameter(Mandatory = $true)][string]$ClaudeExe, [int]$WaitSeconds = 900)
    while ($true) {
        $folder = Join-Path $env:TEMP ('claudeos-signin-' + [guid]::NewGuid().ToString('N'))
        $config = Join-Path $folder 'config'
        New-Item -ItemType Directory -Path $config -Force | Out-Null
        $login = $null
        try {
            Write-Line ''
            Write-Line 'A window called "Sign in to Claude" opens, and then your browser on Claude''s sign-in page.'
            Write-Line 'Sign in with the account that has your Claude subscription, and approve. Setup carries on by itself.'
            Write-Line '(Press Enter here to stop waiting.)'
            if (Start-SignInWindow -ClaudeExe $ClaudeExe -Folder $folder) {
                $login = Wait-ClaudeSignIn -File (Join-Path $config '.credentials.json') -Seconds $WaitSeconds
            } else {
                Write-Warn 'Setup could not open the sign-in window.'
            }
            if ($login) {
                Write-Good 'Signed in to Claude. You can also type to ClaudeOS from the Claude app on your phone.'
                return [pscustomobject]@{ Login = $login; Token = $null }
            }
        } finally {
            Remove-Item -LiteralPath $config -Recurse -Force -ErrorAction SilentlyContinue
        }
        Write-Warn 'The sign-in did not come through.'
        $pick = Read-Choice 'What would you like to do?' @('Try signing in again', 'Paste a sign-in code instead', 'Stop setup') 0
        if ($pick -eq 2) { throw 'Stopped at your request. No disks were changed.' }
        if ($pick -eq 1) {
            $code = Read-SignInCode -ClaudeExe $ClaudeExe
            if ($code) { return [pscustomobject]@{ Login = $null; Token = $code } }
        }
    }
}
