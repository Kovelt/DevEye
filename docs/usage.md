# DevEye : guide d'usage et référence

Référence unique et exacte de l'usage de DevEye : options du binaire, sous-commande `doctor`, suite de conformité, **les 44 outils MCP** avec leurs paramètres et leurs retours, comportement par cible, limites, dépannage. Chaque affirmation a été relue contre le code ; en cas d'écart, le code a raison et ce document a un bug.

- Pour **démarrer** avec un type d'app précis : [guides/](guides/README.md).
- Pour **intégrer le SDK** dans une app : [integration.md](integration.md).
- Pour le **contexte et les ports** : [architecture.md](architecture.md) ; pour les **plateformes** : [platforms.md](platforms.md) ; pour le **contrat HTTP du SDK** : [wire-protocol.md](wire-protocol.md).

---

## Installation et prérequis

- **Linux uniquement** ([platforms.md](platforms.md)) ; un client MCP (Claude Code, par exemple).
- Installer `deveye-server` : archive de release GitHub, `cargo install --git https://github.com/Kovelt/DevEye --tag v1.0.0 deveye-server --locked`, ou depuis les sources (`just install`) ; détail dans le [README](../README.md#installation).
- Bibliothèques de compilation (installation depuis les sources) : voir le README.
- Selon la cible : session KDE Plasma Wayland (`kwin-eis`), `Xvfb` (`x11`), navigateur ou webview Chromium avec le port de débogage ouvert (`cdp`), SDK embarqué dans l'app (`sdk`, état, logs, commandes).

Avant tout, `deveye-server doctor --target <cible>` dit ce qui manque ([Diagnostic](#diagnostic--deveye-server-doctor)).

---

## Choisir la cible

`--target` choisit où vont les **yeux** (captures) et les **mains** (entrées). L'état, les logs et les commandes viennent **toujours** du SDK de l'app (`--sdk-url`).

| `--target` | Pour quelle app | Yeux | Mains |
|---|---|---|---|
| `auto` (défaut) | webview Chromium, avec repli sur le pont SDK | CDP, repli capture OS | CDP, repli événements JS synthétiques |
| `cdp` | webview ou navigateur Chromium | CDP `Page.captureScreenshot` | CDP `Input.*` (événements réels) |
| `sdk` | webview sans CDP (WebKitGTK) avec le SDK et sa pompe | capture OS (KWin sous KDE Wayland, `xcap` sinon) | événements JS synthétiques via la pompe |
| `kwin-eis` | app native Wayland sans webview ni SDK (Skia, Uno) | capture OS | injection OS par KWin EIS (vrais événements) |
| `x11` | app X11 native (egui, GTK, Qt) lancée sur un serveur X privé | `GetImage` du serveur privé | XTEST sur le serveur privé |

`auto` n'est pas une cible de plus : chaque appel essaie CDP, et **seule une erreur de liaison** (cible injoignable) bascule sur le pont SDK ; chaque bascule est journalisée et `target_info.effective_backend` dit lequel sert (`cdp`, `sdk` ou `indetermine`).

---

## Brancher dans Claude Code : `.mcp.json`

Créer `.mcp.json` à la racine du projet de l'app. Le serveur parle MCP sur **stdio** : ses traces vont sur **stderr** (filtre `RUST_LOG`, `info` par défaut), jamais sur stdout. Si `deveye-server` n'est pas dans le `PATH` de Claude Code, mettre son chemin absolu dans `command`.

**CDP** (webview ou navigateur Chromium, port de débogage 9222) :

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

**`auto`** (CDP si disponible, pont SDK sinon) :

```json
{
  "mcpServers": {
    "deveye": {
      "type": "stdio",
      "command": "deveye-server",
      "args": ["--sdk-url", "http://127.0.0.1:31337", "--serve-mcp"]
    }
  }
}
```

**SDK** (webview WebKitGTK, fenêtre repérée par son titre) :

```json
{
  "mcpServers": {
    "deveye": {
      "type": "stdio",
      "command": "deveye-server",
      "args": ["--target", "sdk", "--sdk-url", "http://127.0.0.1:31337",
               "--screencap-window", "Mon App", "--serve-mcp"]
    }
  }
}
```

**KWin EIS** (app native Wayland KDE) :

```json
{
  "mcpServers": {
    "deveye": {
      "type": "stdio",
      "command": "deveye-server",
      "args": ["--target", "kwin-eis", "--screencap-window", "Mon App", "--serve-mcp"]
    }
  }
}
```

**x11** (app X11 sur un `Xvfb` privé, display 100 ou plus) :

```json
{
  "mcpServers": {
    "deveye": {
      "type": "stdio",
      "command": "deveye-server",
      "args": ["--target", "x11", "--display", ":250", "--window", "Mon App",
               "--sdk-url", "http://127.0.0.1:31400", "--serve-mcp"]
    }
  }
}
```

Ici l'app est lancée avec `DEVEYE_PORT=31400` (`DISPLAY=:250 DEVEYE_PORT=31400 ./mon-app`) : le port de `--sdk-url` doit être celui où son SDK écoute, 31337 par défaut.

Vérifier dans Claude Code avec `/mcp` : `deveye` doit apparaître. Variante de développement du serveur lui-même (recompile à la volée, plus lent à démarrer) : `"command": "cargo"`, `"args": ["run", "--quiet", "-p", "deveye-server", "--", "--serve-mcp"]` depuis un clone de DevEye.

---

## `deveye-server` : options

| Option | Valeur par défaut | Rôle |
|---|---|---|
| `--serve-mcp` | | Sert le MCP sur stdio. **Obligatoire** sans sous-commande ; ne se combine pas avec une sous-commande. Sans mode, le binaire s'arrête en erreur d'usage (code 2). |
| `--target <auto\|cdp\|sdk\|kwin-eis\|x11>` | `auto` | Cible des yeux et des mains. |
| `--cdp-url <URL>` | `http://localhost:9222` | URL HTTP du CDP. `auto` et `cdp` seulement. |
| `--cdp-target-filter-url <SOUS-CHAÎNE>` | | La page CDP cible doit avoir une URL contenant cette sous-chaîne (insensible à la casse). `auto` et `cdp` seulement. |
| `--cdp-target-filter-title <SOUS-CHAÎNE>` | | Idem pour le titre ; relation OU avec le filtre d'URL. `auto` et `cdp` seulement. |
| `--cdp-accept-blank` | désactivé | Accepte aussi `about:blank` comme cible. `auto` et `cdp` seulement. |
| `--sdk-url <URL>` | `http://127.0.0.1:31337` | URL du SDK de l'app (état, logs, commandes, pont natif). Toutes les cibles. Préférer `127.0.0.1` à `localhost` : sur la plupart des Linux, `localhost` se résout d'abord en `::1` alors que le SDK n'écoute qu'en IPv4. |
| `--screencap-window <TITRE>` (alias `--window`) | | Sous-chaîne du titre (insensible à la casse) de la fenêtre à capturer ou piloter. En `x11`, la fenêtre pilotée ; sans elle, tout l'écran du serveur X. En `sdk` et `auto` sans ce flag : repli plein écran. Sous KDE Wayland (`cdp`, `sdk`, `kwin-eis`), **requis pour `focus_window`**. |
| `--screencap-backend <auto\|kwin\|xcap>` | `auto` | Capture OS : `auto` choisit KWin sous KDE Wayland, `xcap` sinon. Refusé avec `x11`. |
| `--display <:N>` | | Display du serveur X privé. **Obligatoire** avec `--target x11`, refusé sinon ; aucun repli sur `$DISPLAY`. |
| `--x11-class <SOUS-CHAÎNE>` | | `x11` seulement : sous-chaîne qui doit aussi figurer dans `WM_CLASS` (départage deux fenêtres de même titre). |
| `--version` | | Version, commit de build, versions du wire protocol et du protocole MCP. |

Une option sans effet sur la cible choisie est **refusée** avec un message d'usage, pas ignorée ([ADR-0031](adr/0031-validation-des-flags-selon-la-cible.md)). Codes de sortie : 0 succès, 2 erreur d'usage (clap), 1 échec de composition au démarrage.

### Filtre de page CDP

Sans `--cdp-target-filter-*`, la cible est la **première page qui n'est pas `about:blank`** ; les pages suivantes ne la remplacent pas. Avec un filtre, une page est retenue si son URL **ou** son titre correspond. Si aucune page ne convient après 10 s, l'erreur liste les pages vues (titre et URL tronqués, dix au plus) et propose le flag à ajuster ([ADR-0014](adr/0014-filtre-de-cible-cdp-permissif-par-defaut.md)). La connexion CDP est paresseuse : un navigateur absent au démarrage ou fermé en cours de session n'est pas fatal, l'attache est retentée à chaque appel.

---

## Diagnostic : `deveye-server doctor`

```bash
deveye-server doctor --target kwin-eis
deveye-server doctor --target x11 --display :250
```

Diagnostic **sans effet voulu** : il n'ouvre aucun serveur MCP, ne compose aucun adapter, n'injecte aucun événement. Seule exception : la sonde facultative `ui.widgets` invoque la route de l'app, un handler que l'app fournit (en lecture seule par convention, mais non vérifiable par DevEye), ce qui enregistre une lecture et peut attendre jusqu'à 500 ms. Il accepte les options de ciblage du serveur (`--target`, `--sdk-url`, `--cdp-url`, `--display`, `--screencap-window`…) ; les filtres de cible CDP (`--cdp-target-filter-*`, `--cdp-accept-blank`) ne servent qu’au serveur. Avec `--target x11` sans `--display`, il ne refuse pas la ligne de commande : il le diagnostique. Il ne sonde que ce que la cible utilise (CDP seulement pour `auto` et `cdp`), en parallèle. Neuf vérifications, chacune avec un statut et, si besoin, un conseil :

| Vérification | Rôle |
|---|---|
| Session graphique | type de session et de bureau (KDE Wayland exigé par `kwin-eis`) |
| KWin | processus `kwin_wayland` ou `kwin_x11` présent |
| `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS` | variable posée dans l'environnement de KWin (sinon la capture est refusée) |
| Accès EIS | l'interface `org.kde.KWin.EIS.RemoteDesktop` annonce `connectToEIS` (la connexion réelle n'est pas tentée) |
| `Xvfb` | binaire disponible (ou serveur demandé déjà lancé) |
| Display X privé | le display passe la garde d'isolation et son socket existe |
| SDK de l'app cible | `GET /devtools/info` répond, version du protocole compatible avec celle du serveur |
| CDP (navigateur) | le point d'accès CDP répond |
| Arbre `ui.widgets` (facultatif) | seulement si le SDK annonce `ui.widgets/1` : lit la route (`POST /devtools/command` sur la route de l'app, lecture seule par convention) et vérifie le schéma, les ids uniques et non vides, les parents et le focus connus ; un arbre invalide (ids dupliqués compris) est un **échec**, une route illisible, sans arbre publié ou dont des ids sont synthétisés (format legacy sans `id` : les références `w:<id>` de `snapshot` et `act` ne seraient pas stables d'une relance à l'autre) un avertissement ([ADR-0065](adr/0065-helper-sdk-ui-lecture-serveur-et-controles.md)) |

Statuts : `[OK]`, `[ATTENTION]` (utile mais non bloquant : capacité réduite), `[ÉCHEC]` (prérequis bloquant de la cible demandée), `[SANS OBJET]`. Code de sortie : **0** (prêt, ou avertissements seulement), **1** (au moins un prérequis bloquant manque), **2** (erreur d'usage). Le texte du diagnostic n'est pas couvert par la promesse de stabilité, ses codes de sortie le sont.

---

## Suite de conformité : `deveye-conformance`

Valide qu'une implémentation du wire protocol (le SDK Rust, ou un SDK écrit dans un autre langage) respecte le contrat, à partir de sa seule URL ([wire-protocol.md](wire-protocol.md) §7).

```bash
just conformance                                  # contre le SDK Rust, en processus
just conformance http://127.0.0.1:31337           # contre une implémentation à l'écoute
deveye-conformance --url http://127.0.0.1:31337   # binaire du crate deveye-conformance
```

| Option | Rôle |
|---|---|
| `--url <URL>` | URL de base de l'implémentation (défaut `http://127.0.0.1:31337`) |
| `--slow-expression <JS>` | expression que la pompe laisse sans réponse (vérifie le `504`, dure 10 s) |
| `--abandon-expression <JS>` | expression que la pompe abandonne (vérifie le `500`) |
| `--invoke-route <NOM>` | commande sans effet de bord à invoquer (vérifie le `200` de `POST /devtools/command`) |
| `--strict` | échoue aussi quand une vérification est ignorée (exigence non observable) |

Codes de sortie : 0 conforme, 1 écarts, 2 la suite n'a pas pu s'exécuter. Le binaire est construit depuis le dépôt (`cargo run -p deveye-conformance -- --url …`) ; il n'est pas livré dans l'archive de release.

---

## Les outils MCP

44 outils stables en sept familles, plus deux outils **expérimentaux** (`snapshot` et `act`, [Désigner un élément par référence](#désigner-un-élément-par-référence-expérimental)) hors de la promesse de stabilité. Appeler **`target_info` en premier** : il dit quels outils répondront sur la cible active.

### Conventions de résultat

- **Succès sans valeur** (clic, saisie, défilement, navigation…) : `{ "ok": true }`.
- **Échec sémantique** : résultat d'outil avec `isError: true` et un JSON lisible, jamais une erreur de protocole. Formes courantes : `{ "ok": false, "reason": "unsupported", "detail": … }` (capacité absente sur la cible : DOM en `kwin-eis`, `resize` hors CDP, presse-papiers hors Wayland…), `{ "ok": false, "reason": "sdk_unavailable", "detail": … }` (SDK de l'app injoignable), `{ "ok": false, "reason": "js_exception", "message", "stack"? }` (exception levée par le JavaScript évalué), `{ "ok": false, "reason": "interaction_failed", "detail": … }`.
- **Sorties structurées (depuis la 1.1)** : vingt-huit outils à petite sortie (`target_info`, `wait_*`, `assert_selector`, `assert_state`, `window_geometry`, `focus_window`, `click_selector`, `click_text` et les acquittements `{ "ok": true }`) déclarent un `outputSchema` dans `tools/list` et renvoient en plus de leur texte un `structuredContent` égal au même JSON. Le **texte est identique à celui de la 1.0**, octet pour octet : un client qui ignore ces champs ne voit aucune différence. Un échec sémantique de ces outils (attente expirée, assertion fausse) garde le type du succès ; une erreur typée (`unsupported`, `sdk_unavailable`, `interaction_failed`) porte `structuredContent` `{ ok: false, reason, detail, hint?, candidates? }`, qui ne suit pas l'`outputSchema` (il ne décrit que le succès). `reason` est une liste ouverte : traiter une valeur inconnue comme une erreur générique. Restent sans `outputSchema`, et en texte seul pour leurs **succès** (leurs erreurs typées `unsupported`, `sdk_unavailable` et `interaction_failed` portent, elles, le même `structuredContent` `{ ok: false, reason, detail }` : un client qui retransmet texte et structuré les voit en double, ce qui est assumé car elles sont courtes) : `logs_tail`, `logs_wait`, `list_windows`, `find_element`, `list_commands`, `collect_console_errors`, `eval_js`, `dom`, `state`, `invoke_command`, `observe`, `clipboard_get` et les captures ([ADR-0067](adr/0067-sorties-structurees-mcp-palier-a.md)). Claude Code transmet au modèle le `structuredContent` d'un succès (un seul exemplaire, pas de doublon de jetons) et le texte d'une erreur.
- **Erreur de protocole JSON-RPC** : `-32602` (`invalid_params`) pour un appel mal formé (coordonnées invalides, `level_min` inconnu, commande logique inconnue), `-32603` pour une panne d'infrastructure.
- **Captures** : un bloc image, puis un bloc texte JSON de métadonnées.
- **Un seul outil à la fois** : les appels sont sérialisés dans l'ordre d'arrivée ; les attentes (`wait_*`) ne tiennent pas ce verrou pendant qu'elles patientent.
- **Annotations** : chaque outil déclare ses quatre indices MCP (lecture seule, destructif, idempotent, monde ouvert) pour que le client puisse classer le risque ([ADR-0027](adr/0027-annotations-des-outils-mcp-et-classification-des-risques.md)). Lecture seule : captures (sauf `screenshot_element` qui défile la page), `dom`, `state`, `find_element`, `assert_selector`, `assert_state`, `logs_*` (qui drainent un tampon), `wait_*`, `target_info`, `list_*`, `window_geometry`, `observe` ; `clipboard_get` est lecture seule **et monde ouvert** (il lit le vrai presse-papiers de l'utilisateur : le client doit demander confirmation, [ADR-0050](adr/0050-clipboard-get-monde-ouvert-et-limites-de-la-classification.md)). Destructifs : `eval_js`, `invoke_command`, `key_combo`, `navigate`, `upload_file`, `clipboard_set`, `assert_visual` (peut écrire la baseline).
- **Coordonnées** : pixels de la capture (en CDP, pixels CSS ; voir `screenshot`). Elles sont validées avant l'envoi : négatives, ou hors du viewport quand sa taille est connue, elles donnent `invalid_params`.
- **Options de coût** des captures (`screenshot`, `screenshot_region`, `screenshot_element`, `observe`) : `max_width` (entier ≥ 1, réduction proportionnelle, jamais d'agrandissement), `format` (`png` par défaut, ou `jpeg`), `quality` (JPEG, 1 à 100, défaut 80). Une valeur invalide est `invalid_params`, détectée avant la capture.

### Disponibilité par cible

`oui` : fonctionne ; `non` : résultat `unsupported` ; `SDK` : nécessite le SDK dans l'app (sinon `sdk_unavailable`) ; `KDE` : KDE Plasma Wayland.

| Outil ou famille | `cdp` | `sdk` | `kwin-eis` | `x11` |
|---|---|---|---|---|
| `screenshot`, `screenshot_region`, `assert_visual`, `observe` (capture) | oui | oui | oui | oui |
| `screenshot_element`, `find_element`, `assert_selector`, `eval_js`, `click_selector`, `click_text`, `collect_console_errors` | oui | oui | non | non |
| `wait_for_selector`, `wait_for_text`, `wait_condition` (`dom-selector`, `text-content`) | oui | oui | attend le timeout puis `found: false` | attend le timeout puis `found: false` |
| `wait_condition` (`state-key`, `log-message`) | SDK | SDK | SDK | SDK |
| `wait_condition` (`screenshot-stable`, `screenshot-region-stable`), `wait_screenshot_region`, `wait_target_alive` | oui | oui | oui | oui |
| `click`, `double_click`, `right_click`, `hover`, `drag_drop`, `type_text`, `press_key`, `key_combo`, `scroll` | oui | oui | oui | oui |
| `navigate` | oui | oui | non | non |
| `resize`, `handle_dialog`, `upload_file` | oui | non | non | non |
| `dom` | oui | oui (`outerHTML`) | non | oui, si l'app sert la route `ui.widgets` |
| `state`, `assert_state`, `logs_tail`, `logs_wait`, `wait_for_log`, `wait_ready`, `list_commands`, `invoke_command` | SDK | SDK | SDK | SDK |
| `list_windows` | oui (onglets) | non (liste vide) | KDE | oui |
| `window_geometry` | KDE | KDE | KDE | oui |
| `focus_window` | KDE, avec `--screencap-window` | KDE, avec `--screencap-window` | KDE, avec `--screencap-window` | oui |
| `clipboard_get`, `clipboard_set` | Wayland | Wayland | Wayland | non |
| `snapshot`, `act` (expérimentaux), section `elements` d'`observe` | non (à venir) | oui, pont JS (`auto` : CDP puis pompe SDK) | SDK, route `ui.widgets` | SDK, route `ui.widgets` |

`target_info` rend ces mêmes capacités en booléens, calculés au démarrage d'après la session réelle.

### Capture

#### `screenshot`

Capture le viewport de l'app. Paramètres : `max_width`, `format`, `quality` (tous optionnels). Retour : un bloc image, puis `{ captured_at, viewport: { width, height }, image: { format, width, height, source_width, source_height, scale, bytes } }` (`captured_at` en RFC 3339 UTC, `viewport` en pixels CSS). En CDP, l'image est à l'échelle CSS même sur un écran HiDPI : un pixel de l'image est un pixel de `click`. Sur les autres cibles, l'image est en pixels de la fenêtre capturée. Si `scale` ≠ 1, diviser une coordonnée lue sur l'image par `scale`.

#### `screenshot_region`

Capture puis recadre. Paramètres : `x`, `y`, `width`, `height` (entiers ≥ 0, pixels de la capture, région bornée aux dimensions de la capture) + options de coût. Retour : image, puis `{ image: { …, scale, origin: { x, y } } }` : coordonnée de capture = `origin` + coordonnée de l'image / `scale`. Seule façon d'inspecter un composant précis sans DOM.

#### `screenshot_element`

Capture un élément par sélecteur : le défile au centre (le défilement de la page change), puis recadre sur son rect. Paramètres : `selector` + options de coût. Retour : image, puis `{ image: { …, scale, origin } }` ; `isError { found: false, reason }` si l'élément est absent ou invisible. Exige JS (`cdp`, `sdk`).

#### `assert_visual`

Assertion visuelle (golden). Paramètres : `baseline_path` (obligatoire, **confiné** : relatif à `DEVEYE_BASELINE_DIR`, sinon au répertoire courant du serveur ; extension `.png`, sans `..` ni lien symbolique sortant ; un chemin absolu n'est accepté que dans cette racine, sinon `invalid_params` ; [ADR-0049](adr/0049-confinement-des-baselines-d-assert-visual.md)) ; `threshold` (ratio de pixels différents toléré, défaut `0.01`) ; `x`, `y`, `width`, `height` (région comparée, **les quatre ensemble**, sinon ignorés) ; `ignore_regions` (liste de `{ x, y, width, height }` exclus de la comparaison et du dénominateur ; un masque qui couvre toute l'image ne compare aucun pixel : le résultat est `ok: false` avec `reason: "fully_masked"`) ; `update_baseline` (`true` : remplace la baseline sans comparer ; refusé si le fichier existant n'est pas un PNG lisible).

- baseline absente : elle est créée, `{ ok: true, created: true, baseline_path }` ;
- `update_baseline: true` : `{ ok: true, created, updated: true, baseline_path }` ;
- comparaison : `{ ok, created: false, diff_ratio, threshold, same_dims, baseline_dims, current_dims, baseline_path, diff_pixels, compared_pixels, ignored_regions, diff_bbox? }` ; `isError` si `diff_ratio` > `threshold` ou si les dimensions diffèrent. À dimensions égales et en échec, une **image de différence** suit le JSON (pixels différents en rouge, zones ignorées teintées de bleu, largeur bornée à 1280 px, `diff_image.scale` donne l'échelle) et `diff_bbox` délimite la zone qui diverge.

### Interaction

Tous renvoient `{ ok: true }` sauf mention contraire.

#### `click`

Clic aux coordonnées `x`, `y`. `button` optionnel : `"left"` (défaut), `"right"`, `"middle"`, insensible à la casse.

#### `double_click`

Double-clic gauche en `x`, `y` : un vrai `dblclick` (`clickCount=2` en CDP), plus fiable que deux `click` successifs.

#### `right_click`

Clic droit en `x`, `y` (menu contextuel). Équivalent à `click` avec `button: "right"`.

#### `hover`

Déplace le pointeur en `x`, `y` sans presser (révèle ce qui n'apparaît qu'au survol).

#### `drag_drop`

Glisser-déposer de (`from_x`, `from_y`) vers (`to_x`, `to_y`). CDP : séquence souris native ; pont SDK : `PointerEvent` JS (selon le framework).

#### `click_selector`

Clique le premier élément qui correspond au sélecteur CSS `selector`, par de vrais événements souris (survol puis appui au centre, le focus clavier suit). Robuste aux déplacements et animations. Exige JS.

#### `click_text`

Clique le premier élément interactif (`button`, `a`, `label`, `summary`, rôles ARIA `button`/`tab`/`menuitem`/`menuitemradio`/`option`/`switch`/`checkbox`, ou attribut `data-deveye-click`) dont le texte, après `trim`, est **exactement** `text`. Exige JS.

#### `type_text`

Saisit `text` au focus courant. Sur les cibles webview (`cdp`, `sdk`), sans champ éditable au focus la saisie est refusée en `{ ok: false, reason: "interaction_failed", detail }` : cliquer d'abord sur le champ. Sur `kwin-eis` et `x11`, le texte est envoyé à la fenêtre qui a le focus sans pouvoir savoir si un champ le reçoit : `{ ok: true }` signifie « événements envoyés », pas « texte saisi ».

#### `press_key`

Presse une touche (`key`) : keydown puis keyup, sans modificateur. Noms W3C (`Enter`, `Tab`, `Escape`, `ArrowDown`, `F5`) en `cdp` et `sdk` ; noms XKB (`Return`, `Down`) en `kwin-eis` ; les noms communs passent partout.

#### `key_combo`

Combinaison de touches : `modifiers` (liste parmi `ctrl`/`control`, `shift`, `alt`, `meta`/`super`/`cmd`, insensibles à la casse) maintenus pendant la frappe de `key`, puis relâchés. Noms de touche W3C (`KeyZ` ou `z`) en `cdp`/`sdk`, XKB en `kwin-eis`.

#### `scroll`

Défile de `dx`, `dy` pixels (`dy > 0` vers le bas, `dx > 0` vers la droite, défauts 0), ancré en `x`, `y` (défaut : centre du viewport). CDP : molette qui vise le bon conteneur scrollable ; SDK : conteneur sous le point, sinon `window` ; `kwin-eis` : fenêtre au focus ; `x11` : un cran par tranche de 120 px.

#### `resize`

Émule un viewport de `width` × `height` pixels CSS (1 à 16384). Émulation du moteur : la fenêtre de l'OS garde sa taille, le facteur d'échelle de l'écran est conservé, l'émulation persiste jusqu'au prochain `resize` (dans la même session CDP : après une ré-attache de page, refaire `resize`, ADR-0054) ; `width` et `height` à 0 **ensemble** la retirent. CDP seulement.

#### `navigate`

Navigue vers la route logique `route` par le routeur du SDK (`window.__deveye_router__`). Indisponible en `kwin-eis` et `x11`.

#### `handle_dialog`

Répond à une boîte de dialogue JS (`alert`, `confirm`, `prompt`) : `accept` (obligatoire), `prompt_text` (pour `prompt`). CDP seulement : pendant un dialogue le moteur JS est suspendu, `eval_js` est impossible.

#### `upload_file`

Renseigne les fichiers d'un `input[type=file]` : `selector`, `paths` (chemins absolus). CDP seulement (`DOM.setFileInputFiles`).

#### `focus_window`

Donne le focus OS à une fenêtre : `title_pattern` (sous-chaîne du titre, insensible à la casse ; absent : la fenêtre `--screencap-window`). Retour `{ ok, title }`. Crucial en `kwin-eis` : KWin route les événements vers la fenêtre **active**, une notification peut donc détourner les clics sans erreur. KDE Wayland ou `x11`.

#### `clipboard_get`

Lit le texte du presse-papiers de la session (`wl-paste`). Retour `{ text }`. Wayland seulement. Il s'agit du **vrai** presse-papiers de l'utilisateur (mots de passe copiés, données d'autres applications), non masqué par le SDK : l'outil est annoté monde ouvert.

#### `clipboard_set`

Place `text` dans le presse-papiers (`wl-copy`) ; avec `key_combo` Ctrl+V, saisit du texte long ou Unicode. Wayland seulement.

### Inspection : DOM, état, fenêtres

#### `dom`

DOM courant (JSON), puis `{ captured_at }`. Au-delà de ~100 Ko sérialisés, `isError` explicite : préférer `find_element`, `assert_selector` ou `eval_js`. En `x11`, le « DOM » est la réponse de la route `ui.widgets` de l'app, sans conversion (format legacy conservé) mais **secrets masqués** : la valeur d'un widget secret est remplacée par `<secret>` ; absente, erreur d'inspection explicite.

##### Route `ui.widgets` : schéma `deveye.ui/1`

Pour une app sans DOM, la route `ui.widgets` publie une liste **plate** de widgets, chacun pointant vers son parent. Le contrat est typé dans `deveye-core` (module `ui`, [ADR-0062](adr/0062-schema-deveye-ui-1-de-la-route-ui-widgets.md)) et décrit par [`docs/schemas/ui-widgets-1.json`](schemas/ui-widgets-1.json) ; des fixtures valides, legacy et invalides sont dans `deveye-core/tests/fixtures/ui-1/`.

```json
{
  "schema": "deveye.ui/1",
  "generation": 42,
  "window": { "width": 800, "height": 600, "scale": 1.0 },
  "focus": "name",
  "widgets": [
    { "id": "root", "role": "window", "rect": [0, 0, 800, 600] },
    { "id": "name", "parent": "root", "role": "textbox", "label": "Nom", "value": "Alice",
      "rect": [40, 60, 240, 28], "actions": ["click", "type"], "state": { "focused": true } },
    { "id": "password", "parent": "root", "role": "password", "label": "Mot de passe",
      "rect": [40, 100, 240, 28], "state": { "secret": true } }
  ]
}
```

- `rect` = `[x, y, largeur, hauteur]` en pixels entiers, **dans le repère des gestes** de la cible (origine en haut à gauche de la fenêtre) : cliquer au centre du rect vise le widget. Une app en points logiques (egui) multiplie par son facteur d'échelle avant de publier ; les coordonnées fractionnaires sont arrondies vers la plus petite boîte entière qui contient le rectangle (`x`, `y` au plancher, bords droit et bas au plafond).
- `role` est une chaîne ouverte (vocabulaire recommandé : `window`, `panel`, `button`, `checkbox`, `radio`, `textbox`, `password`, `slider`, `knob`, `list`, `listitem`, `tab`, `menu`, `menuitem`, `label`, `image`, `scrollarea`) ; une valeur inconnue est conservée.
- Un widget `secret` (ou de rôle `password`) ne publie pas sa valeur ; le serveur masque en plus toute valeur reçue pour un tel widget, **y compris dans la sortie de `dom`** (changement de la 1.1 : avant, `dom` renvoyait la valeur brute de la route), et applique le masquage par clés (§2.4 du wire protocol) aux valeurs structurées, y compris pour `invoke_command {name: "ui.widgets"}` ; une réponse qui n'a pas la forme d'un arbre de widgets (liste racine ou champ `widgets`) est renvoyée intacte.
- Côté app Rust, `deveye_sdk::ui` publie l'arbre : `ui::builder(w, h)` construit, `ui::publish(tree)` remplace l'arbre courant, `ui::is_observed()` dit si un agent a lu la route dans les 2 dernières secondes (l'app ne construit son arbre que dans ce cas, coût nul sinon). La route est enregistrée au premier appel de `publish` ou de `is_observed` et annonce la capacité `ui.widgets/1` ; en release, tout est un no-op. Une lecture qui arrive quand personne n'observait attend jusqu'à 500 ms la publication suivante.
- Les lecteurs ignorent les champs inconnus : une évolution mineure ajoute des champs optionnels, une rupture publie `deveye.ui/2`, lu en plus de `/1`.

Format d'avant le schéma (sans champ `schema`), lu et converti par le lecteur tolérant : une liste ou `{ window?, widgets }`, où `kind` tient lieu de `role`, `text` de `label`, `rect` est un tableau ou un objet `{ x, y, width, height }`, et un widget sans `id` reçoit un `id` éphémère `role[index]` (marqué `id_ephemeral`, à ne pas réutiliser d'un arbre à l'autre).

<!-- ui-widgets-legacy-example -->
```json
[
  { "id": "valider", "kind": "button", "text": "Valider",
    "rect": { "x": 40, "y": 120, "width": 96, "height": 28 } }
]
```

#### `find_element`

Premier élément qui correspond à `selector` : `{ found, tag, text, rect: { x, y, width, height }, visible, disabled, attributes, value_prop }` (`value_prop` : propriété `.value` des champs, `null` sinon).

#### `assert_selector`

Paramètres : `selector` ; `expected_text` (texte attendu dans l'`innerText` du premier élément) ; `expected_count` (nombre exact d'éléments). Retour `{ ok, selector, actual_count, actual_text? }` ; `isError` si l'assertion échoue.

#### `eval_js`

Évalue `expression` dans la page et renvoie la valeur JSON ; les promesses sont attendues. Une exception levée revient en `{ ok: false, reason: "js_exception", message, stack? }`. Attention : le résultat n'est **pas** filtré par le masquage des secrets.

#### `state`

État applicatif publié par le SDK (JSON). `keys` (optionnel) restreint aux clés demandées (les absentes sont omises). Un second bloc `{ captured_at, signals, note? }` : `signals: 0` signifie « SDK joignable mais aucun signal publié », distinct de `sdk_unavailable`. Le résultat porte aussi `_meta` : `io.deveye/captured_at`, `io.deveye/entry_count` et `io.deveye/state_source` (`sdk` si un pont SDK est câblé, `aucun` en mode CDP seul, où le state est toujours vide). Les valeurs de clés jugées secrètes sont remplacées par `<secret>`.

#### `assert_state`

`expected` est un objet `{ clé: valeur }` : chaque clé doit exister dans l'état avec exactement cette valeur. Retour `{ ok, mismatches: [{ key, expected, actual }] }` ; `isError` si au moins une diverge.

#### `collect_console_errors`

Draine les messages console du webview depuis le dernier appel, tous niveaux : `[{ level, text, url?, line?, timestamp }]`. `levels` (optionnel) filtre parmi `log`, `info`, `warning`, `error`. Exige JS.

#### `list_windows`

Fenêtres ou onglets de la cible : `[{ id, title, url, kind }]`. CDP : cibles `page` ; `kwin-eis` : fenêtres du workspace via KWin Scripting (`url` vide, `kind` = classe de ressource) ; `x11` : fenêtres visibles titrées ou classées (`id` hexadécimal, `kind` = `x11-window`) ; pont SDK : liste vide.

#### `window_geometry`

Géométrie écran `{ x, y, width, height }` d'une fenêtre : `title` (sous-chaîne, insensible à la casse ; absent : la fenêtre `--screencap-window`). Convertit un rect DOM en coordonnées écran pour un clic OS. KDE Wayland ou `x11`.

#### `target_info`

Décrit la cible : `{ target, effective_backend, eyes, hands, eval_js, dom, state, console_errors, list_windows, window_geometry, focus_window, window_manager, clipboard, elements?, js_dependent_tools, server: { version, commit, sdk_protocol, mcp_protocol }, notes }`. `elements` (expérimental) n'est présent que s'il est vrai : `snapshot` et `act` peuvent désigner des éléments sur cette cible (`x11`, `kwin-eis`, `sdk`, `auto`) ; absent, ils répondent `unsupported`. `effective_backend` dit si le repli de `auto` est actif ; `js_dependent_tools` liste les outils qui répondent `unsupported` quand `eval_js` est faux ; `notes` rappelle les pièges de la cible.

### Logs

#### `logs_tail`

Draine les événements `tracing` accumulés depuis le dernier appel : `{ events, truncated, total }`. `limit` (défaut 200) borne `events` aux plus **récents** ; le drain est complet, `total` compte ce qui a été drainé, `truncated` vaut `true` si `total > limit`.

#### `logs_wait`

Long-poll : attend qu'un log arrive (de niveau ≥ `level_min` si fourni, parmi `trace`, `debug`, `info`, `warn`, `error`), jusqu'à `timeout_ms` (défaut 5000, plafond 30000). Retour `{ events, timed_out, truncated, total }` : `events` contient **tous** les logs drainés pendant l'attente. Si un sondage échoue après que des logs ont été drainés, ils sont renvoyés avec un champ `poll_error` (message) au lieu d'être perdus ; sans log collecté, l'erreur est renvoyée telle quelle. `limit` comme `logs_tail`. `level_min` inconnu : `invalid_params`.

### Attentes

Aucun `sleep` arbitraire : tous bornés par `timeout_ms`.

#### `wait_for_selector`

Attend un sélecteur CSS : `selector` ; `visible` (rect non vide) ; `negate` (attend la **disparition** : spinner, toast, modale) ; `timeout_ms` (défaut 5000, plafond 30000). Retour `{ found, elapsed_ms }` ; `isError` si non satisfait (avec `negate`, `found: true` signifie « disparu »).

#### `wait_for_text`

Attend un texte : `text` ; `selector` (absent : tout le document) ; `visible` ; `timeout_ms` (défaut 5000, plafond 30000). Retour `{ found, elapsed_ms }`, `isError` si non satisfait. Jamais satisfait sans DOM.

#### `wait_condition`

Attend un prédicat JSON : `predicate`, `timeout_ms` (défaut 5000, plafond 60000). Retour `{ satisfied, elapsed_ms, detail }` ; `isError` si non satisfait. Prédicats (`kind`) :

| `kind` | Champs |
|---|---|
| `dom-selector` | `selector`, `visible?`, `negate?` |
| `state-key` | `key`, `expected` (valeur JSON) |
| `screenshot-stable` | `threshold_ms` |
| `screenshot-region-stable` | `x`, `y`, `width`, `height`, `threshold_ms` |
| `log-message` | `contains`, `level_min?` |
| `target-alive` | aucun |
| `text-content` | `text`, `selector?`, `visible?` |

#### `wait_for_log`

Attend un log dont le message contient `contains` (et de niveau ≥ `level_min`, optionnel) ; `timeout_ms` (défaut 5000, plafond 60000). Retour `{ satisfied, elapsed_ms, detail }`, `isError` au délai. Les logs non concernés sont **consommés** (utiliser `logs_wait` pour les récupérer).

#### `wait_screenshot_region`

Attend qu'une région se stabilise (deux captures identiques à `threshold_ms` d'intervalle) : `x`, `y`, `width`, `height` ; `threshold_ms` (défaut 300, plafond 2000) ; `timeout_ms` (défaut 5000, plafond 60000). Ignore une horloge ou un curseur ailleurs. Retour `{ satisfied, elapsed_ms, detail }`.

#### `wait_target_alive`

Attend que la cible redevienne joignable après un crash ou un redémarrage (sonde `eval_js` ; sans JS, retombe sur la capture). `timeout_ms` (défaut 5000, plafond 60000). Retour `{ satisfied, elapsed_ms, detail }`.

#### `wait_ready`

Attend que l'app publie un état non vide (poll toutes les 200 ms) : `timeout_ms` (**obligatoire**, plafond 60000). Retour `{ ready: true, waited_ms }` ; `isError { ready: false, waited_ms }` au délai. À appeler juste après le lancement. Les erreurs de connexion y sont absorbées : « SDK pas encore à l'écoute » est l'état normal du démarrage.

### Commandes logiques

#### `list_commands`

Commandes déclarées par l'app via `deveye_sdk::expose_route` : `[{ name, description }]`, triées par nom ; vide si aucune.

#### `invoke_command`

Invoque la commande `name` avec `args` (JSON ; un objet ou tableau passé en chaîne JSON est relu comme tel). Retour : la valeur produite par le handler. Commande inconnue : `invalid_params`. Plus robuste qu'une cascade de clics pour mettre l'app dans un état donné.

### Composite

#### `observe`

Observe l'app en **un appel** (remplace quatre ou cinq). Paramètres (tous optionnels) : `include`, `exclude` (sections parmi `screenshot`, `dom`, `state`, `console`, `logs` ; `include` d'abord, `exclude` ensuite ; la section expérimentale `elements`, arbre compact de `snapshot` avec ses références et borné par `max_dom_nodes`, n'est produite que si `include` la nomme) ; `max_dom_nodes` (défaut 60, plafond 500) ; `max_dom_depth` (défaut 6, plafond 30) ; `max_events` (défaut 20, plafond 200, pour `console` et `logs`) ; `max_width` (**1024 par défaut**), `format`, `quality`. Retour : l'image (si capturée), puis `{ sections: { screenshot, dom, state, console, logs }, skipped }` ; chaque section porte `available: true` et ses données, ou `available: false` avec `reason` et `detail` sans faire échouer les autres. **Draine** les tampons de logs et de console (seuls les messages `error` et `warning` sont restitués, les autres sont comptés dans `ignored_lower_levels`). L'état et les champs de logs passent par le masquage des secrets du SDK.

### Désigner un élément par référence (expérimental)

> **Expérimental, hors promesse semver** ([ADR-0069](adr/0069-surface-experimentale-par-features-unstable.md), [ADR-0073](adr/0073-widgets-natifs-snapshot-act-relocalisation-et-candidats.md)). Leur description commence par « [expérimental] » ; les noms, paramètres et formes de réponse de ces deux outils peuvent changer dans une version mineure. Ils ne comptent pas dans les 44 outils stables.

Un agent désigne d'ordinaire un élément par sélecteur CSS, par texte ou par coordonnées. `snapshot` rend un arbre compact d'éléments, chacun avec une **référence** ; `act` agit dessus. Sur une cible sans DOM (egui, GTK, Godot, Uno), c'est la seule désignation qui ne passe pas par des pixels.

- **Références** : `w:<id>` pour les widgets natifs (cibles `x11` et `kwin-eis`) : l'`id` que l'app donne à son widget dans l'arbre `deveye.ui/1` de la route `ui.widgets`. Il est stable d'un instantané à l'autre ; il l'est **d'une relance à l'autre seulement si l'app fournit des identifiants déterministes**. Un identifiant dupliqué ou synthétisé (format legacy sans `id`) est signalé `[ephemeral]` par `snapshot` et par `deveye-server doctor`. Les pages web (cibles `sdk` et `auto`) portent des références `e<n>`, calculées par un script JavaScript que le serveur évalue par la pompe `document::eval` du SDK (ou par CDP en `auto` tant qu'il répond) : le **chemin réel sous Linux**, où le webview WebKitGTK n'a pas de CDP. Une référence `e<n>` reste la même d'un instantané à l'autre tant que le nœud vit (même après un déplacement dans le DOM), n'est **jamais réutilisée** et devient périmée au rechargement de la page. Le script est plus fruste que l'arbre d'accessibilité d'un navigateur : rôle implicite par une table minimale (`button`, `link`, `textbox`, `checkbox`, `heading`, `list`…, plus `clickable` pour une zone au curseur « main » ou au `tabindex`), nom accessible simplifié (`aria-labelledby`, `aria-label`, `<label>`, `alt`, contenu, `title`, `placeholder`), shadow DOM ouvert traversé, **iframes non parcourues**, jamais la valeur d'un champ `password` ni d'un champ d'autocomplétion `current-password` ou `new-password`. Les clics passent par `click` (événements JS synthétiques, `isTrusted` faux) ; `--target cdp` pur répond encore `unsupported` (l'arbre d'accessibilité CDP viendra après).
- **Pas de relocalisation silencieuse** : `act` relit l'arbre au moment de l'action et **remesure** l'élément ; le rect d'un ancien instantané ne sert à rien. Un `id` qui a disparu ne se rabat pas sur un voisin : l'erreur propose des `candidates` (même rôle, même nom) et laisse choisir.
- **Aucun secret** : la valeur d'un champ secret (`password`, `state.secret`) n'est jamais dans un instantané.

#### `snapshot`

Décrit les éléments de la cible. Paramètres (tous optionnels) : `scope` (référence d'un sous-arbre ; sur les widgets, une référence `w:<id>`, pas un sélecteur ; sur une page web, une référence `e<n>` ou un sélecteur CSS), `max_depth`, `max_nodes` (défaut 400, plafond 2000), `interactive_only` (défaut `true` : éléments interactifs et conteneurs utiles, les conteneurs anonymes sont aplatis), `viewport_only` (défaut `false`), `format` (`text` par défaut, ou `json`).

Le **texte** est un arbre indenté, une ligne par élément, orienté jetons :

```text
# widgets : 4 élément(s), génération 0
- window "Fixture" [ref=w:root]
  - textbox "Nom" [ref=w:name] = "alice"
  - button "Enregistrer" [ref=w:save]
  - button "Hors vue" [ref=w:off] [offscreen]
focus: w:name
```

Marqueurs : `[disabled]`, `[checked]` ou `[unchecked]`, `[expanded]` ou `[collapsed]`, `[selected]`, `[focused]`, `[offscreen]`, `[scrollable]`, `[ephemeral]`, puis ` = "valeur"` (tronquée à 80 caractères). `focus:` désigne l'élément qui a le focus clavier. L'en-tête dit `TRONQUÉ` quand `max_nodes` ou `max_depth` a coupé l'arbre : cibler un `scope`. Avec `format: "json"`, la réponse porte le même contenu avec les rects et `generation` (texte et `structuredContent` égaux) ; les rects sont **indicatifs**. Sans arbre d'éléments sur la cible : `isError` `{ ok: false, reason: "unsupported", hint }` ; le `hint` dit quoi faire (exposer `ui.widgets` avec `deveye_sdk::ui::publish`, ou `screenshot` puis `click` aux coordonnées).

**Taille mesurée** (test de bout en bout sous Xvfb, route `ui.widgets` factice) : 4 widgets, 200 octets de texte contre 416 de `dom` ; environ 55 widgets (liste de 50 lignes), 2 531 octets contre 6 377 pour `dom` (39 %). Mesure consignée, sans seuil : `format: "json"` coûte environ le double du texte. **Pont JS** (test dans Chromium headless, page de 56 éléments dont une liste de 40 lignes au balisage verbeux) : environ 1,6 Ko de texte compact, 5,3 Ko en JSON, contre 18,8 Ko de DOM brut (`outerHTML`), soit 9 % et 28 %.

#### `act`

Agit sur l'élément `ref`. `action` : `click` (`button` optionnel), `double_click`, `right_click`, `hover`, `type` (clique pour donner le focus puis saisit `text`), `scroll_into_view` (vérifie que l'élément est visible), `scroll` (molette `dx`/`dy` ancrée sur l'élément). Un paramètre sans sens pour l'action (`text` hors `type`, `dx` ou `dy` hors `scroll`, `button` hors `click`) est refusé en `invalid_params` avant tout accès à la cible. Succès : `{ ok: true, action, ref, x, y }` (centre remesuré cliqué, en pixels). Les gestes sont ceux de `click`, `double_click`, `hover`, `type_text` et `scroll`, avec les mêmes cibles et les mêmes limites (`{ ok: true }` signifie « événements envoyés »).

Échecs sémantiques (`isError`, `{ ok: false, reason, ref, detail, hint?, candidates?, by? }`), tous **sans aucun événement émis** :

| `reason` | Cas |
|---|---|
| `stale_ref` | l'élément a disparu depuis l'instantané ; `hint` : relancer `snapshot` ; `candidates` : éléments de même rôle et de même nom |
| `unknown_ref` | référence jamais émise, ou forme inconnue (`e<n>` sur une cible à widgets, `w:<id>` sur une page web) |
| `ambiguous_ref` | plusieurs widgets portent le même `id` : agir serait un pari ; corriger l'app (`doctor` le signale) |
| `offscreen` | élément hors de la zone visible, sans surface, ou centre remesuré hors viewport (sur une page web, `act` amène d'abord l'élément au centre par `scrollIntoView`) |
| `disabled` | élément désactivé (un `hover` ou un `scroll` reste permis) |
| `obscured` | un autre élément recouvre le centre (`by` : sa référence). Produit sur les pages web (`elementFromPoint` au centre remesuré) ; pas pour les widgets natifs, dont l'arbre ne dit pas qui est dessiné au-dessus |
| `unsupported` | la cible n'a pas d'arbre d'éléments (`hint`) |

---

## Comportement par cible

### `cdp` et `auto`

CDP est une session **navigateur** persistante : à chaque rechargement à chaud de la page, l'ancienne cible disparaît et la suivante qui correspond au filtre est rattachée automatiquement ; aucune reconnexion du MCP. Les clics sont de vrais événements natifs (`isTrusted`). `list_windows` reste CDP même en `auto` : il échoue quand le CDP est tombé.

### `sdk`

Le pont natif exige le SDK dans l'app et une **pompe d'évaluation** (`take_eval_requests`, [integration.md](integration.md)). Les événements sont synthétiques (focus compris) : ils ne remplacent pas de vrais événements système. `resize`, `upload_file` et `handle_dialog` répondent `unsupported`.

### `kwin-eis`

Rendu natif sans DOM ni JS. Prérequis : session KDE Plasma Wayland ; `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1` dans l'environnement de session KWin, puis rouvrir la session ; la connexion D-Bus qui initie EIS reste ouverte pendant toute la session (DevEye la maintient). Piège : EIS envoie les événements à la fenêtre **active** ; appeler `focus_window` avant une séquence (et lancer avec `--screencap-window`). `window_geometry` donne l'origine écran d'une fenêtre.

### `x11`

Serveur X **privé et explicite**. La garde d'isolation refuse, sans option pour la lever : un display non local, un numéro inférieur à 100 (donc le `Xvfb :99` habituel des CI : prendre `:100` ou plus), le display de la session courante, un socket appartenant à un autre uid, et tout serveur annonçant `XWAYLAND`.

```bash
Xvfb :250 -nolisten tcp -screen 0 1400x1000x24 &
DISPLAY=:250 ./mon-app &       # l'app tourne SUR le serveur privé
deveye-server doctor --target x11 --display :250
```

Pas de JS : `eval_js`, `find_element`, `click_selector`, `click_text` répondent `unsupported`. `wait_for_selector` et `wait_for_text` n'ont pas de réponse `unsupported` immédiate : sans DOM, ils attendent le `timeout_ms` complet puis répondent `found: false` (ne pas en conclure que l'élément est absent) ; utiliser `wait_condition` avec un prédicat sans DOM. Le « DOM » est la réponse de la route `ui.widgets` de l'app (rects en coordonnées de fenêtre : cliquer au centre avec `click`). Le clavier est celui du serveur X ; un caractère absent est injecté par remappage temporaire. Molette : un cran par tranche de 120 px. Presse-papiers indisponible. Attentes utiles : `wait_condition` avec `state-key`, `screenshot-stable`, `screenshot-region-stable`, `log-message` et `target-alive` (cinq prédicats sur sept ; `dom-selector` et `text-content` exigent un DOM).

#### Limites de la cible x11

La capture est un `GetImage` de la **racine** du serveur X, rognée au rectangle de la fenêtre (le drawable de la fenêtre peut rendre du noir pour un enfant GL). Trois conséquences :

1. **Occultation** : toute fenêtre qui en recouvre une autre apparaît dans l'image à la place de ce qu'elle cache ; la partie hors écran est noire. Ne pas laisser de fenêtre (dialogue, popup d'une autre app) devant la fenêtre à observer.
2. **Pas de mise à l'échelle HiDPI** : l'image est en pixels du serveur X et les coordonnées des gestes sont les mêmes pixels. Aucun facteur à appliquer, mais aucun écran à forte densité émulé non plus.
3. **Sans compositeur ni gestionnaire de fenêtres** : pas de décorations, d'ombres ni de transparence composée ; l'image est exactement ce que contient le serveur.

---

## Garde-fous pour l'agent

- **Focus** : `type_text` et `press_key` agissent au focus. En `cdp` et `sdk`, cliquer le champ d'abord (`click_selector`) ; le clavier synthétique part de `document.activeElement`. En `kwin-eis`, `focus_window` avant toute séquence.
- **Fenêtre masquée ou minimisée** : un webview masqué (`visibilityState: hidden`) bride les minuteries et les animations, donc des attentes qui ne se satisfont jamais ; vérifier par `screenshot` que la fenêtre est visible avant de conclure à une panne.
- **Conteneur scrollable** : `scroll` ancré en (`x`, `y`) vise le conteneur sous le point ; pour une liste interne, ancrer **dans** la liste, pas au centre de la fenêtre.
- **Coûts** : préférer `observe`, `screenshot_region`, `max_width` et `format: "jpeg"` à des captures pleines ; `find_element` et `assert_selector` à `dom`.
- **Attendre, ne pas dormir** : `wait_for_selector`, `wait_for_text`, `wait_ready`, `wait_target_alive` plutôt qu'un délai fixe.
- **Secrets** : `eval_js` et les valeurs de retour de `invoke_command` ne sont pas masqués ; la route `ui.widgets` ne publie pas la valeur des widgets secrets et `dom` la masque en seconde ligne ([wire-protocol.md](wire-protocol.md) §2.4 et §3.7).

---

## Variables d'environnement

| Variable | Où | Rôle |
|---|---|---|
| `DEVEYE_PORT` | app cible (SDK) | port du SDK, entier de 1 à 65535 ; toute autre valeur retombe sur `31337` |
| `DEVEYE_LOG_LEVEL` | app cible (SDK) | niveau minimal capturé par la couche `tracing` du SDK, `trace` à `error` (défaut `info`) |
| `RUST_LOG` | `deveye-server` | filtre des traces du serveur, sur stderr (défaut `info`) |
| `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS` | environnement de session KWin | `1` pour autoriser la capture par `ScreenShot2` (dev-only) |

---

## Dépannage

| Symptôme | Cause et remède |
|---|---|
| `deveye` absent ou `disabled` dans `/mcp` | vérifier le chemin de `command` dans `.mcp.json` ; approuver le serveur dans Claude Code |
| « aucun mode : ajouter --serve-mcp » | le binaire n'a pas d'autre mode : ajouter `--serve-mcp` (ou utiliser `doctor`) |
| « … n'est accepté qu'avec --target … » | une option sans effet sur la cible est refusée ([ADR-0031](adr/0031-validation-des-flags-selon-la-cible.md)) ; retirer l'option ou changer de cible |
| CDP : « timeout en attente d'une page CDP attachable » | l'erreur liste les pages vues ; ajuster `--cdp-target-filter-url` ou `--cdp-target-filter-title`, ou `--cdp-accept-blank` |
| `sdk_unavailable`, erreurs réseau vers le SDK | l'app tourne-t-elle, en build debug, avec `deveye_sdk::init` ? Même port que `--sdk-url` ? Utiliser `127.0.0.1`, pas `localhost` |
| Le SDK « reste inactif » (log `impossible de lier 127.0.0.1:…`) | port déjà pris par une autre instance ou une autre app : changer `DEVEYE_PORT` et `--sdk-url` |
| `eval_js` : « aucune pompe d'évaluation enregistrée » (`503`) | l'app n'appelle pas `take_eval_requests` ; `504` : la pompe n'a pas répondu en 10 s ; `500` : elle a abandonné la requête |
| `kwin-eis` : la capture échoue (`NoAuthorized`) | poser `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1` dans l'environnement de session, rouvrir la session ; `doctor` le vérifie |
| `kwin-eis` : les clics n'ont aucun effet ou touchent une autre fenêtre | une notification a pris le focus : `focus_window`, avec `--screencap-window` au démarrage |
| `x11` : « display refusé » | choisir un display privé ≥ 100 (`Xvfb :250 …`), jamais `$DISPLAY` ni un serveur XWayland |
| `x11` : image noire ou fenêtre recouverte | voir les [limites de la cible x11](#limites-de-la-cible-x11) |
| Toute attente expire alors que l'app « marche » | fenêtre masquée ou minimisée : voir les garde-fous |
| La compilation s'arrête sur `compile_error!` | `deveye-server` ne compile que sous Linux ([platforms.md](platforms.md)) |
| Une anomalie reste inexpliquée | `deveye-server doctor --target <cible>` d'abord ; puis `RUST_LOG=debug` sur stderr du serveur |
