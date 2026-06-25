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
- **Première cible = app Dioxus desktop (wry)** — concentré sur Windows en priorité, macOS/Linux ensuite.
- **Pas de cible web exigée** — on pilote l'app desktop directement, pas une variante web servie séparément.

---

## Architecture hexagonale

Cinq cercles, avec la règle d'or : **le sens des dépendances pointe toujours vers l'intérieur**.

```
┌──────────────────────────────────────────────────────────────────┐
│   ADAPTERS DevEye (cercle extérieur)                              │
│   - CDP WebView (Chromium / WebView2)                             │
│   - Screencap OS (xcap)                                           │
│   - Logs tracing-subscriber                                       │
│   - Dioxus signals (via SDK opt-in)                               │
│   - MCP server (consomme tout, expose à un client MCP)            │
│              ┌──────────────────────────────────────┐             │
│              │  PORTS DevEye (traits Rust)          │             │
│              │  Renderer, AppInspector,             │             │
│              │  InteractionSink, JsEvaluator,       │             │
│              │  LogSource, CommandRouter            │             │
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
├── deveye-core/                ← DOMAINE PUR (aucune dépendance technique)
│   └── src/
│       ├── snapshot.rs         # Snapshot { visual, dom, state, timestamp }
│       ├── command.rs          # InteractionCommand (Click, Hover, Type, KeyPress, Navigate ; Scroll/Resize prévus)
│       └── log_event.rs        # LogEvent, niveaux, fields
│
├── deveye-ports/               ← INTERFACES (traits uniquement)
│   └── src/
│       ├── renderer.rs         # trait Renderer
│       ├── inspector.rs        # trait AppInspector
│       ├── interaction.rs      # trait InteractionSink
│       ├── js_evaluator.rs     # trait JsEvaluator
│       ├── log_source.rs       # trait LogSource
│       └── command_router.rs   # trait CommandRouter
│
├── deveye-adapters/            ← IMPLÉMENTATIONS (un crate par adapter)
│   ├── adapter-cdp-webview/    # Renderer + InteractionSink + AppInspector + JsEvaluator via CDP
│   ├── adapter-screencap-xcap/ # Renderer fallback OS-level (any app)
│   ├── adapter-dioxus-signals/ # AppInspector + pont natif (JsEvaluator/InteractionSink) via SDK
│   ├── adapter-logs-tracing/   # LogSource via le buffer tracing du SDK opt-in
│   ├── adapter-commands-http/  # CommandRouter via POST /devtools/command + GET /devtools/commands
│   └── adapter-mcp-server/     # consomme les ports, expose au Model Context Protocol
│
├── deveye-sdk/                 ← micro-lib opt-in à intégrer dans l'app cible (cfg debug)
└── deveye-server/              ← BINAIRE orchestrateur (compose les adapters selon les options CLI)
```

### Convention de nommage

- **Crates** : `deveye-<role>` pour les composants internes, `adapter-<technologie>` pour les adapters.
- **Versions** : chaque crate `deveye-*` a sa propre version dans son `Cargo.toml` (pas `version.workspace = true`), pour suivre son propre cycle de release.
- **Édition** : Rust 2021, toolchain stable.

---

## Les 6 ports en détail

### 1. `Renderer`

```rust
#[async_trait]
pub trait Renderer: Send + Sync {
    async fn screenshot(&self) -> Result<PngBytes, RendererError>;
    async fn viewport_size(&self) -> Result<Size, RendererError>;
}
```

- `adapter-cdp-webview` (CDP + screencap via DevTools) — app desktop wry WebView2
- `adapter-screencap-xcap` (capture fenêtre OS) — fallback universel

### 2. `AppInspector`

```rust
#[async_trait]
pub trait AppInspector: Send + Sync {
    async fn dom_snapshot(&self) -> Result<DomTree, InspectorError>;
    async fn state_snapshot(&self) -> Result<StateDump, InspectorError>;
}
```

- `adapter-cdp-webview` (DOM via CDP `DOM.getDocument`)
- `adapter-dioxus-signals` (state via SDK opt-in qui sérialise les signaux)

### 3. `InteractionSink`

```rust
#[async_trait]
pub trait InteractionSink: Send + Sync {
    async fn dispatch(&self, cmd: InteractionCommand) -> Result<(), InteractionError>;
}
```

Variantes de `InteractionCommand` :

