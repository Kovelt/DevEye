# ADR-0055 — Refus des requêtes inter-origine sur le drain des logs

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0042 (posture dev-only du SDK), ADR-0017 (protocole 1.0)

## Contexte et problème

`GET /devtools/logs/poll` vide le tampon de logs : c'est une requête GET **à effet de bord**. Le CORS refusé (ADR-0042) empêche une page tierce de lire la réponse, mais pas d'émettre la requête : un `fetch` en mode `no-cors` vers `127.0.0.1:31337` vide le tampon, et l'agent perd ses logs. La garde `Host` ne protège pas ce cas (le navigateur envoie bien `Host: 127.0.0.1:31337`).

## Facteurs de décision

- aucune page web tierce ne doit pouvoir perturber l'état du SDK ;
- les clients légitimes (adapters, `curl`, suite de conformité) ne sont pas des navigateurs ;
- ne pas changer le contrat de réponse du protocole 1.0.

## Options envisagées

1. **Rien** : le défaut subsiste, documenté.
2. **Rendre le drain `POST`** : change le protocole 1.0 déjà figé.
3. **Refuser (403) un `GET /devtools/logs/poll` portant `Origin`, ou `Sec-Fetch-Site` autre que `none`** : un navigateur envoie toujours `Sec-Fetch-Site` sur un `fetch` ; les clients non navigateur n'envoient ni l'un ni l'autre.

## Décision

Option 3, limitée à cette route : les autres routes sont sans effet de bord ou exigent un corps `application/json` (donc un préflight CORS, refusé).

**Justification** : elle ferme le vecteur sans toucher au protocole ni aux clients existants.

## Conséquences

**Positives**

- une page hostile ne vide plus le tampon ; test `logs_poll_refuses_cross_site_requests_without_draining`.

**Coûts et risques**

- un navigateur ancien sans `Sec-Fetch-Site` et sans `Origin` en GET passerait encore (risque résiduel, limité au tampon de logs) ;
- une implémentation tierce du protocole peut, au choix, reproduire ce refus.
