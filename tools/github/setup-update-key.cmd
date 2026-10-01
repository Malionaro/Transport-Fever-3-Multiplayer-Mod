@echo off
rem Double-click to set up the release update key (setup-update-key.ps1).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup-update-key.ps1" %*
echo.
pause
