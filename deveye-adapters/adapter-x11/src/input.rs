//! Mains : traduction `InteractionCommand` -> pas elementaires -> evenements
//! XTEST.
//!
//! Trois couches, les deux premieres pures et testees sans serveur X :
//!
//! 1. [`to_steps`] : commande -> [`Step`]s (souris en coordonnees racine
//!    absolues, clavier symbolique). Valide tout ce qui peut l'etre.
//! 2. [`lower`] : `Step`s + keymap -> [`Action`]s elementaires. Toute touche ou
//!    tout modificateur inconnu echoue ICI, avant la moindre emission.
//! 3. [`execute`] : rejoue les `Action`s sur un [`Emitter`] en suivant ce qui
//!    est enfonce ; sur erreur (ou a la fin) tout est relache en ordre inverse
//!    et les keycodes remappes sont restaures : aucune touche, aucun bouton ni
//!    aucun modificateur ne reste colle.
//!
//! Choix de timing (egui/baseview traitent les evenements par frame) : le
//! pointeur est positionne ~30 ms AVANT l'appui, un glisser interpole 16 pas
//! espaces de ~16 ms (egui exige des mouvements intermediaires), une molette
//! envoie un cran (boutons 4/5/6/7) par tranche de 120 px demandes.

use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt as _, BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, KEY_PRESS_EVENT,
    KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;

use deveye_core::{InteractionCommand, MouseButton, Position};

use crate::conn::Session;
use crate::error::X11Error;
use crate::keys::{self, KeyAction, Keymap};

/// Delai entre le placement du pointeur et l'appui (une frame egui).
const SETTLE: Duration = Duration::from_millis(30);
/// Intervalle entre deux deplacements d'un glisser (~1 frame a 60 Hz).
const DRAG_FRAME: Duration = Duration::from_millis(16);
/// Nombre de deplacements interpoles d'un glisser.
pub const DRAG_STEPS: i64 = 16;
/// Delai entre les deux clics d'un double-clic (< seuil usuel de 250 ms).
const DOUBLE_GAP: Duration = Duration::from_millis(60);
/// Intervalle entre deux crans de molette.
const WHEEL_GAP: Duration = Duration::from_millis(8);
/// Pixels demandes par cran de molette (convention des navigateurs).
pub const PIXELS_PER_NOTCH: u32 = 120;
/// Plafond de crans par appel (un `dy` aberrant ne bloque pas le serveur).
pub const MAX_NOTCHES: u32 = 100;

/// Boutons X11.
const BTN_LEFT: u8 = 1;
const BTN_MIDDLE: u8 = 2;
const BTN_RIGHT: u8 = 3;
const WHEEL_UP: u8 = 4;
const WHEEL_DOWN: u8 = 5;
const WHEEL_LEFT: u8 = 6;
const WHEEL_RIGHT: u8 = 7;

/// Pas haut niveau (clavier encore symbolique).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Deplacement absolu du pointeur (coordonnees racine).
    Motion {
        x: i16,
        y: i16,
    },
    Button {
        button: u8,
        press: bool,
    },
    Pause(Duration),
    Type(String),
    Key(String),
    Combo {
        modifiers: Vec<String>,
        key: String,
    },
}

/// Operation elementaire executable sans autre decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Motion(i16, i16),
    Button { button: u8, press: bool },
    Key(KeyAction),
    Pause(Duration),
}

