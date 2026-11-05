# Chat Computer's own system (os/ in this folder) on this PC: signing its start-up file with a key made on this
# PC, putting it on the PC's start-up (EFI) partition next to Windows' own files, its entry in the power-on
# menu, and getting past Secure Boot. ASCII-only file (see Common.ps1).

$Script:OwnDir = 'chatcomputer-own'              # \EFI\chatcomputer-own on the EFI partition
$Script:OwnEntryName = 'Chat Computer (own system)'
$Script:OwnKeyName = 'Chat Computer Secure Boot key'  # the certificate's friendly name in Windows' certificate store
# The key made for ClaudeOS (Chat Computer's name until 2026-10-19), with the same steps: a PC whose setup screen
# approved it trusts Chat Computer signed with it too, so it starts from the power-on menu with no visit there.
$Script:OldOwnKeyName = 'ClaudeOS Secure Boot key'
$Script:OwnFile = 'chatcomputer.efi'
$Script:OwnCert = 'ChatComputer.cer'
$Script:OwnSettings = 'settings.txt'            # read by the kernel at start-up (os/kernel/src/settings.rs)
$Script:OwnMemory = 'memory.bin'                # the conversation, kept across restarts (os/remember)
$Script:OwnMemorySizes = @(4MB, 1MB)            # the memory file's size: the first that leaves room on the partition
$Script:OwnLog = 'log.bin'                      # its log, kept for this to read back (os/kernel/src/journal.rs)
$Script:OwnLogSize = 512KB
$Script:OwnLastRun = 'last-run.txt'             # the last run's log, as read back, in the work folder
$Script:OwnEspRoom = 16MB                       # left free on the start-up partition, for Windows' own updates

# ------------------------------------------------------------------ its model, and its settings: the clock
# Chat Computer's own system answers with the model Chat Computer's app gets ready on Windows (its own engine runs it
# on the PC itself): it finds it there when it starts (os/kernel/src/assistant.rs), as this does here, to say which.
$Script:OwnModels = 'ai.chatcomputer.desktop\models'   # under %LOCALAPPDATA%: NAME.ccm, as the app's engine leaves them
# Bigger models answered too slowly when Chat Computer's own system used one processor core (it uses all of them now,
# not yet measured on a PC): of the models there, it picks the biggest file at most this (the smallest, if none is).
$Script:OwnOneCoreMost = 3600000000

function Get-OwnModelLabel {
    # A model's name from its file's: "qwen3.5-4b.ccm" is "Qwen3.5 4B" (as the kernel says it).
    param([Parameter(Mandatory = $true)][string]$Name)
    $stem = [IO.Path]::GetFileNameWithoutExtension($Name)
    $at = $stem.IndexOf('-')
    if ($at -lt 0) { return $stem }
    $family = $stem.Substring(0, $at)
    if ($family.StartsWith('qwen')) { $family = 'Qwen' + $family.Substring(4) }
    return ($family + ' ' + $stem.Substring($at + 1).ToUpperInvariant())
}

function Select-OwnModel {
    # Which of these model files (objects with Name, Length and FullName) Chat Computer's own system picks: the
    # biggest at most $Script:OwnOneCoreMost, else the smallest. $null if there are none.
    param([AllowEmptyCollection()][object[]]$Files)
    $files = @($Files | Where-Object { $null -ne $_ })
    if ($files.Count -eq 0) { return $null }
    $quick = @($files | Where-Object { [int64]$_.Length -le $Script:OwnOneCoreMost } | Sort-Object -Property Length -Descending)
    $pick = $(if ($quick.Count -gt 0) { $quick[0] } else { @($files | Sort-Object -Property Length)[0] })
    return [pscustomobject]@{ Path = [string]$pick.FullName; Label = (Get-OwnModelLabel ([string]$pick.Name)); Bytes = [int64]$pick.Length }
}

function Find-OwnModel {
    # The model Chat Computer's own system will answer with: of those Chat Computer's app got ready on this PC, the
    # one it picks (Select-OwnModel). $null if the app hasn't got one ready yet.
    param([string]$Dir = (Join-Path $env:LOCALAPPDATA $Script:OwnModels))
    if (-not (Test-Path -LiteralPath $Dir)) { return $null }
    return (Select-OwnModel @(Get-ChildItem -LiteralPath $Dir -Filter '*.ccm' -File -ErrorAction SilentlyContinue))
}

