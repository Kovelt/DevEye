//! `docs/schemas/ui-widgets-1.json` est égal au schéma généré par les types. Pour
//! régénérer le fichier après un changement voulu des types :
//! `DEVEYE_UPDATE_SCHEMA=1 cargo test -p deveye-core --features schema --test ui_schema`
//! (recette `just schema`).
#![cfg(feature = "schema")]

use std::path::PathBuf;

use deveye_core::ui::json_schema;
use serde_json::Value;

fn schema_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/schemas/ui-widgets-1.json")
}

#[test]
fn committed_schema_matches_the_generated_one() {
    let generated = json_schema();
    let path = schema_path();
    if std::env::var_os("DEVEYE_UPDATE_SCHEMA").is_some() {
        let mut text = serde_json::to_string_pretty(&generated).expect("sérialisation");
        text.push('\n');
        std::fs::write(&path, text).expect("écriture du schéma");
        return;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        // Crate extrait de son archive : pas de dossier docs à comparer.
        eprintln!("{} absent : comparaison ignorée", path.display());
        return;
    };
    let committed: Value = serde_json::from_str(&text).expect("schéma versionné : JSON invalide");
    assert_eq!(
        committed, generated,
        "docs/schemas/ui-widgets-1.json diverge des types : régénérer (just schema)"
    );
}

#[test]
fn generated_schema_is_a_2020_12_object_schema() {
    let schema = json_schema();
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["type"], "object");
    assert!(schema["$id"]
        .as_str()
        .unwrap()
        .ends_with("/ui-widgets-1.json"));
}
