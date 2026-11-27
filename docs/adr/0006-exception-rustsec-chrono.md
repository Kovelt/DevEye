# ADR-0006 — Exception d'audit pour chrono (RUSTSEC-2026-0543)

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0008, ADR-0007 (autre exception d'audit)

## Contexte et problème

`chrono` 0.4 est déclaré non maintenu (RUSTSEC-2026-0543), sans version corrigée. Aucune faille n'est connue : seul le statut de maintenance est en cause. Or `chrono` fait partie de l'API publique de `deveye-core` (horodatages des snapshots et des événements de log) et du wire protocol : le remplacer change les types publics, donc la surface stable de la 1.0.

## Facteurs de décision

- aucune vulnérabilité connue, seulement un statut de maintenance ;
- stabilité de l'API publique de `deveye-core` dès la 1.0 ;
- coût et risque d'un remplacement à quelques jours de la release.

## Options envisagées

1. **Remplacer chrono par `time` ou `jiff` avant la 1.0** — changement cassant de l'API publique, tardif.
2. **Retirer chrono de l'API publique** — même problème.
3. **Exception datée dans `deny.toml`**, à rouvrir avant la 2.0.

## Décision

Option 3 (commit `a2951ed`) : `RUSTSEC-2026-0543` est ignoré dans `deny.toml`, avec commentaire et date (27/11/2026). Le remplacement de chrono est réservé à la 2.0.

**Justification** : un statut de maintenance ne justifie pas de casser l'API publique avant la 1.0 ; l'exception est bornée, datée et visible.

## Conséquences

**Positives**

- l'API de `deveye-core` reste stable en 1.x.

**Coûts et risques**

- une dépendance non maintenue dans le chemin des types publics : si une faille apparaît, la correction sera plus coûteuse ;
- l'exception doit être rouverte avant la 2.0 (et à chaque revue d'audit) ;
- le sujet reste ouvert : choisir le successeur (type de date) sera l'objet d'un ADR dédié.

## Notes

Statut rétabli en « Proposé » : ce choix a été fait par l'agent de la release 1.0 et aucune trace de validation par le mainteneur n'existe dans le dépôt (décisions du propriétaire du 27/11/2026 : aucune ne le couvre explicitement). Il passera à « Accepté » à la validation du mainteneur.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
