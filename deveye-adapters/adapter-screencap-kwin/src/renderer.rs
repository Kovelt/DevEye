use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use image::{ExtendedColorType, ImageEncoder};
use nix::fcntl::OFlag;
use nix::unistd::pipe2;
use tokio::sync::{oneshot, Mutex};
use zbus::zvariant::{Fd, OwnedValue, Value};
use zbus::Connection;

use deveye_core::{Screenshot, Size};
use deveye_ports::{PortError, PortResult, Renderer};

use crate::error::KwinScreencapError;
use crate::proxy::{ScreenShot2Proxy, ScriptingProxy};

/// Chemin de l'objet D-Bus que cet adapter sert le temps d'une résolution ;
/// le script KWin y renvoie l'UUID de la fenêtre par `callDBus`.
const BRIDGE_PATH: &str = "/deveye/bridge";
const BRIDGE_IFACE: &str = "org.deveye.KwinBridge";

/// Compteur monotone pour des noms de plugin/scripts uniques par appel (un
/// même chemin rechargé ne ré-exécuterait pas le script).
fn next_id() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Interface éphémère servie par l'adapter : le script KWin appelle `Report`
/// avec l'UUID de la fenêtre trouvée (ou `""`) et sa géométrie `"LxH"`.
struct Bridge {
    tx: Mutex<Option<oneshot::Sender<(String, String)>>>,
}

#[zbus::interface(name = "org.deveye.KwinBridge")]
impl Bridge {
    #[zbus(name = "Report")]
    async fn report(&self, uuid: String, geometry: String) {
        if let Some(tx) = self.tx.lock().await.take() {
            let _ = tx.send((uuid, geometry));
        }
    }
}

/// Capture une fenêtre KDE/Wayland via `org.kde.KWin.ScreenShot2`.
///
/// La fenêtre est désignée par sous-chaîne (insensible à la casse) de son
/// `caption`. On résout l'UUID à chaque appel (sans cache) pour survivre aux
/// ouvertures/fermetures de fenêtres entre deux outils MCP.
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

    /// Résout `(uuid, taille_logique)` de la première fenêtre dont le titre
    /// contient `title_substring`, via un script KWin éphémère qui renvoie le
    /// résultat sur l'interface [`Bridge`] servie sur `conn`.
    async fn resolve_window(
        &self,
        conn: &Connection,
    ) -> Result<(String, Size), KwinScreencapError> {
        let dest = conn
            .unique_name()
            .map(|n| n.to_string())
            .ok_or_else(|| KwinScreencapError::Dbus("connexion D-Bus sans nom unique".into()))?;

        let (tx, rx) = oneshot::channel();
        conn.object_server()
            .at(
                BRIDGE_PATH,
                Bridge {
                    tx: Mutex::new(Some(tx)),
                },
            )
            .await
            .map_err(|e| KwinScreencapError::Dbus(format!("object_server.at : {e}")))?;

        let plugin = format!("deveye-screencap-{}-{}", std::process::id(), next_id());
        // L11: tempfile cree le fichier avec O_EXCL + nom aleatoire — evite le
        // TOCTOU d'un chemin predictible sous /tmp.
        let temp = tempfile::Builder::new()
            .prefix("deveye-screencap-")
            .suffix(".js")
            .tempfile()
            .map_err(|e| KwinScreencapError::Dbus(format!("tempfile creation : {e}")))?;
        // N1: ecriture asynchrone via tokio::fs pour ne pas bloquer le runtime.
        tokio::fs::write(temp.path(), build_script(&self.title_substring, &dest))
            .await
            .map_err(|e| KwinScreencapError::Dbus(format!("ecriture script KWin : {e}")))?;

        let result = self.run_script(conn, temp.path(), &plugin, rx).await;

        // Nettoyage best-effort : NamedTempFile supprime le fichier a sa destruction
        // (drop). La lecture du pipe (std::fs::File::from(read_fd)) se passe dans
        // spawn_blocking — hors hot-path async, pas de changement necessaire.
        drop(temp);
        let _ = conn.object_server().remove::<Bridge, _>(BRIDGE_PATH).await;

        let (uuid, geometry) = result?;
        if uuid.is_empty() {
            return Err(KwinScreencapError::WindowNotFound(
                self.title_substring.clone(),
            ));
        }
        let size = parse_geometry(&geometry).unwrap_or_else(|| Size::new(0, 0));
        Ok((uuid, size))
    }

    /// Charge + démarre le script, attend la réponse (timeout 4 s), décharge.
    async fn run_script(
        &self,
        conn: &Connection,
        path: &std::path::Path,
        plugin: &str,
        rx: oneshot::Receiver<(String, String)>,
    ) -> Result<(String, String), KwinScreencapError> {
        let scripting = ScriptingProxy::new(conn)
            .await
            .map_err(|e| KwinScreencapError::Dbus(format!("proxy Scripting : {e}")))?;
        scripting
            .load_script(&path.to_string_lossy(), plugin)
            .await
            .map_err(|e| KwinScreencapError::Unavailable(format!("loadScript : {e}")))?;
        scripting
            .start()
            .await
            .map_err(|e| KwinScreencapError::Unavailable(format!("Scripting.start : {e}")))?;

        let received = tokio::time::timeout(Duration::from_secs(4), rx).await;
        let _ = scripting.unload_script(plugin).await;

        received
            .map_err(|_| {
                KwinScreencapError::Unavailable("script KWin sans reponse apres 4 s".into())
            })?
            .map_err(|_| KwinScreencapError::Unavailable("canal de resolution ferme".into()))
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

        let (uuid, _logical) = self.resolve_window(&conn).await.map_err(PortError::from)?;

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
        let meta = ss2
            .capture_window(&uuid, options, Fd::from(write_fd.as_fd()))
            .await
            .map_err(|e| PortError::Render(format!("CaptureWindow : {e}")))?;

        // Fermer NOTRE write_fd : KWin garde sa copie ; sans ça le lecteur ne
        // voit jamais EOF.
        drop(write_fd);

        let raw = reader
            .await
            .map_err(|e| PortError::Render(format!("join lecteur pipe : {e}")))?
            .map_err(|e| PortError::from(KwinScreencapError::Pipe(e.to_string())))?;

        encode_capture(&meta, &raw).map_err(PortError::from)
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        let conn = Connection::session()
            .await
            .map_err(|e| PortError::Render(format!("D-Bus session : {e}")))?;
        let (_uuid, size) = self.resolve_window(&conn).await.map_err(PortError::from)?;
        Ok(size)
    }
}

