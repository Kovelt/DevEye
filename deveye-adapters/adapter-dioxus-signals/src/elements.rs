//! `ElementInspector` du pont JS : désigner un élément du DOM par une référence
//! `e<n>` sans CDP, pour une app dont le webview n'expose pas de protocole de
//! debug (WebKitGTK sous Linux). Surface expérimentale (ADR-0069, ADR-0074).
//!
//! Le parcours du DOM est un script JavaScript **synchrone** ([`SCRIPT`]) évalué
//! par un [`JsEvaluator`] (la pompe `document::eval` du SDK, ou n'importe quel
//! autre évaluateur : le script se teste dans Chromium). Le script donne à chaque
//! élément une clé `<epoch>:<n>` (WeakMap élément vers numéro, Map numéro vers
//! WeakRef) ; ce module la traduit en `e<n>` par une [`RefTable`], qui garantit
//! qu'un numéro n'est jamais réutilisé, même quand la page est rechargée.
//!
//! Pas de rebind silencieux : une clé morte répond `Stale` avec, comme candidats,
//! les éléments vivants de même rôle et de même nom.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use serde::Deserialize;

use deveye_core::elements::{
    parse_ref, ElementLocation, ElementNode, ElementSnapshot, ElementState, RefError, RefKind,
    RefTable, SnapshotQuery,
};
use deveye_ports::{
    ElementError, ElementInspector, ElementResult, JsEvaluator, PortError, PortResult,
};

/// Le script du pont : une expression JavaScript qui s'installe dans
/// `window.__deveye_elements__` et renvoie son API. Exposé pour les tests dans un
/// vrai navigateur.
pub const SCRIPT: &str = include_str!("elements_bridge.js");

/// Version de l'API du script ; une page qui en porte une autre est réinstallée.
const SCRIPT_VERSION: u32 = 1;

/// Famille de cible annoncée par les instantanés de ce module.
pub const BRIDGE_TARGET_KIND: &str = "bridge";

/// Nombre maximal de candidats proposés par une erreur `Stale`.
const MAX_CANDIDATES: usize = 5;

/// Au-delà, la mémoire des rôles et noms vus est repartie de zéro : elle ne sert
/// qu'à nommer des périmés récents.
const MAX_SEEN: usize = 10_000;

#[derive(Debug, Deserialize)]
struct JsState {
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    checked: Option<bool>,
    #[serde(default)]
    expanded: Option<bool>,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    selected: bool,
}

#[derive(Debug, Deserialize)]
struct JsNode {
    key: String,
    role: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    value: Option<String>,
    state: JsState,
    rect: [i32; 4],
    #[serde(default)]
    scrollable: bool,
    #[serde(default)]
    offscreen: bool,
    #[serde(default)]
    children: Vec<JsNode>,
}

#[derive(Debug, Deserialize)]
struct JsSnapshot {
    epoch: String,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    nodes: Vec<JsNode>,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    focus: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JsLocation {
    epoch: String,
    state: String,
    #[serde(default)]
    rect: Option<[i32; 4]>,
    #[serde(default)]
    offscreen: bool,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    by: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    role: String,
    name: Option<String>,
}

/// État partagé entre appels : la table de références, l'epoch de la page qui l'a
/// remplie et le rôle et le nom de chaque référence émise.
#[derive(Default)]
struct State {
    table: RefTable<String>,
    epoch: Option<String>,
    seen: HashMap<String, Seen>,
}

impl State {
    /// Une page rechargée (ou un autre backend) change d'epoch : toutes les
    /// références émises avant deviennent périmées, sans qu'aucun numéro ne
    /// soit réutilisé.
    fn adopt_epoch(&mut self, epoch: &str) {
        if self.epoch.as_deref() != Some(epoch) {
            if self.epoch.is_some() {
                self.table.invalidate();
            }
            self.epoch = Some(epoch.to_string());
        }
    }

