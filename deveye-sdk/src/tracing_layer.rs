//! Layer `tracing` qui capte les événements émis par l'app et les pousse
//! dans le buffer global servi par `GET /devtools/logs/poll`.

use std::fmt::Write;

use tracing::{
    field::{Field, Visit},
    Event, Level, Subscriber,
};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};

use deveye_core::{LogEvent, LogLevel};

use crate::{LOG_BUFFER_CAPACITY, REGISTRY};

/// Installe un layer DevEye dans le subscriber global tracing.
///
/// **À appeler une seule fois**, idéalement juste après `init`. Si un autre
/// subscriber est déjà installé, cet appel échoue silencieusement (le runtime
/// global tracing ne supporte qu'un seul subscriber actif).
pub fn install_tracing_layer() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::Registry;
    let subscriber = Registry::default().with(DeveyeLayer);
    let _ = tracing::subscriber::set_global_default(subscriber);
}

struct DeveyeLayer;

impl<S> Layer<S> for DeveyeLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let log = LogEvent::new(
            map_level(*event.metadata().level()),
            event.metadata().target(),
            visitor.message.unwrap_or_default(),
        )
        .with_fields(visitor.fields.unwrap_or(serde_json::Value::Null));

        let registry = REGISTRY.clone();
        tokio::spawn(async move {
            let mut guard = registry.logs.lock().await;
            if guard.len() >= LOG_BUFFER_CAPACITY {
                guard.pop_front();
            }
            guard.push_back(log);
        });
    }
}

fn map_level(level: Level) -> LogLevel {
    match level {
        Level::TRACE => LogLevel::Trace,
        Level::DEBUG => LogLevel::Debug,
        Level::INFO => LogLevel::Info,
        Level::WARN => LogLevel::Warn,
        Level::ERROR => LogLevel::Error,
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    fields: Option<serde_json::Value>,
}

impl MessageVisitor {
    fn insert_field(&mut self, name: &str, value: serde_json::Value) {
        let obj = self
            .fields
            .get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let serde_json::Value::Object(map) = obj {
            map.insert(name.to_string(), value);
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let mut buf = String::new();
            let _ = write!(buf, "{value:?}");
            self.message = Some(buf);
        } else {
            let mut buf = String::new();
            let _ = write!(buf, "{value:?}");
            self.insert_field(field.name(), serde_json::Value::String(buf));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.insert_field(field.name(), serde_json::Value::String(value.to_string()));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert_field(field.name(), serde_json::Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert_field(field.name(), serde_json::Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert_field(field.name(), serde_json::Value::from(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_mapping_is_total() {
        assert_eq!(map_level(Level::TRACE), LogLevel::Trace);
        assert_eq!(map_level(Level::DEBUG), LogLevel::Debug);
        assert_eq!(map_level(Level::INFO), LogLevel::Info);
        assert_eq!(map_level(Level::WARN), LogLevel::Warn);
        assert_eq!(map_level(Level::ERROR), LogLevel::Error);
    }
}
