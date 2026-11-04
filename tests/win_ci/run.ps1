# Chat Computer Setup's own tests on a real Windows machine, under Windows PowerShell 5.1 (the PowerShell every
# Windows PC has, and the one setup runs in): the unit tests, setup's copy of the installer from a real
# mounted disc image (iso.ps1), then the whole setup conversation on the pretend PC
# (windows/tests/Flow-Test.ps1) through the Python checks. Saves the output into -Out.
# Exits non-zero when anything failed. ASCII-only file.
param([Parameter(Mandatory = $true)][string]$Out)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Path $Out -Force | Out-Null
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$failed = $false

$ps = (Get-Command powershell.exe).Source
$unit = & $ps -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo 'windows\tests\Run-Tests.ps1') 2>&1
$code = $LASTEXITCODE
$unit | Out-File -FilePath (Join-Path $Out 'unit.txt') -Encoding utf8
Write-Host ('Unit tests (Windows PowerShell ' + $PSVersionTable.PSVersion + '): exit ' + $code + '; ' + ($unit | Select-Object -Last 1))
if ($code -ne 0) { $failed = $true }

$iso = & $ps -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'iso.ps1') -Out $Out 2>&1
$code = $LASTEXITCODE
Write-Host ('Copying the installer from a real disc image: exit ' + $code)
$iso | ForEach-Object { Write-Host ('  ' + $_) }
if ($code -ne 0) { $failed = $true }

& python -m pip install --quiet --disable-pip-version-check pytest pyyaml jsonschema 2>&1 | Out-Null
$env:CHATCOMPUTER_PWSH = $ps
$flow = & python -m pytest -q -p no:cacheprovider (Join-Path $repo 'windows\tests\test_windows_setup.py') -k 'flow or removing or windows_can or chatcomputer_can' 2>&1
$code = $LASTEXITCODE
$flow | Out-File -FilePath (Join-Path $Out 'flow.txt') -Encoding utf8
Write-Host ('Setup conversation on the pretend PC: exit ' + $code + '; ' + ($flow | Select-Object -Last 1))
if ($code -ne 0) { $failed = $true; $flow | Select-Object -Last 40 | ForEach-Object { Write-Host $_ } }

if ($failed) { exit 1 }
exit 0
