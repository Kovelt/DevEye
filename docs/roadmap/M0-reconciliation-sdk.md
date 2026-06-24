# M0 — Réconcilier le SDK canonique et figer le wire protocol

## Objectif

Backporter dans `deveye-sdk` (canonique) les pièces manquantes que les copies vendored d'Atelier et Chronos ont introduites indépendamment — en particulier `POST /devtools/eval`, `EvalRequest` et `take_eval_requests()` — afin que `adapter-dioxus-signals/src/bridge.rs` puisse appeler le SDK canonical sans erreur réseau. Ajouter `PortError::Timeout` au type d'erreur partagé, resynchroniser les vendored sur le canonique, et corriger les inexactitudes de `docs/architecture.md` et `docs/usage.md`.

**Effort** : M (3-5 jours)
**Dépend de** : rien — prérequis de tous les jalons suivants
**Statut décision** : tranché — wire protocol figé tôt (décision Morgan)

Le livrable `docs/wire-protocol.md` est traité dans une page de documentation séparée ; ce jalon y renvoie.

---

## Pourquoi (verrou levé)

`adapter-dioxus-signals/src/bridge.rs` appelle `POST /devtools/eval` à la ligne 50 :

```rust
// deveye-adapters/adapter-dioxus-signals/src/bridge.rs:50
let url = format!("{}/devtools/eval", self.base_url.trim_end_matches('/'));
```

Or le routeur axum du SDK canonique (`deveye-sdk/src/lib.rs`, lignes 141-147) n'enregistre que quatre routes :

```rust
// deveye-sdk/src/lib.rs:141-147
let app = Router::new()
    .route("/devtools/state", get(state_handler))
    .route("/devtools/logs/poll", get(logs_poll_handler))
    .route("/devtools/command", post(command_handler))
    .route("/devtools/commands", get(commands_handler))
    .layer(middleware::from_fn(guard_local_host))
    .with_state(registry);
```

`POST /devtools/eval` est absente. Toute session `--target sdk` ou `--target auto` en repli sur le pont natif renvoie donc un `404` sur toute tentative d'évaluation JS, rendant `eval_js`, `click_selector`, `click_text`, `dom` et `navigate` totalement inopérants.

Les vendored Atelier (`Atelier/deveye/deveye-sdk/src/lib.rs`, lignes 148-149 et 191-274) et Chronos (`Chronos/crates/deveye/deveye-sdk/src/lib.rs`, lignes 148-149 et 191-274) ont indépendamment résolu ce problème, mais en divergeant du canonique sur deux points architecturaux majeurs :

- Elles utilisent `once_cell::sync::Lazy` (ligne 116 dans chacune) et des primitives tokio async (`RwLock`, `Mutex` de `tokio::sync`) là où le canonique utilise `std::sync::LazyLock` (ligne 129) et des primitives std sync — deux designs de `Registry` incompatibles.
- Elles embarquent un `runtime()` tokio dédié (lignes 125-135) ; le canonique rely sur le runtime de l'app hôte via `tokio::spawn`.
- Aucune des deux vendored n'a le middleware `guard_local_host` présent dans le canonique (lignes 173-185) : la protection anti DNS-rebinding est absente sur le nouvel endpoint `/devtools/eval` des vendored.

Par ailleurs, `PortError` (`deveye-ports/src/errors.rs`) ne contient pas de variante `Timeout` : le handler `eval_handler` des vendored retourne `StatusCode::GATEWAY_TIMEOUT` (ligne 266 dans chacune), mais le code appelant dans `bridge.rs` ne peut pas distinguer un timeout d'une erreur réseau — les deux tombent dans `PortError::Network`.

Enfin, `docs/architecture.md` décrit le trait `LogSource` avec une signature `fn stream(&self) -> BoxStream<'static, LogEvent>` (ligne 174) qui ne correspond pas à la signature réelle de `deveye-ports/src/log_source.rs` (ligne 24-28, `async fn poll(&self) -> PortResult<Vec<LogEvent>>`), liste l'adapter `adapter-commands-http` dans le texte (ligne 191) mais pas dans l'arborescence du workspace (lignes 87-94), mentionne le « Reconnect du WebSocket browser » comme fonctionnel dans `usage.md` (ligne 189) mais écrit encore « Adapter Linux hors périmètre » dans `architecture.md` (ligne 261) ce qui est faux depuis que le pont natif existe.

