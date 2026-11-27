//! Masquage des secrets dans le state et les champs de logs servis par le SDK.
//!
//! Le masquage repose sur le **nom des clés** JSON, à toute profondeur : la
//! valeur d'une clé jugée sensible est remplacée en bloc par [`REDACTED`], sans
//! descendre dans son contenu. Il s'applique au moment où le SDK sert les
//! données, pas à l'écriture : les clés déclarées via `Config::secret_keys`
//! valent ainsi quel que soit l'ordre des appels `init` / `set_state`, et aucun
//! parcours n'est payé sur le thread UI ni dans le chemin chaud de `tracing`.
//! Le texte libre des messages de log n'est pas analysé, pas plus que les
//! champs formatés par `Debug` (`?valeur`) : ils arrivent en chaîne et seul le
//! nom de leur champ est examiné.
//!
//! Périmètre exact et limites : `docs/wire-protocol.md` §2.4 et ADR-0016.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

/// Valeur substituée à toute donnée masquée.
pub const REDACTED: &str = "<secret>";

/// Termes sensibles où qu'ils apparaissent dans la clé normalisée
/// (`user_password`, `stripeApiKey`, `client_secret`…).
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

/// Termes sensibles uniquement en fin de clé : `accessToken` ou `set-cookie`
/// sont masqués, mais pas `token_count`, `max_tokens` ni `cookie_banner`.
const SUFFIXES: &[&str] = &["token", "cookie", "sessionid"];

/// Clés trop courtes ou ambiguës pour une recherche de sous-chaîne.
/// Les pluriels `tokens` et `cookies` sont exacts, pas en suffixe : `max_tokens`
/// (plafond de génération) ne doit pas être masqué.
const EXACT: &[&str] = &["pwd", "auth", "jwt", "bearer", "otp", "tokens", "cookies"];

/// Forme canonique d'une clé : minuscules ASCII, sans séparateurs, pour que
/// `api_key`, `apiKey`, `API-KEY` et `api.key` se comparent tous à `apikey`.
pub(crate) fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| !matches!(c, '_' | '-' | '.' | ' '))
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Vrai si `key` désigne une donnée sensible, d'après les listes par défaut ou
/// les clés propres à l'app (`extra`, déjà normalisées, comparées à
/// l'identique).
pub(crate) fn is_secret_key(key: &str, extra: &HashSet<String>) -> bool {
    let key = normalize_key(key);
    CONTAINS.iter().any(|term| key.contains(term))
        || SUFFIXES.iter().any(|term| key.ends_with(term))
        || EXACT.contains(&key.as_str())
        || extra.contains(&key)
}

/// Masque récursivement les valeurs des clés sensibles dans `value`.
pub(crate) fn redact(value: &mut Value, extra: &HashSet<String>) {
    match value {
        Value::Object(map) => redact_pairs(map.iter_mut(), extra),
        Value::Array(items) => items.iter_mut().for_each(|item| redact(item, extra)),
        _ => {}
    }
}

/// Variante pour les entrées de premier niveau du state, dont le nom est
/// lui-même une clé (`set_state("api_key", …)`).
pub(crate) fn redact_entries(entries: &mut BTreeMap<String, Value>, extra: &HashSet<String>) {
    redact_pairs(entries.iter_mut(), extra);
}

fn redact_pairs<'a>(
    pairs: impl Iterator<Item = (&'a String, &'a mut Value)>,
    extra: &HashSet<String>,
) {
    for (key, value) in pairs {
        if is_secret_key(key, extra) {
            *value = Value::String(REDACTED.to_string());
        } else {
            redact(value, extra);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn no_extra() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn default_keys_are_secret_whatever_the_case_and_separators() {
        for key in [
            "password",
            "user_password",
            "apiKey",
            "API-KEY",
            "client_secret",
            "Authorization",
            "private_key",
            "accessToken",
            "refresh_token",
            "github.token",
            "set-cookie",
            "sessionId",
            "jwt",
            "tokens",
            "cookies",
            "access_key",
            "signing_key",
            "database_url",
            "mdp",
            "mot_de_passe",
            "jeton",
        ] {
            assert!(
                is_secret_key(key, &no_extra()),
                "{key} devrait être masquée"
            );
        }
    }

    #[test]
    fn ordinary_keys_are_not_secret() {
        for key in [
            "user",
            "page",
            "session",
            "author",
            "token_count",
            "max_tokens",
            "tokenizer",
            "cookie_banner",
        ] {
            assert!(
                !is_secret_key(key, &no_extra()),
                "{key} ne devrait pas être masquée"
            );
        }
    }

    #[test]
    fn app_keys_match_exactly_after_normalization() {
        let extra: HashSet<String> = [normalize_key("iban")].into_iter().collect();
        assert!(is_secret_key("iban", &extra));
        assert!(is_secret_key("IBAN", &extra));
        assert!(!is_secret_key("user_iban", &extra));
    }

    #[test]
    fn redact_masks_nested_keys_and_replaces_whole_subtree() {
        let mut value = json!({
            "user": "alice",
            "credentials": { "login": "alice", "pin": 1234 },
            "devices": [{ "name": "phone", "push_token": "xyz" }]
        });
        redact(&mut value, &no_extra());
        assert_eq!(
            value,
            json!({
                "user": "alice",
                "credentials": REDACTED,
                "devices": [{ "name": "phone", "push_token": REDACTED }]
            })
        );
    }

    #[test]
    fn redact_leaves_scalars_and_null_untouched() {
        let mut value = json!(null);
        redact(&mut value, &no_extra());
        assert_eq!(value, json!(null));
    }

    /// Vecteurs partagés avec la suite de conformité et les SDK tiers
    /// (`docs/schemas/shared-vectors-1.json`, ADR-0064). Absents d'une archive
    /// crates.io : le test est alors ignoré.
    fn shared_vectors() -> Option<serde_json::Value> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../docs/schemas/shared-vectors-1.json");
        let text = std::fs::read_to_string(path).ok()?;
        Some(serde_json::from_str(&text).expect("vecteurs partagés : JSON invalide"))
    }

    fn strings(value: &serde_json::Value) -> Vec<String> {
        value
            .as_array()
            .expect("liste attendue")
            .iter()
            .map(|v| v.as_str().expect("chaîne attendue").to_string())
            .collect()
    }

    #[test]
    fn shared_vectors_agree_with_the_sdk_rules() {
        let Some(vectors) = shared_vectors() else {
            return;
        };
        let r = &vectors["redaction"];
        assert_eq!(r["placeholder"], REDACTED);
        for group in ["required_secret_keys", "extended_secret_keys"] {
            for key in strings(&r[group]) {
                assert!(is_secret_key(&key, &no_extra()), "{key} ({group})");
            }
        }
        for key in strings(&r["ordinary_keys"]) {
            assert!(!is_secret_key(&key, &no_extra()), "{key}");
        }
        let declared = &r["declared_keys"];
        let extra: HashSet<String> = strings(&declared["declared"])
            .iter()
            .map(|k| normalize_key(k))
            .collect();
        for key in strings(&declared["secret"]) {
            assert!(is_secret_key(&key, &extra), "{key}");
        }
        for key in strings(&declared["ordinary"]) {
            assert!(!is_secret_key(&key, &extra), "{key}");
        }
        for case in r["redact"].as_array().unwrap() {
            let mut input = case["input"].clone();
            redact(&mut input, &no_extra());
            assert_eq!(input, case["expected"], "{}", case["name"]);
        }
    }
}
