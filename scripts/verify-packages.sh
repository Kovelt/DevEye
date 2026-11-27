#!/usr/bin/env bash
# Contenu des paquets des quatre crates MIT publiables (ADR-0004) : chacun doit
# embarquer LICENSE-MIT et son README. `cargo package --list` ne publie rien et
# n'exige pas que les dépendances de workspace soient déjà sur crates.io.
set -euo pipefail
cd "$(dirname "$0")/.."

status=0
for crate in deveye-core deveye-ports deveye-sdk deveye-sdk-client; do
  files="$(cargo package -p "${crate}" --list --locked --allow-dirty 2>/dev/null)"
  for required in LICENSE-MIT README.md Cargo.toml; do
    if ! grep -qx "${required}" <<<"${files}"; then
      echo "${crate} : ${required} absent du paquet." >&2
      status=1
    fi
  done
done
[ "${status}" -eq 0 ] && echo "Paquets des 4 crates publiables : LICENSE-MIT et README présents."
exit "${status}"
