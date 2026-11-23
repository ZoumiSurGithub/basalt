#!/bin/sh
# Installation de Basalt sous Debian / Ubuntu (et autres Linux avec cargo déjà installé).
#
#   ./install.sh              compile et installe dans ./dist
#   sudo ./install.sh --service   installe dans /opt/basalt et crée un service systemd
set -eu

# Affichage en couleur quand la sortie est un terminal.
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    C_TAG='\033[36m'; C_OK='\033[32m'; C_WARN='\033[33m'; C_ERR='\033[31m'; C_END='\033[0m'
else
    C_TAG=''; C_OK=''; C_WARN=''; C_ERR=''; C_END=''
fi
say()  { printf '%b[Basalt]%b %s\n' "$C_TAG" "$C_END" "$*"; }
ok()   { printf '%b[Basalt]%b %b%s%b\n' "$C_TAG" "$C_END" "$C_OK" "$*" "$C_END"; }
warn() { printf '%b[Basalt]%b %b%s%b\n' "$C_TAG" "$C_END" "$C_WARN" "$*" "$C_END" >&2; }
fail() { printf '%b[Basalt]%b %bERREUR : %s%b\n' "$C_TAG" "$C_END" "$C_ERR" "$*" "$C_END" >&2; }

cd "$(dirname "$0")"
SERVICE=0
[ "${1:-}" = "--service" ] && SERVICE=1

if [ "$SERVICE" = 1 ] && [ "$(id -u)" != 0 ]; then
    fail "L'option --service doit être lancée avec sudo."
    exit 1
fi

# Dépendances : compilateur Rust + bibliothèque Lua 5.4 (pour les plugins)
if command -v apt-get >/dev/null 2>&1; then
    PKGS=""
    command -v cargo >/dev/null 2>&1 || PKGS="$PKGS cargo"
    command -v cc >/dev/null 2>&1 || PKGS="$PKGS gcc libc6-dev"
    ls /usr/lib/*/liblua5.4.so.0 >/dev/null 2>&1 || PKGS="$PKGS liblua5.4-0"
    command -v curl >/dev/null 2>&1 || PKGS="$PKGS curl ca-certificates"
    if [ -n "$PKGS" ]; then
        say "Installation des paquets :$PKGS"
        if [ "$(id -u)" = 0 ]; then
            apt-get update && apt-get install -y $PKGS
        else
            sudo apt-get update && sudo apt-get install -y $PKGS
        fi
    fi
elif ! command -v cargo >/dev/null 2>&1; then
    fail "cargo est introuvable. Installez Rust (https://rustup.rs) et Lua 5.4, puis relancez."
    exit 1
fi

# rustup installe cargo dans ~/.cargo/bin : il passe avant celui du système, souvent trop ancien
if [ -x "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
    export PATH
fi
# Le transport NetherNet demande un Rust récent ; celui de Debian 12 (1.63) ne suffit pas
CARGO_MINOR=$(cargo --version 2>/dev/null | sed -n 's/^cargo 1\.\([0-9][0-9]*\).*/\1/p')
if [ -z "$CARGO_MINOR" ] || [ "$CARGO_MINOR" -lt 85 ]; then
    fail "Rust trop ancien ($(cargo --version 2>/dev/null || echo 'version inconnue')) : il faut la version 1.85 ou plus."
    warn "Installez la version actuelle, puis relancez ce script :"
    warn "    curl https://sh.rustup.rs -sSf | sh -s -- -y"
    warn "    . \"\$HOME/.cargo/env\""
    warn "(si rustup est déjà installé : rustup update stable)"
    exit 1
fi
say "Compilation avec $(cargo --version)…"
# Cargo.lock fixe les versions des bibliothèques qui compilent : on s'y tient (--locked)
if ! cargo build --release --locked; then
    warn "La compilation avec les versions verrouillées a échoué : nouvel essai en les laissant évoluer."
    if ! cargo build --release; then
        fail "La compilation a échoué. Copiez les lignes d'erreur ci-dessus pour obtenir de l'aide."
        exit 1
    fi
fi

if [ "$SERVICE" = 1 ]; then
    DEST=/opt/basalt
    id basalt >/dev/null 2>&1 || useradd --system --home-dir "$DEST" --shell /usr/sbin/nologin basalt
else
    DEST=./dist
fi

mkdir -p "$DEST/plugins"
cp target/release/basalt "$DEST/basalt"
# Données du jeu (objets, blocs) : toujours remplacées, elles suivent la version du serveur
if [ -d data ]; then
    mkdir -p "$DEST/data"
    cp -r data/. "$DEST/data/"
fi
# Ne pas écraser les plugins déjà présents
for f in plugins/*.lua; do
    [ -e "$f" ] || continue
    [ -e "$DEST/plugins/$(basename "$f")" ] || cp "$f" "$DEST/plugins/"
done
# Plugins fournis sous forme de dossier (EasyEdit…) : leur code suit la version du serveur
for d in plugins/*/; do
    [ -f "$d/plugin.yml" ] || continue
    mkdir -p "$DEST/plugins/$(basename "$d")"
    cp -r "$d". "$DEST/plugins/$(basename "$d")/"
done

if [ "$SERVICE" = 1 ]; then
    chown -R basalt:basalt "$DEST"
    cp basalt.service /etc/systemd/system/basalt.service
    systemctl daemon-reload
    systemctl enable --now basalt
    ok "Service installé. Journal : journalctl -u basalt -f"
    warn "Pensez à ouvrir les ports 19132/tcp et 19140/udp dans votre pare-feu."
else
    ok "Terminé. Lancez le serveur avec : cd dist && ./basalt"
fi
