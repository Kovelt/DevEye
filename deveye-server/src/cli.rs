//! Arguments CLI et enumerations de mode pour `deveye-server`.

use clap::{Parser, Subcommand};

/// URL CDP utilisee quand `--cdp-url` n'est pas donne.
pub(crate) const DEFAULT_CDP_URL: &str = "http://localhost:9222";

#[derive(Parser, Debug)]
#[command(
    name = "deveye-server",
    about = "Orchestrateur DevEye : compose les adapters et sert un MCP a Claude.",
    version = crate::version::clap_short(),
    long_version = crate::version::clap_long()
)]
pub(crate) struct Args {
    // Les options communes avec `doctor` sont `global` : `doctor --target x11`
    // et `--target x11 doctor` sont equivalents.
    /// URL HTTP du webview CDP cible (par defaut http://localhost:9222).
    /// Refuse avec les cibles `sdk`, `kwin-eis` et `x11` (aucun CDP).
    /// Option absente = valeur par défaut : voir [`Args::cdp_url`].
    #[arg(long, global = true)]
    pub(crate) cdp_url: Option<String>,

    /// URL du SDK opt-in expose par l'app cible (par defaut http://127.0.0.1:31337).
    /// On utilise 127.0.0.1 plutot que localhost : sur Fedora (et la plupart des
    /// Linux modernes) "localhost" se resout en ::1 (IPv6) alors que le SDK bind
    /// sur 127.0.0.1 uniquement => echec silencieux de connexion.
    #[arg(long, global = true, default_value = "http://127.0.0.1:31337")]
    pub(crate) sdk_url: String,

    /// Titre (sous-chaine, insensible a la casse) de la fenetre native a
    /// capturer via xcap quand les yeux ne passent pas par CDP (cible `sdk`,
    /// ou repli du mode `auto`). Absent dans ces modes : repli plein écran
    /// (MonitorRenderer). En cible `x11`, c'est la fenêtre pilotée
    /// (alias `--window`) ; sans elle, tout l'ecran du serveur X.
    #[arg(long, global = true, visible_alias = "window")]
    pub(crate) screencap_window: Option<String>,

    /// Display du serveur X PRIVE pilote par `--target x11` (ex. `:250`).
    /// Obligatoire pour cette cible, SANS valeur par defaut et sans repli sur
    /// `$DISPLAY` : refus des displays < 100, de celui de la session courante
    /// et de tout serveur XWayland (voir `adapter-x11`). Exigé par `validate`
    /// (et non par clap) pour que `doctor --target x11` puisse diagnostiquer
    /// l'absence de display au lieu d'être refusé avant de s'exécuter.
    #[arg(long, global = true)]
    pub(crate) display: Option<String>,

    /// Cible `x11` : sous-chaine qui doit en plus figurer dans `WM_CLASS` de la
    /// fenetre pilotee (departage deux fenetres de meme titre).
    #[arg(long, global = true)]
    pub(crate) x11_class: Option<String>,

    /// Backend de capture OS-level (yeux en mode `sdk` ou repli `auto`) :
    /// `auto` (defaut) choisit KWin sous Wayland/KDE, xcap sinon ; `kwin`/`xcap`
    /// forcent. Refuse avec la cible `x11` (qui n'utilise ni KWin ni xcap).
    /// Option absente = `auto` : voir [`Args::screencap_backend`].
    #[arg(long, value_enum, global = true)]
    pub(crate) screencap_backend: Option<ScreencapBackend>,

    /// Cible des yeux et des mains :
    /// - `auto` : CDP d'abord, repli SDK + xcap des que le CDP est injoignable ;
    /// - `cdp`  : CDP uniquement (webview Chromium — `just ui-eyes`) ;
    /// - `sdk`  : pont natif uniquement (app desktop via `deveye-sdk`) ;
    /// - `kwin-eis` : mains OS-level sous Wayland/KDE (EIS) ;
    /// - `x11`  : serveur X prive explicite (`--display`) : yeux GetImage, mains
    ///   XTEST, fenetres QueryTree — pour toute app X11 native (egui, GTK, Qt...).
    #[arg(long, value_enum, global = true, default_value_t = TargetMode::Auto)]
    pub(crate) target: TargetMode,

    /// Sous-chaine (insensible a la casse) que doit contenir l'URL de la page
    /// CDP cible. Sans ce flag ni --cdp-target-filter-title, la cible est la
    /// première page qui n'est pas about:blank ; si aucune page ne convient,
    /// l'erreur liste les pages vues. Sert a viser une page précise quand
    /// plusieurs sont ouvertes (ex. `localhost:5000`). Refuse hors cibles `auto`/`cdp`.
    #[arg(long)]
    pub(crate) cdp_target_filter_url: Option<String>,

    /// Sous-chaine (insensible a la casse) que doit contenir le titre de la
    /// page CDP cible (relation OU avec --cdp-target-filter-url). Refuse hors
    /// cibles `auto`/`cdp`.
    #[arg(long)]
    pub(crate) cdp_target_filter_title: Option<String>,

