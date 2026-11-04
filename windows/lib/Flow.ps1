# The setup conversation, step by step (setup.ps1 runs it; tests run it with a pretend PC).
# On a PC without Chat Computer it asks as little as it can: everything it would change is in one list at the end,
# and nothing changes before "Go ahead?". Nothing to sign in to: Chat Computer thinks with a model on the PC itself,
# which it downloads by itself when it first starts.
# ASCII-only file (see Common.ps1).

function Stop-Setup {
    param([string]$Why)
    throw ($Why + ' No disks were changed.')
}

function Invoke-ChatComputerSetup {
    param(
        [Parameter(Mandatory = $true)][string]$Here,
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$WorkDir,
        [string]$IsoPath,
        [switch]$PlanOnly,
        [string]$DocumentsDir
    )
    try { $host.UI.RawUI.WindowTitle = 'Chat Computer Setup' } catch { $null = $_ }
    Write-Line ''
    Write-Host 'Chat Computer Setup' -ForegroundColor White
    Write-Line 'This adds Chat Computer to this PC''s power-on menu, next to Windows. Windows, your apps and your files stay'
    Write-Line 'as they are. You need about 70 GB of free disk space and the internet. No USB stick is needed.'
    if ($PlanOnly) { Write-Warn 'Plan only: nothing will be changed.' }

    # The menu only when Chat Computer (or an earlier try's installer) is already on this PC; otherwise: install.
    $installed = Find-ChatComputerInstall
    # (These return their list as one object: @(...) around them would count that list as one item.)
    $leftInstallers = Find-InstallerPartitions
    $leftEntries = Get-SetupBootEntries
    $mode = 0
    if ($installed -or $leftInstallers.Count -gt 0 -or $leftEntries.Count -gt 0) {
        $default = 0
        if ($installed) { $default = 1 }
        $mode = Read-Choice 'What would you like to do?' @(
            'Install Chat Computer on this PC',
            'Choose what starts when the PC turns on: Chat Computer or Windows',
            'Remove Chat Computer from this PC') $default
    }
    if ($mode -eq 1) {
        Invoke-StartupChoice -PlanOnly:$PlanOnly
        return
    }
    if ($mode -eq 2) {
        Remove-ChatComputer -WorkDir $WorkDir -DocumentsDir $DocumentsDir -PlanOnly:$PlanOnly
        return
    }

    # ------------------------------------------------------------- 1. this PC
    Write-Step 'Checking this PC'
    $fw = Get-FirmwareType
    if ($fw -eq 'Legacy') {
        Stop-Setup ('This PC starts in the old "Legacy BIOS" mode. Chat Computer needs UEFI mode. ' +
                    'Most PCs can switch in their firmware settings, but Windows would also need converting, so setup stops here.')
    }
    Write-Good 'Starts in UEFI mode.'
    if ($installed) {
        Stop-Setup ('Chat Computer is already installed on this PC. To install it again, first choose "Remove Chat Computer from this PC".')
    }
    if ($leftInstallers.Count -gt 0 -or $leftEntries.Count -gt 0) {
        Stop-Setup ('An installer from an earlier try is still on this PC. Choose "Remove Chat Computer from this PC" first, ' +
                    'then install again.')
    }

    $sb = Get-SecureBootState
    if ($sb -eq $true) { Write-Good 'Secure Boot is on. That is fine: Ubuntu, which Chat Computer runs on, supports it.' }
    elseif ($sb -eq $false) { Write-Line 'Secure Boot is off. That is fine.' }

    $bl = Get-BitLockerState
    if ($bl.Encrypted) {
        Write-Line 'Your Windows drive is encrypted (BitLocker / Device encryption). Two things to know:'
        Write-Line '  * Setup gives Chat Computer a BitLocker key of its own, so Chat Computer can open your Windows files.'
        Write-Line '    Your own keys stay as they are. Removing Chat Computer deletes its key again.'
        Write-Line '  * After Chat Computer is installed, Windows may ask for its recovery key once.'
        if ($bl.RecoveryPasswords.Count -gt 0) {
            Write-Line '    Your recovery key (write it down or take a photo now):'
            foreach ($k in $bl.RecoveryPasswords) { Write-Host ('      ' + $k) -ForegroundColor White }
        } else {
            Write-Line '    You can find it at https://aka.ms/myrecoverykey (your Microsoft account).'
        }
        if (-not (Read-YesNo 'Have you saved the recovery key somewhere other than this PC?' $false)) {
            Stop-Setup 'Please save the recovery key first, then run setup again.'
        }
    }

    # Fast Startup leaves the Windows drive half-closed, so Chat Computer could read your files but not save to them.
    # Turning it off is in the list at the end, with everything else that changes.
    $fastStartup = Get-FastStartupEnabled

    $pods = Get-PairedAirPods
    if ($pods.Count -gt 0) {
        Write-Good ('Found ' + ($pods -join ', ') + '. Chat Computer can take over the pairing, so they connect without pairing again.')
    } else {
        Write-Warn 'No AirPods are paired with Windows. Pair them in Windows Settings now if you can, so Chat Computer can use them straight away.'
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

    # ------------------------------------------------------------- 3. where Chat Computer goes
    # Next to Windows: the empty space right after C:, made bigger by shrinking C: (after "Go ahead?", in steps,
    # dealing with what Windows can't move: Invoke-MakeRoom). Or a whole separate disk.
    # The space Chat Computer gets also holds its installer at first (Installer.ps1); Chat Computer takes that space
    # over when it first starts.
    Write-Step 'Where Chat Computer goes'
    $instGb = [Math]::Round($Script:InstallerBytes / $Script:GB)
    $needBytes = $Script:MinInstallBytes + $Script:InstallerBytes
    $wantBytes = $Script:DefaultInstallBytes + $Script:InstallerBytes
    Write-Line 'Checking how much room Windows can make next to it (this can take a minute)...'
    $plan = Get-SystemDiskPlan
    $sysDisk = $plan.Disk
    $others = Get-OtherDiskCandidates
    $gap = [int64]$plan.GapAfterC
    $options = @()
    $kinds = @()
    $clearing = $false
    if ($plan.FreeBytes -ge $wantBytes -and $gap -lt $needBytes) {
        # Plenty of empty space elsewhere on the disk already (left by the person): use that as it is.
        $options += ('Next to Windows, in the empty space on ' + (Get-DiskLabel $sysDisk) + ' (' + (Format-Size $plan.FreeBytes) + ')')
        $kinds += 'free'
    } elseif ($gap + [int64]$plan.MaxShrinkByFreeSpace -ge $needBytes) {
        # C: has the free space. If Windows can't shrink C: that far right now, Invoke-MakeRoom deals with what's
        # in the way after "Go ahead?".
        $options += ('Next to Windows, on ' + (Get-DiskLabel $sysDisk) + ': room made from C: (your files stay)')
        $kinds += 'room'
        $clearing = ($gap + [int64]$plan.MaxShrink -lt $needBytes)
    }
    foreach ($d in $others) {
        if ([int64]$d.Size -lt $needBytes) { continue }
        $options += ('A whole separate disk: ' + (Get-DiskLabel $d) + ' (everything on it is erased)')
        $kinds += ('disk:' + $d.Number)
    }
    if ($options.Count -eq 0) {
        $short = $needBytes + $Script:MinWindowsFreeBytes - ($gap + [int64]$plan.WindowsFree)
        Stop-Setup ('There is not enough free space: C: has ' + (Format-Size $plan.WindowsFree) + ' free. Chat Computer needs ' +
                    (Format-Size $needBytes) + ', and Windows keeps ' + (Format-Size $Script:MinWindowsFreeBytes) + ' free for itself. ' +
                    'Free up about ' + (Format-Size ([Math]::Max($short, $Script:GB))) + ' on C: (Settings, System, Storage), or add a disk, ' +
                    'then run setup again.')
    }
    $pick = 0
    if ($options.Count -gt 1) { $pick = Read-Choice 'Where should Chat Computer go?' $options 0 } else { Write-Line $options[0] }
    $kind = $kinds[$pick]
    $eraseNumber = -1
    $targetDisk = $sysDisk
    if ($kind -eq 'room') {
        if ($gap -ge $wantBytes) {
            Write-Line ('Chat Computer gets the ' + (Format-Size $gap) + ' of empty space right after C:.')
        } else {
            Write-Line ('Chat Computer gets ' + (Format-Size ($wantBytes - $Script:InstallerBytes)) + ' if Windows can spare it, and at least ' +
                        (Format-Size $Script:MinInstallBytes) + '.')
        }
    } elseif ($kind -like 'disk:*') {
        $eraseNumber = [int]($kind.Substring(5))
        $targetDisk = Get-Disk -Number $eraseNumber
        Write-Warn ('Everything on ' + (Get-DiskLabel $targetDisk) + ' will be erased:')
        Write-Line (Get-DiskVolumesText $eraseNumber)
        $typed = Read-Host ('To confirm, type the disk number (' + $eraseNumber + ')')
        if ($typed.Trim() -ne [string]$eraseNumber) { Stop-Setup 'The disk number did not match.' }
    }

    # ------------------------------------------------------------- 4. you
    Write-Step 'Your Chat Computer account'
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

    # ------------------------------------------------------------- 5. Ubuntu
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

    # ------------------------------------------------------------- 6. the answers file
    $template = [IO.File]::ReadAllText((Join-Path $Here 'autoinstall.template.yaml'))
    $yaml = New-AutoinstallYaml -Template $template -Account $userName -RealName $realName -PasswordHash $hash `
        -TimeZone $tz -InstallId $installId -WifiSsid $wifiSsid -WifiPassword $wifiPassword -WifiAuth $wifiAuth
    $settingsJson = New-SettingsJson -UserName $realName -WindowsUser $windowsUser -TimeZone $tz

    # ------------------------------------------------------------- 7. confirm
    Write-Step 'Ready'
    Write-Line 'This is what happens now:'
    $n = 1
    if ($fastStartup) {
        Write-Line ('  ' + $n + '. Windows "Fast Startup" is turned off, so Chat Computer can save to your Windows files.'); $n++
        Write-Line '     (Windows then starts a few seconds slower.)'
    }
    if ($kind -eq 'room' -and $gap -lt $wantBytes) {
        Write-Line ('  ' + $n + '. C: shrinks by up to ' + (Format-Size ($wantBytes - $gap)) + ' to make room for Chat Computer. Your files stay.'); $n++
        if ($clearing) {
            Write-Line '     Windows keeps some of its own files at the end of C:. Setup moves them out of the way where it can:'
            Write-Line '     it resets the drive''s change journal (Windows starts a new one), and only if that is not enough,'
            Write-Line '     it deletes your restore points (Windows makes new ones) and turns hibernation off.'
        }
    }
    if ($eraseNumber -ge 0) { Write-Line ('  ' + $n + '. ' + (Get-DiskLabel $targetDisk) + ' is erased for Chat Computer.'); $n++ }
    Write-Line ('  ' + $n + '. The last ' + $instGb + ' GB of that space hold Chat Computer''s installer for now, and "Chat Computer setup"')
    Write-Line '     is added to the PC''s power-on menu.'
    Write-Line 'Then the PC restarts into the installer. Chat Computer installs itself in the free space (about 20 to 40 minutes,'
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

    # ------------------------------------------------------------- 8. do it
    # The power-on entry first: if this PC won't take one, nothing else is changed.
    $entry = $null
    try {
        $entry = New-SetupBootEntry
    } catch {
        Stop-Setup ($_.Exception.Message + ' Chat Computer needs that to start its installer.')
    }
    $cSizeBefore = [int64]$plan.Partition.Size
    $shrunk = $false
    $erased = $false
    $inst = $null
    $blKey = $null
    try {
        if ($kind -eq 'room' -and $gap -lt $wantBytes) {
            Write-Step 'Making room on C:'
            $made = Invoke-MakeRoom -NeedBytes $needBytes -WantBytes $wantBytes
            $shrunk = ($made.ShrunkBy -gt 0)
            if ($made.Gap -lt $needBytes) {
                throw (Get-RoomAdvice -Blocker $made.Blocker -Made $made.Gap -NeedBytes $needBytes)
            }
            Write-Good ('There is ' + (Format-Size $made.Gap) + ' of room for Chat Computer next to Windows.')
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
        # An erased disk may have no partitions at all, and Get-Partition reports that as an error.
        $extents = Get-FreeExtents -Partitions @(Get-Partition -DiskNumber $targetDisk.Number -ErrorAction SilentlyContinue) -DiskSize ([int64]$disk.Size)
        if ($extents.Count -eq 0) { throw 'Setup could not find the free space it just made.' }
        $place = Get-InstallerPlacement -Extent $extents[0]
        if ($place.ChatComputerBytes -lt $Script:MinInstallBytes) { throw 'The free space is smaller than Chat Computer needs.' }
        $inst = New-InstallerPartition -DiskNumber $targetDisk.Number -Offset $place.Offset -Size $place.Size
        try {
            Write-InstallerMark -Root $inst.Root -InstallId $installId
            Copy-IsoToInstaller -IsoPath $iso -Root $inst.Root
            if ($bl.Encrypted) {
                try {
                    $blKey = New-ChatComputerBitLockerKey
                    Write-Good 'Chat Computer has a BitLocker key of its own, so it can open your Windows files.'
                } catch {
                    Write-SetupLog ('BitLocker key: ' + $_.Exception.Message)
                    Write-Warn ('Setup could not make a BitLocker key for Chat Computer (' + $_.Exception.Message + '), so ' +
                                'Chat Computer will not be able to open your Windows files. Everything else works.')
                }
            }
            Write-InstallerPayload -Root $inst.Root -RepoRoot $RepoRoot -AutoinstallYaml $yaml -SettingsJson $settingsJson `
                -InstallId $installId -BitLockerKey $blKey
            $esp = Get-WindowsEsp
            if (-not $esp) { throw 'Setup could not find Windows'' EFI partition.' }
            $m = Mount-EspPartition $esp
            try { Install-SetupLoader -InstallerRoot $inst.Root -EspRoot $m.Root } finally { Dismount-EspPartition $m }
        } finally {
            Close-InstallerPartition $inst
        }
        if ($fastStartup) {
            Disable-FastStartup
            Write-Good 'Fast Startup is off.'
        }
        # Start it once, at the next restart; after that the PC starts what it started before.
        Invoke-Bcdedit @('/set', '{fwbootmgr}', 'bootsequence', $entry)
    } catch {
        $why = Get-ErrorText $_
        Write-SetupLog ('Install step failed: ' + $_.Exception.ToString())
        Write-Line 'Something went wrong; setup is taking back what it did...'
        $left = Undo-InstallSteps -Installer $inst -Shrunk:$shrunk -BitLockerKey:($null -ne $blKey) -CSize $cSizeBefore
        $msg = 'Setup stopped: ' + $why + ' Windows starts normally.'
        if ($left -contains 'installer' -or $left -contains 'entry') {
            $msg += ' To delete what setup made, run setup again and choose "Remove Chat Computer from this PC".'
        } elseif ($left -contains 'space') {
            $msg += ' C: is still smaller than before: to give it its space back, open Disk Management, right-click C: and ' +
                    'choose "Extend Volume".'
        } elseif ($shrunk) {
            $msg += ' C: has its space back.'
        }
        if ($erased) { $msg += (' Disk ' + $eraseNumber + ' was erased and is empty now.') }
        elseif (-not $shrunk) { $msg += ' No disks were changed.' }
        throw $msg
    }
    $blKey = $null
    Write-Good 'The installer is ready.'

    # ------------------------------------------------------------- 9. recovery password and restart
    $docs = $DocumentsDir
    if (-not $docs) { $docs = [Environment]::GetFolderPath('MyDocuments') }
    $note = "Chat Computer recovery password`r`n`r`nAccount: " + $userName + "`r`nPassword: " + $recovery + "`r`n`r`n" +
            "You normally never need this: Chat Computer starts without a password. It is for repairs only.`r`n"
    $recoveryFile = $null
    try {
        $recoveryFile = Join-Path $docs 'Chat Computer recovery password.txt'
        [IO.File]::WriteAllText($recoveryFile, $note)
    } catch {
        # The installer is ready by now, so this mustn't end in "Setup stopped": the password is shown instead.
        Write-SetupLog ('Saving the recovery password note: ' + $_.Exception.Message)
        $recoveryFile = $null
    }
    Write-Step 'Last step'
    if ($recoveryFile) {
        Write-Line ('Your Chat Computer recovery password (for repairs only) is saved in: ' + $recoveryFile)
    } else {
        Write-Warn 'Setup could not save your Chat Computer recovery password in your Documents folder. Write it down (it is for repairs only):'
        Write-Host ('  Account: ' + $userName + '   Password: ' + $recovery) -ForegroundColor White
    }
    Write-Line ''
    Write-Line 'The next time this PC restarts, it starts the installer, which runs by itself. When it finishes, the PC'
    Write-Line 'restarts into Chat Computer.'
    Write-Line 'If Windows starts again instead, press the boot-menu key while the PC starts (often F12, F11, F8, F9 or Esc)'
    Write-Line 'and choose "Chat Computer setup".'
    Write-Line 'In Chat Computer your Windows files are there to open and bring over. Say "switch to Windows" whenever you want'
    Write-Line 'Windows back.'
    if (Read-YesNo 'Restart now?' $true) {
        & shutdown.exe /r /t 5
    }
}

