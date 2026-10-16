# The setup conversation, step by step (setup.ps1 runs it; tests run it with a pretend PC).
# ASCII-only file (see Common.ps1).

function Stop-Setup {
    param([string]$Why)
    throw ($Why + ' No disks were changed.')
}

function Invoke-ClaudeOSSetup {
    param(
        [Parameter(Mandatory = $true)][string]$Here,
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$WorkDir,
        [string]$IsoPath,
        [switch]$PlanOnly,
        [string]$DocumentsDir
    )
    try { $host.UI.RawUI.WindowTitle = 'ClaudeOS Setup' } catch { $null = $_ }
    Write-Line ''
    Write-Host 'ClaudeOS Setup' -ForegroundColor White
    Write-Line 'This adds ClaudeOS to this PC''s power-on menu, next to Windows. Windows, your apps and your files stay'
    Write-Line 'as they are. You need about 70 GB of free disk space and the internet. No USB stick is needed.'
    if ($PlanOnly) { Write-Warn 'Plan only: nothing will be changed.' }

    $mode = Read-Choice 'What would you like to do?' @(
        'Install ClaudeOS on this PC',
        'Sign ClaudeOS in to Claude again (ClaudeOS is already installed)',
        'Choose what starts when the PC turns on: ClaudeOS or Windows',
        'Remove ClaudeOS from this PC') 0
    if ($mode -eq 1) {
        Invoke-SignInAgain -WorkDir $WorkDir -PlanOnly:$PlanOnly
        return
    }
    if ($mode -eq 2) {
        Invoke-StartupChoice -PlanOnly:$PlanOnly
        return
    }
    if ($mode -eq 3) {
        Remove-ClaudeOS -WorkDir $WorkDir -DocumentsDir $DocumentsDir -PlanOnly:$PlanOnly
        return
    }

    # ------------------------------------------------------------- 1. this PC
    Write-Step 'Checking this PC'
    $fw = Get-FirmwareType
    if ($fw -eq 'Legacy') {
        Stop-Setup ('This PC starts in the old "Legacy BIOS" mode. ClaudeOS needs UEFI mode. ' +
                    'Most PCs can switch in their firmware settings, but Windows would also need converting, so setup stops here.')
    }
    Write-Good 'Starts in UEFI mode.'
    if (Find-ClaudeOSInstall) {
        Stop-Setup ('ClaudeOS is already installed on this PC. To install it again, first choose "Remove ClaudeOS from this PC".')
    }
    # (These return their list as one object: @(...) around them would count that list as one item.)
    $leftInstallers = Find-InstallerPartitions
    $leftEntries = Get-SetupBootEntries
    if ($leftInstallers.Count -gt 0 -or $leftEntries.Count -gt 0) {
        Stop-Setup ('An installer from an earlier try is still on this PC. Choose "Remove ClaudeOS from this PC" first, ' +
                    'then install again.')
    }

    $sb = Get-SecureBootState
    if ($sb -eq $true) { Write-Good 'Secure Boot is on. That is fine: Ubuntu, which ClaudeOS runs on, supports it.' }
    elseif ($sb -eq $false) { Write-Line 'Secure Boot is off. That is fine.' }

    $bl = Get-BitLockerState
    $windowsFilesNote = $true
    if ($bl.Encrypted) {
        Write-Warn 'Your Windows drive is encrypted (BitLocker / Device encryption).'
        Write-Line 'Two things to know:'
        Write-Line '  * ClaudeOS will not be able to read your Windows files while the drive is encrypted.'
        Write-Line '    To let it, turn off "Device encryption" in Windows Settings first, then run setup again.'
        Write-Line '  * After ClaudeOS is installed, Windows may ask for the BitLocker recovery key once.'
        if ($bl.RecoveryPasswords.Count -gt 0) {
            Write-Line '    Your recovery key (write it down or take a photo now):'
            foreach ($k in $bl.RecoveryPasswords) { Write-Host ('      ' + $k) -ForegroundColor White }
        } else {
            Write-Line '    You can find it at https://aka.ms/myrecoverykey (your Microsoft account).'
        }
        if (-not (Read-YesNo 'Have you saved the recovery key somewhere other than this PC?' $false)) {
            Stop-Setup 'Please save the recovery key first, then run setup again.'
        }
        $windowsFilesNote = $false
    }

    if (Get-FastStartupEnabled) {
        Write-Line ''
        Write-Line 'Windows "Fast Startup" is on. It leaves the Windows drive half-closed, so ClaudeOS could read your'
        Write-Line 'files but not save to them. Turning it off makes Windows start a few seconds slower.'
        if (Read-YesNo 'Turn off Fast Startup?' $true) {
            if (-not $PlanOnly) { Disable-FastStartup }
            Write-Good 'Fast Startup is off.'
        }
    }

    $pods = Get-PairedAirPods
    if ($pods.Count -gt 0) {
        Write-Good ('Found ' + ($pods -join ', ') + '. ClaudeOS can take over the pairing, so they connect without pairing again.')
    } else {
        Write-Warn 'No AirPods are paired with Windows. Pair them in Windows Settings now if you can, so ClaudeOS can use them straight away.'
    }

    # ------------------------------------------------------------- 2. internet for the installer
    Write-Step 'Internet for the installer'
    $wifiSsid = ''
    $wifiPassword = ''
    $wifiAuth = 'WPA2PSK'
    $wired = Test-WiredConnection
    if ($wired) { Write-Good 'A network cable is connected; the installer will use it.' }
    $wifi = Get-WifiForInstaller
    if ($wifi -and $wifi.Usable) {
        Write-Good ('The installer will also join your Wi-Fi "' + $wifi.Ssid + '" with the password saved in Windows.')
        $wifiSsid = [string]$wifi.Ssid
        $wifiPassword = [string]$wifi.Password
        $wifiAuth = [string]$wifi.Authentication
    } elseif (-not $wired) {
        if ($wifi) {
            Write-Warn ('Your Wi-Fi "' + $wifi.Ssid + '" signs in with a user name (or its password could not be read).')
        }
        Write-Line 'The installer needs the internet. Plug in a network cable, or type a Wi-Fi network it can join.'
        $wifiSsid = (Read-Host 'Wi-Fi name (leave empty if you will use a cable)').Trim()
        if ($wifiSsid) { $wifiPassword = Read-Secret ('Password for "' + $wifiSsid + '"') }
    }

    # ------------------------------------------------------------- 3. where ClaudeOS goes
    # The space ClaudeOS gets also holds its installer at first (Installer.ps1); ClaudeOS takes that space
    # over when it first starts.
    Write-Step 'Where ClaudeOS goes'
    $instGb = [Math]::Round($Script:InstallerBytes / $Script:GB)
    $needBytes = $Script:MinInstallBytes + $Script:InstallerBytes
    $plan = Get-SystemDiskPlan
    $sysDisk = $plan.Disk
    $others = Get-OtherDiskCandidates
    $options = @()
    $kinds = @()
    if ($plan.FreeBytes -ge $needBytes) {
        $options += ('Next to Windows, in the free space on ' + (Get-DiskLabel $sysDisk) + ' (' + (Format-Size $plan.FreeBytes) + ' free)')
        $kinds += 'gap'
    } elseif ($plan.MaxShrink -ge $needBytes) {
        $options += ('Next to Windows, on ' + (Get-DiskLabel $sysDisk) + ': make room by shrinking C: (your files stay)')
        $kinds += 'shrink'
    }
    foreach ($d in $others) {
        if ([int64]$d.Size -lt $needBytes) { continue }
        $options += ('A whole separate disk: ' + (Get-DiskLabel $d) + ' (everything on it is erased)')
        $kinds += ('disk:' + $d.Number)
    }
    if ($options.Count -eq 0) {
        Stop-Setup ('There is not enough room: ClaudeOS needs ' + (Format-Size $needBytes) + ', and Windows must keep ' +
                    (Format-Size $Script:MinWindowsFreeBytes) + ' free. Free up space on C: or add a disk, then run setup again.')
    }
    $pick = 0
    if ($options.Count -gt 1) { $pick = Read-Choice 'Where should ClaudeOS go?' $options 0 } else { Write-Line $options[0] }
    $kind = $kinds[$pick]
    $shrinkBytes = [int64]0
    $eraseNumber = -1
    $targetDisk = $sysDisk
    if ($kind -eq 'shrink') {
        $maxGb = [Math]::Floor(($plan.MaxShrink - $Script:InstallerBytes) / $Script:GB)
        $suggest = [Math]::Min(64, $maxGb)
        Write-Line ('Windows can give ClaudeOS up to ' + $maxGb + ' GB, plus ' + $instGb + ' GB for its installer, and still keep ' +
                    (Format-Size $Script:MinWindowsFreeBytes) + ' free.')
        while ($true) {
            $a = Read-Host ('How many GB for ClaudeOS? (40 to ' + $maxGb + ', Enter = ' + $suggest + ')')
            if ([string]::IsNullOrWhiteSpace($a)) { $sizeGb = $suggest; break }
            $sizeGb = 0
            if ([int]::TryParse($a.Trim(), [ref]$sizeGb) -and $sizeGb -ge 40 -and $sizeGb -le $maxGb) { break }
            Write-Line ('Please type a number from 40 to ' + $maxGb + '.')
        }
        $shrinkBytes = [int64]$sizeGb * $Script:GB + $Script:InstallerBytes
    } elseif ($kind -like 'disk:*') {
        $eraseNumber = [int]($kind.Substring(5))
        $targetDisk = Get-Disk -Number $eraseNumber
        Write-Warn ('Everything on ' + (Get-DiskLabel $targetDisk) + ' will be erased:')
        Write-Line (Get-DiskVolumesText $eraseNumber)
        $typed = Read-Host ('To confirm, type the disk number (' + $eraseNumber + ')')
        if ($typed.Trim() -ne [string]$eraseNumber) { Stop-Setup 'The disk number did not match.' }
    }

    # ------------------------------------------------------------- 4. you
    Write-Step 'Your ClaudeOS account'
    $realName = Get-WindowsPersonName
    $userName = ConvertTo-LinuxUserName $env:USERNAME
    $windowsUser = Split-Path -Leaf $env:USERPROFILE
    $winTz = ''
    try { $winTz = (Get-TimeZone).Id } catch { $winTz = [TimeZoneInfo]::Local.Id }
    $tz = Get-IanaTimeZone $winTz
    Write-Line ('Name: ' + $realName + '   Account: ' + $userName + '   Time zone: ' + $tz)
    $recovery = New-RecoveryPassword
    $hash = Get-Sha512Crypt -Password $recovery
    $installId = New-InstallId

    # ------------------------------------------------------------- 5. Claude
    Write-Step 'Sign in to Claude'
    $token = ''
    $login = $null
    if ($PlanOnly) {
        Write-Line '(Plan only: skipping sign-in.)'
    } else {
        $exe = Get-ClaudeProgram -WorkDir $WorkDir
        $token = Invoke-ClaudeSignIn -ClaudeExe $exe
        $login = Read-PhoneSignIn -ClaudeExe $exe
    }

    # ------------------------------------------------------------- 6. Ubuntu
    Write-Step 'Ubuntu installer'
    $iso = $null
    if ($PlanOnly -and -not $IsoPath) {
        Write-Line '(Plan only: skipping the download.)'
    } else {
        $iso = Get-UbuntuIso -WorkDir $WorkDir -IsoPath $IsoPath
        $isoBytes = [int64](Get-Item -LiteralPath $iso).Length
        if ($isoBytes + $Script:GB -gt $Script:InstallerBytes) {
            Stop-Setup ('The Ubuntu installer is bigger than expected (' + (Format-Size $isoBytes) + ').')
        }
    }

    # ------------------------------------------------------------- 7. the answers file
    $template = [IO.File]::ReadAllText((Join-Path $Here 'autoinstall.template.yaml'))
    $yaml = New-AutoinstallYaml -Template $template -Account $userName -RealName $realName -PasswordHash $hash `
        -TimeZone $tz -InstallId $installId -WifiSsid $wifiSsid -WifiPassword $wifiPassword -WifiAuth $wifiAuth
    $settingsJson = New-SettingsJson -UserName $realName -WindowsUser $windowsUser -TimeZone $tz

    # ------------------------------------------------------------- 8. confirm
    Write-Step 'Ready'
    Write-Line 'This is what happens now:'
    $n = 1
    if ($shrinkBytes -gt 0) {
        Write-Line ('  ' + $n + '. C: shrinks by ' + (Format-Size $shrinkBytes) + ' to make room for ClaudeOS. Your files stay.'); $n++
    }
    if ($eraseNumber -ge 0) { Write-Line ('  ' + $n + '. ' + (Get-DiskLabel $targetDisk) + ' is erased for ClaudeOS.'); $n++ }
    Write-Line ('  ' + $n + '. The last ' + $instGb + ' GB of that space hold ClaudeOS''s installer for now, and "ClaudeOS setup"')
    Write-Line '     is added to the PC''s power-on menu.'
    Write-Line 'Then the PC restarts into the installer. ClaudeOS installs itself in the free space (about 20 to 40 minutes,'
    Write-Line 'without questions), becomes the first choice when the PC starts, with Windows second, and takes over the'
    Write-Line ('installer''s ' + $instGb + ' GB the first time it starts.')

    if ($PlanOnly) {
        $planDir = Join-Path $WorkDir 'plan'
        New-Item -ItemType Directory -Path $planDir -Force | Out-Null
        Write-TextFileLf -Path (Join-Path $planDir 'autoinstall.yaml') -Text $yaml
        Write-TextFileLf -Path (Join-Path $planDir 'settings.json') -Text $settingsJson
        Write-Good ('Plan only: nothing was changed. The installer answers are in ' + $planDir)
        return
    }
    if (-not (Read-YesNo 'Go ahead?' $false)) { Stop-Setup 'Stopped at your request.' }

    # ------------------------------------------------------------- 9. do it
    # The power-on entry first: if this PC won't take one, nothing else is changed.
    $entry = $null
    try {
        $entry = New-SetupBootEntry
    } catch {
        Stop-Setup ($_.Exception.Message + ' ClaudeOS needs that to start its installer.')
    }
    $shrunk = $false
    $erased = $false
    $inst = $null
    try {
        if ($shrinkBytes -gt 0) {
            Write-Step 'Making room on C:'
            Invoke-ShrinkWindows -ShrinkBytes $shrinkBytes
            $shrunk = $true
            Write-Good ('C: is ' + (Format-Size $shrinkBytes) + ' smaller; that space is now free for ClaudeOS.')
        }
        if ($eraseNumber -ge 0) {
            Write-Step ('Erasing disk ' + $eraseNumber)
            Invoke-EraseDisk -Number $eraseNumber
            $erased = $true
            Write-Good 'Done.'
        }
        Write-Step 'Making the installer'
        Update-Disk -Number $targetDisk.Number -ErrorAction SilentlyContinue
        $disk = Get-Disk -Number $targetDisk.Number
        $extents = Get-FreeExtents -Partitions @(Get-Partition -DiskNumber $targetDisk.Number) -DiskSize ([int64]$disk.Size)
        if ($extents.Count -eq 0) { throw 'Setup could not find the free space it just made.' }
        $place = Get-InstallerPlacement -Extent $extents[0]
        if ($place.ClaudeOSBytes -lt $Script:MinInstallBytes) { throw 'The free space is smaller than ClaudeOS needs.' }
        $inst = New-InstallerPartition -DiskNumber $targetDisk.Number -Offset $place.Offset -Size $place.Size
        try {
            Write-InstallerMark -Root $inst.Root -InstallId $installId
            Copy-IsoToInstaller -IsoPath $iso -Root $inst.Root
            Write-InstallerPayload -Root $inst.Root -RepoRoot $RepoRoot -AutoinstallYaml $yaml -SettingsJson $settingsJson `
                -Token $token -InstallId $installId -Login $login
            $esp = Get-WindowsEsp
            if (-not $esp) { throw 'Setup could not find Windows'' EFI partition.' }
            $m = Mount-EspPartition $esp
            try { Install-SetupLoader -InstallerRoot $inst.Root -EspRoot $m.Root } finally { Dismount-EspPartition $m }
        } finally {
            Close-InstallerPartition $inst
        }
        # Start it once, at the next restart; after that the PC starts what it started before.
        Invoke-Bcdedit @('/set', '{fwbootmgr}', 'bootsequence', $entry)
    } catch {
        $why = $_.Exception.Message
        Write-SetupLog ('Install step failed: ' + $why)
        Write-Line 'Something went wrong; setup is taking back what it did...'
        $left = Undo-InstallSteps -Installer $inst -Shrunk:$shrunk
        $msg = 'Setup stopped: ' + $why + ' Windows starts normally.'
        if ($left -contains 'installer' -or $left -contains 'entry') {
            $msg += ' To delete what setup made, run setup again and choose "Remove ClaudeOS from this PC".'
        } elseif ($left -contains 'space') {
            $msg += (' C: is still ' + (Format-Size $shrinkBytes) + ' smaller: to give that back, open Disk Management, ' +
                     'right-click C: and choose "Extend Volume".')
        } elseif ($shrunk) {
            $msg += ' C: has its space back.'
        }
        if ($erased) { $msg += (' Disk ' + $eraseNumber + ' was erased and is empty now.') }
        elseif (-not $shrunk) { $msg += ' No disks were changed.' }
        throw $msg
    }
    $token = $null
    $login = $null
    Write-Good 'The installer is ready.'

    # ------------------------------------------------------------- 10. recovery password and restart
    $docs = $DocumentsDir
    if (-not $docs) { $docs = [Environment]::GetFolderPath('MyDocuments') }
    $recoveryFile = Join-Path $docs 'ClaudeOS recovery password.txt'
    $note = "ClaudeOS recovery password`r`n`r`nAccount: " + $userName + "`r`nPassword: " + $recovery + "`r`n`r`n" +
            "You normally never need this: ClaudeOS starts without a password. It is for repairs only.`r`n"
    [IO.File]::WriteAllText($recoveryFile, $note)
    Write-Step 'Last step'
    Write-Line ('Your ClaudeOS recovery password (for repairs only) is saved in: ' + $recoveryFile)
    Write-Line ''
    Write-Line 'The next time this PC restarts, it starts the installer, which runs by itself. When it finishes, the PC'
    Write-Line 'restarts into ClaudeOS.'
    Write-Line 'If Windows starts again instead, press the boot-menu key while the PC starts (often F12, F11, F8, F9 or Esc)'
    Write-Line 'and choose "ClaudeOS setup".'
    if ($windowsFilesNote) { Write-Line 'In ClaudeOS, say "switch to Windows" whenever you want Windows back.' }
    if (Read-YesNo 'Restart now?' $true) {
        & shutdown.exe /r /t 5
    }
}

