//! Session EI active : handshake *sender* sur le fd KWin, binding seat +
//! capabilities, suivi des devices virtuels resumed, et émission des séquences
//! d'événements (pointeur, bouton, scroll, clavier).
//!
//! API `reis` 0.7 (chemins vérifiés sur la source du crate) : voie haut niveau
//! `handshake_tokio` + `EiConvertEventStream` + [`reis::event::EiEvent`], qui
//! corrèle le keymap au device et répond aux pings automatiquement.

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use futures_util::{FutureExt, StreamExt};
use reis::{
    ei,
    event::{DeviceCapability, EiEvent},
    tokio::EiConvertEventStream,
};
use xkbcommon::xkb;

use deveye_core::MouseButton;

use crate::error::EisError;

/// Codes boutons evdev (`linux/input-event-codes.h`).
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
/// `KEY_LEFTSHIFT` evdev — pour les caractères de niveau 1 (majuscules, etc.).
const KEY_LEFTSHIFT: u32 = 42;
/// Délai d'attente d'un device pointeur resumed après le handshake.
const RESUME_TIMEOUT: Duration = Duration::from_secs(5);

/// Session EIS prête à émettre. Conserve le contexte, le flux d'événements (à
/// pomper pour rafraîchir le serial et répondre aux pings), les devices resumed
/// et le keymap résolu.
pub(crate) struct EisSession {
    /// Connexion D-Bus ayant obtenu le fd EIS : KWin lie la validité du fd à sa
    /// durée de vie, on la garde donc ouverte (jamais relue après l'init — d'où
    /// `allow(dead_code)` : c'est un garde-fou RAII).
    #[allow(dead_code)]
    dbus: zbus::Connection,
    context: ei::Context,
    connection: reis::event::Connection,
    events: EiConvertEventStream,
    /// Tous les devices virtuels resumed (KWin peut en exposer un par
    /// capability, ou un seul combiné — on les retrouve par `interface::<T>()`).
    devices: Vec<reis::event::Device>,
    keymap: Option<xkb::Keymap>,
    /// Compteur de séquence (strictement croissant par `start_emulating`).
    sequence: u32,
}

impl EisSession {
    /// Handshake *sender* sur le fd EIS, bind des capabilities, attente d'un
    /// device pointeur resumed (le clavier est capturé au passage s'il arrive).
    pub(crate) async fn connect(fd: OwnedFd, dbus: zbus::Connection) -> Result<Self, EisError> {
        let stream = UnixStream::from(fd);
        let context = ei::Context::new(stream)
            .map_err(|e| EisError::Eis(format!("ei::Context::new : {e}")))?;
        let (connection, mut events) = context
            .handshake_tokio("deveye", ei::handshake::ContextType::Sender)
            .await
            .map_err(|e| EisError::Eis(format!("handshake EIS : {e}")))?;

        let xkb_ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let mut devices: Vec<reis::event::Device> = Vec::new();
        let mut keymap: Option<xkb::Keymap> = None;

        let deadline = tokio::time::Instant::now() + RESUME_TIMEOUT;
        loop {
            let item = match tokio::time::timeout_at(deadline, events.next()).await {
                Err(_) => {
                    if devices
                        .iter()
                        .any(|d| d.interface::<ei::PointerAbsolute>().is_some())
                    {
                        break; // pointeur prêt : on démarre même sans clavier
                    }
                    return Err(EisError::Eis(
                        "aucun device pointeur EIS resumed dans le délai (5 s)".into(),
                    ));
                }
                Ok(None) => return Err(EisError::Eis("flux EIS fermé pendant l'init".into())),
                Ok(Some(r)) => r.map_err(|e| EisError::Eis(format!("flux EIS : {e}")))?,
            };

            match item {
                EiEvent::SeatAdded(evt) => {
                    evt.seat.bind_capabilities(
                        DeviceCapability::PointerAbsolute
                            | DeviceCapability::Button
                            | DeviceCapability::Scroll
                            | DeviceCapability::Keyboard,
                    );
                    context
                        .flush()
                        .map_err(|e| EisError::Eis(format!("flush bind : {e}")))?;
                }
                EiEvent::DeviceResumed(evt) => {
                    if keymap.is_none() {
                        if let Some(km) = evt.device.keymap() {
                            keymap = parse_keymap(&xkb_ctx, km);
                        }
                    }
                    devices.push(evt.device);
                    let has_pointer = devices
                        .iter()
                        .any(|d| d.interface::<ei::PointerAbsolute>().is_some());
                    let has_keyboard = devices
                        .iter()
                        .any(|d| d.interface::<ei::Keyboard>().is_some());
                    if has_pointer && has_keyboard {
                        break; // les deux mains sont prêtes
                    }
                }
                EiEvent::Disconnected(evt) => {
                    return Err(EisError::Eis(format!(
                        "EIS déconnecté pendant l'init : {:?}",
                        evt.reason
                    )));
                }
                _ => {}
            }
        }

        Ok(Self {
            dbus,
            context,
            connection,
            events,
            devices,
            keymap,
            sequence: 0,
        })
    }

