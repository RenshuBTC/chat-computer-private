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
# On a PC whose Secure Boot doesn't trust Chat Computer's key, shim (os\release\shim: Microsoft signs it, as it does for
# Linux distributions) starts it once the key is in shim's own list (MokList), approved once on MokManager's blue screen.
$Script:OwnShim = 'shimx64.efi'
$Script:OwnMokManager = 'mmx64.efi'
$Script:OwnShimNext = 'grubx64.efi'              # the name shim starts: Chat Computer's signed start-up file
$Script:ShimLockGuid = '605dab50-e046-4300-abb6-3dd810dd8b23'  # shim's GUID: MokNew and MokAuth are under it
$Script:OwnMokPassword = 'hello'                 # typed on the blue screen (its keys are the same on QWERTY, QWERTZ and AZERTY)
$Script:OwnMokMark = 'mok.txt'                   # in the work folder: where the key's approval on the blue screen is
$Script:OwnVarGuid = '1dd7d236-3557-417b-b0ab-6cf839b24829'  # Chat Computer's own firmware variables' (os/boot/src/uefi.rs)
$Script:OwnStartedVar = 'ChatComputerStarted'    # its start-up stage's mark that it started (os/boot/src/main.rs)
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
          [AllowEmptyString()][string]$Cores = '')
    $lines = @('# Chat Computer''s settings, read by its own system when it starts. Written by Chat Computer Own System.')
    if ($Model) { $lines += ('model = ' + $Model) }
    $lines += ('clock-offset-minutes = ' + $ClockOffsetMinutes)
    if ($MemoryId -ne 0) { $lines += ('memory-id = ' + $MemoryId.ToString('x8')) }
    if ($LogId -ne 0) { $lines += ('log-id = ' + $LogId.ToString('x8')) }
    if ($Cores) { $lines += ('cores = ' + $Cores) }
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

function Clear-OwnKeys {
    # Throws away keys pressed while it was working, so that one pressed then doesn't answer what comes next.
    try { while ([Console]::KeyAvailable) { $null = [Console]::ReadKey($true) } } catch { $null = $_ }
}

