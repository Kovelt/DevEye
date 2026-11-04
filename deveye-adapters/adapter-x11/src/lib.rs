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

pub mod capture;
mod conn;
mod error;
pub mod guard;
pub mod input;
pub mod keys;
pub mod windows;

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt as _, InputFocus, StackMode};

use deveye_core::{InteractionCommand, Screenshot, Size, WindowInfo, WindowRect};
use deveye_ports::{
    InteractionSink, PortError, PortResult, Renderer, WindowGeometry, WindowLister, WindowManager,
};

use crate::conn::{Session, X11Link};
use crate::error::Op;
use crate::windows::{Matcher, RESOLVE_TIMEOUT};

pub use crate::capture::{convert_to_rgba, crop_to_screen, encode_png, pad_to_window, CropRect};
pub use crate::conn::PixelFormat;
pub use crate::error::X11Error;
pub use crate::guard::{check_display, parse_display, DisplayId, GuardError, MIN_DISPLAY_NUMBER};
pub use crate::input::{execute, lower, to_steps, Action, Emitter, Step};
pub use crate::keys::{KeyAction, Keymap};
pub use crate::windows::{pick_window, WinNode};

/// Configuration de la cible.
#[derive(Clone, Debug, Default)]
pub struct X11Config {
    /// Display explicite du serveur prive (`:250`). Obligatoire, sans repli sur
    /// `$DISPLAY`.
    pub display: String,
    /// Sous-chaine du titre de la fenetre cible (`_NET_WM_NAME`, `WM_NAME`, a
    /// defaut `WM_CLASS`), insensible a la casse. `None` : tout l'ecran.
    pub window: Option<String>,
    /// Sous-chaine imposee en plus dans `WM_CLASS`.
    pub class: Option<String>,
}

struct Inner {
    link: X11Link,
    /// Critere de la fenetre par defaut.
    default: Matcher,
    /// Derniere fenetre resolue pour le critere par defaut (revalidee a chaque
    /// appel, jamais crue sur parole).
    cached: Mutex<Option<u32>>,
}

/// Cible X11 : implemente les cinq ports. `Clone` bon marche (partage la meme
/// liaison).
#[derive(Clone)]
pub struct X11Target {
    inner: Arc<Inner>,
}

impl X11Target {
    /// Valide le display (garde pre-connexion) ; ne se connecte pas encore.
    pub fn new(config: X11Config) -> Result<Self, GuardError> {
        let link = X11Link::new(&config.display)?;
        Ok(Self {
            inner: Arc::new(Inner {
                link,
                default: Matcher::new(config.window.as_deref(), config.class.as_deref()),
                cached: Mutex::new(None),
            }),
        })
    }

    /// Display valide que cible cet adapter.
    pub fn display_id(&self) -> DisplayId {
        self.inner.link.display_id()
    }

    async fn blocking<T, F>(&self, retry: bool, op: Op, f: F) -> PortResult<T>
    where
        T: Send + 'static,
        F: Fn(&Inner, &Session) -> Result<T, X11Error> + Send + 'static,
    {
        let inner = self.inner.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let shared = inner.clone();
            inner.link.run(retry, move |s| f(&shared, s))
        })
        .await;
        match joined {
            Ok(r) => r.map_err(|e| e.into_port(op)),
            Err(e) => Err(PortError::Interaction(format!(
                "tâche X11 interrompue : {e}"
            ))),
        }
    }
}

impl Inner {
    /// Resout la fenetre cible. `title` (non vide) remplace le critere par
    /// defaut. `None` : aucun critere -> toute la racine.
    fn resolve(&self, s: &Session, title: Option<&str>) -> Result<Option<u32>, X11Error> {
        let over = title.filter(|t| !t.is_empty());
        let matcher = match over {
            Some(t) => Matcher::new(Some(t), None),
            None => self.default.clone(),
        };
        if matcher.is_empty() {
            return Ok(None);
        }
        if over.is_none() {
            let cached = *self.cached.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(id) = cached {
                if windows::still_viewable(s, id)? {
                    return Ok(Some(id));
                }
            }
        }
        let id = windows::find_window(s, &matcher, RESOLVE_TIMEOUT)?;
        if over.is_none() {
            *self.cached.lock().unwrap_or_else(PoisonError::into_inner) = Some(id);
        }
        Ok(Some(id))
    }