function Undo-InstallSteps {
    # After a failed install step: takes back what this run of setup did, as far as it can. The installer's
    # power-on entry and boot loader go, then Chat Computer's BitLocker key and the installer's partition, and if C:
    # was shrunk for it, C: grows back to the size it had ($CSize). (A separate disk that was erased stays
    # empty; restore points that were deleted stay deleted.) Returns what is left: 'entry', 'installer' and/or
    # 'space'.
    param($Installer, [switch]$Shrunk, [switch]$BitLockerKey, [int64]$CSize = 0)
    $left = @()
    try { Remove-SetupBootEntries } catch { Write-SetupLog ('Undo, power-on entry: ' + $_.Exception.Message); $left += 'entry' }
    try { Clear-SetupLoader } catch { Write-SetupLog ('Undo, boot loader: ' + $_.Exception.Message) }
    if ($BitLockerKey) {
        try { $null = Remove-ChatComputerBitLockerKey; Write-SetupLog 'Undo: deleted Chat Computer''s BitLocker key.' }
        catch { Write-SetupLog ('Undo, BitLocker key: ' + $_.Exception.Message) }
    }
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
            $target = $max
            if ($CSize -gt 0) { $target = [Math]::Min($max, $CSize) }
            if ($target -gt [int64]$c.Size + 1GB) {
                Resize-Partition -DriveLetter C -Size $target -ErrorAction Stop
                Write-SetupLog ('Undo: C: grew back to ' + $target + ' bytes.')
            } elseif ($target -lt $CSize - 1GB) {
                $left += 'space'
            }
        } catch {
            Write-SetupLog ('Undo, growing C: ' + $_.Exception.Message)
            $left += 'space'
        }
    }
    return , $left
}
