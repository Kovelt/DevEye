use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use deveye_core::{DomTree, StateDump};
use deveye_ports::{AppInspector, PortResult};

use crate::client::CdpClient;

#[derive(Deserialize)]
struct DocumentResult {
    root: serde_json::Value,
}

#[async_trait]
impl AppInspector for CdpClient {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        let result: DocumentResult = self
            .call("DOM.getDocument", json!({ "depth": -1, "pierce": true }))
            .await
            .map_err(Into::<deveye_ports::PortError>::into)?;
        Ok(DomTree::new(result.root))
    }

    /// L'état applicatif (signaux Dioxus, fixtures) ne passe pas par CDP : il
    /// est servi par le SDK opt-in côté app cible
    /// (`adapter-dioxus-signals`). Cette implémentation renvoie un dump vide,
    /// charge au binaire orchestrateur de composer les deux adapters
    /// (`AppInspector` CDP pour le DOM + `AppInspector` SDK pour le state).
    async fn state_snapshot(&self) -> PortResult<StateDump> {
        Ok(StateDump::empty())
    }
}
