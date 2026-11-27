# Recettes DevEye — `just <recette>` a la racine du repo.

# Liste les recettes
default:
    @just --list

# Build du binaire orchestrateur (release, pour .mcp.json)
build:
    cargo build --release --locked -p deveye-server

# Tests du workspace
test:
    cargo test --workspace --locked

# Lint strict (echoue sur tout warning)
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Formatage
fmt:
    cargo fmt --all

# Verification du formatage (comme en CI)
fmt-check:
    cargo fmt --all --check

# Conformité du wire protocol : SDK Rust en processus, ou `just conformance <url>` pour une implémentation tierce
conformance url="":
    if [ -z "{{url}}" ]; then cargo test -p deveye-conformance --locked -- --nocapture; else cargo run -q --locked -p deveye-conformance -- --url "{{url}}"; fi

# Documentation, avertissements rustdoc = erreurs (comme en CI)
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

# Tests de la cible x11 contre un Xvfb privé (requiert le paquet Xvfb)
x11:
    cargo test -p adapter-x11 --locked -- --ignored --test-threads=1

# Session MCP de bout en bout : binaire, stdio, Xvfb privé, fenêtre fixture
e2e:
    cargo test -p deveye-server --test e2e_mcp --locked -- --ignored --test-threads=1

# Pont d'éléments (snapshot/act) et modificateurs dans Chromium headless
# (`DEVEYE_TEST_CHROMIUM` pour choisir le navigateur ; Brave convient)
chromium:
    cargo test -p deveye-server --test bridge_chromium --test modifiers_chromium --locked -- --ignored --test-threads=1

# MSRV : vérifie le plancher annonce par rust-version (requiert `rustup toolchain install 1.88`)
msrv:
    cargo +1.88 check --workspace --all-targets --locked

# Licences, avis RustSec, sources (requiert cargo-deny)
deny:
    cargo deny check

# Preuve que le SDK est un no-op en release : test, puis inspection du binaire
release-noop:
    cargo test --release --locked -p deveye-sdk --test release_noop
    scripts/verify-sdk-release-noop.sh

# Frontière des licences : aucun crate MIT ne depend d'un crate AGPL (ADR-0002)
license-boundary:
    scripts/verify-license-boundary.sh

# Contenu des paquets des 4 crates publiables (LICENSE-MIT embarquée), sans rien publier
packages:
    scripts/verify-packages.sh

# SBOM CycloneDX, un fichier .cdx.json par crate (requiert cargo-cyclonedx)
sbom:
    cargo cyclonedx --format json

# Installe deveye-server dans ~/.local/bin (build release du dépôt local).
# Alternative : `cargo install --path deveye-server --locked` (vers ~/.cargo/bin).
install:
    cargo build --release --locked -p deveye-server
    install -Dm755 target/release/deveye-server "$HOME/.local/bin/deveye-server"

# Synchronise les copies des bibliothèques vers VOS cibles (outil de transition
# déprécié, ADR-0040) : `just sync-deveye --target app=/chemin/copie`, ou avec
# un fichier scripts/sync-targets ; options --check, --dry-run, --force.
sync-deveye *args:
    ./scripts/sync-deveye.sh {{args}}

# Pipeline CI en local, hors msrv, semver et sbom (outils a part ou tag de reference requis)
ci: fmt-check lint test doc x11 e2e deny conformance release-noop license-boundary packages

# Régénère docs/schemas/ui-widgets-1.json depuis les types de deveye-core
schema:
    DEVEYE_UPDATE_SCHEMA=1 cargo test -p deveye-core --features schema --test ui_schema

# Compatibilité semver des bibliothèques MIT contre un tag, avec et sans la surface expérimentale (ADR-0072)
semver baseline="v1.1.0":
    cargo semver-checks check-release -p deveye-core -p deveye-ports -p deveye-sdk -p deveye-sdk-client --baseline-rev {{baseline}}
    cargo semver-checks check-release -p deveye-core -p deveye-ports --features unstable-elements --baseline-rev {{baseline}}
