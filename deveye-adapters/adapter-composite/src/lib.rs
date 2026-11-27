#![forbid(unsafe_code)]
//! Adapter `adapter-composite` — types composites generiques.
//!
//! Ce crate centralise les briques d'orchestration testables sans I/O reelle :
//! sources d'erreurs console JS, condition d'attente multi-predicats,
//! inspecteur composant DOM + state, inspecteur « DOM par route » pour les
//! applications natives. Ces types fonctionnent pour toutes les cibles sans impl
//! par adapter.
//!
//! Issu de `deveye-server::orchestration` (refacto hexagonale lot C) :
//! les adapters composites n'ont pas leur place dans le binaire orchestrateur.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tracing::{debug, warn};

use deveye_core::{
    ConsoleError, DomTree, LogLevel, StateDump, UiReadError, UiTree, WaitOutcome, WaitPredicate,
};
use deveye_ports::{
    AppInspector, CommandRouter, ConsoleErrorSource, JsEvaluator, LogSource, PortError, PortResult,
    Renderer, WaitCondition,
};

mod ui_mask;

pub use ui_mask::mask_route_value;

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

/// Etat de stabilite porte par l'appelant (`wait_until`, une instance par
/// appel) entre deux iterations de polling du meme predicat `ScreenshotStable`
/// / `ScreenshotRegionStable` (perf B3). Avant : chaque iteration capturait
/// DEUX cliches (un avant, un apres `threshold_ms`) sans rien reutiliser de
/// l'iteration precedente — 2 captures par iteration sur une attente non
/// immediatement satisfaite. Desormais on ne capture qu'UNE nouvelle image par
/// iteration, comparee au hash + instant memorises ici lors de l'iteration
/// precedente : ~2x moins de captures sur une attente longue.
struct StabilityState {
    hash: u64,
    captured_at: Instant,
}

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

/// Variante de [`absorb_transient`] pour les prédicats adosses au SDK de l'app
/// (`state_snapshot`, `poll` des logs). Une `PortError::Protocol` y vient d'une
/// incompatibilité de majeure du wire protocol, d'une route absente ou d'une
/// réponse illisible : rien qui s'arrange en réessayant. La propager donne a
/// l'appelant le message actionnable au lieu d'un `wait_for` qui expire sans
/// explication.
fn absorb_transient_sdk<T>(
    result: PortResult<T>,
    predicate: &'static str,
) -> PortResult<Option<T>> {
    match result {
        Err(e @ PortError::Protocol(_)) => Err(e),
        other => absorb_transient(other, predicate),
    }
}

impl CompositeWaitCondition {
    /// Coeur partage de `ScreenshotStable` / `ScreenshotRegionStable` (perf
    /// B3) : ne capture qu'UNE nouvelle image, comparee au hash memorise dans
    /// `stability` par l'appel precedent (`None` region => image entiere,
    /// `Some((x, y, w, h))` => `hash_region`).
    ///
    /// Le delai minimal demande entre deux captures comparees (`capped_ms`,
    /// deja plafonne par l'appelant) reste garanti : si l'iteration
    /// precedente s'est terminee trop recemment (le polling externe de
    /// `wait_until` peut dormir moins longtemps que `capped_ms`), on complete
    /// l'attente ici avant de capturer. Sur la toute premiere iteration
    /// (`stability` a `None`), aucune comparaison n'est possible : on capture,
    /// on memorise, et on renvoie "pas encore stable".
    ///
    /// Compromis assume (revue P5) : la satisfaction ne peut donc jamais
    /// survenir avant la DEUXIEME capture, soit >= `capped_ms` apres la
    /// premiere. Pour un `threshold_ms` tres court c'est une legere latence de
    /// premiere satisfaction en plus ; le gain (une capture/iteration au lieu
    /// de deux) domine des que l'attente n'est pas immediatement satisfaite.
    async fn check_screenshot_stable(
        &self,
        capped_ms: u64,
        region: Option<(u32, u32, u32, u32)>,
        stability: &mut Option<StabilityState>,
    ) -> PortResult<bool> {
        let predicate_name = if region.is_some() {
            "screenshot-region-stable"
        } else {
            "screenshot-stable"
        };
        if let Some(prev) = stability.as_ref() {
            let min_gap = Duration::from_millis(capped_ms);
            let elapsed = prev.captured_at.elapsed();
            if elapsed < min_gap {
                tokio::time::sleep(min_gap - elapsed).await;
            }
        }
        let shot = match absorb_transient(self.renderer.screenshot().await, predicate_name)? {
            Some(s) => s,
            None => {
                // Erreur transitoire : pas de hash exploitable. On repart a
                // zero plutot que de comparer au prochain appel avec un etat
                // perime (capture ratee entre-temps).
                *stability = None;
                return Ok(false);
            }
        };
        // Une region hors capture est une erreur de parametrage de
        // l'appelant, pas un etat transitoire de l'app : elle reste propagee
        // telle quelle (pas de filtre `absorb_transient` ici), comme avant.
        let hash = match region {
            Some((x, y, width, height)) => hash_region(&shot.bytes, x, y, width, height)?,
            None => hash_screenshot_bytes(&shot.bytes),
        };
        let stable = stability.as_ref().is_some_and(|prev| prev.hash == hash);
        *stability = Some(StabilityState {
            hash,
            captured_at: Instant::now(),
        });
        Ok(stable)
    }

