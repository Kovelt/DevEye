use async_trait::async_trait;

use deveye_core::{DomTree, StateDump};

use crate::errors::PortResult;

/// Inspection introspective d'une app cible.
///
/// Deux opérations distinctes :
///
/// - `dom_snapshot` est généralement fourni par un adapter qui parle CDP et
///   appelle `DOM.getDocument` — disponible pour toute cible webview.
///
/// - `state_snapshot` est plus spécifique : il consomme le SDK opt-in
///   embarqué dans l'app (cf. `deveye-sdk`). Il renvoie la composition des
///   signaux exposés par l'app via `expose_signal::<T>(name)`. Une app non
///   instrumentée renverra un `StateDump::empty()`.
#[async_trait]
pub trait AppInspector: Send + Sync {
    async fn dom_snapshot(&self) -> PortResult<DomTree>;
    async fn state_snapshot(&self) -> PortResult<StateDump>;
}
