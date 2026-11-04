//! Liaison paresseuse vers un serveur X explicite.
//!
//! - **Jamais d'environnement** : la cible est le display passe a
//!   [`X11Link::new`] ; `$DISPLAY` n'est lu que par la garde, pour REFUSER un
//!   display identique a la session courante (cf. [`crate::guard`]).
//! - **Paresseuse** : la connexion est ouverte au premier appel, ce qui permet
//!   de demarrer le serveur MCP avant Xvfb et avant l'application cible.
//! - **Reconnexion** : une coupure de transport jette la connexion en cache ;
//!   les operations de lecture sont rejouees une fois sur une connexion neuve,
//!   les injections d'entree non (pas de rejeu d'un geste partiel).
//! - **Serialisation** : un verrou unique ordonne toutes les operations (une
//!   sequence de souris/clavier ne doit pas s'entrelacer avec une autre).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, ConnectionExt as _, Window as XWindow};
use x11rb::rust_connection::RustConnection;

use crate::error::X11Error;
use crate::guard::{self, DisplayId, GuardError, X11Probe};

/// Description du format de pixels de la racine (serveur + visuel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelFormat {
    /// Profondeur du visuel (24 ou 32 supportees).
    pub depth: u8,
    pub bits_per_pixel: u8,
    pub scanline_pad: u8,
    /// Ordre des octets d'un pixel dans l'image (`ImageByteOrder`).
    pub msb_first: bool,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
}

/// Atomes internes utiles aux proprietes de fenetre.
pub(crate) struct Atoms {
    pub net_wm_name: Atom,
    pub utf8_string: Atom,
}

/// Une connexion X etablie, avec les constantes du serveur lues une fois.
pub(crate) struct Session {
    pub conn: RustConnection,
    pub root: XWindow,
    /// Taille de l'ecran (racine) en pixels.
    pub screen: (u16, u16),
    pub pixel: PixelFormat,
    pub atoms: Atoms,
    pub has_xtest: bool,
    pub min_keycode: u8,
    pub max_keycode: u8,
}

impl X11Probe for Session {
    fn has_extension(&self, name: &str) -> Result<bool, X11Error> {
        Ok(self.conn.query_extension(name.as_bytes())?.reply()?.present)
    }
}

impl Session {
    fn open(display: &str) -> Result<Self, X11Error> {
        let (conn, screen_num) = RustConnection::connect(Some(display))?;
        let setup = conn.setup();
        let screen = setup
            .roots
            .get(screen_num)
            .ok_or_else(|| X11Error::Request(format!("écran {screen_num} absent du setup")))?;
        let root = screen.root;
        let root_depth = screen.root_depth;
        let visual = screen
            .allowed_depths
            .iter()
            .flat_map(|d| d.visuals.iter())
            .find(|v| v.visual_id == screen.root_visual)
            .ok_or_else(|| X11Error::Request("visuel racine introuvable".into()))?;
        let format = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == root_depth)
            .ok_or_else(|| {
                X11Error::Request(format!(
                    "aucun format de pixmap pour la profondeur {root_depth}"
                ))
            })?;
        let pixel = PixelFormat {
            depth: root_depth,
            bits_per_pixel: format.bits_per_pixel,
            scanline_pad: format.scanline_pad,
            msb_first: setup.image_byte_order == x11rb::protocol::xproto::ImageOrder::MSB_FIRST,
            red_mask: visual.red_mask,
            green_mask: visual.green_mask,
            blue_mask: visual.blue_mask,
        };
        let screen_size = (screen.width_in_pixels, screen.height_in_pixels);
        let (min_keycode, max_keycode) = (setup.min_keycode, setup.max_keycode);

        let net = conn.intern_atom(false, b"_NET_WM_NAME")?;
        let utf8 = conn.intern_atom(false, b"UTF8_STRING")?;
        let atoms = Atoms {
            net_wm_name: net.reply()?.atom,
            utf8_string: utf8.reply()?.atom,
        };

        let mut session = Session {
            conn,
            root,
            screen: screen_size,
            pixel,
            atoms,
            has_xtest: false,
            min_keycode,
            max_keycode,
        };
        // Garde post-connexion : jamais un serveur XWayland.
        guard::check_probe(&session)?;
        session.has_xtest = session.has_extension("XTEST")?;
        Ok(session)
    }

    /// XTEST est requis pour toute injection d'entree.
    pub(crate) fn require_xtest(&self) -> Result<(), X11Error> {
        if self.has_xtest {
            Ok(())
        } else {
            Err(X11Error::Unsupported(
                "le serveur X n'expose pas l'extension XTEST (injection d'entrée impossible)"
                    .into(),
            ))
        }
    }
}

/// Liaison paresseuse, partageable (`Arc<X11Link>`).
pub struct X11Link {
    display: String,
    id: DisplayId,
    /// Connexion en cache ; le verrou ordonne aussi toutes les operations.
    slot: Mutex<Option<Arc<Session>>>,
}

impl X11Link {
    /// Valide le display (garde pre-connexion) sans se connecter.
    pub fn new(display: &str) -> Result<Self, GuardError> {
        let id = guard::vet_display(display)?;
        Ok(Self {
            display: display.to_string(),
            id,
            slot: Mutex::new(None),
        })
    }

    /// Display valide, forme normalisee `:N.S`.
    pub fn display_id(&self) -> DisplayId {
        self.id
    }

    fn lock(&self) -> MutexGuard<'_, Option<Arc<Session>>> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Execute `f` sur la session (connexion ouverte au besoin) sous le verrou
    /// d'ordre. Une coupure de transport jette la connexion ; si `retry`, `f`
    /// est rejoue UNE fois sur une connexion neuve (operations idempotentes).
    pub(crate) fn run<T>(
        &self,
        retry: bool,
        f: impl Fn(&Session) -> Result<T, X11Error>,
    ) -> Result<T, X11Error> {
        let mut slot = self.lock();
        let mut attempts_left = if retry { 2 } else { 1 };
        loop {
            attempts_left -= 1;
            let result = (|| {
                if slot.is_none() {
                    *slot = Some(Arc::new(Session::open(&self.display)?));
                }
                let session = slot.as_ref().expect("session ouverte ci-dessus");
                f(session)
            })();
            match result {
                Err(e) if e.is_transport() => {
                    *slot = None;
                    if attempts_left == 0 {
                        return Err(e);
                    }
                }
                other => return other,
            }
        }
    }
}
