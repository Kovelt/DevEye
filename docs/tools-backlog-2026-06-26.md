# DevEye — Backlog priorisé des nouveaux outils MCP

> **Source** : audit multi-agents (26 juin 2026) — matrice de capacités par cible, survol des MCP comparables (Playwright MCP, Chrome DevTools MCP, Puppeteer MCP), génération multi-angles, **scoring adversarial** (chaque outil doit mériter sa place face à `eval_js` / `invoke_command`), synthèse priorisée. 43 agents, 50 candidats bruts → **33 candidats notés**.
> **Périmètre** : outils MCP *à ajouter* à DevEye après M0→M7 (distinct de l'audit qualité du 25/06 qui durcissait l'existant).
> **Statut** : backlog à attaquer. Cocher au fur et à mesure.

Légende effort : **S** (≤15 min) · **M** (≤2 h) · **L** (refacto structurel).

---

## Résumé exécutif

Sur 33 candidats analysés, **17 outils sont retenus** (7 maintenant, 10 ensuite), **4 sont reportés** (CDP-only de moindre urgence), et **12 sont rejetés** comme redondants avec `eval_js`, couverts par un outil existant, ou convertis en extension d'outil plutôt qu'en nouvel outil MCP.

Deux leviers dominent :

1. **Quick-wins au domaine déjà câblé** — `KeyPress` et `Navigate` sont des variantes existantes d'`InteractionCommand`, mappées dans les **3 adapters** (CDP, SDK, kwin-eis). Seul le handler MCP manque. `screenshot_region` et `target_info` sont également des ajouts purement MCP-layer.
2. **Le trou kwin-eis** — sur les cibles natives (Uno/Skia Wayland), `eval_js` renvoie `PortError::Unsupported` : les seules modalités sont la capture visuelle et l'injection OS-level EIS. Tout ce qui passe par JS aujourd'hui (clavier riche, scroll, assertion) **n'existe pas** pour ces apps. C'est là que les nouveaux outils ont le plus de valeur.

---

## À faire maintenant

Outils dont le domaine est câblé jusqu'aux adapters, ou dont l'effort est S avec un impact kwin-eis immédiat.

| Outil | Famille | Cibles | Effort | Impact port | Pourquoi maintenant |
|---|---|---|---|---|---|
| `press_key` | Interaction | cdp · sdk · kwin-eis · auto | **S** | Nul — `InteractionCommand::KeyPress` câblé dans les 3 adapters ; ~20 l. dans `adapter-mcp-server` | Seul moyen d'envoyer Return / Tab / Escape / ArrowDown à une app native kwin-eis ; `eval_js` = Unsupported. Sur CDP : `isTrusted=true` (Monaco, CodeMirror, React Synthetic Events). |
| `navigate` | Interaction | cdp · sdk · auto | **S** | Nul — variante `InteractionCommand::Navigate` câblée dans les 3 adapters ; ~25 l. dans `adapter-mcp-server` | Interface contractuelle et découvrable ; l'agent ne doit pas connaître `window.__deveye_router__` (interne SDK) pour écrire l'`eval_js` équivalent. |
| `screenshot_region` | Capture | cdp · sdk · kwin-eis · auto | **S** | Nul — post-traitement PNG via crate `image` v0.25 (déjà dans le workspace) ; ~30 l. | Sur kwin-eis, un screenshot plein écran 2560×1440 coûte cher en tokens pour chaque vérification ciblée ; seule forme de vérification de composant sur cette cible (pas de DOM). |
| `target_info` | Inspection | cdp · sdk · kwin-eis · auto | **S** | 2-3 champs sur `DeveyePorts` + peuplement dans `compose.rs` | L'agent ne peut pas savoir si `eval_js` répondra ou renverra `Unsupported` sans tenter l'appel ; `target_info` coupe la boucle de découverte en une requête déterministe. |
| `key_combo` | Interaction | cdp · sdk · kwin-eis · auto | **M** | Nouvelle variante `InteractionCommand::KeyCombo { modifiers, key }` ; réutilise `xkb_name_to_evdev()` ; ~125 l. sur 5 fichiers | Ctrl+Z/S/A/C/V, Shift+Tab, Alt+F4 absents sur **toutes** les cibles. Le `KeyPress` actuel n'a pas de bitmask `modifiers` → Ctrl+Z est factuellement impossible aujourd'hui. Pré-requis à `clipboard_set`. |
| `scroll` | Interaction | cdp · sdk · kwin-eis · auto | **M** | Nouvelle variante `InteractionCommand::Scroll { position?, dx, dy }` ; `DeviceCapability::Scroll` déjà bindé ; ~70 l. | Toute UI à liste/grille/arbre est bloquée sans scroll sur kwin-eis. Sur CDP : `Input.dispatchMouseEvent mouseWheel` avec x/y frappe le bon conteneur (vs `window.scrollBy` qui ne défile que la racine). |
| `wait_for_gone` *(extension)* | Attente | cdp · sdk · auto | **S** | Pas un nouvel outil — ajouter `negate: bool` à `WaitPredicate::DomSelector` et à `WaitForSelectorParams` ; ~8 l. | Aucun prédicat ne couvre l'absence DOM avec polling + timeout intégré. Pattern fréquent : attendre la disparition d'un spinner / toast / modale. |

