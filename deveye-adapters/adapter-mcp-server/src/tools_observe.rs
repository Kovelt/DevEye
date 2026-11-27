//! Outil composite **observe** : en un appel, capture + résumé borné de l'arbre
//! d'interface + état + erreurs console + derniers logs (ADR-0029). Il ne
//! contient aucune logique d'accès propre : il compose les ports déjà utilisés
//! par `screenshot`, `dom`, `state`, `collect_console_errors` et `logs_tail`, et
//! chaque section se dégrade seule en « indisponible pour cette cible ».
//!
//! État et logs viennent des mêmes ports que `state`/`logs_tail`, donc du même
//! masquage des secrets côté SDK (ADR-0016) : `observe` n'ouvre aucune route qui
//! le contourne. Le texte libre des messages de log et les valeurs issues du DOM
//! ne sont pas filtrés, comme pour les outils qu'il compose.

use deveye_core::elements::SnapshotQuery;
use deveye_core::ConsoleLevel;
use deveye_ports::ElementError;

use super::imaging::{encode_capture, CaptureOptions};
use super::tools_perception::render_snapshot_text;
use super::*;

/// Largeur maximale par défaut de la capture d'`observe` (px) : l'outil vise
/// l'économie de tokens, une capture 4K pleine taille irait contre son objet.
/// `max_width` explicite la remplace.
pub(crate) const OBSERVE_DEFAULT_MAX_WIDTH: u32 = 1_024;
/// Nombre de nœuds de l'arbre restitués par défaut, et plafond demandable.
const DOM_DEFAULT_NODES: usize = 60;
const DOM_MAX_NODES_CAP: usize = 500;
/// Profondeur de l'arbre restituée par défaut, et plafond demandable.
const DOM_DEFAULT_DEPTH: usize = 6;
const DOM_MAX_DEPTH_CAP: usize = 30;
/// Nombre d'événements (logs, console) restitués par défaut, et plafond.
const EVENTS_DEFAULT: usize = 20;
const EVENTS_CAP: usize = 200;
/// Nombre de clés d'état détaillées, longueur max d'un aperçu de valeur, d'un
/// texte de nœud et d'une ligne de l'arbre.
const STATE_MAX_KEYS: usize = 30;
const PREVIEW_CHARS: usize = 120;
const NODE_TEXT_CHARS: usize = 60;
const OUTLINE_LINE_CHARS: usize = 200;
/// Aperçu JSON de repli quand l'arbre n'a pas la forme d'un arbre de nœuds.
const JSON_FALLBACK_CHARS: usize = 2_000;

/// Sections d'`observe`, dans l'ordre où elles sont produites.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Screenshot,
    Dom,
    State,
    Console,
    Logs,
    /// Références `snapshot` (expérimental, ADR-0073) : sur demande seulement.
    Elements,
}

impl Section {
    /// Sections produites quand `include` est absent : celles de la 1.0, dans
    /// l'ordre où elles sont produites. `elements` n'en fait pas partie, pour que
    /// la sortie par défaut d'`observe` reste celle de la 1.0.
    const DEFAULT: [Section; 5] = [
        Section::Screenshot,
        Section::Dom,
        Section::State,
        Section::Console,
        Section::Logs,
    ];

    /// Toutes les sections connues, dans l'ordre de production.
    const KNOWN: [Section; 6] = [
        Section::Screenshot,
        Section::Dom,
        Section::State,
        Section::Console,
        Section::Logs,
        Section::Elements,
    ];

    fn name(self) -> &'static str {
        match self {
            Section::Screenshot => "screenshot",
            Section::Dom => "dom",
            Section::State => "state",
            Section::Console => "console",
            Section::Logs => "logs",
            Section::Elements => "elements",
        }
    }

    fn parse(raw: &str) -> Result<Section, McpError> {
        let wanted = raw.trim().to_ascii_lowercase();
        Section::KNOWN
            .into_iter()
            .find(|s| s.name() == wanted)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "section inconnue : {raw:?} (attendu : screenshot, dom, state, console, logs, elements)"
                    ),
                    None,
                )
            })
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct ObserveParams {
    /// Sections à produire parmi `screenshot`, `dom`, `state`, `console`,
    /// `logs`. Absent : toutes.
    #[serde(default)]
    pub(crate) include: Option<Vec<String>>,
    /// Sections à retirer de l'ensemble retenu (appliqué après `include`).
    #[serde(default)]
    pub(crate) exclude: Option<Vec<String>>,
    /// Nombre maximal de nœuds de l'arbre d'interface restitués (défaut 60,
    /// plafond 500).
    #[serde(default)]
    pub(crate) max_dom_nodes: Option<usize>,
    /// Profondeur maximale de l'arbre restituée (défaut 6, plafond 30).
    #[serde(default)]
    pub(crate) max_dom_depth: Option<usize>,
    /// Nombre maximal d'événements restitués pour `console` et pour `logs`,
    /// les plus récents (défaut 20, plafond 200).
    #[serde(default)]
    pub(crate) max_events: Option<usize>,
    /// Options de capture. `max_width` vaut 1024 par défaut pour `observe`
    /// (au lieu de la taille d'origine pour `screenshot`).
    #[serde(flatten)]
    pub(crate) capture: CaptureOptions,
}