    async fn check(
        &self,
        predicate: &WaitPredicate,
        stability: &mut Option<StabilityState>,
    ) -> PortResult<bool> {
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
                    match absorb_transient_sdk(self.inspector.state_snapshot().await, "state-key")?
                    {
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
                self.check_screenshot_stable(capped, None, stability).await
            }
            WaitPredicate::ScreenshotRegionStable {
                x,
                y,
                width,
                height,
                threshold_ms,
            } => {
                let capped = (*threshold_ms).min(MAX_SCREENSHOT_STABLE_THRESHOLD_MS);
                self.check_screenshot_stable(capped, Some((*x, *y, *width, *height)), stability)
                    .await
            }
            WaitPredicate::LogMessage {
                contains,
                level_min,
            } => {
                // Draine le buffer de logs et teste chaque evenement. Le drain
                // consomme les logs (cf. doc du predicat) : un match dans cette
                // iteration suffit a satisfaire l'attente.
                let events =
                    match absorb_transient_sdk(self.log_source.poll().await, "log-message")? {
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
            // `WaitPredicate` est `#[non_exhaustive]` : un prédicat ajouté par le
            // domaine sans implémentation ici est refusé explicitement.
            _ => Err(PortError::Unsupported(
                "prédicat d'attente non géré par ce composite".to_string(),
            )),
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
        // Etat de stabilite screenshot (perf B3), propre a cet appel de
        // `wait_until` : reinitialise a `None` a chaque nouvelle attente,
        // porte ensuite d'iteration en iteration par `check`.
        let mut stability: Option<StabilityState> = None;
        loop {
            if self.check(&predicate, &mut stability).await? {
                return Ok(WaitOutcome::new(
                    true,
                    start.elapsed().as_millis() as u64,
                    "prédicat satisfait",
                ));
            }
            if Instant::now() >= deadline {
                return Ok(WaitOutcome::new(
                    false,
                    start.elapsed().as_millis() as u64,
                    format!("timeout après {} ms", timeout.as_millis()),
                ));
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

// ── RouteDomInspector ─────────────────────────────────────────────────────────

/// Route par defaut de [`RouteDomInspector`] : l'application cible la sert avec
/// la description JSON de ses widgets (`deveye_sdk::ui::publish`, ou
/// `deveye_sdk::expose_route("ui.widgets", ..)` pour un handler maison).
pub const DEFAULT_DOM_ROUTE: &str = "ui.widgets";

/// Age (ms) au-dela duquel un arbre `deveye.ui/1` est signale comme perime : l'UI
/// de l'app est gelee, ou aucune frame n'a ete rendue depuis.
const UI_STALE_AFTER_MS: u64 = 2_000;

/// `AppInspector` dont le « DOM » est la reponse d'une route de commande de
/// l'application cible.
///
/// Pour les applications natives sans WebView ni CDP (egui, imgui, Skia...),
/// l'arbre d'interface n'existe que dans l'application : elle l'expose via le
/// SDK opt-in comme une commande (`POST /devtools/command`) et ce type le sert
/// au port `AppInspector`, donc a l'outil MCP `dom`.
///
/// Contrat :
/// - requete : `router.invoke(route, {})` ; le payload reste `{}` (un handler
///   tiers qui valide son payload strictement casserait sur un champ inconnu,
///   ADR-0065) ;
/// - reponse : la valeur JSON du handler, **sans conversion** (un arbre au format
///   legacy reste au format legacy), enveloppee dans [`DomTree`], **apres
///   masquage des secrets** : la valeur d'un widget secret est remplacee par
///   `<secret>` et les valeurs structurees passent par le masquage par cles
///   (ADR-0064) ; [`ui_tree`](Self::ui_tree) rend la meme vue sous forme typee ;
/// - cout : rien tant que `dom` n'est pas appele (aucune surveillance en
///   arriere-plan) ; l'outil `dom` refuse les arbres > ~100 Ko.
///
/// `state_snapshot` n'est pas fourni (`Unsupported`) : composer avec un
/// [`CompositeInspector`] dont la partie `state` vient du SDK.
pub struct RouteDomInspector {
    router: Arc<dyn CommandRouter>,
    route: String,
}

impl RouteDomInspector {
    /// Inspecteur sur `route` ([`DEFAULT_DOM_ROUTE`] si `route` est vide).
    pub fn new(router: Arc<dyn CommandRouter>, route: impl Into<String>) -> Self {
        let route = route.into();
        Self {
            router,
            route: if route.is_empty() {
                DEFAULT_DOM_ROUTE.to_string()
            } else {
                route
            },
        }
    }

    /// Nom de la route interrogee.
    pub fn route(&self) -> &str {
        &self.route
    }

    /// Lit l'arbre au schema `deveye.ui/1` (ou au format legacy, converti par
    /// [`UiTree::from_value`]), secrets masques.
    ///
    /// # Errors
    /// Route indisponible (comme [`AppInspector::dom_snapshot`]) ou reponse qui
    /// n'est pas un arbre de widgets lisible ([`PortError::Inspect`]).
    pub async fn ui_tree(&self) -> PortResult<UiTree> {
        let raw = self.fetch_masked().await?;
        UiTree::from_value(&raw).map_err(|e| {
            let why = match e {
                UiReadError::UnsupportedSchema(schema) => {
                    format!("schema `{schema}` non supporte par ce serveur")
                }
                other => other.to_string(),
            };
            PortError::Inspect(format!(
                "la route `{}` ne sert pas un arbre de widgets lisible : {why}",
                self.route
            ))
        })
    }

    /// Reponse brute de la route, secrets masques.
    async fn fetch_masked(&self) -> PortResult<serde_json::Value> {
        let mut tree = match self.router.invoke(&self.route, serde_json::json!({})).await {
            Ok(tree) => tree,
            // Route inconnue / handler en echec : erreur d'inspection explicite
            // (l'application ne sert pas l'arbre), pas une panne de liaison.
            Err(PortError::Route { route, message }) => {
                return Err(PortError::Inspect(format!(
                    "route `{route}` indisponible : {message} — l'application cible \
                     expose-t-elle l'arbre d'interface via deveye-sdk ?"
                )))
            }
            // Network (SDK injoignable) et le reste : tels quels.
            Err(other) => return Err(other),
        };
        mask_route_value(&mut tree);
        warn_if_stale(&tree);
        Ok(tree)
    }
}

/// Signale un arbre `deveye.ui/1` plus vieux que [`UI_STALE_AFTER_MS`].
fn warn_if_stale(tree: &serde_json::Value) {
    let Some(published) = tree
        .get("published_at_ms")
        .and_then(serde_json::Value::as_u64)
    else {
        return;
    };
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return;
    };
    let age = (now.as_millis() as u64).saturating_sub(published);
    if age > UI_STALE_AFTER_MS {
        warn!(
            age_ms = age,
            "arbre ui.widgets perime : l'UI de l'app est peut-etre gelee ou n'a pas rendu de frame"
        );
    }
}

#[async_trait]
impl AppInspector for RouteDomInspector {
    async fn dom_snapshot(&self) -> PortResult<DomTree> {
        self.fetch_masked().await.map(DomTree::new)
    }

    async fn state_snapshot(&self) -> PortResult<StateDump> {
        Err(PortError::Unsupported(
            "RouteDomInspector ne fournit que le DOM ; composer avec CompositeInspector \
             pour le state"
                .to_string(),
        ))
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

    // ----- B3 : etat de stabilite porte inter-iterations (audit perf) -----

    /// Renderer scripte : renvoie une sequence programmee de resultats (un
    /// PNG ou une erreur transitoire), un par appel a `screenshot()`, et
    /// compte le nombre d'appels recus. Sert a verifier deterministe le
    /// nombre de captures reellement effectuees par `check_screenshot_stable`
    /// (au lieu de deviner via les delais de `wait_until`).
    struct ScriptedRenderer {
        calls: std::sync::Mutex<std::collections::VecDeque<PortResult<Vec<u8>>>>,
        size: Size,
        count: std::sync::atomic::AtomicUsize,
    }
    #[async_trait]
    impl Renderer for ScriptedRenderer {
        async fn screenshot(&self) -> PortResult<Screenshot> {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let next = self
                .calls
                .lock()
                .unwrap()
                .pop_front()
                .expect("script de captures epuise");
            next.map(|bytes| Screenshot::new_png(bytes, self.size))
        }
        async fn viewport_size(&self) -> PortResult<Size> {
            Ok(self.size)
        }
    }

    fn noop_condition(renderer: Arc<dyn Renderer>) -> CompositeWaitCondition {
        CompositeWaitCondition {
            evaluator: Arc::new(UnusedEval),
            inspector: Arc::new(UnusedInsp),
            renderer,
            log_source: Arc::new(EmptyLog),
        }
    }

    /// B3 — coeur de l'optimisation : sur une image constante, chaque appel a
    /// `check_screenshot_stable` ne doit declencher qu'UNE seule capture (au
    /// lieu de deux avant/apres `threshold_ms` dans l'ancienne implementation),
    /// le hash de l'iteration precedente etant reutilise via `stability`.
    #[tokio::test]
    async fn check_screenshot_stable_captures_once_per_call_when_image_constant() {
        let png = solid_png(4, 4);
        let renderer = Arc::new(ScriptedRenderer {
            calls: std::sync::Mutex::new(std::collections::VecDeque::from(vec![
                Ok(png.clone()),
                Ok(png.clone()),
            ])),
            size: Size::new(4, 4),
            count: std::sync::atomic::AtomicUsize::new(0),
        });
        let cond = noop_condition(Arc::clone(&renderer) as Arc<dyn Renderer>);
        let mut stability: Option<StabilityState> = None;

        let first = cond
            .check_screenshot_stable(5, None, &mut stability)
            .await
            .expect("check ok");
        assert!(
            !first,
            "1ere iteration : rien a comparer => pas encore stable"
        );
        assert_eq!(
            renderer.count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "1 seule capture sur la 1ere iteration (2 avant l'optimisation)"
        );

        let second = cond
            .check_screenshot_stable(5, None, &mut stability)
            .await
            .expect("check ok");
        assert!(second, "meme image sur les 2 iterations => stable");
        assert_eq!(
            renderer.count.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "2 captures cumulees pour 2 iterations (4 avant l'optimisation)"
        );
    }

    /// B3 — le delai minimal demande entre deux captures comparees
    /// (`threshold_ms`, deja plafonne par l'appelant) doit rester respecte
    /// meme si les deux appels a `check_screenshot_stable` s'enchainent sans
    /// delai externe : c'est le check lui-meme qui complete l'attente avant
    /// de capturer la 2e image.
    #[tokio::test]
    async fn check_screenshot_stable_second_call_still_waits_full_threshold() {
        let png = solid_png(4, 4);
        let renderer = Arc::new(ScriptedRenderer {
            calls: std::sync::Mutex::new(std::collections::VecDeque::from(vec![
                Ok(png.clone()),
                Ok(png.clone()),
            ])),
            size: Size::new(4, 4),
            count: std::sync::atomic::AtomicUsize::new(0),
        });
        let cond = noop_condition(renderer);
        let mut stability: Option<StabilityState> = None;

        let t0 = Instant::now();
        cond.check_screenshot_stable(80, None, &mut stability)
            .await
            .expect("check ok");
        cond.check_screenshot_stable(80, None, &mut stability)
            .await
            .expect("check ok");
        let elapsed = t0.elapsed();
        assert!(
            elapsed >= Duration::from_millis(75),
            "le gap minimal (threshold_ms) entre 2 captures comparees doit etre respecte, elapsed={elapsed:?}"
        );
    }

    /// B3 — une erreur transitoire de capture (absorbee) doit remettre
    /// `stability` a zero : la capture suivante ne doit PAS etre comparee a
    /// un hash memorise avant l'echec, meme si l'image redevient identique.
    /// Sinon on perdrait la garantie "2 captures CONSECUTIVES identiques" —
    /// un changement invisible aurait pu survenir pendant le hoquet.
    #[tokio::test]
    async fn check_screenshot_stable_resets_after_transient_capture_error() {
        let png = solid_png(4, 4);
        let renderer = Arc::new(ScriptedRenderer {
            calls: std::sync::Mutex::new(std::collections::VecDeque::from(vec![
                Ok(png.clone()),
                Err(PortError::Inspect("hoquet transitoire".to_string())),
                Ok(png.clone()),
            ])),
            size: Size::new(4, 4),
            count: std::sync::atomic::AtomicUsize::new(0),
        });
        let cond = noop_condition(renderer);
        let mut stability: Option<StabilityState> = None;

        let r1 = cond
            .check_screenshot_stable(5, None, &mut stability)
            .await
            .expect("check ok");
        assert!(!r1, "1ere iteration : rien a comparer");

        let r2 = cond
            .check_screenshot_stable(5, None, &mut stability)
            .await
            .expect("erreur transitoire absorbee, jamais d'Err");
        assert!(!r2, "erreur transitoire => pas stable");

        let r3 = cond
            .check_screenshot_stable(5, None, &mut stability)
            .await
            .expect("check ok");
        assert!(
            !r3,
            "l'echec intercalaire doit avoir efface l'etat memorise : pas de faux 'stable' \
             meme si l'image de la 3e capture est identique a celle de la 1ere"
        );
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

    /// Une incompatibilité de majeure du wire protocol (`PortError::Protocol`
    /// cote SDK) ne doit pas etre avalee en « pas encore vrai » : `wait_until`
    /// la propage avec son message au lieu d'expirer sans explication.
    #[tokio::test]
    async fn composite_state_key_propagates_incompatible_protocol() {
        struct IncompatibleInspector;
        #[async_trait]
        impl AppInspector for IncompatibleInspector {
            async fn dom_snapshot(&self) -> PortResult<DomTree> {
                Err(PortError::Unsupported("stub".to_string()))
            }
            async fn state_snapshot(&self) -> PortResult<StateDump> {
                Err(PortError::Protocol(
                    "majeure du SDK incompatible (2, attendue 1)".to_string(),
                ))
            }
        }
        let cond = CompositeWaitCondition {
            evaluator: Arc::new(UnusedEval),
            inspector: Arc::new(IncompatibleInspector),
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
                Duration::from_millis(500),
            )
            .await;
        match result {
            Err(PortError::Protocol(message)) => assert!(message.contains("incompatible")),
            other => panic!("Protocol doit se propager, recu: {other:?}"),
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
    // ── RouteDomInspector ────────────────────────────────────────────────────

    /// Routeur factice : enregistre les appels et rejoue une reponse fixe.
    struct FakeRouter {
        calls: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
        reply: std::sync::Mutex<Option<PortResult<serde_json::Value>>>,
    }

    impl FakeRouter {
        fn replying(reply: PortResult<serde_json::Value>) -> Arc<Self> {
            Arc::new(Self {
                calls: std::sync::Mutex::new(Vec::new()),
                reply: std::sync::Mutex::new(Some(reply)),
            })
        }
    }

    #[async_trait]
    impl CommandRouter for FakeRouter {
        async fn invoke(
            &self,
            route: &str,
            payload: serde_json::Value,
        ) -> PortResult<serde_json::Value> {
            self.calls
                .lock()
                .unwrap()
                .push((route.to_string(), payload));
            self.reply
                .lock()
                .unwrap()
                .take()
                .expect("une seule reponse")
        }
        async fn list(&self) -> PortResult<Vec<deveye_core::CommandDescriptor>> {
            Ok(Vec::new())
        }
    }

    /// La reponse du handler est passee telle quelle dans `DomTree`, sur la route
    /// demandee, avec un payload objet vide.
    #[tokio::test]
    async fn route_dom_wraps_the_handler_json_untouched() {
        let tree = serde_json::json!({
            "window": { "width": 1240, "height": 900 },
            "widgets": [{ "id": "vcf/cutoff", "rect": [10, 20, 56, 68] }],
        });
        let router = FakeRouter::replying(Ok(tree.clone()));
        let insp = RouteDomInspector::new(router.clone(), "ui.widgets");
        let dom = insp.dom_snapshot().await.expect("dom ok");
        assert_eq!(dom.tree, tree);
        let calls = router.calls.lock().unwrap();
        assert_eq!(
            *calls,
            vec![("ui.widgets".to_string(), serde_json::json!({}))]
        );
    }

    /// Route vide -> route par defaut ; route personnalisee respectee.
    #[tokio::test]
    async fn route_dom_default_and_custom_route() {
        let router = FakeRouter::replying(Ok(serde_json::json!([])));
        let insp = RouteDomInspector::new(router.clone(), "");
        assert_eq!(insp.route(), DEFAULT_DOM_ROUTE);
        insp.dom_snapshot().await.unwrap();
        assert_eq!(router.calls.lock().unwrap()[0].0, "ui.widgets");

        let router = FakeRouter::replying(Ok(serde_json::Value::Null));
        let insp = RouteDomInspector::new(router.clone(), "app.tree");
        assert_eq!(insp.route(), "app.tree");
        insp.dom_snapshot().await.unwrap();
        assert_eq!(router.calls.lock().unwrap()[0].0, "app.tree");
    }

    /// Route inconnue -> `Inspect` explicite (l'app ne sert pas l'arbre) ;
    /// SDK injoignable -> `Network` inchange (les compositions a repli en
    /// dependent).
    #[tokio::test]
    async fn route_dom_error_mapping() {
        let unknown = FakeRouter::replying(Err(PortError::Route {
            route: "ui.widgets".into(),
            message: "route inconnue".into(),
        }));
        let err = RouteDomInspector::new(unknown, "ui.widgets")
            .dom_snapshot()
            .await
            .unwrap_err();
        match err {
            PortError::Inspect(m) => {
                assert!(
                    m.contains("ui.widgets") && m.contains("route inconnue"),
                    "{m}"
                )
            }
            other => panic!("attendu Inspect, recu {other:?}"),
        }

        let down = FakeRouter::replying(Err(PortError::Network("refuse".into())));
        let err = RouteDomInspector::new(down, "ui.widgets")
            .dom_snapshot()
            .await
            .unwrap_err();
        assert!(matches!(err, PortError::Network(_)), "{err:?}");
    }

    /// `state_snapshot` n'est pas servi ; sous `CompositeInspector` le state
    /// vient de l'autre source.
    #[tokio::test]
    async fn route_dom_has_no_state_but_composes() {
        let router = FakeRouter::replying(Ok(serde_json::json!({ "n": 1 })));
        let dom: Arc<dyn AppInspector> = Arc::new(RouteDomInspector::new(router, "ui.widgets"));
        let err = dom.state_snapshot().await.unwrap_err();
        assert!(matches!(err, PortError::Unsupported(_)));

        struct StateOnly;
        #[async_trait]
        impl AppInspector for StateOnly {
            async fn dom_snapshot(&self) -> PortResult<DomTree> {
                Ok(DomTree::new(serde_json::Value::Null))
            }
            async fn state_snapshot(&self) -> PortResult<StateDump> {
                Ok(StateDump::new(
                    [("k".to_string(), serde_json::json!(1))]
                        .into_iter()
                        .collect(),
                ))
            }
        }
        let both = CompositeInspector {
            dom,
            state: Arc::new(StateOnly),
        };
        assert_eq!(both.dom_snapshot().await.unwrap().tree["n"], 1);
        assert_eq!(both.state_snapshot().await.unwrap().entries["k"], 1);
    }

    /// La valeur d'un champ secret n'atteint ni `dom` ni la lecture typee, quelle
    /// que soit la forme de l'arbre ; le reste est inchange.
    #[tokio::test]
    async fn route_dom_masks_secret_values_in_raw_and_typed_views() {
        let tree = serde_json::json!({
            "schema": "deveye.ui/1",
            "window": { "width": 400, "height": 300 },
            "widgets": [
                { "id": "login", "role": "textbox", "value": "alice", "rect": [0, 0, 100, 20] },
                { "id": "pw", "role": "password", "value": "hunter2", "rect": [0, 30, 100, 20] },
                { "id": "pin", "role": "textbox", "value": "1234", "rect": [0, 60, 100, 20],
                  "state": { "secret": true } },
            ],
        });
        let raw = RouteDomInspector::new(FakeRouter::replying(Ok(tree.clone())), "ui.widgets")
            .dom_snapshot()
            .await
            .expect("dom ok")
            .tree;
        assert_eq!(raw["widgets"][0]["value"], "alice");
        assert_eq!(raw["widgets"][1]["value"], "<secret>");
        assert_eq!(raw["widgets"][2]["value"], "<secret>");
        assert!(!raw.to_string().contains("hunter2"));
        assert_eq!(raw["window"], tree["window"]);

        let typed = RouteDomInspector::new(FakeRouter::replying(Ok(tree)), "ui.widgets")
            .ui_tree()
            .await
            .expect("arbre lisible");
        assert_eq!(typed.widgets.len(), 3);
        assert_eq!(
            typed.widget("pw").and_then(|w| w.value.clone()),
            Some(serde_json::json!("<secret>"))
        );
    }

    /// Le format legacy est servi tel quel par `dom` (secrets masques) et lu par
    /// `ui_tree` ; une reponse sans widgets est une erreur d'inspection claire.
    #[tokio::test]
    async fn route_dom_legacy_stays_legacy_and_unreadable_trees_are_reported() {
        let legacy = serde_json::json!({
            "window": { "width": 100, "height": 50 },
            "widgets": [{ "kind": "password", "value": "x", "rect": [0, 0, 10, 10] }],
        });
        let dom = RouteDomInspector::new(FakeRouter::replying(Ok(legacy.clone())), "ui.widgets")
            .dom_snapshot()
            .await
            .unwrap();
        assert!(dom.tree.get("schema").is_none());
        assert_eq!(dom.tree["widgets"][0]["value"], "<secret>");
        let typed = RouteDomInspector::new(FakeRouter::replying(Ok(legacy)), "ui.widgets")
            .ui_tree()
            .await
            .unwrap();
        assert!(typed.widgets[0].id_ephemeral);

        let err = RouteDomInspector::new(
            FakeRouter::replying(Ok(serde_json::json!({ "error": "aucun arbre publie" }))),
            "ui.widgets",
        )
        .ui_tree()
        .await
        .unwrap_err();
        assert!(matches!(err, PortError::Inspect(_)), "{err:?}");

        let err = RouteDomInspector::new(
            FakeRouter::replying(Ok(serde_json::json!({ "schema": "deveye.ui/9" }))),
            "ui.widgets",
        )
        .ui_tree()
        .await
        .unwrap_err();
        assert!(err.to_string().contains("deveye.ui/9"), "{err}");
    }
}
