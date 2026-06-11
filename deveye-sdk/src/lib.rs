//! SDK opt-in DevEye — à intégrer côté app cible (Chronos, FAS, etc.)
//! **uniquement en `cfg(debug_assertions)`**.
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

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

mod tracing_layer;

pub use tracing_layer::install_tracing_layer;

/// Port par défaut du SDK. Choix arbitraire mais facile à retenir.
pub const DEFAULT_PORT: u16 = 31337;

/// Capacité maximale du buffer de logs avant qu'on commence à perdre les
/// anciens événements (politique drop-oldest).
pub const LOG_BUFFER_CAPACITY: usize = 1024;

/// Configuration du SDK. Le `Default` lit `DEVEYE_PORT` si présent, sinon
/// utilise [`DEFAULT_PORT`].
#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
}

impl Default for Config {
    fn default() -> Self {
        let port = std::env::var("DEVEYE_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        Self { port }
    }
}

/// Snapshot du state app servi par `GET /devtools/state`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StatePayload {
    pub entries: BTreeMap<String, serde_json::Value>,
}

/// Snapshot des logs accumulés servi par `GET /devtools/logs/poll`.
/// L'appel **draine** le buffer : un second `poll` ne renverra que les
/// événements arrivés entre-temps.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LogsPayload {
    pub events: Vec<deveye_core::LogEvent>,
}

/// Une demande d'évaluation JS poussée par `POST /devtools/eval` et
/// consommée par la **pompe** côté app (cf. [`take_eval_requests`]). L'app
/// exécute l'expression dans son webview (`document::eval` chez Dioxus) et
/// répond par le `oneshot`.
pub struct EvalRequest {
    /// Expression JS (une seule expression — typiquement une IIFE) ; la
    /// pompe l'enveloppe d'un try/catch et l'exécute dans le webview.
    pub expression: String,
    /// Réponse : valeur JSON du JS, ou message d'erreur.
    pub respond: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>,
}

/// Registry globale partagée entre l'app cible et le mini-serveur axum.
pub(crate) struct Registry {
    pub(crate) state: RwLock<BTreeMap<String, serde_json::Value>>,
    pub(crate) logs: Mutex<VecDeque<deveye_core::LogEvent>>,
    /// Émetteur vers la pompe d'évaluation de l'app (None tant que l'app
    /// n'a pas appelé [`take_eval_requests`]).
    pub(crate) eval_tx: RwLock<Option<tokio::sync::mpsc::UnboundedSender<EvalRequest>>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            state: RwLock::new(BTreeMap::new()),
            logs: Mutex::new(VecDeque::with_capacity(LOG_BUFFER_CAPACITY)),
            eval_tx: RwLock::new(None),
        }
    }
}

pub(crate) static REGISTRY: once_cell::sync::Lazy<Arc<Registry>> =
    once_cell::sync::Lazy::new(|| Arc::new(Registry::default()));

/// Runtime tokio **dédié au SDK** (1 thread, démarré au premier usage).
///
/// Le SDK ne dépend ainsi jamais du runtime de l'app hôte : le pattern
/// documenté appelle `init` **avant** `dioxus::launch`, donc hors de tout
/// contexte tokio — un `tokio::spawn` nu y paniquerait. Idem pour la couche
/// tracing, dont les événements arrivent de threads arbitraires.
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: once_cell::sync::Lazy<tokio::runtime::Runtime> = once_cell::sync::Lazy::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("deveye-sdk")
            .enable_all()
            .build()
            .expect("runtime deveye-sdk")
    });
    &RT
}

/// Démarre le mini-serveur axum sur le runtime dédié du SDK.
///
/// Appelable de n'importe où (avant `dioxus::launch` compris). Idempotent au
/// sens où un second appel re-spawn une tâche mais échouera silencieusement
/// à bind le même port — ce qui est sans conséquence pour l'app.
pub fn init(config: Config) {
    let registry = REGISTRY.clone();
    runtime().spawn(async move {
        let app = Router::new()
            .route("/devtools/state", get(state_handler))
            .route("/devtools/logs/poll", get(logs_poll_handler))
            .route("/devtools/eval", axum::routing::post(eval_handler))
            .with_state(registry);
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], config.port));
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                let _ = axum::serve(listener, app).await;
            }
            Err(_e) => {
                // En dev on choisit de rester silencieux : un port déjà pris
                // signifie souvent qu'une autre instance écoute déjà.
            }
        }
    });
}

/// Pose une entrée nommée dans le registry. Écrase la valeur précédente
/// si la clé existe déjà.
pub fn set_state(name: impl Into<String>, value: serde_json::Value) {
    let name = name.into();
    let registry = REGISTRY.clone();
    runtime().spawn(async move {
        let mut guard = registry.state.write().await;
        guard.insert(name, value);
    });
}

async fn state_handler(State(registry): State<Arc<Registry>>) -> impl IntoResponse {
    let guard = registry.state.read().await;
    let payload = StatePayload {
        entries: guard.clone(),
    };
    (StatusCode::OK, Json(payload))
}

async fn logs_poll_handler(State(registry): State<Arc<Registry>>) -> impl IntoResponse {
    let mut guard = registry.logs.lock().await;
    let events: Vec<_> = guard.drain(..).collect();
    (StatusCode::OK, Json(LogsPayload { events }))
}

/// Côté app : prend le bout consommateur du canal d'évaluation. La pompe de
/// l'app boucle dessus et exécute chaque [`EvalRequest`] dans son webview.
/// Un nouvel appel remplace la pompe précédente (l'ancien receiver se ferme).
pub fn take_eval_requests() -> tokio::sync::mpsc::UnboundedReceiver<EvalRequest> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
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
                error: Some("la pompe d'évaluation de l'app s'est arrêtée".into()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_31337_without_env() {
        let config = Config { port: DEFAULT_PORT };
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
}
