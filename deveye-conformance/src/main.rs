//! `deveye-conformance --url http://127.0.0.1:PORT` : valide une implémentation
//! du wire protocol DevEye et sort en code 0 (conforme), 1 (écarts) ou 2 (la
//! suite n'a pas pu s'exécuter).

use std::process::ExitCode;

use clap::Parser;
use deveye_conformance::{run, Options};

#[derive(Parser)]
#[command(
    version,
    about = "Suite de conformité boîte noire du wire protocol DevEye"
)]
struct Cli {
    /// URL de base de l'implémentation à valider.
    #[arg(long, default_value = "http://127.0.0.1:31337")]
    url: String,
    /// Expression JS que la pompe laisse sans réponse (vérifie le 504, dure 10 s).
    #[arg(long)]
    slow_expression: Option<String>,
    /// Expression JS que la pompe abandonne (vérifie le 500).
    #[arg(long)]
    abandon_expression: Option<String>,
    /// Commande sans effet de bord à invoquer (vérifie le 200 de POST /devtools/command).
    #[arg(long)]
    invoke_route: Option<String>,
    /// Échoue aussi quand une vérification est ignorée (exigence non observable).
    #[arg(long)]
    strict: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut options = Options::new(cli.url);
    options.slow_expression = cli.slow_expression;
    options.abandon_expression = cli.abandon_expression;
    options.invoke_route = cli.invoke_route;
    match run(&options).await {
        Ok(report) => {
            print!("{}", report.render());
            let ok = if cli.strict {
                report.is_fully_checked()
            } else {
                report.is_conformant()
            };
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("deveye-conformance : {e}");
            ExitCode::from(2)
        }
    }
}
