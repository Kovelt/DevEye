use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Identité d'une app branchée à DevEye à un moment donné.
///
/// `id` est arbitraire et choisi par le binaire orchestrateur — typiquement
/// le nom du projet (`"chronos"`). `kind` aide les couches supérieures (UI,
/// MCP) à présenter les bonnes interactions selon la cible.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSession {
    pub id: String,
    pub kind: AppKind,
    pub started_at: DateTime<Utc>,
}

/// Catégorie d'app cible.
///
/// Sert à choisir le bon adapter et à informer le consommateur (Claude, UI
/// humaine) du genre d'interactions possibles. `Unknown` est un placeholder
/// quand le binaire orchestrateur attache une cible sans la qualifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppKind {
    /// Dioxus desktop (wry WebView2 sur Windows, WKWebView sur macOS, WebKitGTK Linux).
    DioxusDesktop,
    /// Dioxus servi en WASM via `dx serve --platform web`.
    DioxusWeb,
    /// App Tauri (webview + backend Rust).
    Tauri,
    /// Toute autre app embarquant un webview Chromium-based (Electron…).
    GenericWebview,
    /// Process opaque pour DevEye — utile pour le mode screencap pur.
    #[default]
    Unknown,
}

impl AppSession {
    pub fn new(id: impl Into<String>, kind: AppKind) -> Self {
        Self {
            id: id.into(),
            kind,
            started_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_uses_provided_id_and_default_started_at_is_recent() {
        let before = Utc::now();
        let session = AppSession::new("chronos", AppKind::DioxusDesktop);
        let after = Utc::now();
        assert_eq!(session.id, "chronos");
        assert_eq!(session.kind, AppKind::DioxusDesktop);
        assert!(session.started_at >= before && session.started_at <= after);
    }

    #[test]
    fn kind_serializes_in_kebab_case() {
        let json = serde_json::to_string(&AppKind::DioxusDesktop).unwrap();
        assert_eq!(json, "\"dioxus-desktop\"");
        let parsed: AppKind = serde_json::from_str("\"generic-webview\"").unwrap();
        assert_eq!(parsed, AppKind::GenericWebview);
    }
}
