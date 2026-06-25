use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Identité d'une app branchée à DevEye à un moment donné.
///
/// `id` est arbitraire et choisi par le binaire orchestrateur — typiquement
/// le nom du projet (`"chronos"`). `kind` aide les couches supérieures (UI,
/// MCP) à présenter les bonnes interactions selon la cible. `ended_at` reste
/// `None` tant que la session est active ; l'orchestrateur le pose à la clôture.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSession {
    pub id: String,
    pub kind: AppKind,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
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
    /// atlas (.NET 10 / Uno Platform) en tête desktop Skia — pilotée OS-level
    /// (KWin EIS) car ni WebView ni SDK Rust embarquable.
    UnoDesktop,
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
            ended_at: None,
        }
    }

    /// Marque la session comme close (pose `ended_at` au temps courant).
    pub fn close(&mut self) {
        self.ended_at = Some(Utc::now());
    }

    /// Vrai tant que la session n'a pas été close.
    pub fn is_active(&self) -> bool {
        self.ended_at.is_none()
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
        assert!(session.ended_at.is_none());
        assert!(session.is_active());
        assert!(session.started_at >= before && session.started_at <= after);
    }

    #[test]
    fn close_sets_ended_at_and_clears_active() {
        let mut session = AppSession::new("atlas", AppKind::UnoDesktop);
        assert!(session.is_active());
        session.close();
        assert!(session.ended_at.is_some());
        assert!(!session.is_active());
    }

    #[test]
    fn kind_serializes_in_kebab_case() {
        let json = serde_json::to_string(&AppKind::DioxusDesktop).unwrap();
        assert_eq!(json, "\"dioxus-desktop\"");
        let json = serde_json::to_string(&AppKind::UnoDesktop).unwrap();
        assert_eq!(json, "\"uno-desktop\"");
        let parsed: AppKind = serde_json::from_str("\"generic-webview\"").unwrap();
        assert_eq!(parsed, AppKind::GenericWebview);
    }
}
