//! Layer `tracing` qui capte les événements émis par l'app et les pousse
//! dans le buffer global servi par `GET /devtools/logs/poll`.

use std::fmt::Write;

use tracing::{
    field::{Field, Visit},
    Event, Level, Subscriber,
};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};

use deveye_core::{LogEvent, LogLevel};

/// Échec de [`try_install_tracing_layer`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TracingInstallError {
    /// Un abonné `tracing` global existe déjà (le process n'en admet qu'un) :
    /// composer [`layer`] avec cet abonné, ou pousser les logs par
    /// [`log`](crate::log).
    #[error("un abonné tracing global est déjà installé : composer deveye_sdk::layer() avec lui")]
    SubscriberAlreadySet,
}

/// Installe un abonné `tracing` global qui ne contient que le layer DevEye.
///
/// Renvoie [`TracingInstallError::SubscriberAlreadySet`] si un autre abonné
/// global existe : rien n'est alors posé et `/devtools/info` n'annonce pas la
/// capacité `logs`. Un second appel après un succès est sans effet (`Ok`).
/// En build **release**, no-op (`Ok`), comme [`init`](crate::init).
pub fn try_install_tracing_layer() -> Result<(), TracingInstallError> {
    #[cfg(debug_assertions)]
    {
        use tracing_subscriber::layer::SubscriberExt;
        // Verrou tenu pendant tout le test-and-set : deux fils concurrents ne peuvent
        // pas conclure à tort qu'un abonné étranger bloque la capture.
        static INSTALLED: std::sync::Mutex<bool> = std::sync::Mutex::new(false);
        let mut installed = INSTALLED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *installed {
            return Ok(());
        }
        let subscriber = tracing_subscriber::Registry::default().with(DeveyeLayer(()));
        tracing::subscriber::set_global_default(subscriber)
            .map_err(|_| TracingInstallError::SubscriberAlreadySet)?;
        *installed = true;
        crate::enable_logs();
    }
    Ok(())
}

/// Variante historique de [`try_install_tracing_layer`] : l'échec (abonné déjà
/// présent) est ignoré. À préférer : la variante `try_`, qui le signale.
pub fn install_tracing_layer() {
    let _ = try_install_tracing_layer();
}

/// Layer `tracing` à composer avec l'abonné d'un hôte qui possède déjà le
/// sien : `registry().with(deveye_sdk::layer())`. Capture les événements au
/// niveau de `DEVEYE_LOG_LEVEL` (défaut `info`). No-op en build release.
///
/// La capacité `logs` est annoncée dès la construction du layer, qu'il soit
/// branché ou non : un layer construit mais jamais composé dans un abonné actif
/// laisse `logs_tail` vide.
///
/// Expose `tracing_subscriber::Layer` 0.3 dans l'API (couplage consigné par
/// ADR-0066, ADR-0070).
pub fn layer() -> DeveyeLayer {
    #[cfg(debug_assertions)]
    crate::enable_logs();
    DeveyeLayer(())
}

/// Layer DevEye ; se construit uniquement par [`layer`].
#[derive(Clone, Copy, Debug)]
pub struct DeveyeLayer(());

impl<S> Layer<S> for DeveyeLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        #[cfg(not(debug_assertions))]
        let _ = event;

        #[cfg(debug_assertions)]
        {
            let level = map_level(*event.metadata().level());
            if level < crate::min_level() {
                return;
            }
            let mut visitor = MessageVisitor::default();
            event.record(&mut visitor);
            crate::push_log(
                LogEvent::new(
                    level,
                    event.metadata().target(),
                    visitor.message.unwrap_or_default(),
                )
                .with_fields(visitor.fields.unwrap_or(serde_json::Value::Null)),
            );
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
    // Convention (banc d'essai DevEye, 28 juin 2026) : le `message` est rendu
    // via `{:?}`. Pour `info!("… {}", x)` la valeur est un `fmt::Arguments` dont
    // `Debug` délègue à `Display` → pas de guillemets. Idem `info!(message = %x)`.
    // En revanche `info!(message = ?x)` avec `x: &str`/`String` produit
    // `"…"` (guillemets parasites). Règle : ne PAS utiliser `message = ?x` —
    // interpoler (`info!("{x:?}")`) ou préférer `message = %x`.
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
