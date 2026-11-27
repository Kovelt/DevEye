//! SDK opt-in DevEye — à intégrer côté app cible (par exemple une app Dioxus
//! desktop) **uniquement en `cfg(debug_assertions)`**.
//!
//! Démarre un micro-serveur HTTP local sur `127.0.0.1:<port>` (par défaut
//! `31337`) qui expose le state et les logs de l'app pour qu'un binaire
//! `deveye-server` les consomme et les rende disponibles à Claude via MCP.
//!
//! Sécurité :
//! - bind strict sur `127.0.0.1` ; jamais `0.0.0.0`.
//! - port surchargable par variable d'environnement `DEVEYE_PORT`.
//! - **rien** ne s'embarque en build release tant que l'appelant respecte
//!   le pattern `#[cfg(debug_assertions)] deveye_sdk::init(...)`.
//!
//! Usage minimal côté app cible :
//!
//! ```ignore
//! #[cfg(debug_assertions)]
//! {
//!     deveye_sdk::init(deveye_sdk::Config::default());
//!     deveye_sdk::install_tracing_layer(); // optionnel : capture logs tracing
//! }
//!
//! // Plus tard, dans un `use_effect` Dioxus :
//! deveye_sdk::set_state("page", serde_json::json!("Agenda"));
//! ```

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
// En build release, `init` est un no-op (filet de sécurité dev-only) : tout le
// code orienté serveur (handlers, middlewares, imports axum, `host_is_local`)
// devient du code mort *attendu*. On tait ces avertissements uniquement en
// release ; en debug, `dead_code`/`unused_imports` restent pleinement actifs.
#![cfg_attr(not(debug_assertions), allow(dead_code, unused_imports))]

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use axum::{
    extract::{DefaultBodyLimit, Request, State},
    http::{header, HeaderName, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use deveye_core::CommandDescriptor;

mod redact;
mod tracing_layer;

pub use tracing_layer::install_tracing_layer;

/// Port par défaut du SDK. Choix arbitraire mais facile à retenir.
pub const DEFAULT_PORT: u16 = 31337;

/// Capacité maximale du buffer de logs avant qu'on commence à perdre les
/// anciens événements (politique drop-oldest).
pub const LOG_BUFFER_CAPACITY: usize = 1024;

/// Capacité de la file de requêtes d'évaluation. **Bornée** (backpressure) :
/// une app figée qui ne draine plus sa pompe ne fait pas enfler la file sans
/// limite — au-delà, `POST /devtools/eval` répond « file pleine » (503) et
/// libère le client au lieu de laisser la mémoire croître indéfiniment.
pub const EVAL_QUEUE_CAPACITY: usize = 32;

/// Valeur substituée aux données masquées (cf. [`Config::secret_keys`]).
pub use redact::REDACTED;

/// Version du wire protocol DevEye (`MAJEURE.MINEURE`), émise dans le header
/// `X-DevEye-Protocol-Version` de **chaque** réponse. La majeure augmente à
/// chaque changement cassant du contrat HTTP, la mineure à chaque ajout
/// rétrocompatible (cf. `docs/wire-protocol.md` §2.3). Le client s'en sert pour
/// détecter une incompatibilité de version.
pub const PROTOCOL_VERSION: &str = "1.0";

/// Taille maximale du corps d'une requête (octets) : 1 Mio. Au-delà, le SDK
/// répond `413` sans lire le reste. Les corps légitimes (expression JS,
/// `payload` de commande) sont de quelques Kio ; la borne protège l'app hôte
/// d'une allocation démesurée sur un port local ouvert à tout processus de la
/// machine.
pub const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;

/// Configuration du SDK. Le `Default` lit `DEVEYE_PORT` si présent, sinon
/// utilise [`DEFAULT_PORT`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Config {
    pub port: u16,
    /// Clés JSON **en plus** de celles masquées par défaut (`password`,
    /// `token`, `api_key`, `authorization`, `cookie`…) : la valeur de toute clé
    /// de cette liste est remplacée par [`REDACTED`] dans l'état et les champs
    /// de logs servis. Comparaison sans casse ni séparateurs (`API-KEY` =
    /// `api_key`), correspondance exacte sur la clé entière.
    pub secret_keys: Vec<String>,
}

impl Config {
    /// Configuration écoutant sur `port` (127.0.0.1 uniquement).
    pub fn with_port(port: u16) -> Self {
        Self {
            port,
            secret_keys: Vec::new(),
        }
    }

    /// Ajoute des clés à masquer (cf. [`Config::secret_keys`]). Cumulable.
    #[must_use]
    pub fn with_secret_keys<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.secret_keys.extend(keys.into_iter().map(Into::into));
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        let port = std::env::var("DEVEYE_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        Self::with_port(port)
    }
}

/// Snapshot du state app servi par `GET /devtools/state`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StatePayload {
    pub entries: BTreeMap<String, serde_json::Value>,
}

