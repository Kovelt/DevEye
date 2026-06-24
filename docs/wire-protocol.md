# Wire Protocol DevEye — Spécification du contrat HTTP SDK

Version du document : 2026-06-24  
Source de vérité canonique : ce fichier (`docs/wire-protocol.md`).

---

## Périmètre et objectif

Ce document spécife le contrat HTTP que tout **SDK DevEye** doit implémenter pour être piloté par `deveye-server`. L'implémentation de référence est `deveye-sdk` (Rust/axum, workspace DevEye). Une implémentation vendored pour Atelier existe dans `Projets/Atelier/deveye/deveye-sdk/`. Le shim .NET `Atlas.DevEye` (jalon M3) devra satisfaire ce même contrat.

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

Ref. code : `deveye-sdk/src/lib.rs:48` (`pub const DEFAULT_PORT: u16 = 31337`) et `:61-68` (`Config::default` lit `DEVEYE_PORT`).

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

Ref. code : `deveye-sdk/src/lib.rs:173-197` (`guard_local_host` et `host_is_local`).

Valeurs acceptées par `host_is_local` (ref. tests `:314-330`) :

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

**Note** : ce header n'est pas encore émis par l'implémentation de référence au 2026-06-24 — c'est une extension à ajouter lors de la réconciliation M0.

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

Ref. code : `deveye-sdk/src/lib.rs:72-75` (struct `StatePayload`), `:232-239` (handler `state_handler`). Côté client : `adapter-dioxus-signals/src/bridge.rs` consomme `entries` via `SdkEvaluator` → `GET /devtools/eval` (chemin différent), mais la forme `{ "entries": {...} }` est validée par les tests `:305-312` du SDK.

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

**Schéma d'un `LogEvent`** (ref. `deveye-core/src/log_event.rs:27-35`) :

| Champ | Type JSON | Obligatoire | Description |
|---|---|---|---|
| `level` | string | oui | `"trace"`, `"debug"`, `"info"`, `"warn"` ou `"error"` (minuscules, ref. `:12` `#[serde(rename_all = "lowercase")]`) |
| `target` | string | oui | Module émetteur (ex. `my_app::ui::dashboard`) |
| `message` | string | oui | Texte du message |
| `timestamp` | string | oui | ISO 8601 / RFC 3339 avec offset UTC (`Z`) |
| `fields` | objet JSON | non | Absent ou null si aucune paire `key=value` tracing. Ref. `:33` `#[serde(default, skip_serializing_if = "serde_json::Value::is_null")]` |

**Codes HTTP** :

| Code | Cas |
|---|---|
| 200 | Succès (même si `events` est vide) |
| 403 | Header `Host` invalide |
| 4xx/5xx | Erreur interne du SDK |

**Sémantique de drain** (critique) :

Le poll est **destructif**. L'appel `GET /devtools/logs/poll` vide atomiquement le buffer interne et retourne les événements accumulés depuis le dernier poll. Un second appel immédiat retourne `{ "events": [] }`. Il n'y a pas de curseur, pas d'acquittement, pas de replay.

Ref. code : `deveye-sdk/src/lib.rs:241-248` — `guard.drain(..)` vide le `VecDeque` en une seule opération sous le verrou :

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

**Politique de buffer plein** : `drop-oldest`. La capacité maximale est `1024` événements (ref. `deveye-sdk/src/lib.rs:52` `pub const LOG_BUFFER_CAPACITY: usize = 1024`). Quand le buffer est plein, les événements les plus anciens sont écrasés par les nouveaux. Un SDK conforme DOIT implémenter une politique bornée ; la capacité exacte est paramétrable mais doit être documentée.

**Timeout client recommandé** : 10 secondes (ref. `adapter-logs-tracing/src/lib.rs:18` `const SDK_HTTP_TIMEOUT: Duration = Duration::from_secs(10)`). Ce timeout évite de bloquer Claude indéfiniment si l'app cible est figée (breakpoint, deadlock UI).

