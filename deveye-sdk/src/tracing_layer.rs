//! Layer `tracing` qui capte les événements émis par l'app et les pousse
//! dans le buffer global servi par `GET /devtools/logs/poll`.

use std::fmt::Write;

use tracing::{
    field::{Field, Visit},
    Event, Level, Subscriber,
};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};

use deveye_core::{LogEvent, LogLevel};

/// Installe un layer DevEye dans le subscriber global tracing.
///
/// **À appeler une seule fois**, idéalement juste après `init`. Si un autre
/// subscriber est déjà installé, cet appel échoue silencieusement (le runtime
/// global tracing ne supporte qu'un seul subscriber actif). Les appels
/// ultérieurs sont idempotents : le canal de drain et la tâche de consommation
/// sont créés uniquement lors du premier appel (gardé par `OnceLock`).
///
/// En build **release**, cette fonction est un no-op — cohérent avec le fait
/// que le SDK est entièrement inactif hors `debug_assertions`.
pub fn install_tracing_layer() {
    #[cfg(debug_assertions)]
    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::Registry;
        let subscriber = Registry::default().with(DeveyeLayer);
        let _ = tracing::subscriber::set_global_default(subscriber);

        // Canal de drain (N6) : un seul spawn consomme tous les LogEvent dans
        // l'ordre d'émission, éliminant le coût d'un tokio::spawn par log.
        // OnceLock garantit l'idempotence si install_tracing_layer est appelé
        // plusieurs fois : seul le premier appel installe le canal.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<LogEvent>();
        // Si le OnceLock est deja peuple (2e appel), set retourne Err(tx) et
        // tx est dropped ; le drain ci-dessous reoit None immediatement et
        // se termine proprement.
        let _ = crate::REGISTRY.log_tx.set(tx);
        let registry = crate::REGISTRY.clone();
        crate::runtime().spawn(async move {
            let mut rx = rx;
            while let Some(log) = rx.recv().await {
                let mut guard = registry.logs.lock().await;
                if guard.len() >= crate::LOG_BUFFER_CAPACITY {
                    guard.pop_front();
                }
                guard.push_back(log);
            }
        });
    }
}

/// Niveau minimal capturé. INFO par défaut : sans filtre, le buffer se noie
/// dans le TRACE interne de Dioxus (diff de nœuds, signaux — vécu le 11 juin
/// 2026 : 500+ événements de bruit au boot, zéro log applicatif visible).
/// Surchargeable par `DEVEYE_LOG_LEVEL` (`trace`…`error`).
fn min_level() -> Level {
    static LVL: std::sync::LazyLock<Level> = std::sync::LazyLock::new(|| {
        std::env::var("DEVEYE_LOG_LEVEL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(Level::INFO)
    });
    *LVL
}

struct DeveyeLayer;

impl<S> Layer<S> for DeveyeLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // Dans `tracing`, TRACE > DEBUG > INFO > WARN > ERROR : on garde les
        // événements au plus aussi verbeux que le seuil.
        if *event.metadata().level() > min_level() {
            return;
        }
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let log = LogEvent::new(
            map_level(*event.metadata().level()),
            event.metadata().target(),
            visitor.message.unwrap_or_default(),
        )
        .with_fields(visitor.fields.unwrap_or(serde_json::Value::Null));

        // Envoi synchrone non bloquant vers la tâche de drain unique.
        // Si le canal n'est pas encore initialise (install_tracing_layer
        // n'a pas encore ete appele), on abandonne silencieusement.
        if let Some(tx) = crate::REGISTRY.log_tx.get() {
            let _ = tx.send(log);
        }
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

    /// Verifie que le canal unbounded preserve l'ordre FIFO des LogEvent,
    /// propriete sur laquelle la tache de drain de N6 s'appuie.
    /// Test purement synchrone : try_recv ne necessite pas de runtime tokio.
    #[test]
    fn log_drain_channel_preserves_fifo_order() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<LogEvent>();
        let e1 = LogEvent::new(LogLevel::Info, "app", "first");
        let e2 = LogEvent::new(LogLevel::Warn, "app", "second");
        let e3 = LogEvent::new(LogLevel::Error, "app", "third");
        tx.send(e1).unwrap();
        tx.send(e2).unwrap();
        tx.send(e3).unwrap();
        assert_eq!(rx.try_recv().unwrap().message, "first");
        assert_eq!(rx.try_recv().unwrap().message, "second");
        assert_eq!(rx.try_recv().unwrap().message, "third");
    }
}
