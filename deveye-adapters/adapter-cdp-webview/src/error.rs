use deveye_ports::PortError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CdpError {
    #[error("connexion CDP : {0}")]
    Connect(String),

    #[error("envoi WebSocket : {0}")]
    Send(String),

    #[error("réception WebSocket : {0}")]
    Receive(String),

    #[error("décodage JSON-RPC : {0}")]
    Decode(String),

    #[error("réponse CDP en erreur ({code}) : {message}")]
    Remote { code: i64, message: String },

    #[error("réponse CDP inattendue : {0}")]
    Unexpected(String),

    #[error("decodage base64 : {0}")]
    Base64(String),
}

impl From<CdpError> for PortError {
    fn from(err: CdpError) -> Self {
        match err {
            CdpError::Connect(msg) | CdpError::Send(msg) | CdpError::Receive(msg) => {
                PortError::Network(msg)
            }
            other => PortError::Render(other.to_string()),
        }
    }
}
