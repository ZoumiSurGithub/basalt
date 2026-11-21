#!/bin/sh
# Prépare une version de Basalt :
#   ./release.sh 0.0.2
#
# - la section « Non publié » de CHANGELOG.md devient « [0.0.2] — date du jour », et une
#   nouvelle section « Non publié » vide est ouverte au-dessus ;
# - le numéro de version de Cargo.toml (et de Cargo.lock) passe à 0.0.2.
#
# Rien n'est envoyé : le script affiche ensuite les commandes git à lancer (commit,
# étiquette v0.0.2, envoi). GitHub publie alors la version avec ces notes.
# Sous Windows, à lancer depuis « Git Bash ».
set -eu
cd "$(dirname "$0")"

VERSION="${1:-}"
VERSION="${VERSION#v}"
case "$VERSION" in
    "" ) echo "Usage : ./release.sh <version>   (exemple : ./release.sh 0.0.2)" >&2; exit 2 ;;
    *[!0-9A-Za-z.-]* ) echo "Numéro de version invalide : $VERSION" >&2; exit 2 ;;
esac
echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' || { echo "Numéro attendu de la forme 1.2.3 (ou 1.2.3-beta.1) : $VERSION" >&2; exit 2; }
[ -f CHANGELOG.md ] || { echo "CHANGELOG.md introuvable" >&2; exit 1; }
if grep -q "^## \[$VERSION\]" CHANGELOG.md; then
    echo "La version $VERSION est déjà dans CHANGELOG.md." >&2
    exit 1
fi

# Contenu de « Non publié » : il ne doit pas être vide
NOTES=$(awk '/^## \[Non publié\]/{on=1; next} /^## \[/{on=0} on' CHANGELOG.md | grep -v '^[[:space:]]*$' || true)
if [ -z "$NOTES" ]; then
    echo "La section « Non publié » de CHANGELOG.md est vide : décrivez d'abord ce qui a changé." >&2
    exit 1
fi

DATE=$(date +%Y-%m-%d)
TMP=$(mktemp)
awk -v v="$VERSION" -v d="$DATE" '
    /^## \[Non publié\]/ && !done { print; print ""; print "## [" v "] — " d; done = 1; next }
    { print }
' CHANGELOG.md > "$TMP"
mv "$TMP" CHANGELOG.md

# Numéro de version du programme
sed -i.bak "0,/^version = \".*\"/s//version = \"$VERSION\"/" Cargo.toml && rm -f Cargo.toml.bak
awk -v v="$VERSION" '
    /^name = "basalt"$/ { print; getline; if ($0 ~ /^version = /) { print "version = \"" v "\""; next } }
    { print }
' Cargo.lock > Cargo.lock.tmp && mv Cargo.lock.tmp Cargo.lock

echo "Version $VERSION prête (CHANGELOG.md, Cargo.toml, Cargo.lock)."
echo
echo "Notes de cette version :"
echo "$NOTES"
echo
echo "Pour la publier :"
echo "    git add CHANGELOG.md Cargo.toml Cargo.lock"
echo "    git commit -m \"Version $VERSION\""
echo "    git push"
echo "    git tag v$VERSION"
echo "    git push origin v$VERSION"
