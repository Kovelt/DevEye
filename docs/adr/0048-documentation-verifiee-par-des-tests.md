# ADR-0048 — Documentation vérifiée par des tests

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (surface stable : noms d'outils), ADR-0001 (processus des ADR), ADR-0039 (tests et preuves en CI)

## Contexte et problème

Avant la 1.0, le README annonçait quarante et un outils et n'en listait que trente-huit, promettait un outil « à venir » qui n'existait pas, et la documentation d'usage reprenait les mêmes erreurs. Une documentation exacte à la sortie ne le reste que si une dérive casse quelque chose : sinon chaque ajout d'outil la fait diverger à nouveau. Les liens relatifs et l'index des ADR dérivent de la même façon.

## Facteurs de décision

- une dérive doit faire échouer la CI, pas attendre une relecture ;
- aucune dépendance Cargo nouvelle (règle du dépôt) ;
- ne pas générer de documentation : le texte écrit à la main (exemples, conseils) a plus de valeur qu'un tableau généré ;
- un test local, rapide, sans réseau ni session graphique.

## Options envisagées

1. **Générer le README et la référence des outils** depuis le code : toujours exact, mais impose un générateur à maintenir et appauvrit la rédaction.
2. **Relecture à la main** : aucune garantie, c'est le statu quo qui a produit l'écart.
3. **Tests qui comparent la documentation au code**, sans rien générer.

## Décision

Option 3, dans `adapter-mcp-server` (module de tests `docs_tests`), sans dépendance nouvelle :

- la liste des outils du README, encadrée par `<!-- outils:debut -->` et `<!-- outils:fin -->`, est **égale** à l'ensemble des outils enregistrés par le serveur ;
- chaque outil enregistré a un titre `#### \`nom\`` dans `docs/usage.md`, et aucun titre ne cite un outil inconnu ;
- tout nombre d'outils annoncé (« N outils ») dans le README, `docs/usage.md` et le guide de migration égale `EXPECTED_TOOL_COUNT` ;
- les **liens relatifs** et leurs **ancres** des fichiers Markdown du dépôt résolvent (hors `docs/historique/`, archives datées non normatives) ;
- chaque ADR existant figure à l'index de `docs/adr/README.md` avec son titre et son statut, et la numérotation est continue.

**Justification** : seul un test rend l'exactitude durable sans générateur. Il vit dans le crate qui enregistre les outils, seul à pouvoir lire la liste réelle sans dépendance. Le coût est de mettre à jour quelques lignes de documentation quand un outil change, ce que la règle « la doc décrit exactement le code » exige de toute façon.

## Conséquences

**Positives**

- ajouter, retirer ou renommer un outil sans mettre à jour la documentation fait échouer la CI ;
- les liens cassés et les ADR oubliés dans l'index sont détectés ;
- l'exactitude du README, vitrine de la 1.0, ne dépend plus de la vigilance.

**Coûts et risques**

- le test ne vérifie que l'**existence** et la **liste** : il ne dit pas que les paramètres ou les retours décrits sont justes (ils sont relus à la main contre le code) ;
- l'analyse des liens et des ancres est volontairement simple (règle d'ancre de GitHub, liens en ligne) : une syntaxe Markdown plus exotique pourrait lui échapper ou la faire échouer à tort ;
- le test lit des fichiers hors du crate (`../..`) : il ne fonctionne qu'à l'intérieur du dépôt, pas depuis une archive du seul crate (qui n'est pas publié).

## Notes

« Proposé » : choix d'outillage fait pour exécuter la tâche de documentation, à valider par le mainteneur.