/// Génère le script KWin qui résout la fenêtre par titre et renvoie son UUID +
/// géométrie via `callDBus` vers `dest` (le nom unique de notre connexion).
fn build_script(needle: &str, dest: &str) -> String {
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
var geom = "0x0";
for (var i = 0; i < list.length; i++) {{
    var w = list[i];
    if (String(w.caption).toLowerCase().indexOf(needle) !== -1) {{
        uuid = String(w.internalId);
        var g = w.frameGeometry;
        if (g) {{ geom = Math.round(g.width) + "x" + Math.round(g.height); }}
        break;
    }}
}}
callDBus(dest, "{BRIDGE_PATH}", "{BRIDGE_IFACE}", "Report", uuid, geom);
"#
    )
}

/// Parse une géométrie `"LARGEURxHAUTEUR"`.
fn parse_geometry(s: &str) -> Option<Size> {
    let (w, h) = s.split_once('x')?;
    Some(Size::new(w.trim().parse().ok()?, h.trim().parse().ok()?))
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
    fn build_script_inline_needle_lowercase_and_escapes() {
        let js = build_script("Mon \"Atelier\"", ":1.42");
        assert!(js.contains(r#"var needle = "mon \"atelier\"""#));
        assert!(js.contains(r#"var dest = ":1.42""#));
        assert!(js.contains("callDBus(dest"));
        assert!(js.contains("org.deveye.KwinBridge"));
    }

    #[test]
    fn parse_geometry_ok_and_bad() {
        assert_eq!(parse_geometry("1920x1080"), Some(Size::new(1920, 1080)));
        assert_eq!(parse_geometry(" 800 x 600 "), Some(Size::new(800, 600)));
        assert_eq!(parse_geometry("0x0"), Some(Size::new(0, 0)));
        assert_eq!(parse_geometry("nope"), None);
        assert_eq!(parse_geometry("12x"), None);
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
}
