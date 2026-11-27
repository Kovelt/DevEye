# ADR-0062 — Schéma `deveye.ui/1` de la route `ui.widgets`

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (stabilité : politique de versionnage), ADR-0012 (`#[non_exhaustive]`), ADR-0043 (pont natif via le SDK), ADR-0044 (cible x11), ADR-0063 (schemars optionnel), ADR-0064 (masquage transversal)

## Contexte et problème

Pour une app sans DOM (egui, GTK, Godot, Uno…), la cible `x11` lit comme « DOM » la réponse de la route de commande `ui.widgets`, sans l'interpréter : `DomTree.tree` est un `serde_json::Value` libre et le schéma des nœuds « appartient à l'app » (guide x11). Le consommateur existe, mais le contrat n'est qu'une convention : chaque app, chaque shim et le futur addon Godot divergeraient, et aucun agent ne saurait quels champs lire. Les désignations par référence prévues ensuite (snapshot, act) exigent un format commun. La 1.0 est publiée : le contrat doit être figé de façon additive, sans toucher la surface stable.

## Facteurs de décision

- les apps embarquent le SDK : les types doivent être MIT, donc dans `deveye-core` (ADR-0002) ;
- peu coûteux pour une UI en mode immédiat (reconstruite à chaque frame) ;
- évolutif sans rupture : champs ajoutés en 1.x, lecteurs tolérants ;
- aucune nouvelle dépendance obligatoire ;
- les apps déjà écrites (format sans `schema`) doivent rester lisibles ;
- les unités des rectangles sont le piège principal du terrain (points logiques ou pixels de capture).

## Options envisagées

1. **Statu quo** — schéma libre propre à chaque app. Rejeté : divergence garantie.
2. **Arbre imbriqué (`children`)** — rejeté : coûteux à produire en mode immédiat et à référencer ; une liste plate avec `parent` se convertit sans perte dans les deux sens.
3. **Réutiliser AccessKit ou l'arbre d'accessibilité CDP** — rejeté comme contrat : couplage de versions et vocabulaire trop riche ; reste une source de conversion possible (voir « Options reportées »).
4. **Types dans le seul serveur (AGPL)** — rejeté : les apps embarquent le SDK, les types doivent être MIT.
5. **Types MIT dans `deveye-core`, liste plate + `parent`** — retenue.

## Décision

Option 5. Le module `deveye_core::ui` définit `UiTree`, `UiWidget`, `UiState`, `UiScroll`, `UiWindow`, `UiRole`, `UiTreeBuilder` et les types de contrôle (`UiIssue`, `UiReadError`).

- **Forme** : `{ schema: "deveye.ui/1", generation, window { width, height, scale? }, focus?, published_at_ms?, widgets[] }`. Un widget porte `id` (stable, fourni par l'app, unique), `parent?`, `role`, `label?`, `value?`, `rect`, `state`, `actions`, `scroll?`, `clipped?`.
- **Rôle** : chaîne ouverte, vocabulaire recommandé calqué sur ARIA ; une valeur inconnue est conservée telle quelle.
- **Unités** : `rect = [x, y, largeur, hauteur]` en pixels entiers **dans le repère des gestes de la cible** (origine en haut à gauche de la fenêtre) ; `scale` est informatif. Les coordonnées fractionnaires sont ramenées à la plus petite boîte entière qui contient le rectangle réel (`x`, `y` au plancher, bords droit et bas au plafond, taille par différence) : un clic au centre ne sort jamais du widget. egui multiplie ses points logiques par `pixels_per_point` avant de publier.
- **Compatibilité des types** : tous `#[non_exhaustive]` dès leur création, avec constructeurs et méthodes `with_*` publics (les crates tiers ne peuvent pas construire par littéral) ; `UiState` a des valeurs par défaut (`enabled` et `visible` à vrai) pour que les champs absents se lisent sans erreur.
- **Versionnage** : une évolution mineure ajoute des champs optionnels, que les lecteurs ignorent ; une rupture publie `deveye.ui/2`, lu en plus de `/1`. Le lecteur refuse nommément un schéma qu'il ne connaît pas (`UiReadError::UnsupportedSchema`).
- **Lecteur tolérant du format legacy** (sans `schema`) : liste racine ou `{ window?, widgets }` ; `kind` tient lieu de `role`, `text` de `label` ; `rect` en tableau ou en objet `{ x, y, width, height }` ; un widget sans `id` reçoit `role[index]` marqué `id_ephemeral`. L'exemple du guide x11 est lu et converti.
- **Contrôle** : `UiTree::validate` relève ids vides ou dupliqués, parents inconnus, cycles, focus inconnu, tailles négatives (erreurs) et widgets entièrement hors fenêtre (avertissement : le contenu défilant est légitime ; les widgets cachés ou `clipped` sont ignorés).
- **Secrets** : `UiTreeBuilder::build` omet la valeur de tout widget `state.secret` ou de rôle `password` ; `mask_secret_values` sert à la lecture côté serveur (ADR-0064).
- **Publication du schéma** : `docs/schemas/ui-widgets-1.json`, généré par les types (ADR-0063) et comparé par un test ; fixtures versionnées dans `deveye-core/tests/fixtures/ui-1/` pour les implémenteurs tiers.
- **Négociation** : la route continue de recevoir `{}` tant que `/devtools/info` n'annonce pas `ui.widgets/1` (un handler strict sur son payload casserait sinon) ; l'ajout de `accept_schema` relève du lot du helper SDK.
- **Statut** : le schéma `deveye.ui/1` est ajouté à la surface stable (ligne dédiée de `docs/stability.md`), sans modifier aucun élément existant.

**Options reportées**

- **Pont AccessKit** : egui produit un `TreeUpdate` AccessKit exploitable, mais egui 0.37 exige Rust 1.95 (MSRV du dépôt : 1.88), deux versions majeures d'`accesskit` coexistent dans l'écosystème et `egui-baseview` suit un troisième rythme. Rien n'est implémenté tant qu'un spike n'a pas tranché ce couplage ; un éventuel pont sera un ADR et un crate séparés, jamais une dépendance de `deveye-core`.
- **`children` imbriqués** : non reportés mais rejetés ; une vue imbriquée se dérive de la liste plate.

**Justification** : seul ce chemin donne un contrat partagé MIT, bon marché pour le mode immédiat et évolutif sans rupture. Le lecteur tolérant évite de casser les apps existantes ; la règle d'arrondi et l'énoncé du repère retirent l'ambiguïté d'unités.

## Conséquences

**Positives**

- un seul vocabulaire pour les cibles sans DOM : egui, Godot, Uno, GTK ;
- les implémenteurs tiers disposent d'un schéma JSON et de fixtures ;
- aucun changement de forme d'un élément stable existant ; `dom` renvoie la valeur de la route, dont seules les valeurs secrètes sont masquées (amendé par ADR-0064 : la décision initiale « `dom` renvoie toujours la valeur de la route » n'a pas tenu face au masquage).

**Coûts et risques**

- figer trop tôt un schéma qu'on regretterait : atténué par `non_exhaustive`, les champs optionnels, la version n / n-1 et le lecteur tolérant ;
- `rect` entier perd la précision sub-pixel (volontaire : même repère que les gestes) ;
- un `id` éphémère n'est pas une référence durable.

## Notes

Application : `deveye-core/src/ui.rs`, `deveye-core/tests/` (fixtures, schéma, exemples de `docs/usage.md`), `docs/schemas/ui-widgets-1.json`. Le helper SDK, la lecture typée côté serveur, le contrôle de `doctor` et le scénario de conformité arrivent dans les lots suivants de la 1.1.
