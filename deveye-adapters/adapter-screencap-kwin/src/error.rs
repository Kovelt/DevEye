use deveye_ports::PortError;
use thiserror::Error;

/// Erreurs de l'adapter KWin ScreenShot2.
#[derive(Debug, Error)]
pub enum KwinScreencapError {
    #[error("D-Bus KWin : {0}")]
    Dbus(String),

    #[error("aucune fenetre dont le titre contient {0:?}")]
    WindowNotFound(String),

    #[error("le script KWin n'a pas repondu (ScreenShot2/Scripting indisponible ?) : {0}")]
    Unavailable(String),

    #[error("le script KWin n'a pas repondu dans le delai imparti : {0}")]
    Timeout(String),

    #[error("lecture du pipe KWin : {0}")]
    Pipe(String),

    #[error("decodage de l'image KWin : {0}")]
    Decode(String),
}

impl From<KwinScreencapError> for PortError {
    fn from(err: KwinScreencapError) -> Self {
        match err {
            // Absence de cible ou de support : limite de l'adapter, pas un
            // échec de rendu — cohérent avec adapter-screencap-xcap.
            KwinScreencapError::WindowNotFound(_) | KwinScreencapError::Unavailable(_) => {
                PortError::Unsupported(err.to_string())
            }
            // Un KWin qui n'a pas repondu a temps n'est PAS "plateforme non
            // supportee" : c'est un hoquet transitoire, distinct pour que
            // l'appelant (ex. focus_window) puisse retenter (revue T4).
            KwinScreencapError::Timeout(_) => PortError::Timeout(err.to_string()),
            _ => PortError::Render(err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_not_found_maps_to_unsupported() {
        let p: PortError = KwinScreencapError::WindowNotFound("atelier".into()).into();
        assert!(matches!(p, PortError::Unsupported(_)));
    }

    #[test]
    fn dbus_error_maps_to_render() {
        let p: PortError = KwinScreencapError::Dbus("boom".into()).into();
        assert!(matches!(p, PortError::Render(_)));
    }
}
