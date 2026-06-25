use serde::{Deserialize, Serialize};

/// Prédicat évalué périodiquement par le port `WaitCondition`.
///
/// Chaque variante est exprimable par l'agent Claude en JSON (`{ "kind": ... }`) ;
/// le port se charge de la boucle de polling et du timeout. Domaine pur — aucune
/// I/O ici.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WaitPredicate {
    /// L'élément matchant le sélecteur CSS est présent dans le DOM.
    DomSelector { selector: String },
    /// La clé `key` du state exposé par le SDK a pour valeur `expected`
    /// (comparaison JSON).
    StateKey {
        key: String,
        expected: serde_json::Value,
    },
    /// Deux captures consécutives (séparées de `threshold_ms`) sont identiques
    /// octet pour octet — stabilité visuelle (fin d'animation, page au repos).
    ScreenshotStable { threshold_ms: u64 },
}

/// Résultat d'une attente conditionnelle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaitOutcome {
    /// Vrai si le prédicat a été satisfait avant le timeout.
    pub satisfied: bool,
    /// Durée effective d'attente, en millisecondes.
    pub elapsed_ms: u64,
    /// Description pour l'agent (ce qui a été observé si non satisfait).
    pub detail: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dom_selector_serializes_with_kind_tag() {
        let p = WaitPredicate::DomSelector {
            selector: ".error-banner".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({ "kind": "dom-selector", "selector": ".error-banner" })
        );
    }

    #[test]
    fn state_key_roundtrips() {
        let p = WaitPredicate::StateKey {
            key: "count".to_string(),
            expected: json!(3),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v, json!({ "kind": "state-key", "key": "count", "expected": 3 }));
        let back: WaitPredicate = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn screenshot_stable_serializes() {
        let p = WaitPredicate::ScreenshotStable { threshold_ms: 300 };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({ "kind": "screenshot-stable", "threshold_ms": 300 })
        );
    }
}