---

## État actuel du code

**SDK canonique** (`deveye-sdk/src/lib.rs`) :
- Routes exposées (lignes 141-147) : `GET /devtools/state`, `GET /devtools/logs/poll`, `POST /devtools/command`, `GET /devtools/commands`. `POST /devtools/eval` absente.
- `Registry` (lignes 104-127) : champs `state: std::sync::RwLock`, `logs: std::sync::Mutex`, `routes: std::sync::RwLock`. Pas de champ `eval_tx`.
- `REGISTRY` (ligne 129-130) : `std::sync::LazyLock<Arc<Registry>>`.
- Middleware `guard_local_host` présent et branché sur toutes les routes existantes (lignes 145-146 et 173-197).
- `Cargo.toml` canonique : `tokio` avec features `sync, rt, net, macros` — pas `rt-multi-thread`, pas `time`. Pas de `once_cell`.

**Vendored Atelier** (`Atelier/deveye/deveye-sdk/src/lib.rs`) :
- `EvalRequest` (lignes 89-95) : `expression: String`, `respond: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>`.
- `Registry.eval_tx` (ligne 103) : `RwLock<Option<tokio::sync::mpsc::UnboundedSender<EvalRequest>>>` (tokio RwLock).
- `take_eval_requests()` (lignes 191-198) : crée un canal `unbounded_channel`, pose l'émetteur dans `registry.eval_tx`, retourne le récepteur.
- `eval_handler` (lignes 215-274) : `POST /devtools/eval`, corps `{ expression: String }`, timeout 10 s (`tokio::time::timeout`), répond `{ value? }` ou `{ error? }`, `503` si pompe absente, `504` si timeout.
- `init()` (lignes 142-161) : utilise `runtime().spawn()` — pas `tokio::spawn` directement.
- `REGISTRY` (ligne 116-117) : `once_cell::sync::Lazy`.
- Middleware `guard_local_host` : absent.

**Vendored Chronos** (`Chronos/crates/deveye/deveye-sdk/src/lib.rs`) : contenu identique au vendored Atelier (mêmes lignes, même design, même absence de `guard_local_host`).

**Adapter** (`adapter-dioxus-signals/src/bridge.rs`) :
- `SdkEvaluator::evaluate` (lignes 49-73) : appelle `POST /devtools/eval`, décode `EvalResponse { value?, error? }`, mappe `503` en `PortError::Network` et les autres erreurs JS en `PortError::Render`. Pas de distinction `Timeout`.

**Erreurs** (`deveye-ports/src/errors.rs`, lignes 1-32) :
- Variantes présentes : `Render`, `Inspect`, `Interaction`, `Log`, `Route`, `Network`, `Unsupported`. `Timeout` absente.

**Documentation** (`docs/architecture.md`) :
- Ligne 174 : signature `LogSource` affichée `fn stream(&self) -> BoxStream<'static, LogEvent>` — incorrecte.
- Lignes 87-94 : arborescence du workspace liste 5 adapters, omet `adapter-commands-http`.
- Ligne 191 : mentionne `adapter-commands-http` dans le texte mais sans le référencer dans la structure.
- Ligne 261 : « Adapter Linux (WebKitGTK inspector) » listé comme hors périmètre — faux : le pont natif (`adapter-dioxus-signals/bridge.rs`) est opéré en production depuis M0.
- Aucune mention de `LazyCdp` ni de `--target` dans la description de robustesse CDP (section lignes 218-233) — `usage.md` ligne 189 l'évoque mais `architecture.md` reste muet.

**Documentation** (`docs/usage.md`) :
- Lignes 159-161 : commandes de vérification `findstr` et chemin `.exe` — Windows uniquement, incompatibles Linux.
- Pas de référence à `adapter-commands-http` comme crate distinct.

---

## Changements détaillés

### `deveye-sdk/Cargo.toml`

Ajouter les features tokio manquantes (`rt-multi-thread`, `time`) et la dépendance `once_cell` pour aligner le modèle de runtime sur les vendored :

```toml
# Avant
tokio = { workspace = true, features = ["sync", "rt", "net", "macros"] }

# Apres
tokio = { workspace = true, features = ["sync", "rt", "rt-multi-thread", "net", "macros", "time"] }
once_cell = "1"
```

