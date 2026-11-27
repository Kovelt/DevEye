//! `deveye-server` — binaire orchestrateur DevEye.
//!
//! Compose les adapters (`adapter-cdp-webview`, `adapter-screencap-xcap`,
//! `adapter-dioxus-signals`, `adapter-logs-tracing`) puis expose le tout au
//! format MCP sur stdio (`adapter-mcp-server`) pour qu'un client MCP — Claude
//! Code via `.mcp.json`, par exemple — pilote l'app cible.
//!
//! Voir `docs/architecture.md` pour le contexte global et `docs/usage.md`
//! pour le mode d'emploi.

// Linux seul est supporté en 1.0 (ADR-0037) : les adapters natifs (X11, KWin,
// Wayland, D-Bus) n'existent que là. Plutôt qu'un binaire qui échouerait au
// lancement, la compilation hors Linux s'arrête avec ce message.
#[cfg(not(target_os = "linux"))]
compile_error!(
    "deveye-server ne supporte que Linux en 1.0 (ADR-0037) : les adapters X11, KWin et \
     Wayland sont propres à Linux. Voir docs/adr/0037-linux-seul-supporte-en-1-0.md."
);

mod cdp;
mod cli;
mod compose;
mod doctor;
mod doctor_probe;
mod fallback;
mod stubs;
mod version;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

use cli::{Args, Command};

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
    if let Some(Command::Doctor) = args.command {
        // Lecture seule : ni composition des adapters, ni serveur lancé.
        let probes = doctor_probe::gather(&args).await;
        let report = doctor::evaluate(args.target, &probes);
        println!("{}", report.render(cli_version()));
        std::process::exit(report.exit_code());
    }

    let mcp = compose::build_mcp(&args)
        .await
        .context("composition des adapters echouee au demarrage")?;
    mcp.serve_stdio()
        .await
        .context("le serveur MCP stdio s'est arrete sur erreur")?;
    Ok(())
}

/// Version courte (`0.2.0 (commit abc1234)`) pour l'en-tête de `doctor`.
fn cli_version() -> &'static str {
    version::clap_short()
}
