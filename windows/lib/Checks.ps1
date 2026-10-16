# Read-only checks of this PC before anything changes. Windows only.
# ASCII-only file (see Common.ps1).

function Test-IsAdministrator {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $p = New-Object Security.Principal.WindowsPrincipal($id)
    return $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-FirmwareType {
    # 'UEFI' or 'Legacy'. ClaudeOS needs UEFI (every PC sold since about 2012 has it).
    if ($env:firmware_type) { return $env:firmware_type }
    try {
        $v = (Get-ItemProperty -Path 'HKLM:\SYSTEM\CurrentControlSet\Control' -Name PEFirmwareType -ErrorAction Stop).PEFirmwareType
        if ($v -eq 2) { return 'UEFI' }
        if ($v -eq 1) { return 'Legacy' }
    } catch {
        $null = $_
    }
    return 'Unknown'
}

function Get-SecureBootState {
    # $true, $false, or $null when it can't be read.
    try { return [bool](Confirm-SecureBootUEFI -ErrorAction Stop) } catch { return $null }
}

function Get-BitLockerState {
    # Whether C: is encrypted, and its recovery password(s) so the person can save them.
    $state = [pscustomobject]@{ Encrypted = $false; Known = $false; RecoveryPasswords = @() }
    try {
        $v = Get-BitLockerVolume -MountPoint 'C:' -ErrorAction Stop
        $state.Known = $true
        $state.Encrypted = ([string]$v.VolumeStatus -ne 'FullyDecrypted')
        $state.RecoveryPasswords = @($v.KeyProtector | Where-Object { [string]$_.KeyProtectorType -eq 'RecoveryPassword' } |
                ForEach-Object { [string]$_.RecoveryPassword })
        return $state
    } catch {
        Write-SetupLog ('Get-BitLockerVolume unavailable: ' + $_.Exception.Message)
    }
    try {
        $wmi = Get-CimInstance -Namespace 'root\cimv2\Security\MicrosoftVolumeEncryption' -ClassName Win32_EncryptableVolume `
            -Filter "DriveLetter='C:'" -ErrorAction Stop
        if ($wmi) {
            $state.Known = $true
            $r = Invoke-CimMethod -InputObject $wmi -MethodName GetConversionStatus -ErrorAction Stop
            $state.Encrypted = ([int]$r.ConversionStatus -ne 0)
        }
    } catch {
        Write-SetupLog ('BitLocker status unavailable: ' + $_.Exception.Message)
    }
    return $state
}

# ------------------------------------------------------------------ ClaudeOS's own BitLocker key
# When C: is encrypted, ClaudeOS gets a recovery password of its own for it (a second key; the person's own keys
# stay as they are), so it can open the Windows files. Linux can open BitLocker with a recovery password, not
# through the PC's security chip. Its id is kept in ProgramData\ClaudeOS, so "Remove ClaudeOS" can delete exactly
# this key again. These use BitLocker's WMI class, which Windows Home (device encryption) has too.

function Get-ClaudeOSStateDir {
    if ($env:CLAUDEOS_STATE_DIR) { return $env:CLAUDEOS_STATE_DIR }     # tests
    return (Join-Path $env:ProgramData 'ClaudeOS')
}

function Get-SystemEncryptableVolume {
    $drive = $env:SystemDrive
    if (-not $drive) { $drive = 'C:' }
    $vol = Get-CimInstance -Namespace 'root\cimv2\Security\MicrosoftVolumeEncryption' -ClassName Win32_EncryptableVolume `
        -Filter ("DriveLetter='" + $drive + "'") -ErrorAction Stop
    if (-not $vol) { throw ('BitLocker does not know ' + $drive) }
    return $vol
}

function Assert-BitLockerResult {
    param($Result, [Parameter(Mandatory = $true)][string]$What)
    $code = [int64](Get-Prop $Result 'ReturnValue')
    if ($code -ne 0) { throw ($What + ' (BitLocker error 0x' + ('{0:X8}' -f $code) + ').') }
}

function New-ClaudeOSBitLockerKey {
    # Adds the key and records its id. Returns @{ Id = '{guid}'; Password = '000000-...-000000' }.
    $vol = Get-SystemEncryptableVolume
    $r = Invoke-CimMethod -InputObject $vol -MethodName ProtectKeyWithNumericalPassword `
        -Arguments @{ FriendlyName = 'ClaudeOS' } -ErrorAction Stop
    Assert-BitLockerResult $r 'BitLocker would not add a key for ClaudeOS'
    $id = [string](Get-Prop $r 'VolumeKeyProtectorID')
    if ($id -notmatch '^\{[0-9A-Fa-f-]{36}\}$') { throw 'BitLocker added a key but did not say which.' }
    try {
        $p = Invoke-CimMethod -InputObject $vol -MethodName GetKeyProtectorNumericalPassword `
            -Arguments @{ VolumeKeyProtectorID = $id } -ErrorAction Stop
        Assert-BitLockerResult $p 'BitLocker would not show the new key'
        $password = [string](Get-Prop $p 'NumericalPassword')
        if ($password -notmatch '^\d{6}(-\d{6}){7}$') { throw 'BitLocker showed the new key in an unexpected form.' }
        $dir = Get-ClaudeOSStateDir
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        Write-TextFileLf -Path (Join-Path $dir 'bitlocker-key-id') -Text ($id + "`n")
    } catch {
        $null = Invoke-CimMethod -InputObject $vol -MethodName DeleteKeyProtector -Arguments @{ VolumeKeyProtectorID = $id } -ErrorAction SilentlyContinue
        throw
    }
    Write-SetupLog ('Added ClaudeOS''s BitLocker key ' + $id)
    return [pscustomobject]@{ Id = $id; Password = $password }
}

function Remove-ClaudeOSBitLockerKey {
    # Deletes the key setup made for ClaudeOS, if there is one. Returns $true if a key was deleted. Only ever a
    # recovery password with the recorded id: never one of the person's own keys.
    $record = Join-Path (Get-ClaudeOSStateDir) 'bitlocker-key-id'
    if (-not (Test-Path -LiteralPath $record)) { return $false }
    $id = ([IO.File]::ReadAllText($record)).Trim()
    if ($id -notmatch '^\{[0-9A-Fa-f-]{36}\}$') {
        Remove-Item -LiteralPath $record -Force
        return $false
    }
    $vol = Get-SystemEncryptableVolume
    $t = Invoke-CimMethod -InputObject $vol -MethodName GetKeyProtectorType -Arguments @{ VolumeKeyProtectorID = $id } -ErrorAction Stop
    $deleted = $false
    if ([int64](Get-Prop $t 'ReturnValue') -eq 0) {
        # 3 = a recovery password ("numerical password").
        if ([int](Get-Prop $t 'KeyProtectorType') -ne 3) { throw 'The key setup recorded for ClaudeOS is not a recovery password.' }
        $r = Invoke-CimMethod -InputObject $vol -MethodName DeleteKeyProtector -Arguments @{ VolumeKeyProtectorID = $id } -ErrorAction Stop
        Assert-BitLockerResult $r 'BitLocker would not delete ClaudeOS''s key'
        $deleted = $true
        Write-SetupLog ('Deleted ClaudeOS''s BitLocker key ' + $id)
    } else {
        Write-SetupLog ('ClaudeOS''s BitLocker key ' + $id + ' was already gone')
    }
    Remove-Item -LiteralPath $record -Force
    return $deleted
}

function Get-FastStartupEnabled {
    try {
        $v = (Get-ItemProperty -Path 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power' -Name HiberbootEnabled -ErrorAction Stop).HiberbootEnabled
        return ($v -eq 1)
    } catch {
        return $false
    }
}

function Disable-FastStartup {
    # Fast Startup leaves the Windows drive half-closed, so ClaudeOS could only read it, never save to it.
    [Microsoft.Win32.Registry]::SetValue('HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Session Manager\Power',
        'HiberbootEnabled', 0, [Microsoft.Win32.RegistryValueKind]::DWord)
}

function Get-PairedAirPods {
    try {
        return , @(Get-PnpDevice -Class Bluetooth -ErrorAction Stop | Where-Object { ([string]$_.FriendlyName) -match 'AirPods' } |
                ForEach-Object { [string]$_.FriendlyName } | Sort-Object -Unique)
    } catch {
        return , @()
    }
}

function Get-WindowsPersonName {
    # The person's display name, e.g. "Ren Shu", or the account name if none is set.
    try {
        Add-Type -AssemblyName System.DirectoryServices.AccountManagement -ErrorAction Stop
        $n = [System.DirectoryServices.AccountManagement.UserPrincipal]::Current.DisplayName
        if ($n) { return [string]$n }
    } catch {
        $null = $_
    }
    try {
        $u = Get-LocalUser -Name $env:USERNAME -ErrorAction Stop
        if ($u.FullName) { return [string]$u.FullName }
    } catch {
        $null = $_
    }
    return [string]$env:USERNAME
}
