# ADR-0074 — Pont JS d'éléments pour WebKitGTK : références `e<n>` sans CDP

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0072 (modèle de références, `RefTable`), ADR-0073 (outils `snapshot` et `act`, relocalisation à l'action), ADR-0069 (surface expérimentale), ADR-0023 et ADR-0024 (pompe d'évaluation du SDK, erreurs), ADR-0064 (secrets)

## Contexte et problème

Sous Linux, seul système supporté en 1.x, le webview d'une app Dioxus desktop est WebKitGTK, **sans CDP**. Le chemin réellement utilisé pour piloter ces apps est le pont du SDK : `SdkEvaluator` évalue du JavaScript dans la page par la pompe `document::eval` (`POST /devtools/eval`). Les lots précédents ont livré le modèle (ADR-0072) et les widgets natifs (ADR-0073) ; les pages web restaient sans références (`unsupported`). Il faut désigner un élément du DOM par une référence stable avec ce seul outil : une expression JavaScript évaluée de façon **synchrone** (la pompe ne fait pas d'`awaitPromise`).

## Facteurs de décision

- aucun CDP, aucune dépendance nouvelle, aucun changement du protocole du SDK (version 1.0, ADR-0068) : on n'ajoute que du JavaScript envoyé par le canal d'évaluation existant ;
- les références ne doivent jamais être réutilisées, même après un rechargement de page (pas de rebind silencieux, ADR-0072) ;
- le script doit se tester sans app : dans un Chromium headless, par `Runtime.evaluate`, il est évalué exactement comme par la pompe ;
- jamais de valeur pour un champ reconnu comme secret (filet heuristique, voir « Secrets » : la page n'a pas de notion de secret, seulement des indices).

## Options considérées

1. **Parcourir le DOM côté Rust après `outerHTML`** : le HTML brut ne dit ni le rect, ni l'état calculé, ni le rôle ; aucune clé de nœud vivant. Rejeté.
2. **Script avec compteur global dans la page, références rendues telles quelles** : un rechargement remet le compteur à zéro, donc `e1` désignerait un autre bouton. Rejeté.
3. **Script qui donne des clés `<epoch>:<n>`, table de références côté serveur** : retenu.
4. **Attendre l'arbre d'accessibilité CDP et ne rien livrer pour WebKitGTK** : rejeté, c'est la cible réelle sous Linux.

## Décision

**Le script** (`adapter-dioxus-signals/src/elements_bridge.js`, embarqué par `include_str!`) est une expression JavaScript sans dépendance, qui s'installe dans `window.__deveye_elements__` et expose `snapshot(query)` et `locate(key)`. Un `WeakMap` associe l'élément à un numéro, une `Map` le numéro à un `WeakRef` (repli sans `WeakRef` : référence forte) ; un `FinalizationRegistry` nettoie. La clé rendue est `<epoch>:<n>` : l'epoch est tirée à chaque chargement de page.

**Côté Rust**, `BridgeElementInspector` implémente `ElementInspector` au-dessus d'un `JsEvaluator` et traduit les clés en `e<n>` par la `RefTable` d'ADR-0072 : un même nœud garde sa référence d'un instantané au suivant (y compris déplacé dans le DOM), un nouveau nœud reçoit un nouveau numéro, un numéro n'est jamais réutilisé. Un changement d'epoch (page rechargée, ou backend différent) appelle `invalidate` : les anciennes références deviennent périmées et la `generation` augmente. Le script n'est envoyé qu'à la première utilisation (l'appel court répond `__install`), puis seulement après un rechargement.

**Parcours** : rôle implicite par une table minimale (rôle ARIA explicite prioritaire), nom accessible simplifié (`aria-labelledby`, `aria-label`, `<label>`, `alt`, valeur d'un bouton `input`, contenu borné à 120 caractères, `title`, `placeholder`), états (`disabled`, `checked`, `expanded`, `focused`, `selected`), rect `[x, y, l, h]` en pixels CSS du viewport (le repère des clics de `JsInteraction`), `scrollable`, `offscreen`. En mode `interactive_only` : éléments interactifs, titres et points de repère, conteneurs (listes, menus, groupes) seulement s'ils portent des éléments gardés. Une zone sans rôle mais au curseur « main » (non hérité du parent) ou au `tabindex` positif devient `clickable`. `display: none`, `aria-hidden`, `<details>` fermé et `visibility: hidden` sont exclus. Les shadow roots ouverts sont traversés ; **les iframes ne le sont pas** (limite documentée).

**`locate` remesure et agit en humain** : si l'élément est hors vue ou masqué par un conteneur qui défile, il est amené au centre par `scrollIntoView` (sans animation), puis remesuré. Au centre remesuré, `elementFromPoint` (shadow roots traversés) dit si un autre élément le recouvre : `Obscured { by }`, la référence du couvrant. `disabled` (`:disabled` ou `aria-disabled`) est rendu à `act`, qui refuse sans émission.

**Périmé** : une clé morte ou détachée donne `Stale` avec pour `candidates` les éléments vivants de même rôle et de même nom, tirés d'un instantané frais ; la référence est oubliée par la table et reste périmée (pas de rebind).

**Secrets** : le script ne rend pas la valeur d'un champ reconnu comme secret. Indices, par ordre : l'attribut `data-deveye-secret` (sur le champ ou un ancêtre, sauf valeur `false`) ; `input[type=password]` ; un champ vu un jour en `password` (un `WeakSet` retient les champs qu'un bouton « afficher » bascule en `text`, pour autant qu'un instantané les ait vus avant) ; un `autocomplete` `current-password`, `new-password`, `one-time-code` ou `cc-*` ; un `name`, `id`, `aria-label` ou `placeholder` qui satisfait les règles de noms de clés d'ADR-0064 (mêmes termes que `docs/schemas/shared-vectors-1.json`, plus quelques termes de paiement) ; un test dans Chromium vérifie chaque clé requise des vecteurs partagés et qu'aucune clé ordinaire n'est masquée. C'est un **filet, pas une garantie** (SECURITY.md) : un champ sans aucun de ces indices est lu. Le masquage vit dans le script et pas côté Rust, parce que seule la page connaît les attributs du champ ; le nom accessible que reçoit le serveur n'est pas un indice fiable. Les autres valeurs sont tronquées à 200 caractères.

**Câblage** (`deveye-server`) : `--target sdk` compose le pont sur `SdkEvaluator` ; `--target auto` le compose sur l'évaluateur de repli (CDP tant qu'il répond, sinon la pompe) : une bascule de backend ne change **pas** d'epoch : CDP (`Runtime.evaluate`, monde principal) et la pompe `document::eval` exécutent dans la même page, donc le même `window.__deveye_elements__` ; les références restent valides d'un backend à l'autre (rectifié après revue ; seul un rechargement de page change d'epoch, test `bridge_chromium`). `--target cdp` reste `unsupported` avec une marche à suivre (l'arbre d'accessibilité CDP viendra, et `FallbackElementInspector` lui est réservé). `target_info.elements` est vrai sur `sdk` et `auto`.

