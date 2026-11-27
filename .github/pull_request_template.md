## Pourquoi

<!-- Le problème résolu ; renvoyez à l'issue et à l'ADR concernés. -->

## Liste de contrôle

- [ ] Les tests passent et les vérifications sont vertes : `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- [ ] Une décision engagée est consignée dans un ADR indexé (`docs/adr/README.md`), ou aucune décision n'est prise.
- [ ] `CHANGELOG.md` est complété dans la section [Non publié].
- [ ] La documentation (`docs/`, README) est à jour.
- [ ] La compatibilité semver est respectée (pas de rupture de la surface stable, voir `docs/stability.md`).
- [ ] La licence du crate touché est respectée (MIT : `deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client` ; AGPL-3.0-or-later ailleurs).
