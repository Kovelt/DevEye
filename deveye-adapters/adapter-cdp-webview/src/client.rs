use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex, Notify};
use tokio::time::timeout;
use tokio_tungstenite::{tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream};

use crate::error::CdpError;

/// URL HTTP par défaut où le webview wry/WebView2 expose CDP quand on lance
/// l'app avec `--remote-debugging-port=9222`.
pub const DEFAULT_HTTP_URL: &str = "http://localhost:9222";

/// Représente une « target » CDP — typiquement la page principale de l'app.
#[derive(Clone, Debug, Deserialize)]
pub struct CdpTarget {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub ws_url: String,
}

/// Client CDP robuste au hot-reload de la page cible.
///
/// Plutôt qu'une WebSocket attachée à un *target page* précis (qui meurt à
/// chaque recompilation Dioxus / recréation de la webview), on s'attache au
/// **browser-level endpoint** (`/json/version` → `webSocketDebuggerUrl`) et
/// on bascule la session CDP via `Target.attachToTarget` en mode `flatten`.
///
/// Une tâche reader background écoute les events `Target.targetCreated` /
/// `Target.detachedFromTarget` et ré-attache automatiquement à la nouvelle
/// page Dioxus dès qu'elle apparaît. Les appels `call()` attendent une
/// session active (avec timeout court) — l'outil DevEye reste utilisable
/// après un hot-reload sans intervention manuelle (`/mcp reconnect`).
type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsSink = SplitSink<WsStream, Message>;
type WsRead = SplitStream<WsStream>;

#[derive(Clone)]
pub struct CdpClient {
    inner: Arc<CdpInner>,
}

struct CdpInner {
    next_id: AtomicU64,
    cmd_tx: mpsc::UnboundedSender<Message>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>,
    session: Mutex<Option<SessionInfo>>,
    attached: Notify,
    /// Vrai quand le WebSocket browser-level est mort (navigateur fermé).
    /// Permet à l'appelant de jeter ce client et d'en reconnecter un autre
    /// — la session page se ré-attache toute seule, pas le WS navigateur.
    closed: AtomicBool,
}

#[derive(Clone, Debug)]
struct SessionInfo {
    session_id: String,
    target_id: String,
}

#[derive(Debug, Deserialize)]
struct BrowserVersion {
    #[serde(rename = "webSocketDebuggerUrl")]
    ws_url: String,
}

#[derive(Debug, Deserialize)]
struct TargetInfo {
    #[serde(rename = "targetId")]
    target_id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
}

#[derive(Debug, Deserialize)]
struct TargetCreatedParams {
    #[serde(rename = "targetInfo")]
    target_info: TargetInfo,
}

#[derive(Debug, Deserialize)]
struct AttachedToTargetParams {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "targetInfo")]
    target_info: TargetInfo,
}

#[derive(Debug, Deserialize)]
struct DetachedFromTargetParams {
    #[serde(rename = "sessionId")]
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct TargetDestroyedParams {
    #[serde(rename = "targetId")]
    target_id: String,
}

impl CdpClient {
    /// Établit une session CDP browser-level vers `http_url`.
    ///
    /// Bootstrap : `GET /json/version` → WS browser → `setDiscoverTargets`
    /// → attendre `Target.attachedToTarget` (jusqu'à 5 s) pour la première
    /// page qui matche le filtre Dioxus.
    pub async fn connect(http_url: &str) -> Result<Self, CdpError> {
        let version_url = format!("{}/json/version", http_url.trim_end_matches('/'));
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| CdpError::Connect(e.to_string()))?;
        let version: BrowserVersion = http
            .get(&version_url)
            .send()
            .await
            .map_err(|e| CdpError::Connect(format!("GET {version_url} : {e}")))?
            .json()
            .await
            .map_err(|e| CdpError::Connect(format!("JSON {version_url} : {e}")))?;

        let (ws_stream, _) = tokio_tungstenite::connect_async(&version.ws_url)
            .await
            .map_err(|e| CdpError::Connect(format!("WS {} : {e}", version.ws_url)))?;
        let (sink, stream) = ws_stream.split();

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Message>();

        let inner = Arc::new(CdpInner {
            next_id: AtomicU64::new(1),
            cmd_tx,
            pending: Mutex::new(HashMap::new()),
            session: Mutex::new(None),
            attached: Notify::new(),
            closed: AtomicBool::new(false),
        });

