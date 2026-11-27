# Guides par type d'app

Chaque guide donne le chemin le plus court pour qu'un agent voie et pilote un type d'app, avec un `.mcp.json` prêt à copier et les pièges propres au type. Tous supposent `deveye-server` installé ([README](../../README.md#installation)) et un client MCP. La référence complète des options et des outils est [usage.md](../usage.md) ; la matrice des environnements, [platforms.md](../platforms.md).

| Mon app est… | Guide | Cible |
|---|---|---|
| une app **Dioxus desktop** sous Linux | [dioxus-desktop-linux.md](dioxus-desktop-linux.md) | `sdk` (ou `auto`) |
| une page ou une app dans une webview ou un navigateur **Chromium** | [webview-chromium-cdp.md](webview-chromium-cdp.md) | `cdp` (ou `auto`) |
| une app **X11 native** (egui, GTK, Qt…) | [app-x11-xvfb-prive.md](app-x11-xvfb-prive.md) | `x11` |
| une app **native Wayland** sous KDE Plasma | [app-wayland-kde.md](app-wayland-kde.md) | `kwin-eis` |
| écrite dans **un autre langage** et qui doit parler le contrat du SDK | [implementation-tierce.md](implementation-tierce.md) | n'importe laquelle |

Pour intégrer le SDK Rust dans l'app : [integration.md](../integration.md).

## Réflexes communs

1. **`doctor` d'abord** : `deveye-server doctor --target <cible>` dit ce qui manque, en lecture seule.
2. **`target_info` en premier appel** : il dit quels outils répondront sur la cible.
3. **Lancer l'app avant le premier outil** : l'app écoute déjà (CDP, SDK, serveur X) quand l'agent appelle.
4. **Garde-fous** : focus, fenêtre masquée, conteneur scrollable : voir [usage.md](../usage.md#garde-fous-pour-lagent).
