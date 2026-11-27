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

- `--target auto` (défaut) tente CDP puis bascule **appel par appel** sur le pont natif si le CDP est injoignable ; `--target cdp` / `--target sdk` forcent l'un ou l'autre. Sur Linux desktop WebKitGTK, mettre `sdk` (ou `auto`). Pour une cible sans WebView ni SDK (Uno/Skia), utiliser `--target kwin-eis` (injection KWin EIS — Wayland natif KDE). Pour une app X11 native (egui, GTK, Qt…) dans un serveur X privé, utiliser `--target x11 --display :N` (voir la section dédiée).
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

Une fois dans le client, les outils MCP sont sous le préfixe `mcp__deveye__`. **43 outils sont disponibles** :

| Outil | Paramètres | Retour | Usage typique |
|-------|------------|--------|---------------|
| `screenshot` | aucun | image PNG (contenu image natif MCP, affichée par le client) | « prends un screenshot » |
| `screenshot_region` | `{x, y, width, height}` | image PNG recadrée | « capture la zone (0, 0, 320, 200) » (vérif ciblée, moins de tokens ; seule vérif visuelle de composant en kwin-eis) |
| `screenshot_element` | `{selector}` | image PNG recadrée (ou `isError {found:false}`) | « capture `#facture` » (résout le rect par JS, gère le DPI ; cdp/sdk) |
| `assert_visual` | `{baseline_path, threshold?, x?, y?, width?, height?}` | JSON `{ok, diff_ratio, threshold, same_dims, ...}` | « compare au golden `ref.png` » (crée la baseline si absente ; `isError` si > seuil ; **seule assertion déterministe en kwin-eis**) |
| `click` | `{x, y}` | `"ok"` | « clique sur 50, 200 » (envoie `mouseMoved` + sleep 60 ms + press/release) |
| `double_click` | `{x, y}` | `"ok"` | « double-clic sur 50, 200 » (édition inline, sélection de mot ; émet `dblclick`) |
| `right_click` | `{x, y}` | `"ok"` | « clic droit à 50, 200 » (ouvre les menus contextuels — émet `contextmenu`) |
| `hover` | `{x, y}` | `"ok"` | « hover sur 10, 375 » (ouvre une zone rétractable sans cliquer) |
| `click_selector` | `{selector}` | JSON `{ok, reason?}` | « clique sur `button.btn-primary` » (robuste aux décalages de coords) |
| `click_text` | `{text}` | JSON `{ok, reason?}` | « clique sur le bouton "Valider" » (matche `button/a/[role=button]` par `innerText` exact) |
| `type_text` | `{text}` | `"ok"` | « tape "Bonjour" dans le champ focus » |
| `press_key` | `{key}` | `"ok"` | « presse Enter » (Tab/Escape/ArrowDown… ; marche en kwin-eis. Noms W3C en CDP, XKB en kwin-eis) |
| `key_combo` | `{modifiers, key}` | `"ok"` | « Ctrl+S » (`modifiers`: ctrl/shift/alt/meta ; seul moyen d'envoyer Ctrl+Z/C/V) |
| `navigate` | `{route}` | `"ok"` | « navigue vers `Agenda` » (routeur du SDK opt-in ; indispo en kwin-eis) |
| `scroll` | `{x?, y?, dx?, dy?}` | `"ok"` | « scrolle vers le bas de 600 px » (`dy > 0` = bas ; ancrage au centre par défaut ; cible le bon conteneur) |
| `handle_dialog` | `{accept, prompt_text?}` | `"ok"` | « accepte la confirmation » (CDP : pendant un `alert`/`confirm`/`prompt`, `eval_js` est impossible) |
| `upload_file` | `{selector, paths}` | `"ok"` | « charge `/tmp/a.png` dans `input[type=file]` » (CDP `DOM.setFileInputFiles` ; non faisable en JS) |
| `focus_window` | `{title_pattern?}` | JSON `{ok, title}` | « donne le focus à la fenêtre `mon_app` » (KWin ; crucial avant une séquence kwin-eis) |
| `clipboard_get` | aucun | JSON `{text}` | « lis le presse-papiers » (Wayland `wl-paste` ; fiable en contexte automatisé) |
| `clipboard_set` | `{text}` | `"ok"` | « mets ce texte dans le presse-papiers » (Wayland `wl-copy` ; combiner à `key_combo` Ctrl+V) |
| `eval_js` | `{expression}` | JSON sérialisé | « renvoie `document.querySelectorAll('button').length` » (Promise awaitée) |
| `dom` | aucun | JSON du DOM courant (erreur `isError` si l'arbre dépasse ~100 Ko : cibler avec `find_element`/`eval_js`) | « renvoie le DOM courant » |
| `state` | `{keys?}` | JSON `{ ... }` (filtré aux clés demandées si `keys` fourni) | « quel est l'état applicatif courant ? » ; `keys: ["count"]` pour un seul signal |
| `assert_state` | `{expected}` | JSON `{ok, mismatches}` | « vérifie `{count: 3, page: "Agenda"}` dans le state » (`isError` si divergence) |
| `target_info` | aucun | JSON `{target, eyes, hands, eval_js, dom, state, console_errors, list_windows, notes}` | « quelles capacités a la cible ? » (à appeler en premier ; dit si `eval_js`/`dom` répondront) |
| `logs_tail` | `{limit?}` | JSON `{events: [{level, target, message, ...}], truncated, total}` (défaut 200 events les plus récents ; `total` = nombre réellement drainé) | « affiche les derniers logs » |
| `logs_wait` | `{level_min?, timeout_ms?, limit?}` | JSON `{events, timed_out, truncated, total}` | « clique Valider puis attends une erreur (max 10 s) » (long-poll : rend tous les logs drainés ; `level_min` invalide = `invalid_params` ; défaut 5000 ms, plafond 30000) |
| `find_element` | `{selector}` | JSON `{found, tag, text, value, rect, visible, disabled, attributes}` | « inspecte le premier `#save-btn` » (alternatif léger à `dom` ; `value` = valeur des inputs) |
| `assert_selector` | `{selector, expected_text?, expected_count?}` | JSON `{ok, selector, actual_count, actual_text?}` | « vérifie qu'il y a exactement 3 `.card` » (`isError` si l'assertion échoue) |
| `wait_for_selector` | `{selector, timeout_ms?, visible?, negate?}` | JSON `{found, elapsed_ms}` | « attends que `#result` soit présent » ; avec `negate=true` : « attends que `.spinner` disparaisse » (wait_for_gone) |
| `wait_condition` | `{predicate, timeout_ms?}` | JSON `{satisfied, elapsed_ms, detail}` | « attends que `count` vaille 3 » (prédicats : dom-selector / state-key / screenshot-stable / screenshot-region-stable / log-message) |
| `wait_for_log` | `{contains, level_min?, timeout_ms?}` | JSON `{satisfied, elapsed_ms, detail}` | « attends un log contenant "saved" » (logs `tracing` du SDK ; draine le buffer pendant l'attente) |
| `wait_screenshot_region` | `{x, y, width, height, threshold_ms?, timeout_ms?}` | JSON `{satisfied, elapsed_ms, detail}` | « attends que la zone (0,0,320,200) se stabilise » (ignore curseur/horloge ailleurs) |
| `wait_ready` | `{timeout_ms}` | JSON `{ready, waited_ms}` | « attends que l'app soit prête » (state non vide ; poll 200 ms ; `isError` si timeout) |
| `drag_drop` | `{from_x, from_y, to_x, to_y}` | `"ok"` | « glisse de (10, 20) vers (100, 200) » |
| `invoke_command` | `{name, args?}` | JSON (résultat de la commande) | « invoque `set-page` avec `{page:"Agenda"}` » (route exposée par l'app) |
| `list_commands` | aucun | JSON `[{name, description}]` | « quelles commandes l'app expose-t-elle ? » |
| `collect_console_errors` | aucun | JSON `[{level, text, url?, line?, timestamp}]` | « y a-t-il eu des erreurs JS depuis la dernière vérification ? » |
| `list_windows` | aucun | JSON `[{id, title, url, kind}]` | « liste les onglets/fenêtres de la cible CDP » |
| `resize` | `{width, height}` | JSON `{ok}` | « passe en 375×812 » (CDP : émule le viewport responsive, pas la fenêtre OS ; `0`×`0` retire l'émulation ; sdk/kwin-eis/x11 : `isError` `unsupported`) |

**Conseils d'usage** :

- **Coordonnées validées** : `click`, `right_click`, `double_click`, `hover`, `drag_drop` et l'ancrage de `scroll` refusent (erreur `invalid_params`, rien n'est envoyé) une coordonnée négative, ou hors du viewport quand sa taille est connue. Un clic hors fenêtre en kwin-eis/x11 échoue donc au lieu de réussir à vide ; un menu système qui déborde de la fenêtre n'est pas atteignable par coordonnées.
- Pour les UI qui dépendent d'un hover (sidebar rétractable, menus contextuels), envoyer `hover(x, y)` d'abord, puis `click_text` / `click_selector` pour cibler l'élément révélé.
- Préférer `click_text` / `click_selector` aux coordonnées brutes : elles ne cassent pas quand le layout bouge (responsive, animations, refactor CSS) et déclenchent un vrai clic CDP (`mousedown`/`mouseup`/`pointer`), au même titre que `click(x, y)`.
- `eval_js` est le couteau-suisse pour tout cas non couvert : lire un `getBoundingClientRect`, sonder un état frontend, déclencher du JS interne au framework, ou injecter des événements clavier (`new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })`).
- **Touche clavier** : `press_key({ key })` pour une touche unique (`Enter`, `Tab`, `Escape`, `ArrowDown`…), `key_combo({ modifiers, key })` pour une combinaison (`Ctrl+S`, `Ctrl+Shift+P`). Les deux marchent en kwin-eis et en x11 (events OS-level), là où `eval_js` est indisponible. Nuance de nommage : noms W3C en CDP (`Enter`, `ArrowDown`), noms XKB en kwin-eis (`Return`, `Down`), les deux acceptés en x11 ; les noms communs (`Tab`, `Escape`, `F5`) passent partout.
- **Navigation** : `navigate({ route })` (routeur du SDK opt-in, interface découvrable) ; ou `invoke_command` si l'app expose une route logique métier (ex. `set-page`). Indisponible en kwin-eis (pas de routeur applicatif OS-level).
- **Découverte de cible** : appeler `target_info` en début de session dit quelles capacités répondront (ex. `eval_js`/`dom` = false en kwin-eis) — évite de tester un outil pour découvrir qu'il renvoie `Unsupported`.
- **Disparition d'un élément** : `wait_for_selector({ selector, negate: true })` attend qu'un spinner/toast/modale disparaisse (pas de sleep arbitraire).
- Le buffer logs est **drainé** à chaque appel `logs_tail` : un second appel ne renverra que les events arrivés entre-temps. Pour attendre de nouveaux logs après une action sans boucler côté agent, utiliser `logs_wait` (long-poll, filtre `level_min`, rend les logs drainés) ; `wait_for_log` attend un message précis mais consomme les logs qu'il ignore.
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
- `resize` n'est **pas** disponible via le pont natif (l'émulation des métriques du moteur exige CDP) ; tout le reste l'est.

> **Leçons d'usage (Linux/web)** : (1) pour chronométrer une animation, vérifier `document.visibilityState === "visible"` — une fenêtre masquée gèle `requestAnimationFrame`. (2) Le scroll de l'app vit souvent dans un conteneur (`.shell__content`), pas dans `window`. (3) En mode web sans `logs_tail`, poser un collecteur `window.addEventListener('error'/'unhandledrejection', …)` via `eval_js` et le relever en fin de session (à reposer après chaque `location.reload()`).

---

## Linux — piloter une app native Wayland (kwin-eis)

Pour une cible **sans WebView ni SDK** — par exemple une app Uno desktop (rendu Skia) — DevEye injecte des événements d'entrée au niveau OS via **KWin EIS** (`org.kde.KWin.EIS.RemoteDesktop`), sans XWayland, sans shim dans l'app.

```bash
# Terminal 1 — l'app native (Skia/Uno, .NET ou autre)
dotnet run --project src/MonApp -f net10.0-desktop -c Debug

# Terminal 2 — le serveur MCP en mode kwin-eis
cargo run -p deveye-server -- --serve-mcp --target kwin-eis
```

- **Mains** : clics, survols, saisies, touche unique (`press_key`), combinaisons (`key_combo` — Ctrl+S, Ctrl+Z…) et défilement (`scroll`) passent par KWin EIS (événements OS-level vrais, identiques à la souris et au clavier physiques). `navigate` reste indisponible (routeur applicatif).
- **Assertions visuelles** : faute de DOM, `screenshot_region` (vérif ciblée à bas coût) et `assert_visual` (golden) sont les seules vérifications déterministes ; `target_info` confirme les capacités absentes (`eval_js`/`dom` = false).
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
        "--screencap-window", "mon_app",
        "--serve-mcp"
      ]
    }
  }
}
```

---

## Linux — piloter une app X11 native dans un serveur isolé (x11)

Pour une app **graphique native sans WebView** (egui, GTK, Qt, plugin audio avec GUI…) que l'on peut lancer sur un serveur X dédié : `--target x11` parle à un **serveur X explicite** (typiquement `Xvfb`) avec ses propres yeux (`GetImage`), ses mains (extension **XTEST**, de vrais événements souris/clavier) et sa liste de fenêtres (`QueryTree`). Aucun gestionnaire de fenêtres, aucun `xcap`, aucune session Wayland requis, et **la session graphique de l'utilisateur n'est jamais touchée**.

```bash
# 1. Un serveur X PRIVE (numéro >= 100 — pas le :99 habituel des CI —, jamais d'écoute TCP)
Xvfb :250 -nolisten tcp -screen 0 1400x1000x24 &

