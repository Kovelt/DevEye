# ADR-0018 — Portage sélectif de l'ancienne lignée GitHub

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0003 (hébergement GitHub), ADR-0016 (masquage des secrets, premier portage), ADR-0019 (resize), ADR-0020 à ADR-0026 (autres portages et contrat de sortie MCP)

## Contexte et problème

Avant la lignée canonique actuelle, DevEye a vécu sur GitHub dans une histoire disjointe : `origin/main` (2671b68, 33 commits) et la branche `origin/claude/sleepy-pascal-e6hzxr` (trois commits de plus : masquage des secrets, documentation, workflow CI). Les deux lignées ont divergé dès le premier commit : aucun merge ni cherry-pick n'est possible sans importer une histoire parallèle et des conflits sur chaque fichier. Avant d'archiver l'ancienne lignée, il faut savoir ce qu'elle contenait que la canonique n'a pas, et décider fonction par fonction de ce qui est porté, de ce qui ne l'est pas, et pourquoi. Sans cet inventaire, l'archivage perdrait des comportements utiles (redimensionnement du viewport, attente de logs, validation des coordonnées, capture à l'échelle CSS, exceptions JS en erreur d'outil).

## Facteurs de décision

- ne rien perdre de ce qui apporte un comportement utile à l'utilisateur ;
- ne pas importer d'histoire disjointe ni de dette (documents d'audit périmés, code mort) ;
- respecter l'architecture actuelle (ports, adapters, outils MCP découpés par famille) plutôt que recoller du code écrit pour l'ancienne structure ;
- chaque portage est une réimplémentation testée, un commit par fonction, avec son ADR quand il engage la surface stable (ADR-0011) ;
- les décisions du propriétaire priment : le redimensionnement du viewport (`resize`) est voulu (D05).

## Options envisagées

1. **Fusionner la lignée** (`git merge --allow-unrelated-histories`) : conserve les auteurs et l'histoire, mais produit des conflits sur la quasi-totalité des fichiers et réintroduit des documents archivés depuis.
2. **Cherry-pick commit par commit** : impossible sans base commune, chaque commit supposant l'ancienne arborescence.
3. **Porter à la main après lecture** (`git show <ref>:<chemin>`), en réimplémentant dans l'architecture actuelle, et consigner l'inventaire.
4. **Abandonner l'ancienne lignée** : plus simple, mais perd des fonctions utiles et le masquage des secrets.

## Décision

Option 3. Les refs `origin/main` et `origin/claude/sleepy-pascal-e6hzxr` sont lues, jamais fusionnées. L'inventaire ci-dessous recense chaque commit de la lignée et son sort dans la lignée canonique.

### Inventaire fonction par fonction

