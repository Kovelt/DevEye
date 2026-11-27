use serde::{Deserialize, Serialize};

use crate::modifier::{Modifier, MAX_KEY_HOLD_MS};
use crate::snapshot::Position;

/// Bouton de souris pour les commandes de clic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

/// Commande d'interaction envoyée à l'app cible via un `InteractionSink` (trait défini dans `deveye-ports`).
///
/// Les variantes couvrent les besoins MVP du sprint DE-1. `MockAdapter` est
/// gardé sous le pied : il viendra quand on aura un consommateur réel
/// (sprint DE-2+).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[non_exhaustive]
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
    /// Émule des dimensions de viewport de `width` x `height` pixels CSS
    /// (test responsive). Émulation du moteur de rendu : la fenêtre de l'OS
    /// n'est pas redimensionnée. `0` x `0` retire l'émulation et rend le
    /// viewport réel ; un seul côté à `0` est invalide. CDP-only
    /// (`Emulation.setDeviceMetricsOverride`) : les cibles sans moteur
    /// pilotable répondent `PortError::Unsupported`.
    Resize { width: u32, height: u32 },
    /// Décorateur : exécute `command` avec les `modifiers` maintenus enfoncés
    /// (Shift+clic pour étendre une sélection, Ctrl+glisser, Alt+molette…).
    /// N'enveloppe que les gestes de pointeur (`Click`, `DoubleClick`, `Hover`,
    /// `DragDrop`, `Scroll`), jamais un autre `WithModifiers` ni `KeyHold`.
    /// L'invariant est vérifié à la construction ([`Self::with_modifiers`]), à
    /// la désérialisation et par [`Self::validate`], que chaque adapter appelle
    /// avant d'émettre quoi que ce soit.
    WithModifiers {
        modifiers: Vec<Modifier>,
        #[serde(deserialize_with = "deserialize_gesture")]
        command: Box<InteractionCommand>,
    },
    /// Maintient `key` enfoncée `duration_ms` millisecondes puis la relâche,
    /// dans un seul appel (jamais de touche collante entre deux appels).
    /// `duration_ms` ne dépasse pas [`MAX_KEY_HOLD_MS`].
    KeyHold {
        key: String,
        #[serde(deserialize_with = "deserialize_hold_duration")]
        duration_ms: u32,
    },
}

/// Commande refusée avant toute émission : elle viole un invariant du domaine.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum InvalidCommand {
    /// `WithModifiers` n'enveloppe que les gestes de pointeur.
    #[error(
        "WithModifiers n'enveloppe que click, double-click, hover, drag-drop et scroll,          pas « {0} »"
    )]
    NotModifiable(&'static str),
    /// `KeyHold` au-delà du plafond.
    #[error("KeyHold de {requested} ms refusé : le plafond est de {max} ms")]
    HoldTooLong { requested: u32, max: u32 },
    /// `KeyHold` sans nom de touche.
    #[error("KeyHold sans nom de touche")]
    EmptyKey,
}

fn deserialize_gesture<'de, D>(deserializer: D) -> Result<Box<InteractionCommand>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let command = Box::<InteractionCommand>::deserialize(deserializer)?;
    command
        .check_modifiable()
        .map_err(serde::de::Error::custom)?;
    Ok(command)
}

fn deserialize_hold_duration<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let duration_ms = u32::deserialize(deserializer)?;
    check_hold(duration_ms).map_err(serde::de::Error::custom)?;
    Ok(duration_ms)
}

fn check_hold(duration_ms: u32) -> Result<(), InvalidCommand> {
    if duration_ms > MAX_KEY_HOLD_MS {
        return Err(InvalidCommand::HoldTooLong {
            requested: duration_ms,
            max: MAX_KEY_HOLD_MS,
        });
    }
    Ok(())
}

/// Description d'une commande logique exposée par l'app cible, listée par
/// `GET /devtools/commands` et l'outil MCP `list_commands`.
///
/// L'app enregistre ses commandes via `deveye_sdk::expose_route(name,
/// description, closure)` ; un agent (Claude) les découvre par cette liste
/// puis les déclenche par `POST /devtools/command` / `invoke_command`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CommandDescriptor {
    /// Identifiant de la commande, identique au champ `route` de l'invocation.
    pub name: String,
    /// Description lisible par un agent — à quoi sert la commande et ce
    /// qu'attend son `payload`.
    pub description: String,
}

impl CommandDescriptor {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
        }
    }
}

