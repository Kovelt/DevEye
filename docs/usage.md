# DevEye — guide d'usage

> Mode d'emploi pratique pour brancher Claude Code (ou tout client MCP) sur une app cible en cours de dev via DevEye. Pour le contexte stratégique et l'architecture, voir [`architecture.md`](./architecture.md).

---

## Prérequis

- Sous Windows, WebView2 Runtime à jour (livré avec Edge — toute machine Win10/11 récente l'a déjà).
- Sous Linux/macOS (WebKitGTK / WKWebView), pas de CDP : DevEye pilote l'app par le **pont natif** du SDK opt-in (`--target sdk`, ou repli automatique de `--target auto`). Voir « Linux — piloter l'app desktop sans CDP » plus bas.
- Toolchain Rust stable. Pour une app cible Dioxus : `dx` CLI Dioxus 0.7+ alignée sur le `Cargo.lock` (`dx self-update` si décalage).
- Un client MCP (Claude Code, par exemple).

---

## Étape 1 — Build du binaire `deveye-server` une fois

```powershell
cargo build --release -p deveye-server
```

Le binaire atterrit dans `target\release\deveye-server.exe`. Le compiler en release plutôt qu'en debug évite que le client attende ~3 sec à chaque démarrage de session.

Alternative en dev itératif (recompile à la volée, plus lent au démarrage) : pointer `.mcp.json` sur `cargo run` au lieu du binaire — voir Étape 2.

---

## Étape 2 — Créer `.mcp.json` à la racine du projet client

```json
{
  "mcpServers": {
    "deveye": {
      "type": "stdio",
      "command": "C:\\chemin\\vers\\DevEye\\target\\release\\deveye-server.exe",
      "args": [
        "--cdp-url", "http://localhost:9222",
        "--sdk-url", "http://localhost:31337",
        "--serve-mcp"
      ]
    }
  }
}
```

**Notes** :

- `--target auto` (défaut) tente CDP puis bascule **appel par appel** sur le pont natif si le CDP est injoignable ; `--target cdp` / `--target sdk` forcent l'un ou l'autre. Sur Linux desktop WebKitGTK, mettre `sdk` (ou `auto`). Pour une cible sans WebView ni SDK (Uno/Skia), utiliser `--target kwin-eis` (injection KWin EIS — Wayland natif KDE).
- **Chemin absolu obligatoire sous Windows**, double backslashes inclus. Claude Code n'expanse **pas** `${CLAUDE_PROJECT_DIR}` ni `${workspaceFolder}` dans le champ `command` d'un `.mcp.json` — la première est une variable réservée aux *hooks*, la seconde est une variable VSCode. Si tu y mets `${CLAUDE_PROJECT_DIR}`, `claude mcp list` répond `Missing environment variables: CLAUDE_PROJECT_DIR` et `deveye` n'apparaît pas dans `/mcp`.
- Pour pointer sur `cargo run` (pas besoin de rebuild manuel après chaque modif DevEye) :
  ```json
  "command": "cargo",
  "args": ["run", "--quiet", "-p", "deveye-server", "--", "--serve-mcp"]
  ```
  (avec `cargo` lancé depuis le repo DevEye, ou en précisant `--manifest-path`)
- `.mcp.json` contient un chemin absolu spécifique à la machine — soit on le gitignore, soit chaque dev maintient sa version locale (les autres champs sont stables, seul `command` change).

---

## Étape 3 — Activer le MCP dans le client

À l'ouverture du dossier, le client détecte `.mcp.json`. Vérifier :

```
/mcp
```

→ `deveye` doit apparaître dans la liste. Si « disabled » : `/mcp approve deveye`.
(La première fois, le client demande confirmation pour exécuter un binaire local — mesure de sécurité standard.)

---

## Étape 4 — Workflow d'utilisation type

Ordre des lancements **important** : l'app cible doit déjà écouter CDP + SDK quand le client appelle un outil.

```powershell
# Terminal 1 — l'app cible avec CDP (:9222) et SDK opt-in (:31337) actifs
#   (l'app intègre deveye-sdk derrière une feature, en cfg(debug_assertions))
dx serve --platform desktop --features deveye

# Attendre que la fenetre s'ouvre + 2-3 sec pour que WebView2 soit pret

# Terminal 2 — le client MCP dans le projet
claude
```

Une fois dans le client, les outils MCP sont sous le préfixe `mcp__deveye__`. **20 outils sont disponibles** (2 _(à venir)_ pas encore implémentés) :

| Outil | Paramètres | Retour | Usage typique |
|-------|------------|--------|---------------|
| `screenshot` | aucun | image PNG (contenu image natif MCP, affichée par le client) | « prends un screenshot » |
| `click` | `{x, y}` | `"ok"` | « clique sur 50, 200 » (envoie `mouseMoved` + sleep 60 ms + press/release) |
| `right_click` | `{x, y}` | `"ok"` | « clic droit à 50, 200 » (ouvre les menus contextuels — émet `contextmenu`) |
| `hover` | `{x, y}` | `"ok"` | « hover sur 10, 375 » (ouvre une zone rétractable sans cliquer) |
| `click_selector` | `{selector}` | JSON `{ok, reason?}` | « clique sur `button.btn-primary` » (robuste aux décalages de coords) |
| `click_text` | `{text}` | JSON `{ok, reason?}` | « clique sur le bouton "Valider" » (matche `button/a/[role=button]` par `innerText` exact) |
| `type_text` | `{text}` | `"ok"` | « tape "Bonjour" dans le champ focus » |
| `eval_js` | `{expression}` | JSON sérialisé | « renvoie `document.querySelectorAll('button').length` » (Promise awaitée) |
| `dom` | aucun | JSON du DOM courant | « renvoie le DOM courant » |
| `state` | aucun | JSON `{ ... }` | « quel est l'état applicatif courant ? » |
| `logs_tail` | aucun | JSON `[{ level, target, message, ... }]` | « affiche les derniers logs » |
| `find_element` | `{selector}` | JSON `{found, tag, text, rect, visible, disabled, attributes}` | « inspecte le premier `#save-btn` » (alternatif léger à `dom`) |
| `assert_selector` | `{selector, expected_text?, expected_count?}` | JSON `{ok, selector, actual_count, actual_text?}` | « vérifie qu'il y a exactement 3 `.card` » (`isError` si l'assertion échoue) |
| `wait_for_selector` | `{selector, timeout_ms?, visible?}` | JSON `{found, elapsed_ms}` | « attends que `#result` soit présent (max 5 s) » (sans sleep arbitraire) |
| `wait_condition` | `{predicate, timeout_ms?}` | JSON `{satisfied, elapsed_ms, detail}` | « attends que `count` vaille 3 dans le state » (dom-selector / state-key / screenshot-stable) |
| `drag_drop` | `{from_x, from_y, to_x, to_y}` | `"ok"` | « glisse de (10, 20) vers (100, 200) » |
| `invoke_command` | `{name, args?}` | JSON (résultat de la commande) | « invoque `set-page` avec `{page:"Agenda"}` » (route exposée par l'app) |
| `list_commands` | aucun | JSON `[{name, description}]` | « quelles commandes l'app expose-t-elle ? » |
| `collect_console_errors` | aucun | JSON `[{level, text, url?, line?, timestamp}]` | « y a-t-il eu des erreurs JS depuis la dernière vérification ? » |
| `list_windows` | aucun | JSON `[{id, title, url, kind}]` | « liste les onglets/fenêtres de la cible CDP » |
| `scroll` _(à venir)_ | `{dx?, dy?, x?, y?}` | `"ok"` | « scrolle vers le bas de 600 px » (`dy` positif = bas ; ancrage par défaut au centre) |
| `resize` _(à venir)_ | `{width, height}` | `"ok"` | « passe en 375×812 » (émule le rendu responsive, pas la fenêtre OS) |

**Conseils d'usage** :

- Pour les UI qui dépendent d'un hover (sidebar rétractable, menus contextuels), envoyer `hover(x, y)` d'abord, puis `click_text` / `click_selector` pour cibler l'élément révélé.
- Préférer `click_text` / `click_selector` aux coordonnées brutes : elles ne cassent pas quand le layout bouge (responsive, animations, refactor CSS) et déclenchent un vrai clic CDP (`mousedown`/`mouseup`/`pointer`), au même titre que `click(x, y)`.
- `eval_js` est le couteau-suisse pour tout cas non couvert : lire un `getBoundingClientRect`, sonder un état frontend, déclencher du JS interne au framework, ou injecter des événements clavier (`new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })`).
- **Touche clavier** : pas d'outil `press_key` dédié — utiliser `eval_js` pour dispatcher un `KeyboardEvent` sur `document.activeElement` (ex. `Enter`, `Tab`, `Escape`, `ArrowDown`). Exemple : `eval_js({ expression: "document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))" })`.
- **Navigation** : utiliser `invoke_command` si l'app expose une route logique (ex. `set-page`) ; sinon `eval_js` avec le hook routeur du SDK (`window.__deveye_router__`).
- Le buffer logs est **drainé** à chaque appel `logs_tail` : un second appel ne renverra que les events arrivés entre-temps. Pour attendre de nouveaux logs après une action, utiliser une boucle `logs_tail` ou `wait_condition` avec un prédicat `state-key`.
- Pour éviter tout sleep arbitraire après une action asynchrone, préférer `wait_for_selector` (attend un sélecteur CSS) ou `wait_condition` (prédicats : `dom-selector`, `state-key`, `screenshot-stable`).
- Pour piloter l'app par sa **logique métier** plutôt que par l'UI : l'app expose des routes via `deveye_sdk::expose_route("set-page", "Change la page", |payload| { ... })` (en `cfg(debug_assertions)`) ; l'agent les découvre avec `list_commands` puis les déclenche avec `invoke_command`. Plus robuste qu'une cascade de clics pour mettre l'app dans un état donné.
- Plus besoin de `/mcp reconnect` après un hot-reload Dioxus (auto-reattach browser-level — cf. [`architecture.md`](./architecture.md)).

---

## Linux — piloter l'app desktop sans CDP (pont natif)

Sur Linux, le webview Dioxus desktop est **WebKitGTK**, qui n'expose pas de CDP. DevEye pilote alors l'app par le **pont natif** du SDK opt-in (`--target sdk`, ou repli automatique de `--target auto`) :

```bash
# Terminal 1 — l'app desktop avec le SDK (XWayland requis pour le screenshot xcap)
GDK_BACKEND=x11 dx serve --platform desktop --features deveye

# Terminal 2 — le serveur MCP en cible native
cargo run -p deveye-server -- --serve-mcp --target sdk
```

- `click_*` / `type_text` / `dom` / `eval_js` passent par la pompe `document::eval` de l'app (événements JS synthétiques complets : pointer + mouse + focus) — moins fidèles que CDP mais fonctionnels.
- `screenshot` : sans `--screencap-window`, capture **plein écran** (`MonitorRenderer`) — adapté au CI/headless. Avec `--screencap-window <sous-chaîne>`, capture de la fenêtre par titre : sous KDE/Wayland natif ajouter `--screencap-backend kwin` (aucun XWayland requis) ; ailleurs, xcap avec `GDK_BACKEND=x11`. Lancer via `dx serve` (un `cargo run` nu n'a pas ses assets `asset!()` → page sans CSS).
- `state` / `logs_tail` viennent du SDK, comme en mode CDP.
- `resize` n'est **pas** disponible via le pont natif (les métriques du moteur exigent CDP) ; tout le reste l'est.

> **Leçons d'usage (Linux/web)** : (1) pour chronométrer une animation, vérifier `document.visibilityState === "visible"` — une fenêtre masquée gèle `requestAnimationFrame`. (2) Le scroll de l'app vit souvent dans un conteneur (`.shell__content`), pas dans `window`. (3) En mode web sans `logs_tail`, poser un collecteur `window.addEventListener('error'/'unhandledrejection', …)` via `eval_js` et le relever en fin de session (à reposer après chaque `location.reload()`).

---

## Linux — piloter une app native Wayland (kwin-eis)

Pour une cible **sans WebView ni SDK** — notamment atlas Uno desktop (rendu Skia) — DevEye injecte des événements d'entrée au niveau OS via **KWin EIS** (`org.kde.KWin.EIS.RemoteDesktop`), sans XWayland, sans shim dans l'app.

```bash
# Terminal 1 — l'app native (Skia/Uno, .NET ou autre)
dotnet run --project src/Atlas.App -f net10.0-desktop -c Debug

# Terminal 2 — le serveur MCP en mode kwin-eis
cargo run -p deveye-server -- --serve-mcp --target kwin-eis
```

- **Mains** : clics, survols, saisies et touche clavier passent par KWin EIS (événements OS-level vrais, identiques à la souris et au clavier physiques).
- **Yeux** : screencap de la fenêtre (`--screencap-window <sous-chaîne>`) ou plein écran sans ce flag. Sous KDE/Wayland, `--screencap-backend kwin` (défaut auto) ; ailleurs, xcap avec `GDK_BACKEND=x11`.
- **DOM / JS / state** : indisponibles (rendu natif, pas de WebView). `dom`, `eval_js`, `wait_for_selector`, `find_element` renvoient `PortError::Unsupported`.
- **Prérequis** : `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1` dans l'environnement de session KWin (cf. [architecture.md](./architecture.md)) ; la connexion D-Bus qui initie EIS doit rester ouverte pendant toute la session — DevEye la maintient.

```json
{
  "mcpServers": {
    "deveye": {
      "type": "stdio",
      "command": "/chemin/vers/DevEye/target/release/deveye-server",
      "args": [
        "--target", "kwin-eis",
        "--screencap-window", "atlas",
        "--serve-mcp"
      ]
    }
  }
}
```

---

## Piloter une app WASM via un Chromium externe

Pour une tête **WASM** (ex. Uno/atlas `net10.0-browserwasm`, ou toute SPA) servie par un Chromium externe plutôt qu'un webview Dioxus, on pilote en **CDP pur** (`--target cdp`) avec un **filtre de cible explicite** (le défaut `dioxus` ne matcherait pas).

```bash
# Terminal 1 — l'app WASM (dotnet, dx, vite…), ex. sur http://localhost:5000
dotnet run --project src/MonApp -f net10.0-browserwasm
# attendre « Now listening on: http://localhost:5000 »

# Terminal 2 — Chromium ISOLÉ en mode debug (profil dédié OBLIGATOIRE)
brave --remote-debugging-port=9222 --user-data-dir=/tmp/deveye-brave http://localhost:5000

# Terminal 3 — Claude Code dans le projet client
claude
```

> **`--user-data-dir` obligatoire** : sans lui, si Brave/Chrome est déjà ouvert, l'instance existante absorbe la commande et ignore `--remote-debugging-port` → `/json/version` injoignable.

`.mcp.json` ciblant la tête WASM par sous-chaîne d'URL (aucun nom d'app n'est codé en dur dans DevEye — c'est une valeur d'opérateur) :

```json
{
  "mcpServers": {
    "deveye": {
      "type": "stdio",
      "command": "/chemin/vers/DevEye/target/release/deveye-server",
      "args": [
        "--cdp-url", "http://localhost:9222",
        "--cdp-target-filter-url", "localhost:5000",
        "--target", "cdp",
        "--serve-mcp"
      ]
    }
  }
}
```

Variante par titre : `--cdp-target-filter-title atlas` (les deux flags se combinent en OU). En cible CDP pure, le SDK opt-in n'est pas embarqué (cible .NET/WASM) → `state`, `logs_tail`, `invoke_command`, `list_commands` renvoient une erreur réseau explicite ; seules les capacités CDP (`screenshot`, `click`, `right_click`, `hover`, `click_selector`, `click_text`, `type_text`, `eval_js`, `dom`, `find_element`, `assert_selector`, `wait_for_selector`, `wait_condition`, `drag_drop`, `collect_console_errors`, `list_windows`) répondent.

---

## Piloter atlas (.NET / Uno) — shim Atlas.DevEye

atlas n'embarque ni CDP ni le SDK Rust (c'est du .NET 10 / Uno). Le pont est le **shim `Atlas.DevEye`** (projet C# injecté **en Debug uniquement**) qui sert le même wire protocol HTTP que le SDK Rust, sur deux sources / deux ports :

| Source | Port | Capacités |
|---|---|---|
| **Atlas.App** — tête desktop Skia (Debug) | `31337` | `state` (ViewModels), `logs_tail` (logs client), `invoke_command` / `list_commands` |
| **Atlas.Api** — backend ASP.NET (Debug) | `31338` | `logs_tail` (Serilog), `invoke_command` / `list_commands` |

```bash
# Tête desktop atlas en Debug → lance le shim sur 127.0.0.1:31337
dotnet run --project src/Atlas.App -f net10.0-desktop -c Debug

# deveye-server : état + logs du CLIENT atlas (port 31337)
cargo run -p deveye-server -- --target sdk --sdk-url http://localhost:31337 --serve-mcp
```

- **Logs backend** (Serilog) : pointer une seconde instance sur `--sdk-url http://localhost:31338`.
- **Yeux** sur la tête **WASM** d'atlas : passer par le mode CDP (cf. section précédente), p. ex. `--target cdp --cdp-target-filter-title atlas`. La tête desktop Skia n'est pas un webview ; pour la voir, le screencap OS (`--screencap-window atlas`) reste possible.
- `eval_js` répond **503** côté .NET (pas de pompe webview) — les capacités JS/DOM passent par le CDP de la tête WASM.
- Le shim n'existe **qu'en build Debug** (référence `Atlas.DevEye` conditionnée Debug + `#if DEBUG`) : aucun binaire Release d'atlas n'embarque DevEye.

---

## Étape 5 — Vérifier que tout marche

Dans le client, après les deux lancements :

> Prends un screenshot puis dis-moi l'état applicatif courant.

Le client doit appeler `mcp__deveye__screenshot` (et afficher l'image) puis `mcp__deveye__state`. Si ça marche, on est branché.

---

## Étape 6 — Sécurité : vérifier qu'aucune ligne DevEye ne fuite en prod

```bash
# Le binaire release de l'app cible sans la feature deveye ne doit linker aucun crate DevEye
#   Linux / macOS :
cargo tree -p <votre-app> | grep deveye
#   Windows :
cargo tree -p <votre-app> | findstr deveye
# (doit ne rien retourner)

# Le SDK opt-in bind strict localhost
#   Linux :
ss -tlnp | grep 31337
#   macOS :
lsof -nP -iTCP:31337 -sTCP:LISTEN
#   Windows :
netstat -an | findstr 31337
# (adresse doit etre 127.0.0.1:31337, jamais 0.0.0.0)
```

Côté app cible, le seul fichier qui référence DevEye est son `Cargo.toml` (feature optionnelle) et quelques lignes en `#[cfg(feature = "deveye")]`. En build release sans `--features deveye`, le compilateur élimine ces blocs entièrement.

---

## Troubleshooting

| Symptôme | Cause probable | Fix |
|----------|----------------|-----|
| `CDP injoignable au demarrage (...) — reconnexion automatique au premier appel d'outil.` | App cible pas lancée avec `--features deveye`, navigateur CDP absent, ou WebView2 pas prêt | Lancer l'app/navigateur puis **rappeler l'outil** : l'attache est paresseuse, inutile de relancer `deveye-server`. En natif Linux, utiliser `--target sdk`. |
| `timeout en attente d'une page CDP attachable (10 s)` | Aucune page ouverte ne matche le filtre de cible | Vérifier que le navigateur/l'app est chargé(e) ; passer `--cdp-target-filter-url <sous-chaine>` ou `--cdp-target-filter-title <sous-chaine>` pour viser une autre page (ex. tête WASM) |
| `Connection refused: 127.0.0.1:31337` côté `state` ou `logs_tail` | Feature `deveye` absente du build de l'app | Vérifier `dx serve --features deveye`. Le SDK refuse de bind sinon |
| Le client ne voit pas `deveye` dans `/mcp` | `.mcp.json` mal placé, syntaxe JSON invalide, ou chemin du binaire faux | Vérifier `.mcp.json` à la racine ; valider la syntaxe ; vérifier que `target\release\deveye-server.exe` existe |
| `Tool execution failed: ...` quand on appelle un outil | Erreur côté adapter (CDP injoignable, SDK crashé, etc.) | Regarder les logs `deveye-server` dans le panneau MCP du client ; relancer l'app si besoin |
| Le screenshot est blanc / partiel | DevTools du webview ouvert et masquant la page, ou fenêtre minimisée | Fermer les DevTools ; restaurer la fenêtre |
| Warning « not all bits valid » à la compilation avec `--features deveye` | Conflit `schemars` entre une vieille version transitive et celle de `rmcp 1.7` | `cargo update -p schemars` ; `rmcp` requiert schemars 1.x |

---

## Limitations connues

- **CDP = Windows (WebView2) uniquement.** Sur macOS (WKWebView) et Linux (WebKitGTK), CDP n'est pas exposé. Sous Linux, le **pont natif** du SDK (`--target sdk`/`auto`) conserve clic / saisie / DOM / JS / state via la pompe `document::eval` — seul `resize` (métriques moteur) reste indisponible. Pour les cibles sans WebView ni SDK (ex. atlas Uno desktop / Skia), utiliser `--target kwin-eis` (injection OS-level KWin EIS — yeux par screencap).
- **Pas de stream WebSocket** sur les logs : `logs_tail` est un drain ponctuel ; pour attendre de nouveaux événements sans boucle arbitraire, utiliser `wait_condition` (prédicat `state-key` si l'app expose l'état) ou une boucle `logs_tail` côté agent. Pas de flush poussé temps réel (choix assumé).
- **Commandes logiques** : `invoke_command` / `list_commands` sont opérationnels via `adapter-commands-http`, mais ne renvoient quelque chose que si l'app cible a déclaré des routes avec `deveye_sdk::expose_route` (sinon `list_commands` renvoie une liste vide). C'est à l'app d'exposer ce qu'elle veut rendre pilotable.
- **Test runtime end-to-end** non automatisé : à valider manuellement à chaque release majeure Dioxus/wry (on dépend de l'option WebView2 `--remote-debugging-port`).
- **Reconnect du WebSocket browser** : **géré** depuis la feature `--target` / `LazyCdp` (juin 2026). Un navigateur absent au démarrage ou fermé en cours de session n'est plus fatal — chaque appel d'outil retente l'attache (l'outil renvoie une erreur actionnable tant que le CDP manque). Plus besoin de `/mcp reconnect`.
- **`Click` ajoute ~60 ms de latence** (le `mouseMoved` préalable + sleep pour laisser le re-render). Imperceptible en usage manuel ; à garder en tête pour un test stress.

---

## Quand mettre à jour cette doc

- Nouvel outil MCP côté `adapter-mcp-server` → ajouter une ligne dans le tableau Étape 4.
- Nouveau mode de transport autre que stdio (HTTP+SSE pour piloter à distance) → ajouter une section.
- Nouvel OS supporté pour CDP → note dans Limitations et Étape 4.

---

## Docs liées

- [`architecture.md`](./architecture.md) — architecture hexagonale, 9 ports, robustesse CDP