function Get-ClockOffsetMinutes {
    # How far ahead of UTC the clock the PC keeps when it's off is: Windows keeps it in local time (unless
    # RealTimeIsUniversal is set), so it's this time zone's offset now. Chat Computer's kernel reads that clock.
    try {
        $v = Get-ItemProperty -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\TimeZoneInformation' -ErrorAction Stop
        $utc = Get-Prop $v 'RealTimeIsUniversal'
        if ($null -ne $utc -and [int64]$utc -ne 0) { return 0 }
    } catch { $null = $_ }
    return [int][Math]::Round([TimeZoneInfo]::Local.GetUtcOffset([DateTime]::Now).TotalMinutes)
}

function New-OwnSettingsText {
    # The settings file, as the kernel reads it: "name = value" lines, # for notes. $Model: a model's file named in
    # the settings ('': none, and the kernel finds the one Chat Computer's app got ready). $MemoryId: the memory
    # file's id (0: there's no memory file); $LogId: the log file's. $Cores: the most processor cores to use, put
    # there by hand ('': all).
    param([AllowEmptyString()][string]$Model, [int]$ClockOffsetMinutes, [uint32]$MemoryId = 0, [uint32]$LogId = 0,
          [AllowEmptyString()][string]$Cores = '', [AllowEmptyString()][string]$MetaKey = '',
          [AllowEmptyString()][string]$MetaModel = '')
    $lines = @('# Chat Computer''s settings, read by its own system when it starts. Written by Chat Computer Own System.')
    if ($Model) { $lines += ('model = ' + $Model) }
    $lines += ('clock-offset-minutes = ' + $ClockOffsetMinutes)
    if ($MemoryId -ne 0) { $lines += ('memory-id = ' + $MemoryId.ToString('x8')) }
    if ($LogId -ne 0) { $lines += ('log-id = ' + $LogId.ToString('x8')) }
    if ($Cores) { $lines += ('cores = ' + $Cores) }
    if ($MetaKey) { $lines += ('meta-api-key = ' + $MetaKey) }
    if ($MetaModel) { $lines += ('meta-model = ' + $MetaModel) }
    return (($lines -join "`r`n") + "`r`n")
}

function Get-OwnSettingsModel {
    # The model's file a settings file's text names ('' if none): one put there by hand, kept when this runs again.
    param([AllowNull()][string]$Text)
    if (-not $Text) { return '' }
    foreach ($line in ($Text -split "`r?`n")) {
        $m = [regex]::Match($line, '^\s*model\s*=\s*(\S.*?)\s*$')
        if ($m.Success) { return $m.Groups[1].Value }
    }
    return ''
}

function Get-OwnSettingsCores {
    # The most processor cores a settings file's text says to use ('' if it doesn't say): put there by hand, for a PC
    # whose other cores don't start well, and kept when this runs again.
    param([AllowNull()][string]$Text)
    if (-not $Text) { return '' }
    foreach ($line in ($Text -split "`r?`n")) {
        $m = [regex]::Match($line, '(?i)^\s*cores\s*=\s*"*\s*([0-9]{1,3})\s*"*\s*$')
        if ($m.Success -and [int]$m.Groups[1].Value -ge 1) { return [string][int]$m.Groups[1].Value }
    }
    return ''
}

function Get-OwnSettingsValue {
    # A "name = value" line's value in a settings file's text ('' if none): meta-api-key and meta-model (Meta's Model
    # API, for Muse Spark to answer), put there by hand or pasted when this asked, and kept when this runs again.
    param([AllowNull()][string]$Text, [Parameter(Mandatory = $true)][string]$Name)
    if (-not $Text) { return '' }
    foreach ($line in ($Text -split "`r?`n")) {
        $m = [regex]::Match($line, '^\s*' + [regex]::Escape($Name) + '\s*=\s*(\S+)\s*$')
        if ($m.Success) { return $m.Groups[1].Value }
    }
    return ''
}

function Test-OwnMetaKey {
    # Whether text can be a key for Meta's Model API as the kernel takes one (os/kernel/src/settings.rs): visible
    # ASCII characters only, no spaces, at most 512 of them.
    param([AllowNull()][string]$Key)
    return ($null -ne $Key -and $Key.Length -ge 8 -and $Key.Length -le 512 -and $Key -cmatch '^[\x21-\x7E]+$')
}

function Read-OwnSettings {
    # The settings file on the start-up partition now, as text ('' if there's none, or it can't be read: never an
    # error, and the partition is closed again).
    try {
        $esp = Get-WindowsEsp
        if (-not $esp) { return '' }
        $m = Mount-EspPartition $esp
        try {
            $settings = [IO.Path]::Combine($m.Root, 'EFI', $Script:OwnDir, $Script:OwnSettings)
            if (Test-Path -LiteralPath $settings -PathType Leaf) { return [IO.File]::ReadAllText($settings) }
            return ''
        } finally {
            Dismount-EspPartition $m
        }
    } catch {
        Write-SetupLog ('Reading the settings: ' + $_.Exception.Message)
        return ''
    }
}

