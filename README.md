# DevEye

**Outillage dev-only pour observer et piloter une app graphique vivante depuis un agent IA.**
DevEye expose un serveur MCP (Model Context Protocol) que Claude Code — ou tout autre client MCP — utilise pour capturer des screenshots, envoyer des clics/saisies, inspecter le DOM, lire les logs et attendre des conditions, sans intervention humaine.
Conçu pour les apps Dioxus desktop, il se branche aussi sur n'importe quelle cible WebView (Chrome DevTools Protocol), app native Wayland, app X11 native dans un serveur isolé (egui, GTK, Qt…), ou app .NET/Uno.

> Outil interne, **dev-only**. N'a pas vocation à être déployé en production.

---

## Caractéristiques clés

### 41 outils MCP, en six familles

| Famille | Outils |
|---|---|
| **Capture** | `screenshot`, `screenshot_region`, `screenshot_element`, `assert_visual` |
| **Interaction** | `click`, `double_click`, `right_click`, `hover`, `drag_drop`, `click_selector`, `click_text`, `type_text`, `press_key`, `key_combo`, `scroll`, `navigate`, `handle_dialog`, `upload_file`, `focus_window`, `clipboard_get`, `clipboard_set` |
| **Inspection DOM / state** | `dom`, `find_element`, `assert_selector`, `assert_state`, `eval_js`, `state`, `collect_console_errors`, `list_windows`, `target_info` |
| **Logs** | `logs_tail` |
| **Attente / conditions** | `wait_for_selector` (+ `negate` = wait_for_gone), `wait_condition`, `wait_for_log`, `wait_screenshot_region`, `wait_ready` |
| **Commandes logiques** | `list_commands`, `invoke_command` |
| *(à venir)* | `resize` |

Détail complet des paramètres et valeurs de retour : [docs/usage.md](docs/usage.md).

### 5 familles de cibles (`--target`)

| Mode | Description |
|---|---|
| `auto` (défaut) | Tente CDP ; bascule appel par appel sur le pont SDK si CDP est injoignable |
| `cdp` | Chrome DevTools Protocol — webview Chromium (Dioxus/wry sous Windows, Brave/Chrome standalone) |
| `sdk` | Pont natif via `deveye-sdk` embarqué dans l'app (Linux/WebKitGTK, Wayland sans CDP) |
| `kwin-eis` | Injection OS-level via KWin EIS (`org.kde.KWin.EIS.RemoteDesktop`) — cibles sans WebView ni SDK (ex. Uno/Skia) ; yeux par screencap |
| `x11` | Serveur X **privé et explicite** (`--display :250`, typiquement Xvfb) : yeux `GetImage`, mains XTEST, fenêtres `QueryTree` — toute app X11 native (egui, GTK, Qt…), sans WebView ni gestionnaire de fenêtres. Refuse structurellement la session graphique vivante |

### Architecture hexagonale

12 ports (traits Rust purs dans `deveye-ports`), un adapter par technologie, un binaire orchestrateur (`deveye-server`) qui les compose selon les options CLI. Ajouter une cible = nouvel adapter, pas de refonte.

---

## Sécurité / posture dev-only

DevEye est strictement gardé contre toute fuite en production :

- **SDK no-op en release** — `deveye_sdk::init()` est un no-op intégral quand `debug_assertions` est désactivé ; le binaire release de l'app cible n'embarque aucun code DevEye même si l'intégrateur oublie son `#[cfg(debug_assertions)]`.
- **Bind loopback uniquement** — le mini-serveur SDK écoute sur `127.0.0.1` (jamais `0.0.0.0`).
- **Garde Host anti DNS-rebinding** — le middleware `guard_local_host` rejette en `403` toute requête dont le header `Host` n'est pas `localhost`/`127.0.0.1`/`[::1]`, avant toute route.
- **CORS deny-all** — aucune politique CORS permissive.
- **`#[forbid(unsafe_code)]`** — posé sur `deveye-sdk` et `adapter-mcp-server`.
- **Cible `x11` isolée par construction** — `--display` est obligatoire (aucun repli sur `$DISPLAY`) ; l'adapter refuse un display non local, un numéro < 100 (donc pas le `Xvfb :99` habituel des CI : prendre `:100` ou plus), le display de la session courante, un socket appartenant à un autre uid et tout serveur annonçant l'extension `XWAYLAND`. Il n'existe aucune option pour lever ces refus.
- **Canal d'éval borné (backpressure)** — la file des requêtes d'évaluation JS est bornée (capacité 32) : si la pompe de l'app ne draine plus (app figée), `POST /devtools/eval` répond `503` « file pleine » au lieu de laisser la mémoire enfler. Le buffer de logs est, lui, plafonné à 1 024 événements (drop-oldest).

Contrat complet du wire protocol HTTP SDK : [docs/wire-protocol.md](docs/wire-protocol.md).

---

## Démarrage rapide

### Prérequis

