# ADR-0022 — Capture CDP à l'échelle CSS

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (surface stable : le PNG de `screenshot` en fait partie), ADR-0021 (validation des coordonnées, même espace), ADR-0018 (portage depuis l'ancienne lignée)

## Contexte et problème

`Page.captureScreenshot` rend des pixels **physiques**. Sur un écran HiDPI (facteur d'échelle 1,25, 1,5 ou 2), le PNG est plus grand que le viewport : un agent qui lit un point sur l'image et le passe à `click`, `hover` ou `scroll` (pixels **CSS**) tape à côté, et les points proches des bords droit et bas sont refusés. Seul `screenshot_element` compensait, par un ratio calculé côté serveur. L'ancienne lignée GitHub corrigeait cela en capturant un `clip` couvrant le viewport avec `scale: 1`.

## Facteurs de décision

- les coordonnées de l'image et celles des gestes doivent coïncider sans facteur à appliquer ;
- ne pas dépendre de JavaScript : le moteur JS est suspendu pendant une boîte de dialogue, et une capture doit rester possible ;
- ne pas ralentir le cas courant (écran à 100 %) ;
- le comportement doit avoir été mesuré, pas supposé.

## Options envisagées

1. **Statu quo** : PNG en pixels physiques, l'agent applique un facteur qu'il ne connaît pas.
2. **Reprendre telle quelle la solution de l'ancienne lignée** (`clip` en `(0, 0)`, `scale: 1`).
3. **`clip` à l'origine de défilement, avec une échelle recalculée** : capturer le viewport à `scale: 1`, lire la largeur du PNG, et recapturer à `scale = largeur CSS / largeur PNG` si le PNG dépasse le viewport.
4. **Lire `window.devicePixelRatio` en JS** puis capturer à `scale = 1 / ratio` : une capture, mais une dépendance au moteur JS.

## Décision

Option 3. Deux écarts avec l'ancienne lignée, établis en mesurant Chromium (mode headless, facteur d'échelle réel de 1, 1,25, 1,5 et 2) :

- **l'origine du `clip` est en coordonnées de page**, pas de viewport : à 1000 px de défilement, un `clip` en `(0, 0)` capture le haut du document. On utilise la position de défilement (`pageX`, `pageY` de `cssLayoutViewport`) ;
- **`scale: 1` ne donne pas des pixels CSS** quand le facteur d'échelle est réel : le PNG sort en pixels physiques (1570 px pour 785 px CSS à 2x). Il faut `scale = 1 / ratio`. Le ratio est déduit de la largeur du PNG (en-tête IHDR), sans JS ; à 100 % une seule capture suffit.

Conséquence mesurée : le PNG fait exactement la taille du viewport CSS (`Screenshot::viewport`), au pixel près à 1, 1,25, 1,5 et 2 ; il exclut aussi la barre de défilement, que l'ancienne capture incluait (800 px contre un viewport de 785).

**Justification** : l'option 3 est la seule qui soit à la fois correcte sur page défilée, indépendante du JS et sans surcoût à 100 %. L'option 2 capture la mauvaise zone dès que la page est défilée et ne rend pas des pixels CSS sur un écran HiDPI réel. L'option 4 échoue pendant un dialog.

## Conséquences

**Positives**

- les coordonnées lues sur le PNG s'utilisent telles quelles dans `click`, `hover`, `scroll` et `drag_drop`, sur tout écran ;
- `screenshot_element` n'a plus de ratio à compenser en CDP (le calcul reste utile pour les autres renderers).

**Coûts et risques**

- sur écran HiDPI, deux captures au lieu d'une (le PNG physique est jeté) ;
- le PNG perd de la netteté sur HiDPI (résolution CSS, pas physique) : les baselines d'`assert_visual` prises sur un écran HiDPI changent de taille par rapport à l'ancien comportement ; la comparaison reste cohérente d'un écran à l'autre ;
- le comportement dépend de la sémantique de `clip.scale` de Chromium (mesurée ici sur Chromium) : il n'a pas été vérifié sur WebView2 réel ;
- les cibles non CDP (xcap, KWin, X11) gardent des PNG en pixels de la fenêtre capturée.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
