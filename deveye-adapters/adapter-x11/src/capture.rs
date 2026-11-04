//! Capture : `GetImage` de la **racine** rognee au rectangle de la fenetre.
//!
//! On ne lit pas le drawable de la fenetre : un enfant GL (llvmpipe, EGL)
//! peut y rendre du noir, alors que la racine contient la composition finale.
//! Sans gestionnaire de fenetres ni compositeur, c'est exactement ce que voit
//! un utilisateur.
//!
//! Les conversions de pixels et le rognage sont purs (testes sans serveur) :
//! [`crop_to_screen`], [`convert_to_rgba`], [`encode_png`].

use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat};

use deveye_core::{Screenshot, Size, WindowRect};

use crate::conn::{PixelFormat, Session};
use crate::error::X11Error;

/// Rectangle entier en coordonnees ecran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropRect {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

/// Intersecte `rect` (coordonnees racine, peut deborder ou etre negatif) avec
/// l'ecran `screen`. `None` si l'intersection est vide.
pub fn crop_to_screen(rect: &WindowRect, screen: (u16, u16)) -> Option<CropRect> {
    let (sw, sh) = (i64::from(screen.0), i64::from(screen.1));
    let x0 = i64::from(rect.x).max(0);
    let y0 = i64::from(rect.y).max(0);
    let x1 = (i64::from(rect.x) + i64::from(rect.width)).min(sw);
    let y1 = (i64::from(rect.y) + i64::from(rect.height)).min(sh);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(CropRect {
        x: x0 as i16,
        y: y0 as i16,
        width: (x1 - x0) as u16,
        height: (y1 - y0) as u16,
    })
}

/// Plafond (en pixels) de l'image renvoyee quand la fenetre deborde de l'ecran
/// et doit etre completee par du noir : evite d'allouer un tampon demesure pour
/// une fenetre pathologique (les tailles X11 vont jusqu'a 65535 x 65535).
const MAX_PADDED_PIXELS: u64 = 64 * 1024 * 1024;

/// Replace `rgba` (l'image de `crop`, deja rognee a l'ecran) dans un canevas de
/// la taille de la fenetre `rect`, partie hors ecran en noir opaque. Ainsi le
/// pixel `(x, y)` de l'image reste le point `(x, y)` de la fenetre, celui que
/// visent `click`, `drag_drop`, `scroll` et les rectangles de `ui.widgets`.
pub fn pad_to_window(rgba: &[u8], crop: &CropRect, rect: &WindowRect) -> Vec<u8> {
    let (ww, wh) = (rect.width as usize, rect.height as usize);
    let (cw, ch) = (usize::from(crop.width), usize::from(crop.height));
    // Decalage du rognage dans la fenetre (>= 0 par construction).
    let dx = (i64::from(crop.x) - i64::from(rect.x)).max(0) as usize;
    let dy = (i64::from(crop.y) - i64::from(rect.y)).max(0) as usize;
    let mut out = [0u8, 0, 0, 255].repeat(ww * wh);
    for row in 0..ch {
        let src = &rgba[row * cw * 4..(row + 1) * cw * 4];
        let start = ((dy + row) * ww + dx) * 4;
        out[start..start + cw * 4].copy_from_slice(src);
    }
    out
}

/// Etend un canal de `bits` bits vers 8 bits (replication des bits de poids fort).
fn to_u8(value: u32, bits: u32) -> u8 {
    match bits {
        0 => 0,
        8 => value as u8,
        b if b > 8 => (value >> (b - 8)) as u8,
        b => {
            let max = (1u32 << b) - 1;
            ((value * 255 + max / 2) / max) as u8
        }
    }
}

fn channel(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones();
    to_u8((pixel & mask) >> shift, bits)
}

