//! `deveye-server` — binaire orchestrateur DevEye.
//!
//! Compose les adapters (`adapter-cdp-webview`, `adapter-screencap-xcap`,
//! `adapter-dioxus-signals`, `adapter-logs-tracing`) puis expose le tout au
//! format MCP sur stdio (`adapter-mcp-server`) pour qu'un client MCP — Claude
//! Code via `.mcp.json`, par exemple — pilote l'app cible.
//!
//! Voir [`deveye.md`](../../../../../docs/architecture/deveye.md) côté
//! Chronos pour le contexte global et le mode d'emploi.

use std::sync::Arc;

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
use adapter_mcp_server::{DeveyeMcpServer, DeveyePorts};
use adapter_screencap_kwin::KwinRenderer;
use adapter_screencap_xcap::{MonitorRenderer, XcapRenderer};
use deveye_core::{DomTree, InteractionCommand, Screenshot, Size, StateDump, WindowInfo};
use deveye_ports::{
    AppInspector, CommandRouter, ConsoleErrorSource, InteractionSink, JsEvaluator, LogSource,
    PortError, PortResult, Renderer, WaitCondition, WindowLister,
};

mod orchestration;
use orchestration::{
    CompositeInspector, CompositeWaitCondition, FallbackDom, FallbackEvaluator,
    FallbackInteraction, FallbackRenderer, JsConsoleErrorSource, NoopConsoleErrorSource,
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

/// Tente de resoudre l'origine ecran `(x, y)` de la fenetre cible via KWin
/// Scripting, au demarrage du mode `kwin-eis`. Utilise [`KwinRenderer`] pour
/// rejouer le meme script KWin que pour la capture (sans dependance croisee
/// entre adapters : tout passe par `deveye-server`).
///
/// Repli sur `(0, 0)` si :
/// - `title` est absent (`--screencap-window` non fourni) ;
/// - le backend effectif n'est pas KWin (xcap ou session non-KDE/Wayland) ;
/// - la fenetre est introuvable au moment du demarrage.
///
/// L'echec n'est jamais fatal : le mode kwin-eis reste operationnel, mais les
/// clics seront decales si la fenetre n'est pas positionnee en (0, 0).
async fn resolve_kwin_origin(title: Option<&str>, backend: ScreencapBackend) -> (i32, i32) {
    let Some(title) = title else {
        return (0, 0);
    };
    let use_kwin = match backend {
        ScreencapBackend::Kwin => true,
        ScreencapBackend::Xcap => false,
        ScreencapBackend::Auto => is_kde_wayland(),
    };
    if !use_kwin {
        return (0, 0);
    }
    match KwinRenderer::new(title).window_origin().await {
        Ok(origin) => {
            eprintln!(
                "deveye-server : origine fenetre '{}' resolue : ({}, {})",
                title, origin.0, origin.1
            );
            origin
        }
        Err(e) => {
            eprintln!(
                "deveye-server : resolution origine '{}' echouee ({e}), repli sur (0,0)",
                title
            );
            (0, 0)
        }
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
        // L'origine fenetre est resolue au demarrage (best-effort, repli (0,0)).
        TargetMode::KwinEis => {
            let window_origin =
                resolve_kwin_origin(screencap_title.as_deref(), args.screencap_backend).await;
            (
                build_os_renderer(screencap_title.as_deref(), args.screencap_backend),
                Arc::new(KwinEisInteraction::new(window_origin).expect("worker EIS")),
                Arc::new(UnsupportedInspector),
                Arc::new(UnsupportedEvaluator),
                Arc::new(EmptyWindowLister),
            )
        }
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

    let mcp = DeveyeMcpServer::new(DeveyePorts {
        renderer,
        interaction,
        inspector,
        log_source,
        evaluator,
        command_router,
        window_lister,
        console_errors,
        wait_condition,
    });

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
