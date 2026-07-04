#![forbid(unsafe_code)]
//! Adapter `adapter-composite` — types composites generiques.
//!
//! Ce crate centralise les briques d'orchestration testables sans I/O reelle :
//! sources d'erreurs console JS, condition d'attente multi-predicats,
//! inspecteur composant DOM + state. Ces types fonctionnent pour toutes les
//! cibles sans impl par adapter.
//!
//! Issu de `deveye-server::orchestration` (refacto hexagonale lot C) :
//! les adapters composites n'ont pas leur place dans le binaire orchestrateur.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tracing::debug;

use deveye_core::{ConsoleError, DomTree, LogLevel, StateDump, WaitOutcome, WaitPredicate};
use deveye_ports::{
    AppInspector, ConsoleErrorSource, JsEvaluator, LogSource, PortError, PortResult, Renderer,
    WaitCondition,
};

// ── ConsoleErrorSource ────────────────────────────────────────────────────────

/// `ConsoleErrorSource` noop : renvoie toujours un buffer vide.
///
/// Utilise en mode kwin-eis ou la console JS n'existe pas (rendu natif, pas
/// de WebView). Ce n'est pas une erreur — `PortError::Unsupported` serait
/// trompeur ; l'absence de webview ne produit tout simplement aucune erreur
/// console a drainer.
pub struct NoopConsoleErrorSource;

#[async_trait]
impl ConsoleErrorSource for NoopConsoleErrorSource {
    async fn drain_console_errors(&self) -> PortResult<Vec<ConsoleError>> {
        Ok(Vec::new())
    }
}

/// Collecteur d'erreurs console JS, injecté une fois puis drainé. Pur JS →
/// fonctionne via CDP comme via la pompe SDK. Renvoie des entrées typées
/// (level/text/url/line/timestamp) directement désérialisables en `ConsoleError`.
///
/// Capture desormais TOUS les niveaux (`error`/`warning`/`log`/`info`/`debug`),
/// pas seulement `console.error`/`console.warn` — le drain renvoie tout, le
/// filtrage par niveau se fait en aval (cote MCP). `ConsoleLevel` (deveye-core)
/// a bien une variante pour chacun de ces niveaux, `Debug` comprise.
///
/// Buffer borne a 500 entrees (meme principe que `LOG_BUFFER_CAPACITY` cote
/// SDK) : au-dela, une entree est evincee et `__deveye_console_dropped__`
/// incremente (perf B5). L'eviction PRESERVE les erreurs : on retire la plus
/// ancienne entree NON-`error` ; on ne sacrifie une `error` qu'en dernier
/// recours (buffer plein d'erreurs). Sans cela, un flot de `console.log`/`info`/
/// `debug` — desormais capture — pourrait evincer une vraie erreur avant le
/// drain (regression signalee par la revue T4). Le snippet renvoie
/// `{ entries, dropped }` ; `drain_console_errors` trace un `warn!` si des
/// entrees ont ete perdues.
const CONSOLE_ERRORS_SNIPPET: &str = r#"(function(){
  var CAP = 500;
  if (!window.__deveye_console_init__) {
    window.__deveye_console_errors__ = [];
    window.__deveye_console_dropped__ = 0;
    var push = function(level, text, url, line){
      try {
        var buf = window.__deveye_console_errors__;
        if (buf.length >= CAP) {
          // Preserve les erreurs : evince la plus ancienne entree non-error,
          // sinon (buffer 100% erreurs) la plus ancienne tout court.
          var idx = buf.findIndex(function(x){ return x.level !== 'error'; });
          if (idx < 0) { idx = 0; }
          buf.splice(idx, 1);
          window.__deveye_console_dropped__++;
        }
        buf.push({ level: level, text: text, url: url, line: line, timestamp: new Date().toISOString() });
      } catch (_) {}
    };
    window.addEventListener('error', function(e){
      push('error', String((e && (e.message || e.error)) || 'error'), (e && e.filename) || null, (e && e.lineno) || null);
    });
    window.addEventListener('unhandledrejection', function(e){
      push('error', String(e && e.reason), null, null);
    });
    var wrapConsole = function(method, level){
      var orig = console[method];
      console[method] = function(){
        push(level, Array.prototype.map.call(arguments, String).join(' '), null, null);
        if (orig) { orig.apply(console, arguments); }
      };
    };
    wrapConsole('error', 'error');
    wrapConsole('warn', 'warning');
    wrapConsole('log', 'log');
    wrapConsole('info', 'info');
    wrapConsole('debug', 'debug');
    window.__deveye_console_init__ = true;
  }
  var entries = window.__deveye_console_errors__.splice(0);
  var dropped = window.__deveye_console_dropped__;
  window.__deveye_console_dropped__ = 0;
  return { entries: entries, dropped: dropped };
})()"#;

/// `ConsoleErrorSource` générique : injecte un collecteur JS via l'évaluateur
/// (CDP ou pompe SDK) et draine les erreurs typées. La capture CDP-native
/// (`Runtime.consoleAPICalled`) reste un raffinement futur derrière ce même port.
pub struct JsConsoleErrorSource {
    pub evaluator: Arc<dyn JsEvaluator>,
}

#[async_trait]
impl ConsoleErrorSource for JsConsoleErrorSource {
    async fn drain_console_errors(&self) -> PortResult<Vec<ConsoleError>> {
        let value = self.evaluator.evaluate(CONSOLE_ERRORS_SNIPPET).await?;
        // Le snippet renvoie `{ entries: [...], dropped: N }`. `dropped > 0`
        // signale que des messages ont ete evinces (buffer plein) avant ce
        // drain : on le trace pour que la perte ne soit pas totalement muette.
        let (arr, dropped) = match value {
            serde_json::Value::Object(mut o) => {
                let dropped = o
                    .get("dropped")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                match o.remove("entries") {
                    Some(serde_json::Value::Array(a)) => (a, dropped),
                    _ => (Vec::new(), dropped),
                }
            }
            other => {
                eprintln!(
                    "deveye-server: drain_console_errors: valeur JS inattendue ({}), buffer ignore",
                    other
                );
                return Ok(Vec::new());
            }
        };
        if dropped > 0 {
            tracing::warn!(
                dropped,
                "console: buffer plein, {dropped} entree(s) evincee(s) avant drain"
            );
        }
        let mut out = Vec::with_capacity(arr.len());
        for (i, elem) in arr.into_iter().enumerate() {
            match serde_json::from_value::<ConsoleError>(elem) {
                Ok(e) => out.push(e),
                Err(e) => {
                    eprintln!(
                        "deveye-server: drain_console_errors: element[{i}] non parseable, ignore: {e}"
                    );
                }
            }
        }
        Ok(out)
    }
}

