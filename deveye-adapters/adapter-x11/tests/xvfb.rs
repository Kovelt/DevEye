//! Tests d'integration de `adapter-x11` contre un Xvfb PRIVE demarre par les
//! tests eux-memes. Tous `#[ignore]` : ils lancent un vrai serveur X.
//!
//! ```text
//! cargo test -p adapter-x11 -- --ignored --test-threads=1
//! ```
//!
//! Garanties de securite :
//! - le serveur est lance sur un display libre entre `:251` et `:259`, sans
//!   ecoute TCP, et arrete par SIGTERM a la fin (jamais de `pkill -f`) ;
//! - un verrou global serialise les tests : un seul Xvfb a la fois ;
//! - l'adapter recoit le display explicitement et ne touche JAMAIS la session
//!   de l'utilisateur (sa garde refuse tout display < 100 ou egal a `$DISPLAY`).
//! - aucune fenetre GL, aucun plugin : un client x11rb pose une fenetre sonde.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use adapter_x11::{X11Config, X11Target};
use deveye_core::{InteractionCommand, MouseButton, Position};
use deveye_ports::{
    InteractionSink, PortError, Renderer, WindowGeometry, WindowLister, WindowManager,
};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, EventMask, PropMode,
    WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::COPY_DEPTH_FROM_PARENT;

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
const TITLE: &str = "DevEye Probe";
/// Fenetre sonde : position et taille.
const WIN: (i16, i16, u16, u16) = (50, 40, 400, 300);

/// Xvfb prive, arrete proprement au `Drop`.
struct Xvfb {
    child: Option<Child>,
    number: u32,
    _lock: MutexGuard<'static, ()>,
}

fn display_is_free(n: u32) -> bool {
    !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists()
        && !Path::new(&format!("/tmp/.X{n}-lock")).exists()
}

impl Xvfb {
    fn start() -> Self {
        let lock = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
        let number = (251..=259)
            .find(|&n| display_is_free(n))
            .expect("aucun display libre entre :251 et :259");
        let mut x = Xvfb {
            child: None,
            number,
            _lock: lock,
        };
        x.spawn();
        x
    }

    fn display(&self) -> String {
        format!(":{}", self.number)
    }