Motif : `rt-multi-thread` est requis par le `Builder::new_multi_thread()` du runtime dédié ; `time` est requis par `tokio::time::timeout` dans `eval_handler`.

### `deveye-sdk/src/lib.rs`

Cinq ajouts, un remplacement, une extension du `Router` :

**1. Import `once_cell` et primitives tokio async**

Remplacer les imports `std::sync` dans `Registry` par les primitives tokio (pour que `eval_tx` puisse être attendu depuis des contextes async), en alignement avec les vendored. Conserver `std::sync::LazyLock` pour `REGISTRY` (disponible depuis Rust 1.80, préférée à `once_cell` dans le code canonique) mais migrer les champs de `Registry` :

```rust
// Remplacer (lignes 30-31 actuelles)
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

// Par
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{Mutex as TokioMutex, RwLock as TokioRwLock};
```

**2. Struct `EvalRequest` (nouveau)**

À insérer après la définition de `CommandRequest` (après la ligne 91 actuelle) :

```rust
/// Une demande d'evaluation JS poussee par `POST /devtools/eval` et
/// consommee par la **pompe** cote app (cf. [`take_eval_requests`]). L'app
/// execute l'expression dans son webview (`document::eval` chez Dioxus) et
/// repond par le `oneshot`.
pub struct EvalRequest {
    /// Expression JS (une seule expression — typiquement une IIFE).
    pub expression: String,
    /// Reponse : valeur JSON du JS, ou message d'erreur.
    pub respond: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>,
}
```

**3. Extension de `Registry`**

Ajouter le champ `eval_tx` et migrer `logs` vers `TokioMutex` (pour que `eval_handler` puisse acquérir sans conflit depuis le runtime dédié) :

```rust
// Registry avant (lignes 104-117)
pub(crate) struct Registry {
    pub(crate) state: std::sync::RwLock<BTreeMap<String, serde_json::Value>>,
    pub(crate) logs: std::sync::Mutex<VecDeque<deveye_core::LogEvent>>,
    routes: std::sync::RwLock<HashMap<String, Route>>,
}

// Registry apres
pub(crate) struct Registry {
    /// Lecture/ecriture synchrone : `set_state` n'est pas async.
    pub(crate) state: std::sync::RwLock<BTreeMap<String, serde_json::Value>>,
    /// Tokio Mutex : le layer tracing pousse depuis threads arbitraires,
    /// mais `eval_handler` et `logs_poll_handler` sont async et ne doivent
    /// pas bloquer le thread executor sur un mutex std.
    pub(crate) logs: TokioMutex<VecDeque<deveye_core::LogEvent>>,
    routes: std::sync::RwLock<HashMap<String, Route>>,
    /// Emetteur vers la pompe d'evaluation de l'app (None tant que l'app
    /// n'a pas appele [`take_eval_requests`]).
    pub(crate) eval_tx: TokioRwLock<Option<tokio::sync::mpsc::UnboundedSender<EvalRequest>>>,
}
```

Adapter le `Default` en conséquence :

```rust
impl Default for Registry {
    fn default() -> Self {
        Self {
            state: std::sync::RwLock::new(BTreeMap::new()),
            logs: TokioMutex::new(VecDeque::with_capacity(LOG_BUFFER_CAPACITY)),
            routes: std::sync::RwLock::new(HashMap::new()),
            eval_tx: TokioRwLock::new(None),
        }
    }
}
```

**4. Runtime dédié**

Ajouter la fonction `runtime()` (identique aux vendored, lignes 125-135) après `REGISTRY`. Motif : `init()` est appelé avant `dioxus::launch`, donc hors de tout runtime tokio — `tokio::spawn` nu paniquerait :

```rust
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::LazyLock<tokio::runtime::Runtime> =
        std::sync::LazyLock::new(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("deveye-sdk")
                .enable_all()
                .build()
                .expect("runtime deveye-sdk")
        });
    &RT
}
```

Note : on utilise `std::sync::LazyLock` (déjà présent dans le canonique) plutôt que `once_cell::sync::Lazy` pour rester cohérent avec le reste du canonique, même si la dépendance `once_cell` est ajoutée pour compatibilité éventuelle avec les vendored.

