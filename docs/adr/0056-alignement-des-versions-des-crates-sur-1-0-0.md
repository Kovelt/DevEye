# ADR-0056 — Alignement des versions des crates sur 1.0.0

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (politique de stabilité et de versionnage), ADR-0033 (version affichée), ADR-0004 (publication crates.io préparée), ADR-0053 (porte de qualité de la release)

## Contexte et problème

Les 17 crates étaient en `0.2.0`. Le workflow de release compare le tag à la version de `deveye-server` et échoue sur `v1.0.0` ; `deveye-server --version` affichait `0.2.0` alors que la documentation annonce la 1.0 ; les dépendances de chemin versionnées des bibliothèques MIT (`deveye-ports`, `deveye-sdk`, `deveye-sdk-client`) visaient `0.2.0`.

## Facteurs de décision

- le tag `v1.0.0` doit passer la garde « tag = version de `deveye-server` » ;
- `cargo package` exige des dépendances de chemin avec une version compatible ;
- chaque crate garde sa propre `version` (convention du dépôt) ;
- `Cargo.lock` est suivi : le bump doit y être commité.

## Options envisagées

1. **Bumper tous les crates à 1.0.0 avant le tag** (adapters et outil compris).
2. **Bumper seulement les quatre bibliothèques MIT et `deveye-server`**, les adapters restant en 0.x (API Rust interne sans promesse, ADR-0004).
3. **Bumper au moment du tag** par un script de release.

## Décision

Option 1 : tous les crates passent à `1.0.0` dans le dépôt, avec les versions des dépendances de chemin ; le tag `v1.0.0` est posé sur le commit qui porte ce bump.

**Justification** : un seul numéro pour toute la livraison est le plus lisible ; la garde du workflow reste simple ; l'absence de promesse sur l'API Rust des adapters est documentée à part (ADR-0004), pas par un numéro de version.

## Conséquences

**Positives**

- `deveye-server --version` affiche `1.0.0` ; le tag `v1.0.0` passe la garde ;
- aucune édition de manifeste au moment de la publication.

**Coûts et risques**

- les adapters portent un `1.0.0` qui pourrait laisser croire à une API Rust stable ; leur `publish = false` et ADR-0004 le démentent ;
- la garde du workflow refuse un tag de pré-version (`v1.0.0-rc1`) tant que la version du crate vaut `1.0.0` : une pré-version exige un bump préalable à `1.0.0-rc1`.

## Notes

Ordre de publication éventuel sur crates.io (jamais lancé par la release, ADR-0004) : `deveye-core`, `deveye-ports`, `deveye-sdk`, puis `deveye-sdk-client`.
