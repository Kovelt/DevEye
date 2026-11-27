//! Sous-commande `doctor` : diagnostic **en lecture seule** de l'environnement
//! pour la cible choisie (ADR-0032).
//!
//! Ce module est la partie pure : les faits observés ([`Probes`]) entrent, un
//! [`Report`] sort (décision, texte, code de sortie). La collecte des faits,
//! qui touche au système, est dans `doctor_probe.rs`.

use std::fmt::Write as _;

use crate::cli::TargetMode;

/// Verdict d'une vérification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    Ok,
    /// Utile à la cible mais non bloquant : la capacité concernée sera réduite.
    Warn,
    /// Prérequis bloquant de la cible demandée : code de sortie non nul.
    Fail,
    /// Sans objet pour cette cible.
    Skip,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Level::Ok => "[OK]",
            Level::Warn => "[ATTENTION]",
            Level::Fail => "[ÉCHEC]",
            Level::Skip => "[SANS OBJET]",
        }
    }
}

/// Une vérification : un statut, un détail, un conseil quand ce n'est pas `Ok`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Check {
    pub(crate) name: &'static str,
    pub(crate) level: Level,
    pub(crate) detail: String,
    pub(crate) advice: Option<String>,
}

/// Résultat complet du diagnostic.
#[derive(Debug)]
pub(crate) struct Report {
    pub(crate) target: TargetMode,
    pub(crate) checks: Vec<Check>,
}

impl Report {
    fn count(&self, level: Level) -> usize {
        self.checks.iter().filter(|c| c.level == level).count()
    }

    /// 0 si aucun prérequis bloquant ne manque, 1 sinon. Les avertissements ne
    /// font jamais échouer : ils décrivent des capacités réduites.
    pub(crate) fn exit_code(&self) -> i32 {
        i32::from(self.count(Level::Fail) > 0)
    }

    /// Rendu lisible : une ligne par vérification, le conseil en dessous.
    pub(crate) fn render(&self, version_line: &str) -> String {
        let mut out = String::new();
        // `write!` sur une String ne peut pas échouer.
        let _ = writeln!(
            out,
            "deveye-server {version_line} — diagnostic de la cible « {} » (lecture seule)\n",
            self.target.name()
        );
        for c in &self.checks {
            let _ = writeln!(out, "{:<13} {} : {}", c.level.tag(), c.name, c.detail);
            if let Some(advice) = &c.advice {
                let _ = writeln!(out, "{:<13} Conseil : {advice}", "");
            }
        }
        let (fails, warns) = (self.count(Level::Fail), self.count(Level::Warn));
        let verdict = match (fails, warns) {
            (0, 0) => "environnement prêt".to_string(),
            (0, w) => format!("utilisable, {w} avertissement(s) : capacités réduites"),
            (f, w) => format!("{f} prérequis bloquant(s) manquant(s), {w} avertissement(s)"),
        };
        let _ = write!(out, "\nRésumé : {verdict}.");
        out
    }
}

// ── Faits observés ────────────────────────────────────────────────────────────

/// État de KWin vu depuis ce processus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KwinProbe {
    NotRunning {
        /// `kwin_wayland`/`kwin_x11` présent dans le `PATH`.
        binary_installed: bool,
    },
    Running {
        /// `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS` dans l'environnement du
        /// processus KWin : `None` si illisible.
        no_permission_checks: Option<bool>,
    },
}

/// Interface D-Bus EIS de KWin (`org.kde.KWin.EIS.RemoteDesktop`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EisProbe {
    /// Introspection réussie : `connectToEIS` est exposé.
    Available,
    /// Le service ou l'interface est absent.
    Missing,
    /// Impossible de trancher (`busctl` absent, délai dépassé).
    Unknown(String),
}

/// Display de la cible `x11`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct X11Probe {
    pub(crate) display: String,
    /// `Ok(display normalisé)` ou le message de refus de la garde d'isolation.
    pub(crate) guard: Result<String, String>,
    /// Le socket `/tmp/.X11-unix/X<n>` existe.
    pub(crate) socket_exists: bool,
}

