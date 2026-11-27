# CLAUDE.md — Guide pour Claude

Ce fichier oriente les sessions de Claude Code sur ce dépôt. Lecture obligatoire avant toute action non triviale.

---

## Le projet en 30 secondes

**DevEye** — outillage **dev-only** qui permet à un agent (Claude Code ou tout autre client MCP) et à un humain de **voir** le rendu d'une app graphique en cours d'exécution et d'**interagir** avec elle (capture, clic, survol, saisie, glisser-déposer, navigation, évaluation JS, état applicatif, logs, commandes logiques), quel que soit le framework de l'app cible.

Architecture **hexagonale** (ports & adapters) sur un **workspace Cargo multi-crates**, 100 % Rust. Consommateur principal : le serveur **MCP** (`deveye-server --serve-mcp`, transport stdio). Familles de cibles (`--target`) :

- `cdp` : webview ou navigateur Chromium piloté par le Chrome DevTools Protocol ;
- `sdk` : app instrumentée par le SDK opt-in `deveye-sdk` (pont natif, y compris Dioxus desktop sur WebKitGTK) ;
- `auto` : CDP, avec repli appel par appel sur le pont SDK ;
- `kwin-eis` : app native sous Wayland KDE (yeux KWin ScreenShot2, mains KWin EIS) ;
- `x11` : app X11 native dans un serveur X **privé** (Xvfb + XTEST), jamais la session vivante.

DevEye n'est **pas un produit** : c'est un accélérateur de dev. Projet **open source** à partir de la 1.0, hébergé sur GitHub, sous deux licences selon le rôle du crate :

- **`AGPL-3.0-or-later`** pour l'outil : `deveye-server`, tous les adapters, `deveye-conformance`, l'outillage (licence par défaut du dépôt) ;
- **`MIT`** pour les bibliothèques qu'une app embarque ou qu'un tiers réutilise : `deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`. Intégrer le SDK dans une app ne lui impose rien.

---

## Règles non négociables

1. **Toujours répondre en français** (termes techniques et identifiants de code en anglais). Accents et diacritiques **complets** dans les docs, les commentaires et les chaînes affichées — jamais d'ASCII de remplacement. Seule exception : les **messages de commit**, en ASCII (voir « Workflow Git »). Un fichier déjà rédigé en ASCII (ex. aide clap de `deveye-server/src/cli.rs`) garde son style local.
2. **Aucun emoji** dans le code, les commentaires, les docs ou les messages de commit.
3. **Sens des dépendances hexagonal** :
   ```
   deveye-server → deveye-adapters/* → deveye-ports → deveye-core
   ```
   `deveye-core` ne dépend de rien de technique. Un adapter ne dépend jamais d'un autre adapter ; le code partagé entre adapters vit dans un crate dédié (`deveye-sdk-client`) et la composition d'adapters vit dans `deveye-server` ou `adapter-composite` (qui ne connaît que des ports).
4. **Découplage de l'app cible** : aucun crate `deveye-*` ne dépend d'une app cliente, et **aucun nom d'app cliente** n'apparaît dans le code, les valeurs par défaut, les tests ou les docs de référence (utiliser des exemples neutres : `mon_app`, `example`). Une app cible peut dépendre de `deveye-sdk` (en `cfg(debug_assertions)`) ; jamais l'inverse.
5. **Pas de logique technique dans `deveye-core`** : pas de `reqwest`, pas de `tokio` réseau, pas de CDP. Du Rust pur sur le domaine (`chrono`, `serde`, `serde_json`, `thiserror` ; `schemars` uniquement derrière la feature optionnelle `schema`, désactivée par défaut, ADR-0063).
6. **Pas de `unwrap()` / `expect()` / `panic!()`** dans le code de production sauf justification explicite en commentaire (init au démarrage, invariant impossible). Utiliser `Result<T, E>` et `?`.
7. **Commentaires** : code expressif d'abord. Un commentaire = un *pourquoi* non évident, pas un *quoi*.
8. **Surface stable** : depuis la 1.0, `deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`, le wire protocol HTTP du SDK, les noms et schémas des outils MCP et les flags de `deveye-server` suivent semver (voir `docs/stability.md`). Tout changement cassant de cette surface exige une version majeure. Les enums et structs publics évolutifs portent `#[non_exhaustive]`.
9. **Toute décision passe par un ADR** (`docs/adr/NNNN-slug.md`, index et gabarit dans `docs/adr/README.md`) : architecture, dépendance ajoutée ou retirée, licence, protocole, surface stable, plateforme, outillage, exception de sécurité ou de lint, choix entre plusieurs options. L'ADR est écrit avant ou avec le commit qui applique la décision ; une décision qui en remplace une autre crée un nouvel ADR et passe l'ancien en « Remplacé par ».
10. **Frontière de licences** : un crate sous `MIT` ne dépend jamais d'un crate sous `AGPL-3.0-or-later`. Un nouveau crate déclare explicitement sa licence (AGPL par défaut, MIT seulement s'il est destiné à être embarqué ou réutilisé, avec un ADR).

