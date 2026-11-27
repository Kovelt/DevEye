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
use tokio::time::{timeout, timeout_at, Instant};
use tokio_tungstenite::{tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream};
use tracing::{debug, info, warn};

use crate::error::CdpError;

/// Délai max d'attente d'une réponse à une commande CDP déjà envoyée : le
/// navigateur peut ne jamais répondre (page plantée, target détachée sans
/// event `detachedFromTarget`...) — sans borne, `rx.await` dans
/// `send_and_wait` tiendrait indéfiniment le gate MCP (audit A1).
const CDP_RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);

/// Délai de connexion TCP pour le GET `/json/version` (bootstrap HTTP avant
/// l'upgrade WebSocket).
const CDP_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Délai total (connexion + réponse) du GET `/json/version` : un navigateur
/// qui accepte la connexion TCP mais ne répond jamais bloquerait `connect()`
/// pour toujours sans cette borne (audit A2).
const CDP_HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Délai max pour l'upgrade WebSocket vers l'endpoint browser-level ;
/// `tokio_tungstenite::connect_async` n'a pas de timeout interne (audit A2).
const CDP_WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

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
    /// Filtre de sélection de la page cible (configurable ; défaut permissif).
    filter: TargetFilter,
    /// Pages vues par la découverte de cibles (ordre d'apparition), pour
    /// expliquer à l'appelant pourquoi aucune ne correspond au filtre.
    seen: Mutex<Vec<SeenPage>>,
}

