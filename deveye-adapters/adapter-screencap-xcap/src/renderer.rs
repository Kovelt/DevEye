use std::io::Cursor;

use async_trait::async_trait;
use image::ImageFormat;
use xcap::{Monitor, Window};

use deveye_core::{Screenshot, Size};
use deveye_ports::{PortError, PortResult, Renderer};

use crate::error::ScreencapError;

/// Capture la fenêtre dont le titre contient la sous-chaîne fournie.
///
/// Utilise [`xcap`](https://crates.io/crates/xcap) — multi-OS (Windows,
/// macOS, Linux X11+Wayland). Aucune dépendance à un webview, fonctionne sur
/// n'importe quelle app graphique. C'est le fallback universel quand CDP
/// n'est pas disponible (macOS WKWebView, app legacy MAUI, jeu, etc.).
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
            .find(|w| w.title().to_lowercase().contains(&needle))
            .ok_or_else(|| ScreencapError::WindowNotFound(self.title_substring.clone()))
    }
}

#[async_trait]
impl Renderer for XcapRenderer {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        let renderer = self.clone();
        tokio::task::spawn_blocking(move || {
            let window = renderer.find_window()?;
            let img = window
                .capture_image()
                .map_err(|e| ScreencapError::Xcap(e.to_string()))?;
            let viewport = Size::new(img.width(), img.height());
            let mut buf = Vec::new();
            img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
                .map_err(|e| ScreencapError::Encoding(e.to_string()))?;
            Ok::<_, ScreencapError>(Screenshot::new_png(buf, viewport))
        })
        .await
        .map_err(|e| PortError::from(ScreencapError::Xcap(format!("spawn_blocking : {e}"))))?
        .map_err(PortError::from)
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        let renderer = self.clone();
        tokio::task::spawn_blocking(move || {
            let window = renderer.find_window()?;
            Ok::<_, ScreencapError>(Size::new(window.width(), window.height()))
        })
        .await
        .map_err(|e| PortError::from(ScreencapError::Xcap(format!("spawn_blocking : {e}"))))?
        .map_err(PortError::from)
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
        Monitor::all()
            .map_err(|e| ScreencapError::Xcap(e.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| ScreencapError::Xcap("aucun moniteur détecté".into()))
    }
}

#[async_trait]
impl Renderer for MonitorRenderer {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        tokio::task::spawn_blocking(move || {
            let monitor = MonitorRenderer::primary()?;
            let img = monitor
                .capture_image()
                .map_err(|e| ScreencapError::Xcap(e.to_string()))?;
            let viewport = Size::new(img.width(), img.height());
            let mut buf = Vec::new();
            img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
                .map_err(|e| ScreencapError::Encoding(e.to_string()))?;
            Ok::<_, ScreencapError>(Screenshot::new_png(buf, viewport))
        })
        .await
        .map_err(|e| PortError::from(ScreencapError::Xcap(format!("spawn_blocking : {e}"))))?
        .map_err(PortError::from)
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        tokio::task::spawn_blocking(move || {
            let monitor = MonitorRenderer::primary()?;
            Ok::<_, ScreencapError>(Size::new(monitor.width(), monitor.height()))
        })
        .await
        .map_err(|e| PortError::from(ScreencapError::Xcap(format!("spawn_blocking : {e}"))))?
        .map_err(PortError::from)
    }
}
