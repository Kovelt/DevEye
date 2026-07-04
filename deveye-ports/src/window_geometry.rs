use async_trait::async_trait;

use deveye_core::WindowRect;

use crate::errors::PortResult;

/// Lit la géométrie écran (position + taille) d'une fenêtre de l'app cible.
///
/// Motivation (cible `kwin-eis`) : le rendu natif (Uno/Skia) n'a pas de
/// viewport CDP — un agent qui doit cliquer à des coordonnées écran (plutôt
/// que logiques webview) a besoin de l'origine réelle de la fenêtre en plus
/// de sa taille. Complète [`crate::WindowManager`] (focus) et
/// [`crate::Renderer`] (capture pixels).
///
/// Implémenté par `adapter-screencap-kwin` (`KwinWindowGeometry`, via un
/// script KWin). Indisponible hors KDE/Wayland ou en cible CDP/SDK →
/// `PortError::Unsupported`.
#[async_trait]
pub trait WindowGeometry: Send + Sync {
    /// Géométrie de la fenêtre dont le `caption` contient `title_substring`
    /// (insensible à la casse). `None` cible la fenêtre par défaut de
    /// l'adapter (typiquement `--screencap-window`).
    async fn geometry(&self, title_substring: Option<&str>) -> PortResult<WindowRect>;
}
