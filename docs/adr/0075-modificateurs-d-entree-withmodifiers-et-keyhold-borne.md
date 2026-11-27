# ADR-0075 — Modificateurs d'entrée par décorateur `WithModifiers` et `KeyHold` borné

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (stabilité), ADR-0012 (`InteractionCommand` non exhaustif), ADR-0021 (validation des gestes), ADR-0027 (annotations MCP), ADR-0045 (focus et kwin-eis), ADR-0067 (sorties structurées), ADR-0041 (architecture hexagonale)

## Contexte et problème

Les gestes de pointeur (`Click`, `DoubleClick`, `Hover`, `DragDrop`, `Scroll`) ne portent aucun modificateur : seul `KeyCombo` en transporte. Un agent ne peut donc pas étendre une sélection par Shift+clic, ajouter un élément par Ctrl+clic ni glisser avec Alt. Les adapters ont pourtant déjà tout le nécessaire : `adapter-x11` suit les touches et boutons enfoncés et relâche tout sur erreur, `adapter-cdp-webview` calcule déjà un masque de modificateurs pour `KeyCombo`, `adapter-interaction-kwin-eis` sait presser des touches. Il manque un moyen de **porter** les modificateurs sans casser la surface stable de la 1.x, et un appui maintenu (« tant que Shift est tenu ») qui ne laisse jamais une touche collante.

## Facteurs de décision

- aucune rupture de la surface stable (ADR-0011) : `InteractionCommand` est `#[non_exhaustive]` (ADR-0012), mais **ses variantes struct ne le sont pas** : ajouter un champ à `Click`, `Scroll` ou `DragDrop` casserait les destructurations des consommateurs de la bibliothèque MIT ;
- une seule table de noms de modificateurs, partagée par tous les adapters, sans qu'aucun adapter dépende d'un autre (règle hexagonale) ;
- pas d'état collant entre deux appels d'outil : une touche qui reste enfoncée parce que l'agent s'est arrêté, que le serveur a redémarré ou que le client a planté est pire qu'une fonction absente ;
- un nom de modificateur inconnu ne doit jamais produire un geste sans modificateur ;
- les champs des variantes d'une enum étant publics, un invariant tenu par le seul constructeur est contournable (littéral, JSON).

## Options considérées

1. **Ajouter `modifiers` à `Click`, `DoubleClick`, `DragDrop`, `Scroll`** : compatible côté JSON (valeur par défaut serde) mais **cassant côté Rust** tant que les variantes ne sont pas `#[non_exhaustive]`. Reporté à la 2.0 si jamais retenu.
2. **Paire `KeyDown` / `KeyUp` persistante** : état collant entre deux appels, nettoyage impossible sur crash. Rejetée.
3. **Étendre `key_combo` avec une position** : mélange clavier et pointeur, schéma d'outil confus. Rejetée.
4. **Décorateur `WithModifiers` et variante `KeyHold` bornée** : additif sur une enum non exhaustive, se généralise à tout geste de pointeur futur, et `KeyHold` est atomique. Retenue.

## Décision

**Domaine** (`deveye-core`, MIT, aucune dépendance nouvelle) : un module `modifier` avec l'enum `Modifier { Ctrl, Shift, Alt, Meta }` (`#[non_exhaustive]`, sérialisée sous son nom canonique, alias acceptés à la lecture : `control`, `option`, `super`, `win`, `cmd`, `command`) et la constante `MAX_KEY_HOLD_MS` à 10 000. Deux variantes nouvelles :

- `WithModifiers { modifiers: Vec<Modifier>, command: Box<InteractionCommand> }` n'enveloppe que `Click`, `DoubleClick`, `Hover`, `DragDrop` et `Scroll` : ni imbrication, ni `KeyHold` (le plafond de durée ne peut donc pas être contourné par cumul) ;
- `KeyHold { key, duration_ms }` maintient une touche puis la relâche en un seul appel, `duration_ms` au plus `MAX_KEY_HOLD_MS`.

**Invariants vérifiés trois fois** : à la construction (`with_modifiers`, `key_hold` renvoient `Result<_, InvalidCommand>`), **à la désérialisation** (`deserialize_with` sur les deux champs : un JSON hostile, imbriqué ou au-delà du plafond, est refusé) et par `InteractionCommand::validate()`, que **chaque adapter appelle avant toute émission** parce qu'un littéral contourne les constructeurs. Une liste de modificateurs vide est acceptée et vaut le geste nu.

**Adapters** :

