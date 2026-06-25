use std::time::Duration;

use async_trait::async_trait;

use deveye_core::{WaitOutcome, WaitPredicate};

use crate::errors::PortResult;

/// Attente conditionnelle sur l'état de l'app cible.
///
/// L'implémentation boucle jusqu'à ce que `predicate` soit satisfait ou que
/// `timeout` s'écoule, puis renvoie un [`WaitOutcome`]. Un simple timeout n'est
/// **jamais** une erreur : `WaitOutcome { satisfied: false, .. }` suffit à
/// distinguer les deux cas (seules les pannes réelles renvoient `Err`).
#[async_trait]
pub trait WaitCondition: Send + Sync {
    async fn wait_until(
        &self,
        predicate: WaitPredicate,
        timeout: Duration,
    ) -> PortResult<WaitOutcome>;
}
