//! `deveye-server` — binaire orchestrateur DevEye.
//!
//! Compose les adapters (`adapter-cdp-webview`, `adapter-screencap-xcap`,
//! `adapter-dioxus-signals`, `adapter-logs-tracing`) puis expose le tout au
//! format MCP sur stdio (`adapter-mcp-server`) pour qu'un client MCP — Claude
//! Code via `.mcp.json`, par exemple — pilote l'app cible.
//!
//! Voir [`deveye.md`](../../../../../docs/architecture/deveye.md) cote
//! Chronos pour le contexte global et le mode d'emploi.

mod cdp;
mod cli;
mod compose;
mod fallback;
mod stubs;

use anyhow::{Context, Result};
use clap::Parser;

use cli::Args;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let mcp = compose::build_mcp(&args).await;
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
