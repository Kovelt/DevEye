use async_trait::async_trait;

use deveye_core::InteractionCommand;

use crate::errors::PortResult;

/// Émission de commandes d'interaction vers l'app cible.
///
/// L'adapter principal (`adapter-cdp-webview`) traduit les variantes de
/// [`InteractionCommand`] en messages CDP :
/// - `Click` → `Input.dispatchMouseEvent` (pressed + released)
/// - `Type` → `Input.insertText`
/// - `KeyPress` → `Input.dispatchKeyEvent`
/// - `Navigate` → `Runtime.evaluate` sur un script qui consomme le routeur
///   exposé par le SDK opt-in.
///
/// Une implémentation `Send + Sync` peut être partagée entre handlers MCP
/// concurrents.
#[async_trait]
pub trait InteractionSink: Send + Sync {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()>;
}
