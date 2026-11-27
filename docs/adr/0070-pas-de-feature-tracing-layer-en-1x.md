# ADR-0070 — Pas de feature `tracing-layer` en 1.x : compatibilité de `default-features = false`

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0066 (remplace son point « feature `tracing-layer` »), ADR-0011 (stabilité), ADR-0012 (types exposés)

## Contexte et problème

En 1.0.0, `deveye-sdk` n'avait aucune feature : une app qui déclarait `deveye-sdk = { version = "1", default-features = false }` obtenait l'API complète, dont `install_tracing_layer`. La branche 1.1 plaçait `install_tracing_layer`, `try_install_tracing_layer`, `layer()` et `DeveyeLayer` derrière une feature par défaut `tracing-layer`, avec `tracing-subscriber` optionnel. Cette app ne compilerait plus en 1.1.

Le guide SemVer de Cargo range ce cas parmi les ruptures : déplacer des éléments publics existants derrière une feature casse les utilisateurs de `default-features = false`. `cargo-semver-checks` ne le détecte pas, car il ne contrôle que les features par défaut.

## Facteurs de décision

- aucune rupture en 1.x (ADR-0011) ;
- la feature `tracing-layer` n'a jamais été publiée : la retirer ne casse personne ;
- `tracing-subscriber` est déjà exposé par `DeveyeLayer` (ADR-0012, ADR-0066) : l'avoir en dépendance normale ne change pas la surface.

## Options considérées

1. **Garder la feature par défaut `tracing-layer`** et documenter la rupture : contraire à semver, détectée par aucun outil.
2. **Retirer la feature** : `tracing-subscriber` redevient une dépendance normale, tout reste disponible sans cfg.
3. **Inverser la feature** (`no-tracing-layer`) : une feature qui retire de l'API reste non additive.

## Décision

Option 2. `deveye-sdk` n'a aucune feature en 1.x. `install_tracing_layer`, `try_install_tracing_layer`, `layer()` et `DeveyeLayer` sont toujours disponibles ; `tracing-subscriber` (0.3, `default-features = false`, feature `registry`) est une dépendance normale. Le découplage de `tracing-subscriber`, par une feature réellement additive ou par un crate séparé, est reporté à la 2.0, avec le changement de majeure de `tracing-subscriber` déjà consigné comme candidat dans `docs/stability.md`.

## Conséquences

**Positives** : un consommateur 1.0 qui déclare `default-features = false` compile en 1.1 ; plus de code conditionnel dans le SDK ni dans ses tests.

**Coûts et risques** : un hôte qui ne veut pas `tracing-subscriber` le compile quand même (dépendance légère, `default-features = false`). L'étape de CI `cargo test -p deveye-sdk --no-default-features` est conservée comme garde de compatibilité : elle échouerait si une feature retirait de l'API.
