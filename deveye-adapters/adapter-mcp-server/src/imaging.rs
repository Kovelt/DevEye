//! Traitement d'image des outils de capture : redimensionnement proportionnel,
//! encodage PNG ou JPEG et facteur d'échelle restitué à l'agent (ADR-0029), puis
//! comparaison visuelle avec masques et image de différence (ADR-0030).
//!
//! Tout est synchrone et sans port : les outils MCP capturent, ce module
//! transforme des octets.

use std::io::Cursor;

use base64::Engine;
use image::{imageops::FilterType, ExtendedColorType, ImageFormat, Rgba, RgbaImage};
use rmcp::{model::ContentBlock, ErrorData as McpError};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{png_dimensions, B64};

/// Qualité JPEG appliquée quand `format = "jpeg"` sans `quality`.
const JPEG_DEFAULT_QUALITY: u32 = 80;

/// Plafond de `max_width` : au-delà, aucune capture réaliste n'est concernée et
/// le redimensionnement n'aurait d'effet que sur une valeur aberrante.
const MAX_WIDTH_CAP: u32 = 16_384;

/// Largeur maximale de l'image de différence d'`assert_visual` : elle sert à
/// *voir* où ça diverge, pas à mesurer ; le `diff_bbox` du payload porte la
/// précision.
pub(crate) const DIFF_IMAGE_MAX_WIDTH: u32 = 1_280;

/// Options de coût d'une capture, communes à `screenshot`, `screenshot_region`,
/// `screenshot_element` et `observe`. Sans option, la sortie est le PNG d'origine,
/// octet pour octet.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct CaptureOptions {
    /// Largeur maximale en pixels de l'image renvoyée. Si l'image (après
    /// recadrage éventuel) est plus large, elle est réduite proportionnellement ;
    /// jamais agrandie. La réponse donne `scale` (image / source) : une
    /// coordonnée de l'image se convertit en coordonnée de la capture source par
    /// `origin + coordonnée / scale`. Absent : taille d'origine.
    #[serde(default)]
    max_width: Option<u32>,
    /// Format de l'image renvoyée : `"png"` (défaut, sans perte) ou `"jpeg"`
    /// (nettement plus léger sur une UI riche, au prix d'artefacts autour du
    /// texte fin).
    #[serde(default)]
    format: Option<String>,
    /// Qualité JPEG de 1 à 100 (défaut 80). Ignorée en PNG.
    #[serde(default)]
    quality: Option<u32>,
}

/// Format de sortie résolu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Png,
    Jpeg { quality: u8 },
}

/// Options validées, prêtes à être appliquées.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedCapture {
    pub(crate) max_width: Option<u32>,
    pub(crate) format: OutputFormat,
}

impl CaptureOptions {
    /// Valide les options. Une valeur invalide est une faute d'appel
    /// (`invalid_params`) détectée **avant** la capture, pour ne pas gaspiller
    /// une prise d'écran.
    pub(crate) fn resolve(&self) -> Result<ResolvedCapture, McpError> {
        self.resolve_with_default_width(None)
    }

    /// Comme [`Self::resolve`], avec une largeur maximale par défaut quand
    /// `max_width` est absent (outil composite `observe`).
    pub(crate) fn resolve_with_default_width(
        &self,
        default_max_width: Option<u32>,
    ) -> Result<ResolvedCapture, McpError> {
        let max_width = match self.max_width {
            Some(0) => {
                return Err(McpError::invalid_params(
                    "max_width doit être supérieur à 0".to_string(),
                    None,
                ))
            }
            Some(w) => Some(w.min(MAX_WIDTH_CAP)),
            None => default_max_width,
        };
        let format = match self.format.as_deref().map(str::to_ascii_lowercase) {
            None => OutputFormat::Png,
            Some(f) if f == "png" => OutputFormat::Png,
            Some(f) if f == "jpeg" || f == "jpg" => {
                let quality = self.quality.unwrap_or(JPEG_DEFAULT_QUALITY);
                if !(1..=100).contains(&quality) {
                    return Err(McpError::invalid_params(
                        format!("quality doit être comprise entre 1 et 100 (reçu {quality})"),
                        None,
                    ));
                }
                OutputFormat::Jpeg {
                    quality: quality as u8,
                }
            }
            Some(autre) => {
                return Err(McpError::invalid_params(
                    format!("format inconnu : {autre:?} (attendu \"png\" ou \"jpeg\")"),
                    None,
                ))
            }
        };
        Ok(ResolvedCapture { max_width, format })
    }
}