/// Réponse d'un service HTTP local (SDK ou CDP).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServiceProbe<T> {
    pub(crate) url: String,
    /// Le port TCP accepte les connexions.
    pub(crate) tcp_open: bool,
    /// Réponse décodée, ou la raison de l'échec.
    pub(crate) answer: Result<T, String>,
}

/// Ce que dit `GET /devtools/info`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SdkView {
    pub(crate) sdk_version: String,
    pub(crate) protocol_version: String,
    pub(crate) capabilities: Vec<String>,
}

/// Ce que dit `GET /json/version` du navigateur.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CdpView {
    pub(crate) browser: String,
    pub(crate) protocol: String,
}

/// Tous les faits observés par les sondes, sans aucune décision.
#[derive(Clone, Debug)]
pub(crate) struct Probes {
    pub(crate) session_type: Option<String>,
    pub(crate) desktop: Option<String>,
    pub(crate) kwin: KwinProbe,
    pub(crate) eis: EisProbe,
    pub(crate) xvfb_installed: bool,
    pub(crate) x11: Option<X11Probe>,
    pub(crate) sdk: ServiceProbe<SdkView>,
    pub(crate) cdp: ServiceProbe<CdpView>,
    /// Version du wire protocol que ce binaire implémente.
    pub(crate) supported_protocol: String,
}

impl Probes {
    fn is_wayland(&self) -> bool {
        self.session_type
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case("wayland"))
    }

    fn is_kde(&self) -> bool {
        self.desktop
            .as_deref()
            .is_some_and(|d| d.to_uppercase().contains("KDE"))
    }

    fn is_kde_wayland(&self) -> bool {
        self.is_wayland() && self.is_kde()
    }
}

// ── Décision ──────────────────────────────────────────────────────────────────

/// Importance d'une vérification pour une cible donnée.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Need {
    /// Sans elle, la cible ne fonctionne pas : un manque est un échec.
    Required,
    /// Elle enrichit la cible : un manque est un avertissement.
    Useful,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Session,
    Kwin,
    KwinPermissions,
    Eis,
    Xvfb,
    XDisplay,
    Sdk,
    Cdp,
}

const ALL_ITEMS: [Item; 8] = [
    Item::Session,
    Item::Kwin,
    Item::KwinPermissions,
    Item::Eis,
    Item::Xvfb,
    Item::XDisplay,
    Item::Sdk,
    Item::Cdp,
];

/// Table des prérequis par cible. `kde_wayland` : sous une autre session, les
/// yeux passent par xcap et KWin n'a pas à être sondé hors `kwin-eis`.
fn need(target: TargetMode, item: Item, kde_wayland: bool) -> Need {
    use Item::*;
    use TargetMode as T;
    match (target, item) {
        (T::KwinEis, Session | Kwin | KwinPermissions | Eis) => Need::Required,
        (T::KwinEis, Sdk) => Need::Useful,
        (T::KwinEis, _) => Need::NotApplicable,

        (T::X11, XDisplay) => Need::Required,
        (T::X11, Xvfb | Sdk) => Need::Useful,
        (T::X11, _) => Need::NotApplicable,

        (T::Cdp, Cdp) => Need::Required,
        (T::Auto, Cdp) => Need::Useful,
        (T::Sdk, Sdk) => Need::Required,
        (T::Auto | T::Cdp, Sdk) => Need::Useful,
        (T::Auto | T::Cdp | T::Sdk, Session) => Need::Useful,
        (T::Auto | T::Cdp | T::Sdk, Kwin | KwinPermissions) if kde_wayland => Need::Useful,
        (T::Auto | T::Cdp | T::Sdk, _) => Need::NotApplicable,
    }
}

