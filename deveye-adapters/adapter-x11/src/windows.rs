//! Decouverte des fenetres X11 par `QueryTree` recursif depuis la racine.
//!
//! Aucun gestionnaire de fenetres requis : on n'utilise ni
//! `_NET_CLIENT_LIST_STACKING` (vide sans WM) ni EWMH. Un titre peut etre pose
//! tard par l'application (baseview, GLFW...) : [`find_window`] reessaie
//! jusqu'a un delai.
//!
//! La logique de selection ([`Matcher`], [`pick_window`]) est pure et testee
//! sur des arbres construits a la main ; seules les fonctions prenant une
//! [`Session`] parlent au serveur.

use std::time::{Duration, Instant};

use x11rb::errors::ReplyError;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, MapState, Window as XWindow};

use deveye_core::{WindowInfo, WindowRect};

use crate::conn::Session;
use crate::error::X11Error;

/// Profondeur maximale explorée sous la racine (racine = 0). Quatre niveaux
/// couvrent fenetre applicative, cadre de WM et enfant GL.
const MAX_DEPTH: u32 = 4;
/// Garde-fou sur le nombre de fenetres lues par balayage.
const MAX_NODES: usize = 4096;
/// Delai de resolution par defaut (le titre peut arriver apres le mapping).
pub(crate) const RESOLVE_TIMEOUT: Duration = Duration::from_secs(2);
const RESOLVE_POLL: Duration = Duration::from_millis(100);

/// Une fenetre vue lors du balayage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WinNode {
    pub id: u32,
    /// Fenetre mappee ET visible (`map_state == Viewable`).
    pub viewable: bool,
    /// Origine absolue (coordonnees racine) calculee par cumul des parents.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub net_wm_name: Option<String>,
    pub wm_name: Option<String>,
    /// `WM_CLASS` : « instance classe ».
    pub wm_class: Option<String>,
}

impl WinNode {
    /// Titre lisible : `_NET_WM_NAME` d'abord, puis `WM_NAME`.
    pub fn title(&self) -> &str {
        [&self.net_wm_name, &self.wm_name]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .find(|s| !s.is_empty())
            .unwrap_or("")
    }

    fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// Critere de selection d'une fenetre : sous-chaines insensibles a la casse.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Matcher {
    title: Option<String>,
    class: Option<String>,
}

impl Matcher {
    /// `title` : sous-chaine cherchee dans `_NET_WM_NAME`, puis `WM_NAME`, puis
    /// `WM_CLASS`. `class` : sous-chaine qui doit en plus figurer dans
    /// `WM_CLASS`. Une chaine vide equivaut a « absent ».
    pub fn new(title: Option<&str>, class: Option<&str>) -> Self {
        let norm = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(str::to_lowercase);
        Self {
            title: norm(title),
            class: norm(class),
        }
    }

    /// `true` si aucun critere : designe « tout l'ecran » (racine).
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.class.is_none()
    }

    /// Rang de correspondance (0 = `_NET_WM_NAME`, 1 = `WM_NAME`, 2 = `WM_CLASS`),
    /// `None` si la fenetre ne correspond pas.
    fn rank(&self, node: &WinNode) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        let has = |field: &Option<String>, needle: &str| {
            field
                .as_deref()
                .is_some_and(|v| v.to_lowercase().contains(needle))
        };
        if let Some(class) = &self.class {
            if !has(&node.wm_class, class) {
                return None;
            }
        }
        match &self.title {
            None => Some(2),
            Some(t) => {
                if has(&node.net_wm_name, t) {
                    Some(0)
                } else if has(&node.wm_name, t) {
                    Some(1)
                } else if has(&node.wm_class, t) {
                    Some(2)
                } else {
                    None
                }
            }
        }
    }
}

/// Choisit la fenetre cible dans `nodes`.
///
/// Ignore les fenetres non visibles et celles d'aire nulle. Entre plusieurs
/// candidats : meilleur rang de correspondance, puis plus grande aire, puis plus
/// petit identifiant (resultat deterministe).
pub fn pick_window(nodes: &[WinNode], matcher: &Matcher) -> Option<u32> {
    nodes
        .iter()
        .filter(|n| n.viewable && n.area() > 0)
        .filter_map(|n| matcher.rank(n).map(|r| (r, n)))
        .min_by_key(|(r, n)| (*r, std::cmp::Reverse(n.area()), n.id))
        .map(|(_, n)| n.id)
}

