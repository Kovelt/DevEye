# ADR-0068 — Capacités additives et version du protocole

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (stabilité), ADR-0017 (protocole 1.0 et `/devtools/info`), ADR-0065 (capacité `ui.widgets/1`), ADR-0066 (capacité `logs`)

## Contexte et problème

La revue de la 1.1 relève trois sources qui se contredisent. `docs/wire-protocol.md` écrivait qu'ajouter une route, un champ ou une capacité est une « mineure du protocole » (`1.1`), `docs/stability.md` disait à la fois que « la mineure augmente à chaque ajout » et qu'un ajout « ne change pas ce numéro », et les ADR-0065 et ADR-0066 posent que `protocol_version` reste `1.0` quand une capacité est ajoutée. Le code émet `1.0` partout (en-tête, `/devtools/info`, vérification `VERS-01`). Aucun ADR n'amendait ADR-0017.

## Facteurs de décision

- un client doit pouvoir détecter une fonctionnalité sans deviner d'après un numéro : c'est le rôle de `capabilities` ;
- `deveye-sdk-client` compare la majeure et n'avertit que si la mineure diffère : incrémenter la mineure à chaque ajout ferait avertir tous les clients 1.0 devant un SDK 1.1, sans qu'aucun contrat ne soit rompu ;
- un tiers (.NET, Godot) implémente le protocole : une règle unique évite qu'il fige un numéro erroné.

## Options considérées

1. **Incrémenter la mineure du protocole à chaque ajout** (`1.1`) : exige de changer l'en-tête, `/devtools/info`, `VERS-01` et d'accepter l'avertissement de mineure chez les clients 1.0.
2. **Le numéro ne change que pour une rupture ; les ajouts sont annoncés par `capabilities`** : le code actuel est déjà conforme.

## Décision

Option 2. `protocol_version` reste `1.0` pour toute la série 1.x ; une route de commande, un champ de réponse, un champ optionnel de requête ou une capacité ajoutés sont additifs et se découvrent par `capabilities` de `GET /devtools/info`. Le numéro ne change que pour une rupture (majeure `2.0`). « Ajoutée en 1.1 » dans la documentation désigne la version du SDK, pas celle du protocole. Les trois sources (`wire-protocol.md`, `stability.md`, ADR-0017 par cet amendement) sont alignées.

## Conséquences

**Positives** : aucun changement de code, aucun avertissement de mineure chez les clients 1.0 ; une règle unique.

**Coûts et risques** : la mineure du protocole ne porte plus d'information en 1.x ; un client qui veut savoir si un serveur sert `ui.widgets` doit lire `capabilities`, jamais le numéro. Choix qui engage le mainteneur au-delà de la 1.1 (statut Proposé).

Note de validation : le mainteneur a validé cette décision le 2026-11-27 ; le statut passe de « Proposé » à « Accepté », le fond est inchangé.
