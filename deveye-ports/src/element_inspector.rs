//! Port `ElementInspector` (feature `unstable-elements`, hors promesse semver,
//! ADR-0069 et ADR-0072).

use async_trait::async_trait;
use thiserror::Error;

use deveye_core::elements::{ElementLocation, ElementSnapshot, SnapshotQuery};

use crate::errors::PortError;

/// Pourquoi un instantané ou une relocalisation échoue.
///
/// Distincte de [`PortError`] : un agent réagit autrement à une référence périmée
/// (relancer l'instantané) qu'à une panne de liaison.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ElementError {
    /// La référence a existé mais son élément a disparu. `candidates` liste les
    /// références vivantes de même rôle et de même nom : pas de rebind silencieux,
    /// l'agent choisit.
    #[error("référence périmée : {reference}")]
    Stale {
        reference: String,
        candidates: Vec<String>,
    },
    /// La référence n'a jamais été émise ou n'a pas une forme connue.
    #[error("référence inconnue : {0}")]
    UnknownRef(String),
    /// Plusieurs éléments portent cette référence (identifiant dupliqué côté app) :
    /// agir sur le premier serait un pari, l'agent doit corriger l'app ou passer
    /// par des coordonnées.
    #[error("référence ambiguë : {reference} désigne {count} éléments")]
    Ambiguous { reference: String, count: usize },
    /// Un autre élément recouvre le centre ; `by` est sa référence si elle est connue.
    #[error("élément couvert")]
    Obscured { by: Option<String> },
    /// L'arbre servi par l'app est trop ancien pour qu'on agisse dessus : l'app ne
    /// republie plus (gelée, ou rendu à la demande qui ne s'est pas déclenché), les
    /// rects pourraient être périmés. `age_ms` est l'âge mesuré.
    #[error("arbre périmé : publié il y a {age_ms} ms")]
    StaleTree { age_ms: u64 },
    /// Cette cible ne sait pas produire d'éléments ; `hint` dit comment faire.
    #[error("éléments non pris en charge : {hint}")]
    Unsupported { hint: String },
    /// Panne du backend (liaison, protocole, route).
    #[error(transparent)]
    Port(#[from] PortError),
}

pub type ElementResult<T> = Result<T, ElementError>;

/// Désigner un élément par une référence stable.
///
/// `snapshot` produit l'arbre compact ; `locate` **remesure** l'élément au moment
/// de l'action (le rect d'un instantané est indicatif) et dit où cliquer. Aucune
/// action n'est faite ici : l'agent passe par `InteractionSink` avec les
/// coordonnées rendues.
#[async_trait]
pub trait ElementInspector: Send + Sync {
    async fn snapshot(&self, query: &SnapshotQuery) -> ElementResult<ElementSnapshot>;
    async fn locate(&self, reference: &str) -> ElementResult<ElementLocation>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use deveye_core::elements::ElementNode;

    struct Fixed;

    #[async_trait]
    impl ElementInspector for Fixed {
        async fn snapshot(&self, _query: &SnapshotQuery) -> ElementResult<ElementSnapshot> {
            Ok(ElementSnapshot::new(
                0,
                "widgets",
                vec![ElementNode::new("w:ok", "button")],
            ))
        }

        async fn locate(&self, reference: &str) -> ElementResult<ElementLocation> {
            match reference {
                "w:ok" => Ok(ElementLocation::new(reference, [0, 0, 10, 10])),
                other => Err(ElementError::Stale {
                    reference: other.to_string(),
                    candidates: vec!["w:ok".to_string()],
                }),
            }
        }
    }

    /// Les futures de `Fixed` sont prêtes au premier poll : pas besoin de runtime.
    fn ready<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("future non prête"),
        }
    }

    #[test]
    fn trait_is_object_safe_and_reports_stale_candidates() {
        let inspector: Box<dyn ElementInspector> = Box::new(Fixed);
        let snap = ready(inspector.snapshot(&SnapshotQuery::new())).unwrap();
        assert_eq!(snap.count(), 1);
        let err = ready(inspector.locate("w:gone")).unwrap_err();
        assert!(
            matches!(err, ElementError::Stale { ref candidates, .. } if candidates == &["w:ok"])
        );
    }

    #[test]
    fn ambiguous_reports_the_count() {
        let err = ElementError::Ambiguous {
            reference: "w:save".to_string(),
            count: 2,
        };
        assert_eq!(
            err.to_string(),
            "référence ambiguë : w:save désigne 2 éléments"
        );
    }

    #[test]
    fn port_errors_convert() {
        let err: ElementError = PortError::Network("x".into()).into();
        assert!(matches!(err, ElementError::Port(PortError::Network(_))));
    }
}
