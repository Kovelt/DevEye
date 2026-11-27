# ADR-0034 — Capacités complètes de `target_info`, repli observable et choix unique du backend OS

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (ajout de champs de réponse : mineure), ADR-0033 (champ `server`), ADR-0031 (validation des flags)

## Contexte et problème

Trois défauts se cumulaient dans `deveye-server` :

1. En mode `auto`, la bascule du CDP vers le pont natif (SDK + OS) était muette : aucune trace, aucun champ ne disait quel backend servait. L'agent croyait piloter le webview par CDP alors que ses gestes partaient en événements JS synthétiques.
2. `target_info` était une description statique : `list_windows: true` pour `kwin-eis` même hors KDE/Wayland (la limite n'était dite qu'en notes), pas de champ pour la géométrie de fenêtre, le focus, le gestionnaire de fenêtres, le presse-papiers ni pour les outils qui exigent du JS ; le doc-comment de `list_windows` (« CDP uniquement ») était périmé.
3. Le choix « KWin ou xcap » était recopié cinq fois dans `compose.rs` (`match backend { Kwin, Xcap, Auto }`), avec des `eprintln!` épars et un paramètre `_title` inutilisé.

## Facteurs de décision

- l'agent doit pouvoir décider sans tâtonner, d'où `target_info` en premier appel ;
- ne pas annoncer une capacité que la session ne peut pas servir ;
- ajouts uniquement additifs sur la réponse JSON (ADR-0011) ;
- un seul point de décision pour le backend OS ;
- diagnostics sur stderr via `tracing` (stdout est le canal MCP).

## Options envisagées

1. **Notes en texte libre seulement** : rien à coder, mais non exploitable par un agent.
2. **Champs structurés calculés au démarrage + backend effectif lu à chaque appel**.
3. **Tout rendre dynamique** (sonder KWin à chaque `target_info`) : coûteux et inutile, la session ne change pas.

## Décision

Option 2.

- `TargetInfo` gagne `effective_backend`, `window_geometry`, `focus_window`, `window_manager` (`kwin` | `x11` | `aucun`), `clipboard`, `js_dependent_tools` et `server` (ADR-0033). Les capacités sont calculées au démarrage d'après la session réelle (`HostCaps`) : `list_windows`, `window_geometry` et `focus_window` sont faux pour `kwin-eis` hors KDE/Wayland, `focus_window` exige en plus `--screencap-window` (le gestionnaire KWin est construit avec ce titre), le presse-papiers suit la session Wayland et reste faux en `x11`.
- `effective_backend` : `cdp`, `sdk` (repli), `indetermine` (mode `auto` avant le premier appel si le CDP était injoignable au démarrage), ou le nom de la cible pour les autres. Il est relu à chaque appel de `target_info` par une sonde (`EffectiveBackendProbe`) sur un `FallbackState` partagé par les quatre façades de repli.
- Chaque **bascule** est journalisée (avertissement au passage au pont natif, information au retour du CDP), pas chaque appel : le journal reste lisible quand le CDP est tombé pour longtemps.
- `compose.rs` : `use_kwin(backend)` (et son cœur pur `kwin_selected`, testé) est l'unique décision KWin/xcap ; les `eprintln!` deviennent des événements `tracing` ; le paramètre `_title` de `build_kwin_window_lister` disparaît ; le doc-comment de `list_windows` est corrigé.
- `list_windows` reste `true` en `auto` : il est servi par le CDP seul (pas de repli), donc il échoue quand le CDP est tombé ; la note le dit.

**Justification** : les capacités ne changent pas en cours de session (calcul au démarrage), le backend effectif si (sonde). Les champs structurés remplacent des notes qu'un agent devait interpréter.

## Conséquences

**Positives**

- plus de repli silencieux ; l'agent peut relire `target_info` après une erreur de geste ;
- `target_info` ne ment plus sur KWin hors KDE ;
- une seule décision de backend, testée.

**Coûts et risques**

- `TargetInfo` et `DeveyePorts` (adapter interne) changent de forme ; les ajouts à la réponse JSON sont additifs ;
- la liste `js_dependent_tools` est tenue à la main dans `compose.rs` : un outil JS ajouté plus tard doit y figurer ;
- en `auto`, la sonde reflète le dernier appel : un `target_info` pris juste après un retour du CDP peut afficher `sdk` jusqu'au prochain appel primaire.

## Notes

Application : `deveye-adapters/adapter-mcp-server/src/lib.rs` et `tools_inspect.rs`, `deveye-server/src/{compose,fallback,cdp}.rs`, `docs/usage.md` et `docs/architecture.md`. Tests : `compose::tests`, `fallback::tests`, `dispatch_tests::target_info_exposes_versions_capabilities_and_live_backend`.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