---

### 3.3 `POST /devtools/eval`

**Objectif** : évaluer une expression JavaScript dans le webview de l'app cible et retourner le résultat.

**Disponibilité** : uniquement si l'app cible a installé une **pompe d'évaluation** (appel à `take_eval_requests()` côté SDK Rust, ou équivalent). Sans pompe, le SDK retourne `503`.

Ref. code : `Atelier/deveye/deveye-sdk/src/lib.rs:85-95` (struct `EvalRequest` et champ `respond: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>`).

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

Ref. code : `Atelier/deveye/deveye-sdk/src/lib.rs:207-213` (struct `EvalResponse` avec `skip_serializing_if` sur les deux champs optionnels) :

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
| 503 Service Unavailable | Aucune pompe d'évaluation enregistrée, ou la pompe s'est arrêtée |
| 504 Gateway Timeout | L'app n'a pas répondu dans le délai (10 s) |
| 500 Internal Server Error | La pompe a abandonné la requête (channel fermé après envoi) |
| 403 | Header `Host` invalide |

Ref. code : `Atelier/deveye/deveye-sdk/src/lib.rs:244` — timeout de 10 secondes :

```rust
match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
```

**Sémantique d'appariement requête/réponse** :

Le SDK crée un canal `oneshot` par requête entrante. Le canal est passé à la pompe de l'app via un `mpsc::UnboundedSender<EvalRequest>`. La pompe reçoit la requête, exécute l'expression dans le webview, et répond sur le `Sender<Result<serde_json::Value, String>>`. Le handler HTTP attend sur le `Receiver` avec un timeout de 10 secondes. Ce mécanisme garantit qu'une réponse correspond exactement à sa requête, sans état partagé entre les requêtes concurrentes.

**Contrat de la pompe côté app** :

```
loop {
    request = recv(eval_requests)       // bloquant
    result = webview.eval(request.expression)
    request.respond.send(result)
}
```

La pompe ne doit pas oublier d'envoyer sur `respond` : si elle abandonne le `Sender` sans répondre, le handler HTTP reçoit `500`. Si elle met plus de 10 secondes à répondre, le handler retourne `504` et le client (adapter) reçoit `PortError::Network`.

Côté client : `adapter-dioxus-signals/src/bridge.rs:48-73` (`SdkEvaluator::evaluate`) envoie `POST /devtools/eval` avec le corps `{ "expression": "..." }` et mappe les codes HTTP sur `PortError` :

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

Ref. code : `deveye-sdk/src/lib.rs:86-91` (struct `CommandRequest` avec `#[serde(default)]` sur `payload`).

**Réponse 200** :

Le corps est la valeur JSON retournée par le handler de la commande. Il n'y a pas d'enveloppe : si le handler retourne `{ "ok": true }`, la réponse HTTP est directement `{ "ok": true }`. Si le handler retourne une chaîne, la réponse est une chaîne JSON.

**Réponse 404** :

```json
{
  "error": "commande inconnue : <route>"
}
```

Ref. code : `deveye-sdk/src/lib.rs:261-273` (handler `command_handler`).

**Codes HTTP** :

| Code | Cas |
|---|---|
| 200 | Commande trouvée et exécutée |
| 404 | Route inconnue |
| 403 | Header `Host` invalide |

