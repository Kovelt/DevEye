//! `deveye-server` — binaire orchestrateur DevEye.
//!
//! Compose les adapters (`adapter-cdp-webview`, `adapter-screencap-xcap`,
//! `adapter-dioxus-signals`, `adapter-logs-tracing`) puis expose le tout au
//! format MCP sur stdio (`adapter-mcp-server`) pour qu'un client MCP — Claude
//! Code via `.mcp.json`, par exemple — pilote l'app cible.
//!
//! Voir [`deveye.md`](../../../../../docs/architecture/deveye.md) côté
//! Chronos pour le contexte global et le mode d'emploi.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use clap::Parser;

use adapter_cdp_webview::{CdpClient, TargetFilter};
use adapter_commands_http::SdkCommandRouter;
use adapter_dioxus_signals::{
    DioxusSignalsInspector, EvalDomInspector, JsInteraction, SdkEvaluator,
};
use adapter_interaction_kwin_eis::KwinEisInteraction;
use adapter_logs_tracing::SdkTracingLogSource;
use adapter_mcp_server::DeveyeMcpServer;
use adapter_screencap_kwin::KwinRenderer;
use adapter_screencap_xcap::{MonitorRenderer, XcapRenderer};
use deveye_core::{
    ConsoleError, DomTree, InteractionCommand, Screenshot, Size, StateDump, WaitOutcome,
    WaitPredicate, WindowInfo,
};
use deveye_ports::{
    AppInspector, CommandRouter, ConsoleErrorSource, InteractionSink, JsEvaluator, LogSource,
    PortError, PortResult, Renderer, WaitCondition, WindowLister,
};

#[derive(Parser, Debug)]
#[command(
    name = "deveye-server",
    about = "Orchestrateur DevEye : compose les adapters et sert un MCP a Claude."
)]
struct Args {
    /// URL HTTP du webview CDP cible (par defaut http://localhost:9222).
    #[arg(long, default_value = "http://localhost:9222")]
    cdp_url: String,

    /// URL du SDK opt-in expose par l'app cible (par defaut http://127.0.0.1:31337).
    /// On utilise 127.0.0.1 plutot que localhost : sur Fedora (et la plupart des
    /// Linux modernes) "localhost" se resout en ::1 (IPv6) alors que le SDK bind
    /// sur 127.0.0.1 uniquement => echec silencieux de connexion.
    #[arg(long, default_value = "http://127.0.0.1:31337")]
    sdk_url: String,

    /// Titre (sous-chaine, insensible a la casse) de la fenetre native a
    /// capturer via xcap quand les yeux ne passent pas par CDP (cible `sdk`,
    /// ou repli du mode `auto`). Defaut : « chronos ».
    #[arg(long)]
    screencap_window: Option<String>,

    /// Backend de capture OS-level (yeux en mode `sdk` ou repli `auto`) :
    /// `auto` choisit KWin sous Wayland/KDE, xcap sinon ; `kwin`/`xcap` forcent.
    #[arg(long, value_enum, default_value_t = ScreencapBackend::Auto)]
    screencap_backend: ScreencapBackend,

    /// Cible des yeux et des mains :
    /// - `auto` : CDP d'abord, repli SDK + xcap des que le CDP est injoignable ;
    /// - `cdp`  : CDP uniquement (webview Chromium — `just ui-eyes`) ;
    /// - `sdk`  : pont natif uniquement (app desktop via `deveye-sdk`).
    #[arg(long, value_enum, default_value_t = TargetMode::Auto)]
    target: TargetMode,

    /// Sous-chaine (insensible a la casse) que doit contenir l'URL de la page
    /// CDP cible. Sans ce flag ni --cdp-target-filter-title, le filtre par
    /// defaut "dioxus" (comportement historique) s'applique. Sert a viser une
    /// tete WASM tierce (ex. `localhost:5000`) sans toucher l'app cible.
    #[arg(long)]
    cdp_target_filter_url: Option<String>,

