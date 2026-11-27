# DevEye — architecture

> Outillage interne **dev-only** permettant à un agent (Claude Code ou autre) et à un humain de **voir le rendu** d'une app graphique en cours d'exécution et d'**interagir** avec elle, peu importe le framework de l'app cible. Conçu en architecture hexagonale pour se brancher à n'importe quel projet.

---

## Le problème qu'on résout

Sans outil dédié, un agent travaillant sur une UI le fait **à l'aveugle** : il écrit du code, lance `cargo check`, et espère que le rendu est correct. Le seul moyen de vérifier visuellement est qu'un humain lance l'app, prenne un screenshot et colle l'image — ce qui rompt le flux de travail et empêche l'itération autonome.

DevEye répond à trois besoins :

1. **Voir** — capture du rendu visuel d'une app vivante, exploitable par un humain ou un LLM multimodal.
2. **Interagir** — émettre des événements (clic, hover, saisie, navigation) pour valider un parcours sans intervention humaine.
3. **Inspecter** — accéder à l'état interne (DOM, signaux, logs) pour comprendre *pourquoi* un rendu est ce qu'il est.

Et tout ça sans se contenter d'une seule cible : on veut brancher une app Dioxus desktop, puis demain une app Tauri, un webview Chromium quelconque, etc.

---

## Le contexte stratégique

DevEye n'est **pas un produit**. C'est un outillage qui accélère le dev. Décisions cadrantes :

- **Dev-only, jamais en prod** — tout le code DevEye est strictement gardé par `#[cfg(debug_assertions)]` côté SDK et n'apparaît jamais dans un binaire release de l'app cible.
- **Hexagonal dès le départ** — le métier (« observer et piloter une app ») est isolé du *comment* (CDP, screencap, MCP). Ajouter une cible de plus = nouvel adapter, pas de refonte.
- **Linux seul en 1.0** — `deveye-server` ne compile que sous Linux (ADR-0037) ; la matrice complète des environnements supportés, expérimentaux ou non supportés est dans [`platforms.md`](./platforms.md).
- **Première cible historique = app Dioxus desktop (wry)** — pilotée aujourd'hui par CDP (webview Chromium) ou par le pont natif du SDK (WebKitGTK), puis élargie aux apps Wayland et X11 natives.
- **Pas de cible web exigée** — on pilote l'app desktop directement, pas une variante web servie séparément.
- **Chaque décision structurante est un ADR** — voir [`adr/`](./adr/README.md) ; les six choix fondateurs (hexagonal, dev-only, pont natif, x11 isolé, KWin, repli `auto`) y sont consignés rétroactivement (ADR-0041 à 0046).

---

## Architecture hexagonale

Cinq cercles, avec la règle d'or : **le sens des dépendances pointe toujours vers l'intérieur**.

```
┌──────────────────────────────────────────────────────────────────┐
│   ADAPTERS DevEye (cercle extérieur)                              │
│   - CDP WebView (Chromium)                                        │
│   - Screencap xcap (X11 / XWayland / Wayland non-KDE)             │
│   - Screencap KWin (KDE Wayland, D-Bus ScreenShot2 + Scripting)   │
│   - Logs tracing-subscriber                                       │
│   - Dioxus signals (via SDK opt-in)                               │
│   - Interaction KWin EIS (injection OS-level Wayland natif)       │
│   - X11 (serveur X privé : GetImage + XTEST + QueryTree, x11rb)   │
│   - Clipboard Wayland (wl-copy / wl-paste OS-level)               │
│   - Composite (WaitCondition polling, ConsoleErrorSource JS,      │
│     Inspector composite — orchestration sans I/O propre)          │
│   - MCP server (consomme tout, expose à un client MCP)            │
│              ┌──────────────────────────────────────┐             │
│              │  PORTS DevEye (traits Rust)          │             │
│              │  Renderer, AppInspector,             │             │
│              │  InteractionSink, JsEvaluator,       │             │
│              │  LogSource, CommandRouter,           │             │
│              │  ConsoleErrorSource, WaitCondition,  │             │
│              │  WindowLister, WindowManager,        │             │
│              │  Clipboard, WindowGeometry           │             │
│              │  ┌────────────────────────────────┐  │             │
│              │  │   DOMAIN DevEye — métier pur   │  │             │
│              │  │   Snapshot, InteractionCommand,│  │             │
│              │  │   LogEvent, CommandDescriptor  │  │             │
│              │  │   Position, Size, Screenshot…  │  │             │
│              │  └────────────────────────────────┘  │             │
│              └──────────────────────────────────────┘             │
└──────────────────────────────────────────────────────────────────┘
```