**Sémantique** : le handler est exécuté **de façon synchrone** sous le verrou des routes (ref. `:256-263`, le verrou est libéré avant l'exécution — voir commentaire). Le handler reçoit `payload` et retourne un `serde_json::Value`. Il est responsable de sa propre gestion d'erreur ; aucune convention de retour d'erreur n'est imposée à ce niveau (c'est l'app qui décide du schéma de sa réponse).

Côté client : `adapter-commands-http/src/lib.rs:50-54` (struct `CommandRequest`) et `:62-77` (`SdkCommandRouter::invoke`). Un code non-2xx est traduit en `PortError::Route`.

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

Schéma d'un `CommandDescriptor` (ref. `deveye-core/src/command.rs:97-100`) :

| Champ | Type | Description |
|---|---|---|
| `name` | string | Identifiant de la commande, identique à celui de `route` dans `POST /devtools/command` |
| `description` | string | Description lisible par un agent (Claude) pour comprendre la sémantique de la commande |

**Codes HTTP** :

| Code | Cas |
|---|---|
| 200 | Succès (même si la liste est vide) |
| 403 | Header `Host` invalide |

Ref. code : `deveye-sdk/src/lib.rs:276-292` (handler `commands_handler`, tri par `a.name.cmp(&b.name)`). Côté client : `adapter-commands-http/src/lib.rs:83-101` (`SdkCommandRouter::list`).

---

## 4. Réconciliation entre les deux implémentations SDK

Au 2026-06-24, deux implémentations coexistent. Leurs différences constituent le périmètre principal du jalon M0.

| Fonctionnalité | SDK DevEye (`deveye-sdk`) | SDK Atelier vendored (`Atelier/deveye/deveye-sdk`) |
|---|---|---|
| `GET /devtools/state` | oui | oui |
| `GET /devtools/logs/poll` | oui | oui |
| `POST /devtools/eval` | **absent** | oui (ref. `:148`) |
| `POST /devtools/command` | oui | **absent** |
| `GET /devtools/commands` | oui | **absent** |
| Garde `Host` | oui (middleware axum) | **absente** (ref. `:144-149` — pas de layer middleware) |
| Runtime tokio | celui de l'app hôte | runtime dédié SDK (1 thread, ref. `:125-135`) |
| `once_cell` vs `LazyLock` | `std::sync::LazyLock` (stable) | `once_cell::sync::Lazy` (dep externe) |
| Locks | `std::sync::RwLock` / `Mutex` | `tokio::sync::RwLock` / `Mutex` |

Le SDK DevEye canonique doit absorber `POST /devtools/eval` et la logique de pompe. Le SDK Atelier vendored doit absorber la garde `Host`, `POST /devtools/command` et `GET /devtools/commands`. À l'issue de M0, les deux surfaces sont identiques et le SDK Atelier peut être remplacé par une dépendance directe à `deveye-sdk` (sous `cfg(all(feature = "deveye", debug_assertions))`).

---

## 5. Tableau récapitulatif des endpoints

| Méthode | Chemin | Destructif | Corps requête | Corps réponse | Timeout client recommandé |
|---|---|---|---|---|---|
| GET | `/devtools/state` | non | aucun | `{ "entries": {...} }` | 10 s |
| GET | `/devtools/logs/poll` | **oui** (drain) | aucun | `{ "events": [...] }` | 10 s |
| POST | `/devtools/eval` | non | `{ "expression": "..." }` | `{ "value": ... }` ou `{ "error": "..." }` | **20 s** (10 s timeout SDK + marge réseau) |
| POST | `/devtools/command` | selon commande | `{ "route": "...", "payload": ... }` | JSON libre | 10 s |
| GET | `/devtools/commands` | non | aucun | `[{ "name": "...", "description": "..." }]` | 10 s |

Note sur le timeout de `/devtools/eval` : le SDK impose un timeout interne de 10 s (ref. `Atelier/deveye/deveye-sdk/src/lib.rs:244`). Le client doit prévoir une marge supplémentaire pour le réseau loopback (négligeable en pratique) et la latence de sérialisation. 15-20 s côté client est raisonnable pour éviter un double-timeout.

---

## 6. Checklist : implémenter un SDK conforme

Cette liste est le guide opérationnel pour le shim .NET `Atlas.DevEye` (jalon M3) ou tout autre SDK dans un langage tiers.

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
- [ ] Retourner `{ "error": "commande inconnue : <route>" }` avec 404 si la route est inconnue.

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