# 2. L'app cible tourne SUR ce serveur (rendu logiciel si pas de GPU virtuel)
DISPLAY=:250 LIBGL_ALWAYS_SOFTWARE=1 ./mon-app &

# 3. Le serveur MCP, avec un environnement épuré (rien de la session vivante)
env -u WAYLAND_DISPLAY -u XDG_SESSION_TYPE -u DISPLAY -u DBUS_SESSION_BUS_ADDRESS \
  ./target/release/deveye-server --serve-mcp --target x11 \
  --display :250 --window "Mon App" --sdk-url http://127.0.0.1:31400
```

### Options

| Option | Rôle |
|---|---|
| `--display :N` | **Obligatoire**, sans défaut ni repli sur `$DISPLAY`. Seul un display local `:N` / `:N.S` est accepté, avec `N >= 100` (voir la garde). Refusé avec une autre cible |
| `--window <sous-chaîne>` (alias de `--screencap-window`) | Fenêtre pilotée : sous-chaîne, insensible à la casse, de `_NET_WM_NAME`, puis `WM_NAME`, puis `WM_CLASS`. Sans elle : tout l'écran du serveur, origine `(0, 0)` |
| `--x11-class <sous-chaîne>` | (refusé avec une autre cible que `x11`) Contrainte supplémentaire sur `WM_CLASS` (départage deux fenêtres de même titre) |
| `--sdk-url` | SDK de l'app (state, logs, commandes, et DOM — voir plus bas). Défaut `http://127.0.0.1:31337` |

