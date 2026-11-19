@echo off
chcp 65001 >nul
title Basalt - Capture des donnees du jeu
cd /d "%LOCALAPPDATA%\Basalt"
echo Avant de lancer, dans le server.properties du serveur officiel de Mojang :
echo    transport=raknet
echo    online-mode=false
echo puis relancez-le sur ce PC (port 19132). Basalt doit etre arrete.
echo Le serveur officiel affichera une erreur "TRANSPORT TYPE" : c'est attendu.
echo.
basalt.exe capture 127.0.0.1 19132
echo.
pause
