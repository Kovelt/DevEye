use std::io::Cursor;

use async_trait::async_trait;
use image::ImageFormat;
use xcap::{Monitor, Window};

use deveye_core::{Screenshot, Size};
use deveye_ports::{PortError, PortResult, Renderer};

use crate::error::ScreencapError;

/// N8: Execute une closure de capture xcap bloquante sur le pool de threads,
/// avec mapping d'erreur unifie vers PortError. Factorise les 4 blocs
/// spawn_blocking identiques.
async fn run_xcap<T>(
    f: impl FnOnce() -> Result<T, ScreencapError> + Send + 'static,
) -> PortResult<T>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| PortError::from(ScreencapError::Xcap(format!("spawn_blocking : {e}"))))?
        .map_err(PortError::from)
}

/// Capture la fenêtre dont le titre contient la sous-chaîne fournie.
///
/// Utilise [`xcap`](https://crates.io/crates/xcap) — multi-OS (Windows, macOS,
/// Linux X11/XWayland **uniquement** : xcap énumère les fenêtres via XCB,
/// donc sous une session Wayland native il faut lancer l'app cible avec
/// `GDK_BACKEND=x11`). Aucune dépendance à un webview, fonctionne sur n'importe
/// quelle app graphique. C'est le fallback universel quand CDP n'est pas
/// disponible (macOS WKWebView, app legacy MAUI, jeu, etc.).
///
/// Le `title_substring` est cherché de façon **insensible à la casse**.
/// `xcap` ne dispose pas d'API "find by exact title" multiplateforme — on
/// itère sur les fenêtres listées par l'OS.
#[derive(Clone, Debug)]
pub struct XcapRenderer {
    title_substring: String,
}

impl XcapRenderer {
    pub fn new(title_substring: impl Into<String>) -> Self {
        Self {
            title_substring: title_substring.into(),
        }
    }

    fn find_window(&self) -> Result<Window, ScreencapError> {
        let windows = Window::all().map_err(|e| ScreencapError::Xcap(e.to_string()))?;
        // xcap >= 0.9 : `title()` est fallible (Result<String, XCapError>).
        // Une fenêtre dont le titre est illisible (ex : permission refusée
        // par l'OS sur certaines fenêtres système) est simplement ignorée
        // plutôt que de faire échouer toute la recherche (`.ok()` -> `None`).
        let titles: Vec<Option<String>> = windows.iter().map(|w| w.title().ok()).collect();
        let idx = find_title_match(titles.iter().map(|t| t.as_deref()), &self.title_substring)
            .ok_or_else(|| ScreencapError::WindowNotFound(self.title_substring.clone()))?;
        Ok(windows
            .into_iter()
            .nth(idx)
            .expect("idx provient de la meme liste, donc dans les bornes"))
    }
}

/// Cherche l'indice de la première entrée dont le titre (quand lisible)
/// contient `needle`, insensible à la casse. Une entrée `None` (titre
/// illisible) ne matche jamais, quel que soit `needle` — c'est cohérent avec
/// le fait qu'on ne sait rien dire du titre réel de cette fenêtre.
fn find_title_match<'a, I>(titles: I, needle: &str) -> Option<usize>
where
    I: IntoIterator<Item = Option<&'a str>>,
{
    let needle = needle.to_lowercase();
    titles
        .into_iter()
        .position(|title| title.is_some_and(|t| t.to_lowercase().contains(&needle)))
}

#[async_trait]
impl Renderer for XcapRenderer {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        let renderer = self.clone();
        run_xcap(move || {
            let window = renderer.find_window()?;
            let img = window
                .capture_image()
                .map_err(|e| ScreencapError::Xcap(e.to_string()))?;
            let viewport = Size::new(img.width(), img.height());
            let mut buf = Vec::new();
            img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
                .map_err(|e| ScreencapError::Encoding(e.to_string()))?;
            Ok(Screenshot::new_png(buf, viewport))
        })
        .await
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        let renderer = self.clone();
        run_xcap(move || {
            let window = renderer.find_window()?;
            // xcap >= 0.9 : width()/height() renvoient un Result.
            let width = window
                .width()
                .map_err(|e| ScreencapError::Xcap(e.to_string()))?;
            let height = window
                .height()
                .map_err(|e| ScreencapError::Xcap(e.to_string()))?;
            Ok(Size::new(width, height))
        })
        .await
    }
}

/// Capture le premier moniteur détecté — **repli plein-écran**.
///
/// Usage : CI headless (Xvfb), ou quand la fenêtre cible n'est pas encore
/// titrée / introuvable par titre. Indépendant de tout webview et de tout
/// titre de fenêtre. Pour la capture par titre, voir [`XcapRenderer`].
#[derive(Clone, Debug, Default)]
pub struct MonitorRenderer;

