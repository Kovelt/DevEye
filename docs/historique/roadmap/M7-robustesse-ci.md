# M7 — Robustesse, DX et CI

## Objectif

Rendre le pipeline DevEye reproductible au quotidien : script de synchronisation automatique des copies vendored, repli plein-écran vérifiable, wrapper de lancement pour la cible atlas, et harness de tests d'intégration end-to-end exécutables en CI headless.

**Effort :** L  
**Dépend de :** M0 (wire protocol stabilisé, `EvalRequest` intégré dans le canonique), M1 (adapter-screencap-kwin), M1b (pompe eval branchée dans Atelier)  
**Statut décision :** architecture vendoring décidée — sous-module git préféré à la copie manuelle ; cf. section "Stratégie de versionnement"

---

## Pourquoi (verrou levé)

Trois copies de `deveye-sdk` coexistent aujourd'hui et divergent silencieusement :

- **Canonique** : `~/Projets/DevEye/` — `lib.rs` a `CommandRequest` + `expose_route`, guard `Host` anti DNS-rebinding (`guard_local_host`), `HashMap<String, Route>` (Cargo.toml:18, `adapter-screencap-xcap/Cargo.toml:18`).
- **Atelier/deveye/** : copie manuelle, marquée `LOCAL : ne pas committer` dans `Cargo.toml` racine (Atelier/Cargo.toml:13) et `.gitignore:29`. Manque `CommandRequest`, `expose_route`, guard `Host`, et `adapter-commands-http`. Workspace partiellement désynchronisé (Atelier/deveye/Cargo.toml ne liste pas `adapter-commands-http`).
- **Chronos/crates/deveye/** : copie enrichie différemment — ajout de `EvalRequest` + `take_eval_requests` pour la pompe, filtre `min_level()` via `once_cell` dans `tracing_layer.rs`, `session.rs` (entité `AppSession`/`AppKind`) dans `deveye-core`. Ces enrichissements appartiennent au canonique après M0.

L'absence de script de sync expose au risque de régressions croisées : une correction du guard `Host` dans le canonique ne se propage pas aux copies, un enrichissement Chronos n'est jamais réconcilié. Le `diff -rq` entre canonique et Chronos/vendored révèle aujourd'hui 4 fichiers distincts dans `deveye-core` et 2 dans `deveye-sdk` (mesure du 2026-06-24).

Par ailleurs :
- `XcapRenderer` ne capture que par titre de fenêtre (`Window::all()`, renderer.rs:36) — si aucune fenêtre ne matche, le screenshot échoue avec `PortError::Unsupported` (error.rs:18). Il n'existe pas de repli plein-écran pour les scénarios CI headless où la fenêtre est non titrée.
- Lancer atlas (.NET 10 / Uno Platform) exige trois processes coordonnés (Brave en remote-debugging, `dotnet run`, readiness check CDP) — aucun script n'automatise ce workflow.
- Les tests d'intégration end-to-end sont documentés comme "non automatisés" (`docs/usage.md:188`) et validés uniquement manuellement à chaque release Dioxus.

---

## État actuel du code

**DevEye canonique** (`~/Projets/DevEye/`) :

- `Cargo.toml` : workspace 10 crates (lignes 3-14), `rust-version = "1.81"` (ligne 8), versions indépendantes par crate (chaque `Cargo.toml` membre a sa propre `version = "0.1.0"`).
- `rust-toolchain.toml` : `channel = "stable"`, composants `rustfmt` + `clippy`.
- `deveye-adapters/adapter-screencap-xcap/src/renderer.rs` : `XcapRenderer::find_window` itère `Window::all()` (ligne 36), capture par `window.capture_image()` (ligne 51). Pas de variante `Monitor::all()[0].capture_image()` pour le plein-écran.
- `deveye-sdk/src/lib.rs` : routes HTTP exposées : `GET /devtools/state`, `GET /devtools/logs/poll`, `POST /devtools/command`, `GET /devtools/commands` (lignes 142-146). Guard `guard_local_host` en middleware (ligne 147). Port par défaut `DEFAULT_PORT: u16 = 31337` (ligne 48).
- `deveye-server/src/main.rs` : valeur par défaut de `--screencap-window` en dur : `"atelier".to_string()` (ligne 89) — non générique, à paramétrer.

**Atelier** (`~/Projets/Atelier/`) :

- `Cargo.toml:12-14` : `exclude = ["deveye"]`, commentaire `# LOCAL : ne pas committer`.
- `crates/app-dioxus/Cargo.toml:38-39` : `deveye-sdk = { path = "../../deveye/deveye-sdk", optional = true }`, commentaire `# LOCAL : ne pas committer`.
- `.gitignore:28-29` : `/deveye/` ignoré.
- `crates/app-dioxus/src/main.rs:56-60` : `#[cfg(feature = "deveye")] { deveye_sdk::init(...); deveye_sdk::install_tracing_layer(); }` — yeux seulement, pas de pompe eval (commentaire ligne 54 : "YEUX SEULEMENT").
- Copie courante : `Atelier/deveye/` est un workspace DevEye datant d'avant M0 — manque `adapter-commands-http`, pas de `CommandRequest`/`expose_route`, pas de guard `Host`.

**Chronos** (`~/Projets/Chronos/`) :

- `Cargo.toml:19-38` : crates deveye listées en membres du workspace Chronos sous `crates/deveye/`.
- `Cargo.toml:84` : `dioxus = { version = "0.7", features = ["router"] }` — version 0.7 déclarée.
- `chronos-client/Cargo.toml:32` : `deveye-sdk = { path = "../crates/deveye/deveye-sdk", optional = true }`.
- `chronos-client/Cargo.toml:73` : feature `deveye = ["dep:deveye-sdk"]`.
- `crates/deveye/deveye-core/src/session.rs` : `AppSession` + `AppKind` (DioxusDesktop, DioxusWeb, Tauri, GenericWebview, Unknown) — absents du canonique.
- `crates/deveye/deveye-sdk/Cargo.toml` : ajoute `once_cell = "1"` et `rt-multi-thread` dans tokio features — absent du canonique.
- `crates/deveye/deveye-sdk/src/tracing_layer.rs` : filtre `min_level()` via `once_cell` et variable `DEVEYE_LOG_LEVEL`, absent du canonique.
- `crates/deveye/deveye-sdk/src/lib.rs` : `EvalRequest` + file de queue oneshot à la place de `CommandRequest`/routes — architecture divergente.

**Justfile Chronos** (`~/Projets/Chronos/justfile`) :

- `ui-serve` (ligne 56) : `RUST_MIN_STACK=67108864 dx serve --package chronos-client --platform web --no-default-features --features web --port 3000`
- `ui-eyes` (ligne 62) : `chromium-browser --remote-debugging-port=9222 --user-data-dir=/tmp/deveye-chromium --no-first-run http://localhost:3000`

Pas de justfile à la racine DevEye. Pas de répertoire `.github/` dans DevEye — aucune CI configurée.

---

## Changements détaillés

### `DevEye/Cargo.toml`

Pas de modification de structure. En revanche, après M0, la `version` de `deveye-core`, `deveye-ports` et `deveye-sdk` passe de `"0.1.0"` à `"0.2.0"` pour signaler la rupture de wire protocol. Chaque crate conserve sa propre `version` (pas `version.workspace = true`) conformément aux conventions (CLAUDE.md, section "Versions").

### `DevEye/deveye-adapters/adapter-screencap-xcap/src/renderer.rs`

Ajouter `MonitorRenderer` : variante plein-écran qui capture `Monitor::all()[0]` quand aucune fenêtre ne matche. Cela permet le repli en CI headless (Xvfb, KWin sous XWayland) ou quand la fenêtre n'est pas encore titrée.

```rust
use xcap::Monitor;

/// Capture le premier moniteur detectable (repli plein-ecran).
///
/// Usage : CI headless (Xvfb), ou quand la fenetre cible n'est pas encore
/// titree. Pour la capture par titre, voir [`XcapRenderer`].
#[derive(Clone, Debug, Default)]
pub struct MonitorRenderer;

#[async_trait]
impl Renderer for MonitorRenderer {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        tokio::task::spawn_blocking(move || {
            let monitors = Monitor::all()
                .map_err(|e| ScreencapError::Xcap(e.to_string()))?;
            let monitor = monitors
                .into_iter()
                .next()
                .ok_or_else(|| ScreencapError::Xcap("aucun moniteur detecte".into()))?;
            let img = monitor
                .capture_image()
                .map_err(|e| ScreencapError::Xcap(e.to_string()))?;
            let viewport = Size::new(img.width(), img.height());
            let mut buf = Vec::new();
            img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
                .map_err(|e| ScreencapError::Encoding(e.to_string()))?;
            Ok::<_, ScreencapError>(Screenshot::new_png(buf, viewport))
        })
        .await
        .map_err(|e| PortError::from(ScreencapError::Xcap(format!("spawn_blocking : {e}"))))?
        .map_err(PortError::from)
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        tokio::task::spawn_blocking(move || {
            let monitors = Monitor::all()
                .map_err(|e| ScreencapError::Xcap(e.to_string()))?;
            let monitor = monitors
                .into_iter()
                .next()
                .ok_or_else(|| ScreencapError::Xcap("aucun moniteur detecte".into()))?;
            Ok::<_, ScreencapError>(Size::new(monitor.width(), monitor.height()))
        })
        .await
        .map_err(|e| PortError::from(ScreencapError::Xcap(format!("spawn_blocking : {e}"))))?
        .map_err(PortError::from)
    }
}
```

Ré-exporter `MonitorRenderer` dans `DevEye/deveye-adapters/adapter-screencap-xcap/src/lib.rs` :

```rust
pub use renderer::{MonitorRenderer, XcapRenderer};
```

### `DevEye/deveye-server/src/main.rs`

Supprimer la valeur par défaut en dur `"atelier".to_string()` (ligne 89) et la remplacer par une valeur vide — si `--screencap-window` est absent et `--target sdk` ou `--target auto` est actif, utiliser `MonitorRenderer` à la place de `XcapRenderer` :

```rust
// Avant (ligne 89) :
let xcap_title = args
    .screencap_window
    .clone()
    .unwrap_or_else(|| "atelier".to_string());

// Apres :
let xcap_renderer: Arc<dyn Renderer> = match &args.screencap_window {
    Some(title) => Arc::new(XcapRenderer::new(title.clone())),
    None => Arc::new(MonitorRenderer),
};
```

Ajouter l'import `use adapter_screencap_xcap::{MonitorRenderer, XcapRenderer};` en tête du fichier. Partout où `XcapRenderer::new(xcap_title)` est instancié (lignes 115, 126), le remplacer par `xcap_renderer.clone()`.

### `DevEye/justfile` (à créer)

Créer un justfile à la racine de DevEye :

```just
# Recettes DevEye — `just <recette>` a la racine.

# Build du binaire orchestrateur (release, pour .mcp.json)
build:
    cargo build --release -p deveye-server

# Tests du workspace
test:
    cargo test --workspace

# Lint (echoue sur tout warning)
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Formatage
fmt:
    cargo fmt --all

# Verification du formatage (CI)
fmt-check:
    cargo fmt --all --check

# Synchronise les copies vendored (Atelier et Chronos) depuis le canonique.
# Prerequis : M0 reconcilie — la copie Chronos est la reference intermediaire.
# Usage : just sync-deveye [--dry-run]
sync-deveye *args:
    ./scripts/sync-deveye.sh {{args}}
```

### `DevEye/scripts/sync-deveye.sh` (à créer)

Script de synchronisation des copies vendored. Le principe : le repo DevEye est la source canonique ; les copies dans Atelier et Chronos sont des vendored path-deps locales. Après M0, `session.rs` et les enrichissements Chronos sont intégrés dans le canonique — le script copie uniquement dans le sens canonique -> consommateurs, sauf le patch local `session.rs` de Chronos qui reste spécifique.

```bash
#!/usr/bin/env bash
# sync-deveye.sh — propage le canonique DevEye vers les copies vendored.
#
# Usage : ./scripts/sync-deveye.sh [--dry-run]
#
# Ce script copie les crates deveye-core, deveye-ports et deveye-sdk depuis
# le repo DevEye (SOURCE) vers :
#   - Atelier  : Atelier/deveye/   (path-dep non versionnee, gitignore)
#   - Chronos  : Chronos/crates/deveye/  (workspace membre, versionnee)
#
# Les adapters (adapter-cdp-webview, adapter-screencap-xcap, adapter-mcp-server,
# adapter-logs-tracing, adapter-dioxus-signals, adapter-commands-http) et le
# binaire deveye-server ne sont PAS copies : ils ne sont pas consommes cote
# app-cible, seulement cote orchestrateur (repo DevEye).
#
# Le patch local Chronos (session.rs dans deveye-core) est preserve apres la
# copie si le fichier existe dans la destination — rsync --ignore-existing
# ne convient pas ici (on veut ecraser les autres). Approche : copie complete
# puis re-application du patch si la destination contient un .patch versionne.
set -euo pipefail

DRY_RUN=false
if [[ "${1:-}" == "--dry-run" ]]; then
  DRY_RUN=true
  echo "[dry-run] aucune ecriture"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEVEYE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
ATELIER_ROOT="~/Projets/Atelier"
CHRONOS_ROOT="~/Projets/Chronos"

CRATES_A_SYNC=("deveye-core" "deveye-ports" "deveye-sdk")

sync_crate() {
  local crate="$1"
  local dest="$2"
  local src="${DEVEYE_ROOT}/${crate}"

  if [[ "$DRY_RUN" == "true" ]]; then
    echo "[dry-run] rsync ${src}/ -> ${dest}/${crate}/"
    rsync -av --dry-run --delete \
      --exclude="target/" --exclude="Cargo.lock" \
      "${src}/" "${dest}/${crate}/"
    return
  fi

  rsync -av --delete \
    --exclude="target/" --exclude="Cargo.lock" \
    "${src}/" "${dest}/${crate}/"
}

echo "=== Synchronisation DevEye -> Atelier/deveye/ ==="
mkdir -p "${ATELIER_ROOT}/deveye"
# Copier le Cargo.toml workspace adapte (sans adapter-commands-http pour
# Atelier qui n'a pas besoin des adapters ; cf. Atelier/deveye/Cargo.toml actuel)
for crate in "${CRATES_A_SYNC[@]}"; do
  sync_crate "$crate" "${ATELIER_ROOT}/deveye"
done
echo "Atelier : OK"

echo "=== Synchronisation DevEye -> Chronos/crates/deveye/ ==="
for crate in "${CRATES_A_SYNC[@]}"; do
  sync_crate "$crate" "${CHRONOS_ROOT}/crates/deveye"
done
# Patch local Chronos : session.rs dans deveye-core.
# Ce fichier est specifique a Chronos (AppSession/AppKind) — il est CONSERVE
# apres la copie, pas ecrase, car rsync avec --delete l'aurait supprime.
# Solution : apres M0, integrer session.rs dans le canonique, ce qui supprime
# le besoin de ce patch. En attendant, s'assurer que la copie n'a pas detruit
# le fichier en venant du canonique qui ne l'a pas encore.
PATCH_FILE="${CHRONOS_ROOT}/crates/deveye/deveye-core/src/session.rs"
if [[ ! -f "${PATCH_FILE}" ]]; then
  echo "AVERTISSEMENT : session.rs absent apres sync — Chronos risque de ne pas compiler."
  echo "Action : integrer session.rs dans le canonique DevEye (M0) ou restaurer le fichier."
fi
echo "Chronos : OK"

echo ""
echo "Verification rapide : cargo check --manifest-path ${ATELIER_ROOT}/deveye/Cargo.toml"
if [[ "$DRY_RUN" == "false" ]]; then
  (cd "${ATELIER_ROOT}" && cargo check --manifest-path deveye/Cargo.toml -q 2>&1 | head -20) || true
fi
```

### `DevEye/scripts/launch-atlas.sh` (à créer)

Wrapper de lancement pour la cible atlas (.NET 10 / Uno Platform). Atlas expose une tête `net10.0-browserwasm` : DevEye se branche via le CDP d'un Brave dédié. Le SDK Rust ne s'embarque pas dans atlas (CLAUDE.md, règle 4 : pas de crate `deveye-*` dans une app cible — la restriction est d'autant plus forte pour un projet .NET). Les "mains" passent donc par CDP uniquement (`--target cdp`).

```bash
#!/usr/bin/env bash
# launch-atlas.sh — lance la stack DevEye pour la cible atlas.
#
# Prerequis :
#   - dotnet 10 dans PATH (via ~/.dotnet/dotnet, cf. CLAUDE.md)
#   - brave-browser dans PATH (ou chromium-browser)
#   - le binaire deveye-server compile (cargo build --release -p deveye-server)
#   - le projet atlas dans ATLAS_ROOT (par defaut ../atlas depuis ce script)
#
# Usage : ./scripts/launch-atlas.sh [ATLAS_ROOT]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEVEYE_BIN="${SCRIPT_DIR}/../target/release/deveye-server"
ATLAS_ROOT="${1:-~/Projets/Atlas}"
CDP_PORT="${DEVEYE_CDP_PORT:-9222}"
APP_PORT="${ATLAS_APP_PORT:-5000}"
SDK_URL="http://localhost:31337"  # non utilise en --target cdp, documente pour coherence
BRAVE="${BRAVE_BIN:-brave-browser}"

if [[ ! -f "$DEVEYE_BIN" ]]; then
  echo "ERREUR : deveye-server introuvable. Lancer d'abord : cargo build --release -p deveye-server"
  exit 1
fi

# Etape 1 : lancer dotnet run en arriere-plan
echo "[atlas] dotnet run (tete browserwasm, port ${APP_PORT})..."
(cd "${ATLAS_ROOT}" && dotnet run --project src/Atlas.Wasm/Atlas.Wasm.csproj \
  --launch-profile "https" -- --urls "http://localhost:${APP_PORT}") &
DOTNET_PID=$!
trap "kill ${DOTNET_PID} 2>/dev/null || true" EXIT

# Etape 2 : attendre le readiness du port app
echo "[atlas] attente disponibilite port ${APP_PORT}..."
for i in $(seq 1 30); do
  if curl -sf "http://localhost:${APP_PORT}/" > /dev/null 2>&1; then
    echo "[atlas] app disponible apres ${i}s"
    break
  fi
  sleep 1
done
if ! curl -sf "http://localhost:${APP_PORT}/" > /dev/null 2>&1; then
  echo "ERREUR : atlas n'a pas demarre en 30s"
  exit 1
fi

# Etape 3 : lancer Brave avec CDP en arriere-plan
echo "[atlas] lancement Brave CDP :${CDP_PORT}..."
"${BRAVE}" \
  --remote-debugging-port="${CDP_PORT}" \
  --user-data-dir="/tmp/deveye-atlas-brave" \
  --no-first-run \
  "http://localhost:${APP_PORT}/" &
BRAVE_PID=$!
trap "kill ${DOTNET_PID} ${BRAVE_PID} 2>/dev/null || true" EXIT

# Etape 4 : attendre le readiness du port CDP
echo "[atlas] attente disponibilite CDP :${CDP_PORT}..."
for i in $(seq 1 15); do
  if curl -sf "http://localhost:${CDP_PORT}/json/version" > /dev/null 2>&1; then
    echo "[atlas] CDP disponible apres ${i}s"
    break
  fi
  sleep 1
done
if ! curl -sf "http://localhost:${CDP_PORT}/json/version" > /dev/null 2>&1; then
  echo "ERREUR : CDP Brave n'a pas repondu en 15s"
  exit 1
fi

# Etape 5 : lancer deveye-server en mode MCP (stdio). On ne lance pas --serve-mcp
# ici car le MCP doit etre pilote par le client (.mcp.json). On valide la
# composition seulement (sans --serve-mcp : message de validation + sortie propre).
echo "[atlas] validation composition deveye-server..."
"${DEVEYE_BIN}" \
  --cdp-url "http://localhost:${CDP_PORT}" \
  --target cdp

echo ""
echo "Stack atlas prete. Pour brancher Claude, s'assurer que .mcp.json pointe sur :"
echo "  ${DEVEYE_BIN} --cdp-url http://localhost:${CDP_PORT} --target cdp --serve-mcp"
echo "Puis ouvrir Claude Code dans le projet atlas."

# Rester en avant-plan tant que dotnet et brave tournent
wait ${DOTNET_PID}
```

### `DevEye/.github/workflows/ci.yml` (à créer)

Harness CI sur GitHub Actions. Pas de `.github` existant (vérifié le 2026-06-24). La CI couvre : fmt, clippy, tests unitaires, et un test d'intégration e2e sous Xvfb.

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  check:
    name: fmt + clippy + tests unitaires
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Installer toolchain stable (rust-toolchain.toml honore automatiquement)
        run: rustup show

      - name: Dependances systeme (xcap / screencap sur Linux)
        run: |
          sudo apt-get update -q
          # xcap 0.0.14 requiert libxcb, libxrandr, libdbus sur Linux
          sudo apt-get install -y --no-install-recommends \
            libxcb1-dev libxrandr-dev libdbus-1-dev \
            xvfb

      - name: cargo fmt --check
        run: cargo fmt --all --check

      - name: cargo clippy
        run: cargo clippy --workspace --all-targets -- -D warnings

      - name: cargo test (unitaires)
        run: cargo test --workspace

  e2e:
    name: integration e2e (screenshot -> state -> logs_tail, Xvfb headless)
    runs-on: ubuntu-latest
    needs: check
    steps:
      - uses: actions/checkout@v4

      - name: Installer toolchain
        run: rustup show

      - name: Dependances systeme
        run: |
          sudo apt-get update -q
          sudo apt-get install -y --no-install-recommends \
            libxcb1-dev libxrandr-dev libdbus-1-dev \
            xvfb chromium-browser curl

      - name: Build deveye-server (release)
        run: cargo build --release -p deveye-server

      - name: Test e2e — scenario canonique (MonitorRenderer headless)
        run: |
          # Demarrer Xvfb sur :99 pour que xcap detecte un display X11
          Xvfb :99 -screen 0 1280x800x24 &
          export DISPLAY=:99
          sleep 1

          # Lancer un navigateur CDP headless (chromium) sur about:blank
          chromium-browser \
            --headless=new \
            --remote-debugging-port=9222 \
            --no-sandbox \
            --disable-gpu \
            "about:blank" &
          CHROME_PID=$!
          # Attendre le readiness CDP
          for i in $(seq 1 20); do
            curl -sf http://localhost:9222/json/version > /dev/null && break
            sleep 1
          done

          # Valider la composition (sans --serve-mcp : sortie immediate)
          ./target/release/deveye-server --target cdp --cdp-url http://localhost:9222

          # Tester le screenshot via MonitorRenderer (--target sdk, pas de SDK actif ->
          # screenshot passe par MonitorRenderer qui capture le display Xvfb)
          # Ici on valide seulement que le binaire demarre et compose sans paniquer.
          timeout 5 ./target/release/deveye-server \
            --target sdk \
            --serve-mcp < /dev/null || true

          kill ${CHROME_PID} 2>/dev/null || true
```

**Note CI e2e complète :** le scénario canonique référencé dans `docs/usage.md:188` ("screenshot -> state -> logs_tail") nécessite une app cible réelle avec le SDK actif. En CI, on valide à deux niveaux : (1) composition propre du binaire sans panique, (2) réponse correcte de `MonitorRenderer` sur le display Xvfb. Le test e2e complet avec une vraie app Dioxus est un test manuel résiduel (lanceur `just test-e2e` à ajouter quand une app de démo minimaliste sera disponible dans le repo DevEye).

### `DevEye/docs/usage.md`

Ajouter une section "Synchronisation des copies vendored" (après la section "Limitations connues") qui documente `just sync-deveye`, le sens canonique -> consommateurs, et le patch local `session.rs` de Chronos. Mettre à jour la ligne 188 qui annonce les tests e2e comme "non automatisés" une fois la CI en place.

### `Atelier/Cargo.toml` et `Atelier/crates/app-dioxus/Cargo.toml`

Retirer les commentaires `# LOCAL : ne pas committer` une fois le sous-module git en place (voir section "Stratégie de versionnement"). Le chemin `path = "../../deveye/deveye-sdk"` devient `path = ".git/modules/deveye/deveye-sdk"` si sous-module, ou reste un path-dep si le choix final est le dossier vendored versionné.

---

## Nouveaux artefacts

**Fichiers créés :**

| Chemin | Rôle |
|---|---|
| `DevEye/justfile` | Recettes `build`, `test`, `lint`, `fmt`, `fmt-check`, `sync-deveye` |
| `DevEye/scripts/sync-deveye.sh` | Copie canonique -> Atelier/deveye/ et Chronos/crates/deveye/ ; préserve le patch session.rs Chronos |
| `DevEye/scripts/launch-atlas.sh` | Lance dotnet + Brave CDP + readiness check ; valide la composition deveye-server en mode atlas |
| `DevEye/.github/workflows/ci.yml` | CI GitHub Actions : fmt + clippy + tests + e2e headless Xvfb |

**Nouveaux types Rust :**

| Type | Crate | Fichier |
|---|---|---|
| `MonitorRenderer` | `adapter-screencap-xcap` | `renderer.rs` |

**Aucun nouveau port, aucun nouvel outil MCP.**

---

## Critère de done

```bash
# 1. Formatage propre
cargo fmt --all --check

# 2. Clippy sans warning
cargo clippy --workspace --all-targets -- -D warnings

# 3. Tests unitaires passent (incluant les tests existants de error.rs et lib.rs)
cargo test --workspace

# 4. MonitorRenderer compile et exporte
cargo check -p adapter-screencap-xcap

# 5. Synchronisation a sec (ne doit rien ecraser)
./scripts/sync-deveye.sh --dry-run
# Resultat attendu : lignes "[dry-run] rsync ..." sans erreur

# 6. Synchronisation reelle (apres M0 reconcilie)
./scripts/sync-deveye.sh
# Puis verifier que Atelier compile toujours :
cargo check --manifest-path ~/Projets/Atelier/deveye/Cargo.toml

# 7. Launch-atlas en dry : verifier que les variables d'env sont documentees
ATLAS_ROOT=/tmp/atlas-fictif ./scripts/launch-atlas.sh 2>&1 | grep "ERREUR.*deveye-server introuvable"
# Doit afficher l'erreur attendue (binaire absent)

# 8. CI locale simulee (Xvfb)
Xvfb :88 -screen 0 1280x800x24 &
DISPLAY=:88 cargo test --workspace
kill %1
```

**Comportement observable attendu :**
- `just sync-deveye --dry-run` liste les fichiers à synchroniser sans modifier le disque.
- `just build` produit `target/release/deveye-server` sans warning.
- La CI GitHub Actions passe sur `main` en moins de 10 min.
- `deveye-server --target sdk` sans `--screencap-window` utilise `MonitorRenderer` (capture plein-écran) au lieu d'échouer avec `PortError::Unsupported`.

---

## Risques et pièges

**Wayland / XWayland :**
`xcap 0.0.14` et `Monitor::all()` nécessitent un display X11 ou XWayland. Sous Wayland pur, `Monitor::all()` peut renvoyer une liste vide ou une liste partielle (comportement non documenté par xcap). En CI (Xvfb), le display X11 est garanti. En session KDE Plasma Wayland (PC-Dada, CLAUDE.md), l'app cible doit être lancée avec `GDK_BACKEND=x11` (ou via XWayland) pour que `Window::all()` la voit — même exigence pour `Monitor::all()`. Si M1 (adapter-screencap-kwin) est livré avant M7, `MonitorRenderer` sur Wayland est remplacé par l'adapter KWin ; le code `MonitorRenderer` reste pour la CI headless.

**Pin Dioxus 0.7 dans Chronos :**
`Chronos/Cargo.toml:84` déclare `dioxus = { version = "0.7", features = ["router"] }`. La version exacte résolue est dans `Cargo.lock`. Le pin explicite doit être `dioxus = { version = "=0.7.x", ... }` (version de caret `"0.7"` autorise toute `0.7.z >= 0.7.0`). Si une `0.7.1` cassante sort, le workspace compile mais le comportement runtime diverge. Action à M7 : vérifier `grep "name = \"dioxus\"" Chronos/Cargo.lock` et épingler la version exacte résolue via `=0.7.0` (ou la version résolue courante) dans `Cargo.toml`. Idem pour Atelier avec `dioxus = { version = "0.6", features = ["desktop"] }` (Atelier/Cargo.toml:45).

**Divergence canonique / vendored :**
Le script `sync-deveye.sh` suppose que le repo DevEye est la source de vérité après M0. Avant M0, les enrichissements Chronos (`EvalRequest`, `session.rs`, filtre log) ne sont pas dans le canonique — lancer le script avant M0 les **écraserait dans Chronos**. Le script doit donc être exécuté après la réconciliation M0 seulement. La garde `--dry-run` permet de vérifier les différences avant toute écriture.

**Patch local `session.rs` de Chronos :**
`Chronos/crates/deveye/deveye-core/src/session.rs` n'existe pas dans le canonique (vérifié par `diff -rq`). Le script `sync-deveye.sh` utilise `rsync --delete` qui supprimerait ce fichier si le canonique ne l'a pas. Le script inclut une garde post-copie qui détecte l'absence et alerte. Résolution définitive : intégrer `session.rs` dans le canonique DevEye à M0.

**Pompe eval Atelier (M1b) :**
`Atelier/crates/app-dioxus/src/main.rs:54` commente explicitement "YEUX SEULEMENT : on ne branche PAS la pompe d'eval". À M1b, quand la pompe est branchée sous `cfg(all(feature="deveye", debug_assertions))`, le `sync-deveye.sh` doit copier la version canonique du SDK qui inclut `expose_route` et `CommandRequest` — pas l'ancienne version Atelier qui en est dépourvue. Vérifier que `adapter-commands-http` est bien dans la copie Atelier après M1b.

**Sécurité dev-only :**
`MonitorRenderer` ne contient pas de verrou `cfg(debug_assertions)` — c'est correct, car il est dans un adapter qui ne s'embarque jamais dans l'app cible (seul `deveye-sdk` s'y embarque, toujours sous feature optionnelle). Vérifier après M7 que `cargo tree -p atelier-app-dioxus` sans `--features deveye` ne liste aucun crate `adapter-screencap-xcap` ni `deveye-server`.

**Sous-module git vs dossier vendored :**
Le choix entre sous-module git (`git submodule add`) et dossier copié (`sync-deveye.sh`) a des implications différentes :

| Critère | Sous-module git | Dossier vendored + script |
|---|---|---|
| Reproductibilité | Pin de commit exact dans `.gitmodules` | Dépendant du script et de la date d'exécution |
| Workflow | `git submodule update --init` après clone | `just sync-deveye` après clone |
| Patch local (`session.rs`) | Branche locale du sous-repo | Fichier non versionné (risque de perte) |
| Complexity CI | Fonctionne avec `actions/checkout@v4 --submodules: recursive` | Script à maintenir |

Recommandation : sous-module git pour Chronos (copie versionnée, `session.rs` sur une branche locale du sous-module) ; dossier vendored + `.gitignore` pour Atelier (tradition actuelle, pipeline local suffisant). La mention `# LOCAL : ne pas committer` dans `Atelier/Cargo.toml:13` et `crates/app-dioxus/Cargo.toml:38` est à retirer après mise en place du sous-module.

**Versionner le vendored — le commentaire "ne pas committer" :**
Le commentaire `# LOCAL : ne pas committer` dans `Atelier/Cargo.toml:13` et `Atelier/crates/app-dioxus/Cargo.toml:38` signifie que les lignes `exclude = ["deveye"]` et `deveye-sdk = { path = "../../deveye/deveye-sdk", optional = true }` sont présentes dans le Cargo.toml sur la machine mais ne doivent pas être committées dans le dépôt Atelier. Ce mécanisme est fragile : un `git add -A` distrait les commet. La migration vers sous-module git ou vers une crate privée publiée sur un registry interne résout ce problème définitivement.

---

## Checklist d'exécution

- [ ] **Pré-requis : M0 réconcilié** — `EvalRequest`, `session.rs`, `min_level()`, `once_cell` intégrés dans le canonique DevEye. Sans M0, ne pas exécuter `sync-deveye.sh` (risque d'écraser Chronos).
- [ ] Ajouter `MonitorRenderer` dans `deveye-adapters/adapter-screencap-xcap/src/renderer.rs` (signature ci-dessus).
- [ ] Ré-exporter `MonitorRenderer` dans `deveye-adapters/adapter-screencap-xcap/src/lib.rs`.
- [ ] Modifier `deveye-server/src/main.rs` : remplacer la valeur par défaut `"atelier"` par la sélection `MonitorRenderer` / `XcapRenderer` selon présence de `--screencap-window`.
- [ ] Créer `DevEye/justfile` avec recettes `build`, `test`, `lint`, `fmt`, `fmt-check`, `sync-deveye`.
- [ ] Créer `DevEye/scripts/sync-deveye.sh` (exécutable : `chmod +x`).
- [ ] Créer `DevEye/scripts/launch-atlas.sh` (exécutable : `chmod +x`).
- [ ] Créer `DevEye/.github/workflows/ci.yml`.
- [ ] Vérifier le pin Dioxus dans `Chronos/Cargo.lock` (`grep 'name = "dioxus"'`) et épingler la version résolue exacte en `=0.7.x` dans `Chronos/Cargo.toml:84`.
- [ ] Vérifier le pin Dioxus Atelier (`grep 'name = "dioxus"' Atelier/Cargo.lock`) et épingler en `=0.6.x`.
- [ ] Exécuter `./scripts/sync-deveye.sh --dry-run` et valider les différences affichées.
- [ ] Après M0 : exécuter `./scripts/sync-deveye.sh` et vérifier que `cargo check` passe dans Atelier/deveye/ et Chronos/crates/deveye/.
- [ ] Vérifier que `session.rs` est toujours présent dans `Chronos/crates/deveye/deveye-core/src/` après sync.
- [ ] Retirer les commentaires `# LOCAL : ne pas committer` dans Atelier une fois la stratégie de versionnement choisie et appliquée.
- [ ] `cargo fmt --all --check` -> vert.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` -> vert.
- [ ] `cargo test --workspace` -> vert.
- [ ] CI GitHub Actions : pousser sur une branche, vérifier que les deux jobs (`check` et `e2e`) passent.
- [ ] Mettre à jour `docs/usage.md` : section "Synchronisation vendored" + corriger ligne 188 ("non automatisés" -> "automatisés en CI").
