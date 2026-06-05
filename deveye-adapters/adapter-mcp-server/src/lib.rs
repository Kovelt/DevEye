//! Adapter DevEye qui expose les ports `Renderer`, `AppInspector`,
//! `InteractionSink` et `LogSource` au format Model Context Protocol via
//! `rmcp` (transport stdio).
//!
//! Dix outils sont publiés à Claude (ou à n'importe quel client MCP) :
//! - `screenshot` : capture le viewport en PNG, renvoie le base64
//! - `click` : clic à `(x, y)` (bouton gauche)
//! - `hover` : déplace le pointeur à `(x, y)` sans presser (déclenche
//!   `mouseover`/`mouseenter`)
//! - `click_selector` : clique sur le premier élément matchant un sélecteur CSS
//! - `click_text` : clique sur le premier élément cliquable dont le texte
//!   correspond exactement
//! - `eval_js` : évalue une expression JS dans la page cible, retourne la
//!   valeur sérialisée
//! - `type_text` : saisit du texte au focus courant
//! - `dom` : sérialise le DOM de l'app cible
//! - `state` : sérialise le state opt-in poussé par le SDK
//! - `logs_tail` : draine et renvoie le buffer de logs accumulé
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

use deveye_core::{InteractionCommand, Position};
use deveye_ports::{AppInspector, InteractionSink, JsEvaluator, LogSource, Renderer};

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

#[tool_router]
impl DeveyeMcpServer {
    pub fn new(
        renderer: Arc<dyn Renderer>,
        interaction: Arc<dyn InteractionSink>,
        inspector: Arc<dyn AppInspector>,
        log_source: Arc<dyn LogSource>,
        evaluator: Arc<dyn JsEvaluator>,
    ) -> Self {
        Self {
            renderer,
            interaction,
            inspector,
            log_source,
            evaluator,
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
        let cmd = InteractionCommand::click(Position::new(p.x, p.y));
        self.interaction.dispatch(cmd).await.map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(description = "Saisit du texte au focus courant de l'app cible.")]
    async fn type_text(
        &self,
        Parameters(p): Parameters<TypeParams>,
    ) -> Result<CallToolResult, McpError> {
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
        let value = self
            .evaluator
            .evaluate(&p.expression)
            .await
            .map_err(internal)?;
        let json = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Clique sur le premier élément matchant un sélecteur CSS. Robuste aux déplacements de coordonnées et aux animations."
    )]
    async fn click_selector(
        &self,
        Parameters(p): Parameters<ClickSelectorParams>,
    ) -> Result<CallToolResult, McpError> {
        let snippet = format!(
            "(function(){{ const el = document.querySelector({selector}); \
             if (!el) return {{ ok: false, reason: 'not_found' }}; \
             el.scrollIntoView({{ block: 'center', behavior: 'instant' }}); \
             el.click(); \
             return {{ ok: true }}; }})()",
            selector = serde_json::to_string(&p.selector).expect("string serialise"),
        );
        let value = self.evaluator.evaluate(&snippet).await.map_err(internal)?;
        let json = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Clique sur le premier élément cliquable (button, a, [role=button]) dont le texte (après trim) correspond exactement."
    )]
    async fn click_text(
        &self,
        Parameters(p): Parameters<ClickTextParams>,
    ) -> Result<CallToolResult, McpError> {
        let snippet = format!(
            "(function(){{ const target = {text}; \
             const els = Array.from(document.querySelectorAll('button, a, [role=button], [data-deveye-click]')); \
             const m = els.find(e => (e.innerText || e.textContent || '').trim() === target); \
             if (!m) return {{ ok: false, reason: 'not_found' }}; \
             m.scrollIntoView({{ block: 'center', behavior: 'instant' }}); \
             m.click(); \
             return {{ ok: true }}; }})()",
            text = serde_json::to_string(&p.text).expect("string serialise"),
        );
        let value = self.evaluator.evaluate(&snippet).await.map_err(internal)?;
        let json = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Récupère le DOM courant de l'app cible (JSON sérialisé).")]
    async fn dom(&self) -> Result<CallToolResult, McpError> {
        let tree = self.inspector.dom_snapshot().await.map_err(internal)?;
        let json = serde_json::to_string(&tree.tree).unwrap_or_else(|_| "null".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Récupère le state applicatif exposé par le SDK opt-in (JSON sérialisé).")]
    async fn state(&self) -> Result<CallToolResult, McpError> {
        let dump = self.inspector.state_snapshot().await.map_err(internal)?;
        let json = serde_json::to_string(&dump.entries).unwrap_or_else(|_| "{}".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Draine et renvoie les événements `tracing` accumulés depuis le dernier appel."
    )]
    async fn logs_tail(&self) -> Result<CallToolResult, McpError> {
        let events = self.log_source.poll().await.map_err(internal)?;
        let json = serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_string());
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
             Outils : screenshot, click, hover, click_selector, click_text, \
             eval_js, type_text, dom, state, logs_tail."
                .to_string(),
        );
        info
    }
}

fn internal<E: std::fmt::Display>(err: E) -> McpError {
    McpError::internal_error(err.to_string(), None)
}
