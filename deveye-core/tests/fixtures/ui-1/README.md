# Fixtures `deveye.ui/1`

Fichiers versionnés pour les implémenteurs tiers (shim .NET, addon Godot…) et pour les tests de `deveye-core`.

| Préfixe | Attendu du lecteur (`UiTree::from_value`) |
|---|---|
| `valid-` | lu sans erreur, `validate()` sans écart bloquant |
| `legacy-` | format d'avant le schéma : lu et converti, `validate()` sans écart bloquant |
| `invalid-` | lu, mais `validate()` relève au moins un écart bloquant |
| `unsupported-` | refusé (`UiReadError`) |

## Limite connue

Les tests du dépôt lisent ces fixtures avec `UiTree::from_value` et `validate()` ;
ils ne les valident pas contre `docs/schemas/ui-widgets-1.json` (aucune dépendance de
validation de JSON Schema n'est autorisée en 1.x). Un implémenteur tiers peut le faire
avec le validateur de son langage.
