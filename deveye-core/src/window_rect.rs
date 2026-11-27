use serde::{Deserialize, Serialize};

/// Géométrie écran d'une fenêtre (repère natif du compositeur : origine
/// haut-gauche de l'écran, pixels physiques).
///
/// Distinct de [`crate::Size`]/[`crate::Position`] (viewport logique côté
/// webview) : `WindowRect` porte la fenêtre entière côté OS, utile pour un
/// pilotage natif (kwin-eis) qui n'a pas de notion de viewport CDP.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowRect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_rect_roundtrips_through_json() {
        let rect = WindowRect::new(-10, 20, 800, 600);
        let json = serde_json::to_string(&rect).unwrap();
        let back: WindowRect = serde_json::from_str(&json).unwrap();
        assert_eq!(rect, back);
    }
}
