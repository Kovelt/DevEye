//! Types de réponse des outils MCP à petite sortie (palier A, ADR-0067).
//!
//! Chaque type sert deux fois, ce qui rend la dérive impossible par
//! construction : il produit à la fois le **texte** envoyé aux clients
//! (identique, octet pour octet, à celui d'avant les sorties structurées —
//! golden files de `tests/golden`) et le `structuredContent`, et il fournit
//! l'`outputSchema` déclaré dans `tools/list`.
//!
//! Les champs des structures qui remplacent d'anciens objets `json!` sont
//! déclarés **par ordre alphabétique** : c'est l'ordre dans lequel
//! `serde_json::Map` (sans `preserve_order`) sérialisait ces objets, donc
//! l'ordre du texte historique. Les golden files échouent si cet ordre change.

use std::sync::Arc;

use deveye_core::WaitOutcome;
use rmcp::model::{CallToolResult, ContentBlock, JsonObject, MetaObject};
use schemars::JsonSchema;
use serde::Serialize;

use crate::to_json_text;

/// `outputSchema` d'un outil, généré depuis le type qu'il sérialise (contrat de
/// sérialisation : un `Option` sans `skip_serializing_if` est toujours présent).
pub(crate) fn output_schema<T: JsonSchema + 'static>() -> Arc<JsonObject> {
    rmcp::handler::server::common::schema_for_output::<T>()
}

/// Succès : le même JSON en texte (bloc `content`) et en `structuredContent`.
pub(crate) fn structured_ok<T: Serialize>(value: &T) -> CallToolResult {
    let mut result = CallToolResult::success(vec![ContentBlock::text(to_json_text(value))]);
    result.structured_content = serde_json::to_value(value).ok();
    result
}

/// Échec sémantique (`isError = true`) dont la charge reste du type de succès de
/// l'outil (attente expirée, assertion fausse) : elle respecte donc son
/// `outputSchema`.
pub(crate) fn structured_failure<T: Serialize>(value: &T) -> CallToolResult {
    let mut result = CallToolResult::error(vec![ContentBlock::text(to_json_text(value))]);
    result.structured_content = serde_json::to_value(value).ok();
    result
}

/// Résultat d'une attente qui renvoie un [`WaitOutcome`] : succès si le prédicat
/// est satisfait, échec sémantique (délai écoulé) sinon.
pub(crate) fn outcome_result(outcome: &WaitOutcome) -> CallToolResult {
    if outcome.satisfied {
        structured_ok(outcome)
    } else {
        structured_failure(outcome)
    }
}

/// Métadonnées `_meta` du résultat de `state` : additives, ignorées des clients
/// qui ne les connaissent pas, sans toucher au texte (ADR-0067).
pub(crate) fn meta_of(
    entries: impl IntoIterator<Item = (&'static str, serde_json::Value)>,
) -> MetaObject {
    let mut object = JsonObject::new();
    for (key, value) in entries {
        object.insert(key.to_string(), value);
    }
    MetaObject(object)
}

// ----------------------------------------------------------------------
// Succès
// ----------------------------------------------------------------------

/// Acquittement des outils « fire-and-forget » (`click`, `type_text`, `scroll`…).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(crate) struct Ack {
    /// Toujours `true` : un échec est un `isError`, pas un acquittement.
    pub(crate) ok: bool,
}

impl Ack {
    pub(crate) fn done() -> Self {
        Self { ok: true }
    }
}

/// Acquittement de `click_selector` / `click_text` : point cliqué (px).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(crate) struct ClickAck {
    pub(crate) ok: bool,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

/// Acquittement de `focus_window` : titre de la fenêtre activée.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(crate) struct FocusAck {
    pub(crate) ok: bool,
    pub(crate) title: String,
}

/// Issue de `wait_for_selector` et `wait_for_text`. Avec `negate`, `found`
/// signifie « disparu ».
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(crate) struct FoundOutcome {
    pub(crate) elapsed_ms: u64,
    pub(crate) found: bool,
}

/// Issue de `wait_ready`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(crate) struct ReadyOutcome {
    pub(crate) ready: bool,
    pub(crate) waited_ms: u64,
}

/// Une clé du state SDK qui diverge de l'attendu (`assert_state`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(crate) struct StateMismatch {
    /// Valeur observée ; `null` si la clé est absente du state.
    pub(crate) actual: Option<serde_json::Value>,
    pub(crate) expected: serde_json::Value,
    pub(crate) key: String,
}

/// Résultat de `assert_state`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(crate) struct StateAssertion {
    pub(crate) mismatches: Vec<StateMismatch>,
    pub(crate) ok: bool,
}

/// Résultat de `assert_selector`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(crate) struct SelectorAssertion {
    pub(crate) actual_count: usize,
    /// Texte du premier élément ; `null` s'il n'y en a aucun.
    pub(crate) actual_text: Option<String>,
    pub(crate) ok: bool,
    pub(crate) selector: String,
}

