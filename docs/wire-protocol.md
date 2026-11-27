# Wire Protocol DevEye — Spécification du contrat HTTP SDK

Version du document : 2026-11-27  
**Version du protocole : 1.0 (figée pour la série 1.x de DevEye)**  
Source de vérité canonique : ce fichier (`docs/wire-protocol.md`), complété par les [schémas JSON](#8-schémas-json) de `docs/schemas/1.0/`. Décisions : [ADR-0017](adr/0017-protocole-1-0-version-info-et-limites.md) (version, limites), [ADR-0035](adr/0035-suite-de-conformite-boite-noire-du-protocole.md) (suite de conformité).

---

## Périmètre et objectif

Ce document spécifie le contrat HTTP que tout **SDK DevEye** implémente pour être piloté par `deveye-server`. L'implémentation de référence est `deveye-sdk` (Rust/axum, workspace DevEye). Toute implémentation dans un autre langage (par exemple un shim .NET, maintenu hors de ce dépôt) satisfait ce même contrat : la checklist de la section 6 sert de guide d'écriture, et la **suite de conformité** `deveye-conformance` (section 7) la valide à partir de sa seule URL, sans accès à son code.

Les mots **DOIT**, **NE DOIT PAS** et **DEVRAIT** ont le sens de la RFC 2119. Une exigence « DOIT » est vérifiée par la suite de conformité ; une exigence « DEVRAIT » ne l'est pas.

La spec est **agnostique du langage** : n'importe quelle techno capable de servir du HTTP/1.1 sur localhost peut implémenter un SDK conforme. Elle est volontairement minimale : aucun WebSocket, aucun streaming, aucune authentification — la sécurité repose sur le bind loopback et la garde `Host`.

---

## 1. Transport et adressage

| Paramètre | Valeur |
|---|---|
| Protocole | HTTP/1.1 (pas de TLS, pas de HTTP/2 requis) |
| Adresse de bind | `127.0.0.1` exclusivement — jamais `0.0.0.0` ni `::` |
| Port par défaut | `31337` |
| Variable de surcharge | `DEVEYE_PORT` : entier décimal de 1 à 65535, chiffres décimaux seuls (ni espace ni signe, `+4242` est ignoré) |
| Base URL côté client | `http://127.0.0.1:<port>` |

Le choix du port `31337` est arbitraire et mnémonique. Il est surchargeable sans recompilation via `DEVEYE_PORT`, ce qui est indispensable quand plusieurs apps tournent simultanément en dev.

**Lecture de `DEVEYE_PORT`** : toute valeur qui n'est pas un entier décimal de 1 à 65535 (variable absente ou vide, texte, `0`, valeur négative ou supérieure à 65535, espaces autour) est ignorée et le port par défaut `31337` s'applique, sans erreur. `0` est exclu : il demanderait un port éphémère que personne ne pourrait deviner. Les ports inférieurs à 1024 sont acceptés par la règle mais exigent des privilèges sur la plupart des systèmes ; l'échec du bind est alors traité comme un port déjà pris. Un SDK DOIT, en cas d'échec du bind, signaler l'incident (log) et laisser l'app hôte tourner.

**Connexions** : HTTP/1.1, requêtes indépendantes ; aucune session, aucun cookie. Le serveur DEVRAIT accepter la réutilisation de connexion (`keep-alive`) mais un client ne peut pas la présumer.

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

Valeurs rejetées (13 cas, tous couverts par la suite de conformité) :

```
evil.com
evil.com:31337
127.0.0.1.evil.com
localhost.evil.com
sub.localhost
0.0.0.0
::1                  (IPv6 sans crochets : non reconnu)
[::1]toto            (suffixe arbitraire après le crochet)
[::1]evil.com
[::1]:abc            (port non numérique)
[::1]:               (port vide)
127.0.0.1:abc
(chaine vide)
```

Règles précises :

- le `Host` est un nom d'hôte, éventuellement suivi de `:` et d'un port **purement numérique** (au moins un chiffre) ; un `Host` sans port est valide (`localhost`, `127.0.0.1`, `[::1]`) ;
- l'IPv6 loopback n'est reconnu que **entre crochets** (`[::1]`) ; le suffixe après `]` est vide ou `:<chiffres>` ;
- `localhost` est insensible à la casse ; `127.0.0.1` est comparé tel quel ; aucune autre adresse, aucun sous-domaine, aucun `0.0.0.0` ;
- le port annoncé dans `Host` n'a pas à être celui du serveur (seule sa forme numérique compte) ;
- un `Host` **absent** (requête HTTP/1.0 ou HTTP/1.1 sans l'en-tête) est rejeté comme un `Host` étranger (`403`). Exception admise : une pile HTTP qui le refuse elle-même avant tout middleware, ce que la RFC 9112 §3.2 impose à un serveur HTTP/1.1 (par exemple Kestrel), répond `400` ; l'application n'est alors jamais atteinte, ce qui satisfait l'objectif de la garde ;
- la garde s'applique à **toutes** les requêtes, toutes méthodes et tous chemins confondus (y compris un chemin inconnu et `OPTIONS`), et **avant** le routage et le décodage du corps : un corps invalide avec un `Host` étranger donne `403`, jamais `400`.

**Réponse de rejet** : `403 Forbidden`, `Content-Type: application/json`, header de version, corps :

```json
{ "error": "Host non local" }
```

Le texte du message est libre ; le champ `error` (chaîne) DOIT être présent.

Rationale : le bind sur `127.0.0.1` empêche les connexions extérieures, mais ne suffit pas contre le DNS-rebinding — une page web malveillante peut résoudre un domaine sous son contrôle vers `127.0.0.1` et envoyer des requêtes avec un `Host` étranger. La garde coupe ce vecteur.

### 2.3 Version du protocole et compatibilité

Chaque réponse du SDK DOIT inclure le header :

```
X-DevEye-Protocol-Version: 1.0
```

La valeur est `MAJEURE.MINEURE`. **Ce document décrit le protocole `1.0`.** Le numéro ne change que pour une rupture ; les ajouts additifs (route de commande, capacité, champ) sont annoncés par `capabilities` ([ADR-0068](adr/0068-capacites-additives-et-version-du-protocole.md)).

**Règle d'incrément**

| Changement | Incrément |
|---|---|
| Retirer ou renommer une route, un champ, un code de statut ; changer le type ou la sémantique d'un champ ; rendre un champ de requête optionnel obligatoire | **majeure** (`2.0`) |
| Ajouter une route, un champ de réponse, un champ **optionnel** de requête, une capacité | aucun : la version reste `1.0`, la capacité se découvre par `capabilities` (§3.6) |
| Corriger un comportement sans changer le contrat écrit | aucun |

Les clients ignorent les champs de réponse inconnus ; un SDK `1.x` reste donc utilisable par un client `1.y`.

**Comportement du client** (`deveye-sdk-client`, appelé par les adapters après chaque réponse) :

| Cas | Comportement |
|---|---|
| Majeure et mineure identiques à celles du client | silencieux |
| En-tête absent, illisible (dont l'ancienne valeur `1` sans mineure) ou mineure différente | `tracing::warn!` **une seule fois par processus**, la requête continue |
| Majeure différente | erreur `PortError::Protocol` explicite (versions des deux côtés, action à mener) ; aucune requête n'est exploitée |

Un SDK tiers DOIT donc émettre `1.0` sous la forme `MAJEURE.MINEURE`.

**État** : émis par l'implémentation de référence. Le middleware `add_protocol_version` est posé en **couche externe** de la garde Host, de sorte que *chaque* réponse porte le header — y compris le `403` de `guard_local_host` et le `413` de la limite de corps. Ref. code : `deveye-sdk/src/lib.rs` (`PROTOCOL_VERSION`, `add_protocol_version`) ; client : `deveye-sdk-client/src/lib.rs` (`check_protocol_version`).

### 2.4 Masquage des secrets (périmètre exact)

Le SDK remplace par `"<secret>"` (constante `deveye_sdk::REDACTED`) la valeur de toute clé JSON jugée sensible, à toute profondeur, **au moment de servir** :

- `GET /devtools/state` : les entrées de premier niveau (le nom passé à `set_state`) et leurs sous-objets ;
- `GET /devtools/logs/poll` : les champs structurés (`fields`) de chaque événement.

Clés masquées par défaut, comparées sans casse ni séparateurs (`api_key` = `apiKey` = `API-KEY`) : toute clé **contenant** `password`, `passwd`, `secret`, `apikey`, `privatekey`, `credential`, `authorization`, `accesskey`, `signingkey`, `encryptionkey`, `databaseurl`, `connectionstring`, `motdepasse`, `mdp` ou `jeton` ; toute clé **finissant** par `token`, `cookie` ou `sessionid` ; les clés exactes `pwd`, `auth`, `jwt`, `bearer`, `otp`, `tokens`, `cookies`. `token_count`, `max_tokens` ou `cookie_banner` ne sont pas masquées. L'app ajoute ses propres clés par `Config::secret_keys` (correspondance exacte sur la clé entière). Une clé sensible remplace son sous-arbre entier.

**Ce qui n'est PAS filtré** :

- le résultat de `POST /devtools/eval` ;
- les valeurs de retour de `POST /devtools/command` (la route `ui.widgets` fait exception **pour le SDK Rust** : la valeur d'un widget secret n'y est jamais publiée, §3.7 ; une implémentation tierce doit en faire autant, le serveur masque seulement en seconde ligne, dans `dom` et `invoke_command`) ;
- le texte libre des messages de log (`message`) ;
- les champs de log formatés par `Debug` (`info!(user = ?user)`) : la valeur arrive en chaîne et seul le **nom** du champ est examiné, pas le contenu (`"User { password: … }"` passe tel quel) ; préférer des champs structurés scalaires ;
- tout secret stocké sous un nom de clé neutre non déclaré.

Le masquage est un filet par noms de clés, pas une garantie : une app qui manipule des secrets ne doit pas les passer à `set_state` ni les journaliser. Décision : [ADR-0016](adr/0016-masquage-des-secrets-par-cles-dans-le-sdk.md). Un SDK tiers n'est pas tenu d'implémenter le masquage, mais doit alors documenter la limite. Il n'annonce la capacité `redaction` (§3.6) **que** s'il masque réellement selon les règles ci-dessus : la suite de conformité contrôle alors l'état et les logs servis (une clé sensible dont la valeur n'est pas `"<secret>"` est un écart).

### 2.5 Limite de taille des requêtes

Le corps d'une requête est limité à **1 Mio** (1 048 576 octets, `MAX_REQUEST_BODY_BYTES`). Au-delà, le SDK DOIT répondre `413 Payload Too Large` sans traiter la route (header de version présent). Le `413` n'a pas de corps JSON normalisé (un client ne doit pas le décoder) et le serveur peut fermer la connexion avant d'avoir lu tout le corps : un client doit tolérer une connexion coupée pendant l'envoi.

### 2.6 CORS : refus de toute origine

Le SDK n'autorise **aucune** origine cross-origin. Il NE DOIT PAS émettre `Access-Control-Allow-Origin` ni `Access-Control-Allow-Credentials`, que la requête soit une requête simple portant un `Origin` ou un préflight `OPTIONS` (`Access-Control-Request-Method`). Cela interdit à une page web tierce de lire les réponses ou d'émettre des requêtes « non simples » (`Content-Type: application/json`) vers le port local via le navigateur de l'utilisateur. Le serveur répond au préflight sans traiter la route (l'implémentation de référence renvoie `200` sans en-tête d'autorisation) ; le header de version est présent sur cette réponse comme sur toutes les autres. Les clients des adapters ne passent pas par un navigateur : CORS ne les concerne pas.

La garde Host (§2.2) reste la protection principale contre le DNS-rebinding ; CORS en est la seconde barrière.

---

## 3. Endpoints

Le SDK expose six routes. Toutes les réponses `200` retournent `Content-Type: application/json`.

### 3.0 Conventions communes

**Codes d'erreur transverses.** Hors des cas propres à chaque route :

| Situation | Statut | Corps |
|---|---|---|
| `Host` absent (`400` admis s'il est refusé par la pile HTTP), étranger ou mal formé (§2.2) | `403` | `{ "error": "<message>" }` |
| Chemin inconnu (`/`, `/devtools/inconnue`, `/devtools/state/extra`) | `404` | libre (ne pas le décoder) |
| Chemin connu, méthode non prévue (`POST /devtools/state`, `GET /devtools/eval`) | `405` | libre ; un header `Allow` est recommandé |
| Corps `POST` qui n'est pas du JSON valide, `Content-Type` incorrect, champ obligatoire absent ou de mauvais type | `4xx` autre que `403`, `404`, `405` | libre |
| Corps au-delà de 1 Mio (§2.5) | `413` | libre |
| Défaillance interne du SDK | `5xx` | libre, `{ "error": "<message>" }` recommandé |

Pour les erreurs de décodage, l'implémentation de référence répond `400` (JSON syntaxiquement invalide), `415` (`Content-Type` absent ou autre que `application/json`) ou `422` (JSON valide mais champ obligatoire absent ou mal typé). Une implémentation tierce peut choisir un autre `4xx` ; la suite n'exige que « client en erreur, ni garde, ni route, ni méthode ». **Chaque** réponse, erreurs et préflight compris, porte `X-DevEye-Protocol-Version`.

**Corps de requête JSON.** Encodés en UTF-8, `Content-Type: application/json`. Les champs inconnus sont ignorés (ils pourront être ajoutés en `1.x`). Un champ optionnel **absent** et un champ à `null` sont équivalents.

**Horodatages.** Toute date est une chaîne RFC 3339 en **UTC**, suffixée `Z` (jamais `+00:00` ni un autre décalage), avec une fraction de seconde de 0 à 9 chiffres : `2026-06-24T14:32:01.123456789Z`, `2026-06-24T14:32:01Z`. Un client lit ces valeurs avec un analyseur RFC 3339 standard.

**Ordres de tri.** `GET /devtools/commands` est trié par `name`, ordre croissant des octets UTF-8 (`ping` avant `set-page`, `Z` avant `a`) : l'ordre est normatif. Les clés de `entries` (`GET /devtools/state`) sont triées de la même façon dans l'implémentation de référence ; c'est recommandé, pas exigé (un objet JSON n'est pas ordonné). Les événements de `GET /devtools/logs/poll` sont rendus dans leur **ordre d'émission**, donc à horodatage croissant.

**Valeurs absentes.** Un champ marqué « absent si… » n'est pas émis (pas de `null` de remplacement) ; les lecteurs tolèrent néanmoins `null`. Un tableau ou un objet vide est une valeur valide, jamais une erreur.

**Cohérence des identifiants.** Les noms de capacités, de routes de commande et de champs sont sensibles à la casse.

### 3.1 `GET /devtools/state`

**Objectif** : obtenir un snapshot instantané du state applicatif publié par l'app cible.

**Requête** : aucun corps, aucun paramètre de query. Le poll étant destructif, un SDK refuse (`403`) la requête qui porte un en-tête `Origin` ou un `Sec-Fetch-Site` différent de `none` : un navigateur, y compris en `no-cors`, ne doit pas pouvoir vider le buffer ([ADR-0055](adr/0055-refus-des-requetes-inter-origine-sur-le-drain-des-logs.md)). Les clients non navigateur n'envoient aucun de ces en-têtes.

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
| `timestamp` | string | oui | RFC 3339 en UTC, suffixé `Z` (§3.0) |
| `fields` | objet JSON | non | Absent si aucune paire `key=value` (les lecteurs tolèrent `null`). Ref. attribut `#[serde(default, skip_serializing_if = "serde_json::Value::is_null")]` sur le champ `fields` de `LogEvent` |

**Codes HTTP** :

| Code | Cas |
|---|---|
| 200 | Succès (même si `events` est vide) |
| 403 | Header `Host` invalide |
| 5xx | Défaillance interne du SDK |

**Sémantique de drain** (critique) :

Le poll est **destructif**. L'appel `GET /devtools/logs/poll` vide atomiquement le buffer interne et retourne les événements accumulés depuis le dernier poll. Un second appel immédiat retourne `{ "events": [] }`. Il n'y a pas de curseur, pas d'acquittement, pas de replay.

Ref. code : `deveye-sdk/src/lib.rs` — handler `logs_poll_handler` : `guard.drain(..)` vide le `VecDeque` en une seule opération sous le verrou :

```rust
let events: Vec<_> = registry
    .logs
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner)
    .drain(..)
    .collect();
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

Note : le code HTTP est `200` dans les deux cas quand l'expression a été transmise et exécutée. C'est la présence de `error` qui signale l'échec applicatif. Une réponse `200` porte **exactement un** des deux champs ; une expression qui vaut `null` donne `{ "value": null }` (`value` est présent même nul).

Les réponses `503`, `504` et `500` portent un corps `{ "error": "<message>" }`.

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

**Réponse 404** (corps minimal : la route n'est pas répétée, elle est déjà portée par `PortError::Route` côté client). Un `payload` absent ou `null` est valide : une route inconnue répond `404` dans les deux cas, jamais `400` ni `422` :

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
| 4xx | Corps invalide ou champ `route` absent (§3.0) |
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

### 3.6 `GET /devtools/info`

**Objectif** : identifier le SDK et découvrir ses capacités (ajoutée en `1.0`, additive).

**Requête** : aucun corps.

**Réponse 200** :

```json
{
  "sdk_version": "1.0.0",
  "protocol_version": "1.0",
  "capabilities": ["state", "commands", "info", "redaction", "logs", "eval"]
}
```

| Champ | Type | Description |
|---|---|---|
| `sdk_version` | string | Version du SDK (crate `deveye-sdk` ou implémentation tierce) |
| `protocol_version` | string | Version du protocole, `MAJEURE.MINEURE`, identique au header |
| `capabilities` | tableau de string | Capacités **actives** : `state`, `commands` et `info` toujours ; `redaction` quand le masquage des secrets (§2.4) est actif ; `logs` quand une source de logs est armée (abonné ou `layer()` installé, ou premier appel de `log`) ; `ui.widgets/1` quand l'app publie un arbre de widgets (§3.7) ; `eval` quand une pompe d'évaluation est enregistrée. `eval` est cohérente avec `POST /devtools/eval` : annoncée, la route répond `200` ; non annoncée, `503`. De nouvelles valeurs peuvent apparaître en `1.x` : les ignorer si inconnues |

**Codes HTTP** : 200 ; 403 (Host invalide).

Ref. code : `deveye-sdk/src/lib.rs` (`info_handler`). Côté client : `deveye_sdk_client::fetch_info` (`SdkInfo`) ; un `404` y est traduit en `PortError::Protocol` (SDK antérieur à la 1.0).

---

### 3.7 Route de commande `ui.widgets` (`deveye.ui/1`)

**Objectif** : servir l'arbre de widgets d'une app sans DOM (egui, imgui, Skia, Godot, Uno…). C'est une **route de commande** (§3.4), pas un endpoint : `POST /devtools/command { "route": "ui.widgets", "payload": {} }`. Ajoutée en `1.1`, additive ; une implémentation qui ne la sert pas n'annonce pas la capacité.

**Capacité** : `ui.widgets/1`, annoncée par `GET /devtools/info` (§3.6) dès que l'app a recours au helper. Une capacité nouvelle est un ajout additif : `protocol_version` reste `1.0` ([ADR-0068](adr/0068-capacites-additives-et-version-du-protocole.md)). « Ajoutée en 1.1 » désigne la version du SDK, pas celle du protocole.

**Requête** : `payload` vaut `{}`. Un handler ignore son payload ; la négociation (`accept_schema`, `max_nodes`) n'est pas engagée tant que `/devtools/info` n'en annonce pas la prise en charge ([ADR-0065](adr/0065-helper-sdk-ui-lecture-serveur-et-controles.md)).

**Réponse 200** : un arbre au schéma `deveye.ui/1` ([`docs/schemas/ui-widgets-1.json`](schemas/ui-widgets-1.json), forme et exemple dans [usage.md](usage.md)). Une app sans arbre publié répond `200 { "error": "..." }` : l'écart est dit, pas masqué.

**Exigences** (vérifiées par `UI-01` et `UI-02`, §7) :

- `schema` vaut `deveye.ui/1` ; les `id` sont uniques et non vides ; chaque `parent` et le `focus` désignent un widget de l'arbre ; aucun cycle ;
- un widget `state.secret` (ou de rôle `password`) ne porte **aucune valeur** : `value` est absent. Le serveur masque en seconde ligne (`"<secret>"`) la valeur d'un tel widget reçue d'une app tierce, ainsi que les clés sensibles (§2.4) des valeurs structurées, y compris dans la sortie brute de l'outil MCP `dom` ([ADR-0064](adr/0064-masquage-des-secrets-transversal-et-vecteurs-partages.md)) ;
- `published_at_ms` (ms depuis l'epoch Unix) date l'arbre : le serveur signale un arbre vieux de plus de 2 s.

Ref. code : `deveye-sdk/src/ui.rs` (`publish`, `is_observed`, route `ui.widgets` enregistrée au premier usage) ; lecture : `adapter-composite` (`RouteDomInspector`). Types : `deveye-core::ui`.

---

## 4. Garanties de l'implémentation de référence

`deveye-sdk` implémente le contrat complet de ce document :

| Fonctionnalité | Statut |
|---|---|
| `GET /devtools/state`, `GET /devtools/logs/poll`, `POST /devtools/eval`, `GET /devtools/info` | oui |
| `POST /devtools/command` (`command_handler`) et `GET /devtools/commands` (`commands_handler`, tri par `name`) | oui |
| Garde `Host` (`guard_local_host`, middleware, `403` hors localhost) | oui |
| Header `X-DevEye-Protocol-Version: 1.0` (toutes réponses, y compris `403` et `413`) | oui |
| Masquage des secrets par clés (§2.4) | oui |
| Limite de corps 1 Mio, `413` (§2.5) | oui |
| `expose_route` (enregistrement de commandes) | oui |
| Route `ui.widgets` et capacité `ui.widgets/1` (`deveye_sdk::ui`, §3.7) | oui |
| `504` sur dépassement de l'évaluation, traduit en `PortError::Timeout` côté `adapter-dioxus-signals` | oui |

Un runtime tokio dédié (`std::sync::LazyLock`, un thread) isole le SDK du runtime de l'app hôte. Les verrous sont **mixtes** : `state` et `routes` sont des `std::sync::RwLock`, afin que `set_state` et `expose_route` écrivent immédiatement, sans runtime, depuis n'importe quel thread ; le buffer de logs (`logs`) est un `std::sync::Mutex` dont les sections critiques sont minuscules et jamais tenues à travers un `.await` : un push de log doit marcher de n'importe quel fil, y compris hors de tout runtime, où `blocking_lock` d'un verrou tokio paniquerait ([ADR-0066](adr/0066-push-de-logs-et-layer-du-sdk.md)) ; l'émetteur de la pompe d'évaluation (`eval_tx`, `tokio::sync::RwLock`) est un verrou `tokio::sync`, car les handlers l'attendent. Une implémentation tierce n'a pas à reproduire ce choix : seul le comportement observable est normatif.

`deveye-sdk` est l'**implémentation de référence** : en cas de divergence entre ce document et le code de `deveye-sdk`, c'est un défaut à corriger d'un côté ou de l'autre, et la suite de conformité (section 7) départage en exécutant le SDK Rust à chaque modification.

---

## 5. Tableau récapitulatif des endpoints

| Méthode | Chemin | Destructif | Corps requête | Corps réponse | Timeout client réel |
|---|---|---|---|---|---|
| GET | `/devtools/state` | non | aucun | `{ "entries": {...} }` | 15 s (connexion 3 s) |
| GET | `/devtools/logs/poll` | **oui** (drain) | aucun | `{ "events": [...] }` | 15 s (connexion 3 s) |
| POST | `/devtools/eval` | non | `{ "expression": "..." }` | `{ "value": ... }` ou `{ "error": "..." }` | 15 s (connexion 3 s) |
| POST | `/devtools/command` | selon commande | `{ "route": "...", "payload": ... }` | JSON libre | 15 s (connexion 3 s) |
| GET | `/devtools/commands` | non | aucun | `[{ "name": "...", "description": "..." }]` | 15 s (connexion 3 s) |
| GET | `/devtools/info` | non | aucun | `{ "sdk_version", "protocol_version", "capabilities" }` | 15 s (connexion 3 s) |

Les six endpoints partagent le **même** `reqwest::Client`, construit une fois par `deveye_sdk_client::client()` (`CONNECT_TIMEOUT = 3 s`, `REQUEST_TIMEOUT = 15 s`, ref. `deveye-sdk-client/src/lib.rs`) — aucun timeout n'est configuré par endpoint ni par adapter. Il n'existe pas de constante `SDK_HTTP_TIMEOUT`.

Note sur le timeout de `/devtools/eval` : le SDK impose en plus un timeout **interne** de 10 s (ref. `deveye-sdk/src/lib.rs`, `tokio::time::timeout(Duration::from_secs(10), ...)`) avant de répondre `504`. Le `REQUEST_TIMEOUT` de 15 s côté client laisse une marge suffisante (5 s) pour le réseau loopback et la sérialisation sans déclencher de double-timeout. Côté client Rust, un `504` est distingué par `PortError::Timeout` (cf. `adapter-dioxus-signals/src/bridge.rs`).

---

## 6. Checklist : implémenter un SDK conforme

Cette liste est le guide opérationnel pour tout SDK dans un langage tiers (par exemple .NET).

### Prérequis

- [ ] Le SDK n'est actif **qu'en mode debug** (directive de compilation, pas une variable d'environnement). En C# : entourer l'initialisation de `#if DEBUG ... #endif`.
- [ ] Choisir une bibliothèque HTTP embarquable légère (ex. `Microsoft.AspNetCore` minimal API, ou `EmbedIO`, ou `HttpListener` natif .NET).

### Bind et port

- [ ] Binder exclusivement sur `127.0.0.1:<port>` — jamais `0.0.0.0`.
- [ ] Lire `DEVEYE_PORT` en variable d'environnement au démarrage ; utiliser `31337` si absente ou invalide (entier décimal de 1 à 65535 exigé, §1).
- [ ] En cas d'échec de bind (port déjà pris), loguer en debug et continuer sans paniquer l'app hôte.

### Garde Host (obligatoire)

- [ ] Implémenter un middleware (ou filtre de requête) qui vérifie le header `Host` avant toute route.
- [ ] Accepter : `localhost` (insensible à la casse), `localhost:<port>`, `127.0.0.1`, `127.0.0.1:<port>`, `[::1]`, `[::1]:<port>`.
- [ ] Rejeter tout autre `Host` (y compris absent, `::1` sans crochets, `[::1]toto`, port non numérique) avec `403 Forbidden` (un `Host` absent peut aussi recevoir le `400` de la pile HTTP, §2.2) et un corps `{ "error": "..." }`, sans traiter la route ni décoder le corps.
- [ ] Ne jamais émettre `Access-Control-Allow-Origin` ni `Access-Control-Allow-Credentials` (§2.6).
- [ ] Répondre `404` aux chemins inconnus, `405` aux méthodes non prévues, un `4xx` aux corps JSON invalides ou incomplets (§3.0), chaque fois avec le header de version.
- [ ] Émettre les horodatages en RFC 3339 UTC suffixé `Z` (§3.0).
- [ ] Inclure `X-DevEye-Protocol-Version: 1.0` (forme `MAJEURE.MINEURE`) dans toutes les réponses.
- [ ] Limiter le corps des requêtes (1 Mio recommandé) et répondre `413` au-delà.
- [ ] Masquer les valeurs des clés sensibles (§2.4) dans l'état et les champs de logs, ou documenter l'absence de masquage.

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

### Route `ui.widgets` (facultative)

- [ ] Si l'app a une interface sans DOM : servir l'arbre `deveye.ui/1` (§3.7) sur la route de commande `ui.widgets`, ignorer le payload, annoncer `ui.widgets/1` dans `capabilities` **seulement** alors.
- [ ] Ne jamais publier la valeur d'un widget secret.

### `GET /devtools/info`

- [ ] Retourner `{ "sdk_version", "protocol_version": "1.0", "capabilities": [...] }` avec 200.

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
- [ ] Vérifier que le header `X-DevEye-Protocol-Version: 1.0` est présent dans chaque réponse.
- [ ] Vérifier que `GET /devtools/info` annonce `protocol_version` `1.0`.
- [ ] **Lancer la suite de conformité** (section 7) : `deveye-conformance --url http://127.0.0.1:<port>` doit conclure « CONFORME » sans échec. Les `curl` ci-dessus restent utiles pour diagnostiquer un écart, la suite les remplace comme preuve.

---

## 7. Valider une implémentation : la suite de conformité

`deveye-conformance` (crate du dépôt, binaire du même nom) est une suite **boîte noire** : elle ne connaît de l'implémentation que son URL et compare ses réponses à ce document. Elle valide indifféremment le SDK Rust, un shim dans un autre langage ou toute implémentation tierce ; le test `deveye-conformance/tests/sdk_conformance.rs` démarre le SDK Rust en processus sur un port éphémère et la lui fait passer à chaque `cargo test --workspace` **en build debug** (le SDK est un no-op en release : le test n'y valide rien).

### Mode d'emploi

1. Lancer l'app (ou le shim) en mode debug, avec son serveur DevEye sur un port connu, par exemple `DEVEYE_PORT=31337`.
2. Lancer la suite contre cette URL, depuis le dépôt DevEye :

```bash
cargo run -p deveye-conformance -- --url http://127.0.0.1:31337
# ou, via la recette du dépôt :
just conformance http://127.0.0.1:31337
```

Exemple pour un shim .NET (processus indépendant, démarré avant) : la suite ne touche à aucun fichier du shim, elle lui parle en HTTP.

3. Lire le rapport : une ligne par vérification (`OK`, `FAIL`, `SKIP`, `N/A`), l'écart expliqué sous chaque `FAIL`, puis le bilan et le verdict : `CONFORME`, `NON CONFORME`, ou `AUCUN ÉCART OBSERVÉ, MAIS INCOMPLET` quand des vérifications sont restées non observables. **Code de sortie** : `0` aucun `FAIL`, `1` au moins un écart (ou, avec `--strict`, au moins un `SKIP`), `2` suite inexécutable (URL invalide, implémentation injoignable). Le rapport ne cite jamais le contenu des réponses, seulement leur taille.

### Options

| Option | Effet |
|---|---|
| `--url <URL>` | Base de l'implémentation (défaut `http://127.0.0.1:31337`) ; `http://` uniquement |
| `--invoke-route <nom>` | Invoque cette commande (sans effet de bord) et exige `200` + JSON |
| `--slow-expression <js>` | Expression que la pompe d'évaluation laisse sans réponse : exige `504` (dure 10 s) |
| `--abandon-expression <js>` | Expression que la pompe abandonne : exige `500` |
| `--strict` | Un `SKIP` (non observable) fait aussi échouer ; un `N/A` (capacité non annoncée) non |

### Ce que la suite vérifie

Une vérification est `SKIP`, jamais `OK` par défaut, quand l'état de l'implémentation ne permet pas de l'observer (aucun log émis, moins de deux commandes, pas de clé sensible, option absente, connexion coupée avant le `413`). Elle est `N/A` quand l'exigence ne s'applique pas à l'implémentation (capacité `eval`, `redaction` ou `ui.widgets/1` non annoncée) : rien n'est à observer et `--strict` ne l'exige pas. Pour tout observer : poser `set_state("password", ...)`, enregistrer au moins deux commandes, publier un arbre `ui.widgets` avec un champ de rôle `password`, émettre au moins deux logs avant de lancer la suite, et fournir les options ci-dessus.

| Identifiant | Section | Exigence |
|---|---|---|
| `ROUTE-01` à `ROUTE-04` | §3.1, §3.6, §3.5, §3.2 | formes de `state`, `info`, `commands`, `logs/poll`, Content-Type JSON |
| `VERS-01` | §2.3 | header de version `MAJEURE.MINEURE` (majeure `1`) sur chaque route |
| `HOST-01` à `HOST-04` | §2.2 | 7 hôtes acceptés, 13 rejetés, `Host` absent (`403`, ou `400` de la pile HTTP), garde avant routage, corps `{error}` |
| `ERR-01` à `ERR-04` | §3.0 | `404`, `405`, JSON invalide, champ obligatoire absent, avec header de version |
| `SIZE-01` | §2.5 | `413` au-delà de 1 Mio (`SKIP` si le serveur coupe la connexion avant, ce que §2.5 permet) |
| `CORS-01`, `CORS-02` | §2.6 | aucune autorisation cross-origin (préflight et requête simple) |
| `CMD-01` à `CMD-03` | §3.4, §3.5 | commande inconnue `404 {error}`, payload absent ou `null`, tri, invocation `200` |
| `LOGS-01` à `LOGS-03` | §3.0, §3.2 | horodatages RFC 3339 UTC, ordre d'émission, poll destructif |
| `EVAL-01` à `EVAL-03` | §3.3 | `503`/`200` selon la capacité `eval`, `504`, `500` |
| `SEC-01` | §2.4 | masquage des clés sensibles, si la capacité `redaction` est annoncée (sinon `N/A`) |
| `UI-01`, `UI-02` | §3.7 | si la capacité `ui.widgets/1` est annoncée (sinon `N/A`) : arbre `deveye.ui/1` valide (schéma, ids uniques, parents connus), aucune valeur publiée pour un widget secret (`SKIP` sans widget secret) |

Les en-têtes de version sur `200`, `403`, `404`, `405`, `413`, `500` et les erreurs de décodage sont contrôlés dans chacune de ces vérifications. Ce qui n'est **pas** vérifié : le comportement interne des handlers de commande, le contenu des résultats d'évaluation, la sécurité compile-time du §2.1 (`#if DEBUG`), le comportement en cas de bind raté et le masquage par clés ajoutées par l'app (`Config::secret_keys`).

### Intégration continue

Le dépôt exécute la suite contre le SDK Rust via `cargo test --workspace` (job de tests de la CI). Pour une implémentation tierce, la suite est lancée par le dépôt de cette implémentation : cloner DevEye à une version étiquetée, `cargo run -p deveye-conformance -- --url ... --strict`, et échouer sur un code de sortie non nul. Le dépôt de l'implémentation tierce n'est jamais modifié par DevEye.

---

## 8. Schémas JSON

Les corps normatifs sont décrits par des JSON Schema (brouillon 2020-12), versionnés avec le protocole dans `docs/schemas/1.0/`. Un `1.x` additif ajoute des champs optionnels à ces schémas sans en retirer ; une `2.0` crée `docs/schemas/2.0/`. Les schémas autorisent les propriétés inconnues (`additionalProperties` non fermé) : la règle du §2.3 veut que les clients les ignorent.

| Schéma | Corps décrit |
|---|---|
| [`state.json`](schemas/1.0/state.json) | réponse `GET /devtools/state` |
| [`info.json`](schemas/1.0/info.json) | réponse `GET /devtools/info` |
| [`commands.json`](schemas/1.0/commands.json) | réponse `GET /devtools/commands` |
| [`logs-poll.json`](schemas/1.0/logs-poll.json) | réponse `GET /devtools/logs/poll` (événements de log) |
| [`eval-request.json`](schemas/1.0/eval-request.json) | requête `POST /devtools/eval` |
| [`eval-response.json`](schemas/1.0/eval-response.json) | réponse `POST /devtools/eval` (`200`, `500`, `503`, `504`) |
| [`command-request.json`](schemas/1.0/command-request.json) | requête `POST /devtools/command` |
| [`error.json`](schemas/1.0/error.json) | corps d'erreur `{ "error": "..." }` (`403`, `404` de commande) |

Les schémas décrivent la **forme** ; les règles de comportement (codes, tri, drain, garde Host) restent dans le texte et dans la suite de conformité. La réponse `200` de `POST /devtools/command` n'a pas de schéma : c'est la valeur libre du handler de l'app.
