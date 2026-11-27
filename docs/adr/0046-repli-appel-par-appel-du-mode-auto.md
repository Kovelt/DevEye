# ADR-0046 — Repli appel par appel du mode auto

- **Statut** : Accepté
- **Date** : 2026-06-11
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0043 (pont natif), ADR-0034 (repli observable dans `target_info`), ADR-0031 (validation des flags), ADR-0041 (architecture)

## Contexte et problème

Une webview Chromium en cours de développement se ferme, se recompile à chaud, redémarre ; son port CDP disparaît puis revient. Un serveur MCP qui exige le CDP au démarrage meurt avec l'app, et le client MCP doit être relancé. À l'inverse, choisir une fois pour toutes entre CDP et pont natif oblige l'utilisateur à connaître le moteur de sa webview.

## Facteurs de décision

- survivre aux redémarrages et recompilations de l'app cible ;
- fonctionner sans option dans le cas courant (CDP si disponible, pont sinon) ;
- ne pas masquer une vraie erreur derrière un repli ;
- que l'agent sache quel backend le sert réellement.

## Options envisagées

1. **Choix unique au démarrage** : simple, mais fragile face aux redémarrages.
2. **Bascule globale et définitive** à la première panne : une panne passagère condamne le reste de la session au pont.
3. **Repli appel par appel** : chaque appel tente le primaire (CDP) et bascule sur le pont natif seulement si le primaire est injoignable.

## Décision

Option 3. `--target auto` (le défaut) enroule chaque port concerné dans une façade de repli : un appel essaie CDP, et **seule une erreur de liaison** (`PortError::Network`, cible injoignable) le redirige vers le pont natif du SDK ; toute autre erreur est propagée telle quelle. La connexion CDP est paresseuse (`LazyCdp`) : elle se refait à la demande, un CDP absent au démarrage n'est pas fatal. Les façades partagent un état qui rend le repli **observable** : chaque bascule (et chaque retour au primaire) est journalisée une fois, et `target_info.effective_backend` relit le backend réel (`cdp`, `sdk` ou `indetermine`, ADR-0034).

**Justification** : le repli par appel suit la vie de l'app sans état à réinitialiser, et le critère étroit (liaison seulement) évite de transformer un vrai échec d'opération en succès d'un autre backend. Rendre le repli visible évite qu'un agent croie piloter la webview alors que les gestes passent par le pont.

## Conséquences

**Positives**

- un hot-reload ou un redémarrage de l'app ne demande aucune action ;
- aucune option à choisir pour le cas courant ;
- l'agent peut vérifier le backend effectif.

**Coûts et risques**

- les capacités propres à CDP (`resize`, `upload_file`, `handle_dialog`) répondent « non supporté » dès que le repli est actif ;
- un appel qui tombe pendant la bascule peut échouer une fois ;
- le comportement dépend du critère « erreur de liaison » : toute nouvelle classe d'erreur doit être rangée consciemment.

## Notes

ADR rétroactif. Introduit le 2026-06-11 avec le pont natif (`da81991`, `--target auto|cdp|sdk` et `LazyCdp`) ; rendu observable le 2026-11-27 (ADR-0034). Voir [docs/architecture.md](../architecture.md).