- `x11` : `keys::plan_modifiers` résout les modificateurs en keycodes **avant** la première émission (appuis dans l'ordre, relâchements en ordre inverse, doublons écartés) ; les pas du geste interne sont encadrés d'appui et de relâchement ; `execute` relâche déjà tout sur erreur. `KeyHold` insère la pause entre l'appui et le relâchement de la frappe planifiée ;
- `kwin-eis` : le clavier est un device distinct, qui ouvre sa propre séquence d'émission ; `run_with_held_keys` presse les modificateurs, exécute le geste puis relâche **toujours** en ordre inverse, que le geste ait réussi ou non. Une erreur de flush après les relâchements est rapportée sans masquer l'erreur du geste, et une session en erreur est jetée par le worker (la fermeture du descripteur fait lâcher les touches au compositeur). `KeyHold` pompe les événements du compositeur par tranches de 100 ms pendant l'attente (pings) et relâche même si la connexion tombe. Un device clavier confondu avec le pointeur serait une séquence imbriquée : refus explicite (KWin expose un device par capacité) ;
- `cdp` : le masque `modifiers` de `Input.dispatchMouseEvent` (Alt 1, Ctrl 2, Meta 4, Shift 8) ; **aucun** `keyDown` de modificateur n'est émis, donc une page qui écoute les touches (et pas seulement `event.shiftKey`) ne voit rien. `KeyHold` : `Unsupported` ;
- pont SDK (`JsInteraction`) : les booléens `ctrlKey`, `shiftKey`, `altKey`, `metaKey` des événements synthétiques ; avec modificateurs, `Scroll` émet d'abord un `WheelEvent` qui les porte (le défilement natif n'a lieu que si aucun handler n'appelle `preventDefault`). Un événement synthétique n'est pas `isTrusted`. `KeyHold` : `Unsupported`.

**Outils MCP** : paramètre optionnel `modifiers` (liste de chaînes) sur les huit outils de pointeur `click`, `right_click`, `double_click`, `hover`, `drag_drop`, `scroll`, `click_selector` et `click_text` ; un nom inconnu donne `invalid_params`, détecté avant le verrou des gestes et avant tout dispatch. Un outil nouveau, `key_hold { key, duration_ms }`, annoté **destructif, non idempotent, hors monde ouvert** (comme `key_combo` : une touche tenue peut quitter, fermer ou valider), qui déclare l'`outputSchema` d'un acquittement. Il garde le verrou des gestes pendant la durée demandée (au plus 10 s) : les autres appels attendent, ce qui est documenté.

**Stabilité** : mineure. `deveye-core` gagne un type, une constante, une erreur et deux variantes d'une enum `#[non_exhaustive]` ; les ports ne changent pas ; le wire protocol du SDK ne change pas (les interactions ne passent pas par lui). Le schéma des outils existants gagne un paramètre optionnel, la liste des outils stables passe de 44 à 45. Aucun test d'outil existant ne change de résultat ; les seuls ajustements sont les littéraux de paramètres des tests, qui reçoivent le champ optionnel.

## Conséquences

**Positives** : Shift+clic, Ctrl+glisser et Alt+molette fonctionnent sur x11 et kwin-eis par de vraies touches, et sur cdp et le pont SDK pour les pages web ; une touche ne reste jamais collée par construction ; la table des noms est unique ; la désérialisation refuse les commandes hostiles.

**Coûts et risques** :

- sur cdp, le geste modifié n'est pas une vraie frappe (pas de `keydown` de modificateur) ; sur le pont SDK, les événements ne sont pas de confiance : une page qui filtre `isTrusted` ignorera le geste ;
- `KeyHold` n'existe ni en cdp ni via le pont SDK au départ : `Unsupported` explicite plutôt qu'une approximation (pas d'auto-répétition, pas d'état clavier réel) ;
- sous Wayland, l'auto-répétition d'une touche maintenue dépend du compositeur ; le comportement de `KeyHold` sur une session KDE vivante reste à valider à la main (le dialogue d'autorisation EIS est interactif), de même que Shift+clic dans un gestionnaire de fichiers ;
- une touche XTEST pressée reste enfoncée côté serveur X si le serveur DevEye est tué (`SIGKILL`) entre l'appui et le relâchement : le nettoyage de `execute` ne couvre que l'erreur, pas la mort du processus. La borne de 10 s limite la fenêtre d'exposition d'un `KeyHold`, pas celle d'un geste modifié ; un Xvfb privé jetable rend le risque négligeable ;
- `key_hold` bloque le verrou des gestes jusqu'à 10 s : un client dont le délai d'appel est plus court verra l'appel expirer alors que la touche est bien relâchée côté serveur.

**Suites possibles** : `KeyHold` en cdp par `dispatchKeyEvent` si un besoin réel apparaît ; `#[non_exhaustive]` sur les variantes struct en 2.0, qui rendrait possible l'option 1.

## Amendements issus de la revue de suivi

- **Désérialisation** : `KeyHold` refuse aussi un nom de touche vide (les trois contrôles sont ainsi identiques) ; le geste interne d'un `WithModifiers` est lu par une énumération restreinte aux cinq gestes de pointeur, sans récursion sur `InteractionCommand` : une imbrication hostile est refusée à la première couche, même par `from_value`.
- **x11** : les pas de modificateurs comptent comme du clavier (`needs_keyboard`) ; sans gestionnaire de fenêtres, le focus est donc fixé sur la fenêtre cible avant l'appui d'un modificateur.