`--screencap-backend`, `--cdp-*` n'ont aucun effet sur cette cible.

### Garde d'isolation (non contournable)

XTEST injecte de **vrais** événements : pointé sur la session de l'utilisateur, il déplacerait sa souris et taperait dans ses fenêtres. Le serveur refuse donc de démarrer (ou, pour les deux derniers points, à la première connexion) si :

1. le display n'est pas local (`host:N`, chemin, vide) ;
2. son numéro est **< 100** (les sessions réelles sont sur `:0`/`:1` ; `Xvfb :99` est donc refusé, prendre `:100` et plus) ;
3. c'est le display de la session courante (`$DISPLAY`) ;
4. le socket `/tmp/.X11-unix/X<n>` appartient à un autre uid ;
5. le serveur annonce l'extension `XWAYLAND` (XWayland d'un compositeur Wayland = session vivante).

Il n'y a **pas** d'option pour lever ces refus. Le presse-papiers (`clipboard_get`/`clipboard_set`) est volontairement indisponible sur cette cible.

### Ce que chaque outil fait

- **Yeux** — `screenshot` capture la **racine** rognée au rectangle de la fenêtre (pas le drawable de la fenêtre, qui peut rendre du noir avec un enfant GL), au format PNG ; profondeur 24 ou 32 bits uniquement. L'image a toujours la **taille de la fenêtre** : si celle-ci déborde de l'écran, la partie invisible est noire, si bien que le pixel `(x, y)` de l'image est le point `(x, y)` des gestes. Une fenêtre entièrement hors écran est une erreur.
- **Mains** — `click`, `double_click`, `right_click`, `hover`, `drag_drop`, `scroll`, `type_text`, `press_key`, `key_combo`. Les positions sont **relatives à la fenêtre** ; l'origine de la fenêtre est relue à chaque appel (si elle bouge entre deux gestes, le clic suit).
  - Timing adapté aux toolkits immédiats (egui) : pointeur placé ~30 ms avant l'appui ; `drag_drop` interpole 16 déplacements espacés de ~16 ms (un saut unique n'enclencherait aucun glisser) ; `scroll` envoie un cran de molette (boutons 4/5/6/7) par tranche de 120 px de `dx`/`dy`, plafonné à 100 crans ; `dy > 0` = vers le bas.
  - Clavier = celui **du serveur X** (`GetKeyboardMapping`), pas une table `us` : on tape ce que son layout produirait (Shift et AltGr gérés). Un caractère absent du layout est injecté par **remappage temporaire d'un keycode libre**, restauré ensuite. Noms de touches W3C (`Enter`, `Escape`, `ArrowUp`, `KeyA`…) et XKB (`Return`, `Up`…) acceptés ; modificateurs `ctrl`, `shift`, `alt`, `meta`/`super`. `type_text` est limité à 4 096 caractères.
  - Tout est résolu **avant** d'émettre : une touche ou un modificateur inconnu n'émet rien ; sur erreur en cours de geste, tout ce qui était enfoncé (touches, boutons) est relâché en ordre inverse.
  - `navigate`, `handle_dialog`, `upload_file` : `Unsupported` (pas de routeur applicatif ni de dialogue natif).