/// Raison affichée pour une vérification sans objet.
fn skip_reason(target: TargetMode, item: Item, kde_wayland: bool) -> String {
    match item {
        Item::Session if target == TargetMode::X11 => {
            "la cible x11 pilote un serveur X privé, indépendant de la session".to_string()
        }
        Item::Kwin | Item::KwinPermissions | Item::Eis if !kde_wayland => {
            "session non KDE/Wayland : les yeux passent par xcap, KWin n'intervient pas".to_string()
        }
        Item::Eis => "seule la cible kwin-eis injecte les entrées par EIS".to_string(),
        Item::Xvfb | Item::XDisplay => "seule la cible x11 utilise un serveur X privé".to_string(),
        Item::Cdp => format!("la cible {} n'utilise pas de navigateur CDP", target.name()),
        _ => format!("sans objet pour la cible {}", target.name()),
    }
}

fn problem(
    need: Need,
    detail: impl Into<String>,
    advice: impl Into<String>,
) -> (Level, String, Option<String>) {
    let level = if need == Need::Required {
        Level::Fail
    } else {
        Level::Warn
    };
    (level, detail.into(), Some(advice.into()))
}

fn ok(detail: impl Into<String>) -> (Level, String, Option<String>) {
    (Level::Ok, detail.into(), None)
}

/// Compatibilité entre le protocole annoncé par le SDK et celui du serveur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Compat {
    Same,
    /// Même majeure, mineure différente : fonctionne, avec réserves.
    MinorDiffers,
    MajorDiffers,
    Unreadable,
}

/// Compare deux versions `MAJEURE.MINEURE`.
pub(crate) fn protocol_compat(announced: &str, supported: &str) -> Compat {
    fn parse(v: &str) -> Option<(u32, u32)> {
        let (major, minor) = v.trim().split_once('.')?;
        Some((major.parse().ok()?, minor.parse().ok()?))
    }
    match (parse(announced), parse(supported)) {
        (Some(a), Some(s)) if a == s => Compat::Same,
        (Some((am, _)), Some((sm, _))) if am == sm => Compat::MinorDiffers,
        (Some(_), Some(_)) => Compat::MajorDiffers,
        _ => Compat::Unreadable,
    }
}

/// Hôte et port d'une URL `http://hôte:port[/…]` (port par défaut du schéma :
/// 80 pour `http`, et pour une URL sans schéma). Les identifiants éventuels
/// (`user@hôte`) sont ignorés et un littéral IPv6 est rendu sans crochets.
pub(crate) fn host_port(url: &str) -> Option<(String, u16)> {
    let parsed = if url.contains("://") {
        reqwest::Url::parse(url).ok()?
    } else {
        reqwest::Url::parse(&format!("http://{url}")).ok()?
    };
    let host = parsed.host_str()?.trim_matches(['[', ']']).to_string();
    if host.is_empty() {
        return None;
    }
    Some((host, parsed.port_or_known_default()?))
}

/// La cible a-t-elle une raison de sonder le SDK / CDP ? (Sinon la décision
/// les marque « sans objet » : inutile d'attendre leur délai.)
/// Ces deux lignes de la table `need` ne dépendent pas de la session.
pub(crate) fn wants_sdk(target: TargetMode) -> bool {
    need(target, Item::Sdk, false) != Need::NotApplicable
}

pub(crate) fn wants_cdp(target: TargetMode) -> bool {
    need(target, Item::Cdp, false) != Need::NotApplicable
}

/// Décide, pour chaque vérification, de son statut et du conseil associé.
pub(crate) fn evaluate(target: TargetMode, p: &Probes) -> Report {
    let kde_wayland = p.is_kde_wayland();
    let checks = ALL_ITEMS
        .iter()
        .map(|&item| {
            let n = need(target, item, kde_wayland);
            let (level, detail, advice) = if n == Need::NotApplicable {
                (Level::Skip, skip_reason(target, item, kde_wayland), None)
            } else {
                decide(item, n, p)
            };
            Check {
                name: item_name(item),
                level,
                detail,
                advice,
            }
        })
        .collect();
    Report { target, checks }
}

