//! `ElementInspector` des widgets natifs : désigner un élément de la route
//! `ui.widgets` par une référence `w:<id>` (ADR-0072, ADR-0073).
//!
//! La fraîcheur vient de l'app, pas d'un cache : un instantané est un appel de
//! route, et `locate` en refait un pour **remesurer** le rect au moment de
//! l'action. Aucune relocalisation silencieuse : un identifiant qui a disparu
//! répond `Stale` avec des candidats de même rôle et de même nom, l'agent choisit.
//!
//! La seule mémoire est la liste des identifiants déjà vus (rôle et nom), qui sert
//! à distinguer une référence périmée (`Stale`) d'une référence jamais émise
//! (`UnknownRef`) et à proposer des candidats ; elle ne désigne jamais un élément.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;

use deveye_core::elements::{
    parse_ref, ElementLocation, ElementNode, ElementSnapshot, ElementState, RefKind, SnapshotQuery,
    WIDGET_REF_PREFIX,
};
use deveye_core::{UiReadError, UiTree, UiWidget};
use deveye_ports::{CommandRouter, ElementError, ElementInspector, ElementResult, PortError};

use crate::ui_mask::mask_route_value;

/// Famille de cible annoncée par les instantanés de ce module.
pub const WIDGETS_TARGET_KIND: &str = "widgets";

/// Longueur maximale (caractères) d'une valeur d'élément dans un instantané.
const VALUE_MAX_CHARS: usize = 200;

/// Nombre maximal de candidats proposés par une erreur `Stale`.
const MAX_CANDIDATES: usize = 5;

/// Au-delà, la mémoire des identifiants vus est repartie de zéro : elle ne sert
/// qu'à nommer des périmés récents, pas à tenir l'historique d'une session.
const MAX_SEEN: usize = 10_000;

/// Rôles interactifs : gardés quand `interactive_only` est vrai, même sans action
/// déclarée (une app qui ne remplit pas `actions` reste pilotable).
const INTERACTIVE_ROLES: &[&str] = &[
    "button", "checkbox", "radio", "textbox", "password", "slider", "knob", "tab", "menuitem",
];

/// Conteneurs conservés autour des éléments interactifs, parce qu'ils donnent un
/// contexte (une liste, un menu) ; un conteneur anonyme est aplati.
const CONTEXT_ROLES: &[&str] = &["window", "list", "menu", "scrollarea"];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    role: String,
    name: Option<String>,
}

/// Éléments d'une app native, lus dans la route `ui.widgets`.
pub struct RouteWidgetsElements {
    router: Arc<dyn CommandRouter>,
    route: String,
    seen: Mutex<HashMap<String, Seen>>,
}

impl RouteWidgetsElements {
    /// Inspecteur sur `route` ; `crate::DEFAULT_DOM_ROUTE` si `route` est vide.
    pub fn new(router: Arc<dyn CommandRouter>, route: impl Into<String>) -> Self {
        let route = route.into();
        Self {
            router,
            route: if route.is_empty() {
                crate::DEFAULT_DOM_ROUTE.to_string()
            } else {
                route
            },
            seen: Mutex::new(HashMap::new()),
        }
    }

    fn unsupported_hint(&self, why: &str) -> ElementError {
        ElementError::Unsupported {
            hint: format!(
                "{why} : l'app doit exposer son arbre par la route `{}` (deveye_sdk::ui::publish) ; \
                 sinon `screenshot` puis `click` aux coordonnées de l'image",
                self.route
            ),
        }
    }

