# Wire Protocol DevEye — Spécification du contrat HTTP SDK

Version du document : 2026-07-04  
Source de vérité canonique : ce fichier (`docs/wire-protocol.md`).

---

## Périmètre et objectif

Ce document spécife le contrat HTTP que tout **SDK DevEye** doit implémenter pour être piloté par `deveye-server`. L'implémentation de référence est `deveye-sdk` (Rust/axum, workspace DevEye). Toute implémentation dans un autre langage (par exemple un shim .NET) doit satisfaire ce même contrat ; la checklist de la section 6 sert de guide.

La spec est **agnostique du langage** : n'importe quelle techno capable de servir du HTTP/1.1 sur localhost peut implémenter un SDK conforme. Elle est volontairement minimale : aucun WebSocket, aucun streaming, aucune authentification — la sécurité repose sur le bind loopback et la garde `Host`.

---

## 1. Transport et adressage

| Paramètre | Valeur |
|---|---|
| Protocole | HTTP/1.1 (pas de TLS, pas de HTTP/2 requis) |
| Adresse de bind | `127.0.0.1` exclusivement — jamais `0.0.0.0` ni `::` |
| Port par défaut | `31337` |
| Variable de surcharge | `DEVEYE_PORT` (entier, 1024-65535) |
| Base URL côté client | `http://127.0.0.1:<port>` |

Le choix du port `31337` est arbitraire et mnémonique. Il doit être surchargeable sans recompilation via `DEVEYE_PORT`, ce qui est indispensable quand plusieurs apps tournent simultanément en dev.

Ref. code : `deveye-sdk/src/lib.rs` — constante `DEFAULT_PORT` (`pub const DEFAULT_PORT: u16 = 31337`) et `impl Default for Config` (lit `DEVEYE_PORT`).

---

## 2. Sécurité

### 2.1 Restriction au build debug

Le SDK ne doit jamais être embarqué dans un build de production. En Rust canonique, l'intégrateur respecte le pattern :

```rust
#[cfg(debug_assertions)]
{
    deveye_sdk::init(deveye_sdk::Config::default());
}
```