fn item_name(item: Item) -> &'static str {
    match item {
        Item::Session => "Session graphique",
        Item::Kwin => "KWin",
        Item::KwinPermissions => "KWIN_SCREENSHOT_NO_PERMISSION_CHECKS",
        Item::Eis => "Accès EIS (entrées OS-level)",
        Item::Xvfb => "Xvfb",
        Item::XDisplay => "Display X privé",
        Item::Sdk => "SDK de l'app cible",
        Item::Cdp => "CDP (navigateur)",
    }
}

fn decide(item: Item, n: Need, p: &Probes) -> (Level, String, Option<String>) {
    match item {
        Item::Session => decide_session(n, p),
        Item::Kwin => match &p.kwin {
            KwinProbe::Running { .. } => ok("processus KWin en cours d'exécution"),
            KwinProbe::NotRunning { binary_installed } => problem(
                n,
                if *binary_installed {
                    "KWin est installé mais aucun processus kwin_wayland/kwin_x11 ne tourne"
                } else {
                    "KWin introuvable (ni processus, ni binaire dans le PATH)"
                },
                "lancer le diagnostic depuis la session KDE/Wayland à piloter",
            ),
        },
        Item::KwinPermissions => match &p.kwin {
            KwinProbe::Running {
                no_permission_checks: Some(true),
            } => ok("posée dans l'environnement de session (processus KWin ou ce processus)"),
            KwinProbe::Running {
                no_permission_checks: Some(false),
            } => problem(
                n,
                "absente de l'environnement du processus KWin : la capture d'écran par \
                 ScreenShot2 sera refusée",
                "poser KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1 dans l'environnement de session \
                 (ex. ~/.config/plasma-workspace/env/), puis rouvrir la session (docs/architecture.md)",
            ),
            // Illisible (KWin porte des privilèges : /proc/<pid>/environ est
            // protégé) : un doute n'est jamais un échec bloquant.
            KwinProbe::Running {
                no_permission_checks: None,
            } => problem(
                Need::Useful,
                "environnement du processus KWin illisible et variable absente de ce processus : non vérifiable",
                "si la capture est refusée, poser KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1 dans l'environnement de session (~/.config/plasma-workspace/env/) puis rouvrir la session",
            ),
            KwinProbe::NotRunning { .. } => problem(
                n,
                "non vérifiable : KWin ne tourne pas",
                "voir la vérification KWin",
            ),
        },
        Item::Eis => match &p.eis {
            EisProbe::Available => ok("org.kde.KWin.EIS.RemoteDesktop expose connectToEIS"),
            EisProbe::Missing => problem(
                n,
                "interface org.kde.KWin.EIS.RemoteDesktop absente du bus de session",
                "EIS exige KWin 6 sous Wayland natif ; mettre Plasma à jour",
            ),
            // Un fait non vérifiable est un avertissement, jamais un échec
            // (ADR-0032), même quand EIS est exigé par la cible.
            EisProbe::Unknown(why) => problem(
                Need::Useful,
                format!("non vérifiable ({why}) ; la connexion réelle n'est pas tentée par doctor"),
                "installer busctl (systemd) ou tester directement avec --target kwin-eis --serve-mcp",
            ),
        },
        Item::Xvfb => {
            let socket = p.x11.as_ref().is_some_and(|x| x.socket_exists);
            if p.xvfb_installed {
                ok("Xvfb présent dans le PATH")
            } else if socket {
                ok("Xvfb absent du PATH, mais le serveur X demandé tourne déjà")
            } else {
                problem(
                    n,
                    "Xvfb absent du PATH",
                    "installer Xvfb (ex. dnf install xorg-x11-server-Xvfb) pour lancer un serveur X privé",
                )
            }
        }
        Item::XDisplay => match &p.x11 {
            None => problem(n, "aucun --display fourni", "ajouter --display :250 (display ≥ 100)"),
            Some(x) => match &x.guard {
                Err(msg) => problem(
                    n,
                    format!("garde d'isolation : {msg}"),
                    "choisir un display privé ≥ 100, ex. `Xvfb :250 -nolisten tcp -screen 0 1400x1000x24 &`",
                ),
                Ok(id) if !x.socket_exists => problem(
                    n,
                    format!("display {id} accepté par la garde, mais /tmp/.X11-unix/ n'a pas de socket pour lui"),
                    format!("lancer le serveur : Xvfb {} -nolisten tcp -screen 0 1400x1000x24 &", x.display),
                ),
                Ok(id) => ok(format!("display {id} accepté par la garde, socket présent")),
            },
        },
        Item::Sdk => decide_sdk(n, p),
        Item::Cdp => decide_cdp(n, p),
    }
}

