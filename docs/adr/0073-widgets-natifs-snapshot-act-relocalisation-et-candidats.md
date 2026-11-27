# ADR-0073 — Widgets natifs, `snapshot` et `act` : relocalisation à l'action, candidats et surface expérimentale

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0072 (modèle de références, amendé ici), ADR-0069 (surface expérimentale), ADR-0062 et ADR-0065 (route `ui.widgets`, payload `{}`), ADR-0064 (masquage des secrets), ADR-0067 (sorties structurées), ADR-0068 (capacités additives), ADR-0011 (stabilité)

## Contexte et problème

L'ADR-0072 a fixé le modèle (types, `RefTable`, port `ElementInspector`). Ce lot le branche : un inspecteur pour les widgets natifs, les deux outils MCP `snapshot` et `act`, leur câblage par cible et le contrôle de `doctor`. Il faut trancher ce que l'ADR-0072 laissait ouvert : comment un `w:<id>` périmé se distingue d'un identifiant inventé quand rien n'est mis en cache, ce que fait `act` quand deux widgets portent le même identifiant, comment annoncer deux outils de plus sans toucher la promesse des 44 outils stables, et jusqu'où brancher les cibles tant que le pont WebKitGTK et CDP n'ont pas d'inspecteur.

## Facteurs de décision

- aucune rupture de la surface stable : ni outil, ni schéma, ni octet de la sortie d'un outil existant ne change par défaut (ADR-0011, ADR-0067) ;
- pas de rebind silencieux : agir sur le mauvais bouton « Supprimer » est pire qu'une erreur (ADR-0072) ;
- la fraîcheur vient de l'app, pas d'un cache : le rect d'un instantané vieillit vite ;
- le payload de la route reste `{}` : un handler tiers strict casserait sur un champ inconnu (ADR-0065) ;
- un secret ne doit jamais atteindre l'agent par une nouvelle voie.

## Décision

**`RouteWidgetsElements`** (`adapter-composite`) implémente `ElementInspector` au-dessus d'un `CommandRouter` et de la route `ui.widgets` :

