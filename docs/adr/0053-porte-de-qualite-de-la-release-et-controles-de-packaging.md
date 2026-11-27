# ADR-0053 — Porte de qualité de la release et contrôles de packaging

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0002 (licences par crate), ADR-0003 (CI GitHub Actions), ADR-0004 (crates.io sans publier), ADR-0038 (distribution)

## Contexte et problème

La revue de la branche de release a relevé quatre trous d'outillage :

1. `release.yml` se déclenchait sur tout tag `v*` sans dépendre de la CI : un tag posé sur un commit rouge produisait une archive ; une pré-version n'était pas marquée comme telle sur GitHub ;
2. dans `ci.yml`, le job semver prenait le « dernier » tag `v1.*` par tri `v:refname`, qui place `v1.0.0-rc1` après `v1.0.0`, et interpolait la sortie d'étape directement dans le script shell ;
3. l'invariant « un crate MIT ne dépend jamais d'un crate AGPL » (ADR-0002) n'était contrôlé par aucun outil : `cargo deny` accorde l'exception AGPL par nom de crate, sans regarder le sens des dépendances ;
4. aucun contrôle ne vérifiait que les quatre crates publiables embarquent leur `LICENSE-MIT` (une publication crates.io est irréversible).

## Facteurs de décision

- une release ne doit pas pouvoir sortir d'un commit qui échoue à la CI ;
- aucune entrée non maîtrisée (nom de tag) ne doit être interprétée par un shell ;
- les invariants de licence doivent être vérifiés par une machine, pas par la mémoire du mainteneur ;
- ne rien publier : les contrôles restent en lecture seule (ADR-0004).

## Options envisagées

1. **Rien** : les trous subsistent.
2. **Porte de qualité dans `release.yml`** (format, clippy, tests, `cargo deny`, frontière de licences) en prérequis de la construction, filtre de tags sur la forme `vX.Y.Z[-suffixe]`, drapeau `--prerelease` pour les suffixes, baseline semver restreinte aux versions finales passée par `env:`, script `scripts/verify-license-boundary.sh` (`cargo metadata` + `jq`) et script `scripts/verify-packages.sh` (`cargo package --list`) exécutés par un job `packaging` de la CI.
3. **Exiger la CI verte par l'API GitHub** depuis le workflow de release : plus léger en minutes de calcul, mais dépend d'un état externe et d'un jeton supplémentaire.

## Décision

Option 2.

**Justification** : la porte rejoue exactement les contrôles bloquants, sans dépendre d'un état distant ; les deux scripts sont lisibles, exécutables en local (`just license-boundary`, `just packages`) et testés sur un cas de violation ; `cargo package --list` suffit à vérifier le contenu sans publier ni exiger que les dépendances du workspace soient déjà sur crates.io.

## Conséquences

**Positives**

- un tag sur commit rouge ne produit plus d'archive ;
- les pré-versions sont reconnues et ne faussent plus la référence semver ;
- la frontière MIT/AGPL et la présence de `LICENSE-MIT` dans les paquets sont vérifiées à chaque CI.

**Coûts et risques**

- la release rejoue les tests (une quarantaine de minutes de CI en plus) ;
- `jq` est requis par le script de frontière (présent sur les runners GitHub) ;
- `cargo package --list` ne vérifie pas la compilation du paquet (qui exigera les crates déjà publiés dans l'ordre core, ports, sdk-client, sdk) : ce contrôle reste à faire à la main avant une publication.

## Notes

Application : `.github/workflows/ci.yml` (jobs `semver`, `packaging`), `.github/workflows/release.yml` (job `quality`), `scripts/verify-license-boundary.sh`, `scripts/verify-packages.sh`, `justfile`.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
