# ADR-0002 — Licences : AGPL-3.0-or-later pour l'outil, MIT pour les bibliothèques embarquées

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0004 (publication crates.io des bibliothèques MIT)

## Contexte et problème

Jusqu'à la 0.2, DevEye était propriétaire (`LicenseRef-Proprietary`, sans fichier de licence : constat I6 de l'audit du 04/07/2026). La 1.0 est publiée en open source sur GitHub. Il faut choisir la ou les licences, sachant que le dépôt contient deux types de code très différents :

- l'**outil** (`deveye-server`, les adapters) : un binaire que l'on lance à côté de l'app cible ;
- les **bibliothèques** (`deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`) : le SDK s'embarque dans l'app cible (en `cfg(debug_assertions)`), le reste est réutilisé par des tiers qui écrivent leur propre adapter ou client.

## Facteurs de décision

- intégrer le SDK dans une app ne doit imposer aucune contrainte de licence à cette app ;
- protéger l'outil contre une reprise fermée, y compris proposée en service réseau ;
- clarté : un contributeur doit savoir sous quelle licence va sa contribution ;
- compatibilité avec le graphe de dépendances tierces (cargo-deny) ;
- convention connue (« le classique ») plutôt qu'un montage original.

## Options envisagées

1. **`MIT OR Apache-2.0` partout** — d'abord retenu (décision D01 initiale), puis remplacé le 27/11/2026 par le propriétaire : permissif, mais n'empêche pas une reprise fermée de l'outil.
2. **AGPL-3.0-or-later seule** — protège l'outil, mais rendrait l'intégration du SDK contraignante pour toute app cible (copyleft réseau sur l'app).
3. **MIT seule** — maximalement ouvert, aucune protection de l'outil.
4. **MPL-2.0** — copyleft au niveau du fichier, intermédiaire ; moins connu, et ne couvre pas l'usage en service réseau.
5. **Modèle dual par rôle : AGPL-3.0-or-later pour l'outil, MIT pour les bibliothèques embarquées.**

## Décision

Option 5.

- **`MIT`** (SPDX `MIT`) : `deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`.
- **`AGPL-3.0-or-later`** : `deveye-server`, tous les adapters, `deveye-conformance`, les scripts ; c'est la licence par défaut du dépôt (`[workspace.package]`).
- **Invariant** : un crate MIT ne dépend jamais d'un crate AGPL (aujourd'hui `sdk → core`, `ports → core`, `sdk-client → ports`). Un nouveau crate déclare explicitement sa licence ; AGPL par défaut, MIT seulement s'il est destiné à être embarqué ou réutilisé, avec un ADR.
- **Fichiers** : `LICENSE` (texte intégral de la GNU AGPL v3) et `LICENSE-MIT` (« Copyright (c) 2026 NuvoraCode / Morgan Voltz ») à la racine ; une copie de `LICENSE-MIT` dans chacun des quatre crates MIT pour que `cargo package` l'embarque ; section « Licence » du README.
- **cargo-deny** : section `[licenses]` avec une liste blanche des licences tierces réellement présentes dans le graphe, plus une exception `AGPL-3.0-or-later` limitée à nos propres crates (clarification par crate) : aucune dépendance tierce AGPL n'est acceptée implicitement.
- **« or-later »** et non « only » : confirmé par le propriétaire le 27/11/2026.

**Justification** : la réciprocité réseau de l'AGPL protège l'outil, que personne n'a besoin d'embarquer ; les bibliothèques, elles, sont embarquées ou réutilisées, et la MIT n'impose rien aux apps qui les intègrent. C'est le partage classique entre outil et bibliothèques.

## Conséquences

**Positives**

- intégrer le SDK dans une app, propriétaire ou non, ne crée aucune obligation ;
- une reprise de l'outil, même en service hébergé, doit publier ses modifications ;
- l'invariant de frontière est vérifiable mécaniquement (`cargo tree`, revue du `Cargo.toml`).

**Coûts et risques**

- la frontière des crates doit être préservée : ajouter à un crate MIT une dépendance vers un crate AGPL est interdit (la logique partagée descend vers `deveye-core`/`deveye-ports`, jamais l'inverse) ;
- les contributions sont reçues sous la licence du crate touché ; pas de CLA, donc pas de relicenciement unilatéral futur sans l'accord des contributeurs ;
- l'adoption du serveur peut être freinée en entreprise par l'AGPL ; le SDK et les bibliothèques, eux, ne le sont pas ;
- deux fichiers de licence et un contrôle supplémentaire en revue.

## Notes

L'AGPL s'applique à l'outil tel que distribué ; elle ne contamine pas l'app pilotée, qui dialogue avec lui par un protocole (HTTP du SDK, CDP, X11) et non par liaison de code.