/// Snapshot des logs accumulés servi par `GET /devtools/logs/poll`.
/// L'appel **draine** le buffer : un second `poll` ne renverra que les
/// événements arrivés entre-temps.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LogsPayload {
    pub events: Vec<deveye_core::LogEvent>,
}

/// Une demande d'évaluation JS poussée par `POST /devtools/eval` et
/// consommée par la **pompe** côté app (cf. [`take_eval_requests`]). L'app
/// exécute l'expression dans son webview (`document::eval` chez Dioxus) et
/// répond par le `oneshot`.
#[non_exhaustive]
pub struct EvalRequest {
    /// Expression JS (une seule expression — typiquement une IIFE) ; la
    /// pompe l'enveloppe d'un try/catch et l'exécute dans le webview.
    pub expression: String,
    /// Réponse : valeur JSON du JS, ou message d'erreur.
    pub respond: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>,
}

/// Closure synchrone enregistrée par l'app via [`expose_route`] : reçoit le
/// `payload` JSON de l'invocation et retourne la valeur JSON de réponse. Le
/// handler est responsable de sa propre gestion d'erreur (le contrat
/// n'impose pas d'enveloppe de retour).
///
/// Le handler s'exécute dans `tokio::task::spawn_blocking` (thread de pool
/// dédié, hors du runtime SDK) : il peut être bloquant sans figer le
/// micro-serveur. Il ne doit pas appeler `block_on` sur le runtime SDK.
type CommandHandler = Arc<dyn Fn(serde_json::Value) -> serde_json::Value + Send + Sync>;

/// Une commande logique exposée par l'app : sa description (pour la
/// découverte par `GET /devtools/commands`) et son handler.
struct Route {
    description: String,
    handler: CommandHandler,
}

/// Registry globale partagée entre l'app cible et le mini-serveur axum.
///
/// `state` et `routes` utilisent `std::sync::RwLock` (synchrone) pour que
/// [`set_state`] et [`expose_route`] écrivent **immédiatement** sans passer
/// par un spawn tokio. `eval_tx` reste un `tokio::sync::RwLock` car il est
/// lu depuis des handlers async (son API nécessite `.await`).
pub(crate) struct Registry {
    pub(crate) state: std::sync::RwLock<BTreeMap<String, serde_json::Value>>,
    pub(crate) logs: Mutex<VecDeque<deveye_core::LogEvent>>,
    /// Émetteur vers la pompe d'évaluation de l'app (None tant que l'app
    /// n'a pas appelé [`take_eval_requests`]).
    pub(crate) eval_tx: tokio::sync::RwLock<Option<tokio::sync::mpsc::Sender<EvalRequest>>>,
    /// Émetteur du canal de drain des logs (N6) : posé une seule fois par
    /// [`install_tracing_layer`][crate::install_tracing_layer]. La couche
    /// tracing envoie chaque [`deveye_core::LogEvent`] via ce canal
    /// (synchrone, non bloquant) ; une unique tâche tokio consomme le
    /// récepteur et pousse dans `logs`, préservant l'ordre d'émission.
    pub(crate) log_tx:
        std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<deveye_core::LogEvent>>,
    /// Commandes logiques exposées par l'app via [`expose_route`], servies
    /// par `POST /devtools/command` et `GET /devtools/commands`.
    pub(crate) routes: std::sync::RwLock<HashMap<String, Route>>,
    /// Clés sensibles propres à l'app (normalisées), posées par [`init`] ;
    /// s'ajoutent aux clés par défaut au moment de servir state et logs.
    pub(crate) secret_keys: std::sync::RwLock<HashSet<String>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            state: std::sync::RwLock::new(BTreeMap::new()),
            logs: Mutex::new(VecDeque::with_capacity(LOG_BUFFER_CAPACITY)),
            eval_tx: tokio::sync::RwLock::new(None),
            log_tx: std::sync::OnceLock::new(),
            routes: std::sync::RwLock::new(HashMap::new()),
            secret_keys: std::sync::RwLock::new(HashSet::new()),
        }
    }
}

