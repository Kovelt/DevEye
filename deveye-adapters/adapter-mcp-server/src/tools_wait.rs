//! Outils de la famille **wait** : `wait_for_selector`, `wait_condition`,
//! `wait_for_log`, `wait_screenshot_region`, `wait_ready`, `wait_for_text`,
//! `wait_target_alive` — attentes sans sleep arbitraire (polling borne par
//! timeout). Scinde de `lib.rs` (H1) : la logique et les schemas d'outils
//! sont inchanges, seul l'emplacement du code bouge.

use super::*;

#[tool_router(router = wait_router, vis = "pub(crate)")]
impl DeveyeMcpServer {
    #[tool(
        description = "Attend qu'un sélecteur CSS soit présent (et optionnellement visible) dans le DOM. Renvoie `{ found, elapsed_ms }` (`isError` si non satisfait). Avec `negate=true`, attend au contraire que l'élément DISPARAISSE (wait_for_gone : spinner/toast/modale) — `found=true` signifie alors « disparu ». Évite tout sleep arbitraire. Défaut 5000 ms, plafond 30000."
    )]
    pub(crate) async fn wait_for_selector(
        &self,
        Parameters(p): Parameters<WaitForSelectorParams>,
    ) -> Result<CallToolResult, McpError> {
        // L14 : delegation au port WaitCondition (supprime la boucle dupliquee).
        // Le port gere le polling (200 ms), les snippets visible/querySelector,
        // et la distinction transport-mort / erreur-JS-transitoire (M4).
        // Le gate n'est PAS tenu : observationnel, comme wait_condition apres M1
        // — le tenir bloquerait tous les outils pendant jusqu'a 30 s.
        let predicate = WaitPredicate::DomSelector {
            selector: p.selector,
            visible: p.visible.unwrap_or(false),
            negate: p.negate.unwrap_or(false),
        };
        let budget = Duration::from_millis(
            p.timeout_ms
                .unwrap_or(WAIT_DEFAULT_MS)
                .min(WAIT_DOM_TIMEOUT_MAX_MS),
        );
        let outcome = self
            .wait_condition
            .wait_until(predicate, budget)
            .await
            .map_err(port_to_mcp)?;
        if outcome.satisfied {
            Ok(json_success(&serde_json::json!({
                "found": true,
                "elapsed_ms": outcome.elapsed_ms,
            })))
        } else {
            // L13 : timeout = echec semantique => isError=true (coherent avec
            // wait_condition).
            Ok(json_error(&serde_json::json!({
                "found": false,
                "elapsed_ms": outcome.elapsed_ms,
            })))
        }
    }

    #[tool(
        description = "Attend qu'un texte apparaisse dans l'innerText/textContent d'un élément (`selector`, ou le document entier si absent). Renvoie `{ found, elapsed_ms }` (`isError` si non satisfait). Avec `visible=true`, l'élément doit en plus avoir un rect non vide. Indisponible en kwin-eis (pas de DOM) — le prédicat `text-content` sous-jacent ne s'y satisfait jamais. Défaut 5000 ms, plafond 30000."
    )]
    pub(crate) async fn wait_for_text(
        &self,
        Parameters(p): Parameters<WaitForTextParams>,
    ) -> Result<CallToolResult, McpError> {
        let predicate = WaitPredicate::TextContent {
            text: p.text,
            selector: p.selector,
            visible: p.visible.unwrap_or(false),
        };
        let budget = Duration::from_millis(
            p.timeout_ms
                .unwrap_or(WAIT_DEFAULT_MS)
                .min(WAIT_DOM_TIMEOUT_MAX_MS),
        );
        let outcome = self
            .wait_condition
            .wait_until(predicate, budget)
            .await
            .map_err(port_to_mcp)?;
        if outcome.satisfied {
            Ok(json_success(&serde_json::json!({
                "found": true,
                "elapsed_ms": outcome.elapsed_ms,
            })))
        } else {
            Ok(json_error(&serde_json::json!({
                "found": false,
                "elapsed_ms": outcome.elapsed_ms,
            })))
        }
    }

    #[tool(
        description = "Attend qu'un prédicat soit satisfait, ou que le timeout expire. Renvoie `{ satisfied, elapsed_ms, detail }` ; `isError` si non satisfait. Prédicats : dom-selector (présence CSS), state-key (valeur dans le state SDK), screenshot-stable (stabilité visuelle pendant threshold_ms), target-alive (attend qu'une cible crashée redevienne joignable, cf. wait_target_alive), text-content (attend un texte dans le DOM, cf. wait_for_text). Évite tout sleep arbitraire."
    )]
    pub(crate) async fn wait_condition(
        &self,
        Parameters(p): Parameters<WaitConditionParams>,
    ) -> Result<CallToolResult, McpError> {
        // M1 : le gate n'est PAS tenu pendant wait_until (boucle de polling
        // jusqu'a WAIT_TIMEOUT_MAX_MS). wait_until est purement
        // observationnel (aucune interaction UI) — le tenir bloquerait tous
        // les autres outils MCP pendant jusqu'a 60 s. E4 : constantes
        // partagees plutot que des `DEFAULT_MS`/`MAX_MS` locaux redefinis
        // ici (et divergents des autres `wait_*`).
        let predicate: WaitPredicate = serde_json::from_value(p.predicate).map_err(internal)?;
        let timeout = Duration::from_millis(
            p.timeout_ms
                .unwrap_or(WAIT_DEFAULT_MS)
                .min(WAIT_TIMEOUT_MAX_MS),
        );
        let outcome = self
            .wait_condition
            .wait_until(predicate, timeout)
            .await
            .map_err(port_to_mcp)?;
        if outcome.satisfied {
            Ok(json_success(&outcome))
        } else {
            Ok(json_error(&outcome))
        }
    }

    #[tool(
        description = "Attend qu'un log applicatif (tracing du SDK) dont le message contient `contains` (et de niveau >= `level_min`) soit émis. Renvoie `{ satisfied, elapsed_ms, detail }` ; `isError` si timeout. Observe les logs Rust bufferisés, hors de portée d'eval_js. NB : pendant l'attente le buffer de logs est drainé (les logs non concernés sont consommés). Défaut 5000 ms, plafond 60000."
    )]
    pub(crate) async fn wait_for_log(
        &self,
        Parameters(p): Parameters<WaitForLogParams>,
    ) -> Result<CallToolResult, McpError> {
        let level_min = match p.level_min {
            Some(s) => Some(
                serde_json::from_value::<LogLevel>(serde_json::Value::String(s)).map_err(|_| {
                    internal("level_min invalide (trace/debug/info/warn/error attendu)")
                })?,
            ),
            None => None,
        };
        let predicate = WaitPredicate::LogMessage {
            contains: p.contains,
            level_min,
        };
        let budget = Duration::from_millis(
            p.timeout_ms
                .unwrap_or(WAIT_DEFAULT_MS)
                .min(WAIT_TIMEOUT_MAX_MS),
        );
        let outcome = self
            .wait_condition
            .wait_until(predicate, budget)
            .await
            .map_err(port_to_mcp)?;
        if outcome.satisfied {
            Ok(json_success(&outcome))
        } else {
            Ok(json_error(&outcome))
        }
    }

    #[tool(
        description = "Comme la stabilité visuelle, mais sur une RÉGION (x, y, width, height) seulement : attend que cette zone se stabilise (deux captures identiques à `threshold_ms` d'intervalle). Ignore un curseur clignotant / une horloge ailleurs. Renvoie `{ satisfied, elapsed_ms, detail }` ; `isError` si timeout. Défaut threshold 300 (plafond 2000), timeout 5000 (plafond 60000)."
    )]
    pub(crate) async fn wait_screenshot_region(
        &self,
        Parameters(p): Parameters<WaitScreenshotRegionParams>,
    ) -> Result<CallToolResult, McpError> {
        let predicate = WaitPredicate::ScreenshotRegionStable {
            x: p.x,
            y: p.y,
            width: p.width,
            height: p.height,
            threshold_ms: p.threshold_ms.unwrap_or(300),
        };
        let budget = Duration::from_millis(
            p.timeout_ms
                .unwrap_or(WAIT_DEFAULT_MS)
                .min(WAIT_TIMEOUT_MAX_MS),
        );
        let outcome = self
            .wait_condition
            .wait_until(predicate, budget)
            .await
            .map_err(port_to_mcp)?;
        if outcome.satisfied {
            Ok(json_success(&outcome))
        } else {
            Ok(json_error(&outcome))
        }
    }

    #[tool(
        description = "Attend que le CANAL D'INTERACTION de la cible redevienne joignable après un crash/redémarrage (sonde eval_js ; sur kwin-eis, sans JS, retombe sur la capture d'écran). Renvoie `{ satisfied, elapsed_ms, detail }` ; `isError` si timeout. Raccourci du prédicat target-alive (wait_condition) — seul paramètre : le timeout. Défaut 5000 ms, plafond 60000."
    )]
    pub(crate) async fn wait_target_alive(
        &self,
        Parameters(p): Parameters<WaitTargetAliveParams>,
    ) -> Result<CallToolResult, McpError> {
        let budget = Duration::from_millis(
            p.timeout_ms
                .unwrap_or(WAIT_DEFAULT_MS)
                .min(WAIT_TIMEOUT_MAX_MS),
        );
        let outcome = self
            .wait_condition
            .wait_until(WaitPredicate::TargetAlive {}, budget)
            .await
            .map_err(port_to_mcp)?;
        if outcome.satisfied {
            Ok(json_success(&outcome))
        } else {
            Ok(json_error(&outcome))
        }
    }

    #[tool(
        description = "Attend que l'app cible publie un state non vide (signe qu'elle est prête), au plus `timeout_ms` ms (poll toutes les 200 ms, plafond 60000). Évite les captures/clics prématurés juste après le lancement. `isError` si le délai expire."
    )]
    pub(crate) async fn wait_ready(
        &self,
        Parameters(p): Parameters<WaitReadyParams>,
    ) -> Result<CallToolResult, McpError> {
        // Gate locke PAR ITERATION (snapshot atomique), relache pendant le sleep
        // — cf. M1 : un wait observationnel ne doit pas bloquer les autres outils.
        let start = Instant::now();
        let budget = Duration::from_millis(p.timeout_ms.min(WAIT_TIMEOUT_MAX_MS));
        let mut erreur_notifiee = false;
        loop {
            let ready = {
                // Lock direct (pas `acquire_gate`) : instrumenter le gate a
                // chaque tick de 200 ms noierait les logs debug sans information.
                let _serial = self.gate.lock().await;
                // A5 : ABSORBE TOUTES les erreurs, y compris `PortError::Network`
                // — contrairement au filtre de `CompositeWaitCondition` (qui
                // propage Network comme un vrai echec de transport). Ici c'est
                // l'inverse : la raison d'etre de `wait_ready` est d'attendre le
                // DEMARRAGE de la cible, et "connexion refusee" tant que le SDK
                // n'ecoute pas encore EST l'etat transitoire nominal, pas une
                // panne. Une erreur ne doit donc jamais faire echouer l'attente
                // avant l'expiration du budget.
                match self.inspector.state_snapshot().await {
                    Ok(dump) => !dump.entries.is_empty(),
                    Err(e) if !erreur_notifiee => {
                        erreur_notifiee = true;
                        debug!(erreur = %e, "wait_ready: erreur absorbee (demarrage en cours, tracee une seule fois)");
                        false
                    }
                    Err(_) => false,
                }
            };
            if ready {
                return Ok(json_success(&serde_json::json!({
                    "ready": true,
                    "waited_ms": start.elapsed().as_millis() as u64,
                })));
            }
            if start.elapsed() >= budget {
                return Ok(json_error(&serde_json::json!({
                    "ready": false,
                    "waited_ms": start.elapsed().as_millis() as u64,
                })));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}