**5. Mise à jour de `init()` : changer `tokio::spawn` en `runtime().spawn()`**

```rust
// Avant (ligne 140)
tokio::spawn(async move {

// Apres
runtime().spawn(async move {
```

**6. Extension du routeur dans `init()` : ajouter `POST /devtools/eval`**

Le middleware `guard_local_host` doit rester branché sur toutes les routes, y compris le nouvel endpoint :

```rust
// Avant (lignes 141-147)
let app = Router::new()
    .route("/devtools/state", get(state_handler))
    .route("/devtools/logs/poll", get(logs_poll_handler))
    .route("/devtools/command", post(command_handler))
    .route("/devtools/commands", get(commands_handler))
    .layer(middleware::from_fn(guard_local_host))
    .with_state(registry);

// Apres
let app = Router::new()
    .route("/devtools/state", get(state_handler))
    .route("/devtools/logs/poll", get(logs_poll_handler))
    .route("/devtools/command", post(command_handler))
    .route("/devtools/commands", get(commands_handler))
    .route("/devtools/eval", post(eval_handler))
    .layer(middleware::from_fn(guard_local_host))
    .with_state(registry);
```

Le middleware `guard_local_host` (lignes 173-197 actuelles) est conservé tel quel — il s'applique à toutes les routes grâce à `.layer()` en bout de chaîne.

**7. Mise à jour de `logs_poll_handler` : adapter à `TokioMutex`**

```rust
// Avant (ligne 241-247) — std::sync::Mutex
async fn logs_poll_handler(State(registry): State<Arc<Registry>>) -> impl IntoResponse {
    let events: Vec<_> = registry
        .logs
        .lock()
        .map(|mut guard| guard.drain(..).collect())
        .unwrap_or_default();
    (StatusCode::OK, Json(LogsPayload { events }))
}

// Apres — tokio::sync::Mutex (await requis, pas de Result)
async fn logs_poll_handler(State(registry): State<Arc<Registry>>) -> impl IntoResponse {
    let events: Vec<_> = registry.logs.lock().await.drain(..).collect();
    (StatusCode::OK, Json(LogsPayload { events }))
}
```

**8. Nouveaux types et handlers : `take_eval_requests`, `EvalBody`, `EvalResponse`, `eval_handler`**

À ajouter après `commands_handler` (après la ligne 292 actuelle) :

```rust
/// Corps de `POST /devtools/eval`.
#[derive(Deserialize)]
struct EvalBody {
    expression: String,
}

/// Reponse de `POST /devtools/eval` : `{ value }` ou `{ error }`.
#[derive(Serialize)]
struct EvalResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Cote app : prend le bout consommateur du canal d'evaluation. La pompe de
/// l'app boucle dessus et execute chaque [`EvalRequest`] dans son webview.
/// Un nouvel appel remplace la pompe precedente (l'ancien receiver se ferme).
pub fn take_eval_requests() -> tokio::sync::mpsc::UnboundedReceiver<EvalRequest> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let registry = REGISTRY.clone();
    runtime().spawn(async move {
        *registry.eval_tx.write().await = Some(tx);
    });
    rx
}

async fn eval_handler(
    State(registry): State<Arc<Registry>>,
    Json(body): Json<EvalBody>,
) -> impl IntoResponse {
    let Some(tx) = registry.eval_tx.read().await.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(EvalResponse {
                value: None,
                error: Some("aucune pompe d'evaluation enregistree cote app".into()),
            }),
        );
    };
    let (respond, rx) = tokio::sync::oneshot::channel();
    if tx
        .send(EvalRequest {
            expression: body.expression,
            respond,
        })
        .is_err()
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(EvalResponse {
                value: None,
                error: Some("la pompe d'evaluation de l'app s'est arretee".into()),
            }),
        );
    }
    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(Ok(value))) => (
            StatusCode::OK,
            Json(EvalResponse { value: Some(value), error: None }),
        ),
        Ok(Ok(Err(err))) => (
            StatusCode::OK,
            Json(EvalResponse { value: None, error: Some(err) }),
        ),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(EvalResponse {
                value: None,
                error: Some("la pompe a abandonne la requete".into()),
            }),
        ),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(EvalResponse {
                value: None,
                error: Some("evaluation JS sans reponse apres 10 s".into()),
            }),
        ),
    }
}
```