fn clamp_i16(v: i64) -> i16 {
    v.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

/// Position viewport -> racine absolue, sans debordement (`saturating_add`) et
/// bornee a la plage du protocole (i16).
fn to_abs(p: Position, origin: (i32, i32)) -> (i16, i16) {
    (
        clamp_i16(i64::from(origin.0.saturating_add(p.x))),
        clamp_i16(i64::from(origin.1.saturating_add(p.y))),
    )
}

/// `None` pour un bouton inconnu (`MouseButton` est `#[non_exhaustive]`).
fn button_number(b: MouseButton) -> Option<u8> {
    match b {
        MouseButton::Left => Some(BTN_LEFT),
        MouseButton::Middle => Some(BTN_MIDDLE),
        MouseButton::Right => Some(BTN_RIGHT),
        _ => None,
    }
}

/// Nom court de la variante, sans ses champs (le texte tape ou les chemins
/// n'ont rien a faire dans les journaux).
pub fn command_kind(command: &InteractionCommand) -> &'static str {
    match command {
        InteractionCommand::Click { .. } => "click",
        InteractionCommand::Hover { .. } => "hover",
        InteractionCommand::Type { .. } => "type",
        InteractionCommand::KeyPress { .. } => "key-press",
        InteractionCommand::KeyCombo { .. } => "key-combo",
        InteractionCommand::Navigate { .. } => "navigate",
        InteractionCommand::DragDrop { .. } => "drag-drop",
        InteractionCommand::Scroll { .. } => "scroll",
        InteractionCommand::DoubleClick { .. } => "double-click",
        InteractionCommand::HandleDialog { .. } => "handle-dialog",
        InteractionCommand::UploadFile { .. } => "upload-file",
        // `InteractionCommand` est `#[non_exhaustive]`.
        _ => "inconnue",
    }
}

/// Refuse sans toucher au serveur les commandes que cette cible ne sait pas
/// executer (aucune application a router, aucun dialogue natif a piloter).
pub fn check_supported(command: &InteractionCommand) -> Result<(), X11Error> {
    let unsupported = |what: &str| {
        Err(X11Error::Unsupported(format!(
            "{what} n'a pas d'équivalent au niveau OS (pas de WebView ni de routeur \
             applicatif sur une cible X11) ; utiliser une cible CDP ou SDK"
        )))
    };
    match command {
        InteractionCommand::Navigate { .. } => unsupported("navigate"),
        InteractionCommand::HandleDialog { .. } => unsupported("handle_dialog"),
        InteractionCommand::UploadFile { .. } => unsupported("upload_file"),
        _ => Ok(()),
    }
}

fn button_click(button: u8) -> [Step; 3] {
    [
        Step::Button {
            button,
            press: true,
        },
        Step::Pause(SETTLE),
        Step::Button {
            button,
            press: false,
        },
    ]
}

fn notches(delta: i32) -> u32 {
    if delta == 0 {
        return 0;
    }
    delta
        .unsigned_abs()
        .div_ceil(PIXELS_PER_NOTCH)
        .clamp(1, MAX_NOTCHES)
}

/// Traduit une commande en pas ; `origin` = origine absolue de la fenetre
/// cible (les positions de la commande sont relatives a la fenetre).
pub fn to_steps(command: InteractionCommand, origin: (i32, i32)) -> Result<Vec<Step>, X11Error> {
    check_supported(&command)?;
    let motion = |p: (i16, i16)| Step::Motion { x: p.0, y: p.1 };
    let mut steps = Vec::new();
    match command {
        InteractionCommand::Click { position, button } => {
            steps.push(motion(to_abs(position, origin)));
            steps.push(Step::Pause(SETTLE));
            let number = button_number(button).ok_or_else(|| {
                X11Error::Unsupported(format!("bouton de souris {button:?} non géré en X11"))
            })?;
            steps.extend(button_click(number));
        }
        InteractionCommand::Hover { position } => {
            steps.push(motion(to_abs(position, origin)));
            steps.push(Step::Pause(SETTLE));
        }
        InteractionCommand::DoubleClick { position } => {
            steps.push(motion(to_abs(position, origin)));
            steps.push(Step::Pause(SETTLE));
            steps.extend(button_click(BTN_LEFT));
            steps.push(Step::Pause(DOUBLE_GAP));
            steps.extend(button_click(BTN_LEFT));
        }
        InteractionCommand::DragDrop { from, to } => {
            let (fx, fy) = to_abs(from, origin);
            let (tx, ty) = to_abs(to, origin);
            steps.push(Step::Motion { x: fx, y: fy });
            steps.push(Step::Pause(SETTLE));
            steps.push(Step::Button {
                button: BTN_LEFT,
                press: true,
            });
            steps.push(Step::Pause(SETTLE));
            for i in 1..=DRAG_STEPS {
                let lerp = |a: i16, b: i16| {
                    let (a, b) = (i64::from(a), i64::from(b));
                    clamp_i16(a + (b - a) * i / DRAG_STEPS)
                };
                steps.push(Step::Motion {
                    x: lerp(fx, tx),
                    y: lerp(fy, ty),
                });
                steps.push(Step::Pause(DRAG_FRAME));
            }
            steps.push(Step::Button {
                button: BTN_LEFT,
                press: false,
            });
        }
        InteractionCommand::Scroll { position, dx, dy } => {
            steps.push(motion(to_abs(position, origin)));
            steps.push(Step::Pause(SETTLE));
            let mut wheel = |button: u8, count: u32| {
                for _ in 0..count {
                    steps.push(Step::Button {
                        button,
                        press: true,
                    });
                    steps.push(Step::Button {
                        button,
                        press: false,
                    });
                    steps.push(Step::Pause(WHEEL_GAP));
                }
            };
            wheel(if dy > 0 { WHEEL_DOWN } else { WHEEL_UP }, notches(dy));
            wheel(if dx > 0 { WHEEL_RIGHT } else { WHEEL_LEFT }, notches(dx));
        }
        InteractionCommand::Type { text } => steps.push(Step::Type(text)),
        InteractionCommand::KeyPress { key } => steps.push(Step::Key(key)),
        InteractionCommand::KeyCombo { modifiers, key } => {
            steps.push(Step::Combo { modifiers, key })
        }
        // Navigate, HandleDialog et UploadFile sont deja refuses par
        // `check_supported` ; toute commande future sans traduction X11 l'est ici.
        other => {
            return Err(X11Error::Unsupported(format!(
                "commande {} non gérée en X11",
                command_kind(&other)
            )))
        }
    }
    Ok(steps)
}

