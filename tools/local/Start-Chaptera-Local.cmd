@echo off
setlocal
cd /d "%~dp0\..\.."
pwsh -NoProfile -ExecutionPolicy Bypass -File "tools\local\start_chaptera_local.ps1"
if errorlevel 1 pause
