use serde::{Deserialize, Serialize};

/// Une fenêtre/onglet listé par un [`WindowLister`](../../deveye_ports/trait.WindowLister.html).
///
/// En pratique : les cibles CDP de `type = "page"` (onglets d'un Chromium).
/// Le pont SDK ne connaît pas les autres fenêtres → liste vide.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
