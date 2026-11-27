# ADR-0050 — `clipboard_get` en monde ouvert et limites de la classification des outils

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0027 (annotations et classification, précisée ici), ADR-0045 (focus de la cible `kwin-eis`)

## Contexte et problème

ADR-0027 classait `clipboard_get` en lecture seule, hors monde ouvert, donc auto-approuvable par un client. Or l'outil lit le **vrai** presse-papiers de l'utilisateur (mots de passe copiés, données d'autres applications), hors de tout masquage du SDK : l'écart avec la règle « les autres outils agissent sur une app locale désignée » est net. Par ailleurs la revue a relevé que `type_text` et `press_key` en `kwin-eis` partent vers la fenêtre **active**, sans garde de focus (ADR-0045), alors que `key_combo` est destructif et pas eux.

## Facteurs de décision

- un client qui auto-approuve les outils en lecture seule ne doit pas lire des données de l'hôte sans confirmation ;
- ne pas dériver de la règle écrite sans la réécrire ;
- ne pas surclasser des outils dont la dangerosité vient d'un détournement de focus, déjà documenté.

## Options envisagées

1. **Laisser tel quel** : `clipboard_get` auto-approuvable.
2. **`clipboard_get` en monde ouvert** (`openWorldHint = true`, toujours lecture seule et idempotent) ; `type_text`, `press_key` et `navigate` restent classés selon la règle d'ADR-0027 ; le risque de focus est signalé dans les descriptions et la documentation.
3. **`clipboard_get` non lecture seule** : fausse la sémantique du protocole (il ne modifie rien).

## Décision

Option 2. La règle d'ADR-0027 devient : `openWorldHint = true` pour les outils dont les effets ou les données échappent à l'app désignée, soit `eval_js`, `invoke_command` **et `clipboard_get`** (données de l'hôte).

**Justification** : c'est le seul indice du protocole qui dit « ceci touche à l'extérieur de la cible » sans mentir sur la lecture seule. Le focus détourné en `kwin-eis` n'est pas un défaut de classification : la protection est `focus_window` avant une séquence, dite dans la description de `press_key`, de `type_text` et dans `SECURITY.md`.

## Conséquences

**Positives**

- un client prudent demande confirmation avant de lire le presse-papiers ;
- la table de classification du test et `docs/usage.md` suivent la nouvelle règle.

**Coûts et risques**

- changement visible pour les clients qui approuvaient `clipboard_get` seuls (note de version) ;
- un client qui ignore les indices reste exposé : ce sont des indices, pas une barrière (voir ADR-0027) ;
- `type_text` et `press_key` en `kwin-eis` restent sans garde de focus ; une vérification de la fenêtre active avant l'envoi serait un chantier distinct.

## Notes

Application : `tools_inspect.rs` (annotations et description), table `CLASSIFICATION` dans `lib.rs`, `docs/usage.md`, `SECURITY.md`.
