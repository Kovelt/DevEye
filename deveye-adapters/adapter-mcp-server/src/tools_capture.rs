//! Outils de la famille **capture** : `screenshot`, `screenshot_region`,
//! `screenshot_element`, `assert_visual` — les « yeux » de DevEye (PNG du
//! viewport, recadrage, comparaison golden). Scinde de `lib.rs` (H1) : la
//! logique et les schemas d'outils sont inchanges, seul l'emplacement du
//! code bouge.

use super::*;

#[tool_router(router = capture_router, vis = "pub(crate)")]
impl DeveyeMcpServer {
    #[tool(
        description = "Capture un screenshot PNG du viewport de l'app cible (image MCP). En CDP, le PNG est à l'échelle CSS même sur un écran HiDPI : 1 pixel de l'image = 1 pixel CSS = l'unité des coordonnées de `click`/`hover`/`scroll`, sans facteur à appliquer. Sur les autres cibles, le PNG est en pixels de la fenêtre capturée."
    )]
    pub(crate) async fn screenshot(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.acquire_gate("screenshot").await;
        let shot = match unsupported_aware(self.renderer.screenshot().await) {
            Ok(v) => v,
            // La cible ne peut pas capturer (fenetre KWin/xcap introuvable...) :
            // isError semantique structure, pas une erreur protocolaire (E1).
            Err(issue) => return issue,
        };
        let b64 = B64.encode(&shot.bytes);
        // Contenu `image` (et non `text`) : le client MCP affiche le PNG —
        // c'est ce qui rend l'outil utilisable comme « yeux ».
        Ok(CallToolResult::success(vec![ContentBlock::image(
            b64,
            "image/png",
        )]))
    }

    #[tool(
        description = "Capture un screenshot puis le recadre sur la région (x, y, width, height) avant de renvoyer le PNG (image MCP). Réduit nettement le coût en tokens d'une vérification ciblée, et reste la seule façon d'inspecter un composant précis en kwin-eis (pas de DOM). La région est bornée aux dimensions de la capture."
    )]
    pub(crate) async fn screenshot_region(
        &self,
        Parameters(p): Parameters<ScreenshotRegionParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let shot = match unsupported_aware(self.renderer.screenshot().await) {
            Ok(v) => v,
            // La cible ne peut pas capturer (fenetre KWin/xcap introuvable...) :
            // isError semantique structure, pas une erreur protocolaire (E1).
            Err(issue) => return issue,
        };
        let cropped = crop_png(&shot.bytes, p.x, p.y, p.width, p.height).map_err(internal)?;
        let b64 = B64.encode(&cropped);
        Ok(CallToolResult::success(vec![ContentBlock::image(
            b64,
            "image/png",
        )]))
    }

    #[tool(
        description = "Capture un élément par sélecteur CSS : résout son rect via JS (le défile au centre), puis recadre la capture sur ce rect (mise à l'échelle DPI gérée). Renvoie le PNG (image MCP), ou `isError {found:false}` si l'élément est absent/invisible. Nécessite eval_js (cdp/sdk) ; indisponible en kwin-eis (utiliser screenshot_region)."
    )]
    pub(crate) async fn screenshot_element(
        &self,
        Parameters(p): Parameters<ScreenshotElementParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        // 1) Resoudre le rect de l'element (coordonnees CSS viewport).
        let sel = serde_json::to_string(&p.selector).unwrap_or_default();
        let snippet = format!(
            "(function(){{ const el = document.querySelector({sel}); \
             if (!el) return {{ found: false, reason: 'not_found' }}; \
             el.scrollIntoView({{ block: 'center', behavior: 'instant' }}); \
             const r = el.getBoundingClientRect(); \
             if (r.width === 0 || r.height === 0) return {{ found: false, reason: 'not_visible' }}; \
             return {{ found: true, x: r.left, y: r.top, width: r.width, height: r.height }}; }})()"
        );
        let rect = match unsupported_aware(self.evaluator.evaluate(&snippet).await) {
            Ok(v) => v,
            Err(issue) => return issue,
        };
        if !rect.get("found").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Ok(json_error(&rect));
        }
        // 2) Capturer puis recadrer. En CDP le PNG est a l'echelle CSS (ratio
        // 1), mais les autres renderers (xcap, kwin) peuvent rendre des pixels
        // physiques alors que le rect est en pixels CSS : on met a l'echelle
        // par le ratio dimensions-PNG / viewport-logique.
        let shot = match unsupported_aware(self.renderer.screenshot().await) {
            Ok(v) => v,
            // La cible ne peut pas capturer (fenetre KWin/xcap introuvable...) :
            // isError semantique structure, pas une erreur protocolaire (E1).
            Err(issue) => return issue,
        };
        let (pw, ph) = png_dimensions(&shot.bytes).map_err(internal)?;
        let scale_x = if shot.viewport.width > 0 {
            pw as f64 / shot.viewport.width as f64
        } else {
            1.0
        };
        let scale_y = if shot.viewport.height > 0 {
            ph as f64 / shot.viewport.height as f64
        } else {
            1.0
        };
        let get = |k: &str| rect.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let x = (get("x") * scale_x).max(0.0).round() as u32;
        let y = (get("y") * scale_y).max(0.0).round() as u32;
        let w = (get("width") * scale_x).round().max(1.0) as u32;
        let h = (get("height") * scale_y).round().max(1.0) as u32;
        let cropped = crop_png(&shot.bytes, x, y, w, h).map_err(internal)?;
        let b64 = B64.encode(&cropped);
        Ok(CallToolResult::success(vec![ContentBlock::image(
            b64,
            "image/png",
        )]))
    }

    #[tool(
        description = "Assertion visuelle (golden) : capture le viewport (ou la région x/y/width/height) et le compare au PNG `baseline_path`. Si le fichier n'existe pas, il est créé (`{ created: true }`). Sinon renvoie `{ ok, diff_ratio, threshold, same_dims, ... }` ; `isError` si le ratio de pixels différents dépasse `threshold` (défaut 0.01) ou si les dimensions diffèrent. Premier outil d'assertion déterministe disponible en kwin-eis (sans DOM/JS)."
    )]
    pub(crate) async fn assert_visual(
        &self,
        Parameters(p): Parameters<AssertVisualParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let shot = match unsupported_aware(self.renderer.screenshot().await) {
            Ok(v) => v,
            // La cible ne peut pas capturer (fenetre KWin/xcap introuvable...) :
            // isError semantique structure, pas une erreur protocolaire (E1).
            Err(issue) => return issue,
        };
        // Recadrage optionnel : les 4 bornes doivent etre fournies ensemble.
        // B10 : `Cow` evite de cloner tout le PNG quand aucun recadrage n'est
        // demande (chemin le plus frequent) — seul le cas recadre alloue,
        // via le buffer deja produit par `crop_png`.
        let current: std::borrow::Cow<'_, [u8]> = match (p.x, p.y, p.width, p.height) {
            (Some(x), Some(y), Some(w), Some(h)) => {
                std::borrow::Cow::Owned(crop_png(&shot.bytes, x, y, w, h).map_err(internal)?)
            }
            _ => std::borrow::Cow::Borrowed(&shot.bytes),
        };
        let path = std::path::Path::new(&p.baseline_path);
        if !tokio::fs::try_exists(path).await.map_err(internal)? {
            // Baseline absente : on l'ecrit a partir de la capture courante.
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    tokio::fs::create_dir_all(parent).await.map_err(internal)?;
                }
            }
            tokio::fs::write(path, &current).await.map_err(internal)?;
            return Ok(json_success(&serde_json::json!({
                "ok": true,
                "created": true,
                "baseline_path": p.baseline_path,
            })));
        }
        let baseline = tokio::fs::read(path).await.map_err(internal)?;
        let threshold = p.threshold.unwrap_or(0.01);
        let outcome = visual_diff(&baseline, &current).map_err(internal)?;
        let ok = outcome.same_dims && outcome.diff_ratio <= threshold;
        let payload = serde_json::json!({
            "ok": ok,
            "created": false,
            "diff_ratio": outcome.diff_ratio,
            "threshold": threshold,
            "same_dims": outcome.same_dims,
            "baseline_dims": { "width": outcome.baseline_dims.0, "height": outcome.baseline_dims.1 },
            "current_dims": { "width": outcome.current_dims.0, "height": outcome.current_dims.1 },
            "baseline_path": p.baseline_path,
        });
        if ok {
            Ok(json_success(&payload))
        } else {
            Ok(json_error(&payload))
        }
    }
}
