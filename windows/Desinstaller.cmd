@echo off
chcp 65001 >nul
title Basalt - Uninstall
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0basalt-setup.ps1" -Action Uninstall %*
echo.
pause
