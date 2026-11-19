#!/bin/sh
# Met Basalt à jour depuis GitHub (arrêtez puis relancez le serveur ensuite).
set -eu
cd "$(dirname "$0")"
REPO="${BASALT_REPO:-ZoumiSurGithub/basalt}"
curl -sSfL --proto '=https' "https://raw.githubusercontent.com/$REPO/main/get.sh" | BASALT_REPO="$REPO" sh -s -- "$@"