function Read-OwnMetaKey {
    # A key for Meta's Model API, pasted by the person: not shown as it's typed, never logged. '' for none.
    for ($attempt = 0; $attempt -lt 2; $attempt++) {
        $secure = Read-Host -AsSecureString 'Paste the key and press Enter (or just press Enter to get the app''s model instead)'
        $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
        try { $key = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr) } finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }
        $key = ([string]$key).Trim()
        if (-not $key) { return '' }
        if (Test-OwnMetaKey $key) { return $key }
        Write-Warn 'That doesn''t look like a key (one word, at most 512 characters). Paste it again, or just press Enter.'
    }
    return ''
}

function Get-OwnSettingsMemoryId {
    # The memory file's id in a settings file's text ($null if none); with -Name 'log-id', the log file's.
    param([AllowNull()][string]$Text, [string]$Name = 'memory-id')
    if (-not $Text) { return $null }
    foreach ($line in ($Text -split "`r?`n")) {
        $m = [regex]::Match($line, '^\s*' + [regex]::Escape($Name) + '\s*=\s*([0-9a-fA-F]{1,8})\s*$')
        if ($m.Success) {
            $id = [Convert]::ToUInt32($m.Groups[1].Value, 16)
            if ($id -ne 0) { return $id }
        }
    }
    return $null
}

# ------------------------------------------------------------------ its memory file: the conversation, kept across restarts
# Chat Computer's own system has no file system of its own: it keeps the conversation in this file, which it only
# ever changes in place (os/remember), so it's made here at its full size. The file is 512-byte chunks, each starting
# with a tag: "CCMEMORY", the file's id (a random number, also in the settings) and the chunk's number, little-endian.
# Before writing anything, its own system checks that the place it writes to carries this file's tags.
function New-OwnMemoryId {
    # A random id for a new memory file (never 0, which the kernel reads as none).
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $b = New-Object byte[] 4
        do {
            $rng.GetBytes($b)
            $id = [BitConverter]::ToUInt32($b, 0)
        } while ($id -eq 0)
        return $id
    } finally {
        $rng.Dispose()
    }
}

function New-OwnMemoryTag {
    # The 16 bytes chunk $Index of the memory file with $Id starts with.
    param([uint32]$Id, [uint32]$Index)
    $t = New-Object byte[] 16
    [Array]::Copy([Text.Encoding]::ASCII.GetBytes('CCMEMORY'), 0, $t, 0, 8)
    [Array]::Copy([BitConverter]::GetBytes($Id), 0, $t, 8, 4)
    [Array]::Copy([BitConverter]::GetBytes($Index), 0, $t, 12, 4)
    return , $t
}

function New-OwnMemoryBytes {
    # A new memory file of $Bytes (whole chunks): every chunk tagged, nothing kept yet.
    param([uint32]$Id, [int]$Bytes)
    $chunks = [int][Math]::Floor($Bytes / 512)
    $b = New-Object byte[] ($chunks * 512)
    $magic = [Text.Encoding]::ASCII.GetBytes('CCMEMORY')
    $idBytes = [BitConverter]::GetBytes($Id)
    for ($i = 0; $i -lt $chunks; $i++) {
        $o = $i * 512
        [Array]::Copy($magic, 0, $b, $o, 8)
        [Array]::Copy($idBytes, 0, $b, $o + 8, 4)
        [Array]::Copy([BitConverter]::GetBytes([uint32]$i), 0, $b, $o + 12, 4)
    }
    return , $b
}

