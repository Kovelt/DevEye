//! Adapter `adapter-interaction-kwin-eis` — mains OS-level sous Wayland natif.
//!
//! Injecte des evenements d'entree dans n'importe quelle fenetre Wayland en
//! passant par le protocole EIS de KWin (`org.kde.KWin.EIS.RemoteDesktop`),
//! sans XWayland ni shim dans l'app cible. Seule voie d'interaction pour les
//! cibles sans WebView ni SDK — par exemple une app Uno desktop (rendu Skia).
//!
//! # Flux
//!
//! 1. Appel D-Bus `connectToEIS(capabilities) -> (fd, serial)` sur
//!    `org.kde.KWin.EIS.RemoteDesktop` : KWin renvoie un file descriptor EIS.
//! 2. Un client EI (`reis`, pur Rust) fait le handshake *sender* sur ce fd,
//!    lie le seat + les capabilities (pointeur absolu, bouton, scroll, clavier)
//!    et attend qu'un device virtuel soit `resumed`.
//! 3. A chaque `dispatch`, l'`InteractionCommand` est traduite en sequence
//!    d'evenements EI (`start_emulating` -> motion/button/scroll/key ->
//!    `frame` -> `stop_emulating`).
//!
//! # Securite
//!
//! L'acces EIS est accorde par KWin via D-Bus de **session** (jamais le bus
//! systeme) : il est limite a la session active de l'utilisateur courant, sans
//! elevation de privilege, sans acces reseau, sans bind de port.
//!
//! # Variantes non supportees
//!
//! `Navigate` et `Resize` supposent un routeur applicatif ou un protocole
//! WebView : impossibles OS-level, elles renvoient `PortError::Unsupported`.
//!
//! # Coordonnees
//!
//! EIS attend des coordonnees ecran **absolues**. La conversion depuis une
//! `Position` relative au viewport utilise l'origine de la fenetre cible
//! (`window_origin`), fournie a la composition par `deveye-server` (donnee de
//! M1, jamais par dependance inter-adapter — regle hexagonale).

// `deny` (et non `forbid`) : un unique `unsafe` localisé est requis pour mmap
// le keymap fourni par KWin (`xkb::Keymap::new_from_fd`), avec un `#[allow]`
// documenté dans `session.rs`. Tout le reste du crate est sans unsafe.
#![deny(unsafe_code)]
#![deny(rust_2018_idioms)]

mod connection;
mod dispatch;
mod error;
mod session;

pub use connection::KwinEisInteraction;