function Undo-InstallSteps {
    # After a failed install step: takes back what this run of setup did, as far as it can. The installer's
    # power-on entry and boot loader go, then its partition, and if C: was shrunk for it, C: grows back. (A
    # separate disk that was erased stays empty.) Returns what is left: 'entry', 'installer' and/or 'space'.
    param($Installer, [switch]$Shrunk)
    $left = @()
    try { Remove-SetupBootEntries } catch { Write-SetupLog ('Undo, power-on entry: ' + $_.Exception.Message); $left += 'entry' }
    try { Clear-SetupLoader } catch { Write-SetupLog ('Undo, boot loader: ' + $_.Exception.Message) }
    if ($Installer) {
        try {
            Remove-InstallerPartition $Installer
            Write-SetupLog 'Undo: deleted the installer partition.'
        } catch {
            Write-SetupLog ('Undo, installer partition: ' + $_.Exception.Message)
            $left += 'installer'
        }
    }
    if ($Shrunk -and $left -notcontains 'installer') {
        try {
            $c = Get-Partition -DriveLetter C -ErrorAction Stop
            Update-Disk -Number $c.DiskNumber -ErrorAction SilentlyContinue
            $max = [int64](Get-PartitionSupportedSize -DriveLetter C -ErrorAction Stop).SizeMax
            if ($max -gt [int64]$c.Size + 1GB) {
                Resize-Partition -DriveLetter C -Size $max -ErrorAction Stop
                Write-SetupLog ('Undo: C: grew back to ' + $max + ' bytes.')
            } else {
                $left += 'space'
            }
        } catch {
            Write-SetupLog ('Undo, growing C: ' + $_.Exception.Message)
            $left += 'space'
        }
    }
    return , $left
}

