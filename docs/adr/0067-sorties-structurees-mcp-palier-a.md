# ADR-0067 — Sorties structurées MCP : texte inchangé, `structuredContent` restreint, `ToolError`

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0024 (échec de sérialisation visible), ADR-0025 (second bloc de métadonnées), ADR-0027 (annotations), ADR-0028 (version de protocole), ADR-0063 (`schemars` optionnel dans `deveye-core`), ADR-0064 (masquage des secrets)

## Contexte et problème

Les outils MCP renvoyaient du JSON uniquement sous forme de texte. Un client ne peut donc ni connaître d'avance la forme d'une réponse (`outputSchema`), ni la lire sans la parser (`structuredContent`), et les erreurs sémantiques (`isError`) étaient des objets `json!` écrits à la main, sans type commun. La spécification MCP 2025-06-18 ajoute ces trois champs ; `rmcp` 3.12 les porte. La promesse de stabilité (`docs/stability.md`) couvre « la forme de chaque réponse » : le texte existant ne doit pas changer.

Question ouverte de la conception : beaucoup de clients transmettent au modèle le texte **et** `structuredContent`, ce qui doublerait le coût des réponses volumineuses, alors que le thème de la 1.1 est l'économie de jetons.

## Mesure préalable (spike)

Un serveur MCP minimal (stdio, sans dépendance) renvoyant une liste de 120 éléments (9 Ko), lancé deux fois sous Claude Code 2.1.332 (`claude -p --mcp-config … --output-format stream-json`) : texte seul, puis texte + `structuredContent` + `outputSchema`.

- **Succès** : Claude Code transmet au modèle **un seul** exemplaire, le `structuredContent` à la place du texte (vérifié avec deux contenus volontairement différents : seul le contenu structuré apparaît dans le `tool_result`). Le coût d'entrée est identique dans les deux cas (même nombre de jetons en cache, 3 907). Aucun doublon dans ce client.
- **`isError: true`** : Claude Code transmet le **texte**, y compris quand un `structuredContent` non conforme à l'`outputSchema` l'accompagne ; aucune erreur de validation.

Conséquence : pour les outils du palier A, ce que le modèle lit dans Claude Code est le `structuredContent` ; il doit donc valoir exactement le texte. Les autres clients (doublon éventuel, validation stricte, clients officiels TypeScript et Python sur `isError`) n'ont pas pu être mesurés ici : le risque résiduel est borné en limitant le périmètre.

Limites de la mesure : elle date de la 1.1, a été faite à la main sur Claude Code 2.1.332, et son script et sa sortie brute n'ont pas été versionnés : elle n'est pas rejouable à l'identique. Le comportement d'un client peut changer d'une version à l'autre ; en cas de doute, refaire la mesure avec un serveur minimal (même liste de 120 éléments, texte seul puis texte + `structuredContent`). Le test de bout en bout en révision 2024-11-05 vérifie que le texte reste lisible, pas qu'un client strict ignore les champs additifs.

## Facteurs de décision

