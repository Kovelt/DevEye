# ADR-0015 — Retrait du script de synchronisation des copies vendored

- **Statut** : Remplacé par ADR-0040
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0004 (publication crates.io préparée), ADR-0013 (neutralité)

## Contexte et problème

`scripts/sync-deveye.sh` (et la recette `just sync-deveye`) recopiaient les crates `deveye-core`, `deveye-ports` et `deveye-sdk` dans des dossiers de projets consommateurs situés sur la machine du mainteneur, via des chemins absolus. Le script nomme ces projets, écrit dans d'autres dépôts, et suppose une organisation de dossiers privée. Les bibliothèques sont désormais publiables (ADR-0004) et la décision du propriétaire est de ne toucher à aucun autre dépôt que DevEye.

## Facteurs de décision

- ne pas écrire dans d'autres dépôts depuis celui-ci ;
- neutralité (ADR-0013) : ni noms d'apps ni chemins privés ;
- une seule source de vérité : le dépôt DevEye, consommé par dépendance (crates.io ou git) plutôt que par copie.

## Options envisagées

1. **Garder le script en le rendant générique** (chemins en paramètre) : reste un mécanisme de copie à maintenir, contraire à la dépendance versionnée.
2. **Supprimer le script et la recette** ; les consommateurs dépendent des crates publiés ou d'une dépendance git.

## Décision

Option 2 : `scripts/sync-deveye.sh` et la recette `sync-deveye` du `justfile` sont supprimés.

**Justification** : la copie n'a plus de raison d'être une fois les bibliothèques publiables, et le script ne peut pas être rendu neutre sans perdre son objet.

## Conséquences

**Positives**

- plus de chemin privé ni de nom d'app dans les outils du dépôt ;
- plus de risque de dérive entre copies : une dépendance versionnée remplace la duplication.

**Coûts et risques**

- les projets qui consommaient des copies doivent migrer vers une dépendance (hors de ce dépôt, à leur rythme) ;
- le dossier `scripts/` disparaît ; tout futur script reçoit son propre ADR s'il engage une décision.
