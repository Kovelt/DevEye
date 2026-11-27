//! Outils de la famille **inspect** : `eval_js`, `dom`, `state`,
//! `assert_state`, `find_element`, `assert_selector`, `logs_tail`, `logs_wait`,
//! `collect_console_errors`, `invoke_command`, `list_commands`,
//! `target_info`, `list_windows`, `window_geometry`, `focus_window`,
//! `clipboard_get`, `clipboard_set` — inspection de la cible (DOM, state,
//! logs, commandes, fenetres, presse-papiers). Scinde de `lib.rs` (H1) : la
//! logique et les schemas d'outils sont inchanges, seul l'emplacement du
//! code bouge.

use super::*;

#[tool_router(router = inspect_router, vis = "pub(crate)")]
impl DeveyeMcpServer {
    #[tool(
        description = "Évalue une expression JavaScript dans la page cible et renvoie la valeur JSON. Promises supportées (awaitPromise=true). Une exception levée par l'expression revient comme résultat d'outil en erreur `{ ok: false, reason: \"js_exception\", message, stack? }` (la trace n'est présente que si le moteur la fournit) : lisible et corrigeable par l'agent, distincte d'une panne de liaison (erreur protocolaire). Indisponible en kwin-eis (pas de WebView) : renvoie un résultat isError structuré `{ reason: \"unsupported\" }` plutôt qu'une erreur protocolaire (E1)."
    )]
    pub(crate) async fn eval_js(
        &self,
        Parameters(p): Parameters<EvalParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.acquire_gate("eval_js").await;
        let value = match unsupported_aware(self.evaluator.evaluate(&p.expression).await) {
            Ok(v) => v,
            Err(issue) => return issue,
        };
        Ok(json_success(&value))
    }

    #[tool(
        description = "Récupère le DOM courant de l'app cible (JSON sérialisé). Garde-fou (B1) : si la sérialisation dépasse ~100 Ko, renvoie une erreur MCP explicite (`isError`) au lieu de saturer le contexte de l'agent — préférer find_element/assert_selector/eval_js pour cibler un élément précis. Indisponible en kwin-eis (pas de WebView) : résultat isError structuré `{ reason: \"unsupported\" }` (E1)."
    )]
    pub(crate) async fn dom(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let tree = match unsupported_aware(self.inspector.dom_snapshot().await) {
            Ok(v) => v,
            Err(issue) => return issue,
        };
        let serialized = serde_json::to_string(&tree.tree).unwrap_or_else(|_| "null".to_string());
        if let Some(payload) = dom_size_guard(serialized.len()) {
            return Ok(json_error(&payload));
        }
        // Le JSON est deja serialise ci-dessus (pour mesurer sa taille) : on
        // le reutilise tel quel plutot que de rappeler json_success (qui
        // resserialiserait tree.tree en double).
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serialized,
        )]))
    }

    #[tool(
        description = "Récupère le state applicatif exposé par le SDK opt-in (JSON sérialisé). `keys` optionnel restreint la réponse à ce sous-ensemble de clés (les clés absentes du state sont silencieusement omises) — réduit le coût en tokens quand seules quelques clés intéressent l'agent ; absent ou vide, renvoie tout le state (comportement inchangé). Si la cible ne peut pas exposer de state, résultat isError structuré `{ reason: \"unsupported\" }` (E1)."
    )]
    pub(crate) async fn state(
        &self,
        Parameters(p): Parameters<StateParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let dump = match unsupported_aware(self.inspector.state_snapshot().await) {
            Ok(v) => v,
            Err(issue) => return issue,
        };
        let filtered = filter_state_entries(&dump.entries, p.keys.as_deref());
        Ok(json_success(&filtered))
    }

    #[tool(
        description = "Assertion sur le state SDK : `expected` est un objet `{ clé: valeur }` ; chaque clé doit exister dans le state avec exactement cette valeur. Renvoie `{ ok, mismatches: [{ key, expected, actual }] }` ; `isError` si au moins une diverge. Symétrique de assert_selector, mais sur la logique métier. Si la cible ne peut pas exposer de state (pas de SDK), résultat isError structuré `{ reason: \"unsupported\" }` (E1) — distinct d'une divergence d'assertion."
    )]
    pub(crate) async fn assert_state(
        &self,
        Parameters(p): Parameters<AssertStateParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let expected = match p.expected.as_object() {
            Some(map) => map,
            None => {
                return Ok(json_error(&serde_json::json!({
                    "ok": false,
                    "reason": "`expected` doit etre un objet JSON { cle: valeur }",
                })));
            }
        };
        let dump = match unsupported_aware(self.inspector.state_snapshot().await) {
            Ok(v) => v,
            Err(issue) => return issue,
        };
        let mut mismatches = Vec::new();
        for (key, want) in expected {
            let actual = dump.entries.get(key);
            if actual != Some(want) {
                mismatches.push(serde_json::json!({
                    "key": key,
                    "expected": want,
                    "actual": actual,
                }));
            }
        }
        let ok = mismatches.is_empty();
        let payload = serde_json::json!({ "ok": ok, "mismatches": mismatches });
        if ok {
            Ok(json_success(&payload))
        } else {
            Ok(json_error(&payload))
        }
    }

    #[tool(
        description = "Inspecte le premier élément matchant un sélecteur CSS : `{ found, tag, text, rect{x,y,width,height}, visible, disabled, attributes, value_prop }`. `value_prop` porte la propriété `.value` de l'élément (inputs/select/textarea) — `null` si l'élément n'en a pas (ex. lire une saisie, distinct de `text` qui reste l'`innerText`). Réponse compacte au lieu de tout le DOM."
    )]
    pub(crate) async fn find_element(
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
               attributes: attrs, \
               value_prop: ('value' in el) ? el.value : null }}; }})()"
        );
        let value = match unsupported_aware(self.evaluator.evaluate(&snippet).await) {
            Ok(v) => v,
            Err(issue) => return issue,
        };
        Ok(json_success(&value))
    }

    #[tool(
        description = "Assertion sur un sélecteur CSS : présence, et optionnellement texte (`expected_text`) et/ou compte exact (`expected_count`). Renvoie `{ ok, selector, actual_count, actual_text? }`. `isError` si l'assertion échoue."
    )]
    pub(crate) async fn assert_selector(
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
        let value = match unsupported_aware(self.evaluator.evaluate(&snippet).await) {
            Ok(v) => v,
            Err(issue) => return issue,
        };
        let actual_count = value.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let actual_text = value
            .get("text")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let text_ok = match &p.expected_text {
            Some(expected) => actual_text.as_deref() == Some(expected.as_str()),
            None => true,
        };
        let ok = selector_assertion_ok(actual_count, p.expected_count, text_ok);
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
        description = "Draine et renvoie les événements `tracing` accumulés depuis le dernier appel, sous la forme `{ events, truncated, total }`. `limit` (défaut 200) borne le nombre d'événements présents dans `events` aux plus RÉCENTS ; le drain côté source reste complet (rien n'est laissé dans le buffer ni perdu silencieusement) — `total` indique le nombre réellement drainé et `truncated` si `total > limit` (des événements plus anciens ont alors été omis de la réponse, mais comptés)."
    )]
    pub(crate) async fn logs_tail(
        &self,
        Parameters(p): Parameters<LogsTailParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let events = self.log_source.poll().await.map_err(port_to_mcp)?;
        let limit = p.limit.unwrap_or(LOGS_TAIL_DEFAULT_LIMIT);
        let (events, truncated, total) = truncate_recent(events, limit);
        Ok(json_success(&serde_json::json!({
            "events": events,
            "truncated": truncated,
            "total": total,
        })))
    }

    #[tool(
        description = "Long-poll des logs `tracing` : attend qu'un log arrive (ou, avec `level_min`, qu'un log de niveau >= `level_min` arrive), jusqu'à `timeout_ms` (défaut 5000, plafond 30000). Renvoie `{ events, timed_out, truncated, total }` : `events` contient TOUS les logs drainés pendant l'attente (aucun n'est perdu, y compris ceux sous `level_min`), `timed_out` vaut true si rien n'a satisfait le filtre. Contrairement à `wait_for_log` (qui consomme les logs qu'il ignore et ne les renvoie pas), `logs_wait` les rend à l'agent. `level_min` invalide : erreur `invalid_params`. Idéal juste après une action : « clique X, puis attends une erreur »."
    )]
    pub(crate) async fn logs_wait(
        &self,
        Parameters(p): Parameters<LogsWaitParams>,
    ) -> Result<CallToolResult, McpError> {
        let level_min = p.level_min.as_deref().map(parse_log_level).transpose()?;
        let budget = Duration::from_millis(
            p.timeout_ms
                .unwrap_or(WAIT_DEFAULT_MS)
                .min(LOGS_WAIT_MAX_MS),
        );
        let deadline = Instant::now() + budget;

        let mut collected = Vec::new();
        let timed_out = loop {
            // Gate tenu le temps d'un seul sondage, relâché pendant la pause :
            // un long-poll ne doit pas bloquer les autres outils (cf. M1).
            let batch = {
                let _serial = self.gate.lock().await;
                self.log_source.poll().await.map_err(port_to_mcp)?
            };
            let matched = match level_min {
                Some(level) => batch.iter().any(|e| e.level >= level),
                None => !batch.is_empty(),
            };
            collected.extend(batch);
            if matched {
                break false;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break true;
            }
            tokio::time::sleep(remaining.min(Duration::from_millis(LOGS_WAIT_POLL_MS))).await;
        };

        let limit = p.limit.unwrap_or(LOGS_TAIL_DEFAULT_LIMIT);
        let (events, truncated, total) = truncate_recent(collected, limit);
        Ok(json_success(&serde_json::json!({
            "events": events,
            "timed_out": timed_out,
            "truncated": truncated,
            "total": total,
        })))
    }

    #[tool(
        description = "Draine les messages console émis par le webview depuis le dernier appel — tous niveaux confondus (exceptions non capturées, promesses rejetées, console.log/info/warn/error). Renvoie un tableau typé `[{ level, text, url?, line?, timestamp }]` (port ConsoleErrorSource). `levels` filtre optionnellement par sévérité (ex. `[\"error\", \"warning\"]`) ; absent ou vide renvoie tout. Un crash JS de la page devient visible à l'agent."
    )]
    pub(crate) async fn collect_console_errors(
        &self,
        Parameters(p): Parameters<CollectConsoleErrorsParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let mut errors = self
            .console_errors
            .drain_console_errors()
            .await
            .map_err(port_to_mcp)?;
        if let Some(levels) = &p.levels {
            if !levels.is_empty() {
                let voulus: Vec<String> = levels.iter().map(|l| l.to_lowercase()).collect();
                // Compare via la representation serde (deja "lowercase" par
                // construction de ConsoleLevel) plutot qu'un match manuel des
                // variantes : reste correct si l'enum gagne des variantes.
                errors.retain(|e| {
                    serde_json::to_value(e.level)
                        .ok()
                        .and_then(|v| v.as_str().map(|s| voulus.contains(&s.to_string())))
                        .unwrap_or(false)
                });
            }
        }
        Ok(json_success(&errors))
    }

    #[tool(
        description = "Invoque une commande logique exposée par l'app cible (route déclarée via deveye_sdk::expose_route). `name` = nom de la commande, `args` = payload JSON. Renvoie la valeur produite par le handler. Plus robuste qu'une cascade de clics pour mettre l'app dans un état donné."
    )]
    pub(crate) async fn invoke_command(
        &self,
        Parameters(p): Parameters<InvokeCommandParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let value = self
            .command_router
            .invoke(&p.name, p.args)
            .await
            .map_err(port_to_mcp)?;
        Ok(json_success(&value))
    }

    #[tool(
        description = "Liste les commandes logiques exposées par l'app cible (découverte) : tableau JSON `[{name, description}]`, trié par nom. Vide si l'app n'a déclaré aucune route via expose_route."
    )]
    pub(crate) async fn list_commands(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let descriptors = self.command_router.list().await.map_err(port_to_mcp)?;
        Ok(json_success(&descriptors))
    }

    #[tool(
        description = "Décrit la cible active et ses capacités : `{ target, eyes, hands, eval_js, dom, state, console_errors, list_windows, notes }`. Permet de savoir d'avance quels outils répondront (ex. eval_js/dom = false en kwin-eis) sans tenter un appel pour le découvrir."
    )]
    pub(crate) async fn target_info(&self) -> Result<CallToolResult, McpError> {
        // Description statique resolue au demarrage : pas de gate, pas d'I/O.
        Ok(json_success(&self.target_info))
    }

    #[tool(
        description = "Liste les fenêtres/onglets de la cible : `[{ id, title, url, kind }]`. En CDP via Target.getTargets ; en kwin-eis via KWin Scripting (`url` vide, `kind` = resourceClass) ; en x11 via QueryTree (fenêtres visibles titrées/classées, `id` = identifiant X hexadécimal, `kind` = x11-window) ; via le pont SDK : liste vide (pas de visibilité inter-fenêtres)."
    )]
    pub(crate) async fn list_windows(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let windows = self
            .window_lister
            .list_windows()
            .await
            .map_err(port_to_mcp)?;
        Ok(json_success(&windows))
    }

    #[tool(
        description = "Renvoie la géométrie écran `{ x, y, width, height }` de la fenêtre dont le titre contient `title` (ou la fenêtre `--screencap-window` / `--window` par défaut) via KWin Scripting (kwin-eis) ou TranslateCoordinates (x11). Complète `find_element`/`eval_js` (indisponibles en kwin-eis, rendu natif) : convertit un rect DOM en coordonnées écran absolues pour un clic OS-level (EIS) hors du viewport webview. Nécessite KDE/Wayland ou la cible x11 ; sinon résultat isError structuré `{ reason: \"unsupported\" }` (E1)."
    )]
    pub(crate) async fn window_geometry(
        &self,
        Parameters(p): Parameters<WindowGeometryParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let rect = match unsupported_aware(self.window_geometry.geometry(p.title.as_deref()).await)
        {
            Ok(v) => v,
            Err(issue) => return issue,
        };
        Ok(json_success(&rect))
    }

    #[tool(
        description = "Donne le focus OS à une fenêtre (par sous-chaîne de titre, ou la fenêtre --screencap-window par défaut). Crucial en kwin-eis : KWin EIS route les événements vers la fenêtre active ; une notification ou un dialog système peut détourner les clics sans erreur. À appeler avant une séquence d'interaction. Renvoie `{ ok, title }`. Nécessite KDE/Wayland (KWin Scripting) ou la cible x11 ; sinon résultat isError structuré `{ reason: \"unsupported\" }` (E1)."
    )]
    pub(crate) async fn focus_window(
        &self,
        Parameters(p): Parameters<FocusWindowParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let title = match unsupported_aware(
            self.window_manager
                .activate(p.title_pattern.as_deref())
                .await,
        ) {
            Ok(v) => v,
            Err(issue) => return issue,
        };
        Ok(json_success(
            &serde_json::json!({ "ok": true, "title": title }),
        ))
    }

    #[tool(
        description = "Lit le texte du presse-papiers de la session (OS-level via wl-paste sous Wayland). Renvoie `{ text }`. Fiable là où `navigator.clipboard.readText()` lève NotAllowedError (contexte automatisé) et en kwin-eis (pas de WebView). Hors Wayland, résultat isError structuré `{ reason: \"unsupported\" }` (E1)."
    )]
    pub(crate) async fn clipboard_get(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let text = match unsupported_aware(self.clipboard.get().await) {
            Ok(v) => v,
            Err(issue) => return issue,
        };
        Ok(json_success(&serde_json::json!({ "text": text })))
    }

    #[tool(
        description = "Place du texte dans le presse-papiers de la session (OS-level via wl-copy sous Wayland). Combiné à key_combo Ctrl+V, permet de saisir du texte long/Unicode sans frappe touche par touche. Hors Wayland, résultat isError structuré `{ reason: \"unsupported\" }` (E1)."
    )]
    pub(crate) async fn clipboard_set(
        &self,
        Parameters(p): Parameters<ClipboardSetParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        ok_or_unsupported(self.clipboard.set(&p.text).await)
    }
}
