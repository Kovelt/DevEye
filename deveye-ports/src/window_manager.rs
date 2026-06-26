use async_trait::async_trait;

use crate::errors::PortResult;

/// Donne le focus OS à une fenêtre de l'app cible.
///
/// Motivation (cible `kwin-eis`) : KWin EIS route les événements d'entrée vers
/// la fenêtre **active**. Une notification Plasma ou un dialog système peut
/// voler le focus et faire partir tous les clics/saisies vers la mauvaise
/// fenêtre, sans aucun signal d'erreur. `activate` ré-établit le focus avant
/// une séquence d'interaction.
///
/// Implémenté par `adapter-screencap-kwin` (`KwinWindowManager`, via un script
/// KWin). Indisponible hors KDE/Wayland ou en cible CDP → `PortError::Unsupported`.
#[async_trait]
pub trait WindowManager: Send + Sync {
    /// Active la fenêtre dont le `caption` contient `title_substring` (insensible
    /// à la casse). `None` cible la fenêtre par défaut de l'adapter (typiquement
    /// `--screencap-window`). Renvoie le `caption` de la fenêtre activée.
    async fn activate(&self, title_substring: Option<&str>) -> PortResult<String>;
}