/// Image encodée prête à être renvoyée, avec de quoi convertir ses coordonnées.
pub(crate) struct EncodedImage {
    pub(crate) bytes: Vec<u8>,
    pub(crate) mime: &'static str,
    pub(crate) format: &'static str,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Dimensions de la source (capture, ou région recadrée) avant réduction.
    pub(crate) source_width: u32,
    pub(crate) source_height: u32,
}

impl EncodedImage {
    /// Facteur de réduction horizontal (image / source), 1.0 sans réduction.
    pub(crate) fn scale(&self) -> f64 {
        if self.source_width == 0 {
            1.0
        } else {
            f64::from(self.width) / f64::from(self.source_width)
        }
    }

    /// Bloc image MCP.
    pub(crate) fn content(&self) -> ContentBlock {
        ContentBlock::image(B64.encode(&self.bytes), self.mime)
    }

    /// Métadonnées de l'image. `origin` : position de la source dans la capture
    /// complète (région ou élément), absente pour une capture entière.
    pub(crate) fn meta(&self, origin: Option<(u32, u32)>) -> serde_json::Value {
        let mut meta = serde_json::json!({
            "format": self.format,
            "width": self.width,
            "height": self.height,
            "source_width": self.source_width,
            "source_height": self.source_height,
            "scale": self.scale(),
            "bytes": self.bytes.len(),
        });
        if let Some((x, y)) = origin {
            meta["origin"] = serde_json::json!({ "x": x, "y": y });
        }
        meta
    }
}

/// Applique les options à un PNG. Sans réduction ni conversion, les octets
/// d'origine sont renvoyés tels quels (seul l'en-tête est lu).
pub(crate) fn encode_capture(png: &[u8], opts: &ResolvedCapture) -> Result<EncodedImage, McpError> {
    let (source_width, source_height) = png_dimensions(png).map_err(super::internal)?;
    let target_width = opts
        .max_width
        .filter(|&max| max < source_width)
        .unwrap_or(source_width);
    if target_width == source_width && opts.format == OutputFormat::Png {
        return Ok(EncodedImage {
            bytes: png.to_vec(),
            mime: "image/png",
            format: "png",
            width: source_width,
            height: source_height,
            source_width,
            source_height,
        });
    }
    let decoded = image::load_from_memory(png)
        .map_err(|e| super::internal(format!("decode PNG : {e}")))?
        .to_rgba8();
    let resized = downscale(decoded, target_width);
    let (width, height) = resized.dimensions();
    let (bytes, mime, format) = encode_rgba(&resized, opts.format)?;
    Ok(EncodedImage {
        bytes,
        mime,
        format,
        width,
        height,
        source_width,
        source_height,
    })
}

/// Réduit `img` à `target_width` de large en conservant les proportions ; sans
/// effet si `target_width` ne réduit pas l'image.
pub(crate) fn downscale(img: RgbaImage, target_width: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    if target_width == 0 || target_width >= w {
        return img;
    }
    let target_height =
        ((u64::from(h) * u64::from(target_width) + u64::from(w) / 2) / u64::from(w)).max(1) as u32;
    image::imageops::resize(&img, target_width, target_height, FilterType::Lanczos3)
}

/// Encode une image RGBA en PNG ou JPEG (l'alpha est abandonné en JPEG).
pub(crate) fn encode_rgba(
    img: &RgbaImage,
    format: OutputFormat,
) -> Result<(Vec<u8>, &'static str, &'static str), McpError> {
    let mut out = Vec::new();
    match format {
        OutputFormat::Png => {
            img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
                .map_err(|e| super::internal(format!("encode PNG : {e}")))?;
            Ok((out, "image/png", "png"))
        }
        OutputFormat::Jpeg { quality } => {
            let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
                .encode(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    ExtendedColorType::Rgb8,
                )
                .map_err(|e| super::internal(format!("encode JPEG : {e}")))?;
            Ok((out, "image/jpeg", "jpeg"))
        }
    }
}

// ── Comparaison visuelle (assert_visual) ─────────────────────────────────────

/// Rectangle à ignorer lors d'une comparaison (horloge, curseur, animation…).
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Eq)]
pub(crate) struct Region {
    /// Abscisse du coin haut-gauche, en pixels de l'image comparée.
    pub(crate) x: u32,
    /// Ordonnée du coin haut-gauche.
    pub(crate) y: u32,
    /// Largeur (> 0).
    pub(crate) width: u32,
    /// Hauteur (> 0).
    pub(crate) height: u32,
}

