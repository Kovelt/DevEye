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
    /// ou repli du mode `auto`). Absent dans ces modes : repli plein ecran
    /// (MonitorRenderer). En cible `x11`, c'est la fenetre pilotee
    /// (alias `--window`) ; sans elle, tout l'ecran du serveur X.
    #[arg(long, visible_alias = "window")]
    pub(crate) screencap_window: Option<String>,

    /// Display du serveur X PRIVE pilote par `--target x11` (ex. `:250`).
    /// Obligatoire pour cette cible, SANS valeur par defaut et sans repli sur
    /// `$DISPLAY` : refus des displays < 100, de celui de la session courante
    /// et de tout serveur XWayland (voir `adapter-x11`).
    #[arg(long, required_if_eq("target", "x11"))]
    pub(crate) display: Option<String>,

    /// Cible `x11` : sous-chaine qui doit en plus figurer dans `WM_CLASS` de la
    /// fenetre pilotee (departage deux fenetres de meme titre).
    #[arg(long)]
    pub(crate) x11_class: Option<String>,

    /// Backend de capture OS-level (yeux en mode `sdk` ou repli `auto`) :
    /// `auto` choisit KWin sous Wayland/KDE, xcap sinon ; `kwin`/`xcap` forcent.
    #[arg(long, value_enum, default_value_t = ScreencapBackend::Auto)]
    pub(crate) screencap_backend: ScreencapBackend,

    /// Cible des yeux et des mains :
    /// - `auto` : CDP d'abord, repli SDK + xcap des que le CDP est injoignable ;
    /// - `cdp`  : CDP uniquement (webview Chromium — `just ui-eyes`) ;
    /// - `sdk`  : pont natif uniquement (app desktop via `deveye-sdk`) ;
    /// - `kwin-eis` : mains OS-level sous Wayland/KDE (EIS) ;
    /// - `x11`  : serveur X prive explicite (`--display`) : yeux GetImage, mains
    ///   XTEST, fenetres QueryTree — pour toute app X11 native (egui, GTK, Qt...).
    #[arg(long, value_enum, default_value_t = TargetMode::Auto)]
    pub(crate) target: TargetMode,

    /// Sous-chaine (insensible a la casse) que doit contenir l'URL de la page
    /// CDP cible. Sans ce flag ni --cdp-target-filter-title, la cible est la
    /// premiere page qui n'est pas about:blank ; si aucune page ne convient,
    /// l'erreur liste les pages vues. Sert a viser une page precise quand
    /// plusieurs sont ouvertes (ex. `localhost:5000`).
    #[arg(long)]
    pub(crate) cdp_target_filter_url: Option<String>,

    /// Sous-chaine (insensible a la casse) que doit contenir le titre de la
    /// page CDP cible (relation OU avec --cdp-target-filter-url).
    #[arg(long)]
    pub(crate) cdp_target_filter_title: Option<String>,

    /// Accepte aussi la page "about:blank" comme cible CDP (transitoire d'une
    /// webview au boot). Desactive par defaut : un filtre ne doit pas s'attacher
    /// a l'about:blank d'une page avant son chargement.
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
    /// Pour les cibles sans WebView ni SDK (par exemple Uno desktop / Skia).
    KwinEis,
    /// Serveur X11 prive et explicite (`--display`, typiquement Xvfb) : yeux,
    /// mains et fenetres via `adapter-x11`, sans xcap ni KWin. Le DOM vient de
    /// la route `ui.widgets` de l'app (si elle l'expose), le state/logs/commandes
    /// du SDK.
    X11,
}

impl Args {
    /// Analyse la ligne de commande puis verifie les contraintes que clap ne
    /// sait pas exprimer ; quitte avec le message d'usage habituel en cas d'erreur.
    pub(crate) fn parse_checked() -> Self {
        let args = Self::parse();
        if let Err(e) = args.validate() {
            e.exit();
        }
        args
    }

