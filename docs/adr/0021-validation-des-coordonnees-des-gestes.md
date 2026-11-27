# ADR-0021 — Validation des coordonnées des gestes avant dispatch

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (surface stable), ADR-0018 (portage depuis l'ancienne lignée), ADR-0022 (capture à l'échelle CSS, qui fixe l'espace des coordonnées)

## Contexte et problème

`click`, `right_click`, `double_click`, `hover`, `drag_drop` et l'ancrage de `scroll` envoyaient leurs coordonnées à la cible sans aucun contrôle. En CDP, un point négatif ou hors page est ignoré sans bruit ; en `kwin-eis`, il part vers une autre application que la cible ; en `x11`, seul un écrêtage à 16 bits borne la valeur. L'agent reçoit `{ ok: true }` pour un geste qui n'a rien fait ou a touché autre chose. L'ancienne lignée GitHub avait une garde (`ensure_in_viewport`) qui répond à ce défaut.

## Facteurs de décision

- un geste sans effet doit échouer bruyamment, pas réussir en silence ;
- ne pas bloquer un geste légitime quand la taille du viewport est inconnue ou peu fiable ;
- ne pas ajouter une panne de plus : une coordonnée fausse est une faute d'appel (`invalid_params`) ;
- latence : un aller-retour de plus par geste.

## Options envisagées

1. **Aucune validation** : statu quo, le geste « réussit » à vide.
2. **Rejeter seulement les valeurs négatives** : sans I/O, mais ne détecte pas un clic au-delà du bord droit ou bas.
3. **Négatives rejetées, borne haute vérifiée au mieux** via `Renderer::viewport_size()` : détecte aussi le dépassement quand la taille est connue.

## Décision

Option 3, dans `adapter-mcp-server` (la garde est un contrat de l'outil MCP, pas du domaine).

- Une coordonnée **négative** est refusée avant toute I/O, sur toutes les cibles.
- Si `viewport_size()` répond une taille non nulle, un point `x >= largeur` ou `y >= hauteur` est refusé. Si le renderer échoue ou rend une taille nulle, le geste passe.
- L'erreur est `invalid_params`, avec le point fautif et la taille du viewport dans le message. Le geste n'est jamais dispatché.
- Couvre `click`, `right_click`, `double_click`, `hover`, `drag_drop` (les deux points) et l'ancrage de `scroll` quand `x` et `y` sont fournis ; pas `click_selector` ni `click_text`, dont le point vient du DOM.

**Justification** : l'option 3 est la seule qui attrape le clic hors fenêtre en `kwin-eis` et `x11`, motif principal du portage. Laisser passer quand la taille est inconnue évite qu'une garde défaillante devienne une panne.

## Conséquences

**Positives**

- un clic mal placé produit une erreur explicite que l'agent corrige ;
- sur `kwin-eis` et `x11`, plus de clic envoyé par erreur à une autre application ;
- la garde se teste sans cible réelle.

**Coûts et risques**

- un appel `viewport_size()` de plus par geste : un aller-retour CDP, un cache hit sur `kwin` ; coût non mesuré sur `xcap` ;
- sur les cibles OS-level, un menu ou une bulle système qui déborde de la fenêtre reste hors du viewport mesuré : un clic dessus est refusé. Seule la fenêtre cible est comptée ;
- en CDP, le viewport mesuré est le viewport de mise en page : un clic sur la barre de défilement (hors de lui) est refusé ;
- `viewport_size()` est en pixels CSS en CDP et en pixels de la fenêtre ailleurs : la garde suit l'espace dans lequel la cible reçoit ses gestes.
