# ADR-0009 — Lints de workspace et exception unsafe de l'adapter KWin EIS

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0010 (toolchain)

## Contexte et problème

Les lints de sécurité vivaient dans l'en-tête de chaque `lib.rs` (`#![forbid(unsafe_code)]`, `#![deny(rust_2018_idioms)]`) et de façon inégale : `adapter-composite` n'avait pas `rust_2018_idioms`, `deveye-server` (un binaire, sans `lib.rs`) n'avait aucun lint, et `adapter-interaction-kwin-eis` utilisait `deny` pour un unique `unsafe`. Un nouveau crate pouvait oublier ces attributs sans que rien ne le signale.

## Facteurs de décision

- une seule définition, appliquée à chaque crate ;
- `unsafe_code` interdit partout, sauf exception unique, localisée et justifiée ;
- ne pas casser les `#![forbid(...)]` existants (défense en profondeur) ;
- pas de lint nouveau qui rougirait le code existant à quelques jours de la release.

## Options envisagées

1. **Statu quo** (attributs par crate) — inégal, oubliable.
2. **`[workspace.lints]`** avec `[lints] workspace = true` par crate ; exception pour KWin EIS déclarée dans son manifeste.
3. **Idem, avec en plus `clippy::unwrap_used`** — cohérent avec la règle « pas d'`unwrap` en production », mais exige de trier ou d'annoter tout le code existant et les tests.

## Décision

Option 2. Le workspace définit `unsafe_code = "forbid"` et `rust_2018_idioms = "deny"` ; chacun des 16 crates les active par `[lints] workspace = true`, sauf `adapter-interaction-kwin-eis` : Cargo n'autorise pas de surcharge à côté de `workspace = true`, il déclare donc `unsafe_code = "deny"` et `rust_2018_idioms = "deny"` dans son propre manifeste. Cette exception couvre l'unique `unsafe` du projet : la projection mémoire (mmap) du keymap xkb fourni par KWin dans `session.rs`, sous `#[allow(unsafe_code)]` documenté. Les attributs `#![forbid(...)]` des `lib.rs` sont conservés.

**Justification** : la règle devient structurelle (un crate sans `[lints]` se voit en revue) et l'exception est bornée et motivée ; l'option 3 est reportée pour ne pas mêler un grand nettoyage à la release.

## Conséquences

**Positives**

- `deveye-server` et `adapter-composite` sont désormais couverts ;
- l'unique `unsafe` du projet est tracé par un ADR.

**Coûts et risques**

- si l'exception KWin EIS devient inutile (dépendance qui fournit une API sûre), l'aligner sur le workspace et passer cet ADR en « Déprécié » ;
- un `unsafe` supplémentaire dans ce crate passerait inaperçu du compilateur (`deny` autorise `#[allow]`) : il reste à surveiller en revue.

## Notes

`clippy::unwrap_used` en avertissement (hors tests) est une piste ouverte pour une version 1.x.
