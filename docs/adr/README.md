# Décisions d'architecture (ADR)

Toute décision qui engage DevEye passe par un ADR (*Architecture Decision Record*) : architecture, dépendance ajoutée ou retirée, licence, protocole, surface stable, plateforme supportée, outillage, exception de sécurité ou de lint, choix entre plusieurs options pendant un chantier. Un ADR court vaut mieux qu'une décision que personne ne sait plus justifier.

## Processus

1. **Écrire avant ou avec le changement** : l'ADR est créé dans le commit qui applique la décision, ou juste avant. Pas de décision structurante sans ADR.
2. **Numéroter en continu** : `docs/adr/NNNN-slug-kebab.md`, quatre chiffres, sans trou ni réutilisation.
3. **Statut** :
   - `Proposé` : choix d'un contributeur (ou d'un agent) qui engage le mainteneur au-delà du chantier en cours ; en attente de validation ;
   - `Accepté` : validé par le mainteneur, en vigueur ;
   - `Remplacé par ADR-XXXX` : une décision plus récente la supplante ;
   - `Déprécié` : plus d'actualité, sans remplaçante.
4. **Ne jamais réécrire l'histoire** : un ADR accepté ne change pas de fond. Pour revenir sur une décision, créer un nouvel ADR, puis passer l'ancien en « Remplacé par ADR-XXXX » (seul son statut change). Les corrections de forme (coquille, lien) sont libres.
5. **Mettre à jour l'index** ci-dessous dans le même commit.
6. **Commits** : `docs(adr): ...` pour un ADR seul ; sinon l'ADR voyage avec le changement qu'il justifie.

## Gabarit

```markdown
# ADR-NNNN — Titre court à l'indicatif

- **Statut** : Proposé | Accepté | Remplacé par ADR-XXXX | Déprécié
- **Date** : AAAA-MM-JJ
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-XXXX (nature du lien), ou « aucun »

## Contexte et problème

Pourquoi faut-il décider ? Quel est l'état des lieux et la contrainte ?

## Facteurs de décision

- critère 1
- critère 2

## Options envisagées

1. **Option A** — description courte.
2. **Option B** — description courte.

## Décision

Option retenue, en une phrase.

**Justification** : pourquoi elle l'emporte sur les autres, au regard des facteurs.

## Conséquences

**Positives**

- ...

**Coûts et risques**

- ...

## Notes

(Facultatif : références, preuves, chantier de suivi.)
```

## Index

| ADR | Titre | Statut |
|---|---|---|
| [0001](0001-consigner-chaque-decision-dans-un-adr.md) | Consigner chaque décision dans un ADR | Accepté |
| [0002](0002-licences-agpl-outil-mit-bibliotheques.md) | Licences : AGPL-3.0-or-later pour l'outil, MIT pour les bibliothèques embarquées | Accepté |
| [0003](0003-hebergement-github-ci-github-actions.md) | Hébergement sur GitHub, intégration continue GitHub Actions, GitLab archivé | Accepté |
| [0004](0004-preparer-publication-cratesio-sans-publier.md) | Préparer la publication crates.io des bibliothèques sans publier | Accepté |
| [0005](0005-migration-rmcp-3.md) | Migrer rmcp 1.8 vers 3.12 (RUSTSEC-2026-0462) | Accepté |
| [0006](0006-exception-rustsec-chrono.md) | Exception d'audit pour chrono (RUSTSEC-2026-0543) | Accepté |
| [0007](0007-exception-rustsec-quick-xml.md) | Exception d'audit pour quick-xml en build-dependency (RUSTSEC-2026-0194 et 0195) | Accepté |
| [0008](0008-montees-de-version-avis-rustsec.md) | Montées de version des dépendances touchées par les avis RUSTSEC | Accepté |
| [0009](0009-lints-de-workspace-et-exception-kwin-eis.md) | Lints de workspace et exception unsafe de l'adapter KWin EIS | Accepté |
| [0010](0010-toolchain-stable-et-msrv-1-88.md) | Toolchain stable épinglée par fichier et MSRV 1.88 | Accepté |
| [0011](0011-politique-de-stabilite-et-de-versionnage-1-0.md) | Politique de stabilité et de versionnage de la 1.0 | Accepté |
| [0012](0012-application-de-non-exhaustive-et-choix-limites.md) | Application de `#[non_exhaustive]` et choix limites avant le gel | Proposé |
| [0013](0013-neutralite-apps-clientes-et-archivage-historique.md) | Neutralité vis-à-vis des apps clientes et archivage des documents historiques | Accepté |
| [0014](0014-filtre-de-cible-cdp-permissif-par-defaut.md) | Filtre de cible CDP permissif par défaut | Accepté |
| [0015](0015-retrait-du-script-de-synchronisation-des-copies.md) | Retrait du script de synchronisation des copies vendored | Accepté |
| [0016](0016-masquage-des-secrets-par-cles-dans-le-sdk.md) | Masquage des secrets par clés dans le SDK | Accepté |
| [0017](0017-protocole-1-0-version-info-et-limites.md) | Protocole 1.0 : version, compatibilité côté client, `/devtools/info` et limites de taille | Accepté |
| [0018](0018-portage-selectif-de-l-ancienne-lignee-github.md) | Portage sélectif de l'ancienne lignée GitHub | Accepté |
| [0019](0019-resize-variante-d-interaction-command.md) | Redimensionnement du viewport : variante `InteractionCommand::Resize` | Accepté |
| [0020](0020-logs-wait-long-poll-des-logs.md) | `logs_wait` : long-poll des logs et niveau invalide en `invalid_params` | Proposé |
| [0021](0021-validation-des-coordonnees-des-gestes.md) | Validation des coordonnées des gestes avant dispatch | Proposé |
| [0022](0022-capture-cdp-a-l-echelle-css.md) | Capture CDP à l'échelle CSS | Proposé |
| [0023](0023-exceptions-js-en-resultat-d-outil-en-erreur.md) | Exceptions JS en résultat d'outil en erreur (`PortError::Script`) | Proposé |
| [0024](0024-serialisation-sans-panic-et-arguments-stringifies.md) | Sérialisation sans panic et arguments `invoke_command` stringifiés | Proposé |
| [0025](0025-captured-at-et-etat-sdk-vide-ou-indisponible.md) | Contrat de sortie : `captured_at` et distinction état SDK vide ou indisponible | Proposé |
| [0026](0026-type-text-sans-champ-editable-au-focus.md) | `type_text` sans champ éditable au focus | Proposé |
| [0027](0027-annotations-des-outils-mcp-et-classification-des-risques.md) | Annotations des outils MCP et classification des risques | Proposé |
| [0028](0028-version-du-protocole-mcp-epinglee-stdio.md) | Version du protocole MCP épinglée à 2025-11-25, transport stdio seul | Proposé |
| [0029](0029-outil-observe-et-maitrise-du-cout-des-captures.md) | Outil `observe` et maîtrise du coût des captures | Proposé |
| [0030](0030-assert-visual-masques-image-de-difference-et-mise-a-jour.md) | `assert_visual` : masques, image de différence et mise à jour de la référence | Proposé |