    fn spawn(&mut self) {
        let child = Command::new("Xvfb")
            .arg(self.display())
            .args(["-nolisten", "tcp", "-screen", "0", "1400x1000x24"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("lancement de Xvfb");
        self.child = Some(child);
        let deadline = Instant::now() + Duration::from_secs(10);
        while RustConnection::connect(Some(&self.display())).is_err() {
            assert!(Instant::now() < deadline, "Xvfb ne repond pas");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Arrete le serveur (SIGTERM, puis SIGKILL en dernier recours) et nettoie
    /// ses fichiers residuels.
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = Command::new("kill").arg(child.id().to_string()).status();
            let deadline = Instant::now() + Duration::from_secs(5);
            while child.try_wait().ok().flatten().is_none() {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let n = self.number;
        let _ = std::fs::remove_file(format!("/tmp/.X11-unix/X{n}"));
        let _ = std::fs::remove_file(format!("/tmp/.X{n}-lock"));
    }
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Fenetre sonde rouge, titree, qui journalise les evenements d'entree.
struct Probe {
    conn: RustConnection,
    root: u32,
    win: u32,
}

impl Probe {
    fn new(display: &str) -> Self {
        let (conn, screen) = RustConnection::connect(Some(display)).expect("connexion sonde");
        let root = conn.setup().roots[screen].root;
        let win = conn.generate_id().unwrap();
        let mask = EventMask::KEY_PRESS
            | EventMask::KEY_RELEASE
            | EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION
            | EventMask::BUTTON_MOTION
            | EventMask::EXPOSURE;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            win,
            root,
            WIN.0,
            WIN.1,
            WIN.2,
            WIN.3,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .background_pixel(0x00ff_0000)
                .event_mask(mask),
        )
        .unwrap();
        let utf8 = conn
            .intern_atom(false, b"UTF8_STRING")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let net = conn
            .intern_atom(false, b"_NET_WM_NAME")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        conn.change_property8(PropMode::REPLACE, win, net, utf8, TITLE.as_bytes())
            .unwrap();
        conn.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"probe\0DevEyeProbe\0",
        )
        .unwrap();
        conn.map_window(win).unwrap();
        conn.flush().unwrap();
        let p = Probe { conn, root, win };
        p.sync();
        p
    }

    fn sync(&self) {
        self.conn.get_input_focus().unwrap().reply().unwrap();
    }

    /// Evenements recus depuis le dernier appel.
    fn events(&self) -> Vec<Event> {
        self.sync();
        let mut out = Vec::new();
        while let Some(e) = self.conn.poll_for_event().unwrap() {
            out.push(e);
        }
        out
    }

    fn move_to(&self, x: i32, y: i32) {
        self.conn
            .configure_window(self.win, &ConfigureWindowAux::new().x(x).y(y))
            .unwrap();
        self.sync();
    }

    /// (keycode, keysyms par keycode, keymap complet).
    fn keymap(&self) -> (u8, u8, Vec<u32>) {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let r = self
            .conn
            .get_keyboard_mapping(min, max - min + 1)
            .unwrap()
            .reply()
            .unwrap();
        (min, r.keysyms_per_keycode, r.keysyms)
    }

    /// Touches actuellement enfoncees selon le serveur (XQueryKeymap).
    fn pressed_keys(&self) -> Vec<u8> {
        let r = self.conn.query_keymap().unwrap().reply().unwrap();
        let mut keys = Vec::new();
        for (i, byte) in r.keys.iter().enumerate() {
            for bit in 0..8 {
                if byte & (1 << bit) != 0 {
                    keys.push((i * 8 + bit) as u8);
                }
            }
        }
        keys
    }
}

fn target(x: &Xvfb, window: Option<&str>) -> X11Target {
    X11Target::new(X11Config {
        display: x.display(),
        window: window.map(str::to_owned),
        class: None,
    })
    .expect("garde : le display prive doit etre accepte")
}

fn pos(x: i32, y: i32) -> Position {
    Position::new(x, y)
}

#[tokio::test]
#[ignore = "demarre un Xvfb prive"]
async fn pointer_gestures_reach_the_window_at_the_right_place() {
    let x = Xvfb::start();
    let probe = Probe::new(&x.display());
    let t = target(&x, Some(TITLE));
    let _ = probe.events();

    // Clic gauche en (100, 80), relatif a la fenetre.
    t.dispatch(InteractionCommand::click(pos(100, 80)))
        .await
        .unwrap();
    let ev = probe.events();
    let presses: Vec<_> = ev
        .iter()
        .filter_map(|e| match e {
            Event::ButtonPress(b) => Some((b.detail, b.event_x, b.event_y)),
            _ => None,
        })
        .collect();
    assert_eq!(presses, vec![(1, 100, 80)], "{ev:?}");
    assert!(ev.iter().any(|e| matches!(e, Event::ButtonRelease(_))));

    // Clic droit.
    t.dispatch(InteractionCommand::Click {
        position: pos(10, 10),
        button: MouseButton::Right,
    })
    .await
    .unwrap();
    assert!(probe
        .events()
        .iter()
        .any(|e| matches!(e, Event::ButtonPress(b) if b.detail == 3)));

    // Double-clic : deux appuis du bouton 1.
    t.dispatch(InteractionCommand::double_click(pos(60, 60)))
        .await
        .unwrap();
    let n = probe
        .events()
        .iter()
        .filter(|e| matches!(e, Event::ButtonPress(b) if b.detail == 1))
        .count();
    assert_eq!(n, 2);

    // Glisser : au moins 10 MotionNotify, le dernier exactement a l'arrivee.
    t.dispatch(InteractionCommand::drag_drop(pos(20, 20), pos(220, 120)))
        .await
        .unwrap();
    let motions: Vec<(i16, i16)> = probe
        .events()
        .iter()
        .filter_map(|e| match e {
            Event::MotionNotify(m) => Some((m.event_x, m.event_y)),
            _ => None,
        })
        .collect();
    assert!(
        motions.len() >= 10,
        "{} mouvements : {motions:?}",
        motions.len()
    );
    assert_eq!(*motions.last().unwrap(), (220, 120));

    // Molette : bas = 5, haut = 4, gauche = 6, droite = 7.
    t.dispatch(InteractionCommand::scroll(pos(100, 100), 0, 240))
        .await
        .unwrap();
    t.dispatch(InteractionCommand::scroll(pos(100, 100), 0, -120))
        .await
        .unwrap();
    t.dispatch(InteractionCommand::scroll(pos(100, 100), -120, 120))
        .await
        .unwrap();
    let wheel: Vec<u8> = probe
        .events()
        .iter()
        .filter_map(|e| match e {
            Event::ButtonPress(b) if b.detail >= 4 => Some(b.detail),
            _ => None,
        })
        .collect();
    assert_eq!(wheel, vec![5, 5, 4, 5, 6]);

    // L'origine est relue a chaque appel : fenetre deplacee entre deux clics.
    probe.move_to(300, 200);
    let _ = probe.events();
    t.dispatch(InteractionCommand::click(pos(10, 10)))
        .await
        .unwrap();
    let hit: Vec<_> = probe
        .events()
        .iter()
        .filter_map(|e| match e {
            Event::ButtonPress(b) => Some((b.event_x, b.event_y, b.root_x, b.root_y)),
            _ => None,
        })
        .collect();
    assert_eq!(hit, vec![(10, 10, 310, 210)]);
}

#[tokio::test]
#[ignore = "demarre un Xvfb prive"]
async fn unsupported_commands_are_refused() {
    let x = Xvfb::start();
    let probe = Probe::new(&x.display());
    let t = target(&x, Some(TITLE));
    for cmd in [
        InteractionCommand::navigate("x"),
        InteractionCommand::handle_dialog(true, None),
        InteractionCommand::upload_file("i", vec![]),
    ] {
        let e = t.dispatch(cmd).await.unwrap_err();
        assert!(matches!(e, PortError::Unsupported(_)), "{e:?}");
    }
    let _ = probe.events();
}

/// Installe un petit clavier « suisse romand » sur les keycodes 200..=202 par le
/// protocole core (`ChangeKeyboardMapping`) : é / É, à / À, è / ü. On ne passe
/// pas par `setxkbmap` : sur certains Xvfb (dont celui de Fedora) il n'a aucun
/// effet, le serveur ne pouvant pas ecrire sa sortie XKM.
fn install_swiss_like_keys(probe: &Probe) {
    let (_, per, _) = probe.keymap();
    let per = usize::from(per);
    let mut syms = Vec::new();
    for (lower, upper) in [(0xe9u32, 0xc9u32), (0xe0, 0xc0), (0xe8, 0xfc)] {
        let mut row = vec![0u32; per];
        row[0] = lower;
        row[1] = upper;
        syms.extend(row);
    }
    probe
        .conn
        .change_keyboard_mapping(3, 200, per as u8, &syms)
        .unwrap();
    probe.sync();
}

#[tokio::test]
#[ignore = "demarre un Xvfb prive"]
async fn typing_follows_the_server_keymap_and_never_sticks_modifiers() {
    let x = Xvfb::start();
    let probe = Probe::new(&x.display());
    install_swiss_like_keys(&probe);
    let t = target(&x, Some(TITLE));
    let (min, per, before) = probe.keymap();
    let _ = probe.events();

    // « éàü » : é et à directs, ü au niveau Shift de la touche è/ü.
    t.dispatch(InteractionCommand::type_text("éàü"))
        .await
        .unwrap();
    let keysym = |code: u8, shift: bool| {
        before[(usize::from(code) - usize::from(min)) * usize::from(per) + usize::from(shift)]
    };
    let typed: Vec<u32> = probe
        .events()
        .iter()
        .filter_map(|e| match e {
            Event::KeyPress(k) => {
                let shift = u16::from(k.state) & 1 != 0;
                Some(keysym(k.detail, shift))
            }
            _ => None,
        })
        // Les presses du modificateur Shift_L ne sont pas des caracteres.
        .filter(|&s| s != 0xffe1)
        .collect();
    assert_eq!(typed, vec![0xe9, 0xe0, 0xfc]);
    assert!(probe.pressed_keys().is_empty(), "touche collee apres Type");

    // Caractere absent du layout : keycode libre remappe puis restaure.
    let free = (usize::from(min)..before.len() / usize::from(per) + usize::from(min))
        .rev()
        .find(|&c| {
            let i = (c - usize::from(min)) * usize::from(per);
            before[i..i + usize::from(per)].iter().all(|&s| s == 0)
        })
        .expect("keycode libre") as u8;
    let _ = probe.events();
    t.dispatch(InteractionCommand::type_text("☃"))
        .await
        .unwrap();
    let used: Vec<u8> = probe
        .events()
        .iter()
        .filter_map(|e| match e {
            Event::KeyPress(k) => Some(k.detail),
            _ => None,
        })
        .collect();
    assert_eq!(used, vec![free]);
    let (_, _, after) = probe.keymap();
    assert_eq!(after, before, "keymap non restauree");

    // Combo valide puis combo invalide : rien de colle, rien emis pour l'invalide.
    t.dispatch(InteractionCommand::key_combo(vec!["ctrl".into()], "a"))
        .await
        .unwrap();
    assert!(probe.pressed_keys().is_empty());
    let _ = probe.events();
    let err = t
        .dispatch(InteractionCommand::key_combo(
            vec!["ctrl".into(), "hyperdrive".into()],
            "a",
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, PortError::Interaction(_)), "{err:?}");
    assert!(probe
        .events()
        .iter()
        .all(|e| !matches!(e, Event::KeyPress(_))));
    assert!(probe.pressed_keys().is_empty());

    // Touche nommee W3C.
    t.dispatch(InteractionCommand::key("Escape")).await.unwrap();
    assert!(probe
        .events()
        .iter()
        .any(|e| matches!(e, Event::KeyPress(k) if keysym(k.detail, false) == 0xff1b)));
}

#[tokio::test]
#[ignore = "demarre un Xvfb prive"]
async fn capture_geometry_listing_and_focus() {
    let x = Xvfb::start();
    let probe = Probe::new(&x.display());
    let t = target(&x, Some(TITLE));

    let shot = t.screenshot().await.unwrap();
    assert_eq!(shot.viewport.width, u32::from(WIN.2));
    assert_eq!(shot.viewport.height, u32::from(WIN.3));
    let img = image::load_from_memory(&shot.bytes).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (400, 300));
    // Aplat rouge sur toute la surface.
    for (px, py) in [(0, 0), (200, 150), (399, 299)] {
        assert_eq!(
            img.get_pixel(px, py).0,
            [255, 0, 0, 255],
            "pixel ({px},{py})"
        );
    }
    assert_eq!(t.viewport_size().await.unwrap().width, 400);

    let rect = t.geometry(None).await.unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (50, 40, 400, 300)
    );
    probe.move_to(120, 90);
    let rect = t.geometry(None).await.unwrap();
    assert_eq!((rect.x, rect.y), (120, 90));
    // Surcharge de titre insensible a la casse.
    assert_eq!(t.geometry(Some("devEYE probe")).await.unwrap().x, 120);
    assert!(matches!(
        t.geometry(Some("pas de telle fenetre")).await,
        Err(PortError::Inspect(_))
    ));

    let wins = t.list_windows().await.unwrap();
    assert!(
        wins.iter()
            .any(|w| w.title == TITLE && w.kind == "x11-window"),
        "{wins:?}"
    );

    assert_eq!(t.activate(None).await.unwrap(), TITLE);
    let focus = probe.conn.get_input_focus().unwrap().reply().unwrap().focus;
    assert_eq!(focus, probe.win);
    let _ = probe.root;

    // Sans fenetre configuree : ecran entier.
    let full = target(&x, None);
    let shot = full.screenshot().await.unwrap();
    assert_eq!((shot.viewport.width, shot.viewport.height), (1400, 1000));
    assert!(matches!(
        full.geometry(None).await,
        Err(PortError::Inspect(_))
    ));
}

#[tokio::test]
#[ignore = "demarre un Xvfb prive"]
async fn partly_offscreen_window_keeps_window_coordinates() {
    let x = Xvfb::start();
    let probe = Probe::new(&x.display());
    let t = target(&x, Some(TITLE));
    // 100 colonnes et 60 lignes de la fenetre (400x300) sortent a gauche/en haut.
    probe.move_to(-100, -60);
    let shot = t.screenshot().await.unwrap();
    assert_eq!((shot.viewport.width, shot.viewport.height), (400, 300));
    let img = image::load_from_memory(&shot.bytes).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (400, 300));
    // Partie hors ecran : noir ; partie visible : rouge, au bon endroit.
    assert_eq!(img.get_pixel(50, 150).0, [0, 0, 0, 255]);
    assert_eq!(img.get_pixel(200, 30).0, [0, 0, 0, 255]);
    assert_eq!(img.get_pixel(100, 60).0, [255, 0, 0, 255]);
    assert_eq!(img.get_pixel(399, 299).0, [255, 0, 0, 255]);
}

