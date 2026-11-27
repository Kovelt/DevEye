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

# Documentation, avertissements rustdoc = erreurs (comme en CI)
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

# Tests de la cible x11 contre un Xvfb prive (requiert le paquet Xvfb)
x11:
    cargo test -p adapter-x11 --locked -- --ignored --test-threads=1

# Session MCP de bout en bout : binaire, stdio, Xvfb prive, fenetre fixture
e2e:
    cargo test -p deveye-server --test e2e_mcp --locked -- --ignored --test-threads=1

# MSRV : verifie le plancher annonce par rust-version (requiert `rustup toolchain install 1.88`)
msrv:
    cargo +1.88 check --workspace --all-targets --locked

# Licences, avis RustSec, sources (requiert cargo-deny)
deny:
    cargo deny check

# Preuve que le SDK est un no-op en release : test, puis inspection du binaire
release-noop:
    cargo test --release --locked -p deveye-sdk --test release_noop
    scripts/verify-sdk-release-noop.sh

# SBOM CycloneDX, un fichier .cdx.json par crate (requiert cargo-cyclonedx)
sbom:
    cargo cyclonedx --format json

# Installe deveye-server dans ~/.local/bin (build release du depot local).
# Alternative : `cargo install --path deveye-server --locked` (vers ~/.cargo/bin).
install:
    cargo build --release --locked -p deveye-server
    install -Dm755 target/release/deveye-server "$HOME/.local/bin/deveye-server"

# Pipeline CI complet en local (hors msrv et sbom, qui exigent des outils a part)
ci: fmt-check lint test doc x11 e2e deny conformance release-noop
