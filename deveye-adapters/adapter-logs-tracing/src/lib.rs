//! Adapter DevEye implémentant `LogSource` en draining le buffer tracing
//! du SDK opt-in via `GET /devtools/logs/poll`.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use deveye_core::LogEvent;
use deveye_ports::{LogSource, PortError, PortResult};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

pub const DEFAULT_SDK_URL: &str = "http://127.0.0.1:31337";

#[derive(Clone, Debug)]
pub struct SdkTracingLogSource {
    base_url: String,
    http: reqwest::Client,
}

impl SdkTracingLogSource {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("client reqwest (TLS rustls indisponible)"),
        }
    }

    pub fn with_default_url() -> Self {
        Self::new(DEFAULT_SDK_URL)
    }
}

#[derive(Deserialize)]
struct LogsPayload {
    events: Vec<LogEvent>,
}

#[async_trait]
impl LogSource for SdkTracingLogSource {
    async fn poll(&self) -> PortResult<Vec<LogEvent>> {
        let url = format!("{}/devtools/logs/poll", self.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| PortError::Network(format!("GET {url} : {e}")))?;
        if !resp.status().is_success() {
            return Err(PortError::Log(format!(
                "SDK a répondu {} pour {url}",
                resp.status()
            )));
        }
        let payload: LogsPayload = resp
            .json()
            .await
            .map_err(|e| PortError::Log(format!("decode JSON logs : {e}")))?;
        Ok(payload.events)
    }
}

#[cfg(test)]
mod tests {
    use super::SdkTracingLogSource;
    use deveye_ports::{LogSource, PortError};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Construit un LogEvent JSON minimal valide (tous les champs obligatoires).
    fn sample_event_json() -> serde_json::Value {
        serde_json::json!({
            "level": "info",
            "target": "my_app::module",
            "message": "server started",
            "timestamp": "2026-06-26T10:00:00Z"
        })
    }

    /// 200 + corps JSON valide => les evenements sont deserialises et retournes.
    #[tokio::test]
    async fn poll_ok_returns_events() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/devtools/logs/poll"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "events": [sample_event_json()] })),
            )
            .mount(&server)
            .await;

        let adapter = SdkTracingLogSource::new(server.uri());
        let events = adapter.poll().await.expect("should succeed on 200");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].message, "server started");
        assert_eq!(events[0].target, "my_app::module");
    }

    /// 200 + tableau vide => vecteur vide retourne sans erreur.
    #[tokio::test]
    async fn poll_ok_empty_events_returns_empty_vec() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/devtools/logs/poll"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "events": [] })),
            )
            .mount(&server)
            .await;

        let adapter = SdkTracingLogSource::new(server.uri());
        let events = adapter.poll().await.expect("should succeed on 200 empty");
        assert!(events.is_empty());
    }

    /// 500 => PortError::Log (le SDK a signale une erreur cote serveur).
    #[tokio::test]
    async fn poll_500_returns_log_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/devtools/logs/poll"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let adapter = SdkTracingLogSource::new(server.uri());
        let err = adapter.poll().await.expect_err("should fail on 500");
        assert!(
            matches!(err, PortError::Log(_)),
            "expected PortError::Log, got {err:?}"
        );
    }

    /// 200 + corps non-JSON => PortError::Log (echec de deserialisation).
    #[tokio::test]
    async fn poll_200_invalid_json_returns_log_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/devtools/logs/poll"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not valid json"))
            .mount(&server)
            .await;

        let adapter = SdkTracingLogSource::new(server.uri());
        let err = adapter
            .poll()
            .await
            .expect_err("should fail on bad JSON body");
        assert!(
            matches!(err, PortError::Log(_)),
            "expected PortError::Log, got {err:?}"
        );
    }
}
