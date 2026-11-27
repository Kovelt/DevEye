#!/usr/bin/env bash
# sync-deveye.sh -- OUTIL DE TRANSITION, deprecie (ADR-0040).
#
# Copie les bibliothèques que l'app cible embarque (deveye-core, deveye-ports,
# deveye-sdk) vers des dossiers de copie que VOUS designez. Il n'existe que
# tant que ces crates ne sont pas publies sur crates.io : des qu'une version
# est publiee, declarez-les en dépendance versionnee (docs/integration.md) et
# supprimez la copie. Les adapters et deveye-server ne sont jamais copies :
# ils servent cote orchestrateur, pas dans l'app cible.
#
# Le script n'écrit que lorsqu'on le lance, et uniquement dans les cibles
# demandees. Il ne contient aucune cible : elles viennent, par ordre de
# priorite, de
#   1. l'argument repetable   --target [ETIQUETTE=]DOSSIER
#   2. la variable            DEVEYE_SYNC_TARGETS  (entrees separees par ':')
#   3. le fichier             --targets-file FICHIER, sinon la variable
#      DEVEYE_SYNC_TARGETS_FILE, sinon scripts/sync-targets (local, non
#      versionne ; modele : scripts/sync-targets.example)
# Une entree est `DOSSIER` ou `ETIQUETTE=DOSSIER` ; un DOSSIER est le dossier
# qui recevra les trois crates (un sous-dossier par crate).
#
# Durcissements (herites de l'ancien script) :
#   * VENDORED.md écrit a la racine de chaque copie a chaque sync : commit
#     source, date, hash par crate ;
#   * avant d'ecraser, le script vérifie via ce fichier que la copie n'a pas ete
#     editee depuis la dernière sync -- abandon sinon (--force outrepasse,
#     apres avoir rapatrie a la main ce qui doit l'etre) ;
#   * --check compare le dépôt et les copies sans rien ecrire.
#
# Usage : scripts/sync-deveye.sh [--dry-run | --check] [--force]
#                                [--target [ETIQUETTE=]DOSSIER]...
#                                [--targets-file FICHIER]
# Codes de sortie : 0 = ok (ou copies a jour avec --check) ; 1 = derive détectée
# (--check) ou copie editee localement (abandon) ; 2 = usage ou configuration
# invalide : aucune cible, cible absente, rsync manquant.
set -euo pipefail

MODE="sync"
FORCE=0
RSYNC_DRY=()
CLI_TARGETS=()
TARGETS_FILE_ARG=""

