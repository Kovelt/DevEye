# Recettes DevEye — `just <recette>` a la racine du repo.

# Liste les recettes
default:
    @just --list

# Build du binaire orchestrateur (release, pour .mcp.json)
build:
    cargo build --release -p deveye-server

# Tests du workspace
test:
    cargo test --workspace

# Lint strict (echoue sur tout warning)
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Formatage
fmt:
    cargo fmt --all

# Verification du formatage (comme en CI)
fmt-check:
    cargo fmt --all --check

# Pipeline CI complet en local
ci: fmt-check lint test

# Synchronise les copies vendored (Atelier, Chronos) depuis le canonique.
# Exemples : `just sync-deveye` ou `just sync-deveye --dry-run`.
sync-deveye *args:
    ./scripts/sync-deveye.sh {{args}}