function Read-PhoneSignIn {
    # Offers the full sign-in the Claude app on the phone needs. Returns it, or $null.
    param([Parameter(Mandatory = $true)][string]$ClaudeExe)
    Write-Line ''
    Write-Line 'Besides talking, you type to ClaudeOS in the Claude app on your phone: the phone is its keyboard.'
    Write-Line 'That needs one more approval in the browser.'
    if (-not (Read-YesNo 'Set up typing from your phone?' $true)) { return $null }
    return (Invoke-ClaudeLogin -ClaudeExe $ClaudeExe)
}

function Invoke-SignInAgain {
    # For when ClaudeOS says it isn't signed in (a sign-in lasts about a year), or to add the phone: sign
    # in here and leave the new sign-in where ClaudeOS looks on its next start, C:\Users\<you>\ClaudeOS
    # (claude_token, and claude_login.json for the phone). ClaudeOS takes them into its private folder
    # and deletes these copies.
    param([Parameter(Mandatory = $true)][string]$WorkDir, [switch]$PlanOnly)
    Write-Step 'Sign in to Claude'
    if ($PlanOnly) { Write-Line '(Plan only: skipping sign-in.)'; return }
    $exe = Get-ClaudeProgram -WorkDir $WorkDir
    $token = Invoke-ClaudeSignIn -ClaudeExe $exe
    $login = Read-PhoneSignIn -ClaudeExe $exe
    $dir = Join-Path $env:USERPROFILE 'ClaudeOS'
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $file = Join-Path $dir 'claude_token'
    Write-TextFileLf -Path $file -Text ($token + "`n")
    $token = $null
    if ($login) {
        Write-TextFileLf -Path (Join-Path $dir 'claude_login.json') -Text $login
        $login = $null
    }
    Write-Good 'Done. Restart into ClaudeOS; it picks up the new sign-in when it starts.'
    Write-Line ('If ClaudeOS still says it is not signed in, Windows'' Fast Startup may be keeping the drive locked:')
    Write-Line ('shut Windows down with Shift held on "Shut down", then start ClaudeOS. The sign-in waits in ' + $file + '.')
}
