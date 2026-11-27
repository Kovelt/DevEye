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
            // Erreurs de protocole JSON-RPC / CDP : semantique distincte du
            // rendu ; le fallback CDP->SDK se declenche sur Protocol, pas Render.
            CdpError::Decode(msg) | CdpError::Unexpected(msg) => PortError::Protocol(msg),
            CdpError::Remote { code, message } => {
                PortError::Protocol(format!("réponse CDP en erreur ({code}) : {message}"))
            }
            // Remaining variant : Base64 (erreur de decodage image capturée).
            other => PortError::Render(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_errors_map_to_network() {
        for err in [
            CdpError::Connect("x".into()),
            CdpError::Send("x".into()),
            CdpError::Receive("x".into()),
        ] {
            assert!(matches!(PortError::from(err), PortError::Network(_)));
        }
    }

    #[test]
    fn protocol_errors_map_to_protocol() {
        // Le repli CDP -> SDK se déclenche sur `Protocol`, pas sur `Render`.
        let remote = CdpError::Remote {
            code: -32000,
            message: "boom".into(),
        };
        assert!(matches!(PortError::from(remote), PortError::Protocol(_)));
        assert!(matches!(
            PortError::from(CdpError::Decode("x".into())),
            PortError::Protocol(_)
        ));
    }

    #[test]
    fn base64_error_maps_to_render() {
        assert!(matches!(
            PortError::from(CdpError::Base64("x".into())),
            PortError::Render(_)
        ));
    }
}
