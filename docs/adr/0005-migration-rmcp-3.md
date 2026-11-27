# ADR-0005 — Migrer rmcp 1.8 vers 3.12 (RUSTSEC-2026-0462)

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0008 (autres montées de version du même lot)

## Contexte et problème

`adapter-mcp-server` s'appuie sur `rmcp` (SDK Rust du Model Context Protocol) en 1.8. L'avis RUSTSEC-2026-0462 touche cette version, sans correctif dans la plage semver 1.x. Le serveur MCP est la surface exposée aux agents : laisser un avis ouvert dessus est inacceptable pour une 1.0.

## Facteurs de décision

- corriger l'avis plutôt que l'ignorer (surface exposée) ;
- ne pas casser les noms ni les schémas des outils MCP (surface stable de la 1.0) ;
- coût de migration borné.

## Options envisagées

1. **Ignorer l'avis dans `deny.toml`** — masque un défaut sur la surface exposée.
2. **Rester en 1.x et patcher localement** — fork à maintenir.
3. **Migrer vers rmcp 3.12**, version corrigée.

## Décision

Option 3 (commit `001da2a`). Adaptations : `Content` devient `ContentBlock`, `ServerInfo` devient `ServerConfig` ; les macros `#[tool]` restent compatibles ; les 37 tests de l'adapter passent.

**Justification** : l'avis est corrigé à la source, sans fork, et l'impact sur le code est limité à des renommages ; les noms et schémas des outils MCP sont inchangés.

## Conséquences

**Positives**

- plus d'avis ouvert sur le serveur MCP ;
- un graphe plus récent (rmcp 3.x tire son propre lot de dépendances).

**Coûts et risques**

- l'arbre de dépendances grossit (le lockfile gagne environ 250 lignes) ;
- à chaque montée majeure de rmcp, vérifier que les schémas d'outils publiés ne changent pas.

## Notes

Statut rétabli en « Proposé » : ce choix a été fait par l'agent de la release 1.0 et aucune trace de validation par le mainteneur n'existe dans le dépôt (décisions du propriétaire du 27/11/2026 : aucune ne le couvre explicitement). Il passera à « Accepté » à la validation du mainteneur.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