**9. Mise à jour du layer tracing** (`deveye-sdk/src/tracing_layer.rs`)

Le layer pousse des événements dans `REGISTRY.logs` via un `Mutex`. Avec la migration vers `TokioMutex`, le push devient bloquant (`blocking_lock()`) depuis un contexte sync (le layer tracing s'exécute sur des threads arbitraires). Remplacer l'accès :

```rust
// Pattern a remplacer dans tracing_layer.rs
if let Ok(mut guard) = REGISTRY.logs.lock() { ... }

// Par (TokioMutex en contexte sync)
let mut guard = REGISTRY.logs.blocking_lock();
```

### `deveye-ports/src/errors.rs`

Ajouter la variante `Timeout` après `Network` :

```rust
// Avant (lignes 1-32)
#[derive(Debug, Error)]
pub enum PortError {
    #[error("rendu : {0}")]
    Render(String),
    #[error("inspection : {0}")]
    Inspect(String),
    #[error("interaction : {0}")]
    Interaction(String),
    #[error("logs : {0}")]
    Log(String),
    #[error("commande {route} : {message}")]
    Route { route: String, message: String },
    #[error("reseau : {0}")]
    Network(String),
    #[error("indisponible sur cette plateforme : {0}")]
    Unsupported(String),
}

// Apres : ajout de Timeout entre Network et Unsupported
    #[error("reseau : {0}")]
    Network(String),

    #[error("delai depasse : {0}")]
    Timeout(String),

    #[error("indisponible sur cette plateforme : {0}")]
    Unsupported(String),
```

### `deveye-adapters/adapter-dioxus-signals/src/bridge.rs`

Mettre à jour le mapping d'erreur dans `SdkEvaluator::evaluate` (lignes 62-71) pour distinguer les timeouts :

```rust
// Avant (lignes 58-73)
let status = resp.status();
let body: EvalResponse = resp
    .json()
    .await
    .map_err(|e| PortError::Render(format!("decode reponse eval : {e}")))?;
if let Some(error) = body.error {
    if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        return Err(PortError::Network(format!("SDK eval : {error}")));
    }
    return Err(PortError::Render(format!("eval JS : {error}")));
}

// Apres
let status = resp.status();
let body: EvalResponse = resp
    .json()
    .await
    .map_err(|e| PortError::Render(format!("decode reponse eval : {e}")))?;
if let Some(error) = body.error {
    if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        return Err(PortError::Network(format!("SDK eval : {error}")));
    }
    // 504 Gateway Timeout = la pompe a ete jointe mais n'a pas repondu
    // dans les 10 s impartis par le SDK — distinctif d'une erreur JS.
    if status == reqwest::StatusCode::GATEWAY_TIMEOUT {
        return Err(PortError::Timeout(format!("eval JS timeout : {error}")));
    }
    return Err(PortError::Render(format!("eval JS : {error}")));
}
```

### `docs/architecture.md`

Corrections fichier par fichier :

**a) Arborescence workspace (lignes 87-94)** : ajouter `adapter-commands-http` dans la liste des adapters.

```markdown
# Avant (lignes 88-94)
├── deveye-adapters/            ← IMPLÉMENTATIONS (un crate par adapter)
│   ├── adapter-cdp-webview/    # Renderer + InteractionSink + AppInspector + JsEvaluator via CDP
│   ├── adapter-screencap-xcap/ # Renderer fallback OS-level (any app)
│   ├── adapter-dioxus-signals/ # AppInspector pour apps Dioxus (via SDK opt-in)
│   ├── adapter-logs-tracing/   # LogSource via le buffer tracing du SDK opt-in
│   └── adapter-mcp-server/     # consomme les ports, expose au Model Context Protocol

# Apres
├── deveye-adapters/            ← IMPLÉMENTATIONS (un crate par adapter)
│   ├── adapter-cdp-webview/    # Renderer + InteractionSink + AppInspector + JsEvaluator via CDP
│   ├── adapter-screencap-xcap/ # Renderer fallback OS-level (any app)
│   ├── adapter-dioxus-signals/ # AppInspector + pont natif (JsEvaluator/InteractionSink) via SDK
│   ├── adapter-logs-tracing/   # LogSource via le buffer tracing du SDK opt-in
│   ├── adapter-commands-http/  # CommandRouter via POST /devtools/command du SDK opt-in
│   └── adapter-mcp-server/     # consomme les ports, expose au Model Context Protocol
```

