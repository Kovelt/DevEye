//! Golden files du texte des outils à petite sortie et contrat des sorties
//! structurées (ADR-0067).
//!
//! Chaque scénario appelle un outil sur des ports factices et fige le texte
//! renvoyé (drapeau `isError` et blocs texte) dans `tests/golden/<scénario>.txt`.
//! Les fichiers ont été capturés **avant** l'ajout de `structuredContent` : un
//! client qui ignore les champs structurés doit continuer à lire exactement les
//! mêmes octets. Régénération volontaire : `DEVEYE_BLESS_GOLDEN=1 cargo test
//! -p adapter-mcp-server golden`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use async_trait::async_trait;
use deveye_core::{DomTree, WaitOutcome, WindowRect};
use serde_json::{json, Value};

use super::dispatch_tests::base_ports;
use super::*;

// ----------------------------------------------------------------------
// Ports factices configurables par une simple fonction.
// ----------------------------------------------------------------------

struct Wait(WaitOutcome);
#[async_trait]
impl WaitCondition for Wait {
    async fn wait_until(
        &self,
        _predicate: WaitPredicate,
        _timeout: Duration,
    ) -> PortResult<WaitOutcome> {
        Ok(self.0.clone())
    }
}

struct Eval(Box<dyn Fn() -> PortResult<Value> + Send + Sync>);
#[async_trait]
impl JsEvaluator for Eval {
    async fn evaluate(&self, _expression: &str) -> PortResult<Value> {
        (self.0)()
    }
}

struct State(Box<dyn Fn() -> PortResult<BTreeMap<String, Value>> + Send + Sync>);
#[async_trait]
impl AppInspector for State {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        Ok(DomTree::new(json!({})))
    }
    async fn state_snapshot(&self) -> PortResult<StateDump> {
        (self.0)().map(StateDump::new)
    }
}

struct Interaction(Box<dyn Fn() -> PortResult<()> + Send + Sync>);
#[async_trait]
impl InteractionSink for Interaction {
    async fn dispatch(&self, _command: InteractionCommand) -> PortResult<()> {
        (self.0)()
    }
}

struct Geometry(Option<WindowRect>);
#[async_trait]
impl WindowGeometry for Geometry {
    async fn geometry(&self, _title: Option<&str>) -> PortResult<WindowRect> {
        self.0.clone().ok_or_else(unsupported)
    }
}

struct Manager(Option<String>);
#[async_trait]
impl WindowManager for Manager {
    async fn activate(&self, _title: Option<&str>) -> PortResult<String> {
        self.0.clone().ok_or_else(unsupported)
    }
}

struct Pasteboard(bool);
#[async_trait]
impl Clipboard for Pasteboard {
    async fn get(&self) -> PortResult<String> {
        Ok(String::new())
    }
    async fn set(&self, _text: &str) -> PortResult<()> {
        if self.0 {
            Ok(())
        } else {
            Err(unsupported())
        }
    }
}

fn server(customize: impl FnOnce(&mut DeveyePorts)) -> DeveyeMcpServer {
    let mut ports = base_ports();
    customize(&mut ports);
    DeveyeMcpServer::new(ports).with_baseline_root(std::env::temp_dir())
}

fn unsupported() -> PortError {
    PortError::Unsupported("capacité absente (mock)".to_string())
}

fn state_with(entries: &[(&str, Value)]) -> State {
    let map: BTreeMap<String, Value> = entries
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect();
    State(Box::new(move || Ok(map.clone())))
}

/// Un scénario : son nom (nom du golden file), l'outil appelé et le résultat.
pub(crate) struct Scenario {
    pub(crate) name: &'static str,
    pub(crate) tool: &'static str,
    pub(crate) result: CallToolResult,
}

fn pos() -> PositionParams {
    PositionParams { x: 1, y: 1 }
}

fn ok(r: Result<CallToolResult, McpError>) -> CallToolResult {
    r.expect("résultat d'outil attendu, pas une erreur protocolaire")
}