function Test-OwnMemoryFile {
    # Whether the file at $Path is the memory file with $Id: whole chunks, its first and last carrying their tags.
    param([Parameter(Mandatory = $true)][string]$Path, $Id)
    if ($null -eq $Id -or [uint32]$Id -eq 0 -or -not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
    $length = [int64](Get-Item -LiteralPath $Path).Length
    if ($length -lt 4096 -or ($length % 512) -ne 0) { return $false }
    $last = [int64][Math]::Floor($length / 512) - 1
    $stream = [IO.File]::OpenRead($Path)
    try {
        $buf = New-Object byte[] 16
        foreach ($chunk in @([int64]0, $last)) {
            $stream.Position = $chunk * 512
            if ($stream.Read($buf, 0, 16) -ne 16) { return $false }
            $want = New-OwnMemoryTag -Id ([uint32]$Id) -Index ([uint32]$chunk)
            if ([BitConverter]::ToString($buf) -ne [BitConverter]::ToString($want)) { return $false }
        }
        return $true
    } finally {
        $stream.Dispose()
    }
}

function Get-OwnFreeSpace {
    # Free bytes on the drive at $Root ($null if Windows won't say).
    param([Parameter(Mandatory = $true)][string]$Root)
    try {
        return [int64](New-Object IO.DriveInfo $Root).AvailableFreeSpace
    } catch {
        Write-SetupLog ('Free space at ' + $Root + ': ' + $_.Exception.Message)
        return $null
    }
}

function Install-OwnMemory {
    # The memory file in $Dir (on the drive at $Root). The one there stays, with the conversation in it, when it's the
    # one with the settings' id ($Id); otherwise a new one is made, of the first size in $Script:OwnMemorySizes that
    # leaves $Script:OwnEspRoom free. Returns its id: 0 when there's no room for one (nothing is kept then).
    param([Parameter(Mandatory = $true)][string]$Dir, [Parameter(Mandatory = $true)][string]$Root, $Id)
    $path = Join-Path $Dir $Script:OwnMemory
    if (Test-OwnMemoryFile -Path $path -Id $Id) {
        Write-SetupLog ('Kept the memory file there, ' + (Get-Item -LiteralPath $path).Length + ' bytes')
        return [uint32]$Id
    }
    $free = Get-OwnFreeSpace $Root
    $there = 0
    if (Test-Path -LiteralPath $path -PathType Leaf) { $there = [int64](Get-Item -LiteralPath $path).Length }
    foreach ($size in $Script:OwnMemorySizes) {
        if ($null -ne $free -and $free + $there - $size -lt $Script:OwnEspRoom) { continue }
        $newId = New-OwnMemoryId
        try {
            [IO.File]::WriteAllBytes($path, (New-OwnMemoryBytes -Id $newId -Bytes $size))
        } catch {
            Write-SetupLog ('Writing the memory file: ' + $_.Exception.Message)
            break
        }
        if (Test-OwnMemoryFile -Path $path -Id $newId) {
            Write-SetupLog ('Made a memory file of ' + $size + ' bytes')
            return $newId
        }
        Write-SetupLog 'The memory file written is not the same when read back.'
        break
    }
    Write-SetupLog ('No memory file (free on the partition: ' + $free + ' bytes)')
    if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue }
    return [uint32]0
}

# ------------------------------------------------------------------ its log, kept for this to read back
# Chat Computer's own system keeps its log (what it found on this PC, each step of its start, and why it stopped, if
# it did) in a file made as the memory file is, with an id of its own (log-id in the settings): a PC has no other
# way to show it, there being no serial port on most. This reads the last run's back when it next runs.
function Install-OwnLog {
    # The log file in $Dir (on the drive at $Root): the one there stays when it's the one with the settings' id ($Id),
    # else a new one is made, if it leaves $Script:OwnEspRoom free. Returns its id: 0 when there's none.
    param([Parameter(Mandatory = $true)][string]$Dir, [Parameter(Mandatory = $true)][string]$Root, $Id)
    $path = Join-Path $Dir $Script:OwnLog
    if (Test-OwnMemoryFile -Path $path -Id $Id) { return [uint32]$Id }
    $free = Get-OwnFreeSpace $Root
    $there = 0
    if (Test-Path -LiteralPath $path -PathType Leaf) { $there = [int64](Get-Item -LiteralPath $path).Length }
    if ($null -eq $free -or $free + $there - $Script:OwnLogSize -ge $Script:OwnEspRoom) {
        $newId = New-OwnMemoryId
        try {
            [IO.File]::WriteAllBytes($path, (New-OwnMemoryBytes -Id $newId -Bytes $Script:OwnLogSize))
            if (Test-OwnMemoryFile -Path $path -Id $newId) {
                Write-SetupLog ('Made a log file of ' + $Script:OwnLogSize + ' bytes')
                return $newId
            }
            Write-SetupLog 'The log file written is not the same when read back.'
        } catch {
            Write-SetupLog ('Writing the log file: ' + $_.Exception.Message)
        }
    }
    Write-SetupLog ('No log file (free on the partition: ' + $free + ' bytes)')
    if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue }
    return [uint32]0
}