/// Resout les pas clavier contre le keymap : tout echec survient avant la
/// premiere emission.
pub fn lower(keymap: &Keymap, steps: &[Step]) -> Result<Vec<Action>, X11Error> {
    let mut out = Vec::with_capacity(steps.len());
    let from_keys = |ks: Vec<KeyAction>, out: &mut Vec<Action>| {
        out.extend(ks.into_iter().map(|k| match k {
            KeyAction::Pause(d) => Action::Pause(d),
            other => Action::Key(other),
        }));
    };
    for step in steps {
        match step {
            Step::Motion { x, y } => out.push(Action::Motion(*x, *y)),
            Step::Button { button, press } => out.push(Action::Button {
                button: *button,
                press: *press,
            }),
            Step::Pause(d) => out.push(Action::Pause(*d)),
            Step::Type(text) => from_keys(keys::plan_type(keymap, text)?, &mut out),
            Step::Key(key) => from_keys(keys::plan_key(keymap, key)?, &mut out),
            Step::Combo { modifiers, key } => {
                from_keys(keys::plan_combo(keymap, modifiers, key)?, &mut out)
            }
        }
    }
    Ok(out)
}

/// Cible d'emission. `XtestEmitter` pour un vrai serveur ; un faux dans les
/// tests pour verifier l'ordre, la propagation d'erreur et le nettoyage.
pub trait Emitter {
    fn motion(&mut self, x: i16, y: i16) -> Result<(), X11Error>;
    fn button(&mut self, button: u8, press: bool) -> Result<(), X11Error>;
    fn key(&mut self, keycode: u8, press: bool) -> Result<(), X11Error>;
    /// `keysym == 0` : restaure le keycode a « aucun symbole ».
    fn remap(&mut self, keycode: u8, keysym: u32) -> Result<(), X11Error>;
    /// Garantit que tout ce qui precede est traite par le serveur.
    fn sync(&mut self) -> Result<(), X11Error>;
    /// Pause reelle (l'emetteur vide d'abord ses tampons d'ecriture).
    fn pause(&mut self, d: Duration);
}