> **Note `wait_for_gone`** : implémentation préférée via `negate: bool #[serde(default)]` sur `DomSelector`, exposé par `wait_for_selector`. Pas de nouvel outil MCP ; le compteur reste cohérent.

> **Note `press_key` — divergence de nommage** : CDP attend les noms W3C (`Enter`, `ArrowDown`) ; kwin-eis attend les noms XKB (`Return`, `Down`). À documenter dans la description MCP, ou normaliser côté adapter CDP (`Return` → `Enter`). Les noms communs (`Tab`, `Escape`, `F5`, `Space`) passent sans traduction.

---

## Ensuite

Valeur avérée, effort M ou nouveau port, sans bloquer le chemin critique.

| Outil | Famille | Cibles | Effort | Impact port | Pourquoi |
|---|---|---|---|---|---|
| `assert_visual` | Capture | cdp · sdk · kwin-eis · auto | **M** | Nul — pixel diff dans `adapter-mcp-server` ; `image` v0.25 déjà dans le workspace | Sur kwin-eis : premier et **seul** outil d'assertion déterministe (pas de DOM/JS). Réduit aussi le coût token des vérifs visuelles ciblées. |
| `wait_screenshot_region` | Attente | cdp · sdk · kwin-eis · auto | **M** | Nouvelle variante `WaitPredicate::ScreenshotRegionStable` ; `image` dans `adapter-composite` | `ScreenshotStable` plein viewport est inutilisable avec un curseur clignotant / une horloge. Seule attente visuelle praticable sur kwin-eis. |
| `double_click` | Interaction | cdp · sdk · kwin-eis · auto | **M** | Nouvelle variante `InteractionCommand::DoubleClick` ; ~60 l. | Deux `click()` MCP successifs : intervalle imprévisible (5-50 ms). Un `DoubleClick` émet les deux press/release dans un seul dispatch worker. |
| `focus_window` | Interaction | kwin-eis · sdk | **M** | Nouveau port `WindowManager` (10e) ; `KwinWindowManager` via D-Bus Scripting déjà présent | KWin EIS route vers la fenêtre ayant le focus OS ; une notif Plasma ou un dialog système dévie **silencieusement** tous les clics, sans signal d'erreur. |
| `assert_state` | Inspection | cdp · sdk | **S** | Nul — composition sur `AppInspector::state_snapshot()` ; ~20 l. | `state()` expose le snapshot sans sémantique d'assertion ; `isError=true` est un signal déterministe pour les pipelines (symétrie avec `assert_selector`). |
| `wait_for_log` | Attente | cdp · sdk | **M** | Nouvelle variante `WaitPredicate::LogMessage { contains, level_min? }` ; champ `log_source` dans `CompositeWaitCondition` ; ~80 l. | `eval_js` ne peut pas observer les logs `tracing` Rust bufferisés côté SDK. Boucler sur `logs_tail` + sleeps est l'anti-pattern que la famille Wait* supprime. |
| `handle_dialog` | Interaction | cdp | **S** | Nouvelle variante `InteractionCommand::HandleDialog { accept, prompt_text? }` ; ~30 l. | Quand `alert()`/`confirm()`/`prompt()` se déclenche, le moteur JS se suspend → `eval_js` est physiquement impossible. `Page.handleJavaScriptDialog` est la seule issue. |
| `screenshot_element` | Capture | cdp · sdk | **M** | Nouvelle méthode `screenshot_clip(rect)` sur le trait `Renderer` (5 implémenteurs) ; `image` dans `adapter-mcp-server` ; ~140 l. | `eval_js` retourne du JSON, pas un PNG. Recadre sur le composant d'intérêt. |
| `upload_file` | Interaction | cdp | **M** | Nouvelle variante `InteractionCommand::UploadFile { selector, paths }` ; `DOM.setFileInputFiles` ; ~70 l. | La sécurité navigateur interdit de setter `input[type=file].value` depuis JS → `DOM.setFileInputFiles` est le seul chemin. Verrou non contournable par `eval_js`. |
| `clipboard_get` | Workflow | cdp · sdk · kwin-eis · auto | **M** | Nouveau port `ClipboardPort { get(), set() }` + crate `adapter-clipboard-wayland` ; à livrer avec `clipboard_set` | `navigator.clipboard.readText()` lève `NotAllowedError` en contexte automatisé ; sur kwin-eis `eval_js` = Unsupported. `wl-paste` OS-level est la seule approche fiable (`wl-clipboard` validé sur la machine). |

---

## Plus tard

CDP-only ou besoin non encore opérationnel sur le chemin critique.

