#!/usr/bin/env bash
# sync-deveye.sh — propage le canonique DevEye vers les copies vendored.
#
# Depuis M0, le repo DevEye est la SOURCE DE VERITE (les enrichissements
# Chronos/Atelier d'avant M0 y ont ete reconcilies, session.rs compris). Ce
# script copie les crates consommees cote APP-CIBLE :
#     deveye-core, deveye-ports, deveye-sdk
# vers les vendored :
#     - Atelier : Atelier/deveye/          (path-dep gitignoree)
#     - Chronos : Chronos/crates/deveye/   (membre du workspace Chronos)
#
# Les adapters (cdp-webview, screencap-*, mcp-server, ...) et le binaire
# deveye-server ne sont PAS copies : ils ne servent que cote orchestrateur
# (ce repo), jamais embarques dans une app cible.
#
# Usage : ./scripts/sync-deveye.sh [--dry-run]
set -euo pipefail

DRY_RUN=()
if [[ "${1:-}" == "--dry-run" ]]; then
  DRY_RUN=(--dry-run)
  echo "[dry-run] aucune ecriture sur le disque"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEVEYE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
PROJETS_ROOT="$(cd "${DEVEYE_ROOT}/.." && pwd)"

ATELIER_DEVEYE="${PROJETS_ROOT}/Atelier/deveye"
CHRONOS_DEVEYE="${PROJETS_ROOT}/Chronos/crates/deveye"

CRATES=("deveye-core" "deveye-ports" "deveye-sdk")

sync_to() {
  local dest_root="$1"
  local label="$2"
  if [[ ! -d "${dest_root}" ]]; then
    echo "  (ignore : ${label} absent — ${dest_root})"
    return
  fi
  echo "=== ${label} : ${dest_root} ==="
  for crate in "${CRATES[@]}"; do
    rsync -a --delete "${DRY_RUN[@]}" \
      --exclude="target/" --exclude="Cargo.lock" \
      "${DEVEYE_ROOT}/${crate}/" "${dest_root}/${crate}/"
    echo "  ${crate} -> ${dest_root}/${crate}/"
  done
}

sync_to "${ATELIER_DEVEYE}" "Atelier/deveye"
sync_to "${CHRONOS_DEVEYE}" "Chronos/crates/deveye"

echo "Sync termine. Pensez a verifier que les consommateurs compilent :"
echo "  (cd \"${PROJETS_ROOT}/Atelier\" && cargo check --features deveye)"
echo "  (cd \"${PROJETS_ROOT}/Chronos\" && cargo check --features deveye)"