### Pourquoi hexagonal pour un outil de dev

1. **Multi-cibles sans refonte** — chaque cible nouvelle (Dioxus desktop, Tauri, webview générique…) ajoute un *adapter* sans toucher au domaine.
2. **Multi-consommateurs** — un agent consomme via MCP, un humain via une UI HTML, un script CI via CLI. Mêmes ports, mêmes adapters d'observation, différents adapters de présentation.
3. **Testabilité** — on peut simuler une app cible (fake adapters in-memory) et vérifier que le domaine d'enregistrement / replay / diff fait bien son travail.

---

## Structure du workspace

```
DevEye/
├── deveye-core/                ← DOMAINE PUR (aucune dépendance technique), MIT
│   └── src/
│       ├── snapshot.rs         # Screenshot, DomTree, StateDump, Position, Size
│       ├── command.rs          # InteractionCommand (Click, Hover, Type, KeyPress, KeyCombo,
│       │                       # Navigate, DragDrop, Scroll, DoubleClick, HandleDialog, UploadFile,
│       │                       # Resize), MouseButton, CommandDescriptor
│       ├── wait_condition.rs   # WaitPredicate, WaitOutcome
│       ├── console_error.rs    # ConsoleError, ConsoleLevel
│       ├── window_info.rs      # WindowInfo
│       ├── window_rect.rs      # WindowRect
│       └── log_event.rs        # LogEvent, LogLevel, fields
│
├── deveye-ports/               ← INTERFACES (traits uniquement), MIT
│   └── src/
│       ├── renderer.rs              # trait Renderer
│       ├── inspector.rs             # trait AppInspector
│       ├── interaction.rs           # trait InteractionSink
│       ├── js_evaluator.rs          # trait JsEvaluator
│       ├── log_source.rs            # trait LogSource
│       ├── command_router.rs        # trait CommandRouter
│       ├── console_error_source.rs  # trait ConsoleErrorSource
│       ├── wait_condition.rs        # trait WaitCondition
│       ├── window_lister.rs         # trait WindowLister
│       ├── window_manager.rs        # trait WindowManager
│       ├── clipboard.rs             # trait Clipboard
│       └── window_geometry.rs       # trait WindowGeometry
│
├── deveye-adapters/                       ← IMPLÉMENTATIONS (un crate par adapter)
│   ├── adapter-cdp-webview/               # Renderer + InteractionSink + AppInspector + JsEvaluator + WindowLister via CDP
│   ├── adapter-screencap-xcap/            # Renderer OS-level (xcap — X11 / XWayland / Wayland non-KDE)
│   ├── adapter-screencap-kwin/            # Renderer + WindowManager + WindowGeometry + WindowLister KDE Wayland (KWin D-Bus)
│   ├── adapter-dioxus-signals/            # AppInspector + pont natif (JsEvaluator/InteractionSink) via SDK
│   ├── adapter-logs-tracing/              # LogSource via le buffer tracing du SDK opt-in
│   ├── adapter-commands-http/             # CommandRouter via POST /devtools/command + GET /devtools/commands
│   ├── adapter-interaction-kwin-eis/      # InteractionSink OS-level Wayland natif (KWin EIS)
│   ├── adapter-clipboard-wayland/         # Clipboard OS-level via wl-copy / wl-paste (Wayland)
│   ├── adapter-x11/                       # Renderer + InteractionSink + WindowLister + WindowGeometry + WindowManager sur un serveur X privé (x11rb pur Rust, XTEST)
│   ├── adapter-composite/                 # WaitCondition + ConsoleErrorSource + AppInspector composites (sans I/O propre) ; RouteDomInspector (DOM par route de commande)
│   └── adapter-mcp-server/               # consomme les 12 ports, expose au Model Context Protocol
│
├── deveye-sdk/                 ← micro-lib opt-in à intégrer dans l'app cible (cfg debug), MIT
├── deveye-sdk-client/          ← client HTTP partagé (timeouts, URL par défaut, version du protocole) des adapters qui parlent au SDK, MIT
├── deveye-conformance/         ← suite de conformité boîte noire du wire protocol (bibliothèque + binaire `deveye-conformance`)
└── deveye-server/              ← BINAIRE orchestrateur (compose les adapters selon les options CLI)
```

Les quatre crates MIT sont ceux qu'une app embarque ou qu'un tiers réutilise ; l'outil (`deveye-server`, les adapters, la suite de conformité) est sous AGPL-3.0-or-later. Un crate MIT ne dépend jamais d'un crate AGPL ([ADR-0002](adr/0002-licences-agpl-outil-mit-bibliotheques.md)).

