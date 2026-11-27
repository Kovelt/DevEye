#!/usr/bin/env bash
# Preuve sur binaire réel que le SDK est un no-op en release (ADR-0039).
#
# 1. construit l'exemple `release_probe` en debug : il DOIT contenir les routes
#    `/devtools/` (témoin positif : sans lui, une recherche vide ne prouverait
#    rien, la chaîne pouvant simplement avoir changé) ;
# 2. le construit en release : il ne doit contenir AUCUNE trace du serveur HTTP
#    (routes `/devtools/`, en-tête de version du protocole) ;
# 3. l'exécute en release : il doit se terminer sans rien écouter.
#
# Usage : scripts/verify-sdk-release-noop.sh   (depuis n'importe où)
set -euo pipefail

cd "$(dirname "$0")/.."
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-12}"

# Fragments qui n'existent qu'avec le serveur HTTP du SDK.
FORBIDDEN=(
  "/devtools/"
  "x-deveye-protocol"
)

contains() { # contains <binaire> <fragment>
  # grep direct sur le binaire : un `strings | grep -q` sous pipefail échoue
  # par SIGPIPE dès la première correspondance.
  grep -aqiF -- "$2" "$1"
}

echo ">> témoin positif : build debug de la sonde"
cargo build -q -p deveye-sdk --example release_probe
debug_bin="target/debug/examples/release_probe"
for needle in "${FORBIDDEN[@]}"; do
  if ! contains "$debug_bin" "$needle"; then
    echo "ÉCHEC : le témoin debug ne contient pas « $needle » ; la preuve serait vide" >&2
    exit 2
  fi
done

echo ">> build release de la sonde"
cargo build -q --release -p deveye-sdk --example release_probe
release_bin="target/release/examples/release_probe"

status=0
for needle in "${FORBIDDEN[@]}"; do
  if contains "$release_bin" "$needle"; then
    echo "ÉCHEC : le binaire release contient « $needle »" >&2
    status=1
  fi
done
[ "$status" -eq 0 ] || exit "$status"

echo ">> exécution release : rien ne doit écouter"
"$release_bin"
echo "OK : le SDK est un no-op en release (aucune trace du serveur HTTP dans le binaire)"
