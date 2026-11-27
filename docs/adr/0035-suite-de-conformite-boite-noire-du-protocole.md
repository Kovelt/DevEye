# ADR-0035 — Suite de conformité boîte noire du protocole

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (le wire protocol fait partie de la surface stable), ADR-0017 (version, `/devtools/info`, limites), ADR-0016 (masquage des secrets), ADR-0002 (licences), ADR-0036 (statut du shim .NET)

## Contexte et problème

Le wire protocol HTTP du SDK est une surface stable de la 1.0 : des implémentations dans d'autres langages existent déjà (un shim .NET, hors de ce dépôt) et d'autres suivront. Le seul test de contrat était interne au SDK Rust (`deveye-sdk/tests/wire_protocol.rs`), qui connaît le code qu'il teste, et la checklist de la spécification était manuelle. Les écarts constatés sur le shim (en-tête de version absent sur les erreurs, garde Host incomplète, tri, commande inconnue) l'ont montré : sans vérification automatique, le contrat n'est pas opposable à une implémentation tierce, et la spécification peut diverger du code sans que personne ne le voie.

## Facteurs de décision

- vérifier n'importe quelle implémentation à partir de sa seule URL, sans accès à son code ni modification de son dépôt ;
- exécuter la même vérification contre le SDK Rust à chaque `cargo test --workspace`, de sorte que spécification et référence ne divergent pas en silence ;
- rapport lisible et code de sortie exploitable en intégration continue ;
- aucune nouvelle dépendance externe ; invariant de licence ADR-0002 (un crate MIT ne dépend jamais d'un crate AGPL) ;
- ne jamais déclarer « conforme » ce qui n'a pas été observé.

## Options envisagées

1. **Statu quo** : test in-process du SDK Rust et checklist manuelle. Ne couvre aucune autre implémentation.
2. **Sous-commande ou feature de `deveye-server`** : la suite embarquée dans le binaire de l'outil. Ajoute une surface CLI stable (ADR-0011) pour un besoin de développeur de SDK, et alourdit le binaire livré aux utilisateurs.
3. **Crate dédié `deveye-conformance`** : bibliothèque de vérifications boîte noire plus binaire `--url`, hors du binaire de l'outil.
4. **Suite écrite dans un langage tiers ou via un outil externe** (Schemathesis, Postman, scripts `curl`) : dépendance d'outillage, cas de garde Host (requête sans `Host`) et de CORS mal couverts.

## Décision

Option 3 : un crate `deveye-conformance` (membre du workspace, `publish = false`, licence `AGPL-3.0-or-later` : c'est de l'outillage, pas une bibliothèque qu'une app embarque).

- **Bibliothèque** (`run(&Options) -> Report`) et **binaire** `deveye-conformance --url <URL>` ; code de sortie `0` conforme, `1` écarts, `2` suite inexécutable (URL invalide, implémentation injoignable, distingué de « non conforme »). Options pour les cas non observables par défaut : `--invoke-route`, `--slow-expression` (504), `--abandon-expression` (500), `--strict` (un `SKIP` échoue).
- **Honnêteté des verdicts** : une vérification dont l'état de l'implémentation ne permet pas l'observation est `SKIP` avec la raison, jamais `OK`.
- **Dépendances** : uniquement des crates déjà présents dans le workspace (`reqwest`, `tokio`, `serde_json`, `clap`, `chrono`, `thiserror` ; `deveye-sdk` et `tracing` en dev-dependencies). Aucun validateur JSON Schema n'est embarqué : les schémas de `docs/schemas/1.0/` sont vérifiés pour leur validité, leur identité et leur référencement, et la forme des corps l'est par les vérifications.
- **Test d'intégration** : le SDK Rust démarre en processus sur un port éphémère ; la suite doit passer sans aucun échec, puis, avec pompe d'évaluation, couche de logs et expressions spéciales, sans aucune vérification ignorée. Un test négatif (serveur qui répond `200 {}` à tout) prouve que la suite détecte les écarts.
- **Exigences ajoutées à la spécification** par cette suite, toutes additives ou précisant l'existant : CORS deny-all vérifié (§2.6), codes `404`/`405`/`4xx` et en-tête de version sur toute réponse (§3.0), horodatages RFC 3339 UTC `Z`, `DEVEYE_PORT` borné à 1-65535 (le SDK rejette désormais `0`, qui demandait un port éphémère imprévisible), capacité `redaction` annoncée par `/devtools/info` quand le masquage est actif (valeur additive de `capabilities`, sans changement de version du protocole).

**Justification** : seule l'option 3 valide une implémentation tierce sans lui imposer ni outil externe ni modification, sans élargir la surface CLI stable de l'outil, et en gardant la référence et la spécification sous la même épreuve à chaque test du workspace.

## Conséquences

**Positives**

- une implémentation tierce se valide en une commande et un rapport lisible ; le contrat devient opposable ;
- une divergence entre la spécification et le SDK Rust fait échouer les tests du dépôt ;
- les identifiants de vérification (`HOST-02`, `ERR-03`…) servent de vocabulaire commun dans les rapports d'écart.

**Coûts et risques**

- 26 vérifications à maintenir à chaque évolution du protocole ; un ajout `1.x` exige de les compléter ;
- la phase avec expression lente prend 10 s (le délai du `504`) dans `cargo test` ;
- `redaction` est une valeur de capacité ajoutée à `1.0` avant la publication : un client qui ne la connaît pas l'ignore, par la règle d'extensibilité du §2.3 ;
- la suite ne couvre pas le comportement interne des handlers de l'app ni la sécurité compile-time du SDK.

## Notes

Application : `deveye-conformance/` (`src/checks.rs` pour les vérifications), `docs/wire-protocol.md` §7, recette `just conformance`. Le crate est en version `0.2.0` comme les autres en attendant l'alignement des versions de la 1.0.