function Get-OwnFnv1a {
    # FNV-1a, 64 bits, as os/remember hashes a copy (little-endian bytes): in two 32-bit halves, so nothing passes 64 bits.
    param([AllowEmptyCollection()][byte[]]$Data)
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

function Read-OwnKept {
    # The newest whole copy kept in $Bytes, a file made as the memory file is (os/remember: two halves written in turn,
    # each starting with a header: "CCMEMDAT", version 1, a sequence number, the data's length and hash; 496 bytes of
    # each 512-byte chunk carry them, after its tag), as bytes: $null if it holds none. (The hash isn't checked: a
    # header is written last, after its data, so the newest one's data is whole.)
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)
    $chunks = [int][Math]::Floor($Bytes.Length / 512)
    $half = [int][Math]::Floor($chunks / 2)
    if ($half -lt 2) { return $null }
    $best = $null
    $bestSequence = [uint64]0
    for ($h = 0; $h -lt 2; $h++) {
        $first = $h * $half
        $o = $first * 512 + 16
        if ([Text.Encoding]::ASCII.GetString($Bytes, $o, 8) -cne 'CCMEMDAT') { continue }
        if ([BitConverter]::ToUInt32($Bytes, $o + 8) -ne 1) { continue }
        $sequence = [BitConverter]::ToUInt64($Bytes, $o + 12)
        $length = [int64][BitConverter]::ToUInt32($Bytes, $o + 20)
        if ($length -gt ($half * 496 - 32)) { continue }
        if ($null -ne $best -and $sequence -le $bestSequence) { continue }
        $all = New-Object byte[] (32 + $length)
        $at = 0
        $c = 0
        while ($at -lt $all.Length) {
            $n = [Math]::Min(496, $all.Length - $at)
            [Array]::Copy($Bytes, ($first + $c) * 512 + 16, $all, $at, $n)
            $at += $n
            $c++
        }
        $data = New-Object byte[] $length
        [Array]::Copy($all, 32, $data, 0, $length)
        if ([BitConverter]::ToString((Get-OwnFnv1a $data)) -ne [BitConverter]::ToString($Bytes, $o + 24, 8)) { continue }
        $best = $data
        $bestSequence = $sequence
    }
    if ($null -eq $best) { return $null }
    return , $best
}

function Get-OwnLastRunSummary {
    # What to say about the run whose log is $Text: when (its clock's line), and how it went: why it stopped, if it
    # did; else the last step of a start that didn't get as far as being ready; else that it was ready.
    param([AllowEmptyString()][string]$Text)
    $lines = @($Text -split "`r?`n" | Where-Object { $_.Trim() })
    if ($lines.Count -eq 0) { return $null }
    $clock = @($lines | Where-Object { $_ -match 'kernel: clock: ' })
    $when = ''
    if ($clock.Count -gt 0) { $when = ' (' + ($clock[0] -replace '^.*kernel: clock: ', '' -replace ' \(.*$', '') + ')' }
    for ($i = 0; $i -lt $lines.Count; $i++) {
        # Only the kernel's own words for a stop (main.rs, idt.rs), or another of its cores' (cores.rs), which the rest
        # may outlive; a panic's message is on the line after its place.
        $stop = [regex]::Match($lines[$i], '^Chat Computer kernel(?: stopped|: (processor core \d+) stopped): (.*)$')
        if ($stop.Success) {
            $why = $stop.Groups[2].Value
            for ($j = $i + 1; $why.EndsWith(':') -and $j -lt $lines.Count -and $j -le $i + 3; $j++) { $why += ' ' + $lines[$j] }
            $who = $(if ($stop.Groups[1].Success) { 'its ' + $stop.Groups[1].Value } else { 'it' })
            return ('The last time Chat Computer''s own system ran' + $when + ', ' + $who + ' stopped: ' + $why)
        }
    }
    if (-not ($lines -contains 'Chat Computer: ready')) {
        return ('The last time Chat Computer''s own system ran' + $when + ', it didn''t finish starting. The last thing in its log: ' + $lines[-1])
    }
    return ('The last time Chat Computer''s own system ran' + $when + ', it started, and nothing went wrong that it noticed.')
}

function Read-OwnLastRun {
    # The last run's log, from the log file on the start-up partition, saved as $Script:OwnLastRun in $WorkDir.
    # Returns what to say about it ($null: no log, or nothing in it).
    param([Parameter(Mandatory = $true)][string]$WorkDir)
    try {
        $esp = Get-WindowsEsp
        if (-not $esp) { return $null }
        $m = Mount-EspPartition $esp
        try {
            $dir = [IO.Path]::Combine($m.Root, 'EFI', $Script:OwnDir)
            $settings = Join-Path $dir $Script:OwnSettings
            $path = Join-Path $dir $Script:OwnLog
            if (-not (Test-Path -LiteralPath $settings) -or -not (Test-Path -LiteralPath $path)) { return $null }
            $id = Get-OwnSettingsMemoryId ([IO.File]::ReadAllText($settings)) -Name 'log-id'
            if (-not (Test-OwnMemoryFile -Path $path -Id $id)) { return $null }
            $data = Read-OwnKept ([IO.File]::ReadAllBytes($path))
        } finally {
            Dismount-EspPartition $m
        }
        if ($null -eq $data -or $data.Length -eq 0) { return $null }
        $text = [Text.Encoding]::UTF8.GetString($data)
        $saved = Join-Path $WorkDir $Script:OwnLastRun
        [IO.File]::WriteAllText($saved, ($text -replace "`r?`n", "`r`n"), (New-Object Text.UTF8Encoding $false))
        return [pscustomobject]@{ Summary = (Get-OwnLastRunSummary $text); Path = $saved }
    } catch {
        Write-SetupLog ('Reading the last run''s log: ' + $_.Exception.Message)
        return $null
    }
}

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