**b) Trait `LogSource` (ligne 174)** : remplacer la signature fausse.

```markdown
# Avant (ligne 174)
    fn stream(&self) -> BoxStream<'static, LogEvent>;

# Apres
    async fn poll(&self) -> PortResult<Vec<LogEvent>>;
```

Supprimer également la référence à `BoxStream` dans le paragraphe qui suit si elle y figure.

**c) Section « Hors périmètre actuel » (ligne 261)** : retirer ou corriger la ligne « Adapter Linux (WebKitGTK inspector) ».

```markdown
# Avant (ligne 261)
- Adapter Linux (WebKitGTK inspector)

# Apres : reformuler pour refleter la realite
- Adapter Linux natif complet : le pont natif (adapter-dioxus-signals/bridge.rs,
  SdkEvaluator + JsInteraction + EvalDomInspector) couvre clic/saisie/DOM/eval
  via la pompe document::eval. Hors perimetre : resize viewport (exige CDP),
  adapter-screencap-kwin (voir M1).
```

**d) Mentionner `LazyCdp` et le mode `--target`** dans la section « Robustesse CDP » (après la ligne 233), en renvoyant à `usage.md` pour les détails.

**e) Renvoyer vers `docs/wire-protocol.md`** dans la section sécurité (après les quatre garde-fous, ligne 254) : « Le contrat complet des routes HTTP du SDK est figé dans `docs/wire-protocol.md`. »

### `docs/usage.md`

**a) Commandes de vérification section sécurité (lignes 159-161)** : remplacer les commandes Windows-only par des variantes cross-plateforme :

```markdown
# Avant (lignes 159-161)
cargo tree -p <votre-app> | findstr deveye
# (doit ne rien retourner)

netstat -an | findstr 31337
# (adresse doit etre 127.0.0.1:31337, jamais 0.0.0.0)

# Apres
# Linux / macOS
cargo tree -p <votre-app> | grep deveye
# Windows
cargo tree -p <votre-app> | findstr deveye
# (doit ne rien retourner)

# Linux / macOS
ss -tlnp | grep 31337
# Windows
netstat -an | findstr 31337
# (adresse doit etre 127.0.0.1:31337, jamais 0.0.0.0)
```

**b) Mettre à jour `docs/wire-protocol.md`** : créer ce fichier (livrable séparé de M0) qui fige l'ensemble des routes HTTP du SDK wire protocol. Ce jalon produit le squelette ; la page dédiée le documente exhaustivement.

### Vendored Atelier (`Atelier/deveye/deveye-sdk/src/lib.rs`)

Resynchroniser sur le canonique post-backport :
- Remplacer `once_cell::sync::Lazy` (ligne 116) par `std::sync::LazyLock` — aligne avec le canonique et supprime la dépendance `once_cell` si elle est inutile ailleurs dans le workspace Atelier.
- Ajouter le middleware `guard_local_host` (manquant dans les vendored) sur toutes les routes y compris `/devtools/eval`.
- Supprimer la fonction `runtime()` locale si Atelier peut utiliser le runtime de Dioxus via `tokio::spawn` — ou conserver le runtime dédié si Atelier appelle `init()` avant `dioxus::launch` (à vérifier dans `Atelier/src/main.rs`).

### Vendored Chronos (`Chronos/crates/deveye/deveye-sdk/src/lib.rs`)

Mêmes corrections que vendored Atelier. Les deux fichiers étant identiques byte pour byte, un seul diff suffit à copier sur les deux.

---

## Nouveaux artefacts

**`docs/wire-protocol.md`** (squelette créé dans ce jalon, contenu détaillé dans une page dédiée) :

Tableau des routes HTTP du SDK — contrat figé :