    /// Accepte aussi la page "about:blank" comme cible CDP (transitoire d'une
    /// webview au boot). Desactive par défaut : un filtre ne doit pas s'attacher
    /// a l'about:blank d'une page avant son chargement. Refuse hors cibles
    /// `auto`/`cdp`.
    #[arg(long, default_value_t = false)]
    pub(crate) cdp_accept_blank: bool,

    /// Servir le MCP server sur stdio (mode normal d'usage avec Claude).
    /// Obligatoire sans sous-commande : le binaire n'a pas d'autre mode.
    #[arg(long)]
    pub(crate) serve_mcp: bool,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// Sous-commandes de `deveye-server`.
#[derive(Subcommand, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    /// Diagnostique l'environnement pour la cible choisie (`--target`, défaut
    /// `auto`) : session, KWin, EIS, Xvfb, SDK, CDP, ports. Lecture seule, un
    /// statut et un conseil par verification ; code de sortie non nul si un
    /// prerequis bloquant manque.
    Doctor,
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
        if let Err(e) = args.validate().and_then(|()| args.require_mode()) {
            e.exit();
        }
        args
    }

    /// URL CDP effective (`--cdp-url` ou valeur par défaut).
    pub(crate) fn cdp_url(&self) -> &str {
        self.cdp_url.as_deref().unwrap_or(DEFAULT_CDP_URL)
    }

    /// Backend de capture effectif (`--screencap-backend` ou `auto`).
    pub(crate) fn screencap_backend(&self) -> ScreencapBackend {
        self.screencap_backend.unwrap_or(ScreencapBackend::Auto)
    }

    /// Refuse les options sans effet sur la cible choisie, plutôt que de les
    /// ignorer en silence (ADR-0031) :
    /// - `--display` et `--x11-class` : uniquement `--target x11` ;
    /// - `--cdp-url`, `--cdp-target-filter-*`, `--cdp-accept-blank` : uniquement
    ///   les cibles qui s'attachent a un navigateur (`auto`, `cdp`) ;
    /// - `--screencap-backend` : refuse avec `x11` (qui capture par GetImage).
    ///
    /// `--sdk-url` et `--screencap-window` servent toutes les cibles (state,
    /// logs, commandes ; fenetre) : jamais refuses.
    pub(crate) fn validate(&self) -> Result<(), clap::Error> {
        use clap::CommandFactory;
        let mut ignored: Vec<(&str, &str)> = Vec::new();
        if self.target == TargetMode::X11
            && self.display.is_none()
            && !matches!(self.command, Some(Command::Doctor))
        {
            return Err(Self::command().error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "--display est obligatoire avec --target x11 (serveur X privé, ex. --display :250)",
            ));
        }
        if self.target != TargetMode::X11 {
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
        }
        if !self.target.uses_cdp() {
            for (set, flag) in [
                (self.cdp_url.is_some(), "--cdp-url"),
                (
                    self.cdp_target_filter_url.is_some(),
                    "--cdp-target-filter-url",
                ),
                (
                    self.cdp_target_filter_title.is_some(),
                    "--cdp-target-filter-title",
                ),
                (self.cdp_accept_blank, "--cdp-accept-blank"),
            ] {
                if set {
                    ignored.push((flag, "n'est accepté qu'avec --target auto ou cdp"));
                }
            }
        }
        if self.target == TargetMode::X11 && self.screencap_backend.is_some() {
            ignored.push((
                "--screencap-backend",
                "n'a aucun effet avec --target x11 (capture GetImage)",
            ));
        }
        if let Some((flag, why)) = ignored.first() {
            return Err(Self::command().error(
                clap::error::ErrorKind::ArgumentConflict,
                format!("{flag} {why} ; cible actuelle : {}", self.target.name()),
            ));
        }
        Ok(())
    }

    /// Exige un mode : `--serve-mcp` ou une sous-commande, jamais les deux.
    /// Sans mode, le binaire composait puis sortait en silence avec le code 0 ;
    /// c'est maintenant une erreur d'usage (ADR-0031).
    pub(crate) fn require_mode(&self) -> Result<(), clap::Error> {
        use clap::CommandFactory;
        match (self.serve_mcp, self.command) {
            (true, Some(_)) => Err(Self::command().error(
                clap::error::ErrorKind::ArgumentConflict,
                "--serve-mcp ne se combine pas avec une sous-commande",
            )),
            (false, None) => Err(Self::command().error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "aucun mode : ajouter --serve-mcp (serveur MCP sur stdio) ou la \
                 sous-commande doctor",
            )),
            _ => Ok(()),
        }
    }
}

