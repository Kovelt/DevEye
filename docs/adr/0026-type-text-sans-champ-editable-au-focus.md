# ADR-0026 — `type_text` sans champ éditable au focus

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (surface stable), ADR-0023 (résultats `isError` structurés pour les échecs sémantiques), ADR-0025 (contrat de sortie)

## Contexte et problème

`type_text` saisit du texte « au focus courant ». Quand rien d'éditable n'a le focus, les cibles répondaient différemment : le pont SDK refusait (« aucun champ éditable au focus », rendu en erreur protocolaire `-32602`), CDP répondait **succès** (`Input.insertText` sur un élément non éditable ne fait rien et ne signale rien), `kwin-eis` et `x11` répondaient succès (les touches partent à la fenêtre qui a le focus OS). L'agent ne pouvait donc pas se fier à `{ ok: true }`. Le point était resté « à statuer » depuis les essais d'usage. Les cibles OS-level n'ont aucun moyen de savoir ce qui reçoit les touches.

## Facteurs de décision

- un échec doit se voir : jamais un faux succès sur les cibles qui peuvent le détecter ;
- un comportement identique sur les cibles qui partagent la même nature (WebView) ;
- dire honnêtement ce qui ne peut pas être détecté ailleurs plutôt que simuler ;
- ne pas bloquer une saisie légitime sur un doute (iframe, page qui ne répond pas).

## Options envisagées

1. **Statu quo** : trois comportements selon la cible.
2. **Accepter partout** (retirer le refus du pont SDK) : uniforme, mais généralise le faux succès.
3. **Refuser partout où c'est détectable, documenter ailleurs** : CDP vérifie l'élément focalisé avant de saisir ; le pont SDK refuse déjà ; kwin-eis et x11 restent sans vérification, documentés.
4. **Détecter aussi en OS-level** (accessibilité AT-SPI, capture avant et après) : hors de portée (le spike AT-SPI est archivé) et coûteux.

## Décision

Option 3.

- **CDP** : avant `Input.insertText`, une lecture JS sans effet de bord inspecte l'élément focalisé (en traversant les shadow roots) : `input` de type texte, `textarea` ou contenu éditable, ni désactivé ni en lecture seule. Un `iframe` focalisé est opaque : la saisie passe. Si la lecture lève une exception JS, la saisie passe aussi (doute) ; une panne de liaison reste une erreur.
- **Pont SDK** : inchangé (il refusait déjà).
- **Erreur** : l'adapter produit `PortError::Interaction("aucun champ éditable au focus : …")`. L'outil `type_text` rend toute `PortError::Interaction` en résultat `isError` structuré `{ ok: false, reason: "interaction_failed", detail }` au lieu d'une erreur protocolaire `-32602` : l'agent corrige en cliquant d'abord sur le champ, ce n'est pas une faute d'appel.
- **kwin-eis et x11** : inchangés ; la description et la documentation de l'outil disent que `{ ok: true }` signifie « événements envoyés à la fenêtre focus », pas « texte saisi », et renvoient à `focus_window` puis `screenshot_region` pour vérifier.

**Justification** : l'option 3 supprime le faux succès là où il est détectable, aligne CDP et SDK, et reste honnête ailleurs. L'option 2 aurait généralisé le défaut ; l'option 4 est un chantier à part.

## Conséquences

**Positives**

- `type_text` sans focus échoue visiblement en CDP et en SDK, avec une indication de la correction ;
- un seul contrat d'erreur (`interaction_failed`) pour les échecs de saisie.

**Coûts et risques**

- un aller-retour JS de plus par `type_text` en CDP ;
- `reason: "interaction_failed"` est générique (il couvre tout échec d'interaction de `type_text`, pas seulement l'absence de focus) : le détail est dans `detail`, dont le texte n'est pas un contrat ;
- un champ éditable personnalisé qui n'expose ni `contenteditable` ni `input` (éditeur sur canvas) est refusé en CDP à tort : contournement par `press_key` ou `key_combo` (événements clavier), qui ne passent pas par cette garde ;
- en kwin-eis et x11, le faux succès reste possible, par construction.
