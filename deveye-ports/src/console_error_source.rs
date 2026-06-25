use async_trait::async_trait;

use deveye_core::ConsoleError;

use crate::errors::PortResult;

/// Drain des erreurs console émises par le webview de l'app cible.
///
/// Par analogie avec [`LogSource::poll`](crate::LogSource) : chaque appel
/// extrait et vide le buffer accumulé depuis le dernier drain. Un `Vec` vide
/// signifie « aucune nouvelle erreur ».
///
/// L'implémentation CDP active `Log.enable` et s'abonne à
/// `Runtime.consoleAPICalled` ; l'implémentation SDK injecte un collecteur JS
/// (`window.onerror`/`console.error`) via l'évaluateur de la pompe.
#[async_trait]
pub trait ConsoleErrorSource: Send + Sync {
    async fn drain_console_errors(&self) -> PortResult<Vec<ConsoleError>>;
}