---

## Sécurité — dev-only, non négociable

1. **SDK strictement `cfg(debug_assertions)`** — `deveye_sdk::init()` est un no-op intégral en release ; rien ne s'embarque en build release de l'app cible.
2. **Bind loopback uniquement** — le mini-serveur SDK écoute sur `127.0.0.1`, jamais `0.0.0.0`.
3. **Port configurable** — par défaut `31337`, surchargeable par `DEVEYE_PORT`.
4. **Garde Host anti DNS-rebinding + CORS deny-all** sur toutes les routes du SDK.
5. **Pas de secret exposé** — l'état et les logs exposés par le SDK passent par le masquage des clés sensibles ; ne jamais ajouter de route qui le contourne. `eval_js` et les résultats de commandes ne sont pas filtrés (ADR-0016) ; `ui.widgets` ne publie pas la valeur des widgets secrets et `dom` la masque (ADR-0064).
6. **Cible `x11` isolée par construction** — `--display` obligatoire, refus de la session vivante, d'XWayland et des displays < 100 ; aucune option ne lève ces refus.

---

## Structure du dépôt

```
DevEye/
  Cargo.toml                    workspace + [workspace.dependencies]
  deveye-core/                  domaine pur (commandes d'interaction, captures, logs,
                                prédicats d'attente, erreurs console, fenêtres)
  deveye-ports/                 12 traits : Renderer, AppInspector, InteractionSink,
                                JsEvaluator, LogSource, CommandRouter, ConsoleErrorSource,
                                WaitCondition, WindowLister, WindowManager, Clipboard,
                                WindowGeometry
  deveye-sdk/                   micro-lib opt-in embarquée dans l'app cible (cfg debug)
  deveye-sdk-client/            client HTTP du SDK partagé par les adapters
  deveye-adapters/
    adapter-cdp-webview/        yeux, mains, DOM, JS via Chrome DevTools Protocol
    adapter-screencap-xcap/     capture OS-level de repli (X11/XWayland, plein écran)
    adapter-screencap-kwin/     capture de fenêtre Wayland via KWin ScreenShot2
    adapter-interaction-kwin-eis/ mains OS-level Wayland via KWin EIS (reis)
    adapter-x11/                serveur X privé : GetImage, XTEST, QueryTree
    adapter-clipboard-wayland/  presse-papiers (wl-copy / wl-paste)
    adapter-dioxus-signals/     état applicatif via le pont SDK
    adapter-logs-tracing/       logs via le buffer tracing du SDK
    adapter-commands-http/      commandes logiques via le SDK
    adapter-composite/          compositions génériques de ports (attente, console, DOM par route)
    adapter-mcp-server/         expose les ports au Model Context Protocol (outils MCP)
  deveye-server/                binaire orchestrateur (CLI, composition selon --target)
  deveye-conformance/           suite de conformité boîte noire du wire protocol (lib + binaire
                                `--url`) ; AGPL-3.0-or-later, `publish = false` (ADR-0035)
  scripts/                      preuve SDK no-op en release ; script de synchronisation de transition
                                (déprécié, ADR-0040)
  docs/                         architecture, usage (référence des outils), intégration du SDK,
                                guides par type d'app (guides/), wire protocol (+ schemas/),
                                stabilité, plateformes, migration 0.2 vers 1.0
  docs/adr/                     décisions d'architecture (ADR) et leur index
  docs/historique/              archives de conception datées (non normatives)
  README.md, CONTRIBUTING.md, SECURITY.md, CODE_OF_CONDUCT.md   vitrine, contribution, sécurité, conduite
```

**Sources de vérité** : `docs/architecture.md` (conception), `docs/usage.md` (utilisation et référence des 44 outils), `docs/wire-protocol.md` (contrat HTTP du SDK), `docs/platforms.md` (plateformes supportées), `SECURITY.md` (modèle de menace).

**Documentation vérifiée par des tests** (`adapter-mcp-server`, module `docs_tests`, ADR-0048) : la liste des outils du README (entre `<!-- outils:debut -->` et `<!-- outils:fin -->`) et les titres d'outils de `docs/usage.md` doivent égaler les outils enregistrés ; les nombres d'outils annoncés égalent `EXPECTED_TOOL_COUNT` ; les liens relatifs et les ancres des fichiers Markdown (hors `docs/historique/`) doivent exister ; chaque ADR figure à l'index avec son titre et son statut. Ajouter ou retirer un outil impose donc de mettre à jour `EXPECTED_TOOL_COUNT`, la classification des annotations, le README et `docs/usage.md`.

---

## Conventions de code (Rust)

