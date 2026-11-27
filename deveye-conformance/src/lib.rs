//! Suite de conformité **boîte noire** du wire protocol DevEye.
//!
//! Elle ne connaît de l'implémentation testée que son URL : elle émet des
//! requêtes HTTP et compare les réponses à `docs/wire-protocol.md`. Le SDK Rust
//! (`deveye-sdk`), un shim dans un autre langage ou toute implémentation tierce
//! se valident donc de la même façon, par le binaire `deveye-conformance --url`.
//!
//! Les vérifications dépendent de l'état de l'implémentation (un buffer de logs
//! vide ne prouve rien sur l'ordre, une app sans commande rien sur le tri) :
//! quand une exigence n'est pas observable, la vérification est `Skip` avec la
//! raison, jamais `Pass` par défaut. [`Report::is_fully_checked`] dit si tout a
//! été observé.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

mod checks;
mod client;
mod report;

pub use report::{CheckResult, Outcome, Report};

/// Paramètres d'une exécution.
#[derive(Clone, Debug)]
pub struct Options {
    /// URL de base de l'implémentation : `http://127.0.0.1:31337`.
    pub url: String,
    /// Expression dont l'évaluation ne répond jamais (pompe muette) : active la
    /// vérification du `504`, qui dure 10 s.
    pub slow_expression: Option<String>,
    /// Expression dont la pompe abandonne la requête : active la vérification du `500`.
    pub abandon_expression: Option<String>,
    /// Commande sans effet de bord à invoquer pour vérifier le `200`.
    pub invoke_route: Option<String>,
}

impl Options {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            slow_expression: None,
            abandon_expression: None,
            invoke_route: None,
        }
    }
}

/// Raisons pour lesquelles la suite ne peut pas produire de rapport.
#[derive(Debug, thiserror::Error)]
pub enum ConformanceError {
    #[error("URL invalide : {0}")]
    InvalidUrl(String),
    #[error("implémentation injoignable à {url} : {reason}")]
    Unreachable { url: String, reason: String },
    #[error("erreur interne de la suite : {0}")]
    Internal(String),
}

/// Exécute toute la suite contre `options.url`.
///
/// # Errors
/// URL invalide, ou implémentation injoignable (rien à valider).
pub async fn run(options: &Options) -> Result<Report, ConformanceError> {
    let target = client::Target::new(&options.url)?;
    // Une implémentation injoignable n'est pas « non conforme » : on le dit à part.
    if let Err(reason) = target.get("/devtools/info").await {
        return Err(ConformanceError::Unreachable {
            url: options.url.clone(),
            reason,
        });
    }
    Ok(checks::run_all(&target, options).await)
}
