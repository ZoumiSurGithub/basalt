#!/bin/sh
# Publie une version de Basalt, en une commande :
#   ./release.sh 0.0.7          version stable
#   ./release.sh 0.1.0-beta.1   préversion (un tiret dans le numéro)
#
# Le script :
#   1. vérifie que tout est prêt (dépôt git propre, numéro libre, journal rempli) ;
#   2. date la section « Non publié » de CHANGELOG.md sous ce numéro et en ouvre une neuve ;
#   3. inscrit le numéro dans Cargo.toml et Cargo.lock ;
#   4. après votre accord, crée le commit « Version 0.0.7 », l'étiquette v0.0.7, et envoie
#      les deux sur GitHub.
# GitHub compile alors Basalt pour Linux et Windows et publie la version avec ses notes.
#
# Le numéro inscrit dans le programme publié est toujours celui de l'étiquette : il ne
# peut plus différer de la page de version, même si Cargo.toml a été oublié.
#
# Options : -n  montre ce qui serait fait, sans rien modifier
#           -y  ne demande pas de confirmation
# Sous Windows, à lancer depuis « Git Bash ».
set -eu
cd "$(dirname "$0")"

DRY=0
YES=0
VERSION=""
for arg in "$@"; do
    case "$arg" in
        -n) DRY=1 ;;
        -y) YES=1 ;;
        -*) echo "Option inconnue : $arg" >&2; exit 2 ;;
        *) VERSION="$arg" ;;
    esac
done
VERSION="${VERSION#v}"
TAG="v$VERSION"
die() { echo "Erreur : $*" >&2; exit 1; }

[ -n "$VERSION" ] || { echo "Usage : ./release.sh [-n] [-y] <version>   (exemple : ./release.sh 0.0.7)" >&2; exit 2; }
echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' || die "numéro attendu de la forme 1.2.3 (ou 1.2.3-beta.1) : $VERSION"
[ -f CHANGELOG.md ] || die "CHANGELOG.md introuvable"
command -v git >/dev/null 2>&1 || die "git introuvable"
git rev-parse --git-dir >/dev/null 2>&1 || die "ce dossier n'est pas un dépôt git"

# --- Vérifications : rien n'est touché tant qu'une seule échoue ---------------------------
if [ -n "$(git status --porcelain)" ]; then
    git status --short >&2
    die "des fichiers ne sont pas encore enregistrés (commit) : la version doit partir d'un état propre"
fi
BRANCH=$(git rev-parse --abbrev-ref HEAD)
git fetch --quiet --tags origin 2>/dev/null || echo "(attention : GitHub injoignable, étiquettes distantes non vérifiées)"
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
    die "l'étiquette $TAG existe déjà (git tag -d $TAG pour la retirer en local)"
fi
if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
    die "l'étiquette $TAG existe déjà sur GitHub"
fi
if grep -q "^## \[$VERSION\]" CHANGELOG.md; then
    die "la version $VERSION est déjà dans CHANGELOG.md"
fi
# La branche doit être à jour avec GitHub (sinon l'envoi serait refusé à mi-chemin)
if git rev-parse -q --verify "origin/$BRANCH" >/dev/null; then
    BEHIND=$(git rev-list --count "HEAD..origin/$BRANCH" 2>/dev/null || echo 0)
    [ "$BEHIND" = 0 ] || die "la branche $BRANCH a $BEHIND commit(s) de retard sur GitHub : git pull d'abord"
fi
NOTES=$(awk '/^## \[Non publié\]/{on=1; next} /^## \[/{on=0} on' CHANGELOG.md | grep -v '^[[:space:]]*$' || true)
[ -n "$NOTES" ] || die "la section « Non publié » de CHANGELOG.md est vide : décrivez d'abord ce qui a changé"
LAST=$(sed -n 's/^## \[\([0-9][^]]*\)\].*/\1/p' CHANGELOG.md | head -n 1)
case "$VERSION" in
    *-*) KIND="préversion" ;;
    *) KIND="version stable" ;;
esac

echo "Basalt $VERSION ($KIND), depuis la branche $BRANCH${LAST:+ — version précédente : $LAST}"
echo
echo "Notes :"
echo "$NOTES"
echo
if [ "$DRY" = 1 ]; then
    echo "(-n : rien n'a été modifié)"
    exit 0
fi
if [ "$YES" = 0 ]; then
    printf "Publier %s ? [o/N] " "$TAG"
    read -r answer || answer=""
    case "$answer" in
        o | O | oui | y | yes) ;;
        *) echo "Abandon : rien n'a été modifié."; exit 0 ;;
    esac
fi

# --- Journal et numéro de version ---------------------------------------------------------
DATE=$(date +%Y-%m-%d)
TMP=$(mktemp)
awk -v v="$VERSION" -v d="$DATE" '
    /^## \[Non publié\]/ && !done { print; print ""; print "## [" v "] — " d; done = 1; next }
    { print }
' CHANGELOG.md > "$TMP"
mv "$TMP" CHANGELOG.md
# (la première ligne « version = » de Cargo.toml est celle du paquet)
awk -v v="$VERSION" '!done && /^version = "/ { print "version = \"" v "\""; done = 1; next } { print }' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml
if [ -f Cargo.lock ]; then
    awk -v v="$VERSION" '
        /^name = "basalt"$/ { print; getline; if ($0 ~ /^version = /) { print "version = \"" v "\""; next } }
        { print }
    ' Cargo.lock > Cargo.lock.tmp && mv Cargo.lock.tmp Cargo.lock
fi

# --- Commit, étiquette, envoi -------------------------------------------------------------
git add CHANGELOG.md Cargo.toml Cargo.lock 2>/dev/null || git add CHANGELOG.md Cargo.toml
git commit --quiet -m "Version $VERSION"
git tag -a "$TAG" -m "Basalt $VERSION"
if ! git push --quiet origin "$BRANCH" || ! git push --quiet origin "$TAG"; then
    echo >&2
    echo "L'envoi sur GitHub a échoué. Le commit et l'étiquette sont prêts en local ; relancez :" >&2
    echo "    git push origin $BRANCH && git push origin $TAG" >&2
    exit 1
fi

REPO=$(git config --get remote.origin.url | sed -e 's#^git@github.com:#https://github.com/#' -e 's#\.git$##')
echo
echo "Basalt $VERSION envoyée. GitHub la compile et la publie (quelques minutes) :"
echo "    $REPO/actions"
echo "Elle apparaîtra ensuite ici : $REPO/releases/tag/$TAG"
echo
echo "Si la compilation échoue : corrigez, enregistrez (commit), puis publiez le numéro"
echo "suivant (./release.sh avec un numéro plus grand) ; la page $TAG ne sera pas créée."
