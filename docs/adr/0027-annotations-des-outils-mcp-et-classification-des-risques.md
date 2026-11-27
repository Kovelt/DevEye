# ADR-0027 — Annotations des outils MCP et classification des risques

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (stabilité : noms et schémas des outils MCP couverts par semver), ADR-0028 (version du protocole MCP), ADR-0029 (outil `observe`), ADR-0030 (`assert_visual` enrichi), ADR-0050 (`clipboard_get` en monde ouvert : précise la règle)

## Contexte et problème

Aucun des outils MCP de DevEye ne portait d'annotation. Les clients MCP s'en servent pour leur politique d'approbation : lancer sans confirmation un outil de lecture, demander confirmation pour un outil susceptible de détruire. Sans annotation, un client prudent traite `screenshot` comme `eval_js` (confirmation partout, donc fatigue d'approbation), un client permissif traite `eval_js` comme `screenshot`. La 1.0 fige les noms et schémas des outils (ADR-0011) : le classement doit être posé avant le gel et tenu à jour mécaniquement.

## Facteurs de décision

- un client doit pouvoir auto-approuver l'observation pure sans risque ;
- l'omission d'un indice vaut son pire cas dans la spécification (`destructiveHint` absent = vrai) : les poser tous explicitement, sinon le classement n'est pas lisible ;
- les indices sont des **conseils**, jamais une garantie : ils ne doivent pas faire croire à une sécurité que DevEye ne fournit pas ;
- un nouvel outil ne doit pas pouvoir être ajouté sans réflexion sur son risque.

## Options envisagées

1. **Annoter seulement les outils de lecture** (`readOnlyHint`), laisser le reste par défaut : minimal, mais le défaut de la spécification (destructif, non idempotent, monde ouvert) sur-alerte les clics et la saisie.
2. **Poser les quatre indices sur chaque outil**, avec une règle de classement écrite et un test d'exhaustivité.
3. **Une annotation unique « sûr / non sûr »** : ne correspond pas au vocabulaire du protocole, aucun client ne la lit.

## Décision

Option 2 : chaque `#[tool]` déclare `annotations(read_only_hint, destructive_hint, idempotent_hint, open_world_hint)`, et le test `every_tool_is_annotated_and_classified` échoue si un outil n'en porte pas les quatre ou s'il manque à la table de classification du test.

**Règle de classement** (reprise dans `docs/usage.md`) :

- `readOnlyHint = true` : l'outil ne modifie ni l'app cible ni le système hôte. Vider un buffer d'observation propre à DevEye (logs, console) reste de la lecture, mais rend l'outil **non idempotent** (deux appels successifs ne rendent pas la même chose) : `logs_tail`, `logs_wait`, `collect_console_errors`, `wait_for_log`, `wait_condition` (prédicat `log-message`), `observe`.
- Un outil qui modifie l'app sans en détruire l'état (clic, saisie, défilement, glisser-déposer, réponse à un dialogue) est `readOnlyHint = false`, `destructiveHint = false`. Un clic pouvant déclencher n'importe quelle action de l'app, c'est une limite assumée : la spécification d'un outil générique ne peut pas deviner le bouton visé. `screenshot_element` entre ici parce qu'il fait défiler la page.
- `destructiveHint = true` quand l'effet peut être une perte ou ne pas être borné par DevEye : `eval_js` et `invoke_command` (code ou logique arbitraires de l'app), `navigate` (quitte l'écran courant), `upload_file` (transmet des fichiers de l'hôte à l'app), `clipboard_set` (écrase le presse-papiers de l'utilisateur), `key_combo` (les raccourcis avec modificateurs sont le moyen usuel de quitter, fermer, enregistrer par-dessus ou supprimer), `assert_visual` (écrit, et avec `update_baseline` remplace, un fichier de référence).
- `idempotentHint = true` quand répéter l'appel avec les mêmes arguments laisse l'état identique (`hover`, `resize`, `focus_window`, `navigate`, `upload_file`, `clipboard_set`, lectures pures).
- `openWorldHint = true` seulement pour `eval_js` et `invoke_command`, dont les effets (réseau, système) échappent à DevEye ; les autres outils agissent sur une app locale désignée.

**Justification** : seule l'option 2 donne aux clients un classement exploitable et empêche la dérive. Les choix `key_combo`, `upload_file` et `assert_visual` en destructifs dépassent la lettre du plan (qui citait `click`/`type` en non destructifs et `eval_js`, `invoke_command`, `clipboard_set`, `navigate` en destructifs) : ils appliquent la même règle (perte possible ou effet non borné), d'où le statut « Proposé » en attente de validation du mainteneur.

## Conséquences

**Positives**

- les clients peuvent auto-approuver 16 outils d'observation pure (`clipboard_get`, monde ouvert, en est exclu : ADR-0050) et 6 de lecture avec drain ;
- ajouter un outil sans le classer casse la CI ;
- la règle est écrite : un désaccord se tranche en changeant l'ADR, pas en relisant le code.

**Coûts et risques**

- ce sont des indices : un client ne doit pas s'y fier pour la sécurité (un `click` peut supprimer des données) ; la barrière réelle reste l'usage dev-only de DevEye ;
- changer la classification d'un outil change ce que les clients approuvent seuls : à traiter comme un changement visible (note de version), sans casser le schéma ;
- la table du test double l'information des attributs : volontaire, c'est elle qui force la relecture.

## Notes

Application : `#[tool(... annotations(...))]` dans `deveye-adapters/adapter-mcp-server/src/tools_*.rs`, test et table dans `lib.rs` (`dispatch_tests::CLASSIFICATION`). API `rmcp` 3.12 (`ToolAnnotations`, attribut `annotations(...)` de `#[tool]`).

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
