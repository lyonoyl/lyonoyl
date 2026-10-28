@echo off
REM Runs the PowerShell script sitting beside this file.
REM
REM It exists because the window Windows opens by default is cmd, where a .ps1 cannot simply be
REM typed, and because the disc this arrives on is not always drive D. Double-click this file, or
REM run it from cmd, or from PowerShell - all three end in the same place.

net session >nul 2>&1
if errorlevel 1 goto notadmin

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0prepare-windows-vm.ps1"
echo.
pause
exit /b 0

:notadmin
echo.
echo This has to run as administrator.
echo Right-click this file and choose "Run as administrator".
echo.
pause
exit /b 1
