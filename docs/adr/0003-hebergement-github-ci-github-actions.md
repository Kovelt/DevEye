# ADR-0003 — Hébergement sur GitHub, intégration continue GitHub Actions, GitLab archivé

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0002 (licences, condition de l'ouverture), ADR-0004 (publication)

## Contexte et problème

Jusqu'à la 0.2, le dépôt vivait sur un GitLab privé, avec un pipeline `.gitlab-ci.yml` et des copies synchronisées vers des apps tierces. L'ouverture open source impose un hébergement public, une CI accessible aux contributeurs et un seul dépôt de référence : plusieurs sources de vérité ont déjà coûté des divergences (une ancienne ligne GitHub, disjointe, existe encore).

## Facteurs de décision

- visibilité et contribution (forks, pull requests, issues) ;
- CI gratuite et lisible pour les contributeurs externes ;
- un seul dépôt de référence, une seule CI ;
- fonction de signalement de contenu aux mainteneurs (code de conduite).

## Options envisagées

1. **Rester sur GitLab, avec un miroir GitHub** — deux CI ou une CI cachée derrière le miroir ; contributions compliquées.
2. **GitLab public seul** — moins de visibilité pour un projet Rust d'outillage.
3. **GitHub seul** : dépôt `github.com/Kovelt/DevEye`, CI GitHub Actions, GitLab archivé.

## Décision

Option 3. GitHub est la source officielle de DevEye ; la CI est écrite avec GitHub Actions (`.github/workflows/`) ; `.gitlab-ci.yml` disparaît ; les mentions GitLab (glab, MR, instance privée) des documents sont remplacées par GitHub (pull requests). Le champ `repository` des manifestes vaut `https://github.com/Kovelt/DevEye`. Le `main` GitHub sera remplacé par la lignée de la 1.0 au moment de la publication, par le propriétaire.

**Justification** : une seule source de vérité, l'écosystème où se trouvent les contributeurs Rust, et la fonction de signalement du code de conduite.

## Conséquences

**Positives**

- un dépôt, une CI, un point d'entrée pour les contributions ;
- les métadonnées Cargo pointent vers le dépôt public.

**Coûts et risques**

- migration de la CI (travail des lots suivants) ;
- l'ancienne histoire GitHub est disjointe de la lignée de la 1.0 : le remplacement du `main` est un acte destructif, à exécuter par le propriétaire seul ;
- GitLab reste en archive, en lecture seule.

## Notes

La mise en place effective du workflow est tracée par le lot de CI de la release 1.0.

Note de forme (2026-11-27) : le dépôt a ensuite été transféré du compte personnel vers l'organisation GitHub Kovelt (`github.com/Kovelt/DevEye`), car le signalement de contenu aux mainteneurs n'existe que pour les dépôts d'organisation ; voir l'ADR-0071. Les adresses citées dans cet ADR ont été mises à jour ; la décision (GitHub, GitHub Actions, GitLab archivé) est inchangée.