impl InteractionCommand {
    /// Nom court de la variante, sans ses champs : sûr pour les journaux et les
    /// messages d'erreur (le texte tapé ou les chemins n'y ont pas leur place).
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Click { .. } => "click",
            Self::Hover { .. } => "hover",
            Self::Type { .. } => "type",
            Self::KeyPress { .. } => "key-press",
            Self::KeyCombo { .. } => "key-combo",
            Self::Navigate { .. } => "navigate",
            Self::DragDrop { .. } => "drag-drop",
            Self::Scroll { .. } => "scroll",
            Self::DoubleClick { .. } => "double-click",
            Self::HandleDialog { .. } => "handle-dialog",
            Self::UploadFile { .. } => "upload-file",
            Self::Resize { .. } => "resize",
            Self::WithModifiers { .. } => "with-modifiers",
            Self::KeyHold { .. } => "key-hold",
        }
    }

    /// Vrai pour les gestes de pointeur qu'un décorateur [`Self::WithModifiers`]
    /// peut envelopper.
    pub fn accepts_modifiers(&self) -> bool {
        matches!(
            self,
            Self::Click { .. }
                | Self::DoubleClick { .. }
                | Self::Hover { .. }
                | Self::DragDrop { .. }
                | Self::Scroll { .. }
        )
    }

    fn check_modifiable(&self) -> Result<(), InvalidCommand> {
        if self.accepts_modifiers() {
            Ok(())
        } else {
            Err(InvalidCommand::NotModifiable(self.kind_name()))
        }
    }

    /// Vérifie les invariants du domaine. Les champs des variantes étant
    /// publics, une commande peut être construite par littéral : chaque adapter
    /// appelle donc cette méthode avant toute émission.
    pub fn validate(&self) -> Result<(), InvalidCommand> {
        match self {
            Self::WithModifiers { command, .. } => command.check_modifiable(),
            Self::KeyHold { key, duration_ms } => {
                if key.trim().is_empty() {
                    return Err(InvalidCommand::EmptyKey);
                }
                check_hold(*duration_ms)
            }
            _ => Ok(()),
        }
    }

    /// Enveloppe un geste de pointeur avec des modificateurs maintenus. Une
    /// liste vide est acceptée (équivalent au geste seul).
    pub fn with_modifiers(
        modifiers: Vec<Modifier>,
        command: InteractionCommand,
    ) -> Result<Self, InvalidCommand> {
        command.check_modifiable()?;
        Ok(Self::WithModifiers {
            modifiers,
            command: Box::new(command),
        })
    }

    /// Maintient une touche `duration_ms` millisecondes (au plus
    /// [`MAX_KEY_HOLD_MS`]).
    pub fn key_hold(key: impl Into<String>, duration_ms: u32) -> Result<Self, InvalidCommand> {
        let command = Self::KeyHold {
            key: key.into(),
            duration_ms,
        };
        command.validate()?;
        Ok(command)
    }

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

    pub fn resize(width: u32, height: u32) -> Self {
        Self::Resize { width, height }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn command_descriptor_new_keeps_name_and_description() {
        let descriptor = CommandDescriptor::new("ping", "répond pong");
        assert_eq!(descriptor.name, "ping");
        assert_eq!(descriptor.description, "répond pong");
    }

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
    fn resize_serializes_with_kind_tag() {
        let cmd = InteractionCommand::resize(375, 812);
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v, json!({ "kind": "resize", "width": 375, "height": 812 }));
    }

    #[test]
    fn resize_deserializes_from_kind_tag() {
        let v = json!({ "kind": "resize", "width": 1024, "height": 768 });
        let cmd: InteractionCommand = serde_json::from_value(v).unwrap();
        assert_eq!(cmd, InteractionCommand::resize(1024, 768));
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

    // ---- WithModifiers et KeyHold -------------------------------------------

    fn shift_click() -> InteractionCommand {
        InteractionCommand::with_modifiers(
            vec![Modifier::Shift],
            InteractionCommand::click(Position::new(1, 2)),
        )
        .unwrap()
    }

    #[test]
    fn with_modifiers_wraps_every_pointer_gesture() {
        let p = Position::new(0, 0);
        for gesture in [
            InteractionCommand::click(p),
            InteractionCommand::double_click(p),
            InteractionCommand::hover(p),
            InteractionCommand::drag_drop(p, p),
            InteractionCommand::scroll(p, 0, 10),
        ] {
            assert!(InteractionCommand::with_modifiers(vec![Modifier::Ctrl], gesture).is_ok());
        }
    }

    #[test]
    fn with_modifiers_refuses_everything_else() {
        for other in [
            InteractionCommand::type_text("x"),
            InteractionCommand::key("a"),
            InteractionCommand::navigate("r"),
            InteractionCommand::resize(1, 1),
            InteractionCommand::key_hold("Shift_L", 10).unwrap(),
            shift_click(),
        ] {
            let err = InteractionCommand::with_modifiers(vec![Modifier::Alt], other).unwrap_err();
            assert!(matches!(err, InvalidCommand::NotModifiable(_)));
        }
    }

    #[test]
    fn with_modifiers_round_trips_through_json() {
        let cmd = shift_click();
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(
            v,
            json!({
                "kind": "with-modifiers",
                "modifiers": ["shift"],
                "command": { "kind": "click", "position": { "x": 1, "y": 2 }, "button": "left" }
            })
        );
        let back: InteractionCommand = serde_json::from_value(v).unwrap();
        assert_eq!(back, cmd);
    }

    #[test]
    fn deserialization_refuses_hostile_json() {
        // Imbrication.
        let nested = json!({
            "kind": "with-modifiers", "modifiers": ["ctrl"],
            "command": {
                "kind": "with-modifiers", "modifiers": ["shift"],
                "command": { "kind": "click", "position": { "x": 1, "y": 2 } }
            }
        });
        assert!(serde_json::from_value::<InteractionCommand>(nested).is_err());
        // Geste non pointeur.
        let typed = json!({
            "kind": "with-modifiers", "modifiers": ["ctrl"],
            "command": { "kind": "type", "text": "x" }
        });
        assert!(serde_json::from_value::<InteractionCommand>(typed).is_err());
        // KeyHold dans WithModifiers : le plafond cumulé n'est jamais contourné.
        let hold = json!({
            "kind": "with-modifiers", "modifiers": ["ctrl"],
            "command": { "kind": "key-hold", "key": "a", "duration_ms": 5 }
        });
        assert!(serde_json::from_value::<InteractionCommand>(hold).is_err());
        // Modificateur inconnu.
        let unknown = json!({
            "kind": "with-modifiers", "modifiers": ["hyper"],
            "command": { "kind": "click", "position": { "x": 1, "y": 2 } }
        });
        assert!(serde_json::from_value::<InteractionCommand>(unknown).is_err());
        // KeyHold au-delà du plafond.
        let long = json!({ "kind": "key-hold", "key": "a", "duration_ms": MAX_KEY_HOLD_MS + 1 });
        assert!(serde_json::from_value::<InteractionCommand>(long).is_err());
    }

    #[test]
    fn key_hold_is_capped_at_construction_and_validation() {
        assert!(InteractionCommand::key_hold("Shift_L", MAX_KEY_HOLD_MS).is_ok());
        assert_eq!(
            InteractionCommand::key_hold("Shift_L", MAX_KEY_HOLD_MS + 1),
            Err(InvalidCommand::HoldTooLong {
                requested: MAX_KEY_HOLD_MS + 1,
                max: MAX_KEY_HOLD_MS
            })
        );
        assert_eq!(
            InteractionCommand::key_hold(" ", 10),
            Err(InvalidCommand::EmptyKey)
        );
        // Un littéral qui contourne les constructeurs est rattrapé par validate().
        let literal = InteractionCommand::KeyHold {
            key: "a".into(),
            duration_ms: u32::MAX,
        };
        assert!(literal.validate().is_err());
        let wrapped = InteractionCommand::WithModifiers {
            modifiers: vec![Modifier::Ctrl],
            command: Box::new(InteractionCommand::type_text("x")),
        };
        assert!(wrapped.validate().is_err());
        assert!(shift_click().validate().is_ok());
    }

    #[test]
    fn key_hold_serializes_with_kind_tag() {
        let cmd = InteractionCommand::key_hold("Shift_L", 300).unwrap();
        assert_eq!(
            serde_json::to_value(&cmd).unwrap(),
            json!({ "kind": "key-hold", "key": "Shift_L", "duration_ms": 300 })
        );
    }

    #[test]
    fn kind_name_never_leaks_payloads() {
        assert_eq!(InteractionCommand::type_text("secret").kind_name(), "type");
        assert_eq!(shift_click().kind_name(), "with-modifiers");
    }
}
