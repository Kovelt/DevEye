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

use deveye_core::{ConsoleError, DomTree, StateDump, WaitOutcome, WaitPredicate};
use deveye_ports::{
    AppInspector, ConsoleErrorSource, JsEvaluator, PortResult, Renderer, WaitCondition,
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
const CONSOLE_ERRORS_SNIPPET: &str = r#"(function(){
  if (!window.__deveye_console_init__) {
    window.__deveye_console_errors__ = [];
    var push = function(level, text, url, line){
      try { window.__deveye_console_errors__.push({ level: level, text: text, url: url, line: line, timestamp: new Date().toISOString() }); } catch (_) {}
    };
    window.addEventListener('error', function(e){
      push('error', String((e && (e.message || e.error)) || 'error'), (e && e.filename) || null, (e && e.lineno) || null);
    });
    window.addEventListener('unhandledrejection', function(e){
      push('error', String(e && e.reason), null, null);
    });
    var origErr = console.error;
    console.error = function(){ push('error', Array.prototype.map.call(arguments, String).join(' '), null, null); if (origErr) { origErr.apply(console, arguments); } };
    var origWarn = console.warn;
    console.warn = function(){ push('warning', Array.prototype.map.call(arguments, String).join(' '), null, null); if (origWarn) { origWarn.apply(console, arguments); } };
    window.__deveye_console_init__ = true;
  }
  return window.__deveye_console_errors__.splice(0);
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
        // Parse element par element : un element corrompu ne doit jamais faire
        // disparaitre les autres ("unwrap_or_default()" global = inversion du sens).
        let arr = match value {
            serde_json::Value::Array(a) => a,
            other => {
                eprintln!(
                    "deveye-server: drain_console_errors: valeur JS inattendue (non-tableau : {}), buffer ignore",
                    other
                );
                return Ok(Vec::new());
            }
        };
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
}

impl CompositeWaitCondition {
    async fn check(&self, predicate: &WaitPredicate) -> PortResult<bool> {
        match predicate {
            WaitPredicate::DomSelector { selector } => {
                let sel = serde_json::to_string(selector).unwrap_or_default();
                let value = self
                    .evaluator
                    .evaluate(&format!("!!document.querySelector({sel})"))
                    .await?;
                Ok(value.as_bool().unwrap_or(false))
            }
            WaitPredicate::StateKey { key, expected } => {
                let dump = self.inspector.state_snapshot().await?;
                Ok(dump.entries.get(key) == Some(expected))
            }
            WaitPredicate::ScreenshotStable { threshold_ms } => {
                // Plafonne a MAX_SCREENSHOT_STABLE_THRESHOLD_MS pour eviter une
                // pause demesure a chaque iteration (le timeout global du caller
                // borne la duree totale, pas chaque check individuel).
                let capped = (*threshold_ms).min(MAX_SCREENSHOT_STABLE_THRESHOLD_MS);
                let first = self.renderer.screenshot().await?;
                let hash_first = hash_screenshot_bytes(&first.bytes);
                tokio::time::sleep(Duration::from_millis(capped)).await;
                let second = self.renderer.screenshot().await?;
                let hash_second = hash_screenshot_bytes(&second.bytes);
                Ok(hash_first == hash_second)
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
    use deveye_core::{ConsoleLevel, Screenshot, Size};
    use deveye_ports::PortError;

    // Timestamp RFC3339 fixe utilise dans les donnees de test (evite le besoin
    // d'une dependance directe a `chrono` dans le binaire).
    const TS: &str = "2026-01-01T00:00:00Z";

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
        };
        let outcome = cond
            .wait_until(
                WaitPredicate::DomSelector {
                    selector: ".never".to_string(),
                },
                Duration::from_millis(50),
            )
            .await
            .expect("wait_until ne doit pas renvoyer Err");
        assert!(!outcome.satisfied, "le predicat ne peut etre satisfait");
    }
}
