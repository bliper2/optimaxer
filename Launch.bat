@echo off
rem Starts Optimaxer elevated (Windows PowerShell 5.1, STA).
cd /d "%~dp0"
powershell.exe -NoProfile -ExecutionPolicy Bypass -STA -WindowStyle Hidden -File "%~dp0Optimaxer.ps1"