    /// `--display` et `--x11-class` n'ont de sens qu'avec `--target x11` :
    /// les accepter ailleurs les ignorerait en silence.
    pub(crate) fn validate(&self) -> Result<(), clap::Error> {
        use clap::CommandFactory;
        if self.target == TargetMode::X11 {
            return Ok(());
        }
        for (set, flag) in [
            (self.display.is_some(), "--display"),
            (self.x11_class.is_some(), "--x11-class"),
        ] {
            if set {
                return Err(Self::command().error(
                    clap::error::ErrorKind::ArgumentConflict,
                    format!("{flag} n'est accepté qu'avec --target x11"),
                ));
            }
        }
        Ok(())
    }
}

impl TargetMode {
    /// `true` si la cible dispose d'un moteur JS a piloter (webview CDP ou pompe
    /// SDK) ; `false` pour les rendus natifs (`kwin-eis`, `x11`), ou
    /// `eval_js` et la console JS n'ont aucun sens.
    pub(crate) fn has_js(self) -> bool {
        match self {
            TargetMode::Auto | TargetMode::Cdp | TargetMode::Sdk => true,
            TargetMode::KwinEis | TargetMode::X11 => false,
        }
    }

    /// `true` si la cible peut s'attacher a un navigateur CDP.
    pub(crate) fn uses_cdp(self) -> bool {
        matches!(self, TargetMode::Auto | TargetMode::Cdp)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, clap::Error> {
        let a = Args::try_parse_from(std::iter::once("deveye-server").chain(args.iter().copied()))?;
        a.validate()?;
        Ok(a)
    }

    #[test]
    fn x11_requires_an_explicit_display() {
        let err = parse(&["--target", "x11"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
        assert!(err.to_string().contains("--display"));
        // Un window seul ne suffit pas, et $DISPLAY n'est jamais un repli.
        assert!(parse(&["--target", "x11", "--window", "Probe"]).is_err());
    }

    #[test]
    fn x11_accepts_display_window_and_class() {
        let a = parse(&[
            "--target",
            "x11",
            "--display",
            ":250",
            "--window",
            "Synth",
            "--x11-class",
            "synth",
        ])
        .unwrap();
        assert_eq!(a.target, TargetMode::X11);
        assert_eq!(a.display.as_deref(), Some(":250"));
        // `--window` est un alias de `--screencap-window`.
        assert_eq!(a.screencap_window.as_deref(), Some("Synth"));
        assert_eq!(a.x11_class.as_deref(), Some("synth"));
        let b = parse(&[
            "--target",
            "x11",
            "--display",
            ":250",
            "--screencap-window",
            "S",
        ])
        .unwrap();
        assert_eq!(b.screencap_window.as_deref(), Some("S"));
    }

    #[test]
    fn display_is_not_required_for_other_targets() {
        for t in ["auto", "cdp", "sdk", "kwin-eis"] {
            let a = parse(&["--target", t]).unwrap();
            assert!(a.display.is_none(), "{t}");
        }
    }

    #[test]
    fn x11_only_flags_are_rejected_for_other_targets() {
        for t in ["auto", "cdp", "sdk", "kwin-eis"] {
            let e = parse(&["--target", t, "--display", ":250"]).unwrap_err();
            assert_eq!(e.kind(), clap::error::ErrorKind::ArgumentConflict, "{t}");
            assert!(e.to_string().contains("--display"));
            let e = parse(&["--target", t, "--x11-class", "c"]).unwrap_err();
            assert!(e.to_string().contains("--x11-class"), "{t}");
        }
        // Sans --target (defaut auto) : meme refus.
        assert!(parse(&["--display", ":250"]).is_err());
    }

    #[test]
    fn js_and_cdp_capabilities_per_target() {
        for (t, js, cdp) in [
            (TargetMode::Auto, true, true),
            (TargetMode::Cdp, true, true),
            (TargetMode::Sdk, true, false),
            (TargetMode::KwinEis, false, false),
            (TargetMode::X11, false, false),
        ] {
            assert_eq!(t.has_js(), js, "{t:?}");
            assert_eq!(t.uses_cdp(), cdp, "{t:?}");
        }
    }
}