- `Click { position, button }` — séquence `mouseMoved` + sleep 60 ms + `mousePressed` + `mouseReleased`. Le `mouseMoved` préalable est indispensable pour déclencher les `onmouseenter` des UI dont des éléments n'apparaissent qu'au hover. Le sleep laisse le temps au framework de re-render avant le press.
- `Hover { position }` — `mouseMoved` seul, sans press. Utile pour ouvrir une zone dépendante du hover puis cliquer ailleurs.
- `Type { text }` — `Input.insertText`.
- `KeyPress { key }` — `Input.dispatchKeyEvent` (keyDown + keyUp).
- `Navigate { route }` — `Runtime.evaluate` sur un routeur exposé côté JS par l'app cible (nécessite le support dans le SDK opt-in).
- `Scroll { position, delta_x, delta_y }` *(prévu — pas encore dans `InteractionCommand`)* — `Input.dispatchMouseEvent` type `mouseWheel` ancré à `position` (`delta_y` positif = vers le bas).
- `Resize { width, height }` *(prévu — pas encore dans `InteractionCommand`)* — `Emulation.setDeviceMetricsOverride`. Émule les métriques de rendu (responsive), ne redimensionne pas la fenêtre OS réelle.

Adapter : `adapter-cdp-webview` (CDP `Input.dispatchMouseEvent` / `Input.dispatchKeyEvent` / `Runtime.evaluate`).

### 4. `JsEvaluator`

```rust
#[async_trait]
pub trait JsEvaluator: Send + Sync {
    async fn evaluate(&self, expression: &str) -> PortResult<serde_json::Value>;
}
```

Exécute une expression JavaScript dans le contexte de la page cible. L'adapter CDP traduit en `Runtime.evaluate` avec `returnByValue: true` + `awaitPromise: true` + `userGesture: true`. Volontairement permissif côté sécurité — DevEye est un outil de **dev local**.

Cas d'usage : lire un `getBoundingClientRect()` pour cliquer précisément ; déclencher un `.click()` par sélecteur (plus robuste que des coordonnées) ; sonder un état frontend.

Adapter : `adapter-cdp-webview`.

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
    async fn invoke(&self, route: &str, payload: serde_json::Value)
        -> Result<serde_json::Value, RouterError>;
    async fn list(&self) -> Result<Vec<CommandDescriptor>, RouterError>;
}
```

Permet à un agent de **dire à l'app cible** d'exécuter une action logique (« change de page », « charge telle fixture »). Implémenté par `adapter-commands-http` : l'app enregistre ses routes via `deveye_sdk::expose_route(name, description, closure)` (servies en local par `POST /devtools/command` et `GET /devtools/commands`), exposées au client MCP par les outils `invoke_command` et `list_commands`.

---

## Validation technique CDP wry

Dioxus 0.7 expose nativement le hook nécessaire pour activer CDP sur le webview wry desktop côté Windows :

```rust
// côté app cible, en cfg(debug_assertions)
dioxus::desktop::Config::new()
    .with_menu(None)
    .with_windows_browser_args("--remote-debugging-port=9222")
```

`with_windows_browser_args` passe le flag à `wry::WebViewBuilderExtWindows::with_additional_browser_args`, qui le transmet à WebView2. Résultat : un endpoint CDP standard sur `http://localhost:9222/json` auquel n'importe quel client CDP se connecte.

**Portée** :
- **Windows (WebView2)** : confirmé.
- **macOS (WKWebView)** : CDP non disponible nativement. Alternative : DevTools Apple — pilotage par script JS uniquement, pas CDP standard.
- **Linux (WebKitGTK)** : partiel (`WEBKIT_INSPECTOR_HTTP_SERVER`, pas CDP standard).

**Conséquence** : l'adapter CDP cible **Windows** en priorité. Le fallback `adapter-screencap-xcap` couvre les autres OS pour le visuel ; l'interactivité macOS/Linux viendra avec un adapter spécifique.

---

## Robustesse CDP : session browser-level + auto-reattach

L'attachement direct à un *target page* s'est révélé fragile : à chaque recompilation à chaud, la webview est détruite/recréée, le WebSocket vers l'ancien target devient mort, et la commande CDP suivante renvoie une erreur de connexion abandonnée.

`adapter-cdp-webview/src/client.rs` corrige ça avec une **session browser-level** :

1. Bootstrap : `GET /json/version` → récupère le `webSocketDebuggerUrl` racine (browser-wide).
2. Connexion WebSocket unique vers ce endpoint, persistante sur toute la durée du process `deveye-server`.
3. `Target.setDiscoverTargets { discover: true, filter: [{ type: "page" }] }` au boot → CDP émet un event pour chaque page existante et chaque nouvelle page.
4. Filtre target heuristique (`target_matches`) : `type=page` ET (URL/titre évoque `dioxus`, OU `about:blank` transitoire).
5. Pour chaque target qui matche : `Target.attachToTarget { targetId, flatten: true }` → un `sessionId` actif.
6. Les commandes (`Page.captureScreenshot`, `Input.dispatchMouseEvent`, `DOM.getDocument`, `Runtime.evaluate`…) sont envoyées avec `sessionId` au top-level (flatten mode).
7. Sur `Target.detachedFromTarget` / `Target.targetDestroyed`, le `sessionId` est effacé ; le prochain `Target.targetCreated` qui matche ré-attache automatiquement (`call()` attend une session active, timeout 10 s).

