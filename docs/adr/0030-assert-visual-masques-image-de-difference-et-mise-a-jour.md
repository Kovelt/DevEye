# ADR-0030 — `assert_visual` : masques, image de différence et mise à jour de la référence

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (surface MCP stable), ADR-0027 (annotations : `assert_visual` destructif), ADR-0029 (coût des captures)

## Contexte et problème

`assert_visual` compare la capture à une baseline PNG et ne dit, en cas d'échec, qu'un ratio : l'agent ne sait ni où ça diverge ni comment ignorer une horloge ou un curseur clignotant, et pour accepter un changement voulu il doit supprimer le fichier à la main. Ces trois manques rendent l'outil peu utilisable en régression visuelle.

## Facteurs de décision

- compatibilité : sans nouveau paramètre, la sortie ne change pas ;
- l'image de différence ne doit pas coûter plus de tokens que la capture elle-même ;
- le remplacement de la référence doit être explicite, jamais un effet de bord ;
- aucune dépendance supplémentaire.

## Options envisagées

1. **Un second outil `diff_visual`** : duplique la capture et la gestion de baseline.
2. **Enrichir `assert_visual`** : paramètres optionnels `ignore_regions` et `update_baseline`, image de différence jointe à l'échec.
3. **Écrire l'image de différence sur disque seulement** : l'agent doit ensuite l'ouvrir ; perd l'intérêt « un seul appel ».

## Décision

Option 2.

- `ignore_regions: [{x, y, width, height}]` : zones exclues de la comparaison **et du dénominateur** de `diff_ratio` (un masque couvrant la moitié de l'image ne dilue pas la différence restante). Les coordonnées sont celles de l'image comparée, donc relatives à la région quand `x/y/width/height` la recadrent. Un masque de largeur ou de hauteur nulle est `invalid_params`, avant toute capture ; un masque qui déborde est simplement borné par l'image.
- Échec **à dimensions égales** : le résultat `isError` porte le JSON puis une image de différence (fond de la capture éclairci, pixels différents en rouge vif, zones ignorées teintées de bleu), réduite à 1 280 px de large au plus ; `diff_image.scale` donne l'échelle. Le JSON ajoute `diff_pixels`, `compared_pixels`, `ignored_regions` et `diff_bbox` (rectangle englobant exact des pixels différents, en pixels de l'image comparée). Dimensions différentes : pas d'image, rien de pixel à pixel à dessiner.
- `update_baseline: true` : écrit la capture courante comme nouvelle référence sans comparer, répond `{ ok: true, created, updated: true }`. Défaut `false`.

**Justification** : un seul outil, un seul appel, rétrocompatible. Le rapport au dénominateur est le seul qui garde le seuil `threshold` significatif quand on masque une grande zone. `assert_visual` écrivant des fichiers, il est annoté destructif (ADR-0027).

## Conséquences

**Positives**

- l'agent localise la régression et peut ignorer les zones mouvantes ;
- l'acceptation d'un changement voulu tient en un appel explicite.

**Coûts et risques**

- l'image de différence ajoute un bloc à la réponse en cas d'échec (bornée à 1 280 px) ;
- la réduction de l'image peut atténuer un pixel isolé : `diff_bbox` porte la précision ;
- `update_baseline` écrase sans sauvegarde : le dépôt du projet (git) est le filet.

## Notes

Application : `deveye-adapters/adapter-mcp-server/src/imaging.rs` (`visual_diff`, `render_diff_png`), `tools_capture.rs`. Tests : `imaging::tests`, `dispatch_tests::assert_visual_*`.
