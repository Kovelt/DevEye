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
/// Modificateurs evdev (`linux/input-event-codes.h`) pour `key_combo`.
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTMETA: u32 = 125;
/// Decalage de keycode entre la numerotation XKB (base 8) et evdev (base 0).
/// Voir : https://xkbcommon.org/doc/current/group__keycodes.html
const XKB_TO_EVDEV_OFFSET: u32 = 8;
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

    /// Draine les evenements prets sans bloquer : rafraichit le serial suivi
    /// par `EiConvertEventStream` et laisse la couche repondre aux pings.
    /// Retourne `Err` si une deconnexion ou une erreur de flux est detectee
    /// — la session doit alors etre invalidee par l'appelant.
    fn pump(&mut self) -> Result<(), EisError> {
        loop {
            match self.events.next().now_or_never() {
                None => break, // pas d'evenement pret
                Some(None) => {
                    tracing::warn!("pump: flux EIS ferme (deconnexion detectee)");
                    return Err(EisError::Eis(
                        "flux EIS ferme (deconnexion detectee en pump)".into(),
                    ));
                }
                Some(Some(Ok(_))) => {} // evenement consomme normalement
                Some(Some(Err(e))) => {
                    tracing::warn!("pump: erreur flux EIS (deconnexion/device retire) : {e}");
                    return Err(EisError::Eis(format!(
                        "erreur flux EIS (deconnexion/device retire) : {e}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Boilerplate commun aux methodes d'emission : pompe les evenements en
    /// attente et renvoie (serial, seq) a passer a `start_emulating`. La
    /// session est invalidee si pump detecte une deconnexion.
    fn begin_emit(&mut self) -> Result<(u32, u32), EisError> {
        self.pump()?;
        let serial = self.connection.serial();
        let seq = self.next_seq();
        Ok((serial, seq))
    }

    fn next_seq(&mut self) -> u32 {
        // wrapping_add : le debordement est volontaire et inoffensif. Apres
        // 2^32 emissions le compteur repart de 1 ; seq est purement informatif
        // cote KWin (pas cryptographique), une collision lointaine est sans
        // consequence.
        self.sequence = self.sequence.wrapping_add(1);
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

    fn scroll_index(&self) -> Result<usize, EisError> {
        self.devices
            .iter()
            .position(|d| d.interface::<ei::Scroll>().is_some())
            .ok_or_else(|| EisError::Eis("aucun device scroll EIS disponible".into()))
    }

    fn flush(&self) -> Result<(), EisError> {
        self.context
            .flush()
            .map_err(|e| EisError::Eis(format!("flush émission : {e}")))
    }

    /// Déplace le pointeur (hover) en (x, y) absolu écran.
    pub(crate) async fn hover(&mut self, abs: (f64, f64)) -> Result<(), EisError> {
        let (serial, seq) = self.begin_emit()?;
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
        let code = button_code(button)
            .ok_or_else(|| EisError::Eis(format!("bouton de souris {button:?} non géré")))?;
        let (serial, seq) = self.begin_emit()?;
        let device = &self.devices[self.pointer_index()?];
        let pa = device
            .interface::<ei::PointerAbsolute>()
            .ok_or_else(|| EisError::Eis("pas d'interface pointeur absolu".into()))?;
        let btn = device
            .interface::<ei::Button>()
            .ok_or_else(|| EisError::Eis("pas d'interface bouton".into()))?;
        let proto = device.device();
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

    /// Double-clic gauche : déplacement absolu puis deux press/release du
    /// bouton gauche, dans une seule séquence (le compositeur les corrèle en
    /// double-clic via leur proximité temporelle).
    pub(crate) async fn double_click(&mut self, abs: (f64, f64)) -> Result<(), EisError> {
        let (serial, seq) = self.begin_emit()?;
        let device = &self.devices[self.pointer_index()?];
        let pa = device
            .interface::<ei::PointerAbsolute>()
            .ok_or_else(|| EisError::Eis("pas d'interface pointeur absolu".into()))?;
        let btn = device
            .interface::<ei::Button>()
            .ok_or_else(|| EisError::Eis("pas d'interface bouton".into()))?;
        let proto = device.device();
        proto.start_emulating(serial, seq);
        pa.motion_absolute(abs.0 as f32, abs.1 as f32);
        proto.frame(serial, monotonic_us());
        for _ in 0..2 {
            btn.button(BTN_LEFT, ei::button::ButtonState::Press);
            proto.frame(serial, monotonic_us());
            btn.button(BTN_LEFT, ei::button::ButtonState::Released);
            proto.frame(serial, monotonic_us());
        }
        proto.stop_emulating(serial);
        self.flush()
    }

    /// Glisser-déposer : press en `from`, déplacements intermédiaires, release
    /// en `to` (bouton gauche maintenu).
    pub(crate) async fn drag(&mut self, from: (f64, f64), to: (f64, f64)) -> Result<(), EisError> {
        let (serial, seq) = self.begin_emit()?;
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
        let (serial, seq) = self.begin_emit()?;
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
        let (serial, seq) = self.begin_emit()?;
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

    /// Combinaison de touches : maintient les `modifiers` (Ctrl/Shift/Alt/Meta)
    /// pendant la frappe de `key`, puis les relache en ordre inverse. Vrais
    /// events OS-level — les raccourcis du toolkit/compositeur se declenchent.
    pub(crate) async fn key_combo(
        &mut self,
        modifiers: &[String],
        key: &str,
    ) -> Result<(), EisError> {
        // Resolution AVANT d'emettre quoi que ce soit : un modificateur ou une
        // touche inconnue echoue net, sans laisser un modificateur enfonce.
        let mut mod_codes: Vec<u32> = Vec::with_capacity(modifiers.len());
        for m in modifiers {
            let code = modifier_to_evdev(m)
                .ok_or_else(|| EisError::Eis(format!("modificateur inconnu : {m}")))?;
            mod_codes.push(code);
        }
        let (serial, seq) = self.begin_emit()?;
        let device = &self.devices[self.keyboard_index()?];
        let kbd = device
            .interface::<ei::Keyboard>()
            .ok_or_else(|| EisError::Eis("pas d'interface clavier".into()))?;
        let keymap = self
            .keymap
            .as_ref()
            .ok_or_else(|| EisError::Eis("keymap EIS indisponible".into()))?;
        let key_code = xkb_name_to_evdev(keymap, key)
            .ok_or_else(|| EisError::Eis(format!("touche XKB inconnue : {key}")))?;
        let proto = device.device();
        proto.start_emulating(serial, seq);
        for code in &mod_codes {
            kbd.key(*code, ei::keyboard::KeyState::Press);
            proto.frame(serial, monotonic_us());
        }
        kbd.key(key_code, ei::keyboard::KeyState::Press);
        proto.frame(serial, monotonic_us());
        kbd.key(key_code, ei::keyboard::KeyState::Released);
        proto.frame(serial, monotonic_us());
        for code in mod_codes.iter().rev() {
            kbd.key(*code, ei::keyboard::KeyState::Released);
            proto.frame(serial, monotonic_us());
        }
        proto.stop_emulating(serial);
        self.flush()
    }

    /// Defilement lisse de `(dx, dy)` pixels logiques sur le device scroll EIS,
    /// suivi d'un `scroll_stop` (fin du geste, pas d'inertie). `dy > 0` = bas.
    pub(crate) async fn scroll(&mut self, dx: f64, dy: f64) -> Result<(), EisError> {
        let (serial, seq) = self.begin_emit()?;
        let device = &self.devices[self.scroll_index()?];
        let sc = device
            .interface::<ei::Scroll>()
            .ok_or_else(|| EisError::Eis("pas d'interface scroll".into()))?;
        let proto = device.device();
        proto.start_emulating(serial, seq);
        sc.scroll(dx as f32, dy as f32);
        proto.frame(serial, monotonic_us());
        // scroll_stop(x, y, is_cancel) : signale l'arret sur chaque axe utilise.
        let stop_x = if dx != 0.0 { 1 } else { 0 };
        let stop_y = if dy != 0.0 { 1 } else { 0 };
        sc.scroll_stop(stop_x, stop_y, 0);
        proto.frame(serial, monotonic_us());
        proto.stop_emulating(serial);
        self.flush()
    }
}

/// Nom de modificateur (insensible a la casse) -> code evdev de la touche
/// gauche correspondante. Reconnait ctrl/control, shift, alt, meta/super/cmd.
fn modifier_to_evdev(name: &str) -> Option<u32> {
    match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(KEY_LEFTCTRL),
        "shift" => Some(KEY_LEFTSHIFT),
        "alt" => Some(KEY_LEFTALT),
        "meta" | "super" | "cmd" | "command" => Some(KEY_LEFTMETA),
        _ => None,
    }
}

/// Bouton DevEye -> code evdev.
/// `None` pour un bouton inconnu (`MouseButton` est `#[non_exhaustive]`).
pub(crate) fn button_code(button: MouseButton) -> Option<u32> {
    match button {
        MouseButton::Left => Some(BTN_LEFT),
        MouseButton::Right => Some(BTN_RIGHT),
        MouseButton::Middle => Some(BTN_MIDDLE),
        _ => None,
    }
}

/// CLOCK_MONOTONIC en microsecondes — unité attendue par `ei_device.frame`.
fn monotonic_us() -> u64 {
    let ts = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    (ts.tv_sec as u64) * 1_000_000 + (ts.tv_nsec as u64) / 1_000
}

/// Parse le keymap fourni par KWin (memfd) en `xkb::Keymap`.
/// Chaque echec intermediaire est trace via `tracing::warn!` pour faciliter
/// le diagnostic d'un keymap casse (fd clone, mmap, parse xkb).
#[allow(unsafe_code)]
fn parse_keymap(ctx: &xkb::Context, km: &reis::event::Keymap) -> Option<xkb::Keymap> {
    // `new_from_fd` prend possession du fd (il le fermera) : on duplique pour ne
    // pas voler celui de l'événement reis.
    let fd = match km.fd.try_clone() {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("parse_keymap: try_clone fd EIS echoue : {e}");
            return None;
        }
    };
    // SAFETY : `fd` est un memfd valide (dupliqué de celui fourni par KWin) et
    // `km.size` sa taille exacte ; `new_from_fd` le mmap en lecture seule.
    let result = unsafe {
        xkb::Keymap::new_from_fd(
            ctx,
            fd,
            km.size as usize,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::COMPILE_NO_FLAGS,
        )
    };
    match result {
        Err(e) => {
            tracing::warn!("parse_keymap: xkb::Keymap::new_from_fd erreur : {e}");
            None
        }
        Ok(None) => {
            tracing::warn!(
                "parse_keymap: xkb::Keymap::new_from_fd -> None (keymap vide ou format invalide)"
            );
            None
        }
        Ok(Some(keymap)) => Some(keymap),
    }
}

/// Balaye tout le keymap (keycodes XKB, niveaux 0 et 1) a la recherche du
/// premier keycode dont un des deux niveaux produit `target`. Retourne le
/// keycode evdev (offset `XKB_TO_EVDEV_OFFSET` deja retire) et le niveau
/// trouve (0 ou 1). Factorise le balayage double-boucle commun a
/// `char_to_evdev` et `xkb_name_to_evdev`.
fn scan_keymap(keymap: &xkb::Keymap, target: xkb::Keysym) -> Option<(u32, u32)> {
    for xkb_code in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
        let keycode = xkb::Keycode::new(xkb_code);
        for level in 0_u32..=1 {
            if keymap
                .key_get_syms_by_level(keycode, 0, level)
                .contains(&target)
            {
                match xkb_code.checked_sub(XKB_TO_EVDEV_OFFSET) {
                    Some(evdev) => return Some((evdev, level)),
                    None => {
                        tracing::warn!(
                            "scan_keymap: xkb_code {} < XKB_TO_EVDEV_OFFSET pour {:?}, ignore",
                            xkb_code,
                            target
                        );
                        // Poursuit la recherche sur les keycodes suivants.
                        continue;
                    }
                }
            }
        }
    }
    None
}

/// Résout un caractère Unicode en `(keycode evdev, shift requis)` via le keymap.
fn char_to_evdev(keymap: &xkb::Keymap, ch: char) -> Option<(u32, bool)> {
    let target = xkb::Keysym::from_char(ch);
    scan_keymap(keymap, target).map(|(evdev, level)| (evdev, level == 1))
}

/// Résout un nom de touche XKB en keycode evdev.
fn xkb_name_to_evdev(keymap: &xkb::Keymap, name: &str) -> Option<u32> {
    let target = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
    if target == xkb::keysyms::KEY_NoSymbol.into() {
        return None;
    }
    scan_keymap(keymap, target).map(|(evdev, _level)| evdev)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- button_code : mapping MouseButton -> code evdev ----------------------
    //
    // Verrouille les valeurs attendues par KWin EIS.  Toute regression sur les
    // constantes BTN_* ou sur les branches du match sera detectee ici sans avoir
    // besoin d'une session KWin active.

    /// Les constantes BTN_* doivent correspondre aux valeurs de
    /// linux/input-event-codes.h (inchangees depuis le kernel 2.6).
    #[test]
    fn btn_constants_match_linux_input_event_codes() {
        assert_eq!(BTN_LEFT, 0x110);
        assert_eq!(BTN_RIGHT, 0x111);
        assert_eq!(BTN_MIDDLE, 0x112);
    }

    #[test]
    fn button_code_left_maps_to_btn_left() {
        assert_eq!(button_code(MouseButton::Left), Some(BTN_LEFT));
    }

    #[test]
    fn button_code_right_maps_to_btn_right() {
        assert_eq!(button_code(MouseButton::Right), Some(BTN_RIGHT));
    }

    #[test]
    fn button_code_middle_maps_to_btn_middle() {
        assert_eq!(button_code(MouseButton::Middle), Some(BTN_MIDDLE));
    }

    // --- modifier_to_evdev : mapping nom -> code evdev (insensible a la casse) ---

    #[test]
    fn modifier_evdev_constants_match_linux_input_event_codes() {
        assert_eq!(KEY_LEFTCTRL, 29);
        assert_eq!(KEY_LEFTSHIFT, 42);
        assert_eq!(KEY_LEFTALT, 56);
        assert_eq!(KEY_LEFTMETA, 125);
    }

    #[test]
    fn modifier_to_evdev_resolves_known_aliases() {
        assert_eq!(modifier_to_evdev("Ctrl"), Some(KEY_LEFTCTRL));
        assert_eq!(modifier_to_evdev("control"), Some(KEY_LEFTCTRL));
        assert_eq!(modifier_to_evdev("SHIFT"), Some(KEY_LEFTSHIFT));
        assert_eq!(modifier_to_evdev("alt"), Some(KEY_LEFTALT));
        assert_eq!(modifier_to_evdev("Super"), Some(KEY_LEFTMETA));
        assert_eq!(modifier_to_evdev("cmd"), Some(KEY_LEFTMETA));
        assert_eq!(modifier_to_evdev("fn"), None);
    }

    // --- char_to_evdev / xkb_name_to_evdev : resolution via un vrai keymap ---
    //
    // Un keymap "us" se compile localement (xkb::Context::new +
    // Keymap::new_from_names), sans session KWin : xkeyboard-config est
    // installe sur cette machine (paquet `libxkbcommon` + data xkb).

    /// Compile un keymap XKB "us" minimal pour les tests, sans passer par KWin.
    fn us_keymap() -> xkb::Keymap {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_names(&ctx, "", "", "us", "", None, xkb::COMPILE_NO_FLAGS)
            .expect("compilation du keymap XKB \"us\" (xkeyboard-config installe)")
    }

    /// (a) 'a' est en niveau 0 (pas de Shift) sur KEY_A = 30.
    #[test]
    fn char_to_evdev_lowercase_a_maps_to_key_a_without_shift() {
        let keymap = us_keymap();
        assert_eq!(char_to_evdev(&keymap, 'a'), Some((30, false)));
    }

    /// (b) 'A' est la meme touche KEY_A, mais en niveau 1 (Shift requis).
    #[test]
    fn char_to_evdev_uppercase_a_maps_to_key_a_with_shift() {
        let keymap = us_keymap();
        assert_eq!(char_to_evdev(&keymap, 'A'), Some((30, true)));
    }

    /// (c) Noms de touches XKB usuels -> keycodes evdev standards
    /// (linux/input-event-codes.h : KEY_ENTER = 28, KEY_ESC = 1).
    #[test]
    fn xkb_name_to_evdev_resolves_known_key_names() {
        let keymap = us_keymap();
        assert_eq!(xkb_name_to_evdev(&keymap, "Return"), Some(28));
        assert_eq!(xkb_name_to_evdev(&keymap, "Escape"), Some(1));
    }

    /// (d) Un nom de touche XKB inconnu ne resout aucun keysym : `None`.
    #[test]
    fn xkb_name_to_evdev_unknown_name_returns_none() {
        let keymap = us_keymap();
        assert_eq!(xkb_name_to_evdev(&keymap, "PasUneVraieTouche"), None);
    }
}
