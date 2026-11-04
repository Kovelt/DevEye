//! Garde d'isolation : l'adapter ne doit JAMAIS piloter la session graphique
//! vivante de l'utilisateur (XTEST injecte de vrais evenements d'entree).
//!
//! Quatre refus, tous testables sans serveur X :
//!
//! 1. nom de display non local (`host:N`, chemin, vide...) : seuls `:N` et
//!    `:N.S` sont acceptes ;
//! 2. numero de display inferieur a [`MIN_DISPLAY_NUMBER`] (100) : les sessions
//!    reelles vivent sur `:0`/`:1`, un serveur prive se lance sur un numero
//!    haut (`Xvfb :250`) ;
//! 3. display identique a celui de la session courante (`$DISPLAY`) ;
//! 4. socket `/tmp/.X11-unix/X<n>` appartenant a un autre utilisateur.
//!
//! Apres connexion, un cinquieme refus : presence de l'extension `XWAYLAND`
//! (serveur X embarque d'un compositeur Wayland, donc session vivante) — un
//! Xvfb ou un Xorg prive ne l'annonce pas.
//!
//! Il n'existe volontairement AUCUN contournement (pas d'option du type
//! `--allow-live-display`) : pour piloter une session reelle, utiliser un autre
//! adapter (ex. `adapter-interaction-kwin-eis`).

use std::fmt;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use crate::error::X11Error;

/// Numero de display minimal accepte. Choix de securite, pas de protocole :
/// `Xvfb :99` (usage CI classique) est donc refuse, utiliser `:100` ou plus.
pub const MIN_DISPLAY_NUMBER: u32 = 100;

/// Dossier des sockets X11 locaux.
const SOCKET_DIR: &str = "/tmp/.X11-unix";

/// Display local analyse : `:N` ou `:N.S`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayId {
    pub number: u32,
    pub screen: u32,
}

impl fmt::Display for DisplayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ":{}.{}", self.number, self.screen)
    }
}

/// Raison d'un refus de la garde.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardError {
    /// Nom de display illisible ou non local.
    BadName(String),
    /// Numero inferieur au plancher de securite.
    BelowMinimum { number: u32 },
    /// Meme display que la session courante.
    SameAsSession(String),
    /// Socket possede par un autre utilisateur.
    ForeignOwner { number: u32, owner: u32, me: u32 },
    /// Serveur X embarque d'un compositeur Wayland (session vivante).
    Xwayland,
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuardError::BadName(raw) => write!(
                f,
                "display refusé « {raw} » : seul un display local `:N` ou `:N.S` est accepté"
            ),
            GuardError::BelowMinimum { number } => write!(
                f,
                "display :{number} refusé : numéro < {MIN_DISPLAY_NUMBER} (réservé aux sessions \
                 réelles) ; lancer un serveur privé, ex. `Xvfb :250`"
            ),
            GuardError::SameAsSession(d) => write!(
                f,
                "display {d} refusé : c'est celui de la session courante ($DISPLAY) ; cet \
                 adapter ne pilote jamais la session vivante"
            ),
            GuardError::ForeignOwner { number, owner, me } => write!(
                f,
                "display :{number} refusé : socket possédé par l'uid {owner} (nous : {me})"
            ),
            GuardError::Xwayland => write!(
                f,
                "display refusé : extension XWAYLAND détectée (serveur X d'un compositeur \
                 Wayland = session vivante) ; utiliser un serveur privé (Xvfb)"
            ),
        }
    }
}

impl std::error::Error for GuardError {}

/// Analyse un nom de display local (`:N` ou `:N.S`).
pub fn parse_display(raw: &str) -> Result<DisplayId, GuardError> {
    let bad = || GuardError::BadName(raw.to_string());
    let rest = raw.strip_prefix(':').ok_or_else(bad)?;
    let (num, screen) = match rest.split_once('.') {
        Some((n, s)) => (n, s),
        None => (rest, "0"),
    };
    // `parse::<u32>` accepte un `+` initial : on exige des chiffres purs.
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(num) || !digits(screen) {
        return Err(bad());
    }
    Ok(DisplayId {
        number: num.parse().map_err(|_| bad())?,
        screen: screen.parse().map_err(|_| bad())?,
    })
}

/// Verifications pures d'avant connexion (1 a 4 de la doc du module).
///
/// - `session_display` : valeur de `$DISPLAY` du processus (ou `None`) ;
/// - `socket_owner` : uid proprietaire de `/tmp/.X11-unix/X<n>` si le socket
///   existe ;
/// - `my_uid` : uid du processus.
pub fn check_display(
    raw: &str,
    session_display: Option<&str>,
    socket_owner: impl FnOnce(u32) -> Option<u32>,
    my_uid: u32,
) -> Result<DisplayId, GuardError> {
    let id = parse_display(raw)?;
    if id.number < MIN_DISPLAY_NUMBER {
        return Err(GuardError::BelowMinimum { number: id.number });
    }
    if let Some(session) = session_display.filter(|s| !s.is_empty()) {
        let same = match parse_display(session) {
            // Compare les NUMEROS de display : `:250` et `:250.0` sont le meme.
            Ok(s) => s.number == id.number,
            // `$DISPLAY` non local (ssh, hote:N) : comparaison textuelle.
            Err(_) => session == raw,
        };
        if same {
            return Err(GuardError::SameAsSession(raw.to_string()));
        }
    }
    if let Some(owner) = socket_owner(id.number) {
        if owner != my_uid {
            return Err(GuardError::ForeignOwner {
                number: id.number,
                owner,
                me: my_uid,
            });
        }
    }
    Ok(id)
}

