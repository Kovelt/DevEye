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
        // 2) Capturer puis recadrer. En CDP le PNG est a l'échelle CSS (ratio
        // 1), mais les autres renderers (xcap, kwin) peuvent rendre des pixels
        // physiques alors que le rect est en pixels CSS : on met a l'échelle
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
        description = "Assertion visuelle (golden) : capture le viewport (ou la région x/y/width/height) et le compare au PNG `baseline_path`. `baseline_path` est confiné : relatif à `DEVEYE_BASELINE_DIR` (défaut : répertoire courant du serveur), extension `.png`, sans `..` ni lien symbolique sortant ; un chemin hors de cette racine est refusé (`invalid_params`). Si le fichier n'existe pas, il est créé (`{ created: true }`). Sinon renvoie `{ ok, diff_ratio, threshold, same_dims, diff_pixels, compared_pixels, diff_bbox, ... }` ; `isError` si le ratio de pixels différents dépasse `threshold` (défaut 0.01) ou si les dimensions diffèrent, ou si `ignore_regions` couvre toute l'image (`reason: \"fully_masked\"`, aucun pixel comparé). En cas d'échec à dimensions égales, une IMAGE DE DIFFÉRENCE suit le JSON (fond éclairci, pixels différents en rouge, zones ignorées teintées de bleu ; largeur bornée à 1280 px, `diff_image.scale` donne l'échelle) et `diff_bbox` {x, y, width, height} délimite précisément la zone qui diverge. `ignore_regions` [{x, y, width, height}] exclut des zones mouvantes (horloge, curseur) de la comparaison et du dénominateur de `diff_ratio` ; leurs coordonnées sont celles de l'image comparée (relatives à la région si elle est fournie). `update_baseline: true` remplace la baseline par la capture courante sans comparer (`{ updated: true }`) : à n'utiliser qu'après validation du changement (refusé si le fichier existant n'est pas un PNG lisible). Premier outil d'assertion déterministe disponible en kwin-eis (sans DOM/JS).",
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
        let path = confine_baseline(&self.baseline_root, &p.baseline_path)
            .map_err(|reason| McpError::invalid_params(reason, None))?;
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
        let path = path.as_path();
        let exists = tokio::fs::try_exists(path).await.map_err(internal)?;
        let update = p.update_baseline.unwrap_or(false);
        if exists && update {
            // Remplacer n'est permis que sur une vraie baseline : un fichier
            // qui n'est pas un PNG lisible n'est pas à nous.
            let old = tokio::fs::read(path).await.map_err(internal)?;
            if image::load_from_memory_with_format(&old, image::ImageFormat::Png).is_err() {
                return Err(McpError::invalid_params(
                    "update_baseline refusé : le fichier existant n'est pas un PNG lisible",
                    None,
                ));
            }
        }
        if !exists || update {
            // Baseline absente, ou remplacement explicite : on l'écrit a
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
        // Un masque qui couvre toute l'image ne compare rien : le ratio vaut
        // 0 par construction, ce qui ne prouve rien — refus explicite plutôt
        // qu'un succès vide.
        let fully_masked = outcome.same_dims && outcome.compared_pixels == 0;
        let ok = outcome.same_dims && !fully_masked && outcome.diff_ratio <= threshold;
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
        if fully_masked {
            payload["reason"] = "fully_masked".into();
            payload["message"] = "ignore_regions couvre toute l'image : aucun pixel comparé, assertion non concluante".into();
        }
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
        // Échec a dimensions egales : l'image de difference montre OU ca
        // diverge, sans que l'agent ait a ouvrir deux captures. Pas d'image si
        // les dimensions différent (rien de pixel a pixel a dessiner).
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
        blocks.push(ContentBlock::text(to_json_text(&payload)));
        if let Some(enc) = &diff_image {
            blocks.push(enc.content());
        }
        Ok(CallToolResult::error(blocks))
    }
}

/// Racine de confinement des baselines, ce qui borne `assert_visual` (ADR-0049) :
/// sans elle, un client MCP (ou un agent victime d'une injection de prompt)
/// écraserait ou lirait n'importe quel fichier du compte.
pub(crate) fn default_baseline_root() -> std::path::PathBuf {
    std::env::var_os("DEVEYE_BASELINE_DIR")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// Résout `requested` dans `root` ou refuse : extension `.png`, aucun `..`,
/// chemin final (symlinks résolus sur la partie existante) sous la racine, et
/// pas de lien symbolique ni de dossier à l'emplacement de la baseline.
pub(crate) fn confine_baseline(
    root: &std::path::Path,
    requested: &str,
) -> Result<std::path::PathBuf, String> {
    use std::path::{Component, Path, PathBuf};
    if requested.is_empty() {
        return Err("baseline_path vide".to_string());
    }
    let requested_path = Path::new(requested);
    let is_png = requested_path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if !is_png {
        return Err("baseline_path doit avoir l'extension .png".to_string());
    }
    if requested_path
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err("baseline_path ne peut pas contenir `..`".to_string());
    }
    let root_c = std::fs::canonicalize(root).map_err(|e| {
        format!(
            "racine des baselines {} inutilisable ({e}) ; la fixer par DEVEYE_BASELINE_DIR",
            root.display()
        )
    })?;
    let joined = if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        root_c.join(requested_path)
    };
    // Partie existante canonicalisée (suit les symlinks), reste ajouté tel quel.
    let mut existing = joined.as_path();
    let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
    while std::fs::symlink_metadata(existing).is_err() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent;
            }
            _ => return Err("baseline_path invalide".to_string()),
        }
    }
    let mut resolved: PathBuf =
        std::fs::canonicalize(existing).map_err(|e| format!("baseline_path illisible ({e})"))?;
    for name in rest.iter().rev() {
        resolved.push(name);
    }
    if !resolved.starts_with(&root_c) {
        return Err(format!(
            "baseline_path hors de la racine des baselines ({}) ; la fixer par DEVEYE_BASELINE_DIR",
            root_c.display()
        ));
    }
    if let Ok(meta) = std::fs::symlink_metadata(&joined) {
        if !meta.is_file() {
            return Err(
                "baseline_path doit désigner un fichier ordinaire (ni lien ni dossier)".to_string(),
            );
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod confine_tests {
    use super::confine_baseline;

    fn root() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("deveye-confine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn relative_png_inside_root_is_accepted_even_in_new_subdirs() {
        let root = root();
        let path = confine_baseline(&root, "golden/home.png").unwrap();
        assert!(path.starts_with(std::fs::canonicalize(&root).unwrap()));
        assert!(path.ends_with("golden/home.png"));
    }

    #[test]
    fn dotdot_absolute_outside_and_non_png_are_refused() {
        let root = root();
        assert!(confine_baseline(&root, "../evil.png").is_err());
        assert!(confine_baseline(&root, "a/../../evil.png").is_err());
        assert!(confine_baseline(&root, "/etc/passwd.png").is_err());
        assert!(confine_baseline(&root, "/home/user/.bashrc").is_err());
        assert!(confine_baseline(&root, "notes.txt").is_err());
        assert!(confine_baseline(&root, "").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_escaping_the_root_are_refused() {
        let root = root();
        let outside = std::env::temp_dir().join(format!("deveye-outside-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        let _ = std::fs::remove_file(root.join("link.png"));
        let _ = std::fs::remove_file(root.join("dirlink"));
        std::os::unix::fs::symlink(outside.join("target.png"), root.join("link.png")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("dirlink")).unwrap();
        assert!(confine_baseline(&root, "link.png").is_err());
        assert!(confine_baseline(&root, "dirlink/x.png").is_err());
    }
}