| Outil | Famille | Cibles | Effort | Condition de montée en priorité |
|---|---|---|---|---|
| `resize_viewport` | Capture | cdp | M | Quand les tests de la tête WASM atlas à différentes résolutions deviennent un workflow. ⚠️ **Breaking change** sur le trait `Renderer` (6 implémenteurs, dont `MonitorRenderer` et `LazyCdp`). |
| `network_requests` | Inspection | cdp | M | Quand les tests d'intégration atlas WASM via CDP deviennent actifs. Piège : `Network.enable` doit être ré-envoyé dans l'arm `Target.attachedToTarget`. |
| `set_cookie` + `get_cookies` | Workflow | cdp | M | Livrer en bundle (un seul nouveau port `CookieManager`). `HttpOnly` est l'unique argument face à `eval_js`. Intérêt limité hors tests de flux d'auth sur cible CDP. |
| `clipboard_set` | Workflow | kwin-eis | L | Pré-requis : `key_combo` (Ctrl+V impossible sans modifiers EIS). Bundle avec `clipboard_get` quand le pilotage de texte Unicode long sur kwin-eis devient opérationnel. |

---

## Rejetés / couverts par `eval_js` ou un outil existant

À ne pas re-proposer sans argument nouveau.

### Redondants avec `eval_js` ou un outil existant

| Outil | Raison |
|---|---|
| `get_text` | Sous-ensemble de `find_element` (champ `.text`, déjà null-safe) et `assert_selector` (`actual_text`). Si la limite `slice(0, 200)` gêne, l'ajuster. |
| `get_value` | `eval_js` one-liner. Correction correcte : ajouter `value_prop: ('value' in el) ? el.value : null` au snippet de `find_element` (~10 caractères). Aucun nouvel outil. |
| `navigate_history` | `history.back()/forward()` = `eval_js` verbatim, même appel CDP. Seul `Page.reload({ ignoreCache: true })` est un delta réel — trop étroit. |
| `storage_state` | Deux one-liners JS, synchrones et atomiques en JS monothread. Nul gain sur kwin-eis. |

### Mieux traités comme extensions d'outils existants

| Outil | Action correcte |
|---|---|
| `wait_for_text` | Ajouter `WaitPredicate::TextContent { text, selector?, visible? }` et l'exposer via `wait_condition`. kwin-eis Unsupported retire la justification d'un outil dédié. |
| `console_messages` | Étendre le snippet console aux niveaux `log`/`info`/`debug` + filtre `levels?` sur `collect_console_errors`. La voie CDP-native (`Runtime.consoleAPICalled`) est en réalité un effort L. |

### Conflits architecturaux ou portée insuffisante

| Outil | Raison |
|---|---|
| `action_sequence` | Contredit la décision M1 : `wait_condition` relâche le gate pendant l'attente. Tenir le gate toute une séquence bloque tous les outils jusqu'à 60 s ; le relâcher détruit l'atomicité. Un agent réactif séquence déjà mieux. |
| `wait_for_network_idle` | `Page.lifecycleEvent "networkIdle"` ne couvre pas les XHR SPA post-clic. L'implémentation correcte est push-based, incompatible avec le `CompositeWaitCondition` poll-based. Effort L réel. |
| `state_get` | `state()` expose déjà le snapshot complet. `expose_signal` n'est utilisé par aucune app aujourd'hui. Reconsidérer si des registries > 50 entrées apparaissent. |
| `get_page_info` | Sur kwin-eis, `url`/`title` sont toujours `null`. La vraie valeur (exposer `viewport_size()`) justifierait au mieux un `get_viewport_size` sans les champs confusants. |
| `select_option` | Couvert par `eval_js` (sélecteur + `.value` + `dispatchEvent('change')`) ; kwin-eis Unsupported. Faible valeur pour le coût (~100 l.). |

---

## Séquencement recommandé — Premier sprint

**Objectif** : débloquer les trous kwin-eis critiques et les quick-wins domaine câblé, sans dette d'architecture ni nouveau port.

| Priorité | Outil(s) | Durée | Justification |
|---|---|---|---|
| 1 | `press_key` + `navigate` | 0,5 j | Domaine 100 % câblé dans les 3 adapters ; handler MCP seul manque (~45 l.). Débloque le clavier kwin-eis et le routage client-side. |
| 2 | `screenshot_region` + `target_info` | 0,5 j | `image` déjà dans le workspace ; ~90 l. ; aucun port modifié. `target_info` élimine la boucle de découverte de cible. |
| 3 | `wait_for_gone` (via `negate`) | 0,25 j | ~8 lignes ; pas de nouvel outil ; symétrie immédiate avec `wait_for_selector`. |
| 4 | `key_combo` | 1 j | Pré-requis clipboard ; Ctrl+Z/S/A/C/V absents aujourd'hui ; bloc de construction des raccourcis kwin-eis. |
| 5 | `scroll` | 1 j | Deuxième trou kwin-eis après `press_key` ; `DeviceCapability::Scroll` déjà bindé. |
| 6 | `assert_visual` | 1,5 j | Premier outil d'assertion déterministe pour kwin-eis ; valeur immédiate pour les pipelines de vérification. |

**Total estimé : ~4,75 jours.** Ces 6 entrées (8 outils + 1 extension) couvrent l'essentiel des trous kwin-eis et tous les quick-wins domaine, sans nouveau port ni dette d'archi.
