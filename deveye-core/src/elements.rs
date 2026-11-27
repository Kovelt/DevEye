//! Éléments désignables par référence stable (feature `unstable-elements`).
//!
//! **Surface expérimentale** (ADR-0069, ADR-0072) : hors promesse semver, ces types
//! peuvent changer dans toute version mineure.
//!
//! Un instantané d'éléments est un arbre compact d'éléments interactifs ou
//! significatifs, chacun porté par une **référence** que l'agent réutilise pour agir
//! sans sélecteur CSS ni coordonnées. Deux familles de références, visibles à leur
//! préfixe :
//!
//! - `e<n>` pour les cibles à DOM : numéro monotone attribué par la [`RefTable`],
//!   stable tant que le nœud vit, jamais réutilisé dans une session ;
//! - `w:<id>` pour les widgets natifs : l'identifiant fourni par l'app (schéma
//!   `deveye.ui/1`), donc stable aussi d'une relance à l'autre **si** l'app fournit
//!   des identifiants déterministes.
//!
//! Domaine pur : aucune E/S. Le rect d'un instantané est indicatif ; c'est la
//! relocalisation au moment de l'action ([`ElementLocation`]) qui fait foi.

use std::collections::HashMap;
use std::hash::Hash;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Nombre maximal de nœuds d'un instantané quand l'appelant n'en demande pas.
pub const DEFAULT_MAX_NODES: usize = 400;

/// Préfixe des références de widgets natifs.
pub const WIDGET_REF_PREFIX: &str = "w:";

/// Préfixe des références de nœuds de DOM.
pub const DOM_REF_PREFIX: &str = "e";

fn default_max_nodes() -> usize {
    DEFAULT_MAX_NODES
}

fn default_true() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// État d'un élément. Les champs tri-états valent `None` quand la notion ne
/// s'applique pas (un bouton n'est ni coché ni déplié).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ElementState {
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub focused: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
}

impl ElementState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn checked(mut self, checked: Option<bool>) -> Self {
        self.checked = checked;
        self
    }

    pub fn expanded(mut self, expanded: Option<bool>) -> Self {
        self.expanded = expanded;
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

/// Un élément de l'instantané.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ElementNode {
    /// Référence stable (`e12`, `w:vcf/cutoff`), sérialisée sous le nom `ref`.
    #[serde(rename = "ref")]
    pub reference: String,
    /// Rôle (chaîne ouverte, vocabulaire calqué sur ARIA).
    pub role: String,
    /// Nom accessible, quand il existe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Valeur courante, jamais présente pour un champ secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default)]
    pub state: ElementState,
    /// `[x, y, largeur, hauteur]` dans le repère des clics de la cible ; indicatif.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<[i32; 4]>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub scrollable: bool,
    /// Hors de la zone visible au moment de l'instantané.
    #[serde(default, skip_serializing_if = "is_false")]
    pub offscreen: bool,
    /// Référence éphémère (identifiant synthétisé) : sa stabilité n'est pas promise.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ephemeral: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ElementNode>,
}

