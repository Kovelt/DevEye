use thiserror::Error;

/// Erreur générique d'un port DevEye.
///
/// Chaque famille de ports a son propre type d'erreur ; ils convergent tous
/// vers `PortError` via `From` pour faciliter la composition côté binaire
/// orchestrateur.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PortError {
    #[error("rendu : {0}")]
    Render(String),

    #[error("inspection : {0}")]
    Inspect(String),

    #[error("interaction : {0}")]
    Interaction(String),

    #[error("logs : {0}")]
    Log(String),

    #[error("commande {route} : {message}")]
    Route { route: String, message: String },

    #[error("réseau : {0}")]
    Network(String),

    #[error("protocole : {0}")]
    Protocol(String),

    // Réservé à l'impl ConsoleErrorSource CDP-native (M-futur, pas encore construite).
    // Ne pas supprimer : les sites de match exhaustifs et l'affichage en dépendent.
    #[error("console webview : {0}")]
    Console(String),

    #[error("délai dépassé : {0}")]
    Timeout(String),

    #[error("indisponible sur cette plateforme : {0}")]
    Unsupported(String),
}

pub type PortResult<T> = Result<T, PortError>;