/// Fenetres a exposer par `list_windows` : visibles, d'aire utile et
/// identifiables (titre ou classe), hors fenetres techniques 1x1.
pub fn listable(nodes: &[WinNode]) -> Vec<&WinNode> {
    nodes
        .iter()
        .filter(|n| n.viewable && n.width > 1 && n.height > 1)
        .filter(|n| !n.title().is_empty() || n.wm_class.as_deref().is_some_and(|c| !c.is_empty()))
        .collect()
}

/// Projection `WindowInfo` (identifiant hexadecimal X, `url` vide, `kind` fixe).
pub fn to_window_info(n: &WinNode) -> WindowInfo {
    WindowInfo {
        id: format!("0x{:x}", n.id),
        title: n.title().to_string(),
        url: String::new(),
        kind: "x11-window".to_string(),
    }
}

/// Decode une propriete texte : `UTF8_STRING` (UTF-8) ou `STRING` (Latin-1).
fn decode_text(value: &[u8], utf8: bool) -> Option<String> {
    let s = if utf8 {
        String::from_utf8_lossy(value).into_owned()
    } else {
        value.iter().map(|&b| char::from(b)).collect()
    };
    let s = s.trim_end_matches('\0').to_string();
    (!s.is_empty()).then_some(s)
}

/// `WM_CLASS` = deux chaines terminees par NUL : « instance classe ».
pub(crate) fn decode_wm_class(value: &[u8]) -> Option<String> {
    let parts: Vec<String> = value
        .split(|&b| b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| p.iter().map(|&b| char::from(b)).collect())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Erreur X d'une fenetre individuelle (detruite entre deux requetes) :
/// on l'ignore ; une coupure de transport remonte.
fn skip_x_error<T>(r: Result<T, ReplyError>) -> Result<Option<T>, X11Error> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(ReplyError::X11Error(_)) => Ok(None),
        Err(ReplyError::ConnectionError(c)) => Err(c.into()),
    }
}

/// Balaye l'arbre des fenetres sous la racine.
pub(crate) fn snapshot(session: &Session) -> Result<Vec<WinNode>, X11Error> {
    let conn = &session.conn;
    let mut out: Vec<WinNode> = Vec::new();
    // (identifiant, origine absolue) des fenetres dont on explore les enfants.
    let mut frontier: Vec<(XWindow, (i32, i32))> = vec![(session.root, (0, 0))];

    for depth in 0..MAX_DEPTH {
        let trees: Vec<_> = frontier
            .iter()
            .map(|(w, origin)| conn.query_tree(*w).map(|c| (c, *origin)))
            .collect::<Result<_, _>>()?;
        let mut children: Vec<(XWindow, (i32, i32))> = Vec::new();
        for (cookie, origin) in trees {
            if let Some(tree) = skip_x_error(cookie.reply())? {
                children.extend(tree.children.into_iter().map(|c| (c, origin)));
            }
        }
        if children.is_empty() || out.len() + children.len() > MAX_NODES {
            break;
        }

        // Toutes les requetes d'abord (pipeline), les reponses ensuite.
        let mut pending = Vec::with_capacity(children.len());
        for (win, origin) in &children {
            pending.push((
                *win,
                *origin,
                conn.get_window_attributes(*win)?,
                conn.get_geometry(*win)?,
                conn.get_property(
                    false,
                    *win,
                    session.atoms.net_wm_name,
                    AtomEnum::ANY,
                    0,
                    256,
                )?,
                conn.get_property(false, *win, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 256)?,
                conn.get_property(false, *win, AtomEnum::WM_CLASS, AtomEnum::ANY, 0, 256)?,
            ));
        }
        frontier.clear();
        for (win, origin, attrs, geom, net, name, class) in pending {
            let (Some(attrs), Some(geom)) =
                (skip_x_error(attrs.reply())?, skip_x_error(geom.reply())?)
            else {
                continue;
            };
            let viewable = attrs.map_state == MapState::VIEWABLE;
            let abs = (
                origin.0.saturating_add(i32::from(geom.x)),
                origin.1.saturating_add(i32::from(geom.y)),
            );
            let net_wm_name = skip_x_error(net.reply())?
                .and_then(|p| decode_text(&p.value, p.type_ == session.atoms.utf8_string));
            let wm_name = skip_x_error(name.reply())?
                .and_then(|p| decode_text(&p.value, p.type_ == session.atoms.utf8_string));
            let wm_class = skip_x_error(class.reply())?.and_then(|p| decode_wm_class(&p.value));
            out.push(WinNode {
                id: win,
                viewable,
                x: abs.0,
                y: abs.1,
                width: u32::from(geom.width),
                height: u32::from(geom.height),
                net_wm_name,
                wm_name,
                wm_class,
            });
            if viewable && depth + 1 < MAX_DEPTH {
                frontier.push((win, abs));
            }
        }
        if frontier.is_empty() {
            break;
        }
    }
    Ok(out)
}

