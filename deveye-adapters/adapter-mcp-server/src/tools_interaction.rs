//! Outils de la famille **interaction** : `click`, `right_click`,
//! `double_click`, `hover`, `type_text`, `press_key`, `key_combo`,
//! `navigate`, `drag_drop`, `scroll`, `handle_dialog`, `upload_file`,
//! `click_selector`, `click_text` — les « mains » de DevEye. Scinde de
//! `lib.rs` (H1) : la logique et les schemas d'outils sont inchanges, seul
//! l'emplacement du code bouge.

use super::*;

#[tool_router(router = interaction_router, vis = "pub(crate)")]
impl DeveyeMcpServer {
    #[tool(
        description = "Effectue un clic sur le viewport de l'app cible aux coordonnées absolues fournies. `button` optionnel (E6) : \"left\" (défaut), \"right\", \"middle\". Les coordonnées sont validées avant le dispatch : négatives, ou hors du viewport quand sa taille est connue, elles donnent une erreur `invalid_params` (même garde pour right_click, double_click, hover, drag_drop et l'ancrage de scroll)."
    )]
    pub(crate) async fn click(
        &self,
        Parameters(p): Parameters<ClickParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.acquire_gate("click").await;
        let button = parse_mouse_button(p.button.as_deref())?;
        self.ensure_in_viewport(&[(p.x, p.y)]).await?;
        let cmd = InteractionCommand::Click {
            position: Position::new(p.x, p.y),
            button,
        };
        ok_or_unsupported(self.interaction.dispatch(cmd).await)
    }

    #[tool(
        description = "Effectue un clic DROIT (menu contextuel) sur le viewport de l'app cible aux coordonnées absolues fournies. Déclenche button=right (contextmenu) — ouvre les menus contextuels des UI qui les écoutent. Conservé pour compatibilité ; équivalent à click(button: \"right\")."
    )]
    pub(crate) async fn right_click(
        &self,
        Parameters(p): Parameters<PositionParams>,
    ) -> Result<CallToolResult, McpError> {
        let cmd = InteractionCommand::Click {
            position: Position::new(p.x, p.y),
            button: MouseButton::Right,
        };
        self.dispatch_at_ok(cmd, &[(p.x, p.y)]).await
    }

    #[tool(
        description = "Double-clic gauche aux coordonnées (x, y) : édition inline, sélection de mot, ouverture d'item de liste. Émet un vrai `dblclick` (clickCount=2 en CDP). Plus fiable que deux `click` MCP successifs (intervalle imprévisible)."
    )]
    pub(crate) async fn double_click(
        &self,
        Parameters(p): Parameters<PositionParams>,
    ) -> Result<CallToolResult, McpError> {
        self.dispatch_at_ok(
            InteractionCommand::double_click(Position::new(p.x, p.y)),
            &[(p.x, p.y)],
        )
        .await
    }

    #[tool(description = "Saisit du texte au focus courant de l'app cible.")]
    pub(crate) async fn type_text(
        &self,
        Parameters(p): Parameters<TypeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.dispatch_ok(InteractionCommand::type_text(p.text))
            .await
    }

    #[tool(
        description = "Presse une touche unique (keydown + keyup) au focus courant : Enter, Tab, Escape, ArrowDown, F5… Marche sur toutes les cibles, y compris kwin-eis (où eval_js est indisponible). Noms W3C en CDP et sdk (Enter, ArrowDown) ; noms XKB en kwin-eis (Return, Down) ; les noms communs (Tab, Escape, F5) passent partout (E7)."
    )]
    pub(crate) async fn press_key(
        &self,
        Parameters(p): Parameters<PressKeyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.dispatch_ok(InteractionCommand::key(p.key)).await
    }

    #[tool(
        description = "Combinaison de touches : maintient les modificateurs (ctrl/shift/alt/meta) pendant la frappe de `key`, puis les relâche. Ctrl+S, Ctrl+Z, Ctrl+Shift+P, Alt+F4… Marche sur toutes les cibles, y compris kwin-eis (events OS-level : les raccourcis du toolkit se déclenchent). `press_key` seul ne porte aucun modificateur. Noms de touche W3C en CDP et sdk, XKB en kwin-eis (E7, cf. KeyComboParams)."
    )]
    pub(crate) async fn key_combo(
        &self,
        Parameters(p): Parameters<KeyComboParams>,
    ) -> Result<CallToolResult, McpError> {
        self.dispatch_ok(InteractionCommand::key_combo(p.modifiers, p.key))
            .await
    }

    #[tool(
        description = "Navigue vers une route logique exposée par l'app cible (routeur du SDK opt-in). Interface découvrable, à préférer à un eval_js sur le hook interne `window.__deveye_router__`. Indisponible en kwin-eis (pas de routeur applicatif OS-level) : renvoie un résultat isError structuré `{ reason: \"unsupported\" }` plutôt qu'une erreur protocolaire (E1)."
    )]
    pub(crate) async fn navigate(
        &self,
        Parameters(p): Parameters<NavigateParams>,
    ) -> Result<CallToolResult, McpError> {
        self.dispatch_ok(InteractionCommand::navigate(p.route))
            .await
    }

    #[tool(
        description = "Déplace le pointeur à (x, y) sans presser — déclenche mouseover/mouseenter (utile pour révéler des éléments au hover)."
    )]
    pub(crate) async fn hover(
        &self,
        Parameters(p): Parameters<PositionParams>,
    ) -> Result<CallToolResult, McpError> {
        self.dispatch_at_ok(
            InteractionCommand::hover(Position::new(p.x, p.y)),
            &[(p.x, p.y)],
        )
        .await
    }

    #[tool(
        description = "Clique sur le premier élément matchant un sélecteur CSS, par vrais événements souris CDP (hover + press au centre de l'élément — le focus clavier suit). Robuste aux déplacements de coordonnées et aux animations."
    )]
    pub(crate) async fn click_selector(
        &self,
        Parameters(p): Parameters<ClickSelectorParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let snippet = format!(
            "(function(){{ const el = document.querySelector({selector}); {LOCATE_CENTER} }})()",
            selector = js_string(&p.selector),
        );
        self.click_located(&snippet).await
    }

    #[tool(
        description = "Clique sur le premier élément interactif (button, a, label, summary, role=button/tab/menuitem/option/switch/checkbox) dont le texte (après trim) correspond exactement — par vrais événements souris CDP (le focus clavier suit)."
    )]
    pub(crate) async fn click_text(
        &self,
        Parameters(p): Parameters<ClickTextParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let snippet = format!(
            "(function(){{ const target = {text}; \
             const els = Array.from(document.querySelectorAll({CLICKABLES})); \
             const el = els.find(e => (e.innerText || e.textContent || '').trim() === target); \
             {LOCATE_CENTER} }})()",
            text = js_string(&p.text),
        );
        self.click_located(&snippet).await
    }

    /// Évalue un snippet « localisateur » (contrat : renvoie `{{ok, x, y}}`
    /// ou `{{ok: false, reason}}`) puis clique au centre trouvé par vrais
    /// événements souris CDP — mêmes événements qu'un humain, focus compris.
    async fn click_located(&self, snippet: &str) -> Result<CallToolResult, McpError> {
        let value = match unsupported_aware(self.evaluator.evaluate(snippet).await) {
            Ok(v) => v,
            Err(issue) => return issue,
        };
        let found = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !found {
            // H1 : element absent ou invisible — erreur semantique (isError=true)
            // pour ne pas masquer l'echec ni induire un retry trompeur.
            return Ok(json_error(&value));
        }
        let x = value.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let y = value.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        match unsupported_aware(
            self.interaction
                .dispatch(InteractionCommand::click(Position::new(x, y)))
                .await,
        ) {
            Ok(()) => Ok(json_success(
                &serde_json::json!({ "ok": true, "x": x, "y": y }),
            )),
            Err(issue) => issue,
        }
    }

    #[tool(
        description = "Glisse-dépose de (from_x, from_y) vers (to_x, to_y). En CDP : séquence souris native (press/déplacements/release) ; via le pont SDK : PointerEvent JS (support selon le framework de l'app)."
    )]
    pub(crate) async fn drag_drop(
        &self,
        Parameters(p): Parameters<DragDropParams>,
    ) -> Result<CallToolResult, McpError> {
        let cmd = InteractionCommand::drag_drop(
            Position::new(p.from_x, p.from_y),
            Position::new(p.to_x, p.to_y),
        );
        self.dispatch_at_ok(cmd, &[(p.from_x, p.from_y), (p.to_x, p.to_y)])
            .await
    }

    #[tool(
        description = "Répond à une boîte de dialogue JS (`alert`/`confirm`/`prompt`) : `accept` (OK/Cancel), `prompt_text` = texte pour un `prompt`. CDP-only — pendant un dialog le moteur JS est suspendu, donc `eval_js` est impossible : c'est la seule issue. En sdk/kwin-eis, résultat isError structuré `{ reason: \"unsupported\" }` plutôt qu'une erreur protocolaire (E1)."
    )]
    pub(crate) async fn handle_dialog(
        &self,
        Parameters(p): Parameters<HandleDialogParams>,
    ) -> Result<CallToolResult, McpError> {
        self.dispatch_ok(InteractionCommand::handle_dialog(p.accept, p.prompt_text))
            .await
    }

    #[tool(
        description = "Renseigne les fichiers d'un `input[type=file]` (sélecteur CSS + chemins absolus). CDP-only (`DOM.setFileInputFiles`) : la sécurité navigateur interdit de le faire en JS, donc non contournable par eval_js. En sdk/kwin-eis, résultat isError structuré `{ reason: \"unsupported\" }` plutôt qu'une erreur protocolaire (E1)."
    )]
    pub(crate) async fn upload_file(
        &self,
        Parameters(p): Parameters<UploadFileParams>,
    ) -> Result<CallToolResult, McpError> {
        self.dispatch_ok(InteractionCommand::upload_file(p.selector, p.paths))
            .await
    }

    #[tool(
        description = "Redimensionne le viewport à `width` x `height` pixels CSS (1 à 16384), pour tester un rendu responsive. ÉMULATION du moteur de rendu (`Emulation.setDeviceMetricsOverride`) : la fenêtre de l'OS garde sa taille, le facteur d'échelle de l'écran est conservé, et l'émulation persiste jusqu'à un nouveau `resize` ; `width` et `height` à 0 ensemble retirent l'émulation. Les captures et `click`/`hover` suivent le nouveau viewport. CDP-only : sur les autres cibles (sdk, kwin-eis, x11), résultat isError structuré `{ reason: \"unsupported\" }` (E1)."
    )]
    pub(crate) async fn resize(
        &self,
        Parameters(p): Parameters<ResizeParams>,
    ) -> Result<CallToolResult, McpError> {
        validate_resize(p.width, p.height)?;
        self.dispatch_ok(InteractionCommand::resize(p.width, p.height))
            .await
    }

    #[tool(
        description = "Défile de (dx, dy) pixels — `dy > 0` vers le bas. Ancré en (x, y) ou, par défaut, au centre du viewport. En CDP, l'ancrage cible le bon conteneur scrollable (mouseWheel) ; via le pont SDK, le conteneur scrollable sous le point est privilégié (sinon window) ; en kwin-eis, le défilement va à la fenêtre focus. Indispensable pour les listes/grilles en kwin-eis (eval_js indisponible)."
    )]
    pub(crate) async fn scroll(
        &self,
        Parameters(p): Parameters<ScrollParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let dx = p.dx.unwrap_or(0);
        let dy = p.dy.unwrap_or(0);
        // Ancrage : centre du viewport si x ou y manquant. Resolu via le
        // renderer ; en cas d'echec (cible sans dimensions exploitables), on
        // retombe sur (0, 0) — sans incidence en kwin-eis (ancrage ignore).
        // Le gate reste tenu pendant cette resolution (contrairement a
        // `dispatch_ok`) : garder le meme perimetre de serialisation qu'avant
        // ce sprint plutot que de le retrecir a la seule ligne de dispatch.
        if let (Some(x), Some(y)) = (p.x, p.y) {
            self.ensure_in_viewport(&[(x, y)]).await?;
        }
        let (cx, cy) = if p.x.is_none() || p.y.is_none() {
            match self.renderer.viewport_size().await {
                Ok(size) => (size.width as i32 / 2, size.height as i32 / 2),
                Err(_) => (0, 0),
            }
        } else {
            (0, 0)
        };
        let anchor = Position::new(p.x.unwrap_or(cx), p.y.unwrap_or(cy));
        ok_or_unsupported(
            self.interaction
                .dispatch(InteractionCommand::scroll(anchor, dx, dy))
                .await,
        )
    }
}