impl ElementNode {
    pub fn new(reference: impl Into<String>, role: impl Into<String>) -> Self {
        Self {
            reference: reference.into(),
            role: role.into(),
            name: None,
            value: None,
            state: ElementState::default(),
            rect: None,
            scrollable: false,
            offscreen: false,
            ephemeral: false,
            children: Vec::new(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn with_state(mut self, state: ElementState) -> Self {
        self.state = state;
        self
    }

    pub fn with_rect(mut self, rect: [i32; 4]) -> Self {
        self.rect = Some(rect);
        self
    }

    pub fn with_scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    pub fn with_offscreen(mut self, offscreen: bool) -> Self {
        self.offscreen = offscreen;
        self
    }

    pub fn with_ephemeral(mut self, ephemeral: bool) -> Self {
        self.ephemeral = ephemeral;
        self
    }

    pub fn with_children(mut self, children: Vec<ElementNode>) -> Self {
        self.children = children;
        self
    }

    /// Nombre de nœuds du sous-arbre, racine comprise.
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(ElementNode::count).sum::<usize>()
    }
}

/// Instantané d'éléments d'une cible.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ElementSnapshot {
    /// Compteur de la [`RefTable`] : les références d'une génération antérieure sont
    /// périmées. Vaut 0 pour une famille sans table (widgets natifs).
    pub generation: u64,
    /// Famille de cible : `cdp`, `bridge`, `widgets`…
    pub target_kind: String,
    /// Vrai si `max_nodes` a coupé l'instantané : cibler un `scope`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    pub nodes: Vec<ElementNode>,
    /// Référence de l'élément qui a le focus clavier, s'il est dans l'instantané.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}

impl ElementSnapshot {
    pub fn new(generation: u64, target_kind: impl Into<String>, nodes: Vec<ElementNode>) -> Self {
        Self {
            generation,
            target_kind: target_kind.into(),
            truncated: false,
            nodes,
            focus: None,
        }
    }

    pub fn with_truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }

    pub fn with_focus(mut self, focus: Option<String>) -> Self {
        self.focus = focus;
        self
    }

    /// Nombre total d'éléments, tous niveaux confondus.
    pub fn count(&self) -> usize {
        self.nodes.iter().map(ElementNode::count).sum()
    }
}

/// Paramètres d'un instantané.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SnapshotQuery {
    /// Référence ou sélecteur racine ; toute la cible si absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
    #[serde(default = "default_max_nodes")]
    pub max_nodes: usize,
    /// Ne garder que les éléments interactifs et leurs ancêtres utiles (défaut vrai).
    #[serde(default = "default_true")]
    pub interactive_only: bool,
    #[serde(default)]
    pub viewport_only: bool,
}

impl Default for SnapshotQuery {
    fn default() -> Self {
        Self {
            scope: None,
            max_depth: None,
            max_nodes: DEFAULT_MAX_NODES,
            interactive_only: true,
            viewport_only: false,
        }
    }
}

impl SnapshotQuery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    pub fn with_max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = Some(max_depth);
        self
    }

    pub fn with_max_nodes(mut self, max_nodes: usize) -> Self {
        self.max_nodes = max_nodes;
        self
    }

    pub fn with_interactive_only(mut self, interactive_only: bool) -> Self {
        self.interactive_only = interactive_only;
        self
    }

    pub fn with_viewport_only(mut self, viewport_only: bool) -> Self {
        self.viewport_only = viewport_only;
        self
    }
}

/// Position d'un élément remesurée au moment de l'action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ElementLocation {
    pub reference: String,
    /// `[x, y, largeur, hauteur]` frais, dans le repère des clics de la cible.
    pub rect: [i32; 4],
    pub offscreen: bool,
}

impl ElementLocation {
    pub fn new(reference: impl Into<String>, rect: [i32; 4]) -> Self {
        Self {
            reference: reference.into(),
            rect,
            offscreen: false,
        }
    }

    pub fn with_offscreen(mut self, offscreen: bool) -> Self {
        self.offscreen = offscreen;
        self
    }

    /// Centre du rect, point où l'on clique. Division entière : reste dans le rect.
    pub fn center(&self) -> (i32, i32) {
        let [x, y, w, h] = self.rect;
        (x + w / 2, y + h / 2)
    }
}

/// Famille d'une référence, déduite de son préfixe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefKind<'a> {
    /// `e<n>` : numéro attribué par la [`RefTable`].
    Dom(u64),
    /// `w:<id>` : identifiant fourni par l'app.
    Widget(&'a str),
}

/// Analyse une référence ; `None` si elle n'a aucune forme connue.
pub fn parse_ref(reference: &str) -> Option<RefKind<'_>> {
    if let Some(id) = reference.strip_prefix(WIDGET_REF_PREFIX) {
        return (!id.is_empty()).then_some(RefKind::Widget(id));
    }
    let digits = reference.strip_prefix(DOM_REF_PREFIX)?;
    // Refuse `e`, `e+1`, `e01` : une seule écriture canonique par numéro.
    if digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || (digits.len() > 1 && digits.starts_with('0'))
    {
        return None;
    }
    digits.parse().ok().map(RefKind::Dom)
}