    /// Lit l'arbre, secrets retirés (même masquage que `dom`, puis valeurs des
    /// widgets secrets supprimées : un instantané n'a aucune raison de les porter).
    async fn fetch(&self) -> ElementResult<UiTree> {
        let mut raw = match self.router.invoke(&self.route, serde_json::json!({})).await {
            Ok(raw) => raw,
            Err(PortError::Route { message, .. }) => {
                return Err(self
                    .unsupported_hint(&format!("route `{}` indisponible ({message})", self.route)))
            }
            Err(other) => return Err(ElementError::Port(other)),
        };
        mask_route_value(&mut raw);
        if raw.get("schema").is_none() && raw.get("error").is_some_and(serde_json::Value::is_string)
        {
            return Err(self.unsupported_hint("aucun arbre publié par l'app"));
        }
        let mut tree = UiTree::from_value(&raw).map_err(|e| {
            let why = match e {
                UiReadError::UnsupportedSchema(schema) => {
                    format!("schéma `{schema}` non pris en charge par ce serveur")
                }
                other => other.to_string(),
            };
            ElementError::Port(PortError::Inspect(format!(
                "la route `{}` ne sert pas un arbre de widgets lisible : {why}",
                self.route
            )))
        })?;
        tree.strip_secret_values();
        Ok(tree)
    }

    fn remember(&self, tree: &UiTree) {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        if seen.len() > MAX_SEEN {
            seen.clear();
        }
        for widget in &tree.widgets {
            seen.insert(
                widget.id.clone(),
                Seen {
                    role: widget.role.as_str().to_string(),
                    name: widget.label.clone(),
                },
            );
        }
    }

    fn was_seen(&self, id: &str) -> Option<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    /// Absent de l'arbre : périmé s'il a déjà été vu, inconnu sinon.
    fn missing(&self, reference: &str, id: &str, tree: &UiTree) -> ElementError {
        let Some(previous) = self.was_seen(id) else {
            return ElementError::UnknownRef(reference.to_string());
        };
        let candidates = match &previous.name {
            Some(name) => tree
                .widgets
                .iter()
                .filter(|w| {
                    w.state.visible
                        && w.role.as_str() == previous.role
                        && w.label.as_deref() == Some(name.as_str())
                })
                .map(|w| format!("{WIDGET_REF_PREFIX}{}", w.id))
                .take(MAX_CANDIDATES)
                .collect(),
            None => Vec::new(),
        };
        ElementError::Stale {
            reference: reference.to_string(),
            candidates,
        }
    }
}

fn is_interactive(widget: &UiWidget) -> bool {
    !widget.actions.is_empty() || INTERACTIVE_ROLES.contains(&widget.role.as_str())
}

fn is_context(widget: &UiWidget) -> bool {
    widget.label.is_some()
        || widget.scroll.is_some()
        || CONTEXT_ROLES.contains(&widget.role.as_str())
}

/// Rect entièrement hors de la fenêtre connue, ou rogné par un conteneur.
fn is_offscreen(widget: &UiWidget, window: (u32, u32)) -> bool {
    if widget.clipped == Some(true) {
        return true;
    }
    let [x, y, w, h] = widget.rect;
    let (right, bottom) = (i64::from(x) + i64::from(w), i64::from(y) + i64::from(h));
    let known = window.0 > 0 && window.1 > 0;
    right <= 0
        || bottom <= 0
        || (known && (i64::from(x) >= i64::from(window.0) || i64::from(y) >= i64::from(window.1)))
}

fn value_text(value: &serde_json::Value) -> Option<String> {
    let text = match value {
        serde_json::Value::Null => return None,
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    };
    Some(if text.chars().count() > VALUE_MAX_CHARS {
        let mut cut: String = text.chars().take(VALUE_MAX_CHARS).collect();
        cut.push('…');
        cut
    } else {
        text
    })
}

/// Contexte de construction partagé par la récursion.
struct Builder<'a> {
    tree: &'a UiTree,
    /// Indices des enfants, dans l'ordre de l'arbre, par identifiant de parent.
    children: HashMap<&'a str, Vec<usize>>,
    /// Identifiants portés par plus d'un widget : référence ambiguë, éphémère.
    duplicated: HashSet<&'a str>,
    query: &'a SnapshotQuery,
    window: (u32, u32),
    visiting: HashSet<usize>,
}

