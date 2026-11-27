# Plateformes et cibles : ce qui est supporté

DevEye 1.0 ne promet que ce qu'il teste ou ce qu'il peut vérifier à la main, et le dit. Ce document fixe le périmètre ([ADR-0037](adr/0037-linux-seul-supporte-en-1-0.md)) et, pour chaque environnement où tourne l'app à piloter, ce qui fonctionne et comment c'est vérifié.

## Deux questions distinctes

1. **Où tourne `deveye-server` ?** **Linux uniquement.** Sous Windows ou macOS, la compilation s'arrête avec un message explicite (`compile_error!`, ADR-0037) : les adapters X11, KWin et Wayland sont propres à Linux. Rien n'est promis, ni testé, sur ces systèmes.
2. **Où tourne l'app à piloter ?** Sur la même machine Linux, dans l'un des environnements du tableau ci-dessous. Les bibliothèques MIT (`deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`) restent portables : une app qui embarque le SDK peut être compilée sous n'importe quelle plateforme, mais seul son pilotage depuis un `deveye-server` Linux est visé.

## Légende

| Statut | Sens |
|---|---|
| **Supporté** | fait partie de la promesse de la 1.0 ; un défaut est un bug |
| **Expérimental** | fonctionne dans le périmètre décrit, mais non vérifié de bout en bout ou limité ; sans promesse de comportement |
| **Non supporté** | hors périmètre de la 1.0 ; aucun test, aucune promesse |

| Test | Sens |
|---|---|
| **CI** | exécuté par un job bloquant du workflow `.github/workflows/ci.yml` |
| **Manuel** | vérifié à la main par le mainteneur avant chaque release, critère fixé par [ADR-0058](adr/0058-critere-de-support-verification-par-la-ci-ou-manuelle.md) ; les tests existants sont marqués `#[ignore]` car ils exigent une session graphique réelle |

## Matrice

Les cinq capacités sont celles de `target_info` et des outils MCP : **yeux** (captures), **mains** (clic, saisie, défilement, touches), **DOM et JS** (`dom`, `find_element`, `eval_js`, attentes sur sélecteur), **état et logs** (`state`, `logs_tail`, collecte console), **commandes** (`list_commands`, `invoke_command`, `navigate`).

| Environnement de l'app | Yeux | Mains | DOM et JS | État et logs | Commandes | Statut | Test |
|---|---|---|---|---|---|---|---|
| **Linux, serveur X privé** (`--target x11`) | `GetImage` de la racine, rognée à la fenêtre | XTEST : vrais événements sur le serveur privé | pas de JS ; `dom` = réponse de la route `ui.widgets` de l'app, si elle l'expose | via le SDK de l'app | via le SDK de l'app (`navigate` non supporté) | **Supporté** | **CI** : adapter `x11` et session MCP complète contre un `Xvfb` privé |
| **Linux, Wayland KDE Plasma** (`--target kwin-eis`, ou `sdk`/`auto` avec capture KWin) | KWin `ScreenShot2` (fenêtre précise, pixel exact) | KWin EIS : vrais événements d'entrée dans la fenêtre active | aucun en `kwin-eis` (rendu natif) ; avec un webview : voir les lignes CDP et SDK | via le SDK de l'app | via le SDK de l'app | **Supporté** | **Manuel** : les tests `live_*` des adapters KWin exigent une session KDE réelle (`#[ignore]`) ; la logique pure est testée en CI |
| **Linux, webview Chromium par CDP** (`--target cdp`, ou `auto`) | CDP `Page.captureScreenshot` (à l'échelle CSS) | CDP `Input.*` (événements navigateur réels) | complet (`dom`, JS, sélecteurs, `resize`, `upload_file`, `handle_dialog`) | via le SDK de l'app, si embarqué | via le SDK de l'app, si embarqué | **Supporté** | **CI** : protocole simulé (tests unitaires du client CDP). **Manuel** : session réelle avec un navigateur ou une webview Chromium |
| **Linux, webview sans CDP** (WebKitGTK, `--target sdk`, ou repli de `auto`) | capture OS (KWin sous KDE Wayland, `xcap` sinon) | événements JavaScript synthétiques par la pompe du SDK | via le pont SDK (`eval_js`, `dom` par `outerHTML`) ; pas de `resize`, `upload_file`, `handle_dialog` | via le SDK | via le SDK | **Supporté** | **CI** : contrat HTTP du SDK et suite de conformité. **Manuel** : app réelle avec sa pompe |
| **Linux, Wayland GNOME** | `xcap` sous XWayland seulement, non testé | aucune voie (KWin EIS est propre à KDE) | uniquement par CDP ou le pont SDK, indépendants du compositeur | via le SDK | via le SDK | **Non supporté** (hors périmètre ; portails XDG envisagés en 1.x) | aucun |
| **Windows** | non supporté | non supporté | non supporté | non supporté | non supporté | **Non supporté** : `deveye-server` ne compile pas hors Linux | aucun |
| **macOS** | non supporté | non supporté | non supporté | non supporté | non supporté | **Non supporté** : `deveye-server` ne compile pas hors Linux | aucun |

Les outils expérimentaux `snapshot` et `act` désignent un élément par référence : `w:<id>` sur `x11` et `kwin-eis` (route `ui.widgets` du SDK), `e<n>` par le pont JS sur `sdk`/`auto` (webview WebKitGTK comprise) ; voir [Désigner un élément par référence](usage.md#désigner-un-élément-par-référence-expérimental).

Lectures utiles :

- **Le SDK et le CDP ne dépendent pas du compositeur** : une webview Chromium ou une app avec SDK se pilote de la même façon sous X11, sous Wayland KDE et sous Wayland GNOME pour le DOM, l'état et les commandes. Ce qui varie, ce sont les **yeux** et les **mains** natifs, qui passent par le système d'affichage. C'est pourquoi GNOME Wayland est classé non supporté : le pilotage complet d'une app native n'y a pas de voie.
- **Une ligne « Supporté, test manuel »** ne veut pas dire non testée : la logique pure (conversion de pixels, traduction des commandes, mapping d'erreurs) est couverte par des tests en CI ; seul le contact avec le vrai compositeur ou le vrai navigateur est vérifié à la main.
- **`--target auto`** (le défaut) n'est pas une cible de plus : c'est le repli appel par appel entre CDP et pont SDK ([ADR-0046](adr/0046-repli-appel-par-appel-du-mode-auto.md)). Sa couverture est celle des deux lignes concernées.
- **Presse-papiers** (`clipboard_get`, `clipboard_set`) : session Wayland uniquement (`wl-clipboard`) ; jamais en `x11`.
- **Focus et géométrie** (`focus_window`, `window_geometry`, `list_windows` en `kwin-eis`) : KDE Wayland ou cible `x11` ; sinon résultat structuré `{ reason: "unsupported" }`.

## Passer d'un environnement à l'autre

`deveye-server doctor --target <cible>` diagnostique, en lecture seule, ce qui manque pour la cible choisie (session, KWin, EIS, `Xvfb`, SDK, CDP, ports) et donne un conseil par vérification ; voir [usage.md](usage.md#diagnostic--deveye-server-doctor). Les guides par type d'app sont dans [guides/](guides/README.md).

## Évolution

Ouvrir une plateforme ou un compositeur supplémentaire exige un nouvel ADR et un job de CI dédié ([ADR-0037](adr/0037-linux-seul-supporte-en-1-0.md)). Les pistes connues : portails XDG pour GNOME Wayland, un adapter par système pour Windows et macOS.