function Get-OwnKeys {
    # Every key made on this PC for signing Chat Computer (or ClaudeOS before it), with its private key, newest first.
    $names = @($Script:OwnKeyName, $Script:OldOwnKeyName)
    return , @(Get-ChildItem -Path 'Cert:\CurrentUser\My' -ErrorAction SilentlyContinue |
            Where-Object { $names -contains $_.FriendlyName -and $_.HasPrivateKey } | Sort-Object NotBefore -Descending)
}

function Select-OwnKey {
    # The key to sign with: one the PC's firmware already trusts ($Trusted says, with Secure Boot on), so Chat Computer
    # starts with nothing to approve; else Chat Computer's own; else $null (a new one is made).
    param([object[]]$Keys = @(), [scriptblock]$Trusted)
    if ($Trusted) {
        foreach ($k in $Keys) { if ((& $Trusted $k) -eq $true) { return $k } }
    }
    foreach ($k in $Keys) { if ([string](Get-Prop $k 'FriendlyName') -eq $Script:OwnKeyName) { return $k } }
    return $null
}

function Wait-OwnCountdown {
    # Counts down $Seconds on the screen; $true when it gets to the end, $false when a key is pressed first (or there's
    # no keyboard to wait on: then it doesn't wait, and goes ahead).
    param([int]$Seconds = 10, [switch]$Unattended)
    if ($Unattended) { return $true }
    for ($s = $Seconds; $s -gt 0; $s--) {
        Write-Host ("`r  Restarting in " + $s + ' s (press any key to stay in Windows) ') -NoNewline
        for ($t = 0; $t -lt 10; $t++) {
            $pressed = $false
            try { $pressed = [Console]::KeyAvailable } catch { $pressed = $false }
            if ($pressed) {
                try { $null = [Console]::ReadKey($true) } catch { $null = $_ }
                Write-Host ''
                return $false
            }
            Start-Sleep -Milliseconds 100
        }
    }
    Write-Host ''
    return $true
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

function Assert-OwnRoom {
    # Throws, saying so, when the partition at $Root has no room for a file of $Size bytes at $Path (in place of the
    # one there, if any) and a megabyte more, for the settings and the certificate. (Chat Computer's own system is
    # 10 MB since it carries Chinese, Japanese and Korean.) Nothing's said when Windows won't say how much is free.
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)][string]$Path,
          [Parameter(Mandatory = $true)][int64]$Size)
    $free = Get-OwnFreeSpace $Root
    if ($null -eq $free) { return }
    $there = 0
    if (Test-Path -LiteralPath $Path) { $there = (Get-Item -LiteralPath $Path).Length }
    if ($free + $there -lt $Size + 1MB) {
        throw ('The partition this PC starts from has ' + [math]::Floor($free / 1MB) + ' MB free, and Chat Computer''s ' +
            'own system needs ' + [math]::Ceiling(($Size + 1MB - $there) / 1MB) + ' MB there.')
    }
}