        spawn_writer(sink, cmd_rx);
        spawn_reader(stream, inner.clone());

        let client = Self { inner };

        // Active la découverte des targets — déclenche un Target.targetCreated
        // pour chaque page déjà ouverte, ce qui amorce l'auto-attach.
        client
            .send_browser::<Value, _>(
                "Target.setDiscoverTargets",
                json!({ "discover": true, "filter": [{ "type": "page" }] }),
            )
            .await?;

        // On laisse jusqu'à 5 s pour qu'une page matche et soit attachée.
        // Si rien ne matche dans ce délai, on rend la main quand même — les
        // appels suivants ré-attendront via `wait_attached`.
        let _ = timeout(Duration::from_secs(5), client.inner.attached.notified()).await;

        Ok(client)
    }

    /// Envoie une commande CDP rattachée à la session page courante (flatten).
    ///
    /// Si aucune session n'est attachée au moment de l'appel, attend jusqu'à
    /// 10 s qu'une page Dioxus apparaisse — utile pendant les recompilations.
    pub async fn call<P, R>(&self, method: &str, params: P) -> Result<R, CdpError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let session = self.wait_attached().await?;
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = serde_json::to_string(&json!({
            "id": id,
            "sessionId": session.session_id,
            "method": method,
            "params": params,
        }))
        .map_err(|e| CdpError::Decode(format!("encode request : {e}")))?;

        let result = self.send_and_wait(id, payload).await?;
        serde_json::from_value(result).map_err(|e| CdpError::Decode(format!("typed result : {e}")))
    }

    /// Envoie une commande CDP browser-level (sans `sessionId`).
    async fn send_browser<R, P>(&self, method: &str, params: P) -> Result<R, CdpError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = serde_json::to_string(&json!({
            "id": id,
            "method": method,
            "params": params,
        }))
        .map_err(|e| CdpError::Decode(format!("encode request : {e}")))?;

        let result = self.send_and_wait(id, payload).await?;
        serde_json::from_value(result).map_err(|e| CdpError::Decode(format!("typed result : {e}")))
    }

    async fn send_and_wait(&self, id: u64, payload: String) -> Result<Value, CdpError> {
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(id, tx);
        self.inner
            .cmd_tx
            .send(Message::Text(payload))
            .map_err(|e| CdpError::Send(e.to_string()))?;
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(CdpError::Receive("flux WebSocket fermé".into())),
        }
    }

    /// Vrai si le WebSocket browser-level est fermé : ce client est mort
    /// pour de bon (il faut en reconnecter un via [`CdpClient::connect`]).
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    async fn wait_attached(&self) -> Result<SessionInfo, CdpError> {
        if let Some(s) = self.inner.session.lock().await.clone() {
            return Ok(s);
        }
        // Arme le notified AVANT de re-checker pour ne pas rater une
        // notification qui arriverait entre le check et l'await.
        let notified = self.inner.attached.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if let Some(s) = self.inner.session.lock().await.clone() {
            return Ok(s);
        }
        match timeout(Duration::from_secs(10), notified).await {
            Ok(()) => self
                .inner
                .session
                .lock()
                .await
                .clone()
                .ok_or_else(|| CdpError::Connect("attached notifié mais session vide".into())),
            Err(_) => Err(CdpError::Connect(
                "timeout en attente d'une page CDP attachable (10 s)".into(),
            )),
        }
    }
}

fn spawn_writer(mut sink: WsSink, mut cmd_rx: mpsc::UnboundedReceiver<Message>) {
    tokio::spawn(async move {
        while let Some(msg) = cmd_rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });
}

fn spawn_reader(mut stream: WsRead, inner: Arc<CdpInner>) {
    tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            let text = match msg {
                Ok(Message::Text(t)) => t,
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => continue,
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(id) = value.get("id").and_then(|v| v.as_u64()) {
                dispatch_response(&inner, id, value).await;
            } else if let Some(method) = value.get("method").and_then(|v| v.as_str()) {
                let params = value.get("params").cloned().unwrap_or(Value::Null);
                handle_event(&inner, method, params).await;
            }
        }
        // Le flux est fermé : marque le client mort puis libère les awaiters.
        inner.closed.store(true, Ordering::SeqCst);
        let mut pending = inner.pending.lock().await;
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err(CdpError::Receive("flux WebSocket fermé".into())));
        }
    });
}

