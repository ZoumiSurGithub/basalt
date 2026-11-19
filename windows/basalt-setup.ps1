# Basalt - installateur et outil de mise a jour pour Windows.
#
# Double-cliquez sur Installer.cmd, Mettre-a-jour.cmd ou Desinstaller.cmd,
# ou lancez ce script a la main :
#
#   powershell -ExecutionPolicy Bypass -File basalt-setup.ps1 -Action Install
#   powershell -ExecutionPolicy Bypass -File basalt-setup.ps1 -Action Update
#   powershell -ExecutionPolicy Bypass -File basalt-setup.ps1 -Action Uninstall [-Purge]
#
# Deux sources possibles pour le serveur :
#   - le code source de ce dossier (par defaut) : il est compile sur place ;
#   - un depot GitHub qui publie des versions : -Repo proprietaire/depot
#
# Vos fichiers (server.properties, plugins) ne sont jamais ecrases.
# (Messages sans accents : Windows PowerShell 5 lit mal l'UTF-8 sans BOM.)

param(
    [ValidateSet('Install', 'Update', 'Uninstall')]
    [string]$Action = 'Install',
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Basalt'),
    [string]$Repo = '',
    [switch]$Purge,
    [switch]$NoFirewall,
    [switch]$NoShortcut,
    [switch]$Yes
)

$ErrorActionPreference = 'Stop'
# Outils et bibliotheques compilees, gardes d'une mise a jour a l'autre
$CacheDir = Join-Path $env:LOCALAPPDATA 'Basalt-build'
# Affichage en UTF-8, pour que les accents (les notres et ceux de winget) soient lisibles
try {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    $OutputEncoding = [System.Text.Encoding]::UTF8
} catch {}
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$SourceDir   = Split-Path -Parent $PSScriptRoot
$StateFile   = Join-Path $InstallDir 'install.json'
$ExePath     = Join-Path $InstallDir 'basalt.exe'
$RuleName    = 'Basalt (serveur Minecraft)'
# Archives LuaBinaries contenant lua54.dll (64 bits), essayees dans l'ordre
$LuaUrls     = @(
    'https://sourceforge.net/projects/luabinaries/files/5.4.2/Tools%20Executables/lua-5.4.2_Win64_bin.zip/download',
    'https://sourceforge.net/projects/luabinaries/files/5.4.2/Windows%20Libraries/Dynamic/lua-5.4.2_Win64_dllw6_lib.zip/download'
)
$RustupUrl   = 'https://win.rustup.rs/x86_64'

# Affichage en couleur : l'étiquette en cyan, les réussites en vert, les avertissements en
# jaune, les erreurs en rouge.
function Tag { Write-Host "[Basalt] " -ForegroundColor Cyan -NoNewline }
function Say($text) {
    Tag
    $good = $text -match 'terminée?|installée?s?\b|à jour|réussie?|prêt|créée?s?\b|ajoutée?s?\b|supprimée?s?\b|désinstallée?'
    $bad = $text -match "pas |n'|impossible|échou|introuvable|absent|manqu"
    if ($good -and -not $bad) {
        Write-Host $text -ForegroundColor Green
    } else {
        Write-Host $text
    }
}
function Step($text) { Write-Host ""; Tag; Write-Host $text -ForegroundColor White }
function Warn($text) { Tag; Write-Host $text -ForegroundColor Yellow }
function Fail($text) { Tag; Write-Host "ERREUR : $text" -ForegroundColor Red; exit 1 }

function Confirm-Step($question) {
    if ($Yes) { return $true }
    Tag
    Write-Host "$question (o/n) " -ForegroundColor Magenta -NoNewline
    $answer = Read-Host
    return ($answer -match '^[oOyY]')
}

function Get-State {
    if (Test-Path $StateFile) {
        try { return Get-Content $StateFile -Raw | ConvertFrom-Json } catch { }
    }
    return $null
}

function Save-State($version, $source) {
    $state = [ordered]@{
        version   = $version
        source    = $source
        repo      = $Repo
        updated   = (Get-Date).ToString('s')
    }
    $state | ConvertTo-Json | Set-Content -Path $StateFile -Encoding ASCII
}