/// Convertit les octets bruts d'un `GetImage` ZPixmap en RGBA 8 bits (alpha
/// toujours opaque : la racine n'a pas de transparence).
///
/// Supporte `bits_per_pixel == 32` avec profondeur 24 ou 32 ; toute autre
/// combinaison est une erreur explicite (jamais une image corrompue).
pub fn convert_to_rgba(
    data: &[u8],
    width: u32,
    height: u32,
    fmt: &PixelFormat,
) -> Result<Vec<u8>, X11Error> {
    if !matches!(fmt.depth, 24 | 32) || fmt.bits_per_pixel != 32 {
        return Err(X11Error::Request(format!(
            "format de pixels non supporté (profondeur {}, {} bits/pixel) : seuls 24/32 \
             sur 32 bits/pixel sont gérés",
            fmt.depth, fmt.bits_per_pixel
        )));
    }
    let stride = (width as usize) * 4;
    let need = stride * height as usize;
    if data.len() < need {
        return Err(X11Error::Request(format!(
            "image tronquée : {} octets reçus, {need} attendus",
            data.len()
        )));
    }
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height as usize {
        let line = &data[row * stride..row * stride + stride];
        let (pixels, _) = line.as_chunks::<4>();
        for &bytes in pixels {
            let value = if fmt.msb_first {
                u32::from_be_bytes(bytes)
            } else {
                u32::from_le_bytes(bytes)
            };
            out.extend_from_slice(&[
                channel(value, fmt.red_mask),
                channel(value, fmt.green_mask),
                channel(value, fmt.blue_mask),
                255,
            ]);
        }
    }
    Ok(out)
}

/// Encode du RGBA 8 bits en PNG.
pub fn encode_png(rgba: Vec<u8>, width: u32, height: u32) -> Result<Vec<u8>, X11Error> {
    let img = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| X11Error::Request("tampon RGBA de taille incohérente".into()))?;
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| X11Error::Request(format!("encodage PNG : {e}")))?;
    Ok(buf)
}

/// Capture `rect` (coordonnees racine) et renvoie un PNG **de la taille de la
/// fenetre** : si elle deborde de l'ecran, la partie invisible est noire, de
/// sorte que les coordonnees de l'image restent celles des gestes (relatives a
/// l'origine de la fenetre). Une fenetre entierement hors ecran est une erreur.
pub(crate) fn capture_rect(session: &Session, rect: &WindowRect) -> Result<Screenshot, X11Error> {
    let crop = crop_to_screen(rect, session.screen).ok_or_else(|| {
        X11Error::NoWindow(format!(
            "la fenêtre ({}, {}, {}x{}) est entièrement hors de l'écran {}x{}",
            rect.x, rect.y, rect.width, rect.height, session.screen.0, session.screen.1
        ))
    })?;
    if u64::from(rect.width) * u64::from(rect.height) > MAX_PADDED_PIXELS {
        return Err(X11Error::Request(format!(
            "fenêtre {}x{} trop grande pour être capturée (plafond {MAX_PADDED_PIXELS} pixels)",
            rect.width, rect.height
        )));
    }
    let reply = session
        .conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            session.root,
            crop.x,
            crop.y,
            crop.width,
            crop.height,
            u32::MAX,
        )?
        .reply()?;
    let (w, h) = (u32::from(crop.width), u32::from(crop.height));
    let mut rgba = convert_to_rgba(&reply.data, w, h, &session.pixel)?;
    let (out_w, out_h) = (rect.width, rect.height);
    if (w, h) != (out_w, out_h) {
        rgba = pad_to_window(&rgba, &crop, rect);
    }
    let png = encode_png(rgba, out_w, out_h)?;
    Ok(Screenshot::new_png(png, Size::new(out_w, out_h)))
}

