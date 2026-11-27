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

# Conformite du wire protocol : SDK Rust en processus, ou `just conformance <url>` pour une implementation tierce
conformance url="":
    if [ -z "{{url}}" ]; then cargo test -p deveye-conformance -- --nocapture; else cargo run -q -p deveye-conformance -- --url "{{url}}"; fi

# Pipeline CI complet en local
ci: fmt-check lint test
