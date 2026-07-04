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
        let needle = self.title_substring.to_lowercase();
        let windows = Window::all().map_err(|e| ScreencapError::Xcap(e.to_string()))?;
        windows
            .into_iter()
            // xcap >= 0.9 : `title()` est fallible (Result<String, XCapError>).
            // Une fenêtre dont le titre est illisible (ex : permission refusée
            // par l'OS sur certaines fenêtres système) est simplement ignorée
            // plutôt que de faire échouer toute la recherche.
            .find(|w| {
                w.title()
                    .map(|title| title.to_lowercase().contains(&needle))
                    .unwrap_or(false)
            })
            .ok_or_else(|| ScreencapError::WindowNotFound(self.title_substring.clone()))
    }
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
        // xcap >= 0.9 : is_primary() renvoie un Result (echec propage, ce
        // n'est pas equivalent a "non principal").
        let mut fallback: Option<Monitor> = None;
        for m in monitors {
            if m.is_primary()
                .map_err(|e| ScreencapError::Xcap(e.to_string()))?
            {
                return Ok(m);
            }
            if fallback.is_none() {
                fallback = Some(m);
            }
        }
        fallback.ok_or_else(|| ScreencapError::Xcap("aucun moniteur detecte".into()))
    }
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
