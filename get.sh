#!/bin/sh
# Installe ou met à jour Basalt dans le dossier courant, à partir de la dernière version
# publiée sur GitHub. Rien à compiler.
#
#   mkdir basalt && cd basalt
#   curl -sSfL https://raw.githubusercontent.com/ZoumiSurGithub/basalt/main/get.sh | sh
#
# Relancer la même commande (ou ./update.sh) met le serveur à jour. Ne sont jamais touchés :
# server.properties, le monde, les listes (ops, liste blanche), les clés et vos plugins.
#
# Options (après « sh -s -- ») :  -f  réinstalle même si la version est déjà la bonne
#                                 -v v0.2.0  installe cette version précise
#                                 -v dev (ou -d)  installe le dernier envoi sur la branche dev
set -eu

REPO="${BASALT_REPO:-ZoumiSurGithub/basalt}"
FORCE=0
WANTED=""
while [ $# -gt 0 ]; do
    case "$1" in
        -f) FORCE=1 ;;
        -v) shift; WANTED="${1:-}" ;;
        -d) WANTED="dev" ;;
        *) echo "Option inconnue : $1" >&2; exit 2 ;;
    esac
    shift
done

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    C_TAG='\033[36m'; C_OK='\033[32m'; C_WARN='\033[33m'; C_ERR='\033[31m'; C_END='\033[0m'
else
    C_TAG=''; C_OK=''; C_WARN=''; C_ERR=''; C_END=''
fi
say()  { printf '%b[Basalt]%b %s\n' "$C_TAG" "$C_END" "$*"; }
ok()   { printf '%b[Basalt]%b %b%s%b\n' "$C_TAG" "$C_END" "$C_OK" "$*" "$C_END"; }
warn() { printf '%b[Basalt]%b %b%s%b\n' "$C_TAG" "$C_END" "$C_WARN" "$*" "$C_END" >&2; }
die()  { printf '%b[Basalt]%b %bERREUR : %s%b\n' "$C_TAG" "$C_END" "$C_ERR" "$*" "$C_END" >&2; exit 1; }

case "$REPO" in
    *@*) die "Le dépôt GitHub n'est pas renseigné. Lancez : BASALT_REPO=proprietaire/depot sh get.sh" ;;
    */*) ;;
    *) die "Dépôt invalide : « $REPO » (attendu : proprietaire/depot)" ;;
esac
command -v curl >/dev/null 2>&1 || die "curl est introuvable (sudo apt install curl)."
command -v tar >/dev/null 2>&1 || die "tar est introuvable (sudo apt install tar)."
[ "$(uname -s)" = "Linux" ] || die "Ce script est prévu pour Linux. Sous Windows, utilisez windows\\Installer.cmd."
case "$(uname -m)" in
    x86_64 | amd64) ASSET="basalt-linux-x64.tar.gz" ;;
    *) die "Pas de version prête pour ce processeur ($(uname -m)). Compilez depuis les sources : sh install.sh" ;;
esac
[ -w . ] || die "Le dossier courant n'est pas accessible en écriture."

# Adresse de base des téléchargements (remplaçable pour les essais)
if [ -n "${BASALT_BASE_URL:-}" ]; then
    BASE="$BASALT_BASE_URL"
    VERSION="${WANTED:-essai}"
else
    if [ -z "$WANTED" ]; then
        # La page « dernière version » redirige vers .../releases/tag/<version>
        WANTED=$(curl -sSfLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" | sed -n 's|.*/releases/tag/||p')
        [ -n "$WANTED" ] || die "Aucune version publiée sur https://github.com/$REPO/releases"
    fi
    case "$WANTED" in
        *[!A-Za-z0-9._-]*) die "Nom de version invalide : $WANTED" ;;
    esac
    # « dev » : la préversion que chaque envoi sur la branche dev remplace
    if [ "$WANTED" = "dev" ]; then
        WANTED="dev-latest"
        FORCE=1
    fi
    VERSION="$WANTED"
    BASE="https://github.com/$REPO/releases/download/$VERSION"
fi

CURRENT=""
[ -f .version ] && CURRENT=$(cat .version 2>/dev/null || true)
if [ "$FORCE" = 0 ] && [ -x ./basalt ] && [ "$CURRENT" = "$VERSION" ]; then
    ok "Basalt $VERSION est déjà installé ici : rien à faire (-f pour réinstaller)."
    exit 0
fi

TMP=$(mktemp -d "${TMPDIR:-/tmp}/basalt-get.XXXXXX")
trap 'rm -rf "$TMP"' EXIT INT TERM

