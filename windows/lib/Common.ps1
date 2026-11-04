# Shared helpers for Chat Computer Setup: talking to the person, asking questions, and a log that
# never contains secrets. Works in Windows PowerShell 5.1 and PowerShell 7.
# Keep this file ASCII-only: Windows PowerShell 5.1 reads files without a BOM as ANSI.

$Script:SetupLogFile = $null

function Initialize-SetupLog {
    param([Parameter(Mandatory = $true)][string]$Path)
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    $Script:SetupLogFile = $Path
    Write-SetupLog ('Chat Computer Setup started ' + (Get-Date -Format 's'))
}

function Protect-SetupText {
    # Removes anything that looks like an API key (sk-ant-...) from text before it is logged or shown.
    param([AllowNull()][string]$Text)
    if ($null -eq $Text) { return '' }
    return [regex]::Replace($Text, 'sk-ant-[A-Za-z0-9_\-]+', 'sk-ant-(hidden)')
}

function Write-SetupLog {
    param([AllowNull()][string]$Message)
    if (-not $Script:SetupLogFile) { return }
    try {
        $line = (Get-Date -Format 'HH:mm:ss') + '  ' + (Protect-SetupText $Message)
        Add-Content -LiteralPath $Script:SetupLogFile -Value $line -Encoding UTF8
    } catch {
        # Logging must never stop the setup.
        $null = $_
    }
}

function Write-Line {
    param([AllowEmptyString()][string]$Text = '')
    Write-Host $Text
    Write-SetupLog $Text
}

function Write-Step {
    param([Parameter(Mandatory = $true)][string]$Text)
    Write-Host ''
    Write-Host $Text -ForegroundColor Cyan
    Write-SetupLog ('== ' + $Text)
}

function Write-Warn {
    param([Parameter(Mandatory = $true)][string]$Text)
    Write-Host $Text -ForegroundColor Yellow
    Write-SetupLog ('WARNING ' + $Text)
}

function Write-Good {
    param([Parameter(Mandatory = $true)][string]$Text)
    Write-Host $Text -ForegroundColor Green
    Write-SetupLog ('OK ' + $Text)
}

function Read-YesNo {
    param(
        [Parameter(Mandatory = $true)][string]$Question,
        [bool]$Default = $true
    )
    $suffix = ' [y/N]'
    if ($Default) { $suffix = ' [Y/n]' }
    while ($true) {
        $answer = Read-Host ($Question + $suffix)
        $a = ([string]$answer).Trim().ToLowerInvariant()
        # Only real answers are logged: anything else could be part of something secret typed in the wrong place.
        if ($a -in @('', 'y', 'yes', 'n', 'no')) { Write-SetupLog ('Q: ' + $Question + ' -> ' + $a) }
        else { Write-SetupLog ('Q: ' + $Question + ' -> (not y or n)') }
        if ($a -eq '') { return $Default }
        if ($a -eq 'y' -or $a -eq 'yes') { return $true }
        if ($a -eq 'n' -or $a -eq 'no') { return $false }
        Write-Host 'Please type y for yes or n for no.'
    }
}

function Read-Choice {
    # Shows numbered options and returns the index (0-based) of the one picked.
    param(
        [Parameter(Mandatory = $true)][string]$Question,
        [Parameter(Mandatory = $true)][string[]]$Options,
        [int]$Default = 0
    )
    Write-Host ''
    Write-Host $Question
    for ($i = 0; $i -lt $Options.Count; $i++) {
        Write-Host ('  ' + ($i + 1) + '. ' + $Options[$i])
    }
    while ($true) {
        $answer = Read-Host ('Type a number (Enter = ' + ($Default + 1) + ')')
        if ([string]::IsNullOrWhiteSpace($answer)) { Write-SetupLog ('Q: ' + $Question + ' -> (Enter)'); return $Default }
        $n = 0
        if ([int]::TryParse($answer.Trim(), [ref]$n) -and $n -ge 1 -and $n -le $Options.Count) {
            Write-SetupLog ('Q: ' + $Question + ' -> ' + $n)
            return ($n - 1)
        }
        Write-SetupLog ('Q: ' + $Question + ' -> (not a number on the list)')
        Write-Host ('Please type a number from 1 to ' + $Options.Count + '.')
    }
}

