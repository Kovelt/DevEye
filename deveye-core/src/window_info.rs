use serde::{Deserialize, Serialize};

/// Une fenêtre/onglet listé par un [`WindowLister`](../../deveye_ports/trait.WindowLister.html).
///
/// En pratique : les cibles CDP de `type = "page"` (onglets d'un Chromium).
/// Le pont SDK ne connaît pas les autres fenêtres → liste vide.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WindowInfo {
    /// Identifiant opaque de la cible (CDP `targetId`).
    pub id: String,
    /// Titre de la page.
    pub title: String,
    /// URL de la page.
    pub url: String,
    /// Type de cible (`page`, `iframe`…).
    pub kind: String,
}

impl WindowInfo {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        url: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            url: url.into(),
            kind: kind.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_keeps_every_field() {
        let w = WindowInfo::new("0x1", "Titre", "", "x11-window");
        assert_eq!(w.id, "0x1");
        assert_eq!(w.title, "Titre");
        assert!(w.url.is_empty());
        assert_eq!(w.kind, "x11-window");
    }
}