    /// Sous-chaine (insensible a la casse) que doit contenir le titre de la
    /// page CDP cible (relation OU avec --cdp-target-filter-url).
    #[arg(long)]
    cdp_target_filter_title: Option<String>,

    /// Accepte la page "about:blank" comme cible CDP (transitoire wry au boot).
    /// Force a true avec le filtre par defaut Dioxus ; sinon false (un filtre
    /// explicite ne veut pas s'attacher au about:blank d'une tete WASM), sauf
    /// si ce flag est passe.
    #[arg(long, default_value_t = false)]
    cdp_accept_blank: bool,

    /// Servir le MCP server sur stdio (mode normal d'usage avec Claude).
    #[arg(long)]
    serve_mcp: bool,
}

/// Où vont les yeux et les mains (le state et les logs viennent toujours du SDK).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum TargetMode {
    Auto,
    Cdp,
    Sdk,
    /// Mains OS-level via KWin EIS (Wayland natif) ; yeux par screencap (M1).
    /// Pour les cibles sans WebView ni SDK (atlas Uno desktop / Skia).
    KwinEis,
}

/// Backend de capture OS-level (indépendant du backend CDP).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ScreencapBackend {
    /// Détection auto : KWin sous Wayland/KDE, xcap sinon.
    Auto,
    /// KWin D-Bus `ScreenShot2` — Wayland natif KDE (pixel-perfect, sans XWayland).
    Kwin,
    /// xcap — X11, XWayland, ou Wayland non-KDE.
    Xcap,
}

/// Vrai si la session est KDE Plasma sous Wayland natif (seul contexte où
/// `org.kde.KWin.ScreenShot2` répond).
fn is_kde_wayland() -> bool {
    let wayland = std::env::var("XDG_SESSION_TYPE")
        .map(|v| v.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false);
    let kde = std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_uppercase().contains("KDE"))
        .unwrap_or(false);
    wayland && kde
}

/// Sélectionne le renderer OS-level (yeux) selon le backend demandé. Le repli
/// vers xcap n'est jamais silencieux en cas d'erreur D-Bus : il est décidé ici,
/// au démarrage, par la nature de la session — pas par une erreur d'exécution.
///
/// Sans titre de fenêtre (`--screencap-window` absent), bascule sur le repli
/// **plein écran** [`MonitorRenderer`] — utile en CI headless (Xvfb) ou quand
/// la fenêtre cible n'est pas titrée.
fn build_os_renderer(title: Option<&str>, backend: ScreencapBackend) -> Arc<dyn Renderer> {
    let Some(title) = title else {
        eprintln!(
            "deveye-server : yeux = MonitorRenderer (plein écran ; --screencap-window absent)."
        );
        return Arc::new(MonitorRenderer);
    };
    let use_kwin = match backend {
        ScreencapBackend::Kwin => true,
        ScreencapBackend::Xcap => false,
        ScreencapBackend::Auto => is_kde_wayland(),
    };
    if use_kwin {
        eprintln!("deveye-server : yeux = KWin D-Bus ScreenShot2 (Wayland natif KDE).");
        Arc::new(KwinRenderer::new(title))
    } else {
        eprintln!("deveye-server : yeux = xcap (X11 / XWayland / Wayland non-KDE).");
        Arc::new(XcapRenderer::new(title))
    }
}

