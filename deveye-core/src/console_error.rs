use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Sévérité d'une entrée console navigateur.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleLevel {
    Log,
    Info,
    Warning,
    Error,
}

/// Erreur (ou entrée) console émise par le webview de l'app cible.
///
/// `url` et `line` sont optionnels : CDP les fournit pour les messages issus de
/// fichiers JS référencés, mais pas pour les exceptions synchrones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsoleError {
    pub level: ConsoleLevel,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn level_serializes_lowercase() {
        assert_eq!(
            serde_json::to_value(ConsoleLevel::Warning).unwrap(),
            json!("warning")
        );
    }

    #[test]
    fn optional_fields_skipped_when_absent() {
        let e = ConsoleError {
            level: ConsoleLevel::Error,
            text: "boom".to_string(),
            url: None,
            line: None,
            timestamp: Utc::now(),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert!(v.get("url").is_none());
        assert!(v.get("line").is_none());
        assert_eq!(v["level"], json!("error"));
        assert_eq!(v["text"], json!("boom"));
    }
}