pub(crate) static REGISTRY: std::sync::LazyLock<Arc<Registry>> =
    std::sync::LazyLock::new(|| Arc::new(Registry::default()));

/// Runtime tokio **dédié au SDK** (1 thread, démarré au premier usage).
///
/// Le SDK ne dépend ainsi jamais du runtime de l'app hôte : le pattern
/// documenté appelle `init` **avant** `dioxus::launch`, donc hors de tout
/// contexte tokio — un `tokio::spawn` nu y paniquerait. Idem pour la couche
/// tracing, dont les événements arrivent de threads arbitraires.
///
/// L'`expect` est un échec d'initialisation (plus de thread ou de descripteur
/// disponible) : sans runtime le SDK ne peut rien faire, et le code n'existe
/// qu'en build de développement. Un panic explicite y est préférable à un SDK
/// muet.
#[allow(clippy::expect_used)]
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::LazyLock<tokio::runtime::Runtime> = std::sync::LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("deveye-sdk")
            .enable_all()
            .build()
            .expect("echec de creation du runtime tokio du SDK DevEye (dev-only)")
    });
    &RT
}

/// Démarre le mini-serveur axum sur le runtime dédié du SDK.
///
/// Appelable de n'importe où (avant `dioxus::launch` compris). Idempotent au
/// sens où un second appel re-spawn une tâche mais échouera à lier le même
/// port (avertissement `tracing`) — ce qui est sans conséquence pour l'app. Un
/// port déjà pris est signalé de la même façon, sans interrompre l'app.
///
/// **Filet de sécurité (défense en profondeur)** : en build **release**, cette
/// fonction est un **no-op** — le SDK n'ouvre *jamais* de port HTTP hors
/// `debug_assertions`, même si l'intégrateur oublie d'entourer l'appel de
/// `#[cfg(debug_assertions)]`. La garde côté appelant reste la voie normale
/// (cf. doc du module) ; ceci en est le dernier rempart contre une fuite du
/// serveur de debug en production.
pub fn init(config: Config) {
    // Release : no-op total (cf. filet de sécurité ci-dessus). Le corps n'est
    // même pas compilé hors `debug_assertions`.
    #[cfg(not(debug_assertions))]
    let _ = config;

    #[cfg(debug_assertions)]
    {
        let registry = REGISTRY.clone();
        *registry
            .secret_keys
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = config
            .secret_keys
            .iter()
            .map(|key| redact::normalize_key(key))
            .collect();
        runtime().spawn(async move {
            let app = Router::new()
                .route("/devtools/info", get(info_handler))
                .route("/devtools/state", get(state_handler))
                .route("/devtools/logs/poll", get(logs_poll_handler))
                .route("/devtools/eval", post(eval_handler))
                .route("/devtools/command", post(command_handler))
                .route("/devtools/commands", get(commands_handler))
                // CORS deny-all explicite : aucune origine cross-origin n'est
                // autorisée (avant, ce n'était qu'accidentel — simple absence de
                // header CORS). Defense-in-depth : empêche une page web tierce de
                // solliciter ce serveur local via le navigateur de l'utilisateur. Le
                // trafic serveur (clients reqwest des adapters) ignore CORS — non
                // concerné.
                .layer(tower_http::cors::CorsLayer::new())
                // Limite de taille des corps (413 au-delà) ; couche interne, donc
                // le 413 reçoit lui aussi le header de version.
                .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
                // Chaque `.layer()` enveloppe le routeur courant par l'EXTERIEUR :
                // le DERNIER appelé est donc la couche la plus EXTERNE. Ici
                // `add_protocol_version` (dernier) est EXTERNE, puis `guard_local_host`,
                // et `CorsLayer` (premier) la plus INTERNE. Ainsi le 403 de la garde
                // ressort en retraversant `add_protocol_version` (externe) et reçoit le
                // header de version — *chaque* réponse le porte.
                .layer(middleware::from_fn(guard_local_host))
                .layer(middleware::from_fn(add_protocol_version))
                .with_state(registry);
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], config.port));
            match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    if let Err(e) = axum::serve(listener, app).await {
                        tracing::error!(
                            "deveye-sdk: le serveur HTTP s'est arrete avec une erreur: {e}"
                        );
                    }
                }
                Err(e) => {
                    // Non fatal pour l'app hôte, mais jamais silencieux : sans
                    // ce message, un port pris (autre instance, autre app)
                    // laissait l'agent parler à la mauvaise app sans le savoir.
                    tracing::warn!(
                        port = config.port,
                        error = %e,
                        "deveye-sdk: impossible de lier 127.0.0.1:{} ; le SDK reste inactif \
                         (choisir un autre port via DEVEYE_PORT)",
                        config.port
                    );
                }
            }
        });
    }
}