function Stop-Server {
    $procs = Get-Process -Name 'basalt' -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and ($_.Path -ieq $ExePath) }
    if (-not $procs) { return }
    Warn 'Le serveur est en cours d''exécution.'
    if (-not (Confirm-Step 'L''arrêter maintenant ?')) { Fail 'Arrêtez le serveur (commande stop) puis relancez.' }
    $procs | Stop-Process -Force
    Start-Sleep -Seconds 1
}

# --- Source 1 : compilation du code de ce dossier ---------------------------

function Find-Cargo {
    $cmd = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $local = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (Test-Path $local) { return $local }
    return $null
}

function Install-Rust {
    Say 'Rust (le compilateur) n''est pas installé.'
    if (-not (Confirm-Step 'Le telecharger et l''installer depuis rustup.rs (environ 300 Mo) ?')) {
        Fail 'Rust est nécessaire pour compiler le serveur.'
    }
    $init = Join-Path $env:TEMP 'rustup-init.exe'
    Say 'Téléchargement de rustup...'
    Invoke-WebRequest -Uri $RustupUrl -OutFile $init -UseBasicParsing
    # La variante "gnu" fournit son propre editeur de liens : pas besoin de Visual Studio.
    & $init -y --profile minimal --default-host x86_64-pc-windows-gnu | Out-Host
    if ($LASTEXITCODE -ne 0) { Fail 'L''installation de Rust a échoué.' }
    Remove-Item $init -ErrorAction SilentlyContinue
}

function Get-SourceVersion {
    $toml = Join-Path $SourceDir 'Cargo.toml'
    $line = Select-String -Path $toml -Pattern '^version\s*=\s*"(.+)"' | Select-Object -First 1
    if ($line) { return $line.Matches[0].Groups[1].Value }
    return 'inconnue'
}

# Les bibliotheques de NetherNet ont besoin de dlltool.exe (outils MinGW), absent de Rust seul.
function Find-Dlltool {
    $cmd = Get-Command dlltool.exe -ErrorAction SilentlyContinue
    if ($cmd) { return (Split-Path $cmd.Source) }
    $roots = @(
        (Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages'),
        'C:\msys64\ucrt64\bin', 'C:\msys64\mingw64\bin', 'C:\mingw64\bin', 'C:\w64devkit\bin'
    )
    foreach ($r in $roots) {
        if (-not (Test-Path $r)) { continue }
        $hit = Get-ChildItem -Path $r -Filter dlltool.exe -Recurse -ErrorAction SilentlyContinue |
            Where-Object { Test-Path (Join-Path $_.DirectoryName 'as.exe') } | Select-Object -First 1
        if ($hit) { return $hit.DirectoryName }
    }
    return $null
}

# Variante legere : llvm-ar (composant llvm-tools de Rust, ~30 Mo) se comporte comme
# dlltool quand il porte ce nom. Evite de telecharger tout MinGW (~250 Mo).
function Get-LlvmDlltool {
    $rustup = Get-Command rustup -ErrorAction SilentlyContinue
    if (-not $rustup) {
        $local = Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe'
        if (Test-Path $local) { $rustup = $local } else { return $null }
    } else { $rustup = $rustup.Source }
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & $rustup component add llvm-tools 2>&1 | Out-Null
    $ErrorActionPreference = $old
    $root = Join-Path $env:USERPROFILE '.rustup\toolchains'
    if ($env:RUSTUP_HOME) { $root = Join-Path $env:RUSTUP_HOME 'toolchains' }
    $ar = Get-ChildItem -Path $root -Filter llvm-ar.exe -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $ar) { return $null }
    $dir = Join-Path $CacheDir 'tools'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Copy-Item $ar.FullName (Join-Path $dir 'dlltool.exe') -Force
    return $dir
}

function Install-WinLibs {
    $winget = Get-Command winget -ErrorAction SilentlyContinue
    if (-not $winget) { return $null }
    Say 'Installation des outils MinGW (environ 250 Mo, une seule fois)...'
    & winget install --id BrechtSanders.WinLibs.POSIX.UCRT -e --source winget --accept-source-agreements --accept-package-agreements | Out-Host
    return (Find-Dlltool)
}

