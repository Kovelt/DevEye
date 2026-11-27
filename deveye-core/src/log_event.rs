use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Sévérité d'un log applicatif.
///
/// Aligné sur les niveaux standards `tracing` (trace < debug < info < warn < error).
/// Sérialisé en minuscules pour rester compatible avec les conventions courantes.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

/// Événement de log capturé depuis une app cible.
///
/// `target` correspond au module Rust qui émet le log (ex. `mon_app::pages::agenda`).
/// `fields` porte les paires `key=value` posées par les macros `tracing` ; on
/// les garde en `serde_json::Value` plutôt qu'en `BTreeMap<String,String>` parce
/// que `tracing` autorise des types arbitraires.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LogEvent {
    pub level: LogLevel,
    pub target: String,
    pub message: String,
    pub timestamp: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub fields: serde_json::Value,
}

impl LogEvent {
    pub fn new(level: LogLevel, target: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level,
            target: target.into(),
            message: message.into(),
            timestamp: Utc::now(),
            fields: serde_json::Value::Null,
        }
    }

    pub fn with_fields(mut self, fields: serde_json::Value) -> Self {
        self.fields = fields;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn levels_are_ordered() {
        assert!(LogLevel::Trace < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn level_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&LogLevel::Error).unwrap(),
            "\"error\""
        );
        let parsed: LogLevel = serde_json::from_str("\"trace\"").unwrap();
        assert_eq!(parsed, LogLevel::Trace);
    }

    #[test]
    fn empty_fields_are_skipped_in_serialization() {
        let event = LogEvent::new(LogLevel::Info, "mon_app", "Hello");
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(
            !serialized.contains("fields"),
            "got {serialized}, expected no `fields` key"
        );
    }

    #[test]
    fn populated_fields_are_serialized() {
        let event = LogEvent::new(LogLevel::Warn, "mon_app", "slow query")
            .with_fields(json!({ "duration_ms": 1234 }));
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(serialized.contains("\"duration_ms\":1234"));
    }
}