### Convention de nommage

- **Crates** : `deveye-<role>` pour les composants internes, `adapter-<technologie>` pour les adapters.
- **Versions** : chaque crate `deveye-*` a sa propre version dans son `Cargo.toml` (pas `version.workspace = true`), pour suivre son propre cycle de release.
- **Édition** : Rust 2021, toolchain stable.

---

## Les 12 ports en détail

### 1. `Renderer`

```rust
#[async_trait]
pub trait Renderer: Send + Sync {
    async fn screenshot(&self) -> PortResult<Screenshot>;
    async fn viewport_size(&self) -> PortResult<Size>;
}
```

`PortResult<T>` = `Result<T, PortError>` (défini dans `deveye-ports::errors`). `Screenshot` est une struct de `deveye-core` (`bytes` : l'image encodée, `captured_at`, `viewport`).

- `adapter-cdp-webview` (`Page.captureScreenshot` CDP) — webview Chromium (app desktop wry, navigateur)
- `adapter-screencap-xcap` (capture fenêtre OS) — X11 / XWayland / Wayland non-KDE
- `adapter-screencap-kwin` (KWin D-Bus `ScreenShot2`) — KDE Wayland natif

### 2. `AppInspector`

```rust
#[async_trait]
pub trait AppInspector: Send + Sync {
    async fn dom_snapshot(&self) -> PortResult<DomTree>;
    async fn state_snapshot(&self) -> PortResult<StateDump>;
}
```

- `adapter-cdp-webview` (DOM via CDP `DOM.getDocument`)
- `adapter-dioxus-signals` (state via SDK opt-in qui sérialise les signaux ; DOM fallback via `outerHTML`)

### 3. `InteractionSink`

```rust
#[async_trait]
pub trait InteractionSink: Send + Sync {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()>;
}
```

Variantes de `InteractionCommand` (`deveye-core/src/command.rs`) :

- `Click { position, button }` — séquence `mouseMoved` + sleep 60 ms + `mousePressed` + `mouseReleased`. Le `mouseMoved` préalable est indispensable pour déclencher les `onmouseenter` des UI dont des éléments n'apparaissent qu'au hover. Le sleep laisse le temps au framework de re-render avant le press. `button` (`MouseButton::Left|Right|Middle`, défaut `Left`).
- `Hover { position }` — `mouseMoved` seul, sans press. Utile pour ouvrir une zone dépendante du hover puis cliquer ailleurs.
- `Type { text }` — `Input.insertText` (CDP) ou événement `input` synthétique (SDK).
- `KeyPress { key }` — `Input.dispatchKeyEvent` keyDown + keyUp (CDP) ou `KeyboardEvent` synthétique (SDK).
- `KeyCombo { modifiers, key }` — combinaison de touches (Ctrl+S, Ctrl+Shift+P, Alt+F4…) : les `modifiers` (`ctrl`/`control`, `shift`, `alt`, `meta`/`super`/`cmd`, insensibles à la casse) sont maintenus pendant la frappe de `key`. Indispensable en complément de `KeyPress`, qui ne porte aucun modificateur.
- `Navigate { route }` — `Runtime.evaluate` sur le hook routeur exposé côté JS par le SDK opt-in.
- `DragDrop { from, to }` — séquence pointer/souris native CDP ou `PointerEvent` synthétique (SDK).
- `Scroll { position, dx, dy }` — molette/défilement ancré à `position` (`dy > 0` défile vers le bas). En CDP : `Input.dispatchMouseEvent` type `mouseWheel`, qui vise le bon conteneur scrollable. Via le pont SDK : conteneur scrollable sous le point si trouvé, sinon `window`. En kwin-eis : défilement envoyé à la fenêtre focus.
- `DoubleClick { position }` — double-clic gauche en un seul dispatch worker (édition inline, sélection de mot, ouverture d'item de liste…) ; plus fiable que deux `Click` MCP successifs séparés d'un intervalle imprévisible.
- `HandleDialog { accept, prompt_text }` — répond à une boîte de dialogue JS (`alert`/`confirm`/`prompt`). CDP-only : le moteur JS est suspendu pendant un dialog, `eval_js` y est impossible.
- `UploadFile { selector, paths }` — renseigne les fichiers d'un `input[type=file]`. CDP-only (`DOM.setFileInputFiles`) : le verrou de sécurité navigateur interdit de setter `.value` depuis JS.

- `Resize { width, height }` — émule un viewport de `width` x `height` pixels CSS (`Emulation.setDeviceMetricsOverride`, `deviceScaleFactor: 0` pour garder l'échelle réelle de l'écran). Émulation du moteur de rendu : la fenêtre de l'OS n'est pas redimensionnée ; `0` x `0` retire l'émulation. CDP-only : toutes les autres cibles répondent `Unsupported` (ADR-0019).

Adapters : `adapter-x11` (XTEST `FakeInput` sur un serveur X privé — gestes interpolés, clavier du serveur, remappage temporaire des caractères absents ; `Navigate`/`HandleDialog`/`UploadFile`/`Resize` → `Unsupported`), `adapter-cdp-webview` (CDP `Input.*` / `Runtime.evaluate`, toutes variantes), `adapter-dioxus-signals` (`JsInteraction` — événements JS synthétiques via la pompe SDK), `adapter-interaction-kwin-eis` (events EI OS-level via KWin EIS — Wayland natif KDE, sans WebView ni SDK ; `Navigate`/`HandleDialog`/`UploadFile`/`Resize` y renvoient `PortError::Unsupported`, sémantique applicative ou WebView impossible OS-level).

### 4. `JsEvaluator`

```rust
#[async_trait]
pub trait JsEvaluator: Send + Sync {
    async fn evaluate(&self, expression: &str) -> PortResult<serde_json::Value>;
}
```

Exécute une expression JavaScript dans le contexte de la page cible. L'adapter CDP traduit en `Runtime.evaluate` avec `returnByValue: true` + `awaitPromise: true` + `userGesture: true`. Volontairement permissif côté sécurité — DevEye est un outil de **dev local**.

Cas d'usage : lire un `getBoundingClientRect()` pour cliquer précisément ; déclencher un `.click()` par sélecteur (plus robuste que des coordonnées) ; sonder un état frontend.

Adapters : `adapter-cdp-webview` (CDP), `adapter-dioxus-signals` (`SdkEvaluator` → `POST /devtools/eval`).

### 5. `LogSource`

```rust
#[async_trait]
pub trait LogSource: Send + Sync {
    async fn poll(&self) -> PortResult<Vec<LogEvent>>;
}
```

Contrat volontairement minimal : `poll` draine les événements accumulés depuis
le dernier appel (pas de `Stream`, pas de dépendance `futures` dans le domaine).

- `adapter-logs-tracing` (via SDK opt-in qui pose un subscriber `tracing`)

### 6. `CommandRouter`

```rust
#[async_trait]
pub trait CommandRouter: Send + Sync {
    async fn invoke(&self, route: &str, payload: serde_json::Value) -> PortResult<serde_json::Value>;
    async fn list(&self) -> PortResult<Vec<CommandDescriptor>>;
}
```

Permet à un agent de **dire à l'app cible** d'exécuter une action logique (« change de page », « charge telle fixture »). Implémenté par `adapter-commands-http` : l'app enregistre ses routes via `deveye_sdk::expose_route(name, description, closure)` (servies en local par `POST /devtools/command` et `GET /devtools/commands`), exposées au client MCP par les outils `invoke_command` et `list_commands`.

### 7. `ConsoleErrorSource`

```rust
#[async_trait]
pub trait ConsoleErrorSource: Send + Sync {
    async fn drain_console_errors(&self) -> PortResult<Vec<ConsoleError>>;
}
```

Drain des erreurs console émises par le webview (exceptions non capturées, promesses rejetées, `console.error`). Par analogie avec `LogSource::poll` : chaque appel vide le buffer accumulé. En pratique, `deveye-server` compose ce port via un collecteur JS injecté dans l'évaluateur (fonctionne en CDP comme en SDK). Exposé via l'outil MCP `collect_console_errors`.

### 8. `WaitCondition`

```rust
#[async_trait]
pub trait WaitCondition: Send + Sync {
    async fn wait_until(
        &self,
        predicate: WaitPredicate,
        timeout: Duration,
    ) -> PortResult<WaitOutcome>;
}
```

Attente conditionnelle sur l'état de l'app cible. `WaitPredicate` est une enum (domaine, `deveye-core/src/wait_condition.rs`) avec sept variantes : `DomSelector { selector, visible?, negate? }` (présence CSS, optionnellement visible et/ou inversée — base de `wait_for_gone`), `StateKey { key, expected }` (valeur du state SDK), `ScreenshotStable { threshold_ms }` (deux captures identiques = stabilité visuelle), `ScreenshotRegionStable { x, y, width, height, threshold_ms }` (idem, borné à une région — ignore un curseur clignotant ailleurs dans le viewport), `LogMessage { contains, level_min? }` (attend un message dans le buffer de logs), `TargetAlive {}` (sonde d'abord le canal d'interaction réel par `evaluator.evaluate("1")` ; seule une cible sans JavaScript, `Unsupported`, retombe sur `renderer.screenshot()` : attend qu'une cible crashée redevienne joignable) et `TextContent { text, selector?, visible? }` (texte dans l'`innerText`/`textContent`, indisponible en kwin-eis faute de DOM). Retourne un `WaitOutcome { satisfied, elapsed_ms, detail }` — un timeout n'est pas une erreur (`Err`), c'est un `satisfied: false`. Implémenté par `CompositeWaitCondition` dans `adapter-composite` (boucle de polling sur les ports déjà composés). Exposé via l'outil MCP `wait_condition`, et via les outils dédiés `wait_for_selector`/`wait_for_text`/`wait_for_log`/`wait_screenshot_region`/`wait_target_alive`.

### 9. `WindowLister`

```rust
#[async_trait]
pub trait WindowLister: Send + Sync {
    async fn list_windows(&self) -> PortResult<Vec<WindowInfo>>;
}
```

Énumère les fenêtres/onglets de la cible. Implémenté par `adapter-cdp-webview` (`Target.getTargets`, filtré sur `type=page`) et par `adapter-screencap-kwin` (`KwinWindowLister`, `workspace.windowList()` via KWin Scripting — actif dès que le screencap passe par KWin, y compris en cible `kwin-eis` : donne enfin de la visibilité sur les fenêtres qui peuvent dévier silencieusement les clics EIS). Le pont natif SDK n'a pas de notion d'autres fenêtres et renvoie une liste vide — pas une erreur. Pour la cible `x11`, `adapter-x11` l'implémente par `QueryTree` récursif depuis la racine (fenêtres `Viewable` titrées ou classées ; titres `_NET_WM_NAME` → `WM_NAME` → `WM_CLASS`, sans gestionnaire de fenêtres). Exposé via l'outil MCP `list_windows`.

### 10. `WindowManager`

```rust
#[async_trait]
pub trait WindowManager: Send + Sync {
    async fn activate(&self, title_substring: Option<&str>) -> PortResult<String>;
}
```

Donne le focus OS à une fenêtre de l'app cible (le `caption` contenant `title_substring`, insensible à la casse ; `None` cible la fenêtre par défaut de l'adapter, typiquement `--screencap-window`). Motivation propre à `kwin-eis` : KWin EIS route les événements d'entrée vers la fenêtre **active** — une notification Plasma ou un dialog système peut voler le focus et dévier silencieusement tous les clics/saisies vers la mauvaise fenêtre. Implémenté par `adapter-screencap-kwin` (`KwinWindowManager`, via un script KWin) et, en cible `x11`, par `adapter-x11` (`SetInputFocus` + passage au premier plan : sans gestionnaire de fenêtres c'est le seul moyen de fixer le focus clavier). Indisponible hors KDE/Wayland ou en cible CDP → `PortError::Unsupported`. Exposé via l'outil MCP `focus_window`.

