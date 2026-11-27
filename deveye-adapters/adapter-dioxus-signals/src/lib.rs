//! Adapter DevEye consommant le SDK opt-in `deveye-sdk` côté app cible.
//!
//! - [`DioxusSignalsInspector`] : `AppInspector::state_snapshot` (le state
//!   exposé par l'app). Pour le DOM, composer avec `adapter-cdp-webview` —
//!   ou avec le **pont natif** ci-dessous quand le webview n'a pas de CDP.
//! - Module `bridge` : yeux et mains via le SDK (`SdkEvaluator`,
//!   `JsInteraction`, `EvalDomInspector`) pour piloter l'app desktop
//!   WebKitGTK sans CDP.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

mod bridge;

pub use bridge::{EvalDomInspector, JsInteraction, SdkEvaluator};

use async_trait::async_trait;
use serde::Deserialize;

use deveye_core::{DomTree, StateDump};
use deveye_ports::{AppInspector, PortError, PortResult};

#[derive(Clone, Debug)]
pub struct DioxusSignalsInspector {
    base_url: String,
    http: reqwest::Client,
}

impl DioxusSignalsInspector {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: deveye_sdk_client::client(),
        }
    }
}

#[derive(Deserialize)]
struct StatePayload {
    entries: std::collections::BTreeMap<String, serde_json::Value>,
}

#[async_trait]
impl AppInspector for DioxusSignalsInspector {
    /// Le DOM ne passe pas par le SDK : un appel ici renvoie un arbre vide.
    /// Le binaire orchestrateur compose cet inspector avec un
    /// `adapter-cdp-webview` pour obtenir le DOM réel.
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        Ok(DomTree::new(serde_json::Value::Null))
    }

    async fn state_snapshot(&self) -> PortResult<StateDump> {
        let url = format!("{}/devtools/state", self.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| PortError::Network(format!("GET {url} : {e}")))?;
        if !resp.status().is_success() {
            return Err(PortError::Inspect(format!(
                "SDK a répondu {} pour {url}",
                resp.status()
            )));
        }
        let payload: StatePayload = resp
            .json()
            .await
            .map_err(|e| PortError::Inspect(format!("decode JSON state : {e}")))?;
        Ok(StateDump::new(payload.entries))
    }
}
