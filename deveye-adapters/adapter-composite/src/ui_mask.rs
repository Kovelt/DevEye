//! Masquage des secrets dans la réponse brute de la route `ui.widgets`
//! (ADR-0064).
//!
//! Le SDK ne publie jamais la valeur d'un widget secret, mais une app tierce
//! (shim, addon, SDK ancien) peut le faire : le serveur masque en seconde ligne,
//! **sur la valeur brute** que l'outil `dom` renvoie, sans la convertir (un
//! arbre au format legacy reste au format legacy).
//!
//! Deux règles, dans cet ordre :
//! - la valeur (`value`, et `text` du format legacy) d'un widget secret
//!   (`state.secret`, `secret`, rôle `password`) est remplacée par [`REDACTED`] ;
//! - une valeur structurée (objet, liste) passe par le masquage par noms de clés
//!   du wire protocol (§2.4). Les clés propres à l'app (`Config::secret_keys`)
//!   sont inconnues du serveur : le SDK les applique avant publication.
//!
//! Une réponse qui n'a pas la forme d'une liste de widgets n'est pas modifiée
//! (ADR-0064, amendé). Le masquage reste un filet, pas une garantie.

use serde_json::{Map, Value};

/// Valeur substituée aux données masquées.
pub const REDACTED: &str = "<secret>";

/// Termes sensibles où qu'ils apparaissent dans la clé normalisée.
const CONTAINS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "apikey",
    "privatekey",
    "credential",
    "authorization",
    "accesskey",
    "signingkey",
    "encryptionkey",
    "databaseurl",
    "connectionstring",
    "motdepasse",
    "mdp",
    "jeton",
];

/// Termes sensibles uniquement en fin de clé (`accessToken`, pas `token_count`).
const SUFFIXES: &[&str] = &["token", "cookie", "sessionid"];

/// Clés courtes ou ambiguës, comparées à l'identique.
const EXACT: &[&str] = &["pwd", "auth", "jwt", "bearer", "otp", "tokens", "cookies"];

/// Forme canonique d'une clé : minuscules ASCII, sans séparateurs.
fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| !matches!(c, '_' | '-' | '.' | ' '))
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Vrai si `key` désigne une donnée sensible d'après les listes par défaut.
pub fn is_secret_key(key: &str) -> bool {
    let key = normalize_key(key);
    CONTAINS.iter().any(|term| key.contains(term))
        || SUFFIXES.iter().any(|term| key.ends_with(term))
        || EXACT.contains(&key.as_str())
}

/// Masque récursivement les valeurs des clés sensibles ; la valeur d'une clé
/// sensible est remplacée en bloc, sans descendre dans son contenu.
pub fn redact_by_key(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                if is_secret_key(key) {
                    *inner = Value::String(REDACTED.to_string());
                } else {
                    redact_by_key(inner);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_by_key),
        _ => {}
    }
}

/// Widget secret : drapeau `state.secret` ou `secret`, ou rôle `password`
/// (insensible à la casse et aux espaces, comme `UiWidget::is_secret`).
fn is_secret_widget(widget: &Map<String, Value>) -> bool {
    let flag = |value: Option<&Value>| value.and_then(Value::as_bool).unwrap_or(false);
    let flagged = flag(widget.get("state").and_then(|state| state.get("secret")))
        || flag(widget.get("secret"));
    let password = ["role", "kind"]
        .iter()
        .filter_map(|key| widget.get(*key).and_then(Value::as_str))
        .any(|role| role.trim().eq_ignore_ascii_case("password"));
    flagged || password
}

/// Liste de widgets d'une réponse reconnue : la liste racine ou le champ
/// `widgets`. Toute autre forme n'est pas un arbre de widgets.
fn widget_list(tree: &mut Value) -> Option<&mut Vec<Value>> {
    match tree {
        Value::Array(items) => Some(items),
        Value::Object(map) => map.get_mut("widgets").and_then(Value::as_array_mut),
        _ => None,
    }
}

/// Champs de structure d'un widget : jamais pris pour des secrets (`secret` est
/// le drapeau, pas une donnée).
const STRUCTURAL: &[&str] = &["id", "role", "kind", "rect", "parent", "state", "secret"];

/// Masque un widget en place, enfants imbriqués compris.
fn mask_widget(widget: &mut Value) {
    let Some(map) = widget.as_object_mut() else {
        return;
    };
    let secret = is_secret_widget(map);
    for (key, inner) in map.iter_mut() {
        if STRUCTURAL.contains(&key.as_str()) {
            continue;
        }
        if key == "children" {
            if let Some(children) = inner.as_array_mut() {
                children.iter_mut().for_each(mask_widget);
            }
        } else if secret && (key == "value" || key == "text") {
            // `text` : contenu d'un champ dans le format legacy (egui, imgui).
            if !inner.is_null() {
                *inner = Value::String(REDACTED.to_string());
            }
        } else if is_secret_key(key) {
            *inner = Value::String(REDACTED.to_string());
        } else {
            redact_by_key(inner);
        }
    }
}

