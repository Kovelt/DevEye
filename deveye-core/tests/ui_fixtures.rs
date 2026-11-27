//! Les fixtures de `tests/fixtures/ui-1/` se comportent comme le dit leur README.

use std::path::PathBuf;

use deveye_core::{UiRole, UiTree, UI_SCHEMA};
use serde_json::Value;

fn fixtures(prefix: &str) -> Vec<(String, Value)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ui-1");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("dossier des fixtures") {
        let path = entry.expect("entrée").path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.starts_with(prefix) && name.ends_with(".json") {
            let text = std::fs::read_to_string(&path).expect("lecture");
            found.push((name, serde_json::from_str(&text).expect("JSON valide")));
        }
    }
    assert!(!found.is_empty(), "aucune fixture {prefix}*");
    found
}

#[test]
fn valid_fixtures_are_read_and_sane() {
    for (name, value) in fixtures("valid-") {
        let tree = UiTree::from_value(&value).unwrap_or_else(|e| panic!("{name} : {e}"));
        assert_eq!(tree.schema, UI_SCHEMA, "{name}");
        let blocking: Vec<_> = tree
            .validate()
            .into_iter()
            .filter(|i| i.is_error())
            .collect();
        assert!(blocking.is_empty(), "{name} : {blocking:?}");
    }
}

#[test]
fn full_fixture_keeps_every_field_and_the_unknown_role() {
    let (_, value) = fixtures("valid-full").remove(0);
    let tree = UiTree::from_value(&value).unwrap();
    assert_eq!(tree.generation, 42);
    assert_eq!(tree.window.scale, Some(2.0));
    assert_eq!(tree.focus.as_deref(), Some("name"));
    assert_eq!(tree.widgets.len(), 6);
    assert_eq!(tree.widgets[5].role, UiRole::new("gauge"));
    assert!(tree.widget("password").unwrap().is_secret());
    assert_eq!(tree.widget("remember").unwrap().state.checked, Some(true));
}

#[test]
fn legacy_fixtures_are_converted() {
    for (name, value) in fixtures("legacy-") {
        let tree = UiTree::from_value(&value).unwrap_or_else(|e| panic!("{name} : {e}"));
        assert_eq!(tree.schema, UI_SCHEMA, "{name}");
        assert!(!tree.widgets.is_empty(), "{name}");
        assert!(tree.validate().iter().all(|i| !i.is_error()), "{name}");
    }
}

#[test]
fn invalid_fixtures_are_flagged_by_validation() {
    for (name, value) in fixtures("invalid-") {
        let tree = UiTree::from_value(&value).unwrap_or_else(|e| panic!("{name} : {e}"));
        assert!(tree.validate().iter().any(|i| i.is_error()), "{name}");
    }
}

#[test]
fn unsupported_fixtures_are_refused() {
    for (name, value) in fixtures("unsupported-") {
        assert!(UiTree::from_value(&value).is_err(), "{name}");
    }
}
