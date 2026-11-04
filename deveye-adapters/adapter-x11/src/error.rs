//! Erreurs internes de l'adapter et leur projection sur [`PortError`].
//!
//! Regle cle (piege F12, cf. `deveye-server/src/fallback.rs`) : seule une
//! coupure de TRANSPORT (serveur X absent, socket coupe) devient
//! `PortError::Network`, parce que les facades de repli du serveur ne basculent
//! que sur cette variante. Toute autre erreur (fenetre introuvable, requete
//! refusee par le serveur, entree invalide...) reste une erreur metier : elle ne
//! doit jamais declencher un repli silencieux vers une autre cible.

use deveye_ports::PortError;
use x11rb::errors::{ConnectError, ConnectionError, ReplyError, ReplyOrIdError};

use crate::guard::GuardError;

/// Famille d'operation, pour choisir la variante `PortError` d'une erreur
/// non-transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    /// Capture d'ecran (`PortError::Render`).
    Render,
    /// Injection d'evenements, focus (`PortError::Interaction`).
    Interaction,
    /// Enumeration / geometrie de fenetres (`PortError::Inspect`).
    Inspect,
}

#[derive(Debug, thiserror::Error)]
pub enum X11Error {
    /// Garde d'isolation : refus avant toute connexion ou juste apres.
    #[error("{0}")]
    Guard(GuardError),
    /// Le serveur X est injoignable ou la connexion est coupee.
    #[error("serveur X injoignable : {0}")]
    Transport(String),
    /// Le serveur X a repondu par une erreur de protocole.
    #[error("requete X refusee : {0}")]
    Request(String),
    /// Aucune fenetre ne correspond au critere (ou elle a disparu).
    #[error("{0}")]
    NoWindow(String),
    /// Commande valide mais non supportee par cette cible.
    #[error("{0}")]
    Unsupported(String),
    /// Entree invalide (touche inconnue, texte trop long...) : rien n'a ete emis.
    #[error("{0}")]
    Invalid(String),
}

impl X11Error {
    /// `true` pour une coupure de transport : la connexion en cache doit etre
    /// jetee pour que l'appel suivant se reconnecte.
    pub(crate) fn is_transport(&self) -> bool {
        matches!(self, X11Error::Transport(_))
    }

    /// Projection sur le type d'erreur des ports.
    pub(crate) fn into_port(self, op: Op) -> PortError {
        match self {
            X11Error::Transport(m) => PortError::Network(m),
            X11Error::Unsupported(m) => PortError::Unsupported(m),
            // Un refus de la garde d'isolation est une erreur d'usage, pas un
            // probleme reseau : on la rattache a l'operation en cours.
            other => {
                let m = other.to_string();
                match op {
                    Op::Render => PortError::Render(m),
                    Op::Interaction => PortError::Interaction(m),
                    Op::Inspect => PortError::Inspect(m),
                }
            }
        }
    }
}

impl From<ConnectError> for X11Error {
    fn from(e: ConnectError) -> Self {
        // Toute erreur de connexion initiale (serveur absent, auth refusee,
        // nom de display invalide) : le serveur n'est pas joignable.
        X11Error::Transport(e.to_string())
    }
}

impl From<ConnectionError> for X11Error {
    fn from(e: ConnectionError) -> Self {
        match e {
            // Requete trop grosse / extension manquante / erreur de parse :
            // pas une coupure, la connexion reste saine.
            ConnectionError::UnsupportedExtension
            | ConnectionError::MaximumRequestLengthExceeded
            | ConnectionError::FdPassingFailed
            | ConnectionError::ParseError(_) => X11Error::Request(e.to_string()),
            _ => X11Error::Transport(e.to_string()),
        }
    }
}

impl From<ReplyError> for X11Error {
    fn from(e: ReplyError) -> Self {
        match e {
            ReplyError::ConnectionError(c) => c.into(),
            ReplyError::X11Error(x) => X11Error::Request(format!("{x:?}")),
        }
    }
}

impl From<ReplyOrIdError> for X11Error {
    fn from(e: ReplyOrIdError) -> Self {
        match e {
            ReplyOrIdError::ConnectionError(c) => c.into(),
            ReplyOrIdError::X11Error(x) => X11Error::Request(format!("{x:?}")),
            ReplyOrIdError::IdsExhausted => X11Error::Request("identifiants X epuises".into()),
        }
    }
}

impl From<GuardError> for X11Error {
    fn from(e: GuardError) -> Self {
        X11Error::Guard(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn only_transport_cuts_map_to_network() {
        let cut: X11Error =
            ConnectionError::IoError(io::Error::from(io::ErrorKind::BrokenPipe)).into();
        assert!(cut.is_transport());
        assert!(matches!(
            cut.into_port(Op::Interaction),
            PortError::Network(_)
        ));

        let eof: X11Error = ConnectionError::UnknownError.into();
        assert!(matches!(eof.into_port(Op::Render), PortError::Network(_)));

        let refused: X11Error = ConnectionError::UnsupportedExtension.into();
        assert!(!refused.is_transport());
        assert!(matches!(
            refused.into_port(Op::Render),
            PortError::Render(_)
        ));
    }

    #[test]
    fn connect_failure_is_network() {
        let e: X11Error = ConnectError::InvalidScreen.into();
        assert!(matches!(e.into_port(Op::Inspect), PortError::Network(_)));
    }

    #[test]
    fn business_errors_never_become_network() {
        for (e, expect_unsupported) in [
            (X11Error::NoWindow("w".into()), false),
            (X11Error::Invalid("i".into()), false),
            (X11Error::Request("r".into()), false),
            (X11Error::Unsupported("u".into()), true),
        ] {
            let p = e.into_port(Op::Interaction);
            assert!(!matches!(p, PortError::Network(_)), "{p:?}");
            assert_eq!(matches!(p, PortError::Unsupported(_)), expect_unsupported);
        }
    }

    #[test]
    fn op_selects_the_port_variant() {
        assert!(matches!(
            X11Error::NoWindow("x".into()).into_port(Op::Render),
            PortError::Render(_)
        ));
        assert!(matches!(
            X11Error::NoWindow("x".into()).into_port(Op::Interaction),
            PortError::Interaction(_)
        ));
        assert!(matches!(
            X11Error::NoWindow("x".into()).into_port(Op::Inspect),
            PortError::Inspect(_)
        ));
    }
}
