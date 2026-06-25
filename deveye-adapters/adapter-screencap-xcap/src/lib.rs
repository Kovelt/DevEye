//! Adapter DevEye fallback : capture une fenêtre OS via la crate `xcap`.
//!
//! Utilité : couvrir le besoin minimal (« voir l'app ») même quand CDP n'est
//! pas disponible — macOS WKWebView, apps legacy MAUI, jeux, n'importe quoi
//! d'affiché à l'écran. Pas d'interactivité (clics, saisie) : c'est de
//! l'observation passive. Pour l'interactivité, voir `adapter-cdp-webview`.
//!
//! L'implémentation cherche la fenêtre par sous-chaîne du titre (insensible
//! à la casse) ; si aucune ne matche, [`Renderer::screenshot`] renvoie
//! `PortError::Unsupported`.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

mod error;
mod renderer;

pub use error::ScreencapError;
pub use renderer::{MonitorRenderer, XcapRenderer};