/// Section indisponible : l'agent lit pourquoi, et la suite continue.
fn unavailable(err: &PortError) -> serde_json::Value {
    let (reason, prefix) = match err {
        PortError::Unsupported(_) => ("unsupported", "indisponible pour cette cible"),
        PortError::Network(_) => ("unreachable", "indisponible (liaison coupée)"),
        PortError::Script { .. } => ("js_exception", "indisponible (exception JS)"),
        _ => ("error", "indisponible"),
    };
    let detail = match err {
        PortError::Unsupported(d) | PortError::Network(d) => d.clone(),
        other => other.to_string(),
    };
    serde_json::json!({
        "available": false,
        "reason": reason,
        "detail": format!("{prefix} : {detail}"),
    })
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let mut cut: String = text.chars().take(max_chars).collect();
        cut.push('…');
        cut
    }
}

/// Résumé borné d'un arbre d'interface.
#[derive(Debug, PartialEq)]
struct TreeSummary {
    /// `"outline"` (arbre de nœuds, une ligne par nœud) ou `"json"` (repli : aperçu
    /// du JSON brut quand la forme n'est pas celle d'un arbre de nœuds).
    kind: &'static str,
    text: String,
    total_nodes: usize,
    shown_nodes: usize,
}

/// Premier champ texte non vide parmi `keys`.
fn first_str<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .filter_map(|k| obj.get(*k).and_then(|v| v.as_str()))
        .find(|s| !s.trim().is_empty())
}