| Méthode | Route | Corps (JSON) | Réponse (JSON) | Notes |
|---------|-------|--------------|----------------|-------|
| `GET` | `/devtools/state` | — | `{ entries: { [key]: any } }` | `StatePayload` |
| `GET` | `/devtools/logs/poll` | — | `{ events: LogEvent[] }` | draine le buffer |
| `POST` | `/devtools/command` | `{ route: string, payload?: any }` | `any` | `CommandRequest` |
| `GET` | `/devtools/commands` | — | `CommandDescriptor[]` | liste des routes enregistrées |
| `POST` | `/devtools/eval` | `{ expression: string }` | `{ value?: any, error?: string }` | timeout SDK = 10 s, `503` si pompe absente, `504` si timeout |

Toutes les routes exigent un header `Host` localhost (`localhost`, `127.0.0.1` ou `[::1]`) — middleware `guard_local_host` rejette sinon avec `403`.

**Aucun nouveau crate, aucun nouvel outil MCP** créé dans ce jalon.

---

## Critère de done

```bash
# 1. Workspace DevEye compile sans erreur
cargo check --workspace

# 2. Aucun warning clippy
cargo clippy --workspace --all-targets -- -D warnings

# 3. Tests unitaires passent (incluant les tests host_is_local du SDK)
cargo test --workspace

# 4. Verification manuelle : le SDK canonique sert bien POST /devtools/eval
#    (lancer le SDK en integration test ou via un cargo test d'integration)
curl -s -H "Host: localhost" \
  -X POST http://127.0.0.1:31337/devtools/eval \
  -H "Content-Type: application/json" \
  -d '{"expression":"1+1"}' \
  | grep -E '"value"|"error"'
# Attendu sans pompe : {"error":"aucune pompe d'evaluation enregistree cote app"}
# Attendu avec pompe active : {"value":2}

# 5. Verification que le garde-fous Host bloque un Host etranger
curl -s -H "Host: evil.com" \
  http://127.0.0.1:31337/devtools/eval \
  -X POST -H "Content-Type: application/json" \
  -d '{"expression":"1+1"}' \
  -o /dev/null -w "%{http_code}"
# Attendu : 403

# 6. Verification que PortError::Timeout compile et est utilisable
#    (present dans adapter-dioxus-signals/bridge.rs)
cargo check -p adapter-dioxus-signals

# 7. Documentation : les deux fichiers de doc ne mentionnent plus LogSource::stream
grep -n "BoxStream" docs/architecture.md
# Attendu : aucune occurrence

# 8. Vendored resynchronises : guard_local_host present dans Atelier et Chronos
grep -n "guard_local_host" \
  ../Atelier/deveye/deveye-sdk/src/lib.rs \
  ../Chronos/crates/deveye/deveye-sdk/src/lib.rs
# Attendu : au moins une occurrence dans chaque
```

---

## Risques et pièges

**1. Collision de runtime tokio (risque élevé)**
Le canonique actuel utilise `tokio::spawn` dans `init()`, ce qui suppose un runtime actif. La migration vers `runtime().spawn()` avec un runtime dédié résout le problème pour les apps qui appellent `init()` avant leur propre runtime (avant `dioxus::launch`). Piège : si l'app cible possède déjà son propre runtime et que `init()` est appelé depuis un contexte `async`, le runtime dédié reste valide mais crée un second executor — acceptable en dev, jamais en prod (le `cfg(debug_assertions)` reste le garde-fou).

**2. `TokioMutex` dans le layer tracing (risque moyen)**
Le layer tracing s'exécute depuis des threads arbitraires (sync). `blocking_lock()` sur un `TokioMutex` est la méthode correcte en contexte sync, mais elle paniquera si appelée depuis un thread qui est lui-même dans le runtime tokio du SDK (deadlock potentiel). Vérifier dans `tracing_layer.rs` que le code ne tourne jamais depuis une tâche tokio du runtime du SDK.

**3. `guard_local_host` manquant dans les vendored (risque sécurité)**
Les vendored Atelier et Chronos exposent `/devtools/eval` sans protection anti DNS-rebinding. En dev local le risque est faible (port 31337 non exposé au réseau), mais la resynchronisation doit absolument rajouter le middleware — c'est une règle non négociable du CLAUDE.md (sécurité dev-only).

**4. `once_cell` vs `std::sync::LazyLock` (risque de rupture Cargo)**
Si `once_cell` est ajouté au `Cargo.toml` canonique mais que les vendored continuent à l'utiliser directement, les deux coexistent sans problème. Si les vendored migrent vers `std::sync::LazyLock`, vérifier que la version MSRV des apps cibles supporte Rust 1.80 (introduction de `LazyLock` en stable).

