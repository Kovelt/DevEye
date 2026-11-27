# Journal des modifications

Tous les changements notables de DevEye sont consignés ici.

Le format suit [Keep a Changelog 1.1.0](https://keepachangelog.com/fr/1.1.0/) et le projet respecte le [versionnage sémantique 2.0.0](https://semver.org/lang/fr/) selon la politique décrite dans [`docs/stability.md`](docs/stability.md). Les décisions qui motivent un changement sont consignées dans [`docs/adr/`](docs/adr/README.md).

## [Non publié]

## [1.1.0] - 2026-11-27

Version mineure « Perception structurée » : contrat des widgets natifs figé, réponses MCP typées et lisibles par machine, logs poussés par le SDK. Aucun changement cassant de la surface stable ([`docs/stability.md`](docs/stability.md)) ; le protocole du SDK reste en version 1.0, les ajouts additifs se découvrent par `capabilities` ([ADR-0068](docs/adr/0068-capacites-additives-et-version-du-protocole.md)). Aucune surface expérimentale n'est ajoutée en 1.1 : la règle des features `unstable-*`, hors promesse semver, est posée pour les versions suivantes ([ADR-0069](docs/adr/0069-surface-experimentale-par-features-unstable.md)).

### Added

- `deveye-core` : module `ui` (contrat `deveye.ui/1` de la route `ui.widgets` : `UiTree`, `UiWidget`, `UiRole` ouvert, builders, validation, lecteur tolérant du format legacy) et feature optionnelle `schema` générant `docs/schemas/ui-widgets-1.json` ([ADR-0062](docs/adr/0062-schema-deveye-ui-1-de-la-route-ui-widgets.md)).
- Fixtures versionnées `deveye.ui/1` et vecteurs partagés de masquage des secrets et de garde Host (`docs/schemas/shared-vectors-1.json`) pour les implémentations tierces ([ADR-0064](docs/adr/0064-masquage-des-secrets-transversal-et-vecteurs-partages.md)).
- `deveye-sdk::ui` (`publish`, `is_observed`, builder) : l'app publie son arbre `deveye.ui/1` ; la route `ui.widgets` est enregistrée au premier usage et `/devtools/info` annonce la capacité `ui.widgets/1` ; sans effet en release ([ADR-0065](docs/adr/0065-helper-sdk-ui-lecture-serveur-et-controles.md)).
- `deveye-server doctor` contrôle l'arbre `ui.widgets` quand le SDK l'annonce (échec sur ids dupliqués, parent ou schéma invalide).
- Scénarios UI-01 et UI-02 de la suite `deveye-conformance` pour la route `ui.widgets`.
- `RouteDomInspector::ui_tree()` et avertissement d'arbre périmé (> 2 s).
- `deveye-sdk` : `log()` et `log_with()` poussent des logs vers `logs_tail` sans abonné `tracing`, `LogLevel` réexporté ; `layer()` / `DeveyeLayer` composent la capture avec le subscriber de l'hôte, et `try_install_tracing_layer()` signale un abonné global déjà installé (feature `tracing-layer`, par défaut) ([ADR-0066](docs/adr/0066-push-de-logs-et-layer-du-sdk.md)).
- Sorties structurées MCP : les 28 outils à petite sortie (`target_info`, `wait_*`, `assert_selector`, `assert_state`, `window_geometry`, `focus_window`, `click_selector`, `click_text` et les acquittements) déclarent un `outputSchema` dans `tools/list` et renvoient un `structuredContent` égal à leur texte ; le texte reste identique à celui de la 1.0 ([ADR-0067](docs/adr/0067-sorties-structurees-mcp-palier-a.md)).
- Erreurs MCP typées `ToolError { ok, reason, detail, hint?, candidates? }` pour `unsupported`, `sdk_unavailable` et `interaction_failed`, avec `structuredContent` identique au texte ; `reason` est une liste ouverte.
- L'outil `state` porte `_meta` (`io.deveye/captured_at`, `io.deveye/entry_count`, `io.deveye/state_source`) sans changer son texte.
- `deveye-core` dérive `schemars::JsonSchema` sur `WaitOutcome` et `WindowRect` derrière la feature optionnelle `schema` ([ADR-0063](docs/adr/0063-dependance-optionnelle-schemars-dans-deveye-core.md)).

### Changed

- Règle 5 du `CLAUDE.md` amendée : `schemars` est autorisée dans `deveye-core` derrière la feature optionnelle `schema`.
- Le buffer de logs du SDK est un `std::sync::Mutex` sans canal de drain ; la capacité `logs` est annoncée dès qu'une source de logs est armée.
- Le numéro de version du protocole reste 1.0 pour tout ajout additif (route de commande, capacité, champ), découvert par `capabilities` ; `docs/stability.md` gagne une section « Surface expérimentale » (features `unstable-*`).
- `deveye_sdk::ui::OBSERVE_WINDOW` existe aussi en release ; `cargo test -p deveye-sdk --no-default-features` est vérifié en CI.
- `deveye-sdk` avec `default-features = false` n'expose plus `install_tracing_layer`, `try_install_tracing_layer` ni `layer()` (sans effet en 1.0 ; consommateur 1.0 concerné à signaler).

### Fixed

- La détection d'un widget secret ignore la casse et les espaces du rôle `password` dans `deveye-core`, le serveur et la conformité ; le drapeau `secret` du format legacy est lu par `UiTree::from_value`.
- Le lecteur legacy de `deveye.ui` conserve un `rect` de taille négative ou à valeur nulle au lieu de faire échouer tout l'arbre ; `validate` le relève (`NegativeSize`).
- `try_install_tracing_layer` est désormais atomique (plus de `SubscriberAlreadySet` spurieux en cas d'appels concurrents).
- La capacité `ui.widgets/1` n'est annoncée qu'une fois la route réellement enregistrée.

### Security

- L'outil `dom` et `invoke_command {name: "ui.widgets"}` remplacent par `<secret>` la valeur (`value`, et `text` du format legacy, enfants `children` compris) d'un widget secret et appliquent le masquage par clés aux autres champs d'un arbre de widgets ; une réponse qui n'a pas la forme d'un arbre de widgets est renvoyée intacte (durcissement de classe mineure, [`docs/stability.md`](docs/stability.md)).
- Le SDK ne publie jamais la valeur d'un widget secret (`state.secret` ou rôle `password`) ([ADR-0064](docs/adr/0064-masquage-des-secrets-transversal-et-vecteurs-partages.md), [ADR-0065](docs/adr/0065-helper-sdk-ui-lecture-serveur-et-controles.md)).

## [1.0.0] - 2026-11-27

Première version publique et stable. Le dépôt s'ouvre sous **AGPL-3.0-or-later** (outil : `deveye-server`, adapters, `deveye-conformance`, scripts) et **MIT** (bibliothèques embarquées : `deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`). Linux est la seule plateforme supportée. Le protocole du SDK passe en version 1.0, figé pour toute la série 1.x ([ADR-0061](docs/adr/0061-version-1-0-0-perimetre-de-la-sortie.md)).

### Added

- Licences open source : AGPL-3.0-or-later pour l'outil et MIT pour les bibliothèques qu'une app embarque (`deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`) ; fichiers `LICENSE` et `LICENSE-MIT`, une copie de `LICENSE-MIT` dans chaque crate MIT. Intégrer le SDK dans une app n'impose rien à cette app ([ADR-0002](docs/adr/0002-licences-agpl-outil-mit-bibliotheques.md)).
- Code de conduite (`CODE_OF_CONDUCT.md`, Contributor Covenant 2.1 en français, signalement par GitHub), politique de sécurité (`SECURITY.md`, modèle de menace dev-only, signalement privé des vulnérabilités) et guide de contribution (`CONTRIBUTING.md`).
- Processus ADR : chaque décision (architecture, dépendance, licence, protocole, surface stable, plateforme, outillage, exception) est consignée dans `docs/adr/` avec un index et un gabarit ; ADR-0001 à ADR-0061, dont six ADR rétroactifs sur les choix fondateurs (hexagonal, dev-only, pont natif, x11 isolé, KWin, repli auto).
- Cible `--target x11` : capture et gestes sur un display X11 privé obligatoire (`--display`), avec `adapter-x11` (capture `GetImage`, gestes XTEST, fenêtres) ; limites de capture (occultation, pas de HiDPI, sans compositeur) documentées.
- Outils MCP `observe` (capture réduite, résumé borné de l'arbre, état SDK, erreurs console et derniers logs en un appel, chaque section se dégradant seule), `resize` (émulation de viewport, `unsupported` structuré sur sdk, kwin-eis et x11) et `logs_wait` (long-poll des logs avec `level_min`, sans perte de logs drainés) : 44 outils au total.
- Annotations `readOnlyHint`, `destructiveHint`, `idempotentHint` et `openWorldHint` sur les 44 outils MCP (classification documentée, ADR-0027).
- `screenshot`, `screenshot_region` et `screenshot_element` acceptent `max_width`, `format` (png ou jpeg) et `quality` ; la réponse donne la taille finale, le facteur d'échelle et l'origine. `screenshot`, `dom` et `state` joignent un second bloc texte JSON de métadonnées (`captured_at`, `viewport`, `signals`, `note`).
- `assert_visual` accepte `ignore_regions` et `update_baseline`, et renvoie en cas d'échec une image de différence, `diff_bbox`, `diff_pixels` et `compared_pixels`.
- Variante `InteractionCommand::Resize` et variante `PortError::Script { message, stack }` pour les exceptions levées par le JS évalué ; constructeurs `ConsoleError::new` et `with_location`, `WindowInfo::new`, `WaitOutcome::new`, `CommandDescriptor::new`, `Config::with_port`, `SdkInfo::new`.
- Masquage des secrets dans le SDK : les valeurs des clés sensibles (`password`, `token`, `api_key`, `authorization`, `cookie`, `tokens`, `cookies`, `access_key`, `signing_key`, `database_url`, `mdp`, `jeton`…) sont remplacées par `<secret>` dans `/devtools/state` et dans les champs de `/devtools/logs/poll` ; `Config::secret_keys` et `with_secret_keys` ajoutent des clés. Les résultats d'`eval`, de commandes et `ui.widgets` ne sont pas filtrés ; les champs de log formatés par `Debug` ne sont pas analysés.
- Route `GET /devtools/info` du SDK (version du SDK, version du protocole, capacités dont `redaction` et `logs`) et `deveye_sdk_client::fetch_info` ; limite de 1 Mio sur le corps des requêtes du SDK (réponse 413).
- `deveye-sdk-client` vérifie `X-DevEye-Protocol-Version` : `PortError::Protocol` si la majeure diffère, avertissement unique si l'en-tête est absent ou la mineure différente.
- `deveye-server --version` (version, commit de build, versions des protocoles SDK et MCP) et sous-commande `deveye-server doctor [--target ...]` : diagnostic en lecture seule de l'environnement (session, KWin, EIS, Xvfb, SDK, CDP, ports), un statut et un conseil par vérification, code de sortie 1 si un prérequis bloquant manque.
- `target_info` expose `effective_backend`, `window_geometry`, `focus_window`, `window_manager`, `clipboard`, `js_dependent_tools` et `server {version, commit, sdk_protocol, mcp_protocol}`.
- Quand aucune page CDP ne correspond au filtre, l'erreur décrit le filtre et liste les pages vues (titre et URL tronqués).
- `deveye-conformance` : suite de conformité boîte noire du protocole (binaire `deveye-conformance --url`, recette `just conformance`) pour valider le SDK Rust ou toute implémentation tierce ; verdicts `SKIP` (non observable) et `N/A` (capacité non annoncée). Schémas JSON versionnés des corps du protocole dans `docs/schemas/1.0/`.
- Documentation de sortie : README open source, `docs/usage.md` (référence des 44 outils), `docs/integration.md`, `docs/platforms.md`, `docs/stability.md` (politique semver, dépréciation, MSRV, protocole), `docs/migration-1.0.md` et guides par type d'app dans `docs/guides/` avec `.mcp.json` prêts à copier ; tests de cohérence entre documentation et code (outils, nombres annoncés, liens, ancres, index des ADR).
- `rust-toolchain.toml` (stable, rustfmt, clippy), lints de workspace (`unsafe_code` interdit, `rust_2018_idioms`) et métadonnées crates.io (description, mots-clés, catégories, README) pour les quatre bibliothèques MIT, prêtes à être publiées (aucune publication effectuée, ADR-0004).
- CI GitHub Actions (format, clippy, tests, documentation, Xvfb, MSRV 1.88, `cargo deny`, conformité, preuve no-op du SDK, SBOM CycloneDX) et workflow de release sur tag `v*` : porte de qualité, archive Linux x86_64, `SHA256SUMS` et brouillon de GitHub Release ; recettes `just` associées (`doc`, `x11`, `e2e`, `msrv`, `deny`, `release-noop`, `sbom`, `install`, `license-boundary`, `packages`, `conformance`).
- Test de bout en bout du binaire (session MCP stdio complète sur un Xvfb privé) et preuve sur binaire que le SDK n'embarque aucun serveur HTTP en release.
- `scripts/sync-deveye.sh`, recréé en outil de transition déprécié (cibles configurables, `--check`, `--dry-run`, `--force`) et `scripts/sync-targets.example` (ADR-0040).

### Changed

- **Support** : Linux est la seule plateforme supportée en 1.0 ; la compilation de `deveye-server` hors Linux échoue avec un message explicite. Les tests Xvfb de la cible x11 sont bloquants en CI.
- **Hébergement** : le développement passe sur GitHub (CI GitHub Actions) ; la licence propriétaire est remplacée par AGPL-3.0-or-later (outil) et MIT (bibliothèques).
- **Protocole du SDK 1.0** : `X-DevEye-Protocol-Version: 1.0` (au lieu de `1`) ; majeure = cassant, mineure = additif. La spécification précise les erreurs 403/404/405/4xx, CORS, horodatages RFC 3339 UTC, `Host` IPv6 ou absent (le 400 de la pile HTTP est admis), tri et limites. `deveye-sdk-client` dépend désormais de `deveye-ports`, `serde` et `tracing`.
- **Version du protocole MCP** épinglée à 2025-11-25 (transport stdio seul) au lieu de suivre la dernière révision du SDK (ADR-0028). Le serveur MCP repose sur `rmcp` 3.12 (au lieu de 1.8) ; la liste des outils d'origine, leurs descriptions et leurs schémas d'entrée sont inchangés.
- `deveye-sdk` migre vers `axum` 0.8 et `tower-http` 0.7 (protocole filaire inchangé) ; dépendances transitives mises à jour dans leurs plages semver (`h2`, `rustls`, `rustls-webpki`, `webpki-roots`, `url`, `rustix`, `tokio`, `async-executor`).
- **API Rust** (`#[non_exhaustive]`) : `MouseButton`, `InteractionCommand`, `WaitPredicate`, `ConsoleLevel`, `LogLevel`, `PortError` ; `ConsoleError`, `WindowInfo`, `WaitOutcome`, `CommandDescriptor`, `Screenshot`, `DomTree`, `StateDump`, `LogEvent` (`deveye-core`), `Config`, `StatePayload`, `LogsPayload`, `EvalRequest` (`deveye-sdk`). Un `match` externe doit prévoir un bras `_` ; la construction passe par les constructeurs, la destructuration par `..`. Une variante ajoutée sera désormais un changement mineur.
- Une commande d'interaction, un prédicat d'attente ou un bouton de souris inconnu d'un adapter renvoie `PortError::Unsupported` au lieu de paniquer (x11).
- **Filtre de cible CDP** : sans `--cdp-target-filter-url` ni `--cdp-target-filter-title`, la cible est la première page qui n'est pas `about:blank` (fin du filtre codé en dur) ; `--cdp-accept-blank` autorise aussi `about:blank`. `TargetFilter` sans critère accepte toute page non vide.
- **Capture CDP** à l'échelle CSS (1 pixel d'image = 1 pixel CSS), y compris sur écran HiDPI et page défilée : les baselines `assert_visual` prises sur écran HiDPI changent de taille.
- Une exception JS d'`eval_js` (et des outils à base de JS) revient en résultat `isError` `{reason: "js_exception", message, stack?}` au lieu d'une erreur `-32603`.
- `click`, `right_click`, `double_click`, `hover`, `drag_drop` et l'ancrage de `scroll` refusent en `invalid_params` une coordonnée négative ou hors du viewport quand sa taille est connue ; un `level_min` invalide de `wait_for_log` donne `invalid_params`.
- `state` et `assert_state` distinguent un SDK joignable sans signal (`signals: 0` et `note`) d'un SDK injoignable (`isError` `{reason: "sdk_unavailable"}`).
- `type_text` sans champ éditable au focus est refusé en `isError` `{reason: "interaction_failed"}` sur les cibles WebView ; sur kwin-eis et x11, `ok` signifie « événements envoyés ».
- `assert_visual` confine `baseline_path` à `DEVEYE_BASELINE_DIR` (défaut : répertoire courant du serveur), en `.png`, sans `..` ni lien symbolique sortant, et refuse d'écraser un fichier qui n'est pas un PNG lisible. `clipboard_get` est annoté « monde ouvert ».
- En mode `auto`, chaque bascule du CDP vers le pont natif SDK (et son retour) est journalisée sur stderr.
- Les options sans effet sur la cible sont refusées au démarrage ; sans `--serve-mcp` ni sous-commande, `deveye-server` s'arrête en erreur d'usage (code 2) au lieu de sortir en succès. `target_info` n'annonce plus `list_windows`, `window_geometry` ni `focus_window` pour kwin-eis hors KDE et Wayland.
- `deveye-conformance` : `--strict` n'échoue plus pour une implémentation sans `redaction` ni `eval` ; le verdict affiche `INCOMPLET` quand des vérifications restent non observables ; les rapports ne citent plus le contenu des réponses.
- La release exige une porte de qualité (format, clippy, tests, deny, frontière des licences) avant de construire l'archive ; les pré-versions `vX.Y.Z-suffixe` sont marquées prerelease.
- Tous les crates passent en version `1.0.0` (ADR-0056) ; les manifestes déclarent aussi la version `1.0.0` de leurs dépendances internes par chemin. Métadonnées de package (édition, `rust-version` 1.88, dépôt GitHub) centralisées dans `[workspace.package]`.
- Les audits, la feuille de route, le backlog d'outils et le spike AT-SPI2 sont archivés sous `docs/historique/` (non normatifs) ; `docs/architecture.md` décrit Linux seul ; `docs/stability.md` liste les dépendances exposées dans l'API stable (`serde_json`, `chrono`, `tokio`, `reqwest`, `async-trait`, `thiserror`).

### Fixed

- Lint clippy `chunks_exact_to_as_chunks` dans la conversion BGRA vers RGBA de `adapter-screencap-kwin` (clippy 1.98) ; `cargo doc` ne produit plus d'avertissement (liens intra-doc non résolus).
- Le SDK ne panique plus sur un verrou empoisonné, signale sur stderr l'échec de liaison de son port, et un second `deveye_sdk::init` ne démasque plus les clés ajoutées par `with_secret_keys`. `DEVEYE_PORT=0` est rejeté (retombée sur 31337) et seuls les chiffres décimaux sont acceptés (`+4242` est ignoré). `/devtools/info` n'annonce plus `logs` quand la couche tracing n'a pas pu s'installer.
- Le serverInfo de la négociation MCP annonce `deveye-server` et sa version au lieu de l'identité de la bibliothèque `rmcp`.
- `invoke_command` relit un objet ou tableau transmis sous forme de chaîne JSON ; plus d'`expect` de sérialisation dans les outils MCP, l'adapter CDP, le pont SDK et l'adapter KWin ; les sorties JSON ne retombent plus sur un `null` silencieux.
- `wait_for` et `wait_condition` remontent l'incompatibilité de version majeure du protocole du SDK au lieu d'expirer sans explication ; le pont SDK ne présente plus un refus HTTP (403, autres 4xx) comme une exception JS.
- L'adapter CDP garde la première page rencontrée en filtre ouvert malgré une rafale d'ouvertures, se rattache à une page connue quand la page attachée est détruite, et rend la valeur levée par un `throw` d'objet non-Error.
- `assert_visual` dont `ignore_regions` couvre toute l'image renvoie `ok: false` avec `reason: fully_masked` ; `logs_wait` renvoie les logs déjà drainés avec `poll_error` quand un sondage ultérieur échoue.
- `deveye-server doctor` : un SDK de protocole majeur différent est signalé incompatible, `--target kwin-eis` donne un avertissement quand l'accès EIS n'est pas vérifiable, `--target x11` sans `--display` diagnostique l'absence de display, et les sondes réseau sont limitées à la cible et parallèles.
- La provenance du commit affichée par `--version` est correcte dans un worktree git, avec des refs packées, et n'est pas empruntée au dépôt hôte d'un crate vendorisé.
- Les workflows CI et release installent les bibliothèques de développement requises par `xcap` et `xkbcommon` (`libwayland-dev`, `libegl-dev`, `libgles-dev`, `libdrm-dev`, `libxkbcommon-x11-dev`, `libpipewire-0.3-dev`, `libspa-0.2-dev`, `libclang-dev`).
- Documentation : attentes sans DOM (`wait_for_selector`, `wait_for_text` attendent le timeout puis `found: false` en kwin-eis et x11), port SDK du guide x11, description et liens rustdoc de `deveye-core` et `deveye-ports`.
- `scripts/sync-deveye.sh` détecte la dérive de tous les fichiers des crates copiées (README et `LICENSE-MIT` inclus), vérifie la garde de toutes les cibles avant d'écrire et refuse `--check` combiné à `--dry-run`.

### Removed

- Code mort de l'API : `AppSession` et `AppKind` (`deveye-core`), `CdpTarget`, `DEFAULT_HTTP_URL` et les constructeurs `with_default_url` des adapters.
- `.gitlab-ci.yml` : la CI est désormais GitHub seul. L'ancien script de synchronisation des copies vendorisées est retiré (les consommateurs dépendent des crates) puis remplacé par l'outil de transition déprécié décrit plus haut.
- Le filtre de cible `dioxus` codé en dur et les noms d'apps clientes du code, des tests et des commentaires (ADR-0013).

### Security

- `assert_visual` ne peut plus écrire ni lire un fichier arbitraire hors de la racine des baselines (ADR-0049).
- Le drain `GET /devtools/logs/poll` du SDK refuse (403) les requêtes inter-origine (en-tête `Origin` ou `Sec-Fetch-Site` différent de `none`) : une page web ne peut plus vider le tampon de logs (ADR-0055).
- Le masquage des secrets du SDK remplace les valeurs des clés sensibles dans l'état et les logs (ADR-0016) ; les corps de requête sont bornés à 1 Mio.
- RUSTSEC-2026-0462 (`rmcp`, buffering non borné des clients Streamable HTTP) corrigé par la montée vers `rmcp` 3.12 ; mises à jour de `h2`, `rustls`, `rustls-webpki`, `webpki-roots`, `url`, `rustix` et `tokio` pour les avis RUSTSEC correspondants ; `cargo deny check advisories` passe (exceptions justifiées par les ADR-0006 et ADR-0007).
- Le SDK n'embarque aucun serveur HTTP en build release, preuve exécutée en CI (ADR-0042).

## [0.2.0] - 2026-07-04

Audit profond du 4 juillet 2026 : première version taguée. 41 outils MCP, 12 ports, 205 tests (123 au départ de l'audit). Consolide sept demandes de fusion issues de l'audit (voir [`docs/historique/audit-2026-07-04.md`](docs/historique/audit-2026-07-04.md)).

### Added

- Outils MCP : 38 portés à 41 ; réconciliation du SDK sur le protocole filaire complet.
- Tests : de 123 à 205.

### Changed

- Hygiène CI et MSRV 1.88 ; optimisations de jetons et de latence ; cohérence de l'API et resynchronisation de la documentation ; le serveur MCP est scindé en modules par famille d'outils.
- Les 15 crates du workspace passent de `0.1.0` à `0.2.0` ; migration vers un dépôt dédié.

### Fixed

- Robustesse anti-gel : délais d'attente sur les réponses CDP, la connexion CDP, `connectToEIS` et le tube de capture KWin, afin qu'un point d'entrée bloqué ne fige plus tous les outils.

## Versions antérieures

Les versions `0.1.x` (juin 2026) n'ont pas été taguées ; leur contenu est résumé ici d'après l'historique git et [`docs/historique/`](docs/historique/README.md).

- **0.1.x (2026-06-02 au 2026-07-03)** : mise en place du workspace hexagonal ; première capture et premières interactions via CDP ; pont natif du SDK quand CDP est indisponible ; audit CI (offline-first) et `cargo deny` ; wire protocol formalisé ; jalons M0 à M7 (réconciliation du SDK, capture KWin ScreenShot2, clic droit, filtre de cible CDP configurable, outils de scénarios `wait_*`, `assert_*` et `drag_drop`, `list_windows`, interaction OS-level Wayland via KWin EIS, domaine d'orchestration, `justfile` et repli plein écran) ; audit qualité du 25 juin et durcissements.

[Non publié]: https://github.com/Morgan-Voltz/DevEye/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/Morgan-Voltz/DevEye/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/Morgan-Voltz/DevEye/compare/v0.2.0...v1.0.0
[0.2.0]: https://github.com/Morgan-Voltz/DevEye/releases/tag/v0.2.0