Un SDK conforme dans un autre langage doit être conditionné par un équivalent compile-time ou un flag explicitement activé en dev (ex. `#if DEBUG` en C#). Ne jamais activer par variable d'environnement seule : une variable peut fuiter en prod ; une directive de compilation, non.

### 2.2 Garde Host anti DNS-rebinding

Toute requête dont le header `Host` n'est pas `localhost`, `127.0.0.1` ou `[::1]` (avec port optionnel, insensible à la casse pour `localhost`) doit être rejetée avec `403 Forbidden`, **avant** tout traitement de la route.

Ref. code : `deveye-sdk/src/lib.rs` — fonctions `guard_local_host` et `host_is_local`.

Valeurs acceptées par `host_is_local` (ref. tests `host_is_local_accepts_loopback_forms` / `host_is_local_rejects_foreign_hosts`) :

```
localhost
localhost:31337
127.0.0.1
127.0.0.1:31337
LocalHost:31337      (insensible a la casse)
[::1]
[::1]:31337
```

Valeurs rejetées :

```
evil.com
evil.com:31337
127.0.0.1.evil.com
(chaine vide)
```

Rationale : le bind sur `127.0.0.1` empêche les connexions extérieures, mais ne suffit pas contre le DNS-rebinding — une page web malveillante peut résoudre un domaine sous son contrôle vers `127.0.0.1` et envoyer des requêtes avec un `Host` étranger. La garde coupe ce vecteur.

### 2.3 Header de version protocole

Chaque réponse du SDK DOIT inclure le header :

```
X-DevEye-Protocol-Version: 1
```

Le client (adapters DevEye) peut utiliser ce header pour détecter une incompatibilité de version et produire un message d'erreur exploitable plutôt qu'un échec JSON silencieux. La valeur courante est `1` ; elle sera incrémentée en cas de changement cassant du contrat.

**État** : émis par l'implémentation de référence. Le middleware `add_protocol_version` est posé en **couche externe** de la garde Host, de sorte que *chaque* réponse porte le header — y compris le `403` de `guard_local_host`. Ref. code : `deveye-sdk/src/lib.rs` (`PROTOCOL_VERSION`, `add_protocol_version`).

---

## 3. Endpoints

Le SDK expose cinq routes. Toutes retournent `Content-Type: application/json`.

### 3.1 `GET /devtools/state`

**Objectif** : obtenir un snapshot instantané du state applicatif publié par l'app cible.

**Requête** : aucun corps, aucun paramètre de query.

**Réponse 200** :

```json
{
  "entries": {
    "<cle>": <valeur JSON arbitraire>,
    ...
  }
}
```

`entries` est un objet dont les clés sont des chaînes UTF-8 et les valeurs sont des valeurs JSON quelconques (string, number, bool, object, array, null). Les clés sont triées alphabétiquement (BTreeMap côté SDK Rust — comportement non garanti pour d'autres implémentations, mais recommandé pour la reproductibilité des tests).

**Codes HTTP** :

| Code | Cas |
|---|---|
| 200 | Succès (même si `entries` est vide) |
| 403 | Header `Host` invalide (garde localhost) |

**Sémantique** : lecture non destructive. Plusieurs appels successifs retournent le dernier état connu. L'app cible met à jour `entries` via `set_state(name, value)` (écrasement, dernière écriture gagne).

Ref. code : `deveye-sdk/src/lib.rs` — struct `StatePayload` et handler `state_handler`. Côté client : `adapter-dioxus-signals/src/lib.rs` (`DioxusSignalsInspector`) consomme `entries` via `GET /devtools/state` — la forme `{ "entries": {...} }` est validée par le test `state_payload_serializes_with_entries_key` du SDK. À ne pas confondre avec `SdkEvaluator` (`adapter-dioxus-signals/src/bridge.rs`) qui emprunte un chemin distinct : `POST /devtools/eval` avec corps `{ "expression": "..." }` (section 3.3).

Exemple :

```json
{
  "entries": {
    "page": "Agenda",
    "user_id": 42,
    "modal_open": false
  }
}
```

---

### 3.2 `GET /devtools/logs/poll`

**Objectif** : récupérer et **vider** le buffer de logs applicatifs accumulés depuis le dernier poll.

**Requête** : aucun corps, aucun paramètre de query.

**Réponse 200** :

```json
{
  "events": [
    {
      "level": "info",
      "target": "mon_app::ui::agenda",
      "message": "rendu agenda termine",
      "timestamp": "2026-06-24T14:32:01.123456789Z",
      "fields": { "duration_ms": 12 }
    },
    {
      "level": "warn",
      "target": "mon_app::db",
      "message": "requete lente",
      "timestamp": "2026-06-24T14:32:01.200000000Z"
    }
  ]
}
```

**Schéma d'un `LogEvent`** (ref. `deveye-core/src/log_event.rs` — struct `LogEvent`) :

| Champ | Type JSON | Obligatoire | Description |
|---|---|---|---|
| `level` | string | oui | `"trace"`, `"debug"`, `"info"`, `"warn"` ou `"error"` (minuscules, ref. attribut `#[serde(rename_all = "lowercase")]` sur l'enum `LogLevel`) |
| `target` | string | oui | Module émetteur (ex. `my_app::ui::dashboard`) |
| `message` | string | oui | Texte du message |
| `timestamp` | string | oui | ISO 8601 / RFC 3339 avec offset UTC (`Z`) |
| `fields` | objet JSON | non | Absent ou null si aucune paire `key=value` tracing. Ref. attribut `#[serde(default, skip_serializing_if = "serde_json::Value::is_null")]` sur le champ `fields` de `LogEvent` |

**Codes HTTP** :

| Code | Cas |
|---|---|
| 200 | Succès (même si `events` est vide) |
| 403 | Header `Host` invalide |
| 4xx/5xx | Erreur interne du SDK |

**Sémantique de drain** (critique) :

Le poll est **destructif**. L'appel `GET /devtools/logs/poll` vide atomiquement le buffer interne et retourne les événements accumulés depuis le dernier poll. Un second appel immédiat retourne `{ "events": [] }`. Il n'y a pas de curseur, pas d'acquittement, pas de replay.

Ref. code : `deveye-sdk/src/lib.rs` — handler `logs_poll_handler` : `guard.drain(..)` vide le `VecDeque` en une seule opération sous le verrou :

```rust
async fn logs_poll_handler(State(registry): State<Arc<Registry>>) -> impl IntoResponse {
    let events: Vec<_> = registry
        .logs
        .lock()
        .map(|mut guard| guard.drain(..).collect())
        .unwrap_or_default();
    (StatusCode::OK, Json(LogsPayload { events }))
}
```

**Comportement en cas de crash ou de déconnexion du client** :

Si `deveye-server` s'arrête ou crashe entre deux polls, les événements déjà drainés sont **perdus définitivement**. Ceux qui n'ont pas encore été pollés restent dans le buffer SDK tant que l'app cible tourne. À la reconnexion, le prochain poll récupère le contenu restant (éventuellement tronqué si le buffer a débordé).

**Politique de buffer plein** : `drop-oldest`. La capacité maximale est `1024` événements (ref. `deveye-sdk/src/lib.rs` — constante `LOG_BUFFER_CAPACITY`). Quand le buffer est plein, les événements les plus anciens sont écrasés par les nouveaux. Un SDK conforme DOIT implémenter une politique bornée ; la capacité exacte est paramétrable mais doit être documentée.

**Timeout client recommandé** : ce que configure effectivement `adapter-logs-tracing` (et tout adapter qui parle au SDK) via le client HTTP partagé `deveye_sdk_client::client()` — `CONNECT_TIMEOUT = 3 s` et `REQUEST_TIMEOUT = 15 s` (ref. `deveye-sdk-client/src/lib.rs`). Il n'existe pas de constante `SDK_HTTP_TIMEOUT` dédiée côté `adapter-logs-tracing` : ce crate, comme `adapter-commands-http` et `adapter-dioxus-signals`, réutilise le même `reqwest::Client` préconfiguré pour éviter la répétition. Ce timeout évite de bloquer Claude indéfiniment si l'app cible est figée (breakpoint, deadlock UI).

---

### 3.3 `POST /devtools/eval`

**Objectif** : évaluer une expression JavaScript dans le webview de l'app cible et retourner le résultat.

**Disponibilité** : uniquement si l'app cible a installé une **pompe d'évaluation** (appel à `take_eval_requests()` côté SDK Rust, ou équivalent). Sans pompe, le SDK retourne `503`. Le SDK retourne aussi `503` si la file d'évaluation est **pleine** (backpressure, cf. ci-dessous) : les deux causes partagent le même code HTTP mais un message distinct (`"aucune pompe d'évaluation enregistrée..."` vs `"file d'évaluation pleine (backpressure)..."`).

Ref. code : `deveye-sdk/src/lib.rs` (struct `EvalRequest` et champ `respond: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>`).

**Requête** :

```json
{
  "expression": "<expression JS>"
}
```

| Champ | Type | Obligatoire | Description |
|---|---|---|---|
| `expression` | string | oui | Expression JS à évaluer. Typiquement une IIFE `(function(){ ... })()`. La pompe l'exécute dans le contexte du webview de l'app. |

**Réponse 200 — succès** :

```json
{
  "value": <valeur JSON retournee par JS>
}
```

**Réponse 200 — erreur JS** (l'expression a levé une exception ou retourne une erreur) :

```json
{
  "error": "message d'erreur"
}
```

Note : le code HTTP est `200` dans les deux cas quand l'expression a été transmise et exécutée. C'est l'absence de `value` (ou la présence de `error`) qui signale l'échec applicatif.

Ref. code : `deveye-sdk/src/lib.rs` (struct `EvalResponse` avec `skip_serializing_if` sur les deux champs optionnels) :

```rust
#[derive(Serialize)]
struct EvalResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}
```

**Codes HTTP** :

| Code | Cas |
|---|---|
| 200 | Expression transmise et exécutée (succès ou erreur JS — voir corps) |
| 503 Service Unavailable | Aucune pompe d'évaluation enregistrée, la pompe s'est arrêtée, ou la file d'évaluation est pleine (backpressure) |
| 504 Gateway Timeout | L'app n'a pas répondu dans le délai (10 s) |
| 500 Internal Server Error | La pompe a abandonné la requête (channel fermé après envoi) |
| 403 | Header `Host` invalide |

Ref. code : `deveye-sdk/src/lib.rs` — timeout de 10 secondes :

```rust
match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
```

**Sémantique d'appariement requête/réponse** :

Le SDK crée un canal `oneshot` par requête entrante. Le canal est passé à la pompe de l'app via un `mpsc::Sender<EvalRequest>` **borné** (capacité `EVAL_QUEUE_CAPACITY = 32`, cf. `deveye-sdk/src/lib.rs`) — pas un `UnboundedSender` : un canal illimité ferait enfler la mémoire sans limite si l'app cible cesse de drainer sa pompe (figée). Le handler HTTP envoie via `try_send` (non bloquant) : sur file pleine, il répond `503` immédiatement plutôt que d'attendre. La pompe reçoit la requête, exécute l'expression dans le webview, et répond sur le `Sender<Result<serde_json::Value, String>>`. Le handler HTTP attend sur le `Receiver` avec un timeout de 10 secondes. Ce mécanisme garantit qu'une réponse correspond exactement à sa requête, sans état partagé entre les requêtes concurrentes.

**Contrat de la pompe côté app** :

```
loop {
    request = recv(eval_requests)       // bloquant
    result = webview.eval(request.expression)
    request.respond.send(result)
}
```

La pompe ne doit pas oublier d'envoyer sur `respond` : si elle abandonne le `Sender` sans répondre, le handler HTTP reçoit `500`. Si elle met plus de 10 secondes à répondre, le handler retourne `504` et le client (adapter) reçoit `PortError::Timeout` (et non `PortError::Network` — distinct d'une absence de liaison, cf. `SdkEvaluator::evaluate` ci-dessous).

Côté client : `adapter-dioxus-signals/src/bridge.rs` (`SdkEvaluator::evaluate`) envoie `POST /devtools/eval` avec le corps `{ "expression": "..." }` et mappe les codes HTTP sur `PortError` :

```rust
let url = format!("{}/devtools/eval", self.base_url.trim_end_matches('/'));
let resp = self
    .http
    .post(&url)
    .json(&serde_json::json!({ "expression": expression }))
    .send()
    .await
    .map_err(|e| PortError::Network(format!("POST {url} : {e}")))?;
```

**Cas `503`** : l'adapter le traduit en `PortError::Network` (pas `Render`), ce qui permet aux compositions à repli de basculer sur un autre évaluateur plutôt que de signaler une erreur JS.

---

### 3.4 `POST /devtools/command`

**Objectif** : invoquer une commande logique nommée enregistrée par l'app cible.

**Requête** :

```json
{
  "route": "nom-de-la-commande",
  "payload": <valeur JSON arbitraire>
}
```

| Champ | Type | Obligatoire | Description |
|---|---|---|---|
| `route` | string | oui | Nom de la commande, tel qu'enregistré via `expose_route` |
| `payload` | JSON | non (défaut `null`) | Arguments de la commande. Schéma défini par l'app cible. |

Ref. code : `deveye-sdk/src/lib.rs` — struct `CommandRequest` (avec `#[serde(default)]` sur `payload`).

**Réponse 200** :

Le corps est la valeur JSON retournée par le handler de la commande. Il n'y a pas d'enveloppe : si le handler retourne `{ "ok": true }`, la réponse HTTP est directement `{ "ok": true }`. Si le handler retourne une chaîne, la réponse est une chaîne JSON.

**Réponse 404** (corps minimal : la route n'est pas répétée, elle est déjà portée par `PortError::Route` côté client — audit C2-b) :

```json
{
  "error": "inconnue"
}
```

Ref. code : `deveye-sdk/src/lib.rs` — handler `command_handler`.

**Codes HTTP** :

| Code | Cas |
|---|---|
| 200 | Commande trouvée et exécutée |
| 404 | Route inconnue |
| 403 | Header `Host` invalide |

**Sémantique** : dans `command_handler`, le verrou de lecture sur les routes n'est tenu que pour cloner le handler (`Arc`) ; il est **relâché avant l'exécution**, qui a lieu dans `tokio::task::spawn_blocking` (le handler peut être bloquant sans figer le runtime SDK, et un panic y est capturé plutôt que de faire tomber le serveur). Le handler reçoit `payload` et retourne un `serde_json::Value`. Il est responsable de sa propre gestion d'erreur ; aucune convention de retour d'erreur n'est imposée à ce niveau (c'est l'app qui décide du schéma de sa réponse).

Côté client : `adapter-commands-http/src/lib.rs` (`SdkCommandRouter::invoke`) — **il n'existe pas de struct `CommandRequest` côté client** : le corps `{ "route": ..., "payload": ... }` est construit directement via `serde_json::json!({ "route": route, "payload": payload })`, sans type dédié (seul le serveur, `deveye-sdk`, a une struct `CommandRequest` pour désérialiser sa requête entrante — cf. section 3.4 ci-dessus). Un code non-2xx est traduit en `PortError::Route` (le corps `{ "error": ... }` est extrait si présent, sinon le texte brut est inclus dans le message).

---

### 3.5 `GET /devtools/commands`

**Objectif** : lister les commandes logiques disponibles (découverte).

**Requête** : aucun corps.

**Réponse 200** :

Tableau JSON de `CommandDescriptor`, trié alphabétiquement par `name` :

```json
[
  {
    "name": "navigate",
    "description": "Navigue vers une page de l'application"
  },
  {
    "name": "set-theme",
    "description": "Change le theme visuel (valeurs : light, dark)"
  }
]
```

Schéma d'un `CommandDescriptor` (ref. `deveye-core/src/command.rs` — struct `CommandDescriptor`) :

| Champ | Type | Description |
|---|---|---|
| `name` | string | Identifiant de la commande, identique à celui de `route` dans `POST /devtools/command` |
| `description` | string | Description lisible par un agent (Claude) pour comprendre la sémantique de la commande |

**Codes HTTP** :

| Code | Cas |
|---|---|
| 200 | Succès (même si la liste est vide) |
| 403 | Header `Host` invalide |

Ref. code : `deveye-sdk/src/lib.rs` — handler `commands_handler` (tri par `a.name.cmp(&b.name)`). Côté client : `adapter-commands-http/src/lib.rs` (`SdkCommandRouter::list`) — un code non-2xx y est traduit en `PortError::Network` (et non `PortError::Route` : un échec de listing est un problème de liaison/SDK, pas une route inconnue).

---

## 4. Garanties de l'implémentation de référence

`deveye-sdk` implémente le contrat complet de ce document :

| Fonctionnalité | Statut |
|---|---|
| `GET /devtools/state`, `GET /devtools/logs/poll`, `POST /devtools/eval` | oui |
| `POST /devtools/command` (`command_handler`) et `GET /devtools/commands` (`commands_handler`, tri par `name`) | oui |
| Garde `Host` (`guard_local_host`, middleware, `403` hors localhost) | oui |
| Header `X-DevEye-Protocol-Version` (toutes réponses, y compris `403`) | oui |
| `expose_route` (enregistrement de commandes) | oui |
| `504` sur dépassement de l'évaluation, traduit en `PortError::Timeout` côté `adapter-dioxus-signals` | oui |

Un runtime tokio dédié (`std::sync::LazyLock`) et des locks `tokio::sync` isolent le SDK du runtime de l'app hôte. `deveye-sdk` est l'**implémentation de référence** : en cas de divergence entre ce document et une autre implémentation, c'est le code de `deveye-sdk` qui fait foi.

---

## 5. Tableau récapitulatif des endpoints

| Méthode | Chemin | Destructif | Corps requête | Corps réponse | Timeout client réel |
|---|---|---|---|---|---|
| GET | `/devtools/state` | non | aucun | `{ "entries": {...} }` | 15 s (connexion 3 s) |
| GET | `/devtools/logs/poll` | **oui** (drain) | aucun | `{ "events": [...] }` | 15 s (connexion 3 s) |
| POST | `/devtools/eval` | non | `{ "expression": "..." }` | `{ "value": ... }` ou `{ "error": "..." }` | 15 s (connexion 3 s) |
| POST | `/devtools/command` | selon commande | `{ "route": "...", "payload": ... }` | JSON libre | 15 s (connexion 3 s) |
| GET | `/devtools/commands` | non | aucun | `[{ "name": "...", "description": "..." }]` | 15 s (connexion 3 s) |

Les cinq endpoints partagent le **même** `reqwest::Client`, construit une fois par `deveye_sdk_client::client()` (`CONNECT_TIMEOUT = 3 s`, `REQUEST_TIMEOUT = 15 s`, ref. `deveye-sdk-client/src/lib.rs`) — aucun timeout n'est configuré par endpoint ni par adapter. Il n'existe pas de constante `SDK_HTTP_TIMEOUT`.

Note sur le timeout de `/devtools/eval` : le SDK impose en plus un timeout **interne** de 10 s (ref. `deveye-sdk/src/lib.rs`, `tokio::time::timeout(Duration::from_secs(10), ...)`) avant de répondre `504`. Le `REQUEST_TIMEOUT` de 15 s côté client laisse une marge suffisante (5 s) pour le réseau loopback et la sérialisation sans déclencher de double-timeout. Côté client Rust, un `504` est distingué par `PortError::Timeout` (cf. `adapter-dioxus-signals/src/bridge.rs`).

---

## 6. Checklist : implémenter un SDK conforme

Cette liste est le guide opérationnel pour tout SDK dans un langage tiers (par exemple .NET).

### Prérequis

- [ ] Le SDK n'est actif **qu'en mode debug** (directive de compilation, pas une variable d'environnement). En C# : entourer l'initialisation de `#if DEBUG ... #endif`.
- [ ] Choisir une bibliothèque HTTP embarquable légère (ex. `Microsoft.AspNetCore` minimal API, ou `EmbedIO`, ou `HttpListener` natif .NET).

### Bind et port

- [ ] Binder exclusivement sur `127.0.0.1:<port>` — jamais `0.0.0.0`.
- [ ] Lire `DEVEYE_PORT` en variable d'environnement au démarrage ; utiliser `31337` si absente ou invalide.
- [ ] En cas d'échec de bind (port déjà pris), loguer en debug et continuer sans paniquer l'app hôte.

### Garde Host (obligatoire)

- [ ] Implémenter un middleware (ou filtre de requête) qui vérifie le header `Host` avant toute route.
- [ ] Accepter : `localhost` (insensible à la casse), `localhost:<port>`, `127.0.0.1`, `127.0.0.1:<port>`, `[::1]`, `[::1]:<port>`.
- [ ] Rejeter tout autre `Host` avec `403 Forbidden` sans traiter la route.
- [ ] Inclure `X-DevEye-Protocol-Version: 1` dans toutes les réponses.

### `GET /devtools/state`

- [ ] Maintenir un dictionnaire thread-safe `string → JSON`.
- [ ] Exposer une fonction publique `set_state(name, value)` appelable depuis l'app hôte.
- [ ] Retourner `{ "entries": { ... } }` avec 200.
- [ ] Lecture non destructive.

### `GET /devtools/logs/poll`

- [ ] Maintenir un buffer circulaire d'événements `LogEvent`, capacité bornée (1024 recommandé, politique drop-oldest).
- [ ] Exposer un point d'injection pour la couche de logging de l'app (ex. `ILogger` sink en .NET).
- [ ] Drain atomique : vider le buffer en une opération et retourner les événements.
- [ ] Schéma `LogEvent` : `level` (minuscules : `trace|debug|info|warn|error`), `target` (string), `message` (string), `timestamp` (RFC 3339 UTC), `fields` (objet JSON, omis si null).
- [ ] Retourner `{ "events": [] }` (pas d'erreur) si le buffer est vide.

### `POST /devtools/eval`

- [ ] Exposer une fonction `take_eval_requests()` (ou équivalent) qui retourne un canal de réception de requêtes d'évaluation.
- [ ] L'app enregistre une **pompe** : boucle qui reçoit une requête `{ expression, oneshot_sender }`, exécute l'expression dans le webview, et répond sur le sender.
- [ ] Le handler HTTP crée un oneshot par requête, envoie `{ expression, sender }` à la pompe, attend la réponse avec un timeout de **10 secondes**.
- [ ] Réponse 200 avec `{ "value": ... }` si succès JS.
- [ ] Réponse 200 avec `{ "error": "..." }` si l'expression lève une exception.
- [ ] Réponse 503 si aucune pompe n'est enregistrée ou si le canal est fermé.
- [ ] Réponse 504 si le timeout de 10 s expire.
- [ ] Réponse 500 si la pompe abandonne le sender sans répondre.

### `POST /devtools/command`

- [ ] Maintenir un registre thread-safe `string → handler`.
- [ ] Exposer une fonction `expose_route(name, description, handler)` appelable depuis l'app hôte.
- [ ] Lire `{ "route": "...", "payload": ... }` (payload peut être null ou absent).
- [ ] Exécuter le handler correspondant, retourner son résultat JSON avec 200.
- [ ] Retourner `{ "error": "inconnue" }` avec 404 si la route est inconnue.

### `GET /devtools/commands`

- [ ] Retourner la liste triée alphabétiquement des commandes enregistrées.
- [ ] Schéma : `[{ "name": "...", "description": "..." }]`.
- [ ] Retourner `[]` (pas d'erreur) si aucune commande n'est enregistrée.

### Validation

- [ ] Vérifier avec `curl -H "Host: 127.0.0.1:31337" http://127.0.0.1:31337/devtools/state` que la garde accepte localhost.
- [ ] Vérifier avec `curl -H "Host: evil.com" http://127.0.0.1:31337/devtools/state` que la garde retourne 403.
- [ ] Vérifier que `GET /devtools/logs/poll` appelé deux fois consécutivement retourne une liste vide au second appel.
- [ ] Vérifier que `POST /devtools/eval` sans pompe retourne 503.
- [ ] Vérifier que `POST /devtools/command` avec une route inexistante retourne 404.
- [ ] Vérifier que le serveur ne démarre pas si `debug_assertions` est false (ou `#if DEBUG` est désactivé).
- [ ] Vérifier que le header `X-DevEye-Protocol-Version: 1` est présent dans chaque réponse.
