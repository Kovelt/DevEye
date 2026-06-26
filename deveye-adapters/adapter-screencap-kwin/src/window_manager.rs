//! `WindowManager` via un script KWin : active (focus OS) la fenêtre dont le
//! `caption` contient une sous-chaîne. Même mécanique que la résolution de
//! géométrie de [`crate::KwinRenderer`] (script éphémère + callback D-Bus
//! `Bridge`), mais le script appelle l'API d'activation KWin puis renvoie le
//! `caption` activé.

use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{oneshot, Mutex};
use zbus::Connection;

use deveye_ports::{PortError, PortResult, WindowManager};

use crate::proxy::ScriptingProxy;

const BRIDGE_PATH: &str = "/deveye/bridge";
const BRIDGE_IFACE: &str = "org.deveye.KwinBridge";

/// Interface éphémère : le script KWin appelle `Report(caption, "")` avec le
/// `caption` de la fenêtre activée (ou `""` si aucune ne correspond).
struct Bridge {
    tx: Mutex<Option<oneshot::Sender<(String, String)>>>,
}

#[zbus::interface(name = "org.deveye.KwinBridge")]
impl Bridge {
    #[zbus(name = "Report")]
    async fn report(&self, caption: String, _extra: String) {
        if let Some(tx) = self.tx.lock().await.take() {
            let _ = tx.send((caption, String::new()));
        }
    }
}

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

        let (tx, rx) = oneshot::channel();
        conn.object_server()
            .at(
                BRIDGE_PATH,
                Bridge {
                    tx: Mutex::new(Some(tx)),
                },
            )
            .await
            .map_err(|e| PortError::Render(format!("object_server.at : {e}")))?;

        let plugin = format!("deveye-activate-{}", std::process::id());
        let temp = tempfile::Builder::new()
            .prefix("deveye-activate-")
            .suffix(".js")
            .tempfile()
            .map_err(|e| PortError::Render(format!("tempfile : {e}")))?;
        tokio::fs::write(temp.path(), build_activate_script(needle, &dest))
            .await
            .map_err(|e| PortError::Render(format!("ecriture script KWin : {e}")))?;

        let result = run_script(&conn, temp.path(), &plugin, rx).await;
        drop(temp);
        let _ = conn.object_server().remove::<Bridge, _>(BRIDGE_PATH).await;

        let (caption, _) = result?;
        if caption.is_empty() {
            return Err(PortError::Interaction(format!(
                "fenêtre introuvable pour activation : {needle}"
            )));
        }
        Ok(caption)
    }
}

/// Charge + démarre le script, attend la réponse (timeout 4 s), décharge.
async fn run_script(
    conn: &Connection,
    path: &std::path::Path,
    plugin: &str,
    rx: oneshot::Receiver<(String, String)>,
) -> PortResult<(String, String)> {
    let scripting = ScriptingProxy::new(conn)
        .await
        .map_err(|e| PortError::Render(format!("proxy Scripting : {e}")))?;
    scripting
        .load_script(&path.to_string_lossy(), plugin)
        .await
        .map_err(|e| PortError::Render(format!("loadScript : {e}")))?;
    scripting
        .start()
        .await
        .map_err(|e| PortError::Render(format!("Scripting.start : {e}")))?;

    let received = tokio::time::timeout(Duration::from_secs(4), rx).await;
    let _ = scripting.unload_script(plugin).await;

    received
        .map_err(|_| PortError::Timeout("script KWin sans reponse apres 4 s".to_string()))?
        .map_err(|_| PortError::Render("canal de resolution ferme".to_string()))
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
        let js = build_activate_script("Mon \"Atelier\"", ":1.42");
        assert!(js.contains(r#"var needle = "mon \"atelier\"""#));
        assert!(js.contains(r#"var dest = ":1.42""#));
        assert!(js.contains("workspace.activeWindow = w"));
        assert!(js.contains("workspace.activeClient = w"));
        assert!(js.contains("callDBus(dest"));
        assert!(js.contains("org.deveye.KwinBridge"));
    }
}