- `snapshot` fait **un appel de route**, lit l'arbre (`deveye.ui/1` ou legacy), en retire les valeurs des widgets secrets (après le masquage de la route, ADR-0064), puis construit l'arbre imbriqué : références `w:<id>`, widgets invisibles exclus avec leur sous-arbre, conteneurs anonymes aplatis quand `interactive_only` est vrai (défaut), `max_depth` et `max_nodes` appliqués après coup avec `truncated`. `generation` vaut 0 (pas de table), `scope` n'accepte qu'une référence `w:<id>` ;
- `locate` refait **un appel de route** et rend le rect frais : c'est ce rect, jamais celui d'un instantané, qui sert à cliquer ;
- **périmé ou inconnu** : l'inspecteur ne garde en mémoire que la liste des identifiants déjà vus (rôle et nom), jamais un élément. Un identifiant absent de l'arbre mais déjà vu répond `Stale` avec les widgets visibles de même rôle et de même nom (5 au plus) ; jamais vu, `UnknownRef`. Cette mémoire sert à nommer et ne désigne rien : aucun rebind. Elle ne survit pas à un redémarrage du serveur (une référence d'avant devient alors « inconnue », ce qui reste une erreur claire) ;
- **identifiants dupliqués** : tous les widgets concernés sont marqués `ephemeral` dans l'instantané ; `locate` répond `Ambiguous` (nouvelle variante de `ElementError`) plutôt que de choisir le premier. Les identifiants synthétisés du format legacy (`id_ephemeral`) sont marqués `ephemeral` ;
- **état** : un widget invisible, de surface nulle, rogné ou entièrement hors de la fenêtre est `offscreen` ; `enabled: false` devient `disabled` dans la localisation (nouveau champ d'`ElementLocation`). Pas de garde d'occlusion pour les widgets : l'arbre ne dit pas qui est dessiné au-dessus, `Obscured` n'est donc pas produit.

**Amendements à l'ADR-0072** (types expérimentaux, ajouts seuls) : `ElementLocation::disabled` (avec `with_disabled`) et `ElementError::Ambiguous { reference, count }`.

**Outils MCP** (`tools_perception.rs`) :

- `snapshot` : texte compact par défaut (une ligne par élément, marqueurs `[disabled]`, `[focused]`, `[offscreen]`, `[ephemeral]`…, valeur tronquée, ligne `focus:`) ; `format: "json"` rend le même contenu avec les rects, aussi en `structuredContent`. Pas d'`outputSchema` : le contenu dépend de la cible et le texte doit rester bon marché ;
- `act` : un seul outil, `action` parmi `click`, `double_click`, `right_click`, `hover`, `type`, `scroll_into_view`, `scroll`. Tout est validé avant le moindre accès au port (un paramètre sans sens pour l'action est refusé). Séquence : `locate`, gardes (`disabled`, `offscreen`, centre dans le viewport), puis des `InteractionCommand` **existantes** sur le centre remesuré ; `type` clique puis saisit. Aucun événement n'est émis sur un échec ;
- erreurs : `ErrorReason` gagne `stale_ref` (avec `hint` et `candidates`), `unknown_ref`, `ambiguous_ref`, `obscured` (champ `by`), `offscreen`, `disabled` ; `ToolError` gagne les champs optionnels `ref` et `by`, absents du texte des erreurs existantes ;
- `observe` accepte `include: ["elements"]` : la section n'est produite **que sur demande** (la sortie par défaut et la liste `skipped` restent celles de la 1.1) ;
- `TargetInfo` gagne `elements`, **omis quand il est faux** : la réponse de `target_info` reste identique octet pour octet sur les cibles sans éléments, et une cible qui en a l'annonce.

**Surface expérimentale** : les deux outils sont hors promesse semver (ADR-0069). Leur description commence par « [expérimental] » ; `EXPECTED_TOOL_COUNT` passe à 46, `EXPERIMENTAL_TOOLS` en liste deux, `STABLE_TOOL_COUNT` reste 44 et c'est ce nombre que la documentation annonce. Les tests de cohérence documentaire comparent la liste du README et les nombres annoncés aux outils **stables**, exigent que `docs/usage.md` documente les deux et que `docs/stability.md` les liste comme expérimentaux. Aucun des 44 outils historiques n'est modifié ; leurs tests passent inchangés, à la seule exception des constantes de comptage et de classification.

**Câblage** (`deveye-server`) : `x11` et `kwin-eis` reçoivent `RouteWidgetsElements` sur l'URL du SDK (`--sdk-url`) ; `sdk`, `cdp` et `auto` reçoivent un inspecteur `unsupported` dont le `hint` renvoie vers `dom`, `find_element`, `click_selector` et `click_text` en attendant le pont. `FallbackElementInspector` est écrit et testé (bascule sur `PortError::Network` seulement, jamais sur une référence périmée, inconnue ou ambiguë, qui serait un rebind entre deux backends) mais **pas câblé** : il n'existe pas encore de primaire. Sans route `ui.widgets`, `snapshot` répond `unsupported` avec un `hint` qui dit d'exposer la route ou de passer par `screenshot` puis `click`.

**`doctor`** : les identifiants dupliqués restent un échec bloquant (ADR-0065) et le conseil mentionne désormais les références ; les identifiants synthétisés du format legacy deviennent un avertissement, car les références `w:<id>` ne seraient pas stables d'une relance à l'autre.

**Taille** (critère de la 1.2, mesure sans seuil) : sous Xvfb avec un SDK factice, 4 widgets donnent 200 octets de texte contre 416 pour `dom` ; environ 55 widgets (une liste de 50 lignes), 2 531 octets contre 6 377 (39 %). Le JSON de `snapshot` coûte environ le double du texte. Le gain réel est plus net sur un DOM de webview, hors de ce lot.

**Aucune dépendance nouvelle.**

## Conséquences

**Positives** : un agent désigne un widget natif sans coordonnées ni CSS, sur la cible Linux de référence ; le clic suit un widget qui s'est déplacé entre l'instantané et l'action ; une référence périmée coûte un `snapshot` et ne clique jamais un voisin ; la 1.1 est inchangée octet pour octet pour tout appel qui ne demande pas les éléments.

**Coûts et risques** : chaque `snapshot` et chaque `act` relisent la route : le coût est celui d'un appel de commande, avec l'attente de fraîcheur du SDK au premier appel (jusqu'à 500 ms, ADR-0065) ; sans garde d'occlusion, un menu ou un dialogue dessiné par-dessus peut recevoir le clic (le widget couvert n'est pas détecté) ; `Stale` suppose que le serveur a vu l'identifiant, donc un identifiant connu seulement du code de l'app donne `unknown_ref` tant qu'aucun `snapshot` ne l'a listé ; un champ `elements` absent plutôt que `false` est un choix de compatibilité qui demande de lire « absent » comme « faux » ; les webviews n'ont pas encore de références (lots suivants de la 1.2).
