#!/usr/bin/env bash
# sync-deveye.sh — propage le canonique DevEye vers les copies vendored.
#
# Depuis M0 (25/06/2026), le repo DevEye est la SOURCE DE VERITE (les
# enrichissements Chronos/Atelier d'avant M0 y ont ete reconcilies,
# session.rs compris). Ce script copie les crates consommees cote APP-CIBLE :
#     deveye-core, deveye-ports, deveye-sdk
# vers les vendored :
#     - Atelier : Atelier/deveye/          (path-dep gitignoree)
#     - Chronos : Chronos/crates/deveye/   (membre du workspace Chronos)
#
# Les adapters (cdp-webview, screencap-*, mcp-server, ...) et le binaire
# deveye-server ne sont PAS copies : ils ne servent que cote orchestrateur
# (ce repo), jamais embarques dans une app cible. (Decision du 04/07/2026 :
# les vieilles copies vendored de ces crates ont ete supprimees des deux
# arbres — un instantane fige qui a l'air a jour est pire que rien.)
#
# Durcissements (audit du 04/07/2026, section C) :
#   * un VENDORED.md est genere a la racine de chaque copie a chaque sync
#     (commit source, date, hash par crate) ;
#   * avant d'ecraser, le script verifie via ce VENDORED.md que la copie n'a
#     pas ete editee depuis la derniere sync — abort sinon (--force outrepasse,
#     apres avoir rapatrie a la main ce qui doit l'etre) ;
#   * --check compare canonique et copies sans rien ecrire (0 = a jour,
#     1 = derive detectee) — utilisable en pre-commit.
#
# Usage : ./scripts/sync-deveye.sh [--dry-run] [--check] [--force]
set -euo pipefail

MODE="sync"
FORCE=0
RSYNC_DRY=()
for arg in "$@"; do
  case "$arg" in
    --dry-run) MODE="dry-run"; RSYNC_DRY=(--dry-run) ;;
    --check)   MODE="check" ;;
    --force)   FORCE=1 ;;
    *) echo "usage : $0 [--dry-run] [--check] [--force]" >&2; exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEVEYE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
PROJETS_ROOT="$(cd "${DEVEYE_ROOT}/.." && pwd)"

ATELIER_DEVEYE="${PROJETS_ROOT}/Atelier/deveye"
CHRONOS_DEVEYE="${PROJETS_ROOT}/Chronos/crates/deveye"

CRATES=("deveye-core" "deveye-ports" "deveye-sdk")

# Hash stable d'une crate : contenu des .rs + Cargo.toml, chemins relatifs
# tries — comparable entre le canonique et une copie quel que soit le prefixe.
hash_crate() {
  local root="$1" crate="$2"
  if [[ ! -d "${root}/${crate}" ]]; then
    echo "ABSENT"
    return
  fi
  (cd "${root}/${crate}" \
    && find . -type f \( -name '*.rs' -o -name 'Cargo.toml' \) -not -path './target/*' \
       | LC_ALL=C sort | xargs -r sha256sum | sha256sum | cut -d' ' -f1)
}

# Hash enregistre pour une crate dans le VENDORED.md d'une copie (vide si absent).
recorded_hash() {
  local dest_root="$1" crate="$2"
  [[ -f "${dest_root}/VENDORED.md" ]] || return 0
  awk -v c="${crate}" '$1 == "<!--" && $2 == "hash" && $3 == c { print $4 }' \
    "${dest_root}/VENDORED.md"
}

write_vendored_md() {
  local dest_root="$1"
  local commit
  commit="$(git -C "${DEVEYE_ROOT}" rev-parse HEAD)"
  {
    echo "# Copie vendored de DevEye — NE PAS EDITER"
    echo
    echo "Generee par \`scripts/sync-deveye.sh\` depuis le depot canonique DevEye."
    echo "Tout correctif doit naitre dans le canonique puis etre re-propage ;"
    echo "une edition locale sera detectee et bloquera la prochaine sync."
    echo
    echo "- Commit source : \`${commit}\`"
    echo "- Date de sync : $(date -Iseconds)"
    echo "- Perimetre : ${CRATES[*]} (les adapters et deveye-server ne sont"
    echo "  volontairement PAS vendorises — ils vivent cote orchestrateur)."
    echo
    for crate in "${CRATES[@]}"; do
      echo "<!-- hash ${crate} $(hash_crate "${dest_root}" "${crate}") -->"
    done
  } > "${dest_root}/VENDORED.md"
}

# --check : derive canonique/copies, sans ecriture. 0 = tout a jour.
if [[ "${MODE}" == "check" ]]; then
  drift=0
  for dest_root in "${ATELIER_DEVEYE}" "${CHRONOS_DEVEYE}"; do
    [[ -d "${dest_root}" ]] || { echo "(ignore : ${dest_root} absent)"; continue; }
    for crate in "${CRATES[@]}"; do
      src_hash="$(hash_crate "${DEVEYE_ROOT}" "${crate}")"
      dst_hash="$(hash_crate "${dest_root}" "${crate}")"
      if [[ "${src_hash}" != "${dst_hash}" ]]; then
        echo "DERIVE  ${dest_root}/${crate} (copie != canonique)"
        drift=1
      else
        echo "ok      ${dest_root}/${crate}"
      fi
    done
  done
  exit "${drift}"
fi

sync_to() {
  local dest_root="$1" label="$2"
  if [[ ! -d "${dest_root}" ]]; then
    echo "  (ignore : ${label} absent — ${dest_root})"
    return
  fi
  echo "=== ${label} : ${dest_root} ==="

  # Garde anti-ecrasement : si la copie a ete editee depuis la derniere sync
  # (hash courant != hash consigne dans VENDORED.md), on refuse d'ecraser —
  # c'est exactement ainsi que les correctifs Chronos du 28/06 ont failli
  # etre perdus. --force outrepasse une fois le rapatriement fait.
  if [[ "${FORCE}" -ne 1 ]]; then
    local crate rec cur
    for crate in "${CRATES[@]}"; do
      rec="$(recorded_hash "${dest_root}" "${crate}")"
      cur="$(hash_crate "${dest_root}" "${crate}")"
      if [[ -n "${rec}" && "${cur}" != "ABSENT" && "${rec}" != "${cur}" ]]; then
        echo "ABORT : ${dest_root}/${crate} a ete edite depuis la derniere sync" >&2
        echo "        (hash consigne ${rec}, hash courant ${cur})." >&2
        echo "        Rapatrier les changements dans le canonique, puis --force." >&2
        exit 1
      fi
    done
  fi

  local crate
  for crate in "${CRATES[@]}"; do
    rsync -a --delete "${RSYNC_DRY[@]}" \
      --exclude="target/" --exclude="Cargo.lock" \
      "${DEVEYE_ROOT}/${crate}/" "${dest_root}/${crate}/"
    echo "  ${crate} -> ${dest_root}/${crate}/"
  done

  if [[ "${MODE}" != "dry-run" ]]; then
    write_vendored_md "${dest_root}"
    echo "  VENDORED.md regenere"
  fi
}

[[ "${MODE}" == "dry-run" ]] && echo "[dry-run] aucune ecriture sur le disque"

sync_to "${ATELIER_DEVEYE}" "Atelier/deveye"
sync_to "${CHRONOS_DEVEYE}" "Chronos/crates/deveye"

echo "Sync termine. Pensez a verifier que les consommateurs compilent :"
echo "  (cd \"${PROJETS_ROOT}/Atelier\" && cargo check --features deveye)"
echo "  (cd \"${PROJETS_ROOT}/Chronos\" && cargo check --features deveye)"
