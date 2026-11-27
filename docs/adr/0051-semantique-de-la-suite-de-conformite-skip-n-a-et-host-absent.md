# ADR-0051 — Sémantique de la suite de conformité : `SKIP`, `N/A`, `Host` absent

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0035 (suite de conformité), ADR-0036 (shim .NET, `--strict`), ADR-0017 (protocole 1.0)

## Contexte et problème

La revue de la suite a montré que le mode `--strict`, recommandé pour valider une implémentation tierce (ADR-0036), est **inatteignable** pour une implémentation conforme sans capacité `redaction` (`SEC-01` est ignorée) ; que `SIZE-01` échoue si le serveur coupe la connexion avant le `413`, ce que §2.5 permet ; que `HOST-03` exige `403` alors que la RFC 9112 impose `400` à un serveur HTTP/1.1 devant une requête sans `Host` (Kestrel refuse avant tout middleware) ; que `LOGS-03` réussit sur un tampon vide, sans rien observer ; que le rapport affichait « CONFORME » avec un code de sortie 1 en `--strict` ; et que les messages d'échec citaient 80 caractères de corps de réponse, donc des données de l'app dans des journaux de CI.

## Facteurs de décision

- `--strict` doit pouvoir réussir pour toute implémentation conforme ;
- ne jamais conclure « conforme » sur une exigence non observée ;
- ne pas imposer à une implémentation ce que sa pile HTTP interdit ;
- aucune donnée de l'app testée dans les rapports.

## Options envisagées

1. **Statu quo** : `--strict` inutilisable, faux négatifs.
2. **Deux issues distinctes** : `SKIP` (non observable : l'exigence reste non vérifiée, `--strict` l'exige) et `N/A` (capacité non annoncée : sans objet, jamais bloquant) ; `HOST-03` accepte `400` de la pile HTTP ; `SIZE-01` devient `SKIP` sur connexion coupée ; `LOGS-03` `SKIP` sur tampon vide ; verdict « INCOMPLET » quand des `SKIP` subsistent ; corps jamais cités (taille seulement).
3. **Retirer `--strict`** : l'agent ne pourrait plus exiger une validation complète.

## Décision

Option 2 : `Outcome::NotApplicable` est ajouté, le §2.2 de `docs/wire-protocol.md` admet le `400` de la pile HTTP pour un `Host` absent, et le §7 documente la nouvelle sémantique.

**Justification** : elle rend `--strict` atteignable sans rien relâcher de l'observation, aligne le protocole sur la RFC et protège les journaux.

## Conséquences

**Positives**

- une implémentation sans `redaction` ni `eval` peut passer `--strict` ;
- le verdict ne ment plus (`CONFORME`, `NON CONFORME` ou `INCOMPLET`) ;
- aucun extrait de réponse dans les rapports.

**Coûts et risques**

- le §2.2 du protocole est assoupli (un `400` pour `Host` absent) : un client ne doit pas supposer que la garde de l'application répond toujours ;
- `Outcome` gagne une variante : sans conséquence tant que la suite n'est pas une API stable (crate AGPL, hors surface semver, ADR-0011).

## Notes

Application : `deveye-conformance/src/{checks,report,client}.rs`, `docs/wire-protocol.md` (§2.2, §3.0, §6, §7). Le SDK Rust passe la suite à chaque `cargo test` en build debug seulement (no-op en release).

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