    /// Draine les événements prêts sans bloquer : rafraîchit le serial suivi par
    /// `EiConvertEventStream` et laisse la couche répondre aux pings.
    fn pump(&mut self) {
        while self.events.next().now_or_never().flatten().is_some() {}
    }

    fn next_seq(&mut self) -> u32 {
        self.sequence += 1;
        self.sequence
    }

    fn pointer_index(&self) -> Result<usize, EisError> {
        self.devices
            .iter()
            .position(|d| d.interface::<ei::PointerAbsolute>().is_some())
            .ok_or_else(|| EisError::Eis("aucun device pointeur EIS disponible".into()))
    }

    fn keyboard_index(&self) -> Result<usize, EisError> {
        self.devices
            .iter()
            .position(|d| d.interface::<ei::Keyboard>().is_some())
            .ok_or_else(|| EisError::Eis("aucun device clavier EIS disponible".into()))
    }

    fn flush(&self) -> Result<(), EisError> {
        self.context
            .flush()
            .map_err(|e| EisError::Eis(format!("flush émission : {e}")))
    }

    /// Déplace le pointeur (hover) en (x, y) absolu écran.
    pub(crate) async fn hover(&mut self, abs: (f64, f64)) -> Result<(), EisError> {
        self.pump();
        let serial = self.connection.serial();
        let seq = self.next_seq();
        let device = &self.devices[self.pointer_index()?];
        let pa = device
            .interface::<ei::PointerAbsolute>()
            .ok_or_else(|| EisError::Eis("pas d'interface pointeur absolu".into()))?;
        let proto = device.device();
        proto.start_emulating(serial, seq);
        pa.motion_absolute(abs.0 as f32, abs.1 as f32);
        proto.frame(serial, monotonic_us());
        proto.stop_emulating(serial);
        self.flush()
    }

    /// Clic complet : déplacement absolu, puis press/release du bouton.
    pub(crate) async fn click(
        &mut self,
        abs: (f64, f64),
        button: MouseButton,
    ) -> Result<(), EisError> {
        self.pump();
        let serial = self.connection.serial();
        let seq = self.next_seq();
        let device = &self.devices[self.pointer_index()?];
        let pa = device
            .interface::<ei::PointerAbsolute>()
            .ok_or_else(|| EisError::Eis("pas d'interface pointeur absolu".into()))?;
        let btn = device
            .interface::<ei::Button>()
            .ok_or_else(|| EisError::Eis("pas d'interface bouton".into()))?;
        let proto = device.device();
        let code = button_code(button);
        proto.start_emulating(serial, seq);
        pa.motion_absolute(abs.0 as f32, abs.1 as f32);
        proto.frame(serial, monotonic_us());
        btn.button(code, ei::button::ButtonState::Press);
        proto.frame(serial, monotonic_us());
        btn.button(code, ei::button::ButtonState::Released);
        proto.frame(serial, monotonic_us());
        proto.stop_emulating(serial);
        self.flush()
    }

    /// Glisser-déposer : press en `from`, déplacements intermédiaires, release
    /// en `to` (bouton gauche maintenu).
    pub(crate) async fn drag(&mut self, from: (f64, f64), to: (f64, f64)) -> Result<(), EisError> {
        self.pump();
        let serial = self.connection.serial();
        let seq = self.next_seq();
        let device = &self.devices[self.pointer_index()?];
        let pa = device
            .interface::<ei::PointerAbsolute>()
            .ok_or_else(|| EisError::Eis("pas d'interface pointeur absolu".into()))?;
        let btn = device
            .interface::<ei::Button>()
            .ok_or_else(|| EisError::Eis("pas d'interface bouton".into()))?;
        let proto = device.device();
        let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
        proto.start_emulating(serial, seq);
        pa.motion_absolute(from.0 as f32, from.1 as f32);
        proto.frame(serial, monotonic_us());
        btn.button(BTN_LEFT, ei::button::ButtonState::Press);
        proto.frame(serial, monotonic_us());
        pa.motion_absolute(mid.0 as f32, mid.1 as f32);
        proto.frame(serial, monotonic_us());
        pa.motion_absolute(to.0 as f32, to.1 as f32);
        proto.frame(serial, monotonic_us());
        btn.button(BTN_LEFT, ei::button::ButtonState::Released);
        proto.frame(serial, monotonic_us());
        proto.stop_emulating(serial);
        self.flush()
    }

