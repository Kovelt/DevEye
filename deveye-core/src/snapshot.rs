use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Dimensions d'un viewport en pixels logiques.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// Coordonnées d'un clic ou d'un point d'intérêt, en pixels logiques depuis
/// le coin haut-gauche du viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

impl Position {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Capture visuelle d'une app à un instant.
///
/// `bytes` est l'encodage PNG brut. On choisit PNG pour la portabilité (les
/// MCP servers savent le passer en base64) et la fidélité (loss-less, important
/// pour des outils de VRT futurs).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screenshot {
    pub bytes: Vec<u8>,
    pub captured_at: DateTime<Utc>,
    pub viewport: Size,
}

impl Screenshot {
    pub fn new_png(bytes: Vec<u8>, viewport: Size) -> Self {
        Self {
            bytes,
            captured_at: Utc::now(),
            viewport,
        }
    }
}

/// Snapshot du DOM d'une app webview.
///
/// `tree` est laissé en JSON libre : selon l'adapter on aura soit un
/// résultat brut de CDP `DOM.getDocument`, soit un arbre simplifié. Un type
/// fort viendra quand un consommateur le réclamera (sprint VRT).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomTree {
    pub tree: serde_json::Value,
    pub captured_at: DateTime<Utc>,
}

impl DomTree {
    pub fn new(tree: serde_json::Value) -> Self {
        Self {
            tree,
            captured_at: Utc::now(),
        }
    }
}

/// Snapshot de l'état applicatif exposé par le SDK opt-in (signaux Dioxus,
/// fixtures actives, etc.).
///
/// `entries` est une map `nom → valeur JSON` — chaque clé correspond à un
/// signal nommé par `deveye_sdk::expose_signal::<T>(name)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateDump {
    pub entries: std::collections::BTreeMap<String, serde_json::Value>,
    pub captured_at: DateTime<Utc>,
}

impl StateDump {
    pub fn new(entries: std::collections::BTreeMap<String, serde_json::Value>) -> Self {
        Self {
            entries,
            captured_at: Utc::now(),
        }
    }

    pub fn empty() -> Self {
        Self::new(std::collections::BTreeMap::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn screenshot_preserves_bytes_and_viewport() {
        let bytes = vec![0x89, 0x50, 0x4E, 0x47];
        let shot = Screenshot::new_png(bytes.clone(), Size::new(1024, 768));
        assert_eq!(shot.bytes, bytes);
        assert_eq!(shot.viewport, Size::new(1024, 768));
    }

    #[test]
    fn state_dump_empty_is_serializable() {
        let dump = StateDump::empty();
        let serialized = serde_json::to_string(&dump).unwrap();
        assert!(serialized.contains("\"entries\":{}"));
    }

    #[test]
    fn state_dump_keys_are_sorted_via_btreemap() {
        let mut entries = std::collections::BTreeMap::new();
        entries.insert("z_signal".to_string(), json!(1));
        entries.insert("a_signal".to_string(), json!(2));
        let dump = StateDump::new(entries);
        let keys: Vec<&String> = dump.entries.keys().collect();
        assert_eq!(keys, vec!["a_signal", "z_signal"]);
    }
}
