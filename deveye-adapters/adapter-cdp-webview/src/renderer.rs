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

/// Viewport de mise en page, en pixels CSS. `pageX`/`pageY` sont la position
/// de défilement : l'origine d'un `clip` CDP est en coordonnées de PAGE, pas
/// de viewport.
#[derive(Deserialize)]
struct ViewportRect {
    #[serde(default, rename = "pageX")]
    page_x: f64,
    #[serde(default, rename = "pageY")]
    page_y: f64,
    #[serde(rename = "clientWidth")]
    client_width: f64,
    #[serde(rename = "clientHeight")]
    client_height: f64,
}

impl ViewportRect {
    fn size(&self) -> Size {
        Size::new(
            self.client_width.max(0.0) as u32,
            self.client_height.max(0.0) as u32,
        )
    }
}

/// Écart toléré entre la largeur du PNG et celle du viewport avant de
/// considérer que la capture n'est pas à l'échelle CSS (arrondis de
/// `devicePixelRatio` fractionnaires, 1,25 par exemple).
const CSS_SCALE_TOLERANCE: f64 = 1.005;

// B8 (audit 2026-07-04, reporte) : `Page.captureScreenshot` accepte un
// parametre `clip` natif qui recadrerait a la source au lieu de renvoyer la
// pleine resolution (le recadrage se fait aujourd'hui cote Rust, apres coup,
// dans `adapter-mcp-server::crop_png`). Une methode `screenshot_clip(rect)`
// sur le trait `Renderer` avec impl par defaut (delegant a `screenshot()` +
// crop) permettrait a CDP de surcharger avec le `clip` natif sans casser les
// 5-6 autres implémenteurs. Non fait : changement de trait, a grouper avec
// d'autres evolutions de `Renderer` après la 1.0.

#[async_trait]
impl Renderer for CdpClient {
    /// Capture le viewport **à l'échelle CSS** : un pixel du PNG vaut un pixel
    /// CSS, l'unité des coordonnées de `click`/`hover`/`scroll` (ADR-0022).
    ///
    /// Un `Page.captureScreenshot` simple rend des pixels physiques : sur un
    /// écran HiDPI le PNG est plus grand que le viewport et tout point lu sur
    /// l'image est décalé. On capture donc un `clip` couvrant le viewport. Mais
    /// `clip.scale = 1` donne des pixels *physiques* (mesuré sur Chromium avec
    /// un facteur d'échelle réel de 1,25 à 2) : si le PNG dépasse le viewport,
    /// on relit son ratio et on recapture à `scale = 1 / ratio`, sans JS (le
    /// moteur JS est suspendu pendant un dialog). Sur un écran à 100 %, une
    /// seule capture suffit.
    async fn screenshot(&self) -> PortResult<Screenshot> {
        let rect = self.layout_viewport().await?;
        let viewport = rect.size();
        let origin = (rect.page_x, rect.page_y);

        let mut captured = self.capture(capture_params(viewport, origin, 1.0)).await?;
        if let Some(scale) = css_rescale(png_width(&captured), viewport.width) {
            captured = self
                .capture(capture_params(viewport, origin, scale))
                .await?;
        }
        Ok(Screenshot::new_png(captured, viewport))
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        Ok(self.layout_viewport().await?.size())
    }
}

impl CdpClient {
    async fn layout_viewport(&self) -> PortResult<ViewportRect> {
        let metrics: LayoutMetrics = self
            .call("Page.getLayoutMetrics", json!({}))
            .await
            .map_err(PortError::from)?;
        Ok(metrics.css_layout_viewport)
    }

    async fn capture(&self, params: serde_json::Value) -> PortResult<Vec<u8>> {
        let captured: CaptureResult = self
            .call("Page.captureScreenshot", params)
            .await
            .map_err(PortError::from)?;
        B64.decode(captured.data.as_bytes())
            .map_err(|e| CdpError::Base64(e.to_string()))
            .map_err(PortError::from)
    }
}

