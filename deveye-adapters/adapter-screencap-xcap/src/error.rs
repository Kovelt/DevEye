use deveye_ports::PortError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScreencapError {
    #[error("aucune fenêtre dont le titre contient {0:?}")]
    WindowNotFound(String),

    #[error("xcap : {0}")]
    Xcap(String),

    #[error("encodage PNG : {0}")]
    Encoding(String),
}

impl From<ScreencapError> for PortError {
    fn from(err: ScreencapError) -> Self {
        match err {
            ScreencapError::WindowNotFound(_) => PortError::Unsupported(err.to_string()),
            _ => PortError::Render(err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_not_found_maps_to_unsupported() {
        let err = ScreencapError::WindowNotFound("MonApp".into());
        assert!(matches!(PortError::from(err), PortError::Unsupported(_)));
    }

    #[test]
    fn encoding_and_xcap_errors_map_to_render() {
        let encoding = ScreencapError::Encoding("png".into());
        assert!(matches!(PortError::from(encoding), PortError::Render(_)));
        let xcap = ScreencapError::Xcap("backend".into());
        assert!(matches!(PortError::from(xcap), PortError::Render(_)));
    }
}
