//! `deveye-server` — binaire orchestrateur DevEye.
//!
//! Compose les adapters (`adapter-cdp-webview`, `adapter-screencap-xcap`,
//! `adapter-dioxus-signals`, `adapter-logs-tracing`) puis expose le tout au
//! format MCP sur stdio (`adapter-mcp-server`) pour qu'un client MCP — Claude
//! Code via `.mcp.json`, par exemple — pilote l'app cible.
//!
//! Voir `docs/architecture.md` pour le contexte global et `docs/usage.md`
//! pour le mode d'emploi.

mod cdp;
mod cli;
mod compose;
mod fallback;
mod stubs;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

use cli::Args;

/// Sortie des traces sur STDERR exclusivement : stdout est le canal MCP
/// (stdio), une seule ligne de log dessus corromprait le protocole cote
/// client. `RUST_LOG` pilote le filtre (`info` par defaut si absent/invalide).
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let args = Args::parse_checked();
    let mcp = compose::build_mcp(&args)
        .await
        .context("composition des adapters echouee au demarrage")?;
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
