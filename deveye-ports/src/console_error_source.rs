use async_trait::async_trait;

use deveye_core::ConsoleError;

use crate::errors::PortResult;

/// Drain des erreurs console émises par le webview de l'app cible.
///
/// Par analogie avec [`LogSource::poll`](crate::LogSource) : chaque appel
/// extrait et vide le buffer accumulé depuis le dernier drain. Un `Vec` vide
/// signifie « aucune nouvelle erreur ».
///
/// Implémentations disponibles :
/// - `JsConsoleErrorSource` (SDK / cibles Dioxus desktop) : injecte un
///   collecteur JS (`window.onerror` / `console.error`) via l'évaluateur.
/// - `NoopConsoleErrorSource` : retourne toujours `Ok(vec![])`, utilisée
///   pour les cibles qui ne supportent pas le webview.
///
/// TODO (M-futur) : impl CDP-native activant `Log.enable` et s'abonnant à
/// `Runtime.consoleAPICalled` via un channel dédié au pilote Chrome.
#[async_trait]
pub trait ConsoleErrorSource: Send + Sync {
    async fn drain_console_errors(&self) -> PortResult<Vec<ConsoleError>>;
}
