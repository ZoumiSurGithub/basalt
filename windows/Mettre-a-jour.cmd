@echo off
chcp 65001 >nul
title Basalt - Update
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0basalt-setup.ps1" -Action Update %*
echo.
pause