- **Édition Rust 2021**, toolchain `stable` ; MSRV déclarée par `rust-version`.
- **Nommage** : `snake_case` (variables, fonctions, modules, fichiers), `PascalCase` (types, traits), `SCREAMING_SNAKE_CASE` (constantes). Crates : `deveye-<role>` ou `adapter-<technologie>`.
- **Traits de ports** : `#[async_trait]`, `Send + Sync`, retournant `PortResult<T>`.
- **Erreurs** : `thiserror` dans le domaine, les ports et les adapters ; `anyhow` toléré dans le binaire `deveye-server` uniquement.
- **DTOs** : `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]` sur les structures simples.
- **Versions** : chaque crate porte sa propre `version` (pas `version.workspace = true`).
- **Formatage** : `cargo fmt --all`.
- **Lint** : `cargo clippy --workspace --all-targets -- -D warnings`.
- **Lints crate** : `#![forbid(unsafe_code)]` + `#![deny(rust_2018_idioms)]` en tête des crates internes (exception documentée pour l'adapter KWin EIS si une dépendance l'impose).
- **Dépendances** : pas de dépendance Cargo nouvelle sans ADR ; préférer les dépendances déjà présentes dans `[workspace.dependencies]`.

---

## Workflow Git

**Stratégie : GitHub Flow** sur `github.com/Morgan-Voltz/DevEye`. `main` est toujours propre et publiable. Tout travail se fait sur une branche dédiée dérivée de `main`, fusionnée par pull request.

**Nommage des branches** : `<type>/<slug-kebab-case>` (`feat/outil-resize`, `fix/cdp-reconnect-timeout`, `docs/usage-mcp-json`, `release/1.0.0`).

**Commits : Conventional Commits, `type(scope): sujet`.**

- **Sujet** en français, impératif, minuscule, sans point final, **en ASCII sans accents** (les accents restent la règle partout ailleurs). Aucun emoji.
- **Types** : `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `chore`, `build`, `ci`.
- **Scope = crate concerné** (omis si transverse) : `core`, `ports`, `sdk`, `sdk-client`, `server`, `adapter-cdp`, `adapter-xcap`, `adapter-kwin`, `adapter-eis`, `adapter-x11`, `adapter-clipboard`, `adapter-dioxus`, `adapter-logs`, `adapter-commands`, `adapter-composite`, `adapter-mcp`, `conformance` ; transverse : `workspace`, `docs`, `ci`, `release`, `adr` (un ADR seul : `docs(adr): ...`).
- **Aucun trailer** dans les messages de commit (pas de `Co-Authored-By`, pas de signature d'outil).
- Adresse d'auteur : l'adresse noreply GitHub du mainteneur (la protection de confidentialité de GitHub refuse les pushs exposant une adresse privée).

```
feat(adapter-mcp): ajoute l'outil resize
fix(adapter-cdp): reconnecte la session apres un timeout WebSocket
docs(usage): clarifie l'ordre de lancement dans le .mcp.json
chore(workspace): aligne les versions des dependances partagees
```

---

## Commandes utiles

```bash
# Compiler / tester / linter
cargo check --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings

# Tests de l'adapter x11 (lancent Xvfb, display >= 100)
cargo test -p adapter-x11 -- --ignored --test-threads=1

# Build du binaire orchestrateur
cargo build --release -p deveye-server      # -> target/release/deveye-server

# Lancer l'orchestrateur en mode MCP (stdio)
cargo run -p deveye-server -- --serve-mcp

# Recettes équivalentes
just --list
```

Pour le workflow complet avec une app cible (`.mcp.json`, ordre des lancements, dépannage), voir `docs/usage.md`.

---

## Ce qu'il NE faut PAS faire spontanément

- **Ne pas ajouter** de dépendance Cargo sans ADR (chaque dépendance pèse à long terme).
- **Ne pas trancher** une décision (architecture, licence, protocole, surface stable, plateforme, outillage) sans l'écrire dans un ADR.
- **Ne pas créer** de fichiers `.md` de « résumé » ou de « plan » sans demande explicite.
- **Ne pas faire** de `git push`, de tag ou de publication (crates.io, release GitHub) sans demande explicite.
- **Ne pas introduire** de logique technique dans `deveye-core`.
- **Ne pas coupler** DevEye à une app cible précise (pas de nom de projet en dur, pas d'heuristique métier).
- **Ne pas casser** la surface stable sans version majeure (voir `docs/stability.md`).
- **Ne pas exposer** le SDK hors `cfg(debug_assertions)` ni binder ailleurs que `127.0.0.1`.

---

## Workflow recommandé pour ajouter une capacité

0. **Décider** : consigner les choix structurants dans un ADR (`docs/adr/`).
1. **Modéliser** le besoin dans le domaine (`deveye-core`) si une nouvelle entité est nécessaire.
2. **Définir / enrichir** le port (trait) concerné dans `deveye-ports`.
3. **Implémenter** l'adapter dans `deveye-adapters/<adapter>` (un crate par technologie).
4. **Exposer** la capacité au client MCP dans `adapter-mcp-server` si elle doit être pilotable par un agent (avec ses annotations d'outil).
5. **Câbler** dans `deveye-server` (composition des adapters selon `--target`).
6. **Tester** : unitaire sur le domaine, intégration sur l'adapter, bout en bout via le serveur MCP.
7. **Mettre à jour** `docs/usage.md`, `docs/architecture.md` et le `CHANGELOG.md`.
