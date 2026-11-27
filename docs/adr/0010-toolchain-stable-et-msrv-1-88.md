# ADR-0010 — Toolchain stable épinglée par fichier et MSRV 1.88

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0009 (lints), ADR-0004 (bibliothèques publiables)

## Contexte et problème

Aucun `rust-toolchain.toml` n'existait : la version du compilateur dépendait de la machine. La MSRV (version minimale de Rust, `rust-version = "1.88"`) était déclarée crate par crate, avec un commentaire de CI périmé (« 1.81 »). Une toolchain récente fait apparaître de nouveaux avertissements clippy (par exemple `chunks_exact_to_as_chunks` en 1.98) qui cassent la CI `-D warnings`.

## Facteurs de décision

- environnement de développement identique pour tous (rustfmt et clippy présents) ;
- MSRV unique, vérifiée, héritée par tous les crates ;
- les bibliothèques publiables annoncent la MSRV à leurs utilisateurs ;
- ne pas figer une version de compilateur précise qui vieillirait mal.

## Options envisagées

1. **Rien** — chacun sa toolchain.
2. **Épingler une version précise** (par exemple 1.98.1) — reproductible, mais à maintenir à chaque version.
3. **Canal `stable`** dans `rust-toolchain.toml`, avec `rustfmt` et `clippy`, et MSRV 1.88 vérifiée séparément par `cargo +1.88 check --all-targets`.

## Décision

Option 3. `rust-toolchain.toml` fixe `channel = "stable"` et les composants `rustfmt` et `clippy`. La MSRV est `rust-version = "1.88"` dans `[workspace.package]`, héritée par les 17 crates ; la CI la vérifie avec `cargo +1.88 check --workspace --all-targets --locked` (la version est nommée explicitement par `+1.88`, ce qui prime sur `rust-toolchain.toml`).

**Justification** : le canal stable suit les correctifs et avertissements actuels (la CI échoue tôt plutôt que tard) et la MSRV reste une promesse explicite, testée, vers les utilisateurs des bibliothèques.

## Conséquences

**Positives**

- rustfmt et clippy toujours disponibles pour les contributeurs ;
- une MSRV, un seul endroit à modifier.

**Coûts et risques**

- une nouvelle stable peut introduire de nouveaux avertissements clippy et rougir la CI ; c'est voulu, la correction suit ;
- relever la MSRV est un changement de politique : il exige un nouvel ADR et une note de version.

## Notes

Statut rétabli en « Proposé » : ce choix a été fait par l'agent de la release 1.0 et aucune trace de validation par le mainteneur n'existe dans le dépôt (décisions du propriétaire du 27/11/2026 : aucune ne le couvre explicitement). Il passera à « Accepté » à la validation du mainteneur.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
