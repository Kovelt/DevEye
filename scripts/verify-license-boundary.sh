#!/usr/bin/env bash
# Frontière des licences (ADR-0002) : un crate MIT ne dépend jamais d'un crate
# AGPL du workspace, quel que soit le type de dépendance (normale, build, dev).
# `cargo deny` accorde l'exception AGPL par nom de crate sans regarder le sens
# des dépendances ; ce contrôle le fait.
#
# Usage : scripts/verify-license-boundary.sh [fichier-metadata.json]
# (sans argument : `cargo metadata` du workspace ; l'argument sert aux tests).
set -euo pipefail
cd "$(dirname "$0")/.."

FILTER='
  .packages as $pkgs
  | ($pkgs | map({(.name): .license}) | add) as $license
  | $pkgs[]
  | select(.license == "MIT")
  | . as $p
  | .dependencies[]
  | select($license[.name] != null and $license[.name] != "MIT")
  | "\($p.name) (MIT) dépend de \(.name) (\($license[.name]))"
'

if [ $# -ge 1 ]; then
  violations="$(jq -r "${FILTER}" "$1")"
else
  violations="$(cargo metadata --format-version 1 --no-deps --locked | jq -r "${FILTER}")"
fi

if [ -n "${violations}" ]; then
  echo "${violations}" >&2
  echo "Frontière MIT/AGPL violée (ADR-0002)." >&2
  exit 1
fi
echo "Frontière MIT/AGPL respectée : aucun crate MIT ne dépend d'un crate AGPL."
