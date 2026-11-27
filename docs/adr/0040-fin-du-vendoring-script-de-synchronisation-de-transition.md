# ADR-0040 — Fin du vendoring : dépendance versionnée, script de synchronisation de transition

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0015 (remplacé par celui-ci), ADR-0004 (crates.io préparé, jamais publié), ADR-0013 (neutralité), ADR-0002 (licences des crates copiés)

## Contexte et problème

Les apps qui embarquent le SDK n'avaient qu'un moyen de se mettre à jour : recopier `deveye-core`, `deveye-ports` et `deveye-sdk` dans leur arbre (« vendoring »), avec un script de synchronisation aux chemins codés en dur. [ADR-0015](0015-retrait-du-script-de-synchronisation-des-copies.md) supprimait ce script au motif que les bibliothèques deviennent publiables ([ADR-0004](0004-preparer-publication-cratesio-sans-publier.md)). Mais rien n'est publié sur crates.io tant que le mainteneur ne l'a pas décidé : sans script, les apps du mainteneur n'auraient aucun moyen simple de se mettre à jour. Le propriétaire a donc explicitement choisi de **garder un script de synchronisation corrigé pour la période de transition**, et de documenter la mise à jour. ADR-0015 avait été marqué « Accepté » alors qu'il contredisait ce choix.

## Facteurs de décision

- ne jamais écrire dans un autre dépôt sans que son propriétaire lance la commande ;
- aucun chemin personnel ni nom d'app dans le dépôt ([ADR-0013](0013-neutralite-apps-clientes-et-archivage-historique.md)) ;
- ne pas laisser une faute de chemin passer pour une copie à jour ;
- ne pas écraser une copie éditée à la main (les correctifs locaux ont déjà failli se perdre) ;
- viser la dépendance versionnée comme état final, pas pérenniser la copie.

## Options envisagées

1. **Supprimer le script** (ADR-0015) : propre, mais bloque les apps tant que rien n'est publié.
2. **Garder l'ancien script** : chemins personnels et noms d'apps en dur, cible absente ignorée en silence.
3. **Réécrire le script en outil de transition neutre**, déprécié d'emblée, et documenter la dépendance versionnée comme voie normale.

## Décision

Option 3. `scripts/sync-deveye.sh` est recréé, **neutre et déprécié** :

- **cibles configurables** : argument répétable `--target [ÉTIQUETTE=]DOSSIER`, ou variable `DEVEYE_SYNC_TARGETS` (entrées séparées par `:`), ou fichier de cibles (`--targets-file`, `DEVEYE_SYNC_TARGETS_FILE`, sinon `scripts/sync-targets`, ignoré par git) ; modèle versionné `scripts/sync-targets.example` avec des chemins fictifs ; aucune cible par défaut ;
- **crates copiés** : `deveye-core`, `deveye-ports`, `deveye-sdk` (les adapters et `deveye-server` ne le sont jamais) ;
- **`--check`** : code 1 en cas de dérive ; **`--dry-run`** ; **`--force`** ;
- **garde anti-écrasement** : un `VENDORED.md` consigne le commit source et un hash par crate ; une copie éditée depuis la dernière synchronisation fait abandonner le script (code 1) tant que `--force` n'est pas donné ;
- **échec explicite** (code 2) si aucune cible n'est configurée, si une cible demandée est absente, ou si `rsync` manque ;
- la recette `just sync-deveye` est rétablie.

Le script n'écrit que lorsqu'on le lance, et uniquement dans les cibles demandées ; aucun agent ne l'exécute sur les projets du mainteneur. La voie normale devient la **dépendance versionnée** : crates.io une fois les bibliothèques publiées, sinon une dépendance git sur le tag `v1.0.0` ([docs/integration.md](../integration.md)). Le script sera supprimé quand les bibliothèques seront publiées et les apps migrées.

**Justification** : c'est la seule option qui respecte à la fois le choix du propriétaire (un script pour la transition) et les règles du dépôt (neutralité, pas d'écriture implicite). La dépréciation écrite dans l'en-tête, la documentation et cet ADR évite de pérenniser le mécanisme.

## Conséquences

**Positives**

- les apps peuvent se mettre à jour avant toute publication sur crates.io ;
- plus de chemin privé ni de nom d'app dans les outils du dépôt ; une cible mal écrite échoue au lieu de passer pour « à jour » ;
- les copies éditées localement sont protégées.

**Coûts et risques**

- un mécanisme de copie reste à maintenir jusqu'à la publication des bibliothèques ;
- les crates copiés héritent de `[workspace.package]`, `[workspace.dependencies]` et `[workspace.lints]` : le workspace qui contient la copie doit déclarer les mêmes entrées (voir le `Cargo.toml` racine) ; le script n'y touche pas ;
- le script n'est pas exercé en CI (il écrit hors du dépôt) ; il a été essayé à la main sur des dossiers temporaires pour tous ses codes de sortie.

## Notes

Remplace ADR-0015, dont seul le statut change (« Remplacé par ADR-0040 »).
