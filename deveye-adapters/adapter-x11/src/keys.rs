//! Clavier : resolution caractere / nom de touche -> keycode du serveur X.
//!
//! La table de reference est le **keymap du serveur** (`GetKeyboardMapping`),
//! jamais une table `us` locale : on tape donc ce que produirait le clavier du
//! serveur (par exemple QWERTZ ch-fr apres `setxkbmap ch fr`).
//!
//! Tout est pur : les fonctions `plan_*` transforment un keymap et une entree
//! en une liste d'[`KeyAction`] a rejouer, ou en erreur — **avant** la moindre
//! emission (une touche inconnue au milieu d'un combo ne doit jamais laisser un
//! modificateur enfonce).
//!
//! Niveaux du groupe 1 : 0 = rien, 1 = Shift, 2 = AltGr (`ISO_Level3_Shift`),
//! 3 = Shift + AltGr. Un caractere absent du layout est injecte par
//! **remappage temporaire d'un keycode libre** (technique de xdotool) : le
//! keycode est reecrit, presse, relache, puis restaure.

use std::time::Duration;

use crate::error::X11Error;

/// Longueur maximale (en caracteres) d'un `Type`.
pub const MAX_TYPE_CHARS: usize = 4096;
/// Pause entre deux caracteres tapes (laisse le client lire sa file).
const TYPE_GAP: Duration = Duration::from_millis(6);
/// Pause apres un changement de keymap : les clients doivent traiter le
/// `MappingNotify` avant de lire le keycode.
const REMAP_SETTLE: Duration = Duration::from_millis(30);
/// Pause entre l'appui des modificateurs et celui de la touche.
const MOD_SETTLE: Duration = Duration::from_millis(8);

pub const XK_SHIFT_L: u32 = 0xffe1;
pub const XK_CONTROL_L: u32 = 0xffe3;
pub const XK_ALT_L: u32 = 0xffe9;
pub const XK_SUPER_L: u32 = 0xffeb;
pub const XK_ISO_LEVEL3_SHIFT: u32 = 0xfe03;

/// Operation elementaire de clavier, rejouee telle quelle par l'executeur.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Press(u8),
    Release(u8),
    /// Reecrit `keycode` pour qu'il produise `keysym` ; `keysym == 0` le
    /// restaure a « aucun symbole » (il etait libre avant).
    Remap {
        keycode: u8,
        keysym: u32,
    },
    Pause(Duration),
}

/// Keymap du serveur (reponse a `GetKeyboardMapping`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keymap {
    min_keycode: u8,
    per: usize,
    syms: Vec<u32>,
}

impl Keymap {
    /// `syms` : `keysyms_per_keycode` symboles par keycode, a partir de
    /// `min_keycode`.
    pub fn new(min_keycode: u8, keysyms_per_keycode: u8, syms: Vec<u32>) -> Self {
        Self {
            min_keycode,
            per: usize::from(keysyms_per_keycode),
            syms,
        }
    }

    pub fn keysyms_per_keycode(&self) -> u8 {
        self.per as u8
    }

    fn rows(&self) -> impl Iterator<Item = (u8, &[u32])> {
        let (min, per) = (usize::from(self.min_keycode), self.per);
        let count = self.syms.len().checked_div(per).unwrap_or(0);
        (0..count).filter_map(move |i| {
            let code = u8::try_from(min + i).ok()?;
            Some((code, &self.syms[i * per..(i + 1) * per]))
        })
    }

    fn row(&self, code: u8) -> Option<&[u32]> {
        let idx = usize::from(code).checked_sub(usize::from(self.min_keycode))?;
        if self.per == 0 {
            return None;
        }
        self.syms.get(idx * self.per..(idx + 1) * self.per)
    }

    /// Symbole d'un keycode a un niveau (0..4), avec la regle du protocole
    /// core : une ligne `[a, NoSymbol]` signifie `[a, A]` pour une lettre.
    pub fn keysym_at(&self, code: u8, level: usize) -> u32 {
        let Some(row) = self.row(code) else { return 0 };
        let get = |i: usize| row.get(i).copied().unwrap_or(0);
        let sym = get(level);
        if sym == 0 && level == 1 && (0x61..=0x7a).contains(&get(0)) {
            return get(0) - 0x20;
        }
        sym
    }