- **Fenêtres** — `list_windows` (fenêtres visibles titrées ou classées, `id` = identifiant X en hexadécimal, `kind` = `x11-window`), `window_geometry` (rect écran absolu), `focus_window` (`SetInputFocus` + premier plan : sans gestionnaire de fenêtres le focus suit le pointeur, donc l'appeler avant de taper — `type_text`/`press_key` le font déjà pour la fenêtre configurée). Le titre peut arriver tard (baseview…) : la résolution réessaie jusqu'à 2 s.
- **DOM** — pas de HTML : `dom` renvoie ce que l'app répond à la route de commande **`ui.widgets`** du SDK (voir ci-dessous). Sans cette route : erreur d'inspection explicite.
- **State / logs / commandes** — comme pour toute cible, via `--sdk-url`.
- **JS** — indisponible : `eval_js`, `find_element`, `click_selector`, `click_text`, `wait_for_selector`, `wait_for_text` renvoient `Unsupported`. Pour attendre : `wait_condition` avec `state-key`, `screenshot-stable` ou `log-message`. Le prédicat `state-key` compare une clé de **premier niveau** de l'état à une valeur **exacte** : publier les grandeurs à attendre au premier niveau (ou attendre une image stable).
- **Presse-papiers** — `clipboard_get` / `clipboard_set` répondent `Unsupported` sur cette cible (le seul presse-papiers disponible serait celui de la session vivante) : pour saisir du texte, utiliser `type_text`.

### Exposer l'arbre de widgets : la route `ui.widgets`

DevEye ne sait pas lire l'arbre d'interface d'une app native (egui peint à la main) : c'est l'app qui le publie. Avec `deveye-sdk` :

```rust
deveye_sdk::expose_route("ui.widgets", "Arbre des widgets (rects en coordonnées fenêtre)", |_payload| {
    serde_json::json!({
        "window": { "width": 1240, "height": 900 },
        "widgets": [
            { "id": "vcf/cutoff", "kind": "knob", "label": "Cutoff", "rect": [10, 20, 56, 68] }
        ]
    })
});
```

Contrat de `RouteDomInspector` (crate `adapter-composite`) : à chaque appel de l'outil `dom`, il fait `POST /devtools/command` avec `{ "route": "ui.widgets", "payload": {} }` et renvoie la valeur JSON du handler **telle quelle** — le schéma des nœuds appartient à l'app. Conseils : rects en **coordonnées fenêtre** (le même repère que les positions de `click`, qui ajoute l'origine de la fenêtre), arbre compact (l'outil `dom` refuse au-delà de ~100 Ko). L'agent lit les rects dans `dom`, puis `click` au centre. Route absente → `PortError::Inspect` (« route `ui.widgets` indisponible… »), SDK injoignable → erreur réseau.

### `.mcp.json`

```json
{
  "mcpServers": {
    "deveye-x11": {
      "type": "stdio",
      "command": "/chemin/vers/DevEye/target/release/deveye-server",
      "args": [
        "--target", "x11",
        "--display", ":250",
        "--window", "Mon App",
        "--sdk-url", "http://127.0.0.1:31400",
        "--serve-mcp"
      ]
    }
  }
}
```

La connexion au serveur X est **paresseuse** : le MCP peut démarrer avant Xvfb et avant l'app, et se reconnecte seul si Xvfb est relancé (une coupure de socket est la seule erreur « réseau » ; une fenêtre introuvable n'en est jamais une, donc aucune bascule silencieuse vers une autre cible).

