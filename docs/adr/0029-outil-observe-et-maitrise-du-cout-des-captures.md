# ADR-0029 — Outil `observe` et maîtrise du coût des captures

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (surface MCP stable : ajouts compatibles), ADR-0016 (masquage des secrets), ADR-0022 (capture CDP à l'échelle CSS), ADR-0025 (`captured_at`, état vide ou indisponible), ADR-0027 (annotations)

## Contexte et problème

Chaque observation coûte des tokens à l'agent : une capture 4K en PNG pèse plusieurs centaines de milliers d'octets, et comprendre l'état de l'app prend quatre à cinq appels (`screenshot`, `dom`, `state`, `collect_console_errors`, `logs_tail`). Deux besoins : **réduire le coût d'une capture** sans perdre la correspondance entre l'image et les coordonnées de clic (corrigée par l'ADR-0022), et **résumer l'app en un appel**, chaque section se dégradant seule quand la cible ne la fournit pas.

## Facteurs de décision

- l'agent doit pouvoir convertir une coordonnée de l'image réduite en coordonnée de clic ;
- aucun changement de comportement sans option : les appels existants restent identiques ;
- pas de nouveau crate ;
- un résumé d'arbre doit être borné quelle que soit la taille de l'app ;
- `observe` ne doit ouvrir aucune voie qui contourne le masquage des secrets (ADR-0016).

## Options envisagées

**Coût des captures**

1. **Ne rien faire, conseiller `screenshot_region`** : ne règle pas la vue d'ensemble.
2. **`max_width`, `format` (`png`/`jpeg`) et `quality` sur les trois outils de capture**, avec taille finale et facteur d'échelle restitués.
3. **Un niveau de qualité nommé** (`low`/`high`) : moins précis, ne donne pas le facteur d'échelle.

**Outil composite**

4. **Laisser l'agent enchaîner les outils** : coûte 4 à 5 allers-retours.
5. **`observe`** composant les ports existants, sections choisies par `include`/`exclude`.

## Décision

Options 2 et 5.

*Captures.* `screenshot`, `screenshot_region`, `screenshot_element` (et `observe`) acceptent `max_width` (réduction proportionnelle par filtre de Lanczos, jamais d'agrandissement, plafonnée à 16 384), `format` et `quality` (JPEG, 1 à 100, défaut 80 ; ignorée en PNG). Une option invalide est `invalid_params`, détectée avant la capture. Sans option, les octets PNG d'origine sont renvoyés tels quels. Un bloc texte JSON accompagne l'image : `image: { format, width, height, source_width, source_height, scale, bytes }` et, pour une région ou un élément, `origin: {x, y}` ; **coordonnée de capture = `origin` + coordonnée de l'image / `scale`**. Le champ `scale` est le rapport horizontal image/source ; l'écart vertical tient à l'arrondi de la hauteur (au plus un pixel).

*Dépendance.* Aucun crate n'est ajouté à la liste des dépendances directes : l'encodeur JPEG est dans `image`, mais derrière sa feature `jpeg`, qui ajoute au graphe les crates transitifs `zune-core` et `zune-jpeg` (licences MIT, Apache-2.0 ou Zlib, déjà admises par `deny.toml`). La feature n'est activée que pour `adapter-mcp-server`.

*`observe`.* Cinq sections dans l'ordre `screenshot`, `dom`, `state`, `console`, `logs`, toutes par défaut, choisies par `include` puis filtrées par `exclude` (une section inconnue ou un résultat vide est `invalid_params`, avant tout accès aux ports : un appel refusé ne vide aucun buffer). Bornes :

- capture : `max_width` vaut **1024** par défaut (au lieu de la taille d'origine), le reste des options comme `screenshot` ; PNG par défaut, car le JPEG dégrade le texte fin ;
- `dom` : résumé en lignes d'indentation (`balise#id.classe "texte"`), au plus 60 nœuds sur 6 niveaux par défaut (plafonds 500 et 30), nœuds repliés signalés `[+N]`, commentaires, scripts, styles et textes blancs écartés ; il lit les arbres CDP (`nodeName`, `attributes`) comme les arbres de widgets (`type`, `id`, `label`/`text`) ; une forme non reconnue retombe sur un aperçu JSON borné à 2 000 caractères ;
- `state` : nombre de signaux et aperçu de 120 caractères pour au plus 30 clés ;
- `console` : seuls `error` et `warning` sont restitués (les autres niveaux drainés sont comptés dans `ignored_lower_levels`), les plus récents, bornés à `max_events` (20, plafond 200) ;
- `logs` : les `max_events` plus récents.

Chaque section renvoie `available: true` et ses données, ou `available: false` avec `reason` (`unsupported`, `unreachable`, `js_exception`, `error`) et un `detail` du type « indisponible pour cette cible : … », sans faire échouer l'appel. L'état et les champs de logs viennent des mêmes ports que `state` et `logs_tail`, donc du même masquage côté SDK : `observe` n'ajoute aucune route. Les limites de l'ADR-0016 restent valables (texte libre des messages, valeurs issues du DOM). `observe` est annoté lecture seule et non idempotent : il **draine** les buffers de logs et de console.

**Justification** : l'option 2 est la seule qui garde la conversion de coordonnées exacte et laisse l'agent doser le coût ; l'option 5 compose sans nouvelle logique d'accès, donc sans nouvelle surface à protéger. Les valeurs par défaut (1024 px, 60 nœuds, 20 événements) sont des choix d'agent non tranchés par le plan, d'où le statut « Proposé ».

## Conséquences

**Positives**

- une vue d'ensemble de l'app en un appel et quelques milliers de tokens ;
- les appels existants sont inchangés ; les ajouts sont compatibles avec la promesse semver sur les noms et schémas des outils (paramètres optionnels, nouvel outil) ;
- l'agent sait toujours dans quelle échelle il lit l'image.

**Coûts et risques**

- deux crates transitifs de plus dans `Cargo.lock` (feature `jpeg`) ;
- le redimensionnement coûte un décodage et un ré-encodage ; le JPEG perd du détail sur le texte fin ;
- le résumé d'arbre est heuristique (noms de champs usuels) : un arbre de widgets très singulier tombe sur l'aperçu JSON ;
- `observe` vide les buffers de console de niveau inférieur à `warning` sans les restituer : documenté, l'agent qui les veut appelle `collect_console_errors` avant.

## Notes

Application : `deveye-adapters/adapter-mcp-server/src/imaging.rs` (options, redimensionnement, encodage), `tools_capture.rs`, `tools_observe.rs`. Tests : `imaging::tests`, `tools_observe::tests`, `dispatch_tests` (ports simulés).

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
