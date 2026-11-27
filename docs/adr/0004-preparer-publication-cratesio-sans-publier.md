# ADR-0004 — Préparer la publication crates.io des bibliothèques sans publier

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0002 (seuls les crates MIT sont publiables), ADR-0003

## Contexte et problème

Les bibliothèques de DevEye (`deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`) sont faites pour être réutilisées : le SDK s'embarque dans des apps tierces, les ports servent à écrire un adapter. Une dépendance Cargo par `git` ou par chemin est fragile pour elles. Mais publier sur crates.io est irréversible (une version publiée ne se supprime pas, seulement se retire) et engage la surface semver.

## Facteurs de décision

- permettre à un tiers de dépendre des bibliothèques sans copie vendored ;
- ne rien figer avant que le propriétaire ne le décide ;
- métadonnées complètes et vérifiables par `cargo package` ;
- noms libres sur crates.io (vérifié le 27/11/2026).

## Options envisagées

1. **Ne rien préparer** — les dépendances restent par `git` ; fragile et peu découvrable.
2. **Publier en même temps que la 1.0** — irréversible, sans validation du propriétaire.
3. **Préparer sans publier** : manifestes complets et vérifiés, publication décidée plus tard.

## Décision

Option 3. Les quatre crates MIT deviennent publiables (pas de `publish = false`, avec description, readme, keywords, categories, documentation, `LICENSE-MIT` embarqué, dépendances internes en `path` + `version`) ; tous les autres gardent `publish = false` (« interne, non publié »). Aucune publication, aucun tag n'est exécuté par la release : la publication effective reste une décision ultérieure du propriétaire (ADR à venir à ce moment). Le binaire se distribue par `cargo install --git` et par les archives de release GitHub.

**Justification** : tout est prêt le jour où le propriétaire le décide, sans engager de surface semver avant.

## Conséquences

**Positives**

- `cargo package` valide les manifestes et le contenu des archives dès maintenant ;
- la frontière publiable (MIT) coïncide avec la frontière de licence.

**Coûts et risques**

- `cargo publish --dry-run` des dépendants (`deveye-ports`, `deveye-sdk`) échoue tant que `deveye-core` n'est pas publié : attendu ; l'ordre de publication est core, puis ports et sdk, sdk-client étant indépendant ;
- les adapters ne sont pas utilisables comme dépendances crates.io : ils sont internes, sans promesse sur leur API Rust.
