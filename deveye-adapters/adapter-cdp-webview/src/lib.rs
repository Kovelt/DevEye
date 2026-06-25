//! Adapter DevEye pilotant un webview Chromium-based via Chrome DevTools
//! Protocol (CDP).
//!
//! Cible principale : le webview `wry`/`WebView2` qu'embarque Dioxus desktop
//! quand on l'a lancé avec
//! `dioxus::desktop::Config::with_windows_browser_args("--remote-debugging-port=9222")`.
//! Marche aussi avec n'importe quel autre webview Chromium-based exposant
//! CDP sur HTTP (Tauri en mode debug, Edge, etc.).
//!
//! Implémentations de ports fournies :
//! - [`deveye_ports::Renderer`] (screenshot + viewport_size) ;
//! - [`deveye_ports::AppInspector`] (`dom_snapshot` ; `state_snapshot` renvoie
//!   un dump vide — le state passe par le SDK opt-in côté app, pas par CDP) ;
//! - [`deveye_ports::InteractionSink`] (Click, Type, KeyPress, Navigate via
//!   le routeur JS exposé par le SDK).

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

mod client;
mod error;
mod inspector;
mod interaction;
mod js_evaluator;
mod renderer;

pub use client::{CdpClient, CdpTarget, TargetFilter, DEFAULT_HTTP_URL};
pub use error::CdpError;