    /// Saisit une chaîne UTF-8 : chaque caractère est résolu en keycode via le
    /// keymap KWin (avec Shift si niveau 1).
    pub(crate) async fn type_text(&mut self, text: &str) -> Result<(), EisError> {
        self.pump();
        let serial = self.connection.serial();
        let seq = self.next_seq();
        let device = &self.devices[self.keyboard_index()?];
        let kbd = device
            .interface::<ei::Keyboard>()
            .ok_or_else(|| EisError::Eis("pas d'interface clavier".into()))?;
        let keymap = self
            .keymap
            .as_ref()
            .ok_or_else(|| EisError::Eis("keymap EIS indisponible (saisie impossible)".into()))?;
        let proto = device.device();
        proto.start_emulating(serial, seq);
        for ch in text.chars() {
            let Some((code, needs_shift)) = char_to_evdev(keymap, ch) else {
                continue; // caractère hors keymap : ignoré, pas d'échec global
            };
            if needs_shift {
                kbd.key(KEY_LEFTSHIFT, ei::keyboard::KeyState::Press);
                proto.frame(serial, monotonic_us());
            }
            kbd.key(code, ei::keyboard::KeyState::Press);
            proto.frame(serial, monotonic_us());
            kbd.key(code, ei::keyboard::KeyState::Released);
            proto.frame(serial, monotonic_us());
            if needs_shift {
                kbd.key(KEY_LEFTSHIFT, ei::keyboard::KeyState::Released);
                proto.frame(serial, monotonic_us());
            }
        }
        proto.stop_emulating(serial);
        self.flush()
    }

    /// Appuie puis relâche une touche nommée XKB (« Return », « Tab », « Escape »…).
    pub(crate) async fn key_press(&mut self, name: &str) -> Result<(), EisError> {
        self.pump();
        let serial = self.connection.serial();
        let seq = self.next_seq();
        let device = &self.devices[self.keyboard_index()?];
        let kbd = device
            .interface::<ei::Keyboard>()
            .ok_or_else(|| EisError::Eis("pas d'interface clavier".into()))?;
        let keymap = self
            .keymap
            .as_ref()
            .ok_or_else(|| EisError::Eis("keymap EIS indisponible".into()))?;
        let code = xkb_name_to_evdev(keymap, name)
            .ok_or_else(|| EisError::Eis(format!("touche XKB inconnue : {name}")))?;
        let proto = device.device();
        proto.start_emulating(serial, seq);
        kbd.key(code, ei::keyboard::KeyState::Press);
        proto.frame(serial, monotonic_us());
        kbd.key(code, ei::keyboard::KeyState::Released);
        proto.frame(serial, monotonic_us());
        proto.stop_emulating(serial);
        self.flush()
    }
}

/// Bouton DevEye -> code evdev.
fn button_code(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => BTN_LEFT,
        MouseButton::Right => BTN_RIGHT,
        MouseButton::Middle => BTN_MIDDLE,
    }
}

/// CLOCK_MONOTONIC en microsecondes — unité attendue par `ei_device.frame`.
fn monotonic_us() -> u64 {
    let ts = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    (ts.tv_sec as u64) * 1_000_000 + (ts.tv_nsec as u64) / 1_000
}

/// Parse le keymap fourni par KWin (memfd) en `xkb::Keymap`.
#[allow(unsafe_code)]
fn parse_keymap(ctx: &xkb::Context, km: &reis::event::Keymap) -> Option<xkb::Keymap> {
    // `new_from_fd` prend possession du fd (il le fermera) : on duplique pour ne
    // pas voler celui de l'événement reis.
    let fd = km.fd.try_clone().ok()?;
    // SAFETY : `fd` est un memfd valide (dupliqué de celui fourni par KWin) et
    // `km.size` sa taille exacte ; `new_from_fd` le mmap en lecture seule.
    unsafe {
        xkb::Keymap::new_from_fd(
            ctx,
            fd,
            km.size as usize,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::COMPILE_NO_FLAGS,
        )
    }
    .ok()
    .flatten()
}

/// Résout un caractère Unicode en `(keycode evdev, shift requis)` via le keymap.
fn char_to_evdev(keymap: &xkb::Keymap, ch: char) -> Option<(u32, bool)> {
    let target = xkb::Keysym::from_char(ch);
    for xkb_code in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
        let keycode = xkb::Keycode::new(xkb_code);
        for level in 0_u32..=1 {
            if keymap
                .key_get_syms_by_level(keycode, 0, level)
                .contains(&target)
            {
                return Some((xkb_code - 8, level == 1)); // XKB -> evdev
            }
        }
    }
    None
}

/// Résout un nom de touche XKB en keycode evdev.
fn xkb_name_to_evdev(keymap: &xkb::Keymap, name: &str) -> Option<u32> {
    let target = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
    if target == xkb::keysyms::KEY_NoSymbol.into() {
        return None;
    }
    for xkb_code in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
        let keycode = xkb::Keycode::new(xkb_code);
        for level in 0_u32..=1 {
            if keymap
                .key_get_syms_by_level(keycode, 0, level)
                .contains(&target)
            {
                return Some(xkb_code - 8);
            }
        }
    }
    None
}