/// Rejoue `actions`. Ce qui est enfonce est suivi ; sur erreur comme en fin de
/// sequence, tout est relache en ordre inverse (touches, puis boutons) et les
/// keycodes remappes sont restaures, en ignorant les erreurs de nettoyage.
pub fn execute(em: &mut dyn Emitter, actions: &[Action]) -> Result<(), X11Error> {
    let mut held_keys: Vec<u8> = Vec::new();
    let mut held_buttons: Vec<u8> = Vec::new();
    let mut remapped: Vec<u8> = Vec::new();

    let result = (|| -> Result<(), X11Error> {
        for action in actions {
            match action {
                Action::Motion(x, y) => em.motion(*x, *y)?,
                Action::Button { button, press } => {
                    if *press {
                        // Marque AVANT l'emission : si elle echoue a moitie, on
                        // relachera quand meme.
                        held_buttons.push(*button);
                        em.button(*button, true)?;
                    } else {
                        em.button(*button, false)?;
                        if let Some(i) = held_buttons.iter().rposition(|b| b == button) {
                            held_buttons.remove(i);
                        }
                    }
                }
                Action::Key(KeyAction::Press(code)) => {
                    held_keys.push(*code);
                    em.key(*code, true)?;
                }
                Action::Key(KeyAction::Release(code)) => {
                    em.key(*code, false)?;
                    if let Some(i) = held_keys.iter().rposition(|c| c == code) {
                        held_keys.remove(i);
                    }
                }
                Action::Key(KeyAction::Remap { keycode, keysym }) => {
                    if *keysym != 0 {
                        remapped.push(*keycode);
                    }
                    em.remap(*keycode, *keysym)?;
                    // Un remap restaure ne doit plus etre re-restaure.
                    if *keysym == 0 {
                        remapped.retain(|c| c != keycode);
                    }
                }
                Action::Key(KeyAction::Pause(d)) | Action::Pause(d) => em.pause(*d),
            }
        }
        em.sync()
    })();

    // Garde « release all » : meme chemin pour la fin normale et l'erreur.
    for code in held_keys.iter().rev() {
        let _ = em.key(*code, false);
    }
    for button in held_buttons.iter().rev() {
        let _ = em.button(*button, false);
    }
    for code in remapped.iter().rev() {
        let _ = em.remap(*code, 0);
    }
    if !held_keys.is_empty() || !held_buttons.is_empty() || !remapped.is_empty() {
        let _ = em.sync();
    }
    result
}

/// Emetteur reel : requetes XTEST `FakeInput` sur la racine.
pub(crate) struct XtestEmitter<'a> {
    pub session: &'a Session,
    pub keysyms_per_keycode: u8,
}

impl XtestEmitter<'_> {
    fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) -> Result<(), X11Error> {
        self.session
            .conn
            .xtest_fake_input(kind, detail, 0, self.session.root, x, y, 0)?;
        Ok(())
    }
}

impl Emitter for XtestEmitter<'_> {
    fn motion(&mut self, x: i16, y: i16) -> Result<(), X11Error> {
        // detail = 0 : deplacement ABSOLU.
        self.fake(MOTION_NOTIFY_EVENT, 0, x, y)
    }

    fn button(&mut self, button: u8, press: bool) -> Result<(), X11Error> {
        let kind = if press {
            BUTTON_PRESS_EVENT
        } else {
            BUTTON_RELEASE_EVENT
        };
        self.fake(kind, button, 0, 0)
    }

    fn key(&mut self, keycode: u8, press: bool) -> Result<(), X11Error> {
        let kind = if press {
            KEY_PRESS_EVENT
        } else {
            KEY_RELEASE_EVENT
        };
        self.fake(kind, keycode, 0, 0)
    }

    fn remap(&mut self, keycode: u8, keysym: u32) -> Result<(), X11Error> {
        let per = self.keysyms_per_keycode.max(1);
        let syms = vec![keysym; usize::from(per)];
        self.session
            .conn
            .change_keyboard_mapping(1, keycode, per, &syms)?;
        Ok(())
    }

    fn sync(&mut self) -> Result<(), X11Error> {
        self.session.conn.flush()?;
        // Aller-retour : le serveur a traite toutes les requetes precedentes.
        self.session.conn.get_input_focus()?.reply()?;
        Ok(())
    }

    fn pause(&mut self, d: Duration) {
        // Sans vidage, les requetes resteraient dans le tampon pendant la pause.
        let _ = self.session.conn.flush();
        std::thread::sleep(d);
    }
}

