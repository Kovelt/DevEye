//! L'exemple legacy de `docs/usage.md` est lu et converti sans erreur.

use std::path::PathBuf;

use deveye_core::UiTree;

const MARKER: &str = "<!-- ui-widgets-legacy-example -->";

/// Premier bloc ```json qui suit `marker` dans le document.
fn block_after<'a>(doc: &'a str, marker: &str) -> &'a str {
    let after = doc.split_once(marker).expect("marqueur absent").1;
    let body = after.split_once("```json\n").expect("bloc json absent").1;
    body.split_once("```").expect("bloc non fermé").0
}

#[test]
fn usage_legacy_example_is_read_and_converted() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/usage.md");
    let Ok(doc) = std::fs::read_to_string(path) else {
        return; // archive crates.io : pas de docs
    };
    let value = serde_json::from_str(block_after(&doc, MARKER)).expect("exemple : JSON invalide");
    let tree = UiTree::from_value(&value).expect("exemple legacy illisible");
    assert_eq!(tree.widgets.len(), 1);
    assert_eq!(tree.widgets[0].rect, [40, 120, 96, 28]);
    assert!(tree.validate().is_empty());
}

#[test]
fn usage_v1_example_is_read_without_issue() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/usage.md");
    let Ok(doc) = std::fs::read_to_string(path) else {
        return;
    };
    let section = doc.split_once("Route `ui.widgets`").expect("section").1;
    let body = section.split_once("```json\n").expect("bloc").1;
    let json = body.split_once("```").expect("fin").0;
    let value = serde_json::from_str(json).expect("JSON invalide");
    let tree = UiTree::from_value(&value).expect("exemple v1 illisible");
    assert!(tree.validate().is_empty());
    assert!(tree.widget("password").unwrap().is_secret());
}