/// Pose une entrée nommée dans le registry. Écrase la valeur précédente
/// si la clé existe déjà.
///
/// L'écriture est **synchrone** (verrou std) : une lecture immédiatement
/// après le retour de cette fonction verra la nouvelle valeur.
pub fn set_state(name: impl Into<String>, value: serde_json::Value) {
    REGISTRY
        .state
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(name.into(), value);
}

async fn state_handler(State(registry): State<Arc<Registry>>) -> impl IntoResponse {
    let mut entries = registry
        .state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    // Masquage au service (et non à l'écriture) : les clés de `Config`
    // s'appliquent quel que soit l'ordre `init` / `set_state`.
    let extra = registry
        .secret_keys
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    redact::redact_entries(&mut entries, &extra);
    (StatusCode::OK, Json(StatePayload { entries }))
}

async fn logs_poll_handler(State(registry): State<Arc<Registry>>) -> impl IntoResponse {
    let mut guard = registry.logs.lock().await;
    let mut events: Vec<_> = guard.drain(..).collect();
    drop(guard);
    let extra = registry
        .secret_keys
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Seuls les champs structurés sont filtrés ; le texte du message ne l'est pas.
    for event in &mut events {
        redact::redact(&mut event.fields, &extra);
    }
    (StatusCode::OK, Json(LogsPayload { events }))
}

/// Côté app : prend le bout consommateur du canal d'évaluation. La pompe de
/// l'app boucle dessus et exécute chaque [`EvalRequest`] dans son webview.
/// Un nouvel appel remplace la pompe précédente (l'ancien receiver se ferme).
pub fn take_eval_requests() -> tokio::sync::mpsc::Receiver<EvalRequest> {
    let (tx, rx) = tokio::sync::mpsc::channel(EVAL_QUEUE_CAPACITY);
    let registry = REGISTRY.clone();
    runtime().spawn(async move {
        *registry.eval_tx.write().await = Some(tx);
    });
    rx
}

/// Corps de `POST /devtools/eval`.
#[derive(Deserialize)]
struct EvalBody {
    expression: String,
}

/// Réponse de `POST /devtools/eval` : `{ value }` ou `{ error }`.
#[derive(Serialize)]
struct EvalResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
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
                error: Some("aucune pompe d'évaluation enregistrée côté app".into()),
            }),
        );
    };
    let (respond, rx) = tokio::sync::oneshot::channel();
    // `try_send` (non bloquant) : sur file pleine, on refuse (503) au lieu de
    // bloquer le handler async — la backpressure protège contre une pompe qui
    // ne draine plus (app figée) et empêche la file d'enfler sans limite.
    if let Err(err) = tx.try_send(EvalRequest {
        expression: body.expression,
        respond,
    }) {
        use tokio::sync::mpsc::error::TrySendError;
        let message = match err {
            TrySendError::Full(_) => "file d'évaluation pleine (backpressure) — réessayer",
            TrySendError::Closed(_) => "la pompe d'évaluation de l'app s'est arrêtée",
        };
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(EvalResponse {
                value: None,
                error: Some(message.into()),
            }),
        );
    }
    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(Ok(value))) => (
            StatusCode::OK,
            Json(EvalResponse {
                value: Some(value),
                error: None,
            }),
        ),
        Ok(Ok(Err(err))) => (
            StatusCode::OK,
            Json(EvalResponse {
                value: None,
                error: Some(err),
            }),
        ),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(EvalResponse {
                value: None,
                error: Some("la pompe a abandonné la requête".into()),
            }),
        ),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(EvalResponse {
                value: None,
                error: Some("évaluation JS sans réponse après 10 s".into()),
            }),
        ),
    }
}

