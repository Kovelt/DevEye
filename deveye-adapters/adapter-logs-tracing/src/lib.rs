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
