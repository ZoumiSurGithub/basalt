#!/bin/sh
# Met Basalt à jour depuis GitHub (arrêtez puis relancez le serveur ensuite).
#   ./update.sh            dernière version stable
#   ./update.sh -v dev     dernier envoi sur la branche dev
#   ./update.sh -v v0.2.0  une version précise
set -eu
cd "$(dirname "$0")"
REPO="${BASALT_REPO:-ZoumiSurGithub/basalt}"
# Le script d'installation est pris sur la branche dev quand c'est elle qu'on demande :
# il peut être plus récent que celui de la branche principale
BRANCH=main
for arg in "$@"; do
    case "$arg" in
        dev | -d) BRANCH=dev ;;
    esac
done
curl -sSfL --proto '=https' "https://raw.githubusercontent.com/$REPO/$BRANCH/get.sh" | BASALT_REPO="$REPO" sh -s -- "$@"
