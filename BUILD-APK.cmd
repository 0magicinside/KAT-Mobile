@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Build.ps1"
if errorlevel 1 echo BUILD NIJE PROSAO - pregledaj poruku iznad.
pause