Conséquence : un hot-reload ne nécessite plus de reconnexion manuelle du MCP. Le binaire `deveye-server` reste vivant et bascule tout seul sur la nouvelle page dès qu'elle apparaît.

**Reconnexion paresseuse (`LazyCdp`, `--target`)** : `deveye-server` n'exige plus le navigateur au démarrage. La façade `LazyCdp` (re)connecte **à la demande** : chaque appel d'outil dont le WebSocket est mort retente l'attache via `/json/version`. Un navigateur absent au boot ou fermé en cours de session n'est donc pas fatal — l'outil renvoie une erreur actionnable tant que le CDP manque, puis repart seul. Le drapeau `--target {auto|cdp|sdk}` choisit la source des yeux/mains : `auto` tente CDP puis bascule **appel par appel** sur le pont natif du SDK (Linux/WebKitGTK), `cdp` et `sdk` forcent l'un ou l'autre. Détails opérationnels : [`usage.md`](./usage.md).

**Hors périmètre** : le pont natif ne couvre pas le `resize` de viewport (métriques moteur, exige CDP). La capture pixel en Wayland natif passe encore par XWayland (`adapter-screencap-kwin` à venir).

---

## Garde-fous de découplage

Pour que DevEye reste un outil générique réutilisable par n'importe quelle app :

1. **Aucun crate `deveye-*` ne dépend d'un crate applicatif.** L'app cible peut dépendre de `deveye-sdk` (en `cfg(debug_assertions)`) ; jamais l'inverse.
2. **Versions indépendantes** : `version` par crate, pas hérité d'un workspace applicatif.
3. **Pas de logique métier d'une app cible dans le code DevEye** — les heuristiques spécifiques (ex. filtre de target par titre) doivent rester génériques ou configurables.

---

## Sécurité

Tout DevEye est **dev-only**. Cinq garde-fous :

1. **SDK strictement `cfg(debug_assertions)`** — le binaire release de l'app cible n'embarque aucune ligne DevEye. Filet de sécurité en plus de la garde côté appelant : `deveye_sdk::init` est un **no-op en build release**, donc le serveur de debug ne peut pas fuiter en production même si l'intégrateur oublie son `#[cfg(debug_assertions)]`.
2. **Bind localhost uniquement** — le mini serveur SDK écoute sur `127.0.0.1`, jamais `0.0.0.0`.
3. **Garde Host anti DNS-rebinding** — le middleware `guard_local_host` rejette en `403` toute requête dont le header `Host` n'est pas `localhost`/`127.0.0.1`/`[::1]`, **avant** la route. Le bind loopback seul ne suffit pas : une page malveillante peut résoudre un domaine sous son contrôle vers `127.0.0.1`. Chaque réponse porte aussi `X-DevEye-Protocol-Version` (détection d'incompatibilité de version).
4. **Port configurable** — par défaut `31337`, surchargable par `DEVEYE_PORT`.
5. **Pas de secret exposé** — l'inspecteur de signaux ne traverse pas les types annotés `#[deveye(secret)]` (à implémenter dans le SDK).

Le contrat complet des routes HTTP du SDK (codes de statut, formes des corps, garde Host) est figé dans [`wire-protocol.md`](./wire-protocol.md).

---

## Hors périmètre actuel

- Adapter macOS (alternative à CDP sur WKWebView)
- Adapter Linux pixel natif Wayland (`adapter-screencap-kwin`, KWin `ScreenShot2`) — l'intérim XWayland (`GDK_BACKEND=x11`) couvre déjà la capture. Le pont natif (`adapter-dioxus-signals`) couvre déjà clic/saisie/DOM/eval via WebKitGTK ; seul `resize` (métriques moteur) reste indisponible hors CDP.
- `adapter-tauri`
- `adapter-cdp-chrome-standalone` (piloter Chrome déjà ouvert sans wry)
- VRT (Visual Regression Testing) — `Diff` est dans le domaine, le comparateur viendra plus tard
- Replay de `Recording` — tests E2E reproductibles
- Dashboard HTML pour humain (alternative au MCP)

---

## Risques identifiés

| Risque | Probabilité | Contre-mesure |
|--------|-------------|---------------|
| CDP wry ne fonctionne pas en runtime malgré l'API présente | Faible | Test runtime ; fallback `xcap` couvre le besoin minimal |
| Charge de maintenance face aux évolutions Dioxus | Moyenne | Versions pinned dans `Cargo.lock` ; revue à chaque bump majeur |
| Logique d'une app cible qui s'infiltre dans DevEye | Élevée | Règle de découplage rappelée ; revue obligatoire à chaque PR |
| Surface MCP qui explose | Moyenne | Review avant tout ajout d'outil MCP |