impl<'a> Builder<'a> {
    fn new(tree: &'a UiTree, query: &'a SnapshotQuery) -> Self {
        let mut children: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for (index, widget) in tree.widgets.iter().enumerate() {
            *counts.entry(widget.id.as_str()).or_default() += 1;
            if let Some(parent) = widget.parent.as_deref() {
                children.entry(parent).or_default().push(index);
            }
        }
        let duplicated = counts
            .into_iter()
            .filter(|(_, n)| *n > 1)
            .map(|(id, _)| id)
            .collect();
        Self {
            tree,
            children,
            duplicated,
            query,
            window: (tree.window.width, tree.window.height),
            visiting: HashSet::new(),
        }
    }

    fn roots(&self) -> Vec<usize> {
        let known: HashSet<&str> = self.tree.widgets.iter().map(|w| w.id.as_str()).collect();
        self.tree
            .widgets
            .iter()
            .enumerate()
            .filter(|(_, w)| w.parent.as_deref().is_none_or(|p| !known.contains(p)))
            .map(|(index, _)| index)
            .collect()
    }

    /// Nœuds produits par le widget `index` : lui-même, ou ses descendants
    /// remontés d'un cran quand il n'est pas gardé.
    fn build(&mut self, index: usize) -> Vec<ElementNode> {
        let widget = &self.tree.widgets[index];
        // Un cycle de parents n'a pas de racine : le parcours depuis les racines
        // ne l'atteint pas, la garde protège le parcours depuis un `scope`.
        if !widget.state.visible || !self.visiting.insert(index) {
            return Vec::new();
        }
        let offscreen = is_offscreen(widget, self.window);
        if self.query.viewport_only && offscreen {
            self.visiting.remove(&index);
            return Vec::new();
        }
        let kids: Vec<usize> = self
            .children
            .get(widget.id.as_str())
            .cloned()
            .unwrap_or_default();
        let mut nested = Vec::new();
        for kid in kids {
            nested.extend(self.build(kid));
        }
        self.visiting.remove(&index);

        let keep = !self.query.interactive_only
            || is_interactive(widget)
            || (!nested.is_empty() && is_context(widget));
        if !keep {
            return nested;
        }
        let mut node = ElementNode::new(
            format!("{WIDGET_REF_PREFIX}{}", widget.id),
            widget.role.as_str(),
        )
        .with_state(
            ElementState::new()
                .disabled(!widget.state.enabled)
                .checked(widget.state.checked)
                .expanded(widget.state.expanded)
                .focused(widget.state.focused)
                .selected(widget.state.selected),
        )
        .with_rect(widget.rect)
        .with_scrollable(widget.scroll.is_some())
        .with_offscreen(offscreen)
        .with_ephemeral(widget.id_ephemeral || self.duplicated.contains(widget.id.as_str()))
        .with_children(nested);
        if let Some(label) = &widget.label {
            node = node.with_name(label.clone());
        }
        if let Some(value) = widget.value.as_ref().and_then(value_text) {
            node = node.with_value(value);
        }
        vec![node]
    }
}

/// Coupe l'arbre à `max_depth` niveaux ; vrai si quelque chose a été retranché.
fn limit_depth(nodes: &mut Vec<ElementNode>, remaining: u32) -> bool {
    if remaining == 0 {
        let cut = !nodes.is_empty();
        nodes.clear();
        return cut;
    }
    let mut cut = false;
    for node in nodes.iter_mut() {
        cut |= limit_depth(&mut node.children, remaining - 1);
    }
    cut
}

/// Garde les `budget` premiers nœuds (ordre du document) ; vrai si coupé.
fn limit_nodes(nodes: &mut Vec<ElementNode>, budget: &mut usize) -> bool {
    let mut cut = false;
    let mut kept = 0;
    for node in nodes.iter_mut() {
        if *budget == 0 {
            cut = true;
            break;
        }
        *budget -= 1;
        cut |= limit_nodes(&mut node.children, budget);
        kept += 1;
    }
    nodes.truncate(kept);
    cut
}

fn contains_ref(nodes: &[ElementNode], reference: &str) -> bool {
    nodes
        .iter()
        .any(|n| n.reference == reference || contains_ref(&n.children, reference))
}

