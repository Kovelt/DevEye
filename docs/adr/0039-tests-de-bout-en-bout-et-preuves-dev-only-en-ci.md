# ADR-0039 — Tests de bout en bout et preuves dev-only en CI

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0003 (GitHub Actions), ADR-0010 (toolchain et MSRV), ADR-0011 (surface stable), ADR-0035 (conformité), ADR-0037 (Linux seul), ADR-0038 (distribution)

## Contexte et problème

Aucun test ne lançait le binaire en `--serve-mcp` ; les tests Xvfb de l'adapter X11 étaient facultatifs en CI (`allow_failure`) et l'un d'eux passait en silence quand sa précondition manquait ; la promesse centrale (« le SDK n'ouvre rien en release ») n'était prouvée que par un test, pas sur un binaire réel ; et la CI GitLab disparaît avec la migration vers GitHub (ADR-0003). Il faut décider de la forme de ces garanties et de la CI qui les porte.

## Facteurs de décision

- tester ce que livre vraiment le projet : le binaire, parlant JSON-RPC sur stdio ;
- aucune dépendance Cargo nouvelle, aucune interférence avec la session graphique de l'utilisateur ;
- une preuve négative (« rien dans le binaire ») n'est crédible qu'avec un témoin positif ;
- supply chain : actions épinglées, permissions minimales, SBOM ;
- mêmes commandes en local (`just`) et en CI.

## Options envisagées

1. **Test MCP dans `adapter-mcp-server` contre des ports factices** : ne couvre ni le câblage du binaire ni le transport stdio.
2. **Test d'intégration de `deveye-server` lançant le binaire sur un Xvfb privé**, avec une fenêtre fixture créée par le test.
3. **Fixture en application externe** (un binaire d'exemple) : un artefact de plus à construire, sans gain face à une fenêtre X11 de quelques lignes.

Pour la preuve no-op : (a) test seul en release (existant) ; (b) inspection des symboles par `nm` ; (c) recherche des chaînes propres au serveur HTTP dans un exemple compilé, avec témoin positif en debug.

## Décision

- **Bout en bout** : option 2. `deveye-server/tests/e2e_mcp.rs` lance `deveye-server --target x11 --serve-mcp` sur un Xvfb privé (display libre entre `:231` et `:239`, plage distincte de celle d'`adapter-x11`, sans écoute TCP, jamais `pkill -f`), avec une fenêtre fixture rouge titrée créée en `x11rb` (dev-dependency déjà présente dans le graphe). Il vérifie `initialize`, la liste d'outils (nombre exact), `target_info` cohérent, un `screenshot` PNG non vide et un `click` dont l'appui est reçu par la fixture aux bonnes coordonnées. `#[ignore]`, exécuté en CI avec `--ignored --test-threads=1`, bloquant. Le nombre d'outils est la constante publique `EXPECTED_TOOL_COUNT` d'`adapter-mcp-server`, exigée aussi par le test de classification des outils.
- **Preuve no-op** : option (c). L'exemple `deveye-sdk/examples/release_probe.rs` appelle `init` sans garde `cfg` ; `scripts/verify-sdk-release-noop.sh` le compile en debug (témoin : les routes `/devtools/` et l'en-tête de version y figurent), puis en release (elles ne doivent y figurer à aucun titre), puis l'exécute. Recette `just release-noop`, job CI dédié.
- **CI** : `.github/workflows/ci.yml` (ubuntu-24.04 épinglé, permissions `contents: read`, cache Cargo) avec les jobs : format ; clippy, tests et documentation avec `RUSTDOCFLAGS=-D warnings` ; tests Xvfb et e2e (bloquants) ; MSRV 1.88 ; `cargo deny check` ; conformance ; preuve no-op du SDK ; `cargo-semver-checks` seulement si un tag `v1.*` existe (étape sautée sinon, avec note dans le résumé) ; SBOM CycloneDX en artefact. Les actions tierces sont épinglées par SHA de commit ; la toolchain s'installe par `rustup` (préinstallé sur le runner), sans action tierce.
- Le test x11 `unsupported_commands_never_touch_the_server` échoue désormais au lieu de passer en silence.

**Justification** : le test porte sur le binaire livré ; l'inspection par chaînes est plus robuste qu'`nm` (les symboles dépendent de l'éditeur de liens, les chaînes de route non) et le témoin positif évite une preuve vide ; l'épinglage par SHA ferme la porte au détournement d'un tag d'action.

## Conséquences

**Positives**

- la session MCP complète est vérifiée à chaque PR, hors session graphique de l'utilisateur ;
- la promesse « no-op en release » est prouvée sur un binaire réel ;
- la CI se reproduit en local par `just ci`.

**Coûts et risques**

- le test e2e ne couvre que la cible x11 (la seule exécutable sans session graphique réelle) ; CDP, SDK et KWin restent testés par leurs suites propres ;
- les SHA épinglés demandent une mise à jour périodique (Dependabot ou revue manuelle) ;
- `cargo-semver-checks` et `cargo-cyclonedx` sont installés via `taiki-e/install-action` ; ni l'un ni l'autre n'a pu être exécuté localement, leur première exécution réelle sera celle de la CI.

## Notes

Choix d'agent qui engage le mainteneur au-delà du lot (plage de displays, politique d'épinglage, forme de la preuve) : statut « Proposé » en attendant validation.