impl Region {
    fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x
            && py >= self.y
            && u64::from(px) < u64::from(self.x) + u64::from(self.width)
            && u64::from(py) < u64::from(self.y) + u64::from(self.height)
    }
}

/// Refuse les masques vides : une région nulle est une faute d'appel, pas un
/// masque qui ne masque rien.
pub(crate) fn validate_masks(masks: &[Region]) -> Result<(), McpError> {
    match masks.iter().position(|m| m.width == 0 || m.height == 0) {
        Some(i) => Err(McpError::invalid_params(
            format!("ignore_regions[{i}] : width et height doivent être supérieurs à 0"),
            None,
        )),
        None => Ok(()),
    }
}

/// Rectangle englobant des pixels différents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoundingBox {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// Résultat d'une comparaison visuelle.
pub(crate) struct VisualOutcome {
    /// Vrai si baseline et capture ont les mêmes dimensions.
    pub(crate) same_dims: bool,
    /// Ratio de pixels différents parmi les pixels **comparés** (hors masques),
    /// 0.0–1.0 ; `1.0` si les dimensions diffèrent.
    pub(crate) diff_ratio: f64,
    pub(crate) diff_pixels: u64,
    /// Pixels réellement comparés (total moins les masques).
    pub(crate) compared_pixels: u64,
    pub(crate) diff_bbox: Option<BoundingBox>,
    pub(crate) baseline_dims: (u32, u32),
    pub(crate) current_dims: (u32, u32),
    /// Images décodées, conservées pour dessiner la différence sans les
    /// redécoder (présentes seulement quand les dimensions concordent).
    pub(crate) images: Option<(RgbaImage, RgbaImage)>,
}