function Install-OwnFiles {
    # The signed start-up file, the key's certificate, the settings and the memory file, in \EFI\chatcomputer-own on
    # the partition Windows starts from. The settings are written anew (a model named in them by hand is kept; an
    # older setup's Claude API key isn't; Meta's key and model are, or the key pasted now, $MetaKey). Returns whether
    # there's a memory file (Memory), and whether the settings have a key for Meta's Model API (Meta).
    param([Parameter(Mandatory = $true)][string]$Signed, [Parameter(Mandatory = $true)][string]$Certificate,
          [AllowEmptyString()][string]$MetaKey = '')
    $esp = Get-WindowsEsp
    if (-not $esp) { throw 'Windows could not find the partition this PC starts from.' }
    $m = Mount-EspPartition $esp
    try {
        $dir = [IO.Path]::Combine($m.Root, 'EFI', $Script:OwnDir)
        if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
        $file = Join-Path $dir $Script:OwnFile
        Assert-OwnRoom -Root $m.Root -Path $file -Size (Get-Item -LiteralPath $Signed).Length
        Copy-Item -LiteralPath $Signed -Destination $file -Force
        Copy-Item -LiteralPath $Certificate -Destination (Join-Path $dir $Script:OwnCert) -Force
        if ((Get-OwnSha256 $file) -ne (Get-OwnSha256 $Signed)) { throw 'The copy on the start-up partition is not the same as the original.' }
        $settings = Join-Path $dir $Script:OwnSettings
        $before = ''
        if (Test-Path -LiteralPath $settings) { $before = [IO.File]::ReadAllText($settings) }
        $model = Get-OwnSettingsModel $before
        $memoryId = Install-OwnMemory -Dir $dir -Root $m.Root -Id (Get-OwnSettingsMemoryId $before)
        $logId = Install-OwnLog -Dir $dir -Root $m.Root -Id (Get-OwnSettingsMemoryId $before -Name 'log-id')
        $key = $MetaKey
        if (-not $key) { $key = Get-OwnSettingsValue $before 'meta-api-key' }
        $text = New-OwnSettingsText -Model $model -ClockOffsetMinutes (Get-ClockOffsetMinutes) -MemoryId $memoryId `
            -LogId $logId -Cores (Get-OwnSettingsCores $before) -MetaKey $key -MetaModel (Get-OwnSettingsValue $before 'meta-model')
        [IO.File]::WriteAllText($settings, $text, [Text.Encoding]::ASCII)
        Write-SetupLog ('Copied to ' + $dir + '; settings ' + $(if ($model) { 'naming a model' } else { 'naming no model' }) +
                        $(if ($key) { ', with a key for Meta''s Model API' } else { '' }))
        return [pscustomobject]@{ Memory = ($memoryId -ne 0); Meta = [bool]$key }
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
    } elseif ($Maker -match '(?i)micro-star|\bmsi\b') {
        Write-Line '  2. (MSI: F7 switches to Advanced Mode.) Open Settings, then Security, then "Secure Boot".'
        Write-Line '  3. Set "Secure Boot Mode" to Custom. Then open "Key Management".'
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
    } elseif ($Maker -match '(?i)micro-star|\bmsi\b') {
        Write-Line '  2. (MSI: F7 switches to Advanced Mode.) Open Settings, then Security, then "Secure Boot".'
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
    $last = Read-OwnLastRun -WorkDir $WorkDir
    if ($last) {
        Write-Line $last.Summary
        Write-Line ('Its whole log is in ' + $last.Path + '.')
    }
    $release = Get-OwnRelease -RepoRoot $RepoRoot
    $sb = Get-SecureBootState
    if ($sb -eq $true) { Write-Line 'Secure Boot is on.' } elseif ($sb -eq $false) { Write-Line 'Secure Boot is off.' }
    $parts = Save-PcParts -Path (Join-Path $WorkDir 'pc.txt') -SecureBoot $sb
    Write-Line ('A list of this PC''s parts is in ' + $parts + ', so Chat Computer''s drivers can be written for them.')

    Write-Step 'Signing Chat Computer with this PC''s own key'
    $trusted = $null
    if ($sb -eq $true) { $trusted = { param($k) Test-OwnKeyApproved -Cert $k } }
    $cert = Select-OwnKey -Keys (Get-OwnKeys) -Trusted $trusted
    if ($cert -and [string](Get-Prop $cert 'FriendlyName') -eq $Script:OldOwnKeyName) {
        Write-Good 'Using the key this PC made for ClaudeOS, which its Secure Boot already trusts.'
    } elseif ($cert) {
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

    Write-Step 'Its model'
    $model = Find-OwnModel
    $metaKey = ''
    # (A key for Meta's Model API in the settings comes first, as the kernel takes it: Muse Spark answers then.)
    if (Get-OwnSettingsValue (Read-OwnSettings) 'meta-api-key') {
        Write-Good 'It answers with Meta''s Muse Spark, over the internet, with the key for Meta''s Model API in its settings.'
        Write-Line 'That needs a network cable on this PC (or a USB network adapter, or an Android phone''s USB tethering):'
        Write-Line 'Wi-Fi isn''t supported yet. What you type goes to Meta.'
        if ($model) {
            Write-Line ('For ' + $model.Label + ', the model on this PC, instead: run this once with -Remove (which takes the')
            Write-Line 'key off with the rest), then again.'
        }
    } elseif ($model) {
        Write-Good ('It answers with ' + $model.Label + ' (' + (Format-Size $model.Bytes) + '), the model Chat Computer''s app got ready on this PC.')
        Write-Line 'It runs on the PC itself, with no account and nothing to pay: nothing you type leaves it. Starting up, it'
        Write-Line 'reads the model into memory, which takes a moment.'
    } else {
        Write-Warn 'Chat Computer''s app hasn''t got its model ready on this PC yet. Open the app once (it gets its model by'
        Write-Warn 'itself, free); Chat Computer''s own system finds it when it starts.'
        Write-Line 'Or it can answer with Meta''s Muse Spark, over the internet: that needs a key for Meta''s Model API (from'
        Write-Line 'dev.meta.ai) and a network cable on this PC (Wi-Fi isn''t supported yet), and what you type goes to Meta. The'
        Write-Line 'key is kept in its settings on the start-up partition, where anyone who can read that partition can read it.'
        if (-not $Unattended) { $metaKey = Read-OwnMetaKey }
        if ($metaKey) { Write-Good 'It answers with Meta''s Muse Spark.' }
    }

    $approved = $null
    if ($sb -eq $true) { $approved = Test-OwnKeyApproved -Cert $cert }
    $setupScreen = ($sb -eq $true -and $approved -ne $true)
    $bitlocker = $setupScreen -and (Get-BitLockerOn)
    if ($approved -eq $true) { Write-Good 'This PC already trusts Chat Computer''s key.' }

    # (One double-click does it all, its owner asked: "make it easier to work with a single click". What changes is
    # said as it's done, with no question before it; the restart can still be stopped, with any key.)
    Write-Step 'Putting it on this PC'
    Write-Line '  * Its start-up file (signed with this PC''s key), the key''s certificate, its settings (how far this PC''s'
    Write-Line '    clock is from UTC), its memory file (where it keeps your conversation across restarts) and its log (what'
    Write-Line '    it found on this PC and how its start went, read back here next time) go in \EFI\chatcomputer-own on'
    Write-Line '    the partition this PC starts from (about 8 MB). Windows'' own files stay as they are. The conversation is'
    Write-Line '    kept there as it is, where anyone who can read that partition can read it.'
    Write-Line ('  * "' + $Script:OwnEntryName + '" stays in the power-on menu (the PC''s boot menu), to choose whenever you like.')
    if ($bitlocker) {
        Write-Line '  * BitLocker pauses for two restarts, so the Secure Boot change doesn''t make it ask for its recovery key.'
    }
    $installed = Install-OwnFiles -Signed $signed -Certificate $cer -MetaKey $metaKey
    if (-not $installed.Memory) {
        Write-Warn 'Its memory file couldn''t be made on the start-up partition (too little room, or it couldn''t be written), so the conversation won''t be kept across restarts.'
    }
    $id = Get-OwnBootEntry
    Write-Good ('Chat Computer''s own system is on the start-up partition, and "' + $Script:OwnEntryName + '" is in the power-on menu.')

    if (-not $setupScreen) {
        if ($NoRestart) { return }
        if (-not $model -and -not $installed.Meta) {
            # (Without a model it would start and only say it needs one: it stays in the menu for when it's here.)
            Write-Line ''
            Write-Line 'Get its model first: install Chat Computer''s app from chatcomputer.ai/get (choose App), and leave it open'
            Write-Line ('until it says its model is ready. Then choose "' + $Script:OwnEntryName + '" in the boot menu, or double-click')
            Write-Line 'this again.'
            try { Start-Process 'https://chatcomputer.ai/get' } catch { Write-SetupLog ('Opening chatcomputer.ai/get: ' + $_.Exception.Message) }
            return
        }
        Invoke-Bcdedit @('/set', '{fwbootmgr}', 'bootsequence', $id)
        Write-Line ''
        Write-Line 'To come back to Windows: restart (the next start is Windows), or hold the power button until the PC turns off.'
        Write-Line ('Next time, choose "' + $Script:OwnEntryName + '" in the boot menu as the PC starts (often F12, F11, F8 or Esc).')
        if (Wait-OwnCountdown 10 -Unattended:$Unattended) {
            & shutdown.exe /r /t 0
        } else {
            Write-Line ('Staying in Windows: it starts at the next restart, once, and whenever you choose it in the boot menu.')
        }
        return
    }
    Invoke-Bcdedit @('/set', '{fwbootmgr}', 'bootsequence', $id)

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
    param([switch]$Unattended, [string]$WorkDir)
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
    if ($WorkDir) {
        # (Its last run's log, kept here before its files go.)
        $last = Read-OwnLastRun -WorkDir $WorkDir
        if ($last) { Write-Line ('Its last run''s log is kept in ' + $last.Path + '.') }
    }
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
