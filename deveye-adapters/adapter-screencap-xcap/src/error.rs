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