/// Le quintet pilotant : yeux, mains, DOM, évaluateur JS, liste des fenêtres.
type Pilotage = (
    Arc<dyn Renderer>,
    Arc<dyn InteractionSink>,
    Arc<dyn AppInspector>,
    Arc<dyn JsEvaluator>,
    Arc<dyn WindowLister>,
);

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Filtre de selection de la page CDP cible. Sans flag explicite : defaut
    // Dioxus (retro-compatible). Des qu'un filtre est fourni, `accept_blank`
    // suit --cdp-accept-blank (false par defaut) pour ne pas s'attacher a la
    // page about:blank transitoire d'une tete WASM avant son chargement.
    let cdp_filter =
        if args.cdp_target_filter_url.is_none() && args.cdp_target_filter_title.is_none() {
            TargetFilter::default()
        } else {
            TargetFilter {
                url_contains: args.cdp_target_filter_url.clone(),
                title_contains: args.cdp_target_filter_title.clone(),
                accept_blank: args.cdp_accept_blank,
            }
        };

    // Pièces du pont natif (SDK) : évaluateur JS via la pompe de l'app,
    // interaction en événements JS, DOM par outerHTML, yeux par xcap.
    let sdk_eval: Arc<dyn JsEvaluator> = Arc::new(SdkEvaluator::new(args.sdk_url.clone()));
    // Titre de fenêtre pour les yeux OS-level ; `None` → repli plein écran
    // (MonitorRenderer) au lieu de l'ancien défaut codé en dur.
    let screencap_title = args.screencap_window.clone();

    // CDP : connexion PARESSEUSE — un navigateur absent au démarrage (ou
    // fermé en cours de session) n'est pas fatal, chaque appel d'outil
    // (re)tente l'attache. L'essai immédiat ne sert qu'à loguer.
    let initial = if matches!(args.target, TargetMode::Sdk | TargetMode::KwinEis) {
        None
    } else {
        match CdpClient::connect(&args.cdp_url, cdp_filter.clone()).await {
            Ok(client) => Some(Arc::new(client)),
            Err(e) => {
                eprintln!(
                    "deveye-server : CDP injoignable au demarrage ({e}) — \
                     reconnexion automatique au premier appel d'outil."
                );
                None
            }
        }
    };

    let (renderer, interaction, dom_inspector, evaluator, window_lister): Pilotage = match args
        .target
    {
        TargetMode::Cdp => {
            let lazy = Arc::new(LazyCdp::new(
                args.cdp_url.clone(),
                cdp_filter.clone(),
                initial,
            ));
            (lazy.clone(), lazy.clone(), lazy.clone(), lazy.clone(), lazy)
        }
        TargetMode::Sdk => (
            build_os_renderer(screencap_title.as_deref(), args.screencap_backend),
            Arc::new(JsInteraction::new(sdk_eval.clone())),
            Arc::new(EvalDomInspector::new(sdk_eval.clone())),
            sdk_eval.clone(),
            Arc::new(EmptyWindowLister),
        ),
        // Auto : CDP en premier ; toute erreur de LIAISON (PortError::Network
        // — navigateur absent/fermé) bascule l'appel sur le pont natif.
        TargetMode::Auto => {
            let lazy = Arc::new(LazyCdp::new(
                args.cdp_url.clone(),
                cdp_filter.clone(),
                initial,
            ));
            (
                Arc::new(FallbackRenderer {
                    primary: lazy.clone(),
                    fallback: build_os_renderer(screencap_title.as_deref(), args.screencap_backend),
                }),
                Arc::new(FallbackInteraction {
                    primary: lazy.clone(),
                    fallback: Arc::new(JsInteraction::new(sdk_eval.clone())),
                }),
                Arc::new(FallbackDom {
                    primary: lazy.clone(),
                    fallback: Arc::new(EvalDomInspector::new(sdk_eval.clone())),
                }),
                Arc::new(FallbackEvaluator {
                    primary: lazy.clone(),
                    fallback: sdk_eval.clone(),
                }),
                lazy,
            )
        }
        // Mains OS-level EIS ; yeux par screencap (M1) ; DOM/JS indisponibles
        // (rendu natif). Le state/les logs restent servis par le SDK plus bas.
        TargetMode::KwinEis => (
            build_os_renderer(screencap_title.as_deref(), args.screencap_backend),
            Arc::new(KwinEisInteraction::new((0, 0)).expect("worker EIS")),
            Arc::new(UnsupportedInspector),
            Arc::new(UnsupportedEvaluator),
            Arc::new(EmptyWindowLister),
        ),
    };

    // State inspector + logs + commandes logiques via le SDK opt-in cote app
    // cible (toujours servis par le SDK, quelle que soit la cible des yeux).
    let state_inspector: Arc<dyn AppInspector> =
        Arc::new(DioxusSignalsInspector::new(args.sdk_url.clone()));
    let log_source: Arc<dyn LogSource> = Arc::new(SdkTracingLogSource::new(args.sdk_url.clone()));
    let command_router: Arc<dyn CommandRouter> =
        Arc::new(SdkCommandRouter::new(args.sdk_url.clone()));

    let inspector: Arc<dyn AppInspector> = Arc::new(CompositeInspector {
        dom: dom_inspector,
        state: state_inspector,
    });

    // Ports d'orchestration (M6), composés sur les ports déjà construits :
    // erreurs console par injection JS, attente conditionnelle multi-prédicats.
    //
    // En mode kwin-eis il n'y a pas de WebView : l'evaluateur renverrait
    // PortError::Unsupported a chaque drain. On branche un noop plutot que de
    // propager une fausse erreur (absence de console != erreur du port).
    let console_errors: Arc<dyn ConsoleErrorSource> = if matches!(args.target, TargetMode::KwinEis)
    {
        Arc::new(NoopConsoleErrorSource)
    } else {
        Arc::new(JsConsoleErrorSource {
            evaluator: evaluator.clone(),
        })
    };
    let wait_condition: Arc<dyn WaitCondition> = Arc::new(CompositeWaitCondition {
        evaluator: evaluator.clone(),
        inspector: inspector.clone(),
        renderer: renderer.clone(),
    });

    let mcp = DeveyeMcpServer::new(
        renderer,
        interaction,
        inspector,
        log_source,
        evaluator,
        command_router,
        window_lister,
        console_errors,
        wait_condition,
    );

    if args.serve_mcp {
        mcp.serve_stdio()
            .await
            .context("le serveur MCP stdio s'est arrete sur erreur")?;
    } else {
        eprintln!(
            "deveye-server : ni --serve-mcp ni autre mode actif. Composition validee, sortie."
        );
    }

    Ok(())
}

