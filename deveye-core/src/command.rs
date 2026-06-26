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
    /// Combinaison de touches : les `modifiers` sont maintenus pendant la
    /// frappe de `key` (Ctrl+S, Ctrl+Shift+P, Alt+F4…). Modificateurs
    /// reconnus (insensibles a la casse) : `ctrl`/`control`, `shift`, `alt`,
    /// `meta`/`super`/`cmd`. Indispensable : `KeyPress` seul ne porte aucun
    /// modificateur (Ctrl+Z etait impossible).
    KeyCombo { modifiers: Vec<String>, key: String },
    /// Navigation logique à une route définie par l'app (ex. `"Agenda"`).
    /// L'app cible décide comment l'interpréter via son `CommandRouter`.
    Navigate { route: String },
    /// Glisser-déposer de `from` vers `to`, bouton gauche maintenu pendant le
    /// déplacement (réordonnancement, Kanban…).
    DragDrop { from: Position, to: Position },
    /// Molette / défilement de `dx`/`dy` pixels, ancré en `position`. `dy > 0`
    /// défile vers le bas. En CDP l'ancrage cible le bon conteneur scrollable ;
    /// via le pont SDK, le conteneur scrollable sous le point est privilégié
    /// (sinon `window`) ; en kwin-eis le défilement va à la fenêtre focus.
    Scroll {
        position: Position,
        dx: i32,
        dy: i32,
    },
    /// Double-clic gauche à une position absolue (édition inline, sélection de
    /// mot, ouverture d'item de liste…).
    DoubleClick { position: Position },
    /// Répond à une boîte de dialogue JS (`alert`/`confirm`/`prompt`) : `accept`
    /// pour OK/annuler, `prompt_text` = texte saisi pour un `prompt`. CDP-only
    /// (le moteur JS est suspendu : `eval_js` est impossible pendant un dialog).
    HandleDialog {
        accept: bool,
        prompt_text: Option<String>,
    },
    /// Renseigne les fichiers d'un `input[type=file]` (le verrou de sécurité
    /// navigateur interdit de le faire en JS). CDP-only (`DOM.setFileInputFiles`).
    UploadFile {
        selector: String,
        paths: Vec<String>,
    },
}

/// Description d'une commande logique exposée par l'app cible, listée par
/// `GET /devtools/commands` et l'outil MCP `list_commands`.
///
/// L'app enregistre ses commandes via `deveye_sdk::expose_route(name,
/// description, closure)` ; un agent (Claude) les découvre par cette liste
/// puis les déclenche par `POST /devtools/command` / `invoke_command`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandDescriptor {
    /// Identifiant de la commande, identique au champ `route` de l'invocation.
    pub name: String,
    /// Description lisible par un agent — à quoi sert la commande et ce
    /// qu'attend son `payload`.
    pub description: String,
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

    pub fn key_combo(modifiers: Vec<String>, key: impl Into<String>) -> Self {
        Self::KeyCombo {
            modifiers,
            key: key.into(),
        }
    }

    pub fn navigate(route: impl Into<String>) -> Self {
        Self::Navigate {
            route: route.into(),
        }
    }

    pub fn hover(position: Position) -> Self {
        Self::Hover { position }
    }

    pub fn drag_drop(from: Position, to: Position) -> Self {
        Self::DragDrop { from, to }
    }

    pub fn scroll(position: Position, dx: i32, dy: i32) -> Self {
        Self::Scroll { position, dx, dy }
    }

    pub fn double_click(position: Position) -> Self {
        Self::DoubleClick { position }
    }

    pub fn handle_dialog(accept: bool, prompt_text: Option<String>) -> Self {
        Self::HandleDialog {
            accept,
            prompt_text,
        }
    }

    pub fn upload_file(selector: impl Into<String>, paths: Vec<String>) -> Self {
        Self::UploadFile {
            selector: selector.into(),
            paths,
        }
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
    fn drag_drop_serializes_with_kind_tag() {
        let cmd = InteractionCommand::drag_drop(Position::new(10, 20), Position::new(100, 200));
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(
            v,
            json!({
                "kind": "drag-drop",
                "from": { "x": 10, "y": 20 },
                "to": { "x": 100, "y": 200 }
            })
        );
    }

    #[test]
    fn double_click_serializes_with_kind_tag() {
        let cmd = InteractionCommand::double_click(Position::new(5, 6));
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(
            v,
            json!({ "kind": "double-click", "position": { "x": 5, "y": 6 } })
        );
    }

    #[test]
    fn handle_dialog_serializes_with_kind_tag() {
        let cmd = InteractionCommand::handle_dialog(true, Some("hello".to_string()));
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(
            v,
            json!({ "kind": "handle-dialog", "accept": true, "prompt_text": "hello" })
        );
    }

    #[test]
    fn upload_file_serializes_with_kind_tag() {
        let cmd =
            InteractionCommand::upload_file("input[type=file]", vec!["/tmp/a.png".to_string()]);
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(
            v,
            json!({ "kind": "upload-file", "selector": "input[type=file]", "paths": ["/tmp/a.png"] })
        );
    }

    #[test]
    fn scroll_serializes_with_kind_tag() {
        let cmd = InteractionCommand::scroll(Position::new(10, 20), 0, 600);
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(
            v,
            json!({ "kind": "scroll", "position": { "x": 10, "y": 20 }, "dx": 0, "dy": 600 })
        );
    }

    #[test]
    fn key_combo_serializes_with_kind_tag() {
        let cmd =
            InteractionCommand::key_combo(vec!["ctrl".to_string(), "shift".to_string()], "KeyP");
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(
            v,
            json!({ "kind": "key-combo", "modifiers": ["ctrl", "shift"], "key": "KeyP" })
        );
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
