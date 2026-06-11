//! Pont natif DevEye — les ports « yeux et mains » servis **par le SDK**
//! plutôt que par CDP, pour piloter une app Dioxus desktop dont le webview
//! n'expose pas de protocole de debug (WebKitGTK sous Linux).
//!
//! Trois pièces, composées par `deveye-server` quand la cible est `sdk`
//! (ou en repli quand le CDP est injoignable) :
//! - [`SdkEvaluator`] : `JsEvaluator` → `POST /devtools/eval` du SDK ; l'app
//!   exécute l'expression dans son webview via la pompe `document::eval`.
//! - [`JsInteraction`] : `InteractionSink` au-dessus d'un `JsEvaluator` —
//!   clics/saisies en événements JS synthétiques complets (pointer + mouse
//!   + focus). Moins fidèle que les événements souris CDP, mais fonctionnel.
//! - [`EvalDomInspector`] : `AppInspector::dom_snapshot` via `outerHTML`.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use deveye_core::{DomTree, InteractionCommand, MouseButton, Position, StateDump};
use deveye_ports::{AppInspector, InteractionSink, JsEvaluator, PortError, PortResult};

/// `JsEvaluator` parlant au SDK opt-in (`POST /devtools/eval`).
#[derive(Clone, Debug)]
pub struct SdkEvaluator {
    base_url: String,
    http: reqwest::Client,
}

impl SdkEvaluator {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::new(),
        }
    }
}

/// Réponse de `POST /devtools/eval` : `{ value }` ou `{ error }`.
#[derive(Deserialize)]
struct EvalResponse {
    #[serde(default)]
    value: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<String>,
}

#[async_trait]
impl JsEvaluator for SdkEvaluator {
    async fn evaluate(&self, expression: &str) -> PortResult<serde_json::Value> {
        let url = format!("{}/devtools/eval", self.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "expression": expression }))
            .send()
            .await
            .map_err(|e| PortError::Network(format!("POST {url} : {e}")))?;
        let status = resp.status();
        let body: EvalResponse = resp
            .json()
            .await
            .map_err(|e| PortError::Render(format!("decode réponse eval : {e}")))?;
        if let Some(error) = body.error {
            // 503 = pompe absente (app sans intégration eval) : c'est un
            // problème de *liaison*, pas de JS — Network, pour que les
            // compositions à repli sachent basculer.
            if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                return Err(PortError::Network(format!("SDK eval : {error}")));
            }
            return Err(PortError::Render(format!("eval JS : {error}")));
        }
        Ok(body.value.unwrap_or(serde_json::Value::Null))
    }
}

/// `InteractionSink` générique au-dessus d'un [`JsEvaluator`] : traduit les
/// commandes en événements JS synthétiques **complets** (pointer + mouse +
/// focus), dispatchés sur l'élément sous le point visé.
pub struct JsInteraction {
    evaluator: Arc<dyn JsEvaluator>,
}

impl JsInteraction {
    pub fn new(evaluator: Arc<dyn JsEvaluator>) -> Self {
        Self { evaluator }
    }

    /// Évalue un snippet au contrat `{ ok, reason? }` et convertit l'échec
    /// JS en erreur d'interaction.
    async fn run(&self, snippet: String) -> PortResult<()> {
        let value = self.evaluator.evaluate(&snippet).await?;
        let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if ok {
            Ok(())
        } else {
            let reason = value
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("échec JS sans raison");
            Err(PortError::Interaction(reason.to_string()))
        }
    }
}

