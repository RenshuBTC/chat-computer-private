@echo off
rem Chat Computer Setup: double-click this file.
rem Windows asks for permission to make changes (needed to prepare a disk and the installer).
setlocal
set "CHATCOMPUTER_SELF=%~f0"
set "SETUP=%~dp0setup.ps1"
if not exist "%SETUP%" set "SETUP=%~dp0windows\setup.ps1"
if not exist "%SETUP%" (
  echo Could not find setup.ps1 next to this file. Please extract the whole Chat Computer folder first.
  pause
  exit /b 1
)
net session >nul 2>&1
if errorlevel 1 (
  rem Not running as administrator yet: start again with permission, in a new window.
  powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "Start-Process -FilePath $env:CHATCOMPUTER_SELF -Verb RunAs"
  exit /b 0
)
title Chat Computer Setup
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%SETUP%" %*
echo.
pause
