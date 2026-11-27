# Migrer de la 0.2 vers la 1.0

Ce guide liste tout ce qui change pour qui consomme DevEye en 0.2 : une app qui embarque `deveye-sdk` (ou copie `deveye-core`, `deveye-ports`, `deveye-sdk`), un projet qui lance `deveye-server`, un client du protocole HTTP. La 1.0 fige la surface décrite dans [stability.md](stability.md) : ce qui suit est donc le **dernier lot de changements cassants** avant la prochaine version majeure.

| Vous êtes… | Lire |
|---|---|
| une app Rust qui embarque le SDK | [1](#1-licences), [2](#2-api-rust--non_exhaustive-et-constructeurs), [3](#3-code-mort-retiré), [5](#5-sdk--axum-08-secrets-protocole), [8](#8-distribution-et-vendoring) |
| un projet qui configure `deveye-server` et `.mcp.json` | [4](#4-filtre-de-page-cdp), [6](#6-binaire-et-outils-mcp), [7](#7-plateformes) |
| une implémentation du protocole dans un autre langage | [5](#5-sdk--axum-08-secrets-protocole) |

## 1. Licences

La 0.2 était sous licence propriétaire. La 1.0 est open source, avec deux licences selon le rôle du crate ([ADR-0002](adr/0002-licences-agpl-outil-mit-bibliotheques.md)) :

- **MIT** : `deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`. Intégrer le SDK dans une app **n'impose rien** à cette app ;
- **AGPL-3.0-or-later** : `deveye-server`, tous les `adapter-*`, `deveye-conformance`, les scripts.

À faire : rien dans le code. Si vous redistribuez `deveye-server` (ou un dérivé), respecter l'AGPL ; si vous embarquez seulement le SDK, rien de plus que la mention de licence MIT.

## 2. API Rust : `#[non_exhaustive]` et constructeurs

Pour pouvoir ajouter des variantes et des champs en version mineure ([ADR-0011](adr/0011-politique-de-stabilite-et-de-versionnage-1-0.md), [ADR-0012](adr/0012-application-de-non-exhaustive-et-choix-limites.md)), ces types sont désormais `#[non_exhaustive]` :

- **enums** : `MouseButton`, `InteractionCommand`, `WaitPredicate`, `ConsoleLevel`, `LogLevel`, `PortError` ;
- **structs** : `Screenshot`, `DomTree`, `StateDump`, `WindowInfo`, `WaitOutcome`, `ConsoleError`, `LogEvent`, `CommandDescriptor` (`deveye-core`) ; `Config`, `StatePayload`, `LogsPayload`, `EvalRequest` (`deveye-sdk`).

Conséquences pour votre code :

1. **`match` sur un de ces enums** : ajouter un bras `_`.

   ```rust
   match error {
       PortError::Network(_) => { /* … */ }
       _ => { /* toute autre erreur, y compris celles ajoutées plus tard */ }
   }
   ```

2. **Construire un de ces structs** : plus de littéral hors de la crate qui le définit ; utiliser le constructeur.

   | Avant (littéral) | Après |
   |---|---|
   | `ConsoleError { level, text, url, line, timestamp }` | `ConsoleError::new(level, text)`, puis `.with_location(url, line)` |
   | `WindowInfo { id, title, url, kind }` | `WindowInfo::new(…)` |
   | `WaitOutcome { satisfied, elapsed_ms, detail }` | `WaitOutcome::new(satisfied, elapsed_ms, detail)` |
   | `CommandDescriptor { name, description }` | `CommandDescriptor::new(name, description)` |
   | `Config { port, .. }` | `Config::with_port(port)`, ou `Config::default()` |

   `Screenshot`, `DomTree`, `StateDump` et `LogEvent` avaient déjà leurs constructeurs.

3. **Destructurer `EvalRequest`** : ajouter `..` : `EvalRequest { expression, respond, .. }`.

4. **Rester exhaustifs** : `Size`, `Position`, `WindowRect` et les **champs des variantes** de `InteractionCommand` et `WaitPredicate` restent constructibles par littéral.

Nouveautés d'API : `InteractionCommand::Resize { width, height }` et son constructeur `resize` ([ADR-0019](adr/0019-resize-variante-d-interaction-command.md)) ; `PortError::Script { message, stack }` pour une exception levée par le JavaScript évalué ([ADR-0023](adr/0023-exceptions-js-en-resultat-d-outil-en-erreur.md)). Un adapter ou un `match` sur `InteractionCommand` doit traiter `Resize` ou le refuser par `PortError::Unsupported`.

## 3. Code mort retiré

Retirés avant le gel, sans usage dans le dépôt :

- `deveye-core` : le module `session` (`AppSession`, `AppKind`) ;
- `adapter-cdp-webview` : `CdpTarget` et `DEFAULT_HTTP_URL` ;
- `with_default_url()` de `SdkCommandRouter`, `DioxusSignalsInspector` et `SdkTracingLogSource` : utiliser `::new(deveye_sdk_client::DEFAULT_SDK_URL)`.

Les adapters sont internes (`publish = false`, aucune promesse sur leur API Rust) ; seuls les retraits de `deveye-core` peuvent toucher un consommateur.

## 4. Filtre de page CDP

En 0.2, sans option, le filtre ne retenait que les pages dont l'URL ou le titre contenait un nom de framework précis (plus `about:blank`) : toute autre page était ignorée, avec un simple délai dépassé.

En 1.0 ([ADR-0014](adr/0014-filtre-de-cible-cdp-permissif-par-defaut.md)) :

- **sans** `--cdp-target-filter-url` ni `--cdp-target-filter-title`, la cible est la **première page qui n'est pas `about:blank`** ;
- `--cdp-accept-blank` accepte aussi `about:blank` ;
- si rien ne correspond, l'erreur **liste les pages vues** et le flag à ajuster.

À faire : si votre `.mcp.json` contournait l'ancien filtre par `--cdp-target-filter-url`, il continue de fonctionner. Si **plusieurs** pages sont ouvertes et que vous comptiez sur l'ancien filtre pour choisir, ajouter un filtre explicite.

## 5. SDK : axum 0.8, secrets, protocole

**Dépendances.** `deveye-sdk` passe à `axum` 0.8 et `tower-http` 0.7 ([ADR-0008](adr/0008-montees-de-version-avis-rustsec.md)) ; leur plage de versions apparaît dans le graphe de votre app. MSRV : Rust 1.88 ([ADR-0010](adr/0010-toolchain-stable-et-msrv-1-88.md)).

**`init` inerte en release.** `deveye_sdk::init` est un no-op hors `debug_assertions` ([ADR-0042](adr/0042-posture-dev-only-du-sdk.md)) ; garder votre garde `#[cfg(debug_assertions)]` (ou, mieux, la feature optionnelle de [integration.md](integration.md)).

**Masquage des secrets.** Les valeurs des clés sensibles (`password`, `token`, `api_key`, `authorization`, `cookie`…) sont remplacées par `<secret>` dans l'état et les champs de logs ; `Config::with_secret_keys` ajoute les vôtres ([ADR-0016](adr/0016-masquage-des-secrets-par-cles-dans-le-sdk.md)). L'annotation `#[deveye(secret)]`, évoquée dans d'anciens documents, **n'a jamais existé** et n'est pas promise. Le masquage ne couvre ni `eval_js`, ni les retours de commandes, ni le texte libre des logs.

**Protocole 1.0** ([ADR-0017](adr/0017-protocole-1-0-version-info-et-limites.md), [wire-protocol.md](wire-protocol.md)) :

- l'en-tête `X-DevEye-Protocol-Version` passe de `1` à **`1.0`** (`MAJEURE.MINEURE`), émis sur chaque réponse, y compris les `403` ;
- **nouvelle route** `GET /devtools/info` : version du SDK, version du protocole, capacités (`state`, `commands`, `info`, `redaction`, `logs`, `eval`) ; elle est déclarée dans les schémas de `docs/schemas/1.0/` ;
- corps de requête limités à **1 Mio** (`413` au-delà) ;
- `DEVEYE_PORT` : entier de 1 à 65535, toute autre valeur retombe sur `31337` ;
- ordre de tri normatif de `GET /devtools/commands` (par nom, octets UTF-8) ;
- le client (`deveye-sdk-client`) lit et vérifie la version du protocole : `fetch_info`, `check_protocol_header`, `SUPPORTED_PROTOCOL_VERSION`.

**Implémentation dans un autre langage.** Émettre l'en-tête `1.0`, ajouter `GET /devtools/info`, borner les corps à 1 Mio, puis valider avec la suite de conformité : [guides/implementation-tierce.md](guides/implementation-tierce.md).

## 6. Binaire et outils MCP

**Outils : 41 en 0.2, 44 en 1.0.** Nouveaux : `resize` (viewport émulé, CDP seulement), `logs_wait` (long-poll des logs), `observe` (capture réduite, arbre, état, console et logs en un appel). Les noms, paramètres et formes de réponse des 41 autres sont conservés, avec les comportements suivants à connaître :

- `eval_js` : une exception JavaScript revient en résultat d'outil en erreur `{ ok: false, reason: "js_exception", message, stack? }` au lieu d'une erreur de protocole ;
- `type_text` sans champ éditable au focus (cibles webview) : `{ ok: false, reason: "interaction_failed" }` au lieu d'un faux succès ;
- `screenshot` en **CDP** : l'image est à l'échelle CSS, même sur un écran HiDPI ; un pixel de l'image est un pixel de `click`. Un second bloc texte donne `captured_at`, `viewport` et `image.scale` ;
- `state` et `dom` ajoutent un second bloc texte (`captured_at`, `signals`) et distinguent « SDK joignable sans état » de `sdk_unavailable` ;
- les outils de capture acceptent `max_width`, `format`, `quality` ; `assert_visual` accepte `ignore_regions` et `update_baseline` et rend une image de différence en cas d'échec ;
- les coordonnées de `click`, `hover`, `drag_drop`, `scroll` sont validées (négatives ou hors viewport : `invalid_params`) ;
- chaque outil déclare ses annotations MCP ([ADR-0027](adr/0027-annotations-des-outils-mcp-et-classification-des-risques.md)) ; la version du protocole MCP est épinglée à `2025-11-25`, transport stdio seul ([ADR-0028](adr/0028-version-du-protocole-mcp-epinglee-stdio.md)).

**Options.** Une option sans effet sur la cible choisie est maintenant **refusée** ([ADR-0031](adr/0031-validation-des-flags-selon-la-cible.md)) : `--display` et `--x11-class` seulement avec `--target x11` ; `--cdp-url`, `--cdp-target-filter-*`, `--cdp-accept-blank` seulement avec `auto` et `cdp` ; `--screencap-backend` jamais avec `x11`. Sans `--serve-mcp` ni sous-commande, le binaire s'arrête en erreur d'usage au lieu de sortir en silence. À faire : relire vos `.mcp.json` et retirer les options devenues refusées.

**Ajouts.** `deveye-server --version` (version, commit, protocoles), sous-commande `doctor` ([ADR-0032](adr/0032-sous-commande-doctor-en-lecture-seule.md)), `target_info` enrichi (capacités de fenêtres, de presse-papiers, backend effectif, `server`), cible `x11`, suite `deveye-conformance`.

## 7. Plateformes

**Linux seul** ([ADR-0037](adr/0037-linux-seul-supporte-en-1-0.md), [platforms.md](platforms.md)). `deveye-server` ne compile plus hors Linux : `compile_error!` avec un message explicite. Les mentions de Windows (WebView2) décrivent l'app cible, jamais le serveur. Les bibliothèques MIT restent portables.

## 8. Distribution et vendoring

- **Forge** : GitHub (`github.com/Kovelt/DevEye`), CI GitHub Actions ; l'ancienne forge est archivée ([ADR-0003](adr/0003-hebergement-github-ci-github-actions.md)).
- **Binaire** : archive de release et `cargo install --git … --tag v1.0.0` ([ADR-0038](adr/0038-distribution-archives-github-et-cargo-install.md)) ; plus de `cargo run` à chaque démarrage.
- **Bibliothèques** : dépendance versionnée (crates.io une fois publiée, sinon git et tag) à la place des copies vendorées ([integration.md](integration.md)). Le script `scripts/sync-deveye.sh` est conservé **pour la transition seulement** ([ADR-0040](adr/0040-fin-du-vendoring-script-de-synchronisation-de-transition.md)), avec cibles configurables et sans aucun chemin personnel : il faut lui fournir vos dossiers.
- **Neutralité** : aucun nom d'app cliente dans le code ni les docs de référence ([ADR-0013](adr/0013-neutralite-apps-clientes-et-archivage-historique.md)) ; les anciens audits sont archivés dans `docs/historique/`.

## Liste de contrôle

1. Mettre à jour la dépendance (`deveye-sdk = "1"` ou git sur `v1.0.0`) et recompiler ; corriger les `match` (bras `_`) et les littéraux de structs devenus `#[non_exhaustive]`.
2. Remplacer `with_default_url()` et les imports retirés.
3. Remplacer l'ancien `deveye-server` par la 1.0 (archive ou `cargo install`) ; relire `.mcp.json` ; lancer `deveye-server doctor --target <cible>`.
4. Si vous implémentez le protocole : en-tête `1.0`, `GET /devtools/info`, limite de 1 Mio ; passer `deveye-conformance`.
5. Si vous copiez les bibliothèques : migrer vers la dépendance versionnée, ou utiliser le script de transition avec vos cibles.