/// Côté app : enregistre une commande logique nommée. L'agent la découvre via
/// `GET /devtools/commands` (outil MCP `list_commands`) et la déclenche via
/// `POST /devtools/command` (`invoke_command`). Un second appel sur le même
/// `name` remplace le handler précédent.
///
/// L'enregistrement est **synchrone** (verrou std) : la route est visible
/// immédiatement au retour de cette fonction.
///
/// ```ignore
/// deveye_sdk::expose_route("set-page", "Change la page affichee", |payload| {
///     let page = payload.get("page").and_then(|v| v.as_str()).unwrap_or("Accueil");
///     // ... muter le signal de l'app ...
///     serde_json::json!({ "ok": true, "page": page })
/// });
/// ```
pub fn expose_route<F>(name: impl Into<String>, description: impl Into<String>, handler: F)
where
    F: Fn(serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
{
    let name = name.into();
    let description = description.into();
    let handler: CommandHandler = Arc::new(handler);
    REGISTRY
        .routes
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            name,
            Route {
                description,
                handler,
            },
        );
}

/// Corps de `POST /devtools/command`.
#[derive(Deserialize)]
struct CommandRequest {
    route: String,
    #[serde(default)]
    payload: serde_json::Value,
}

async fn command_handler(
    State(registry): State<Arc<Registry>>,
    Json(body): Json<CommandRequest>,
) -> impl IntoResponse {
    // Clone l'Arc du handler sous le verrou std puis relache immediatement :
    // le handler s'execute hors verrou dans spawn_blocking (il peut solliciter
    // le registry sans risque de deadlock, et ne bloque pas le runtime SDK).
    let handler = registry
        .routes
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&body.route)
        .map(|route| route.handler.clone());
    match handler {
        Some(handler) => {
            let payload = body.payload;
            match tokio::task::spawn_blocking(move || handler(payload)).await {
                Ok(result) => (StatusCode::OK, Json(result)).into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("handler panique: {e}") })),
                )
                    .into_response(),
            }
        }
        None => (
            StatusCode::NOT_FOUND,
            // Corps minimal : la route est déjà portée par `PortError::Route`
            // côté client, dont le Display la préfixe (« commande {route} :
            // {message} ») — la répéter ici donnait « commande X : commande
            // inconnue : X » (constaté en conditions réelles).
            Json(serde_json::json!({ "error": "inconnue" })),
        )
            .into_response(),
    }
}

/// Corps de `GET /devtools/info` : identité du SDK et capacités actives.
#[derive(Serialize)]
struct InfoPayload {
    sdk_version: &'static str,
    protocol_version: &'static str,
    capabilities: Vec<&'static str>,
}

async fn info_handler(State(registry): State<Arc<Registry>>) -> impl IntoResponse {
    let mut capabilities = vec!["state", "commands", "info"];
    if registry.log_tx.get().is_some() {
        capabilities.push("logs");
    }
    if registry.eval_tx.read().await.is_some() {
        capabilities.push("eval");
    }
    (
        StatusCode::OK,
        Json(InfoPayload {
            sdk_version: env!("CARGO_PKG_VERSION"),
            protocol_version: PROTOCOL_VERSION,
            capabilities,
        }),
    )
}

async fn commands_handler(State(registry): State<Arc<Registry>>) -> impl IntoResponse {
    let mut descriptors: Vec<CommandDescriptor> = {
        let guard = registry
            .routes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .iter()
            .map(|(name, route)| CommandDescriptor::new(name.clone(), route.description.clone()))
            .collect()
    };
    descriptors.sort_by(|a, b| a.name.cmp(&b.name));
    (StatusCode::OK, Json(descriptors))
}

