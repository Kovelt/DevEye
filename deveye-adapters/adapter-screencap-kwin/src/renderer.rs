use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsFd;
use std::time::Duration;

use async_trait::async_trait;
use image::{ExtendedColorType, ImageEncoder};
use nix::fcntl::OFlag;
use nix::unistd::pipe2;
use zbus::zvariant::{Fd, OwnedValue, Value};
use zbus::Connection;

use deveye_core::{Screenshot, Size, WindowRect};
use deveye_ports::{PortError, PortResult, Renderer};

use crate::error::KwinScreencapError;
use crate::kwin_bridge::{run_kwin_script, BRIDGE_IFACE, BRIDGE_PATH};
use crate::proxy::ScreenShot2Proxy;
use crate::resolve_cache::{self, ResolvedWindow};

/// Délai d'attente de la lecture du pipe de capture ScreenShot2 : marge large
/// au-dessus du temps normal de rendu+écriture (quelques dizaines de ms) sans
/// tenir le gate MCP indéfiniment si KWin ne ferme jamais sa copie du fd.
const KWIN_PIPE_TIMEOUT: Duration = Duration::from_secs(5);

/// Capture une fenêtre KDE/Wayland via `org.kde.KWin.ScreenShot2`.
///
/// La fenêtre est désignée par sous-chaîne (insensible à la casse) de son
/// `caption`. La résolution (UUID + géométrie) passe par
/// [`crate::resolve_cache`] (TTL court, partagé avec [`crate::KwinWindowGeometry`])
/// pour éviter de rejouer un cycle `loadScript`/`start`/`unloadScript` complet
/// à chaque appel — tout en restant à jour aux ouvertures/fermetures de
/// fenêtres entre deux outils MCP (TTL expiré, ou purge immédiate sur erreur).
#[derive(Clone, Debug)]
pub struct KwinRenderer {
    title_substring: String,
}

impl KwinRenderer {
    pub fn new(title_substring: impl Into<String>) -> Self {
        Self {
            title_substring: title_substring.into(),
        }
    }

    /// Résout `(uuid, géométrie_écran_complète)` via une connexion **déjà
    /// ouverte par l'appelant** : cache-first (cf. [`crate::resolve_cache`]),
    /// sinon script KWin éphémère (cycle de vie porté par [`run_kwin_script`])
    /// sur `conn`. Utilisé par [`Renderer::screenshot`], qui a de toute façon
    /// besoin d'une connexion pour `CaptureWindow` — pas la peine d'en ouvrir
    /// une seconde sur un cache miss.
    async fn resolve_rect_with(
        &self,
        conn: &Connection,
    ) -> Result<(String, WindowRect), KwinScreencapError> {
        if let Some(cached) = resolve_cache::get(&self.title_substring).await {
            return Ok((cached.uuid, cached.rect));
        }

        let dest = conn
            .unique_name()
            .map(|n| n.to_string())
            .ok_or_else(|| KwinScreencapError::Dbus("connexion D-Bus sans nom unique".into()))?;

        let script = build_resolve_script(&self.title_substring, &dest);
        let (uuid, rect_str) = run_kwin_script(conn, "deveye-screencap", &script).await?;

        if uuid.is_empty() {
            resolve_cache::invalidate(&self.title_substring).await;
            return Err(KwinScreencapError::WindowNotFound(
                self.title_substring.clone(),
            ));
        }
        let rect = parse_resolved_rect(&rect_str).unwrap_or_else(|| WindowRect::new(0, 0, 0, 0));
        resolve_cache::put(
            &self.title_substring,
            ResolvedWindow {
                uuid: uuid.clone(),
                rect: rect.clone(),
            },
        )
        .await;
        Ok((uuid, rect))
    }

    /// Résout `(uuid, géométrie_écran_complète)` sans connexion préexistante :
    /// cache-first, et ouvre sa propre connexion D-Bus **seulement** sur cache
    /// miss (sur cache hit, [`Self::window_origin`]/`viewport_size` n'ouvrent
    /// donc aucune connexion D-Bus du tout). Utilisé par les appelants qui
    /// n'ont besoin de D-Bus que pour cette résolution.
    async fn resolve_rect_lazy(&self) -> Result<(String, WindowRect), KwinScreencapError> {
        if let Some(cached) = resolve_cache::get(&self.title_substring).await {
            return Ok((cached.uuid, cached.rect));
        }
        let conn = Connection::session()
            .await
            .map_err(|e| KwinScreencapError::Dbus(format!("D-Bus session : {e}")))?;
        self.resolve_rect_with(&conn).await
    }
}