- Toolchain Rust **stable** (`rustup update stable`) — MSRV : 1.88
- Bibliothèques système (les mêmes que le `before_script` CI) :
  - Fedora : `sudo dnf install libxcb-devel libXrandr-devel dbus-devel libxkbcommon-devel mesa-libgbm-devel`
  - Debian/Ubuntu : `sudo apt install libxcb1-dev libxrandr-dev libdbus-1-dev libxkbcommon-dev libgbm-dev`

  (xcap → libxcb/libXrandr + libgbm ; zbus → libdbus ; injection EIS → libxkbcommon)
- Pour les cibles KWin : session KDE Plasma Wayland
- Pour la cible `x11` : un serveur X privé (`Xvfb`, paquet `xorg-x11-server-Xvfb` / `xvfb`) — aucune bibliothèque système en plus (client X11 en Rust pur)
- Un client MCP (Claude Code recommandé)

### Build

```bash
cargo build --release -p deveye-server
# ou, avec just :
just build
```

Le binaire atterrit dans `target/release/deveye-server`.

### Lancer le serveur MCP

```bash
# Mode CDP (app Dioxus desktop sous Windows avec WebView2 sur :9222)
./target/release/deveye-server \
  --cdp-url http://localhost:9222 \
  --serve-mcp

# Mode SDK (Linux desktop WebKitGTK, SDK sur :31337)
./target/release/deveye-server \
  --target sdk \
  --sdk-url http://127.0.0.1:31337 \
  --serve-mcp

# Mode kwin-eis (app native Wayland sans WebView, ex. Uno/Skia)
./target/release/deveye-server \
  --target kwin-eis \
  --screencap-window mon_app \
  --serve-mcp

# Mode x11 (app X11 native dans un Xvfb privé — display >= 100, jamais la session vivante)
Xvfb :250 -nolisten tcp -screen 0 1400x1000x24 &
DISPLAY=:250 ./mon-app &          # l'app tourne SUR le serveur privé
./target/release/deveye-server \
  --target x11 \
  --display :250 \
  --window "Mon App" \
  --sdk-url http://127.0.0.1:31400 \
  --serve-mcp

# CDP sur un navigateur externe (tête WASM sur localhost:5000)
./target/release/deveye-server \
  --target cdp \
  --cdp-url http://localhost:9222 \
  --cdp-target-filter-url localhost:5000 \
  --serve-mcp
```

### Brancher dans Claude Code (`.mcp.json`)

Créer `.mcp.json` à la racine du projet client :

```json
{
  "mcpServers": {
    "deveye": {
      "type": "stdio",
      "command": "/chemin/absolu/vers/DevEye/target/release/deveye-server",
      "args": [
        "--cdp-url", "http://localhost:9222",
        "--sdk-url", "http://127.0.0.1:31337",
        "--serve-mcp"
      ]
    }
  }
}
```

Variante développement (recompile à la volée) :

```json
"command": "cargo",
"args": ["run", "--quiet", "-p", "deveye-server", "--", "--serve-mcp"]
```

Vérifier dans Claude Code : `/mcp` → `deveye` doit apparaître. Si `disabled` : `/mcp approve deveye`.

Pour le détail complet (options, Linux sans CDP, cible WASM, troubleshooting) : [docs/usage.md](docs/usage.md).

---

## Architecture en bref

```
Client MCP (Claude Code)
        │  stdio (JSON-RPC)
        ▼
┌─────────────────────────────────┐
│  deveye-server                  │  binaire orchestrateur
│  compose les ports selon --target│
└────────────┬────────────────────┘
             │
     ┌───────┴────────┐
     │  adapter-mcp-  │  expose les 41 outils MCP
     │  server        │  (sérialise les appels — 1 à la fois)
     └───────┬────────┘
             │ implémente les 12 ports (traits deveye-ports)
    ┌────────┼────────────────────┐
    │        │                    │
    ▼        ▼                    ▼
adapter-   adapter-           adapter-           adapter-
cdp-       screencap-kwin /   interaction-       x11
webview    xcap               kwin-eis           (GetImage + XTEST
    │                             │               + QueryTree)
    ▼                             ▼                    │
App WebView                App native                  ▼
(Dioxus/wry,              (Uno/Skia, etc.)      App X11 native dans
 Chromium CDP)                                  un serveur X privé
         │ (opt-in, cfg debug)
         ▼
    deveye-sdk
    (HTTP :31337)
```

Les 12 ports : `Renderer`, `AppInspector`, `InteractionSink`, `JsEvaluator`, `LogSource`, `CommandRouter`, `ConsoleErrorSource`, `WaitCondition`, `WindowLister`, `WindowManager`, `Clipboard`, `WindowGeometry`.

Architecture détaillée (hexagonale, ports, robustesse CDP, reconnexion paresseuse) : [docs/architecture.md](docs/architecture.md).

---

## Structure du dépôt

