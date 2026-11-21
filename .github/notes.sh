#!/bin/sh
# Écrit dans notes.md les notes d'une version tirées de CHANGELOG.md, pour la page de la
# version sur GitHub.
#   .github/notes.sh v0.0.2     section « [0.0.2] » (échoue si elle manque, ou si Cargo.toml
#                               annonce un autre numéro)
#   .github/notes.sh dev        section « Non publié » (préversion de la branche dev)
set -eu
TAG="${1:-}"
VERSION="${TAG#v}"
if [ "$TAG" = "dev" ] || [ "$TAG" = "dev-latest" ]; then
    SECTION="Non publié"
else
    SECTION="$VERSION"
    CARGO=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
    if [ "$CARGO" != "$VERSION" ]; then
        echo "L'étiquette $TAG ne correspond pas à la version de Cargo.toml ($CARGO) : lancez ./release.sh $VERSION" >&2
        exit 1
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
        echo "CHANGELOG.md ne contient pas de section [$SECTION] : lancez ./release.sh $VERSION" >&2
        exit 1
    fi
fi
echo "Notes de $TAG :"
cat notes.md