#[tokio::test]
#[ignore = "demarre un Xvfb prive"]
async fn reconnects_after_the_server_restarts() {
    let mut x = Xvfb::start();
    let probe = Probe::new(&x.display());
    let t = target(&x, Some(TITLE));
    assert!(t.screenshot().await.is_ok());
    drop(probe);

    // Serveur tue : seule cette coupure est une erreur reseau.
    x.stop();
    let err = t.screenshot().await.unwrap_err();
    assert!(matches!(err, PortError::Network(_)), "{err:?}");

    // Serveur relance sur le meme display : l'appel suivant se reconnecte seul.
    x.spawn();
    let _probe = Probe::new(&x.display());
    assert!(t.screenshot().await.is_ok());
    let cmd = t.dispatch(InteractionCommand::click(pos(5, 5))).await;
    assert!(cmd.is_ok(), "{cmd:?}");
}

#[tokio::test]
#[ignore = "demarre un Xvfb prive"]
async fn a_missing_window_is_not_a_network_error() {
    let x = Xvfb::start();
    let t = target(&x, Some("fenetre-qui-n-existe-pas"));
    let err = t.screenshot().await.unwrap_err();
    assert!(matches!(err, PortError::Render(_)), "{err:?}");
    let err = t
        .dispatch(InteractionCommand::click(pos(1, 1)))
        .await
        .unwrap_err();
    assert!(matches!(err, PortError::Interaction(_)), "{err:?}");
}

#[tokio::test]
#[ignore = "demarre un Xvfb prive"]
async fn no_server_is_a_network_error() {
    let _x = Xvfb::start(); // verrou + display libre garanti
    let n = (251..=259).find(|&n| display_is_free(n)).unwrap_or(259);
    // Display valide pour la garde, mais sans serveur derriere.
    let t = X11Target::new(X11Config {
        display: format!(":{n}"),
        ..X11Config::default()
    })
    .unwrap();
    let err = t.screenshot().await.unwrap_err();
    assert!(matches!(err, PortError::Network(_)), "{err:?}");
}
