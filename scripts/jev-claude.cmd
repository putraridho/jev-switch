@echo off
rem jev-claude on Windows: runs jev-claude.ps1 beside it. See that file.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0jev-claude.ps1" %*
exit /b %ERRORLEVEL%