fn decide_session(n: Need, p: &Probes) -> (Level, String, Option<String>) {
    let session = p.session_type.as_deref().unwrap_or("inconnue");
    let desktop = p.desktop.as_deref().unwrap_or("inconnu");
    let summary = format!("session {session}, bureau {desktop}");
    if p.is_kde_wayland() {
        return ok(format!("{summary} : yeux et fenêtres par KWin"));
    }
    if n == Need::Required {
        return problem(
            n,
            format!("{summary} : la cible exige KDE Plasma sous Wayland natif"),
            "lancer depuis une session Plasma Wayland ; sinon viser une autre cible (--target x11 pour une app X11 dans Xvfb)",
        );
    }
    ok(format!("{summary} : yeux par xcap (KWin non utilisé)"))
}

fn decide_sdk(n: Need, p: &Probes) -> (Level, String, Option<String>) {
    let svc = &p.sdk;
    match &svc.answer {
        Ok(info) => {
            let id = format!(
                "SDK {} (protocole {}) sur {}",
                info.sdk_version, info.protocol_version, svc.url
            );
            match protocol_compat(&info.protocol_version, &p.supported_protocol) {
                Compat::MajorDiffers | Compat::Unreadable => problem(
                    n,
                    format!(
                        "{id} : protocole incompatible avec le {} de ce serveur",
                        p.supported_protocol
                    ),
                    "aligner les versions : mettre à jour deveye-sdk (ou le serveur) pour une même majeure",
                ),
                Compat::MinorDiffers => problem(
                    Need::Useful,
                    format!(
                        "{id} : mineure différente du {} de ce serveur (compatible, fonctions additives possibles)",
                        p.supported_protocol
                    ),
                    "aligner les versions quand c'est possible",
                ),
                Compat::Same => {
                    if n == Need::Required && !info.capabilities.iter().any(|c| c == "eval") {
                        problem(
                            n,
                            format!("{id} : capacité `eval` absente, la pompe JS n'est pas branchée"),
                            "appeler take_eval_requests / brancher la pompe d'évaluation dans l'app cible",
                        )
                    } else {
                        ok(format!("{id}, capacités : {}", info.capabilities.join(", ")))
                    }
                }
            }
        }
        Err(why) if svc.tcp_open => problem(
            n,
            format!("le port de {} est occupé mais ne répond pas comme un SDK DevEye ({why})", svc.url),
            "un autre service occupe ce port, ou le SDK est antérieur à la 1.0 : changer DEVEYE_PORT / --sdk-url",
        ),
        Err(_) => problem(
            n,
            format!("aucun SDK joignable sur {}", svc.url),
            "lancer l'app cible avec deveye-sdk initialisé (debug uniquement), ou corriger --sdk-url / DEVEYE_PORT",
        ),
    }
}

