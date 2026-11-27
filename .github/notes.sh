#!/bin/sh
# Écrit dans notes.md les notes d'une version tirées de CHANGELOG.md, pour la page de la
# version sur GitHub.
#   .github/notes.sh v0.0.2     section « [0.0.2] » (à défaut : « Non publié », avec un
#                               avertissement)
#   .github/notes.sh dev        section « Non publié » (préversion de la branche dev)
set -eu
TAG="${1:-}"
VERSION="${TAG#v}"
if [ "$TAG" = "dev" ] || [ "$TAG" = "dev-latest" ]; then
    SECTION="Non publié"
else
    SECTION="$VERSION"
    # (le programme publié prend son numéro de l'étiquette : un Cargo.toml en retard ne
    # change rien, il est seulement signalé)
    CARGO=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
    if [ "$CARGO" != "$VERSION" ]; then
        echo "::warning::Cargo.toml annonce $CARGO, l'étiquette $TAG : le programme publié portera le numéro $VERSION"
    fi
fi
awk -v s="$SECTION" '
    index($0, "## [" s "]") == 1 { on = 1; next }
    /^## \[/ { on = 0 }
    on
' CHANGELOG.md > notes.md
if ! grep -q '[^[:space:]]' notes.md; then
    if [ "$SECTION" = "Non publié" ]; then
        echo "Rien de nouveau depuis la dernière version." > notes.md
    else
        # Étiquette posée à la main, sans ./release.sh : les notes sont celles de « Non publié »
        echo "::warning::CHANGELOG.md n'a pas de section [$SECTION] : notes prises dans « Non publié » (./release.sh la crée)"
        awk 'index($0, "## [Non publié]") == 1 { on = 1; next } /^## \[/ { on = 0 } on' CHANGELOG.md > notes.md
        grep -q '[^[:space:]]' notes.md || echo "Aucune note pour cette version." > notes.md
    fi
fi
echo "Notes de $TAG :"
cat notes.md
