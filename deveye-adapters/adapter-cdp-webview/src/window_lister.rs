use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use deveye_core::WindowInfo;
use deveye_ports::{PortError, PortResult, WindowLister};

use crate::client::CdpClient;

#[derive(Debug, Deserialize)]
struct RawTarget {
    #[serde(rename = "targetId")]
    target_id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
}

#[derive(Debug, Deserialize)]
struct GetTargetsResult {
    #[serde(rename = "targetInfos")]
    target_infos: Vec<RawTarget>,
}

#[async_trait]
impl WindowLister for CdpClient {
    async fn list_windows(&self) -> PortResult<Vec<WindowInfo>> {
        // Browser-level : `Target.getTargets` ne dépend pas d'une session page.
        let result: GetTargetsResult = self
            .send_browser("Target.getTargets", json!({}))
            .await
            .map_err(PortError::from)?;
        Ok(result
            .target_infos
            .into_iter()
            .filter(|t| t.kind == "page")
            .map(|t| WindowInfo {
                id: t.target_id,
                title: t.title,
                url: t.url,
                kind: t.kind,
            })
            .collect())
    }
}
