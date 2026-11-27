# ADR-0063 — Dépendance optionnelle `schemars` dans `deveye-core`

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0002 (licences), ADR-0011 (stabilité), ADR-0062 (schéma `deveye.ui/1`)

## Contexte et problème

Le schéma `deveye.ui/1` (ADR-0062) doit être publié en JSON Schema pour les implémenteurs non-Rust (shim .NET, addon Godot). Un schéma écrit à la main dérive des types ; un schéma généré reste égal à eux. Générer exige `schemars`, alors que la règle 5 du `CLAUDE.md` limite `deveye-core` à `chrono`, `serde`, `serde_json` et `thiserror` : les apps qui embarquent le SDK n'ont aucune raison de payer cette dépendance.

## Facteurs de décision

- `deveye-core` est MIT et embarqué dans les apps : aucun poids ajouté par défaut ;
- le schéma publié ne doit pas pouvoir dériver des types ;
- licence compatible (`cargo deny`) ;
- `schemars` 1.x est déjà dans le graphe du serveur (via `rmcp` et `adapter-mcp-server`) : poids marginal nul côté outil.

## Options envisagées

1. **Schéma écrit à la main** — dérive garantie, aucune dépendance.
2. **Pas de JSON Schema** — se contenter des types Rust ; perd les implémenteurs non-Rust.
3. **`schemars` obligatoire dans `deveye-core`** — viole l'esprit de la règle 5 et alourdit chaque app.
4. **`schemars` optionnelle derrière une feature `schema`, désactivée par défaut** — retenue.

## Décision

Option 4 : `schemars = { version = "1", optional = true }` et `schema = ["dep:schemars"]` dans `deveye-core`. Les types du module `ui` dérivent `JsonSchema` par `cfg_attr(feature = "schema", …)`. `deveye_core::ui::json_schema()` (feature `schema`) produit le schéma ; un test compare `docs/schemas/ui-widgets-1.json` à cette sortie (égalité de valeurs JSON, donc indépendante de l'ordre des clés et de la mise en forme) et la recette `just schema` régénère le fichier.

La règle 5 du `CLAUDE.md` est amendée : « Du Rust pur sur le domaine (`chrono`, `serde`, `serde_json`, `thiserror` ; `schemars` uniquement derrière la feature optionnelle `schema` désactivée par défaut) ». Toute autre dépendance de `deveye-core` reste soumise à un ADR.

**Justification** : seule une feature optionnelle donne un schéma généré sans toucher au graphe des apps. La licence (MIT) est acceptée par `deny.toml`.

## Conséquences

**Positives**

- le schéma publié ne dérive pas ; les types restent la source ;
- aucune dépendance supplémentaire dans un build par défaut de `deveye-core`, du SDK ou d'une app ;
- aucun nouveau crate dans `Cargo.lock` (déjà présent).

**Coûts et risques**

- la feature `schema` entre dans la surface de `deveye-core` (additive, documentée dans `docs/stability.md`) ; une montée de version majeure de `schemars` est un changement majeur pour `deveye-core` (la dépendance est exposée par la feature, cf. `docs/stability.md`, « Dépendances exposées ») : elle n'est pas publiée dans une mineure ;
- le texte des doc-comments des types alimente les descriptions du schéma : ils sont rédigés pour le fil, pas pour Rust seul.

## Notes

Application : `deveye-core/Cargo.toml`, `deveye-core/src/ui.rs` (`json_schema`), `deveye-core/tests/ui_schema.rs`, `justfile` (`schema`), `CLAUDE.md` (règle 5).
