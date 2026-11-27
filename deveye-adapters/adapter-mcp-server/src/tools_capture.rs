//! Outils de la famille **capture** : `screenshot`, `screenshot_region`,
//! `screenshot_element`, `assert_visual` — les « yeux » de DevEye (image du
//! viewport, recadrage, comparaison golden). Les trois captures acceptent les
//! options de coût `max_width`/`format`/`quality` (ADR-0029) ; `assert_visual`
//! sait ignorer des régions, rendre l'image de différence et mettre à jour sa
//! baseline (ADR-0030).

use super::imaging::{
    encode_capture, render_diff_png, validate_masks, visual_diff, EncodedImage, ResolvedCapture,
};
use super::*;

/// Résultat des trois outils de capture : bloc image, puis un bloc texte JSON
/// de métadonnées (taille finale, facteur d'échelle) joint à `extra`.
fn capture_blocks(
    enc: &EncodedImage,
    origin: Option<(u32, u32)>,
    mut extra: serde_json::Value,
) -> CallToolResult {
    extra["image"] = enc.meta(origin);
    CallToolResult::success(vec![enc.content(), meta_block(&extra)])
}

/// Applique les options de capture à un PNG déjà recadré/complet.
fn finish_capture(
    png: &[u8],
    opts: &ResolvedCapture,
    origin: Option<(u32, u32)>,
    extra: serde_json::Value,
) -> Result<CallToolResult, McpError> {
    let enc = encode_capture(png, opts)?;
    Ok(capture_blocks(&enc, origin, extra))
}