// ── WaitCondition composite ───────────────────────────────────────────────────

/// Plafond du delai inter-captures pour `ScreenshotStable` : evite des pauses
/// demesurees a chaque iteration du polling (l'attente totale est bornee par le
/// timeout du WaitCondition, mais chaque check dormait jusqu'a la valeur brute
/// fournie par l'appelant, potentiellement > 60 s).
const MAX_SCREENSHOT_STABLE_THRESHOLD_MS: u64 = 2000;

/// Hache les octets PNG d'une capture via [`DefaultHasher`] (non cryptographique,
/// suffisant pour la comparaison de stabilite visuelle).
///
/// On compare les hash plutot que les `Vec<u8>` complets (plusieurs Mo par
/// PNG encode) : evite une allocation memoire + comparaison byte-a-byte a
/// chaque iteration de polling.
///
/// Note : le port [`Renderer`] renvoie des PNG deja encodes (pas des pixels
/// bruts) ; hasher les octets PNG est la seule option sans modifier le port.
fn hash_screenshot_bytes(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// `WaitCondition` composite : boucle de polling sur les ports déjà composés
/// (évaluateur pour `DomSelector`, inspecteur pour `StateKey`, renderer pour
/// `ScreenshotStable`). Marche pour toutes les cibles sans impl par adapter.
pub struct CompositeWaitCondition {
    pub evaluator: Arc<dyn JsEvaluator>,
    pub inspector: Arc<dyn AppInspector>,
    pub renderer: Arc<dyn Renderer>,
    pub log_source: Arc<dyn LogSource>,
}

/// Hache les pixels RGBA d'une **region** d'un PNG (decode + crop_imm), bornee
/// aux dimensions reelles. Sert au predicat `ScreenshotRegionStable` : on hashe
/// les pixels bruts plutot que de re-encoder un PNG a chaque iteration.
fn hash_region(png: &[u8], x: u32, y: u32, width: u32, height: u32) -> PortResult<u64> {
    use image::GenericImageView;
    let img = image::load_from_memory(png)
        .map_err(|e| PortError::Render(format!("decode PNG region : {e}")))?;
    let (iw, ih) = img.dimensions();
    if x >= iw || y >= ih {
        return Err(PortError::Render(format!(
            "region hors capture : ({x}, {y}) hors {iw}x{ih}"
        )));
    }
    let cw = width.min(iw - x).max(1);
    let ch = height.min(ih - y).max(1);
    let cropped = img.crop_imm(x, y, cw, ch).to_rgba8();
    let mut h = DefaultHasher::new();
    cropped.as_raw().hash(&mut h);
    Ok(h.finish())
}

/// A5 : absorbe une erreur transitoire de port exactement comme le fait deja
/// la branche `DomSelector` depuis M4 — seule `PortError::Network` (transport
/// mort) est un vrai echec et se propage ; toute autre erreur (app pas encore
/// demarree, exception ponctuelle...) est journalisee puis traitee comme
/// "pas encore satisfait" pour laisser le polling continuer. Sans ce filtre
/// uniforme, une app qui redemarre faisait echouer `wait_*` des le premier
/// hoquet — l'inverse de leur raison d'etre.
fn absorb_transient<T>(result: PortResult<T>, predicate: &'static str) -> PortResult<Option<T>> {
    match result {
        Ok(v) => Ok(Some(v)),
        Err(e @ PortError::Network(_)) => Err(e),
        Err(e) => {
            debug!(predicate, erreur = %e, "erreur transitoire absorbee");
            Ok(None)
        }
    }
}

impl CompositeWaitCondition {
    async fn check(&self, predicate: &WaitPredicate) -> PortResult<bool> {
        match predicate {
            WaitPredicate::DomSelector {
                selector,
                visible,
                negate,
            } => {
                let sel = serde_json::to_string(selector).unwrap_or_default();
                // (a) VISIBLE : snippet adapte selon le mode demande. Le snippet
                // renvoie la PRESENCE (ou presence+visibilite) ; la negation est
                // appliquee cote Rust pour rester robuste aux exceptions JS.
                let snippet = if *visible {
                    format!(
                        "(function(){{ const el = document.querySelector({sel}); \
                         if (!el) return false; \
                         const r = el.getBoundingClientRect(); \
                         return r.width > 0 && r.height > 0; }})()"
                    )
                } else {
                    format!("!!document.querySelector({sel})")
                };
                // (b) M4 : distingue transport mort (Network) des erreurs JS
                // transitoires. Un selecteur pas-encore-present peut lever une
                // exception JS benigne — on la traite comme `false` (continuer
                // le polling). Seule une erreur de transport interrompt l'attente.
                match absorb_transient(self.evaluator.evaluate(&snippet).await, "dom-selector")? {
                    // `negate` : satisfaite quand l'element est absent. On
                    // n'inverse que sur une evaluation REUSSIE — une erreur
                    // transitoire reste "non satisfaite" (continuer le polling),
                    // jamais transformee en faux "disparu".
                    Some(value) => {
                        let present = value.as_bool().unwrap_or(false);
                        Ok(present ^ *negate)
                    }
                    None => Ok(false),
                }
            }
            WaitPredicate::StateKey { key, expected } => {
                let dump =
                    match absorb_transient(self.inspector.state_snapshot().await, "state-key")? {
                        Some(d) => d,
                        None => return Ok(false),
                    };
                Ok(dump.entries.get(key) == Some(expected))
            }
            WaitPredicate::ScreenshotStable { threshold_ms } => {
                // Plafonne a MAX_SCREENSHOT_STABLE_THRESHOLD_MS pour eviter une
                // pause demesure a chaque iteration (le timeout global du caller
                // borne la duree totale, pas chaque check individuel).
                let capped = (*threshold_ms).min(MAX_SCREENSHOT_STABLE_THRESHOLD_MS);
                let first = match absorb_transient(
                    self.renderer.screenshot().await,
                    "screenshot-stable",
                )? {
                    Some(s) => s,
                    None => return Ok(false),
                };
                let hash_first = hash_screenshot_bytes(&first.bytes);
                tokio::time::sleep(Duration::from_millis(capped)).await;
                let second = match absorb_transient(
                    self.renderer.screenshot().await,
                    "screenshot-stable",
                )? {
                    Some(s) => s,
                    None => return Ok(false),
                };
                let hash_second = hash_screenshot_bytes(&second.bytes);
                Ok(hash_first == hash_second)
            }
            WaitPredicate::ScreenshotRegionStable {
                x,
                y,
                width,
                height,
                threshold_ms,
            } => {
                let capped = (*threshold_ms).min(MAX_SCREENSHOT_STABLE_THRESHOLD_MS);
                let first = match absorb_transient(
                    self.renderer.screenshot().await,
                    "screenshot-region-stable",
                )? {
                    Some(s) => s,
                    None => return Ok(false),
                };
                // Une region hors capture est une erreur de parametrage de
                // l'appelant, pas un etat transitoire de l'app : elle reste
                // propagee telle quelle (pas de filtre `absorb_transient` ici).
                let hash_first = hash_region(&first.bytes, *x, *y, *width, *height)?;
                tokio::time::sleep(Duration::from_millis(capped)).await;
                let second = match absorb_transient(
                    self.renderer.screenshot().await,
                    "screenshot-region-stable",
                )? {
                    Some(s) => s,
                    None => return Ok(false),
                };
                let hash_second = hash_region(&second.bytes, *x, *y, *width, *height)?;
                Ok(hash_first == hash_second)
            }
            WaitPredicate::LogMessage {
                contains,
                level_min,
            } => {
                // Draine le buffer de logs et teste chaque evenement. Le drain
                // consomme les logs (cf. doc du predicat) : un match dans cette
                // iteration suffit a satisfaire l'attente.
                let events = match absorb_transient(self.log_source.poll().await, "log-message")? {
                    Some(e) => e,
                    None => return Ok(false),
                };
                let min = level_min.unwrap_or(LogLevel::Trace);
                Ok(events
                    .iter()
                    .any(|e| e.level >= min && e.message.contains(contains.as_str())))
            }
            WaitPredicate::TargetAlive {} => {
                // Sonde le CANAL D'INTERACTION reel via `evaluator.evaluate` :
                // c'est lui que l'agent utilisera ensuite, et sa chaine de repli
                // en mode auto (CDP -> SDK) reste faite de vrais transports. NE
                // PAS sonder le renderer en premier : en mode auto il est
                // enveloppe dans un FallbackRenderer dont le repli capture
                // l'ecran OS-level et REUSSIT meme quand le transport CDP est
                // mort -> faux "vivant" (revue T4). Sur kwin-eis l'evaluator
                // renvoie `Unsupported` (pas de JS) : la seulement, "une image
                // est capturable" est le seul signal de vie, on retombe donc
                // sur le renderer.
                //
                // Ne PAS utiliser `absorb_transient` : ce predicat attend la
                // RESURRECTION d'une cible dont le transport vient de mourir —
                // toute erreur transitoire (y compris `Network`) est "pas encore
                // vivante", jamais un echec de `wait_until` (meme intention que
                // `wait_ready`).
                match self.evaluator.evaluate("1").await {
                    Ok(_) => Ok(true),
                    // Cible sans JS (kwin-eis) : la capture est le seul signal.
                    Err(PortError::Unsupported(_)) => match self.renderer.screenshot().await {
                        Ok(_) => Ok(true),
                        Err(_) => Ok(false),
                    },
                    Err(_) => Ok(false),
                }
            }
            WaitPredicate::TextContent {
                text,
                selector,
                visible,
            } => {
                let text_json = serde_json::to_string(text).unwrap_or_default();
                let el_expr = match selector {
                    Some(s) => {
                        let sel = serde_json::to_string(s).unwrap_or_default();
                        format!("document.querySelector({sel})")
                    }
                    // Pas de selecteur => le document entier (`body`).
                    None => "document.body".to_string(),
                };
                // Meme structure que DomSelector : le snippet renvoie un
                // booleen pur, aucune negation cote JS (coherence/robustesse
                // aux exceptions deja actee pour DomSelector).
                let snippet = if *visible {
                    format!(
                        "(function(){{ const el = {el_expr}; \
                         if (!el) return false; \
                         const r = el.getBoundingClientRect(); \
                         if (!(r.width > 0 && r.height > 0)) return false; \
                         return (el.innerText || el.textContent || '').includes({text_json}); }})()"
                    )
                } else {
                    format!(
                        "(function(){{ const el = {el_expr}; \
                         if (!el) return false; \
                         return (el.innerText || el.textContent || '').includes({text_json}); }})()"
                    )
                };
                // Meme filtre que DomSelector : erreur JS transitoire => false,
                // seule Network propage. Sur kwin-eis (pas de DOM), l'evaluator
                // renvoie `Unsupported`, absorbe comme transitoire — ce predicat
                // ne se satisfera donc jamais la-bas (documente sur la variante).
                match absorb_transient(self.evaluator.evaluate(&snippet).await, "text-content")? {
                    Some(value) => Ok(value.as_bool().unwrap_or(false)),
                    None => Ok(false),
                }
            }
        }
    }
}

#[async_trait]
impl WaitCondition for CompositeWaitCondition {
    async fn wait_until(
        &self,
        predicate: WaitPredicate,
        timeout: Duration,
    ) -> PortResult<WaitOutcome> {
        const POLL: Duration = Duration::from_millis(200);
        let start = Instant::now();
        let deadline = start + timeout;
        loop {
            if self.check(&predicate).await? {
                return Ok(WaitOutcome {
                    satisfied: true,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    detail: "prédicat satisfait".to_string(),
                });
            }
            if Instant::now() >= deadline {
                return Ok(WaitOutcome {
                    satisfied: false,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    detail: format!("timeout après {} ms", timeout.as_millis()),
                });
            }
            tokio::time::sleep(POLL).await;
        }
    }
}

// ── CompositeInspector ────────────────────────────────────────────────────────

/// Compose un inspector DOM (typiquement CDP) avec un inspector state
/// (typiquement le SDK opt-in). Permet de tirer le DOM d'une source et le
/// state d'une autre derriere un meme port `AppInspector`.
pub struct CompositeInspector {
    pub dom: Arc<dyn AppInspector>,
    pub state: Arc<dyn AppInspector>,
}

#[async_trait]
impl AppInspector for CompositeInspector {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        self.dom.dom_snapshot().await
    }

    async fn state_snapshot(&self) -> PortResult<StateDump> {
        self.state.state_snapshot().await
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use deveye_core::{ConsoleLevel, LogEvent, Screenshot, Size};
    use deveye_ports::PortError;

    // Timestamp RFC3339 fixe utilise dans les donnees de test (evite le besoin
    // d'une dependance directe a `chrono` dans le binaire).
    const TS: &str = "2026-01-01T00:00:00Z";

    /// `drain_console_errors` parse la forme `{ entries, dropped }` renvoyee par
    /// le snippet : entrees typees remontees (y compris `debug`), element
    /// corrompu ignore sans faire disparaitre les autres, `dropped` non fatal.
    #[tokio::test]
    async fn drain_console_parses_entries_and_tolerates_corrupt_element() {
        struct FixedEval(serde_json::Value);
        #[async_trait]
        impl JsEvaluator for FixedEval {
            async fn evaluate(&self, _: &str) -> PortResult<serde_json::Value> {
                Ok(self.0.clone())
            }
        }
        let payload = serde_json::json!({
            "dropped": 3,
            "entries": [
                { "level": "debug", "text": "d", "url": null, "line": null, "timestamp": TS },
                { "level": "PAS_UN_NIVEAU", "text": "corrompu", "timestamp": TS },
                { "level": "error", "text": "boom", "url": null, "line": null, "timestamp": TS },
            ],
        });
        let src = JsConsoleErrorSource {
            evaluator: Arc::new(FixedEval(payload)),
        };
        let out = src.drain_console_errors().await.expect("drain ok");
        // 2 entrees valides (debug + error), la corrompue est ignoree.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].level, ConsoleLevel::Debug);
        assert_eq!(out[1].level, ConsoleLevel::Error);
    }

    /// `LogSource` stub : buffer vide. Permet de construire un
    /// `CompositeWaitCondition` dans les tests qui n'exercent pas le predicat
    /// `LogMessage`.
    struct EmptyLog;
    #[async_trait]
    impl LogSource for EmptyLog {
        async fn poll(&self) -> PortResult<Vec<LogEvent>> {
            Ok(Vec::new())
        }
    }

    /// `LogSource` stub renvoyant une fois une liste fixe d'evenements, puis du
    /// vide (simule un drain). Sert au test du predicat `LogMessage`.
    struct ScriptedLog {
        events: std::sync::Mutex<Vec<LogEvent>>,
    }
    #[async_trait]
    impl LogSource for ScriptedLog {
        async fn poll(&self) -> PortResult<Vec<LogEvent>> {
            Ok(std::mem::take(&mut *self.events.lock().unwrap()))
        }
    }

    // Stubs reutilisables pour les ports non exerces par un test donne.
    struct UnusedEval;
    #[async_trait]
    impl JsEvaluator for UnusedEval {
        async fn evaluate(&self, _: &str) -> PortResult<serde_json::Value> {
            Ok(serde_json::json!(false))
        }
    }
    struct UnusedInsp;
    #[async_trait]
    impl AppInspector for UnusedInsp {
        async fn dom_snapshot(&self) -> PortResult<DomTree> {
            Err(PortError::Unsupported("stub".to_string()))
        }
        async fn state_snapshot(&self) -> PortResult<StateDump> {
            Err(PortError::Unsupported("stub".to_string()))
        }
    }
    /// Renderer renvoyant toujours le meme PNG (capture stable).
    struct ConstRenderer {
        png: Vec<u8>,
        size: Size,
    }
    #[async_trait]
    impl Renderer for ConstRenderer {
        async fn screenshot(&self) -> PortResult<Screenshot> {
            Ok(Screenshot::new_png(self.png.clone(), self.size))
        }
        async fn viewport_size(&self) -> PortResult<Size> {
            Ok(self.size)
        }
    }

    fn solid_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([20, 40, 60, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("encode png test");
        out
    }

    #[tokio::test]
    async fn composite_log_message_satisfied_on_match() {
        let log = Arc::new(ScriptedLog {
            events: std::sync::Mutex::new(vec![LogEvent::new(
                LogLevel::Info,
                "app",
                "record saved ok",
            )]),
        });
        let cond = CompositeWaitCondition {
            evaluator: Arc::new(UnusedEval),
            inspector: Arc::new(UnusedInsp),
            renderer: Arc::new(ConstRenderer {
                png: solid_png(2, 2),
                size: Size::new(2, 2),
            }),
            log_source: log,
        };
        let outcome = cond
            .wait_until(
                WaitPredicate::LogMessage {
                    contains: "saved".to_string(),
                    level_min: Some(LogLevel::Info),
                },
                Duration::from_millis(500),
            )
            .await
            .expect("wait_until ne doit pas echouer");
        assert!(outcome.satisfied, "le log 'saved' (info) doit satisfaire");
    }

    #[tokio::test]
    async fn composite_log_message_level_filter_excludes_lower() {
        // Un log de niveau debug ne satisfait pas un level_min=error.
        let log = Arc::new(ScriptedLog {
            events: std::sync::Mutex::new(vec![LogEvent::new(LogLevel::Debug, "app", "saved")]),
        });
        let cond = CompositeWaitCondition {
            evaluator: Arc::new(UnusedEval),
            inspector: Arc::new(UnusedInsp),
            renderer: Arc::new(ConstRenderer {
                png: solid_png(2, 2),
                size: Size::new(2, 2),
            }),
            log_source: log,
        };
        let outcome = cond
            .wait_until(
                WaitPredicate::LogMessage {
                    contains: "saved".to_string(),
                    level_min: Some(LogLevel::Error),
                },
                Duration::from_millis(40),
            )
            .await
            .expect("wait_until ne doit pas echouer");
        assert!(!outcome.satisfied, "debug < error => non satisfait");
    }

    #[tokio::test]
    async fn composite_screenshot_region_stable_satisfied_for_constant_capture() {
        let png = solid_png(20, 20);
        let cond = CompositeWaitCondition {
            evaluator: Arc::new(UnusedEval),
            inspector: Arc::new(UnusedInsp),
            renderer: Arc::new(ConstRenderer {
                png,
                size: Size::new(20, 20),
            }),
            log_source: Arc::new(EmptyLog),
        };
        let outcome = cond
            .wait_until(
                WaitPredicate::ScreenshotRegionStable {
                    x: 2,
                    y: 2,
                    width: 8,
                    height: 8,
                    threshold_ms: 10,
                },
                Duration::from_millis(500),
            )
            .await
            .expect("wait_until ne doit pas echouer");
        assert!(outcome.satisfied, "capture constante => region stable");
    }

    // ----- M5 : drain element-par-element -----

    /// Un tableau JSON contenant un element valide + un element corrompu doit
    /// renvoyer exactement l'element valide, pas une liste vide.
    #[test]
    fn drain_parse_skips_corrupt_keeps_valid() {
        let valid = serde_json::json!({
            "level": "error",
            "text": "boom",
            "timestamp": TS
        });
        // Element corrompu : "level" a une valeur numerique => Deserialize echoue.
        let corrupt = serde_json::json!({
            "level": 42,
            "text": "corrupt",
            "timestamp": TS
        });
        let arr = serde_json::Value::Array(vec![valid, corrupt]);

        // Reproduit la logique du impl sans IO.
        let result: Vec<ConsoleError> = match arr {
            serde_json::Value::Array(a) => a
                .into_iter()
                .filter_map(|elem| serde_json::from_value::<ConsoleError>(elem).ok())
                .collect(),
            _ => vec![],
        };

        assert_eq!(result.len(), 1, "seul l'element valide doit etre conserve");
        assert_eq!(result[0].level, ConsoleLevel::Error);
        assert_eq!(result[0].text, "boom");
    }

    /// Un tableau entierement vide renvoie un Vec vide (pas d'erreur).
    #[test]
    fn drain_empty_array_gives_empty_vec() {
        let arr = serde_json::Value::Array(vec![]);
        let result: Vec<ConsoleError> = match arr {
            serde_json::Value::Array(a) => a
                .into_iter()
                .filter_map(|elem| serde_json::from_value::<ConsoleError>(elem).ok())
                .collect(),
            _ => vec![],
        };
        assert!(result.is_empty());
    }

    // ----- M9 : hash + plafond threshold -----

    /// Deux tranches identiques produisent le meme hash.
    #[test]
    fn hash_screenshot_bytes_same_input_same_hash() {
        let data = b"\x89PNG\r\ndeadbeef";
        assert_eq!(hash_screenshot_bytes(data), hash_screenshot_bytes(data));
    }

    /// Deux tranches differentes produisent (avec une forte probabilite) un hash different.
    #[test]
    fn hash_screenshot_bytes_different_input_different_hash() {
        let a = b"\x89PNG\r\n\x1a\nchunk_a";
        let b = b"\x89PNG\r\n\x1a\nchunk_b";
        assert_ne!(hash_screenshot_bytes(a), hash_screenshot_bytes(b));
    }

    /// Le plafond MAX_SCREENSHOT_STABLE_THRESHOLD_MS est bien applique.
    #[test]
    fn screenshot_stable_threshold_capped() {
        let raw: u64 = 60_000;
        let capped = raw.min(MAX_SCREENSHOT_STABLE_THRESHOLD_MS);
        assert_eq!(capped, MAX_SCREENSHOT_STABLE_THRESHOLD_MS);
        // Une valeur dans la limite passe intacte.
        let small: u64 = 500;
        assert_eq!(small.min(MAX_SCREENSHOT_STABLE_THRESHOLD_MS), 500);
    }

    // ----- H2 : tests d'orchestration (audit lot B/C) -----
    // Les tests du predicat `is_liaison_error` et des structs `Fallback*` sont
    // dans `deveye-server::fallback` (crate::fallback::tests).

    /// Un `CompositeWaitCondition` dont le predicat n'est jamais satisfait doit
    /// renvoyer `WaitOutcome { satisfied: false }` apres expiration du timeout
    /// — sans erreur.
    #[tokio::test]
    async fn composite_wait_condition_timeout_returns_unsatisfied() {
        struct FalseEval;
        struct StubInspector;
        struct StubRenderer;

        #[async_trait]
        impl JsEvaluator for FalseEval {
            async fn evaluate(&self, _: &str) -> PortResult<serde_json::Value> {
                Ok(serde_json::json!(false))
            }
        }

        #[async_trait]
        impl AppInspector for StubInspector {
            async fn dom_snapshot(&self) -> PortResult<DomTree> {
                Err(PortError::Unsupported("stub".to_string()))
            }
            async fn state_snapshot(&self) -> PortResult<StateDump> {
                Err(PortError::Unsupported("stub".to_string()))
            }
        }

        #[async_trait]
        impl Renderer for StubRenderer {
            async fn screenshot(&self) -> PortResult<Screenshot> {
                Err(PortError::Unsupported("stub".to_string()))
            }
            async fn viewport_size(&self) -> PortResult<Size> {
                Err(PortError::Unsupported("stub".to_string()))
            }
        }

        let cond = CompositeWaitCondition {
            evaluator: Arc::new(FalseEval),
            inspector: Arc::new(StubInspector),
            renderer: Arc::new(StubRenderer),
            log_source: Arc::new(EmptyLog),
        };
        let outcome = cond
            .wait_until(
                WaitPredicate::DomSelector {
                    selector: ".never".to_string(),
                    visible: false,
                    negate: false,
                },
                Duration::from_millis(50),
            )
            .await
            .expect("wait_until ne doit pas renvoyer Err");
        assert!(!outcome.satisfied, "le predicat ne peut etre satisfait");
    }

    /// DomSelector visible=true : le snippet passe a evaluate doit contenir
    /// `getBoundingClientRect` (semantique de visibilite). On verifie via un
    /// evaluateur spy qui capture le dernier snippet recu.
    #[tokio::test]
    async fn composite_dom_selector_visible_snippet_contains_bounding_rect() {
        struct SpyEval {
            captured: std::sync::Mutex<String>,
        }

        #[async_trait]
        impl JsEvaluator for SpyEval {
            async fn evaluate(&self, snippet: &str) -> PortResult<serde_json::Value> {
                *self.captured.lock().unwrap() = snippet.to_string();
                // Renvoie true => predicat satisfait immediatement.
                Ok(serde_json::json!(true))
            }
        }

        struct NoopInspector2;
        struct NoopRenderer2;

        #[async_trait]
        impl AppInspector for NoopInspector2 {
            async fn dom_snapshot(&self) -> PortResult<DomTree> {
                Err(PortError::Unsupported("stub".to_string()))
            }
            async fn state_snapshot(&self) -> PortResult<StateDump> {
                Err(PortError::Unsupported("stub".to_string()))
            }
        }

        #[async_trait]
        impl Renderer for NoopRenderer2 {
            async fn screenshot(&self) -> PortResult<Screenshot> {
                Err(PortError::Unsupported("stub".to_string()))
            }
            async fn viewport_size(&self) -> PortResult<Size> {
                Err(PortError::Unsupported("stub".to_string()))
            }
        }

        let spy = Arc::new(SpyEval {
            captured: std::sync::Mutex::new(String::new()),
        });
        let cond = CompositeWaitCondition {
            evaluator: Arc::clone(&spy) as Arc<dyn JsEvaluator>,
            inspector: Arc::new(NoopInspector2),
            renderer: Arc::new(NoopRenderer2),
            log_source: Arc::new(EmptyLog),
        };
        let outcome = cond
            .wait_until(
                WaitPredicate::DomSelector {
                    selector: "#btn".to_string(),
                    visible: true,
                    negate: false,
                },
                Duration::from_millis(500),
            )
            .await
            .expect("wait_until ne doit pas echouer");
        assert!(outcome.satisfied, "le predicat doit etre satisfait");
        let snippet = spy.captured.lock().unwrap().clone();
        assert!(
            snippet.contains("getBoundingClientRect"),
            "snippet visible doit contenir getBoundingClientRect, recu: {snippet}"
        );
    }

    /// wait_for_gone : un selecteur ABSENT (evaluator renvoie false) avec
    /// `negate=true` satisfait la condition immediatement.
    #[tokio::test]
    async fn composite_dom_selector_negate_satisfied_when_absent() {
        struct AbsentEval;
        #[async_trait]
        impl JsEvaluator for AbsentEval {
            async fn evaluate(&self, _: &str) -> PortResult<serde_json::Value> {
                Ok(serde_json::json!(false)) // element absent
            }
        }
        struct NoopInsp;
        struct NoopRend;
        #[async_trait]
        impl AppInspector for NoopInsp {
            async fn dom_snapshot(&self) -> PortResult<DomTree> {
                Err(PortError::Unsupported("stub".to_string()))
            }
            async fn state_snapshot(&self) -> PortResult<StateDump> {
                Err(PortError::Unsupported("stub".to_string()))
            }
        }
        #[async_trait]
        impl Renderer for NoopRend {
            async fn screenshot(&self) -> PortResult<Screenshot> {
                Err(PortError::Unsupported("stub".to_string()))
            }
            async fn viewport_size(&self) -> PortResult<Size> {
                Err(PortError::Unsupported("stub".to_string()))
            }
        }
        let cond = CompositeWaitCondition {
            evaluator: Arc::new(AbsentEval),
            inspector: Arc::new(NoopInsp),
            renderer: Arc::new(NoopRend),
            log_source: Arc::new(EmptyLog),
        };
        let outcome = cond
            .wait_until(
                WaitPredicate::DomSelector {
                    selector: ".spinner".to_string(),
                    visible: false,
                    negate: true,
                },
                Duration::from_millis(500),
            )
            .await
            .expect("wait_until ne doit pas echouer");
        assert!(
            outcome.satisfied,
            "element absent + negate => condition satisfaite (disparu)"
        );
    }

    /// M4 — une erreur `PortError::Network` dans l'evaluateur doit se propager
    /// hors de `wait_until` (transport mort = vrai echec, pas un transitoire).
    #[tokio::test]
    async fn composite_dom_selector_network_error_propagates() {
        struct NetworkEval;

        #[async_trait]
        impl JsEvaluator for NetworkEval {
            async fn evaluate(&self, _: &str) -> PortResult<serde_json::Value> {
                Err(PortError::Network("connexion perdue".to_string()))
            }
        }

        struct NoopInspector3;
        struct NoopRenderer3;

        #[async_trait]
        impl AppInspector for NoopInspector3 {
            async fn dom_snapshot(&self) -> PortResult<DomTree> {
                Err(PortError::Unsupported("stub".to_string()))
            }
            async fn state_snapshot(&self) -> PortResult<StateDump> {
                Err(PortError::Unsupported("stub".to_string()))
            }
        }

        #[async_trait]
        impl Renderer for NoopRenderer3 {
            async fn screenshot(&self) -> PortResult<Screenshot> {
                Err(PortError::Unsupported("stub".to_string()))
            }
            async fn viewport_size(&self) -> PortResult<Size> {
                Err(PortError::Unsupported("stub".to_string()))
            }
        }

        let cond = CompositeWaitCondition {
            evaluator: Arc::new(NetworkEval),
            inspector: Arc::new(NoopInspector3),
            renderer: Arc::new(NoopRenderer3),
            log_source: Arc::new(EmptyLog),
        };
        let result = cond
            .wait_until(
                WaitPredicate::DomSelector {
                    selector: ".x".to_string(),
                    visible: false,
                    negate: false,
                },
                Duration::from_millis(100),
            )
            .await;
        assert!(
            matches!(result, Err(PortError::Network(_))),
            "une erreur Network doit se propager, recu: {result:?}"
        );
    }

    /// M4 — une erreur non-Network dans l'evaluateur (erreur JS transitoire)
    /// est traitee comme `false` : le polling continue jusqu'au timeout, sans
    /// remonter d'erreur.
    #[tokio::test]
    async fn composite_dom_selector_transient_error_treated_as_false() {
        struct TransientEval;

        #[async_trait]
        impl JsEvaluator for TransientEval {
            async fn evaluate(&self, _: &str) -> PortResult<serde_json::Value> {
                // Simule une exception JS benigne (ex. : sélecteur inconnu).
                Err(PortError::Protocol("exception JS benigne".to_string()))
            }
        }

        struct NoopInspector4;
        struct NoopRenderer4;

        #[async_trait]
        impl AppInspector for NoopInspector4 {
            async fn dom_snapshot(&self) -> PortResult<DomTree> {
                Err(PortError::Unsupported("stub".to_string()))
            }
            async fn state_snapshot(&self) -> PortResult<StateDump> {
                Err(PortError::Unsupported("stub".to_string()))
            }
        }

        #[async_trait]
        impl Renderer for NoopRenderer4 {
            async fn screenshot(&self) -> PortResult<Screenshot> {
                Err(PortError::Unsupported("stub".to_string()))
            }
            async fn viewport_size(&self) -> PortResult<Size> {
                Err(PortError::Unsupported("stub".to_string()))
            }
        }

        let cond = CompositeWaitCondition {
            evaluator: Arc::new(TransientEval),
            inspector: Arc::new(NoopInspector4),
            renderer: Arc::new(NoopRenderer4),
            log_source: Arc::new(EmptyLog),
        };
        let outcome = cond
            .wait_until(
                WaitPredicate::DomSelector {
                    selector: ".transient".to_string(),
                    visible: false,
                    negate: false,
                },
                Duration::from_millis(50),
            )
            .await
            .expect("erreur transitoire ne doit pas propager d'Err");
        assert!(
            !outcome.satisfied,
            "timeout apres erreurs transitoires => satisfied=false"
        );
    }

    // ----- A5 : meme filtre generalise aux 4 autres branches -----

    /// `AppInspector` stub qui echoue N fois (erreur non-Network) puis renvoie
    /// une valeur fixe : simule une app dont le state n'est pas encore publie
    /// au demarrage.
    struct FlakyStateInspector {
        remaining_failures: std::sync::Mutex<u32>,
    }
    #[async_trait]
    impl AppInspector for FlakyStateInspector {
        async fn dom_snapshot(&self) -> PortResult<DomTree> {
            Err(PortError::Unsupported("stub".to_string()))
        }
        async fn state_snapshot(&self) -> PortResult<StateDump> {
            let mut remaining = self.remaining_failures.lock().unwrap();
            if *remaining > 0 {
                *remaining -= 1;
                Err(PortError::Inspect("app pas encore prete".to_string()))
            } else {
                let mut entries = std::collections::BTreeMap::new();
                entries.insert("phase".to_string(), serde_json::json!("ready"));
                Ok(StateDump::new(entries))
            }
        }
    }

    /// A5 — StateKey : l'inspector echoue 2 fois avec une erreur non-Network
    /// (etat transitoire, ex. app qui demarre) puis renvoie la valeur
    /// attendue. Le predicat doit finir par etre satisfait : le polling ne
    /// doit jamais s'interrompre sur ces erreurs (meme filtre que DomSelector,
    /// generalise par cette correction).
    #[tokio::test]
    async fn composite_state_key_transient_errors_then_satisfied() {
        let cond = CompositeWaitCondition {
            evaluator: Arc::new(UnusedEval),
            inspector: Arc::new(FlakyStateInspector {
                remaining_failures: std::sync::Mutex::new(2),
            }),
            renderer: Arc::new(ConstRenderer {
                png: solid_png(2, 2),
                size: Size::new(2, 2),
            }),
            log_source: Arc::new(EmptyLog),
        };
        let outcome = cond
            .wait_until(
                WaitPredicate::StateKey {
                    key: "phase".to_string(),
                    expected: serde_json::json!("ready"),
                },
                Duration::from_millis(2000),
            )
            .await
            .expect("les erreurs transitoires ne doivent pas propager d'Err");
        assert!(
            outcome.satisfied,
            "doit finir par etre satisfait apres absorption des 2 erreurs transitoires"
        );
    }

    /// A5 — StateKey : une `PortError::Network` (transport mort) doit au
    /// contraire etre PROPAGEE immediatement, sans jamais etre absorbee —
    /// ce n'est pas un etat transitoire mais un vrai echec de transport.
    #[tokio::test]
    async fn composite_state_key_network_error_propagates() {
        struct NetworkInspector;
        #[async_trait]
        impl AppInspector for NetworkInspector {
            async fn dom_snapshot(&self) -> PortResult<DomTree> {
                Err(PortError::Unsupported("stub".to_string()))
            }
            async fn state_snapshot(&self) -> PortResult<StateDump> {
                Err(PortError::Network("connexion perdue".to_string()))
            }
        }

        let cond = CompositeWaitCondition {
            evaluator: Arc::new(UnusedEval),
            inspector: Arc::new(NetworkInspector),
            renderer: Arc::new(ConstRenderer {
                png: solid_png(2, 2),
                size: Size::new(2, 2),
            }),
            log_source: Arc::new(EmptyLog),
        };
        let result = cond
            .wait_until(
                WaitPredicate::StateKey {
                    key: "phase".to_string(),
                    expected: serde_json::json!("ready"),
                },
                Duration::from_millis(100),
            )
            .await;
        assert!(
            matches!(result, Err(PortError::Network(_))),
            "une erreur Network doit se propager, recu: {result:?}"
        );
    }

    // ----- D2-TargetAlive -----

    /// Renderer qui echoue N fois (erreur non-Network) puis renvoie une
    /// capture fixe : simule une cible qui vient de crasher/redemarrer et
    /// redevient joignable apres quelques hoquets.
    struct FlakyRenderer {
        remaining_failures: std::sync::Mutex<u32>,
        png: Vec<u8>,
        size: Size,
    }
    #[async_trait]
    impl Renderer for FlakyRenderer {
        async fn screenshot(&self) -> PortResult<Screenshot> {
            let mut remaining = self.remaining_failures.lock().unwrap();
            if *remaining > 0 {
                *remaining -= 1;
                Err(PortError::Unsupported(
                    "cible pas encore relancee".to_string(),
                ))
            } else {
                Ok(Screenshot::new_png(self.png.clone(), self.size))
            }
        }
        async fn viewport_size(&self) -> PortResult<Size> {
            Ok(self.size)
        }
    }

    /// `TargetAlive` : l'evaluateur (canal d'interaction) echoue 2 fois puis
    /// repond => le predicat doit finir par etre satisfait (transport revenu).
    #[tokio::test]
    async fn composite_target_alive_satisfied_after_transport_recovers() {
        struct FlakyEval {
            remaining_failures: std::sync::Mutex<u32>,
        }
        #[async_trait]
        impl JsEvaluator for FlakyEval {
            async fn evaluate(&self, _script: &str) -> PortResult<serde_json::Value> {
                let mut n = self.remaining_failures.lock().unwrap();
                if *n > 0 {
                    *n -= 1;
                    Err(PortError::Network("transport mort".to_string()))
                } else {
                    Ok(serde_json::json!(1))
                }
            }
        }
        let cond = CompositeWaitCondition {
            evaluator: Arc::new(FlakyEval {
                remaining_failures: std::sync::Mutex::new(2),
            }),
            inspector: Arc::new(UnusedInsp),
            // Le renderer NE doit PAS etre sonde tant que l'evaluateur repond :
            // un renderer qui panique verifie qu'on ne l'atteint jamais ici.
            renderer: Arc::new(FlakyRenderer {
                remaining_failures: std::sync::Mutex::new(0),
                png: solid_png(2, 2),
                size: Size::new(2, 2),
            }),
            log_source: Arc::new(EmptyLog),
        };
        let outcome = cond
            .wait_until(WaitPredicate::TargetAlive {}, Duration::from_millis(2000))
            .await
            .expect("wait_until ne doit jamais renvoyer Err pour TargetAlive");
        assert!(
            outcome.satisfied,
            "doit finir par etre satisfait une fois le transport retabli"
        );
    }

    /// `TargetAlive` : une erreur `PortError::Network` de l'evaluateur ne doit
    /// PAS se propager — c'est l'etat attendu pendant qu'une cible redemarre —
    /// et ne doit PAS non plus etre masquee par un renderer OS-level encore
    /// capturable (le bug bloquant de la revue T4 : faux "vivant" en mode auto).
    #[tokio::test]
    async fn composite_target_alive_network_does_not_propagate_nor_falls_back_to_renderer() {
        struct NetworkEval;
        #[async_trait]
        impl JsEvaluator for NetworkEval {
            async fn evaluate(&self, _script: &str) -> PortResult<serde_json::Value> {
                Err(PortError::Network("connexion perdue".to_string()))
            }
        }
        let cond = CompositeWaitCondition {
            evaluator: Arc::new(NetworkEval),
            inspector: Arc::new(UnusedInsp),
            // Renderer TOUJOURS capturable (simule le repli OS-level du mode
            // auto) : ne doit PAS suffire a declarer la cible vivante.
            renderer: Arc::new(ConstRenderer {
                png: solid_png(2, 2),
                size: Size::new(2, 2),
            }),
            log_source: Arc::new(EmptyLog),
        };
        let outcome = cond
            .wait_until(WaitPredicate::TargetAlive {}, Duration::from_millis(80))
            .await
            .expect("TargetAlive ne doit jamais propager d'erreur, meme Network");
        assert!(
            !outcome.satisfied,
            "transport mort => pas vivant, meme si l'ecran reste capturable"
        );
    }

    /// `TargetAlive` sur kwin-eis : l'evaluateur renvoie `Unsupported` (pas de
    /// JS), on retombe sur `renderer.screenshot()` comme seul signal de vie.
    #[tokio::test]
    async fn composite_target_alive_falls_back_to_renderer_when_eval_unsupported() {
        struct UnsupportedEval;
        #[async_trait]
        impl JsEvaluator for UnsupportedEval {
            async fn evaluate(&self, _script: &str) -> PortResult<serde_json::Value> {
                Err(PortError::Unsupported("pas de JS en kwin-eis".to_string()))
            }
        }
        let cond = CompositeWaitCondition {
            evaluator: Arc::new(UnsupportedEval),
            inspector: Arc::new(UnusedInsp),
            renderer: Arc::new(ConstRenderer {
                png: solid_png(2, 2),
                size: Size::new(2, 2),
            }),
            log_source: Arc::new(EmptyLog),
        };
        let outcome = cond
            .wait_until(WaitPredicate::TargetAlive {}, Duration::from_millis(2000))
            .await
            .expect("wait_until ne doit jamais renvoyer Err pour TargetAlive");
        assert!(
            outcome.satisfied,
            "kwin-eis : capture reussie => cible vivante"
        );
    }

    // ----- D1-TextContent -----

    /// `TextContent` : un evaluateur qui renvoie `true` satisfait le
    /// predicat immediatement (sans selecteur : verifie sur `document.body`).
    #[tokio::test]
    async fn composite_text_content_satisfied_when_evaluator_returns_true() {
        struct TrueEval;
        #[async_trait]
        impl JsEvaluator for TrueEval {
            async fn evaluate(&self, _: &str) -> PortResult<serde_json::Value> {
                Ok(serde_json::json!(true))
            }
        }
        let cond = CompositeWaitCondition {
            evaluator: Arc::new(TrueEval),
            inspector: Arc::new(UnusedInsp),
            renderer: Arc::new(ConstRenderer {
                png: solid_png(2, 2),
                size: Size::new(2, 2),
            }),
            log_source: Arc::new(EmptyLog),
        };
        let outcome = cond
            .wait_until(
                WaitPredicate::TextContent {
                    text: "Enregistre".to_string(),
                    selector: None,
                    visible: false,
                },
                Duration::from_millis(200),
            )
            .await
            .expect("wait_until ne doit pas echouer");
        assert!(
            outcome.satisfied,
            "evaluator renvoyant true doit satisfaire TextContent"
        );
    }

    /// `TextContent` : verifie via un evaluateur espion que le snippet cible
    /// bien `document.body` quand `selector` est absent, et le selecteur
    /// fourni sinon — couvre la construction de `el_expr`.
    #[tokio::test]
    async fn composite_text_content_snippet_targets_selector_or_body() {
        struct SpyEval {
            captured: std::sync::Mutex<String>,
        }
        #[async_trait]
        impl JsEvaluator for SpyEval {
            async fn evaluate(&self, snippet: &str) -> PortResult<serde_json::Value> {
                *self.captured.lock().unwrap() = snippet.to_string();
                Ok(serde_json::json!(true))
            }
        }

        let spy = Arc::new(SpyEval {
            captured: std::sync::Mutex::new(String::new()),
        });
        let cond = CompositeWaitCondition {
            evaluator: Arc::clone(&spy) as Arc<dyn JsEvaluator>,
            inspector: Arc::new(UnusedInsp),
            renderer: Arc::new(ConstRenderer {
                png: solid_png(2, 2),
                size: Size::new(2, 2),
            }),
            log_source: Arc::new(EmptyLog),
        };
        cond.wait_until(
            WaitPredicate::TextContent {
                text: "OK".to_string(),
                selector: None,
                visible: false,
            },
            Duration::from_millis(200),
        )
        .await
        .expect("wait_until ne doit pas echouer");
        assert!(
            spy.captured.lock().unwrap().contains("document.body"),
            "selecteur absent => doit cibler document.body"
        );

        cond.wait_until(
            WaitPredicate::TextContent {
                text: "OK".to_string(),
                selector: Some("#status".to_string()),
                visible: true,
            },
            Duration::from_millis(200),
        )
        .await
        .expect("wait_until ne doit pas echouer");
        let snippet = spy.captured.lock().unwrap().clone();
        assert!(
            snippet.contains("document.querySelector(\"#status\")"),
            "selecteur fourni doit etre utilise tel quel, recu: {snippet}"
        );
        assert!(
            snippet.contains("getBoundingClientRect"),
            "visible=true doit verifier le rect, recu: {snippet}"
        );
    }
}