/// Pourquoi une référence ne se résout pas.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum RefError {
    /// La référence n'a pas la forme `e<n>` ni `w:<id>`, ou n'a jamais été émise.
    #[error("référence inconnue : {0}")]
    Unknown(String),
    /// La référence a existé mais son nœud a disparu ou sa génération est close.
    #[error("référence périmée : {0}")]
    Stale(String),
}

/// Table de références d'une session : associe la clé d'un nœud côté backend
/// (`backendNodeId` CDP, identifiant du pont JS…) à un `e<n>`.
///
/// Règles : un nœud qui vit garde sa référence d'un instantané au suivant ; un
/// nouveau nœud reçoit un nouveau numéro ; **un numéro n'est jamais réutilisé**,
/// même après [`invalidate`](Self::invalidate) ou [`retain`](Self::retain). Pure et
/// sans E/S : la fraîcheur côté backend est l'affaire de l'adapter.
#[derive(Clone, Debug)]
pub struct RefTable<K> {
    generation: u64,
    next: u64,
    by_key: HashMap<K, u64>,
    by_ref: HashMap<u64, K>,
}

impl<K> Default for RefTable<K> {
    fn default() -> Self {
        Self {
            generation: 0,
            next: 1,
            by_key: HashMap::new(),
            by_ref: HashMap::new(),
        }
    }
}