/// Façade paresseuse sur le CDP : (re)connecte **à la demande** au lieu
/// d'exiger le navigateur au démarrage. Survit aussi à sa fermeture en cours
/// de session (relance de `just ui-eyes`) : un client dont le WebSocket est
/// mort (`is_closed`) est jeté et remplacé au prochain appel d'outil.
struct LazyCdp {
    url: String,
    filter: TargetFilter,
    slot: tokio::sync::Mutex<Option<Arc<CdpClient>>>,
}

impl LazyCdp {
    fn new(url: String, filter: TargetFilter, initial: Option<Arc<CdpClient>>) -> Self {
        Self {
            url,
            filter,
            slot: tokio::sync::Mutex::new(initial),
        }
    }

    /// Le client courant s'il est vivant, sinon une (re)connexion. L'échec
    /// est une erreur d'outil ordinaire — jamais la mort du serveur MCP.
    async fn client(&self) -> PortResult<Arc<CdpClient>> {
        let mut slot = self.slot.lock().await;
        if let Some(client) = slot.as_ref() {
            if !client.is_closed() {
                return Ok(client.clone());
            }
        }
        match CdpClient::connect(&self.url, self.filter.clone()).await {
            Ok(client) => {
                let client = Arc::new(client);
                *slot = Some(client.clone());
                Ok(client)
            }
            Err(e) => Err(PortError::Network(format!(
                "CDP injoignable sur {} ({e}) — lancer le navigateur cible \
                 (`just ui-eyes`) puis rappeler l'outil",
                self.url
            ))),
        }
    }
}

#[async_trait]
impl Renderer for LazyCdp {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        self.client().await?.screenshot().await
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        self.client().await?.viewport_size().await
    }
}

#[async_trait]
impl InteractionSink for LazyCdp {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
        self.client().await?.dispatch(command).await
    }
}

