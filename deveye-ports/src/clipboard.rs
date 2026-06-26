use async_trait::async_trait;

use crate::errors::PortResult;

/// Accès au presse-papiers de la session.
///
/// Motivation : `navigator.clipboard.readText()` lève `NotAllowedError` en
/// contexte automatisé (pas de geste utilisateur), et en `kwin-eis` il n'y a
/// pas de WebView du tout. Un accès OS-level (sous Wayland : `wl-copy` /
/// `wl-paste`) est la seule voie fiable sur toutes les cibles.
///
/// Implémenté par `adapter-clipboard-wayland`. Hors Wayland (ou outils
/// `wl-clipboard` absents) → `PortError::Unsupported`.
#[async_trait]
pub trait Clipboard: Send + Sync {
    /// Lit le texte courant du presse-papiers (chaîne vide si vide).
    async fn get(&self) -> PortResult<String>;
    /// Remplace le contenu du presse-papiers par `text`.
    async fn set(&self, text: &str) -> PortResult<()>;
}