| Commit GitHub | Fonction | Sort dans la canonique |
|---|---|---|
| 9c2d679 | base : domaine, ports, adapters CDP/xcap/Dioxus/logs, serveur MCP | déjà présent (c'est la souche de la lignée canonique) |
| d78424c | workflow Git (GitHub Flow, Conventional Commits) | déjà présent (`CLAUDE.md`) |
| 4f52909 | audit de la surface MCP (document) | non porté : document daté, remplacé par l'archive historique (ADR-0013) |
| 3bf625a | `screenshot` en contenu image natif MCP | déjà présent |
| 51a213d | `cargo fmt --all` | sans objet |
| 0357978 | outils `press_key` et `navigate` | déjà présents |
| 12080cd, 1eed28b | retrait des `expect` de sérialisation (MCP, CDP) | partiellement présent ; le reste est porté par ADR-0024 |
| f2f2f3d | `click_selector` et `click_text` par vrai clic CDP | déjà présents |
| 0b90dd8 | validation des coordonnées `click`/`hover` contre le viewport | **porté** (ADR-0021) |
| b7d5475 | outil `logs_wait` (long-poll des logs, filtre de niveau) | **porté** (ADR-0020) |
| 267c19f | outil `scroll` | déjà présent |
| ef47967 | outil `resize` (core, CDP, MCP) | **porté** (ADR-0019, décision D05) |
| 53d7fb9 | port `CommandRouter` (`invoke_command`, `list_commands`) | déjà présent |
| c12a9c3 | audit transverse (document) | non porté : document daté (ADR-0013) |
| 4d5002e | suppression du code mort | déjà présent (nettoyage du domaine avant le gel) |
| f37d19d | buffer de logs synchrone du SDK | déjà présent (canal de drain unique) |
| 224a515 | profondeur du DOM CDP bornée à 20 | **non porté, volontairement** : la lignée canonique demande `depth: -1` avec `pierce: true` pour voir les iframes et les shadow roots ; le garde-fou de taille de l'outil `dom` (~100 Ko) borne déjà le coût côté agent |
| 7d608e2 | ordre des `set_state` garanti par écriture synchrone | déjà présent (`RwLock` synchrone) |
| c54fcb0 | balayage cosmétique (`doc_markdown`, `LazyLock`) | déjà présent (`LazyLock`, lints de workspace, ADR-0009) |
| b14e5f5 | timeouts sur les appels HTTP du SDK et le bootstrap CDP | déjà présent (`deveye-sdk-client::client()`, délais de réponse CDP) |
| d89d46c | capture CDP à l'échelle CSS (corrige le décalage HiDPI) | **porté** (ADR-0022) |
| 18bbd04 | `attach_to` sans panic ; tests de reconnexion et de mapping `CdpError` | **porté** : `expect` retiré (ADR-0024) ; tests de mapping portés ; les tests de reconnexion existent déjà (`client.rs`) |
| ac1ea6a | tests du mapping `ScreencapError` vers `PortError` | **porté** (commit de tests) |
| 87b1e00 | catégorisation des erreurs MCP ; exceptions JS renvoyées en résultat d'outil | **porté** : catégorisation déjà présente (`port_to_mcp`, E1) ; exceptions JS en `isError` portées (ADR-0023) |
| 1cb1ced | garde `Host` anti DNS-rebinding du SDK | déjà présent |
| 9c52c52 | `reqwest::Client` partagé entre adapters SDK | déjà présent sous une autre forme (`deveye-sdk-client::client()`) |
| 6b69cac | mise à jour de `Cargo.lock` | sans objet |
| 2eecad5, 74d62ea | fusions de PR | sans objet |
| 50ef667, 9d6801b | cible `--target {auto,cdp,sdk}` avec repli natif ; documentation du pont Linux | déjà présents |
| f2ddeab, 3ec2c1a | masquage des secrets du SDK ; documentation associée | déjà portés (ADR-0016) |
| f1907d7 | workflow GitHub Actions (format, clippy, tests) | traité par le chantier de CI (ADR-0003) ; non porté tel quel |

Seuls ces éléments n'étaient pas présents et sont donc portés dans cette release : `resize`, `logs_wait`, la validation des coordonnées, la capture à l'échelle CSS, les exceptions JS en `isError`, le retrait des derniers `expect` de sérialisation, et trois jeux de tests de mapping d'erreurs.

**Justification** : l'option 3 est la seule qui préserve les comportements utiles sans importer d'histoire ni de document périmé. Elle coûte une réécriture, mais celle-ci était de toute façon nécessaire : le serveur MCP a été découpé en familles d'outils et les ports ont évolué (`#[non_exhaustive]`, erreurs structurées) depuis la divergence.

## Conséquences

**Positives**

- l'ancienne lignée peut être archivée sans perte : tout ce qu'elle apportait est soit présent, soit porté, soit écarté pour une raison écrite ;
- chaque portage est testé et réimplémenté dans l'architecture actuelle ;
- l'inventaire sert de table de correspondance si un contributeur retrouve une référence à un ancien commit.

**Coûts et risques**

- les auteurs et les dates des commits d'origine ne figurent pas dans l'histoire canonique (ils restent lisibles dans l'archive de l'ancienne lignée) ;
- un comportement de l'ancienne lignée absent de cet inventaire serait perdu : l'inventaire est fondé sur la lecture des 33 commits et des trois commits de la branche de travail, pas sur une exécution des deux binaires.

## Notes

- Refs lues : `origin/main` (2671b68), `origin/claude/sleepy-pascal-e6hzxr` (f1907d7).
- La ref `canon/*` pointe vers le dépôt privé d'origine : on n'y pousse jamais.