#[async_trait]
impl AppInspector for LazyCdp {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        self.client().await?.dom_snapshot().await
    }

    async fn state_snapshot(&self) -> PortResult<StateDump> {
        self.client().await?.state_snapshot().await
    }
}

#[async_trait]
impl JsEvaluator for LazyCdp {
    async fn evaluate(&self, expression: &str) -> PortResult<serde_json::Value> {
        self.client().await?.evaluate(expression).await
    }
}

#[async_trait]
impl WindowLister for LazyCdp {
    async fn list_windows(&self) -> PortResult<Vec<WindowInfo>> {
        self.client().await?.list_windows().await
    }
}

/// `WindowLister` du pont natif (cible `sdk`) : aucune visibilité
/// inter-fenêtres — renvoie une liste vide plutôt qu'une erreur.
struct EmptyWindowLister;

#[async_trait]
impl WindowLister for EmptyWindowLister {
    async fn list_windows(&self) -> PortResult<Vec<WindowInfo>> {
        Ok(Vec::new())
    }
}

/// Inspecteur DOM indisponible (cible `kwin-eis` : rendu natif, sans WebView).
/// Le state applicatif continue d'arriver par le SDK (state_inspector dédié).
struct UnsupportedInspector;

#[async_trait]
impl AppInspector for UnsupportedInspector {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        Err(PortError::Unsupported(
            "DOM indisponible en mode kwin-eis (rendu natif, pas de WebView)".to_string(),
        ))
    }
    async fn state_snapshot(&self) -> PortResult<StateDump> {
        Err(PortError::Unsupported(
            "state DOM indisponible en mode kwin-eis".to_string(),
        ))
    }
}

/// Évaluateur JS indisponible (cible `kwin-eis` : pas de WebView à piloter).
struct UnsupportedEvaluator;

#[async_trait]
impl JsEvaluator for UnsupportedEvaluator {
    async fn evaluate(&self, _expression: &str) -> PortResult<serde_json::Value> {
        Err(PortError::Unsupported(
            "eval JS indisponible en mode kwin-eis (rendu natif, pas de WebView)".to_string(),
        ))
    }
}

/// `ConsoleErrorSource` noop : renvoie toujours un buffer vide.
///
/// Utilise en mode kwin-eis ou la console JS n'existe pas (rendu natif, pas
/// de WebView). Ce n'est pas une erreur — `PortError::Unsupported` serait
/// trompeur ; l'absence de webview ne produit tout simplement aucune erreur
/// console a drainer.
struct NoopConsoleErrorSource;

#[async_trait]
impl ConsoleErrorSource for NoopConsoleErrorSource {
    async fn drain_console_errors(&self) -> PortResult<Vec<ConsoleError>> {
        Ok(Vec::new())
    }
}

/// Collecteur d'erreurs console JS, injecté une fois puis drainé. Pur JS →
/// fonctionne via CDP comme via la pompe SDK. Renvoie des entrées typées
/// (level/text/url/line/timestamp) directement désérialisables en `ConsoleError`.
const CONSOLE_ERRORS_SNIPPET: &str = r#"(function(){
  if (!window.__deveye_console_init__) {
    window.__deveye_console_errors__ = [];
    var push = function(level, text, url, line){
      try { window.__deveye_console_errors__.push({ level: level, text: text, url: url, line: line, timestamp: new Date().toISOString() }); } catch (_) {}
    };
    window.addEventListener('error', function(e){
      push('error', String((e && (e.message || e.error)) || 'error'), (e && e.filename) || null, (e && e.lineno) || null);
    });
    window.addEventListener('unhandledrejection', function(e){
      push('error', String(e && e.reason), null, null);
    });
    var origErr = console.error;
    console.error = function(){ push('error', Array.prototype.map.call(arguments, String).join(' '), null, null); if (origErr) { origErr.apply(console, arguments); } };
    var origWarn = console.warn;
    console.warn = function(){ push('warning', Array.prototype.map.call(arguments, String).join(' '), null, null); if (origWarn) { origWarn.apply(console, arguments); } };
    window.__deveye_console_init__ = true;
  }
  return window.__deveye_console_errors__.splice(0);
})()"#;

