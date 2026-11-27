//! Modificateurs de clavier (Ctrl, Shift, Alt, Meta) partagés par tous les
//! adapters d'interaction.
//!
//! La table des noms vit ici, dans le domaine, pour que x11, kwin-eis, CDP et
//! le pont SDK reconnaissent exactement les mêmes alias sans dépendre les uns
//! des autres (règle hexagonale : jamais un adapter qui dépend d'un autre).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Durée maximale d'un appui maintenu (`InteractionCommand::KeyHold`), en
/// millisecondes. Borne dure : une touche coincée par un crash du serveur ou du
/// client ne le reste jamais plus longtemps que cela côté logique, et un appel
/// d'outil ne bloque pas indéfiniment le verrou de sérialisation des gestes.
pub const MAX_KEY_HOLD_MS: u32 = 10_000;

/// Modificateur de clavier appliqué à un geste de pointeur.
///
/// Sérialisé sous son nom canonique (`ctrl`, `shift`, `alt`, `meta`) ; à la
/// lecture, les alias de [`FromStr`] sont acceptés, insensibles à la casse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
    Meta,
}

/// Nom de modificateur inconnu.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("modificateur inconnu : « {0} » (attendu ctrl, shift, alt ou meta)")]
pub struct UnknownModifier(pub String);

impl Modifier {
    /// Nom canonique, celui qui est sérialisé.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ctrl => "ctrl",
            Self::Shift => "shift",
            Self::Alt => "alt",
            Self::Meta => "meta",
        }
    }
}

impl fmt::Display for Modifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Modifier {
    type Err = UnknownModifier;

    /// Alias reconnus (insensibles à la casse) : `ctrl`/`control`, `shift`,
    /// `alt`/`option`, `meta`/`super`/`win`/`cmd`/`command`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "ctrl" | "control" => Ok(Self::Ctrl),
            "shift" => Ok(Self::Shift),
            "alt" | "option" => Ok(Self::Alt),
            "meta" | "super" | "win" | "cmd" | "command" => Ok(Self::Meta),
            _ => Err(UnknownModifier(s.to_string())),
        }
    }
}

impl Serialize for Modifier {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Modifier {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        name.parse().map_err(serde::de::Error::custom)
    }
}

/// Parse une liste de noms de modificateurs, en refusant le tout au premier
/// nom inconnu : un geste ne part jamais avec un modificateur en moins.
pub fn parse_modifiers<S: AsRef<str>>(names: &[S]) -> Result<Vec<Modifier>, UnknownModifier> {
    names.iter().map(|n| n.as_ref().parse()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn aliases_resolve_to_the_same_modifier() {
        for (name, expected) in [
            ("ctrl", Modifier::Ctrl),
            ("Control", Modifier::Ctrl),
            ("SHIFT", Modifier::Shift),
            ("alt", Modifier::Alt),
            ("Option", Modifier::Alt),
            ("meta", Modifier::Meta),
            ("super", Modifier::Meta),
            ("cmd", Modifier::Meta),
            ("win", Modifier::Meta),
            ("command", Modifier::Meta),
        ] {
            assert_eq!(name.parse::<Modifier>(), Ok(expected), "{name}");
        }
    }

    #[test]
    fn unknown_name_is_refused() {
        let err = "hyper".parse::<Modifier>().unwrap_err();
        assert_eq!(err, UnknownModifier("hyper".to_string()));
        assert!("".parse::<Modifier>().is_err());
        assert!(parse_modifiers(&["ctrl", "fn"]).is_err());
    }

    #[test]
    fn serializes_to_the_canonical_name_and_reads_aliases() {
        assert_eq!(serde_json::to_value(Modifier::Meta).unwrap(), json!("meta"));
        let m: Modifier = serde_json::from_value(json!("Control")).unwrap();
        assert_eq!(m, Modifier::Ctrl);
        assert!(serde_json::from_value::<Modifier>(json!("hyper")).is_err());
        assert!(serde_json::from_value::<Modifier>(json!(3)).is_err());
    }
}