/// Joue tous les scénarios des outils candidats au palier A : branches de
/// succès et échecs sémantiques (`isError`).
pub(crate) async fn scenarios() -> Vec<Scenario> {
    let mut out: Vec<Scenario> = Vec::new();
    macro_rules! push {
        ($name:literal, $tool:literal, $result:expr) => {
            out.push(Scenario {
                name: $name,
                tool: $tool,
                result: ok($result.await),
            })
        };
    }

    // target_info
    push!("target_info", "target_info", server(|_| {}).target_info());

    // wait_*
    let satisfied = || Wait(WaitOutcome::new(true, 12, "ok (mock)"));
    let timeout = || Wait(WaitOutcome::new(false, 5000, "délai écoulé (mock)"));
    let sel = |visible: Option<bool>, negate: Option<bool>| WaitForSelectorParams {
        selector: "#go".to_string(),
        timeout_ms: Some(10),
        visible,
        negate,
    };
    let s = server(|p| p.wait_condition = Arc::new(satisfied()));
    push!(
        "wait_for_selector_found",
        "wait_for_selector",
        s.wait_for_selector(Parameters(sel(Some(true), None)))
    );
    let s = server(|p| p.wait_condition = Arc::new(timeout()));
    push!(
        "wait_for_selector_timeout",
        "wait_for_selector",
        s.wait_for_selector(Parameters(sel(None, Some(true))))
    );
    let text = || WaitForTextParams {
        text: "Bonjour".to_string(),
        selector: None,
        visible: None,
        timeout_ms: Some(10),
    };
    let s = server(|p| p.wait_condition = Arc::new(satisfied()));
    push!(
        "wait_for_text_found",
        "wait_for_text",
        s.wait_for_text(Parameters(text()))
    );
    let s = server(|p| p.wait_condition = Arc::new(timeout()));
    push!(
        "wait_for_text_timeout",
        "wait_for_text",
        s.wait_for_text(Parameters(text()))
    );
    let cond = || WaitConditionParams {
        predicate: json!({ "kind": "target-alive" }),
        timeout_ms: Some(10),
    };
    let s = server(|p| p.wait_condition = Arc::new(satisfied()));
    push!(
        "wait_condition_satisfied",
        "wait_condition",
        s.wait_condition(Parameters(cond()))
    );
    let s = server(|p| p.wait_condition = Arc::new(timeout()));
    push!(
        "wait_condition_timeout",
        "wait_condition",
        s.wait_condition(Parameters(cond()))
    );
    let log = || WaitForLogParams {
        contains: "prêt".to_string(),
        level_min: Some("info".to_string()),
        timeout_ms: Some(10),
    };
    let s = server(|p| p.wait_condition = Arc::new(satisfied()));
    push!(
        "wait_for_log_satisfied",
        "wait_for_log",
        s.wait_for_log(Parameters(log()))
    );
    let s = server(|p| p.wait_condition = Arc::new(timeout()));
    push!(
        "wait_for_log_timeout",
        "wait_for_log",
        s.wait_for_log(Parameters(log()))
    );
    let region = || WaitScreenshotRegionParams {
        x: 0,
        y: 0,
        width: 2,
        height: 2,
        threshold_ms: Some(10),
        timeout_ms: Some(10),
    };
    let s = server(|p| p.wait_condition = Arc::new(satisfied()));
    push!(
        "wait_screenshot_region_satisfied",
        "wait_screenshot_region",
        s.wait_screenshot_region(Parameters(region()))
    );
    let s = server(|p| p.wait_condition = Arc::new(timeout()));
    push!(
        "wait_screenshot_region_timeout",
        "wait_screenshot_region",
        s.wait_screenshot_region(Parameters(region()))
    );
    let alive = || WaitTargetAliveParams {
        timeout_ms: Some(10),
    };
    let s = server(|p| p.wait_condition = Arc::new(satisfied()));
    push!(
        "wait_target_alive_satisfied",
        "wait_target_alive",
        s.wait_target_alive(Parameters(alive()))
    );
    let s = server(|p| p.wait_condition = Arc::new(timeout()));
    push!(
        "wait_target_alive_timeout",
        "wait_target_alive",
        s.wait_target_alive(Parameters(alive()))
    );
    let s = server(|p| p.inspector = Arc::new(state_with(&[("ready", json!(true))])));
    push!(
        "wait_ready_ready",
        "wait_ready",
        s.wait_ready(Parameters(WaitReadyParams { timeout_ms: 1000 }))
    );
    let s = server(|p| p.inspector = Arc::new(state_with(&[])));
    push!(
        "wait_ready_timeout",
        "wait_ready",
        s.wait_ready(Parameters(WaitReadyParams { timeout_ms: 0 }))
    );

    // assert_state
    let expect = |v: Value| AssertStateParams { expected: v };
    let s = server(|p| p.inspector = Arc::new(state_with(&[("page", json!("Agenda"))])));
    push!(
        "assert_state_ok",
        "assert_state",
        s.assert_state(Parameters(expect(json!({ "page": "Agenda" }))))
    );
    push!(
        "assert_state_mismatch",
        "assert_state",
        s.assert_state(Parameters(expect(json!({ "page": "Autre", "absent": 1 }))))
    );
    push!(
        "assert_state_expected_not_an_object",
        "assert_state",
        s.assert_state(Parameters(expect(json!([1, 2]))))
    );
    let s = server(|p| {
        p.inspector = Arc::new(State(Box::new(|| {
            Err(PortError::Network("connexion refusée (mock)".to_string()))
        })))
    });
    push!(
        "assert_state_sdk_unavailable",
        "assert_state",
        s.assert_state(Parameters(expect(json!({ "a": 1 }))))
    );
    let s = server(|p| {
        p.inspector = Arc::new(State(Box::new(|| Err(unsupported()))));
    });
    push!(
        "assert_state_unsupported",
        "assert_state",
        s.assert_state(Parameters(expect(json!({ "a": 1 }))))
    );

    // assert_selector
    let asel = |count: Option<usize>, text: Option<&str>| AssertSelectorParams {
        selector: ".item".to_string(),
        expected_text: text.map(str::to_string),
        expected_count: count,
    };
    let s = server(|p| {
        p.evaluator = Arc::new(Eval(Box::new(|| Ok(json!({ "count": 2, "text": "Un" })))))
    });
    push!(
        "assert_selector_ok",
        "assert_selector",
        s.assert_selector(Parameters(asel(Some(2), Some("Un"))))
    );
    push!(
        "assert_selector_fail",
        "assert_selector",
        s.assert_selector(Parameters(asel(Some(3), None)))
    );
    let s = server(|p| {
        p.evaluator = Arc::new(Eval(Box::new(|| Ok(json!({ "count": 0, "text": null })))))
    });
    push!(
        "assert_selector_absent",
        "assert_selector",
        s.assert_selector(Parameters(asel(None, None)))
    );
    let s = server(|p| p.evaluator = Arc::new(Eval(Box::new(|| Err(unsupported())))));
    push!(
        "assert_selector_unsupported",
        "assert_selector",
        s.assert_selector(Parameters(asel(None, None)))
    );

    // Acquittements (outils fire-and-forget) : une seule branche de succès
    // par outil, plus les échecs sémantiques de type_text.
    let s = server(|_| {});
    push!(
        "click_ok",
        "click",
        s.click(Parameters(ClickParams {
            x: 1,
            y: 1,
            button: None
        }))
    );
    push!(
        "right_click_ok",
        "right_click",
        s.right_click(Parameters(pos()))
    );
    push!(
        "double_click_ok",
        "double_click",
        s.double_click(Parameters(pos()))
    );
    push!("hover_ok", "hover", s.hover(Parameters(pos())));
    push!(
        "type_text_ok",
        "type_text",
        s.type_text(Parameters(TypeParams {
            text: "abc".to_string()
        }))
    );
    push!(
        "press_key_ok",
        "press_key",
        s.press_key(Parameters(PressKeyParams {
            key: "Enter".to_string()
        }))
    );
    push!(
        "key_combo_ok",
        "key_combo",
        s.key_combo(Parameters(KeyComboParams {
            modifiers: vec!["Ctrl".to_string()],
            key: "s".to_string(),
        }))
    );
    push!(
        "navigate_ok",
        "navigate",
        s.navigate(Parameters(NavigateParams {
            route: "/agenda".to_string()
        }))
    );
    push!(
        "drag_drop_ok",
        "drag_drop",
        s.drag_drop(Parameters(DragDropParams {
            from_x: 0,
            from_y: 0,
            to_x: 1,
            to_y: 1,
        }))
    );
    push!(
        "scroll_ok",
        "scroll",
        s.scroll(Parameters(ScrollParams {
            x: None,
            y: None,
            dx: Some(0),
            dy: Some(10),
        }))
    );
    push!(
        "handle_dialog_ok",
        "handle_dialog",
        s.handle_dialog(Parameters(HandleDialogParams {
            accept: true,
            prompt_text: None,
        }))
    );
    push!(
        "upload_file_ok",
        "upload_file",
        s.upload_file(Parameters(UploadFileParams {
            selector: "input".to_string(),
            paths: vec!["/tmp/a".to_string()],
        }))
    );
    push!(
        "resize_ok",
        "resize",
        s.resize(Parameters(ResizeParams {
            width: 800,
            height: 600
        }))
    );
    push!(
        "clipboard_set_ok",
        "clipboard_set",
        s.clipboard_set(Parameters(ClipboardSetParams {
            text: "x".to_string()
        }))
    );
    let s = server(|p| p.clipboard = Arc::new(Pasteboard(false)));
    push!(
        "clipboard_set_unsupported",
        "clipboard_set",
        s.clipboard_set(Parameters(ClipboardSetParams {
            text: "x".to_string()
        }))
    );
    let s = server(|p| p.interaction = Arc::new(Interaction(Box::new(|| Err(unsupported())))));
    push!(
        "click_unsupported",
        "click",
        s.click(Parameters(ClickParams {
            x: 1,
            y: 1,
            button: None
        }))
    );
    push!(
        "navigate_unsupported",
        "navigate",
        s.navigate(Parameters(NavigateParams {
            route: "/x".to_string()
        }))
    );
    let s = server(|p| {
        p.interaction = Arc::new(Interaction(Box::new(|| {
            Err(PortError::Interaction(
                "aucun champ éditable (mock)".to_string(),
            ))
        })))
    });
    push!(
        "type_text_interaction_failed",
        "type_text",
        s.type_text(Parameters(TypeParams {
            text: "abc".to_string()
        }))
    );

    // click_selector / click_text
    let s = server(|p| {
        p.evaluator = Arc::new(Eval(Box::new(|| Ok(json!({ "ok": true, "x": 4, "y": 5 })))))
    });
    push!(
        "click_selector_ok",
        "click_selector",
        s.click_selector(Parameters(ClickSelectorParams {
            selector: "#go".to_string()
        }))
    );
    push!(
        "click_text_ok",
        "click_text",
        s.click_text(Parameters(ClickTextParams {
            text: "Go".to_string()
        }))
    );
    let s = server(|p| {
        p.evaluator = Arc::new(Eval(Box::new(|| {
            Ok(json!({ "ok": false, "reason": "not_found" }))
        })))
    });
    push!(
        "click_selector_not_found",
        "click_selector",
        s.click_selector(Parameters(ClickSelectorParams {
            selector: "#nope".to_string()
        }))
    );

    // focus_window / window_geometry
    let s = server(|p| p.window_manager = Arc::new(Manager(Some("Fenêtre test".to_string()))));
    push!(
        "focus_window_ok",
        "focus_window",
        s.focus_window(Parameters(FocusWindowParams {
            title_pattern: None
        }))
    );
    let s = server(|p| p.window_manager = Arc::new(Manager(None)));
    push!(
        "focus_window_unsupported",
        "focus_window",
        s.focus_window(Parameters(FocusWindowParams {
            title_pattern: None
        }))
    );
    let s = server(|p| {
        p.window_geometry = Arc::new(Geometry(Some(WindowRect::new(10, -20, 800, 600))))
    });
    push!(
        "window_geometry_ok",
        "window_geometry",
        s.window_geometry(Parameters(WindowGeometryParams { title: None }))
    );
    let s = server(|p| p.window_geometry = Arc::new(Geometry(None)));
    push!(
        "window_geometry_unsupported",
        "window_geometry",
        s.window_geometry(Parameters(WindowGeometryParams { title: None }))
    );
    out
}

