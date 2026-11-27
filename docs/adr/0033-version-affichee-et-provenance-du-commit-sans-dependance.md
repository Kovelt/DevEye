# ADR-0033 — Version affichée et provenance du commit (sans dépendance)

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (versionnage), ADR-0017 (protocole 1.0), ADR-0028 (version du protocole MCP), ADR-0032 (`doctor`)

## Contexte et problème

`deveye-server --version` échouait (clap sans `version`), et ni `target_info` ni le `serverInfo` MCP ne disaient quelle version tournait : le serveur annonçait à l'handshake l'identité de la bibliothèque `rmcp`. Or un des incidents vécus est le binaire périmé : sans version ni commit, impossible de savoir que le `deveye-server` lancé n'est pas celui qu'on vient de compiler.

## Facteurs de décision

- version du crate, commit de build et versions des protocoles (SDK et MCP) visibles ;
- aucune nouvelle dépendance (pas de crate de type `vergen`, `built`, `shadow-rs`) ;
- repli propre quand git est absent (archive source, crate publié, CI sans `.git`) ;
- une source unique pour `--version`, `target_info`, `serverInfo` et `doctor`.

## Options envisagées

1. **Seulement `CARGO_PKG_VERSION`** : pas de commit, donc n'aide pas pour le binaire périmé.
2. **Un crate de métadonnées de build** : couvre tout, mais ajoute une dépendance de build.
3. **Un `build.rs` sans dépendance** : lit une variable d'environnement optionnelle, sinon appelle `git rev-parse` s'il existe, sinon chaîne vide.

## Décision

Option 3.

- `deveye-server/build.rs` pose `DEVEYE_BUILD_COMMIT` : valeur de `DEVEYE_GIT_COMMIT` si elle est définie (builds de release et CI, qui contrôlent ainsi la valeur), sinon `git rev-parse --short=9 HEAD` si `git` répond, sinon vide. Il relance la compilation quand `HEAD` ou la branche courante change (sans quoi le commit affiché serait celui de la première compilation).
- `--version` affiche trois lignes (`0.2.0 (commit abc123def)`, protocole SDK, protocole MCP) ; `-V` la première seulement. Hors dépôt : `commit inconnu`.
- La même identité (`ServerVersion` : version, commit, `sdk_protocol`, `mcp_protocol`) est exposée dans le champ `server` de `target_info` et, pour la version, dans `serverInfo` de l'handshake MCP (`name: "deveye-server"`).
- La version du protocole SDK vient de `deveye_sdk_client::SUPPORTED_PROTOCOL_VERSION`, celle du protocole MCP de `adapter_mcp_server::MCP_PROTOCOL_VERSION` : pas de littéral recopié.

**Justification** : seule cette option fournit le commit sans dépendance ni cérémonie, avec un repli propre. La variable d'environnement permet des builds reproductibles hors dépôt (archive de release).

## Conséquences

**Positives**

- l'agent et l'humain savent quel binaire tourne et à quel protocole il parle ;
- le `serverInfo` MCP porte l'identité de DevEye et non celle de `rmcp`.

**Coûts et risques**

- un commit « sale » (modifications non commitées) n'est pas signalé : le commit court désigne le dernier commit, pas l'arbre ;
- `git` est exécuté pendant la compilation (lecture seule) ; son absence est sans conséquence ;
- l'ajout de `deveye-sdk-client` aux dépendances de `deveye-server` (ADR-0032) est ce qui permet de lire la constante du protocole.

## Notes

Application : `deveye-server/build.rs`, `deveye-server/src/version.rs`, `deveye-adapters/adapter-mcp-server/src/lib.rs` (`ServerVersion`, `get_info`).

Précisions (revue de la 1.0) : le commit est lu uniquement si le dépôt git trouvé est celui de DevEye (`deveye-server/Cargo.toml` à sa racine), pour qu'un crate vendorisé dans un autre dépôt n'affiche pas le commit de son hôte ; `build.rs` relit `HEAD` dans le répertoire git du worktree (`--absolute-git-dir`) et la ref ou `packed-refs` dans le répertoire commun. Un suffixe `-dirty` n'est volontairement pas ajouté : un script de build ne peut pas suivre les modifications de l'arbre de travail et l'indicateur serait périmé.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
