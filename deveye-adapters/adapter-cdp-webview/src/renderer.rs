use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::Deserialize;
use serde_json::json;

use deveye_core::{Screenshot, Size};
use deveye_ports::{PortError, PortResult, Renderer};

use crate::client::CdpClient;
use crate::error::CdpError;

#[derive(Deserialize)]
struct CaptureResult {
    data: String,
}

#[derive(Deserialize)]
struct LayoutMetrics {
    #[serde(rename = "cssLayoutViewport")]
    css_layout_viewport: ViewportRect,
}

#[derive(Deserialize)]
struct ViewportRect {
    #[serde(rename = "clientWidth")]
    client_width: f64,
    #[serde(rename = "clientHeight")]
    client_height: f64,
}

#[async_trait]
impl Renderer for CdpClient {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        let captured: CaptureResult = self
            .call(
                "Page.captureScreenshot",
                json!({ "format": "png", "captureBeyondViewport": false }),
            )
            .await
            .map_err(PortError::from)?;
        let bytes = B64
            .decode(captured.data.as_bytes())
            .map_err(|e| CdpError::Base64(e.to_string()))
            .map_err(PortError::from)?;
        let viewport = self.viewport_size().await?;
        Ok(Screenshot::new_png(bytes, viewport))
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        let metrics: LayoutMetrics = self
            .call("Page.getLayoutMetrics", json!({}))
            .await
            .map_err(PortError::from)?;
        Ok(Size::new(
            metrics.css_layout_viewport.client_width.max(0.0) as u32,
            metrics.css_layout_viewport.client_height.max(0.0) as u32,
        ))
    }
}
