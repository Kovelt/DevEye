use async_trait::async_trait;

use deveye_core::{Screenshot, Size};

use crate::errors::PortResult;

/// Capture visuelle d'une app cible.
///
/// Une implémentation typique :
/// - tape le Chrome DevTools Protocol via `Page.captureScreenshot`
///   (`adapter-cdp-webview`) ;
/// - ou capture la fenêtre OS via `xcap` (`adapter-screencap-xcap`) — utile
///   en fallback universel quand le webview n'expose pas CDP (macOS) ;
/// - ou renvoie un PNG figé pour les tests d'intégration des couches au-dessus.
///
/// `Send + Sync` permet au serveur MCP de partager le renderer entre tâches
/// `tokio::spawn`.
#[async_trait]
pub trait Renderer: Send + Sync {
    /// Capture l'app dans l'état courant. Le PNG retourné représente le
    /// viewport tel qu'un utilisateur le voit.
    async fn screenshot(&self) -> PortResult<Screenshot>;

    /// Dimensions actuelles du viewport. Sert au routage des clics et à
    /// l'affichage côté consommateur.
    async fn viewport_size(&self) -> PortResult<Size>;
}
