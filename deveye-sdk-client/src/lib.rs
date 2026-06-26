//! Client HTTP partage des adapters DevEye communiquant avec le SDK opt-in.
//!
//! Centralise l'URL par defaut et les constantes de timeout afin d'eviter la
//! repetition dans `adapter-dioxus-signals`, `adapter-logs-tracing` et
//! `adapter-commands-http`.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use std::time::Duration;

/// URL par defaut ou le SDK opt-in expose ses endpoints devtools.
/// On prefere 127.0.0.1 a `localhost` : sous Linux moderne `localhost` se
/// resout souvent en ::1 (IPv6) alors que le SDK bind sur 127.0.0.1 seulement.
pub const DEFAULT_SDK_URL: &str = "http://127.0.0.1:31337";

/// Delai de connexion TCP applique a chaque `reqwest::Client` SDK.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Delai global (connexion + transfert) applique a chaque `reqwest::Client` SDK.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Construit un `reqwest::Client` preconfigure avec les timeouts DevEye.
///
/// Utilise le backend TLS rustls (active dans la feature workspace `rustls-tls`).
/// Panique si rustls n'est pas disponible — ce qui ne peut pas arriver dans ce
/// workspace.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("client reqwest (TLS rustls indisponible)")
}
