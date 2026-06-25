//! Proxies D-Bus zbus pour KWin. Signatures **validées par introspection** sur
//! KWin 6.7 (`busctl --user introspect org.kde.KWin ...`).

use std::collections::HashMap;

use zbus::proxy;
use zbus::zvariant::{Fd, OwnedValue, Value};

/// `org.kde.KWin.ScreenShot2` — capture d'écran moderne (pipe).
///
/// `CaptureWindow(handle:s, options:a{sv}, pipe:h) -> a{sv}` : `handle` est
/// l'UUID interne KWin de la fenêtre (chaîne `{...}`), `options` un dict
/// (vide = format brut par défaut), `pipe` le côté écriture d'un pipe anonyme
/// où KWin écrit les pixels bruts (ARGB32). Le retour est un dict de
/// métadonnées : `width`, `height`, `stride`, `format`, `scale`…
#[proxy(
    interface = "org.kde.KWin.ScreenShot2",
    default_service = "org.kde.KWin",
    default_path = "/org/kde/KWin/ScreenShot2"
)]
pub trait ScreenShot2 {
    fn capture_window(
        &self,
        handle: &str,
        options: HashMap<String, Value<'_>>,
        pipe: Fd<'_>,
    ) -> zbus::Result<HashMap<String, OwnedValue>>;
}

/// `org.kde.kwin.Scripting` — chargement de scripts KWin (QJSEngine).
///
/// Les noms D-Bus réels sont en camelCase (`loadScript`, `unloadScript`) — on
/// les fixe explicitement car zbus PascalCase-rait par défaut.
#[proxy(
    interface = "org.kde.kwin.Scripting",
    default_service = "org.kde.KWin",
    default_path = "/Scripting"
)]
pub trait Scripting {
    /// `loadScript(path, pluginName) -> scriptId`. On fournit un `plugin_name`
    /// explicite pour pouvoir décharger ensuite par ce nom.
    #[zbus(name = "loadScript")]
    fn load_script(&self, path: &str, plugin_name: &str) -> zbus::Result<i32>;

    /// Démarre les scripts chargés et pas encore lancés.
    #[zbus(name = "start")]
    fn start(&self) -> zbus::Result<()>;

    /// `unloadScript(pluginName) -> bool`.
    #[zbus(name = "unloadScript")]
    fn unload_script(&self, plugin_name: &str) -> zbus::Result<bool>;
}
