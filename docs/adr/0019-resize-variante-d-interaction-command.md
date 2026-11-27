# ADR-0019 — Redimensionnement du viewport : variante `InteractionCommand::Resize`

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (surface stable), ADR-0012 (`#[non_exhaustive]` sur `InteractionCommand`), ADR-0018 (portage depuis l'ancienne lignée)

## Contexte et problème

La documentation annonçait un outil `resize` « à venir » sans qu'il existe. Le propriétaire a décidé de le livrer (décision D05) plutôt que de retirer la promesse. Il faut choisir où le modéliser : le redimensionnement agit sur le moteur de rendu (CDP : `Emulation.setDeviceMetricsOverride`), et la plupart des cibles (SDK, KWin EIS, X11) n'ont aucun moyen de le faire. L'ancienne lignée GitHub l'avait modélisé comme variante de `InteractionCommand`. Une alternative est une méthode du port `Renderer`, qui toucherait six implémenteurs.

## Facteurs de décision

- ne pas casser les implémenteurs existants des ports (surface stable, ADR-0011) ;
- ajouter une capacité sans version majeure, ce qu'autorise `#[non_exhaustive]` sur `InteractionCommand` (ADR-0012) ;
- répondre honnêtement `Unsupported` sur les cibles sans moteur pilotable ;
- cohérence avec les commandes déjà « CDP-only » (`HandleDialog`, `UploadFile`).

## Options envisagées

1. **Méthode `resize` sur le trait `Renderer`** : sémantiquement défendable (le viewport est une propriété du rendu), mais un changement de trait est cassant pour les six implémenteurs et pour toute implémentation externe, sauf à fournir une implémentation par défaut.
2. **Variante `InteractionCommand::Resize { width, height }`** : ajout non cassant ; les adapters qui ne la connaissent pas la refusent déjà par leur branche `_` (`PortError::Unsupported`) ; modélisation identique à celle de l'ancienne lignée.
3. **Nouveau port dédié** (`ViewportControl`) : propre, mais un port de plus (12 aujourd'hui) à composer dans `deveye-server` pour une seule commande.

## Décision

Option 2 : `InteractionCommand::Resize { width: u32, height: u32 }`, exposée par l'outil MCP `resize`.

- **Sémantique** : émulation du viewport en pixels CSS. La fenêtre de l'OS n'est pas redimensionnée ; le facteur d'échelle de l'écran est conservé (`deviceScaleFactor: 0`). L'émulation persiste jusqu'à un nouveau `Resize`.
- **Retrait de l'émulation** : `width` et `height` à `0` ensemble appellent `Emulation.clearDeviceMetricsOverride`. Sans cela, un agent resterait coincé en émulation. Un seul côté à `0` est invalide.
- **Validation** : l'outil MCP refuse (`invalid_params`) toute dimension hors `1..=16384` (hors `0`x`0`), avant tout dispatch et quelle que soit la cible ; l'adapter CDP refuse défensivement un seul côté nul (`PortError::Interaction`).
- **Cibles** : CDP seul l'implémente. Le pont SDK, KWin EIS et X11 répondent `PortError::Unsupported`, que l'outil rend en `isError` structuré `{ reason: "unsupported" }` (convention E1). Une façade `auto` qui retombe de CDP sur le pont SDK donne donc `unsupported`, pas un faux succès.

**Justification** : l'option 2 est la seule qui ajoute la capacité sans toucher aux implémenteurs existants ni ajouter de port, et elle suit le précédent de `HandleDialog` et `UploadFile`. Que le viewport soit conceptuellement une propriété du rendu pèse moins que le coût d'un changement de trait avant le gel 1.0.

## Conséquences

**Positives**

- la promesse « à venir » disparaît de la documentation : l'outil existe et est testé ;
- aucune rupture pour les implémenteurs de ports ; les consommateurs qui font un `match` exhaustif sur `InteractionCommand` ont déjà un bras `_` (ADR-0012).

**Coûts et risques**

- une commande de plus dans la surface stable : le nom `resize`, la forme `{ width, height }` et la valeur spéciale `0`x`0` sont désormais figés (ADR-0011) ;
- `Resize` est une propriété d'état (l'émulation persiste) rangée parmi des commandes d'événements : un agent doit penser à la retirer en fin de test ;
- hors CDP, aucun moyen de redimensionner : un éventuel redimensionnement de la fenêtre OS réelle serait une autre commande, additive.
