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
