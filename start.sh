#!/bin/sh
# Démarre Basalt depuis son dossier.
cd "$(dirname "$0")"
exec ./basalt "$@"