if [ "$VERSION" = "dev-latest" ]; then
    say "Installation du dernier envoi sur la branche dev"
elif [ -n "$CURRENT" ]; then
    say "Mise à jour : $CURRENT -> $VERSION"
else
    say "Installation de Basalt $VERSION dans $(pwd)"
fi
say "Téléchargement de $ASSET…"
if ! curl -sSfL --proto '=https,file' -o "$TMP/$ASSET" "$BASE/$ASSET"; then
    warn "La version $VERSION n'est pas (encore) publiée. Après l'envoi d'une étiquette, GitHub met"
    warn "une dizaine de minutes à la compiler : https://github.com/$REPO/actions"
    die "Téléchargement impossible : $BASE/$ASSET"
fi
curl -sSfL --proto '=https,file' -o "$TMP/$ASSET.sha256" "$BASE/$ASSET.sha256" || die "Somme de contrôle introuvable : $BASE/$ASSET.sha256"

# Vérification : l'archive doit correspondre à l'empreinte publiée avec elle
EXPECTED=$(awk '{print $1; exit}' "$TMP/$ASSET.sha256")
if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL=$(sha256sum "$TMP/$ASSET" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL=$(shasum -a 256 "$TMP/$ASSET" | awk '{print $1}')
else
    die "sha256sum est introuvable (sudo apt install coreutils)."
fi
[ -n "$EXPECTED" ] && [ "$EXPECTED" = "$ACTUAL" ] || die "Archive corrompue : empreinte inattendue. Rien n'a été modifié."

# Extraction à part ; aucun fichier de l'archive ne doit sortir de son dossier
if tar tzf "$TMP/$ASSET" | grep -E '(^/|(^|/)\.\.(/|$))' >/dev/null; then
    die "Archive refusée : elle contient des chemins dangereux."
fi
mkdir "$TMP/x"
tar xzf "$TMP/$ASSET" -C "$TMP/x" --no-same-owner
SRC="$TMP/x/basalt"
[ -f "$SRC/basalt" ] || die "Archive inattendue : exécutable absent."

# Le serveur : remplacé d'un coup (un serveur en marche continue avec l'ancien jusqu'au redémarrage)
cp "$SRC/basalt" ./basalt.new
chmod 755 ./basalt.new
mv -f ./basalt.new ./basalt
# Données du jeu : elles suivent la version du serveur
if [ -d "$SRC/data" ]; then
    rm -rf ./data.new
    cp -r "$SRC/data" ./data.new
    rm -rf ./data
    mv ./data.new ./data
fi
# Plugins : ceux fournis en dossier (EasyEdit…) suivent la version ; vos fichiers ne sont pas écrasés
mkdir -p plugins
if [ -d "$SRC/plugins" ]; then
    for f in "$SRC"/plugins/*.lua; do
        [ -e "$f" ] || continue
        [ -e "plugins/$(basename "$f")" ] || cp "$f" plugins/
    done
    for d in "$SRC"/plugins/*/; do
        [ -f "$d/plugin.yml" ] || continue
        mkdir -p "plugins/$(basename "$d")"
        cp -r "$d". "plugins/$(basename "$d")/"
    done
fi
for f in start.sh update.sh README.md LICENSE LICENCE-Lua.txt liblua5.4.so.0 basalt.service; do
    [ -f "$SRC/$f" ] && cp "$SRC/$f" "./$f"
done
[ -f start.sh ] && chmod 755 start.sh
[ -f update.sh ] && chmod 755 update.sh
[ -d "$SRC/docs" ] && { rm -rf ./docs; cp -r "$SRC/docs" ./docs; }
printf '%s\n' "$VERSION" > .version

# Lua 5.4 (pour les plugins) est livré avec le serveur ; à défaut, celui du système sert
HAS_LUA=0
for lib in ./liblua5.4.so.0 /usr/lib/*/liblua5.4.so* /usr/lib/liblua5.4.so* /usr/lib64/liblua*5.4* /usr/local/lib/liblua*5.4*; do
    [ -e "$lib" ] && HAS_LUA=1
done
if [ "$HAS_LUA" = 0 ]; then
    warn "Lua 5.4 est absent : les plugins ne se chargeront pas. Installez-le avec :"
    warn "    sudo apt install liblua5.4-0"
fi

ok "Basalt $VERSION est prêt."
say "Démarrer le serveur :   ./start.sh"
say "Mettre à jour plus tard : ./update.sh"
say "Ports à ouvrir : 19132/udp, 19132/tcp et 19140/udp"
