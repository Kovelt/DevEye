# ADR-0008 — Montées de version des dépendances touchées par les avis RUSTSEC

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0005 (rmcp), ADR-0006 (chrono)

## Contexte et problème

L'audit de santé de la base de la release a relevé douze avis RUSTSEC, dont neuf vulnérabilités sur la pile TLS/HTTP et `rmcp`, plus un crate retiré du registre (`async-executor` yanked). `cargo deny check` échouait sur les avis.

## Facteurs de décision

- corriger plutôt qu'ignorer, tant qu'une version corrigée existe ;
- rester dans les plages semver déclarées quand c'est possible (pas de changement de surface) ;
- ne pas introduire de dépendance nouvelle.

## Options envisagées

1. **Ignorer les avis dans `deny.toml`** — masque des vulnérabilités réelles.
2. **Monter les dépendances touchées**, dans leur plage semver quand elle contient un correctif, par migration sinon.

## Décision

Option 2.

- Commit `a2951ed` : `h2`, `rustls`, `rustls-webpki`, `webpki-roots`, `url`, `rustix`, `tokio` et `async-executor` montent dans leurs plages semver (`Cargo.lock` seul) ; chrono reçoit une exception (ADR-0006) et rmcp une migration (ADR-0005).
- Commit `691f48a` : `deveye-sdk` migre vers `axum` 0.8 et `tower-http` 0.7 ; le code du SDK compile sans changement (pas de paramètres de route) et le test du wire protocol reste vert. Le seul doublon `tower-http` restant (0.6) vient de `reqwest` 0.12.

**Justification** : les vulnérabilités sont corrigées sans toucher aux surfaces stables (wire protocol inchangé, vérifié par le test `wire_protocol`).

## Conséquences

**Positives**

- les avis de vulnérabilité sont levés ; la pile TLS est à jour.

**Coûts et risques**

- les doublons de versions restent des avertissements de `cargo deny` (`multiple-versions = "warn"`) ;
- `axum` 0.8 et `tower-http` 0.7 sont des dépendances du SDK, embarqué dans les apps cibles : leur plage de versions fait partie de ce qu'un intégrateur voit.

## Notes

Statut rétabli en « Proposé » : ce choix a été fait par l'agent de la release 1.0 et aucune trace de validation par le mainteneur n'existe dans le dépôt (décisions du propriétaire du 27/11/2026 : aucune ne le couvre explicitement). Il passera à « Accepté » à la validation du mainteneur.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
