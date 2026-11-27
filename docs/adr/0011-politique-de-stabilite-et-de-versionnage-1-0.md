# ADR-0011 — Politique de stabilité et de versionnage de la 1.0

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0002 (licences par crate), ADR-0004 (crates publiables), ADR-0010 (MSRV), ADR-0012 (choix limites de l'application de cette politique)

## Contexte et problème

Le numéro « 1.0 » est une promesse : tant qu'une version majeure dure, ce qui est déclaré stable ne casse pas. Aucune politique de ce genre n'existait. Rien n'indiquait ce que couvre la promesse (les bibliothèques Rust, le protocole HTTP du SDK, les outils MCP exposés aux agents, les options de la ligne de commande), ce qui reste libre d'évoluer (les adapters), ni comment on déprécie. Les enums et structs publics du domaine pouvaient être étendus sans casser le code appelant seulement par chance : une variante ajoutée à `InteractionCommand` (par exemple `resize`) cassait tout `match` exhaustif d'un consommateur. Enfin du code mort (`AppSession`, `AppKind`, `CdpTarget`, `DEFAULT_HTTP_URL`, trois `with_default_url`) élargissait la surface à promettre sans aucun usage.

## Facteurs de décision

- une promesse que le mainteneur peut tenir seul, sans l'alourdir de surfaces inutiles ;
- laisser la liberté d'ajouter (outils, commandes, prédicats) sans version majeure ;
- distinguer ce qu'un tiers consomme (bibliothèques, protocole, outils MCP, CLI) de ce qui est interne ;
- ne pas casser les applications qui embarquent le SDK ;
- cohérence avec ADR-0002 (seules les bibliothèques MIT sont réutilisées) et ADR-0004 (crates.io préparé, jamais publié sans décision).

## Options envisagées

1. **Ne rien promettre** : 1.0 de façade, cassures possibles à tout moment. Contraire au sens du mot.
2. **Tout déclarer stable**, adapters compris : surface énorme, impossible à tenir pour un seul mainteneur.
3. **Surface stable étroite et explicite** : `deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`, wire protocol v1, noms et schémas des outils MCP, flags et sous-commandes de `deveye-server` ; le reste est interne. Les types évolutifs portent `#[non_exhaustive]`.

## Décision

Option 3, avec les règles suivantes.

- **Surface stable** et **surface interne** exactement comme ci-dessus ; la règle détaillée vit dans `docs/stability.md`.
- **Semver** : ajout d'un outil MCP, d'un paramètre optionnel, d'une variante à un enum `#[non_exhaustive]`, d'une fonction publique = version mineure ; renommage, suppression, passage d'un paramètre en obligatoire, changement de type ou de sémantique = version majeure.
- **`#[non_exhaustive]`** sur les enums publics évolutifs et sur les structs à champs publics susceptibles de grandir, avec constructeurs ou builders pour les structs ; les `match` hors du crate d'origine traitent le cas inconnu par une erreur `Unsupported` explicite, jamais par un `panic`.
- **Code mort retiré avant le gel** : `AppSession`, `AppKind`, `CdpTarget`, `DEFAULT_HTTP_URL` et les trois `with_default_url` sont supprimés. Un grep en lecture seule dans les dépôts consommateurs connus n'a trouvé aucun usage dans du code applicatif ; la suppression est donc directe, sans dépréciation. Traçabilité de cette vérification : la commande était `git grep -nE "AppSession|AppKind|CdpTarget|DEFAULT_HTTP_URL|with_default_url"` lancée à la racine de chaque dépôt qui embarque une copie de `deveye-core`, `deveye-ports` ou `deveye-sdk` (les cibles de la liste de synchronisation du mainteneur, [ADR-0040](0040-fin-du-vendoring-script-de-synchronisation-de-transition.md)). Les noms de ces dépôts ne figurent pas ici, la règle de neutralité ([ADR-0013](0013-neutralite-apps-clientes-et-archivage-historique.md)) interdisant de citer une app cliente hors des archives ; le mainteneur détient la liste nominative et peut rejouer la commande. Dans le dépôt DevEye lui-même, les seules occurrences restantes sont les listes de retraits de `docs/migration-1.0.md` et l'historique `docs/historique/`.
- **Versions** : chaque crate publie sa propre version, en 1.0.0 à la release ; les adapters et le binaire `deveye-server` restent `publish = false` et sans promesse sur leur API Rust (leur contrat externe est la CLI et les outils MCP).
- **Dépréciation** : un symbole stable à retirer reçoit `#[deprecated]` avec une note, reste au moins une version mineure, et ne disparaît qu'à la version majeure suivante, annoncée au CHANGELOG.
- **MSRV** : relever la MSRV (ADR-0010) est un changement mineur, annoncé au CHANGELOG.
- **Protocole** : la version du wire protocol est portée par `X-DevEye-Protocol-Version` (`docs/wire-protocol.md`) ; la 1.0 fige la version 1.

**Justification** : seule l'option 3 est tenable et utile : elle protège ceux qui embarquent le SDK ou écrivent un adapter, laisse la place aux ajouts courants, et n'engage rien sur les détails d'implémentation.

## Conséquences

**Positives**

- une promesse lisible, vérifiable par la CI (outil de contrôle semver, mis en place avec la CI GitHub) ;
- ajouter une commande ou un prédicat ne casse plus aucun consommateur ;
- surface réduite : moins de code mort à stabiliser.

**Coûts et risques**

- un struct `#[non_exhaustive]` ne se construit plus par littéral hors de son crate : constructeurs et builders à maintenir ;
- tout `match` d'un consommateur sur ces enums doit prévoir un bras `_` ;
- toute erreur de classement (un type laissé exhaustif à tort) ne se corrige qu'à la version majeure suivante.

## Notes

Application : commits `refactor(core)`, `feat(core)` et `docs(stability)` du même lot ; la montée des versions à 1.0.0 et le CHANGELOG relèvent du lot de release.