// ----------------------------------------------------------------------
// Golden files.
// ----------------------------------------------------------------------

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Forme figée : drapeau `isError` puis chaque bloc texte, tels que le client
/// les reçoit.
pub(crate) fn render_text(result: &CallToolResult) -> String {
    let mut out = format!("isError: {}\n", result.is_error.unwrap_or(false));
    for (index, block) in result.content.iter().enumerate() {
        let text = block
            .as_text()
            .map(|t| t.text.as_str())
            .unwrap_or("<bloc non texte>");
        out.push_str(&format!("--- bloc {index}\n{}\n", mask_timing(text)));
    }
    out
}

/// Masque la durée mesurée (`waited_ms`, variable d'une machine à l'autre) pour
/// que le golden reste déterministe ; tout le reste du texte est comparé tel quel.
fn mask_timing(text: &str) -> String {
    const KEY: &str = "\"waited_ms\":";
    let Some(start) = text.find(KEY).map(|i| i + KEY.len()) else {
        return text.to_string();
    };
    let digits = text[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .count();
    format!("{}0{}", &text[..start], &text[start + digits..])
}

#[tokio::test]
async fn golden_text_of_small_output_tools_is_unchanged() {
    let bless = std::env::var_os("DEVEYE_BLESS_GOLDEN").is_some();
    let dir = golden_dir();
    if bless {
        std::fs::create_dir_all(&dir).expect("création du répertoire golden");
    }
    let mut expected_files = std::collections::BTreeSet::new();
    let registered: std::collections::BTreeSet<String> = DeveyeMcpServer::tool_router()
        .list_all()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    for scenario in scenarios().await {
        assert!(
            registered.contains(scenario.tool),
            "scénario `{}` : l'outil `{}` n'existe pas",
            scenario.name,
            scenario.tool
        );
        let path = dir.join(format!("{}.txt", scenario.name));
        expected_files.insert(format!("{}.txt", scenario.name));
        let actual = render_text(&scenario.result);
        if bless {
            std::fs::write(&path, &actual).expect("écriture du golden");
            continue;
        }
        let golden = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("golden {path:?} illisible : {e}"));
        assert_eq!(
            golden, actual,
            "le texte de `{}` a changé (golden {path:?})",
            scenario.name
        );
    }
    // Aucun golden orphelin : un scénario retiré doit retirer son fichier.
    let on_disk: std::collections::BTreeSet<String> = std::fs::read_dir(&dir)
        .expect("lecture du répertoire golden")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".txt"))
        .collect();
    assert_eq!(on_disk, expected_files, "golden orphelins ou manquants");
}