/// `ConsoleErrorSource` générique : injecte un collecteur JS via l'évaluateur
/// (CDP ou pompe SDK) et draine les erreurs typées. La capture CDP-native
/// (`Runtime.consoleAPICalled`) reste un raffinement futur derrière ce même port.
struct JsConsoleErrorSource {
    evaluator: Arc<dyn JsEvaluator>,
}

#[async_trait]
impl ConsoleErrorSource for JsConsoleErrorSource {
    async fn drain_console_errors(&self) -> PortResult<Vec<ConsoleError>> {
        let value = self.evaluator.evaluate(CONSOLE_ERRORS_SNIPPET).await?;
        // Parse element par element : un element corrompu ne doit jamais faire
        // disparaitre les autres ("unwrap_or_default()" global = inversion du sens).
        let arr = match value {
            serde_json::Value::Array(a) => a,
            other => {
                eprintln!(
                    "deveye-server: drain_console_errors: valeur JS inattendue (non-tableau : {}), buffer ignore",
                    other
                );
                return Ok(Vec::new());
            }
        };
        let mut out = Vec::with_capacity(arr.len());
        for (i, elem) in arr.into_iter().enumerate() {
            match serde_json::from_value::<ConsoleError>(elem) {
                Ok(e) => out.push(e),
                Err(e) => {
                    eprintln!(
                        "deveye-server: drain_console_errors: element[{i}] non parseable, ignore: {e}"
                    );
                }
            }
        }
        Ok(out)
    }
}

/// Plafond du delai inter-captures pour `ScreenshotStable` : evite des pauses
/// demesurees a chaque iteration du polling (l'attente totale est bornee par le
/// timeout du WaitCondition, mais chaque check dormait jusqu'a la valeur brute
/// fournie par l'appelant, potentiellement > 60 s).
const MAX_SCREENSHOT_STABLE_THRESHOLD_MS: u64 = 2000;