```
DevEye/
├── deveye-core/                  # Domaine pur — types métier (Snapshot, InteractionCommand, LogEvent…)
├── deveye-ports/                 # Interfaces — 12 traits Rust sans dépendance technique
├── deveye-sdk/                   # Micro-lib opt-in à embarquer dans l'app cible (cfg debug_assertions)
├── deveye-sdk-client/            # Client HTTP du SDK (partagé entre les adapters)
├── deveye-server/                # Binaire orchestrateur — compose les adapters selon la CLI
├── deveye-adapters/
│   ├── adapter-cdp-webview/      # Renderer + Interaction + Inspector + JsEval via CDP (Chromium/WebView2)
│   ├── adapter-screencap-xcap/   # Renderer OS-level (X11 / XWayland / Wayland non-KDE)
│   ├── adapter-screencap-kwin/   # Renderer + WindowManager (focus_window) KDE Wayland (KWin D-Bus)
│   ├── adapter-dioxus-signals/   # Inspector + JsEval via le pont natif SDK (Dioxus signals)
│   ├── adapter-logs-tracing/     # LogSource via le subscriber tracing du SDK opt-in
│   ├── adapter-commands-http/    # CommandRouter via POST /devtools/command + GET /devtools/commands
│   ├── adapter-interaction-kwin-eis/ # InteractionSink OS-level Wayland natif (KWin EIS)
│   ├── adapter-clipboard-wayland/ # Clipboard OS-level via wl-copy / wl-paste (Wayland)
│   ├── adapter-x11/              # Renderer + InteractionSink + WindowLister + WindowGeometry + WindowManager sur un serveur X privé (x11rb, XTEST)
│   ├── adapter-mcp-server/       # Consomme les 12 ports, expose les 41 outils MCP via rmcp/stdio
│   └── adapter-composite/        # Adapters composites génériques (ConsoleErrorSource JS, WaitCondition, Inspector, DOM par route)
└── docs/
    ├── architecture.md           # Ports, adapters, robustesse CDP, sécurité
    ├── usage.md                  # Guide d'usage pratique, tous les outils MCP, troubleshooting
    └── wire-protocol.md          # Spécification du contrat HTTP SDK (5 endpoints, garde Host, schémas)
```

---

## Développement

```bash
# Pipeline CI complet (formatage + lint strict + tests)
just ci

# Équivalent manuel
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Les tests live KWin/EIS (session Wayland requise) sont marqués `#[ignore]` et exclus du `cargo test` standard.

Les tests d'intégration de la cible X11 démarrent **leur propre Xvfb privé** (display libre entre `:251` et `:259`, sans écoute TCP, arrêté à la fin) ; ils sont aussi `#[ignore]` :

```bash
cargo test -p adapter-x11 -- --ignored --test-threads=1
```

---

## Statut / contexte

Infrastructure hexagonale (12 ports, 41 outils MCP), adapter CDP avec reconnexion paresseuse, pont natif SDK Linux, capture KWin Wayland, injection KWin EIS, pilotage X11 isolé. La politique de stabilité de la 1.0 est décrite dans [docs/stability.md](docs/stability.md).

Les audits, la feuille de route et les notes de conception des premiers jalons sont conservés comme archives datées, non normatives, dans [docs/historique/](docs/historique/README.md).

---

## Docs

- [docs/usage.md](docs/usage.md) — outils MCP détaillés, configuration client MCP, modes Linux/kwin-eis/x11/WASM, troubleshooting
- [docs/architecture.md](docs/architecture.md) — architecture hexagonale, 12 ports, robustesse CDP, sécurité
- [docs/wire-protocol.md](docs/wire-protocol.md) — contrat HTTP du SDK (5 endpoints, formes JSON, garde Host, checklist implémentation tierce)
- [docs/historique/README.md](docs/historique/README.md) — archives de conception datées (audits, feuille de route, spike), non normatives
- [docs/stability.md](docs/stability.md) — politique semver de la 1.0 : surface stable, dépréciation, MSRV, versionnage du protocole

---

## Licence

DevEye est open source, sous deux licences selon le rôle du crate (décision : [ADR-0002](docs/adr/0002-licences-agpl-outil-mit-bibliotheques.md)) :

| Crates | Licence | Pourquoi |
|---|---|---|
| `deveye-server`, tous les `adapter-*`, les scripts | **AGPL-3.0-or-later** ([LICENSE](LICENSE)) | l'outil ; la réciprocité réseau de l'AGPL protège contre une reprise fermée |
| `deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client` | **MIT** ([LICENSE-MIT](LICENSE-MIT)) | les bibliothèques qu'une app embarque ou qu'un tiers réutilise |

Concrètement : intégrer `deveye-sdk` dans votre app ne lui impose aucune contrainte de licence. Lancer ou modifier `deveye-server` est soumis à l'AGPL (publier ses modifications si on le propose en service). Un crate MIT ne dépend jamais d'un crate AGPL.

Une contribution est reçue sous la licence du crate qu'elle touche. Les décisions du projet sont consignées dans [docs/adr/](docs/adr/README.md).