/// Masque `tree` en place (réponse brute de la route `ui.widgets`).
///
/// Une réponse qui n'a pas la forme d'un arbre de widgets (liste racine ou champ
/// `widgets`) est laissée intacte : la route est celle d'un handler quelconque et
/// `dom` ne doit pas en altérer la sortie.
pub fn mask_route_value(tree: &mut Value) {
    if let Some(widgets) = widget_list(tree) {
        widgets.iter_mut().for_each(mask_widget);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vectors() -> Option<Value> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/schemas/shared-vectors-1.json");
        let text = std::fs::read_to_string(path).ok()?;
        Some(serde_json::from_str(&text).expect("vecteurs partagés : JSON invalide"))
    }

    fn strings(value: &Value) -> Vec<String> {
        value
            .as_array()
            .expect("liste attendue")
            .iter()
            .map(|v| v.as_str().expect("chaîne attendue").to_string())
            .collect()
    }

    /// Même règle que le SDK et la suite de conformité (ADR-0064). Absents d'une
    /// archive : le test est alors ignoré.
    #[test]
    fn key_rule_matches_the_shared_vectors() {
        let Some(vectors) = vectors() else {
            return;
        };
        let r = &vectors["redaction"];
        assert_eq!(r["placeholder"], REDACTED);
        for group in ["required_secret_keys", "extended_secret_keys"] {
            for key in strings(&r[group]) {
                assert!(is_secret_key(&key), "{key} ({group})");
            }
        }
        for key in strings(&r["ordinary_keys"]) {
            assert!(!is_secret_key(&key), "{key}");
        }
        for case in r["redact"].as_array().unwrap() {
            let mut input = case["input"].clone();
            redact_by_key(&mut input);
            assert_eq!(input, case["expected"], "{}", case["name"]);
        }
    }

    #[test]
    fn secret_widget_value_is_masked_whatever_the_flavour() {
        let mut tree = json!({
            "schema": "deveye.ui/1",
            "window": { "width": 100, "height": 100 },
            "widgets": [
                { "id": "a", "role": "textbox", "value": "alice", "rect": [0, 0, 1, 1] },
                { "id": "b", "role": "textbox", "value": "s3", "state": { "secret": true } },
                { "id": "c", "role": "password", "value": "hunter2" },
                { "id": "d", "kind": "Password", "value": "x" },
                { "id": "e", "role": "textbox", "secret": true, "value": 7 },
                { "id": "f", "role": "password" },
                { "id": "g", "role": "label", "value": { "user": "u", "api_key": "k" } },
            ]
        });
        mask_route_value(&mut tree);
        let widgets = tree["widgets"].as_array().unwrap();
        assert_eq!(widgets[0]["value"], "alice");
        for i in [1, 2, 3, 4] {
            assert_eq!(widgets[i]["value"], REDACTED, "{i}");
        }
        assert!(widgets[5].get("value").is_none());
        assert_eq!(
            widgets[6]["value"],
            json!({ "user": "u", "api_key": REDACTED })
        );
        // Les champs de structure (`state.secret`) ne sont pas pris pour des secrets.
        assert_eq!(widgets[1]["state"]["secret"], true);
    }

    #[test]
    fn legacy_list_and_unknown_shapes_are_masked_too() {
        let mut legacy = json!([
            { "kind": "password", "value": "hunter2", "rect": [0, 0, 1, 1] },
            { "kind": "button", "text": "OK" }
        ]);
        mask_route_value(&mut legacy);
        assert_eq!(legacy[0]["value"], REDACTED);
        assert_eq!(legacy[1]["text"], "OK");

        // Forme inconnue : sortie de `dom` inchangée par rapport à la 1.0.
        let mut unknown = json!({ "nodes": [{ "name": "x", "password": "p" }], "page": 3 });
        let before = unknown.clone();
        mask_route_value(&mut unknown);
        assert_eq!(unknown, before);
    }

    #[test]
    fn role_case_text_and_children_are_covered() {
        let mut tree = json!({
            "widgets": [
                { "kind": " PASSWORD ", "text": "hunter2" },
                { "kind": "textbox", "state": { "secret": false }, "secret": true, "value": "x" },
                { "kind": "textbox", "password": "p", "value": "ok" },
                { "kind": "panel", "children": [
                    { "kind": "password", "value": "deep" },
                    { "kind": "label", "text": "visible" }
                ] }
            ]
        });
        mask_route_value(&mut tree);
        let w = &tree["widgets"];
        assert_eq!(w[0]["text"], REDACTED);
        assert_eq!(w[1]["value"], REDACTED);
        assert_eq!(w[1]["secret"], true);
        assert_eq!(w[2]["password"], REDACTED);
        assert_eq!(w[2]["value"], "ok");
        assert_eq!(w[3]["children"][0]["value"], REDACTED);
        assert_eq!(w[3]["children"][1]["text"], "visible");
    }
}