/// Page `type=page` observée via `Target.targetCreated` / `targetInfoChanged`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SeenPage {
    target_id: String,
    title: String,
    url: String,
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
    /// page qui matche `filter`.
    pub async fn connect(http_url: &str, filter: TargetFilter) -> Result<Self, CdpError> {
        let version_url = format!("{}/json/version", http_url.trim_end_matches('/'));
        let http = reqwest::Client::builder()
            .connect_timeout(CDP_HTTP_CONNECT_TIMEOUT)
            .timeout(CDP_HTTP_TIMEOUT)
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

        let (ws_stream, _) = timeout(
            CDP_WS_CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async(&version.ws_url),
        )
        .await
        .map_err(|_| {
            CdpError::Connect(format!(
                "WS {} : timeout de connexion ({} s)",
                version.ws_url,
                CDP_WS_CONNECT_TIMEOUT.as_secs()
            ))
        })?
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
            filter,
            seen: Mutex::new(Vec::new()),
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

        info!(url = http_url, "connexion CDP établie");
        Ok(client)
    }

    /// Envoie une commande CDP rattachée à la session page courante (flatten).
    ///
    /// Si aucune session n'est attachée au moment de l'appel, attend jusqu'à
    /// 10 s qu'une page correspondant au filtre apparaisse — utile pendant les
    /// recompilations.
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

        let result = self.send_and_wait(id, method, payload).await?;
        serde_json::from_value(result).map_err(|e| CdpError::Decode(format!("typed result : {e}")))
    }

    /// Envoie une commande CDP browser-level (sans `sessionId`).
    pub(crate) async fn send_browser<R, P>(&self, method: &str, params: P) -> Result<R, CdpError>
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

        let result = self.send_and_wait(id, method, payload).await?;
        serde_json::from_value(result).map_err(|e| CdpError::Decode(format!("typed result : {e}")))
    }

    async fn send_and_wait(
        &self,
        id: u64,
        method: &str,
        payload: String,
    ) -> Result<Value, CdpError> {
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(id, tx);
        self.inner
            .cmd_tx
            .send(Message::Text(payload))
            .map_err(|e| CdpError::Send(e.to_string()))?;
        debug!(id, method, "commande CDP envoyée");
        let sent_at = Instant::now();
        match timeout(CDP_RESPONSE_TIMEOUT, rx).await {
            Ok(Ok(Ok(v))) => {
                debug!(
                    id,
                    elapsed_ms = sent_at.elapsed().as_millis() as u64,
                    "réponse CDP reçue"
                );
                Ok(v)
            }
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(_)) => Err(CdpError::Receive("flux WebSocket fermé".into())),
            Err(_) => {
                // Timeout : retirer l'entrée 'pending' (sinon fuite mémoire —
                // la HashMap grossirait indéfiniment si le navigateur reste
                // muet). CdpError::Receive se mappe vers PortError::Network,
                // ce qui déclenche le fallback CDP->SDK du mode auto (voulu).
                self.inner.pending.lock().await.remove(&id);
                warn!(id, method, "timeout en attente de la réponse CDP (20 s)");
                Err(CdpError::Receive(
                    "timeout en attente de la réponse CDP (20 s)".into(),
                ))
            }
        }
    }

    /// Vrai si le WebSocket browser-level est fermé : ce client est mort
    /// pour de bon (il faut en reconnecter un via [`CdpClient::connect`]).
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    async fn wait_attached(&self) -> Result<SessionInfo, CdpError> {
        // Deadline commune a toutes les iterations de la boucle : 10 s au total.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
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
            match timeout_at(deadline, notified).await {
                Ok(()) => {
                    // Re-verifier apres la notification : un hot-reload peut
                    // avoir efface la session entre notify_waiters() et cette
                    // lecture. Si elle est vide on reboucle jusqu'a la deadline.
                    if let Some(s) = self.inner.session.lock().await.clone() {
                        return Ok(s);
                    }
                    // session vide apres notification (race hot-reload) : reboucle.
                }
                Err(_) => {
                    let seen = self.inner.seen.lock().await.clone();
                    return Err(CdpError::Connect(no_target_message(
                        &self.inner.filter,
                        &seen,
                    )));
                }
            }
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
        // C'est le seul signal de fermeture — `is_closed()` fait le reste
        // (LazyCdp::client() reconnecte au prochain appel, cf. cdp.rs).
        warn!("WebSocket CDP fermé — le client sera reconnecté au prochain appel");
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
            record_seen(inner, &p.target_info).await;
            if !inner.filter.matches(&p.target_info) {
                return;
            }
            // Filtre ouvert : la cible est la première page rencontrée ; les
            // suivantes (fenêtres secondaires, popups) ne la détrônent pas.
            if inner.filter.is_open() && inner.session.lock().await.is_some() {
                return;
            }
            attach_to(inner, &p.target_info.target_id);
        }
        // Une page `about:blank` à sa création change d'URL en navigant (boot
        // d'une webview) : on la rattrape ici si rien n'est encore attaché.
        "Target.targetInfoChanged" => {
            let Ok(p) = serde_json::from_value::<TargetCreatedParams>(params) else {
                return;
            };
            record_seen(inner, &p.target_info).await;
            if inner.filter.matches(&p.target_info) && inner.session.lock().await.is_none() {
                attach_to(inner, &p.target_info.target_id);
            }
        }
        "Target.attachedToTarget" => {
            let Ok(p) = serde_json::from_value::<AttachedToTargetParams>(params) else {
                return;
            };
            if !inner.filter.matches(&p.target_info) {
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
            inner
                .seen
                .lock()
                .await
                .retain(|s| s.target_id != p.target_id);
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

async fn record_seen(inner: &Arc<CdpInner>, info: &TargetInfo) {
    if info.kind != "page" {
        return;
    }
    let page = SeenPage {
        target_id: info.target_id.clone(),
        title: info.title.clone(),
        url: info.url.clone(),
    };
    let mut seen = inner.seen.lock().await;
    match seen.iter_mut().find(|s| s.target_id == page.target_id) {
        Some(existing) => *existing = page,
        None => seen.push(page),
    }
}

/// Nombre maximal de caractères d'un titre ou d'une URL cités dans une erreur.
const SEEN_FIELD_MAX_CHARS: usize = 60;
/// Nombre maximal de pages citées dans une erreur.
const SEEN_PAGES_MAX: usize = 10;

fn truncate_chars(text: &str) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(SEEN_FIELD_MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// Message d'erreur quand aucune page ne correspond au filtre : il liste les
/// pages vues (titre et URL tronqués) pour que l'agent corrige son filtre.
fn no_target_message(filter: &TargetFilter, seen: &[SeenPage]) -> String {
    let mut msg = format!(
        "timeout en attente d'une page CDP attachable (10 s) : aucune page ne correspond au filtre ({})",
        filter.describe()
    );
    if seen.is_empty() {
        msg.push_str(" ; aucune page vue par le navigateur");
    } else {
        msg.push_str(" ; pages vues : ");
        let shown: Vec<String> = seen
            .iter()
            .take(SEEN_PAGES_MAX)
            .map(|p| {
                format!(
                    "« {} » ({})",
                    truncate_chars(&p.title),
                    truncate_chars(&p.url)
                )
            })
            .collect();
        msg.push_str(&shown.join(", "));
        if seen.len() > SEEN_PAGES_MAX {
            msg.push_str(&format!(" … (+{})", seen.len() - SEEN_PAGES_MAX));
        }
    }
    msg.push_str(
        ". Ajuster --cdp-target-filter-url / --cdp-target-filter-title, ou --cdp-accept-blank si la page est encore about:blank",
    );
    msg
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

/// Filtre de sélection de la page CDP cible.
///
/// Sur un target `type=page` : `about:blank` n'est accepté que si
/// `accept_blank` ; sinon, **sans critère** (`url_contains` et `title_contains`
/// à `None`) toute page est acceptée (filtre ouvert) ; avec critère(s), il faut
/// que l'URL contienne `url_contains` **ou** que le titre contienne
/// `title_contains`. Comparaisons insensibles à la casse. Configurable depuis
/// la CLI (`--cdp-target-filter-url` / `--cdp-target-filter-title` /
/// `--cdp-accept-blank`) — **aucun nom d'app ni de framework n'est codé en
/// dur** ici (règle de découplage, cf. `architecture.md`).
///
/// `Default` est le filtre **ouvert** : la première page qui n'est pas
/// `about:blank` est la cible (ADR-0014). Si plusieurs pages sont ouvertes,
/// cibler explicitement avec un critère.
#[derive(Clone, Debug, Default)]
pub struct TargetFilter {
    /// Sous-chaîne attendue dans l'URL de la page (insensible à la casse).
    pub url_contains: Option<String>,
    /// Sous-chaîne attendue dans le titre de la page (insensible à la casse).
    pub title_contains: Option<String>,
    /// Accepte les pages `about:blank` (transitoire d'une webview au boot).
    pub accept_blank: bool,
}

impl TargetFilter {
    /// Vrai si aucun critère d'URL ni de titre n'est posé (filtre ouvert).
    pub fn is_open(&self) -> bool {
        self.url_contains.is_none() && self.title_contains.is_none()
    }

    /// Description lisible du filtre, pour les messages d'erreur.
    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(u) = &self.url_contains {
            parts.push(format!("url contient « {u} »"));
        }
        if let Some(t) = &self.title_contains {
            parts.push(format!("titre contient « {t} »"));
        }
        if parts.is_empty() {
            parts.push("première page non about:blank".to_string());
        }
        if self.accept_blank {
            parts.push("about:blank accepté".to_string());
        }
        parts.join(" ou ")
    }

    /// Vrai si `t` est une page acceptée par ce filtre.
    ///
    /// 1. Rejette tout ce qui n'est pas `kind == "page"`.
    /// 2. `about:blank` : accepté si et seulement si `accept_blank`.
    /// 3. Filtre ouvert (aucun critère) : accepte toute autre page.
    /// 4. Sinon : `url_contains` apparaît dans l'URL **ou** `title_contains`
    ///    apparaît dans le titre (insensible à la casse).
    fn matches(&self, t: &TargetInfo) -> bool {
        if t.kind != "page" {
            return false;
        }
        let url = t.url.to_lowercase();
        if url == "about:blank" {
            return self.accept_blank;
        }
        if self.is_open() {
            return true;
        }
        let title = t.title.to_lowercase();
        let url_ok = self
            .url_contains
            .as_deref()
            .map(|s| url.contains(&s.to_lowercase()))
            .unwrap_or(false);
        let title_ok = self
            .title_contains
            .as_deref()
            .map(|s| title.contains(&s.to_lowercase()))
            .unwrap_or(false);
        url_ok || title_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicU64},
        Arc,
    };
    use tokio::sync::{mpsc, oneshot, Mutex, Notify};

    // -------------------------------------------------------------------------
    // Helper : construit un CdpInner sans WebSocket reel (cmd_rx abandonne).
    // -------------------------------------------------------------------------
    fn make_inner() -> Arc<CdpInner> {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        Arc::new(CdpInner {
            next_id: AtomicU64::new(1),
            cmd_tx,
            pending: Mutex::new(std::collections::HashMap::new()),
            session: Mutex::new(None),
            attached: Notify::new(),
            closed: AtomicBool::new(false),
            filter: TargetFilter::default(),
            seen: Mutex::new(Vec::new()),
        })
    }

    // -------------------------------------------------------------------------
    // dispatch_response
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn dispatch_response_routes_result_to_oneshot() {
        let inner = make_inner();
        let (tx, rx) = oneshot::channel();
        inner.pending.lock().await.insert(42, tx);

        let msg = serde_json::json!({ "id": 42, "result": { "foo": "bar" } });
        dispatch_response(&inner, 42, msg).await;

        let result = rx.await.expect("sender must not be dropped");
        let v = result.expect("should be Ok");
        assert_eq!(v, serde_json::json!({ "foo": "bar" }));
    }

    #[tokio::test]
    async fn dispatch_response_routes_error_to_oneshot() {
        let inner = make_inner();
        let (tx, rx) = oneshot::channel();
        inner.pending.lock().await.insert(7, tx);

        let msg = serde_json::json!({
            "id": 7,
            "error": { "code": -32601, "message": "Method not found" }
        });
        dispatch_response(&inner, 7, msg).await;

        let result = rx.await.expect("sender must not be dropped");
        assert!(
            matches!(result, Err(CdpError::Remote { code: -32601, .. })),
            "expected Remote error"
        );
    }

    #[tokio::test]
    async fn dispatch_response_ignores_unknown_id() {
        // No pending entry for id 99 : must not panic.
        let inner = make_inner();
        let msg = serde_json::json!({ "id": 99, "result": {} });
        dispatch_response(&inner, 99, msg).await;
        // No assertion needed — absence of panic is the contract.
    }

    // -------------------------------------------------------------------------
    // handle_event
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn handle_event_attached_to_target_sets_session() {
        let inner = make_inner();
        let params = serde_json::json!({
            "sessionId": "sess-abc",
            "targetInfo": {
                "targetId": "tgt-001",
                "type": "page",
                "title": "dioxus app",
                "url": "http://dioxus.index.html/"
            }
        });
        handle_event(&inner, "Target.attachedToTarget", params).await;

        let session = inner.session.lock().await.clone();
        assert!(
            session.is_some(),
            "session should be set after attachedToTarget"
        );
        let s = session.unwrap();
        assert_eq!(s.session_id, "sess-abc");
        assert_eq!(s.target_id, "tgt-001");
    }

    #[tokio::test]
    async fn handle_event_attached_non_matching_target_ignored() {
        // Target type "worker" does not match the default filter (page only).
        let inner = make_inner();
        let params = serde_json::json!({
            "sessionId": "sess-worker",
            "targetInfo": {
                "targetId": "tgt-w",
                "type": "worker",
                "title": "",
                "url": ""
            }
        });
        handle_event(&inner, "Target.attachedToTarget", params).await;

        assert!(
            inner.session.lock().await.is_none(),
            "session must stay None for non-matching targets"
        );
    }

    #[tokio::test]
    async fn handle_event_detached_clears_matching_session() {
        let inner = make_inner();
        *inner.session.lock().await = Some(SessionInfo {
            session_id: "sess-abc".into(),
            target_id: "tgt-001".into(),
        });
        let params = serde_json::json!({ "sessionId": "sess-abc" });
        handle_event(&inner, "Target.detachedFromTarget", params).await;

        assert!(
            inner.session.lock().await.is_none(),
            "session should be cleared after detachedFromTarget"
        );
    }

    #[tokio::test]
    async fn handle_event_detached_ignores_other_session() {
        // Detach from a different sessionId must not clear our session.
        let inner = make_inner();
        *inner.session.lock().await = Some(SessionInfo {
            session_id: "sess-ours".into(),
            target_id: "tgt-001".into(),
        });
        let params = serde_json::json!({ "sessionId": "sess-other" });
        handle_event(&inner, "Target.detachedFromTarget", params).await;

        assert!(
            inner.session.lock().await.is_some(),
            "our session must not be cleared by an unrelated detach"
        );
    }

    fn page(title: &str, url: &str) -> TargetInfo {
        TargetInfo {
            target_id: "x".into(),
            kind: "page".into(),
            title: title.into(),
            url: url.into(),
        }
    }

    #[test]
    fn default_filter_accepts_any_non_blank_page() {
        let f = TargetFilter::default();
        assert!(f.matches(&page("Mon app", "http://localhost:8080/")));
        assert!(f.matches(&page("", "http://app.local/index.html")));
        assert!(f.matches(&page("Dioxus app", "http://dioxus.index.html/")));
    }

    #[test]
    fn default_filter_rejects_about_blank() {
        let f = TargetFilter::default();
        assert!(f.is_open());
        assert!(!f.accept_blank);
        assert!(!f.matches(&page("", "about:blank")));
    }

    #[test]
    fn default_filter_rejects_service_worker() {
        let mut t = page("Mon app", "http://localhost:8080/");
        t.kind = "service_worker".into();
        assert!(!TargetFilter::default().matches(&t));
    }

    #[test]
    fn open_filter_accepts_blank_only_when_asked() {
        let filter = TargetFilter {
            accept_blank: true,
            ..TargetFilter::default()
        };
        assert!(filter.matches(&page("", "about:blank")));
        assert!(filter.matches(&page("Mon app", "http://localhost:8080/")));
    }

    #[test]
    fn filter_url_contains_matches_wasm_page() {
        let filter = TargetFilter {
            url_contains: Some("localhost:5000".to_string()),
            title_contains: None,
            accept_blank: false,
        };
        assert!(filter.matches(&page("Mon app", "http://localhost:5000/")));
        // accept_blank=false : la page transitoire about:blank est rejetée.
        assert!(!filter.matches(&page("", "about:blank")));
        // une autre URL ne matche pas.
        assert!(!filter.matches(&page("Mon app", "http://localhost:9999/")));
    }

    #[test]
    fn filter_title_contains_is_case_insensitive() {
        let filter = TargetFilter {
            url_contains: None,
            title_contains: Some("Mon App".to_string()),
            accept_blank: false,
        };
        // filtre "Mon App" (majuscules) doit matcher un titre "mon app".
        assert!(filter.matches(&page("mon app", "http://localhost:5000/")));
    }

    #[test]
    fn filter_with_criteria_accepts_blank_when_asked() {
        let filter = TargetFilter {
            url_contains: Some("localhost:5000".to_string()),
            title_contains: None,
            accept_blank: true,
        };
        assert!(filter.matches(&page("", "about:blank")));
        assert!(!filter.matches(&page("Autre", "http://localhost:9999/")));
    }

    // Filtre par url_contains seul (title_contains=None) : seules les pages
    // dont l'URL contient la sous-chaîne sont acceptées.
    #[test]
    fn filter_url_only_rejects_title_match_without_url_match() {
        let filter = TargetFilter {
            url_contains: Some("myapp".to_string()),
            title_contains: None,
            accept_blank: false,
        };
        assert!(filter.matches(&page("Anything", "http://myapp.local/")));
        assert!(!filter.matches(&page("myapp", "http://other.local/")));
    }

    // Filtre par title_contains seul (url_contains=None) : seules les pages
    // dont le titre contient la sous-chaîne sont acceptées.
    #[test]
    fn filter_title_only_rejects_url_match_without_title_match() {
        let filter = TargetFilter {
            url_contains: None,
            title_contains: Some("myapp".to_string()),
            accept_blank: false,
        };
        assert!(filter.matches(&page("MyApp Dashboard", "http://other.local/")));
        assert!(!filter.matches(&page("Unrelated", "http://myapp.local/")));
    }

    // -------------------------------------------------------------------------
    // Erreur « aucune cible » et suivi des pages vues
    // -------------------------------------------------------------------------

    fn seen(title: &str, url: &str) -> SeenPage {
        SeenPage {
            target_id: "x".into(),
            title: title.into(),
            url: url.into(),
        }
    }

    #[test]
    fn no_target_message_lists_seen_pages() {
        let filter = TargetFilter {
            url_contains: Some("zzz".to_string()),
            ..TargetFilter::default()
        };
        let msg = no_target_message(
            &filter,
            &[
                seen("Accueil", "http://localhost:8080/"),
                seen("Autre", "http://localhost:9000/"),
            ],
        );
        assert!(msg.contains("url contient « zzz »"), "{msg}");
        assert!(
            msg.contains("« Accueil » (http://localhost:8080/)"),
            "{msg}"
        );
        assert!(msg.contains("« Autre » (http://localhost:9000/)"), "{msg}");
        assert!(msg.contains("--cdp-target-filter-url"), "{msg}");
    }

    #[test]
    fn no_target_message_truncates_long_fields() {
        let long_url = format!("http://localhost/{}", "a".repeat(200));
        let msg = no_target_message(
            &TargetFilter::default(),
            &[seen(&"t".repeat(100), &long_url)],
        );
        assert!(msg.contains('…'), "{msg}");
        assert!(!msg.contains(&"a".repeat(100)), "{msg}");
    }

    #[test]
    fn no_target_message_without_pages_says_so() {
        let msg = no_target_message(&TargetFilter::default(), &[]);
        assert!(msg.contains("aucune page vue"), "{msg}");
        assert!(msg.contains("première page non about:blank"), "{msg}");
    }

    #[tokio::test]
    async fn target_created_records_seen_pages_and_destroy_forgets_them() {
        let inner = make_inner();
        let params = |id: &str, url: &str| json!({ "targetInfo": { "targetId": id, "type": "page", "title": "T", "url": url } });
        handle_event(&inner, "Target.targetCreated", params("p1", "http://a/")).await;
        handle_event(&inner, "Target.targetCreated", params("p2", "about:blank")).await;
        handle_event(
            &inner,
            "Target.targetInfoChanged",
            params("p2", "http://b/"),
        )
        .await;
        let seen_now = inner.seen.lock().await.clone();
        assert_eq!(seen_now.len(), 2);
        assert_eq!(seen_now[1].url, "http://b/");
        handle_event(
            &inner,
            "Target.targetDestroyed",
            json!({ "targetId": "p1" }),
        )
        .await;
        assert_eq!(inner.seen.lock().await.len(), 1);
    }
}
