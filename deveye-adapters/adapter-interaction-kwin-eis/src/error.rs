use thiserror::Error;

/// Erreurs internes de l'adapter EIS. Converties en
/// [`deveye_ports::PortError::Interaction`] à la frontière du port.
#[derive(Debug, Error)]
pub(crate) enum EisError {
    #[error("D-Bus KWin EIS : {0}")]
    DBus(#[from] zbus::Error),
    /// Toute erreur côté protocole EI/libei ou keymap (message explicite).
    #[error("EIS/libei : {0}")]
    Eis(String),
}

impl From<EisError> for deveye_ports::PortError {
    fn from(e: EisError) -> Self {
        deveye_ports::PortError::Interaction(e.to_string())
    }
}