**Correctif d'adjacence** : `JsInteraction` (clic, double-clic, survol) utilisait `document.elementFromPoint`, qui s'arrête à l'hôte d'un shadow root : un clic sur un bouton du shadow DOM n'atteignait pas le bouton alors que le pont le désigne. Les trois gestes traversent désormais les shadow roots ouverts (comportement inchangé hors shadow DOM). La revue du lot l'a étendu à tous les gestes que `act` produit : la saisie résout l'élément actif en profondeur (`shadowRoot.activeElement`) et gère `contenteditable` (`execCommand('insertText')`, repli par ajout du texte), le défilement et le glisser partent de l'élément en profondeur, et la remontée vers un conteneur défilant sort d'un shadow root par son hôte. Les touches (`key`, `key_combo`) visent l'élément actif en profondeur.

**Validation** : `deveye-server/tests/bridge_chromium.rs` (`#[ignore]`, Chromium ou Brave installé, un seul processus) exécute le **même script** dans un Chromium headless par `Runtime.evaluate` : mêmes références d'un instantané à l'autre, clic au centre remesuré après un décalage de mise en page, élément hors vue ou dans un conteneur défilant amené puis cliqué, shadow DOM, désactivé, couvert, nœud déplacé qui garde sa référence, nœud retiré périmé avec candidats, rechargement de page sans réutilisation de numéro, absence de toute valeur secrète. **Taille mesurée** (page de 56 éléments dont une liste de 40 lignes au balisage verbeux) : environ 1,6 Ko pour une approximation du texte (une ligne par élément, sans indentation ni marqueurs : le rendu réel est un peu plus gros) et 5,3 Ko pour l'`ElementSnapshot` en JSON (borne haute du rendu réel), contre 18,8 Ko pour `outerHTML` (pas la sortie de l'outil `dom`) ; ordres de grandeur seulement, consignés sans seuil bloquant.

## Conséquences

**Positives** : un agent désigne un élément d'une app WebKitGTK sans CSS ni coordonnées ; aucune dépendance, aucun changement de protocole ; le script est testé dans un vrai navigateur sans compiler d'app.

**Coûts et risques** : le script est plus fruste que l'arbre d'accessibilité (rôles par table, noms simplifiés, iframes ignorées, pas de calcul d'accessibilité complet) ; un clic synthétique n'est pas `isTrusted` ; `locate` fait défiler la page (effet de bord voulu, comme un humain) ; le rect d'un élément en ligne sur plusieurs lignes est l'enveloppe des lignes, son centre peut tomber hors de l'élément (alors `Obscured` ou clic voisin) ; le test réel sur une app Dioxus WebKitGTK reste manuel (compilation coûteuse). Un `WeakRef` absent (WebKitGTK avant 2.34) retient les éléments tant que la page vit.

## Amendements issus de la revue du lot

- **Centre hors fenêtre** : `locate` amène l'élément en vue aussi quand son rect recouvre la fenêtre mais que son **centre** (le point cliqué) est dehors ; l'élément « à moitié visible » n'est plus refusé en `offscreen`.
- **Profondeur de sortie** : chaque niveau d'arbre coûte deux niveaux JSON ; le décodeur `serde_json` du serveur refuse au-delà de 128. La sortie est donc bornée à 50 niveaux (`truncated` quand coupée), quelle que soit la profondeur du DOM (parcours borné à 300). `max_depth: 0` vaut 1, comme sur les widgets natifs.
- **Secrets** : voir « Secrets » ci-dessus ; la limite est consignée dans `SECURITY.md`.

## Amendements issus de la revue de suivi

- **Coupe des libellés** : un libellé coupé en pleine paire de substitution UTF-16 laisserait une moitié d'emoji, que `serde_json` refuse : la coupe retire la surrogate haute finale.
- **`tabindex`** : seul un `tabindex` strictement positif classe un élément sans rôle en `clickable` ; `tabindex="0"` ou vide désigne un conteneur simplement focusable.
- **Défilement de `locate`** : un simple recouvrement ne fait plus défiler la page, sauf si l'élément vit dans un conteneur qui défile (le défilement peut alors le dégager). Hors vue ou centre hors fenêtre, le défilement reste systématique. Limite assumée : un élément de taille 0x0 (`display: none` apparu après l'instantané) est rendu `offscreen`, faute d'état distinct dans le port stable `ElementLocation`.
