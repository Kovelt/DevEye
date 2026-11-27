# ADR-0045 — Yeux et mains Wayland via KWin : ScreenShot2 et EIS

- **Statut** : Accepté
- **Date** : 2026-06-25
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0009 (exception `unsafe` de l'adapter EIS), ADR-0037 (GNOME hors périmètre), ADR-0044 (x11 privé), ADR-0041 (architecture)

## Contexte et problème

Sous Wayland, une application ne peut ni capturer une autre fenêtre ni lui envoyer des entrées : le protocole l'interdit, par conception. Les apps sans webview ni SDK (rendu natif, par exemple Skia) n'ont ni CDP ni pont. Il faut des yeux et des mains au niveau du compositeur, sans passer par XWayland.

## Facteurs de décision

- Wayland natif, sans XWayland ;
- pas d'app cible à modifier ;
- limiter l'`unsafe` et les dépendances système ;
- KDE Plasma est l'environnement de développement du mainteneur, avec des interfaces D-Bus documentées.

## Options envisagées

1. **xcap seul** : capture correcte sous X11 ou XWayland, mais pas de mains, et pas de capture Wayland native.
2. **Portails XDG (ScreenCast, RemoteDesktop)** : portables entre compositeurs, mais l'autorisation passe par une boîte de dialogue, incompatible avec un pilotage non interactif.
3. **KWin directement** : `org.kde.KWin.ScreenShot2` pour les yeux, `org.kde.KWin.EIS.RemoteDesktop` pour les mains, `org.kde.kwin.Scripting` pour résoudre fenêtres, géométrie et focus.

## Décision

Option 3, limitée à **KDE Plasma sous Wayland**.

- **Yeux** (`adapter-screencap-kwin`) : un script KWin éphémère résout le titre de la fenêtre en identifiant interne, puis `ScreenShot2.CaptureWindow` écrit les pixels dans un tube ; l'adapter convertit en PNG. Prérequis de développement : `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1` dans l'environnement de session KWin.
- **Mains** (`adapter-interaction-kwin-eis`) : `connectToEIS` renvoie un descripteur EIS, sur lequel un client EI en Rust pur (`reis`) émet les événements (souris absolue, boutons, défilement, clavier). Un unique `unsafe` localisé (projection du keymap) fait l'objet d'une exception de lint (ADR-0009).
- **Fenêtres** : liste, géométrie et focus par KWin Scripting (`window_geometry`, `focus_window`, `list_windows`), car EIS route les événements vers la fenêtre **active**.
- sans KDE (GNOME, par exemple), `xcap` reste le repli de capture sous XWayland ; GNOME Wayland est hors périmètre en 1.0 (portails XDG envisagés en 1.x).

**Justification** : seul KWin offre, sans dialogue d'autorisation, la capture d'une fenêtre précise et l'injection d'entrées. Les portails (option 2) sont la bonne voie portable mais demandent une interaction humaine ; ils pourront s'ajouter comme adapter sans toucher au reste (ADR-0041).

## Conséquences

**Positives**

- pilotage d'apps Wayland natives sans modification ni XWayland ;
- capture pixel exacte d'une fenêtre ;
- aucune dépendance à `libei` (client EI en Rust pur).

**Coûts et risques**

- limité à KDE Plasma Wayland ; la variable de permission est un réglage de la session de développement, à ne pas poser sur une machine qui n'est pas de confiance ;
- une notification ou un dialogue système peut voler le focus : les clics partent alors vers la mauvaise fenêtre (d'où `focus_window`) ;
- la connexion D-Bus qui appelle `connectToEIS` doit rester ouverte, sinon KWin invalide le descripteur ;
- ces adapters ne sont testés qu'à la main (tests `live_*` ignorés, session KDE requise).

## Notes

ADR rétroactif. Les yeux datent du 2026-06-25 (`d9b19d7`), les mains du même jour (`4caa335`). Voir [docs/guides/app-wayland-kde.md](../guides/app-wayland-kde.md) et [docs/platforms.md](../platforms.md).
