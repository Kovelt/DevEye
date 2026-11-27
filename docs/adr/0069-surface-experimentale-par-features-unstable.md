# ADR-0069 — Surface expérimentale par features `unstable-*`

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (stabilité), ADR-0012 (`#[non_exhaustive]`), ADR-0058 (critère de support, statut expérimental des plateformes)

## Contexte et problème

La feuille de route de la série 1.x introduit des capacités dont la forme n'est pas prête pour la promesse semver (références d'éléments, outils `snapshot` et `act`, schéma de scénario). Sans règle écrite, tout ajout provisoire serait couvert de fait par semver 1.x : le corriger exigerait une version majeure. Jusqu'ici « expérimental » n'existait que comme statut de plateforme (ADR-0058).

## Facteurs de décision

- livrer tôt sans figer ; ne jamais exposer une surface instable par accident ;
- une règle simple à vérifier dans le code (nom de feature) et à lire dans la documentation ;
- la décision du propriétaire pour la série 1.x : ports et outils `snapshot` et `act` hors promesse, graduation évaluée à la 1.5 ; schéma de scénario expérimental une mineure puis gelé.

## Options considérées

1. **Tout dans la surface stable, en acceptant la majeure** : bloque l'itération.
2. **Features Cargo `unstable-*` côté bibliothèques, outils marqués expérimentaux côté MCP** : exclusion explicite de la promesse.
3. **Branche ou crate séparé** : double la maintenance.

## Décision

Option 2. Une feature `unstable-*` est désactivée par défaut, peut changer ou disparaître dans toute version mineure et ne figure pas dans le tableau de `docs/stability.md`. Un outil MCP expérimental l'annonce dans sa description (préfixe « [expérimental] »), est signalé au CHANGELOG et exclu de la promesse sur ses noms, paramètres et formes de réponse. La graduation vers le stable est un ADR qui l'ajoute au tableau. La règle est écrite dans la section « Surface expérimentale » de `docs/stability.md`. Aucune surface expérimentale n'existe en 1.1.

## Conséquences

**Positives** : les ajouts provisoires des versions suivantes ont un cadre avant d'être écrits.

**Coûts et risques** : une feature `unstable-*` activée par un consommateur l'expose à des ruptures en mineure ; un test de cohérence des outils expérimentaux sera ajouté avec le premier d'entre eux.
