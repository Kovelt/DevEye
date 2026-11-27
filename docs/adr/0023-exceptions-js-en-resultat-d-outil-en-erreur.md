# ADR-0023 — Exceptions JS en résultat d'outil en erreur (`PortError::Script`)

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (surface stable : `PortError` est public), ADR-0012 (`PortError` est `#[non_exhaustive]`), ADR-0018 (portage depuis l'ancienne lignée)

## Contexte et problème

Une exception levée par l'expression d'`eval_js` (`ReferenceError`, `throw`…) remontait comme une erreur JSON-RPC `-32603` opaque, indistinguable d'une panne du serveur. Or c'est un résultat que l'agent doit lire et corriger : son expression est fausse, la liaison fonctionne. De plus, côté CDP le message se réduisait souvent à « Uncaught » : une `Error` n'a pas de `value`, son texte et sa trace sont dans `description`, que le code ignorait. Côté SDK, l'exception arrivait sous `PortError::Render`, donc sous une autre variante que côté CDP. L'ancienne lignée GitHub renvoyait l'exception en `isError`.

## Facteurs de décision

- l'agent doit voir le message et la trace pour corriger son expression ;
- une panne de liaison doit rester distincte d'une exception (le repli CDP vers SDK ne doit pas se déclencher sur une exception) ;
- les adapters doivent parler d'une seule voix : les mêmes cas, la même variante ;
- la surface des ports est figée par la 1.0 : toute addition doit être non cassante.

## Options envisagées

1. **Reconnaître l'exception au texte** de l'erreur (`« exception »`, `« Uncaught »`) dans le serveur MCP : fragile, dépend de la langue et du moteur.
2. **Nouvelle variante `PortError::Script { message, stack }`** : explicite, non cassante (`PortError` est `#[non_exhaustive]`), produite par chaque `JsEvaluator`.
3. **Faire porter l'exception par le `Result::Ok`** de `evaluate` (`Ok(Err(..))`) : change la signature du port.

## Décision

Option 2.

- `PortError::Script { message: String, stack: Option<String> }` (affiché « exception JS : … »). `message` est la première ligne de la description de l'exception, ou la valeur levée, ou à défaut le texte générique ; `stack` est la description complète quand elle a plusieurs lignes, sinon absent.
- L'adapter CDP lit `exceptionDetails.exception.description` ; l'évaluateur du pont SDK produit `Script` pour les erreurs JS (statut 200 avec `error` uniquement ; tout autre statut inattendu, dont le 403 de la garde Host, est un `Protocol`, ADR-0054), `Network` et `Timeout` restant réservés aux pannes de liaison.
- Le serveur MCP rend `Script` en résultat `isError` structuré `{ ok: false, reason: "js_exception", message, stack? }`. Le traitement est dans la fonction commune des outils qui passent par un évaluateur (`unsupported_aware`) : il vaut pour `eval_js`, mais aussi `click_selector`, `find_element`, `assert_selector`, etc., dont le JS peut lever. Une panne de liaison reste une erreur protocolaire.
- Le repli de la façade `auto` ne bascule que sur `PortError::Network` : une exception JS ne le déclenche pas.

**Justification** : l'option 2 est la seule explicite et non cassante. Elle donne la même sémantique sur CDP et sur le pont SDK, et ne dépend d'aucun texte.

## Conséquences

**Positives**

- l'agent corrige son expression d'après le message et la trace, au lieu d'un `-32603` sans détail ;
- le message CDP « Uncaught » est remplacé par le vrai texte de l'erreur.

**Coûts et risques**

- une variante de plus dans une énumération publique : un `match` externe sur `PortError` a déjà un bras `_` (ADR-0012), mais une implémentation externe de `JsEvaluator` qui ne produit pas `Script` garde l'ancien comportement (erreur protocolaire) ;
- la trace du pont SDK est absente (le SDK ne renvoie qu'un texte) : `stack` est alors omis ;
- `reason: "js_exception"` rejoint `unsupported` parmi les valeurs de `reason` figées.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
