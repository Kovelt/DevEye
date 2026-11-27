//! Impls stub/noop locales au binaire : `EmptyWindowLister`, `UnsupportedInspector`,
//! `UnsupportedEvaluator`, `UnsupportedWindowManager`, `UnsupportedWindowGeometry`,
//! `UnsupportedClipboard`. Les messages sont GENERIQUES (aucune cible nommee) :
//! ces stubs servent toutes les cibles natives (kwin-eis, x11...).

use async_trait::async_trait;

use deveye_core::elements::{ElementLocation, ElementSnapshot, SnapshotQuery};
use deveye_core::{DomTree, StateDump, WindowInfo, WindowRect};
use deveye_ports::{
    AppInspector, Clipboard, ElementError, ElementInspector, ElementResult, JsEvaluator, PortError,
    PortResult, WindowGeometry, WindowLister, WindowManager,
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

/// Inspecteur DOM indisponible (cible native : rendu sans WebView).
/// Le state applicatif continue d'arriver par le SDK (state_inspector dedie).
pub(crate) struct UnsupportedInspector;

#[async_trait]
impl AppInspector for UnsupportedInspector {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        Err(PortError::Unsupported(
            "DOM indisponible sur cette cible (rendu natif, pas de WebView)".to_string(),
        ))
    }
    async fn state_snapshot(&self) -> PortResult<StateDump> {
        Err(PortError::Unsupported(
            "state DOM indisponible sur cette cible (rendu natif)".to_string(),
        ))
    }
}

/// Evaluateur JS indisponible (cible native : pas de WebView a piloter).
pub(crate) struct UnsupportedEvaluator;

#[async_trait]
impl JsEvaluator for UnsupportedEvaluator {
    async fn evaluate(&self, _expression: &str) -> PortResult<serde_json::Value> {
        Err(PortError::Unsupported(
            "eval JS indisponible sur cette cible (rendu natif, pas de WebView)".to_string(),
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

/// Geometrie de fenetre indisponible : cible CDP/SDK (webview, pas de fenetre
/// OS a mesurer) ou session hors KDE/Wayland (pas de KWin Scripting).
pub(crate) struct UnsupportedWindowGeometry;

#[async_trait]
impl WindowGeometry for UnsupportedWindowGeometry {
    async fn geometry(&self, _title_substring: Option<&str>) -> PortResult<WindowRect> {
        Err(PortError::Unsupported(
            "window_geometry nécessite KDE/Wayland (KWin Scripting) avec --screencap-window"
                .to_string(),
        ))
    }
}

/// Presse-papiers indisponible : session hors Wayland (pas de wl-clipboard), ou
/// cible isolee (x11) qui ne doit jamais toucher le presse-papiers de la
/// session vivante.
pub(crate) struct UnsupportedClipboard;

#[async_trait]
impl Clipboard for UnsupportedClipboard {
    async fn get(&self) -> PortResult<String> {
        Err(PortError::Unsupported(
            "presse-papiers indisponible sur cette cible (session Wayland requise)".to_string(),
        ))
    }
    async fn set(&self, _text: &str) -> PortResult<()> {
        Err(PortError::Unsupported(
            "presse-papiers indisponible sur cette cible (session Wayland requise)".to_string(),
        ))
    }
}

/// Éléments par référence indisponibles sur cette cible : `hint` dit quoi faire à la
/// place. Sert les cibles à webview tant qu'aucun inspecteur d'éléments ne leur est
/// câblé (ADR-0073).
pub(crate) struct UnsupportedElementInspector {
    hint: String,
}

impl UnsupportedElementInspector {
    pub(crate) fn new(hint: impl Into<String>) -> Self {
        Self { hint: hint.into() }
    }

    fn error(&self) -> ElementError {
        ElementError::Unsupported {
            hint: self.hint.clone(),
        }
    }
}

#[async_trait]
impl ElementInspector for UnsupportedElementInspector {
    async fn snapshot(&self, _query: &SnapshotQuery) -> ElementResult<ElementSnapshot> {
        Err(self.error())
    }

    async fn locate(&self, _reference: &str) -> ElementResult<ElementLocation> {
        Err(self.error())
    }
}