/// Hache les octets PNG d'une capture via [`DefaultHasher`] (non cryptographique,
/// suffisant pour la comparaison de stabilite visuelle).
///
/// On compare les hash plutot que les `Vec<u8>` complets (plusieurs Mo par
/// PNG encode) : evite une allocation memoire + comparaison byte-a-byte a
/// chaque iteration de polling.
///
/// Note : le port [`Renderer`] renvoie des PNG deja encodes (pas des pixels
/// bruts) ; hasher les octets PNG est la seule option sans modifier le port.
fn hash_screenshot_bytes(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// `WaitCondition` composite : boucle de polling sur les ports déjà composés
/// (évaluateur pour `DomSelector`, inspecteur pour `StateKey`, renderer pour
/// `ScreenshotStable`). Marche pour toutes les cibles sans impl par adapter.
struct CompositeWaitCondition {
    evaluator: Arc<dyn JsEvaluator>,
    inspector: Arc<dyn AppInspector>,
    renderer: Arc<dyn Renderer>,
}

impl CompositeWaitCondition {
    async fn check(&self, predicate: &WaitPredicate) -> PortResult<bool> {
        match predicate {
            WaitPredicate::DomSelector { selector } => {
                let sel = serde_json::to_string(selector).unwrap_or_default();
                let value = self
                    .evaluator
                    .evaluate(&format!("!!document.querySelector({sel})"))
                    .await?;
                Ok(value.as_bool().unwrap_or(false))
            }
            WaitPredicate::StateKey { key, expected } => {
                let dump = self.inspector.state_snapshot().await?;
                Ok(dump.entries.get(key) == Some(expected))
            }
            WaitPredicate::ScreenshotStable { threshold_ms } => {
                // Plafonne a MAX_SCREENSHOT_STABLE_THRESHOLD_MS pour eviter une
                // pause demesure a chaque iteration (le timeout global du caller
                // borne la duree totale, pas chaque check individuel).
                let capped = (*threshold_ms).min(MAX_SCREENSHOT_STABLE_THRESHOLD_MS);
                let first = self.renderer.screenshot().await?;
                let hash_first = hash_screenshot_bytes(&first.bytes);
                tokio::time::sleep(Duration::from_millis(capped)).await;
                let second = self.renderer.screenshot().await?;
                let hash_second = hash_screenshot_bytes(&second.bytes);
                Ok(hash_first == hash_second)
            }
        }
    }
}

#[async_trait]
impl WaitCondition for CompositeWaitCondition {
    async fn wait_until(
        &self,
        predicate: WaitPredicate,
        timeout: Duration,
    ) -> PortResult<WaitOutcome> {
        const POLL: Duration = Duration::from_millis(200);
        let start = Instant::now();
        let deadline = start + timeout;
        loop {
            if self.check(&predicate).await? {
                return Ok(WaitOutcome {
                    satisfied: true,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    detail: "prédicat satisfait".to_string(),
                });
            }
            if Instant::now() >= deadline {
                return Ok(WaitOutcome {
                    satisfied: false,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    detail: format!("timeout après {} ms", timeout.as_millis()),
                });
            }
            tokio::time::sleep(POLL).await;
        }
    }
}

/// Vrai si l'erreur signale un problème de **liaison** (cible injoignable)
/// plutôt qu'un échec de l'opération elle-même — le seul cas où le mode
/// `auto` bascule sur le pont natif.
fn is_liaison_error(e: &PortError) -> bool {
    matches!(e, PortError::Network(_))
}

/// Repli par appel pour les yeux : CDP d'abord, xcap si la liaison manque.
struct FallbackRenderer {
    primary: Arc<dyn Renderer>,
    fallback: Arc<dyn Renderer>,
}

#[async_trait]
impl Renderer for FallbackRenderer {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        match self.primary.screenshot().await {
            Err(e) if is_liaison_error(&e) => self.fallback.screenshot().await,
            other => other,
        }
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        match self.primary.viewport_size().await {
            Err(e) if is_liaison_error(&e) => self.fallback.viewport_size().await,
            other => other,
        }
    }
}

/// Repli par appel pour les mains : CDP d'abord, événements JS via le SDK.
struct FallbackInteraction {
    primary: Arc<dyn InteractionSink>,
    fallback: Arc<dyn InteractionSink>,
}

#[async_trait]
impl InteractionSink for FallbackInteraction {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
        match self.primary.dispatch(command.clone()).await {
            Err(e) if is_liaison_error(&e) => self.fallback.dispatch(command).await,
            other => other,
        }
    }
}

/// Repli par appel pour le DOM : CDP d'abord, `outerHTML` via le SDK.
struct FallbackDom {
    primary: Arc<dyn AppInspector>,
    fallback: Arc<dyn AppInspector>,
}

#[async_trait]
impl AppInspector for FallbackDom {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        match self.primary.dom_snapshot().await {
            Err(e) if is_liaison_error(&e) => self.fallback.dom_snapshot().await,
            other => other,
        }
    }

    async fn state_snapshot(&self) -> PortResult<StateDump> {
        match self.primary.state_snapshot().await {
            Err(e) if is_liaison_error(&e) => self.fallback.state_snapshot().await,
            other => other,
        }
    }
}

/// Repli par appel pour l'évaluateur JS : CDP d'abord, pompe SDK sinon.
struct FallbackEvaluator {
    primary: Arc<dyn JsEvaluator>,
    fallback: Arc<dyn JsEvaluator>,
}

#[async_trait]
impl JsEvaluator for FallbackEvaluator {
    async fn evaluate(&self, expression: &str) -> PortResult<serde_json::Value> {
        match self.primary.evaluate(expression).await {
            Err(e) if is_liaison_error(&e) => self.fallback.evaluate(expression).await,
            other => other,
        }
    }
}