#[async_trait]
impl ElementInspector for RouteWidgetsElements {
    async fn snapshot(&self, query: &SnapshotQuery) -> ElementResult<ElementSnapshot> {
        let tree = self.fetch().await?;
        self.remember(&tree);
        let mut builder = Builder::new(&tree, query);
        let mut nodes = match query.scope.as_deref() {
            None => {
                let roots = builder.roots();
                roots.into_iter().flat_map(|i| builder.build(i)).collect()
            }
            Some(scope) => match parse_ref(scope) {
                Some(RefKind::Widget(id)) => {
                    let mut found = tree
                        .widgets
                        .iter()
                        .enumerate()
                        .filter(|(_, w)| w.id == id)
                        .map(|(i, _)| i);
                    let (Some(index), None) = (found.next(), found.next()) else {
                        return Err(if tree.widgets.iter().any(|w| w.id == id) {
                            ElementError::Ambiguous {
                                reference: scope.to_string(),
                                count: tree.widgets.iter().filter(|w| w.id == id).count(),
                            }
                        } else {
                            self.missing(scope, id, &tree)
                        });
                    };
                    builder.build(index)
                }
                _ => {
                    return Err(ElementError::Unsupported {
                        hint: "`scope` attend une référence `w:<id>` sur les widgets natifs \
                               (pas de sélecteur CSS sans DOM)"
                            .to_string(),
                    })
                }
            },
        };
        let mut truncated = false;
        if let Some(depth) = query.max_depth {
            truncated |= limit_depth(&mut nodes, depth.max(1));
        }
        let mut budget = query.max_nodes.max(1);
        truncated |= limit_nodes(&mut nodes, &mut budget);
        let focus = tree
            .focus
            .as_deref()
            .map(|id| format!("{WIDGET_REF_PREFIX}{id}"))
            .filter(|reference| contains_ref(&nodes, reference));
        Ok(ElementSnapshot::new(0, WIDGETS_TARGET_KIND, nodes)
            .with_truncated(truncated)
            .with_focus(focus))
    }

