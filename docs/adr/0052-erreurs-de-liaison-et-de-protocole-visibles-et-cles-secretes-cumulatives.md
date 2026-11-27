# ADR-0052 — Erreurs de liaison et de protocole visibles, clés secrètes cumulatives

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0016 (masquage des secrets par clés), ADR-0017 (version du protocole), ADR-0042 (posture dev-only du SDK)

## Contexte et problème

Trois défauts de visibilité et de sûreté ont été relevés à la revue :

1. un port déjà pris faisait seulement émettre un `tracing::warn!` : `init` s'exécute avant l'installation de la couche `tracing` du SDK, donc le message était perdu et l'agent parlait à la mauvaise app sans le savoir ;
2. un second `init(Config::default())` **remplaçait** les clés de `with_secret_keys` du premier appel, alors que le premier serveur servait encore le même registre : les clés applicatives étaient démasquées ;
3. dans `adapter-composite`, une `PortError::Protocol` issue du SDK (majeure du protocole incompatible) était absorbée comme « pas encore vrai » : `wait_for` expirait sans message actionnable.

## Facteurs de décision

- un échec de configuration ne doit jamais être silencieux ;
- un second `init` ne doit jamais réduire la protection ;
- une incompatibilité de protocole ne s'arrange pas en réessayant.

## Options envisagées

1. **Rien** : les défauts subsistent.
2. **stderr en plus de `tracing`**, **ensemble de clés cumulatif**, **propagation de `Protocol` pour les prédicats adossés au SDK** (`state-key`, `log-message`) tout en gardant l'absorption pour les prédicats CDP (où `Protocol` recouvre des erreurs transitoires de la page).
3. **Refuser un second `init`** : plus strict, mais casse l'usage du port de repli et des tests.

## Décision

Option 2.

**Justification** : le message sur stderr est visible sans abonné `tracing` ; l'accumulation est monotone (on ne retire jamais une clé secrète) ; la propagation sépare l'erreur persistante (protocole SDK) de l'erreur transitoire (page CDP).

## Conséquences

**Positives**

- port pris signalé immédiatement sur stderr, avec le moyen d'en choisir un autre ;
- aucune clé secrète perdue entre deux `init` ;
- l'incompatibilité de version remonte à l'appelant avec son message.

**Coûts et risques**

- les clés ne peuvent plus être retirées sans redémarrer l'app (acceptable : le masquage va dans le sens de la prudence) ;
- l'outil `target_info` n'expose pas encore la version du protocole de l'**app** (seul `deveye-server doctor` la lit via `fetch_info`) : à traiter dans un chantier distinct.

## Notes

Application : `deveye-sdk/src/lib.rs` (`bind_or_report`, `init`), `deveye-adapters/adapter-composite/src/lib.rs` (`absorb_transient_sdk`), avec tests unitaires.