function Use-Dlltool($dir) {
    $kept = ($env:Path -split ';') | Where-Object { $_ -and ($_ -ne (Join-Path $CacheDir 'tools')) }
    $env:Path = (@($dir) + $kept) -join ';'
}

# Renvoie 'mingw', 'llvm' ou $null
function Ensure-Dlltool {
    $dir = Find-Dlltool
    if ($dir) { Use-Dlltool $dir; return 'mingw' }
    $dir = Get-LlvmDlltool
    if ($dir) { Use-Dlltool $dir; return 'llvm' }
    $dir = Install-WinLibs
    if ($dir) { Use-Dlltool $dir; return 'mingw' }
    Warn 'dlltool.exe est introuvable : NetherNet ne pourra pas être compilé.'
    Warn 'Installez-le avec : winget install BrechtSanders.WinLibs.POSIX.UCRT'
    return $null
}

# Renvoie un dossier contenant basalt.exe et plugins\
function Build-FromSource {
    if (-not (Test-Path (Join-Path $SourceDir 'Cargo.toml'))) {
        Fail "Code source introuvable dans $SourceDir. Utilisez -Repo propriétaire/dépôt."
    }
    $cargo = Find-Cargo
    if (-not $cargo) {
        Install-Rust
        $cargo = Find-Cargo
        if (-not $cargo) { Fail 'cargo reste introuvable après l''installation de Rust. Rouvrez une fenêtre et relancez.' }
    }
    $tool = Ensure-Dlltool
    Say 'Compilation du serveur (quelques minutes la première fois, bien plus rapide ensuite)...'
    $manifest = Join-Path $SourceDir 'Cargo.toml'
    $buildLog = Join-Path $SourceDir 'build-errors.txt'
    # Dossier de compilation fixe : les bibliotheques deja compilees sont reutilisees
    # d'une mise a jour a l'autre, meme si l'archive est dezippee ailleurs.
    $env:CARGO_TARGET_DIR = Join-Path $CacheDir 'target'
    # cargo ecrit sa progression sur la sortie d'erreur : ne pas la traiter comme un echec
    $oldPref = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & $cargo build --release --manifest-path $manifest 2>&1 | ForEach-Object { "$_" } | Tee-Object -FilePath $buildLog | Out-Host
    $code = $LASTEXITCODE
    if ($code -ne 0 -and $tool -eq 'llvm') {
        # La variante legere n'a pas suffi : on passe aux outils MinGW complets
        $dir = Install-WinLibs
        if ($dir) {
            Use-Dlltool $dir
            & $cargo build --release --manifest-path $manifest 2>&1 | ForEach-Object { "$_" } | Tee-Object -FilePath $buildLog | Out-Host
            $code = $LASTEXITCODE
        }
    }
    if ($code -ne 0) {
        # L'outil de capture est recent : en cas d'echec, on garde au moins le serveur complet
        Warn 'Compilation complete impossible : nouvel essai sans l''outil de capture...'
        Copy-Item $buildLog (Join-Path $SourceDir 'build-errors-capture.txt') -Force -ErrorAction SilentlyContinue
        & $cargo build --release --no-default-features --features nethernet --manifest-path $manifest 2>&1 | ForEach-Object { "$_" } | Tee-Object -FilePath $buildLog | Out-Host
        $code = $LASTEXITCODE
        if ($code -eq 0) {
            Warn "Serveur compile sans l'outil de capture. Envoyez le fichier build-errors-capture.txt pour le faire corriger."
        }
    }
    if ($code -ne 0) {
        Warn 'La compilation avec NetherNet (liaison WebRTC) a échoué.'
        Warn "Les erreurs sont enregistrées dans : $buildLog"
        Warn 'Nouvel essai sans NetherNet pour garder un serveur utilisable...'
        & $cargo build --release --no-default-features --manifest-path $manifest 2>&1 | ForEach-Object { "$_" } | Out-Host
        $code = $LASTEXITCODE
        if ($code -eq 0) {
            Warn 'Serveur compilé SANS NetherNet : les jeux 1.26.60 et plus ne pourront pas entrer.'
            Warn "Envoyez le fichier $buildLog pour faire corriger le problème."
        }
    } else {
        Remove-Item $buildLog -ErrorAction SilentlyContinue
    }
    $ErrorActionPreference = $oldPref
    if ($code -ne 0) {
        Warn 'Si l''erreur mentionne "link.exe", installez la variante gnu de Rust :'
        Warn '    rustup default stable-x86_64-pc-windows-gnu'
        Fail 'La compilation a échoué.'
    }
    $stage = Join-Path $env:TEMP ('basalt-stage-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path (Join-Path $stage 'plugins') | Out-Null
    Copy-Item (Join-Path $env:CARGO_TARGET_DIR 'release\basalt.exe') $stage
    Copy-Item (Join-Path $SourceDir 'plugins\*.lua') (Join-Path $stage 'plugins') -ErrorAction SilentlyContinue
    # Plugins fournis sous forme de dossier (EasyEdit...)
    Get-ChildItem -Path (Join-Path $SourceDir 'plugins') -Directory -ErrorAction SilentlyContinue | Where-Object { Test-Path (Join-Path $_.FullName 'plugin.yml') } | ForEach-Object {
        Copy-Item $_.FullName (Join-Path $stage 'plugins') -Recurse -Force
    }
    # Donnees du jeu (objets, blocs) : livrees avec le code source
    $data = Join-Path $SourceDir 'data'
    if (Test-Path $data) { Copy-Item $data (Join-Path $stage 'data') -Recurse -Force }
    $dll = Join-Path $PSScriptRoot 'lua54.dll'
    if (Test-Path $dll) { Copy-Item $dll $stage }
    return @{ Folder = $stage; Version = (Get-SourceVersion); Source = 'source' }
}

# --- Source 2 : derniere version publiee sur GitHub -------------------------

function Get-LatestRelease {
    $api = "https://api.github.com/repos/$Repo/releases/latest"
    try {
        return Invoke-RestMethod -Uri $api -Headers @{ 'User-Agent' = 'basalt-setup' }
    } catch {
        Fail "Impossible de lire les versions de $Repo : $($_.Exception.Message)"
    }
}

function Get-FromGitHub($release) {
    $asset = $release.assets | Where-Object { $_.name -like '*windows*.zip' } | Select-Object -First 1
    if (-not $asset) { Fail "La version $($release.tag_name) ne contient pas d'archive Windows (*windows*.zip)." }
    $zip = Join-Path $env:TEMP $asset.name
    $stage = Join-Path $env:TEMP ('basalt-stage-' + [Guid]::NewGuid().ToString('N'))
    Say "Téléchargement de $($asset.name)..."
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip -UseBasicParsing
    Expand-Archive -Path $zip -DestinationPath $stage -Force
    Remove-Item $zip -ErrorAction SilentlyContinue
    $exe = Get-ChildItem -Path $stage -Recurse -Filter 'basalt.exe' | Select-Object -First 1
    if (-not $exe) { Fail 'basalt.exe est absent de l''archive téléchargée.' }
    return @{ Folder = $exe.DirectoryName; Version = $release.tag_name; Source = 'github' }
}

# --- Installation des fichiers ----------------------------------------------

function Install-Files($package) {
    New-Item -ItemType Directory -Force -Path (Join-Path $InstallDir 'plugins') | Out-Null
    if (Test-Path $ExePath) {
        Copy-Item $ExePath "$ExePath.bak" -Force   # copie de secours de la version precedente
    }
    Copy-Item (Join-Path $package.Folder 'basalt.exe') $ExePath -Force

    $dll = Join-Path $package.Folder 'lua54.dll'
    if (Test-Path $dll) { Copy-Item $dll (Join-Path $InstallDir 'lua54.dll') -Force }

    # Plugins fournis : ajoutes seulement s'ils n'existent pas deja
    $plugins = Join-Path $package.Folder 'plugins'
    if (Test-Path $plugins) {
        Get-ChildItem -Path $plugins -Filter '*.lua' | ForEach-Object {
            $target = Join-Path $InstallDir ("plugins\" + $_.Name)
            if (-not (Test-Path $target)) { Copy-Item $_.FullName $target }
        }
        # Plugins fournis sous forme de dossier (EasyEdit...) : leur code suit la version du
        # serveur ; les autres fichiers de leur dossier (donnees) ne sont pas touches
        Get-ChildItem -Path $plugins -Directory | Where-Object { Test-Path (Join-Path $_.FullName 'plugin.yml') } | ForEach-Object {
            $target = Join-Path $InstallDir ("plugins\" + $_.Name)
            New-Item -ItemType Directory -Force -Path $target | Out-Null
            Copy-Item (Join-Path $_.FullName '*') $target -Recurse -Force
        }
    }

    # Donnees du jeu : toujours remplacees, elles suivent la version du serveur
    $data = Join-Path $package.Folder 'data'
    if (Test-Path $data) {
        $target = Join-Path $InstallDir 'data'
        New-Item -ItemType Directory -Force -Path $target | Out-Null
        Copy-Item (Join-Path $data '*') $target -Recurse -Force
    }

    $start = "@echo off`r`ntitle Basalt`r`ncd /d `"%~dp0`"`r`nbasalt.exe`r`necho.`r`npause`r`n"
    Set-Content -Path (Join-Path $InstallDir 'Demarrer Basalt.cmd') -Value $start -Encoding ASCII -NoNewline
    Save-State $package.Version $package.Source
}

function Install-Lua {
    $dll = Join-Path $InstallDir 'lua54.dll'
    if (Test-Path $dll) { return }
    Say 'Téléchargement de Lua 5.4 (nécessaire aux plugins)...'
    $lastError = ''
    foreach ($url in $LuaUrls) {
        $zip = Join-Path $env:TEMP 'basalt-lua.zip'
        $dir = Join-Path $env:TEMP ('basalt-lua-' + [Guid]::NewGuid().ToString('N'))
        try {
            Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing -UserAgent 'Wget/1.21'
            Expand-Archive -Path $zip -DestinationPath $dir -Force
            $found = Get-ChildItem -Path $dir -Recurse -Filter 'lua54.dll' | Select-Object -First 1
            if (-not $found) { throw 'lua54.dll absent de l''archive' }
            Copy-Item $found.FullName $dll
            Say 'Lua installé : les plugins sont actifs.'
            return
        } catch {
            $lastError = $_.Exception.Message
        } finally {
            Remove-Item $zip -Force -ErrorAction SilentlyContinue
            Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    Warn "Lua n'a pas pu être téléchargé ($lastError)."
    Warn "Le serveur fonctionne, mais sans plugins. Pour les activer, placez lua54.dll (64 bits) dans :"
    Warn "    $InstallDir"
}

function Add-Shortcuts {
    if ($NoShortcut) { return }
    $shell = New-Object -ComObject WScript.Shell
    $targets = @(
        (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Basalt.lnk'),
        (Join-Path ([Environment]::GetFolderPath('Programs')) 'Basalt.lnk')
    )
    foreach ($path in $targets) {
        $link = $shell.CreateShortcut($path)
        $link.TargetPath = Join-Path $InstallDir 'Demarrer Basalt.cmd'
        $link.WorkingDirectory = $InstallDir
        $link.Description = 'Serveur Minecraft Bedrock Basalt'
        $link.Save()
    }
}

function Remove-Shortcuts {
    foreach ($folder in @('Desktop', 'Programs')) {
        $path = Join-Path ([Environment]::GetFolderPath($folder)) 'Basalt.lnk'
        if (Test-Path $path) { Remove-Item $path -Force }
    }
}

# Lance une commande avec les droits administrateur (une fenetre Windows demande l'accord).
function Invoke-Elevated($command) {
    $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    try {
        if ($admin) {
            powershell -NoProfile -EncodedCommand $encoded | Out-Null
        } else {
            Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden `
                -ArgumentList '-NoProfile', '-EncodedCommand', $encoded
        }
        return $true
    } catch {
        return $false
    }
}

function Add-Firewall {
    if ($NoFirewall) { return }
    if ((Get-State) -and (Get-NetFirewallRule -DisplayName $RuleName -ErrorAction SilentlyContinue)) { return }
    Say 'Ouverture du pare-feu pour le serveur (Windows va demander votre accord)...'
    $exe = $ExePath.Replace("'", "''")
    $command = @"
Remove-NetFirewallRule -DisplayName '$RuleName' -ErrorAction SilentlyContinue
New-NetFirewallRule -DisplayName '$RuleName' -Direction Inbound -Action Allow -Protocol UDP -Program '$exe' | Out-Null
New-NetFirewallRule -DisplayName '$RuleName' -Direction Inbound -Action Allow -Protocol TCP -Program '$exe' | Out-Null
CheckNetIsolation.exe LoopbackExempt -a -n=Microsoft.MinecraftUWP_8wekyb3d8bbwe | Out-Null
"@
    if (-not (Invoke-Elevated $command)) {
        Warn 'Pare-feu non modifié. Au premier lancement, autorisez basalt.exe quand Windows le demande.'
    }
}

function Remove-Firewall {
    if (-not (Get-NetFirewallRule -DisplayName $RuleName -ErrorAction SilentlyContinue)) { return }
    if (-not (Invoke-Elevated "Remove-NetFirewallRule -DisplayName '$RuleName'")) {
        Warn "La règle de pare-feu '$RuleName' n'a pas été supprimée."
    }
}

# --- Actions -----------------------------------------------------------------

function Get-Package {
    if ($Repo) {
        return Get-FromGitHub (Get-LatestRelease)
    }
    return Build-FromSource
}

function Do-Install {
    Say "Installation dans $InstallDir"
    Stop-Server
    $package = Get-Package
    Install-Files $package
    Install-Lua
    Add-Firewall
    Add-Shortcuts
    Say "Basalt $($package.Version) est installé."
    Say 'Démarrage : raccourci "Basalt" sur le Bureau.'
    Say 'Dans Minecraft : Jouer > Serveurs > Ajouter un serveur, adresse 127.0.0.1, port 19132.'
}

function Do-Update {
    $state = Get-State
    if (-not $state) {
        Warn 'Basalt n''est pas encore installé : installation.'
        Do-Install
        return
    }
    if (-not $Repo -and $state.repo) { $script:Repo = $state.repo }

    if ($Repo) {
        $release = Get-LatestRelease
        if ($release.tag_name -eq $state.version) {
            Say "Déjà à jour ($($state.version))."
            return
        }
        Say "Mise à jour : $($state.version) -> $($release.tag_name)"
        Stop-Server
        $package = Get-FromGitHub $release
    } else {
        Say "Mise à jour depuis le code source de $SourceDir"
        $package = Build-FromSource
        Stop-Server
    }
    Install-Files $package
    Install-Lua
    Add-Firewall
    Say "Basalt est à jour ($($package.Version)). La version précédente est gardée dans basalt.exe.bak."
    Say 'server.properties et vos plugins n''ont pas été modifiés.'
}

function Do-Uninstall {
    if (-not (Test-Path $InstallDir)) { Say 'Basalt n''est pas installé.'; return }
    Stop-Server
    Remove-Shortcuts
    Remove-Firewall
    if ($Purge) {
        if (-not (Confirm-Step "Supprimer aussi server.properties et les plugins de $InstallDir ?")) { return }
        Remove-Item $InstallDir -Recurse -Force
        Say 'Basalt et ses données ont été supprimés.'
    } else {
        foreach ($name in @('basalt.exe', 'basalt.exe.bak', 'lua54.dll', 'Demarrer Basalt.cmd', 'install.json')) {
            $path = Join-Path $InstallDir $name
            if (Test-Path $path) { Remove-Item $path -Force }
        }
        Say "Basalt est désinstallé. Vos réglages et plugins restent dans $InstallDir"
        Say '(ajoutez -Purge pour tout supprimer).'
    }
}

switch ($Action) {
    'Install'   { Do-Install }
    'Update'    { Do-Update }
    'Uninstall' { Do-Uninstall }
}