impl MonitorRenderer {
    fn primary() -> Result<Monitor, ScreencapError> {
        let monitors = Monitor::all().map_err(|e| ScreencapError::Xcap(e.to_string()))?;
        // N2: preference au moniteur marque is_primary(), avec repli sur le
        // premier si aucun n'est marque principal.
        // xcap >= 0.9 : is_primary() renvoie un Result — on le resout pour
        // chaque moniteur (echec propage immediatement, ce n'est pas
        // equivalent a "non principal") avant de choisir via `pick_primary`.
        let flags: Vec<bool> = monitors
            .iter()
            .map(|m| {
                m.is_primary()
                    .map_err(|e| ScreencapError::Xcap(e.to_string()))
            })
            .collect::<Result<_, _>>()?;
        let idx = pick_primary(flags)
            .ok_or_else(|| ScreencapError::Xcap("aucun moniteur detecte".into()))?;
        Ok(monitors
            .into_iter()
            .nth(idx)
            .expect("idx provient de la meme liste, donc dans les bornes"))
    }
}

/// Choisit l'indice du moniteur "principal" parmi les flags `is_primary`
/// fournis dans l'ordre d'énumération OS ; repli sur le premier moniteur si
/// aucun n'est marqué principal. `None` si la liste est vide.
fn pick_primary<I>(flags: I) -> Option<usize>
where
    I: IntoIterator<Item = bool>,
{
    let mut fallback: Option<usize> = None;
    for (i, is_primary) in flags.into_iter().enumerate() {
        if is_primary {
            return Some(i);
        }
        if fallback.is_none() {
            fallback = Some(i);
        }
    }
    fallback
}

#[async_trait]
impl Renderer for MonitorRenderer {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        run_xcap(|| {
            let monitor = MonitorRenderer::primary()?;
            let img = monitor
                .capture_image()
                .map_err(|e| ScreencapError::Xcap(e.to_string()))?;
            let viewport = Size::new(img.width(), img.height());
            let mut buf = Vec::new();
            img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
                .map_err(|e| ScreencapError::Encoding(e.to_string()))?;
            Ok(Screenshot::new_png(buf, viewport))
        })
        .await
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        run_xcap(|| {
            let monitor = MonitorRenderer::primary()?;
            // xcap >= 0.9 : width()/height() renvoient un Result.
            let width = monitor
                .width()
                .map_err(|e| ScreencapError::Xcap(e.to_string()))?;
            let height = monitor
                .height()
                .map_err(|e| ScreencapError::Xcap(e.to_string()))?;
            Ok(Size::new(width, height))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{find_title_match, pick_primary};

    // ---- find_title_match ------------------------------------------------

    #[test]
    fn find_title_match_insensible_a_la_casse() {
        let titles = vec![Some("Mon Editeur"), Some("Terminal")];
        assert_eq!(
            find_title_match(titles, "EDITEUR"),
            Some(0),
            "doit matcher malgre la casse differente"
        );
    }

    #[test]
    fn find_title_match_renvoie_le_premier_des_multiples() {
        let titles = vec![Some("Firefox - a"), Some("Terminal"), Some("Firefox - b")];
        assert_eq!(
            find_title_match(titles, "firefox"),
            Some(0),
            "doit renvoyer le premier match, pas le suivant"
        );
    }

    #[test]
    fn find_title_match_none_si_aucun_titre_ne_matche() {
        let titles = vec![Some("Terminal"), Some("Editeur")];
        assert_eq!(find_title_match(titles, "navigateur"), None);
    }

    #[test]
    fn find_title_match_ignore_les_titres_illisibles() {
        // Un titre `None` (title() a echoue cote xcap) ne doit jamais
        // matcher, meme quand needle est vide.
        let titles = vec![None, Some("Terminal")];
        assert_eq!(find_title_match(titles, ""), Some(1));
    }

    #[test]
    fn find_title_match_none_sur_iterateur_vide() {
        let titles: Vec<Option<&str>> = vec![];
        assert_eq!(find_title_match(titles, "quoi que ce soit"), None);
    }

    // ---- pick_primary ------------------------------------------------------

    #[test]
    fn pick_primary_prefere_le_moniteur_marque() {
        let flags = vec![false, true, false];
        assert_eq!(pick_primary(flags), Some(1));
    }

    #[test]
    fn pick_primary_replie_sur_le_premier_si_aucun_marque() {
        let flags = vec![false, false, false];
        assert_eq!(pick_primary(flags), Some(0));
    }

    #[test]
    fn pick_primary_none_si_liste_vide() {
        let flags: Vec<bool> = vec![];
        assert_eq!(pick_primary(flags), None);
    }
}