    fn convert(&mut self, node: JsNode) -> ElementNode {
        let reference = self.table.reference_for(&node.key);
        if self.seen.len() > MAX_SEEN {
            self.seen.clear();
        }
        self.seen.insert(
            reference.clone(),
            Seen {
                role: node.role.clone(),
                name: node.name.clone(),
            },
        );
        let children = node.children.into_iter().map(|c| self.convert(c)).collect();
        let mut out = ElementNode::new(reference, node.role)
            .with_state(
                ElementState::new()
                    .disabled(node.state.disabled)
                    .checked(node.state.checked)
                    .expanded(node.state.expanded)
                    .focused(node.state.focused)
                    .selected(node.state.selected),
            )
            .with_rect(node.rect)
            .with_scrollable(node.scrollable)
            .with_offscreen(node.offscreen)
            .with_children(children);
        if let Some(name) = node.name {
            out = out.with_name(name);
        }
        if let Some(value) = node.value {
            out = out.with_value(value);
        }
        out
    }
}

/// Éléments d'une page web, lus par le script du pont via un [`JsEvaluator`].
pub struct BridgeElementInspector {
    evaluator: Arc<dyn JsEvaluator>,
    state: Mutex<State>,
}

impl BridgeElementInspector {
    pub fn new(evaluator: Arc<dyn JsEvaluator>) -> Self {
        Self {
            evaluator,
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Appelle `api.<method>(<arg>)` dans la page. Le script n'est envoyé qu'à la
    /// première utilisation (ou après un rechargement de page) : l'appel court
    /// répond `__install` quand l'API est absente.
    async fn call(&self, method: &str, arg: &serde_json::Value) -> PortResult<serde_json::Value> {
        let short = format!(
            "(function(){{ var a = window.__deveye_elements__; \
             if (!a || a.version !== {SCRIPT_VERSION}) return {{ __install: true }}; \
             return a.{method}({arg}); }})()"
        );
        let value = self.evaluator.evaluate(&short).await?;
        if value.get("__install").is_none() {
            return Ok(value);
        }
        let full = format!(
            "(function(){{ var a = window.__deveye_elements__; \
             if (!a || a.version !== {SCRIPT_VERSION}) a = ({SCRIPT}); \
             return a.{method}({arg}); }})()"
        );
        self.evaluator.evaluate(&full).await
    }

    fn decode<T: for<'de> Deserialize<'de>>(value: serde_json::Value) -> ElementResult<T> {
        serde_json::from_value(value).map_err(|e| {
            ElementError::Port(PortError::Inspect(format!(
                "réponse du script d'éléments illisible : {e}"
            )))
        })
    }

    fn stale(reference: &str, candidates: Vec<String>) -> ElementError {
        ElementError::Stale {
            reference: reference.to_string(),
            candidates,
        }
    }

    /// Éléments vivants de même rôle et de même nom que `reference`, d'après un
    /// instantané frais. Meilleur effort : une panne donne une liste vide.
    async fn candidates(&self, reference: &str) -> Vec<String> {
        let Some(wanted) = self.state().seen.get(reference).cloned() else {
            return Vec::new();
        };
        let Some(name) = wanted.name.clone() else {
            return Vec::new();
        };
        let Ok(fresh) = self.snapshot(&SnapshotQuery::new()).await else {
            return Vec::new();
        };
        let mut found = Vec::new();
        collect_matching(&fresh.nodes, &wanted.role, &name, &mut found);
        found.retain(|r| r != reference);
        found.truncate(MAX_CANDIDATES);
        found
    }
}

/// Référence portée par une erreur de résolution (`RefError` est `#[non_exhaustive]`).
fn ref_text(error: RefError) -> String {
    match error {
        RefError::Unknown(r) | RefError::Stale(r) => r,
        other => other.to_string(),
    }
}

fn collect_matching(nodes: &[ElementNode], role: &str, name: &str, out: &mut Vec<String>) {
    for node in nodes {
        if node.role == role && node.name.as_deref() == Some(name) {
            out.push(node.reference.clone());
        }
        collect_matching(&node.children, role, name, out);
    }
}

#[async_trait]
impl ElementInspector for BridgeElementInspector {
    async fn snapshot(&self, query: &SnapshotQuery) -> ElementResult<ElementSnapshot> {
        let mut arg = serde_json::json!({
            "max_nodes": query.max_nodes,
            "interactive_only": query.interactive_only,
            "viewport_only": query.viewport_only,
        });
        if let Some(depth) = query.max_depth {
            arg["max_depth"] = depth.into();
        }
        let scope_ref = query.scope.as_deref();
        if let Some(scope) = scope_ref {
            match parse_ref(scope) {
                Some(RefKind::Dom(_)) => {
                    let key = self.state().table.resolve(scope).cloned();
                    match key {
                        Ok(key) => arg["scope_key"] = key.into(),
                        Err(RefError::Stale(r)) => return Err(Self::stale(&r, Vec::new())),
                        Err(other) => return Err(ElementError::UnknownRef(ref_text(other))),
                    }
                }
                Some(RefKind::Widget(_)) => {
                    return Err(ElementError::UnknownRef(scope.to_string()));
                }
                None => arg["scope_selector"] = scope.into(),
            }
        }

        let raw = self.call("snapshot", &arg).await?;
        let reply: JsSnapshot = Self::decode(raw)?;
        let mut state = self.state();
        state.adopt_epoch(&reply.epoch);
        match reply.error.as_deref() {
            None => {}
            Some("foreign" | "gone") => {
                return Err(Self::stale(scope_ref.unwrap_or_default(), Vec::new()));
            }
            Some("scope_not_found") => {
                return Err(ElementError::Port(PortError::Inspect(format!(
                    "aucun élément ne correspond au scope `{}`",
                    scope_ref.unwrap_or_default()
                ))));
            }
            Some(other) => {
                return Err(ElementError::Port(PortError::Inspect(format!(
                    "scope refusé par le script d'éléments ({other}) : `{}`",
                    scope_ref.unwrap_or_default()
                ))));
            }
        }
        let nodes: Vec<ElementNode> = reply.nodes.into_iter().map(|n| state.convert(n)).collect();
        let focus = reply.focus.map(|key| state.table.reference_for(&key));
        let generation = state.table.generation();
        Ok(ElementSnapshot::new(generation, BRIDGE_TARGET_KIND, nodes)
            .with_truncated(reply.truncated)
            .with_focus(focus))
    }

    async fn locate(&self, reference: &str) -> ElementResult<ElementLocation> {
        let key = match parse_ref(reference) {
            Some(RefKind::Dom(_)) => self.state().table.resolve(reference).cloned(),
            _ => return Err(ElementError::UnknownRef(reference.to_string())),
        };
        let key = match key {
            Ok(key) => key,
            Err(RefError::Stale(r)) => {
                let candidates = self.candidates(&r).await;
                return Err(Self::stale(&r, candidates));
            }
            Err(other) => return Err(ElementError::UnknownRef(ref_text(other))),
        };

        let raw = self
            .call("locate", &serde_json::Value::from(key.as_str()))
            .await?;
        let reply: JsLocation = Self::decode(raw)?;
        let foreign = {
            let mut state = self.state();
            let changed = state.epoch.as_deref() != Some(reply.epoch.as_str());
            state.adopt_epoch(&reply.epoch);
            changed
        };
        match reply.state.as_str() {
            "ok" => {
                let rect = reply.rect.ok_or_else(|| {
                    ElementError::Port(PortError::Inspect(
                        "le script d'éléments n'a pas rendu de rect".to_string(),
                    ))
                })?;
                Ok(ElementLocation::new(reference, rect)
                    .with_offscreen(reply.offscreen)
                    .with_disabled(reply.disabled))
            }
            "obscured" => {
                let by = reply
                    .by
                    .map(|covering| self.state().table.reference_for(&covering));
                Err(ElementError::Obscured { by })
            }
            "gone" | "foreign" => {
                if !foreign {
                    self.state().table.forget(&key);
                }
                let candidates = self.candidates(reference).await;
                Err(Self::stale(reference, candidates))
            }
            other => Err(ElementError::Port(PortError::Inspect(format!(
                "état de localisation inconnu : {other}"
            )))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Évaluateur scripté : rend les réponses dans l'ordre, note les expressions.
    struct Scripted {
        replies: Mutex<VecDeque<PortResult<serde_json::Value>>>,
        seen: Mutex<Vec<String>>,
    }

    impl Scripted {
        fn new(replies: Vec<PortResult<serde_json::Value>>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies.into()),
                seen: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl JsEvaluator for Scripted {
        async fn evaluate(&self, expression: &str) -> PortResult<serde_json::Value> {
            self.seen.lock().unwrap().push(expression.to_string());
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(PortError::Inspect("plus de réponse".into())))
        }
    }

    fn node(key: &str, role: &str, name: &str) -> serde_json::Value {
        serde_json::json!({
            "key": key, "role": role, "name": name,
            "state": {"disabled": false, "checked": null, "expanded": null,
                      "focused": false, "selected": false},
            "rect": [10, 20, 100, 40], "scrollable": false, "offscreen": false,
            "children": []
        })
    }

    fn snap(epoch: &str, nodes: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({ "epoch": epoch, "nodes": nodes, "truncated": false, "focus": null })
    }

    #[tokio::test]
    async fn script_is_sent_only_when_the_page_lacks_it() {
        let eval = Scripted::new(vec![
            Ok(serde_json::json!({ "__install": true })),
            Ok(snap("p1", vec![node("p1:1", "button", "OK")])),
            Ok(snap("p1", vec![node("p1:1", "button", "OK")])),
        ]);
        let inspector = BridgeElementInspector::new(eval.clone());
        inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        let seen = eval.seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert!(!seen[0].contains("WeakMap"), "premier appel : sans script");
        assert!(seen[1].contains("WeakMap"), "installation : avec le script");
    }

    #[tokio::test]
    async fn same_key_keeps_its_reference_across_snapshots() {
        let eval = Scripted::new(vec![
            Ok(snap("p1", vec![node("p1:1", "button", "OK")])),
            Ok(snap(
                "p1",
                vec![
                    node("p1:2", "button", "Annuler"),
                    node("p1:1", "button", "OK"),
                ],
            )),
        ]);
        let inspector = BridgeElementInspector::new(eval);
        let first = inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        let second = inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        assert_eq!(first.nodes[0].reference, "e1");
        assert_eq!(second.nodes[0].reference, "e2");
        assert_eq!(second.nodes[1].reference, "e1");
        assert_eq!(first.target_kind, BRIDGE_TARGET_KIND);
    }

    #[tokio::test]
    async fn a_new_epoch_stales_old_references_and_never_reuses_numbers() {
        let eval = Scripted::new(vec![
            Ok(snap("p1", vec![node("p1:1", "button", "OK")])),
            // Page rechargée : autre epoch, mêmes numéros côté page.
            Ok(snap("p2", vec![node("p2:1", "button", "OK")])),
            Ok(snap("p2", vec![node("p2:1", "button", "OK")])),
        ]);
        let inspector = BridgeElementInspector::new(eval);
        let before = inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        let after = inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        assert_eq!(before.nodes[0].reference, "e1");
        assert_eq!(after.nodes[0].reference, "e2");
        assert!(after.generation > before.generation);
        // `e1` est périmé : réponse sans appel à la page, candidats par instantané.
        match inspector.locate("e1").await {
            Err(ElementError::Stale { candidates, .. }) => assert_eq!(candidates, ["e2"]),
            other => panic!("attendu Stale : {other:?}"),
        }
    }

    #[tokio::test]
    async fn locate_maps_gone_to_stale_with_candidates_and_forgets_the_node() {
        let eval = Scripted::new(vec![
            Ok(snap(
                "p1",
                vec![
                    node("p1:1", "button", "Supprimer"),
                    node("p1:2", "button", "Supprimer"),
                ],
            )),
            Ok(serde_json::json!({ "epoch": "p1", "state": "gone" })),
            // Instantané frais pour les candidats : seul p1:2 survit.
            Ok(snap("p1", vec![node("p1:2", "button", "Supprimer")])),
            // Deuxième tentative sur la même référence : déjà oubliée.
            Ok(snap("p1", vec![node("p1:2", "button", "Supprimer")])),
        ]);
        let inspector = BridgeElementInspector::new(eval.clone());
        inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        match inspector.locate("e1").await {
            Err(ElementError::Stale {
                reference,
                candidates,
            }) => {
                assert_eq!(reference, "e1");
                assert_eq!(candidates, ["e2"]);
            }
            other => panic!("attendu Stale : {other:?}"),
        }
        assert!(matches!(
            inspector.locate("e1").await,
            Err(ElementError::Stale { .. })
        ));
        // Une référence jamais émise reste « inconnue ».
        assert!(matches!(
            inspector.locate("e99").await,
            Err(ElementError::UnknownRef(_))
        ));
        assert!(matches!(
            inspector.locate("w:x").await,
            Err(ElementError::UnknownRef(_))
        ));
    }

    #[tokio::test]
    async fn locate_reports_rect_disabled_offscreen_and_obscuring_reference() {
        let eval = Scripted::new(vec![
            Ok(snap("p1", vec![node("p1:1", "button", "OK")])),
            Ok(serde_json::json!({
                "epoch": "p1", "state": "ok", "rect": [5, 6, 30, 10],
                "offscreen": false, "disabled": true
            })),
            Ok(serde_json::json!({ "epoch": "p1", "state": "obscured", "by": "p1:7" })),
        ]);
        let inspector = BridgeElementInspector::new(eval);
        inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        let location = inspector.locate("e1").await.unwrap();
        assert_eq!(location.rect, [5, 6, 30, 10]);
        assert_eq!(location.center(), (20, 11));
        assert!(location.disabled && !location.offscreen);
        match inspector.locate("e1").await {
            Err(ElementError::Obscured { by }) => assert_eq!(by.as_deref(), Some("e2")),
            other => panic!("attendu Obscured : {other:?}"),
        }
    }

    #[tokio::test]
    async fn scope_is_a_reference_or_a_selector() {
        let eval = Scripted::new(vec![
            Ok(snap("p1", vec![node("p1:1", "button", "OK")])),
            Ok(snap("p1", vec![])),
            Ok(snap("p1", vec![])),
        ]);
        let inspector = BridgeElementInspector::new(eval.clone());
        inspector.snapshot(&SnapshotQuery::new()).await.unwrap();
        inspector
            .snapshot(&SnapshotQuery::new().with_scope("e1"))
            .await
            .unwrap();
        inspector
            .snapshot(&SnapshotQuery::new().with_scope("#panel"))
            .await
            .unwrap();
        {
            let seen = eval.seen.lock().unwrap();
            assert!(seen[1].contains(r#""scope_key":"p1:1""#), "{}", seen[1]);
            assert!(
                seen[2].contains(r##""scope_selector":"#panel""##),
                "{}",
                seen[2]
            );
        }
        assert!(matches!(
            inspector
                .snapshot(&SnapshotQuery::new().with_scope("e42"))
                .await,
            Err(ElementError::UnknownRef(_))
        ));
        assert!(matches!(
            inspector
                .snapshot(&SnapshotQuery::new().with_scope("w:x"))
                .await,
            Err(ElementError::UnknownRef(_))
        ));
    }

    #[tokio::test]
    async fn liaison_errors_pass_through_for_the_fallback() {
        let eval = Scripted::new(vec![Err(PortError::Network("pompe absente".into()))]);
        let inspector = BridgeElementInspector::new(eval);
        match inspector.snapshot(&SnapshotQuery::new()).await {
            Err(ElementError::Port(PortError::Network(_))) => {}
            other => panic!("attendu Port(Network) : {other:?}"),
        }
    }

    #[test]
    fn script_is_a_single_expression_without_trailing_semicolon() {
        let trimmed = SCRIPT.trim_end();
        assert!(trimmed.ends_with("})()"), "IIFE attendue");
        assert!(SCRIPT.contains("__deveye_elements__"));
        // La version de l'API du script et celle de Rust restent alignées.
        assert!(SCRIPT.contains(&format!("var VERSION = {SCRIPT_VERSION};")));
    }

    #[test]
    fn script_never_reads_password_values() {
        // Garde-fou de relecture : la lecture de valeur passe par `isSecretField`.
        let value_fn = SCRIPT.split("function valueOf").nth(1).unwrap();
        let head = &value_fn[..value_fn.find("var tag").unwrap()];
        assert!(head.contains("isSecretField(el)"));
    }

    /// Extrait un tableau de chaînes `var NOM = ['a', 'b'];` du script.
    fn js_list(name: &str) -> Vec<String> {
        let start = SCRIPT.find(&format!("var {name} = [")).expect(name);
        let body = &SCRIPT[start..];
        let body = &body[body.find('[').unwrap() + 1..body.find(']').unwrap()];
        body.split(',')
            .map(|item| item.trim().trim_matches('\'').to_string())
            .filter(|item| !item.is_empty())
            .collect()
    }

    #[test]
    fn script_secret_lists_cover_the_shared_vectors() {
        // Garde hors navigateur (le test Chromium est ignoré par défaut) : le
        // filet heuristique du script reconnaît chaque clé secrète des vecteurs
        // partagés (ADR-0064), sans quoi un champ nommé ainsi fuirait sa valeur.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/schemas/shared-vectors-1.json"
        );
        let vectors: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let contains = js_list("SECRET_CONTAINS");
        let suffixes = js_list("SECRET_SUFFIXES");
        let exact = js_list("SECRET_EXACT");
        assert!(!contains.is_empty() && !suffixes.is_empty() && !exact.is_empty());
        for list in ["required_secret_keys", "extended_secret_keys"] {
            for key in vectors["redaction"][list].as_array().unwrap() {
                let key = key.as_str().unwrap();
                let k: String = key
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .collect();
                let hit = contains.iter().any(|w| k.contains(w.as_str()))
                    || suffixes.iter().any(|w| k.ends_with(w.as_str()))
                    || exact.contains(&k);
                assert!(hit, "clé secrète non reconnue par le script : {key}");
            }
        }
    }
}