- aucun changement de texte : un client qui ignore les champs structurés lit les mêmes octets qu'en 1.0 ;
- pas de dérive entre le schéma déclaré et la réponse ;
- pas d'enveloppe pérenne créée pour contourner la contrainte « racine = objet » ;
- coût de `tools/list` (les schémas s'y ajoutent à chaque session).

## Options envisagées

1. **Remplacer le texte par du structuré seul** — casse les clients sans `structuredContent`. Rejetée.
2. **Envelopper tout en `{ value }`** — change le texte des outils existants. Rejetée.
3. **Texte inchangé + `structuredContent` + `outputSchema` sur les seuls outils à petite sortie objet** — retenue.
4. **Idem, généralisé** (`logs_tail`, `list_windows`, `find_element`, tableaux racine enveloppés) — doublerait le coût dans les clients qui transmettent les deux, et figerait des enveloppes (`{ windows }`) en dette de contrat. Rejetée pour la 1.1.
5. **`outputSchema` en union succès | erreur** — alourdit `tools/list`, brouille les clients. Rejetée.

## Décision

Option 3.

- **Palier A** (28 outils à la 1.1, 29 depuis la 1.2 avec `key_hold`, [ADR-0075](0075-modificateurs-d-entree-withmodifiers-et-keyhold-borne.md) ; liste de la 1.1, `tools/list` déclare un `outputSchema`, les résultats portent `structuredContent`) : `target_info`, `wait_for_selector`, `wait_for_text`, `wait_condition`, `wait_for_log`, `wait_screenshot_region`, `wait_target_alive`, `wait_ready`, `assert_selector`, `assert_state`, `window_geometry`, les acquittements `{ ok: true }` (`click`, `right_click`, `double_click`, `hover`, `type_text`, `press_key`, `key_combo`, `navigate`, `drag_drop`, `scroll`, `handle_dialog`, `upload_file`, `resize`, `clipboard_set`), `click_selector` et `click_text` (`{ ok, x, y }`), `focus_window` (`{ ok, title }`).
- **Hors palier** (texte ou image seuls, aucun `outputSchema`) : `logs_tail`, `logs_wait`, `list_windows`, `find_element`, `list_commands`, `collect_console_errors` (tableaux racine ou sorties volumineuses : le texte historique reste, sans enveloppe pérenne), `eval_js`, `dom`, `state`, `invoke_command` (valeur JSON arbitraire, racine possiblement non objet), `observe`, les captures, `assert_visual`, `clipboard_get` (contenu arbitraire de l'utilisateur).
- **Même source pour le texte, le structuré et le schéma** : chaque type de `responses.rs` (ou de `deveye-core` pour `WaitOutcome` et `WindowRect`, qui reçoivent la dérive `schemars` sous la feature `schema`, ADR-0063) produit le texte, le `structuredContent` et l'`outputSchema`. Les champs des structures qui remplacent d'anciens `json!` sont déclarés par ordre alphabétique pour conserver l'ordre historique des clés.
- **Échecs sémantiques du type de succès** (attente expirée, assertion fausse) : `isError: true`, texte et `structuredContent` du même type, conformes à l'`outputSchema`.
- **`ToolError`** `{ ok: false, reason, detail, hint?, candidates? }` remplace `unsupported`, `sdk_unavailable` et `interaction_failed`. `reason` est une énumération ouverte (`#[non_exhaustive]`, `snake_case`) ; `hint` et `candidates` sont omis tant qu'aucun site d'appel ne les renseigne, ce qui garde le texte de chaque erreur existante champ pour champ. Texte et `structuredContent` sont identiques, sur tous les outils.
- **Erreurs héritées non typées** (texte seul, inchangées) : `dom_too_large`, `js_exception`, `serialization_error`, l'échec d'un localisateur de `click_selector`/`click_text` (`not_found`, `not_visible`) et `expected` mal formé de `assert_state`, dont la forme porte des champs propres ou un `reason` en texte libre. Leur migration vers `ToolError` (champs supplémentaires, nouvelles causes) est reportée.
- **`outputSchema` ne décrit que le succès.** Une `ToolError` ne s'y conforme pas : un client qui valide `structuredContent` malgré `isError` peut la signaler comme erreur de validation (l'`isError` reste porté). Claude Code ne valide pas dans ce cas (mesuré). Retirer l'`outputSchema` d'un outil douteux se fait outil par outil, sans toucher au texte.
- **`state`** (hors palier) porte `_meta` : `io.deveye/captured_at`, `io.deveye/entry_count` et `io.deveye/state_source` (`sdk` si un pont SDK est câblé, `aucun` en mode CDP seul où le dump est toujours vide, ce qui distingue ce cas d'un SDK joignable sans signal). Le texte et les deux blocs sont inchangés.

## Conséquences

**Positives** : réponses lisibles par machine et schémas publiés pour les outils les plus appelés ; dérive impossible par construction ; texte garanti identique par les golden files ; aucun doublon de jetons dans Claude Code.

**Coûts et risques** : les `outputSchema` pèsent environ 10,7 Ko dans `tools/list` (dont 3,3 Ko pour `target_info`) ; un test fixe le plafond à 12 Ko. Les clients qui transmettent texte et structuré au modèle verront le palier A doublé : la restriction aux petites sorties le limite. Un client à validation stricte peut mal rendre une `ToolError` (ci-dessus). Les erreurs héritées coexistent avec `ToolError` jusqu'à leur migration. Cohérence texte/structuré non vérifiable pour les clients tiers (Cursor, clients Python ou TypeScript anciens) : couvert par le texte inchangé.

**Semver** : additif (champs `structuredContent`, `outputSchema` et `_meta` ajoutés, aucun texte modifié) : mineure 1.1.

## Notes

Tests (`adapter-mcp-server/src/contract_tests.rs`, `tests/golden/`) : golden files du texte capturés **avant** le changement (49 scénarios, succès et échecs), contrat sur toutes les branches des outils déclarant un `outputSchema` (`structuredContent` présent, égal au texte, conforme au schéma ; au moins un succès par outil), liste exacte des outils du palier A, plafond de taille, `_meta` de `state`. Client forçant la révision 2024-11-05 : JSON-RPC brut sur un tube mémoire et dans le test de bout en bout (`deveye-server/tests/e2e_mcp.rs`) ; un client `rmcp` aurait exigé la feature `client`, qui tire `tokio-stream`, dépendance non prévue pour cette version. Régénération volontaire des golden files : `DEVEYE_BLESS_GOLDEN=1 cargo test -p adapter-mcp-server golden`.
