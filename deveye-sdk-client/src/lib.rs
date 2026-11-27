//! Client HTTP partage des adapters DevEye communiquant avec le SDK opt-in.
//!
//! Centralise l'URL par defaut et les constantes de timeout afin d'eviter la
//! repetition dans `adapter-dioxus-signals`, `adapter-logs-tracing` et
//! `adapter-commands-http`.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use deveye_ports::{PortError, PortResult};
use serde::{Deserialize, Serialize};

/// URL par defaut ou le SDK opt-in expose ses endpoints devtools.
/// On prefere 127.0.0.1 a `localhost` : sous Linux moderne `localhost` se
/// resout souvent en ::1 (IPv6) alors que le SDK bind sur 127.0.0.1 seulement.
pub const DEFAULT_SDK_URL: &str = "http://127.0.0.1:31337";

/// Délai de connexion TCP appliqué à chaque `reqwest::Client` SDK.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Délai global (connexion + transfert) appliqué à chaque `reqwest::Client` SDK.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// En-tête HTTP portant la version du wire protocol dans chaque réponse du SDK.
pub const PROTOCOL_VERSION_HEADER: &str = "x-deveye-protocol-version";

/// Version du wire protocol (`MAJEURE.MINEURE`) que ce client implémente.
pub const SUPPORTED_PROTOCOL_VERSION: &str = "1.0";

const SUPPORTED_MAJOR: u32 = 1;
const SUPPORTED_MINOR: u32 = 0;

/// Construit un `reqwest::Client` preconfigure avec les timeouts DevEye.
///
/// Utilise le backend TLS rustls (active dans la feature workspace `rustls-tls`).
///
/// L'`expect` est un invariant d'initialisation : la construction n'échoue que
/// si le backend TLS ne peut pas s'initialiser, ce qui ne peut pas arriver dans
/// ce workspace (rustls est compile en dur). Un repli silencieux sur un client
/// sans timeouts masquerait une app figée, d'où le choix de ne pas le faire ;
/// changer la signature en `Result` serait cassant (API stable 1.0).
#[allow(clippy::expect_used)]
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("client reqwest (TLS rustls indisponible)")
}

/// Verdict de comparaison entre la version annoncée par le SDK et celle du client.
#[derive(Debug, PartialEq, Eq)]
enum Assessment {
    Compatible,
    /// Compatible en principe, mais à signaler une fois (absent, mineure
    /// differente, valeur illisible).
    Warn(String),
    Incompatible(String),
}

fn assess(header: Option<&str>) -> Assessment {
    let Some(raw) = header.map(str::trim).filter(|v| !v.is_empty()) else {
        return Assessment::Warn(format!(
            "le SDK n'annonce pas de version de protocole (en-tête {PROTOCOL_VERSION_HEADER} \
             absent) ; ce client implémente la {SUPPORTED_PROTOCOL_VERSION}. SDK trop ancien \
             ou serveur tiers ?"
        ));
    };
    let (major, minor) = match raw.split_once('.') {
        Some((major, minor)) => (major.parse::<u32>(), minor.parse::<u32>().ok()),
        None => (raw.parse::<u32>(), None),
    };
    let Ok(major) = major else {
        return Assessment::Warn(format!(
            "version de protocole illisible ({raw:?}) ; ce client implémente la \
             {SUPPORTED_PROTOCOL_VERSION}"
        ));
    };
    if major != SUPPORTED_MAJOR {
        return Assessment::Incompatible(format!(
            "le SDK parle le protocole {raw} (majeure {major}) ; ce client implémente la \
             majeure {SUPPORTED_MAJOR}. Aligner les versions de deveye-sdk et de deveye-server."
        ));
    }
    if minor == Some(SUPPORTED_MINOR) {
        Assessment::Compatible
    } else {
        Assessment::Warn(format!(
            "le SDK annonce le protocole {raw}, ce client implémente la \
             {SUPPORTED_PROTOCOL_VERSION} : mineure differente, fonctions additives \
             possiblement absentes ou ignorees"
        ))
    }
}

/// Émet `message` en `tracing::warn!` si `flag` n'a jamais été levé ; renvoie
/// vrai quand l'avertissement a été émis.
fn warn_once(flag: &AtomicBool, message: &str) -> bool {
    if flag.swap(true, Ordering::Relaxed) {
        return false;
    }
    tracing::warn!("deveye-sdk-client : {message}");
    true
}

