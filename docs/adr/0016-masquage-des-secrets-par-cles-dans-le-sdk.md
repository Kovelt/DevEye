# ADR-0016 — Masquage des secrets par clés dans le SDK

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (stabilité : `Config` et `set_state` font partie de la surface stable), ADR-0017 (protocole 1.0 et limites)

## Contexte et problème

Le SDK publie l'état de l'app (`set_state`) et ses logs (couche `tracing`) sur un port loopback lisible par tout processus de la machine, puis par un agent via MCP. Une app qui y place un mot de passe, un jeton ou un en-tête `Authorization` l'expose donc à l'agent et à ses journaux. La documentation promettait une protection (« l'inspecteur de signaux ne traverse pas les types annotés `#[deveye(secret)]`, à implémenter ») sans qu'aucun code ne la livre : une promesse de sécurité non tenue est pire qu'une limite assumée. La 1.0 fige le contrat ; il faut soit livrer un mécanisme, soit retirer la promesse.

## Facteurs de décision

- ne jamais laisser croire à une protection qui n'existe pas ;
- sûr par défaut : les secrets courants sont masqués sans configuration ;
- aucun coût dans le chemin chaud (thread UI, émission d'un événement `tracing`) ;
- le SDK ne connaît pas les types de l'app : un mécanisme par type suppose de lui imposer une macro et une dépendance ;
- périmètre exact et documenté, y compris ce qui n'est pas protégé.

## Options envisagées

1. **Attribut dérivé `#[deveye(secret)]`** : précis (par champ), mais exige une macro procédurale (nouveau crate, nouvelle dépendance `syn`/`quote`), ne couvre pas les `serde_json::Value` libres ni les champs de logs, et impose à l'app de typer son état.
2. **Masquage par noms de clés, extensible par configuration** : le SDK parcourt la valeur JSON servie et remplace la valeur de toute clé jugée sensible (liste par défaut, plus `Config::secret_keys`). Une approximation (on juge le nom, pas la donnée) mais sans macro ni dépendance, et applicable à l'état comme aux champs de logs.
3. **Aucun filtre, limite documentée** : le plus simple, mais laisse l'intégrateur seul face à une fuite facile (`set_state("session", &user)`).

## Décision

Option 2, portée de la lignée GitHub (`redact.rs`) et adaptée au SDK actuel ; la promesse `#[deveye(secret)]` est retirée de la documentation.

- Clés sensibles par défaut, comparées sans casse ni séparateurs (`api_key` = `apiKey` = `API-KEY`) : termes contenus (`password`, `passwd`, `secret`, `apikey`, `privatekey`, `credential`, `authorization`), suffixes (`token`, `cookie`, `sessionid`), clés exactes courtes (`pwd`, `auth`, `jwt`, `bearer`, `otp`). `token_count`, `max_tokens` ou `cookie_banner` ne sont pas masquées.
- Extension : `Config::secret_keys` / `Config::with_secret_keys`, correspondance exacte sur la clé entière après normalisation.
- La valeur d'une clé sensible est remplacée en bloc (sous-arbre compris) par `deveye_sdk::REDACTED` (`<secret>`), à toute profondeur.
- Le masquage s'applique **au moment de servir** : `GET /devtools/state` (y compris le nom des entrées de premier niveau) et les champs structurés des événements de `GET /devtools/logs/poll`. Il ne coûte rien à `set_state` ni à l'émission d'un log, et les clés de `Config` valent quel que soit l'ordre `init` / `set_state`.

**Périmètre : ce qui n'est PAS filtré** : le résultat de `POST /devtools/eval`, les valeurs de retour de `POST /devtools/command`, la réponse de la route `ui.widgets`, et le texte libre des messages de log. Ces sorties sont produites à la demande par du code ou du JS de l'app, hors de portée d'un filtre par nom de clé.

**Justification** : seule l'option 2 protège par défaut sans macro ni nouvelle dépendance et couvre aussi les logs. L'option 1 coûte un crate procédural pour une couverture partielle ; l'option 3 laisse la fuite la plus probable ouverte. L'approximation (faux positifs possibles, faux négatifs sur un nom de clé anodin) est assumée et documentée.

## Conséquences

**Positives**

- état et champs de logs sont sûrs par défaut pour les noms de clés usuels ;
- aucune dépendance ajoutée ; coût nul hors des appels de lecture ;
- la documentation ne promet plus rien qu'elle ne livre.

**Coûts et risques**

- un secret stocké sous un nom neutre (`"x"`) ou dans un message de log passe : l'intégrateur doit déclarer ses clés ;
- les sorties de `eval`, de commandes et de `ui.widgets` restent non filtrées : c'est la limite documentée, à rappeler dans `SECURITY.md` ;
- ajout d'un champ public à `Config` (struct `#[non_exhaustive]`, donc compatible : changement mineur, ADR-0011).

## Notes

Application : `deveye-sdk/src/redact.rs`, `deveye-sdk/src/lib.rs` (`Config`, `state_handler`, `logs_poll_handler`), tests dans `deveye-sdk/tests/wire_protocol.rs`. Documentation : `docs/wire-protocol.md` §2.4 et `docs/architecture.md` (Sécurité).