    /// Premier (keycode, niveau) produisant `keysym` : niveau le plus bas, puis
    /// keycode le plus bas. Seuls les 4 premiers niveaux du groupe 1 comptent.
    pub fn find(&self, keysym: u32) -> Option<(u8, usize)> {
        if keysym == 0 {
            return None;
        }
        for level in 0..self.per.min(4) {
            for (code, _) in self.rows() {
                if self.keysym_at(code, level) == keysym {
                    return Some((code, level));
                }
            }
        }
        None
    }

    /// Keycode sans aucun symbole (candidat au remappage temporaire) : le plus
    /// eleve, comme xdotool, pour s'eloigner des touches reelles.
    pub fn free_keycode(&self) -> Option<u8> {
        self.rows()
            .filter(|(_, row)| row.iter().all(|&s| s == 0))
            .map(|(c, _)| c)
            .last()
    }
}

/// Keysym canonique d'un caractere Unicode : Latin-1 direct, keysym historique
/// quand il existe (ex. `EuroSign` 0x20ac), sinon `0x01000000 + cp`.
pub fn keysym_for_char(c: char) -> u32 {
    let cp = u32::from(c);
    match cp {
        0x20..=0x7e | 0xa0..=0xff => cp,
        _ => match xkeysym::Keysym::from_char(c).raw() {
            0 => 0x0100_0000 + cp,
            sym => sym,
        },
    }
}

/// Keysyms equivalents d'un caractere, par ordre de preference : un clavier
/// peut porter la forme historique OU la forme Unicode `0x01000000 + cp`.
fn char_candidates(c: char) -> Vec<u32> {
    let primary = keysym_for_char(c);
    let unicode = 0x0100_0000 + u32::from(c);
    if primary == unicode || u32::from(c) <= 0xff {
        vec![primary]
    } else {
        vec![primary, unicode]
    }
}

/// Noms de touches : W3C (`KeyboardEvent.key` / `.code`) et XKB, insensibles a
/// la casse.
const NAMED: &[(&[&str], u32)] = &[
    (&["enter", "return"], 0xff0d),
    (&["escape", "esc"], 0xff1b),
    (&["backspace"], 0xff08),
    (&["tab"], 0xff09),
    (&["delete", "del"], 0xffff),
    (&["insert"], 0xff63),
    (&["home"], 0xff50),
    (&["end"], 0xff57),
    (&["pageup", "page_up", "prior"], 0xff55),
    (&["pagedown", "page_down", "next"], 0xff56),
    (&["arrowleft", "left"], 0xff51),
    (&["arrowup", "up"], 0xff52),
    (&["arrowright", "right"], 0xff53),
    (&["arrowdown", "down"], 0xff54),
    (&["space", "spacebar"], 0x20),
    (&["capslock", "caps_lock"], 0xffe5),
    (&["shift", "shift_l"], XK_SHIFT_L),
    (&["control", "ctrl", "control_l"], XK_CONTROL_L),
    (&["alt", "alt_l"], XK_ALT_L),
    (&["meta", "super", "super_l"], XK_SUPER_L),
    (&["kp_enter", "numpadenter"], 0xff8d),
    (&["minus"], 0x2d),
    (&["equal"], 0x3d),
    (&["comma"], 0x2c),
    (&["period"], 0x2e),
    (&["slash"], 0x2f),
];

/// Keysym d'un nom de touche ou d'un caractere unique. `None` si inconnu.
pub fn keysym_for_name(name: &str) -> Option<u32> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(keysym_for_char(c));
    }
    let lower = name.to_lowercase();
    if let Some(&(_, sym)) = NAMED
        .iter()
        .find(|(names, _)| names.contains(&lower.as_str()))
    {
        return Some(sym);
    }
    // F1..F24.
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0xffbd + n);
        }
    }
    // `KeyA` / `Digit1` (valeurs `KeyboardEvent.code`).
    if let Some(letter) = lower.strip_prefix("key") {
        let mut it = letter.chars();
        if let (Some(c), None) = (it.next(), it.next()) {
            if c.is_ascii_lowercase() {
                return Some(u32::from(c));
            }
        }
    }
    if let Some(digit) = lower.strip_prefix("digit") {
        let mut it = digit.chars();
        if let (Some(c), None) = (it.next(), it.next()) {
            if c.is_ascii_digit() {
                return Some(u32::from(c));
            }
        }
    }
    None
}

