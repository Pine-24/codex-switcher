@echo off
setlocal
echo This diagnostic does not modify credentials or stop Codex.
echo It runs for 60 seconds. Switch accounts once in Pine while it observes.
echo The execution-policy override below applies to this PowerShell process only.
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0Diagnose-CodexAuth.ps1" -ObserveSeconds 60
echo.
echo Copy only the report between BEGIN and END. Do not send auth.json.
pause
endlocal
