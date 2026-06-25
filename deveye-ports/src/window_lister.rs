use async_trait::async_trait;

use deveye_core::WindowInfo;

use crate::errors::PortResult;

/// Énumère les fenêtres/onglets de la cible.
///
/// Implémenté par l'adapter CDP (`Target.getTargets`, filtré sur `type=page`).
/// Le pont natif SDK ne connaît pas les autres fenêtres → retourne une liste
/// vide (pas une erreur), pour qu'un agent puisse continuer son scénario.
#[async_trait]
pub trait WindowLister: Send + Sync {
    async fn list_windows(&self) -> PortResult<Vec<WindowInfo>>;
}