### 11. `Clipboard`

```rust
#[async_trait]
pub trait Clipboard: Send + Sync {
    async fn get(&self) -> PortResult<String>;
    async fn set(&self, text: &str) -> PortResult<()>;
}
```

Accès au presse-papiers de la session. Motivation : `navigator.clipboard.readText()` lève `NotAllowedError` en contexte automatisé (pas de geste utilisateur), et en `kwin-eis` il n'y a pas de WebView du tout — un accès OS-level est la seule voie fiable sur toutes les cibles. Implémenté par `adapter-clipboard-wayland` (`wl-copy`/`wl-paste`, Wayland uniquement). Hors Wayland (ou outils `wl-clipboard` absents) → `PortError::Unsupported`. Exposé via les outils MCP `clipboard_get`/`clipboard_set`.

### 12. `WindowGeometry`

```rust
#[async_trait]
pub trait WindowGeometry: Send + Sync {
    async fn geometry(&self, title_substring: Option<&str>) -> PortResult<WindowRect>;
}
```

Lit la géométrie écran (position + taille) d'une fenêtre de l'app cible. Motivation propre à `kwin-eis` : le rendu natif (Uno/Skia) n'a pas de viewport CDP — un agent qui doit cliquer à des coordonnées écran (plutôt que logiques webview) a besoin de l'origine réelle de la fenêtre en plus de sa taille. Complète `WindowManager` (focus) et `Renderer` (capture pixels). Implémenté par `adapter-screencap-kwin` (`KwinWindowGeometry`, via un script KWin) et, en cible `x11`, par `adapter-x11` (`TranslateCoordinates` relu à chaque appel — jamais d'origine figée). Indisponible hors KDE/Wayland ou en cible CDP/SDK → `PortError::Unsupported`. Exposé via l'outil MCP `window_geometry`.

---

## Disponibilité du CDP selon la webview

Le protocole CDP n'existe que sur les webviews Chromium. Une app Dioxus desktop l'active par un argument de lancement de sa webview (côté app, par exemple `--remote-debugging-port=9222`) ; sur un navigateur Chromium, par `--remote-debugging-port`.

| Webview | CDP | Pilotage par DevEye |
|---|---|---|
| Chromium, WebView2, Chrome, Brave | oui (point d'accès `http://localhost:9222/json`) | `--target cdp` (ou `auto`) |
| WebKitGTK (apps desktop Linux) | non standard | pont natif du SDK (`--target sdk`, ou repli de `auto`, [ADR-0043](adr/0043-pont-natif-via-le-sdk-sans-cdp.md)) |
| Rendu natif sans webview (Skia, egui, GTK, Qt) | sans objet | `kwin-eis` (Wayland KDE) ou `x11` (serveur X privé) |

`deveye-server` ne compile que sous Linux ([`platforms.md`](./platforms.md)) : la mention de WebView2 décrit l'app cible, pas une plateforme du serveur.

---

## Robustesse CDP : session browser-level + auto-reattach

L'attachement direct à un *target page* s'est révélé fragile : à chaque recompilation à chaud, la webview est détruite/recréée, le WebSocket vers l'ancien target devient mort, et la commande CDP suivante renvoie une erreur de connexion abandonnée.

`adapter-cdp-webview/src/client.rs` corrige ça avec une **session browser-level** :

1. Bootstrap : `GET /json/version` → récupère le `webSocketDebuggerUrl` racine (browser-wide).
2. Connexion WebSocket unique vers ce endpoint, persistante sur toute la durée du process `deveye-server`.
3. `Target.setDiscoverTargets { discover: true, filter: [{ type: "page" }] }` au boot → CDP émet un event pour chaque page existante et chaque nouvelle page.
4. Filtre target **configurable** (`TargetFilter`) : `type=page` ET (URL contient `url_contains`, OU titre contient `title_contains`, OU `about:blank` si `accept_blank`), insensible à la casse. Sans critère, le filtre est **ouvert** : la première page qui n'est pas `about:blank` devient la cible (ADR-0014) ; `about:blank` n'est accepté qu'avec `--cdp-accept-blank`. Surchargeable depuis la CLI via `--cdp-target-filter-url` / `--cdp-target-filter-title` (ex. viser une page précise sur `localhost:5000`) — aucun nom d'app ni de framework codé en dur. Quand aucune page ne correspond, l'erreur liste les pages vues (titre et URL tronqués) pour corriger le filtre.
5. Pour chaque target qui matche : `Target.attachToTarget { targetId, flatten: true }` → un `sessionId` actif.
6. Les commandes (`Page.captureScreenshot`, `Input.dispatchMouseEvent`, `DOM.getDocument`, `Runtime.evaluate`…) sont envoyées avec `sessionId` au top-level (flatten mode).
7. Sur `Target.detachedFromTarget` / `Target.targetDestroyed`, le `sessionId` est effacé ; le prochain `Target.targetCreated` qui matche ré-attache automatiquement (`call()` attend une session active, timeout 10 s).

Conséquence : un hot-reload ne nécessite plus de reconnexion manuelle du MCP. Le binaire `deveye-server` reste vivant et bascule tout seul sur la nouvelle page dès qu'elle apparaît.

**Reconnexion paresseuse (`LazyCdp`, `--target`)** : `deveye-server` n'exige plus le navigateur au démarrage. La façade `LazyCdp` (re)connecte **à la demande** : chaque appel d'outil dont le WebSocket est mort retente l'attache via `/json/version`. Un navigateur absent au boot ou fermé en cours de session n'est donc pas fatal — l'outil renvoie une erreur actionnable tant que le CDP manque, puis repart seul. Le drapeau `--target {auto|cdp|sdk|kwin-eis|x11}` choisit la source des yeux/mains :

- `auto` — tente CDP puis bascule **appel par appel** sur le pont natif du SDK (Linux/WebKitGTK).
- `cdp` / `sdk` — forcent l'un ou l'autre.
- `kwin-eis` — injection d'entrée OS-level via `org.kde.KWin.EIS.RemoteDesktop` (Wayland natif KDE, sans XWayland ni shim dans l'app). C'est la voie « mains » pour les cibles sans WebView ni SDK, par exemple une app Uno desktop (rendu Skia). Les yeux viennent du screencap (`adapter-screencap-kwin` en KDE/Wayland, xcap sinon) ; DOM et JS sont indisponibles (`PortError::Unsupported`).
- `x11` — serveur X **privé et explicite** (`--display`, sans repli sur `$DISPLAY`) piloté par `adapter-x11` : yeux `GetImage` de la racine rognée à la fenêtre, mains XTEST, fenêtres `QueryTree`, focus `SetInputFocus`. Pas de JS ; le « DOM » est la réponse de la route de commande `ui.widgets` de l'app (`RouteDomInspector` dans `adapter-composite`, au-dessus du `CommandRouter` du SDK, sans changement de `deveye-core`/`deveye-ports`). Garde d'isolation non contournable (display < 100, `$DISPLAY`, uid du socket, extension `XWAYLAND`). Seule une coupure de transport donne `PortError::Network` — c'est la seule erreur sur laquelle les façades de repli basculent —, et la connexion est paresseuse (reconnexion après redémarrage du serveur X). Le composé `x11` n'appelle aucun `build_kwin_*` et ne touche jamais le presse-papiers de la session.

**Validation des options par cible** ([ADR-0031](adr/0031-validation-des-flags-selon-la-cible.md)) : `--cdp-url`, `--cdp-target-filter-*` et `--cdp-accept-blank` ne valent que pour `auto` et `cdp` ; `--display` et `--x11-class` que pour `x11` ; `--screencap-backend` est refusé avec `x11`. Une option sans effet est une erreur au démarrage, pas un silence. Sans `--serve-mcp` ni sous-commande, le binaire s'arrête en erreur d'usage.

**Repli observable** ([ADR-0034](adr/0034-capacites-completes-de-target-info-et-repli-observable.md)) : en mode `auto`, les quatre façades de repli partagent un `FallbackState`. Chaque bascule vers le pont natif (puis chaque retour au CDP) est journalisée une fois, et l'outil `target_info` relit à chaque appel le backend effectif (`cdp`, `sdk`, `indetermine`). Les capacités qu'il annonce (fenêtres, géométrie, focus, presse-papiers) sont calculées au démarrage d'après la session réelle, par un seul choix de backend OS (`use_kwin` dans `compose.rs`).

**Diagnostic et version** : `deveye-server doctor` ([ADR-0032](adr/0032-sous-commande-doctor-en-lecture-seule.md)) est la seule sous-commande ; il réutilise la garde d'`adapter-x11` et le client de `deveye-sdk-client` sans composer d'adapter. `--version` expose version, commit de build et versions des protocoles ([ADR-0033](adr/0033-version-affichee-et-provenance-du-commit-sans-dependance.md)).

Détails opérationnels : [`usage.md`](./usage.md).

**Limites de la cible `x11`** : l'image est un `GetImage` de la **racine** du serveur X rognée au rectangle de la fenêtre (le drawable de la fenêtre peut rendre du noir pour un enfant GL). Trois conséquences à connaître : (1) **occultation** — toute fenêtre qui en recouvre une autre apparaît dans la capture à la place de ce qu'elle cache, et la partie hors écran est noire ; (2) **pas de mise à l'échelle HiDPI** — l'image est en pixels du serveur X, et les coordonnées des gestes sont les mêmes pixels ; (3) **sans compositeur ni gestionnaire de fenêtres** — pas de décorations, d'ombres ni de transparence composée : l'image est exactement ce que contient le serveur, ce que verrait un utilisateur de ce serveur. Voir [`usage.md`](./usage.md#limites-de-la-cible-x11).

**Hors périmètre** : le pont natif ne couvre pas `resize` (émulation des métriques du moteur, exige CDP). Sous Wayland non-KDE, xcap reste le fallback screencap (XWayland requis) ; GNOME Wayland n'est pas supporté ([`platforms.md`](./platforms.md)).

---

## Garde-fous de découplage

Pour que DevEye reste un outil générique réutilisable par n'importe quelle app :

1. **Aucun crate `deveye-*` ne dépend d'un crate applicatif.** L'app cible peut dépendre de `deveye-sdk` (en `cfg(debug_assertions)`) ; jamais l'inverse.
2. **Versions indépendantes** : `version` par crate, pas hérité d'un workspace applicatif.
3. **Pas de logique métier d'une app cible dans le code DevEye** — les heuristiques spécifiques (ex. filtre de target par titre) doivent rester génériques ou configurables.

---

## Sécurité

Tout DevEye est **dev-only**. Six garde-fous côté SDK (la cible `x11` ajoute une garde d'isolation propre, cf. [`usage.md`](./usage.md)) :

1. **SDK strictement `cfg(debug_assertions)`** — le binaire release de l'app cible n'embarque aucune ligne DevEye. Filet de sécurité en plus de la garde côté appelant : `deveye_sdk::init` est un **no-op en build release**, donc le serveur de debug ne peut pas fuiter en production même si l'intégrateur oublie son `#[cfg(debug_assertions)]`.
2. **Bind localhost uniquement** — le mini serveur SDK écoute sur `127.0.0.1`, jamais `0.0.0.0`.
3. **Garde Host anti DNS-rebinding** — le middleware `guard_local_host` rejette en `403` toute requête dont le header `Host` n'est pas `localhost`/`127.0.0.1`/`[::1]`, **avant** la route. Le bind loopback seul ne suffit pas : une page malveillante peut résoudre un domaine sous son contrôle vers `127.0.0.1`. Chaque réponse porte aussi `X-DevEye-Protocol-Version` (`1.0`), que les clients lisent pour détecter une incompatibilité (cf. [`wire-protocol.md`](./wire-protocol.md) §2.3).
4. **Port configurable** — par défaut `31337`, surchargable par `DEVEYE_PORT`.
5. **Masquage des secrets par clés** — le SDK remplace par `<secret>` la valeur des clés sensibles (`password`, `token`, `api_key`, `authorization`, `cookie`…, sans casse ni séparateurs ; extensibles par `Config::secret_keys`), à toute profondeur, dans l'état (`GET /devtools/state`) et dans les champs structurés des logs (`GET /devtools/logs/poll`). **Périmètre exact** : le résultat de `eval_js`, les valeurs de retour des commandes et le texte libre des messages de log ne sont **pas** filtrés, de même qu'un secret stocké sous un nom de clé neutre non déclaré. C'est un filet par noms de clés, pas une garantie ([ADR-0016](adr/0016-masquage-des-secrets-par-cles-dans-le-sdk.md)). La route `ui.widgets` fait exception : le helper `deveye_sdk::ui` n'y publie jamais la valeur d'un widget secret, et `RouteDomInspector` la masque en seconde ligne, y compris dans la sortie de l'outil `dom` ([ADR-0064](adr/0064-masquage-des-secrets-transversal-et-vecteurs-partages.md), [ADR-0065](adr/0065-helper-sdk-ui-lecture-serveur-et-controles.md)).
6. **Corps de requête bornés** — au plus 1 Mio (`MAX_REQUEST_BODY_BYTES`), `413` au-delà ([ADR-0017](adr/0017-protocole-1-0-version-info-et-limites.md)).

Le contrat complet des routes HTTP du SDK (codes de statut, formes des corps, garde Host) est figé dans [`wire-protocol.md`](./wire-protocol.md).

---

## Hors périmètre actuel

- Windows et macOS (`deveye-server` ne compile que sous Linux, ADR-0037) ; GNOME Wayland (portails XDG envisagés en 1.x)
- `adapter-tauri`
- `adapter-cdp-chrome-standalone` : adapter dédié à un Chrome déjà ouvert (`--target cdp` couvre déjà un Chrome lancé avec son port de débogage, sans adapter dédié)
- Replay de scénarios enregistrés (`Recording`, absent du domaine) — tests E2E reproductibles ; la régression visuelle est couverte par l'outil `assert_visual` (comparaison à une baseline, [ADR-0030](adr/0030-assert-visual-masques-image-de-difference-et-mise-a-jour.md)), mais DevEye ne produit pas de modèle `Diff` générique
- Dashboard HTML pour humain (alternative au MCP)
- Redimensionnement de la **fenêtre OS** réelle : `resize` n'émule que le viewport du moteur de rendu (ADR-0019) ; un `window_geometry_set` serait un ajout mineur ultérieur.

---

## Risques identifiés

| Risque | Probabilité | Contre-mesure |
|--------|-------------|---------------|
| CDP wry ne fonctionne pas en runtime malgré l'API présente | Faible | Test runtime ; fallback `xcap` couvre le besoin minimal |
| Charge de maintenance face aux évolutions Dioxus | Moyenne | Versions pinned dans `Cargo.lock` ; revue à chaque bump majeur |
| Logique d'une app cible qui s'infiltre dans DevEye | Élevée | Règle de découplage rappelée ; revue obligatoire à chaque PR |
| Surface MCP qui explose | Moyenne | Review avant tout ajout d'outil MCP |