fn decide_cdp(n: Need, p: &Probes) -> (Level, String, Option<String>) {
    let svc = &p.cdp;
    match &svc.answer {
        Ok(v) => ok(format!(
            "{} (protocole CDP {}) sur {}",
            v.browser, v.protocol, svc.url
        )),
        Err(why) if svc.tcp_open => problem(
            n,
            format!("le port de {} est occupé mais ne répond pas comme un navigateur CDP ({why})", svc.url),
            "un autre service occupe ce port : vérifier --cdp-url et le port de débogage du navigateur",
        ),
        Err(_) => problem(
            n,
            format!("aucun navigateur CDP joignable sur {}", svc.url),
            "lancer le navigateur ou la webview avec --remote-debugging-port, ou corriger --cdp-url ; en mode auto le repli SDK prend le relais",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sdk_down() -> ServiceProbe<SdkView> {
        ServiceProbe {
            url: "http://127.0.0.1:31337".into(),
            tcp_open: false,
            answer: Err("connexion refusée".into()),
        }
    }

    fn cdp_down() -> ServiceProbe<CdpView> {
        ServiceProbe {
            url: "http://localhost:9222".into(),
            tcp_open: false,
            answer: Err("connexion refusée".into()),
        }
    }

    fn sdk_up(protocol: &str, caps: &[&str]) -> ServiceProbe<SdkView> {
        ServiceProbe {
            url: "http://127.0.0.1:31337".into(),
            tcp_open: true,
            answer: Ok(SdkView {
                sdk_version: "1.0.0".into(),
                protocol_version: protocol.into(),
                capabilities: caps.iter().map(|c| c.to_string()).collect(),
            }),
        }
    }

    fn base() -> Probes {
        Probes {
            session_type: Some("wayland".into()),
            desktop: Some("KDE".into()),
            kwin: KwinProbe::Running {
                no_permission_checks: Some(true),
            },
            eis: EisProbe::Available,
            xvfb_installed: true,
            x11: None,
            sdk: sdk_up("1.0", &["state", "commands", "info", "eval"]),
            cdp: cdp_down(),
            supported_protocol: "1.0".into(),
        }
    }

    fn level_of(report: &Report, name: &str) -> Level {
        report
            .checks
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.level)
            .unwrap_or_else(|| panic!("vérification `{name}` absente"))
    }

    #[test]
    fn every_target_has_one_status_per_check() {
        for t in [
            TargetMode::Auto,
            TargetMode::Cdp,
            TargetMode::Sdk,
            TargetMode::KwinEis,
            TargetMode::X11,
        ] {
            let r = evaluate(t, &base());
            assert_eq!(r.checks.len(), ALL_ITEMS.len(), "{t:?}");
            // Un conseil accompagne tout statut qui n'est ni Ok ni Skip.
            for c in &r.checks {
                assert_eq!(
                    c.advice.is_some(),
                    matches!(c.level, Level::Warn | Level::Fail),
                    "{t:?} / {}",
                    c.name
                );
            }
        }
    }

    #[test]
    fn kwin_eis_ready_session_exits_zero() {
        let r = evaluate(TargetMode::KwinEis, &base());
        assert_eq!(r.exit_code(), 0, "{}", r.render("t"));
        assert_eq!(level_of(&r, "Accès EIS (entrées OS-level)"), Level::Ok);
    }

    #[test]
    fn kwin_eis_blocks_without_the_permission_variable() {
        let mut p = base();
        p.kwin = KwinProbe::Running {
            no_permission_checks: Some(false),
        };
        let r = evaluate(TargetMode::KwinEis, &p);
        assert_eq!(r.exit_code(), 1);
        assert_eq!(
            level_of(&r, "KWIN_SCREENSHOT_NO_PERMISSION_CHECKS"),
            Level::Fail
        );
        // La même absence n'est qu'un avertissement pour une cible qui capture par KWin sans l'exiger.
        let r = evaluate(TargetMode::Sdk, &p);
        assert_eq!(r.exit_code(), 0);
        assert_eq!(
            level_of(&r, "KWIN_SCREENSHOT_NO_PERMISSION_CHECKS"),
            Level::Warn
        );
    }

    #[test]
    fn unreadable_kwin_environment_is_only_a_warning() {
        let mut p = base();
        p.kwin = KwinProbe::Running {
            no_permission_checks: None,
        };
        let r = evaluate(TargetMode::KwinEis, &p);
        assert_eq!(
            level_of(&r, "KWIN_SCREENSHOT_NO_PERMISSION_CHECKS"),
            Level::Warn
        );
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn kwin_eis_requires_kde_wayland() {
        let mut p = base();
        p.session_type = Some("x11".into());
        p.kwin = KwinProbe::NotRunning {
            binary_installed: false,
        };
        let r = evaluate(TargetMode::KwinEis, &p);
        assert_eq!(r.exit_code(), 1);
        assert_eq!(level_of(&r, "Session graphique"), Level::Fail);
    }

    #[test]
    fn non_kde_session_skips_kwin_checks_for_other_targets() {
        let mut p = base();
        p.session_type = Some("x11".into());
        p.desktop = Some("GNOME".into());
        p.kwin = KwinProbe::NotRunning {
            binary_installed: false,
        };
        let r = evaluate(TargetMode::Sdk, &p);
        assert_eq!(level_of(&r, "KWin"), Level::Skip);
        assert_eq!(level_of(&r, "Session graphique"), Level::Ok);
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn sdk_target_needs_a_reachable_sdk_with_eval() {
        let mut p = base();
        p.sdk = sdk_down();
        let r = evaluate(TargetMode::Sdk, &p);
        assert_eq!(r.exit_code(), 1);
        // Pour une autre cible, le SDK absent ne bloque pas.
        assert_eq!(evaluate(TargetMode::Auto, &p).exit_code(), 0);
        assert_eq!(
            level_of(&evaluate(TargetMode::Auto, &p), "SDK de l'app cible"),
            Level::Warn
        );

        p.sdk = sdk_up("1.0", &["state", "commands", "info"]);
        let r = evaluate(TargetMode::Sdk, &p);
        assert_eq!(level_of(&r, "SDK de l'app cible"), Level::Fail);
        assert!(r.render("t").contains("`eval` absente"));
    }

    #[test]
    fn busy_port_is_told_apart_from_a_dead_port() {
        let mut p = base();
        p.sdk = ServiceProbe {
            url: "http://127.0.0.1:31337".into(),
            tcp_open: true,
            answer: Err("route absente".into()),
        };
        let text = evaluate(TargetMode::Sdk, &p).render("t");
        assert!(text.contains("occupé"), "{text}");
        p.sdk = sdk_down();
        let text = evaluate(TargetMode::Sdk, &p).render("t");
        assert!(text.contains("aucun SDK joignable"), "{text}");
    }

    #[test]
    fn protocol_major_mismatch_fails_and_minor_only_warns() {
        let mut p = base();
        p.sdk = sdk_up("2.0", &["eval"]);
        let r = evaluate(TargetMode::Sdk, &p);
        assert_eq!(level_of(&r, "SDK de l'app cible"), Level::Fail);
        p.sdk = sdk_up("1.3", &["eval"]);
        let r = evaluate(TargetMode::Sdk, &p);
        assert_eq!(level_of(&r, "SDK de l'app cible"), Level::Warn);
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn unverifiable_eis_warns_without_failing_even_when_required() {
        let mut p = base();
        p.eis = EisProbe::Unknown("busctl absent".into());
        let r = evaluate(TargetMode::KwinEis, &p);
        assert_eq!(level_of(&r, "Accès EIS (entrées OS-level)"), Level::Warn);
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn cdp_is_required_for_cdp_useful_for_auto_and_skipped_elsewhere() {
        let p = base();
        assert_eq!(evaluate(TargetMode::Cdp, &p).exit_code(), 1);
        let auto = evaluate(TargetMode::Auto, &p);
        assert_eq!(auto.exit_code(), 0);
        assert_eq!(level_of(&auto, "CDP (navigateur)"), Level::Warn);
        assert_eq!(
            level_of(&evaluate(TargetMode::Sdk, &p), "CDP (navigateur)"),
            Level::Skip
        );
    }

    #[test]
    fn x11_requires_a_guarded_display_with_a_socket() {
        let mut p = base();
        p.x11 = Some(X11Probe {
            display: ":250".into(),
            guard: Ok(":250.0".into()),
            socket_exists: true,
        });
        assert_eq!(evaluate(TargetMode::X11, &p).exit_code(), 0);
        p.x11 = Some(X11Probe {
            display: ":250".into(),
            guard: Ok(":250.0".into()),
            socket_exists: false,
        });
        let r = evaluate(TargetMode::X11, &p);
        assert_eq!(r.exit_code(), 1);
        assert!(r.render("t").contains("Xvfb :250"));
        p.x11 = Some(X11Probe {
            display: ":0".into(),
            guard: Err("numéro < 100".into()),
            socket_exists: true,
        });
        assert_eq!(evaluate(TargetMode::X11, &p).exit_code(), 1);
    }

    #[test]
    fn missing_xvfb_only_warns_unless_a_server_already_runs() {
        let mut p = base();
        p.xvfb_installed = false;
        p.x11 = Some(X11Probe {
            display: ":250".into(),
            guard: Ok(":250.0".into()),
            socket_exists: false,
        });
        assert_eq!(
            level_of(&evaluate(TargetMode::X11, &p), "Xvfb"),
            Level::Warn
        );
        p.x11 = Some(X11Probe {
            display: ":250".into(),
            guard: Ok(":250.0".into()),
            socket_exists: true,
        });
        assert_eq!(level_of(&evaluate(TargetMode::X11, &p), "Xvfb"), Level::Ok);
    }

    #[test]
    fn render_has_one_status_and_one_advice_per_problem() {
        let mut p = base();
        p.sdk = sdk_down();
        let text = evaluate(TargetMode::Sdk, &p).render("1.0.0 (commit abc)");
        assert!(text.starts_with("deveye-server 1.0.0 (commit abc)"));
        assert!(text.contains("[ÉCHEC]"));
        assert_eq!(text.matches("Conseil :").count(), 1, "{text}");
        assert!(text.contains("Résumé : 1 prérequis bloquant(s)"), "{text}");
    }

    #[test]
    fn protocol_compat_cases() {
        assert_eq!(protocol_compat("1.0", "1.0"), Compat::Same);
        assert_eq!(protocol_compat("1.2", "1.0"), Compat::MinorDiffers);
        assert_eq!(protocol_compat("2.0", "1.0"), Compat::MajorDiffers);
        assert_eq!(protocol_compat("abc", "1.0"), Compat::Unreadable);
        assert_eq!(protocol_compat("1", "1.0"), Compat::Unreadable);
    }

    #[test]
    fn only_browser_targets_probe_cdp() {
        for t in [TargetMode::Sdk, TargetMode::KwinEis, TargetMode::X11] {
            assert!(!wants_cdp(t), "{t:?}");
            assert!(wants_sdk(t), "{t:?}");
        }
        assert!(wants_cdp(TargetMode::Auto) && wants_cdp(TargetMode::Cdp));
    }

    #[test]
    fn host_port_parsing() {
        assert_eq!(
            host_port("http://127.0.0.1:31337"),
            Some(("127.0.0.1".into(), 31337))
        );
        assert_eq!(
            host_port("http://localhost:9222/json"),
            Some(("localhost".into(), 9222))
        );
        assert_eq!(
            host_port("http://exemple.test"),
            Some(("exemple.test".into(), 80))
        );
        assert_eq!(host_port("http://"), None);
        assert_eq!(host_port("http://hote:abc"), None);
        assert_eq!(host_port("http://[::1]"), Some(("::1".into(), 80)));
        assert_eq!(
            host_port("http://[::1]:31337/x"),
            Some(("::1".into(), 31337))
        );
        assert_eq!(
            host_port("http://user:pw@127.0.0.1:9222"),
            Some(("127.0.0.1".into(), 9222))
        );
        assert_eq!(
            host_port("localhost:9222"),
            Some(("localhost".into(), 9222))
        );
        assert_eq!(
            host_port("https://exemple.test"),
            Some(("exemple.test".into(), 443))
        );
    }
}
