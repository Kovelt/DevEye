# ADR-0025 — Contrat de sortie : `captured_at` et distinction état SDK vide ou indisponible

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (la forme de chaque réponse d'outil MCP est stable), ADR-0023 (même famille de `reason` structurés), ADR-0018 (portage)

## Contexte et problème

Le domaine date chaque observation (`Screenshot`, `DomTree` et `StateDump` portent `captured_at`), mais aucun outil MCP ne le restituait : l'agent ne savait pas l'âge de ce qu'il lisait. Par ailleurs `state` ne distinguait pas deux situations d'allure voisine : un SDK **joignable qui n'a publié aucun signal** (réponse `{}`) et un SDK **injoignable** (erreur protocolaire opaque `-32603`). Un agent concluait « l'app n'a pas d'état » alors qu'elle n'était pas lancée, ou l'inverse. Ces deux points relèvent du contrat de sortie, figé par la 1.0 (ADR-0011).

## Facteurs de décision

- ne pas casser les clients qui lisent déjà la sortie (forme du premier bloc) ;
- la distinction vide / indisponible doit être lisible par un agent sans connaître les internes ;
- cohérence avec les `reason` structurés existants (`unsupported`, `js_exception`) ;
- une forme figée à la 1.0 ne se corrige plus qu'en version majeure.

## Options envisagées

1. **Envelopper** : `dom` devient `{ captured_at, tree }`, `state` devient `{ captured_at, entries }`. Contrat net, mais cassant pour tout consommateur de `dom` et `state`.
2. **Métadonnées dans `_meta`** du résultat MCP : non cassant, mais la plupart des clients ne montrent pas `_meta` à l'agent.
3. **Second bloc de contenu texte JSON**, le premier gardant sa forme : non cassant, visible de l'agent, extensible.

## Décision

Option 3, avec trois précisions.

- **Forme** : le premier bloc de `screenshot` (image), `dom` et `state` est inchangé. Un second bloc texte porte un JSON : `{ captured_at, viewport: { width, height } }` pour `screenshot`, `{ captured_at }` pour `dom`, `{ captured_at, signals, note? }` pour `state`. `captured_at` est une date RFC 3339 en UTC ; `viewport` est en pixels CSS en CDP (ADR-0022).
- **État vide** : un `state` dont le SDK répond sans aucune clé reste un **succès** avec `signals: 0` et une `note` (« SDK joignable mais aucun signal publié »). `signals` compte tous les signaux publiés, y compris quand `keys` en filtre l'affichage.
- **SDK indisponible** : une coupure de liaison (`PortError::Network`) sur `state` et `assert_state` devient un résultat `isError` `{ ok: false, reason: "sdk_unavailable", detail }` au lieu d'une erreur protocolaire. Le message d'`assert_state` distingue ainsi un SDK absent d'une divergence d'assertion.
- **Périmètre** : `screenshot_region`, `screenshot_element` et `assert_visual`, qui dérivent de la même capture, ne portent pas de bloc de métadonnées en 1.0 ; l'ajouter plus tard serait mineur.

**Justification** : l'option 3 est la seule non cassante et visible de l'agent. L'option 1 aurait été plus propre en l'absence de contrat existant ; à la veille du gel, la rupture n'est pas justifiée.

## Conséquences

**Positives**

- l'agent connaît l'âge d'un screenshot, d'un DOM ou d'un état et distingue « rien publié » de « SDK absent » ;
- aucune rupture pour les consommateurs du premier bloc.

**Coûts et risques**

- un client qui concatène tous les blocs texte avant de parser du JSON verra un second document : à lire par bloc ;
- les clés `signals`, `note`, `viewport` et `captured_at`, ainsi que `reason: "sdk_unavailable"`, entrent dans la surface stable ;
- `logs_tail`, `logs_wait`, `invoke_command` et `list_commands` dépendent aussi du SDK : leur coupure de liaison reste, en 1.0, une erreur protocolaire (extension possible, additive).