function Wait-OwnCountdown {
    # Counts down $Seconds on the screen; $true when it gets to the end, $false when a key is pressed first (or there's
    # no keyboard to wait on: then it doesn't wait, and goes ahead).
    param([int]$Seconds = 10, [switch]$Unattended)
    if ($Unattended) { return $true }
    Clear-OwnKeys
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

# ------------------------------------------------------------------ ClaudeOS, out of the power-on menu
# ClaudeOS (Chat Computer's name until 2026-10-19) left entries in the power-on menu that its owner no longer wants
# ("i no longer want claudeOS, i dont know why it still is an option when i restart my computer, remove that, i only
# want chat computer"): its own system's (\EFI\claudeos-own), its installers' (\EFI\claudeos-setup,
# \EFI\chatcomputer-setup), and the Ubuntu-based one's (\EFI\ubuntu, Ubuntu's boot loader, shown as "Ubuntu"), that
# one only when its mark is on Windows' start-up partition (\EFI\claudeos\installed-*, or \EFI\chatcomputer\ from
# after the name changed), so a Ubuntu of the person's own is left alone. Only the menu's entries go: their files
# and partitions stay as they are, with whatever is in them.
function Test-OldClaudeOsMark {
    # Whether Windows' start-up partition has the Ubuntu-based ClaudeOS's mark. $false when it can't be read.
    try {
        $esp = Get-WindowsEsp
        if (-not $esp) { return $false }
        $m = Mount-EspPartition $esp
        try {
            foreach ($d in @('claudeos', 'chatcomputer')) {
                $dir = [IO.Path]::Combine($m.Root, 'EFI', $d)
                if ((Test-Path -LiteralPath $dir) -and
                    @(Get-ChildItem -LiteralPath $dir -Filter 'installed-*' -ErrorAction SilentlyContinue).Count -gt 0) { return $true }
            }
            return $false
        } finally {
            Dismount-EspPartition $m
        }
    } catch {
        Write-SetupLog ('Looking for ClaudeOS''s mark: ' + $_.Exception.Message)
        return $false
    }
}

function Select-OldClaudeOsEntries {
    # Of the power-on menu's entries, ClaudeOS's: Ubuntu's boot loader's only with $Ubuntu (its mark is there).
    param([object[]]$Entries = @(), [bool]$Ubuntu)
    return , @($Entries | Where-Object {
            $_.Id -ne '{fwbootmgr}' -and $_.Id -ne '{bootmgr}' -and (
                [string]$_.Path -match '(?i)^\\EFI\\(claudeos-own|claudeos-setup|chatcomputer-setup)\\' -or
                ($Ubuntu -and [string]$_.Path -match '(?i)^\\EFI\\ubuntu\\')) })
}

function Remove-OldClaudeOsEntries {
    # Takes ClaudeOS's entries out of the power-on menu, and puts Windows first in it. Returns how many went.
    $old = Select-OldClaudeOsEntries -Entries (Get-FirmwareBootEntries) -Ubuntu (Test-OldClaudeOsMark)
    if ($old.Count -eq 0) { return 0 }
    if (Get-BitLockerOn) {
        # (Changing the power-on menu can make BitLocker ask for its recovery key: paused for one restart.)
        try { Suspend-BitLocker -MountPoint 'C:' -RebootCount 1 -ErrorAction Stop | Out-Null } catch { Write-SetupLog ('Suspend-BitLocker: ' + $_.Exception.Message) }
    }
    foreach ($e in $old) { Invoke-Bcdedit @('/delete', $e.Id) }
    Set-StartsFirst '{bootmgr}'
    return $old.Count
}

# ------------------------------------------------------------------ shim and MokManager
function Get-OwnShim {
    # Shim and MokManager (os\release\shim), checked against their SHA-256 (SHA256SUMS). Returns their paths by name.
    param([Parameter(Mandatory = $true)][string]$RepoRoot)
    $dir = [IO.Path]::Combine($RepoRoot, 'os', 'release', 'shim')
    $sums = Join-Path $dir 'SHA256SUMS'
    if (-not (Test-Path -LiteralPath $sums)) {
        throw ('Shim is not in this folder (' + $dir + '). Download the whole Chat Computer folder again.')
    }
    $files = @{}
    foreach ($line in [IO.File]::ReadAllLines($sums)) {
        $parts = @($line.Trim() -split '\s+')
        if ($parts.Count -ne 2) { continue }
        $path = Join-Path $dir $parts[1]
        if (-not (Test-Path -LiteralPath $path) -or (Get-OwnSha256 $path) -ne $parts[0].ToLowerInvariant()) {
            throw ('Shim in this folder is damaged (' + $parts[1] + '''s checksum is wrong). Download the Chat Computer folder again.')
        }
        $files[$parts[1]] = $path
    }
    foreach ($name in @($Script:OwnShim, $Script:OwnMokManager)) {
        if (-not $files.ContainsKey($name)) { throw ('Shim in this folder has no ' + $name + '. Download the Chat Computer folder again.') }
    }
    return $files
}

function Test-OwnShimTrusted {
    # Whether this PC's Secure Boot starts shim: its allowed list (db) has the certificate Microsoft signs shim with,
    # Microsoft Corporation UEFI CA 2011 (some PCs leave it out, trusting only Windows'), and its list of revoked ones
    # (dbx) doesn't. $null when db can't be read.
    $ca = [Text.Encoding]::ASCII.GetBytes('Microsoft Corporation UEFI CA 2011')
    try {
        $db = Get-SecureBootUEFI -Name db -ErrorAction Stop
    } catch {
        Write-SetupLog ('Reading the firmware''s db: ' + $_.Exception.Message)
        return $null
    }
    if (-not (Test-BytesContain ([byte[]](Get-Prop $db 'Bytes')) $ca)) { return $false }
    try {
        $dbx = Get-SecureBootUEFI -Name dbx -ErrorAction Stop
        if (Test-BytesContain ([byte[]](Get-Prop $dbx 'Bytes')) $ca) { return $false }
    } catch {
        Write-SetupLog ('Reading the firmware''s dbx: ' + $_.Exception.Message)
    }
    return $true
}

function New-OwnMokRequest {
    # MokNew and MokAuth as mokutil writes them, for MokManager: $Certificate (DER) in an EFI_SIGNATURE_LIST (X.509,
    # owned by shim's GUID), and the SHA-256 of that followed by $Password in UTF-16, which MokManager checks what's
    # typed against. (os/check.py's mok_request makes the same, and a virtual PC's MokManager takes it.)
    param([Parameter(Mandatory = $true)][byte[]]$Certificate, [Parameter(Mandatory = $true)][string]$Password)
    $list = New-Object 'System.Collections.Generic.List[byte]'
    $list.AddRange(([guid]'a5c059a1-94e4-4aa7-87b5-ab155c2bf072').ToByteArray())        # EFI_CERT_X509_GUID
    $list.AddRange([BitConverter]::GetBytes([uint32](28 + 16 + $Certificate.Length)))  # the list's size
    $list.AddRange([BitConverter]::GetBytes([uint32]0))                                 # its header's
    $list.AddRange([BitConverter]::GetBytes([uint32](16 + $Certificate.Length)))       # each signature's
    $list.AddRange(([guid]$Script:ShimLockGuid).ToByteArray())                           # its owner
    $list.AddRange($Certificate)
    $new = $list.ToArray()
    $typed = [Text.Encoding]::Unicode.GetBytes($Password)
    $all = New-Object byte[] ($new.Length + $typed.Length)
    [Array]::Copy($new, 0, $all, 0, $new.Length)
    [Array]::Copy($typed, 0, $all, $new.Length, $typed.Length)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $auth = $sha.ComputeHash($all) } finally { $sha.Dispose() }
    return [pscustomobject]@{ MokNew = $new; MokAuth = $auth }
}

function Initialize-OwnUefi {
    # Windows' calls for the firmware's variables (they need the SeSystemEnvironmentPrivilege an administrator has,
    # switched on for this process).
    if ('ChatComputerUefi' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class ChatComputerUefi {
    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    struct TokenPrivileges { public uint Count; public long Luid; public uint Attributes; }
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool SetFirmwareEnvironmentVariableExW(string name, string guid, byte[] value, uint size, uint attributes);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern uint GetFirmwareEnvironmentVariableExW(string name, string guid, byte[] buffer, uint size, out uint attributes);
    [DllImport("kernel32.dll")]
    static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr handle);
    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool LookupPrivilegeValueW(string system, string name, out long luid);
    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool AdjustTokenPrivileges(IntPtr token, bool disableAll, ref TokenPrivileges state, uint length, IntPtr previous, IntPtr returned);

    static void Allow() {
        IntPtr token;
        if (!OpenProcessToken(GetCurrentProcess(), 0x0028, out token)) throw new Win32Exception();
        try {
            long luid;
            if (!LookupPrivilegeValueW(null, "SeSystemEnvironmentPrivilege", out luid)) throw new Win32Exception();
            TokenPrivileges tp = new TokenPrivileges();
            tp.Count = 1;
            tp.Luid = luid;
            tp.Attributes = 2;
            if (!AdjustTokenPrivileges(token, false, ref tp, 0, IntPtr.Zero, IntPtr.Zero)) throw new Win32Exception();
            int error = Marshal.GetLastWin32Error();
            if (error != 0) throw new Win32Exception(error);
        } finally {
            CloseHandle(token);
        }
    }

    public static void Set(string name, string guid, byte[] value, uint attributes) {
        Allow();
        uint size = value == null ? 0u : (uint)value.Length;
        if (!SetFirmwareEnvironmentVariableExW(name, guid, value, size, attributes)) {
            int error = Marshal.GetLastWin32Error();
            if (value == null && error == 203) return;
            throw new Win32Exception(error);
        }
    }

    public static byte[] Get(string name, string guid) {
        Allow();
        byte[] buffer = new byte[65536];
        uint attributes;
        uint n = GetFirmwareEnvironmentVariableExW(name, guid, buffer, (uint)buffer.Length, out attributes);
        if (n == 0) {
            int error = Marshal.GetLastWin32Error();
            if (error == 203) return null;
            throw new Win32Exception(error);
        }
        byte[] result = new byte[n];
        Array.Copy(buffer, result, (int)n);
        return result;
    }
}
'@
}

function Set-OwnUefiVariable {
    # A firmware variable under $Guid (shim's), kept and seen by the firmware and the OS (non-volatile, boot and
    # runtime: 7, as mokutil writes MokNew); $Value $null deletes it.
    param([Parameter(Mandatory = $true)][string]$Name, [AllowNull()][byte[]]$Value, [string]$Guid = $Script:ShimLockGuid)
    Initialize-OwnUefi
    [ChatComputerUefi]::Set($Name, '{' + $Guid + '}', $Value, 7)
}

function Get-OwnUefiVariable {
    # A firmware variable's bytes ($null if it isn't there).
    param([Parameter(Mandatory = $true)][string]$Name, [string]$Guid = $Script:ShimLockGuid)
    Initialize-OwnUefi
    $b = [ChatComputerUefi]::Get($Name, '{' + $Guid + '}')
    if ($null -eq $b) { return $null }
    return , $b
}

function Get-OwnStartedMark {
    # What Chat Computer's start-up stage last said when it started (its firmware variable: "secure-boot=on
    # shim=yes", say), or $null: none since the setup took it away, or the firmware won't say.
    try {
        $b = Get-OwnUefiVariable -Name $Script:OwnStartedVar -Guid $Script:OwnVarGuid
    } catch {
        Write-SetupLog ('Reading the start-up stage''s mark: ' + $_.Exception.Message)
        return $null
    }
    if ($null -eq $b) { return $null }
    return [Text.Encoding]::ASCII.GetString($b)
}

function Test-OwnStartedThroughShim {
    # Whether Chat Computer's own system has started through shim, with Secure Boot on, since its key's approval was
    # asked for (the setup takes the start-up stage's mark away then, and makes its log new): its start-up stage's mark
    # ($Mark) says so; with no mark (a firmware that won't keep one), the log has something in it ($Last, from
    # Read-OwnLastRun).
    param([AllowNull()][string]$Mark, $Last)
    if ($Mark) {
        $words = @($Mark.Trim() -split '\s+')
        return (($words -contains 'secure-boot=on') -and ($words -contains 'shim=yes'))
    }
    return ($null -ne $Last)
}

function Get-OwnMokState {
    # Where the approval of the key with $Thumbprint on MokManager's blue screen is, as the mark in $WorkDir has it:
    # 'enrolled' (it's in shim's list), 'shown' (asked for, and the PC restarted to the blue screen), 'noshim' (the
    # blue screen never came up: this PC doesn't start shim), else 'no'. Windows can't read shim's list (MokList), so
    # $Started (Test-OwnStartedThroughShim) is the proof that turns 'shown' into 'enrolled'.
    param([Parameter(Mandatory = $true)][string]$WorkDir, [Parameter(Mandatory = $true)][string]$Thumbprint, [bool]$Started)
    $path = Join-Path $WorkDir $Script:OwnMokMark
    $state = 'no'
    if (Test-Path -LiteralPath $path -PathType Leaf) {
        $m = [regex]::Match([IO.File]::ReadAllText($path), '^(enrolled|shown|noshim) ([0-9A-Fa-f]+)\s*$')
        if ($m.Success -and $m.Groups[2].Value -eq $Thumbprint) { $state = $m.Groups[1].Value }
    }
    if ($state -eq 'shown' -and $Started) {
        Set-OwnMokState -WorkDir $WorkDir -Thumbprint $Thumbprint -State 'enrolled'
        Write-SetupLog ('Shim knows the key ' + $Thumbprint + ': Chat Computer has started through it since its approval was asked for')
        return 'enrolled'
    }
    return $state
}

function Set-OwnMokState {
    param([Parameter(Mandatory = $true)][string]$WorkDir, [Parameter(Mandatory = $true)][string]$Thumbprint,
          [Parameter(Mandatory = $true)][ValidateSet('enrolled', 'shown', 'noshim')][string]$State)
    [IO.File]::WriteAllText((Join-Path $WorkDir $Script:OwnMokMark), ($State + ' ' + $Thumbprint + "`r`n"))
}

function Test-OwnMokRequest {
    # Whether the request for MokManager to enroll $Certificate (DER) is still there (MokNew, as Request-OwnMok writes
    # it): MokManager takes a request away whenever its blue screen comes up, approved or not (os/check.py --shim
    # checks it), so one still there after a restart to shim says the blue screen never came up. $null when the
    # firmware won't say.
    param([Parameter(Mandatory = $true)][byte[]]$Certificate)
    try {
        $there = Get-OwnUefiVariable -Name 'MokNew'
    } catch {
        Write-SetupLog ('Reading MokNew: ' + $_.Exception.Message)
        return $null
    }
    if ($null -eq $there) { return $false }
    $ours = (New-OwnMokRequest -Certificate $Certificate -Password $Script:OwnMokPassword).MokNew
    return ([BitConverter]::ToString([byte[]]$there) -eq [BitConverter]::ToString($ours))
}

function Request-OwnMok {
    # Asks shim's MokManager to enroll $Certificate (DER) at the next start through shim: MokNew and MokAuth, with
    # the password $Script:OwnMokPassword. The start-up stage's mark from before goes, so one there later says it has
    # started since.
    param([Parameter(Mandatory = $true)][byte[]]$Certificate)
    $request = New-OwnMokRequest -Certificate $Certificate -Password $Script:OwnMokPassword
    Set-OwnUefiVariable -Name $Script:OwnStartedVar -Value $null -Guid $Script:OwnVarGuid
    Set-OwnUefiVariable -Name 'MokNew' -Value $request.MokNew
    Set-OwnUefiVariable -Name 'MokAuth' -Value $request.MokAuth
    Write-SetupLog ('Asked shim to enroll the key (MokNew, ' + $request.MokNew.Length + ' bytes)')
}

function Clear-OwnMokRequest {
    # Takes the request for MokManager to enroll $Certificate away, if it's still there (a request of someone else's,
    # for a Linux of theirs, stays).
    param([Parameter(Mandatory = $true)][byte[]]$Certificate)
    if ((Test-OwnMokRequest -Certificate $Certificate) -ne $true) { return }
    foreach ($name in @('MokNew', 'MokAuth')) {
        try { Set-OwnUefiVariable -Name $name -Value $null } catch { Write-SetupLog ('Deleting ' + $name + ': ' + $_.Exception.Message) }
    }
    Write-SetupLog 'Took the request for the blue screen away'
}

function Write-OwnMokSteps {
    # Approving Chat Computer's key on MokManager's blue screen, once: the steps, in its own words (as os/check.py
    # --shim answers it).
    Write-Line 'On it, with the arrow keys and Enter:'
    Write-Line '  1. Press any key when it says "Press any key to perform MOK management" (it waits 10 seconds).'
    Write-Line '  2. Choose "Enroll MOK", then "Continue", then "Yes".'
    Write-Line ('  3. Type the password ' + $Script:OwnMokPassword + ' and press Enter.')
    Write-Line ('  4. Choose "Reboot". As the PC starts again, press F12 and choose "' + $Script:OwnEntryName + '".')
    Write-Line '(If the 10 seconds go by, it says "Verification failed": press Enter, and in Windows double-click this again.)'
}

# ------------------------------------------------------------------ the start-up partition and the power-on menu
function Get-OwnBootEntries {
    return , @(Get-FirmwareBootEntries | Where-Object { $_.Path -match ('(?i)^\\EFI\\' + $Script:OwnDir + '\\') -and $_.Id -ne '{fwbootmgr}' })
}

function Get-OwnBootEntry {
    # Chat Computer's own system's entry in the power-on menu, starting $Path (\EFI\...): made if it isn't there, its
    # path set if it starts another file (shim, or the start-up file itself). Returns its {id}.
    param([string]$Path = ('\EFI\' + $Script:OwnDir + '\' + $Script:OwnFile))
    $entries = Get-OwnBootEntries
    if ($entries.Count -gt 0) {
        if ([string]$entries[0].Path -ne $Path) { Invoke-Bcdedit @('/set', $entries[0].Id, 'path', $Path) }
        return $entries[0].Id
    }
    return (New-FirmwareBootEntry -Name $Script:OwnEntryName -Path $Path)
}

function Assert-OwnRoom {
    # Throws, saying so, when the partition at $Root has no room for files of $Size bytes in all, in place of those at
    # $Paths (ours there now, if any), and a megabyte more, for the settings and the certificate. (Chat Computer's own
    # system is 10 MB since it carries Chinese, Japanese and Korean; shim and MokManager are 2 MB.) Nothing's said when
    # Windows won't say how much is free.
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)][string[]]$Paths,
          [Parameter(Mandatory = $true)][int64]$Size)
    $free = Get-OwnFreeSpace $Root
    if ($null -eq $free) { return }
    $there = [int64]0
    foreach ($p in $Paths) { if (Test-Path -LiteralPath $p -PathType Leaf) { $there += (Get-Item -LiteralPath $p).Length } }
    if ($free + $there -lt $Size + 1MB) {
        throw ('The partition this PC starts from has ' + [math]::Floor($free / 1MB) + ' MB free, and Chat Computer''s ' +
            'own system needs ' + [math]::Ceiling(($Size + 1MB - $there) / 1MB) + ' MB there.')
    }
}

function Install-OwnFiles {
    # The signed start-up file, the key's certificate, the settings and the memory file, in \EFI\chatcomputer-own on
    # the partition Windows starts from. The settings are written anew (a model named in them by hand is kept; an
    # older setup's Claude API key isn't). With $Shim (shim's and MokManager's paths, from Get-OwnShim), they go
    # there too, and the start-up file as grubx64.efi, the name shim starts. With -FreshLog, the log file is a new
    # one, so the last run's log has something in it only once the own system has run since. Returns whether there's
    # a memory file (Memory), and the file the power-on menu's entry starts (Start, \EFI\...).
    param([Parameter(Mandatory = $true)][string]$Signed, [Parameter(Mandatory = $true)][string]$Certificate,
          [hashtable]$Shim, [switch]$FreshLog)
    $esp = Get-WindowsEsp
    if (-not $esp) { throw 'Windows could not find the partition this PC starts from.' }
    $m = Mount-EspPartition $esp
    try {
        $dir = [IO.Path]::Combine($m.Root, 'EFI', $Script:OwnDir)
        if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
        $put = [ordered]@{}
        if ($Shim) {
            $put[$Script:OwnShimNext] = $Signed
            $put[$Script:OwnShim] = $Shim[$Script:OwnShim]
            $put[$Script:OwnMokManager] = $Shim[$Script:OwnMokManager]
            $start = $Script:OwnShim
        } else {
            $put[$Script:OwnFile] = $Signed
            $start = $Script:OwnFile
        }
        $ours = @($Script:OwnFile, $Script:OwnShimNext, $Script:OwnShim, $Script:OwnMokManager | ForEach-Object { Join-Path $dir $_ })
        $size = [int64]0
        foreach ($from in $put.Values) { $size += (Get-Item -LiteralPath $from).Length }
        Assert-OwnRoom -Root $m.Root -Paths $ours -Size $size
        # (The other way's files go: the start-up file itself, or shim's.)
        foreach ($name in @($Script:OwnFile, $Script:OwnShimNext, $Script:OwnShim, $Script:OwnMokManager)) {
            $p = Join-Path $dir $name
            if (-not $put.Contains($name) -and (Test-Path -LiteralPath $p -PathType Leaf)) { Remove-Item -LiteralPath $p -Force }
        }
        foreach ($name in $put.Keys) {
            $to = Join-Path $dir $name
            Copy-Item -LiteralPath $put[$name] -Destination $to -Force
            if ((Get-OwnSha256 $to) -ne (Get-OwnSha256 $put[$name])) { throw 'The copy on the start-up partition is not the same as the original.' }
        }
        Copy-Item -LiteralPath $Certificate -Destination (Join-Path $dir $Script:OwnCert) -Force
        # (And at the partition's top: the setup screen's list of files starts there, so it's the first thing on it
        # when the key is approved.)
        Copy-Item -LiteralPath $Certificate -Destination (Join-Path $m.Root $Script:OwnCert) -Force
        $settings = Join-Path $dir $Script:OwnSettings
        $before = ''
        if (Test-Path -LiteralPath $settings) { $before = [IO.File]::ReadAllText($settings) }
        $model = Get-OwnSettingsModel $before
        $memoryId = Install-OwnMemory -Dir $dir -Root $m.Root -Id (Get-OwnSettingsMemoryId $before)
        $logWas = $(if ($FreshLog) { $null } else { Get-OwnSettingsMemoryId $before -Name 'log-id' })
        $logId = Install-OwnLog -Dir $dir -Root $m.Root -Id $logWas
        $text = New-OwnSettingsText -Model $model -ClockOffsetMinutes (Get-ClockOffsetMinutes) -MemoryId $memoryId `
            -LogId $logId -Cores (Get-OwnSettingsCores $before)
        [IO.File]::WriteAllText($settings, $text, [Text.Encoding]::ASCII)
        Write-SetupLog ('Copied to ' + $dir + $(if ($Shim) { ', through shim' } else { '' }) + '; settings ' +
                        $(if ($model) { 'naming a model' } else { 'naming no model' }))
        return [pscustomobject]@{ Memory = ($memoryId -ne 0); Start = ('\EFI\' + $Script:OwnDir + '\' + $start) }
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
        $top = Join-Path $m.Root $Script:OwnCert
        if (Test-Path -LiteralPath $top -PathType Leaf) { Remove-Item -LiteralPath $top -Force }
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
    # Approving Chat Computer's key in the PC's setup screen, once: the steps, in the setup screen's own words.
    # $Maker is the motherboard's maker, whose menus they name. (MSI's, as MSI's setup screen and ccboot.com's guide
    # to adding a key there have them: Settings, Security, Secure Boot; Secure Boot Mode Custom; Key Management;
    # Authorized Signatures; Append; No, to load a file; the file system; the file; Public Key Certificate; the
    # owner's GUID as it is; Yes.) Its certificate is at the top of the start-up partition (Install-OwnFiles), the
    # first list of files the setup screen shows.
    param([string]$Maker)
    $msi = $Maker -match '(?i)micro-star|\bmsi\b'
    Write-Line 'In the setup screen:'
    if ($msi) {
        Write-Line '  1. If the top of the screen says "Advanced (F7)", press F7.'
        Write-Line '  2. Open Settings, then Security, then Secure Boot. Set "Secure Boot Mode" to Custom, then open'
        Write-Line '     "Key Management".'
    } elseif ($Maker -match '(?i)gigabyte') {
        Write-Line '  1. If it opens in a simple view, switch to the advanced one ("Advanced Mode").'
        Write-Line '  2. Open the Boot tab, then "Secure Boot". Set "Secure Boot Mode" to Custom, then open'
        Write-Line '     "Expert Key Management".'
    } else {
        Write-Line '  1. If it opens in a simple view, switch to the advanced one ("Advanced Mode").'
        Write-Line '  2. Find "Secure Boot" (under Boot, or Security). If there''s a "Secure Boot Mode", set it to Custom.'
        Write-Line '     Then open "Key Management" (on some PCs "Expert Key Management").'
    }
    Write-Line '  3. Choose "Authorized Signatures" (on some PCs "DB Management"), then "Append".'
    Write-Line '  4. Answer No (to load it from a file). Choose the drive it lists, then ChatComputer.cer, then'
    Write-Line '     "Public Key Certificate". If it asks for a GUID, just press Enter. Answer Yes.'
    if ($msi) {
        Write-Line '  5. Press F10 and answer Yes. The PC restarts into Chat Computer.'
    } else {
        Write-Line '  5. Save and exit (F10 on most PCs). The PC restarts into Chat Computer.'
    }
    Write-Line 'Only Append: Update, Set or Delete there would take out Microsoft''s keys, which Windows needs. Leave'
    Write-Line '"Secure Boot Mode" on Custom afterwards. If something goes wrong, "Restore Factory Keys" (or "Enroll all'
    Write-Line 'Factory Default Keys") there puts the PC''s own keys back. If there''s no Append, set Secure Boot itself to'
    Write-Line 'Disabled on the same page instead.'
    Write-Line ('From then on, choosing "' + $Script:OwnEntryName + '" in the boot menu as the PC starts (often F12, F11,')
    Write-Line 'F8 or Esc) starts Chat Computer, every time.'
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
    # (As ClaudeOS's own system was, its owner asked: "press yes/enter in 1-2 times and it immediately restarts to
    # the OS". So: Windows' permission, then Enter, and it restarts into it, with nothing asked on the way. What it
    # does is said in a few lines, and the rest is in its log.)
    Write-Step 'Getting Chat Computer ready'
    if ((Get-FirmwareType) -eq 'Legacy') {
        throw 'This PC starts in the old BIOS mode, and Chat Computer needs UEFI. Nothing was changed.'
    }
    $last = Read-OwnLastRun -WorkDir $WorkDir
    if ($last) {
        Write-Line $last.Summary
        Write-SetupLog ('Its whole log: ' + $last.Path)
    }
    $release = Get-OwnRelease -RepoRoot $RepoRoot
    $sb = Get-SecureBootState
    $parts = Save-PcParts -Path (Join-Path $WorkDir 'pc.txt') -SecureBoot $sb
    Write-SetupLog ('This PC''s parts, for writing its drivers: ' + $parts)

    # The key: one this PC's Secure Boot already trusts (the one it approved for ClaudeOS, say), so it starts with
    # nothing to approve; else Chat Computer's own; else a new one.
    $keys = Get-OwnKeys
    $trusted = $null
    if ($sb -eq $true) { $trusted = { param($k) Test-OwnKeyApproved -Cert $k } }
    $cert = Select-OwnKey -Keys $keys -Trusted $trusted
    if (-not $cert) { $cert = New-OwnKey }
    $signed = Join-Path $WorkDir $Script:OwnFile
    $cer = Join-Path $WorkDir $Script:OwnCert
    New-OwnSignedFile -Release $release -Destination $signed -Cert $cert
    [IO.File]::WriteAllBytes($cer, [byte[]]$cert.RawData)
    if ($Out) {
        if (-not (Test-Path -LiteralPath $Out)) { New-Item -ItemType Directory -Path $Out -Force | Out-Null }
        Copy-Item -LiteralPath $signed -Destination $Out -Force
        Copy-Item -LiteralPath $cer -Destination $Out -Force
    }
    $approved = $null
    if ($sb -eq $true) { $approved = Test-OwnKeyApproved -Cert $cert }
    # With Secure Boot on and the key not in the firmware's own list: through shim (which Microsoft signs, as for
    # Linux), the key approved once on its blue screen, MokManager's; on a PC whose Secure Boot doesn't start shim, in
    # the PC's setup screen.
    $shim = $null
    $mok = 'no'
    $shimTrusted = $null
    if ($sb -eq $true -and $approved -ne $true) {
        $mok = Get-OwnMokState -WorkDir $WorkDir -Thumbprint $cert.Thumbprint `
            -Started (Test-OwnStartedThroughShim -Mark (Get-OwnStartedMark) -Last $last)
        if ($mok -eq 'shown' -and (Test-OwnMokRequest -Certificate ([byte[]]$cert.RawData)) -eq $true) {
            Write-SetupLog 'The request was still there after the restart to shim: the blue screen never came up'
            Clear-OwnMokRequest -Certificate ([byte[]]$cert.RawData)
            Set-OwnMokState -WorkDir $WorkDir -Thumbprint $cert.Thumbprint -State 'noshim'
            $mok = 'noshim'
        }
        $shimTrusted = Test-OwnShimTrusted
        Write-SetupLog ('Shim: its signer trusted: ' + $shimTrusted + '; the key''s approval on the blue screen: ' + $mok)
        if ($mok -ne 'noshim' -and $shimTrusted -ne $false) { $shim = Get-OwnShim -RepoRoot $RepoRoot }
    }
    $blueScreen = ($null -ne $shim -and $mok -ne 'enrolled')
    $setupScreen = ($sb -eq $true -and $approved -ne $true -and $null -eq $shim)
    $old = [string](Get-Prop $cert 'FriendlyName') -eq $Script:OldOwnKeyName
    if ($sb -eq $false) {
        Write-Good 'Signed with this PC''s own key. (Secure Boot is off, so it starts with any key.)'
    } elseif ($sb -ne $true) {
        Write-Good 'Signed with this PC''s own key.'
    } elseif ($approved -eq $true -and $old) {
        Write-Good 'Signed with the key this PC approved for ClaudeOS, so Secure Boot lets it start.'
    } elseif ($approved -eq $true) {
        Write-Good 'Signed with Chat Computer''s key, which this PC approved, so Secure Boot lets it start.'
    } elseif ($shim -and -not $blueScreen) {
        Write-Good 'Signed with Chat Computer''s key, which this PC approved on the blue screen, so Secure Boot lets it start.'
    } else {
        Write-Line 'Signed with Chat Computer''s key, which this PC hasn''t approved yet.'
        foreach ($k in $keys) {
            if ([string](Get-Prop $k 'FriendlyName') -eq $Script:OldOwnKeyName -and (Test-OwnKeyApproved -Cert $k) -eq $false) {
                Write-Line 'The key it approved for ClaudeOS isn''t on its list any more (updating or resetting the BIOS can clear it).'
                break
            }
        }
    }

    # Its model: the one Chat Computer's app got ready on this PC (Qwen, run on the PC itself).
    $model = Find-OwnModel
    $bitlocker = $setupScreen -and (Get-BitLockerOn)
    $installed = Install-OwnFiles -Signed $signed -Certificate $cer -Shim $shim -FreshLog:$blueScreen
    if (-not $installed.Memory) {
        Write-Warn 'Its memory file couldn''t be made on the start-up partition (too little room, or it couldn''t be written), so the conversation won''t be kept across restarts.'
    }
    $id = Get-OwnBootEntry -Path $installed.Start
    Write-Good 'Put on the partition this PC starts from, next to Windows'' own files (which stay as they are).'
    if ((Remove-OldClaudeOsEntries) -gt 0) {
        Write-Good 'ClaudeOS is out of the boot menu, so only Windows and Chat Computer are there. (Its files stay on the drive.)'
    }
    if ($model) {
        Write-Line ('It answers with ' + $model.Label + ', on this PC itself: nothing you type leaves it.')
    } else {
        Write-Line 'It has no model on this PC yet, so it starts and says how to get one (Chat Computer''s app, from'
        Write-Line 'chatcomputer.ai/get, gets it).'
    }

    if (-not $setupScreen -and -not $blueScreen) {
        if ($NoRestart) { return }
        Write-Line ''
        Write-Line ('To come back to Windows, restart. To start Chat Computer again, choose "' + $Script:OwnEntryName + '" in the')
        Write-Line 'boot menu as the PC starts (F12 on many PCs).'
        if (-not $Unattended) {
            Clear-OwnKeys
            $answer = ([string](Read-Host 'Press Enter to start Chat Computer (or close this window to stay in Windows)')).Trim()
            if ($answer -match '^(?i)n') {
                Write-SetupLog 'Stayed in Windows'
                Write-Line 'Staying in Windows.'
                return
            }
        }
        Invoke-Bcdedit @('/set', '{fwbootmgr}', 'bootsequence', $id)
        Write-SetupLog 'Restarting into Chat Computer'
        & shutdown.exe /r /t 0
        return
    }

    if ($blueScreen) {
        # (Secure Boot is on, the firmware's own list doesn't have the key, and the firmware starts shim: the key is
        # approved once on shim's blue screen. Nothing changes before Enter: closing the window leaves the next start to
        # Windows. BitLocker isn't paused: Secure Boot's own lists, which it checks as Windows starts, stay as they are;
        # the key goes in shim's.)
        Write-Step 'Once: approve Chat Computer''s key on a blue screen'
        Write-Line 'Secure Boot only starts what this PC knows. Press Enter, and the PC restarts to a blue screen that asks to'
        Write-Line 'approve Chat Computer''s key: once, with Secure Boot still on.'
        Write-OwnMokSteps
        Write-Line 'From then on, Enter here or the boot menu starts Chat Computer, with nothing to approve.'
        Write-Line ''
        if ($NoRestart) { return }
        if (-not $Unattended) {
            Write-Line 'Take a photo of these steps: this window won''t be there. (To stay in Windows, close this window.)'
            Clear-OwnKeys
            $answer = ([string](Read-Host 'Press Enter to restart to the blue screen')).Trim()
            if ($answer -match '^(?i)n') {
                Write-SetupLog 'Stayed in Windows before the blue screen'
                Write-Line 'Staying in Windows. Double-click "Chat Computer Own System" again whenever you''re ready.'
                return
            }
        }
        try {
            Request-OwnMok -Certificate ([byte[]]$cert.RawData)
        } catch {
            # (Then this PC gets the other way, in its setup screen, the next time.)
            Write-SetupLog ('Asking for the blue screen: ' + $_.Exception.ToString())
            Set-OwnMokState -WorkDir $WorkDir -Thumbprint $cert.Thumbprint -State 'noshim'
            Write-Warn ('Windows wouldn''t ask for the blue screen (' + $_.Exception.Message + '), so nothing was restarted.')
            Write-Line 'Double-click "Chat Computer Own System" again: it shows the other way, in the PC''s setup screen.'
            return
        }
        Set-OwnMokState -WorkDir $WorkDir -Thumbprint $cert.Thumbprint -State 'shown'
        Invoke-Bcdedit @('/set', '{fwbootmgr}', 'bootsequence', $id)
        Write-SetupLog 'Restarting to the blue screen'
        & shutdown.exe /r /t 0
        return
    }

    # (Secure Boot is on, and it doesn't trust any key this PC has: it needs one approval in the PC's setup screen,
    # which nothing in Windows can do for it. No choice is asked: approving the key keeps Secure Boot on, for Windows
    # and for games whose anti-cheat needs it; turning Secure Boot off is in the steps, for a PC that can't add a key.
    # Nothing changes before Enter: closing the window leaves the next start to Windows, and BitLocker as it was.)
    Write-Step 'Once: approve Chat Computer''s key in the PC''s setup screen'
    if ($mok -eq 'noshim' -or $shimTrusted -eq $false) {
        Write-Line 'The blue screen for approving it doesn''t come up on this PC, so it''s approved in the setup screen instead.'
    }
    Write-Line 'Secure Boot stops Chat Computer until the PC knows its key ("Secure Boot Violation"). Approve it once, and'
    Write-Line 'from then on Chat Computer starts with Enter here, or from the boot menu, with Secure Boot still on.'
    $maker = ''
    try { $maker = [string](Get-Prop (Get-CimInstance -ClassName Win32_BaseBoard -ErrorAction Stop | Select-Object -First 1) 'Manufacturer') } catch { $null = $_ }
    Write-Line ''
    Write-ApproveKeySteps -Maker $maker
    Write-Line ''
    if ($NoRestart) { return }
    if (-not $Unattended) {
        Write-Line 'Take a photo of these steps: this window won''t be there. (To stay in Windows, close this window.)'
        Clear-OwnKeys
        $answer = ([string](Read-Host 'Press Enter to restart into the setup screen')).Trim()
        if ($answer -match '^(?i)n') {
            Write-SetupLog 'Stayed in Windows before the setup screen'
            Write-Line 'Staying in Windows. Double-click "Chat Computer Own System" again whenever you''re ready.'
            return
        }
    }
    if ($bitlocker) {
        try {
            Suspend-BitLocker -MountPoint 'C:' -RebootCount 2 -ErrorAction Stop | Out-Null
            Write-Good 'BitLocker is paused for two restarts, so the change doesn''t make it ask for its recovery key.'
        } catch {
            Write-SetupLog ('Suspend-BitLocker: ' + $_.Exception.Message)
            Write-Warn 'BitLocker could not be paused, so Windows may ask for its recovery key once after the change.'
        }
    }
    # (After the setup screen, the PC starts Chat Computer, once: the next start is Windows again.)
    Invoke-Bcdedit @('/set', '{fwbootmgr}', 'bootsequence', $id)
    Write-SetupLog 'Restarting into the setup screen'
    $out = & shutdown.exe /r /fw /t 0 2>&1
    if ($LASTEXITCODE -ne 0) {
        Write-SetupLog ('shutdown /fw: ' + (($out | Out-String).Trim()))
        Write-Line 'Windows can''t open the setup screen on this PC by itself: restart, and press the setup key as the PC'
        Write-Line 'starts (Del on MSI boards; often Del or F2 on others: the PC''s first screen usually says which).'
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
    # (And what it left for its approval on the blue screen: a request still waiting, its start-up stage's mark.)
    if ($cert) { Clear-OwnMokRequest -Certificate ([byte[]]$cert.RawData) }
    try { Set-OwnUefiVariable -Name $Script:OwnStartedVar -Value $null -Guid $Script:OwnVarGuid } catch { Write-SetupLog ('Deleting the start-up stage''s mark: ' + $_.Exception.Message) }
    if ($WorkDir) {
        $mark = Join-Path $WorkDir $Script:OwnMokMark
        if (Test-Path -LiteralPath $mark -PathType Leaf) { Remove-Item -LiteralPath $mark -Force }
    }
    if ($cert) {
        Remove-Item -LiteralPath ('Cert:\CurrentUser\My\' + $cert.Thumbprint) -DeleteKey -ErrorAction Stop
        Write-SetupLog ('Deleted the key ' + $cert.Thumbprint)
    }
    Write-Good 'Done: Chat Computer''s own system is off this PC.'
    if ($cert) {
        Write-Line 'If you approved its key (on the blue screen, or in the PC''s setup screen), the PC still lists it, but'
        Write-Line 'nothing can be signed with it any more. ("Restore Factory Keys" in the setup screen takes it off the setup'
        Write-Line 'screen''s list.)'
    }
}
