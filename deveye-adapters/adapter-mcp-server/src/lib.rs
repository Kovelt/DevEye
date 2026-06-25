//! Adapter DevEye qui expose les ports `Renderer`, `AppInspector`,
//! `InteractionSink` et `LogSource` au format Model Context Protocol via
//! `rmcp` (transport stdio).
//!
//! Treize outils sont publiés à Claude (ou à n'importe quel client MCP) :
//! - `screenshot` : capture le viewport en PNG, renvoie le base64
//! - `click` : clic à `(x, y)` (bouton gauche)
//! - `right_click` : clic droit à `(x, y)` (ouvre les menus contextuels)
//! - `hover` : déplace le pointeur à `(x, y)` sans presser (déclenche
//!   `mouseover`/`mouseenter`)
//! - `click_selector` : clique sur le premier élément matchant un sélecteur
//!   CSS — repéré en JS, cliqué par **vrais événements souris CDP** (le
//!   focus clavier suit, comme pour un humain)
//! - `click_text` : pareil, par texte exact (boutons, liens, labels, rôles
//!   interactifs)
//! - `eval_js` : évalue une expression JS dans la page cible, retourne la
//!   valeur sérialisée
//! - `type_text` : saisit du texte au focus courant
//! - `dom` : sérialise le DOM de l'app cible
//! - `state` : sérialise le state opt-in poussé par le SDK
//! - `logs_tail` : draine et renvoie le buffer de logs accumulé
//! - `invoke_command` : invoque une commande logique exposée par l'app
//!   (route déclarée via `deveye_sdk::expose_route`)
//! - `list_commands` : liste les commandes logiques exposées par l'app
//!
//! Les appels d'outils sont **sérialisés** (un seul à la fois, mutex) : des
//! interactions concurrentes pendant un re-render Dioxus touchaient le
//! mauvais nœud (leçon du 5 juin 2026).
//!
//! Voir [`deveye.md`](../../../../../../docs/architecture/deveye.md) côté
//! Chronos pour le contexte global.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    service::ServiceExt,
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler,
};
use schemars::JsonSchema;
use serde::Deserialize;

use deveye_core::{InteractionCommand, MouseButton, Position};
use deveye_ports::{
    AppInspector, CommandRouter, InteractionSink, JsEvaluator, LogSource, Renderer,
};