### Tests de l'adapter

Les tests purs (garde, fenêtres, clavier, gestes, conversion de pixels) tournent avec `cargo test`. Les tests d'intégration lancent leur **propre Xvfb privé** (`:251`..`:259`) et sont `#[ignore]` :

```bash
cargo test -p adapter-x11 -- --ignored --test-threads=1
```

> **Piège** : `setxkbmap` n'a aucun effet sur certains Xvfb (ex. Fedora : le serveur ne peut pas écrire sa sortie XKM). Pour tester un clavier non `us`, écrire le keymap par le protocole core (`ChangeKeyboardMapping`), comme le fait le test d'intégration.

---

## Piloter une app WASM via un Chromium externe

Pour une tête **WASM** (ex. Uno `net10.0-browserwasm`, ou toute SPA) servie par un Chromium externe plutôt qu'un webview Dioxus, on pilote en **CDP pur** (`--target cdp`). Sans filtre, la cible est la **première page qui n'est pas `about:blank`** ; avec plusieurs pages ouvertes, poser un **filtre de cible explicite**.

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

Variante par titre : `--cdp-target-filter-title mon_app` (les deux flags se combinent en OU). `--cdp-accept-blank` autorise en plus la page `about:blank` transitoire d'une webview au boot (désactivé par défaut). En cible CDP pure, le SDK opt-in n'est pas embarqué (cible .NET/WASM) → `state`, `logs_tail`, `invoke_command`, `list_commands` renvoient une erreur réseau explicite ; seules les capacités CDP (`screenshot`, `click`, `right_click`, `hover`, `click_selector`, `click_text`, `type_text`, `eval_js`, `dom`, `find_element`, `assert_selector`, `wait_for_selector`, `wait_condition`, `drag_drop`, `collect_console_errors`, `list_windows`) répondent.

