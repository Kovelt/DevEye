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

// B8 (audit 2026-07-04, reporte) : `Page.captureScreenshot` accepte un
// parametre `clip` natif qui recadrerait a la source au lieu de renvoyer la
// pleine resolution (le recadrage se fait aujourd'hui cote Rust, apres coup,
// dans `adapter-mcp-server::crop_png`). Une methode `screenshot_clip(rect)`
// sur le trait `Renderer` avec impl par defaut (delegant a `screenshot()` +
// crop) permettrait a CDP de surcharger avec le `clip` natif sans casser les
// 5-6 autres implementeurs (xcap Window/Monitor, KWin, LazyCdp, doublures de
// test `adapter-composite`). Non fait dans ce sprint car invasif au-dela du
// perimetre de ce fichier : (1) le trait vit dans `deveye-ports`, en dehors
// du perimetre exclusif de cette tache ; (2) l'impl par defaut a besoin du
// crop PNG (`crop_png`/`png_dimensions`), qui vit aujourd'hui dans
// `adapter-mcp-server` — un crate qui *depend de* `deveye-ports`, pas
// l'inverse ; le faire proprement demande soit de faire remonter ce crop
// dans `deveye-ports` (nouvelle dependance a l'encodage PNG), soit de le
// dupliquer, ce qui deborde largement de ce fichier. A reprendre dans un
// sprint dedie aux changements de trait, groupe avec d'autres evolutions de
// `Renderer` pour ne payer le risque de cascade qu'une fois.

#[async_trait]
impl Renderer for CdpClient {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        // B9 (audit 2026-07-04) : capture et metriques de viewport sont deux
        // commandes CDP independantes (aucune ne consomme le resultat de
        // l'autre) — on les envoie desormais en parallele plutot qu'en
        // sequence (l'ancien code attendait la fin complete de
        // `Page.captureScreenshot` avant meme d'emettre
        // `Page.getLayoutMetrics`). Double gain : la latence de
        // getLayoutMetrics, avant purement additive, est maintenant recouverte
        // par celle de la capture (qui est de toute facon sur le chemin
        // chaud pour les 4 outils MCP) ; et les deux commandes partant
        // quasi simultanement, la fenetre de non-atomicite entre l'image et
        // son viewport associe (un resize de la fenetre entre les deux
        // appels) se retrouve fortement reduite par rapport a l'ancien
        // enchainement strictement sequentiel.
        //
        // Elimination totale de l'aller-retour getLayoutMetrics pour les 3
        // outils sur 4 qui n'utilisent pas `Screenshot.viewport`
        // (screenshot, screenshot_region, assert_visual — seul
        // screenshot_element s'en sert, pour la mise a l'echelle DPI) non
        // retenue ici : `viewport` doit rester en pixels CSS logiques pour
        // ce consommateur, alors que le PNG decode est en pixels physiques
        // (HiDPI) — les confondre casserait silencieusement le recadrage DPI
        // de `screenshot_element`. Un tel routage conditionnel demanderait
        // de faire remonter le besoin jusqu'a l'appelant (trait ou
        // adapter-mcp-server), hors perimetre de ce fichier.
        let (captured, viewport) = futures_util::future::join(
            self.call::<_, CaptureResult>(
                "Page.captureScreenshot",
                json!({ "format": "png", "captureBeyondViewport": false }),
            ),
            self.viewport_size(),
        )
        .await;
        let captured = captured.map_err(PortError::from)?;
        let viewport = viewport?;
        let bytes = B64
            .decode(captured.data.as_bytes())
            .map_err(|e| CdpError::Base64(e.to_string()))
            .map_err(PortError::from)?;
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
