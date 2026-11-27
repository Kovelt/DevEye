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

## Validation avant la 1.0

Les ADR rédigés pendant la préparation de la 1.0 avec le statut « Proposé » (ADR-0005 à 0010, 0012, 0020 à 0035, 0039, 0048 à 0060) ont été acceptés en bloc par le mainteneur le 2026-11-27, avant le tag `v1.0.0`. Ils figent la surface 1.0 (`#[non_exhaustive]`, types `tokio` exposés par le SDK, annotations et version du protocole MCP, `doctor`, `PortError::Script`, CI de release). Le choix d'exposer les types `tokio` (version majeure de `tokio` = surface du SDK) est consigné dans ADR-0012 ; `docs/stability.md` en est le relais. Revenir sur l'une de ces décisions passe par un nouvel ADR.

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
| [0012](0012-application-de-non-exhaustive-et-choix-limites.md) | Application de `#[non_exhaustive]` et choix limites avant le gel | Accepté |
| [0013](0013-neutralite-apps-clientes-et-archivage-historique.md) | Neutralité vis-à-vis des apps clientes et archivage des documents historiques | Accepté |
| [0014](0014-filtre-de-cible-cdp-permissif-par-defaut.md) | Filtre de cible CDP permissif par défaut | Accepté |
| [0015](0015-retrait-du-script-de-synchronisation-des-copies.md) | Retrait du script de synchronisation des copies vendored | Remplacé par ADR-0040 |
| [0016](0016-masquage-des-secrets-par-cles-dans-le-sdk.md) | Masquage des secrets par clés dans le SDK | Accepté |
| [0017](0017-protocole-1-0-version-info-et-limites.md) | Protocole 1.0 : version, compatibilité côté client, `/devtools/info` et limites de taille | Accepté |
| [0018](0018-portage-selectif-de-l-ancienne-lignee-github.md) | Portage sélectif de l'ancienne lignée GitHub | Accepté |
| [0019](0019-resize-variante-d-interaction-command.md) | Redimensionnement du viewport : variante `InteractionCommand::Resize` | Accepté |
| [0020](0020-logs-wait-long-poll-des-logs.md) | `logs_wait` : long-poll des logs et niveau invalide en `invalid_params` | Accepté |
| [0021](0021-validation-des-coordonnees-des-gestes.md) | Validation des coordonnées des gestes avant dispatch | Accepté |
| [0022](0022-capture-cdp-a-l-echelle-css.md) | Capture CDP à l'échelle CSS | Accepté |
| [0023](0023-exceptions-js-en-resultat-d-outil-en-erreur.md) | Exceptions JS en résultat d'outil en erreur (`PortError::Script`) | Accepté |
| [0024](0024-serialisation-sans-panic-et-arguments-stringifies.md) | Sérialisation sans panic et arguments `invoke_command` stringifiés | Accepté |
| [0025](0025-captured-at-et-etat-sdk-vide-ou-indisponible.md) | Contrat de sortie : `captured_at` et distinction état SDK vide ou indisponible | Accepté |
| [0026](0026-type-text-sans-champ-editable-au-focus.md) | `type_text` sans champ éditable au focus | Accepté |
| [0027](0027-annotations-des-outils-mcp-et-classification-des-risques.md) | Annotations des outils MCP et classification des risques | Accepté |
| [0028](0028-version-du-protocole-mcp-epinglee-stdio.md) | Version du protocole MCP épinglée à 2025-11-25, transport stdio seul | Accepté |
| [0029](0029-outil-observe-et-maitrise-du-cout-des-captures.md) | Outil `observe` et maîtrise du coût des captures | Accepté |
| [0030](0030-assert-visual-masques-image-de-difference-et-mise-a-jour.md) | `assert_visual` : masques, image de différence et mise à jour de la référence | Accepté |
| [0031](0031-validation-des-flags-selon-la-cible.md) | Validation des flags selon la cible | Accepté |
| [0032](0032-sous-commande-doctor-en-lecture-seule.md) | Sous-commande `doctor` en lecture seule | Accepté |
| [0033](0033-version-affichee-et-provenance-du-commit-sans-dependance.md) | Version affichée et provenance du commit (sans dépendance) | Accepté |
| [0034](0034-capacites-completes-de-target-info-et-repli-observable.md) | Capacités complètes de `target_info`, repli observable et choix unique du backend OS | Accepté |
| [0035](0035-suite-de-conformite-boite-noire-du-protocole.md) | Suite de conformité boîte noire du protocole | Accepté |
| [0036](0036-statut-du-shim-dotnet-implementation-tierce.md) | Statut du shim .NET : implémentation tierce validée par la suite de conformité | Accepté |
| [0037](0037-linux-seul-supporte-en-1-0.md) | Linux seul supporté en 1.0 | Accepté |
| [0038](0038-distribution-archives-github-et-cargo-install.md) | Distribution du binaire : archives de release GitHub et cargo install | Accepté |
| [0039](0039-tests-de-bout-en-bout-et-preuves-dev-only-en-ci.md) | Tests de bout en bout et preuves dev-only en CI | Accepté |
| [0040](0040-fin-du-vendoring-script-de-synchronisation-de-transition.md) | Fin du vendoring : dépendance versionnée, script de synchronisation de transition | Accepté |
| [0041](0041-architecture-hexagonale-ports-et-adapters.md) | Architecture hexagonale : ports et adapters | Accepté |
| [0042](0042-posture-dev-only-du-sdk.md) | Posture dev-only du SDK : debug seulement, loopback, garde Host, CORS | Accepté |
| [0043](0043-pont-natif-via-le-sdk-sans-cdp.md) | Pont natif via le SDK quand CDP est indisponible | Accepté |
| [0044](0044-cible-x11-isolee-par-construction.md) | Cible x11 isolée par construction : display privé obligatoire | Accepté |
| [0045](0045-yeux-et-mains-wayland-via-kwin.md) | Yeux et mains Wayland via KWin : ScreenShot2 et EIS | Accepté |
| [0046](0046-repli-appel-par-appel-du-mode-auto.md) | Repli appel par appel du mode auto | Accepté |
| [0047](0047-adopter-le-contributor-covenant-2-1-comme-code-de-conduite.md) | Adopter le Contributor Covenant 2.1 comme code de conduite | Accepté |
| [0048](0048-documentation-verifiee-par-des-tests.md) | Documentation vérifiée par des tests | Accepté |
| [0049](0049-confinement-des-baselines-d-assert-visual.md) | Confinement des baselines d'`assert_visual` | Accepté |
| [0050](0050-clipboard-get-monde-ouvert-et-limites-de-la-classification.md) | `clipboard_get` en monde ouvert et limites de la classification des outils | Accepté |
| [0051](0051-semantique-de-la-suite-de-conformite-skip-n-a-et-host-absent.md) | Sémantique de la suite de conformité : `SKIP`, `N/A`, `Host` absent | Accepté |
| [0052](0052-erreurs-de-liaison-et-de-protocole-visibles-et-cles-secretes-cumulatives.md) | Erreurs de liaison et de protocole visibles, clés secrètes cumulatives | Accepté |
| [0053](0053-porte-de-qualite-de-la-release-et-controles-de-packaging.md) | Porte de qualité de la release et contrôles de packaging | Accepté |
| [0054](0054-limites-documentees-issues-des-revues-de-la-1-0.md) | Limites documentées issues des revues de la 1.0 | Accepté |
| [0055](0055-refus-des-requetes-inter-origine-sur-le-drain-des-logs.md) | Refus des requêtes inter-origine sur le drain des logs | Accepté |
| [0056](0056-alignement-des-versions-des-crates-sur-1-0-0.md) | Alignement des versions des crates sur 1.0.0 | Accepté |
| [0057](0057-doctor-display-optionnel-et-sondes-limitees-a-la-cible.md) | `doctor` : `--display` vérifié après le parsing, sondes limitées à la cible et parallèles | Accepté |
| [0058](0058-critere-de-support-verification-par-la-ci-ou-manuelle.md) | Critère de support : vérification par la CI ou par le mainteneur | Accepté |
| [0059](0059-politique-de-securite-et-conventions-de-contribution.md) | Politique de sécurité et conventions de contribution | Accepté |
| [0060](0060-dependances-d-adapters-cache-de-ci-et-verrous-empoisonnes.md) | Dépendances d'adapters, cache de CI et verrous empoisonnés du SDK | Accepté |
| [0061](0061-version-1-0-0-perimetre-de-la-sortie.md) | Version 1.0.0 : périmètre de la sortie | Accepté |
