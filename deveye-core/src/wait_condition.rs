use serde::{Deserialize, Serialize};

use crate::LogLevel;

/// Prédicat évalué périodiquement par le port `WaitCondition`.
///
/// Chaque variante est exprimable par l'agent Claude en JSON (`{ "kind": ... }`) ;
/// le port se charge de la boucle de polling et du timeout. Domaine pur — aucune
/// I/O ici.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum WaitPredicate {
    /// L'element matchant le selecteur CSS est present dans le DOM.
    /// Si `visible` est vrai, l'element doit en plus avoir un
    /// `BoundingClientRect` non vide (present ET visible a l'ecran).
    /// Si `negate` est vrai, la condition est inversee : satisfaite quand
    /// l'element est ABSENT (ou non visible si `visible`) — base de
    /// `wait_for_gone` (attendre la disparition d'un spinner/toast/modale).
    /// Retro-compatible : champs absents dans le JSON => `false`.
    DomSelector {
        selector: String,
        /// Si true, attend aussi que l'element soit visible (rect non vide).
        #[serde(default)]
        visible: bool,
        /// Si true, inverse la condition : satisfaite quand l'element est
        /// absent (ou disparait).
        #[serde(default)]
        negate: bool,
    },
    /// La clé `key` du state exposé par le SDK a pour valeur `expected`
    /// (comparaison JSON).
    StateKey {
        key: String,
        expected: serde_json::Value,
    },
    /// Deux captures consécutives (séparées de `threshold_ms`) sont identiques
    /// octet pour octet — stabilité visuelle (fin d'animation, page au repos).
    ScreenshotStable { threshold_ms: u64 },
    /// Comme `ScreenshotStable`, mais sur une **région** (x, y, width, height)
    /// uniquement : ignore un curseur clignotant ou une horloge ailleurs dans
    /// le viewport. Bornée aux dimensions de la capture.
    ScreenshotRegionStable {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        threshold_ms: u64,
    },
    /// Un log applicatif dont le message contient `contains` (et de niveau au
    /// moins `level_min` si fourni) a été émis. Observe les logs `tracing`
    /// bufferisés par le SDK — hors de portée de `eval_js`. Note : pendant
    /// l'attente, le buffer de logs est drainé (les logs non concernés sont
    /// consommés, donc invisibles à un `logs_tail` ultérieur).
    LogMessage {
        contains: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level_min: Option<LogLevel>,
    },
    /// Satisfait des que la cible REPOND (peu importe l'etat applicatif) :
    /// sert a attendre qu'une app cible (re)devienne vivante apres un
    /// crash/redemarrage. Aucun autre predicat ne le permet — `eval_js` est
    /// structurellement incapable de repondre sur une cible morte, il faut
    /// une sonde plus basse (le renderer, cf. `CompositeWaitCondition`).
    TargetAlive {},
    /// Alias produit : `wait_for_text`. Satisfaite quand `text` apparait
    /// dans l'`innerText` (ou `textContent`) de l'element `selector`, ou du
    /// document entier si `selector` est absent. Si `visible` est vrai,
    /// l'element doit en plus avoir un rect non vide (comme `DomSelector`).
    /// Sur une cible sans DOM (kwin-eis), ce predicat ne se satisfait
    /// jamais — utiliser `assert_visual`/OCR a la place.
    TextContent {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selector: Option<String>,
        #[serde(default)]
        visible: bool,
    },
}

/// Résultat d'une attente conditionnelle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WaitOutcome {
    /// Vrai si le prédicat a été satisfait avant le timeout.
    pub satisfied: bool,
    /// Durée effective d'attente, en millisecondes.
    pub elapsed_ms: u64,
    /// Description pour l'agent (ce qui a été observé si non satisfait).
    pub detail: String,
}

