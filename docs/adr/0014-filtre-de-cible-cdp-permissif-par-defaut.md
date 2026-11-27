# ADR-0014 — Filtre de cible CDP permissif par défaut

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (stabilité : flags de `deveye-server` dans la surface stable), ADR-0013 (neutralité)

## Contexte et problème

Sans option, `TargetFilter` retenait les pages dont l'URL ou le titre contenait « dioxus », plus `about:blank`. Ce défaut codait un nom de framework en dur : toute page hors Dioxus (SPA quelconque, tête WASM) n'était jamais attachée et obligeait chaque utilisateur à passer `--cdp-target-filter-url`. L'échec était muet : un délai de 10 s puis « timeout en attente d'une page CDP attachable », sans dire quelles pages le navigateur exposait. Le défaut devient une promesse avec la 1.0 (flags de `deveye-server`), il faut donc le fixer avant le gel.

## Facteurs de décision

- aucun nom d'app ni de framework codé en dur ;
- « ça marche sans option » dans le cas courant : une seule page utile ouverte ;
- un échec qui dit quoi corriger, notamment pour un agent qui pilote l'outil ;
- ne pas s'attacher à la page transitoire `about:blank` avant son chargement.

## Options envisagées

1. **Garder « dioxus »** : rétrocompatible, mais nom de framework codé en dur et échec muet hors Dioxus.
2. **Exiger un filtre explicite** : neutre, mais impose une option dans tous les cas, même avec une seule page.
3. **Défaut permissif (première page qui n'est pas `about:blank`) et erreur listant les cibles vues** quand rien ne correspond.

## Décision

Option 3.

- `TargetFilter::default()` est un filtre **ouvert** : sans critère d'URL ni de titre, toute page `type=page` autre que `about:blank` est acceptée ; la première rencontrée est la cible, les suivantes ne la remplacent pas.
- `about:blank` n'est accepté qu'avec `--cdp-accept-blank` (désactivé par défaut) ; une page `about:blank` qui navigue ensuite est rattrapée sur l'événement de changement d'information de cible.
- Avec `--cdp-target-filter-url` et/ou `--cdp-target-filter-title`, le comportement est inchangé (OU des critères). Le cas « aucun critère » ne signifie donc plus « refuser toutes les pages » mais « accepter toutes les pages ».
- Quand aucune page ne correspond après 10 s, l'erreur décrit le filtre et liste les pages vues (titre et URL tronqués à 60 caractères, 10 pages au plus) et propose les flags à ajuster.

**Justification** : seule cette option est neutre, fonctionne sans option dans le cas courant et rend l'échec exploitable ; l'option 2 alourdit l'usage courant, l'option 1 perpétue un nom de framework en dur.

## Conséquences

**Positives**

- plus de nom de framework dans le code ; le cas « une seule page » ne demande aucune option ;
- l'agent voit les pages disponibles et corrige son filtre sans deviner.

**Coûts et risques**

- changement de comportement pour qui comptait sur le défaut « dioxus » : avec plusieurs pages ouvertes, la première non vide est choisie, pas forcément la page Dioxus ; il faut alors poser un filtre explicite ;
- `about:blank` n'étant plus accepté par défaut, une webview qui reste sur `about:blank` demande `--cdp-accept-blank`.

## Notes

Application : `deveye-adapters/adapter-cdp-webview/src/client.rs` (filtre, suivi des pages vues, message d'erreur), `deveye-server/src/{cdp,cli}.rs` (flags), `docs/usage.md` et `docs/architecture.md`.