/// Sonde d'un serveur X deja connecte (abstraite pour tester la garde sans
/// serveur reel).
pub trait X11Probe {
    /// `true` si le serveur annonce l'extension `name`.
    fn has_extension(&self, name: &str) -> Result<bool, X11Error>;
}

/// Verification post-connexion (5 de la doc du module).
pub(crate) fn check_probe(probe: &dyn X11Probe) -> Result<(), X11Error> {
    if probe.has_extension("XWAYLAND")? {
        return Err(GuardError::Xwayland.into());
    }
    Ok(())
}

/// uid proprietaire de `/tmp/.X11-unix/X<number>`, `None` si absent.
pub(crate) fn socket_owner(number: u32) -> Option<u32> {
    let path: PathBuf = [SOCKET_DIR, &format!("X{number}")].iter().collect();
    std::fs::metadata(path).ok().map(|m| m.uid())
}

/// uid du processus courant, sans `unsafe` ni dependance : proprietaire de
/// `/proc/self`.
pub(crate) fn current_uid() -> Option<u32> {
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// Garde complete d'avant connexion : lit `$DISPLAY` (UNIQUEMENT pour le
/// comparer, jamais pour choisir une cible), le proprietaire du socket et
/// l'uid courant.
pub(crate) fn vet_display(raw: &str) -> Result<DisplayId, GuardError> {
    let session = std::env::var("DISPLAY").ok();
    // Sans uid lisible (/proc absent), on ne peut pas prouver la propriete : on
    // refuse plutot que de laisser passer.
    let me = current_uid().ok_or_else(|| GuardError::BadName(format!("{raw} (uid illisible)")))?;
    check_display(raw, session.as_deref(), socket_owner, me)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: u32 = 1000;
    fn mine(_: u32) -> Option<u32> {
        Some(ME)
    }

    #[test]
    fn parses_local_displays() {
        assert_eq!(
            parse_display(":250").unwrap(),
            DisplayId {
                number: 250,
                screen: 0
            }
        );
        assert_eq!(
            parse_display(":251.1").unwrap(),
            DisplayId {
                number: 251,
                screen: 1
            }
        );
    }

    #[test]
    fn rejects_non_local_or_malformed_names() {
        for bad in [
            "",
            ":",
            "250",
            "host:250",
            "localhost:250.0",
            "unix:250",
            ":-1",
            ":+5",
            ":250.",
            ":.0",
            ":2x",
            ":250.0.0",
            "/tmp/.X11-unix/X250",
            ":99999999999",
        ] {
            assert!(
                matches!(parse_display(bad), Err(GuardError::BadName(_))),
                "{bad:?} aurait du etre refuse"
            );
        }
    }

    #[test]
    fn refuses_numbers_below_the_floor() {
        for n in [0, 1, 2, 99] {
            let r = check_display(&format!(":{n}"), None, mine, ME);
            assert_eq!(r, Err(GuardError::BelowMinimum { number: n }));
        }
        assert!(check_display(":100", None, mine, ME).is_ok());
        assert!(check_display(":250", None, mine, ME).is_ok());
    }

    #[test]
    fn refuses_the_session_display_even_with_other_spelling() {
        let r = check_display(":250", Some(":250"), mine, ME);
        assert!(matches!(r, Err(GuardError::SameAsSession(_))));
        let r = check_display(":250.0", Some(":250"), mine, ME);
        assert!(matches!(r, Err(GuardError::SameAsSession(_))));
        // Session reelle sur un autre display : accepte.
        assert!(check_display(":250", Some(":1"), mine, ME).is_ok());
        // `$DISPLAY` vide ou illisible : ne bloque pas un display different.
        assert!(check_display(":250", Some(""), mine, ME).is_ok());
        assert!(check_display(":250", Some("localhost:10.0"), mine, ME).is_ok());
    }

    #[test]
    fn refuses_a_foreign_socket_owner() {
        let r = check_display(":250", None, |_| Some(0), ME);
        assert_eq!(
            r,
            Err(GuardError::ForeignOwner {
                number: 250,
                owner: 0,
                me: ME
            })
        );
        // Socket absent : pas de refus (la connexion echouera d'elle-meme).
        assert!(check_display(":250", None, |_| None, ME).is_ok());
    }

    #[test]
    fn socket_owner_receives_the_display_number() {
        let mut seen = None;
        let _ = check_display(
            ":251",
            None,
            |n| {
                seen = Some(n);
                Some(ME)
            },
            ME,
        );
        assert_eq!(seen, Some(251));
    }

    struct FakeProbe(Vec<&'static str>);
    impl X11Probe for FakeProbe {
        fn has_extension(&self, name: &str) -> Result<bool, X11Error> {
            Ok(self.0.contains(&name))
        }
    }

    #[test]
    fn refuses_xwayland() {
        let r = check_probe(&FakeProbe(vec!["XTEST", "XWAYLAND"]));
        assert!(matches!(r, Err(X11Error::Guard(GuardError::Xwayland))));
        assert!(check_probe(&FakeProbe(vec!["XTEST", "RANDR"])).is_ok());
    }

    #[test]
    fn messages_name_the_remedy() {
        assert!(GuardError::BelowMinimum { number: 1 }
            .to_string()
            .contains("Xvfb"));
        assert!(GuardError::Xwayland.to_string().contains("XWAYLAND"));
    }
}
