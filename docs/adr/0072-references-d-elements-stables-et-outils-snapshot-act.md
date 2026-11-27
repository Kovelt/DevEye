# ADR-0072 — Références d'éléments stables et outils `snapshot` et `act`

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0069 (surface expérimentale), ADR-0062 (schéma `deveye.ui/1`), ADR-0067 (sorties structurées), ADR-0068 (capacités additives), ADR-0070 (compatibilité de `default-features = false`), ADR-0041 (architecture hexagonale)

## Contexte et problème

Un agent désigne aujourd'hui un élément par un sélecteur CSS, un texte ou des coordonnées. Aucune référence ne survit d'un appel à l'autre, aucun arbre compact n'existe, et hors DOM (widgets natifs) il n'y a rien à désigner à part des pixels. La feuille de route 1.x prévoit deux outils, `snapshot` (arbre compact d'éléments avec références) et `act` (agir par référence), pour toutes les familles de cible : widgets natifs (`ui.widgets`), pont JS de WebKitGTK (le chemin réel sous Linux) et CDP.

Cet ADR décide du **modèle** (types, table de références, port). Les adapters et les outils MCP arrivent dans les lots suivants de la 1.2 et reprennent ces choix ; leurs ajustements éventuels amendent cet ADR.

## Facteurs de décision

- aucune rupture de la surface stable (ADR-0011) : le port et les types ne sont pas prêts pour la promesse semver (ADR-0069) ;
- une feature Cargo ne peut qu'**ajouter** (ADR-0070) : aucun élément existant ne passe derrière elle ;
- pas de rebind silencieux : cliquer le mauvais bouton « Supprimer » est pire qu'une erreur ;
- le rect d'un instantané vieillit vite : seule une mesure au moment de l'action est fiable ;
- domaine pur (règle 5 du `CLAUDE.md`) : la table de références doit se tester sans E/S.

## Options considérées

1. **Références recalculées à chaque instantané (`e1..eN`)** : l'agent perd ses références alors que les nœuds n'ont pas bougé. Rejeté.
2. **Références par sélecteur CSS généré** : fragile, long, inexistant hors DOM. Rejeté.
3. **Rebind automatique par rôle et nom après re-rendu** : risque d'agir sur le mauvais élément. Rejeté par défaut ; l'erreur propose des candidats à la place.
4. **Deux familles de références, table pure, relocalisation à l'action** : retenu.

## Décision

**Types** (`deveye-core`, module `elements`, feature `unstable-elements`) : `ElementNode` (`ref`, rôle, nom, valeur, état, rect indicatif, `scrollable`, `offscreen`, `ephemeral`, enfants), `ElementSnapshot` (`generation`, `target_kind`, `truncated`, nœuds, `focus`), `SnapshotQuery` (`scope`, `max_depth`, `max_nodes` à 400 par défaut, `interactive_only` vrai par défaut, `viewport_only`), `ElementLocation` (rect remesuré, centre). Tous `#[non_exhaustive]`, construits par constructeurs.

**Deux familles de références**, lisibles à leur préfixe :

- `e<n>` pour les cibles à DOM : numéro monotone attribué par `RefTable`, stable tant que le nœud vit, **jamais réutilisé** dans une session (le compteur survit à l'invalidation) ;
- `w:<id>` pour les widgets natifs : l'identifiant fourni par l'app. La stabilité d'une relance à l'autre n'est promise que si l'app fournit des identifiants déterministes ; un identifiant dupliqué ou synthétisé (`ephemeral`) ne l'est pas, et `doctor` le signalera (lot suivant).

**`RefTable<K>`** (pure, sans E/S) associe la clé d'un nœud côté backend à `e<n>`. `retain` et `forget` périment les nœuds disparus ; `invalidate` ouvre une nouvelle génération (navigation principale, reconnexion, changement de backend) et périme tout. Une référence se résout en clé, ou en `RefError::Stale` (numéro déjà émis mais mort) ou `RefError::Unknown` (jamais émis ou forme invalide) : la distinction commande le hint de l'outil (« relancer snapshot » contre « référence inventée »).

**Port `ElementInspector`** (`deveye-ports`, même feature) : `snapshot(&SnapshotQuery)` et `locate(&str)`. Son erreur, `ElementError` (`Stale` avec candidats de même rôle et nom, `UnknownRef`, `Obscured`, `Unsupported` avec hint, `Port`), est **distincte de `PortError`** : `PortError` reste inchangé, donc aucun élément stable n'est touché. `locate` remesure à chaque appel ; l'action elle-même passe par `InteractionSink` avec une `InteractionCommand` existante, donc aucune obligation nouvelle pour les adapters d'interaction.

**Garde-fous de livraison** : la feature `unstable-elements` de `deveye-ports` active celle de `deveye-core`, n'ajoute aucune dépendance et est désactivée par défaut. `cargo-semver-checks` est exécuté **avec et sans** la feature (CI et `just semver`). Les outils MCP `snapshot` et `act` seront annoncés « [expérimental] » (ADR-0069) et exclus de la promesse ; ils figureront dans `docs/stability.md` seulement à la graduation, évaluée à la 1.5.

## Conséquences

**Positives** : un agent désigne un élément sans CSS ni coordonnées sur toute famille de cible ; la table est testée sans navigateur ; la 1.2 ne promet rien qu'elle ne puisse changer.

**Coûts et risques** : les features Cargo s'unifient, donc un consommateur qui active `unstable-elements` expose tout son graphe à des ruptures en mineure ; les références `e<n>` sont perdues quand une liste à clés est recréée (atténué par les candidats) ; `RefTable` suppose qu'un adapter fournit une clé backend stable dans une session (`backendNodeId` CDP, identifiant JS du pont), à invalider sur `Target.detachedFromTarget` et navigation.
