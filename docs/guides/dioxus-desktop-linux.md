# App Dioxus desktop sous Linux

Une app Dioxus desktop affiche son interface dans **WebKitGTK**, qui n'expose pas de CDP. DevEye la pilote par le **pont natif** du SDK : le SDK embarqué expose l'état, les logs, les commandes et une route d'évaluation JavaScript, et DevEye capture la fenêtre au niveau du système ([ADR-0043](../adr/0043-pont-natif-via-le-sdk-sans-cdp.md)).

## 1. Intégrer le SDK

Suivre [integration.md](../integration.md) : dépendance optionnelle `deveye-sdk` derrière une feature `deveye`, `init` et `install_tracing_layer` en debug seulement, `set_state`, `expose_route`, et **la pompe d'évaluation** (section 5), qui exécute les requêtes JavaScript dans la webview. Sans pompe, `eval_js`, `dom` et les clics répondent « aucune pompe d'évaluation enregistrée ».

## 2. Lancer l'app

```bash
dx serve --platform desktop --features deveye
```

Passer par `dx serve` : un `cargo run` nu n'a pas ses ressources `asset!()` et la page s'affiche sans CSS. Donner à la fenêtre un **titre** distinctif : il sert à la repérer pour la capture.

Sous KDE Plasma Wayland, rien d'autre n'est requis. Sous une autre session, la capture passe par `xcap` : lancer l'app avec `GDK_BACKEND=x11` (XWayland requis).

## 3. Brancher DevEye

`.mcp.json` à la racine du projet de l'app :

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

Sous KDE Plasma Wayland, ajouter au besoin `"--screencap-backend", "kwin"` et poser `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1` dans l'environnement de session ([usage.md](../usage.md#kwin-eis)). Vérifier : `deveye-server doctor --target sdk --screencap-window "Mon App"`.

`--target auto` convient aussi : il essaie CDP et retombe sur le pont si le CDP est injoignable (utile si la même app tourne parfois dans un navigateur).

## 4. Ce qui marche, ce qui ne marche pas

| Marche | Ne marche pas |
|---|---|
| `screenshot*`, `assert_visual`, `observe` | `resize`, `upload_file`, `handle_dialog` (CDP seulement) |
| `click*`, `hover`, `drag_drop`, `type_text`, `press_key`, `key_combo`, `scroll` (événements JS synthétiques) | tout ce qui exige de **vrais** événements système (menus natifs, dialogues du système) |
| `eval_js`, `dom` (`outerHTML`), `find_element`, `wait_for_*`, `collect_console_errors` | `list_windows` (liste vide : pas de vue inter-fenêtres) |
| `state`, `logs_*`, `invoke_command`, `navigate` | |

## 5. Garde-fous

- **Focus.** La saisie part de `document.activeElement` : `click_selector` sur le champ avant `type_text`. Sans champ éditable au focus, `type_text` répond `interaction_failed` plutôt qu'un faux succès.
- **Fenêtre masquée.** Une webview masquée ou minimisée gèle `requestAnimationFrame` et bride les minuteries : les attentes n'aboutissent plus. Vérifier par `screenshot` que la fenêtre est visible avant de conclure à une panne ; en CI, la lancer sur un serveur d'affichage virtuel.
- **Conteneur scrollable.** Le défilement d'une app vit souvent dans un conteneur interne, pas dans `window`. Ancrer `scroll` **dans** le conteneur (`x`, `y`) ; le pont privilégie le conteneur scrollable sous le point.
- **Attendre.** `wait_ready` juste après le lancement ; `wait_for_selector` ou `wait_for_text` après une action asynchrone.
- **Secrets.** `eval_js` et les retours de commandes ne sont pas masqués ([integration.md](../integration.md#6-secrets-et-limites)).
