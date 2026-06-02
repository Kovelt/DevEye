use async_trait::async_trait;

use deveye_core::LogEvent;

use crate::errors::PortResult;

/// Flux de logs émis par l'app cible.
///
/// Le contrat est volontairement minimal : `poll` extrait les événements
/// accumulés depuis le dernier appel et les retourne dans l'ordre
/// d'émission. C'est suffisant pour le MVP DE-1 (consommé par un outil MCP
/// `logs_tail`) et évite d'imposer une dépendance `futures::Stream` au
/// domaine.
///
/// Implémentations envisagées :
/// - `adapter-logs-tracing` (via SDK opt-in qui pose un `tracing::Subscriber`
///   custom et bufferise localement) ;
/// - `adapter-process-attach` (capture du `stdout`/`stderr` d'un process
///   lancé par `deveye-server`).
///
/// Un futur sprint peut introduire un trait `LogStream` basé sur
/// `futures::Stream` pour le mode WebSocket — le MVP reste sur `poll`.
#[async_trait]
pub trait LogSource: Send + Sync {
    /// Extrait les événements accumulés depuis le dernier appel. Le buffer
    /// est ensuite vidé côté implémentation. `Vec` vide = aucun nouveau log.
    async fn poll(&self) -> PortResult<Vec<LogEvent>>;
}
