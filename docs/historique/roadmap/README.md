# Roadmap DevEye — yeux et mains sur Atelier, Chronos, atlas

Audit du 24 juin 2026 (Fedora 44, KDE Plasma Wayland). Cette roadmap rend DevEye
capable de donner à un agent (Claude via MCP) et à un humain des **yeux** (voir le
rendu d'une app GUI en cours d'exécution) et des **mains** (clic, hover, saisie,
navigation, eval JS) sur trois cibles locales, pour orchestrer des revues en temps
réel et automatiser la chasse aux bugs.

Source de vérité du domaine : [`../architecture.md`](../architecture.md). Guide
pratique : [`../usage.md`](../usage.md). Contrat SDK : [`../wire-protocol.md`](../wire-protocol.md).

---

## Le constat central

DevEye fut conçu pour Windows / WebView2 / CDP. Les trois cibles tournent sous
Linux/Wayland et aucune n'est dans ce moule. Trois vérités structurent tout :

1. **CDP ne parle pas à WebKitGTK.** Atelier et Chronos sont des apps Dioxus
   desktop, donc WebKitGTK (wry) sur Linux, pas Chromium. La voie pour Dioxus est
   le **pont natif SDK in-process** (eval JS via `wry::evaluate_script`).
2. **Wayland bride les yeux et les mains.** `xcap` capture en X11 pur (0 fenêtre en
   Wayland natif) ; `XTEST`/xdotool idem. Voie propre : DBus KWin (`ScreenShot2`,
   `connectToEIS`/libei). XWayland (`GDK_BACKEND=x11`) sert d'intérim.
3. **atlas (.NET / Uno) ne peut pas embarquer le SDK Rust.** Intégration
   boîte-noire : **CDP contre la tête WASM** dans un Chromium + **shim .NET**
   (`Atlas.DevEye`) implémentant le wire protocol pour l'état interne.

Bug latent découvert : le **SDK canonique et les copies vendored ont divergé**
(`/devtools/eval` manque au canonique ; `/devtools/command` et la garde Host
manquent aux vendored). C'est le point de départ obligatoire (M0).

## Décisions verrouillées par Morgan

1. **Mains Atelier** : brancher la pompe eval sous `cfg(all(feature = "deveye", debug_assertions))` (invisible en release). Voir [M1b](M1b-mains-atelier.md).
2. **Yeux Wayland** : construire `adapter-screencap-kwin` ; XWayland reste l'intérim. Voir [M1](M1-screencap-kwin.md).
3. **atlas** : **shim .NET prioritaire** ; le wire protocol est formalisé tôt (contrat commun SDK Rust + shim C#). Voir [M3](M3-shim-dotnet-atlas.md) et [wire-protocol](../wire-protocol.md).

## Matrice de capacités par cible

| Capacité | Atelier (Dioxus 0.6 desktop) | Chronos (Dioxus 0.7 desktop + web WASM) | atlas (.NET 10 / Uno + Blazor transitoire) |
|---|---|---|---|
| EYES (pixel) | xcap via XWayland -> M1 KWin | idem natif ; WASM/CDP : screenshot complet | Uno desktop : xcap/XWayland ; WASM : CDP |
| DOM / inspect | bloqué (pompe eval) -> M1b | natif OK ; WASM/CDP : `DOM.getDocument` | Uno desktop : néant (Skia) ; WASM : CDP |
| HANDS (input) | bloqué -> M1b ; OS-level -> M5 | natif OK ; WASM/CDP : OK | WASM : CDP ; Uno desktop : M5 |
| Logs | OK | natif OK ; WASM indispo | shim .NET (M3) |
| State / signals | OK | natif OK ; WASM indispo | shim .NET (M3) |

## Jalons

| # | Page | Objectif | Effort | Dépend de |
|---|---|---|---|---|
| M0 | [Réconcilier le SDK + wire protocol](M0-reconciliation-sdk.md) | route `--target sdk` complète, contrat figé | M | — |
| M1 | [adapter-screencap-kwin](M1-screencap-kwin.md) | yeux fiables sous Wayland natif | M | M0 |
| M1b | [Mains Atelier (pompe eval)](M1b-mains-atelier.md) | click/type/dom sur Atelier | S | M0 |
| M2 | [CDP atlas WASM](M2-cdp-atlas-wasm.md) | piloter la tête WASM sans toucher atlas | S | M0 |
| M3 | [Shim .NET Atlas.DevEye](M3-shim-dotnet-atlas.md) | état interne d'atlas (ViewModels/Serilog) | M | M0, M2 |
| M4 | [Outils MCP scénarios/bugs](M4-mcp-scenarios.md) | wait/find/assert/console-errors/drag | M | M0, M1 |
| M5 | [adapter-interaction-kwin-eis](M5-interaction-kwin-eis.md) | mains OS-level (Uno desktop, cibles natives) | L | M1 |
| M6 | [Domaine + orchestration](M6-domaine-orchestration.md) | Session, WaitCondition, ConsoleErrorSource | L | M4 |
| M7 | [Robustesse, DX, CI](M7-robustesse-ci.md) | sync vendored, pin Dioxus, e2e headless | M | M0-M4 |

Contrat transverse : [Wire Protocol DevEye](../wire-protocol.md).

## Séquence recommandée

- **Sprint 1 — débloquer (M0).** Seul vrai prérequis ; le wire protocol conditionne le shim .NET.
- **Sprint 2 — deux fronts en parallèle.** Dioxus : `M1 -> M1b`. atlas : `M2 -> M3`.
- **Sprint 3 — orchestration (M4 -> M6).** Transforme "yeux + mains" en vraie chasse aux bugs : attentes conditionnelles, assertions, collecte d'erreurs console, sessions reproductibles.
- **M5 et M7** : extensions ciblées (input OS pour Uno desktop ; durcissement CI), insérables dès que leur prérequis est vert.

## Les cinq verrous transverses

- **(a)** CDP perpendiculaire à WebKitGTK -> pont natif SDK (M0). Ne pas tenter une traduction de protocole.
- **(b)** Wayland/KWin bride capture et input -> `adapter-screencap-kwin` (M1) + `adapter-interaction-kwin-eis` (M5) ; XWayland en intérim.
- **(c)** atlas .NET sans SDK Rust -> CDP WASM (M2) + shim .NET (M3) sur le wire protocol.
- **(d)** filtre cible CDP hard-code "dioxus" -> `TargetFilter` configurable (M2), sans nom d'app en dur.
- **(e)** SDK canonique vs vendored divergents -> backport + `just sync-deveye` (M0, M7).
