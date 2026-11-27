# ADR-0054 — Limites documentées issues des revues de la 1.0

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0019 (resize), ADR-0022 (capture à l'échelle CSS), ADR-0023 (exception JS), ADR-0024 (sérialisation sans panic), ADR-0029 (observe et coût des captures), ADR-0030 (assert_visual)

## Contexte et problème

Les revues adversariales de la branche de release ont relevé une douzaine d'écarts. La plupart sont corrigés dans le code, avec test. Quelques-uns relèvent d'un choix : les corriger coûterait plus que le défaut ne vaut, ou changerait un contrat déjà documenté. La 1.0 fige ces contrats ; il faut donc dire explicitement ce qui est corrigé, ce qui est assumé, et pourquoi.

## Facteurs de décision

- ne figer en 1.0 que des comportements décrits dans la documentation ;
- ne pas changer le trait `Renderer` ni les types de `deveye-ports` (bibliothèque MIT) pour un cas qui ne concerne que des doublures de test ;
- chaque décision doit pouvoir être rouverte en 1.x sans rupture.

## Options envisagées

1. **Tout corriger dans le code**, y compris la mémorisation du resize et un repli sans lecture d'en-tête PNG.
2. **Corriger les défauts de comportement et documenter les limites inhérentes** (retenue).
3. **Ne rien changer** : laisse des messages trompeurs et des pertes de données silencieuses.

## Décision

Option 2.

**Corrigé dans le code, avec test** :

- l'exception JS d'un objet non-`Error` (`throw {code: 42}`) rend sa valeur ; `description` ne prime que pour une instance d'`Error` (ADR-0023, adapter CDP) ;
- le pont SDK ne produit `PortError::Script` que pour un statut 200 avec `error` ; tout autre statut inattendu (403 de la garde Host, 4xx) devient `PortError::Protocol` (ADR-0023) ;
- après destruction de la page CDP attachée, l'adapter se rattache à la première page déjà vue qui correspond au filtre ; l'erreur d'attente distingue « une page correspond mais n'a pu être attachée » de « aucune page ne correspond » ;
- `assert_visual` : un masque qui couvre toute l'image ne compare aucun pixel et renvoie `ok: false` avec `reason: "fully_masked"` (ADR-0030) ;
- `logs_wait` renvoie les événements déjà drainés avec `poll_error` quand un sondage ultérieur échoue ;
- la sérialisation JSON des sorties MCP ne retombe plus sur un `"null"` silencieux : un échec produit un objet d'erreur visible (`to_json_text`, ADR-0024).

**Assumé et documenté** :

- **Capture CDP** : la mise en page (`getLayoutMetrics`) puis la capture (`captureScreenshot`) sont séquentielles, car le `clip` dépend du résultat de la première (ADR-0022). Le parallélisme d'une capture simple de l'ancienne lignée est perdu ; la fenêtre de désalignement entre les deux appels est acceptée.
- **`resize` et ré-attache** : `Emulation.setDeviceMetricsOverride` est lié à la session CDP. Après une ré-attache (page détruite puis recréée), l'émulation n'est pas rejouée : l'agent doit refaire `resize`. La phrase « l'émulation persiste jusqu'au prochain `resize` » (ADR-0019) vaut pour une session continue.
- **`Renderer` et PNG valide** : le port annonce un PNG ; le serveur MCP lit son en-tête pour tout `screenshot*`. Un `Renderer` tiers qui renverrait autre chose échoue en `internal_error`, même sans option. La documentation du port le dit. La phrase « les appels existants restent identiques » (ADR-0029) vaut pour un `Renderer` conforme.
- **Vocabulaire d'`observe`** : la raison `unreachable` (panne de liaison) reste propre à `observe`, listée dans l'ADR-0029, là où `state` dit `sdk_unavailable`. Les harmoniser est possible en 1.x en ajoutant un alias, pas en retirant une valeur.
- **`observe` et logs** : `observe` draine les logs et rapporte ce qu'il ignore (`ignored_lower_levels`, `total`, `truncated`) ; c'est un coût accepté et chiffré (ADR-0029).

**Justification** : corriger ce qui trompe l'agent (message d'erreur faux, perte silencieuse, assertion vide qui passe) ; documenter ce qui découle de la nature du protocole (session CDP, capture à `clip`) ou ne touche que des doublures de test.

## Conséquences

**Positives**

- plus de succès vide ni d'exception JS inventée par un refus de garde ;
- les limites restantes sont écrites, donc testables et rouvrables.

**Coûts et risques**

- le resize perdu à la ré-attache surprend un agent qui ne relit pas la doc : à reconsidérer (mémoriser et rejouer) si le cas se présente ;
- le vocabulaire d'erreur de la 1.0 garde deux noms pour une même panne entre `state` et `observe`.

## Notes

La suite de revue contenait aussi un constat sur l'intitulé d'un commit antérieur (`fix(adapter-screencap): ...`, scope absent de la table). Le corriger exigerait de réécrire l'historique ; l'écart est accepté, sans ADR de plus.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