usage() {
  sed -n '/^# Usage/,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' >&2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      [[ "${MODE}" != "check" ]] || { echo "--check et --dry-run sont exclusifs" >&2; usage; exit 2; }
      MODE="dry-run"; RSYNC_DRY=(--dry-run) ;;
    --check)
      [[ "${MODE}" != "dry-run" ]] || { echo "--check et --dry-run sont exclusifs" >&2; usage; exit 2; }
      MODE="check" ;;
    --force) FORCE=1 ;;
    --target)
      [[ $# -ge 2 ]] || { echo "--target attend une valeur" >&2; usage; exit 2; }
      CLI_TARGETS+=("$2"); shift ;;
    --targets-file)
      [[ $# -ge 2 ]] || { echo "--targets-file attend une valeur" >&2; usage; exit 2; }
      TARGETS_FILE_ARG="$2"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "argument inconnu : $1" >&2; usage; exit 2 ;;
  esac
  shift
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEVEYE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CRATES=("deveye-core" "deveye-ports" "deveye-sdk")

# --- Resolution des cibles -------------------------------------------------
ENTRIES=()
if [[ ${#CLI_TARGETS[@]} -gt 0 ]]; then
  ENTRIES=("${CLI_TARGETS[@]}")
elif [[ -n "${DEVEYE_SYNC_TARGETS:-}" ]]; then
  IFS=':' read -r -a ENTRIES <<< "${DEVEYE_SYNC_TARGETS}"
else
  targets_file="${TARGETS_FILE_ARG:-${DEVEYE_SYNC_TARGETS_FILE:-${SCRIPT_DIR}/sync-targets}}"
  if [[ ! -f "${targets_file}" ]]; then
    echo "Aucune cible de synchronisation configuree." >&2
    echo "Fournir --target, DEVEYE_SYNC_TARGETS ou un fichier de cibles" >&2
    echo "(${targets_file} introuvable ; modele : scripts/sync-targets.example)." >&2
    exit 2
  fi
  while IFS= read -r line || [[ -n "${line}" ]]; do
    line="${line%%#*}"
    line="$(echo "${line}" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')"
    [[ -n "${line}" ]] && ENTRIES+=("${line}")
  done < "${targets_file}"
fi
if [[ ${#ENTRIES[@]} -eq 0 ]]; then
  echo "La liste de cibles est vide." >&2
  exit 2
fi

LABELS=()
DIRS=()
for entry in "${ENTRIES[@]}"; do
  [[ -n "${entry}" ]] || continue
  if [[ "${entry}" == *=* ]]; then
    label="${entry%%=*}"; dir="${entry#*=}"
  else
    dir="${entry}"; label="$(basename "${dir}")"
  fi
  LABELS+=("${label}")
  DIRS+=("${dir}")
done

# Une cible demandee mais absente est une erreur, jamais un « ignore » muet :
# sinon une faute de chemin passerait pour une copie a jour.
missing=0
for i in "${!DIRS[@]}"; do
  if [[ ! -d "${DIRS[$i]}" ]]; then
    echo "ERREUR : cible absente : ${LABELS[$i]} (${DIRS[$i]})" >&2
    missing=1
  fi
done
[[ "${missing}" -eq 0 ]] || exit 2

if [[ "${MODE}" != "check" ]] && ! command -v rsync >/dev/null 2>&1; then
  echo "ERREUR : rsync est requis (sauf avec --check)." >&2
  exit 2
fi

# --- Hash et VENDORED.md ---------------------------------------------------
# Hash stable d'une crate : contenu de tous les fichiers que rsync copie
# (README, LICENSE-MIT, etc. compris ; target/ et Cargo.lock exclus comme dans
# rsync), chemins relatifs tries -- comparable entre le dépôt et une copie
# quel que soit le prefixe.
hash_crate() {
  local root="$1" crate="$2"
  if [[ ! -d "${root}/${crate}" ]]; then
    echo "ABSENT"
    return
  fi
  (cd "${root}/${crate}" \
    && find . -type f -not -path './target/*' -not -name 'Cargo.lock' \
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
  local dest_root="$1" commit crate
  commit="$(git -C "${DEVEYE_ROOT}" rev-parse HEAD)"
  {
    echo "# Copie de DevEye -- NE PAS EDITER"
    echo
    echo "Générée par \`scripts/sync-deveye.sh\` depuis le dépôt DevEye (outil de"
    echo "transition, déprécié : preferer une dépendance versionnee)."
    echo "Tout correctif doit naitre dans DevEye puis être re-propage ; une edition"
    echo "locale sera détectée et bloquera la prochaine synchronisation."
    echo
    echo "- Commit source : \`${commit}\`"
    echo "- Date de sync : $(date -Iseconds)"
    echo "- Perimetre : ${CRATES[*]} (les adapters et deveye-server ne sont"
    echo "  volontairement PAS copies)."
    echo
    for crate in "${CRATES[@]}"; do
      echo "<!-- hash ${crate} $(hash_crate "${dest_root}" "${crate}") -->"
    done
  } > "${dest_root}/VENDORED.md"
}

# --- --check : derive, sans écriture ---------------------------------------
if [[ "${MODE}" == "check" ]]; then
  drift=0
  for i in "${!DIRS[@]}"; do
    for crate in "${CRATES[@]}"; do
      src_hash="$(hash_crate "${DEVEYE_ROOT}" "${crate}")"
      dst_hash="$(hash_crate "${DIRS[$i]}" "${crate}")"
      if [[ "${src_hash}" != "${dst_hash}" ]]; then
        echo "DERIVE  ${LABELS[$i]}/${crate} (copie != depot)"
        drift=1
      else
        echo "ok      ${LABELS[$i]}/${crate}"
      fi
    done
  done
  exit "${drift}"
fi

# --- Synchronisation ---------------------------------------------------------
# Garde anti-ecrasement : si la copie a ete editee depuis la derniere sync
# (hash courant != hash consigne dans VENDORED.md), on refuse d'ecraser.
# --force outrepasse, une fois les changements rapatries. Elle est evaluee sur
# TOUTES les cibles avant la moindre ecriture : aucune sync partielle.
guard_target() {
  local dest_root="$1" crate rec cur
  for crate in "${CRATES[@]}"; do
    rec="$(recorded_hash "${dest_root}" "${crate}")"
    cur="$(hash_crate "${dest_root}" "${crate}")"
    if [[ -n "${rec}" && "${cur}" != "ABSENT" && "${rec}" != "${cur}" ]]; then
      echo "ABANDON : ${dest_root}/${crate} a ete edite depuis la dernière sync" >&2
      echo "          (hash consigne ${rec}, hash courant ${cur})." >&2
      echo "          Rapatrier les changements dans DevEye, puis relancer avec --force." >&2
      return 1
    fi
  done
}

sync_to() {
  local dest_root="$1" label="$2" crate
  echo "=== ${label} : ${dest_root} ==="

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

if [[ "${FORCE}" -ne 1 ]]; then
  for i in "${!DIRS[@]}"; do
    guard_target "${DIRS[$i]}" || exit 1
  done
fi

for i in "${!DIRS[@]}"; do
  sync_to "${DIRS[$i]}" "${LABELS[$i]}"
done

echo "Synchronisation terminee. Vérifier que les consommateurs compilent."
