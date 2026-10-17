@echo off
rem ClaudeOS Own System: double-click this file.
rem Starts ClaudeOS's own system (from scratch, in os\) on this PC, once, without a USB stick.
rem Windows asks for permission to make changes (needed for the start-up partition and the power-on menu).
setlocal
set "CLAUDEOS_SELF=%~f0"
set "OWNOS=%~dp0own-os.ps1"
if not exist "%OWNOS%" set "OWNOS=%~dp0windows\own-os.ps1"
if not exist "%OWNOS%" (
  echo Could not find own-os.ps1 next to this file. Please extract the whole ClaudeOS folder first.
  pause
  exit /b 1
)
net session >nul 2>&1
if errorlevel 1 (
  rem Not running as administrator yet: start again with permission, in a new window.
  powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "Start-Process -FilePath $env:CLAUDEOS_SELF -Verb RunAs"
  exit /b 0
)
title ClaudeOS Own System
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%OWNOS%" %*
echo.
pause
