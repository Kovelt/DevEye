//! Adapter DevEye qui expose les ports `Renderer`, `AppInspector`,
//! `InteractionSink` et `LogSource` au format Model Context Protocol via
//! `rmcp` (transport stdio).
//!
//! Vingt outils sont publiés à Claude (ou à n'importe quel client MCP) :
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
//! - `wait_for_selector` : attend qu'un sélecteur CSS soit présent/visible
//!   (sans sleep arbitraire)
//! - `find_element` : inspecte un élément (tag, texte, rect, visible, disabled,
//!   attributs)
//! - `assert_selector` : compte les éléments d'un sélecteur (`{ ok, count }`)
//! - `drag_drop` : glisser-déposer de (from) vers (to)
//! - `collect_console_errors` : draine les erreurs JS de la page (onerror,
//!   unhandledrejection, console.error)
//! - `list_windows` : liste les fenêtres/onglets de la cible (CDP ; vide en SDK)
//! - `wait_condition` : attend un prédicat (dom-selector / state-key /
//!   screenshot-stable) avec timeout
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
use std::time::{Duration, Instant};

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
use serde::{Deserialize, Serialize};

use deveye_core::{InteractionCommand, MouseButton, Position, WaitPredicate};
use deveye_ports::{
    AppInspector, CommandRouter, ConsoleErrorSource, InteractionSink, JsEvaluator, LogSource,
    PortError, Renderer, WaitCondition, WindowLister,
};

/// Ports passes a [`DeveyeMcpServer::new`] — evite les 9 arguments positionnels
/// dont l'ordre fragile etait source d'erreurs silencieuses.
pub struct DeveyePorts {
    pub renderer: Arc<dyn Renderer>,
    pub interaction: Arc<dyn InteractionSink>,
    pub inspector: Arc<dyn AppInspector>,
    pub log_source: Arc<dyn LogSource>,
    pub evaluator: Arc<dyn JsEvaluator>,
    pub command_router: Arc<dyn CommandRouter>,
    pub window_lister: Arc<dyn WindowLister>,
    pub console_errors: Arc<dyn ConsoleErrorSource>,
    pub wait_condition: Arc<dyn WaitCondition>,
}

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
    window_lister: Arc<dyn WindowLister>,
    console_errors: Arc<dyn ConsoleErrorSource>,
    wait_condition: Arc<dyn WaitCondition>,
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

