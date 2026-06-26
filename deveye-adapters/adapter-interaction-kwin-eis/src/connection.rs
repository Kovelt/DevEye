use std::os::fd::OwnedFd;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};
use zbus::Connection as ZbusConn;

use deveye_core::MouseButton;
use deveye_ports::PortError;

use crate::error::EisError;
use crate::session::EisSession;

/// Bitmask de capabilities attendu par `connectToEIS` (DeviceType du portail
/// xdg RemoteDesktop, repris par KWin) : clavier = 1, pointeur = 2.
const CAP_KEYBOARD: i32 = 1;
const CAP_POINTER: i32 = 2;

/// Proxy D-Bus vers `org.kde.KWin.EIS.RemoteDesktop`.
///
/// Signature réelle vérifiée sur KWin 6.7 (Plasma 6) :
/// `connectToEIS(i capabilities) -> (h fd, i serial)`. Le doc de roadmap
/// supposait à tort `connectToEIS()` sans argument.
#[zbus::proxy(
    interface = "org.kde.KWin.EIS.RemoteDesktop",
    default_service = "org.kde.KWin",
    default_path = "/org/kde/KWin/EIS/RemoteDesktop"
)]
trait KwinEis {
    #[zbus(name = "connectToEIS")]
    fn connect_to_eis(&self, capabilities: i32) -> zbus::Result<(zbus::zvariant::OwnedFd, i32)>;
}

/// Ouvre une session EIS via KWin et renvoie le file descriptor brut.
///
/// Le bus de **session** est toujours utilisé (jamais le bus système). Sous
/// Plasma 5 ou hors KDE, le service ou l'interface EIS est absent : l'erreur
/// D-Bus remonte telle quelle, jamais de `panic!`.
async fn acquire_eis() -> Result<(OwnedFd, ZbusConn), EisError> {
    let conn = ZbusConn::session().await?;
    let proxy = KwinEisProxy::new(&conn).await?;
    let (fd, _serial) = proxy.connect_to_eis(CAP_KEYBOARD | CAP_POINTER).await?;
    // KWin lie la validité du fd EIS à la durée de vie de `conn` : on la rend
    // pour la garder ouverte tant que la session EI vit.
    Ok((OwnedFd::from(fd), conn))
}

/// Ouvre une session EI complète (D-Bus `connectToEIS` + handshake + device prêt).
pub(crate) async fn open_session() -> Result<EisSession, EisError> {
    let (fd, dbus) = acquire_eis().await?;
    EisSession::connect(fd, dbus).await
}

/// Action résolue (coordonnées déjà converties en absolu écran) transmise au
/// thread worker EIS. Découple le port (`Send`) des types `reis`/`xkb` qui ne
/// le sont pas.
pub(crate) enum EisAction {
    Click {
        abs: (f64, f64),
        button: MouseButton,
    },
    Hover {
        abs: (f64, f64),
    },
    Drag {
        from: (f64, f64),
        to: (f64, f64),
    },
    Type {
        text: String,
    },
    KeyPress {
        key: String,
    },
    KeyCombo {
        modifiers: Vec<String>,
        key: String,
    },
    Scroll {
        dx: f64,
        dy: f64,
    },
}

/// Un travail soumis au worker : l'action + le canal de réponse.
struct Job {
    action: EisAction,
    reply: oneshot::Sender<Result<(), String>>,
}

/// Adapter principal exposé côté `pub`.
///
/// Les types `reis`/`xkb` ne sont pas `Send` ; ils vivent donc sur un **thread
/// dédié** avec son propre runtime current-thread + `LocalSet`. L'adapter ne
/// détient qu'un `mpsc::Sender` (`Send`/`Sync`), si bien que le futur de
/// `dispatch` reste `Send` comme l'exige le trait `InteractionSink`.
///
/// La session EI elle-même est **paresseuse** : le worker n'appelle
/// `connectToEIS` qu'au premier job. Un `deveye-server` démarre donc sans erreur
/// hors session KDE — le mode `kwin-eis` n'échoue qu'à l'usage.
pub struct KwinEisInteraction {
    jobs: mpsc::Sender<Job>,
    /// Origine (x, y) de la fenêtre cible en coordonnées écran absolues, pour
    /// convertir les `Position` viewport-relatives.
    window_origin: Arc<Mutex<(i32, i32)>>,
}

impl KwinEisInteraction {
    /// Construit l'adapter et démarre le thread worker EIS. `window_origin` est
    /// l'origine écran de la fenêtre cible (0, 0 si inconnue).
    ///
    /// Retourne une erreur si l'OS refuse la creation du thread (cas rare :
    /// limite RLIMIT_NPROC atteinte, OOM kernel...).
    pub fn new(window_origin: (i32, i32)) -> Result<Self, PortError> {
        let (tx, rx) = mpsc::channel::<Job>(32);
        spawn_worker(rx)?;
        Ok(Self {
            jobs: tx,
            window_origin: Arc::new(Mutex::new(window_origin)),
        })
    }