/// Compare deux PNG pixel à pixel (RGBA), en ignorant les pixels couverts par
/// `masks` (exclus du dénominateur : un masque couvrant la moitié de l'image ne
/// dilue pas la différence restante). Des dimensions différentes => échec
/// (`same_dims = false`, `diff_ratio = 1.0`) sans itérer.
pub(crate) fn visual_diff(
    baseline_png: &[u8],
    current_png: &[u8],
    masks: &[Region],
) -> Result<VisualOutcome, String> {
    let a = image::load_from_memory(baseline_png)
        .map_err(|e| format!("decode baseline : {e}"))?
        .to_rgba8();
    let b = image::load_from_memory(current_png)
        .map_err(|e| format!("decode capture : {e}"))?
        .to_rgba8();
    let baseline_dims = a.dimensions();
    let current_dims = b.dimensions();
    if baseline_dims != current_dims {
        return Ok(VisualOutcome {
            same_dims: false,
            diff_ratio: 1.0,
            diff_pixels: 0,
            compared_pixels: 0,
            diff_bbox: None,
            baseline_dims,
            current_dims,
            images: None,
        });
    }
    let mut compared: u64 = 0;
    let mut diff: u64 = 0;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (x, y, pa) in a.enumerate_pixels() {
        if masks.iter().any(|m| m.contains(x, y)) {
            continue;
        }
        compared += 1;
        if pa != b.get_pixel(x, y) {
            diff += 1;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    let diff_ratio = if compared == 0 {
        0.0
    } else {
        diff as f64 / compared as f64
    };
    let diff_bbox = (diff > 0).then(|| BoundingBox {
        x: min_x,
        y: min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    });
    Ok(VisualOutcome {
        same_dims: true,
        diff_ratio,
        diff_pixels: diff,
        compared_pixels: compared,
        diff_bbox,
        baseline_dims,
        current_dims,
        images: Some((a, b)),
    })
}

/// Dessine l'image de différence : la capture courante éclaircie en fond, les
/// pixels différents en rouge vif, les zones masquées teintées de bleu. Réduite
/// à [`DIFF_IMAGE_MAX_WIDTH`] pour rester bon marché en tokens.
pub(crate) fn render_diff_png(
    baseline: &RgbaImage,
    current: &RgbaImage,
    masks: &[Region],
) -> Result<EncodedImage, McpError> {
    let (w, h) = current.dimensions();
    let mut canvas = RgbaImage::new(w, h);
    for (x, y, px) in canvas.enumerate_pixels_mut() {
        let cur = current.get_pixel(x, y);
        *px = if masks.iter().any(|m| m.contains(x, y)) {
            let [r, g, b, _] = lighten(cur);
            Rgba([r / 2, g / 2, b.saturating_add(90).max(160), 255])
        } else if baseline.get_pixel(x, y) != cur {
            Rgba([255, 0, 0, 255])
        } else {
            let [r, g, b, _] = lighten(cur);
            Rgba([r, g, b, 255])
        };
    }
    let resized = downscale(canvas, DIFF_IMAGE_MAX_WIDTH);
    let (width, height) = resized.dimensions();
    let (bytes, mime, format) = encode_rgba(&resized, OutputFormat::Png)?;
    Ok(EncodedImage {
        bytes,
        mime,
        format,
        width,
        height,
        source_width: w,
        source_height: h,
    })
}

/// Rapproche un pixel du blanc (60 %) pour que le rouge de la différence ressorte.
fn lighten(px: &Rgba<u8>) -> [u8; 4] {
    let mix = |c: u8| (u16::from(c) * 2 / 5 + 153).min(255) as u8;
    [mix(px[0]), mix(px[1]), mix(px[2]), 255]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, Rgba(rgba));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .expect("encodage PNG de test");
        out
    }

    fn opts(max_width: Option<u32>, format: Option<&str>, quality: Option<u32>) -> CaptureOptions {
        CaptureOptions {
            max_width,
            format: format.map(str::to_string),
            quality,
        }
    }

    #[test]
    fn default_options_keep_the_original_png_bytes() {
        let png = solid_png(40, 20, [10, 20, 30, 255]);
        let resolved = CaptureOptions::default()
            .resolve()
            .expect("options valides");
        let enc = encode_capture(&png, &resolved).expect("encodage");
        assert_eq!(enc.bytes, png, "aucun ré-encodage sans option");
        assert_eq!((enc.width, enc.height), (40, 20));
        assert_eq!(enc.scale(), 1.0);
        assert_eq!(enc.mime, "image/png");
    }

    #[test]
    fn max_width_scales_down_proportionally_and_reports_the_factor() {
        let png = solid_png(200, 100, [10, 20, 30, 255]);
        let resolved = opts(Some(50), None, None)
            .resolve()
            .expect("options valides");
        let enc = encode_capture(&png, &resolved).expect("encodage");
        assert_eq!((enc.width, enc.height), (50, 25));
        assert_eq!((enc.source_width, enc.source_height), (200, 100));
        assert!((enc.scale() - 0.25).abs() < 1e-9);
        let meta = enc.meta(Some((8, 16)));
        assert_eq!(meta["scale"], 0.25);
        assert_eq!(meta["origin"], serde_json::json!({ "x": 8, "y": 16 }));
    }

    #[test]
    fn max_width_never_upscales() {
        let png = solid_png(30, 30, [1, 2, 3, 255]);
        let resolved = opts(Some(500), None, None)
            .resolve()
            .expect("options valides");
        let enc = encode_capture(&png, &resolved).expect("encodage");
        assert_eq!((enc.width, enc.height), (30, 30));
        assert_eq!(enc.bytes, png);
    }

    #[test]
    fn jpeg_output_is_a_decodable_jpeg_smaller_than_noisy_png() {
        let mut img = RgbaImage::new(128, 128);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let v = ((x * 31 + y * 17) % 251) as u8;
            *px = Rgba([v, v.wrapping_mul(3), v.wrapping_add(40), 255]);
        }
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .expect("encodage PNG de test");
        let resolved = opts(None, Some("jpeg"), Some(60))
            .resolve()
            .expect("options valides");
        let enc = encode_capture(&png, &resolved).expect("encodage");
        assert_eq!(enc.mime, "image/jpeg");
        assert_eq!(enc.format, "jpeg");
        assert_eq!(&enc.bytes[..2], &[0xFF, 0xD8], "magic JPEG");
        assert!(enc.bytes.len() < png.len(), "jpeg plus léger sur du bruit");
        let back = image::load_from_memory(&enc.bytes).expect("JPEG décodable");
        assert_eq!((back.width(), back.height()), (128, 128));
    }

    #[test]
    fn invalid_options_are_invalid_params() {
        let invalid = rmcp::model::ErrorCode::INVALID_PARAMS;
        for bad in [
            opts(Some(0), None, None),
            opts(None, Some("gif"), None),
            opts(None, Some("jpeg"), Some(0)),
            opts(None, Some("jpeg"), Some(101)),
        ] {
            let err = bad.resolve().expect_err("doit être refusé");
            assert_eq!(err.code, invalid);
        }
        // `quality` n'a aucun effet en PNG : pas une erreur.
        assert!(opts(None, Some("png"), Some(5)).resolve().is_ok());
    }

    #[test]
    fn default_width_applies_only_when_max_width_is_absent() {
        let r = opts(None, None, None)
            .resolve_with_default_width(Some(1024))
            .expect("options valides");
        assert_eq!(r.max_width, Some(1024));
        let r = opts(Some(300), None, None)
            .resolve_with_default_width(Some(1024))
            .expect("options valides");
        assert_eq!(r.max_width, Some(300));
    }

    #[test]
    fn masks_exclude_pixels_from_diff_and_denominator() {
        let base = solid_png(10, 10, [0, 0, 0, 255]);
        let mut cur_img = RgbaImage::from_pixel(10, 10, Rgba([0, 0, 0, 255]));
        cur_img.put_pixel(1, 1, Rgba([255, 255, 255, 255]));
        cur_img.put_pixel(8, 8, Rgba([255, 255, 255, 255]));
        let mut cur = Vec::new();
        cur_img
            .write_to(&mut Cursor::new(&mut cur), ImageFormat::Png)
            .expect("encodage PNG de test");

        let unmasked = visual_diff(&base, &cur, &[]).expect("comparaison");
        assert_eq!(unmasked.diff_pixels, 2);
        assert_eq!(unmasked.compared_pixels, 100);
        assert_eq!(
            unmasked.diff_bbox,
            Some(BoundingBox {
                x: 1,
                y: 1,
                width: 8,
                height: 8
            })
        );

        // Masque 5x5 en haut à gauche : couvre (1, 1), retire 25 pixels du total.
        let mask = Region {
            x: 0,
            y: 0,
            width: 5,
            height: 5,
        };
        let masked = visual_diff(&base, &cur, &[mask]).expect("comparaison");
        assert_eq!(masked.diff_pixels, 1);
        assert_eq!(masked.compared_pixels, 75);
        assert_eq!(
            masked.diff_bbox,
            Some(BoundingBox {
                x: 8,
                y: 8,
                width: 1,
                height: 1
            })
        );
    }

    #[test]
    fn fully_masked_image_compares_as_identical() {
        let base = solid_png(4, 4, [0, 0, 0, 255]);
        let cur = solid_png(4, 4, [255, 0, 0, 255]);
        let mask = Region {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        let out = visual_diff(&base, &cur, &[mask]).expect("comparaison");
        assert_eq!(out.diff_ratio, 0.0);
        assert_eq!(out.compared_pixels, 0);
        assert!(out.diff_bbox.is_none());
    }

    #[test]
    fn empty_mask_is_rejected() {
        let ok = Region {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        let bad = Region {
            x: 3,
            y: 3,
            width: 0,
            height: 2,
        };
        assert!(validate_masks(&[ok]).is_ok());
        let err = validate_masks(&[ok, bad]).expect_err("masque vide");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("ignore_regions[1]"), "{}", err.message);
    }

    #[test]
    fn diff_image_marks_changed_pixels_red_and_masks_blue() {
        let base = RgbaImage::from_pixel(6, 6, Rgba([0, 0, 0, 255]));
        let mut cur = base.clone();
        cur.put_pixel(4, 4, Rgba([255, 255, 255, 255]));
        let mask = Region {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        };
        let enc = render_diff_png(&base, &cur, &[mask]).expect("rendu");
        assert_eq!((enc.width, enc.height), (6, 6));
        let img = image::load_from_memory(&enc.bytes).expect("PNG").to_rgba8();
        assert_eq!(img.get_pixel(4, 4).0, [255, 0, 0, 255]);
        let masked = img.get_pixel(0, 0).0;
        assert!(
            masked[2] > masked[0],
            "zone masquée teintée de bleu : {masked:?}"
        );
        let same = img.get_pixel(3, 0).0;
        assert!(
            same[0] > 100 && same[0] == same[1],
            "fond éclairci : {same:?}"
        );
    }

    #[test]
    fn diff_image_is_bounded_in_width() {
        let big = RgbaImage::from_pixel(2_000, 10, Rgba([0, 0, 0, 255]));
        let enc = render_diff_png(&big, &big, &[]).expect("rendu");
        assert_eq!(enc.width, DIFF_IMAGE_MAX_WIDTH);
        assert_eq!(enc.source_width, 2_000);
    }
}