// ----------------------------------------------------------------------
// Erreurs
// ----------------------------------------------------------------------

/// Cause d'une [`ToolError`]. Ouverte : de nouvelles causes s'ajoutent sans
/// rupture, un client doit traiter une valeur inconnue comme une erreur générique.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub(crate) enum ErrorReason {
    /// Capacité absente sur la cible active (cas normal, pas une panne).
    Unsupported,
    /// Le SDK de l'app est injoignable.
    SdkUnavailable,
    /// L'interaction a échoué côté cible (ex. aucun champ éditable au focus).
    InteractionFailed,
    /// La référence d'élément a existé mais l'élément a disparu (`act`).
    StaleRef,
    /// L'app n'a pas republié son arbre d'éléments depuis trop longtemps (`act`).
    StaleTree,
    /// La référence n'a jamais été émise ou n'a pas une forme connue (`act`).
    UnknownRef,
    /// Plusieurs éléments portent la même référence : agir serait un pari.
    AmbiguousRef,
    /// Un autre élément recouvre le centre de la cible.
    Obscured,
    /// L'élément est hors de la zone visible, ou sans surface.
    Offscreen,
    /// L'élément refuse les interactions (désactivé).
    Disabled,
}

/// Erreur sémantique d'un outil (`isError = true`) : `{ ok: false, reason,
/// detail, hint?, candidates? }`. `hint` et `candidates` sont absents tant
/// qu'aucun site d'appel ne les renseigne, ce qui garde le texte historique des
/// erreurs existantes champ pour champ (ADR-0067).
///
/// Le texte et le `structuredContent` sont identiques. `outputSchema` ne décrit
/// que le succès : une erreur ne s'y conforme pas, ce que les clients qui
/// valident `structuredContent` malgré `isError` signalent comme une erreur de
/// validation, sans perte de l'`isError` (risque documenté dans l'ADR).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(crate) struct ToolError {
    /// Référence de l'élément qui en recouvre un autre (`reason: obscured`).
    #[serde(skip_serializing_if = "Option::is_none")]
    by: Option<String>,
    /// Candidats proposés pour corriger l'appel (ex. noms valides).
    #[serde(skip_serializing_if = "Option::is_none")]
    candidates: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    /// Piste de correction lisible par l'agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
    /// Toujours `false`.
    ok: bool,
    reason: ErrorReason,
    /// Référence d'élément en cause (`snapshot` et `act`).
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    reference: Option<String>,
}

impl ToolError {
    pub(crate) fn new(reason: ErrorReason, detail: impl Into<String>) -> Self {
        Self {
            by: None,
            candidates: None,
            detail: Some(detail.into()),
            hint: None,
            ok: false,
            reason,
            reference: None,
        }
    }

    pub(crate) fn with_reference(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }

    pub(crate) fn with_covered_by(mut self, by: impl Into<String>) -> Self {
        self.by = Some(by.into());
        self
    }

    pub(crate) fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub(crate) fn with_candidates(mut self, candidates: Vec<String>) -> Self {
        self.candidates = Some(candidates);
        self
    }

    pub(crate) fn into_result(self) -> CallToolResult {
        structured_failure(&self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_error_text_matches_the_historical_unsupported_payload() {
        let result = ToolError::new(ErrorReason::Unsupported, "pas de webview").into_result();
        let text = result.content[0].as_text().expect("texte").text.clone();
        assert_eq!(
            text,
            json!({ "ok": false, "reason": "unsupported", "detail": "pas de webview" }).to_string()
        );
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content,
            Some(serde_json::from_str(&text).expect("json"))
        );
    }

    #[test]
    fn tool_error_adds_hint_and_candidates_only_when_set() {
        let error = ToolError::new(ErrorReason::InteractionFailed, "aucun champ")
            .with_hint("cliquer d'abord sur un champ")
            .with_candidates(vec!["#a".to_string(), "#b".to_string()]);
        let value = serde_json::to_value(&error).expect("sérialisation");
        assert_eq!(value["hint"], "cliquer d'abord sur un champ");
        assert_eq!(value["candidates"], json!(["#a", "#b"]));
        assert_eq!(value["reason"], "interaction_failed");
        let bare = serde_json::to_value(ToolError::new(ErrorReason::SdkUnavailable, "x"))
            .expect("sérialisation");
        assert!(bare.get("hint").is_none() && bare.get("candidates").is_none());
        assert_eq!(bare["reason"], "sdk_unavailable");
    }

    #[test]
    fn outcome_result_flags_a_timeout_as_an_error_with_the_same_payload() {
        let ok = outcome_result(&WaitOutcome::new(true, 3, "ok"));
        let ko = outcome_result(&WaitOutcome::new(false, 3, "délai"));
        assert_eq!(ok.is_error, Some(false));
        assert_eq!(ko.is_error, Some(true));
        assert_eq!(
            ko.structured_content.expect("structuré")["satisfied"],
            false
        );
    }
}