#[tool_router(router = capture_router, vis = "pub(crate)")]
impl DeveyeMcpServer {
    #[tool(
        description = "Capture un screenshot du viewport de l'app cible (image MCP, PNG par défaut). En CDP, le PNG est à l'échelle CSS même sur un écran HiDPI : 1 pixel de l'image = 1 pixel CSS = l'unité des coordonnées de `click`/`hover`/`scroll`, sans facteur à appliquer. Sur les autres cibles, le PNG est en pixels de la fenêtre capturée. Options de coût en tokens : `max_width` (réduction proportionnelle, jamais d'agrandissement), `format` (`png` ou `jpeg`) et `quality` (JPEG, 1 à 100, défaut 80). Un second bloc texte JSON `{ captured_at, viewport: { width, height }, image: { format, width, height, source_width, source_height, scale, bytes } }` date la capture (RFC 3339, UTC), donne la taille du viewport en pixels CSS et le facteur d'échelle : si `scale` != 1, diviser une coordonnée lue sur l'image par `scale` pour obtenir la coordonnée de capture (puis de `click` en CDP).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(crate) async fn screenshot(
        &self,
        Parameters(p): Parameters<ScreenshotParams>,
    ) -> Result<CallToolResult, McpError> {
        let opts = p.capture.resolve()?;
        let _serial = self.acquire_gate("screenshot").await;
        let shot = match unsupported_aware(self.renderer.screenshot().await) {
            Ok(v) => v,
            // La cible ne peut pas capturer (fenetre KWin/xcap introuvable...) :
            // isError semantique structure, pas une erreur protocolaire (E1).
            Err(issue) => return issue,
        };
        // Contenu `image` (et non `text`) : le client MCP affiche l'image —
        // c'est ce qui rend l'outil utilisable comme « yeux ». Un second bloc
        // texte date la capture et donne la taille du viewport (ADR-0025) puis
        // la taille finale et le facteur d'échelle (ADR-0029).
        finish_capture(
            &shot.bytes,
            &opts,
            None,
            serde_json::json!({
                "captured_at": shot.captured_at,
                "viewport": { "width": shot.viewport.width, "height": shot.viewport.height },
            }),
        )
    }

    #[tool(
        description = "Capture un screenshot puis le recadre sur la région (x, y, width, height) avant de renvoyer l'image (image MCP, PNG par défaut). Réduit nettement le coût en tokens d'une vérification ciblée, et reste la seule façon d'inspecter un composant précis en kwin-eis (pas de DOM). La région est bornée aux dimensions de la capture. Mêmes options de coût que `screenshot` (`max_width`, `format`, `quality`). Un second bloc texte JSON `{ image: { ..., scale, origin: { x, y } } }` donne la taille finale et le facteur d'échelle : coordonnée de capture = `origin` + coordonnée de l'image / `scale`.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(crate) async fn screenshot_region(
        &self,
        Parameters(p): Parameters<ScreenshotRegionParams>,
    ) -> Result<CallToolResult, McpError> {
        let opts = p.capture.resolve()?;
        let _serial = self.gate.lock().await;
        let shot = match unsupported_aware(self.renderer.screenshot().await) {
            Ok(v) => v,
            // La cible ne peut pas capturer (fenetre KWin/xcap introuvable...) :
            // isError semantique structure, pas une erreur protocolaire (E1).
            Err(issue) => return issue,
        };
        let cropped = crop_png(&shot.bytes, p.x, p.y, p.width, p.height).map_err(internal)?;
        finish_capture(&cropped, &opts, Some((p.x, p.y)), serde_json::json!({}))
    }

    #[tool(
        description = "Capture un élément par sélecteur CSS : résout son rect via JS (le défile au centre), puis recadre la capture sur ce rect (mise à l'échelle DPI gérée). Renvoie l'image (image MCP, PNG par défaut), ou `isError {found:false}` si l'élément est absent/invisible. Mêmes options de coût que `screenshot` (`max_width`, `format`, `quality`) ; un second bloc texte JSON `{ image: { ..., scale, origin: { x, y } } }` donne la taille finale, le facteur d'échelle et la position de l'élément dans la capture. L'élément est défilé au centre (le défilement de la page change). Nécessite eval_js (cdp/sdk) ; indisponible en kwin-eis (utiliser screenshot_region).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(crate) async fn screenshot_element(
        &self,
        Parameters(p): Parameters<ScreenshotElementParams>,
    ) -> Result<CallToolResult, McpError> {
        let opts = p.capture.resolve()?;
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
        finish_capture(&cropped, &opts, Some((x, y)), serde_json::json!({}))
    }

    #[tool(
        description = "Assertion visuelle (golden) : capture le viewport (ou la région x/y/width/height) et le compare au PNG `baseline_path`. Si le fichier n'existe pas, il est créé (`{ created: true }`). Sinon renvoie `{ ok, diff_ratio, threshold, same_dims, diff_pixels, compared_pixels, diff_bbox, ... }` ; `isError` si le ratio de pixels différents dépasse `threshold` (défaut 0.01) ou si les dimensions diffèrent. En cas d'échec à dimensions égales, une IMAGE DE DIFFÉRENCE suit le JSON (fond éclairci, pixels différents en rouge, zones ignorées teintées de bleu ; largeur bornée à 1280 px, `diff_image.scale` donne l'échelle) et `diff_bbox` {x, y, width, height} délimite précisément la zone qui diverge. `ignore_regions` [{x, y, width, height}] exclut des zones mouvantes (horloge, curseur) de la comparaison et du dénominateur de `diff_ratio` ; leurs coordonnées sont celles de l'image comparée (relatives à la région si elle est fournie). `update_baseline: true` remplace la baseline par la capture courante sans comparer (`{ updated: true }`) : à n'utiliser qu'après validation du changement. Premier outil d'assertion déterministe disponible en kwin-eis (sans DOM/JS).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(crate) async fn assert_visual(
        &self,
        Parameters(p): Parameters<AssertVisualParams>,
    ) -> Result<CallToolResult, McpError> {
        let masks = p.ignore_regions.clone().unwrap_or_default();
        validate_masks(&masks)?;
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
        let exists = tokio::fs::try_exists(path).await.map_err(internal)?;
        let update = p.update_baseline.unwrap_or(false);
        if !exists || update {
            // Baseline absente, ou remplacement explicite : on l'ecrit a
            // partir de la capture courante.
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    tokio::fs::create_dir_all(parent).await.map_err(internal)?;
                }
            }
            tokio::fs::write(path, &current).await.map_err(internal)?;
            let mut payload = serde_json::json!({
                "ok": true,
                "created": !exists,
                "baseline_path": p.baseline_path,
            });
            if update {
                payload["updated"] = serde_json::Value::Bool(true);
            }
            return Ok(json_success(&payload));
        }
        let baseline = tokio::fs::read(path).await.map_err(internal)?;
        let threshold = p.threshold.unwrap_or(0.01);
        let outcome = visual_diff(&baseline, &current, &masks).map_err(internal)?;
        let ok = outcome.same_dims && outcome.diff_ratio <= threshold;
        let mut payload = serde_json::json!({
            "ok": ok,
            "created": false,
            "diff_ratio": outcome.diff_ratio,
            "threshold": threshold,
            "same_dims": outcome.same_dims,
            "baseline_dims": { "width": outcome.baseline_dims.0, "height": outcome.baseline_dims.1 },
            "current_dims": { "width": outcome.current_dims.0, "height": outcome.current_dims.1 },
            "baseline_path": p.baseline_path,
        });
        if outcome.same_dims {
            payload["diff_pixels"] = outcome.diff_pixels.into();
            payload["compared_pixels"] = outcome.compared_pixels.into();
            payload["ignored_regions"] = masks.len().into();
            if let Some(b) = outcome.diff_bbox {
                payload["diff_bbox"] =
                    serde_json::json!({ "x": b.x, "y": b.y, "width": b.width, "height": b.height });
            }
        }
        if ok {
            return Ok(json_success(&payload));
        }
        // Echec a dimensions egales : l'image de difference montre OU ca
        // diverge, sans que l'agent ait a ouvrir deux captures. Pas d'image si
        // les dimensions different (rien de pixel a pixel a dessiner).
        let mut blocks = Vec::with_capacity(2);
        let diff_image = match &outcome.images {
            Some((baseline_img, current_img)) => {
                Some(render_diff_png(baseline_img, current_img, &masks)?)
            }
            None => None,
        };
        if let Some(enc) = &diff_image {
            payload["diff_image"] = enc.meta(None);
        }
        blocks.push(ContentBlock::text(
            serde_json::to_string(&payload).unwrap_or_else(|_| "null".to_string()),
        ));
        if let Some(enc) = &diff_image {
            blocks.push(enc.content());
        }
        Ok(CallToolResult::error(blocks))
    }
}