/// Resout la fenetre cible, en reessayant jusqu'a `wait` (titre pose tard).
pub(crate) fn find_window(
    session: &Session,
    matcher: &Matcher,
    wait: Duration,
) -> Result<u32, X11Error> {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(id) = pick_window(&snapshot(session)?, matcher) {
            return Ok(id);
        }
        if Instant::now() >= deadline {
            return Err(X11Error::NoWindow(format!(
                "aucune fenêtre visible ne correspond à {} (attendu {} ms)",
                describe(matcher),
                wait.as_millis()
            )));
        }
        std::thread::sleep(RESOLVE_POLL);
    }
}

fn describe(m: &Matcher) -> String {
    match (&m.title, &m.class) {
        (Some(t), Some(c)) => format!("titre « {t} » et classe « {c} »"),
        (Some(t), None) => format!("« {t} »"),
        (None, Some(c)) => format!("classe « {c} »"),
        (None, None) => "(aucun critère)".to_string(),
    }
}

/// Origine absolue d'une fenetre, relue a chaque appel (la fenetre peut avoir
/// bouge depuis la derniere action ; aucune valeur n'est mise en cache).
pub(crate) fn origin_of(session: &Session, win: XWindow) -> Result<(i32, i32), X11Error> {
    let r = session
        .conn
        .translate_coordinates(win, session.root, 0, 0)?
        .reply()?;
    Ok((i32::from(r.dst_x), i32::from(r.dst_y)))
}

/// Rectangle ecran d'une fenetre : origine absolue + taille.
pub(crate) fn rect_of(session: &Session, win: XWindow) -> Result<WindowRect, X11Error> {
    let (x, y) = origin_of(session, win)?;
    let g = session.conn.get_geometry(win)?.reply()?;
    Ok(WindowRect::new(
        x,
        y,
        u32::from(g.width),
        u32::from(g.height),
    ))
}

/// Verifie qu'une fenetre mise en cache existe encore et est visible.
pub(crate) fn still_viewable(session: &Session, win: XWindow) -> Result<bool, X11Error> {
    match session.conn.get_window_attributes(win)?.reply() {
        Ok(a) => Ok(a.map_state == MapState::VIEWABLE),
        Err(ReplyError::X11Error(_)) => Ok(false),
        Err(ReplyError::ConnectionError(c)) => Err(c.into()),
    }
}

