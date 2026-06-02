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

use adapter_cdp_webview::CdpClient;
use adapter_dioxus_signals::DioxusSignalsInspector;
use adapter_logs_tracing::SdkTracingLogSource;
use adapter_mcp_server::DeveyeMcpServer;
use adapter_screencap_xcap::XcapRenderer;
use deveye_core::{DomTree, StateDump};
use deveye_ports::{AppInspector, InteractionSink, JsEvaluator, LogSource, PortResult, Renderer};

#[derive(Parser, Debug)]
#[command(
    name = "deveye-server",
    about = "Orchestrateur DevEye : compose les adapters et sert un MCP a Claude."
)]
struct Args {
    /// URL HTTP du webview CDP cible (par defaut http://localhost:9222).
    #[arg(long, default_value = "http://localhost:9222")]
    cdp_url: String,

    /// URL du SDK opt-in expose par l'app cible (par defaut http://localhost:31337).
    #[arg(long, default_value = "http://localhost:31337")]
    sdk_url: String,

    /// Si CDP est indisponible, capturer cette fenetre via xcap a la place.
    /// (Vision passive : pas d'interactivite cote screencap.)
    #[arg(long)]
    screencap_window: Option<String>,

    /// Servir le MCP server sur stdio (mode normal d'usage avec Claude).
    #[arg(long)]
    serve_mcp: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Renderer + InteractionSink + DOM inspector via CDP, avec fallback
    // screencap pour le renderer si l'attache CDP echoue.
    let cdp = match CdpClient::connect(&args.cdp_url).await {
        Ok(client) => Some(Arc::new(client)),
        Err(e) => {
            eprintln!("deveye-server : connexion CDP echouee ({e}). Fallback screencap si fenetre fournie.");
            None
        }
    };

    let (renderer, interaction, dom_inspector, evaluator) =
        match (cdp.clone(), &args.screencap_window) {
            (Some(cdp), _) => {
                let r: Arc<dyn Renderer> = cdp.clone();
                let i: Arc<dyn InteractionSink> = cdp.clone();
                let d: Arc<dyn AppInspector> = cdp.clone();
                let e: Arc<dyn JsEvaluator> = cdp;
                (r, i, d, e)
            }
            (None, Some(window_title)) => {
                let r: Arc<dyn Renderer> = Arc::new(XcapRenderer::new(window_title.clone()));
                let i: Arc<dyn InteractionSink> = Arc::new(NoopInteraction);
                let d: Arc<dyn AppInspector> = Arc::new(NoopInspector);
                let e: Arc<dyn JsEvaluator> = Arc::new(NoopEvaluator);
                (r, i, d, e)
            }
            (None, None) => {
                anyhow::bail!(
                    "ni connexion CDP ni --screencap-window : impossible de servir un renderer"
                );
            }
        };

    // State inspector + logs via le SDK opt-in cote app cible.
    let state_inspector: Arc<dyn AppInspector> =
        Arc::new(DioxusSignalsInspector::new(args.sdk_url.clone()));
    let log_source: Arc<dyn LogSource> = Arc::new(SdkTracingLogSource::new(args.sdk_url.clone()));

    let inspector: Arc<dyn AppInspector> = Arc::new(CompositeInspector {
        dom: dom_inspector,
        state: state_inspector,
    });

    let mcp = DeveyeMcpServer::new(renderer, interaction, inspector, log_source, evaluator);

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

/// `InteractionSink` qui retourne une erreur sur toute commande — utilise
/// quand on attache uniquement en mode screencap (vision passive).
struct NoopInteraction;

#[async_trait]
impl InteractionSink for NoopInteraction {
    async fn dispatch(&self, _command: deveye_core::InteractionCommand) -> PortResult<()> {
        Err(deveye_ports::PortError::Unsupported(
            "mode screencap : pas d'interactivite (clic/saisie indisponibles)".into(),
        ))
    }
}

/// `AppInspector` neutre pour le mode screencap.
struct NoopInspector;

#[async_trait]
impl AppInspector for NoopInspector {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        Ok(DomTree::new(serde_json::Value::Null))
    }

    async fn state_snapshot(&self) -> PortResult<StateDump> {
        Ok(StateDump::empty())
    }
}

/// `JsEvaluator` neutre pour le mode screencap (pas d'accès JS sans CDP).
struct NoopEvaluator;

#[async_trait]
impl JsEvaluator for NoopEvaluator {
    async fn evaluate(&self, _expression: &str) -> PortResult<serde_json::Value> {
        Err(deveye_ports::PortError::Unsupported(
            "mode screencap : pas d'evaluation JS (CDP indisponible)".into(),
        ))
    }
}