static VERSION_WARNED: AtomicBool = AtomicBool::new(false);

/// Vérifie la valeur de l'en-tête `X-DevEye-Protocol-Version` (ou son absence).
///
/// - majeure identique et mineure identique : `Ok` silencieux ;
/// - en-tête absent, illisible ou mineure differente : `Ok` et un avertissement
///   `tracing` émis **une seule fois** par processus ;
/// - majeure differente : [`PortError::Protocol`] explicite.
pub fn check_protocol_header(header: Option<&str>) -> PortResult<()> {
    match assess(header) {
        Assessment::Compatible => Ok(()),
        Assessment::Warn(message) => {
            warn_once(&VERSION_WARNED, &message);
            Ok(())
        }
        Assessment::Incompatible(message) => Err(PortError::Protocol(message)),
    }
}

/// [`check_protocol_header`] appliquée à une réponse HTTP du SDK. À appeler par
/// tout adapter après `send()`, avant de décoder le corps.
pub fn check_protocol_version(response: &reqwest::Response) -> PortResult<()> {
    check_protocol_header(
        response
            .headers()
            .get(PROTOCOL_VERSION_HEADER)
            .and_then(|value| value.to_str().ok()),
    )
}

/// Identité et capacités d'un SDK, telles que servies par `GET /devtools/info`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SdkInfo {
    /// Version du crate `deveye-sdk` (ou de l'implémentation tierce).
    pub sdk_version: String,
    /// Version du wire protocol, `MAJEURE.MINEURE`.
    pub protocol_version: String,
    /// Capacités actives : `state`, `commands`, `info`, et `logs` / `eval`
    /// quand la couche tracing / la pompe d'évaluation sont installées.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Lit `GET /devtools/info` sur le SDK à `base_url`.
///
/// Erreurs : [`PortError::Network`] si le SDK est injoignable ou répond en
/// erreur, [`PortError::Protocol`] si la majeure diffère, si la route n'existe
/// pas (SDK antérieur a la 1.0) ou si le corps est illisible.
pub async fn fetch_info(http: &reqwest::Client, base_url: &str) -> PortResult<SdkInfo> {
    let url = format!("{}/devtools/info", base_url.trim_end_matches('/'));
    let response = http
        .get(&url)
        .send()
        .await
        .map_err(|e| PortError::Network(format!("GET {url} : {e}")))?;
    check_protocol_version(&response)?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(PortError::Protocol(format!(
            "GET {url} : route absente, SDK antérieur au protocole 1.0 ; mettre deveye-sdk a jour"
        )));
    }
    if !status.is_success() {
        return Err(PortError::Network(format!("GET {url} : statut {status}")));
    }
    response
        .json()
        .await
        .map_err(|e| PortError::Protocol(format!("décodage JSON de l'info {url} : {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_version_is_compatible() {
        assert_eq!(assess(Some("1.0")), Assessment::Compatible);
        assert_eq!(assess(Some(" 1.0 ")), Assessment::Compatible);
    }

    #[test]
    fn missing_or_unreadable_header_warns() {
        assert!(matches!(assess(None), Assessment::Warn(_)));
        assert!(matches!(assess(Some("")), Assessment::Warn(_)));
        assert!(matches!(assess(Some("abc")), Assessment::Warn(_)));
        // Ancien SDK : « 1 » sans mineure.
        assert!(matches!(assess(Some("1")), Assessment::Warn(_)));
    }

    #[test]
    fn different_minor_warns_and_different_major_is_incompatible() {
        assert!(matches!(assess(Some("1.3")), Assessment::Warn(_)));
        assert!(matches!(assess(Some("2.0")), Assessment::Incompatible(_)));
        assert!(matches!(assess(Some("0.9")), Assessment::Incompatible(_)));
    }

    #[test]
    fn check_header_maps_verdicts_to_results() {
        assert!(check_protocol_header(Some("1.0")).is_ok());
        assert!(check_protocol_header(None).is_ok());
        assert!(matches!(
            check_protocol_header(Some("2.0")),
            Err(PortError::Protocol(_))
        ));
    }

    #[test]
    fn warning_is_emitted_only_once() {
        let flag = AtomicBool::new(false);
        assert!(warn_once(&flag, "premier"));
        assert!(!warn_once(&flag, "second"));
        assert!(!warn_once(&flag, "troisieme"));
    }
}
