//! Les schémas JSON de `docs/schemas/1.0/` sont du JSON valide, bien identifiés,
//! et tous référencés par la spécification (et inversement). Aucun validateur
//! JSON Schema n'est embarqué : la conformité des corps est vérifiée par la suite.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn schema_files() -> Vec<PathBuf> {
    let dir = repo_root().join("docs/schemas/1.0");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("dossier des schémas")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}

#[test]
fn schemas_are_valid_and_identified() {
    let files = schema_files();
    assert!(files.len() >= 8, "schémas attendus : {files:?}");
    for path in files {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let text = std::fs::read_to_string(&path).expect("lecture");
        let schema: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} : JSON invalide : {e}"));
        assert_eq!(
            schema["$schema"], "https://json-schema.org/draft/2020-12/schema",
            "{name}"
        );
        let id = schema["$id"].as_str().unwrap_or_default();
        assert!(
            id.ends_with(&format!("/schemas/1.0/{name}")),
            "{name} : $id {id}"
        );
        assert!(schema["title"].is_string(), "{name} : titre absent");
    }
}

#[test]
fn spec_references_every_schema() {
    let spec = std::fs::read_to_string(repo_root().join("docs/wire-protocol.md")).expect("spec");
    for path in schema_files() {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        assert!(
            spec.contains(&format!("schemas/1.0/{name}")),
            "{name} n'est pas référencé par docs/wire-protocol.md"
        );
    }
}

#[test]
fn log_levels_match_the_domain() {
    let text = std::fs::read_to_string(repo_root().join("docs/schemas/1.0/logs-poll.json"))
        .expect("lecture");
    let schema: serde_json::Value = serde_json::from_str(&text).expect("json");
    let levels = &schema["$defs"]["logEvent"]["properties"]["level"]["enum"];
    assert_eq!(
        levels,
        &serde_json::json!(["trace", "debug", "info", "warn", "error"])
    );
}