/// Capture de l'ecran entier.
pub(crate) fn capture_screen(session: &Session) -> Result<Screenshot, X11Error> {
    let rect = WindowRect::new(
        0,
        0,
        u32::from(session.screen.0),
        u32::from(session.screen.1),
    );
    capture_rect(session, &rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Visuel TrueColor classique : R=0xff0000, G=0x00ff00, B=0x0000ff, LSB.
    fn bgrx(depth: u8) -> PixelFormat {
        PixelFormat {
            depth,
            bits_per_pixel: 32,
            scanline_pad: 32,
            msb_first: false,
            red_mask: 0x00ff_0000,
            green_mask: 0x0000_ff00,
            blue_mask: 0x0000_00ff,
        }
    }

    #[test]
    fn bgra_to_rgba_depth_24() {
        // Pixel rouge pur : en memoire LSB = B,G,R,X = 00 00 ff 00.
        let data = [0x00, 0x00, 0xff, 0x00, 0x10, 0x20, 0x30, 0x00];
        let out = convert_to_rgba(&data, 2, 1, &bgrx(24)).unwrap();
        assert_eq!(out, vec![0xff, 0x00, 0x00, 255, 0x30, 0x20, 0x10, 255]);
    }

    #[test]
    fn depth_32_ignores_the_alpha_byte_and_stays_opaque() {
        let data = [0x01, 0x02, 0x03, 0x00];
        let out = convert_to_rgba(&data, 1, 1, &bgrx(32)).unwrap();
        assert_eq!(out, vec![0x03, 0x02, 0x01, 255]);
    }

    #[test]
    fn msb_first_order_is_honoured() {
        let fmt = PixelFormat {
            msb_first: true,
            ..bgrx(24)
        };
        // MSB : octets = X,R,G,B.
        let data = [0x00, 0xaa, 0xbb, 0xcc];
        let out = convert_to_rgba(&data, 1, 1, &fmt).unwrap();
        assert_eq!(out, vec![0xaa, 0xbb, 0xcc, 255]);
    }

    #[test]
    fn rgb_masks_in_other_order() {
        let fmt = PixelFormat {
            red_mask: 0x0000_00ff,
            green_mask: 0x0000_ff00,
            blue_mask: 0x00ff_0000,
            ..bgrx(24)
        };
        let data = [0x11, 0x22, 0x33, 0x00]; // LE : valeur 0x00332211
        let out = convert_to_rgba(&data, 1, 1, &fmt).unwrap();
        assert_eq!(out, vec![0x11, 0x22, 0x33, 255]);
    }

    #[test]
    fn narrow_channels_are_scaled_to_8_bits() {
        assert_eq!(to_u8(0b11111, 5), 255);
        assert_eq!(to_u8(0, 5), 0);
        assert_eq!(to_u8(0b111111, 6), 255);
        assert_eq!(to_u8(0x3ff, 10), 255);
        assert_eq!(to_u8(0xff, 8), 255);
        assert_eq!(channel(0x00ff_0000, 0), 0);
    }

    #[test]
    fn rejects_unsupported_formats_and_truncated_data() {
        let bad_depth = PixelFormat {
            depth: 16,
            ..bgrx(24)
        };
        assert!(convert_to_rgba(&[0; 8], 1, 1, &bad_depth).is_err());
        let bad_bpp = PixelFormat {
            bits_per_pixel: 16,
            ..bgrx(24)
        };
        assert!(convert_to_rgba(&[0; 8], 1, 1, &bad_bpp).is_err());
        assert!(convert_to_rgba(&[0; 3], 1, 1, &bgrx(24)).is_err());
    }

    #[test]
    fn multi_row_images_keep_row_order() {
        let data = [
            0, 0, 0xff, 0, // ligne 0 : rouge
            0xff, 0, 0, 0, // ligne 1 : bleu
        ];
        let out = convert_to_rgba(&data, 1, 2, &bgrx(24)).unwrap();
        assert_eq!(&out[..4], &[0xff, 0, 0, 255]);
        assert_eq!(&out[4..], &[0, 0, 0xff, 255]);
    }

    #[test]
    fn crop_inside_screen_is_identity() {
        let c = crop_to_screen(&WindowRect::new(10, 20, 300, 200), (1400, 1000)).unwrap();
        assert_eq!(
            c,
            CropRect {
                x: 10,
                y: 20,
                width: 300,
                height: 200
            }
        );
    }

    #[test]
    fn crop_clips_negative_origin_and_overflow() {
        let c = crop_to_screen(&WindowRect::new(-50, -10, 200, 100), (1400, 1000)).unwrap();
        assert_eq!((c.x, c.y, c.width, c.height), (0, 0, 150, 90));
        let c = crop_to_screen(&WindowRect::new(1300, 950, 500, 500), (1400, 1000)).unwrap();
        assert_eq!((c.x, c.y, c.width, c.height), (1300, 950, 100, 50));
    }

    #[test]
    fn padding_keeps_window_coordinates() {
        // Fenetre 4x3 en (-2, -1) sur un ecran assez grand : seule la partie
        // x >= 2, y >= 1 est visible (2x2), le reste doit etre noir.
        let rect = WindowRect::new(-2, -1, 4, 3);
        let crop = crop_to_screen(&rect, (100, 100)).unwrap();
        assert_eq!((crop.x, crop.y, crop.width, crop.height), (0, 0, 2, 2));
        let rgba: Vec<u8> = (0..4).flat_map(|i| [10 * (i + 1), 0, 0, 255]).collect();
        let out = pad_to_window(&rgba, &crop, &rect);
        assert_eq!(out.len(), 4 * 3 * 4);
        let px = |x: usize, y: usize| &out[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4];
        assert_eq!(px(0, 0), &[0, 0, 0, 255]);
        assert_eq!(px(1, 1), &[0, 0, 0, 255]);
        // Le 1er pixel visible est la colonne 2, ligne 1 de la fenetre.
        assert_eq!(px(2, 1), &[10, 0, 0, 255]);
        assert_eq!(px(3, 1), &[20, 0, 0, 255]);
        assert_eq!(px(2, 2), &[30, 0, 0, 255]);
        assert_eq!(px(3, 2), &[40, 0, 0, 255]);
    }

    #[test]
    fn padding_handles_overflow_on_the_far_side() {
        let rect = WindowRect::new(8, 8, 4, 4);
        let crop = crop_to_screen(&rect, (10, 10)).unwrap();
        assert_eq!((crop.width, crop.height), (2, 2));
        let rgba = [200u8, 0, 0, 255].repeat(4);
        let out = pad_to_window(&rgba, &crop, &rect);
        assert_eq!(&out[..4], &[200, 0, 0, 255]);
        assert_eq!(&out[(3 * 4 + 3) * 4..(3 * 4 + 3) * 4 + 4], &[0, 0, 0, 255]);
    }

    #[test]
    fn crop_fully_offscreen_is_none() {
        assert!(crop_to_screen(&WindowRect::new(1400, 0, 10, 10), (1400, 1000)).is_none());
        assert!(crop_to_screen(&WindowRect::new(-100, 0, 100, 10), (1400, 1000)).is_none());
        assert!(crop_to_screen(&WindowRect::new(0, 0, 0, 10), (1400, 1000)).is_none());
        assert!(crop_to_screen(&WindowRect::new(i32::MIN, i32::MIN, 5, 5), (10, 10)).is_none());
        assert!(crop_to_screen(
            &WindowRect::new(i32::MAX, i32::MAX, u32::MAX, u32::MAX),
            (10, 10)
        )
        .is_none());
    }

    #[test]
    fn png_roundtrip_preserves_pixels() {
        let rgba = vec![255, 0, 0, 255, 0, 255, 0, 255];
        let png = encode_png(rgba.clone(), 2, 1).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(img.as_raw(), &rgba);
    }

    #[test]
    fn png_rejects_inconsistent_buffer() {
        assert!(encode_png(vec![0; 3], 2, 1).is_err());
    }
}
