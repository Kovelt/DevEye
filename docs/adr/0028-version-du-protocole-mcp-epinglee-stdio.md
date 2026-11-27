# ADR-0028 — Version du protocole MCP épinglée à 2025-11-25, transport stdio seul

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0005 (migration vers rmcp 3.12), ADR-0011 (stabilité), ADR-0027 (annotations d'outils)

## Contexte et problème

Sans configuration, `rmcp` 3.12 annonce sa révision la plus récente (`2026-07-28`) et accepte toutes celles qu'il connaît. Cette révision supprime la poignée de main `initialize` au profit de métadonnées par requête : DevEye ne l'a jamais testée contre un client réel. Annoncer une version qu'on ne teste pas revient à promettre un comportement qu'on ne vérifie pas, alors que la 1.0 fige la surface MCP (noms et schémas, ADR-0011).

## Facteurs de décision

- n'annoncer que ce qui est testé ;
- rester joignable des clients courants, qui négocient à la baisse ;
- ne pas dépendre implicitement des montées de version du SDK (une mise à jour de `rmcp` ne doit pas changer le contrat sans décision) ;
- les annotations d'outils existent depuis la révision 2025-03-26.

## Options envisagées

1. **Laisser le défaut du SDK** : suit automatiquement `rmcp`, mais le contrat dérive à chaque montée de version.
2. **Épingler `2025-11-25`** (dernière révision avec `initialize`) comme valeur annoncée et plafond de négociation ; versions antérieures acceptées.
3. **Épingler une version plus ancienne** (2025-06-18) : plus conservateur, mais renonce à des révisions disponibles sans gain mesuré.

## Décision

Option 2 : `get_info()` annonce `2025-11-25`, `supported_protocol_versions()` renvoie les révisions connues jusqu'à celle-ci (de `2024-11-05` à `2025-11-25`), et la constante publique `MCP_PROTOCOL_VERSION` la documente. Un client qui demande `2026-07-28` est ramené à `2025-11-25`. Transport : **stdio uniquement** (`serve_stdio`), pas de HTTP.

**Justification** : c'est la plus récente révision pour laquelle le serveur fonctionne de bout en bout aujourd'hui, et le test `protocol_version_is_pinned_and_negotiated_down_from_newer_clients` en fige le comportement. Relever le plafond sera un ADR (test préalable avec un client de la nouvelle révision), pas un effet de bord d'une mise à jour de dépendance.

## Conséquences

**Positives**

- contrat de protocole explicite, vérifié par test ;
- les clients actuels (qui demandent 2025-06-18 ou 2025-11-25) négocient exactement ce qu'ils demandent.

**Coûts et risques**

- un client qui n'implémenterait que `2026-07-28` ne pourra pas se connecter tant que le plafond n'est pas relevé ;
- une montée de `rmcp` qui retirerait `known_up_to` ou une révision connue obligerait à revoir ce choix (le test le signalera).

## Notes

Application : `deveye-adapters/adapter-mcp-server/src/lib.rs` (`MCP_PROTOCOL_VERSION`, `ServerHandler::get_info`, `supported_protocol_versions`). Documentation : `docs/usage.md`.