    fn focus(&self, s: &Session, win: u32) -> Result<(), X11Error> {
        s.conn
            .set_input_focus(InputFocus::PARENT, win, x11rb::CURRENT_TIME)?;
        s.conn
            .configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
        s.conn.flush()?;
        s.conn.get_input_focus()?.reply()?;
        Ok(())
    }
}

#[async_trait]
impl Renderer for X11Target {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        self.blocking(true, Op::Render, |inner, s| {
            match inner.resolve(s, None)? {
                Some(win) => capture::capture_rect(s, &windows::rect_of(s, win)?),
                None => capture::capture_screen(s),
            }
        })
        .await
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        self.blocking(true, Op::Render, |inner, s| {
            Ok(match inner.resolve(s, None)? {
                Some(win) => {
                    let r = windows::rect_of(s, win)?;
                    Size::new(r.width, r.height)
                }
                None => Size::new(u32::from(s.screen.0), u32::from(s.screen.1)),
            })
        })
        .await
    }
}

#[async_trait]
impl InteractionSink for X11Target {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
        // Refus sans toucher au serveur ; nom de variante SEUL dans les logs
        // (le texte tape ou des chemins n'y ont pas leur place).
        input::check_supported(&command).map_err(|e| e.into_port(Op::Interaction))?;
        tracing::debug!(commande = input::command_kind(&command), "dispatch X11");
        self.blocking(false, Op::Interaction, move |inner, s| {
            let win = inner.resolve(s, None)?;
            // Origine relue A CHAQUE dispatch : la fenetre a pu bouger.
            let origin = match win {
                Some(w) => windows::origin_of(s, w)?,
                None => (0, 0),
            };
            let steps = input::to_steps(command.clone(), origin)?;
            if let (Some(w), true) = (win, input::needs_keyboard(&steps)) {
                // Sans WM le focus suit le pointeur (PointerRoot) : on le fixe
                // explicitement sur la fenetre cible avant de taper.
                inner.focus(s, w)?;
            }
            input::run_steps(s, &steps)
        })
        .await
    }
}

#[async_trait]
impl WindowLister for X11Target {
    async fn list_windows(&self) -> PortResult<Vec<WindowInfo>> {
        self.blocking(true, Op::Inspect, |_, s| {
            let nodes = windows::snapshot(s)?;
            Ok(windows::listable(&nodes)
                .into_iter()
                .map(windows::to_window_info)
                .collect())
        })
        .await
    }
}

#[async_trait]
impl WindowGeometry for X11Target {
    async fn geometry(&self, title_substring: Option<&str>) -> PortResult<WindowRect> {
        let title = title_substring.map(str::to_owned);
        self.blocking(true, Op::Inspect, move |inner, s| {
            match inner.resolve(s, title.as_deref())? {
                Some(win) => windows::rect_of(s, win),
                None => Err(X11Error::NoWindow(
                    "aucune fenêtre cible : passer un titre ou configurer --window".into(),
                )),
            }
        })
        .await
    }
}

#[async_trait]
impl WindowManager for X11Target {
    async fn activate(&self, title_substring: Option<&str>) -> PortResult<String> {
        let title = title_substring.map(str::to_owned);
        self.blocking(false, Op::Interaction, move |inner, s| {
            let win = inner.resolve(s, title.as_deref())?.ok_or_else(|| {
                X11Error::NoWindow(
                    "aucune fenêtre cible : passer un titre ou configurer --window".into(),
                )
            })?;
            inner.focus(s, win)?;
            windows::title_of(s, win)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_applies_the_guard_without_connecting() {
        for bad in ["", ":0", ":1", ":99", "localhost:250", "250"] {
            let r = X11Target::new(X11Config {
                display: bad.into(),
                ..X11Config::default()
            });
            assert!(r.is_err(), "{bad:?} aurait du etre refuse");
        }
    }

    #[tokio::test]
    async fn unsupported_commands_never_touch_the_server() {
        // Display valide mais sans serveur : une commande Unsupported doit
        // echouer en Unsupported (pas en Network).
        let t = X11Target::new(X11Config {
            display: ":259".into(),
            ..X11Config::default()
        });
        let Ok(t) = t else { return }; // :259 appartient a un autre uid ou meme $DISPLAY
        for cmd in [
            InteractionCommand::navigate("x"),
            InteractionCommand::handle_dialog(true, None),
            InteractionCommand::upload_file("i", vec![]),
        ] {
            let e = t.dispatch(cmd).await.unwrap_err();
            assert!(matches!(e, PortError::Unsupported(_)), "{e:?}");
        }
    }
}