#[async_trait]
impl Renderer for KwinRenderer {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        // Connexion fraîche par appel : nom unique distinct (donc destination
        // de callback distincte) et serveur d'objet propre, sans état partagé.
        let conn = Connection::session()
            .await
            .map_err(|e| PortError::Render(format!("D-Bus session : {e}")))?;

        let (uuid, _rect) = self
            .resolve_rect_with(&conn)
            .await
            .map_err(PortError::from)?;

        // Pipe anonyme O_CLOEXEC (pas de fuite aux sous-process).
        let (read_fd, write_fd) =
            pipe2(OFlag::O_CLOEXEC).map_err(|e| PortError::Render(format!("pipe2 : {e}")))?;

        // Lecteur lancé AVANT l'appel D-Bus : si l'image dépasse le buffer du
        // pipe (64 Ko), KWin bloque en écriture tant que personne ne lit —
        // lire en parallèle évite le deadlock.
        let read_file = std::fs::File::from(read_fd);
        let reader = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
            let mut file = read_file;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            Ok(buf)
        });

        let ss2 = ScreenShot2Proxy::new(&conn)
            .await
            .map_err(|e| PortError::Render(format!("proxy ScreenShot2 : {e}")))?;
        let options: HashMap<String, Value<'_>> = HashMap::new();
        let meta = match ss2
            .capture_window(&uuid, options, Fd::from(write_fd.as_fd()))
            .await
        {
            Ok(meta) => meta,
            Err(e) => {
                // L'UUID venait peut-etre du cache et ne designe plus une
                // fenetre vivante (fermee entre la resolution et la capture) :
                // purge pour forcer une resolution fraiche au prochain appel,
                // sans attendre l'expiration du TTL.
                resolve_cache::invalidate(&self.title_substring).await;
                return Err(PortError::Render(format!("CaptureWindow : {e}")));
            }
        };

        // Fermer NOTRE write_fd : KWin garde sa copie ; sans ça le lecteur ne
        // voit jamais EOF.
        drop(write_fd);

        // A4: timeout sur la lecture du pipe — si KWin ne ferme jamais sa copie
        // du fd (bug/gel côté compositeur), le thread spawn_blocking reste
        // bloqué dans read_to_end (fuite acceptée) mais le gate MCP est libéré.
        // Limite assumée : chaque timeout fuit UN thread du pool blocking tokio
        // (512 par défaut) — des retries en boucle contre un KWin durablement
        // figé finiraient par l'épuiser. L'appelant (l'agent) ne doit pas
        // marteler un screenshot qui répond « timeout » ; une lecture non
        // bloquante (AsyncFd + O_NONBLOCK) est la vraie sortie, prévue en P5.
        let raw = match tokio::time::timeout(KWIN_PIPE_TIMEOUT, reader).await {
            Ok(joined) => joined
                .map_err(|e| PortError::Render(format!("join lecteur pipe : {e}")))?
                .map_err(|e| PortError::from(KwinScreencapError::Pipe(e.to_string())))?,
            Err(_) => {
                tracing::warn!(
                    "screenshot: timeout {:?} lecture pipe ScreenShot2 (KWin n'a jamais ferme le pipe)",
                    KWIN_PIPE_TIMEOUT
                );
                return Err(PortError::from(KwinScreencapError::Pipe(
                    "timeout lecture pipe de capture (KWin n'a jamais ferme le pipe)".into(),
                )));
            }
        };

        encode_capture(&meta, &raw).map_err(PortError::from)
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        let (_uuid, rect) = self.resolve_rect_lazy().await.map_err(PortError::from)?;
        Ok(Size::new(rect.width, rect.height))
    }
}

impl KwinRenderer {
    /// Resout l'origine ecran `(x, y)` de la fenetre cible : cache d'abord
    /// (meme entree que [`KwinRenderer::viewport_size`], cf.
    /// [`KwinRenderer::resolve_rect_lazy`]), sinon script KWin ephemere — sans
    /// jamais ouvrir de connexion D-Bus sur un cache hit.
    ///
    /// Retourne [`KwinScreencapError::WindowNotFound`] si aucune fenetre ne
    /// correspond au titre. Repli sur `(0, 0)` si `frameGeometry` ne renvoie
    /// pas de position valide (fenetre en cours d'ouverture, cas rare).
    pub async fn window_origin(&self) -> Result<(i32, i32), KwinScreencapError> {
        let (_uuid, rect) = self.resolve_rect_lazy().await?;
        Ok((rect.x, rect.y))
    }
}

