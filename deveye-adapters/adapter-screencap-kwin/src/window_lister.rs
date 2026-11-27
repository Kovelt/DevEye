//! `WindowLister` via un script KWin : énumère `workspace.windowList()`
//! (KWin 6 ; repli `.clientList()` sur KWin 5) et renvoie un `WindowInfo` par
//! fenêtre. Même mécanique de script éphémère + callback D-Bus que
//! [`crate::KwinRenderer`]/[`crate::KwinWindowManager`] (cf.
//! [`crate::kwin_bridge`]), mais reporte une liste entière (sérialisée en
//! JSON) plutôt qu'un unique résultat.
//!
//! Décodage via `serde_json::Value` brut (pas de `#[derive(Deserialize)]`) :
//! ce crate ne dépend que de `serde_json`, pas de `serde` en direct.

use async_trait::async_trait;
use serde_json::Value;
use zbus::Connection;

use deveye_core::WindowInfo;
use deveye_ports::{PortError, PortResult, WindowLister};

use crate::kwin_bridge::{run_kwin_script, BRIDGE_IFACE, BRIDGE_PATH};

/// Énumère les fenêtres KDE/Wayland ouvertes via KWin Scripting.
///
/// Contrairement à l'adapter CDP (`targetId`/`url` réels), KWin n'a pas de
/// notion d'URL : [`WindowInfo::url`] est toujours vide et `kind` porte la
/// `resourceClass` (nom d'appli, ex. `"org.kde.konsole"`) plutôt qu'un type
/// de cible CDP (`"page"`, `"iframe"`).
#[derive(Clone, Copy, Debug, Default)]
pub struct KwinWindowLister;

impl KwinWindowLister {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl WindowLister for KwinWindowLister {
    async fn list_windows(&self) -> PortResult<Vec<WindowInfo>> {
        let conn = Connection::session()
            .await
            .map_err(|e| PortError::Render(format!("D-Bus session : {e}")))?;
        let dest = conn
            .unique_name()
            .map(|n| n.to_string())
            .ok_or_else(|| PortError::Render("connexion D-Bus sans nom unique".to_string()))?;

        let script = build_list_script(&dest);
        let (json, _) = run_kwin_script(&conn, "deveye-list", &script).await?;

        let value: Value = serde_json::from_str(&json)
            .map_err(|e| PortError::Render(format!("decodage liste fenetres KWin : {e}")))?;
        let array = value.as_array().ok_or_else(|| {
            PortError::Render(format!("liste fenetres KWin : JSON non-tableau ({json:?})"))
        })?;
        Ok(array.iter().map(window_info_from_json).collect())
    }
}

/// Extrait un `WindowInfo` d'un objet JSON `{id, title, kind}` (miroir du
/// JSON produit par [`build_list_script`]). Champs absents/mal typés ->
/// chaîne vide plutôt qu'une erreur : mieux vaut une liste dégradée qu'un
/// `list_windows` qui échoue en bloc sur une seule fenêtre exotique.
fn window_info_from_json(v: &Value) -> WindowInfo {
    let field = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let kind = field("kind");
    WindowInfo::new(
        field("id"),
        field("title"),
        String::new(),
        if kind.is_empty() {
            "kwin".to_string()
        } else {
            kind
        },
    )
}

/// Génère le script KWin qui liste les fenêtres et renvoie un tableau JSON
/// `[{id, title, kind}, ...]` via `callDBus` vers `dest`.
fn build_list_script(dest: &str) -> String {
    let dest_js = serde_json::Value::from(dest).to_string();
    format!(
        r#"var dest = {dest_js};
var list = (typeof workspace.windowList === "function") ? workspace.windowList()
         : (typeof workspace.clientList === "function") ? workspace.clientList()
         : workspace.stackingOrder;
var out = [];
for (var i = 0; i < list.length; i++) {{
    var w = list[i];
    out.push({{
        id: String(w.internalId),
        title: String(w.caption),
        kind: String(w.resourceClass || "")
    }});
}}
callDBus(dest, "{BRIDGE_PATH}", "{BRIDGE_IFACE}", "Report", JSON.stringify(out), "");
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_list_script_embeds_dest_and_reports_json_array() {
        let js = build_list_script(":1.42");
        assert!(js.contains(r#"var dest = ":1.42""#));
        assert!(js.contains("JSON.stringify(out)"));
        assert!(js.contains("w.internalId"));
        assert!(js.contains("w.caption"));
        assert!(js.contains("w.resourceClass"));
        assert!(js.contains("callDBus(dest"));
    }

    #[test]
    fn window_info_from_json_empty_kind_falls_back_to_kwin() {
        let v = serde_json::json!({"id": "abc", "title": "Mon App", "kind": ""});
        let info = window_info_from_json(&v);
        assert_eq!(info.id, "abc");
        assert_eq!(info.title, "Mon App");
        assert_eq!(info.kind, "kwin");
        assert_eq!(info.url, "");
    }

    #[test]
    fn window_info_from_json_keeps_non_empty_kind() {
        let v = serde_json::json!({"id": "1", "title": "t", "kind": "org.kde.konsole"});
        assert_eq!(window_info_from_json(&v).kind, "org.kde.konsole");
    }

    #[test]
    fn window_info_from_json_missing_fields_default_to_empty() {
        let v = serde_json::json!({});
        let info = window_info_from_json(&v);
        assert_eq!(info.id, "");
        assert_eq!(info.title, "");
        assert_eq!(info.kind, "kwin");
    }
}