/// Lit le keymap courant du serveur.
pub(crate) fn fetch_keymap(session: &Session) -> Result<Keymap, X11Error> {
    let (min, max) = (session.min_keycode, session.max_keycode);
    let count = max - min + 1;
    let reply = session.conn.get_keyboard_mapping(min, count)?.reply()?;
    Ok(Keymap::new(min, reply.keysyms_per_keycode, reply.keysyms))
}

/// Execute une liste de pas sur le serveur : XTEST requis, keymap lu une fois,
/// resolution complete avant la premiere emission.
pub(crate) fn run_steps(session: &Session, steps: &[Step]) -> Result<(), X11Error> {
    session.require_xtest()?;
    let keymap = fetch_keymap(session)?;
    let actions = lower(&keymap, steps)?;
    let mut emitter = XtestEmitter {
        session,
        keysyms_per_keycode: keymap.keysyms_per_keycode(),
    };
    execute(&mut emitter, &actions)
}

/// `true` si les pas contiennent du clavier (le focus clavier est alors
/// ajuste avant l'emission).
pub fn needs_keyboard(steps: &[Step]) -> bool {
    steps
        .iter()
        .any(|s| matches!(s, Step::Type(_) | Step::Key(_) | Step::Combo { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: (i32, i32) = (100, 50);

    fn pos(x: i32, y: i32) -> Position {
        Position::new(x, y)
    }

    fn motions(steps: &[Step]) -> Vec<(i16, i16)> {
        steps
            .iter()
            .filter_map(|s| match s {
                Step::Motion { x, y } => Some((*x, *y)),
                _ => None,
            })
            .collect()
    }

    fn buttons(steps: &[Step]) -> Vec<(u8, bool)> {
        steps
            .iter()
            .filter_map(|s| match s {
                Step::Button { button, press } => Some((*button, *press)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn click_is_motion_pause_press_pause_release() {
        let steps = to_steps(
            InteractionCommand::Click {
                position: pos(10, 20),
                button: MouseButton::Right,
            },
            ORIGIN,
        )
        .unwrap();
        assert_eq!(
            steps,
            vec![
                Step::Motion { x: 110, y: 70 },
                Step::Pause(SETTLE),
                Step::Button {
                    button: 3,
                    press: true
                },
                Step::Pause(SETTLE),
                Step::Button {
                    button: 3,
                    press: false
                },
            ]
        );
    }

    #[test]
    fn button_numbers_follow_x11() {
        for (b, n) in [
            (MouseButton::Left, 1),
            (MouseButton::Middle, 2),
            (MouseButton::Right, 3),
        ] {
            let s = to_steps(
                InteractionCommand::Click {
                    position: pos(0, 0),
                    button: b,
                },
                (0, 0),
            )
            .unwrap();
            assert_eq!(buttons(&s), vec![(n, true), (n, false)]);
        }
    }

    #[test]
    fn hover_moves_and_settles_without_buttons() {
        let s = to_steps(InteractionCommand::hover(pos(1, 2)), (0, 0)).unwrap();
        assert_eq!(motions(&s), vec![(1, 2)]);
        assert!(buttons(&s).is_empty());
        assert!(matches!(s.last(), Some(Step::Pause(_))));
    }

    #[test]
    fn double_click_is_two_left_clicks() {
        let s = to_steps(InteractionCommand::double_click(pos(5, 5)), (0, 0)).unwrap();
        assert_eq!(
            buttons(&s),
            vec![(1, true), (1, false), (1, true), (1, false)]
        );
        assert_eq!(motions(&s).len(), 1);
    }

    #[test]
    fn drag_interpolates_between_endpoints() {
        let s = to_steps(
            InteractionCommand::drag_drop(pos(0, 0), pos(160, -320)),
            ORIGIN,
        )
        .unwrap();
        let m = motions(&s);
        // 1 placement + DRAG_STEPS deplacements, le dernier exactement a l'arrivee.
        assert_eq!(m.len(), 1 + DRAG_STEPS as usize);
        assert!(m.len() > 10);
        assert_eq!(m[0], (100, 50));
        assert_eq!(*m.last().unwrap(), (260, -270));
        // Monotone : x croit, y decroit, jamais de saut unique.
        assert!(m.windows(2).all(|w| w[1].0 >= w[0].0 && w[1].1 <= w[0].1));
        assert_eq!(m[1], (110, 30));
        // L'appui precede le premier mouvement interpole, le relachement le suit.
        let idx = |pred: &dyn Fn(&Step) -> bool| s.iter().position(pred).unwrap();
        let press = idx(&|x| matches!(x, Step::Button { press: true, .. }));
        let first_move = idx(&|x| matches!(x, Step::Motion { x: 110, .. }));
        let release = idx(&|x| matches!(x, Step::Button { press: false, .. }));
        assert!(press < first_move && first_move < release);
    }

    #[test]
    fn drag_to_the_same_point_still_emits_steps() {
        let s = to_steps(InteractionCommand::drag_drop(pos(3, 3), pos(3, 3)), (0, 0)).unwrap();
        assert!(motions(&s).iter().all(|&p| p == (3, 3)));
        assert_eq!(buttons(&s), vec![(1, true), (1, false)]);
    }

    #[test]
    fn scroll_uses_wheel_buttons_with_direction() {
        let down = to_steps(InteractionCommand::scroll(pos(0, 0), 0, 240), (0, 0)).unwrap();
        assert_eq!(
            buttons(&down),
            vec![(5, true), (5, false), (5, true), (5, false)]
        );
        let up = to_steps(InteractionCommand::scroll(pos(0, 0), 0, -1), (0, 0)).unwrap();
        assert_eq!(buttons(&up), vec![(4, true), (4, false)]);
        let right = to_steps(InteractionCommand::scroll(pos(0, 0), 120, 0), (0, 0)).unwrap();
        assert_eq!(buttons(&right), vec![(7, true), (7, false)]);
        let left = to_steps(InteractionCommand::scroll(pos(0, 0), -121, 0), (0, 0)).unwrap();
        assert_eq!(buttons(&left).len(), 4);
        assert_eq!(buttons(&left)[0].0, 6);
    }

    #[test]
    fn scroll_zero_delta_only_moves_the_pointer() {
        let s = to_steps(InteractionCommand::scroll(pos(7, 8), 0, 0), (0, 0)).unwrap();
        assert!(buttons(&s).is_empty());
        assert_eq!(motions(&s), vec![(7, 8)]);
    }

    #[test]
    fn scroll_is_capped() {
        let s = to_steps(InteractionCommand::scroll(pos(0, 0), 0, i32::MAX), (0, 0)).unwrap();
        assert_eq!(buttons(&s).len(), 2 * MAX_NOTCHES as usize);
        let s = to_steps(InteractionCommand::scroll(pos(0, 0), 0, i32::MIN), (0, 0)).unwrap();
        assert_eq!(buttons(&s).len(), 2 * MAX_NOTCHES as usize);
        assert_eq!(buttons(&s)[0].0, 4);
    }

    #[test]
    fn extreme_coordinates_saturate_without_panic() {
        for origin in [(i32::MAX, i32::MAX), (i32::MIN, i32::MIN), (-5, 7)] {
            for p in [pos(i32::MAX, i32::MIN), pos(i32::MIN, i32::MAX), pos(0, 0)] {
                let s = to_steps(InteractionCommand::click(p), origin).unwrap();
                assert_eq!(motions(&s).len(), 1);
                let d = to_steps(
                    InteractionCommand::drag_drop(p, pos(i32::MAX, i32::MAX)),
                    origin,
                )
                .unwrap();
                assert_eq!(motions(&d).len(), 1 + DRAG_STEPS as usize);
            }
        }
        let s = to_steps(
            InteractionCommand::click(pos(i32::MAX, i32::MIN)),
            (i32::MAX, i32::MIN),
        )
        .unwrap();
        assert_eq!(motions(&s), vec![(i16::MAX, i16::MIN)]);
    }

    #[test]
    fn negative_origin_is_applied() {
        let s = to_steps(InteractionCommand::click(pos(10, 10)), (-30, -40)).unwrap();
        assert_eq!(motions(&s), vec![(-20, -30)]);
    }

    #[test]
    fn unsupported_commands_are_refused_up_front() {
        for cmd in [
            InteractionCommand::navigate("x"),
            InteractionCommand::handle_dialog(true, None),
            InteractionCommand::upload_file("input", vec![]),
        ] {
            let e = to_steps(cmd.clone(), (0, 0)).unwrap_err();
            assert!(matches!(e, X11Error::Unsupported(_)), "{cmd:?}");
            assert!(check_supported(&cmd).is_err());
            // Message generique : ne cite aucune autre cible.
            assert!(!e.to_string().contains("kwin"));
        }
    }

    #[test]
    fn keyboard_commands_become_symbolic_steps() {
        assert_eq!(
            to_steps(InteractionCommand::type_text("é"), (0, 0)).unwrap(),
            vec![Step::Type("é".into())]
        );
        assert_eq!(
            to_steps(InteractionCommand::key("Escape"), (0, 0)).unwrap(),
            vec![Step::Key("Escape".into())]
        );
        let s = to_steps(
            InteractionCommand::key_combo(vec!["ctrl".into()], "a"),
            (0, 0),
        )
        .unwrap();
        assert!(needs_keyboard(&s));
        assert!(!needs_keyboard(&[Step::Pause(SETTLE)]));
    }

    #[test]
    fn command_kind_hides_payloads() {
        assert_eq!(
            command_kind(&InteractionCommand::type_text("secret")),
            "type"
        );
        assert_eq!(command_kind(&InteractionCommand::navigate("x")), "navigate");
    }

    // ---- lower / execute -------------------------------------------------

    fn keymap() -> Keymap {
        // keycode 8 : a ; 9 : Shift_L ; 10 : Control_L ; reste libre.
        let mut syms = vec![0u32; 4 * 30];
        syms[0] = 0x61;
        syms[4] = keys::XK_SHIFT_L;
        syms[8] = keys::XK_CONTROL_L;
        Keymap::new(8, 4, syms)
    }

    #[derive(Debug, PartialEq, Eq, Clone)]
    enum Ev {
        Motion(i16, i16),
        Button(u8, bool),
        Key(u8, bool),
        Remap(u8, u32),
        Sync,
        Pause,
    }

    #[derive(Default)]
    struct Fake {
        log: Vec<Ev>,
        /// Echoue a la N-ieme emission (hors nettoyage) si defini.
        fail_at: Option<usize>,
        emitted: usize,
    }

    impl Fake {
        fn gate(&mut self) -> Result<(), X11Error> {
            self.emitted += 1;
            if self.fail_at == Some(self.emitted) {
                return Err(X11Error::Transport("coupe".into()));
            }
            Ok(())
        }
    }

    impl Emitter for Fake {
        fn motion(&mut self, x: i16, y: i16) -> Result<(), X11Error> {
            self.gate()?;
            self.log.push(Ev::Motion(x, y));
            Ok(())
        }
        fn button(&mut self, b: u8, p: bool) -> Result<(), X11Error> {
            self.gate()?;
            self.log.push(Ev::Button(b, p));
            Ok(())
        }
        fn key(&mut self, c: u8, p: bool) -> Result<(), X11Error> {
            self.gate()?;
            self.log.push(Ev::Key(c, p));
            Ok(())
        }
        fn remap(&mut self, c: u8, s: u32) -> Result<(), X11Error> {
            self.gate()?;
            self.log.push(Ev::Remap(c, s));
            Ok(())
        }
        fn sync(&mut self) -> Result<(), X11Error> {
            self.log.push(Ev::Sync);
            Ok(())
        }
        fn pause(&mut self, _d: Duration) {
            self.log.push(Ev::Pause);
        }
    }

    fn run(steps: &[Step], fake: &mut Fake) -> Result<(), X11Error> {
        let actions = lower(&keymap(), steps)?;
        execute(fake, &actions)
    }

    /// Pas de nettoyage observable : l'etat final est « rien d'enfonce ».
    fn held(log: &[Ev]) -> (Vec<u8>, Vec<u8>) {
        let (mut keys, mut btns) = (vec![], vec![]);
        for e in log {
            match e {
                Ev::Key(c, true) => keys.push(*c),
                Ev::Key(c, false) => keys.retain(|k| k != c),
                Ev::Button(b, true) => btns.push(*b),
                Ev::Button(b, false) => btns.retain(|x| x != b),
                _ => {}
            }
        }
        (keys, btns)
    }

    #[test]
    fn unknown_key_in_a_sequence_emits_nothing() {
        let mut fake = Fake::default();
        let steps = [
            Step::Motion { x: 1, y: 1 },
            Step::Combo {
                modifiers: vec!["ctrl".into(), "hyper".into()],
                key: "a".into(),
            },
        ];
        assert!(run(&steps, &mut fake).is_err());
        assert!(
            fake.log.is_empty(),
            "rien ne doit etre emis : {:?}",
            fake.log
        );
    }

    #[test]
    fn error_mid_combo_releases_every_modifier() {
        // ctrl + shift + a : echec apres l'appui des deux modificateurs.
        let steps = [Step::Combo {
            modifiers: vec!["ctrl".into(), "shift".into()],
            key: "a".into(),
        }];
        for fail_at in 1..=6 {
            let mut fake = Fake {
                fail_at: Some(fail_at),
                ..Fake::default()
            };
            let r = run(&steps, &mut fake);
            assert!(r.is_err(), "fail_at={fail_at}");
            let (k, b) = held(&fake.log);
            assert!(
                k.is_empty() && b.is_empty(),
                "fail_at={fail_at} : {:?}",
                fake.log
            );
        }
    }

    #[test]
    fn error_mid_drag_releases_the_button() {
        let steps = to_steps(
            InteractionCommand::drag_drop(pos(0, 0), pos(50, 50)),
            (0, 0),
        )
        .unwrap();
        for fail_at in [2, 5, 10, 17] {
            let mut fake = Fake {
                fail_at: Some(fail_at),
                ..Fake::default()
            };
            assert!(run(&steps, &mut fake).is_err());
            assert!(held(&fake.log).1.is_empty(), "fail_at={fail_at}");
        }
    }

    #[test]
    fn remap_is_restored_even_when_the_press_fails() {
        // Caractere absent du keymap -> Remap, Press, Release, Remap(0).
        let steps = [Step::Type("☃".into())];
        let mut ok = Fake::default();
        run(&steps, &mut ok).unwrap();
        let remaps: Vec<&Ev> = ok
            .log
            .iter()
            .filter(|e| matches!(e, Ev::Remap(..)))
            .collect();
        assert_eq!(remaps.len(), 2);
        assert!(matches!(remaps[1], Ev::Remap(_, 0)));

        // Echec a l'appui (2e emission) : le keycode est quand meme restaure.
        let mut bad = Fake {
            fail_at: Some(2),
            ..Fake::default()
        };
        assert!(run(&steps, &mut bad).is_err());
        assert!(matches!(bad.log.last(), Some(Ev::Sync)));
        assert!(bad.log.iter().any(|e| matches!(e, Ev::Remap(_, 0))));
    }

    #[test]
    fn normal_sequence_ends_with_a_sync_and_nothing_held() {
        let steps = to_steps(InteractionCommand::click(pos(3, 4)), (0, 0)).unwrap();
        let mut fake = Fake::default();
        run(&steps, &mut fake).unwrap();
        assert_eq!(fake.log.last(), Some(&Ev::Sync));
        assert_eq!(held(&fake.log), (vec![], vec![]));
        assert_eq!(fake.log[0], Ev::Motion(3, 4));
    }

    #[test]
    fn type_emits_key_events_in_text_order() {
        let mut fake = Fake::default();
        run(&[Step::Type("aa".into())], &mut fake).unwrap();
        let keys: Vec<&Ev> = fake
            .log
            .iter()
            .filter(|e| matches!(e, Ev::Key(..)))
            .collect();
        assert_eq!(keys.len(), 4);
    }
}
