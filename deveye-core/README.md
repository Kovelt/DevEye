# deveye-core

Domaine pur de DevEye : types de commande d'interaction, de snapshot, de prédicat d'attente, de log, d'erreur console et de fenêtre. Rust pur (`chrono`, `serde`, `thiserror`), sans dépendance technique.

Depuis la 1.1, le module `ui` définit le contrat `deveye.ui/1` de la route `ui.widgets` (arbre de widgets d'une app native). La feature optionnelle `schema` (désactivée par défaut, ajoute `schemars`) génère son JSON Schema, publié dans `docs/schemas/ui-widgets-1.json`.

Fait partie de [DevEye](https://github.com/Morgan-Voltz/DevEye), outillage dev-only qui permet à un agent (client MCP) de voir et de piloter une app graphique en cours d'exécution.

## Licence

MIT, voir [LICENSE-MIT](LICENSE-MIT). Les bibliothèques de DevEye (`deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`) sont sous MIT pour pouvoir être embarquées sans contrainte ; l'outil `deveye-server` est sous AGPL-3.0-or-later.