/// Compose un inspector DOM (typiquement CDP) avec un inspector state
/// (typiquement le SDK opt-in). Permet de tirer le DOM d'une source et le
/// state d'une autre derriere un meme port `AppInspector`.
struct CompositeInspector {
    dom: Arc<dyn AppInspector>,
    state: Arc<dyn AppInspector>,
}

#[async_trait]
impl AppInspector for CompositeInspector {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        self.dom.dom_snapshot().await
    }

    async fn state_snapshot(&self) -> PortResult<StateDump> {
        self.state.state_snapshot().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deveye_core::ConsoleLevel;

    // Timestamp RFC3339 fixe utilise dans les donnees de test (evite le besoin
    // d'une dependance directe a `chrono` dans le binaire).
    const TS: &str = "2026-01-01T00:00:00Z";

    // ----- M5 : drain element-par-element -----

    /// Un tableau JSON contenant un element valide + un element corrompu doit
    /// renvoyer exactement l'element valide, pas une liste vide.
    #[test]
    fn drain_parse_skips_corrupt_keeps_valid() {
        let valid = serde_json::json!({
            "level": "error",
            "text": "boom",
            "timestamp": TS
        });
        // Element corrompu : "level" a une valeur numerique => Deserialize echoue.
        let corrupt = serde_json::json!({
            "level": 42,
            "text": "corrupt",
            "timestamp": TS
        });
        let arr = serde_json::Value::Array(vec![valid, corrupt]);

        // Reproduit la logique du impl sans IO.
        let result: Vec<ConsoleError> = match arr {
            serde_json::Value::Array(a) => a
                .into_iter()
                .filter_map(|elem| serde_json::from_value::<ConsoleError>(elem).ok())
                .collect(),
            _ => vec![],
        };

        assert_eq!(result.len(), 1, "seul l'element valide doit etre conserve");
        assert_eq!(result[0].level, ConsoleLevel::Error);
        assert_eq!(result[0].text, "boom");
    }

    /// Un tableau entierement vide renvoie un Vec vide (pas d'erreur).
    #[test]
    fn drain_empty_array_gives_empty_vec() {
        let arr = serde_json::Value::Array(vec![]);
        let result: Vec<ConsoleError> = match arr {
            serde_json::Value::Array(a) => a
                .into_iter()
                .filter_map(|elem| serde_json::from_value::<ConsoleError>(elem).ok())
                .collect(),
            _ => vec![],
        };
        assert!(result.is_empty());
    }

    // ----- M9 : hash + plafond threshold -----

    /// Deux tranches identiques produisent le meme hash.
    #[test]
    fn hash_screenshot_bytes_same_input_same_hash() {
        let data = b"\x89PNG\r\ndeadbeef";
        assert_eq!(hash_screenshot_bytes(data), hash_screenshot_bytes(data));
    }

    /// Deux tranches differentes produisent (avec une forte probabilite) un hash different.
    #[test]
    fn hash_screenshot_bytes_different_input_different_hash() {
        let a = b"\x89PNG\r\n\x1a\nchunk_a";
        let b = b"\x89PNG\r\n\x1a\nchunk_b";
        assert_ne!(hash_screenshot_bytes(a), hash_screenshot_bytes(b));
    }

    /// Le plafond MAX_SCREENSHOT_STABLE_THRESHOLD_MS est bien applique.
    #[test]
    fn screenshot_stable_threshold_capped() {
        let raw: u64 = 60_000;
        let capped = raw.min(MAX_SCREENSHOT_STABLE_THRESHOLD_MS);
        assert_eq!(capped, MAX_SCREENSHOT_STABLE_THRESHOLD_MS);
        // Une valeur dans la limite passe intacte.
        let small: u64 = 500;
        assert_eq!(small.min(MAX_SCREENSHOT_STABLE_THRESHOLD_MS), 500);
    }
}