impl WaitOutcome {
    pub fn new(satisfied: bool, elapsed_ms: u64, detail: impl Into<String>) -> Self {
        Self {
            satisfied,
            elapsed_ms,
            detail: detail.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dom_selector_serializes_with_kind_tag() {
        let p = WaitPredicate::DomSelector {
            selector: ".error-banner".to_string(),
            visible: false,
            negate: false,
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({ "kind": "dom-selector", "selector": ".error-banner", "visible": false, "negate": false })
        );
    }

    #[test]
    fn dom_selector_visible_true_roundtrips() {
        let p = WaitPredicate::DomSelector {
            selector: "#save-btn".to_string(),
            visible: true,
            negate: false,
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(
            v,
            json!({ "kind": "dom-selector", "selector": "#save-btn", "visible": true, "negate": false })
        );
        let back: WaitPredicate = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn dom_selector_visible_defaults_to_false_when_absent() {
        // Retro-compatibilite : JSON sans le champ `visible` => false.
        let v = json!({ "kind": "dom-selector", "selector": ".foo" });
        let p: WaitPredicate = serde_json::from_value(v).unwrap();
        assert_eq!(
            p,
            WaitPredicate::DomSelector {
                selector: ".foo".to_string(),
                visible: false,
                negate: false,
            }
        );
    }

    #[test]
    fn state_key_roundtrips() {
        let p = WaitPredicate::StateKey {
            key: "count".to_string(),
            expected: json!(3),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(
            v,
            json!({ "kind": "state-key", "key": "count", "expected": 3 })
        );
        let back: WaitPredicate = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn dom_selector_negate_defaults_false_and_roundtrips_true() {
        // Absent => false (retro-compat).
        let v = json!({ "kind": "dom-selector", "selector": ".spinner" });
        let p: WaitPredicate = serde_json::from_value(v).unwrap();
        assert_eq!(
            p,
            WaitPredicate::DomSelector {
                selector: ".spinner".to_string(),
                visible: false,
                negate: false,
            }
        );
        // negate=true roundtrip.
        let p = WaitPredicate::DomSelector {
            selector: ".spinner".to_string(),
            visible: false,
            negate: true,
        };
        let back: WaitPredicate =
            serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
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

    #[test]
    fn screenshot_region_stable_serializes() {
        let p = WaitPredicate::ScreenshotRegionStable {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
            threshold_ms: 300,
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({ "kind": "screenshot-region-stable", "x": 10, "y": 20, "width": 100, "height": 50, "threshold_ms": 300 })
        );
    }

    #[test]
    fn log_message_serializes_and_omits_absent_level() {
        let p = WaitPredicate::LogMessage {
            contains: "saved".to_string(),
            level_min: None,
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({ "kind": "log-message", "contains": "saved" })
        );
        let p2 = WaitPredicate::LogMessage {
            contains: "boom".to_string(),
            level_min: Some(LogLevel::Error),
        };
        let v = serde_json::to_value(&p2).unwrap();
        assert_eq!(
            v,
            json!({ "kind": "log-message", "contains": "boom", "level_min": "error" })
        );
        let back: WaitPredicate = serde_json::from_value(v).unwrap();
        assert_eq!(back, p2);
    }

    #[test]
    fn target_alive_serializes_with_kind_tag() {
        let p = WaitPredicate::TargetAlive {};
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({ "kind": "target-alive" })
        );
        let back: WaitPredicate =
            serde_json::from_value(json!({ "kind": "target-alive" })).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn text_content_omits_absent_selector_and_defaults_visible_false() {
        let p = WaitPredicate::TextContent {
            text: "Enregistre".to_string(),
            selector: None,
            visible: false,
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(
            v,
            json!({ "kind": "text-content", "text": "Enregistre", "visible": false })
        );
        // Retro-compatibilite : JSON sans selector/visible => None/false.
        let back: WaitPredicate =
            serde_json::from_value(json!({ "kind": "text-content", "text": "Enregistre" }))
                .unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn text_content_with_selector_and_visible_roundtrips() {
        let p = WaitPredicate::TextContent {
            text: "OK".to_string(),
            selector: Some("#status".to_string()),
            visible: true,
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(
            v,
            json!({ "kind": "text-content", "text": "OK", "selector": "#status", "visible": true })
        );
        let back: WaitPredicate = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);
    }
}
