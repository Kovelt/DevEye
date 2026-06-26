//! Arguments CLI et enumerations de mode pour `deveye-server`.

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "deveye-server",
    about = "Orchestrateur DevEye : compose les adapters et sert un MCP a Claude."
)]
pub(crate) struct Args {
    /// URL HTTP du webview CDP cible (par defaut http://localhost:9222).
    #[arg(long, default_value = "http://localhost:9222")]
    pub(crate) cdp_url: String,

    /// URL du SDK opt-in expose par l'app cible (par defaut http://127.0.0.1:31337).
    /// On utilise 127.0.0.1 plutot que localhost : sur Fedora (et la plupart des
    /// Linux modernes) "localhost" se resout en ::1 (IPv6) alors que le SDK bind
    /// sur 127.0.0.1 uniquement => echec silencieux de connexion.
    #[arg(long, default_value = "http://127.0.0.1:31337")]
    pub(crate) sdk_url: String,

    /// Titre (sous-chaine, insensible a la casse) de la fenetre native a
    /// capturer via xcap quand les yeux ne passent pas par CDP (cible `sdk`,
    /// ou repli du mode `auto`). Defaut : « chronos ».
    #[arg(long)]
    pub(crate) screencap_window: Option<String>,

    /// Backend de capture OS-level (yeux en mode `sdk` ou repli `auto`) :
    /// `auto` choisit KWin sous Wayland/KDE, xcap sinon ; `kwin`/`xcap` forcent.
    #[arg(long, value_enum, default_value_t = ScreencapBackend::Auto)]
    pub(crate) screencap_backend: ScreencapBackend,

    /// Cible des yeux et des mains :
    /// - `auto` : CDP d'abord, repli SDK + xcap des que le CDP est injoignable ;
    /// - `cdp`  : CDP uniquement (webview Chromium — `just ui-eyes`) ;
    /// - `sdk`  : pont natif uniquement (app desktop via `deveye-sdk`).
    #[arg(long, value_enum, default_value_t = TargetMode::Auto)]
    pub(crate) target: TargetMode,

    /// Sous-chaine (insensible a la casse) que doit contenir l'URL de la page
    /// CDP cible. Sans ce flag ni --cdp-target-filter-title, le filtre par
    /// defaut "dioxus" (comportement historique) s'applique. Sert a viser une
    /// tete WASM tierce (ex. `localhost:5000`) sans toucher l'app cible.
    #[arg(long)]
    pub(crate) cdp_target_filter_url: Option<String>,

    /// Sous-chaine (insensible a la casse) que doit contenir le titre de la
    /// page CDP cible (relation OU avec --cdp-target-filter-url).
    #[arg(long)]
    pub(crate) cdp_target_filter_title: Option<String>,

    /// Accepte la page "about:blank" comme cible CDP (transitoire wry au boot).
    /// Force a true avec le filtre par defaut Dioxus ; sinon false (un filtre
    /// explicite ne veut pas s'attacher au about:blank d'une tete WASM avant son
    /// chargement), sauf si ce flag est passe.
    #[arg(long, default_value_t = false)]
    pub(crate) cdp_accept_blank: bool,

    /// Servir le MCP server sur stdio (mode normal d'usage avec Claude).
    #[arg(long)]
    pub(crate) serve_mcp: bool,
}

/// Ou vont les yeux et les mains (le state et les logs viennent toujours du SDK).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetMode {
    Auto,
    Cdp,
    Sdk,
    /// Mains OS-level via KWin EIS (Wayland natif) ; yeux par screencap (M1).
    /// Pour les cibles sans WebView ni SDK (atlas Uno desktop / Skia).
    KwinEis,
}

/// Backend de capture OS-level (independant du backend CDP).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScreencapBackend {
    /// Detection auto : KWin sous Wayland/KDE, xcap sinon.
    Auto,
    /// KWin D-Bus `ScreenShot2` — Wayland natif KDE (pixel-perfect, sans XWayland).
    Kwin,
    /// xcap — X11, XWayland, ou Wayland non-KDE.
    Xcap,
}