#[derive(Debug, Deserialize, JsonSchema)]
struct SelectorParams {
    /// Sélecteur CSS de l'élément (ex. `button.btn-primary`).
    selector: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WaitForSelectorParams {
    /// Sélecteur CSS à attendre (ex. `#save-btn:not([disabled])`).
    selector: String,
    /// Timeout en millisecondes (défaut 5000, plafond 30000).
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Si true, attend aussi que l'élément soit *visible* (rect non vide).
    #[serde(default)]
    visible: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DragDropParams {
    /// Abscisse de départ (pixels logiques viewport).
    from_x: i32,
    /// Ordonnée de départ.
    from_y: i32,
    /// Abscisse d'arrivée.
    to_x: i32,
    /// Ordonnée d'arrivée.
    to_y: i32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AssertSelectorParams {
    /// Sélecteur CSS à vérifier.
    selector: String,
    /// Texte attendu dans l'`innerText` du premier élément (optionnel).
    #[serde(default)]
    expected_text: Option<String>,
    /// Nombre d'éléments attendus (optionnel ; sinon vérifie juste la présence).
    #[serde(default)]
    expected_count: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WaitConditionParams {
    /// Prédicat à attendre, en JSON inline. Exemples :
    /// `{ "kind": "dom-selector", "selector": ".btn" }`,
    /// `{ "kind": "state-key", "key": "count", "expected": 3 }`,
    /// `{ "kind": "screenshot-stable", "threshold_ms": 300 }`.
    predicate: serde_json::Value,
    /// Timeout en millisecondes (défaut 5000, plafond 60000).
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[tool_router]
impl DeveyeMcpServer {
    pub fn new(ports: DeveyePorts) -> Self {
        Self {
            renderer: ports.renderer,
            interaction: ports.interaction,
            inspector: ports.inspector,
            log_source: ports.log_source,
            evaluator: ports.evaluator,
            command_router: ports.command_router,
            window_lister: ports.window_lister,
            console_errors: ports.console_errors,
            wait_condition: ports.wait_condition,
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
        Ok(json_success(&value))
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
            // H1 : element absent ou invisible — erreur semantique (isError=true)
            // pour ne pas masquer l'echec ni induire un retry trompeur.
            return Ok(json_error(&value));
        }
        let x = value.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let y = value.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        self.interaction
            .dispatch(InteractionCommand::click(Position::new(x, y)))
            .await
            .map_err(internal)?;
        Ok(json_success(
            &serde_json::json!({ "ok": true, "x": x, "y": y }),
        ))
    }

    #[tool(description = "Récupère le DOM courant de l'app cible (JSON sérialisé).")]
    async fn dom(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let tree = self.inspector.dom_snapshot().await.map_err(internal)?;
        Ok(json_success(&tree.tree))
    }

    #[tool(description = "Récupère le state applicatif exposé par le SDK opt-in (JSON sérialisé).")]
    async fn state(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let dump = self.inspector.state_snapshot().await.map_err(internal)?;
        Ok(json_success(&dump.entries))
    }

    #[tool(
        description = "Draine et renvoie les événements `tracing` accumulés depuis le dernier appel."
    )]
    async fn logs_tail(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let events = self.log_source.poll().await.map_err(internal)?;
        Ok(json_success(&events))
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
        Ok(json_success(&value))
    }

    #[tool(
        description = "Liste les commandes logiques exposées par l'app cible (découverte) : tableau JSON `[{name, description}]`, trié par nom. Vide si l'app n'a déclaré aucune route via expose_route."
    )]
    async fn list_commands(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let descriptors = self.command_router.list().await.map_err(internal)?;
        Ok(json_success(&descriptors))
    }

    #[tool(
        description = "Attend qu'un sélecteur CSS soit présent (et optionnellement visible) dans le DOM. Renvoie `{ found, elapsed_ms }`. Évite tout sleep arbitraire après une action asynchrone. Défaut 5000 ms, plafond 30000."
    )]
    async fn wait_for_selector(
        &self,
        Parameters(p): Parameters<WaitForSelectorParams>,
    ) -> Result<CallToolResult, McpError> {
        // M1 : le gate est locke PAR ITERATION (evaluate atomique), pas pour
        // toute la boucle. Le sleep de 100 ms s'ecoule gate relache, ce qui
        // laisse les autres outils MCP s'executer pendant le polling.
        let budget = Duration::from_millis(p.timeout_ms.unwrap_or(5_000).min(30_000));
        let deadline = Instant::now() + budget;
        let start = Instant::now();
        let sel = serde_json::to_string(&p.selector).unwrap_or_default();
        let snippet = if p.visible.unwrap_or(false) {
            format!(
                "(function(){{ const el = document.querySelector({sel}); if (!el) return false; \
                 const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0; }})()"
            )
        } else {
            format!("!!document.querySelector({sel})")
        };
        loop {
            let eval_result = {
                let _serial = self.gate.lock().await;
                self.evaluator.evaluate(&snippet).await
            };
            let present = match eval_result {
                // M4 : transport mort → sortie immediate en erreur MCP, pas 30 s
                // de silence puis {found:false} trompeur.
                Err(e @ PortError::Network(_)) => return Err(internal(e)),
                // Erreur JS transitoire (selecteur absent, exception benigne) :
                // le selecteur n'existe pas encore, continuer le polling.
                Err(_) => false,
                Ok(v) => v.as_bool().unwrap_or(false),
            };
            if present {
                return Ok(json_success(&serde_json::json!({
                    "found": true,
                    "elapsed_ms": start.elapsed().as_millis() as u64
                })));
            }
            if Instant::now() >= deadline {
                // L13 : timeout = echec semantique → isError=true (coherent avec
                // wait_condition).
                return Ok(json_error(&serde_json::json!({
                    "found": false,
                    "elapsed_ms": start.elapsed().as_millis() as u64
                })));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[tool(
        description = "Inspecte le premier élément matchant un sélecteur CSS : `{ found, tag, text, rect{x,y,width,height}, visible, disabled, attributes }`. Réponse compacte au lieu de tout le DOM."
    )]
    async fn find_element(
        &self,
        Parameters(p): Parameters<SelectorParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let sel = serde_json::to_string(&p.selector).unwrap_or_default();
        let snippet = format!(
            "(function(){{ const el = document.querySelector({sel}); \
             if (!el) return {{ found: false }}; \
             const r = el.getBoundingClientRect(); const attrs = {{}}; \
             for (const a of el.attributes) {{ attrs[a.name] = a.value; }} \
             return {{ found: true, tag: el.tagName.toLowerCase(), \
               text: (el.innerText || el.textContent || '').trim().slice(0, 200), \
               rect: {{ x: r.left, y: r.top, width: r.width, height: r.height }}, \
               visible: r.width > 0 && r.height > 0, \
               disabled: el.disabled === true || el.hasAttribute('disabled'), \
               attributes: attrs }}; }})()"
        );
        let value = self.evaluator.evaluate(&snippet).await.map_err(internal)?;
        Ok(json_success(&value))
    }

    #[tool(
        description = "Assertion sur un sélecteur CSS : présence, et optionnellement texte (`expected_text`) et/ou compte exact (`expected_count`). Renvoie `{ ok, selector, actual_count, actual_text? }`. `isError` si l'assertion échoue."
    )]
    async fn assert_selector(
        &self,
        Parameters(p): Parameters<AssertSelectorParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let sel = serde_json::to_string(&p.selector).unwrap_or_default();
        let snippet = format!(
            "(function(){{ const nodes = document.querySelectorAll({sel}); \
             const count = nodes.length; \
             const text = count > 0 ? (nodes[0].innerText || nodes[0].textContent || '').trim() : null; \
             return {{ count, text }}; }})()"
        );
        let value = self.evaluator.evaluate(&snippet).await.map_err(internal)?;
        let actual_count = value.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let actual_text = value
            .get("text")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let mut ok = actual_count >= 1;
        if let Some(expected) = &p.expected_text {
            ok = ok && actual_text.as_deref() == Some(expected.as_str());
        }
        if let Some(expected) = p.expected_count {
            ok = ok && actual_count == expected;
        }
        let payload = serde_json::json!({
            "ok": ok,
            "selector": p.selector,
            "actual_count": actual_count,
            "actual_text": actual_text,
        });
        if ok {
            Ok(json_success(&payload))
        } else {
            Ok(json_error(&payload))
        }
    }

    #[tool(
        description = "Attend qu'un prédicat soit satisfait, ou que le timeout expire. Renvoie `{ satisfied, elapsed_ms, detail }` ; `isError` si non satisfait. Prédicats : dom-selector (présence CSS), state-key (valeur dans le state SDK), screenshot-stable (stabilité visuelle pendant threshold_ms). Évite tout sleep arbitraire."
    )]
    async fn wait_condition(
        &self,
        Parameters(p): Parameters<WaitConditionParams>,
    ) -> Result<CallToolResult, McpError> {
        const DEFAULT_MS: u64 = 5_000;
        const MAX_MS: u64 = 60_000;
        // M1 : le gate n'est PAS tenu pendant wait_until (boucle de polling
        // jusqu'a MAX_MS). wait_until est purement observationnel (aucune
        // interaction UI) — le tenir bloquerait tous les autres outils MCP
        // pendant jusqu'a 60 s.
        let predicate: WaitPredicate = serde_json::from_value(p.predicate).map_err(internal)?;
        let timeout = Duration::from_millis(p.timeout_ms.unwrap_or(DEFAULT_MS).min(MAX_MS));
        let outcome = self
            .wait_condition
            .wait_until(predicate, timeout)
            .await
            .map_err(internal)?;
        if outcome.satisfied {
            Ok(json_success(&outcome))
        } else {
            Ok(json_error(&outcome))
        }
    }

    #[tool(
        description = "Glisse-dépose de (from_x, from_y) vers (to_x, to_y). En CDP : séquence souris native (press/déplacements/release) ; via le pont SDK : PointerEvent JS (support selon le framework de l'app). Renvoie \"ok\"."
    )]
    async fn drag_drop(
        &self,
        Parameters(p): Parameters<DragDropParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let cmd = InteractionCommand::drag_drop(
            Position::new(p.from_x, p.from_y),
            Position::new(p.to_x, p.to_y),
        );
        self.interaction.dispatch(cmd).await.map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Draine les erreurs console émises par le webview depuis le dernier appel (exceptions non capturées, promesses rejetées, console.error). Renvoie un tableau typé `[{ level, text, url?, line?, timestamp }]` (port ConsoleErrorSource). Un crash JS de la page devient visible à l'agent."
    )]
    async fn collect_console_errors(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let errors = self
            .console_errors
            .drain_console_errors()
            .await
            .map_err(internal)?;
        Ok(json_success(&errors))
    }

    #[tool(
        description = "Liste les fenêtres/onglets de la cible : `[{ id, title, url, kind }]` (type=page). En CDP via Target.getTargets ; via le pont SDK : liste vide (pas de visibilité inter-fenêtres)."
    )]
    async fn list_windows(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let windows = self.window_lister.list_windows().await.map_err(internal)?;
        Ok(json_success(&windows))
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
             invoke_command, list_commands, wait_for_selector, find_element, \
             assert_selector, drag_drop, collect_console_errors, list_windows, \
             wait_condition."
                .to_string(),
        );
        info
    }
}

/// Serialise `value` en texte JSON et emballe dans un [`CallToolResult`] succes.
/// La serialisation ne peut echouer sur les types utilises ici (tous implementent
/// `Serialize` correctement) ; le fallback `"null"` est purement defensif. N14.
fn json_success<T: Serialize>(value: &T) -> CallToolResult {
    CallToolResult::success(vec![Content::text(
        serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
    )])
}

/// Idem mais renvoie un [`CallToolResult`] erreur (`isError = true`).
/// Utilise pour les echecs semantiques (element absent H1, timeout L13) qui ne
/// doivent pas etre confondus avec un succes par le client MCP.
fn json_error<T: Serialize>(value: &T) -> CallToolResult {
    CallToolResult::error(vec![Content::text(
        serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
    )])
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
