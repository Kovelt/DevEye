# deveye-sdk

SDK à intégrer dans l'app cible, **uniquement en `cfg(debug_assertions)`** : démarre un micro-serveur HTTP lié à `127.0.0.1` qui expose l'état, les logs et les commandes de l'app à `deveye-server`. Sans effet en build release.

Fait partie de [DevEye](https://github.com/Kovelt/DevEye), outillage dev-only qui permet à un agent (client MCP) de voir et de piloter une app graphique en cours d'exécution.

## Secrets et limites

Les valeurs des clés sensibles (`password`, `token`, `api_key`, `authorization`, `cookie`…) sont masquées dans l'état et les champs de logs servis ; `Config::with_secret_keys` ajoute les clés propres à l'app. Les résultats d'`eval` et de commandes ne sont pas filtrés ; l'arbre `ui.widgets` publié par `deveye_sdk::ui::publish` n'a jamais la valeur d'un widget secret. Les corps de requête sont limités à 1 Mio. Le contrat complet (version de protocole `1.0`, route `/devtools/info`) est dans `docs/wire-protocol.md` du dépôt.

## Licence

MIT, voir [LICENSE-MIT](LICENSE-MIT). Les bibliothèques de DevEye (`deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`) sont sous MIT pour pouvoir être embarquées sans contrainte ; l'outil `deveye-server` est sous AGPL-3.0-or-later.
