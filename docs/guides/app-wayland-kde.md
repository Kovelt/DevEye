# App native Wayland sous KDE Plasma

Pour une app sans webview ni SDK (rendu natif : Skia, Uno, Qt Quick, GTK 4…) sur une session **KDE Plasma Wayland**, DevEye passe par KWin : `ScreenShot2` pour les yeux, EIS (`org.kde.KWin.EIS.RemoteDesktop`) pour les mains, KWin Scripting pour les fenêtres ([ADR-0045](../adr/0045-yeux-et-mains-wayland-via-kwin.md)). Aucun XWayland, aucune modification de l'app.

## 1. Préparer la session

1. **Session KDE Plasma sous Wayland natif.**
2. Poser `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1` dans l'environnement de session de KWin, par exemple dans `~/.config/plasma-workspace/env/deveye.sh` (`export KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1`), puis **fermer et rouvrir la session**. C'est un réglage de développement : sans lui, la capture est refusée (`NoAuthorized`). Ne pas le poser sur une machine qui n'est pas de confiance.
3. Vérifier : `deveye-server doctor --target kwin-eis` contrôle la session, KWin, la variable et l'interface EIS.

Le test de bout en bout de cette cible se fait à la main sur une vraie session : voir [platforms.md](../platforms.md).

## 2. Brancher DevEye

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

`--screencap-window` : sous-chaîne du titre de la fenêtre, insensible à la casse. Il est **requis pour `focus_window`**, et sert de fenêtre par défaut à `window_geometry`. Sans lui, la capture est plein écran. L'état, les logs et les commandes viennent du SDK de l'app si elle en embarque un (`--sdk-url`, [integration.md](../integration.md)).

## 3. Ce qui marche

| Capacité | Outils |
|---|---|
| Yeux | `screenshot`, `screenshot_region`, `assert_visual`, `observe` (la seule vérification déterministe sans DOM est la comparaison de captures) |
| Mains | `click`, `double_click`, `right_click`, `hover`, `drag_drop`, `type_text`, `press_key`, `key_combo`, `scroll` : de vrais événements système, identiques à la souris et au clavier physiques |
| Fenêtres | `focus_window`, `window_geometry`, `list_windows` (toutes les fenêtres du workspace) |
| Presse-papiers | `clipboard_get`, `clipboard_set` (`wl-clipboard`) |
| Avec le SDK de l'app | `state`, `logs_*`, `invoke_command`, `list_commands` |

`dom`, `eval_js`, `find_element`, `click_selector`, `click_text`, `navigate`, `resize`, `upload_file`, `handle_dialog` répondent `{ reason: "unsupported" }` : un rendu natif n'a ni DOM ni JavaScript. `wait_for_selector` et `wait_for_text` attendent alors le `timeout_ms` complet puis répondent `found: false` : utiliser `wait_condition` avec un prédicat sans DOM (`state-key`, `screenshot-stable`, `screenshot-region-stable`, `log-message`, `target-alive`).

## 4. Garde-fous

- **Focus.** EIS envoie les événements à la fenêtre **active**. Une notification Plasma ou un dialogue système peut la remplacer et détourner les clics **sans erreur**. Appeler `focus_window` avant chaque séquence d'interaction.
- **Coordonnées.** Les gestes sont relatifs au viewport de la fenêtre capturée ; `window_geometry` donne l'origine écran de la fenêtre si l'agent doit raisonner en coordonnées absolues.
- **Touches.** Noms XKB (`Return`, `Down`) ; les noms communs (`Tab`, `Escape`, `F5`, `space`) passent partout. `key_combo` déclenche les raccourcis du toolkit (Ctrl+S, Ctrl+Z).
- **Saisie.** `type_text` envoie des événements sans savoir si un champ les reçoit : vérifier par `screenshot_region`.
- **Connexion EIS.** La connexion D-Bus qui appelle `connectToEIS` doit rester ouverte pendant toute la session ; DevEye la maintient, ne pas la couper.
- **Liste des fenêtres.** `list_windows` rend visibles les fenêtres qui peuvent voler le focus ; l'utiliser quand les clics « ne font rien ».
- **Fenêtre masquée.** La capture d'une fenêtre minimisée ou sur un autre bureau n'est pas garantie : la ramener d'abord (`focus_window`).
