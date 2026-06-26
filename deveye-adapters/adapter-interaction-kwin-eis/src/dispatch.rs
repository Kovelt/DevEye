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

#[async_trait]
impl InteractionSink for KwinEisInteraction {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
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

        self.submit(action).await
    }
}
