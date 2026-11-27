# ADR-0042 — Posture dev-only du SDK : debug seulement, loopback, garde Host, CORS

- **Statut** : Accepté
- **Date** : 2026-06-02
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0016 (masquage des secrets), ADR-0017 (protocole 1.0 et limites), ADR-0039 (preuves dev-only en CI), ADR-0041 (architecture)

## Contexte et problème

Pour qu'un agent voie l'état et les logs d'une app, l'app cible embarque un SDK qui ouvre un petit serveur HTTP local. C'est un canal d'inspection et d'évaluation de code JavaScript : s'il fuitait dans un binaire de production, ou s'il était joignable depuis le réseau ou depuis une page web quelconque, ce serait une porte ouverte sur l'app et sur la machine du développeur.

## Facteurs de décision

- aucune trace de DevEye dans un binaire de production, même si l'intégrateur oublie une précaution ;
- le serveur n'est joignable que depuis la machine ;
- une page web ouverte dans le navigateur du développeur ne peut pas l'atteindre (attaques par rebond DNS, requêtes inter-origines) ;
- un contrat simple, implémentable dans un autre langage sans bibliothèque de sécurité.

## Options envisagées

1. **Authentification par jeton** : protège aussi contre d'autres processus locaux, mais impose un secret à distribuer entre l'app et le serveur, et une complexité de plus pour toute implémentation tierce.
2. **Protection par construction** : n'exister qu'en build de débogage, n'écouter que sur la boucle locale, refuser tout en-tête `Host` non local, refuser toute origine inter-origine.
3. **Aucune protection** : acceptable seulement si l'outil n'est jamais lancé qu'isolé ; ce n'est pas le cas d'un poste de développement.

## Décision

Option 2, posée dès l'origine pour le SDK (`cfg(debug_assertions)`) et complétée à la formalisation du protocole (24-25 juin 2026) :

- **debug seulement** : le SDK n'est destiné à être ajouté à l'app cible que sous `cfg(debug_assertions)` ; en plus, `deveye_sdk::init` est un **no-op en build release**, de sorte qu'un oubli de l'intégrateur n'ouvre aucun port ;
- **boucle locale uniquement** : le serveur lie `127.0.0.1`, jamais `0.0.0.0` ; le port par défaut est `31337`, surchargeable par `DEVEYE_PORT` ;
- **garde Host** : toute requête dont l'en-tête `Host` n'est pas `localhost`, `127.0.0.1` ou `[::1]` (port optionnel) reçoit `403`, avant toute route ; le lien loopback seul ne suffit pas contre le rebond DNS ;
- **CORS refusé** : aucune politique inter-origine permissive ;
- **bornes** : corps de requête plafonnés (ADR-0017), file d'évaluation bornée.

**Justification** : ces garde-fous sont indépendants du langage, sans secret à gérer, et vérifiables par des tests (les tests de sécurité du SDK et la preuve sur binaire release, ADR-0039). Ils couvrent le risque réaliste (fuite en production, page web hostile) ; le jeton (option 1) ne couvre que le cas d'un autre processus local de confiance égale, au prix d'un contrat plus lourd.

## Conséquences

**Positives**

- impossible d'expédier le SDK actif par accident ;
- inaccessible depuis le réseau et depuis une page web ;
- contrat HTTP minimal et implémentable ailleurs (docs/wire-protocol.md).

**Coûts et risques**

- **tout processus local** de l'utilisateur peut parler au SDK sur la boucle locale : le modèle de menace suppose un poste de développement de confiance (SECURITY.md) ;
- le masquage des secrets n'est qu'un filet par noms de clés (ADR-0016) ;
- le SDK n'est pas conçu pour des machines partagées.

## Notes

ADR rétroactif. Date : la restriction `cfg(debug_assertions)` figure dès le premier commit de l'historique (`c88e0ea`, 2026-06-02) ; la garde Host, le refus CORS et le contrat HTTP ont été formalisés le 2026-06-24 (`57b4c14`) et implémentés le 2026-06-25 (`da90975`). Voir [docs/wire-protocol.md](../wire-protocol.md) §2.
