# Webview ou navigateur Chromium (CDP)

Toute webview Chromium (une app Dioxus desktop sur WebView2, un navigateur Chrome, Chromium ou Brave, une tête WASM servie en local) peut être pilotée par le **Chrome DevTools Protocol**. C'est la cible la plus complète : DOM, JavaScript, sélecteurs, événements navigateur réels, `resize`, `upload_file`, `handle_dialog`.

## 1. Ouvrir le port de débogage

- **Navigateur** : le lancer avec `--remote-debugging-port=9222` (par exemple `chromium --remote-debugging-port=9222 http://localhost:5000`). Sans profil dédié, certains navigateurs ignorent le flag si une instance tourne déjà : ajouter `--user-data-dir=/tmp/profil-dev`.
- **Webview d'une app** : lui passer l'argument de lancement équivalent. Pour une app Dioxus desktop sur WebView2, `with_windows_browser_args("--remote-debugging-port=9222")` dans la configuration de la fenêtre, en debug seulement.

Vérifier : `curl http://localhost:9222/json` liste les pages.

## 2. Brancher DevEye

```json
{
  "mcpServers": {
    "deveye": {
      "type": "stdio",
      "command": "deveye-server",
      "args": ["--target", "cdp", "--cdp-url", "http://localhost:9222",
               "--cdp-target-filter-url", "localhost:5000", "--serve-mcp"]
    }
  }
}
```

`--cdp-target-filter-url` (et `--cdp-target-filter-title`, relation OU) vise une page précise quand plusieurs sont ouvertes. **Sans filtre**, la cible est la première page qui n'est pas `about:blank` : le cas « une seule page utile » ne demande aucune option. Si rien ne correspond, l'erreur liste les pages vues et le flag à ajuster ([ADR-0014](../adr/0014-filtre-de-cible-cdp-permissif-par-defaut.md)). `--cdp-accept-blank` accepte aussi `about:blank`.

Pour l'**état, les logs et les commandes**, l'app peut en plus embarquer le SDK ([integration.md](../integration.md)) ; sans lui, ces outils répondent `sdk_unavailable` mais tout le reste fonctionne.

Vérifier : `deveye-server doctor --target cdp`.

## 3. Comportement à connaître

- **Hot-reload.** La session CDP est celle du **navigateur** : quand la page est recréée, DevEye se rattache tout seul à la suivante qui correspond au filtre ; aucune reconnexion du MCP. Un navigateur fermé puis relancé est repris à l'appel suivant (connexion paresseuse).
- **Captures à l'échelle CSS.** Même sur un écran HiDPI, un pixel de l'image est un pixel CSS, l'unité de `click` : aucun facteur à appliquer.
- **Événements réels.** Les clics et touches sont des événements navigateur natifs (`isTrusted`), focus clavier compris.
- **`list_windows`** liste les onglets (`Target.getTargets`).

## 4. Garde-fous

- **Focus.** Cliquer le champ (`click_selector`) avant `type_text` ; sans champ éditable au focus, la saisie est refusée en `interaction_failed`.
- **Fenêtre masquée.** Un onglet en arrière-plan ou une fenêtre minimisée bride minuteries et animations : `wait_*` n'aboutit plus. Garder la page visible.
- **Conteneur scrollable.** `scroll` ancré en (`x`, `y`) vise le bon conteneur par la molette ; ancrer dans le conteneur de la liste.
- **Dialogues JS.** Pendant un `alert`, `confirm` ou `prompt`, le moteur JS est suspendu : `eval_js` est impossible, seul `handle_dialog` répond.
- **Coût.** `observe`, `screenshot_region`, `max_width` et `format: "jpeg"` avant une capture pleine ; `find_element` avant `dom`.
- **`eval_js` n'est pas masqué** par le filtre de secrets du SDK.