/// Génère le script KWin qui résout la fenêtre par titre et renvoie son UUID +
/// géométrie écran complète (`"x,y,largeur,hauteur"`) via `callDBus` vers
/// `dest` (le nom unique de notre connexion). Sert à la fois la taille
/// ([`Renderer::viewport_size`]) et la position
/// ([`KwinRenderer::window_origin`]) : une seule résolution/un seul cache
/// pour les deux plutôt que deux scripts quasi identiques.
fn build_resolve_script(needle: &str, dest: &str) -> String {
    // serde_json produit des littéraux JS/JSON sûrs (échappement des guillemets).
    let needle_js = serde_json::to_string(&needle.to_lowercase()).expect("string serialise");
    let dest_js = serde_json::to_string(dest).expect("string serialise");
    format!(
        r#"var dest = {dest_js};
var needle = {needle_js};
var list = (typeof workspace.windowList === "function") ? workspace.windowList()
         : (typeof workspace.clientList === "function") ? workspace.clientList()
         : workspace.stackingOrder;
var uuid = "";
var rect = "0,0,0,0";
for (var i = 0; i < list.length; i++) {{
    var w = list[i];
    if (String(w.caption).toLowerCase().indexOf(needle) !== -1) {{
        uuid = String(w.internalId);
        var g = w.frameGeometry;
        if (g) {{ rect = Math.round(g.x) + "," + Math.round(g.y) + "," + Math.round(g.width) + "," + Math.round(g.height); }}
        break;
    }}
}}
callDBus(dest, "{BRIDGE_PATH}", "{BRIDGE_IFACE}", "Report", uuid, rect);
"#
    )
}

/// Parse une géométrie `"x,y,largeur,hauteur"` en [`WindowRect`].
fn parse_resolved_rect(s: &str) -> Option<WindowRect> {
    let mut parts = s.split(',');
    let x = parts.next()?.trim().parse().ok()?;
    let y = parts.next()?.trim().parse().ok()?;
    let width = parts.next()?.trim().parse().ok()?;
    let height = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(WindowRect::new(x, y, width, height))
}

/// Extrait une clé entière du dict de métadonnées (KWin peut typer en i32/u32).
fn meta_u32(meta: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    let v = meta.get(key)?;
    if let Ok(x) = u32::try_from(v) {
        return Some(x);
    }
    if let Ok(x) = i32::try_from(v) {
        return u32::try_from(x).ok();
    }
    if let Ok(x) = u64::try_from(v) {
        return u32::try_from(x).ok();
    }
    if let Ok(x) = i64::try_from(v) {
        return u32::try_from(x).ok();
    }
    None
}

/// Convertit un buffer brut BGRA (ARGB32 little-endian de KWin) — éventuellement
/// avec un `stride` > `width*4` — en RGBA compact attendu par l'encodeur PNG.
///
/// L1 : retourne une erreur si `stride < width * 4` (evite une panique dans la
/// slice sur les lignes).
fn bgra_to_rgba(
    raw: &[u8],
    width: u32,
    height: u32,
    stride: u32,
) -> Result<Vec<u8>, KwinScreencapError> {
    let row_bytes = (width as usize).saturating_mul(4);
    let stride = stride as usize;
    if stride < row_bytes {
        return Err(KwinScreencapError::Decode(format!(
            "stride ({stride}) inferieur a width*4 ({row_bytes})"
        )));
    }
    let mut rgba = vec![0u8; row_bytes * height as usize];
    for y in 0..height as usize {
        let src = &raw[y * stride..y * stride + row_bytes];
        let dst = &mut rgba[y * row_bytes..(y + 1) * row_bytes];
        for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
            d[0] = s[2]; // R <- B
            d[1] = s[1]; // G
            d[2] = s[0]; // B <- R
            d[3] = s[3]; // A
        }
    }
    Ok(rgba)
}

