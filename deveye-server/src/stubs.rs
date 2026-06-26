//! Impls stub/noop locales au binaire : `EmptyWindowLister`, `UnsupportedInspector`,
//! `UnsupportedEvaluator`, `UnsupportedWindowManager`.

use async_trait::async_trait;

use deveye_core::{DomTree, StateDump, WindowInfo};
use deveye_ports::{
    AppInspector, Clipboard, JsEvaluator, PortError, PortResult, WindowLister, WindowManager,
};

/// `WindowLister` du pont natif (cible `sdk`) : aucune visibilite
/// inter-fenetres — renvoie une liste vide plutot qu'une erreur.
pub(crate) struct EmptyWindowLister;

#[async_trait]
impl WindowLister for EmptyWindowLister {
    async fn list_windows(&self) -> PortResult<Vec<WindowInfo>> {
        Ok(Vec::new())
    }
}

/// Inspecteur DOM indisponible (cible `kwin-eis` : rendu natif, sans WebView).
/// Le state applicatif continue d'arriver par le SDK (state_inspector dedie).
pub(crate) struct UnsupportedInspector;

#[async_trait]
impl AppInspector for UnsupportedInspector {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        Err(PortError::Unsupported(
            "DOM indisponible en mode kwin-eis (rendu natif, pas de WebView)".to_string(),
        ))
    }
    async fn state_snapshot(&self) -> PortResult<StateDump> {
        Err(PortError::Unsupported(
            "state DOM indisponible en mode kwin-eis".to_string(),
        ))
    }
}

/// Evaluateur JS indisponible (cible `kwin-eis` : pas de WebView a piloter).
pub(crate) struct UnsupportedEvaluator;

#[async_trait]
impl JsEvaluator for UnsupportedEvaluator {
    async fn evaluate(&self, _expression: &str) -> PortResult<serde_json::Value> {
        Err(PortError::Unsupported(
            "eval JS indisponible en mode kwin-eis (rendu natif, pas de WebView)".to_string(),
        ))
    }
}

/// Gestion de fenetres indisponible : cible CDP (webview, pas de fenetre OS a
/// activer) ou session hors KDE/Wayland (pas de KWin Scripting).
pub(crate) struct UnsupportedWindowManager;

#[async_trait]
impl WindowManager for UnsupportedWindowManager {
    async fn activate(&self, _title_substring: Option<&str>) -> PortResult<String> {
        Err(PortError::Unsupported(
            "focus_window nécessite KDE/Wayland (KWin Scripting) avec --screencap-window"
                .to_string(),
        ))
    }
}

/// Presse-papiers indisponible : session hors Wayland (pas de wl-clipboard).
pub(crate) struct UnsupportedClipboard;

#[async_trait]
impl Clipboard for UnsupportedClipboard {
    async fn get(&self) -> PortResult<String> {
        Err(PortError::Unsupported(
            "presse-papiers indisponible hors session Wayland".to_string(),
        ))
    }
    async fn set(&self, _text: &str) -> PortResult<()> {
        Err(PortError::Unsupported(
            "presse-papiers indisponible hors session Wayland".to_string(),
        ))
    }
}
