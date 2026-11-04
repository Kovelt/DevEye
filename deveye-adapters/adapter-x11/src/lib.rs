//! Adapter `adapter-x11` — cible X11 isolee : yeux, mains et fenetres d'un
//! serveur X **explicite** (typiquement un `Xvfb` prive), sans `xcap`, sans
//! gestionnaire de fenetres et sans jamais lire l'environnement pour choisir la
//! cible.
//!
//! | Port | Implementation |
//! |---|---|
//! | [`Renderer`] | `GetImage` de la racine rognee a la fenetre (PNG) |
//! | [`InteractionSink`] | evenements reels via XTEST (souris, molette, clavier du serveur) |
//! | [`WindowLister`] | `QueryTree` recursif, titres `_NET_WM_NAME`/`WM_NAME`/`WM_CLASS` |
//! | [`WindowGeometry`] | `TranslateCoordinates` relu a chaque appel (jamais d'origine figee) |
//! | [`WindowManager`] | `SetInputFocus` + passage au premier plan (utile sans WM) |
//!
//! `Navigate`, `HandleDialog` et `UploadFile` renvoient
//! `PortError::Unsupported` : aucun routeur applicatif ni dialogue natif a
//! piloter sur une cible X11.
//!
//! # Isolation
//!
//! XTEST injecte de VRAIS evenements d'entree : pointee sur la session de
//! l'utilisateur, l'adapter deplacerait sa souris et taperait dans ses
//! fenetres. La garde ([`guard`]) refuse donc, a la construction ou a la
//! premiere connexion : un display non local, un numero < 100, le display de la
//! session courante (`$DISPLAY`), un socket appartenant a un autre uid et tout
//! serveur annoncant l'extension `XWAYLAND`. Il n'existe aucun contournement.
//!
//! # Erreurs et repli
//!
//! Seule une coupure de transport donne `PortError::Network` (c'est la seule
//! erreur sur laquelle les facades de repli du serveur basculent) ; une
//! fenetre introuvable ou une requete refusee n'en est jamais une. La
//! connexion est ouverte au premier appel et se refait apres une coupure
//! (lectures rejouees une fois ; injections jamais rejouees).
//!
//! # Timing
//!
//! Les toolkits immediats (egui, imgui) lisent les entrees par frame : le
//! pointeur est place ~30 ms avant l'appui, un glisser interpole 16 deplacements
//! et la molette emet un cran (boutons 4/5/6/7) par tranche de 120 px.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
// Temporaire (commits 1 et 2 de la serie adapter-x11) : les consommateurs
// (capture, ports) arrivent au commit 3, qui retire cette ligne.
#![allow(dead_code)]

mod conn;
mod error;
pub mod guard;
pub mod input;
pub mod keys;
pub mod windows;

pub use crate::conn::PixelFormat;
pub use crate::error::X11Error;
pub use crate::guard::{check_display, parse_display, DisplayId, GuardError, MIN_DISPLAY_NUMBER};
pub use crate::input::{execute, lower, to_steps, Action, Emitter, Step};
pub use crate::keys::{KeyAction, Keymap};
pub use crate::windows::{pick_window, WinNode};
