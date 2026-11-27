//! `WindowGeometry` via un script KWin : lit `frameGeometry` (x, y, largeur,
//! hauteur) de la fenêtre dont le `caption` contient une sous-chaîne. Même
//! mécanique de script éphémère + callback D-Bus que
//! [`crate::KwinWindowManager`] (cf. [`crate::kwin_bridge`]).
//!
//! Résolution passée par [`crate::resolve_cache`] (TTL court) : le cache est
//! partagé avec [`crate::KwinRenderer`] (même UUID + géométrie sous-jacents),
//! donc un `screenshot`/`viewport_size`/`window_origin` recent sur la même
//! fenêtre évite ici tout aller-retour D-Bus.

use async_trait::async_trait;
use zbus::Connection;

use deveye_core::WindowRect;
use deveye_ports::{PortError, PortResult, WindowGeometry};

use crate::error::KwinScreencapError;
use crate::kwin_bridge::{run_kwin_script, BRIDGE_IFACE, BRIDGE_PATH};
use crate::resolve_cache::{self, ResolvedWindow};

/// Lit la géométrie écran d'une fenêtre KDE/Wayland via KWin Scripting.
#[derive(Clone, Debug)]
pub struct KwinWindowGeometry {
    /// Titre par défaut (typiquement `--screencap-window`) utilisé quand
    /// `geometry(None)` est appelé — même convention que
    /// [`crate::KwinWindowManager`].
    default_title: String,
}

impl KwinWindowGeometry {
    pub fn new(default_title: impl Into<String>) -> Self {
        Self {
            default_title: default_title.into(),
        }
    }
}

#[async_trait]
impl WindowGeometry for KwinWindowGeometry {
    async fn geometry(&self, title_substring: Option<&str>) -> PortResult<WindowRect> {
        let needle = title_substring.unwrap_or(&self.default_title);
        if needle.is_empty() {
            return Err(PortError::Unsupported(
                "window_geometry_get nécessite un titre (title_pattern ou --screencap-window)"
                    .to_string(),
            ));
        }

        // NE LIT PAS le cache : `window_geometry` renvoie des coordonnées que
        // l'agent exploite pour DÉCIDER (calcul d'un clic, « la fenêtre a-t-elle
        // bougé ? »). Une fenêtre fermée-rouverte au même titre reçoit un
        // nouvel `internalId` et souvent une autre géométrie ; servir une entrée
        // périmée donnerait un rect faux SANS erreur (revue P5). Cet outil est
        // rare (pas un chemin chaud) : on résout toujours frais. On ÉCRIT quand
        // même le résultat pour qu'un `screenshot` proche en profite.
        let conn = Connection::session()
            .await
            .map_err(|e| PortError::Render(format!("D-Bus session : {e}")))?;
        let dest = conn
            .unique_name()
            .map(|n| n.to_string())
            .ok_or_else(|| PortError::Render("connexion D-Bus sans nom unique".to_string()))?;

        let script = build_geometry_script(needle, &dest);
        let (uuid, rect_str) = run_kwin_script(&conn, "deveye-geometry", &script).await?;

        if uuid.is_empty() {
            resolve_cache::invalidate(needle).await;
            return Err(KwinScreencapError::WindowNotFound(needle.to_string()).into());
        }
        let rect = parse_rect(&rect_str)
            .ok_or_else(|| PortError::Render(format!("geometrie KWin illisible : {rect_str:?}")))?;
        resolve_cache::put(
            needle,
            ResolvedWindow {
                uuid,
                rect: rect.clone(),
            },
        )
        .await;
        Ok(rect)
    }
}

/// Génère le script KWin qui trouve la fenêtre par titre et renvoie sa
/// `frameGeometry` en `"x,y,largeur,hauteur"` via `callDBus` ; le premier
/// retour est l'`internalId` (UUID KWin) de la fenêtre trouvée (vide si
/// aucune fenêtre ne correspond) — même convention que
/// [`crate::renderer::KwinRenderer`], ce qui permet de partager le cache de
/// résolution entre les deux.
fn build_geometry_script(needle: &str, dest: &str) -> String {
    let needle_js = serde_json::Value::from(needle.to_lowercase()).to_string();
    let dest_js = serde_json::Value::from(dest).to_string();
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
        if (g) {{
            rect = Math.round(g.x) + "," + Math.round(g.y) + "," + Math.round(g.width) + "," + Math.round(g.height);
        }}
        break;
    }}
}}
callDBus(dest, "{BRIDGE_PATH}", "{BRIDGE_IFACE}", "Report", uuid, rect);
"#
    )
}

/// Parse `"x,y,largeur,hauteur"` en [`WindowRect`].
fn parse_rect(s: &str) -> Option<WindowRect> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_geometry_script_lowercases_needle_and_reports_rect() {
        let js = build_geometry_script("Mon \"App\"", ":1.42");
        assert!(js.contains(r#"var needle = "mon \"app\"""#));
        assert!(js.contains(r#"var dest = ":1.42""#));
        assert!(js.contains("g.x"));
        assert!(js.contains("g.y"));
        assert!(js.contains("g.width"));
        assert!(js.contains("g.height"));
        assert!(js.contains("callDBus(dest"));
    }

    #[test]
    fn parse_rect_ok_and_bad() {
        assert_eq!(
            parse_rect("10,20,800,600"),
            Some(WindowRect::new(10, 20, 800, 600))
        );
        assert_eq!(
            parse_rect(" -5 , 0 , 1920 , 1080 "),
            Some(WindowRect::new(-5, 0, 1920, 1080))
        );
        assert_eq!(parse_rect("0,0,0,0"), Some(WindowRect::new(0, 0, 0, 0)));
        assert_eq!(parse_rect("nope"), None);
        assert_eq!(parse_rect("1,2,3"), None);
        assert_eq!(parse_rect("1,2,3,4,5"), None);
    }

    #[tokio::test]
    async fn geometry_with_empty_needle_is_unsupported_before_any_dbus_call() {
        // Le garde-fou renvoie avant Connection::session() : verifiable sans
        // session D-Bus live.
        let geo = KwinWindowGeometry::new("");
        let err = geo.geometry(None).await.unwrap_err();
        assert!(matches!(err, PortError::Unsupported(_)));
    }

    // NB : `geometry()` ne LIT PAS le cache (résolution toujours fraîche, cf.
    // revue P5) — le chemin nominal exige donc une session KWin live et n'est
    // pas testable ici sans mock D-Bus (couverture prévue au sprint Q6 via un
    // seam sur `run_kwin_script`). Les primitives de cache elles-mêmes sont
    // couvertes par `crate::resolve_cache::tests`.
}
