use serde::{Deserialize, Serialize};

use crate::snapshot::Position;

/// Bouton de souris pour les commandes de clic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

/// Commande d'interaction envoyée à l'app cible via un [`InteractionSink`].
///
/// Les variantes couvrent les besoins MVP du sprint DE-1. `MockAdapter` est
/// gardé sous le pied : il viendra quand on aura un consommateur réel
/// (sprint DE-2+).
///
/// [`InteractionSink`]: la doc vit dans `deveye-ports`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum InteractionCommand {
    /// Clic à une position absolue du viewport.
    Click {
        position: Position,
        #[serde(default)]
        button: MouseButton,
    },
    /// Déplacement du pointeur (sans press/release) — déclenche les
    /// `mouseover`/`mouseenter` côté app. Utile pour les UI dont des éléments
    /// n'apparaissent qu'au hover (sidebars rétractables, menus contextuels).
    Hover { position: Position },
    /// Saisie de texte au focus courant.
    Type { text: String },
    /// Pression d'une touche unique (`Enter`, `Tab`, `Escape`, …).
    KeyPress { key: String },
    /// Navigation logique à une route définie par l'app (ex. `"Agenda"`).
    /// L'app cible décide comment l'interpréter via son `CommandRouter`.
    Navigate { route: String },
}

impl InteractionCommand {
    pub fn click(position: Position) -> Self {
        Self::Click {
            position,
            button: MouseButton::Left,
        }
    }

    pub fn type_text(text: impl Into<String>) -> Self {
        Self::Type { text: text.into() }
    }

    pub fn key(name: impl Into<String>) -> Self {
        Self::KeyPress { key: name.into() }
    }

    pub fn navigate(route: impl Into<String>) -> Self {
        Self::Navigate {
            route: route.into(),
        }
    }

    pub fn hover(position: Position) -> Self {
        Self::Hover { position }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn click_constructor_defaults_to_left_button() {
        let cmd = InteractionCommand::click(Position::new(10, 20));
        assert_eq!(
            cmd,
            InteractionCommand::Click {
                position: Position::new(10, 20),
                button: MouseButton::Left,
            }
        );
    }

    #[test]
    fn serializes_with_kind_tag() {
        let cmd = InteractionCommand::type_text("Bonjour");
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v, json!({ "kind": "type", "text": "Bonjour" }));
    }

    #[test]
    fn deserializes_navigate_with_kind_tag() {
        let v = json!({ "kind": "navigate", "route": "Agenda" });
        let cmd: InteractionCommand = serde_json::from_value(v).unwrap();
        assert_eq!(cmd, InteractionCommand::navigate("Agenda"));
    }

    #[test]
    fn click_with_explicit_right_button() {
        let v = json!({
            "kind": "click",
            "position": { "x": 100, "y": 200 },
            "button": "right"
        });
        let cmd: InteractionCommand = serde_json::from_value(v).unwrap();
        assert_eq!(
            cmd,
            InteractionCommand::Click {
                position: Position::new(100, 200),
                button: MouseButton::Right,
            }
        );
    }
}