/// Titre courant d'une fenetre (pour `activate`).
pub(crate) fn title_of(session: &Session, win: XWindow) -> Result<String, X11Error> {
    let conn = &session.conn;
    let net = conn
        .get_property(false, win, session.atoms.net_wm_name, AtomEnum::ANY, 0, 256)?
        .reply()?;
    if let Some(t) = decode_text(&net.value, true) {
        return Ok(t);
    }
    let name = conn
        .get_property(false, win, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 256)?
        .reply()?;
    Ok(decode_text(&name.value, false).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u32, w: u32, h: u32) -> WinNode {
        WinNode {
            id,
            viewable: true,
            width: w,
            height: h,
            ..WinNode::default()
        }
    }

    fn titled(id: u32, w: u32, h: u32, title: &str) -> WinNode {
        WinNode {
            net_wm_name: Some(title.to_string()),
            ..node(id, w, h)
        }
    }

    #[test]
    fn picks_by_case_insensitive_substring() {
        let tree = [
            titled(1, 100, 100, "Other"),
            titled(2, 100, 100, "My Probe 1.0"),
        ];
        assert_eq!(
            pick_window(&tree, &Matcher::new(Some("probe"), None)),
            Some(2)
        );
    }

    #[test]
    fn skips_non_viewable_and_empty_area() {
        let mut hidden = titled(1, 800, 600, "Probe");
        hidden.viewable = false;
        let flat = titled(2, 0, 900, "Probe");
        let ok = titled(3, 10, 10, "Probe");
        assert_eq!(
            pick_window(&[hidden, flat, ok], &Matcher::new(Some("Probe"), None)),
            Some(3)
        );
    }

    #[test]
    fn empty_title_does_not_match() {
        let tree = [titled(1, 100, 100, ""), node(2, 100, 100)];
        assert_eq!(pick_window(&tree, &Matcher::new(Some("x"), None)), None);
    }

    #[test]
    fn empty_matcher_matches_nothing() {
        let m = Matcher::new(Some(""), None);
        assert!(m.is_empty());
        assert_eq!(pick_window(&[titled(1, 10, 10, "a")], &m), None);
    }

    #[test]
    fn title_field_priority_then_area_then_id() {
        // 1 : correspond par classe (rang 2) mais tres grande ; 2 : par
        // _NET_WM_NAME (rang 0) mais petite -> 2 gagne par le rang.
        let by_class = WinNode {
            wm_class: Some("probe Probe".into()),
            ..node(1, 1000, 1000)
        };
        let by_net = titled(2, 10, 10, "Probe");
        let m = Matcher::new(Some("probe"), None);
        assert_eq!(pick_window(&[by_class.clone(), by_net], &m), Some(2));
        // WM_NAME (rang 1) bat WM_CLASS (rang 2).
        let by_wm = WinNode {
            wm_name: Some("probe".into()),
            ..node(3, 5, 5)
        };
        assert_eq!(pick_window(&[by_class, by_wm], &m), Some(3));
        // Meme rang : la plus grande, puis le plus petit id.
        let tree = [
            titled(7, 100, 100, "bm"),
            titled(5, 200, 100, "bm"),
            titled(4, 200, 100, "bm"),
        ];
        assert_eq!(pick_window(&tree, &Matcher::new(Some("bm"), None)), Some(4));
    }

    #[test]
    fn class_filter_is_an_extra_constraint() {
        let a = WinNode {
            wm_class: Some("foo Foo".into()),
            ..titled(1, 100, 100, "Synth")
        };
        let b = WinNode {
            wm_class: Some("bar Bar".into()),
            ..titled(2, 100, 100, "Synth")
        };
        assert_eq!(
            pick_window(
                &[a.clone(), b.clone()],
                &Matcher::new(Some("synth"), Some("bar"))
            ),
            Some(2)
        );
        // Classe seule, sans titre.
        assert_eq!(
            pick_window(&[a, b], &Matcher::new(None, Some("foo"))),
            Some(1)
        );
    }

    #[test]
    fn title_prefers_net_wm_name_over_wm_name() {
        let n = WinNode {
            net_wm_name: Some("net".into()),
            wm_name: Some("old".into()),
            ..node(1, 1, 1)
        };
        assert_eq!(n.title(), "net");
        let n = WinNode {
            net_wm_name: Some(String::new()),
            wm_name: Some("old".into()),
            ..node(1, 1, 1)
        };
        assert_eq!(n.title(), "old");
        assert_eq!(node(1, 1, 1).title(), "");
    }

    #[test]
    fn listable_hides_anonymous_and_tiny_windows() {
        let anon = node(1, 500, 500);
        let tiny = titled(2, 1, 1, "x");
        let hidden = WinNode {
            viewable: false,
            ..titled(3, 100, 100, "h")
        };
        let ok = titled(4, 100, 100, "ok");
        let classed = WinNode {
            wm_class: Some("a A".into()),
            ..node(5, 100, 100)
        };
        let tree = [anon, tiny, hidden, ok, classed];
        let ids: Vec<u32> = listable(&tree).iter().map(|n| n.id).collect();
        assert_eq!(ids, vec![4, 5]);
    }

    #[test]
    fn window_info_projection() {
        let info = to_window_info(&titled(0x2a, 10, 10, "T"));
        assert_eq!(info.id, "0x2a");
        assert_eq!(info.title, "T");
        assert_eq!(info.kind, "x11-window");
        assert!(info.url.is_empty());
    }

    #[test]
    fn decodes_text_properties() {
        assert_eq!(decode_text(b"abc\0", true).as_deref(), Some("abc"));
        assert_eq!(decode_text("é".as_bytes(), true).as_deref(), Some("é"));
        // STRING = Latin-1 : 0xE9 = é.
        assert_eq!(decode_text(&[0xe9], false).as_deref(), Some("é"));
        assert_eq!(decode_text(b"", true), None);
        assert_eq!(decode_text(b"\0", false), None);
    }

    #[test]
    fn decodes_wm_class() {
        assert_eq!(
            decode_wm_class(b"probe\0Probe\0").as_deref(),
            Some("probe Probe")
        );
        assert_eq!(decode_wm_class(b""), None);
        assert_eq!(decode_wm_class(b"\0\0"), None);
    }
}
