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
            http: deveye_sdk_client::client(),
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
            // 504 = la pompe a été jointe mais n'a pas répondu dans les 10 s
            // impartis par le SDK — distinct d'une erreur JS (Render) comme
            // d'une absence de liaison (Network).
            if status == reqwest::StatusCode::GATEWAY_TIMEOUT {
                return Err(PortError::Timeout(format!("eval JS timeout : {error}")));
            }
            // 500 = la pompe a abandonné la requête (oneshot fermé) : rupture
            // de liaison avec la pompe, pas un échec JS — Network, pour que le
            // mode auto bascule comme pour un 503.
            if status == reqwest::StatusCode::INTERNAL_SERVER_ERROR {
                return Err(PortError::Network(format!(
                    "SDK eval (pompe abandonnee) : {error}"
                )));
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

/// `None` pour un bouton inconnu (`MouseButton` est `#[non_exhaustive]`).
fn button_code(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    }
}

fn click_snippet(position: Position, button: MouseButton) -> PortResult<String> {
    let b = button_code(button).ok_or_else(|| {
        PortError::Unsupported(format!(
            "bouton de souris {button:?} non géré par le pont SDK"
        ))
    })?;
    let (x, y) = (position.x, position.y);
    // `click` pour le bouton gauche, `contextmenu` pour le droit — comme un
    // navigateur. `el.focus()` entre press et release : ordre humain.
    let activation = match button {
        MouseButton::Right => "el.dispatchEvent(new MouseEvent('contextmenu', opts));",
        _ => "el.dispatchEvent(new MouseEvent('click', opts));",
    };
    Ok(format!(
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
    ))
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

/// Vrai si `modifiers` contient l'un des alias donnes (insensible a la casse).
fn has_modifier(modifiers: &[String], aliases: &[&str]) -> bool {
    modifiers
        .iter()
        .any(|m| aliases.iter().any(|a| m.eq_ignore_ascii_case(a)))
}

fn keycombo_snippet(modifiers: &[String], key: &str) -> String {
    let key = serde_json::to_string(key).expect("string serialise");
    let ctrl = has_modifier(modifiers, &["ctrl", "control"]);
    let shift = has_modifier(modifiers, &["shift"]);
    let alt = has_modifier(modifiers, &["alt"]);
    let meta = has_modifier(modifiers, &["meta", "super", "cmd", "command"]);
    // KeyboardEvent synthetique avec les booleens de modificateurs : c'est ce
    // que lisent les frameworks (e.ctrlKey, e.shiftKey...). Moins fidele qu'un
    // vrai event OS (les raccourcis du moteur ne se declenchent pas), mais
    // suffisant pour les handlers applicatifs.
    format!(
        "(function(){{ const el = document.activeElement || document.body; \
         const opts = {{ key: {key}, ctrlKey: {ctrl}, shiftKey: {shift}, altKey: {alt}, metaKey: {meta}, \
                         bubbles: true, cancelable: true }}; \
         el.dispatchEvent(new KeyboardEvent('keydown', opts)); \
         el.dispatchEvent(new KeyboardEvent('keyup', opts)); \
         return {{ ok: true }}; }})()"
    )
}

fn doubleclick_snippet(position: Position) -> String {
    let (x, y) = (position.x, position.y);
    // Deux clicks + un dblclick synthetiques sur l'element sous le point.
    format!(
        "(function(){{ const el = document.elementFromPoint({x}, {y}); \
         if (!el) return {{ ok: false, reason: 'aucun élément à ({x}, {y})' }}; \
         const opts = {{ bubbles: true, cancelable: true, composed: true, view: window, \
                         clientX: {x}, clientY: {y}, button: 0, buttons: 1 }}; \
         for (let i = 0; i < 2; i++) {{ \
           el.dispatchEvent(new MouseEvent('mousedown', opts)); \
           el.dispatchEvent(new MouseEvent('mouseup', opts)); \
           el.dispatchEvent(new MouseEvent('click', {{ ...opts, detail: i + 1 }})); \
         }} \
         el.dispatchEvent(new MouseEvent('dblclick', {{ ...opts, detail: 2 }})); \
         return {{ ok: true }}; }})()"
    )
}

fn scroll_snippet(position: Position, dx: i32, dy: i32) -> String {
    let (x, y) = (position.x, position.y);
    // Piege connu (usage.md) : le scroll de l'app vit souvent dans un conteneur
    // (.shell__content), pas dans window. On remonte les ancetres a la recherche
    // du premier element reellement scrollable sous le point ; sinon window.
    format!(
        "(function(){{ let n = document.elementFromPoint({x}, {y}); \
         const scrollable = (e) => {{ while (e && e !== document.body && e !== document.documentElement) {{ \
           const s = getComputedStyle(e); \
           if (/(auto|scroll)/.test((s.overflowY || '') + (s.overflowX || '')) \
               && (e.scrollHeight > e.clientHeight || e.scrollWidth > e.clientWidth)) return e; \
           e = e.parentElement; }} return null; }}; \
         const t = scrollable(n); \
         if (t) t.scrollBy({dx}, {dy}); else window.scrollBy({dx}, {dy}); \
         return {{ ok: true }}; }})()"
    )
}

fn navigate_snippet(route: &str) -> String {
    let route = serde_json::to_string(route).expect("string serialise");
    // Le hook routeur `__deveye_router__` est injecté par le SDK opt-in.
    format!(
        "(function(){{ if (!window.__deveye_router__) \
         return {{ ok: false, reason: 'hook routeur absent' }}; \
         window.__deveye_router__.invoke('navigate', {{ route: {route} }}); \
         return {{ ok: true }}; }})()"
    )
}

/// Glisser-déposer en événements pointer/souris synthétiques (fallback SDK,
/// sans CDP). Moins fidèle que la séquence souris native CDP — un widget qui
/// n'écoute que le drag-drop HTML5 natif (`dragstart`/`drop`) ne réagira pas.
fn dragdrop_snippet(from: Position, to: Position) -> String {
    let (fx, fy, tx, ty) = (from.x, from.y, to.x, to.y);
    format!(
        "(function(){{ const src = document.elementFromPoint({fx}, {fy}); \
         if (!src) return {{ ok: false, reason: 'aucun élément source à ({fx}, {fy})' }}; \
         const o = (x, y) => ({{ bubbles: true, cancelable: true, composed: true, view: window, \
                                 clientX: x, clientY: y, pointerId: 1, isPrimary: true, button: 0, buttons: 1 }}); \
         src.dispatchEvent(new PointerEvent('pointerdown', o({fx}, {fy}))); \
         src.dispatchEvent(new MouseEvent('mousedown', o({fx}, {fy}))); \
         const dst = document.elementFromPoint({tx}, {ty}) || src; \
         dst.dispatchEvent(new PointerEvent('pointermove', o({tx}, {ty}))); \
         dst.dispatchEvent(new MouseEvent('mousemove', o({tx}, {ty}))); \
         dst.dispatchEvent(new PointerEvent('pointerup', o({tx}, {ty}))); \
         dst.dispatchEvent(new MouseEvent('mouseup', o({tx}, {ty}))); \
         return {{ ok: true }}; }})()"
    )
}

#[async_trait]
impl InteractionSink for JsInteraction {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
        match command {
            InteractionCommand::Click { position, button } => {
                self.run(click_snippet(position, button)?).await
            }
            InteractionCommand::Hover { position } => self.run(hover_snippet(position)).await,
            InteractionCommand::Type { text } => self.run(type_snippet(&text)).await,
            InteractionCommand::KeyPress { key } => self.run(keypress_snippet(&key)).await,
            InteractionCommand::KeyCombo { modifiers, key } => {
                self.run(keycombo_snippet(&modifiers, &key)).await
            }
            InteractionCommand::Navigate { route } => {
                // Meme contrat que le chemin CDP : le hook routeur du SDK.
                self.run(navigate_snippet(&route)).await
            }
            InteractionCommand::DragDrop { from, to } => self.run(dragdrop_snippet(from, to)).await,
            InteractionCommand::Scroll { position, dx, dy } => {
                self.run(scroll_snippet(position, dx, dy)).await
            }
            InteractionCommand::DoubleClick { position } => {
                self.run(doubleclick_snippet(position)).await
            }
            // CDP-only : un dialog JS suspend la pompe eval (eval_js impossible),
            // et la securite navigateur interdit de remplir un input fichier en
            // JS — il n'y a pas d'equivalent synthetique honnete via le pont SDK.
            InteractionCommand::HandleDialog { .. } => Err(PortError::Unsupported(
                "handle_dialog nécessite CDP (Page.handleJavaScriptDialog) ; \
                 indisponible via le pont SDK"
                    .to_string(),
            )),
            InteractionCommand::UploadFile { .. } => Err(PortError::Unsupported(
                "upload_file nécessite CDP (DOM.setFileInputFiles) ; la sécurité \
                 navigateur interdit de remplir un input fichier en JS"
                    .to_string(),
            )),
            // `InteractionCommand` est `#[non_exhaustive]` : refus explicite.
            // Pas de `Debug` de la commande : il embarquerait le texte tapé.
            _ => Err(PortError::Unsupported(
                "commande non gérée par le pont SDK".to_string(),
            )),
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ---- SdkEvaluator::evaluate — mapping HTTP -> PortError ----------------

    /// POST /devtools/eval 503 (pompe absente) => PortError::Network, pour
    /// que les compositions a repli sachent basculer.
    #[tokio::test]
    async fn evaluate_503_pompe_absente_renvoie_network() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/devtools/eval"))
            .respond_with(
                ResponseTemplate::new(503)
                    .set_body_json(serde_json::json!({ "error": "pompe absente" })),
            )
            .mount(&server)
            .await;

        let evaluator = SdkEvaluator::new(server.uri());
        let err = evaluator
            .evaluate("1 + 1")
            .await
            .expect_err("503 doit echouer");
        assert!(
            matches!(err, PortError::Network(_)),
            "attendu PortError::Network, obtenu {err:?}"
        );
    }

    /// POST /devtools/eval 504 (pompe jointe mais muette) => PortError::Timeout.
    #[tokio::test]
    async fn evaluate_504_pompe_muette_renvoie_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/devtools/eval"))
            .respond_with(
                ResponseTemplate::new(504)
                    .set_body_json(serde_json::json!({ "error": "pompe muette" })),
            )
            .mount(&server)
            .await;

        let evaluator = SdkEvaluator::new(server.uri());
        let err = evaluator
            .evaluate("1 + 1")
            .await
            .expect_err("504 doit echouer");
        assert!(
            matches!(err, PortError::Timeout(_)),
            "attendu PortError::Timeout, obtenu {err:?}"
        );
    }

    /// POST /devtools/eval 500 (pompe abandonnee, oneshot ferme) => PortError::Network.
    #[tokio::test]
    async fn evaluate_500_pompe_abandonnee_renvoie_network() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/devtools/eval"))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_body_json(serde_json::json!({ "error": "pompe abandonnee" })),
            )
            .mount(&server)
            .await;

        let evaluator = SdkEvaluator::new(server.uri());
        let err = evaluator
            .evaluate("1 + 1")
            .await
            .expect_err("500 doit echouer");
        assert!(
            matches!(err, PortError::Network(_)),
            "attendu PortError::Network, obtenu {err:?}"
        );
    }

    /// POST /devtools/eval 200 sans champ `value` => Value::Null (pas d'erreur).
    #[tokio::test]
    async fn evaluate_200_sans_value_renvoie_null() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/devtools/eval"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        let evaluator = SdkEvaluator::new(server.uri());
        let value = evaluator
            .evaluate("undefined")
            .await
            .expect("200 doit reussir");
        assert_eq!(value, serde_json::Value::Null);
    }

    #[test]
    fn click_snippet_gauche_active_par_click() {
        let js = click_snippet(Position::new(10, 20), MouseButton::Left).unwrap();
        assert!(js.contains("elementFromPoint(10, 20)"));
        assert!(js.contains("new MouseEvent('click'"));
        assert!(js.contains("el.focus()"));
    }

    #[test]
    fn click_snippet_droit_active_par_contextmenu() {
        let js = click_snippet(Position::new(1, 2), MouseButton::Right).unwrap();
        assert!(js.contains("contextmenu"));
        assert!(!js.contains("new MouseEvent('click'"));
    }

    #[test]
    fn type_snippet_echappe_le_texte() {
        let js = type_snippet("l'apostrophe \"double\"");
        assert!(js.contains(r#""l'apostrophe \"double\"""#));
    }

    #[test]
    fn dragdrop_snippet_source_et_cible() {
        let js = dragdrop_snippet(Position::new(10, 20), Position::new(100, 200));
        assert!(js.contains("elementFromPoint(10, 20)"));
        assert!(js.contains("elementFromPoint(100, 200)"));
        assert!(js.contains("pointerdown"));
        assert!(js.contains("pointerup"));
    }

    #[test]
    fn hover_snippet_contient_coordonnees_et_evenements() {
        let js = hover_snippet(Position::new(42, 77));
        assert!(js.contains("elementFromPoint(42, 77)"));
        assert!(js.contains("pointerover"));
        assert!(js.contains("mouseover"));
        assert!(js.contains("pointermove"));
        assert!(js.contains("mousemove"));
        // aucun click ne doit etre genere
        assert!(!js.contains("click"));
        assert!(!js.contains("pointerdown"));
    }

    #[test]
    fn keypress_snippet_echappe_la_cle_et_dispatch_keydown_keyup() {
        let js = keypress_snippet("Enter");
        assert!(js.contains("\"Enter\""));
        assert!(js.contains("keydown"));
        assert!(js.contains("keyup"));
    }

    #[test]
    fn keypress_snippet_echappe_guillemets_dans_la_cle() {
        // Cle pathologique : verifier que serde_json echappe correctement.
        let js = keypress_snippet("a\"b");
        assert!(js.contains(r#""a\"b""#));
    }

    #[test]
    fn doubleclick_snippet_emet_deux_clicks_et_un_dblclick() {
        let js = doubleclick_snippet(Position::new(12, 34));
        assert!(js.contains("elementFromPoint(12, 34)"));
        assert!(js.contains("'click'"));
        assert!(js.contains("'dblclick'"));
        assert!(js.contains("mousedown"));
        assert!(js.contains("mouseup"));
    }

    #[test]
    fn scroll_snippet_cible_le_conteneur_scrollable_sinon_window() {
        let js = scroll_snippet(Position::new(42, 77), 0, 600);
        assert!(js.contains("elementFromPoint(42, 77)"));
        assert!(js.contains("scrollBy(0, 600)"));
        assert!(js.contains("window.scrollBy(0, 600)"));
        assert!(js.contains("scrollHeight"));
    }

    #[test]
    fn keycombo_snippet_active_les_bons_modificateurs() {
        let js = keycombo_snippet(&["Ctrl".to_string(), "shift".to_string()], "KeyP");
        assert!(js.contains("ctrlKey: true"));
        assert!(js.contains("shiftKey: true"));
        assert!(js.contains("altKey: false"));
        assert!(js.contains("metaKey: false"));
        assert!(js.contains("\"KeyP\""));
        assert!(js.contains("keydown"));
        assert!(js.contains("keyup"));
    }

    #[test]
    fn has_modifier_est_insensible_a_la_casse_et_aux_alias() {
        let mods = vec!["SUPER".to_string()];
        assert!(has_modifier(&mods, &["meta", "super", "cmd"]));
        assert!(!has_modifier(&mods, &["ctrl", "control"]));
    }

    #[test]
    fn navigate_snippet_contient_route_et_hook() {
        let js = navigate_snippet("/accueil");
        assert!(js.contains("__deveye_router__"));
        assert!(js.contains("navigate"));
        assert!(js.contains("\"/accueil\""));
    }

    #[test]
    fn navigate_snippet_echappe_route_avec_guillemets() {
        let js = navigate_snippet("/path?q=\"x\"");
        // serde_json doit echapper les guillemets internes
        assert!(js.contains(r#""/path?q=\"x\"""#));
    }
}
