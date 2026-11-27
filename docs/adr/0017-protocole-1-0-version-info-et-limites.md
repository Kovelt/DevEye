# ADR-0017 — Protocole 1.0 : version, compatibilité côté client, `/devtools/info` et limites de taille

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0068 (amendement : un ajout additif ne change pas le numéro), ADR-0011 (le wire protocol fait partie de la surface stable), ADR-0016 (masquage des secrets)

## Contexte et problème

Le SDK émet `X-DevEye-Protocol-Version: 1` mais aucun client ne le lit : un SDK trop ancien, une copie vendored périmée ou une implémentation tierce produisent des erreurs de décodage obscures. La 1.0 fige le protocole ; il faut dire ce que « version » signifie, comment un client réagit à un écart, comment découvrir ce que le SDK sait faire, et borner la taille des requêtes d'un serveur local ouvert à tout processus de la machine.

## Facteurs de décision

- une règle d'incrément simple (majeure = cassant, mineure = additif) compatible avec ADR-0011 ;
- un client qui explique l'écart au lieu d'échouer au décodage, sans bloquer pour une différence bénigne ;
- l'ajout d'une route doit rester additif ;
- pas de nouvelle dépendance externe, MIT -> MIT uniquement (ADR-0002) ;
- pas de panic dans le code de production.

## Options envisagées

1. **Statu quo** (entier `1` émis, jamais lu) : aucune détection.
2. **Entier unique, refus dès qu'il diffère** : simple, mais interdit toute évolution additive sans casser les anciens clients.
3. **`MAJEURE.MINEURE`, refus sur la majeure, avertissement sur le reste**, plus une route `GET /devtools/info` pour la découverte.

## Décision

Option 3.

- **Version** : le protocole est en `1.0`, valeur de l'en-tête `X-DevEye-Protocol-Version` de chaque réponse. Majeure : changement cassant ; mineure : ajout rétrocompatible (route, champ de réponse, paramètre optionnel).
- **Client** (`deveye-sdk-client`, appelé par les adapters après chaque `send()`) : majeure différente -> `PortError::Protocol` explicite, sans panic ; en-tête absent, illisible (dont l'ancien `1` sans mineure) ou mineure différente -> avertissement `tracing`, émis une seule fois par processus, et la requête continue.
- **`GET /devtools/info`** (additive) : `sdk_version`, `protocol_version`, `capabilities` (`state`, `commands`, `info`, `redaction` toujours ; `logs` quand la couche tracing est installée ; `eval` quand une pompe est enregistrée). Soumise à la garde Host et à l'en-tête de version. `deveye-sdk-client::fetch_info` la lit ; le doctor et `target_info` s'en serviront.
- **Limite de corps** : `MAX_REQUEST_BODY_BYTES` = 1 Mio (constante publique), appliquée par `DefaultBodyLimit` ; au-delà, `413`, avec l'en-tête de version.
- **Dépendances** : `deveye-sdk-client` dépend désormais de `deveye-ports` (pour `PortError`, MIT -> MIT), de `serde` et de `tracing` (déjà dans le graphe) ; `wiremock` et `serde_json` en dev-dependencies, déjà utilisés par les adapters.

**Justification** : seule l'option 3 permet d'ajouter des fonctions sans casser les clients existants tout en détectant l'incompatibilité réelle (majeure). L'avertissement unique évite de noyer les logs d'un agent qui interroge le SDK en boucle.

## Conséquences

**Positives**

- un écart de version produit un message actionnable ; les ajouts 1.x restent permis ;
- la découverte des capacités ne passe plus par essais et erreurs ;
- un corps démesuré ne peut plus allouer sans borne dans l'app hôte.

**Coûts et risques**

- un SDK d'avant 1.0 émet `1` et n'a pas `/devtools/info` : le client avertit une fois et fonctionne ; `fetch_info` signale la route absente ;
- une mineure plus récente côté SDK avertit aussi : bruit accepté, une seule fois ;
- 1 Mio est une valeur d'agent : à relever si un usage légitime la dépasse (changement mineur, constante publique).

## Notes

Application : `deveye-sdk/src/lib.rs`, `deveye-sdk-client/src/lib.rs`, adapters `adapter-dioxus-signals`, `adapter-logs-tracing`, `adapter-commands-http` ; tests `deveye-sdk/tests/{wire_protocol,security}.rs`, `deveye-sdk-client/tests/protocol_version.rs`. Spécification : `docs/wire-protocol.md`.