    async fn locate(&self, reference: &str) -> ElementResult<ElementLocation> {
        let Some(RefKind::Widget(id)) = parse_ref(reference) else {
            return Err(ElementError::UnknownRef(reference.to_string()));
        };
        let tree = self.fetch().await?;
        self.remember(&tree);
        let mut matching = tree.widgets.iter().filter(|w| w.id == id);
        let Some(widget) = matching.next() else {
            return Err(self.missing(reference, id, &tree));
        };
        let extra = matching.count();
        if extra > 0 {
            return Err(ElementError::Ambiguous {
                reference: reference.to_string(),
                count: extra + 1,
            });
        }
        let [_, _, w, h] = widget.rect;
        let offscreen = !widget.state.visible
            || w <= 0
            || h <= 0
            || is_offscreen(widget, (tree.window.width, tree.window.height));
        Ok(ElementLocation::new(reference, widget.rect)
            .with_offscreen(offscreen)
            .with_disabled(!widget.state.enabled))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deveye_core::{CommandDescriptor, UiState, UiTree};
    use deveye_ports::PortResult;
    use serde_json::{json, Value};

    /// Route fixe dont l'arbre change entre deux appels, comme une app vivante.
    struct Live(Mutex<PortResult<Value>>);

    impl Live {
        fn serving(tree: &UiTree) -> Arc<Self> {
            Arc::new(Self(Mutex::new(Ok(serde_json::to_value(tree).unwrap()))))
        }

        fn set(&self, tree: &UiTree) {
            *self.0.lock().unwrap() = Ok(serde_json::to_value(tree).unwrap());
        }

        fn set_raw(&self, value: PortResult<Value>) {
            *self.0.lock().unwrap() = value;
        }
    }

    #[async_trait]
    impl CommandRouter for Live {
        async fn invoke(&self, route: &str, _payload: Value) -> PortResult<Value> {
            assert_eq!(route, "ui.widgets");
            match &*self.0.lock().unwrap() {
                Ok(value) => Ok(value.clone()),
                Err(PortError::Route { route, message }) => Err(PortError::Route {
                    route: route.clone(),
                    message: message.clone(),
                }),
                Err(PortError::Network(m)) => Err(PortError::Network(m.clone())),
                Err(other) => Err(PortError::Inspect(other.to_string())),
            }
        }

        async fn list(&self) -> PortResult<Vec<CommandDescriptor>> {
            Ok(Vec::new())
        }
    }

    fn inspector(live: &Arc<Live>) -> RouteWidgetsElements {
        RouteWidgetsElements::new(live.clone(), "ui.widgets")
    }

    fn form() -> UiTree {
        let mut tree = UiTree::new(deveye_core::UiWindow::new(400, 300));
        tree.focus = Some("name".to_string());
        tree.widgets = vec![
            UiWidget::new("root", "window")
                .with_label("Formulaire")
                .with_rect([0, 0, 400, 300]),
            UiWidget::new("panel", "panel")
                .with_parent("root")
                .with_rect([0, 0, 400, 300]),
            UiWidget::new("title", "label")
                .with_parent("panel")
                .with_label("Titre")
                .with_rect([0, 0, 100, 20]),
            UiWidget::new("name", "textbox")
                .with_parent("panel")
                .with_label("Nom")
                .with_value("alice")
                .with_rect([10, 30, 200, 24]),
            UiWidget::new("pwd", "password")
                .with_parent("panel")
                .with_label("Mot de passe")
                .with_value("hunter2")
                .with_rect([10, 60, 200, 24]),
            UiWidget::new("save", "button")
                .with_parent("panel")
                .with_label("Enregistrer")
                .with_rect([100, 80, 80, 30]),
            UiWidget::new("off", "button")
                .with_parent("panel")
                .with_label("Hors vue")
                .with_rect([100, 900, 80, 30]),
            UiWidget::new("ghost", "button")
                .with_parent("panel")
                .with_label("Caché")
                .with_rect([0, 0, 10, 10])
                .with_state(UiState::new().with_visible(false)),
        ];
        tree
    }

    fn refs(nodes: &[ElementNode], out: &mut Vec<String>) {
        for node in nodes {
            out.push(node.reference.clone());
            refs(&node.children, out);
        }
    }

    fn all_refs(snapshot: &ElementSnapshot) -> Vec<String> {
        let mut out = Vec::new();
        refs(&snapshot.nodes, &mut out);
        out
    }

    #[tokio::test]
    async fn snapshot_lists_widget_references_and_flattens_anonymous_panels() {
        let live = Live::serving(&form());
        let snap = inspector(&live)
            .snapshot(&SnapshotQuery::new())
            .await
            .unwrap();
        assert_eq!(snap.target_kind, "widgets");
        assert_eq!(snap.generation, 0);
        // Fenêtre nommée gardée, panneau anonyme aplati, label écarté (non interactif),
        // widget invisible absent.
        assert_eq!(
            all_refs(&snap),
            ["w:root", "w:name", "w:pwd", "w:save", "w:off"]
        );
        assert_eq!(snap.nodes[0].children.len(), 4);
        assert_eq!(snap.focus.as_deref(), Some("w:name"));
        let off = snap.nodes[0].children.last().unwrap();
        assert!(off.offscreen && !snap.nodes[0].children[0].offscreen);
    }

    #[tokio::test]
    async fn snapshot_never_carries_a_secret_value() {
        let live = Live::serving(&form());
        let snap = inspector(&live)
            .snapshot(&SnapshotQuery::new().with_interactive_only(false))
            .await
            .unwrap();
        let text = serde_json::to_string(&snap).unwrap();
        assert!(!text.contains("hunter2"), "{text}");
        assert!(text.contains("alice"));
        let pwd = snap.nodes[0].children[0]
            .children
            .iter()
            .find(|n| n.reference == "w:pwd")
            .unwrap();
        assert!(pwd.value.is_none());
    }

    #[tokio::test]
    async fn two_snapshots_give_the_same_references() {
        let live = Live::serving(&form());
        let inspector = inspector(&live);
        let a = inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        let b = inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        assert_eq!(all_refs(&a), all_refs(&b));
    }

    #[tokio::test]
    async fn non_interactive_mode_keeps_every_visible_widget() {
        let live = Live::serving(&form());
        let snap = inspector(&live)
            .snapshot(&SnapshotQuery::new().with_interactive_only(false))
            .await
            .unwrap();
        let refs = all_refs(&snap);
        assert!(refs.contains(&"w:title".to_string()) && refs.contains(&"w:panel".to_string()));
        assert!(!refs.contains(&"w:ghost".to_string()));
    }

    #[tokio::test]
    async fn viewport_only_drops_offscreen_widgets() {
        let live = Live::serving(&form());
        let snap = inspector(&live)
            .snapshot(&SnapshotQuery::new().with_viewport_only(true))
            .await
            .unwrap();
        assert!(!all_refs(&snap).contains(&"w:off".to_string()));
    }

    #[tokio::test]
    async fn node_and_depth_budgets_flag_the_truncation() {
        let live = Live::serving(&form());
        let inspector = inspector(&live);
        let snap = inspector
            .snapshot(&SnapshotQuery::new().with_max_nodes(2))
            .await
            .unwrap();
        assert_eq!(snap.count(), 2);
        assert!(snap.truncated);
        let snap = inspector
            .snapshot(&SnapshotQuery::new().with_max_depth(1))
            .await
            .unwrap();
        assert_eq!(all_refs(&snap), ["w:root"]);
        assert!(snap.truncated);
        let snap = inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        assert!(!snap.truncated);
    }

    #[tokio::test]
    async fn scope_restricts_to_a_subtree_and_rejects_css() {
        let live = Live::serving(&form());
        let inspector = inspector(&live);
        let snap = inspector
            .snapshot(&SnapshotQuery::new().with_scope("w:save"))
            .await
            .unwrap();
        assert_eq!(all_refs(&snap), ["w:save"]);
        let err = inspector
            .snapshot(&SnapshotQuery::new().with_scope("button.primary"))
            .await
            .unwrap_err();
        assert!(matches!(err, ElementError::Unsupported { .. }), "{err:?}");
        let err = inspector
            .snapshot(&SnapshotQuery::new().with_scope("w:nope"))
            .await
            .unwrap_err();
        assert!(matches!(err, ElementError::UnknownRef(_)), "{err:?}");
    }

    #[tokio::test]
    async fn locate_remeasures_the_rect_at_every_call() {
        let mut tree = form();
        let live = Live::serving(&tree);
        let inspector = inspector(&live);
        let first = inspector.locate("w:save").await.unwrap();
        assert_eq!(first.rect, [100, 80, 80, 30]);
        assert_eq!(first.center(), (140, 95));
        // L'app re-met en page entre l'instantané et l'action.
        tree.widgets[5].rect = [200, 150, 80, 30];
        live.set(&tree);
        let moved = inspector.locate("w:save").await.unwrap();
        assert_eq!(moved.center(), (240, 165));
        assert!(!moved.offscreen && !moved.disabled);
    }

    #[tokio::test]
    async fn locate_reports_offscreen_hidden_and_disabled_elements() {
        let mut tree = form();
        tree.widgets[5].state = UiState::new().with_enabled(false);
        let live = Live::serving(&tree);
        let inspector = inspector(&live);
        assert!(inspector.locate("w:save").await.unwrap().disabled);
        assert!(inspector.locate("w:off").await.unwrap().offscreen);
        assert!(inspector.locate("w:ghost").await.unwrap().offscreen);
    }

    #[tokio::test]
    async fn a_removed_id_is_stale_with_same_role_and_name_candidates() {
        let mut tree = form();
        let live = Live::serving(&tree);
        let inspector = inspector(&live);
        inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        // L'id disparaît, un autre bouton de même rôle et nom apparaît (liste recréée).
        tree.widgets[5].id = "save#2".to_string();
        live.set(&tree);
        let err = inspector.locate("w:save").await.unwrap_err();
        match err {
            ElementError::Stale {
                reference,
                candidates,
            } => {
                assert_eq!(reference, "w:save");
                assert_eq!(candidates, ["w:save#2"]);
            }
            other => panic!("attendu Stale : {other:?}"),
        }
        // Une seconde tentative reste périmée (la mémoire n'oublie pas le périmé).
        assert!(matches!(
            inspector.locate("w:save").await.unwrap_err(),
            ElementError::Stale { .. }
        ));
    }

    #[tokio::test]
    async fn a_never_seen_id_and_a_dom_reference_are_unknown() {
        let live = Live::serving(&form());
        let inspector = inspector(&live);
        for reference in ["w:jamais", "e12", "n'importe quoi", "w:"] {
            let err = inspector.locate(reference).await.unwrap_err();
            assert!(
                matches!(err, ElementError::UnknownRef(_)),
                "{reference} : {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn duplicated_ids_are_ephemeral_and_refuse_to_locate() {
        let mut tree = form();
        tree.widgets.push(
            UiWidget::new("save", "button")
                .with_parent("panel")
                .with_label("Autre")
                .with_rect([300, 80, 50, 30]),
        );
        let live = Live::serving(&tree);
        let inspector = inspector(&live);
        let snap = inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        let ephemeral: Vec<_> = snap.nodes[0]
            .children
            .iter()
            .filter(|n| n.ephemeral)
            .map(|n| n.reference.as_str())
            .collect();
        assert_eq!(ephemeral, ["w:save", "w:save"]);
        let err = inspector.locate("w:save").await.unwrap_err();
        assert!(
            matches!(err, ElementError::Ambiguous { count: 2, .. }),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn legacy_trees_yield_ephemeral_references() {
        let live = Live::serving(&form());
        live.set_raw(Ok(json!({
            "widgets": [
                { "kind": "button", "text": "OK", "rect": [0, 0, 10, 10] },
                { "id": "fixe", "kind": "button", "text": "Fixe", "rect": [0, 20, 10, 10] }
            ]
        })));
        let snap = inspector(&live)
            .snapshot(&SnapshotQuery::new())
            .await
            .unwrap();
        let by_ref: HashMap<_, _> = snap
            .nodes
            .iter()
            .map(|n| (n.reference.as_str(), n.ephemeral))
            .collect();
        assert!(!by_ref["w:fixe"]);
        assert_eq!(by_ref.values().filter(|e| **e).count(), 1, "{by_ref:?}");
    }

    #[tokio::test]
    async fn missing_route_and_unpublished_tree_are_unsupported_with_a_hint() {
        let live = Live::serving(&form());
        let inspector = inspector(&live);
        live.set_raw(Err(PortError::Route {
            route: "ui.widgets".into(),
            message: "route inconnue".into(),
        }));
        match inspector.snapshot(&SnapshotQuery::new()).await.unwrap_err() {
            ElementError::Unsupported { hint } => {
                assert!(
                    hint.contains("ui.widgets") && hint.contains("route inconnue"),
                    "{hint}"
                )
            }
            other => panic!("{other:?}"),
        }
        live.set_raw(Ok(json!({ "error": "aucun arbre publié" })));
        assert!(matches!(
            inspector.locate("w:x").await.unwrap_err(),
            ElementError::Unsupported { .. }
        ));
    }

    #[tokio::test]
    async fn a_dead_link_is_a_port_error_not_a_stale_reference() {
        let live = Live::serving(&form());
        let inspector = inspector(&live);
        inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        live.set_raw(Err(PortError::Network("coupé".into())));
        let err = inspector.locate("w:save").await.unwrap_err();
        assert!(
            matches!(err, ElementError::Port(PortError::Network(_))),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn long_values_are_cut() {
        let mut tree = form();
        tree.widgets[3] = UiWidget::new("name", "textbox")
            .with_parent("panel")
            .with_value("x".repeat(500))
            .with_rect([10, 30, 200, 24]);
        let live = Live::serving(&tree);
        let snap = inspector(&live)
            .snapshot(&SnapshotQuery::new())
            .await
            .unwrap();
        let name = snap.nodes[0]
            .children
            .iter()
            .find(|n| n.reference == "w:name")
            .unwrap();
        assert_eq!(
            name.value.as_ref().unwrap().chars().count(),
            VALUE_MAX_CHARS + 1
        );
    }
}