/// Keysym d'un modificateur (`ctrl`, `shift`, `alt`, `meta`...).
pub fn modifier_keysym(name: &str) -> Option<u32> {
    match name.to_lowercase().as_str() {
        "ctrl" | "control" | "control_l" => Some(XK_CONTROL_L),
        "shift" | "shift_l" => Some(XK_SHIFT_L),
        "alt" | "option" | "alt_l" => Some(XK_ALT_L),
        "meta" | "super" | "win" | "cmd" | "command" | "super_l" => Some(XK_SUPER_L),
        _ => None,
    }
}

/// Keycode (niveau 0) d'un modificateur, ou erreur explicite.
fn modifier_keycode(keymap: &Keymap, keysym: u32, label: &str) -> Result<u8, X11Error> {
    match keymap.find(keysym) {
        Some((code, 0)) => Ok(code),
        _ => Err(X11Error::Invalid(format!(
            "le modificateur « {label} » n'existe pas dans le clavier du serveur X"
        ))),
    }
}

/// Frappe d'un keysym : eventuels modificateurs `extra_mods` (deja resolus en
/// keycodes) + modificateurs de niveau (Shift/AltGr) + touche, ou remappage
/// temporaire si le keysym est absent du layout.
fn plan_stroke(
    keymap: &Keymap,
    candidates: &[u32],
    extra_mods: &[u8],
) -> Result<Vec<KeyAction>, X11Error> {
    let keysym = candidates.first().copied().unwrap_or(0);
    let mut mods: Vec<u8> = extra_mods.to_vec();
    let push_mod = |code: u8, mods: &mut Vec<u8>| {
        if !mods.contains(&code) {
            mods.push(code);
        }
    };

    // Chemin direct : keysym present, et modificateurs de niveau disponibles.
    if let Some((code, level)) = candidates.iter().find_map(|&k| keymap.find(k)) {
        let shift = (level & 1 == 1).then(|| {
            keymap
                .find(XK_SHIFT_L)
                .filter(|&(_, l)| l == 0)
                .map(|(c, _)| c)
        });
        let altgr = (level & 2 == 2).then(|| {
            keymap
                .find(XK_ISO_LEVEL3_SHIFT)
                .filter(|&(_, l)| l == 0)
                .map(|(c, _)| c)
        });
        // `Some(None)` = niveau requis mais modificateur introuvable.
        let usable = shift.is_none_or(|m| m.is_some()) && altgr.is_none_or(|m| m.is_some());
        if usable {
            if let Some(Some(c)) = shift {
                push_mod(c, &mut mods);
            }
            if let Some(Some(c)) = altgr {
                push_mod(c, &mut mods);
            }
            return Ok(stroke(&mods, code));
        }
    }

    // Repli : remappage temporaire d'un keycode libre.
    let free = keymap.free_keycode().ok_or_else(|| {
        X11Error::Invalid(format!(
            "keysym 0x{keysym:x} absent du clavier du serveur X et aucun keycode libre pour \
             le remapper"
        ))
    })?;
    let mut out = vec![
        KeyAction::Remap {
            keycode: free,
            keysym,
        },
        KeyAction::Pause(REMAP_SETTLE),
    ];
    out.extend(stroke(&mods, free));
    out.push(KeyAction::Pause(REMAP_SETTLE));
    out.push(KeyAction::Remap {
        keycode: free,
        keysym: 0,
    });
    Ok(out)
}

/// Appui des modificateurs dans l'ordre, touche, relachement en ordre inverse.
fn stroke(mods: &[u8], code: u8) -> Vec<KeyAction> {
    let mut out: Vec<KeyAction> = mods.iter().map(|&m| KeyAction::Press(m)).collect();
    if !mods.is_empty() {
        out.push(KeyAction::Pause(MOD_SETTLE));
    }
    out.push(KeyAction::Press(code));
    out.push(KeyAction::Release(code));
    out.extend(mods.iter().rev().map(|&m| KeyAction::Release(m)));
    out
}

/// Plan de frappe d'un texte. Limite : [`MAX_TYPE_CHARS`] caracteres.
/// `\n` = Entree, `\t` = Tab, `\r` ignore ; autres caracteres de controle refuses.
pub fn plan_type(keymap: &Keymap, text: &str) -> Result<Vec<KeyAction>, X11Error> {
    let count = text.chars().count();
    if count > MAX_TYPE_CHARS {
        return Err(X11Error::Invalid(format!(
            "texte trop long : {count} caractères (maximum {MAX_TYPE_CHARS})"
        )));
    }
    let mut out = Vec::new();
    for c in text.chars() {
        let candidates = match c {
            '\r' => continue,
            '\n' => vec![0xff0d],
            '\t' => vec![0xff09],
            c if c.is_control() => {
                return Err(X11Error::Invalid(format!(
                    "caractère de contrôle U+{:04X} non tapable (utiliser key_press)",
                    u32::from(c)
                )))
            }
            c => char_candidates(c),
        };
        out.extend(plan_stroke(keymap, &candidates, &[])?);
        out.push(KeyAction::Pause(TYPE_GAP));
    }
    Ok(out)
}