/// Middleware : rejette toute requête dont le header `Host` n'est pas local,
/// **avant** d'atteindre la route. Le bind sur `127.0.0.1` empêche les
/// connexions extérieures mais pas le DNS-rebinding (une page malveillante
/// peut résoudre un domaine sous son contrôle vers `127.0.0.1`) ; cette garde
/// coupe ce vecteur.
async fn guard_local_host(req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    match host {
        Some(host) if host_is_local(host) => next.run(req).await,
        _ => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Host non local" })),
        )
            .into_response(),
    }
}

/// Middleware : ajoute `X-DevEye-Protocol-Version` à **chaque** réponse. Posé
/// en couche externe de [`guard_local_host`], il enveloppe aussi le 403 de la
/// garde, conformément au contrat (cf. `docs/wire-protocol.md` §2.3).
async fn add_protocol_version(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    response.headers_mut().insert(
        HeaderName::from_static("x-deveye-protocol-version"),
        HeaderValue::from_static(PROTOCOL_VERSION),
    );
    response
}

/// Vrai si `suffix` est un port HTTP optionnel valide : la chaîne vide, ou
/// `:` suivi d'**au moins** un chiffre décimal. Tout autre suffixe (`toto`,
/// `:abc`, `:` seul) est rejeté — garde-fou symétrique entre les branches
/// IPv4/`localhost` et IPv6 de [`host_is_local`].
fn is_optional_port(suffix: &str) -> bool {
    suffix.is_empty()
        || (suffix.len() > 1
            && suffix.starts_with(':')
            && suffix.as_bytes()[1..].iter().all(u8::is_ascii_digit))
}

/// Vrai si `host` (valeur du header HTTP `Host`, port optionnel) désigne la
/// machine locale. `localhost` est insensible à la casse ; les littéraux IPv6
/// sont acceptés sous leur forme entre crochets (`[::1]`). Le port, s'il est
/// présent, doit être purement numérique — tout autre suffixe est rejeté.
pub(crate) fn host_is_local(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() {
        return false;
    }
    // IPv6 littéral entre crochets : `[::1]` ou `[::1]:<port>`. Le suffixe
    // après `]` ne peut être que vide ou un `:port` numérique — tout autre
    // suffixe (`[::1]toto`, `[::1]:abc`) est rejeté.
    if let Some(rest) = host.strip_prefix('[') {
        return matches!(
            rest.split_once(']'),
            Some((addr, suffix)) if addr == "::1" && is_optional_port(suffix)
        );
    }
    // Sinon, isoler le hostname d'un éventuel `:port` (un seul `:` hors IPv6) ;
    // un port non numérique laisse `host` entier, qui ne matchera pas.
    let hostname = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    };
    hostname.eq_ignore_ascii_case("localhost") || hostname == "127.0.0.1"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_31337_without_env() {
        let config = Config::with_port(DEFAULT_PORT);
        assert_eq!(config.port, 31337);
    }

    #[test]
    fn state_payload_serializes_with_entries_key() {
        let mut entries = BTreeMap::new();
        entries.insert("page".to_string(), serde_json::json!("Agenda"));
        let payload = StatePayload { entries };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"entries\""));
        assert!(json.contains("\"page\":\"Agenda\""));
    }

    #[test]
    fn host_is_local_accepts_loopback_forms() {
        for host in [
            "localhost",
            "localhost:31337",
            "127.0.0.1",
            "127.0.0.1:31337",
            "LocalHost:31337",
            "[::1]",
            "[::1]:31337",
        ] {
            assert!(host_is_local(host), "devrait accepter {host}");
        }
    }

    #[test]
    fn host_is_local_rejects_foreign_hosts() {
        for host in [
            "evil.com",
            "evil.com:31337",
            "127.0.0.1.evil.com",
            "localhost.evil.com",
            "sub.localhost",
            "0.0.0.0",
            "::1",       // IPv6 sans crochets : non reconnu
            "[::1]toto", // suffixe arbitraire apres le ] (ex-bypass)
            "[::1]evil.com",
            "[::1]:abc",     // port non numerique
            "[::1]:",        // port vide
            "127.0.0.1:abc", // port non numerique
            "",
        ] {
            assert!(!host_is_local(host), "devrait rejeter {host:?}");
        }
    }
}