function Get-Prop {
    # Reads a property that may not exist on this version of Windows (safe under StrictMode).
    param($Object, [Parameter(Mandatory = $true)][string]$Name)
    if ($null -eq $Object) { return $null }
    $p = $Object.PSObject.Properties[$Name]
    if ($null -eq $p) { return $null }
    return $p.Value
}

function Format-Size {
    param([double]$Bytes)
    if ($Bytes -ge 1TB) { return ('{0:N1} TB' -f ($Bytes / 1TB)) }
    if ($Bytes -ge 1GB) { return ('{0:N0} GB' -f ($Bytes / 1GB)) }
    return ('{0:N0} MB' -f ($Bytes / 1MB))
}

function Write-TextFileLf {
    # Writes UTF-8 without a byte-order mark and with Linux line endings. The installer, GRUB and
    # the shell scripts on the installer partition all expect exactly that.
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text
    )
    $t = $Text -replace "`r`n", "`n"
    $enc = New-Object System.Text.UTF8Encoding($false)
    if ([System.IO.File]::Exists($Path)) {
        # A file copied from a disc image (the Ubuntu installer's grub.cfg) can still carry the disc's
        # read-only mark, and Windows then refuses to write it.
        $attrs = [System.IO.File]::GetAttributes($Path)
        if (($attrs -band [System.IO.FileAttributes]::ReadOnly) -ne 0) {
            [System.IO.File]::SetAttributes($Path, ($attrs -band (-bnot [System.IO.FileAttributes]::ReadOnly)))
        }
    }
    [System.IO.File]::WriteAllText($Path, $t, $enc)
}

function Get-ErrorText {
    # What went wrong, in the error's own words. A .NET method that failed is reported by PowerShell as
    # 'Exception calling "WriteAllText" with "3" argument(s): ...'; the error inside says it plainly.
    param([Parameter(Mandatory = $true)]$ErrorRecord)
    $e = $ErrorRecord
    if ($e -is [System.Management.Automation.ErrorRecord]) { $e = $e.Exception }
    while ($e -is [System.Management.Automation.MethodException] -and $null -ne $e.InnerException) { $e = $e.InnerException }
    return [string]$e.Message
}

function ConvertTo-YamlString {
    # A double-quoted YAML scalar (the JSON subset), safe for any text.
    param([AllowNull()][AllowEmptyString()][string]$Value)
    if ($null -eq $Value) { $Value = '' }
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append('"')
    $chars = $Value.ToCharArray()
    for ($i = 0; $i -lt $chars.Length; $i++) {
        $ch = $chars[$i]
        $code = [int]$ch
        switch ($code) {
            34 { [void]$sb.Append('\"') }
            92 { [void]$sb.Append('\\') }
            9 { [void]$sb.Append('\t') }
            10 { [void]$sb.Append('\n') }
            13 { [void]$sb.Append('\r') }
            default {
                $next = 0
                if ($i + 1 -lt $chars.Length) { $next = [int]$chars[$i + 1] }
                if ($code -ge 0xD800 -and $code -le 0xDBFF -and $next -ge 0xDC00 -and $next -le 0xDFFF) {
                    # A character past the first 65,536 (an emoji, a rarer Chinese character) is a pair of halves
                    # here. It stays as it is: escaped as two halves, YAML reads two broken characters (and the
                    # installer's YAML reader refuses them), where JSON would read one.
                    [void]$sb.Append($ch)
                    [void]$sb.Append($chars[$i + 1])
                    $i++
                } elseif ($code -ge 0xD800 -and $code -le 0xDFFF) {
                    [void]$sb.Append('\ufffd')      # half a pair on its own is no character at all
                } elseif ($code -lt 32 -or $code -eq 127 -or ($code -ge 0x80 -and $code -le 0x9F) -or $code -eq 0x2028 -or
                          $code -eq 0x2029 -or $code -ge 0xFFFE) {
                    # Control characters, and the ones YAML would take as a line break or refuse.
                    [void]$sb.Append(('\u{0:x4}' -f $code))
                } else {
                    [void]$sb.Append($ch)
                }
            }
        }
    }
    [void]$sb.Append('"')
    return $sb.ToString()
}

