//! Implémentation de [`InteractionSink`] : mapping `InteractionCommand` ->
//! `EisAction` (coordonnées converties en absolu écran), puis soumission au
//! thread worker EIS.
//!
//! Variante purement applicative (`Navigate`) : `PortError::Unsupported`.
//! Toutes les autres sont injectées OS-level.

use async_trait::async_trait;

use deveye_core::{InteractionCommand, Position};
use deveye_ports::{InteractionSink, PortError, PortResult};

use crate::connection::{EisAction, KwinEisInteraction};

/// Nom court de la variante, sans ses champs (donnees potentiellement
/// sensibles ou volumineuses).
fn command_kind(command: &InteractionCommand) -> &'static str {
    match command {
        InteractionCommand::Click { .. } => "click",
        InteractionCommand::Hover { .. } => "hover",
        InteractionCommand::Type { .. } => "type",
        InteractionCommand::KeyPress { .. } => "key-press",
        InteractionCommand::KeyCombo { .. } => "key-combo",
        InteractionCommand::Navigate { .. } => "navigate",
        InteractionCommand::DragDrop { .. } => "drag-drop",
        InteractionCommand::Scroll { .. } => "scroll",
        InteractionCommand::DoubleClick { .. } => "double-click",
        InteractionCommand::HandleDialog { .. } => "handle-dialog",
        InteractionCommand::UploadFile { .. } => "upload-file",
    }
}

#[async_trait]
impl InteractionSink for KwinEisInteraction {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
        // Nom de variante SEUL : le Debug complet embarquerait le texte tape
        // (`Type`) ou des chemins (`UploadFile`) dans les logs debug.
        tracing::debug!(commande = command_kind(&command), "dispatch EIS");
        let (ox, oy) = self.origin().await;
        let to_abs = |p: Position| ((p.x + ox) as f64, (p.y + oy) as f64);

        let action = match command {
            InteractionCommand::Click { position, button } => EisAction::Click {
                abs: to_abs(position),
                button,
            },
            InteractionCommand::Hover { position } => EisAction::Hover {
                abs: to_abs(position),
            },
            InteractionCommand::DragDrop { from, to } => EisAction::Drag {
                from: to_abs(from),
                to: to_abs(to),
            },
            // Le defilement EIS va a la fenetre focus : l'ancrage `position`
            // n'a pas de sens OS-level, seuls les deltas comptent.
            InteractionCommand::Scroll { dx, dy, .. } => EisAction::Scroll {
                dx: dx as f64,
                dy: dy as f64,
            },
            InteractionCommand::DoubleClick { position } => EisAction::DoubleClick {
                abs: to_abs(position),
            },
            // CDP-only : un dialog JS et le remplissage d'input fichier n'ont
            // pas de sens OS-level (pas de WebView en kwin-eis).
            InteractionCommand::HandleDialog { .. } => {
                return Err(PortError::Unsupported(
                    "handle_dialog nécessite CDP ; indisponible en kwin-eis".to_string(),
                ));
            }
            InteractionCommand::UploadFile { .. } => {
                return Err(PortError::Unsupported(
                    "upload_file nécessite CDP ; indisponible en kwin-eis".to_string(),
                ));
            }
            InteractionCommand::Type { text } => EisAction::Type { text },
            InteractionCommand::KeyPress { key } => EisAction::KeyPress { key },
            InteractionCommand::KeyCombo { modifiers, key } => {
                EisAction::KeyCombo { modifiers, key }
            }
            // `Navigate` suppose un routeur applicatif : pas de sens OS-level.
            InteractionCommand::Navigate { .. } => {
                return Err(PortError::Unsupported(
                    "Navigate nécessite un routeur applicatif (SDK ou CDP) ; \
                     indisponible en mode kwin-eis (mains OS-level pures)"
                        .to_string(),
                ));
            }
        };

        let result = self.submit(action).await;
        if let Err(ref e) = result {
            tracing::warn!("erreur du worker EIS lors du dispatch : {e}");
        }
        result
    }
}