**5. Règle hexagonale : aucun crate deveye-* ne doit dépendre des apps cibles**
La migration des vendored vers le canonique implique de copier du code *depuis* les apps vers DevEye — sens inverse de la règle. Vérifier que les artefacts backportés (`EvalRequest`, `take_eval_requests`, `eval_handler`) sont entièrement génériques et ne référencent aucun type spécifique à Atelier ou Chronos.

**6. Wayland / XWayland pour les tests runtime**
Le critère de done 4 (curl vers le SDK) peut être exécuté en CLI pure sans interface graphique. En revanche, tester la pompe complète (app Dioxus active + take_eval_requests + eval JS réel) requiert une session graphique Wayland ou X11. Sur la machine de dev (Fedora 44 KDE Plasma Wayland), utiliser `GDK_BACKEND=x11` pour forcer XWayland si xcap est impliqué dans le même test.

**7. Absence de `docs/wire-protocol.md` bloque M1b**
Ce fichier est le prérequis contractuel de M1b (pompe eval dans Atelier) et M3 (shim .NET). Ne pas fermer M0 tant que le squelette du wire-protocol n'est pas validé par Morgan.

---

## Checklist d'exécution

- [ ] Lire `deveye-sdk/src/tracing_layer.rs` en entier pour identifier tous les accès à `REGISTRY.logs` et préparer la migration `std::sync::Mutex` -> `TokioMutex::blocking_lock()`.
- [ ] Mettre à jour `deveye-sdk/Cargo.toml` : ajouter `rt-multi-thread`, `time` aux features tokio ; ajouter `once_cell = "1"` (nécessaire si les vendored doivent rester compatibles pendant la transition).
- [ ] Ajouter `pub struct EvalRequest { ... }` dans `deveye-sdk/src/lib.rs`.
- [ ] Étendre `struct Registry` avec le champ `eval_tx: TokioRwLock<Option<...>>` ; migrer `logs` vers `TokioMutex`.
- [ ] Adapter `Registry::default()` avec les nouveaux types.
- [ ] Ajouter la fonction `runtime()` avec `std::sync::LazyLock`.
- [ ] Remplacer `tokio::spawn` par `runtime().spawn()` dans `init()`.
- [ ] Ajouter `.route("/devtools/eval", post(eval_handler))` dans le routeur de `init()` — avant `.layer(middleware::from_fn(guard_local_host))`.
- [ ] Mettre à jour `logs_poll_handler` pour `TokioMutex` (`.lock().await`).
- [ ] Ajouter `take_eval_requests()`, `EvalBody`, `EvalResponse`, `eval_handler` dans `lib.rs`.
- [ ] Mettre à jour `tracing_layer.rs` : remplacer l'accès mutex par `blocking_lock()`.
- [ ] Ajouter `PortError::Timeout(String)` dans `deveye-ports/src/errors.rs`.
- [ ] Mettre à jour `adapter-dioxus-signals/src/bridge.rs` : mapper `504 GATEWAY_TIMEOUT` en `PortError::Timeout`.
- [ ] `cargo check --workspace` : vert.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` : vert.
- [ ] `cargo test --workspace` : vert.
- [ ] Corriger `docs/architecture.md` : arborescence (ajouter `adapter-commands-http`), signature `LogSource::poll`, section hors-périmètre Linux, mention `LazyCdp`/`--target`, renvoi vers `wire-protocol.md`.
- [ ] Corriger `docs/usage.md` : commandes cross-plateforme `grep`/`ss` en remplacement de `findstr`/`netstat`.
- [ ] Créer le squelette `docs/wire-protocol.md` avec le tableau des cinq routes et les codes de statut.
- [ ] Resynchroniser vendored Atelier : `std::sync::LazyLock`, ajout `guard_local_host`, supprimer `once_cell` si inutile ailleurs dans le workspace Atelier.
- [ ] Resynchroniser vendored Chronos : mêmes corrections.
- [ ] Vérification manuelle curl (critères 4 et 5 de la section Critère de done).
- [ ] Relecture par Morgan du `docs/wire-protocol.md` — validation avant fermeture de M0.
