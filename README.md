# DevEye

**Donnez des yeux et des mains à un agent IA sur votre app graphique en cours de développement.**

DevEye est un serveur [MCP](https://modelcontextprotocol.io) (Model Context Protocol) que Claude Code, ou tout autre client MCP, utilise pour **voir** une app vivante (captures d'écran, DOM, état, logs) et **agir** dessus (clics, saisie, touches, défilement, commandes logiques), puis **attendre** que le résultat soit là, sans intervention humaine. Il se branche sur une webview ou un navigateur Chromium (CDP), sur une app desktop sans CDP (pont natif par un SDK embarqué), sur une app native Wayland sous KDE Plasma, ou sur une app X11 native dans un serveur X privé.

> Outil de **développement** : il n'a pas vocation à tourner en production, et son SDK est inerte en build release ([Sécurité](#sécurité-et-posture-dev-only)).

## Statut

- **Version 1.0.** Les bibliothèques (`deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`), le protocole HTTP du SDK (version 1), les noms et schémas des outils MCP et les options du binaire suivent le versionnage sémantique ([docs/stability.md](docs/stability.md)).
- **Linux seul.** `deveye-server` ne compile que sous Linux ; Windows et macOS ne sont pas supportés, GNOME Wayland est hors périmètre ([docs/platforms.md](docs/platforms.md)).
- **Licences.** AGPL-3.0-or-later pour l'outil, MIT pour les bibliothèques qu'on embarque ([Licence](#licence)).
- Migration depuis la 0.2 : [docs/migration-1.0.md](docs/migration-1.0.md).

## Installation

**Prérequis** : Linux ; un client MCP. Pour compiler : toolchain Rust **stable** (MSRV 1.88) et les bibliothèques de développement suivantes (les mêmes que la CI) :

- Fedora : `sudo dnf install libxcb-devel libXrandr-devel dbus-devel libxkbcommon-devel mesa-libgbm-devel`
- Debian et Ubuntu : `sudo apt install pkg-config libxcb1-dev libxrandr-dev libdbus-1-dev libxkbcommon-dev libgbm-dev`

À l'exécution, le binaire a besoin de `libxcb`, `libxkbcommon`, `libgbm` et `libwayland-client`. Selon la cible : session KDE Plasma Wayland (`kwin-eis`), `Xvfb` (`x11`), navigateur ou webview Chromium (`cdp`).

**1. `cargo install` depuis GitHub** (compile le binaire, version épinglée) :

```bash
cargo install --git https://github.com/Morgan-Voltz/DevEye --tag v1.0.0 deveye-server --locked
```

**2. Archive de release** : télécharger `deveye-server-v1.0.0-x86_64-unknown-linux-gnu.tar.gz` et `SHA256SUMS` depuis les [versions GitHub](https://github.com/Morgan-Voltz/DevEye/releases), puis :

```bash
sha256sum -c SHA256SUMS --ignore-missing
tar -xzf deveye-server-v1.0.0-x86_64-unknown-linux-gnu.tar.gz
install -Dm755 deveye-server-v1.0.0-x86_64-unknown-linux-gnu/deveye-server ~/.local/bin/deveye-server
```

L'archive contient le binaire, `LICENSE` (AGPL-3.0), `LICENSE-MIT`, ce README et le CHANGELOG.

**3. Depuis les sources** :

```bash
git clone https://github.com/Morgan-Voltz/DevEye && cd DevEye
just install                       # build release, copié dans ~/.local/bin
# ou : cargo install --path deveye-server --locked
```

Vérifier : `deveye-server --version` (version, commit, versions des protocoles), puis `deveye-server doctor --target <cible>` pour diagnostiquer l'environnement en lecture seule.

## Démarrage rapide

Choisir la cible selon l'app, suivre son guide, puis créer un `.mcp.json` à la racine du projet de l'app. Exemple pour une webview Chromium :

```json
{
  "mcpServers": {
    "deveye": {
      "type": "stdio",
      "command": "deveye-server",
      "args": ["--target", "cdp", "--cdp-url", "http://localhost:9222", "--serve-mcp"]
    }
  }
}
```

Dans Claude Code, `/mcp` doit lister `deveye`. Premier appel conseillé : l'outil `target_info`, qui dit quels outils répondront sur la cible.

| Mon app est… | `--target` | Guide |
|---|---|---|
| une page ou une app dans une webview ou un navigateur **Chromium** | `cdp` (ou `auto`) | [webview-chromium-cdp.md](docs/guides/webview-chromium-cdp.md) |
| une app **Dioxus desktop** sous Linux (WebKitGTK, sans CDP) | `sdk` (ou `auto`) | [dioxus-desktop-linux.md](docs/guides/dioxus-desktop-linux.md) |
| une app **X11 native** (egui, GTK, Qt…), lancée dans un `Xvfb` privé | `x11` | [app-x11-xvfb-prive.md](docs/guides/app-x11-xvfb-prive.md) |
| une app **native Wayland** sous KDE Plasma | `kwin-eis` | [app-wayland-kde.md](docs/guides/app-wayland-kde.md) |
| écrite dans **un autre langage** (contrat HTTP à implémenter) | selon l'app | [implementation-tierce.md](docs/guides/implementation-tierce.md) |

Pour donner à l'agent l'état, les logs et les commandes de votre app Rust, intégrez le SDK : [docs/integration.md](docs/integration.md). Toutes les options, tous les outils : [docs/usage.md](docs/usage.md).

## Les outils MCP

<!-- outils:debut -->
### 44 outils MCP, en sept familles

| Famille | Outils |
|---|---|
| **Capture** | `screenshot`, `screenshot_region`, `screenshot_element`, `assert_visual` |
| **Interaction** | `click`, `double_click`, `right_click`, `hover`, `drag_drop`, `click_selector`, `click_text`, `type_text`, `press_key`, `key_combo`, `scroll`, `resize`, `navigate`, `handle_dialog`, `upload_file`, `focus_window`, `clipboard_get`, `clipboard_set` |
| **Inspection DOM et état** | `dom`, `find_element`, `assert_selector`, `assert_state`, `eval_js`, `state`, `collect_console_errors`, `list_windows`, `window_geometry`, `target_info` |
| **Logs** | `logs_tail`, `logs_wait` |
| **Attente** | `wait_for_selector`, `wait_for_text`, `wait_condition`, `wait_for_log`, `wait_screenshot_region`, `wait_target_alive`, `wait_ready` |
| **Commandes logiques** | `list_commands`, `invoke_command` |
| **Composite** | `observe` |
<!-- outils:fin -->

Un test échoue si cette liste diffère des outils réellement enregistrés par le serveur. Paramètres, retours et disponibilité par cible : [docs/usage.md](docs/usage.md#les-outils-mcp).

## Les cibles

| `--target` | Yeux | Mains | DOM et JS |
|---|---|---|---|
| `auto` (défaut) | CDP, repli capture OS | CDP, repli événements JS synthétiques | oui, par CDP ou par le pont |
| `cdp` | CDP | événements navigateur réels | oui |
| `sdk` | capture OS (KWin ou `xcap`) | événements JS synthétiques (pompe du SDK) | oui, par le pont |
| `kwin-eis` | capture OS | KWin EIS : vrais événements | non |
| `x11` | `GetImage` du serveur privé | XTEST sur le serveur privé | non (le « DOM » est la route `ui.widgets` de l'app) |

L'état, les logs et les commandes viennent toujours du SDK embarqué dans l'app. Environnements et mode de test (CI ou manuel) : [docs/platforms.md](docs/platforms.md).

## Sécurité et posture dev-only

- **SDK inerte en release** : `deveye_sdk::init` est un no-op quand `debug_assertions` est désactivé ; le dépôt le prouve sur un binaire release réel.
- **Boucle locale seulement** : le SDK écoute sur `127.0.0.1`, jamais `0.0.0.0`.
- **Garde `Host`** contre le rebond DNS : toute requête dont `Host` n'est pas `localhost`, `127.0.0.1` ou `[::1]` reçoit `403`.
- **CORS refusé**, corps de requête bornés à 1 Mio, file d'évaluation bornée.
- **Cible `x11` isolée par construction** : `--display` est obligatoire (aucun repli sur `$DISPLAY`) ; elle refuse un display non local, un numéro < 100, la session courante, le socket d'un autre utilisateur et tout serveur XWayland, sans option pour lever ces refus.
- **Masquage des secrets par clés** dans l'état et les logs : un filet, **pas une garantie** (il ne couvre ni `eval_js`, ni les retours de commandes).
- **Pas d'authentification** : tout processus local peut parler au SDK ; l'outil suppose un poste de développement de confiance.

Contrat HTTP complet : [docs/wire-protocol.md](docs/wire-protocol.md).

## Architecture en bref

```
Client MCP (Claude Code)
        │  stdio (JSON-RPC)
        ▼
  deveye-server  ── compose les adapters selon --target
        │
  adapter-mcp-server ── expose les 44 outils, consomme les 12 ports (deveye-ports)
        │
  ┌─────┴───────────────┬───────────────────┬──────────────────┐
  adapter-cdp-webview   adapter-screencap-*  adapter-interaction-kwin-eis   adapter-x11
  (Chromium, CDP)       (KWin, xcap)         (injection KWin EIS)           (GetImage, XTEST)
        │
  deveye-sdk (HTTP 127.0.0.1:31337, embarqué dans l'app, debug seulement)
```

Architecture hexagonale : un domaine pur, 12 ports (traits Rust), un adapter par technologie, un binaire qui compose le tout ([ADR-0041](docs/adr/0041-architecture-hexagonale-ports-et-adapters.md)). Détail : [docs/architecture.md](docs/architecture.md).

## Structure du dépôt

17 crates dans un workspace Cargo.

| Crate | Rôle | Licence |
|---|---|---|
| `deveye-core` | domaine pur : commandes, prédicats, snapshots, niveaux de log | MIT |
| `deveye-ports` | les 12 traits de ports et leurs erreurs | MIT |
| `deveye-sdk` | micro-serveur HTTP à embarquer dans l'app (debug seulement) | MIT |
| `deveye-sdk-client` | client HTTP du SDK, partagé par les adapters | MIT |
| `deveye-server` | binaire orchestrateur : compose les adapters, sert le MCP | AGPL-3.0-or-later |
| `deveye-conformance` | suite de conformité boîte noire du wire protocol | AGPL-3.0-or-later |
| `adapter-mcp-server` | expose les 44 outils MCP (rmcp, stdio) | AGPL-3.0-or-later |
| `adapter-cdp-webview` | yeux, mains, DOM et JS par CDP | AGPL-3.0-or-later |
| `adapter-screencap-kwin` | capture, fenêtres, géométrie et focus par KWin | AGPL-3.0-or-later |
| `adapter-screencap-xcap` | capture OS (X11, XWayland, Wayland non KDE) | AGPL-3.0-or-later |
| `adapter-interaction-kwin-eis` | injection d'entrées Wayland par KWin EIS | AGPL-3.0-or-later |
| `adapter-x11` | yeux, mains et fenêtres sur un serveur X privé | AGPL-3.0-or-later |
| `adapter-clipboard-wayland` | presse-papiers par `wl-copy` et `wl-paste` | AGPL-3.0-or-later |
| `adapter-dioxus-signals` | inspecteur et pont natif via le SDK | AGPL-3.0-or-later |
| `adapter-logs-tracing` | logs `tracing` via le SDK | AGPL-3.0-or-later |
| `adapter-commands-http` | commandes logiques via le SDK | AGPL-3.0-or-later |
| `adapter-composite` | attentes, erreurs console, inspecteurs composites | AGPL-3.0-or-later |

Documentation : [`docs/`](docs/) (usage, intégration, guides, plateformes, architecture, protocole, stabilité, migration), décisions dans [`docs/adr/`](docs/adr/README.md), archives datées non normatives dans [`docs/historique/`](docs/historique/README.md).

## Développement

```bash
just ci          # formatage, lint strict, tests, doc, Xvfb, e2e MCP, cargo deny, conformité, SDK no-op
# cœur, à la main :
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Les tests de la cible x11 et de la session MCP de bout en bout démarrent **leur propre `Xvfb` privé** ; ils sont `#[ignore]` mais bloquants en CI (`just x11`, `just e2e`). Les tests `live_*` des adapters KWin exigent une session KDE réelle et se lancent à la main. La CI est dans `.github/workflows/ci.yml`.

## Contribuer, sécurité, conduite

- **Contribuer** : [CONTRIBUTING.md](CONTRIBUTING.md) (prérequis, commandes, conventions de commit, revue, stabilité). **Toute décision passe par un ADR** : [docs/adr/](docs/adr/README.md).
- **Signaler une faille** : [SECURITY.md](SECURITY.md) (modèle de menace, garanties, limites, signalement privé de vulnérabilité de GitHub).
- **Code de conduite** : [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), adaptation française du Contributor Covenant 2.1 ; en participant, vous l'acceptez.

## Licence

DevEye est open source, sous deux licences selon le rôle du crate ([ADR-0002](docs/adr/0002-licences-agpl-outil-mit-bibliotheques.md)) :

| Crates | Licence | Pourquoi |
|---|---|---|
| `deveye-server`, `deveye-conformance`, tous les `adapter-*`, les scripts | **AGPL-3.0-or-later** ([LICENSE](LICENSE)) | l'outil ; la réciprocité réseau de l'AGPL protège contre une reprise fermée |
| `deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client` | **MIT** ([LICENSE-MIT](LICENSE-MIT)) | les bibliothèques qu'une app embarque ou qu'un tiers réutilise |

Concrètement : intégrer `deveye-sdk` dans votre app ne lui impose **aucune** contrainte de licence. Lancer ou modifier `deveye-server` est soumis à l'AGPL (publier ses modifications si on le propose comme service). Un crate MIT ne dépend jamais d'un crate AGPL. Une contribution est reçue sous la licence du crate qu'elle touche.