/// Décode le dict de métadonnées + les pixels bruts du pipe en PNG.
fn encode_capture(
    meta: &HashMap<String, OwnedValue>,
    raw: &[u8],
) -> Result<Screenshot, KwinScreencapError> {
    let width = meta_u32(meta, "width")
        .ok_or_else(|| KwinScreencapError::Decode("metadonnee 'width' absente".into()))?;
    let height = meta_u32(meta, "height")
        .ok_or_else(|| KwinScreencapError::Decode("metadonnee 'height' absente".into()))?;
    let stride = meta_u32(meta, "stride").unwrap_or_else(|| width.saturating_mul(4));

    let needed = (stride as usize).saturating_mul(height as usize);
    if raw.len() < needed {
        return Err(KwinScreencapError::Decode(format!(
            "pipe trop court : {} octets (attendu >= stride*height = {}*{} = {needed})",
            raw.len(),
            stride,
            height
        )));
    }

    let rgba = bgra_to_rgba(raw, width, height, stride)?;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(|e| KwinScreencapError::Decode(format!("encodage PNG : {e}")))?;

    Ok(Screenshot::new_png(png, Size::new(width, height)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_resolve_script_inline_needle_lowercase_and_escapes() {
        let js = build_resolve_script("Mon \"Atelier\"", ":1.42");
        assert!(js.contains(r#"var needle = "mon \"atelier\"""#));
        assert!(js.contains(r#"var dest = ":1.42""#));
        assert!(js.contains("callDBus(dest"));
        assert!(js.contains("org.deveye.KwinBridge"));
        assert!(js.contains("g.x"));
        assert!(js.contains("g.y"));
        assert!(js.contains("g.width"));
        assert!(js.contains("g.height"));
    }

    #[test]
    fn parse_resolved_rect_ok_and_bad() {
        assert_eq!(
            parse_resolved_rect("10,20,1920,1080"),
            Some(WindowRect::new(10, 20, 1920, 1080))
        );
        assert_eq!(
            parse_resolved_rect(" -5 , 0 , 800 , 600 "),
            Some(WindowRect::new(-5, 0, 800, 600))
        );
        assert_eq!(
            parse_resolved_rect("0,0,0,0"),
            Some(WindowRect::new(0, 0, 0, 0))
        );
        assert_eq!(parse_resolved_rect("nope"), None);
        assert_eq!(parse_resolved_rect("1,2,3"), None);
        assert_eq!(parse_resolved_rect("1,2,3,4,5"), None);
    }

    #[test]
    fn bgra_to_rgba_swaps_red_blue() {
        // 1x1 BGRA rouge pur -> RGBA rouge pur.
        let raw = [0u8, 0, 255, 255];
        assert_eq!(bgra_to_rgba(&raw, 1, 1, 4).unwrap(), vec![255, 0, 0, 255]);
    }

    #[test]
    fn bgra_to_rgba_honours_stride_padding() {
        // 1x2, stride 8 (4 octets de padding par ligne, valeurs 9).
        let raw = [
            0, 0, 255, 255, 9, 9, 9, 9, // ligne 0 : rouge + padding
            0, 255, 0, 255, 9, 9, 9, 9, // ligne 1 : vert + padding
        ];
        assert_eq!(
            bgra_to_rgba(&raw, 1, 2, 8).unwrap(),
            vec![255, 0, 0, 255, 0, 255, 0, 255]
        );
    }

    #[test]
    fn bgra_to_rgba_rejects_stride_too_small() {
        // stride 3 < width 1 * 4 = 4 : doit retourner une erreur Decode.
        let raw = [0u8; 4];
        let err = bgra_to_rgba(&raw, 1, 1, 3).unwrap_err();
        assert!(matches!(err, KwinScreencapError::Decode(_)));
    }

    #[tokio::test]
    async fn window_origin_returns_cached_position_without_dbus() {
        // Cache pre-rempli => window_origin() doit repondre depuis le cache
        // (resolve_rect_lazy) sans jamais tenter Connection::session() : donc
        // sans risque de toucher une session KWin live pendant `cargo test`
        // (contrairement a tests/live_resolve.rs, deliberement #[ignore]).
        let title = "Renderer-Test-Window-Origin";
        resolve_cache::put(
            title,
            ResolvedWindow {
                uuid: "u-origin".to_string(),
                rect: WindowRect::new(42, 7, 300, 200),
            },
        )
        .await;
        let renderer = KwinRenderer::new(title);
        let origin = renderer
            .window_origin()
            .await
            .expect("cache hit : pas de D-Bus necessaire");
        assert_eq!(origin, (42, 7));
        resolve_cache::invalidate(title).await;
    }

    #[tokio::test]
    async fn viewport_size_returns_cached_size_without_dbus() {
        let title = "Renderer-Test-Viewport-Size";
        resolve_cache::put(
            title,
            ResolvedWindow {
                uuid: "u-size".to_string(),
                rect: WindowRect::new(0, 0, 1024, 768),
            },
        )
        .await;
        let renderer = KwinRenderer::new(title);
        let size = renderer
            .viewport_size()
            .await
            .expect("cache hit : pas de D-Bus necessaire");
        assert_eq!(size, Size::new(1024, 768));
        resolve_cache::invalidate(title).await;
    }
}
