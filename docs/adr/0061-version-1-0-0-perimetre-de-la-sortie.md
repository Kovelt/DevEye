# ADR-0061 — Version 1.0.0 : périmètre de la sortie

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0002 (licences), ADR-0003 (GitHub), ADR-0004 (crates.io préparé), ADR-0011 (stabilité), ADR-0037 (Linux seul), ADR-0053 (porte de qualité), ADR-0056 (versions alignées)

## Contexte et problème

DevEye passe de la série 0.x, interne et propriétaire, à une première version publique et stable. Il faut dire précisément ce que « 1.0.0 » contient et ne contient pas, comment la sortie est consignée, et ce qui reste sous la main du mainteneur (tag, publication, historique).

## Facteurs de décision

- la surface stable (ADR-0011) et le protocole 1.0 (ADR-0017) doivent être figés par un seul numéro lisible ;
- le dépôt devient public sous AGPL-3.0-or-later (outil) et MIT (bibliothèques), avec une CI GitHub seule ;
- aucune action irréversible ne doit être prise par un agent : ni tag, ni push, ni publication, ni réécriture de l'historique ;
- l'historique publiable doit avoir été scruté (secrets, adresses, chemins personnels) avant l'ouverture.

## Options envisagées

1. **Publier 1.0.0 avec Linux seul, tous les crates en 1.0.0, sans publication crates.io** — la surface promise est celle de la documentation ; les crates se publient plus tard sur décision du mainteneur.
2. **Publier d'abord une 1.0.0-rc1** — une pré-version pour laisser les consommateurs essayer ; la garde du workflow de release l'exige alors un bump préalable (ADR-0056).
3. **Rester en 0.x** jusqu'à l'arrivée d'un second consommateur — plus prudent, mais retarde l'ouverture voulue par le mainteneur.

## Décision

Option 1 : la version 1.0.0 date du 2026-11-27 ; elle porte :

- **le périmètre fonctionnel** : 44 outils MCP, cibles `auto`/`cdp`/`sdk`/`kwin-eis`/`x11`, sous-commande `doctor`, suite de conformité `deveye-conformance`, protocole SDK 1.0 ;
- **la plateforme** : Linux seul (ADR-0037) ;
- **les licences** : AGPL-3.0-or-later pour l'outil, MIT pour `deveye-core`, `deveye-ports`, `deveye-sdk` et `deveye-sdk-client` (ADR-0002) ;
- **la gouvernance** : code de conduite (ADR-0047), politique de sécurité et contribution (ADR-0059), processus ADR obligatoire (ADR-0001) ;
- **l'hébergement** : GitHub et GitHub Actions seuls (ADR-0003) ;
- **les versions** : tous les crates en `1.0.0`, y compris les versions des dépendances internes déclarées par chemin (ADR-0056 ; les manifestes des adapters et de l'outil les portent désormais aussi, pour qu'un futur `cargo package` ou une extraction d'un crate n'exige aucune édition) ;
- **le journal des modifications** : `CHANGELOG.md` au format Keep a Changelog 1.1.0 et SemVer 2.0.0, avec les rubriques Added, Changed, Fixed, Removed et Security, et les liens de comparaison GitHub en pied.

Ne font **pas** partie de la sortie : le tag `v1.0.0`, la publication sur crates.io, le `git push`, la réécriture de l'historique, l'activation du signalement de contenu sur GitHub. Ce sont des actes du mainteneur.

**Justification** : un numéro unique et une date de référence rendent la promesse de stabilité vérifiable ; laisser les actes irréversibles au mainteneur respecte la règle du dépôt (pas de push ni de commit sans demande) ; le contrôle de l'historique avant l'ouverture permet d'arbitrer en connaissance de cause.

## Conséquences

**Positives**

- une sortie lisible : un changelog, une date, un périmètre ;
- les consommateurs savent ce qui est stable (`docs/stability.md`) et ce qui ne l'est pas (API Rust des adapters) ;
- le tag peut être posé sur le commit de release sans édition de manifeste.

**Coûts et risques**

- plusieurs ADR restent au statut « Proposé » (voir l'index) : le mainteneur doit les passer en « Accepté » ou les remplacer avant de poser le tag ;
- l'historique des commits contient des adresses d'auteur et d'anciens chemins : leur traitement (conservation ou réécriture) est une décision du mainteneur, rapportée à part ;
- une pré-version `1.0.0-rcN` exigerait un bump préalable des crates (ADR-0056).

## Notes

Les ADR passés de « Proposé » à « Accepté » avant le tag ne changent pas de fond : seul leur statut et l'index sont modifiés (processus de `docs/adr/README.md`).
