# App X11 native dans un serveur X privé

Pour une app X11 sans webview ni SDK (egui, GTK, Qt, imgui), la cible `x11` lui donne des yeux (`GetImage`) et des mains (XTEST, de vrais événements) **sur un serveur X privé** que vous lancez : elle ne touche jamais votre session ([ADR-0044](../adr/0044-cible-x11-isolee-par-construction.md)).

## 1. Lancer le serveur privé et l'app

```bash
Xvfb :250 -nolisten tcp -screen 0 1400x1000x24 &
DISPLAY=:250 DEVEYE_PORT=31400 ./mon-app &   # l'app tourne SUR le serveur privé ; son SDK écoute 31400
```

`Xvfb` : paquet `xorg-x11-server-Xvfb` (Fedora) ou `xvfb` (Debian et Ubuntu). **Le display doit être ≥ 100** : `:99`, habituel des CI, est refusé, prendre `:100` ou plus. Aucune bibliothèque système de plus n'est nécessaire côté DevEye (client X en Rust pur).

## 2. Brancher DevEye

Le `--sdk-url` ci-dessous vise le port **31400** : l'app doit donc être lancée avec `DEVEYE_PORT=31400` (sinon son SDK écoute le port par défaut 31337 et `state`, `logs_*`, `invoke_command` répondent `sdk_unavailable`).

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

`--display` est **obligatoire** et sans repli sur `$DISPLAY`. `--window` (alias de `--screencap-window`) désigne la fenêtre pilotée ; `--x11-class` départage deux fenêtres de même titre. Vérifier : `deveye-server doctor --target x11 --display :250`.

La garde refuse, sans option pour la lever : un display non local, un numéro < 100, le display de la session courante, un socket d'un autre utilisateur, et tout serveur annonçant `XWAYLAND`.

## 3. État, logs, commandes et « DOM »

`--sdk-url` désigne le SDK que l'app embarque, s'il y en a un ([integration.md](../integration.md)) : il fournit `state`, `logs_*`, `list_commands`, `invoke_command`.

Pas de JavaScript : `eval_js`, `find_element`, `click_selector`, `click_text` répondent `unsupported`. `wait_for_selector` et `wait_for_text` n'ont pas de réponse `unsupported` immédiate : sans DOM, ils attendent le `timeout_ms` complet puis répondent `found: false` (ne pas en conclure que l'élément est absent) ; utiliser `wait_condition` avec un prédicat sans DOM (`state-key`, `screenshot-stable`, `screenshot-region-stable`, `log-message`, `target-alive`). Le « DOM » est la **réponse de la route `ui.widgets`** de l'app, renvoyée sans interprétation par l'outil `dom` :

Pour une app Rust, le helper `deveye_sdk::ui` publie l'arbre `deveye.ui/1` : à chaque frame, `if deveye_sdk::ui::is_observed() { deveye_sdk::ui::publish(tree) }`, où `tree` vient de `deveye_sdk::ui::builder(largeur, hauteur)` (rects en pixels de capture, c'est-à-dire points logiques × `pixels_per_point` sous egui). Pas de widget secret : sa valeur n'est jamais publiée. Une app non Rust, ou qui préfère un handler maison, expose la route à la main :

```rust
#[cfg(all(debug_assertions, feature = "deveye"))]
deveye_sdk::expose_route("ui.widgets", "Arbre des widgets", |_payload| {
    serde_json::json!([
        { "id": "valider", "kind": "button", "text": "Valider",
          "rect": { "x": 40, "y": 120, "width": 96, "height": 28 } }
    ])
});
```

Le contrat recommandé est le schéma `deveye.ui/1` ([usage.md](../usage.md), section « Route `ui.widgets` ») ; l'exemple ci-dessus est le format d'avant le schéma, toujours lu par le serveur. Il est conseillé d'y mettre des rects en coordonnées de fenêtre : l'agent clique alors au centre avec `click`. Sans cette route, `dom` répond une erreur d'inspection explicite. Au-delà de ~100 Ko, `dom` refuse l'arbre.

## 4. Limites de la capture

L'image est le `GetImage` de la racine du serveur X rognée à la fenêtre :

- **occultation** : une fenêtre qui en recouvre une autre apparaît dans l'image ; la partie hors écran est noire ;
- **pas de HiDPI** : l'image et les coordonnées sont en pixels du serveur ;
- **sans compositeur** ni gestionnaire de fenêtres : pas de décorations, d'ombres, de transparence composée.

Détail : [usage.md](../usage.md#limites-de-la-cible-x11).

## 5. Garde-fous

- **Focus.** Sans gestionnaire de fenêtres, `focus_window` (`SetInputFocus` et passage au premier plan) est le seul moyen de fixer le focus clavier : l'appeler avant `type_text`.
- **`type_text` ne sait pas si un champ le reçoit** : `{ ok: true }` veut dire « événements envoyés ». Vérifier par une capture.
- **Toolkits immédiats (egui, imgui).** Ils lisent les entrées par frame : DevEye place le pointeur environ 30 ms avant l'appui et interpole 16 déplacements pour un glisser ; ne pas enchaîner des gestes plus vite que le rendu.
- **Molette.** Un cran par tranche de 120 px de `dx`/`dy`.
- **Presse-papiers** : indisponible.
- **Fenêtre recouverte** : ne rien laisser devant la fenêtre observée.
- En CI : lancer `Xvfb` en début de job, le tuer en fin ; deux jobs simultanés = deux displays distincts.