---

## Piloter une app non Rust (.NET, autre) via le wire protocol

Une app qui n'embarque ni CDP ni le SDK Rust (par exemple une app .NET / Uno) peut quand même fournir `state`, `logs_tail` et les commandes logiques : il suffit qu'elle serve le **wire protocol HTTP** de DevEye ([wire-protocol.md](./wire-protocol.md)) sur `127.0.0.1`, en build debug uniquement. Une instance de `deveye-server` par port :

```bash
# Tête desktop de l'app en Debug → serveur du wire protocol sur 127.0.0.1:31337
dotnet run --project src/MonApp -f net10.0-desktop -c Debug

# deveye-server : état + logs de l'app (port 31337)
cargo run -p deveye-server -- --target sdk --sdk-url http://localhost:31337 --serve-mcp
```

- Un second composant (par exemple un backend) se pilote avec une seconde instance sur un autre port : `--sdk-url http://localhost:31338`.
- **Yeux** : en CDP (`--target cdp`) pour une tête WASM, ou par screencap OS (`--screencap-window mon_app`) pour une tête desktop non webview.
- `eval_js` répond **503** côté app sans webview (pas de pompe) — les capacités JS/DOM passent alors par le CDP d'une tête WASM.
- Le serveur du protocole ne doit exister **qu'en build debug** : aucun binaire release n'embarque DevEye.

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
| `timeout en attente d'une page CDP attachable (10 s) : aucune page ne correspond au filtre (…) ; pages vues : …` | Aucune page ouverte ne correspond au filtre de cible ; le message liste les pages vues (titre et URL tronqués) | Corriger le filtre d'après la liste : `--cdp-target-filter-url <sous-chaine>` ou `--cdp-target-filter-title <sous-chaine>` ; `--cdp-accept-blank` si la page est encore `about:blank` ; vérifier que le navigateur/l'app est chargé(e) |
| `Connection refused: 127.0.0.1:31337` côté `state` ou `logs_tail` | Feature `deveye` absente du build de l'app | Vérifier `dx serve --features deveye`. Le SDK refuse de bind sinon |
| Le client ne voit pas `deveye` dans `/mcp` | `.mcp.json` mal placé, syntaxe JSON invalide, ou chemin du binaire faux | Vérifier `.mcp.json` à la racine ; valider la syntaxe ; vérifier que `target\release\deveye-server.exe` existe |
| `Tool execution failed: ...` quand on appelle un outil | Erreur côté adapter (CDP injoignable, SDK crashé, etc.) | Regarder les logs `deveye-server` dans le panneau MCP du client ; relancer l'app si besoin |
| Le screenshot est blanc / partiel | DevTools du webview ouvert et masquant la page, ou fenêtre minimisée | Fermer les DevTools ; restaurer la fenêtre |
| Warning « not all bits valid » à la compilation avec `--features deveye` | Conflit `schemars` entre une vieille version transitive et celle de `rmcp 1.7` | `cargo update -p schemars` ; `rmcp` requiert schemars 1.x |

---

## Limitations connues

- **CDP = Windows (WebView2) uniquement.** Sur macOS (WKWebView) et Linux (WebKitGTK), CDP n'est pas exposé. Sous Linux, le **pont natif** du SDK (`--target sdk`/`auto`) conserve clic / saisie / DOM / JS / state via la pompe `document::eval` — seul `resize` (émulation des métriques du moteur) reste indisponible. Pour les cibles sans WebView ni SDK (ex. Uno desktop / Skia), utiliser `--target kwin-eis` (injection OS-level KWin EIS — yeux par screencap).
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

- [`architecture.md`](./architecture.md) — architecture hexagonale, 12 ports, robustesse CDP
