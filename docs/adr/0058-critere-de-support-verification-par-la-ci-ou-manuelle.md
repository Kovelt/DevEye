# ADR-0058 — Critère de support : vérification par la CI ou par le mainteneur

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0037 (Linux seul supporté), ADR-0039 (tests de bout en bout en CI), ADR-0045 (Wayland via KWin)

## Contexte et problème

ADR-0037 retient le facteur « n'annoncer que ce que la CI vérifie ». Or `docs/platforms.md` classe « Supporté » la cible Wayland KDE Plasma, dont les tests `live_*` exigent une session graphique réelle (`#[ignore]`) et ne tournent qu'à la main. La matrice et l'ADR se contredisent à la lettre. Il faut dire quel critère s'applique vraiment.

## Facteurs de décision

- ne pas promettre ce que personne ne vérifie ;
- ne pas écarter la cible principale du mainteneur (KDE Plasma Wayland) parce que GitHub Actions n'a pas de compositeur KWin ;
- une règle lisible, applicable cible par cible.

## Options envisagées

1. **CI seule** : rétrograder Wayland KDE en « Expérimental ». Honnête, mais présente comme fragile la voie la mieux éprouvée.
2. **CI ou vérification manuelle par le mainteneur** : « Supporté » exige soit un job CI bloquant, soit une vérification manuelle, répétée à chaque release et consignée ; la colonne « Test » de la matrice dit lequel.
3. **Ne rien dire** : l'incohérence reste.

## Décision

Option 2 : une cible est « Supporté » lorsqu'elle est couverte par un job CI bloquant **ou** vérifiée à la main par le mainteneur avant chaque release (colonne « Test » = « Manuel » dans `docs/platforms.md`), la logique pure restant testée en CI dans tous les cas. Sans l'une ni l'autre, la cible est « Expérimental » ou « Non supporté ».

**Justification** : seule cette règle décrit la pratique réelle sans retirer à KDE Plasma un statut mérité, tout en gardant une exigence vérifiable. Elle précise ADR-0037 sans le contredire : « n'annoncer que ce que l'on vérifie », la CI étant le moyen préféré.

## Conséquences

**Positives**

- la matrice et l'ADR disent la même chose ;
- une cible « Manuel » est identifiable, donc une régression non détectée par la CI est assumée plutôt que cachée.

**Coûts et risques**

- une vérification manuelle dépend du mainteneur et peut être oubliée : la checklist de release doit la lister ;
- ADR proposé : il engage le mainteneur sur cette vérification avant chaque release.

## Notes

Application : `docs/platforms.md` (légende et colonne « Test ») ; guides `webview-chromium-cdp.md` et `dioxus-desktop-linux.md` corrigés dans le même lot.
