# ADR-0038 — Distribution du binaire : archives de release GitHub et cargo install

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0002 (licences : les archives embarquent AGPL et MIT), ADR-0003 (GitHub Actions), ADR-0004 (crates.io préparé, jamais publié), ADR-0037 (Linux seul)

## Contexte et problème

Aucun artefact versionné n'existait : les apps clientes pointaient des binaires inexistants ou lançaient `cargo run` à chaque démarrage. La 1.0 doit fournir un binaire installable sans compiler le dépôt, et un chemin simple pour qui préfère compiler.

## Facteurs de décision

- un binaire reproductible, accompagné de sommes de contrôle vérifiables ;
- les textes de licence et le CHANGELOG livrés avec le binaire (AGPL oblige à fournir la licence) ;
- publication relue par un humain avant d'être visible ;
- permissions minimales de la CI ;
- crates.io n'est pas utilisé pour `deveye-server` (`publish = false`, ADR-0004).

## Options envisagées

1. **`cargo install --git --tag`** seul : lent, exige la chaîne de compilation chez l'utilisateur.
2. **RPM via mock ou Copr** : bon pour Fedora, mais un canal de plus à maintenir dès la 1.0.
3. **Archives de GitHub Release** (`x86_64-unknown-linux-gnu`) construites par un workflow sur tag, plus recette `just install` et `cargo install --path`.

## Décision

Option 3, conformément à la décision du propriétaire. Le workflow `.github/workflows/release.yml`, déclenché par un tag `vX.Y.Z` (ou une pré-version `vX.Y.Z-…`, voir ADR-0053) :

- vérifie que le tag égale la version de `deveye-server` et qu'une section du `CHANGELOG.md` existe pour elle ;
- construit `deveye-server` en release avec `--locked` ;
- produit `deveye-server-<tag>-x86_64-unknown-linux-gnu.tar.gz` contenant le binaire, `LICENSE` (AGPL-3.0), `LICENSE-MIT`, `README.md` et `CHANGELOG.md`, plus un fichier `SHA256SUMS` ;
- crée une GitHub Release **en brouillon**, avec la section du CHANGELOG pour notes ; le mainteneur la relit et la publie.

Le job de construction n'a que `contents: read` ; la permission `contents: write` est réservée à un second job qui ne fait que créer le brouillon à partir des fichiers produits. Côté développeur : `just install` (build release copié dans `~/.local/bin`) ou `cargo install --path deveye-server --locked`. RPM et Copr sont renvoyés en 1.x.

**Justification** : peu de pièces mobiles, un artefact vérifiable, aucun secret, et la publication reste un geste humain.

## Conséquences

**Positives**

- les apps clientes peuvent pointer un binaire versionné ;
- un compromis de la construction ne donne pas de droit d'écriture (séparation des jobs) ;
- les licences voyagent avec le binaire.

**Coûts et risques**

- une seule cible (Linux x86_64, ADR-0037) ; pas d'archive pour d'autres architectures ;
- le format attendu du CHANGELOG (`## [x.y.z]` ou `## x.y.z`) est une convention que le lot de rédaction doit respecter ; sinon le workflow échoue volontairement ;
- le workflow ne peut pas être exercé avant le premier tag : il n'a été validé que syntaxiquement et ses commandes de shell à la main.
