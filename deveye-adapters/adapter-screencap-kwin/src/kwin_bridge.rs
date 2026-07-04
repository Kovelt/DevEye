//! Bridge D-Bus éphémère pour exécuter un script KWin et récupérer son
//! résultat par callback.
//!
//! Factorise le cycle de vie commun aux mécaniques de résolution/action de
//! cet adapter (capture, origine de fenêtre, activation, liste, géométrie) :
//! un script KWin éphémère (`org.kde.kwin.Scripting`) appelle `callDBus` vers
//! une interface [`Bridge`] servie le temps de l'appel par cet adapter, qui
//! relaie le résultat par un canal oneshot. Seule la génération du texte JS
//! et l'interprétation du tuple résultat restent spécifiques à chaque
//! appelant (cf. [`crate::renderer`], [`crate::window_manager`]).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{oneshot, Mutex};
use zbus::Connection;

use crate::error::KwinScreencapError;
use crate::proxy::ScriptingProxy;

/// Chemin de l'objet D-Bus que cet adapter sert le temps d'un appel ; le
/// script KWin y renvoie son résultat par `callDBus`.
pub(crate) const BRIDGE_PATH: &str = "/deveye/bridge";
pub(crate) const BRIDGE_IFACE: &str = "org.deveye.KwinBridge";

/// Compteur monotone pour des noms de plugin/tempfile uniques par appel :
/// deux résolutions concurrentes dans le même process ne doivent pas charger
/// le même nom de plugin KWin (un nom déjà chargé ne ré-exécute pas le script).
fn next_id() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Interface éphémère servie par l'adapter : le script KWin y renvoie son
/// résultat via `callDBus(dest, ..., "Report", a, b)`.
struct Bridge {
    tx: Mutex<Option<oneshot::Sender<(String, String)>>>,
}

#[zbus::interface(name = "org.deveye.KwinBridge")]
impl Bridge {
    #[zbus(name = "Report")]
    async fn report(&self, first: String, second: String) {
        if let Some(tx) = self.tx.lock().await.take() {
            let _ = tx.send((first, second));
        }
    }
}

/// Exécute un script KWin éphémère et renvoie le couple qu'il a reporté via
/// [`Bridge`].
///
/// Cycle de vie complet : enregistrement de l'interface `Bridge` sur `conn`,
/// écriture de `script_body` dans un tempfile (nom préfixé par
/// `script_prefix`), `loadScript` + `start`, attente bornée à 4 s, puis
/// `unloadScript` et nettoyage. `script_prefix` sert à la fois de préfixe de
/// fichier temporaire et, combiné au PID et à un compteur, de nom de plugin
/// unique.
///
/// Le retrait de `Bridge` de `conn` est inconditionnel (même si une étape
/// antérieure échoue) : `conn` est toujours une connexion éphémère créée pour
/// l'appel courant, donc sans conséquence pratique, mais centraliser le
/// nettoyage évite de le sauter par erreur comme le faisait une des 3 copies
/// d'origine.
pub(crate) async fn run_kwin_script(
    conn: &Connection,
    script_prefix: &str,
    script_body: &str,
) -> Result<(String, String), KwinScreencapError> {
    let (tx, rx) = oneshot::channel();
    conn.object_server()
        .at(
            BRIDGE_PATH,
            Bridge {
                tx: Mutex::new(Some(tx)),
            },
        )
        .await
        .map_err(|e| KwinScreencapError::Dbus(format!("object_server.at : {e}")))?;

    let result = write_load_start_wait_unload(conn, script_prefix, script_body, rx).await;

    let _ = conn.object_server().remove::<Bridge, _>(BRIDGE_PATH).await;
    result
}

/// Écrit `script_body` en tempfile puis charge/démarre/attend/décharge.
async fn write_load_start_wait_unload(
    conn: &Connection,
    script_prefix: &str,
    script_body: &str,
    rx: oneshot::Receiver<(String, String)>,
) -> Result<(String, String), KwinScreencapError> {
    let plugin = format!("{script_prefix}-{}-{}", std::process::id(), next_id());
    // L11: tempfile cree le fichier avec O_EXCL + nom aleatoire — evite le
    // TOCTOU d'un chemin predictible sous /tmp.
    let temp = tempfile::Builder::new()
        .prefix(&format!("{script_prefix}-"))
        .suffix(".js")
        .tempfile()
        .map_err(|e| KwinScreencapError::Dbus(format!("tempfile creation : {e}")))?;
    // N1: ecriture asynchrone via tokio::fs pour ne pas bloquer le runtime.
    tokio::fs::write(temp.path(), script_body)
        .await
        .map_err(|e| KwinScreencapError::Dbus(format!("ecriture script KWin : {e}")))?;

    let result = load_start_wait_unload(conn, temp.path(), &plugin, rx).await;
    // Nettoyage best-effort : NamedTempFile supprime le fichier a sa
    // destruction (drop). Hors hot-path async (juste un unlink).
    drop(temp);
    result
}

/// Charge + démarre le script, attend la réponse (timeout 4 s), décharge.
async fn load_start_wait_unload(
    conn: &Connection,
    path: &std::path::Path,
    plugin: &str,
    rx: oneshot::Receiver<(String, String)>,
) -> Result<(String, String), KwinScreencapError> {
    let scripting = ScriptingProxy::new(conn)
        .await
        .map_err(|e| KwinScreencapError::Dbus(format!("proxy Scripting : {e}")))?;
    scripting
        .load_script(&path.to_string_lossy(), plugin)
        .await
        .map_err(|e| KwinScreencapError::Unavailable(format!("loadScript : {e}")))?;

    // A partir d'ici le script est CHARGE : tout chemin de sortie doit le
    // decharger, y compris un echec de `start()` (sinon un plugin reste charge
    // dans KWin et un nom identique ne se re-executerait plus). D'ou l'appel a
    // `unload_script` avant chaque `?` de cette section.
    if let Err(e) = scripting.start().await {
        let _ = scripting.unload_script(plugin).await;
        return Err(KwinScreencapError::Unavailable(format!(
            "Scripting.start : {e}"
        )));
    }

    let received = tokio::time::timeout(Duration::from_secs(4), rx).await;
    let _ = scripting.unload_script(plugin).await;

    received
        .map_err(|_| KwinScreencapError::Timeout("script KWin sans reponse apres 4 s".into()))?
        .map_err(|_| KwinScreencapError::Unavailable("canal de resolution ferme".into()))
}