/// Valeur d'un attribut : format CDP (`attributes: [nom, valeur, ...]`) ou champ
/// direct d'un arbre de widgets (`{ "id": ... }`).
fn attribute(obj: &serde_json::Map<String, serde_json::Value>, name: &str) -> Option<String> {
    if let Some(flat) = obj.get("attributes").and_then(|a| a.as_array()) {
        let mut it = flat.iter();
        while let (Some(k), Some(v)) = (it.next(), it.next()) {
            if k.as_str() == Some(name) {
                return v.as_str().map(str::to_string).filter(|s| !s.is_empty());
            }
        }
    }
    match obj.get(name)? {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Nœuds sans intérêt pour un agent (commentaires, texte blanc, scripts, styles).
fn is_noise(obj: &serde_json::Map<String, serde_json::Value>) -> bool {
    match first_str(obj, &["nodeName"]) {
        Some("#comment") | Some("SCRIPT") | Some("STYLE") | Some("NOSCRIPT") => true,
        Some("#text") => obj
            .get("nodeValue")
            .and_then(|v| v.as_str())
            .is_none_or(|t| t.trim().is_empty()),
        _ => false,
    }
}

fn node_label(obj: &serde_json::Map<String, serde_json::Value>) -> String {
    let name = first_str(
        obj,
        &["nodeName", "tag", "type", "kind", "widget", "role", "name"],
    )
    .unwrap_or("?")
    .to_string();
    let mut line = name.to_ascii_lowercase();
    if line.starts_with('#') {
        // `#text`, `#document` : garder le dièse, il est déjà lisible.
        line = name;
    }
    if let Some(id) = attribute(obj, "id") {
        line.push('#');
        line.push_str(&id);
    }
    if let Some(class) = attribute(obj, "class") {
        for c in class.split_whitespace().take(3) {
            line.push('.');
            line.push_str(c);
        }
    }
    let text = obj
        .get("nodeValue")
        .and_then(|v| v.as_str())
        .filter(|t| !t.trim().is_empty())
        .or_else(|| first_str(obj, &["text", "label", "title", "value"]));
    if let Some(t) = text {
        let flat = t.split_whitespace().collect::<Vec<_>>().join(" ");
        line.push_str(&format!(" \"{}\"", clip(&flat, NODE_TEXT_CHARS)));
    }
    line
}

fn children_of(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> impl Iterator<Item = &serde_json::Map<String, serde_json::Value>> {
    obj.get("children")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .filter_map(|c| c.as_object())
        .filter(|c| !is_noise(c))
}

/// Compte les nœuds significatifs sous (et y compris) `obj`.
fn count_nodes(obj: &serde_json::Map<String, serde_json::Value>) -> usize {
    1 + children_of(obj).map(count_nodes).sum::<usize>()
}

/// Résume `tree` en au plus `max_nodes` lignes sur `max_depth` niveaux (parcours
/// en profondeur, ordre du document). Les nœuds retranchés par la profondeur
/// sont signalés (`[+N]`) ; ceux retranchés par le budget sont comptés dans
/// `total_nodes`.
fn summarize_tree(tree: &serde_json::Value, max_nodes: usize, max_depth: usize) -> TreeSummary {
    let is_node_tree = tree.as_object().is_some_and(|o| {
        first_str(
            o,
            &["nodeName", "tag", "type", "kind", "widget", "role", "name"],
        )
        .is_some()
            || o.contains_key("children")
    });
    let Some(root) = tree.as_object().filter(|_| is_node_tree) else {
        let raw = to_json_text(tree);
        return TreeSummary {
            kind: "json",
            text: clip(&raw, JSON_FALLBACK_CHARS),
            total_nodes: 0,
            shown_nodes: 0,
        };
    };
    let total_nodes = count_nodes(root);
    let mut lines = Vec::new();
    let mut stack = vec![(root, 0usize)];
    while let Some((node, depth)) = stack.pop() {
        if lines.len() >= max_nodes {
            break;
        }
        let mut line = format!("{}{}", "  ".repeat(depth), node_label(node));
        let kids: Vec<_> = children_of(node).collect();
        if depth + 1 >= max_depth && !kids.is_empty() {
            let hidden: usize = kids.iter().map(|k| count_nodes(k)).sum();
            line.push_str(&format!(" [+{hidden}]"));
        } else {
            for k in kids.into_iter().rev() {
                stack.push((k, depth + 1));
            }
        }
        lines.push(clip(&line, OUTLINE_LINE_CHARS));
    }
    TreeSummary {
        kind: "outline",
        shown_nodes: lines.len(),
        text: lines.join("\n"),
        total_nodes,
    }
}

#[tool_router(router = observe_router, vis = "pub(crate)")]
impl DeveyeMcpServer {
    #[tool(
        description = "Observe l'app en UN appel (remplace 4 à 5 appels) : capture (image MCP, 1024 px de large max par défaut ; options `max_width`, `format`, `quality` comme `screenshot`), résumé borné de l'arbre DOM/widgets (outline, `max_dom_nodes` 60, `max_dom_depth` 6), état SDK (aperçu des clés), erreurs et avertissements console récents, derniers logs (`max_events` 20, les plus récents). Renvoie l'image (si capturée) puis un bloc texte JSON `{ sections: { screenshot, dom, state, console, logs }, skipped }` ; chaque section porte `available: true` et ses données, ou `available: false` avec `reason` et `detail` (« indisponible pour cette cible ») sans faire échouer les autres. `include`/`exclude` choisissent les sections ; `include: [\"elements\"]` ajoute (expérimental, ADR-0073) l'arbre d'éléments de `snapshot` (`tree` texte compact avec les références, borné par `max_dom_nodes`), absent par défaut. DRAINE les buffers de logs et de console (comme `logs_tail`/`collect_console_errors`) : seuls les messages console error/warning sont restitués, les autres niveaux drainés sont comptés dans `ignored_lower_levels`. L'état et les champs de logs passent par le masquage des secrets du SDK. Pour le détail d'une section, utiliser l'outil dédié (`dom`, `state`, `logs_tail`, `find_element`).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(crate) async fn observe(
        &self,
        Parameters(p): Parameters<ObserveParams>,
    ) -> Result<CallToolResult, McpError> {
        // Validation de tous les paramètres AVANT le moindre accès aux ports :
        // un appel invalide ne doit ni capturer ni drainer un buffer.
        let capture = p
            .capture
            .resolve_with_default_width(Some(OBSERVE_DEFAULT_MAX_WIDTH))?;
        let mut wanted: Vec<Section> = match &p.include {
            Some(names) if !names.is_empty() => names
                .iter()
                .map(|n| Section::parse(n))
                .collect::<Result<_, _>>()?,
            _ => Section::DEFAULT.to_vec(),
        };
        let excluded: Vec<Section> = p
            .exclude
            .iter()
            .flatten()
            .map(|n| Section::parse(n))
            .collect::<Result<_, _>>()?;
        wanted.retain(|s| !excluded.contains(s));
        if wanted.is_empty() {
            return Err(McpError::invalid_params(
                "aucune section à observer après include/exclude".to_string(),
                None,
            ));
        }
        let max_nodes = p
            .max_dom_nodes
            .unwrap_or(DOM_DEFAULT_NODES)
            .clamp(1, DOM_MAX_NODES_CAP);
        let max_depth = p
            .max_dom_depth
            .unwrap_or(DOM_DEFAULT_DEPTH)
            .clamp(1, DOM_MAX_DEPTH_CAP);
        let max_events = p.max_events.unwrap_or(EVENTS_DEFAULT).min(EVENTS_CAP);

        let _serial = self.acquire_gate("observe").await;
        let mut sections = serde_json::Map::new();
        let mut image_block = None;

        for section in Section::KNOWN.into_iter().filter(|s| wanted.contains(s)) {
            let value = match section {
                Section::Screenshot => match self.renderer.screenshot().await {
                    Ok(shot) => match encode_capture(&shot.bytes, &capture) {
                        Ok(enc) => {
                            image_block = Some(enc.content());
                            serde_json::json!({
                                "available": true,
                                "captured_at": shot.captured_at,
                                "viewport": {
                                    "width": shot.viewport.width,
                                    "height": shot.viewport.height,
                                },
                                "image": enc.meta(None),
                            })
                        }
                        Err(e) => serde_json::json!({
                            "available": false,
                            "reason": "error",
                            "detail": format!("indisponible : {}", e.message),
                        }),
                    },
                    Err(e) => unavailable(&e),
                },
                Section::Dom => match self.inspector.dom_snapshot().await {
                    Ok(dom) => {
                        let summary = summarize_tree(&dom.tree, max_nodes, max_depth);
                        serde_json::json!({
                            "available": true,
                            "captured_at": dom.captured_at,
                            "format": summary.kind,
                            "total_nodes": summary.total_nodes,
                            "shown_nodes": summary.shown_nodes,
                            "truncated": summary.shown_nodes < summary.total_nodes,
                            "outline": summary.text,
                        })
                    }
                    Err(e) => unavailable(&e),
                },
                Section::State => match self.inspector.state_snapshot().await {
                    Ok(dump) => {
                        let entries: serde_json::Map<String, serde_json::Value> = dump
                            .entries
                            .iter()
                            .take(STATE_MAX_KEYS)
                            .map(|(k, v)| {
                                let raw = to_json_text(v);
                                (
                                    k.clone(),
                                    serde_json::Value::String(clip(&raw, PREVIEW_CHARS)),
                                )
                            })
                            .collect();
                        serde_json::json!({
                            "available": true,
                            "captured_at": dump.captured_at,
                            "signals": dump.entries.len(),
                            "truncated": dump.entries.len() > STATE_MAX_KEYS,
                            "entries": entries,
                        })
                    }
                    Err(e) => unavailable(&e),
                },
                Section::Console => match self.console_errors.drain_console_errors().await {
                    Ok(all) => {
                        let drained = all.len();
                        let serious: Vec<_> = all
                            .into_iter()
                            .filter(|e| e.level >= ConsoleLevel::Warning)
                            .collect();
                        let ignored = drained - serious.len();
                        let (kept, truncated, total) = truncate_recent(serious, max_events);
                        serde_json::json!({
                            "available": true,
                            "errors": kept,
                            "total": total,
                            "truncated": truncated,
                            "ignored_lower_levels": ignored,
                        })
                    }
                    Err(e) => unavailable(&e),
                },
                Section::Elements => {
                    let query = SnapshotQuery::new().with_max_nodes(max_nodes);
                    match self.elements.snapshot(&query).await {
                        Ok(snapshot) => serde_json::json!({
                            "available": true,
                            "target_kind": snapshot.target_kind,
                            "total_nodes": snapshot.count(),
                            "truncated": snapshot.truncated,
                            "focus": snapshot.focus,
                            "tree": render_snapshot_text(&snapshot),
                        }),
                        Err(ElementError::Unsupported { hint }) => serde_json::json!({
                            "available": false,
                            "reason": "unsupported",
                            "detail": format!("indisponible pour cette cible : {hint}"),
                        }),
                        Err(ElementError::Port(port)) => unavailable(&port),
                        Err(other) => serde_json::json!({
                            "available": false,
                            "reason": "error",
                            "detail": format!("indisponible : {other}"),
                        }),
                    }
                }
                Section::Logs => match self.log_source.poll().await {
                    Ok(events) => {
                        let (kept, truncated, total) = truncate_recent(events, max_events);
                        serde_json::json!({
                            "available": true,
                            "events": kept,
                            "total": total,
                            "truncated": truncated,
                        })
                    }
                    Err(e) => unavailable(&e),
                },
            };
            sections.insert(section.name().to_string(), value);
        }

        // `elements`, sur demande seulement, n'est jamais « sauté » : son absence est le défaut.
        let skipped: Vec<&str> = Section::DEFAULT
            .into_iter()
            .filter(|s| !wanted.contains(s))
            .map(Section::name)
            .collect();
        let mut blocks = Vec::with_capacity(2);
        blocks.extend(image_block);
        blocks.push(meta_block(&serde_json::json!({
            "sections": sections,
            "skipped": skipped,
        })));
        Ok(CallToolResult::success(blocks))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text_node(value: &str) -> serde_json::Value {
        json!({ "nodeName": "#text", "nodeValue": value })
    }

    fn sample_dom() -> serde_json::Value {
        json!({
            "nodeName": "#document",
            "children": [{
                "nodeName": "HTML",
                "children": [
                    { "nodeName": "HEAD", "children": [
                        { "nodeName": "STYLE", "children": [text_node("body{}")] },
                        { "nodeName": "#comment", "nodeValue": "x" }
                    ]},
                    { "nodeName": "BODY", "attributes": ["class", "app dark"], "children": [
                        { "nodeName": "DIV", "attributes": ["id", "root"], "children": [
                            { "nodeName": "BUTTON", "attributes": ["class", "btn primary"], "children": [text_node("  Valider  ")] },
                            text_node("   ")
                        ]}
                    ]}
                ]
            }]
        })
    }

    #[test]
    fn cdp_tree_is_summarized_without_noise() {
        let s = summarize_tree(&sample_dom(), 60, 10);
        assert_eq!(s.kind, "outline");
        assert!(
            !s.text.contains("STYLE") && !s.text.contains("style"),
            "{}",
            s.text
        );
        assert!(!s.text.contains("#comment"), "{}", s.text);
        assert!(s.text.contains("body.app.dark"), "{}", s.text);
        assert!(s.text.contains("div#root"), "{}", s.text);
        assert!(s.text.contains("button.btn.primary"), "{}", s.text);
        assert!(s.text.contains("\"Valider\""), "texte aplati : {}", s.text);
        assert_eq!(s.total_nodes, s.shown_nodes);
        // document, html, head, body, div, button, #text "Valider"
        assert_eq!(s.total_nodes, 7);
    }

    #[test]
    fn node_budget_truncates_and_reports_the_total() {
        let s = summarize_tree(&sample_dom(), 3, 10);
        assert_eq!(s.shown_nodes, 3);
        assert_eq!(s.total_nodes, 7);
        assert_eq!(s.text.lines().count(), 3);
    }

    #[test]
    fn depth_budget_folds_subtrees_with_a_count() {
        let s = summarize_tree(&sample_dom(), 60, 2);
        // document (profondeur 0), html (1, ses enfants repliés).
        assert_eq!(s.shown_nodes, 2);
        assert!(
            s.text.lines().nth(1).is_some_and(|l| l.contains("[+")),
            "{}",
            s.text
        );
    }

    #[test]
    fn widget_tree_uses_direct_fields() {
        let tree = json!({
            "type": "Window",
            "children": [
                { "type": "Button", "id": 7, "label": "Enregistrer", "children": [] },
                { "type": "Label", "text": "Prêt" }
            ]
        });
        let s = summarize_tree(&tree, 60, 6);
        assert_eq!(s.kind, "outline");
        assert!(s.text.contains("button#7 \"Enregistrer\""), "{}", s.text);
        assert!(s.text.contains("label \"Prêt\""), "{}", s.text);
        assert_eq!(s.total_nodes, 3);
    }

    #[test]
    fn unrecognized_shape_falls_back_to_a_bounded_json_preview() {
        let big = json!({ "payload": "x".repeat(5_000) });
        let s = summarize_tree(&big, 60, 6);
        assert_eq!(s.kind, "json");
        assert!(s.text.chars().count() <= JSON_FALLBACK_CHARS + 1);
        let s = summarize_tree(&json!([1, 2, 3]), 60, 6);
        assert_eq!(s.kind, "json");
    }

    #[test]
    fn section_parse_is_case_insensitive_and_rejects_unknown_names() {
        assert_eq!(Section::parse(" DOM ").ok(), Some(Section::Dom));
        let err = Section::parse("network").expect_err("section inconnue");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn clip_is_char_boundary_safe() {
        assert_eq!(clip("éàü", 2), "éà…");
        assert_eq!(clip("abc", 3), "abc");
    }
}