function Disable-ConsoleQuickEdit {
    # Turns off "QuickEdit" in this console window: with it on, a click inside the window starts selecting text
    # and pauses whatever runs in it (the title then begins with "Select") until a key is pressed. People click
    # into the window without knowing that. A right-click still pastes. Windows only; harmless elsewhere.
    try {
        if (-not ('ChatComputerConsoleMode' -as [type])) {
            Add-Type -Namespace '' -Name 'ChatComputerConsoleMode' -MemberDefinition @'
[DllImport("kernel32.dll", SetLastError = true)] public static extern System.IntPtr GetStdHandle(int nStdHandle);
[DllImport("kernel32.dll", SetLastError = true)] public static extern bool GetConsoleMode(System.IntPtr h, out uint mode);
[DllImport("kernel32.dll", SetLastError = true)] public static extern bool SetConsoleMode(System.IntPtr h, uint mode);
'@ -ErrorAction Stop
        }
        $h = [ChatComputerConsoleMode]::GetStdHandle(-10)    # the console's input
        $mode = [uint32]0
        if (-not [ChatComputerConsoleMode]::GetConsoleMode($h, [ref]$mode)) { return $false }
        # ENABLE_EXTENDED_FLAGS (0x80) on, so the change sticks; ENABLE_QUICK_EDIT_MODE (0x40) off.
        $new = ($mode -bor 0x80) -band (-bnot [uint32]0x40)
        return [ChatComputerConsoleMode]::SetConsoleMode($h, [uint32]$new)
    } catch {
        Write-SetupLog ('QuickEdit: ' + $_.Exception.Message)
        return $false
    }
}

function Get-LongPath {
    # The full spelling of a path that may contain short (8.3) folder names: C:\Users\Sam Lee for
    # C:\Users\SAMLEE~1. Windows spells %TEMP% the short way when the user folder's name has a space in it, and
    # some of Windows PowerShell 5.1's commands fail on such paths (see Remove-Folder). The path as given when
    # it has no short names, doesn't exist, or can't be expanded.
    param([AllowNull()][AllowEmptyString()][string]$Path)
    if ([string]::IsNullOrEmpty($Path) -or $Path.IndexOf('~') -lt 0) { return $Path }
    try {
        if (-not ('ChatComputerLongPath' -as [type])) {
            Add-Type -Namespace '' -Name 'ChatComputerLongPath' -MemberDefinition @'
[DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
public static extern uint GetLongPathName(string shortPath, System.Text.StringBuilder longPath, uint size);
'@ -ErrorAction Stop
        }
        $sb = New-Object System.Text.StringBuilder 32768
        $n = [ChatComputerLongPath]::GetLongPathName($Path, $sb, [uint32]$sb.Capacity)
        if ($n -gt 0 -and $n -lt $sb.Capacity) { return $sb.ToString() }
    } catch {
        Write-SetupLog ('Full name of ' + $Path + ': ' + $_.Exception.Message)
    }
    return $Path
}

function Use-LongTempPath {
    # Gives %TEMP% and %TMP% their full spelling (Get-LongPath) in setup and the programs it starts.
    foreach ($name in @('TEMP', 'TMP')) {
        $value = [Environment]::GetEnvironmentVariable($name)
        $long = Get-LongPath $value
        if ($long -and $long -ne $value) {
            [Environment]::SetEnvironmentVariable($name, $long)
            Write-SetupLog ('%' + $name + '% is ' + $long)
        }
    }
}

function Remove-Folder {
    # Deletes a folder and everything in it. It never stops setup: a folder that can't be deleted is only
    # logged. Not Remove-Item -Recurse: in Windows PowerShell 5.1 that fails on a path with a short (8.3) name
    # in it when the current folder is on the same drive ("An object at the specified path C:\Users\SAMLEE~1
    # does not exist"), even with -ErrorAction SilentlyContinue.
    param([Parameter(Mandatory = $true)][string]$Path)
    try {
        if ([IO.Directory]::Exists($Path)) { [IO.Directory]::Delete($Path, $true) }
    } catch {
        Write-SetupLog ('Could not delete ' + $Path + ': ' + $_.Exception.Message)
    }
}

function Unblock-SetupFiles {
    # Takes the "downloaded from the internet" mark off setup's own files, so Windows doesn't warn "The publisher
    # could not be verified" every time setup is opened (only the first time, before this has run).
    param([Parameter(Mandatory = $true)][string]$Root)
    try {
        Get-ChildItem -LiteralPath $Root -Recurse -File -Force -ErrorAction SilentlyContinue |
            Unblock-File -ErrorAction SilentlyContinue
    } catch {
        Write-SetupLog ('Unblocking setup''s files: ' + $_.Exception.Message)
    }
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
