# Contribuer à DevEye

Merci de votre intérêt. DevEye est maintenu par une personne ; ce guide dit comment proposer un changement pour qu'il soit relu et intégré sans allers-retours inutiles. En participant, vous acceptez le [code de conduite](CODE_OF_CONDUCT.md).

## Avant de commencer

- **Une décision passe par un ADR** (voir plus bas). Un correctif de bogue ou une amélioration de documentation n'en demande pas ; un nouvel outil, une nouvelle dépendance, un changement de contrat, de plateforme ou de licence, si.
- **Ouvrez d'abord une issue** pour tout changement non trivial (nouvel outil MCP, nouvelle cible, changement de protocole) : on y discute de l'approche avant que vous ne codiez. Des formulaires guident le rapport de bogue et la demande de fonctionnalité (`.github/ISSUE_TEMPLATE/`) ; les issues vierges sont désactivées.
- **Failles de sécurité** : pas d'issue publique, voir [SECURITY.md](SECURITY.md).

## Prérequis

- Linux ; toolchain Rust **stable** (`rustup update stable`), MSRV 1.88 ([ADR-0010](docs/adr/0010-toolchain-stable-et-msrv-1-88.md)) ; `rustfmt` et `clippy`.
- Bibliothèques de développement : voir [README](README.md#installation).
- `just` (recettes), `Xvfb` (tests de la cible x11 et de bout en bout), `cargo-deny` (audit des dépendances) ; facultatifs : `cargo-cyclonedx`, `cargo-semver-checks`.

## Commandes

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
just x11      # cible x11 contre un Xvfb privé (tests #[ignore], bloquants en CI)
just e2e      # session MCP de bout en bout
just chromium # pont d'éléments et modificateurs dans un Chromium headless (bloquant en CI)
just deny     # licences, avis RustSec, sources
just ci       # la CI en local, sauf msrv, semver et sbom (outils à part : `just msrv`, `just sbom`)
```

La machine peut être lourdement sollicitée par une compilation complète : limitez le parallélisme si besoin (`CARGO_BUILD_JOBS=4`) et ne lancez pas deux commandes cargo à la fois. Les tests `live_*` des adapters KWin exigent une session KDE Plasma Wayland réelle et se lancent à la main ; ils ne sont pas requis pour une contribution qui ne les touche pas.

## Architecture et règles du code

L'architecture est hexagonale ([ADR-0041](docs/adr/0041-architecture-hexagonale-ports-et-adapters.md)) :

```
deveye-server -> adapters (adapter-*) -> deveye-ports -> deveye-core
```

- **`deveye-core` reste pur** : aucune dépendance technique (ni `reqwest`, ni `tokio` réseau, ni CDP). `chrono`, `serde`, `thiserror` seulement.
- **Jamais un adapter ne dépend d'un autre adapter** : c'est `deveye-server` qui les compose.
- **Un crate MIT ne dépend jamais d'un crate AGPL** (`deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`).
- **Aucun crate DevEye ne dépend d'une app cliente**, et aucune logique ni aucun nom d'une app cliente n'entre dans le code ou les docs de référence ([ADR-0013](docs/adr/0013-neutralite-apps-clientes-et-archivage-historique.md)). Une heuristique reste générique ou configurable.
- **Pas de `unwrap()` ni de `panic!()`** en code de production, sauf invariant impossible justifié ; `Result` et `?`. `#![forbid(unsafe_code)]` partout (une seule exception, [ADR-0009](docs/adr/0009-lints-de-workspace-et-exception-kwin-eis.md)).
- **Erreurs** : `thiserror` dans le domaine et les ports ; `anyhow` toléré dans le binaire `deveye-server` seulement.
- **Un commentaire dit un *pourquoi* non évident**, pas un *quoi*.
- **Nouvelle dépendance Cargo** : à discuter d'abord, et consignée dans un ADR.
- Le SDK reste **dev-only** : `cfg(debug_assertions)`, `127.0.0.1` seulement, garde `Host` ([ADR-0042](docs/adr/0042-posture-dev-only-du-sdk.md)).

Pour ajouter une capacité : modéliser dans le domaine si une nouvelle entité est nécessaire, définir ou enrichir le port, implémenter l'adapter, exposer l'outil MCP, câbler dans `deveye-server`, tester, puis mettre à jour la documentation.

## Stabilité

La 1.0 promet une surface ([docs/stability.md](docs/stability.md)) : les quatre bibliothèques MIT, le protocole HTTP v1, les noms, paramètres et schémas des outils MCP, les options du binaire. Ajouter est une version mineure ; renommer, retirer ou changer la sémantique est une version majeure et passe par une dépréciation. Avant de modifier ce qui est stable, lisez la table « ce qui est cassant ». Un test échoue si la liste des outils du README ou de `docs/usage.md` diverge des outils enregistrés : ajouter un outil demande de mettre à jour `EXPECTED_TOOL_COUNT`, la classification des annotations et ces deux documents.

## Décisions : un ADR pour chacune

**Toute décision qui engage DevEye est consignée dans un ADR** sous [docs/adr/](docs/adr/README.md) : architecture, dépendance ajoutée ou retirée, licence, protocole, surface stable, plateforme, outillage, exception de sécurité ou de lint, choix entre plusieurs options pendant un chantier. L'ADR est écrit **dans le commit qui applique la décision, ou juste avant** : fichier `docs/adr/NNNN-slug-kebab.md` (numérotation continue sur quatre chiffres), gabarit et processus dans [docs/adr/README.md](docs/adr/README.md), ligne ajoutée à l'index dans le même commit. Un test vérifie que chaque ADR est indexé avec son titre et son statut. Statut « Proposé » tant que le mainteneur n'a pas validé ; on ne réécrit pas un ADR accepté : on en crée un qui le remplace.

## Commits

[Conventional Commits](https://www.conventionalcommits.org/fr) : `type(scope): sujet`.

- **Sujet** en français, à l'impératif, en minuscules, sans point final, **en ASCII sans accents**, sans émoji. Le corps est facultatif et en ASCII.
- **Types** : `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `chore`, `build`, `ci`.
- **Scope** = le crate concerné, omis si le changement est transverse :

| Crate | Scope |
|---|---|
| `deveye-core` | `core` |
| `deveye-ports` | `ports` |
| `deveye-sdk`, `deveye-sdk-client` | `sdk`, `sdk-client` |
| `deveye-server` | `server` |
| `deveye-conformance` | `conformance` |
| `adapter-cdp-webview` | `adapter-cdp` |
| `adapter-screencap-xcap` | `adapter-xcap` |
| `adapter-screencap-kwin` | `adapter-kwin` |
| `adapter-interaction-kwin-eis` | `adapter-eis` |
| `adapter-x11` | `adapter-x11` |
| `adapter-clipboard-wayland` | `adapter-clipboard` |
| `adapter-dioxus-signals` | `adapter-dioxus` |
| `adapter-logs-tracing` | `adapter-logs` |
| `adapter-commands-http` | `adapter-commands` |
| `adapter-composite` | `adapter-composite` |
| `adapter-mcp-server` | `adapter-mcp` |
| transverse | `workspace`, `docs`, `ci`, `release`, `adr` (un ADR seul) |

```
feat(core): ajoute l'entite Snapshot
fix(adapter-cdp): reconnecte la session apres un timeout WebSocket
docs(adr): consigne le choix du format de baseline
```

Pas de trailer de signature d'outil dans les messages. Un commit par sujet cohérent.

## Branches et revue

- **GitHub Flow.** `main` est toujours propre et intégrable ; tout travail se fait sur une branche dédiée issue de `main`, nommée `<type>/<slug-kebab>` (`feat/inspecteur-signaux`, `fix/cdp-reconnect-timeout`, `docs/guide-wayland`), fusionnée par demande de fusion.
- **Une demande de fusion** : suit le modèle `.github/pull_request_template.md` (cases à cocher : tests, ADR, CHANGELOG, documentation, semver, licence), décrit le *pourquoi*, renvoie à l'issue et à l'ADR concernés, et passe la CI (format, clippy, tests, documentation, Xvfb et e2e, MSRV, `cargo deny`, conformité, SDK no-op, semver, frontière MIT/AGPL et contenu des paquets).
- **Revue** : le mainteneur relit le code, les tests et la documentation ; une contribution qui change du comportement documenté met à jour la documentation dans la même demande.
- Les `unwrap`, les noms d'app cliente, les dépendances nouvelles et les changements de la surface stable sont les points sur lesquels la revue est la plus stricte.

## Licence de vos contributions

Une contribution est reçue sous **la licence du crate qu'elle touche** : MIT pour `deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client` ; AGPL-3.0-or-later pour `deveye-server`, `deveye-conformance` et les `adapter-*` ([ADR-0002](docs/adr/0002-licences-agpl-outil-mit-bibliotheques.md)). En ouvrant une demande de fusion, vous attestez avoir le droit de la proposer sous cette licence. Une contribution qui touche plusieurs crates est reçue, pour chaque fichier, sous la licence de son crate. Ne copiez pas de code sous une licence incompatible ; `cargo deny` vérifie les dépendances.

## Documentation

La documentation est en français, avec accents complets ; les identifiants de code et les termes techniques restent en anglais. Aucun nom d'app cliente en dehors de `docs/historique/` (archives datées). Les liens relatifs et les ancres sont vérifiés par un test.