/// Paramètres de `Page.captureScreenshot` pour capturer le viewport. `origin`
/// est la position de défilement (coordonnées de page, cf. [`ViewportRect`]).
/// Le `clip` n'est posé que si le viewport est non dégénéré : un `clip` de
/// taille nulle ferait échouer la commande CDP.
fn capture_params(viewport: Size, origin: (f64, f64), scale: f64) -> serde_json::Value {
    let mut params = json!({ "format": "png", "captureBeyondViewport": false });
    if viewport.width > 0 && viewport.height > 0 {
        params["clip"] = json!({
            "x": origin.0,
            "y": origin.1,
            "width": viewport.width,
            "height": viewport.height,
            "scale": scale,
        });
    }
    params
}

/// Largeur en pixels d'un PNG, lue dans l'en-tête IHDR sans décoder l'image.
fn png_width(png: &[u8]) -> Option<u32> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if png.len() < 24 || !png.starts_with(SIGNATURE) || &png[12..16] != b"IHDR" {
        return None;
    }
    Some(u32::from_be_bytes([png[16], png[17], png[18], png[19]]))
}

/// Échelle de recapture qui ramène un PNG de `png_width` pixels sur un
/// viewport de `css_width` pixels CSS, ou `None` si la capture est déjà à
/// l'échelle CSS (ou si l'une des largeurs est inconnue).
fn css_rescale(png_width: Option<u32>, css_width: u32) -> Option<f64> {
    let png_width = f64::from(png_width?);
    let css_width = f64::from(css_width);
    if css_width <= 0.0 || png_width / css_width <= CSS_SCALE_TOLERANCE {
        return None;
    }
    Some(css_width / png_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_header(width: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&100u32.to_be_bytes());
        png
    }

    #[test]
    fn capture_params_clips_the_viewport_at_the_scroll_origin() {
        // L'origine d'un clip est en coordonnées de PAGE : à 1000 px de
        // défilement, un clip en (0, 0) capturerait le haut du document.
        let params = capture_params(Size::new(1024, 768), (0.0, 1000.0), 1.0);
        let clip = params.get("clip").expect("clip présent");
        assert_eq!(clip["x"], json!(0.0));
        assert_eq!(clip["y"], json!(1000.0));
        assert_eq!(clip["width"], json!(1024));
        assert_eq!(clip["height"], json!(768));
        assert_eq!(clip["scale"], json!(1.0));
        assert_eq!(params["captureBeyondViewport"], json!(false));
    }

    #[test]
    fn capture_params_omits_clip_when_viewport_is_degenerate() {
        let params = capture_params(Size::new(0, 0), (0.0, 0.0), 1.0);
        assert!(params.get("clip").is_none());
        assert_eq!(params["format"], json!("png"));
    }

    #[test]
    fn png_width_reads_the_ihdr_header() {
        assert_eq!(png_width(&png_header(1570)), Some(1570));
        assert_eq!(png_width(b"pas un png"), None);
        assert_eq!(png_width(&[]), None);
    }

    #[test]
    fn css_rescale_is_none_when_the_capture_is_already_css_scale() {
        assert_eq!(css_rescale(Some(785), 785), None);
        // Un pixel d'écart d'arrondi ne justifie pas une recapture.
        assert_eq!(css_rescale(Some(1024), 1024), None);
    }

    #[test]
    fn css_rescale_brings_a_hidpi_capture_back_to_css_pixels() {
        // Écran 2x : 785 px CSS capturés en 1570 px physiques.
        let scale = css_rescale(Some(1570), 785).expect("recapture requise");
        assert!((scale - 0.5).abs() < 1e-9);
        // Facteur fractionnaire (1,25) : 787 px CSS donnent 984 px physiques.
        let scale = css_rescale(Some(984), 787).expect("recapture requise");
        assert!((f64::from(787u32) / scale - 984.0).abs() < 1e-6);
    }

    #[test]
    fn css_rescale_ignores_unknown_or_degenerate_widths() {
        assert_eq!(css_rescale(None, 785), None);
        assert_eq!(css_rescale(Some(1570), 0), None);
    }
}
