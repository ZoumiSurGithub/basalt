@echo off
chcp 65001 >nul
title Basalt - Install
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0basalt-setup.ps1" -Action Install %*
echo.
pause
