# Politique de stabilité et de versionnage (1.0)

Ce document dit ce que la version 1.0 de DevEye promet, ce qu'elle ne promet pas, et comment les changements sont classés. Il applique [ADR-0011](adr/0011-politique-de-stabilite-et-de-versionnage-1-0.md) ; les choix limites de son application sont dans [ADR-0012](adr/0012-application-de-non-exhaustive-et-choix-limites.md).

DevEye suit le [versionnage sémantique](https://semver.org/lang/fr/) : `MAJEURE.MINEURE.CORRECTIF`. Chaque crate publiable porte **sa propre version** (cycle de release indépendant).

## Surface stable

Tant que la version majeure ne change pas, ces éléments ne cassent pas.

| Élément | Ce qui est couvert |
|---|---|
| `deveye-core` | types publics du domaine (commandes, prédicats, snapshots, niveaux de log, descripteurs) |
| `deveye-ports` | traits des ports, `PortError`, `PortResult` |
| `deveye-sdk` | API publique embarquée dans l'app cible (`init`, `Config`, `set_state`, `expose_route`, `take_eval_requests`, constantes publiques) |
| `deveye-sdk-client` | client HTTP et constantes partagés avec les adapters |
| **Wire protocol v1** | endpoints, formes JSON et en-têtes décrits dans [wire-protocol.md](wire-protocol.md), dont `X-DevEye-Protocol-Version: 1` |
| **Outils MCP** | le nom de chaque outil, le nom et le type de chaque paramètre, la forme de chaque réponse |
| **`deveye-server`** | les options (`--target`, `--display`, `--cdp-url`, `--sdk-url`, `--screencap-window`, etc.), leurs valeurs possibles et leurs valeurs par défaut |

Les crates MIT (`deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`) sont les seuls destinés à être consommés comme bibliothèques ; ils ne dépendent jamais d'un crate AGPL ([ADR-0002](adr/0002-licences-agpl-outil-mit-bibliotheques.md)).

## Surface interne

Aucune promesse de compatibilité sur :

- les crates `adapter-*` : leur API Rust est interne (`publish = false`) ; seuls comptent les comportements qu'ils exposent à travers les ports, l'outil MCP et la CLI ;
- les modules internes de `deveye-server` (composition, façades de repli, journalisation) ;
- les messages d'erreur et de log (leur texte peut changer ; les codes et la structure d'erreur MCP, non) ;
- les éléments marqués `#[doc(hidden)]` ou non exportés.

## Ce qui est cassant, ce qui ne l'est pas

| Changement | Classe |
|---|---|
| Corriger un bug sans changer le contrat documenté | correctif |
| Ajouter un outil MCP | mineure |
| Ajouter un paramètre **optionnel** à un outil, à une option de CLI ou à un endpoint | mineure |
| Ajouter un champ à une réponse JSON (les clients ignorent les champs inconnus) | mineure |
| Ajouter une variante à un enum `#[non_exhaustive]` (`InteractionCommand`, `WaitPredicate`, `PortError`, `MouseButton`, `LogLevel`, `ConsoleLevel`) | mineure |
| Ajouter un champ à un struct `#[non_exhaustive]` | mineure |
| Ajouter une fonction, un type ou un trait par défaut implémentable | mineure |
| Ajouter une méthode **sans implémentation par défaut** à un trait de port | majeure |
| Renommer ou supprimer un outil, un paramètre, une option de CLI, un endpoint, un type ou une fonction publics | majeure |
| Rendre un paramètre optionnel obligatoire, changer son type ou la sémantique d'une valeur | majeure |
| Ajouter un champ à une variante existante d'un enum (les variantes restent constructibles par littéral) | majeure |
| Ajouter un champ à un struct **non** `#[non_exhaustive]` (`Size`, `Position`, `WindowRect`) | majeure |
| Changer la version du wire protocol | majeure |

Un consommateur Rust doit donc écrire un bras `_` dans tout `match` sur un enum `#[non_exhaustive]` et construire les structs `#[non_exhaustive]` par leurs constructeurs, pas par littéral.

## Dépréciation

1. Le symbole à retirer reçoit `#[deprecated(since = "...", note = "...")]` avec l'alternative à utiliser ; pour un outil MCP ou une option de CLI, la description l'indique.
2. Il reste disponible **au moins une version mineure** après l'annonce.
3. Il n'est retiré qu'à la **version majeure suivante**.
4. Chaque dépréciation et chaque retrait figurent au CHANGELOG.

## Version minimale de Rust (MSRV)

La MSRV est déclarée dans `[workspace.package]` (`rust-version`), vérifiée en CI, et fixée par [ADR-0010](adr/0010-toolchain-stable-et-msrv-1-88.md). La relever est un **changement mineur**, annoncé au CHANGELOG ; elle ne change jamais dans une version de correctif.

## Versionnage du protocole

Le wire protocol HTTP du SDK porte sa propre version, émise dans l'en-tête `X-DevEye-Protocol-Version` de chaque réponse. La 1.0 fige la **version 1**. Une évolution compatible (champ ou endpoint ajouté) ne change pas ce numéro ; une évolution cassante le fait passer à 2 et exige une version majeure des crates concernés. Le contrat détaillé est dans [wire-protocol.md](wire-protocol.md).

## Dépendances exposées

Certains types de dépendances apparaissent dans l'API stable, par exemple `tokio` (`Receiver<EvalRequest>`, `oneshot::Sender`) dans `deveye-sdk`, ou `serde_json::Value`. Un changement de version majeure de ces dépendances est donc un changement majeur pour le crate qui les expose.

## Ce que la 1.0 ne promet pas

- Linux seul est supporté ; les autres plateformes ne sont pas couvertes par la promesse.
- Les cibles et adapters peuvent apparaître, changer ou disparaître sans version majeure tant que les ports, les outils MCP et la CLI restent compatibles.
- Aucune publication sur crates.io n'est faite sans décision du mainteneur ([ADR-0004](adr/0004-preparer-publication-cratesio-sans-publier.md)).
