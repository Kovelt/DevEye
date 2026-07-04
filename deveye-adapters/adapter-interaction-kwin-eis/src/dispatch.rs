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

/// Traduit une `InteractionCommand` en `EisAction` prete a etre soumise au
/// worker EIS, en convertissant les `Position` viewport-relatives en absolu
/// ecran via `origin` (x, y). Fonction libre, testable sans session KWin ni
/// thread worker (`dispatch` l'appelle apres avoir lu `self.origin()`).
fn to_action(command: InteractionCommand, origin: (i32, i32)) -> Result<EisAction, PortError> {
    let (ox, oy) = origin;
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
        InteractionCommand::KeyCombo { modifiers, key } => EisAction::KeyCombo { modifiers, key },
        // `Navigate` suppose un routeur applicatif : pas de sens OS-level.
        InteractionCommand::Navigate { .. } => {
            return Err(PortError::Unsupported(
                "Navigate nécessite un routeur applicatif (SDK ou CDP) ; \
                 indisponible en mode kwin-eis (mains OS-level pures)"
                    .to_string(),
            ));
        }
    };
    Ok(action)
}

#[async_trait]
impl InteractionSink for KwinEisInteraction {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
        // Nom de variante SEUL : le Debug complet embarquerait le texte tape
        // (`Type`) ou des chemins (`UploadFile`) dans les logs debug.
        tracing::debug!(commande = command_kind(&command), "dispatch EIS");
        let origin = self.origin().await;
        let action = to_action(command, origin)?;

        let result = self.submit(action).await;
        if let Err(ref e) = result {
            tracing::warn!("erreur du worker EIS lors du dispatch : {e}");
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deveye_core::MouseButton;

    const ORIGIN: (i32, i32) = (10, 20);

    /// (a) Click : la position viewport-relative est convertie en absolu
    /// ecran (position + origine), et le bouton est propage tel quel.
    #[test]
    fn click_converts_viewport_position_and_origin_to_absolute() {
        let command = InteractionCommand::Click {
            position: Position::new(5, 5),
            button: MouseButton::Right,
        };
        let action = to_action(command, ORIGIN).expect("Click est supporte en kwin-eis");
        match action {
            EisAction::Click { abs, button } => {
                assert_eq!(abs, (15.0, 25.0));
                assert_eq!(button, MouseButton::Right);
            }
            _ => panic!("attendu EisAction::Click"),
        }
    }

    /// (b) Navigate suppose un routeur applicatif : indisponible en kwin-eis.
    #[test]
    fn navigate_is_unsupported() {
        let command = InteractionCommand::Navigate {
            route: "Agenda".to_string(),
        };
        let result = to_action(command, ORIGIN);
        assert!(matches!(result, Err(PortError::Unsupported(_))));
    }

    /// (c) HandleDialog est CDP-only (pas de WebView en kwin-eis).
    #[test]
    fn handle_dialog_is_unsupported() {
        let command = InteractionCommand::HandleDialog {
            accept: true,
            prompt_text: None,
        };
        let result = to_action(command, ORIGIN);
        assert!(matches!(result, Err(PortError::Unsupported(_))));
    }

    /// (d) UploadFile est CDP-only (verrou navigateur sur les input file).
    #[test]
    fn upload_file_is_unsupported() {
        let command = InteractionCommand::UploadFile {
            selector: "#input".to_string(),
            paths: vec!["/tmp/f.txt".to_string()],
        };
        let result = to_action(command, ORIGIN);
        assert!(matches!(result, Err(PortError::Unsupported(_))));
    }

    /// (e) Scroll ignore l'ancrage `position` : seuls dx/dy comptent, quels
    /// que soient la position ancree ou l'origine de fenetre.
    #[test]
    fn scroll_ignores_anchored_position_only_deltas_matter() {
        let command = InteractionCommand::Scroll {
            position: Position::new(999, 999),
            dx: 3,
            dy: -7,
        };
        let action = to_action(command, ORIGIN).expect("Scroll est supporte en kwin-eis");
        match action {
            EisAction::Scroll { dx, dy } => {
                assert_eq!(dx, 3.0);
                assert_eq!(dy, -7.0);
            }
            _ => panic!("attendu EisAction::Scroll"),
        }
    }

    /// (f) KeyCombo : modificateurs et touche propages sans transformation
    /// (la resolution en codes evdev se fait plus tard, dans session.rs).
    #[test]
    fn key_combo_maps_modifiers_and_key() {
        let command = InteractionCommand::KeyCombo {
            modifiers: vec!["ctrl".to_string(), "shift".to_string()],
            key: "s".to_string(),
        };
        let action = to_action(command, ORIGIN).expect("KeyCombo est supporte en kwin-eis");
        match action {
            EisAction::KeyCombo { modifiers, key } => {
                assert_eq!(modifiers, vec!["ctrl".to_string(), "shift".to_string()]);
                assert_eq!(key, "s");
            }
            _ => panic!("attendu EisAction::KeyCombo"),
        }
    }

    /// (f) DoubleClick : meme conversion viewport -> absolu que Click.
    #[test]
    fn double_click_converts_viewport_position_and_origin_to_absolute() {
        let command = InteractionCommand::DoubleClick {
            position: Position::new(5, 5),
        };
        let action = to_action(command, ORIGIN).expect("DoubleClick est supporte en kwin-eis");
        match action {
            EisAction::DoubleClick { abs } => assert_eq!(abs, (15.0, 25.0)),
            _ => panic!("attendu EisAction::DoubleClick"),
        }
    }

    /// Hover et DragDrop passent par la meme conversion viewport -> absolu ;
    /// couverts pour completer l'exhaustivite du match de `to_action`.
    #[test]
    fn hover_converts_viewport_position_and_origin_to_absolute() {
        let command = InteractionCommand::Hover {
            position: Position::new(1, 2),
        };
        let action = to_action(command, ORIGIN).expect("Hover est supporte en kwin-eis");
        match action {
            EisAction::Hover { abs } => assert_eq!(abs, (11.0, 22.0)),
            _ => panic!("attendu EisAction::Hover"),
        }
    }

    #[test]
    fn drag_drop_converts_both_endpoints_to_absolute() {
        let command = InteractionCommand::DragDrop {
            from: Position::new(1, 2),
            to: Position::new(3, 4),
        };
        let action = to_action(command, ORIGIN).expect("DragDrop est supporte en kwin-eis");
        match action {
            EisAction::Drag { from, to } => {
                assert_eq!(from, (11.0, 22.0));
                assert_eq!(to, (13.0, 24.0));
            }
            _ => panic!("attendu EisAction::Drag"),
        }
    }

    /// Type et KeyPress ne portent aucune position : propages tels quels.
    #[test]
    fn type_and_key_press_pass_through_unchanged() {
        let type_action = to_action(
            InteractionCommand::Type {
                text: "bonjour".to_string(),
            },
            ORIGIN,
        )
        .expect("Type est supporte en kwin-eis");
        match type_action {
            EisAction::Type { text } => assert_eq!(text, "bonjour"),
            _ => panic!("attendu EisAction::Type"),
        }

        let key_press_action = to_action(
            InteractionCommand::KeyPress {
                key: "Return".to_string(),
            },
            ORIGIN,
        )
        .expect("KeyPress est supporte en kwin-eis");
        match key_press_action {
            EisAction::KeyPress { key } => assert_eq!(key, "Return"),
            _ => panic!("attendu EisAction::KeyPress"),
        }
    }
}