/// Serveur MCP DevEye.
///
/// Le `tool_router` est construit une fois au `new` ; le serveur lui-même est
/// `Clone` parce que `rmcp` peut le partager entre tâches.
#[derive(Clone)]
pub struct DeveyeMcpServer {
    renderer: Arc<dyn Renderer>,
    interaction: Arc<dyn InteractionSink>,
    inspector: Arc<dyn AppInspector>,
    log_source: Arc<dyn LogSource>,
    evaluator: Arc<dyn JsEvaluator>,
    command_router: Arc<dyn CommandRouter>,
    /// Sérialise les appels d'outils : rmcp les traite en concurrence, mais
    /// deux interactions entrelacées pendant un re-render visent le mauvais
    /// nœud. Un seul outil à la fois, dans l'ordre d'arrivée.
    gate: Arc<tokio::sync::Mutex<()>>,
    /// Le routeur d'outils généré par `#[tool_router]` est utilisé par le
    /// macro `#[tool_handler]` via metaprogramming — clippy ne voit pas
    /// l'accès et le déclare unread.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ClickParams {
    /// Abscisse en pixels logiques depuis le coin haut-gauche du viewport.
    x: i32,
    /// Ordonnée en pixels logiques depuis le coin haut-gauche du viewport.
    y: i32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TypeParams {
    /// Texte UTF-8 à insérer au focus courant.
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct EvalParams {
    /// Expression JavaScript à évaluer dans le contexte de la page.
    /// Promises supportées (awaitPromise=true) ; la valeur est renvoyée par
    /// valeur sérialisée JSON.
    expression: String,
}

/// Fragment JS commun aux deux clics ciblés : `el` étant l'élément trouvé
/// (ou `undefined`), le faire défiler au centre puis renvoyer le **centre de
/// son rect** en pixels CSS viewport — les mêmes unités que
/// `Input.dispatchMouseEvent`. Le clic lui-même part ensuite en vrais
/// événements souris CDP (focus clavier compris), jamais en `el.click()`.
const LOCATE_CENTER: &str = "if (!el) return { ok: false, reason: 'not_found' }; \
     el.scrollIntoView({ block: 'center', behavior: 'instant' }); \
     const r = el.getBoundingClientRect(); \
     if (r.width === 0 || r.height === 0) return { ok: false, reason: 'not_visible' }; \
     return { ok: true, x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) };";

/// Sélecteur des éléments « cliquables par texte » : éléments nativement
/// interactifs (dont les `<label>` — leçon du 5 juin 2026 : ils manquaient),
/// rôles ARIA interactifs, et l'échappatoire `data-deveye-click`.
const CLICKABLES: &str = "'button, a, label, summary, [role=button], [role=tab], \
     [role=menuitem], [role=menuitemradio], [role=option], [role=switch], \
     [role=checkbox], [data-deveye-click]'";

#[derive(Debug, Deserialize, JsonSchema)]
struct ClickSelectorParams {
    /// Sélecteur CSS du premier élément à cliquer (ex. `button.btn-primary`).
    selector: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ClickTextParams {
    /// Texte exact (après `trim`) de l'élément à cliquer.
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct InvokeCommandParams {
    /// Nom de la commande logique à invoquer (tel que listé par
    /// `list_commands`, déclaré côté app par `deveye_sdk::expose_route`).
    name: String,
    /// Arguments JSON passés au handler de la commande. Schéma défini par
    /// l'app cible ; absent ou `null` si la commande n'en attend pas.
    #[serde(default)]
    args: serde_json::Value,
}

#[tool_router]
impl DeveyeMcpServer {
    pub fn new(
        renderer: Arc<dyn Renderer>,
        interaction: Arc<dyn InteractionSink>,
        inspector: Arc<dyn AppInspector>,
        log_source: Arc<dyn LogSource>,
        evaluator: Arc<dyn JsEvaluator>,
        command_router: Arc<dyn CommandRouter>,
    ) -> Self {
        Self {
            renderer,
            interaction,
            inspector,
            log_source,
            evaluator,
            command_router,
            gate: Arc::new(tokio::sync::Mutex::new(())),
            tool_router: Self::tool_router(),
        }
    }

    /// Lance le serveur MCP en stdio. Bloque jusqu'à la fermeture du client.
    pub async fn serve_stdio(self) -> Result<(), McpError> {
        let service = self.serve(stdio()).await.map_err(internal)?;
        service.waiting().await.map_err(internal)?;
        Ok(())
    }

    #[tool(description = "Capture un screenshot PNG du viewport de l'app cible (image MCP).")]
    async fn screenshot(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let shot = self.renderer.screenshot().await.map_err(internal)?;
        let b64 = B64.encode(&shot.bytes);
        // Contenu `image` (et non `text`) : le client MCP affiche le PNG —
        // c'est ce qui rend l'outil utilisable comme « yeux ».
        Ok(CallToolResult::success(vec![Content::image(
            b64,
            "image/png",
        )]))
    }

    #[tool(
        description = "Effectue un clic gauche sur le viewport de l'app cible aux coordonnées absolues fournies."
    )]
    async fn click(
        &self,
        Parameters(p): Parameters<ClickParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let cmd = InteractionCommand::click(Position::new(p.x, p.y));
        self.interaction.dispatch(cmd).await.map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Effectue un clic DROIT (menu contextuel) sur le viewport de l'app cible aux coordonnées absolues fournies. Déclenche button=right (contextmenu) — ouvre les menus contextuels des UI qui les écoutent."
    )]
    async fn right_click(
        &self,
        Parameters(p): Parameters<ClickParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let cmd = InteractionCommand::Click {
            position: Position::new(p.x, p.y),
            button: MouseButton::Right,
        };
        self.interaction.dispatch(cmd).await.map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(description = "Saisit du texte au focus courant de l'app cible.")]
    async fn type_text(
        &self,
        Parameters(p): Parameters<TypeParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        self.interaction
            .dispatch(InteractionCommand::type_text(p.text))
            .await
            .map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Déplace le pointeur à (x, y) sans presser — déclenche mouseover/mouseenter (utile pour révéler des éléments au hover)."
    )]
    async fn hover(
        &self,
        Parameters(p): Parameters<ClickParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        self.interaction
            .dispatch(InteractionCommand::hover(Position::new(p.x, p.y)))
            .await
            .map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Évalue une expression JavaScript dans la page cible et renvoie la valeur JSON. Promises supportées (awaitPromise=true)."
    )]
    async fn eval_js(
        &self,
        Parameters(p): Parameters<EvalParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let value = self
            .evaluator
            .evaluate(&p.expression)
            .await
            .map_err(internal)?;
        let json = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Clique sur le premier élément matchant un sélecteur CSS, par vrais événements souris CDP (hover + press au centre de l'élément — le focus clavier suit). Robuste aux déplacements de coordonnées et aux animations."
    )]
    async fn click_selector(
        &self,
        Parameters(p): Parameters<ClickSelectorParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let snippet = format!(
            "(function(){{ const el = document.querySelector({selector}); {LOCATE_CENTER} }})()",
            selector = serde_json::to_string(&p.selector).expect("string serialise"),
        );
        self.click_located(&snippet).await
    }

    #[tool(
        description = "Clique sur le premier élément interactif (button, a, label, summary, role=button/tab/menuitem/option/switch/checkbox) dont le texte (après trim) correspond exactement — par vrais événements souris CDP (le focus clavier suit)."
    )]
    async fn click_text(
        &self,
        Parameters(p): Parameters<ClickTextParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let snippet = format!(
            "(function(){{ const target = {text}; \
             const els = Array.from(document.querySelectorAll({CLICKABLES})); \
             const el = els.find(e => (e.innerText || e.textContent || '').trim() === target); \
             {LOCATE_CENTER} }})()",
            text = serde_json::to_string(&p.text).expect("string serialise"),
        );
        self.click_located(&snippet).await
    }

    /// Évalue un snippet « localisateur » (contrat : renvoie `{{ok, x, y}}`
    /// ou `{{ok: false, reason}}`) puis clique au centre trouvé par vrais
    /// événements souris CDP — mêmes événements qu'un humain, focus compris.
    async fn click_located(&self, snippet: &str) -> Result<CallToolResult, McpError> {
        let value = self.evaluator.evaluate(snippet).await.map_err(internal)?;
        let found = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !found {
            let json = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }
        let x = value.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let y = value.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        self.interaction
            .dispatch(InteractionCommand::click(Position::new(x, y)))
            .await
            .map_err(internal)?;
        let json = serde_json::json!({ "ok": true, "x": x, "y": y }).to_string();
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Récupère le DOM courant de l'app cible (JSON sérialisé).")]
    async fn dom(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let tree = self.inspector.dom_snapshot().await.map_err(internal)?;
        let json = serde_json::to_string(&tree.tree).unwrap_or_else(|_| "null".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Récupère le state applicatif exposé par le SDK opt-in (JSON sérialisé).")]
    async fn state(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let dump = self.inspector.state_snapshot().await.map_err(internal)?;
        let json = serde_json::to_string(&dump.entries).unwrap_or_else(|_| "{}".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Draine et renvoie les événements `tracing` accumulés depuis le dernier appel."
    )]
    async fn logs_tail(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let events = self.log_source.poll().await.map_err(internal)?;
        let json = serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Invoque une commande logique exposée par l'app cible (route déclarée via deveye_sdk::expose_route). `name` = nom de la commande, `args` = payload JSON. Renvoie la valeur produite par le handler. Plus robuste qu'une cascade de clics pour mettre l'app dans un état donné."
    )]
    async fn invoke_command(
        &self,
        Parameters(p): Parameters<InvokeCommandParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let value = self
            .command_router
            .invoke(&p.name, p.args)
            .await
            .map_err(internal)?;
        let json = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Liste les commandes logiques exposées par l'app cible (découverte) : tableau JSON `[{name, description}]`, trié par nom. Vide si l'app n'a déclaré aucune route via expose_route."
    )]
    async fn list_commands(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let descriptors = self.command_router.list().await.map_err(internal)?;
        let json = serde_json::to_string(&descriptors).unwrap_or_else(|_| "[]".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[tool_handler]
impl ServerHandler for DeveyeMcpServer {
    fn get_info(&self) -> ServerInfo {
        // `ServerInfo` est `#[non_exhaustive]` : on part du default et on
        // mute les champs qui nous intéressent.
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "DevEye MCP — observe et pilote une app vivante en dev. \
             Outils : screenshot, click, right_click, hover, click_selector, \
             click_text, eval_js, type_text, dom, state, logs_tail, \
             invoke_command, list_commands."
                .to_string(),
        );
        info
    }
}

fn internal<E: std::fmt::Display>(err: E) -> McpError {
    McpError::internal_error(err.to_string(), None)
}

#[cfg(test)]
mod tests {
    use deveye_core::{InteractionCommand, MouseButton, Position};

    #[test]
    fn right_click_command_serializes_button_right() {
        // Contrat sur lequel s'appuie l'outil `right_click` : le bouton droit
        // se sérialise en `"right"`, ce qu'attend l'adapter d'interaction.
        let cmd = InteractionCommand::Click {
            position: Position::new(100, 200),
            button: MouseButton::Right,
        };
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v["button"], "right");
        assert_eq!(v["kind"], "click");
    }
}