/// Plan d'un appui simple (nom de touche ou caractere unique).
pub fn plan_key(keymap: &Keymap, key: &str) -> Result<Vec<KeyAction>, X11Error> {
    plan_stroke(keymap, &key_candidates(key)?, &[])
}

/// Keysyms candidats d'un nom de touche (un caractere unique a ses equivalents).
fn key_candidates(key: &str) -> Result<Vec<u32>, X11Error> {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Ok(char_candidates(c));
    }
    keysym_for_name(key)
        .map(|k| vec![k])
        .ok_or_else(|| X11Error::Invalid(format!("touche inconnue : « {key} »")))
}

/// Plan d'une combinaison : tout est resolu AVANT d'emettre ; un modificateur ou
/// une touche inconnus ne produisent aucune action.
pub fn plan_combo(
    keymap: &Keymap,
    modifiers: &[String],
    key: &str,
) -> Result<Vec<KeyAction>, X11Error> {
    let mut mod_codes: Vec<u8> = Vec::new();
    for name in modifiers {
        let sym = modifier_keysym(name)
            .ok_or_else(|| X11Error::Invalid(format!("modificateur inconnu : « {name} »")))?;
        let code = modifier_keycode(keymap, sym, name)?;
        if !mod_codes.contains(&code) {
            mod_codes.push(code);
        }
    }
    plan_stroke(keymap, &key_candidates(key)?, &mod_codes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keymap fictif QWERTZ ch-fr (extrait) : 4 colonnes
    /// [niveau0, niveau1, AltGr, Shift+AltGr], keycodes de 8 a 40.
    ///
    /// 10:'1' 11:'2' 24:'q' 25:'w' 26:'e' 29:'z' 34:'è'/'ü' 47:'é'/'ö'
    /// 48:'à'/'ä' 50:Shift_L 37:Control_L 64:Alt_L 108:ISO_Level3_Shift
    /// 133:Super_L 36:Return 9:Escape ; 100..=255 libres sauf 108, 133.
    fn swiss() -> Keymap {
        let mut rows: Vec<[u32; 4]> = vec![[0; 4]; 248]; // keycodes 8..=255
        let mut set = |code: u8, r: [u32; 4]| rows[usize::from(code) - 8] = r;
        set(10, [0x31, 0x2b, 0x7c, 0]); // 1 + |
        set(11, [0x32, 0x22, 0x40, 0]); // 2 " @
        set(24, [0x71, 0x51, 0, 0]);
        set(25, [0x77, 0x57, 0, 0]);
        set(26, [0x65, 0x45, 0x20ac, 0]); // e E €
        set(29, [0x7a, 0x5a, 0, 0]); // z (QWERTZ)
        set(34, [0xe8, 0xfc, 0x5b, 0]); // è ü [
        set(47, [0xe9, 0xf6, 0, 0]); // é ö
        set(48, [0xe0, 0xe4, 0x7b, 0]); // à ä {
        set(50, [XK_SHIFT_L, 0, 0, 0]);
        set(37, [XK_CONTROL_L, 0, 0, 0]);
        set(64, [XK_ALT_L, 0, 0, 0]);
        set(108, [XK_ISO_LEVEL3_SHIFT, 0, 0, 0]);
        set(133, [XK_SUPER_L, 0, 0, 0]);
        set(36, [0xff0d, 0, 0, 0]);
        set(9, [0xff1b, 0, 0, 0]);
        set(65, [0x20, 0, 0, 0]);
        Keymap::new(8, 4, rows.into_iter().flatten().collect())
    }

    fn presses(actions: &[KeyAction]) -> Vec<u8> {
        actions
            .iter()
            .filter_map(|a| match a {
                KeyAction::Press(c) => Some(*c),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn direct_character_is_a_plain_press_release() {
        let k = swiss();
        let a = plan_key(&k, "q").unwrap();
        assert_eq!(a, vec![KeyAction::Press(24), KeyAction::Release(24)]);
    }

    #[test]
    fn qwertz_differs_from_qwerty() {
        let k = swiss();
        assert_eq!(k.find(u32::from('z')), Some((29, 0)));
        assert_eq!(k.find(u32::from('y')), None);
    }

    #[test]
    fn accented_characters_are_direct_at_level_zero() {
        let k = swiss();
        assert_eq!(k.find(0xe9), Some((47, 0))); // é
        assert_eq!(k.find(0xe0), Some((48, 0))); // à
    }

    #[test]
    fn shifted_character_presses_shift_around_the_key() {
        let k = swiss();
        let a = plan_type(&k, "ü").unwrap();
        // Shift_L (50) puis touche 34, relachees en ordre inverse.
        let core: Vec<&KeyAction> = a
            .iter()
            .filter(|x| !matches!(x, KeyAction::Pause(_)))
            .collect();
        assert_eq!(
            core,
            vec![
                &KeyAction::Press(50),
                &KeyAction::Press(34),
                &KeyAction::Release(34),
                &KeyAction::Release(50)
            ]
        );
    }

    #[test]
    fn uppercase_letters_use_the_shift_level_even_with_implicit_case() {
        // Ligne [a, NoSymbol] : 'A' doit etre trouve au niveau 1 (regle core).
        let mut syms = vec![0u32; 4 * 10];
        syms[0] = 0x61; // keycode 8 : a, rien d'autre
        let k = Keymap::new(8, 4, syms);
        assert_eq!(k.keysym_at(8, 1), 0x41);
        assert_eq!(k.find(0x41), Some((8, 1)));
    }

    #[test]
    fn altgr_character_uses_iso_level3_shift() {
        let k = swiss();
        let a = plan_type(&k, "€").unwrap();
        assert_eq!(presses(&a), vec![108, 26]);
        let a = plan_type(&k, "@").unwrap();
        assert_eq!(presses(&a), vec![108, 11]);
    }

    #[test]
    fn missing_character_is_remapped_then_restored() {
        let k = swiss();
        let snowman = '☃';
        let a = plan_type(&k, &snowman.to_string()).unwrap();
        let free = k.free_keycode().unwrap();
        let sym = keysym_for_char(snowman);
        assert_eq!(sym, 0x0100_2603);
        assert!(matches!(
            a.first(),
            Some(KeyAction::Remap { keycode, keysym }) if *keycode == free && *keysym == sym
        ));
        // Dernier changement de keymap = restauration a « libre ».
        let last_remap = a
            .iter()
            .rev()
            .find(|x| matches!(x, KeyAction::Remap { .. }));
        assert_eq!(
            last_remap,
            Some(&KeyAction::Remap {
                keycode: free,
                keysym: 0
            })
        );
        assert_eq!(presses(&a), vec![free]);
        // Pause de stabilisation entre remap et appui.
        let remap_pos = a
            .iter()
            .position(|x| matches!(x, KeyAction::Remap { .. }))
            .unwrap();
        assert!(matches!(a[remap_pos + 1], KeyAction::Pause(_)));
    }

    #[test]
    fn free_keycode_is_the_highest_empty_row() {
        let k = swiss();
        assert_eq!(k.free_keycode(), Some(255));
        // Aucun libre : erreur explicite, pas de panique.
        let full = Keymap::new(8, 2, vec![0x61; 8]);
        assert_eq!(full.free_keycode(), None);
        assert!(plan_type(&full, "☃").is_err());
    }

    #[test]
    fn unmappable_level_modifier_falls_back_to_remap() {
        // Layout sans ISO_Level3_Shift : '€' ne peut pas passer en direct.
        let mut syms = vec![0u32; 4 * 20];
        syms[0] = 0x65; // keycode 8 : e
        syms[2] = 0x20ac; // niveau AltGr
        syms[4] = XK_SHIFT_L; // keycode 9
        let k = Keymap::new(8, 4, syms);
        let a = plan_type(&k, "€").unwrap();
        assert!(a
            .iter()
            .any(|x| matches!(x, KeyAction::Remap { keysym: 0x20ac, .. })));
    }

    #[test]
    fn newline_and_tab_map_to_return_and_tab() {
        let mut syms = vec![0u32; 4 * 10];
        syms[0] = 0xff0d; // 8 : Return
        syms[4] = 0xff09; // 9 : Tab
        let k = Keymap::new(8, 4, syms);
        assert_eq!(presses(&plan_type(&k, "\n\t\r").unwrap()), vec![8, 9]);
    }

    #[test]
    fn control_characters_are_refused_before_emission() {
        let k = swiss();
        assert!(plan_type(&k, "ab\u{1b}c").is_err());
        assert!(plan_type(&k, "\u{7f}").is_err());
    }

    #[test]
    fn type_length_limit_counts_characters_not_bytes() {
        let k = swiss();
        let ok = "é".repeat(MAX_TYPE_CHARS);
        assert!(plan_type(&k, &ok).is_ok());
        let too_long = "é".repeat(MAX_TYPE_CHARS + 1);
        let err = plan_type(&k, &too_long).unwrap_err();
        assert!(err.to_string().contains("4096"));
    }

    #[test]
    fn combo_presses_modifiers_first_and_releases_in_reverse() {
        let k = swiss();
        let a = plan_combo(&k, &["ctrl".into(), "shift".into()], "q").unwrap();
        let core: Vec<&KeyAction> = a
            .iter()
            .filter(|x| !matches!(x, KeyAction::Pause(_)))
            .collect();
        assert_eq!(
            core,
            vec![
                &KeyAction::Press(37),
                &KeyAction::Press(50),
                &KeyAction::Press(24),
                &KeyAction::Release(24),
                &KeyAction::Release(50),
                &KeyAction::Release(37)
            ]
        );
    }

    #[test]
    fn combo_needing_shift_level_adds_shift_once() {
        let k = swiss();
        // ctrl + "Q" : Shift requis par le niveau ; deja demande : une seule fois.
        let a = plan_combo(&k, &["ctrl".into(), "shift".into()], "Q").unwrap();
        assert_eq!(presses(&a), vec![37, 50, 24]);
        let a = plan_combo(&k, &["ctrl".into()], "Q").unwrap();
        assert_eq!(presses(&a), vec![37, 50, 24]);
    }

    #[test]
    fn unknown_modifier_or_key_emits_nothing() {
        let k = swiss();
        assert!(plan_combo(&k, &["ctrl".into(), "hyperdrive".into()], "q").is_err());
        assert!(plan_combo(&k, &["ctrl".into()], "NoSuchKey").is_err());
        // Modificateur connu mais absent du clavier (ici Super_L retire).
        let mut k2 = swiss();
        k2.syms.iter_mut().for_each(|s| {
            if *s == XK_SUPER_L {
                *s = 0;
            }
        });
        assert!(plan_combo(&k2, &["meta".into()], "q").is_err());
    }

    #[test]
    fn key_names_accept_w3c_and_xkb() {
        for (name, sym) in [
            ("Enter", 0xff0d),
            ("Return", 0xff0d),
            ("Escape", 0xff1b),
            ("ArrowUp", 0xff52),
            ("Up", 0xff52),
            ("Backspace", 0xff08),
            ("BackSpace", 0xff08),
            ("Tab", 0xff09),
            ("Page_Down", 0xff56),
            ("PageDown", 0xff56),
            ("F1", 0xffbe),
            ("F12", 0xffc9),
            ("KeyA", 0x61),
            ("Digit7", 0x37),
            ("Space", 0x20),
            (" ", 0x20),
            ("é", 0xe9),
        ] {
            assert_eq!(keysym_for_name(name), Some(sym), "{name}");
        }
        for bad in ["", "F0", "F25", "KeyAA", "Nope"] {
            assert_eq!(keysym_for_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn modifier_names() {
        for n in ["ctrl", "Control", "SHIFT", "alt", "meta", "Super", "cmd"] {
            assert!(modifier_keysym(n).is_some(), "{n}");
        }
        assert_eq!(modifier_keysym("fn"), None);
    }

    #[test]
    fn empty_keymap_never_panics() {
        let k = Keymap::new(8, 0, vec![]);
        assert_eq!(k.find(0x61), None);
        assert_eq!(k.free_keycode(), None);
        assert!(plan_type(&k, "a").is_err());
    }

    #[test]
    fn keysym_for_char_covers_latin1_and_unicode() {
        assert_eq!(keysym_for_char('a'), 0x61);
        assert_eq!(keysym_for_char('é'), 0xe9);
        assert_eq!(keysym_for_char('€'), 0x20ac);
        assert_eq!(char_candidates('€'), vec![0x20ac, 0x0100_20ac]);
        assert_eq!(char_candidates('a'), vec![0x61]);
        assert_eq!(keysym_for_char('\u{1F600}'), 0x0101_F600);
    }
}