impl TargetMode {
    /// Nom de la cible tel qu'on le passe a `--target`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            TargetMode::Auto => "auto",
            TargetMode::Cdp => "cdp",
            TargetMode::Sdk => "sdk",
            TargetMode::KwinEis => "kwin-eis",
            TargetMode::X11 => "x11",
        }
    }

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
    fn doctor_diagnoses_x11_without_display() {
        let a = parse(&["--target", "x11", "doctor"]).unwrap();
        assert!(a.display.is_none());
        assert!(matches!(a.command, Some(Command::Doctor)));
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
    #[test]
    fn cdp_flags_are_refused_with_non_browser_targets() {
        for t in ["sdk", "kwin-eis"] {
            for flags in [
                vec!["--cdp-url", "http://localhost:9222"],
                vec!["--cdp-target-filter-url", "x"],
                vec!["--cdp-target-filter-title", "x"],
                vec!["--cdp-accept-blank"],
            ] {
                let mut argv = vec!["--target", t];
                argv.extend(flags.iter().copied());
                let e = parse(&argv).unwrap_err();
                assert_eq!(
                    e.kind(),
                    clap::error::ErrorKind::ArgumentConflict,
                    "{argv:?}"
                );
                let msg = e.to_string();
                assert!(msg.contains(flags[0]) && msg.contains(t), "{msg}");
            }
        }
        let e = parse(&[
            "--target",
            "x11",
            "--display",
            ":250",
            "--cdp-url",
            "http://h:1",
        ])
        .unwrap_err();
        assert!(e.to_string().contains("--cdp-url"));
    }

    #[test]
    fn cdp_flags_are_accepted_with_auto_and_cdp() {
        for t in ["auto", "cdp"] {
            let a = parse(&[
                "--target",
                t,
                "--cdp-url",
                "http://localhost:9333",
                "--cdp-target-filter-url",
                "x",
                "--cdp-accept-blank",
            ])
            .unwrap();
            assert_eq!(a.cdp_url(), "http://localhost:9333");
        }
        // Valeur par défaut quand l'option est absente.
        assert_eq!(parse(&[]).unwrap().cdp_url(), DEFAULT_CDP_URL);
    }

    #[test]
    fn screencap_backend_is_refused_only_with_x11() {
        let e = parse(&[
            "--target",
            "x11",
            "--display",
            ":250",
            "--screencap-backend",
            "xcap",
        ])
        .unwrap_err();
        assert!(e.to_string().contains("--screencap-backend"));
        // Les autres cibles s'en servent (yeux, géométrie et focus KWin).
        for t in ["auto", "cdp", "sdk", "kwin-eis"] {
            let a = parse(&["--target", t, "--screencap-backend", "xcap"]).unwrap();
            assert_eq!(a.screencap_backend(), ScreencapBackend::Xcap, "{t}");
        }
        assert_eq!(
            parse(&[]).unwrap().screencap_backend(),
            ScreencapBackend::Auto
        );
    }

    #[test]
    fn sdk_url_and_window_are_accepted_by_every_target() {
        for t in ["auto", "cdp", "sdk", "kwin-eis"] {
            parse(&[
                "--target",
                t,
                "--sdk-url",
                "http://127.0.0.1:1",
                "--window",
                "w",
            ])
            .unwrap();
        }
        parse(&[
            "--target",
            "x11",
            "--display",
            ":250",
            "--sdk-url",
            "http://127.0.0.1:1",
        ])
        .unwrap();
    }

    #[test]
    fn a_mode_is_required_and_modes_do_not_combine() {
        let none = parse(&["--target", "sdk"]).unwrap();
        let e = none.require_mode().unwrap_err();
        assert!(e.to_string().contains("--serve-mcp"));
        assert!(parse(&["--serve-mcp"]).unwrap().require_mode().is_ok());
        assert!(parse(&["doctor"]).unwrap().require_mode().is_ok());
        let both = parse(&["--serve-mcp", "doctor"]).unwrap();
        assert!(both.require_mode().is_err());
    }

    #[test]
    fn doctor_shares_the_global_options_before_or_after() {
        let a = parse(&[
            "doctor",
            "--target",
            "sdk",
            "--sdk-url",
            "http://127.0.0.1:2",
        ])
        .unwrap();
        assert_eq!(a.command, Some(Command::Doctor));
        assert_eq!(a.target, TargetMode::Sdk);
        let b = parse(&["--target", "sdk", "doctor"]).unwrap();
        assert_eq!(b.target, TargetMode::Sdk);
        // La validation par cible s'applique aussi au diagnostic.
        assert!(parse(&["doctor", "--target", "sdk", "--cdp-url", "http://h:1"]).is_err());
    }

    #[test]
    fn version_flag_prints_version_commit_and_protocols() {
        let e = Args::try_parse_from(["deveye-server", "--version"]).unwrap_err();
        assert_eq!(e.kind(), clap::error::ErrorKind::DisplayVersion);
        let text = e.to_string();
        assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
        assert!(text.contains("commit"), "{text}");
        assert!(
            text.contains("protocole SDK") && text.contains("protocole MCP"),
            "{text}"
        );
    }
}