async fn dispatch_response(inner: &Arc<CdpInner>, id: u64, mut value: Value) {
    let tx = match inner.pending.lock().await.remove(&id) {
        Some(tx) => tx,
        None => return,
    };
    let res = if let Some(err) = value.get("error") {
        let code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
        let message = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Err(CdpError::Remote { code, message })
    } else {
        value
            .get_mut("result")
            .map(std::mem::take)
            .map(Ok)
            .unwrap_or_else(|| Err(CdpError::Unexpected("réponse sans result ni error".into())))
    };
    let _ = tx.send(res);
}

async fn handle_event(inner: &Arc<CdpInner>, method: &str, params: Value) {
    match method {
        "Target.targetCreated" => {
            let Ok(p) = serde_json::from_value::<TargetCreatedParams>(params) else {
                return;
            };
            if !target_matches(&p.target_info) {
                return;
            }
            attach_to(inner, &p.target_info.target_id);
        }
        "Target.attachedToTarget" => {
            let Ok(p) = serde_json::from_value::<AttachedToTargetParams>(params) else {
                return;
            };
            if !target_matches(&p.target_info) {
                return;
            }
            *inner.session.lock().await = Some(SessionInfo {
                session_id: p.session_id,
                target_id: p.target_info.target_id,
            });
            inner.attached.notify_waiters();
        }
        "Target.detachedFromTarget" => {
            let Ok(p) = serde_json::from_value::<DetachedFromTargetParams>(params) else {
                return;
            };
            let mut guard = inner.session.lock().await;
            if guard
                .as_ref()
                .map(|s| s.session_id == p.session_id)
                .unwrap_or(false)
            {
                *guard = None;
            }
        }
        "Target.targetDestroyed" => {
            let Ok(p) = serde_json::from_value::<TargetDestroyedParams>(params) else {
                return;
            };
            let mut guard = inner.session.lock().await;
            if guard
                .as_ref()
                .map(|s| s.target_id == p.target_id)
                .unwrap_or(false)
            {
                *guard = None;
            }
        }
        _ => {}
    }
}

fn attach_to(inner: &Arc<CdpInner>, target_id: &str) {
    let id = inner.next_id.fetch_add(1, Ordering::Relaxed);
    let payload = serde_json::to_string(&json!({
        "id": id,
        "method": "Target.attachToTarget",
        "params": { "targetId": target_id, "flatten": true },
    }))
    .expect("serialise attachToTarget");
    // La réponse arrive en parallèle d'un event Target.attachedToTarget ;
    // c'est l'event qui pose `session`. On ignore la réponse ici.
    let _ = inner.cmd_tx.send(Message::Text(payload));
}

/// Filtre les targets CDP candidats à l'attache automatique.
///
/// Pour Chronos / Dioxus desktop, la page principale a typiquement :
/// - `type = "page"`,
/// - `url = "http://dioxus.index.html/"` (asset manganis),
/// - `title = "Dioxus app"` (ou personnalisé via la config Dioxus).
///
/// On accepte large : toute page dont l'URL ou le titre évoque Dioxus, ou
/// une `about:blank` (cas transitoire pendant le boot wry).
fn target_matches(t: &TargetInfo) -> bool {
    if t.kind != "page" {
        return false;
    }
    let title = t.title.to_lowercase();
    let url = t.url.to_lowercase();
    title.contains("dioxus")
        || title.contains("chronos")
        || url.contains("dioxus")
        || url == "about:blank"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_matches_page_dioxus_url() {
        let t = TargetInfo {
            target_id: "x".into(),
            kind: "page".into(),
            title: "Dioxus app".into(),
            url: "http://dioxus.index.html/".into(),
        };
        assert!(target_matches(&t));
    }

    #[test]
    fn target_matches_rejects_service_worker() {
        let t = TargetInfo {
            target_id: "x".into(),
            kind: "service_worker".into(),
            title: "Dioxus app".into(),
            url: "http://dioxus.index.html/".into(),
        };
        assert!(!target_matches(&t));
    }

    #[test]
    fn target_matches_accepts_about_blank_page() {
        let t = TargetInfo {
            target_id: "x".into(),
            kind: "page".into(),
            title: "".into(),
            url: "about:blank".into(),
        };
        assert!(target_matches(&t));
    }
}
