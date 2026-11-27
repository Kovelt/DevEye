//! `WindowManager` via un script KWin : active (focus OS) la fenêtre dont le
//! `caption` contient une sous-chaîne. Même mécanique que la résolution de
//! géométrie de [`crate::KwinRenderer`] (script éphémère + callback D-Bus
//! porté par [`crate::kwin_bridge`]), mais le script appelle l'API
//! d'activation KWin puis renvoie le `caption` activé.
//!
//! Ne consulte volontairement PAS [`crate::resolve_cache`] : son script
//! combine résolution ET action (rendre la fenêtre active), donc un cache hit
//! sauterait l'activation elle-même. Il invalide l'entrée après une
//! activation réussie (une fenêtre qui vient d'être activée/déminimisée peut
//! avoir changé de position sous certains gestionnaires).

use async_trait::async_trait;
use zbus::Connection;

use deveye_ports::{PortError, PortResult, WindowManager};

use crate::kwin_bridge::{run_kwin_script, BRIDGE_IFACE, BRIDGE_PATH};
use crate::resolve_cache;

/// Active une fenêtre KDE/Wayland par sous-chaîne de titre via KWin Scripting.
#[derive(Clone, Debug)]
pub struct KwinWindowManager {
    /// Titre par défaut (typiquement `--screencap-window`) utilisé quand
    /// `activate(None)` est appelé.
    default_title: String,
}

impl KwinWindowManager {
    pub fn new(default_title: impl Into<String>) -> Self {
        Self {
            default_title: default_title.into(),
        }
    }
}

#[async_trait]
impl WindowManager for KwinWindowManager {
    async fn activate(&self, title_substring: Option<&str>) -> PortResult<String> {
        let needle = title_substring.unwrap_or(&self.default_title);
        if needle.is_empty() {
            return Err(PortError::Unsupported(
                "focus_window nécessite un titre (title_pattern ou --screencap-window)".to_string(),
            ));
        }

        let conn = Connection::session()
            .await
            .map_err(|e| PortError::Render(format!("D-Bus session : {e}")))?;
        let dest = conn
            .unique_name()
            .map(|n| n.to_string())
            .ok_or_else(|| PortError::Render("connexion D-Bus sans nom unique".to_string()))?;

        let script = build_activate_script(needle, &dest);
        let (caption, _) = run_kwin_script(&conn, "deveye-activate", &script).await?;

        if caption.is_empty() {
            // Échec = fenêtre introuvable : purge aussi l'entrée de cache pour
            // ce titre (symétrie avec le succès et avec les invalidations sur
            // échec de KwinRenderer/KwinWindowGeometry) — une entrée stale ne
            // doit pas survivre à une activation qui vient d'échouer sur ce
            // même titre (revue P5).
            resolve_cache::invalidate(needle).await;
            return Err(PortError::Interaction(format!(
                "fenêtre introuvable pour activation : {needle}"
            )));
        }
        // Purge l'entree de cache partagee (cf. crate::resolve_cache) : une
        // fenetre qui vient d'etre activee/deminimisee peut avoir change de
        // position, la prochaine resolution doit repartir d'un script frais.
        resolve_cache::invalidate(needle).await;
        Ok(caption)
    }
}

/// Script KWin : trouve la fenêtre par `caption` (sous-chaîne, minuscules), la
/// rend active (tente `activeWindow` puis `activeClient` selon la version de
/// KWin), et renvoie son `caption` via `callDBus`.
fn build_activate_script(needle: &str, dest: &str) -> String {
    let needle_js = serde_json::to_string(&needle.to_lowercase()).expect("string serialise");
    let dest_js = serde_json::to_string(dest).expect("string serialise");
    format!(
        r#"var dest = {dest_js};
var needle = {needle_js};
var list = (typeof workspace.windowList === "function") ? workspace.windowList()
         : (typeof workspace.clientList === "function") ? workspace.clientList()
         : workspace.stackingOrder;
var caption = "";
for (var i = 0; i < list.length; i++) {{
    var w = list[i];
    if (String(w.caption).toLowerCase().indexOf(needle) !== -1) {{
        caption = String(w.caption);
        try {{ workspace.activeWindow = w; }} catch (e) {{}}
        try {{ workspace.activeClient = w; }} catch (e) {{}}
        break;
    }}
}}
callDBus(dest, "{BRIDGE_PATH}", "{BRIDGE_IFACE}", "Report", caption, "");
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_activate_script_lowercases_needle_and_sets_active() {
        let js = build_activate_script("Mon \"App\"", ":1.42");
        assert!(js.contains(r#"var needle = "mon \"app\"""#));
        assert!(js.contains(r#"var dest = ":1.42""#));
        assert!(js.contains("workspace.activeWindow = w"));
        assert!(js.contains("workspace.activeClient = w"));
        assert!(js.contains("callDBus(dest"));
        assert!(js.contains("org.deveye.KwinBridge"));
    }
}
