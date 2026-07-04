//! Adapter DevEye : capture de fenêtre via `org.kde.KWin.ScreenShot2` (D-Bus).
//!
//! Cible : **KDE Plasma sous Wayland natif** (`XDG_SESSION_TYPE=wayland`,
//! `XDG_CURRENT_DESKTOP` contenant `KDE`). Capture pixel-perfect d'une fenêtre
//! par son UUID interne KWin, sans passer par XWayland ni CDP. Hors KDE/Wayland
//! (X11, GNOME, XFCE), utiliser `adapter-screencap-xcap`.
//!
//! Stratégie (la sélection de fenêtre par titre n'existe pas sur ScreenShot2) :
//! 1. un **script KWin** éphémère (`org.kde.kwin.Scripting`) itère
//!    `workspace.windowList()`, trouve la fenêtre dont le `caption` contient la
//!    sous-chaîne cible, et renvoie son `internalId` (UUID) par `callDBus` vers
//!    une petite interface servie par cet adapter ;
//! 2. l'UUID alimente `ScreenShot2.CaptureWindow(uuid, {}, pipe)` ; KWin écrit
//!    les pixels bruts (ARGB32 little-endian = BGRA) dans le pipe ;
//! 3. on convertit BGRA → RGBA et on encode en PNG.
//!
//! Prérequis système (dev-only) : KWin doit tourner avec
//! `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1` dans son environnement de session,
//! sinon `CaptureWindow` renvoie `NoAuthorized`. Voir `docs/roadmap/M1-*`.
//!
//! Aucune dépendance à une app cible : la fenêtre est désignée par sous-chaîne
//! de titre, fournie à la construction.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

mod error;
mod kwin_bridge;
mod proxy;
mod renderer;
mod resolve_cache;
mod window_geometry;
mod window_lister;
mod window_manager;

pub use error::KwinScreencapError;
pub use renderer::KwinRenderer;
pub use window_geometry::KwinWindowGeometry;
pub use window_lister::KwinWindowLister;
pub use window_manager::KwinWindowManager;
