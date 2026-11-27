# ADR-0036 — Statut du shim .NET : implémentation tierce validée par la suite de conformité

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0035 (suite de conformité), ADR-0013 (neutralité vis-à-vis des apps clientes), ADR-0011 (surface stable)

## Contexte et problème

Une app .NET peut parler à DevEye en servant le wire protocol HTTP : c'est le rôle d'un shim, aujourd'hui maintenu dans le dépôt d'un projet client, pas dans DevEye. Le plan de release se demandait s'il fallait le rapatrier, ou ajouter à la CI de DevEye un job .NET pour le tester. La suite de conformité (ADR-0035) rend la question simple : le shim est une implémentation parmi d'autres du protocole, et DevEye doit pouvoir le valider sans y toucher.

## Facteurs de décision

- DevEye ne dépend d'aucune app cliente et n'embarque pas de code qui leur appartient (règle de neutralité, ADR-0013) ;
- ne modifier aucun autre dépôt que celui de DevEye ;
- ne pas ajouter une chaîne d'outils .NET à la CI d'un dépôt Rust ;
- une preuve de conformité reproductible par le mainteneur du shim.

## Options envisagées

1. **Rapatrier le shim dans DevEye** : un second langage, une chaîne d'outils .NET et du code d'une app cliente dans le dépôt de l'outil.
2. **Job .NET dans la CI de DevEye**, qui construit le shim d'un autre dépôt et lui applique la suite : couplage de la CI de DevEye à un dépôt tiers et à son cycle de vie.
3. **Implémentation tierce, validée par la suite** : le shim reste chez son propriétaire ; DevEye fournit le contrat (spécification, schémas) et l'outil (`deveye-conformance --url`), et documente la procédure.

## Décision

Option 3. Le shim .NET est une **implémentation tierce** du wire protocol 1.0, sans statut particulier. DevEye ne le contient pas, ne le construit pas en CI et ne le modifie pas ; le dépôt qui l'héberge n'est pas touché. Sa conformité se prouve en lançant `deveye-conformance --url http://127.0.0.1:<port> --strict` contre le shim démarré (`docs/wire-protocol.md` §7), idéalement depuis la CI de son propre dépôt, à une version étiquetée de DevEye.

**Justification** : cette option respecte la neutralité des apps clientes, ne crée aucun couplage de dépôts ni de chaînes d'outils, et donne pourtant au shim une preuve objective et rejouable. Elle vaut pour toute autre implémentation future (autre langage, autre framework).

## Conséquences

**Positives**

- aucun code d'app cliente ni outillage .NET dans DevEye ;
- un contrat unique pour toutes les implémentations, vérifié de la même façon ;
- le mainteneur du shim choisit où et quand exécuter la preuve.

**Coûts et risques**

- DevEye ne garantit pas, par sa propre CI, que le shim reste conforme : la preuve est à la charge de son dépôt ;
- le shim existant n'a pas encore été passé à la suite dans le cadre de cette release (le dépôt qui l'héberge n'est pas modifié ici) : son premier passage peut révéler des écarts à corriger de son côté.

## Notes

Procédure : `docs/wire-protocol.md` §7. Les vérifications `VERS-01`, `HOST-01` à `HOST-04` et `CMD-02` couvrent les points où une implémentation tierce s'écarte le plus souvent du contrat (en-tête de version sur les erreurs, garde Host, tri des commandes) ; aucun écart du shim n'a été constaté, puisqu'il n'a pas été passé à la suite. Pour un rapport sans `SKIP` à garantir, la suite se lance avec `--strict` ([ADR-0051](0051-semantique-de-la-suite-de-conformite-skip-n-a-et-host-absent.md)).
