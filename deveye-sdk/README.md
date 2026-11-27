# deveye-sdk

SDK à intégrer dans l'app cible, **uniquement en `cfg(debug_assertions)`** : démarre un micro-serveur HTTP lié à `127.0.0.1` qui expose l'état, les logs et les commandes de l'app à `deveye-server`. Sans effet en build release.

Fait partie de [DevEye](https://github.com/Morgan-Voltz/DevEye), outillage dev-only qui permet à un agent (client MCP) de voir et de piloter une app graphique en cours d'exécution.

## Licence

MIT, voir [LICENSE-MIT](LICENSE-MIT). Les bibliothèques de DevEye (`deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`) sont sous MIT pour pouvoir être embarquées sans contrainte ; l'outil `deveye-server` est sous AGPL-3.0-or-later.
