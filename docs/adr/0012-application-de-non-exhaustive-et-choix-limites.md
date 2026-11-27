# ADR-0012 — Application de `#[non_exhaustive]` et choix limites avant le gel

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (politique de stabilité qui prescrit `#[non_exhaustive]`)

## Contexte et problème

ADR-0011 impose `#[non_exhaustive]` sur les types publics évolutifs. Son application a demandé des choix là où l'attribut a un coût (constructeurs, `match` des consommateurs) ou un effet limité. Ces choix engagent le mainteneur au-delà du chantier : ils figent, ou non, ce qui pourra changer en version mineure.

## Facteurs de décision

- pouvoir ajouter des variantes (`resize`, nouveaux prédicats, nouveaux boutons) sans version majeure ;
- ne pas imposer de constructeur à des types dont l'exhaustivité est le contrat (géométrie) ;
- ne jamais faire un `panic` sur une valeur inconnue : refuser avec `PortError::Unsupported` ;
- limiter le coût de migration des usages internes.

## Options envisagées

1. **Tout marquer**, y compris les variantes à champs de `InteractionCommand` et `WaitPredicate` : les adapters et le serveur MCP ne pourraient plus construire ces variantes par littéral, il faudrait un constructeur par variante.
2. **Marquer les enums et les structs évolutifs, laisser les variantes constructibles** : on peut ajouter une variante, mais pas un champ à une variante existante sans casser les littéraux Rust.
3. **Ne rien marquer** et accepter les cassures : contraire à ADR-0011.

## Décision

Option 2, avec ces précisions.

- **Marqués** : enums `MouseButton`, `InteractionCommand`, `WaitPredicate`, `ConsoleLevel`, `LogLevel`, `PortError` ; structs `Screenshot`, `DomTree`, `StateDump`, `WindowInfo`, `WaitOutcome`, `ConsoleError`, `LogEvent`, `CommandDescriptor` (`deveye-core`) ; `Config`, `StatePayload`, `LogsPayload`, `EvalRequest` (`deveye-sdk`). Constructeurs ajoutés : `ConsoleError::new` et `with_location`, `WindowInfo::new`, `WaitOutcome::new`, `CommandDescriptor::new`, `Config::with_port` ; `Screenshot`, `DomTree`, `StateDump` et `LogEvent` en avaient déjà.
- **Laissés exhaustifs volontairement** : `Size`, `Position`, `WindowRect` (la géométrie est un contrat : deux ou quatre champs, sans évolution plausible) et les **champs des variantes** de `InteractionCommand` et `WaitPredicate`. Conséquence assumée : ajouter un champ à une variante existante (par exemple des modificateurs sur `Click`) est un changement cassant pour les consommateurs Rust qui construisent la variante par littéral ; il passe donc par une nouvelle variante, ou par la version majeure. Côté protocole JSON, un champ optionnel `#[serde(default)]` reste un ajout mineur.
- **Valeur inconnue** : les adapters répondent `PortError::Unsupported` (ou l'erreur d'adapter équivalente) ; le message ne reprend pas le `Debug` de la commande, qui embarquerait le texte tapé. Pour `MouseButton`, les fonctions de traduction (`button_code` et leurs équivalents) renvoient `Option`.
- **Types tokio exposés** dans `deveye-sdk` (`Receiver<EvalRequest>`, `oneshot::Sender`) : conservés tels quels. Les encapsuler casserait les intégrations existantes sans gain ; la version majeure de `tokio` fait donc partie de la surface du SDK.
- **Destructuration** : `EvalRequest` étant `#[non_exhaustive]`, un consommateur qui la destructure écrit `EvalRequest { expression, respond, .. }`.
- **Contrôle automatique** (`cargo-semver-checks` en CI) : mis en place dans le job `semver` de `.github/workflows/ci.yml` (actif une fois une version 1.x publiée, le tag `v1.*` servant de référence).

**Justification** : l'option 2 ouvre l'extension la plus fréquente (nouvelles commandes et prédicats) au moindre coût, et garde la construction littérale pour les crates internes ; l'option 1 coûterait un constructeur par variante pour un gain limité.

## Conséquences

**Positives**

- ajouter `resize` ou un prédicat ne casse plus aucun `match` externe ;
- les structs évolutifs peuvent gagner des champs sans casser le code appelant.

**Coûts et risques**

- ajouter un champ à une variante existante reste cassant (voir ci-dessus) : à connaître avant d'implémenter les modificateurs sur clics et glissers ;
- les consommateurs hors du dépôt (copies vendorées) doivent ajouter un bras `_` à leurs `match` et un `..` à leurs destructurations lors de la mise à jour.

## Notes

À valider par le mainteneur. Si la décision sur les variantes est revue, passer l'attribut aux variantes impose des constructeurs et une nouvelle migration.