fn button_code(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

fn click_snippet(position: Position, button: MouseButton) -> String {
    let (x, y, b) = (position.x, position.y, button_code(button));
    // `click` pour le bouton gauche, `contextmenu` pour le droit — comme un
    // navigateur. `el.focus()` entre press et release : ordre humain.
    let activation = match button {
        MouseButton::Right => "el.dispatchEvent(new MouseEvent('contextmenu', opts));",
        _ => "el.dispatchEvent(new MouseEvent('click', opts));",
    };
    format!(
        "(function(){{ const el = document.elementFromPoint({x}, {y}); \
         if (!el) return {{ ok: false, reason: 'aucun élément à ({x}, {y})' }}; \
         const opts = {{ bubbles: true, cancelable: true, composed: true, view: window, \
                         clientX: {x}, clientY: {y}, button: {b}, buttons: 1 }}; \
         el.dispatchEvent(new PointerEvent('pointerover', opts)); \
         el.dispatchEvent(new MouseEvent('mouseover', opts)); \
         el.dispatchEvent(new PointerEvent('pointermove', opts)); \
         el.dispatchEvent(new MouseEvent('mousemove', opts)); \
         el.dispatchEvent(new PointerEvent('pointerdown', opts)); \
         el.dispatchEvent(new MouseEvent('mousedown', opts)); \
         if (typeof el.focus === 'function') el.focus(); \
         el.dispatchEvent(new PointerEvent('pointerup', opts)); \
         el.dispatchEvent(new MouseEvent('mouseup', opts)); \
         {activation} \
         return {{ ok: true }}; }})()"
    )
}

fn hover_snippet(position: Position) -> String {
    let (x, y) = (position.x, position.y);
    format!(
        "(function(){{ const el = document.elementFromPoint({x}, {y}); \
         if (!el) return {{ ok: false, reason: 'aucun élément à ({x}, {y})' }}; \
         const opts = {{ bubbles: true, cancelable: true, composed: true, view: window, \
                         clientX: {x}, clientY: {y} }}; \
         el.dispatchEvent(new PointerEvent('pointerover', opts)); \
         el.dispatchEvent(new MouseEvent('mouseover', opts)); \
         el.dispatchEvent(new PointerEvent('pointermove', opts)); \
         el.dispatchEvent(new MouseEvent('mousemove', opts)); \
         return {{ ok: true }}; }})()"
    )
}

fn type_snippet(text: &str) -> String {
    let text = serde_json::to_string(text).expect("string serialise");
    // Setter natif du prototype : les frameworks réactifs interceptent la
    // propriété `value` de l'instance — passer par le prototype garantit que
    // l'événement `input` voit la nouvelle valeur.
    format!(
        "(function(){{ const el = document.activeElement; const text = {text}; \
         if (!el || !('value' in el)) return {{ ok: false, reason: 'aucun champ éditable au focus' }}; \
         const desc = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), 'value'); \
         const next = (el.value ?? '') + text; \
         if (desc && desc.set) desc.set.call(el, next); else el.value = next; \
         el.dispatchEvent(new InputEvent('input', {{ bubbles: true, data: text, inputType: 'insertText' }})); \
         return {{ ok: true }}; }})()"
    )
}

fn keypress_snippet(key: &str) -> String {
    let key = serde_json::to_string(key).expect("string serialise");
    format!(
        "(function(){{ const el = document.activeElement || document.body; \
         const opts = {{ key: {key}, bubbles: true, cancelable: true }}; \
         el.dispatchEvent(new KeyboardEvent('keydown', opts)); \
         el.dispatchEvent(new KeyboardEvent('keyup', opts)); \
         return {{ ok: true }}; }})()"
    )
}

#[async_trait]
impl InteractionSink for JsInteraction {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
        match command {
            InteractionCommand::Click { position, button } => {
                self.run(click_snippet(position, button)).await
            }
            InteractionCommand::Hover { position } => self.run(hover_snippet(position)).await,
            InteractionCommand::Type { text } => self.run(type_snippet(&text)).await,
            InteractionCommand::KeyPress { key } => self.run(keypress_snippet(&key)).await,
            InteractionCommand::Navigate { route } => {
                // Même contrat que le chemin CDP : le hook routeur du SDK.
                let route = serde_json::to_string(&route).expect("string serialise");
                let snippet = format!(
                    "(function(){{ if (!window.__deveye_router__) \
                     return {{ ok: false, reason: 'hook routeur absent' }}; \
                     window.__deveye_router__.invoke('navigate', {{ route: {route} }}); \
                     return {{ ok: true }}; }})()"
                );
                self.run(snippet).await
            }
        }
    }
}

/// `AppInspector` dont le DOM vient de l'évaluateur (`outerHTML`). Le state
/// reste servi par [`crate::DioxusSignalsInspector`] via la composition du
/// binaire orchestrateur.
pub struct EvalDomInspector {
    evaluator: Arc<dyn JsEvaluator>,
}

impl EvalDomInspector {
    pub fn new(evaluator: Arc<dyn JsEvaluator>) -> Self {
        Self { evaluator }
    }
}

#[async_trait]
impl AppInspector for EvalDomInspector {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        let value = self
            .evaluator
            .evaluate("document.documentElement.outerHTML")
            .await?;
        Ok(DomTree::new(value))
    }

    async fn state_snapshot(&self) -> PortResult<StateDump> {
        Ok(StateDump::empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_snippet_gauche_active_par_click() {
        let js = click_snippet(Position::new(10, 20), MouseButton::Left);
        assert!(js.contains("elementFromPoint(10, 20)"));
        assert!(js.contains("new MouseEvent('click'"));
        assert!(js.contains("el.focus()"));
    }

    #[test]
    fn click_snippet_droit_active_par_contextmenu() {
        let js = click_snippet(Position::new(1, 2), MouseButton::Right);
        assert!(js.contains("contextmenu"));
        assert!(!js.contains("new MouseEvent('click'"));
    }

    #[test]
    fn type_snippet_echappe_le_texte() {
        let js = type_snippet("l'apostrophe \"double\"");
        assert!(js.contains(r#""l'apostrophe \"double\"""#));
    }
}