    /// Origine ecran courante de la fenetre cible.
    pub(crate) async fn origin(&self) -> (i32, i32) {
        *self.window_origin.lock().await
    }

    /// Met a jour l'origine ecran de la fenetre cible (x, y absolus).
    ///
    /// Appelee par `deveye-server` apres resolution de la geometrie au
    /// demarrage, ou lors d'un repositionnement de la fenetre. Tous les clics
    /// EIS suivants utilisent la nouvelle origine immediatement.
    pub async fn set_window_origin(&self, origin: (i32, i32)) {
        *self.window_origin.lock().await = origin;
    }

    /// Soumet une action au worker et attend son résultat.
    pub(crate) async fn submit(&self, action: EisAction) -> Result<(), PortError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.jobs
            .send(Job {
                action,
                reply: reply_tx,
            })
            .await
            .map_err(|_| PortError::Interaction("worker EIS arrêté".to_string()))?;
        match reply_rx.await {
            Ok(result) => result.map_err(PortError::Interaction),
            Err(_) => Err(PortError::Interaction(
                "worker EIS terminé sans réponse".to_string(),
            )),
        }
    }
}

/// Démarre le thread worker : runtime current-thread + `LocalSet` pour héberger
/// la session EI non-`Send`, traite les jobs en série (un seul device virtuel).
/// Retourne une erreur si l'OS refuse la creation du thread.
fn spawn_worker(mut rx: mpsc::Receiver<Job>) -> Result<(), EisError> {
    std::thread::Builder::new()
        .name("deveye-eis".to_string())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("deveye-eis : impossible de créer le runtime worker : {e}");
                    return;
                }
            };
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async move {
                let mut session: Option<EisSession> = None;
                while let Some(job) = rx.recv().await {
                    if session.is_none() {
                        match open_session().await {
                            Ok(s) => session = Some(s),
                            Err(e) => {
                                let _ = job.reply.send(Err(e.to_string()));
                                continue;
                            }
                        }
                    }
                    let active = session.as_mut().expect("session initialisée ci-dessus");
                    let result = run_action(active, job.action).await;
                    // Une erreur d'émission invalide souvent le fd : on jette la
                    // session pour forcer un handshake neuf au prochain job.
                    if result.is_err() {
                        session = None;
                    }
                    let _ = job.reply.send(result.map_err(|e| e.to_string()));
                }
            });
        })
        .map_err(|e| EisError::Eis(format!("impossible de demarrer le thread worker EIS : {e}")))?;
    Ok(())
}

async fn run_action(session: &mut EisSession, action: EisAction) -> Result<(), EisError> {
    match action {
        EisAction::Click { abs, button } => session.click(abs, button).await,
        EisAction::Hover { abs } => session.hover(abs).await,
        EisAction::Drag { from, to } => session.drag(from, to).await,
        EisAction::Type { text } => session.type_text(&text).await,
        EisAction::KeyPress { key } => session.key_press(&key).await,
        EisAction::KeyCombo { modifiers, key } => session.key_combo(&modifiers, &key).await,
        EisAction::Scroll { dx, dy } => session.scroll(dx, dy).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// set_window_origin met bien a jour la valeur renvoyee par origin().
    #[tokio::test]
    async fn set_and_read_origin_roundtrip() {
        let eis = KwinEisInteraction::new((0, 0)).expect("creation KwinEisInteraction");
        assert_eq!(eis.origin().await, (0, 0));
        eis.set_window_origin((120, 340)).await;
        assert_eq!(eis.origin().await, (120, 340));
    }

    /// La translation viewport -> absolu est correcte avant et apres mise a
    /// jour de l'origine (simule le comportement de dispatch.rs sans session
    /// KWin reelle).
    #[tokio::test]
    async fn origin_drives_coordinate_translation() {
        let eis = KwinEisInteraction::new((10, 20)).expect("creation KwinEisInteraction");
        let (ox, oy) = eis.origin().await;
        // Position viewport (5, 5) + origine (10, 20) => absolu (15, 25).
        assert_eq!((5 + ox, 5 + oy), (15, 25));

        eis.set_window_origin((100, 200)).await;
        let (ox2, oy2) = eis.origin().await;
        // Apres mise a jour : (5, 5) + (100, 200) => (105, 205).
        assert_eq!((5 + ox2, 5 + oy2), (105, 205));
    }
}