impl<K: Clone + Eq + Hash> RefTable<K> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Génération courante, à reporter dans l'[`ElementSnapshot`].
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Nombre de références vivantes.
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    /// Référence du nœud `key`, créée si le nœud est nouveau.
    pub fn reference_for(&mut self, key: &K) -> String {
        if let Some(n) = self.by_key.get(key) {
            return format!("{DOM_REF_PREFIX}{n}");
        }
        let n = self.next;
        self.next += 1;
        self.by_key.insert(key.clone(), n);
        self.by_ref.insert(n, key.clone());
        format!("{DOM_REF_PREFIX}{n}")
    }

    /// Clé backend d'une référence vivante.
    pub fn resolve(&self, reference: &str) -> Result<&K, RefError> {
        let Some(RefKind::Dom(n)) = parse_ref(reference) else {
            return Err(RefError::Unknown(reference.to_string()));
        };
        if let Some(key) = self.by_ref.get(&n) {
            return Ok(key);
        }
        // Un numéro déjà attribué mais absent de la table est périmé ; au-delà du
        // compteur, la référence n'a jamais été émise.
        if n >= 1 && n < self.next {
            Err(RefError::Stale(reference.to_string()))
        } else {
            Err(RefError::Unknown(reference.to_string()))
        }
    }

    /// Oublie le nœud `key` (détaché côté backend). Sa référence devient périmée.
    pub fn forget(&mut self, key: &K) -> bool {
        match self.by_key.remove(key) {
            Some(n) => {
                self.by_ref.remove(&n);
                true
            }
            None => false,
        }
    }

    /// Ne garde que les nœuds pour lesquels `alive` est vrai ; renvoie le nombre
    /// de références périmées par l'opération.
    pub fn retain(&mut self, mut alive: impl FnMut(&K) -> bool) -> usize {
        let before = self.by_key.len();
        let by_ref = &mut self.by_ref;
        self.by_key.retain(|key, n| {
            let keep = alive(key);
            if !keep {
                by_ref.remove(n);
            }
            keep
        });
        before - self.by_key.len()
    }

    /// Périme toutes les références et ouvre une nouvelle génération : navigation
    /// principale, reconnexion de la cible, changement de backend. Le compteur de
    /// numéros continue, donc une ancienne référence ne désigne jamais un autre nœud.
    pub fn invalidate(&mut self) {
        self.generation += 1;
        self.by_key.clear();
        self.by_ref.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_key_keeps_its_reference_across_snapshots() {
        let mut table = RefTable::new();
        let a = table.reference_for(&10_u64);
        let b = table.reference_for(&20_u64);
        assert_eq!((a.as_str(), b.as_str()), ("e1", "e2"));
        assert_eq!(table.reference_for(&10), "e1");
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn numbers_are_never_reused_after_forget() {
        let mut table = RefTable::new();
        let old = table.reference_for(&1_u64);
        assert!(table.forget(&1));
        let fresh = table.reference_for(&2_u64);
        assert_ne!(old, fresh);
        assert_eq!(table.resolve(&old), Err(RefError::Stale(old.clone())));
        // Le même nœud qui revient reçoit un numéro neuf, pas l'ancien.
        let back = table.reference_for(&1);
        assert_ne!(back, old);
    }

    #[test]
    fn retain_stales_only_dead_nodes() {
        let mut table = RefTable::new();
        let keep = table.reference_for(&1_u64);
        let gone = table.reference_for(&2_u64);
        assert_eq!(table.retain(|k| *k == 1), 1);
        assert_eq!(table.resolve(&keep), Ok(&1));
        assert_eq!(table.resolve(&gone), Err(RefError::Stale(gone)));
    }

    #[test]
    fn invalidate_bumps_generation_and_keeps_counter() {
        let mut table = RefTable::new();
        let before = table.reference_for(&1_u64);
        assert_eq!(table.generation(), 0);
        table.invalidate();
        assert_eq!(table.generation(), 1);
        assert!(table.is_empty());
        assert_eq!(table.resolve(&before), Err(RefError::Stale(before.clone())));
        let after = table.reference_for(&1);
        assert_ne!(after, before);
    }

    #[test]
    fn unissued_and_malformed_references_are_unknown() {
        let mut table: RefTable<u64> = RefTable::new();
        table.reference_for(&1);
        for bad in ["e99", "e0", "e", "e01", "e+1", "x1", "w:abc", ""] {
            assert_eq!(
                table.resolve(bad),
                Err(RefError::Unknown(bad.to_string())),
                "{bad}"
            );
        }
    }

    #[test]
    fn parse_ref_recognises_both_families() {
        assert_eq!(parse_ref("e12"), Some(RefKind::Dom(12)));
        assert_eq!(
            parse_ref("w:vcf/cutoff"),
            Some(RefKind::Widget("vcf/cutoff"))
        );
        assert_eq!(parse_ref("w:"), None);
        assert_eq!(parse_ref("e99999999999999999999999"), None);
        assert_eq!(parse_ref("button"), None);
    }

    #[test]
    fn node_serializes_ref_and_omits_defaults() {
        let node = ElementNode::new("e1", "button").with_name("Enregistrer");
        let json = serde_json::to_value(&node).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"ref": "e1", "role": "button", "name": "Enregistrer", "state": {}})
        );
        let back: ElementNode = serde_json::from_value(json).unwrap();
        assert_eq!(back, node);
    }

    #[test]
    fn snapshot_counts_nested_nodes() {
        let tree = ElementNode::new("e1", "list").with_children(vec![
            ElementNode::new("e2", "listitem"),
            ElementNode::new("e3", "listitem"),
        ]);
        let snap = ElementSnapshot::new(0, "cdp", vec![tree, ElementNode::new("e4", "button")]);
        assert_eq!(snap.count(), 4);
    }

    #[test]
    fn query_defaults_apply_on_deserialization() {
        let query: SnapshotQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(query, SnapshotQuery::default());
        assert_eq!(query.max_nodes, DEFAULT_MAX_NODES);
        assert!(query.interactive_only);
        assert!(!query.viewport_only);
    }

    #[test]
    fn location_center_stays_inside_rect() {
        let loc = ElementLocation::new("w:ok", [10, 20, 5, 3]);
        assert_eq!(loc.center(), (12, 21));
    }
}
