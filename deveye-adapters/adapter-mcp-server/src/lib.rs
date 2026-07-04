//! Adapter DevEye qui expose les ports `Renderer`, `AppInspector`,
//! `InteractionSink` et `LogSource` au format Model Context Protocol via
//! `rmcp` (transport stdio).
//!
//! Quarante-et-un outils sont publiés à Claude (ou à n'importe quel client MCP) :
//! - `screenshot` : capture le viewport en PNG, renvoie le base64
//! - `screenshot_region` : capture puis recadre sur (x, y, width, height)
//! - `screenshot_element` : capture recadrée sur le rect d'un sélecteur CSS
//! - `assert_visual` : compare le viewport (ou une région) à un PNG baseline
//!   (golden) ; crée la baseline si absente
//! - `click` : clic à `(x, y)` (bouton gauche)
//! - `double_click` : double-clic gauche à `(x, y)`
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
//! - `press_key` : presse une touche unique (Enter, Tab, Escape, ArrowDown…)
//! - `key_combo` : combinaison avec modificateurs (Ctrl+S, Ctrl+Shift+P…)
//! - `navigate` : navigue vers une route logique (routeur du SDK opt-in)
//! - `scroll` : défile de (dx, dy) pixels, ancré (centre par défaut)
//! - `handle_dialog` : répond à un dialog JS (alert/confirm/prompt) — CDP
//! - `upload_file` : renseigne un input[type=file] (DOM.setFileInputFiles) — CDP
//! - `focus_window` : donne le focus OS à une fenêtre (KWin) — kwin-eis/sdk
//! - `clipboard_get` / `clipboard_set` : presse-papiers OS-level (Wayland)
//! - `dom` : sérialise le DOM de l'app cible ; garde-fou (B1) : erreur
//!   explicite au-delà d'environ 100 Ko sérialisés, plutôt que de saturer le
//!   contexte de l'agent (cibler via find_element/assert_selector/eval_js)
//! - `state` : sérialise le state opt-in poussé par le SDK ; `keys` filtre
//!   optionnellement un sous-ensemble de clés (B7)
//! - `assert_state` : vérifie des clés du state SDK (`isError` si divergence)
//! - `logs_tail` : draine et renvoie le buffer de logs accumulé, sous la
//!   forme `{ events, truncated, total }` ; `limit` (défaut 200, B4) borne
//!   `events` aux plus récents sans perdre le principe de drain
//! - `invoke_command` : invoque une commande logique exposée par l'app
//!   (route déclarée via `deveye_sdk::expose_route`)
//! - `list_commands` : liste les commandes logiques exposées par l'app
//! - `wait_for_selector` : attend qu'un sélecteur CSS soit présent/visible
//!   (sans sleep arbitraire)
//! - `wait_for_text` : attend qu'un texte apparaisse dans le DOM (document
//!   entier ou un sélecteur donné)
//! - `find_element` : inspecte un élément (tag, texte, rect, visible, disabled,
//!   attributs, `value_prop` pour les champs de saisie)
//! - `assert_selector` : compte les éléments d'un sélecteur (`{ ok, count }`)
//! - `drag_drop` : glisser-déposer de (from) vers (to)
//! - `collect_console_errors` : draine les messages console de la page, tous
//!   niveaux confondus (onerror, unhandledrejection, console.*) ; `levels`
//!   filtre optionnellement par sévérité
//! - `list_windows` : liste les fenêtres/onglets de la cible (CDP et kwin-eis
//!   via KWin Scripting ; vide en SDK)
//! - `window_geometry` : géométrie écran `{x, y, width, height}` d'une fenêtre
//!   (KWin Scripting ; `Unsupported` hors KDE/Wayland)
//! - `wait_condition` : attend un prédicat (dom-selector / state-key /
//!   screenshot-stable / screenshot-region-stable / log-message /
//!   target-alive / text-content) avec timeout
//! - `wait_for_log` : attend un log applicatif (contains + level_min)
//! - `wait_screenshot_region` : attend la stabilité visuelle d'une région
//! - `wait_target_alive` : attend qu'une cible crashée/relancée redevienne
//!   joignable (raccourci du prédicat `target-alive`)
//! - `wait_ready` : attend que l'app publie un state non vide (app prête)
//! - `target_info` : décrit la cible active et ses capacités (eval_js, dom…)
//!
//! `wait_for_selector` accepte aussi `negate` (attendre la disparition d'un
//! élément — « wait_for_gone »).
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
use tracing::debug;

use deveye_core::{InteractionCommand, LogLevel, MouseButton, Position, WaitPredicate};
use deveye_ports::{
    AppInspector, Clipboard, CommandRouter, ConsoleErrorSource, InteractionSink, JsEvaluator,
    LogSource, PortError, Renderer, WaitCondition, WindowGeometry, WindowLister, WindowManager,
};

/// Plafond partage par les outils `wait_*` a boucle de polling explicite
/// (`wait_condition`, `wait_for_log`, `wait_screenshot_region` l'appliquent
/// deja en local) : borne a 60 s le temps de blocage cote agent MCP, quel que
/// soit le `timeout_ms` demande. `wait_ready` (A5) etait le seul sans plafond.
const WAIT_TIMEOUT_MAX_MS: u64 = 60_000;

/// B1 : garde-fou de l'outil `dom` — au-dela de ce nombre d'octets JSON
/// serialises, l'arbre DOM complet n'est plus renvoye (erreur MCP explicite)
/// pour eviter de saturer le contexte de l'agent LLM (chaque octet de sortie
/// coute des tokens). L'agent est alors invite a cibler via
/// `find_element`/`assert_selector`/`eval_js` plutot que de dumper tout
/// l'arbre. Raffinement futur (hors perimetre de ce sprint) : parametres
/// `selector`/`max_depth` cote CDP (`DOM.querySelector` +
/// `DOM.describeNode(depth=N)`) pour ne demander que le sous-arbre utile.
const DOM_MAX_BYTES: usize = 100_000;

/// Description statique de la cible active et de ses capacites, renvoyee par
/// l'outil `target_info`. Construite par `deveye-server` (compose) selon le
/// `--target` choisi : permet a l'agent de savoir d'avance quels outils
/// repondront (ex. `eval_js`/`dom` absents en kwin-eis) sans tenter un appel
/// pour le decouvrir.
#[derive(Clone, Debug, Serialize)]
pub struct TargetInfo {
    /// Mode actif : "auto" | "cdp" | "sdk" | "kwin-eis".
    pub target: String,
    /// Comment les yeux (screenshot) sont obtenus.
    pub eyes: String,
    /// Comment les mains (clic/saisie) agissent.
    pub hands: String,
    /// `eval_js` / `dom` / `find_element` / `wait_for_selector` disponibles ?
    pub eval_js: bool,
    /// Le DOM est inspectable (`dom`, `find_element`) ?
    pub dom: bool,
    /// Le pont SDK (state/logs/commandes) est cable ? (peut renvoyer vide si le
    /// SDK/shim de l'app n'est pas lance).
    pub state: bool,
    /// Les erreurs console JS sont collectables (`collect_console_errors`) ?
    pub console_errors: bool,
    /// `list_windows` renvoie des fenetres (CDP uniquement) ?
    pub list_windows: bool,
    /// Notes libres : pieges, prerequis, capacites indisponibles.
    pub notes: String,
}

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
    pub window_manager: Arc<dyn WindowManager>,
    pub window_geometry: Arc<dyn WindowGeometry>,
    pub clipboard: Arc<dyn Clipboard>,
    pub target_info: TargetInfo,
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
    window_manager: Arc<dyn WindowManager>,
    window_geometry: Arc<dyn WindowGeometry>,
    clipboard: Arc<dyn Clipboard>,
    target_info: TargetInfo,
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

/// A8 : garde du gate instrumentee — la duree de detention n'est mesurable
/// qu'a la liberation (`Drop`), le mutex lui-meme ne portant aucun hook. Sans
/// cette trace, un gel (cf. A1-A4) ne permet pas de savoir quel outil tenait
/// le gate au moment du blocage.
struct GateGuard<'a> {
    _inner: tokio::sync::MutexGuard<'a, ()>,
    outil: &'static str,
    acquired_at: Instant,
}

impl Drop for GateGuard<'_> {
    fn drop(&mut self) {
        let duree_ms = self.acquired_at.elapsed().as_millis() as u64;
        debug!(outil = self.outil, duree_ms, "gate libere");
    }
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
struct PressKeyParams {
    /// Nom de la touche à presser (keydown + keyup). En CDP, noms W3C
    /// (`Enter`, `Tab`, `Escape`, `ArrowDown`, `F5`…). En kwin-eis, noms XKB
    /// (`Return`, `Down`…). Les noms communs (`Tab`, `Escape`, `F5`, `space`)
    /// passent sur les deux.
    key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct KeyComboParams {
    /// Modificateurs maintenus pendant la frappe (insensibles à la casse) :
    /// `ctrl`/`control`, `shift`, `alt`, `meta`/`super`/`cmd`.
    modifiers: Vec<String>,
    /// Touche principale. En CDP, nom W3C (`KeyZ`, `KeyS`, `Tab`…) ou caractère
    /// (`z`, `s`) ; en kwin-eis, nom XKB (`z`, `s`, `Tab`…).
    key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct NavigateParams {
    /// Route logique à atteindre via le routeur exposé par le SDK opt-in
    /// (`window.__deveye_router__`). Indisponible en kwin-eis (pas de routeur
    /// applicatif OS-level).
    route: String,
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
    /// Si true, inverse la condition : attend que l'élément DISPARAISSE
    /// (absent du DOM, ou non visible si `visible` est aussi vrai). C'est le
    /// mode « wait_for_gone » : attendre la disparition d'un spinner, d'un
    /// toast ou d'une modale après une action.
    #[serde(default)]
    negate: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WaitForTextParams {
    /// Texte à attendre dans l'`innerText`/`textContent` de l'élément ciblé.
    text: String,
    /// Sélecteur CSS de l'élément à observer. Absent : le document entier
    /// (`document.body`).
    #[serde(default)]
    selector: Option<String>,
    /// Si true, l'élément doit en plus être visible (rect non vide).
    #[serde(default)]
    visible: Option<bool>,
    /// Timeout en millisecondes (défaut 5000, plafond 30000).
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WaitTargetAliveParams {
    /// Timeout en millisecondes (défaut 5000, plafond 60000).
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CollectConsoleErrorsParams {
    /// Filtre par sévérité (`"log"`/`"info"`/`"warning"`/`"error"`, insensible
    /// à la casse). Absent ou vide : tous les niveaux drainés sont renvoyés.
    #[serde(default)]
    levels: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WindowGeometryParams {
    /// Sous-chaîne du titre de la fenêtre (insensible à la casse). Absent :
    /// la fenêtre `--screencap-window` de la session.
    #[serde(default)]
    title: Option<String>,
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
struct ScrollParams {
    /// Abscisse d'ancrage (pixels viewport). Défaut : centre du viewport.
    #[serde(default)]
    x: Option<i32>,
    /// Ordonnée d'ancrage. Défaut : centre du viewport.
    #[serde(default)]
    y: Option<i32>,
    /// Défilement horizontal en pixels (positif = droite). Défaut 0.
    #[serde(default)]
    dx: Option<i32>,
    /// Défilement vertical en pixels (positif = bas). Défaut 0.
    #[serde(default)]
    dy: Option<i32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct HandleDialogParams {
    /// `true` pour accepter (OK), `false` pour annuler (Cancel) le dialog JS.
    accept: bool,
    /// Texte à saisir pour un `prompt()` (ignoré pour `alert`/`confirm`).
    #[serde(default)]
    prompt_text: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UploadFileParams {
    /// Sélecteur CSS de l'`input[type=file]` cible.
    selector: String,
    /// Chemins absolus des fichiers à renseigner.
    paths: Vec<String>,
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

#[derive(Debug, Deserialize, JsonSchema)]
struct WaitReadyParams {
    /// Délai maximal d'attente, en millisecondes.
    timeout_ms: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ScreenshotRegionParams {
    /// Abscisse du coin haut-gauche de la région (pixels du screenshot).
    x: u32,
    /// Ordonnée du coin haut-gauche.
    y: u32,
    /// Largeur de la région.
    width: u32,
    /// Hauteur de la région.
    height: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AssertVisualParams {
    /// Chemin du PNG de référence (baseline). S'il n'existe pas, il est créé à
    /// partir de la capture courante (`{ created: true }`) ; sinon la capture
    /// est comparée pixel à pixel à ce fichier.
    baseline_path: String,
    /// Ratio maximal de pixels différents toléré (0.0–1.0). Défaut 0.01 (1 %).
    #[serde(default)]
    threshold: Option<f64>,
    /// Région optionnelle à comparer (sinon tout le viewport) : x, y.
    #[serde(default)]
    x: Option<u32>,
    #[serde(default)]
    y: Option<u32>,
    /// Région optionnelle : largeur, hauteur.
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AssertStateParams {
    /// Objet JSON `{ clé: valeur }` : chaque clé doit exister dans le state SDK
    /// avec exactement cette valeur. `isError` si au moins une diverge.
    expected: serde_json::Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ScreenshotElementParams {
    /// Sélecteur CSS de l'élément à capturer (le premier match). L'élément est
    /// défilé au centre, puis la capture est recadrée sur son `BoundingClientRect`.
    selector: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ClipboardSetParams {
    /// Texte à placer dans le presse-papiers.
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FocusWindowParams {
    /// Sous-chaîne du titre de la fenêtre à activer (insensible à la casse).
    /// Absent : la fenêtre `--screencap-window` de la session.
    #[serde(default)]
    title_pattern: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WaitForLogParams {
    /// Sous-chaîne à chercher dans le message des logs `tracing` de l'app.
    contains: String,
    /// Niveau minimal (`trace`/`debug`/`info`/`warn`/`error`). Défaut : tous.
    #[serde(default)]
    level_min: Option<String>,
    /// Timeout en millisecondes (défaut 5000, plafond 60000).
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WaitScreenshotRegionParams {
    /// Région à surveiller : x, y du coin haut-gauche (pixels de la capture).
    x: u32,
    y: u32,
    /// Région : largeur, hauteur.
    width: u32,
    height: u32,
    /// Délai inter-captures pour juger de la stabilité (défaut 300, plafond 2000).
    #[serde(default)]
    threshold_ms: Option<u64>,
    /// Timeout total en millisecondes (défaut 5000, plafond 60000).
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LogsTailParams {
    /// Nombre maximal d'événements les plus RÉCENTS à renvoyer (défaut 200).
    /// Le drain côté source reste complet (rien n'est laissé dans le buffer
    /// ni perdu silencieusement) : `total` indique le nombre réellement
    /// drainé, `truncated` signale si `total > limit`.
    #[serde(default)]
    limit: Option<usize>,
}

/// B4 : défaut de `LogsTailParams::limit` — plafonne le coût token d'un
/// `logs_tail` sans paramètre explicite, tout en restant assez large pour
/// couvrir la plupart des sessions de debug.
const LOGS_TAIL_DEFAULT_LIMIT: usize = 200;

#[derive(Debug, Deserialize, JsonSchema)]
struct StateParams {
    /// Sous-ensemble de clés à renvoyer (sinon, tout le state). Réduit le
    /// coût token quand seules quelques clés intéressent l'agent. Absent ou
    /// vide : aucun filtrage (comportement inchangé).
    #[serde(default)]
    keys: Option<Vec<String>>,
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
            window_manager: ports.window_manager,
            window_geometry: ports.window_geometry,
            clipboard: ports.clipboard,
            target_info: ports.target_info,
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

    /// A8 : acquiert le gate en traçant l'attente ; la libération (durée
    /// comprise) est tracée par le `Drop` de [`GateGuard`]. Instrumenté sur
    /// les outils les plus longs (screenshot, eval_js, click, wait_ready) —
    /// pas les 41, cf. audit §A8.
    async fn acquire_gate(&self, outil: &'static str) -> GateGuard<'_> {
        let inner = self.gate.lock().await;
        debug!(outil, "gate acquis");
        GateGuard {
            _inner: inner,
            outil,
            acquired_at: Instant::now(),
        }
    }

    #[tool(description = "Capture un screenshot PNG du viewport de l'app cible (image MCP).")]
    async fn screenshot(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.acquire_gate("screenshot").await;
        let shot = self.renderer.screenshot().await.map_err(port_to_mcp)?;
        let b64 = B64.encode(&shot.bytes);
        // Contenu `image` (et non `text`) : le client MCP affiche le PNG —
        // c'est ce qui rend l'outil utilisable comme « yeux ».
        Ok(CallToolResult::success(vec![Content::image(
            b64,
            "image/png",
        )]))
    }

    #[tool(
        description = "Capture un screenshot puis le recadre sur la région (x, y, width, height) avant de renvoyer le PNG (image MCP). Réduit nettement le coût en tokens d'une vérification ciblée, et reste la seule façon d'inspecter un composant précis en kwin-eis (pas de DOM). La région est bornée aux dimensions de la capture."
    )]
    async fn screenshot_region(
        &self,
        Parameters(p): Parameters<ScreenshotRegionParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let shot = self.renderer.screenshot().await.map_err(port_to_mcp)?;
        let cropped = crop_png(&shot.bytes, p.x, p.y, p.width, p.height).map_err(internal)?;
        let b64 = B64.encode(&cropped);
        Ok(CallToolResult::success(vec![Content::image(
            b64,
            "image/png",
        )]))
    }

    #[tool(
        description = "Capture un élément par sélecteur CSS : résout son rect via JS (le défile au centre), puis recadre la capture sur ce rect (mise à l'échelle DPI gérée). Renvoie le PNG (image MCP), ou `isError {found:false}` si l'élément est absent/invisible. Nécessite eval_js (cdp/sdk) ; indisponible en kwin-eis (utiliser screenshot_region)."
    )]
    async fn screenshot_element(
        &self,
        Parameters(p): Parameters<ScreenshotElementParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        // 1) Resoudre le rect de l'element (coordonnees CSS viewport).
        let sel = serde_json::to_string(&p.selector).unwrap_or_default();
        let snippet = format!(
            "(function(){{ const el = document.querySelector({sel}); \
             if (!el) return {{ found: false, reason: 'not_found' }}; \
             el.scrollIntoView({{ block: 'center', behavior: 'instant' }}); \
             const r = el.getBoundingClientRect(); \
             if (r.width === 0 || r.height === 0) return {{ found: false, reason: 'not_visible' }}; \
             return {{ found: true, x: r.left, y: r.top, width: r.width, height: r.height }}; }})()"
        );
        let rect = self
            .evaluator
            .evaluate(&snippet)
            .await
            .map_err(port_to_mcp)?;
        if !rect.get("found").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Ok(json_error(&rect));
        }
        // 2) Capturer puis recadrer. Le PNG peut etre en pixels physiques
        // (HiDPI) alors que le rect est en pixels CSS : on met a l'echelle par
        // le ratio dimensions-PNG / viewport-logique.
        let shot = self.renderer.screenshot().await.map_err(port_to_mcp)?;
        let (pw, ph) = png_dimensions(&shot.bytes).map_err(internal)?;
        let scale_x = if shot.viewport.width > 0 {
            pw as f64 / shot.viewport.width as f64
        } else {
            1.0
        };
        let scale_y = if shot.viewport.height > 0 {
            ph as f64 / shot.viewport.height as f64
        } else {
            1.0
        };
        let get = |k: &str| rect.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let x = (get("x") * scale_x).max(0.0).round() as u32;
        let y = (get("y") * scale_y).max(0.0).round() as u32;
        let w = (get("width") * scale_x).round().max(1.0) as u32;
        let h = (get("height") * scale_y).round().max(1.0) as u32;
        let cropped = crop_png(&shot.bytes, x, y, w, h).map_err(internal)?;
        let b64 = B64.encode(&cropped);
        Ok(CallToolResult::success(vec![Content::image(
            b64,
            "image/png",
        )]))
    }

    #[tool(
        description = "Assertion visuelle (golden) : capture le viewport (ou la région x/y/width/height) et le compare au PNG `baseline_path`. Si le fichier n'existe pas, il est créé (`{ created: true }`). Sinon renvoie `{ ok, diff_ratio, threshold, same_dims, ... }` ; `isError` si le ratio de pixels différents dépasse `threshold` (défaut 0.01) ou si les dimensions diffèrent. Premier outil d'assertion déterministe disponible en kwin-eis (sans DOM/JS)."
    )]
    async fn assert_visual(
        &self,
        Parameters(p): Parameters<AssertVisualParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let shot = self.renderer.screenshot().await.map_err(port_to_mcp)?;
        // Recadrage optionnel : les 4 bornes doivent etre fournies ensemble.
        // B10 : `Cow` evite de cloner tout le PNG quand aucun recadrage n'est
        // demande (chemin le plus frequent) — seul le cas recadre alloue,
        // via le buffer deja produit par `crop_png`.
        let current: std::borrow::Cow<'_, [u8]> = match (p.x, p.y, p.width, p.height) {
            (Some(x), Some(y), Some(w), Some(h)) => {
                std::borrow::Cow::Owned(crop_png(&shot.bytes, x, y, w, h).map_err(internal)?)
            }
            _ => std::borrow::Cow::Borrowed(&shot.bytes),
        };
        let path = std::path::Path::new(&p.baseline_path);
        if !tokio::fs::try_exists(path).await.map_err(internal)? {
            // Baseline absente : on l'ecrit a partir de la capture courante.
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    tokio::fs::create_dir_all(parent).await.map_err(internal)?;
                }
            }
            tokio::fs::write(path, &current).await.map_err(internal)?;
            return Ok(json_success(&serde_json::json!({
                "ok": true,
                "created": true,
                "baseline_path": p.baseline_path,
            })));
        }
        let baseline = tokio::fs::read(path).await.map_err(internal)?;
        let threshold = p.threshold.unwrap_or(0.01);
        let outcome = visual_diff(&baseline, &current).map_err(internal)?;
        let ok = outcome.same_dims && outcome.diff_ratio <= threshold;
        let payload = serde_json::json!({
            "ok": ok,
            "created": false,
            "diff_ratio": outcome.diff_ratio,
            "threshold": threshold,
            "same_dims": outcome.same_dims,
            "baseline_dims": { "width": outcome.baseline_dims.0, "height": outcome.baseline_dims.1 },
            "current_dims": { "width": outcome.current_dims.0, "height": outcome.current_dims.1 },
            "baseline_path": p.baseline_path,
        });
        if ok {
            Ok(json_success(&payload))
        } else {
            Ok(json_error(&payload))
        }
    }

    #[tool(
        description = "Effectue un clic gauche sur le viewport de l'app cible aux coordonnées absolues fournies."
    )]
    async fn click(
        &self,
        Parameters(p): Parameters<ClickParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.acquire_gate("click").await;
        let cmd = InteractionCommand::click(Position::new(p.x, p.y));
        self.interaction.dispatch(cmd).await.map_err(port_to_mcp)?;
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
        self.interaction.dispatch(cmd).await.map_err(port_to_mcp)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Double-clic gauche aux coordonnées (x, y) : édition inline, sélection de mot, ouverture d'item de liste. Émet un vrai `dblclick` (clickCount=2 en CDP). Plus fiable que deux `click` MCP successifs (intervalle imprévisible)."
    )]
    async fn double_click(
        &self,
        Parameters(p): Parameters<ClickParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        self.interaction
            .dispatch(InteractionCommand::double_click(Position::new(p.x, p.y)))
            .await
            .map_err(port_to_mcp)?;
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
            .map_err(port_to_mcp)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Presse une touche unique (keydown + keyup) au focus courant : Enter, Tab, Escape, ArrowDown, F5… Marche sur toutes les cibles, y compris kwin-eis (où eval_js est indisponible). Noms W3C en CDP (Enter, ArrowDown) ; noms XKB en kwin-eis (Return, Down) ; les noms communs (Tab, Escape, F5) passent partout."
    )]
    async fn press_key(
        &self,
        Parameters(p): Parameters<PressKeyParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        self.interaction
            .dispatch(InteractionCommand::key(p.key))
            .await
            .map_err(port_to_mcp)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Combinaison de touches : maintient les modificateurs (ctrl/shift/alt/meta) pendant la frappe de `key`, puis les relâche. Ctrl+S, Ctrl+Z, Ctrl+Shift+P, Alt+F4… Marche sur toutes les cibles, y compris kwin-eis (events OS-level : les raccourcis du toolkit se déclenchent). `press_key` seul ne porte aucun modificateur."
    )]
    async fn key_combo(
        &self,
        Parameters(p): Parameters<KeyComboParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        self.interaction
            .dispatch(InteractionCommand::key_combo(p.modifiers, p.key))
            .await
            .map_err(port_to_mcp)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Navigue vers une route logique exposée par l'app cible (routeur du SDK opt-in). Interface découvrable, à préférer à un eval_js sur le hook interne `window.__deveye_router__`. Indisponible en kwin-eis (pas de routeur applicatif OS-level)."
    )]
    async fn navigate(
        &self,
        Parameters(p): Parameters<NavigateParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        self.interaction
            .dispatch(InteractionCommand::navigate(p.route))
            .await
            .map_err(port_to_mcp)?;
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
            .map_err(port_to_mcp)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Évalue une expression JavaScript dans la page cible et renvoie la valeur JSON. Promises supportées (awaitPromise=true)."
    )]
    async fn eval_js(
        &self,
        Parameters(p): Parameters<EvalParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.acquire_gate("eval_js").await;
        let value = self
            .evaluator
            .evaluate(&p.expression)
            .await
            .map_err(port_to_mcp)?;
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
        let value = self
            .evaluator
            .evaluate(snippet)
            .await
            .map_err(port_to_mcp)?;
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
            .map_err(port_to_mcp)?;
        Ok(json_success(
            &serde_json::json!({ "ok": true, "x": x, "y": y }),
        ))
    }

    #[tool(
        description = "Récupère le DOM courant de l'app cible (JSON sérialisé). Garde-fou (B1) : si la sérialisation dépasse ~100 Ko, renvoie une erreur MCP explicite (`isError`) au lieu de saturer le contexte de l'agent — préférer find_element/assert_selector/eval_js pour cibler un élément précis."
    )]
    async fn dom(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let tree = self.inspector.dom_snapshot().await.map_err(port_to_mcp)?;
        let serialized = serde_json::to_string(&tree.tree).unwrap_or_else(|_| "null".to_string());
        if let Some(payload) = dom_size_guard(serialized.len()) {
            return Ok(json_error(&payload));
        }
        // Le JSON est deja serialise ci-dessus (pour mesurer sa taille) : on
        // le reutilise tel quel plutot que de rappeler json_success (qui
        // resserialiserait tree.tree en double).
        Ok(CallToolResult::success(vec![Content::text(serialized)]))
    }

    #[tool(
        description = "Récupère le state applicatif exposé par le SDK opt-in (JSON sérialisé). `keys` optionnel restreint la réponse à ce sous-ensemble de clés (les clés absentes du state sont silencieusement omises) — réduit le coût en tokens quand seules quelques clés intéressent l'agent ; absent ou vide, renvoie tout le state (comportement inchangé)."
    )]
    async fn state(
        &self,
        Parameters(p): Parameters<StateParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let dump = self.inspector.state_snapshot().await.map_err(port_to_mcp)?;
        let filtered = filter_state_entries(&dump.entries, p.keys.as_deref());
        Ok(json_success(&filtered))
    }

    #[tool(
        description = "Assertion sur le state SDK : `expected` est un objet `{ clé: valeur }` ; chaque clé doit exister dans le state avec exactement cette valeur. Renvoie `{ ok, mismatches: [{ key, expected, actual }] }` ; `isError` si au moins une diverge. Symétrique de assert_selector, mais sur la logique métier."
    )]
    async fn assert_state(
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
        let dump = self.inspector.state_snapshot().await.map_err(port_to_mcp)?;
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
        description = "Draine et renvoie les événements `tracing` accumulés depuis le dernier appel, sous la forme `{ events, truncated, total }`. `limit` (défaut 200) borne le nombre d'événements présents dans `events` aux plus RÉCENTS ; le drain côté source reste complet (rien n'est laissé dans le buffer ni perdu silencieusement) — `total` indique le nombre réellement drainé et `truncated` si `total > limit` (des événements plus anciens ont alors été omis de la réponse, mais comptés)."
    )]
    async fn logs_tail(
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
            .map_err(port_to_mcp)?;
        Ok(json_success(&value))
    }

    #[tool(
        description = "Liste les commandes logiques exposées par l'app cible (découverte) : tableau JSON `[{name, description}]`, trié par nom. Vide si l'app n'a déclaré aucune route via expose_route."
    )]
    async fn list_commands(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let descriptors = self.command_router.list().await.map_err(port_to_mcp)?;
        Ok(json_success(&descriptors))
    }

    #[tool(
        description = "Attend qu'un sélecteur CSS soit présent (et optionnellement visible) dans le DOM. Renvoie `{ found, elapsed_ms }` (`isError` si non satisfait). Avec `negate=true`, attend au contraire que l'élément DISPARAISSE (wait_for_gone : spinner/toast/modale) — `found=true` signifie alors « disparu ». Évite tout sleep arbitraire. Défaut 5000 ms, plafond 30000."
    )]
    async fn wait_for_selector(
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
        let budget = Duration::from_millis(p.timeout_ms.unwrap_or(5_000).min(30_000));
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
    async fn wait_for_text(
        &self,
        Parameters(p): Parameters<WaitForTextParams>,
    ) -> Result<CallToolResult, McpError> {
        let predicate = WaitPredicate::TextContent {
            text: p.text,
            selector: p.selector,
            visible: p.visible.unwrap_or(false),
        };
        let budget = Duration::from_millis(p.timeout_ms.unwrap_or(5_000).min(30_000));
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
        description = "Inspecte le premier élément matchant un sélecteur CSS : `{ found, tag, text, rect{x,y,width,height}, visible, disabled, attributes, value_prop }`. `value_prop` porte la propriété `.value` de l'élément (inputs/select/textarea) — `null` si l'élément n'en a pas (ex. lire une saisie, distinct de `text` qui reste l'`innerText`). Réponse compacte au lieu de tout le DOM."
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
               attributes: attrs, \
               value_prop: ('value' in el) ? el.value : null }}; }})()"
        );
        let value = self
            .evaluator
            .evaluate(&snippet)
            .await
            .map_err(port_to_mcp)?;
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
        let value = self
            .evaluator
            .evaluate(&snippet)
            .await
            .map_err(port_to_mcp)?;
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
        description = "Attend qu'un prédicat soit satisfait, ou que le timeout expire. Renvoie `{ satisfied, elapsed_ms, detail }` ; `isError` si non satisfait. Prédicats : dom-selector (présence CSS), state-key (valeur dans le state SDK), screenshot-stable (stabilité visuelle pendant threshold_ms), target-alive (attend qu'une cible crashée redevienne joignable, cf. wait_target_alive), text-content (attend un texte dans le DOM, cf. wait_for_text). Évite tout sleep arbitraire."
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
    async fn wait_for_log(
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
        let budget = Duration::from_millis(p.timeout_ms.unwrap_or(5_000).min(60_000));
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
    async fn wait_screenshot_region(
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
        let budget = Duration::from_millis(p.timeout_ms.unwrap_or(5_000).min(60_000));
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
    async fn wait_target_alive(
        &self,
        Parameters(p): Parameters<WaitTargetAliveParams>,
    ) -> Result<CallToolResult, McpError> {
        let budget = Duration::from_millis(p.timeout_ms.unwrap_or(5_000).min(60_000));
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
        self.interaction.dispatch(cmd).await.map_err(port_to_mcp)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Répond à une boîte de dialogue JS (`alert`/`confirm`/`prompt`) : `accept` (OK/Cancel), `prompt_text` = texte pour un `prompt`. CDP-only — pendant un dialog le moteur JS est suspendu, donc `eval_js` est impossible : c'est la seule issue. Renvoie une erreur `Unsupported` en sdk/kwin-eis."
    )]
    async fn handle_dialog(
        &self,
        Parameters(p): Parameters<HandleDialogParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        self.interaction
            .dispatch(InteractionCommand::handle_dialog(p.accept, p.prompt_text))
            .await
            .map_err(port_to_mcp)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Renseigne les fichiers d'un `input[type=file]` (sélecteur CSS + chemins absolus). CDP-only (`DOM.setFileInputFiles`) : la sécurité navigateur interdit de le faire en JS, donc non contournable par eval_js. Renvoie une erreur `Unsupported` en sdk/kwin-eis."
    )]
    async fn upload_file(
        &self,
        Parameters(p): Parameters<UploadFileParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        self.interaction
            .dispatch(InteractionCommand::upload_file(p.selector, p.paths))
            .await
            .map_err(port_to_mcp)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Défile de (dx, dy) pixels — `dy > 0` vers le bas. Ancré en (x, y) ou, par défaut, au centre du viewport. En CDP, l'ancrage cible le bon conteneur scrollable (mouseWheel) ; via le pont SDK, le conteneur scrollable sous le point est privilégié (sinon window) ; en kwin-eis, le défilement va à la fenêtre focus. Indispensable pour les listes/grilles en kwin-eis (eval_js indisponible)."
    )]
    async fn scroll(
        &self,
        Parameters(p): Parameters<ScrollParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let dx = p.dx.unwrap_or(0);
        let dy = p.dy.unwrap_or(0);
        // Ancrage : centre du viewport si x ou y manquant. Resolu via le
        // renderer ; en cas d'echec (cible sans dimensions exploitables), on
        // retombe sur (0, 0) — sans incidence en kwin-eis (ancrage ignore).
        let (cx, cy) = if p.x.is_none() || p.y.is_none() {
            match self.renderer.viewport_size().await {
                Ok(size) => (size.width as i32 / 2, size.height as i32 / 2),
                Err(_) => (0, 0),
            }
        } else {
            (0, 0)
        };
        let anchor = Position::new(p.x.unwrap_or(cx), p.y.unwrap_or(cy));
        self.interaction
            .dispatch(InteractionCommand::scroll(anchor, dx, dy))
            .await
            .map_err(port_to_mcp)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Draine les messages console émis par le webview depuis le dernier appel — tous niveaux confondus (exceptions non capturées, promesses rejetées, console.log/info/warn/error). Renvoie un tableau typé `[{ level, text, url?, line?, timestamp }]` (port ConsoleErrorSource). `levels` filtre optionnellement par sévérité (ex. `[\"error\", \"warning\"]`) ; absent ou vide renvoie tout. Un crash JS de la page devient visible à l'agent."
    )]
    async fn collect_console_errors(
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
        description = "Décrit la cible active et ses capacités : `{ target, eyes, hands, eval_js, dom, state, console_errors, list_windows, notes }`. Permet de savoir d'avance quels outils répondront (ex. eval_js/dom = false en kwin-eis) sans tenter un appel pour le découvrir."
    )]
    async fn target_info(&self) -> Result<CallToolResult, McpError> {
        // Description statique resolue au demarrage : pas de gate, pas d'I/O.
        Ok(json_success(&self.target_info))
    }

    #[tool(
        description = "Donne le focus OS à une fenêtre (par sous-chaîne de titre, ou la fenêtre --screencap-window par défaut). Crucial en kwin-eis : KWin EIS route les événements vers la fenêtre active ; une notification ou un dialog système peut détourner les clics sans erreur. À appeler avant une séquence d'interaction. Renvoie `{ ok, title }`. Nécessite KDE/Wayland (KWin Scripting) ; `Unsupported` sinon."
    )]
    async fn focus_window(
        &self,
        Parameters(p): Parameters<FocusWindowParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let title = self
            .window_manager
            .activate(p.title_pattern.as_deref())
            .await
            .map_err(port_to_mcp)?;
        Ok(json_success(
            &serde_json::json!({ "ok": true, "title": title }),
        ))
    }

    #[tool(
        description = "Renvoie la géométrie écran `{ x, y, width, height }` de la fenêtre dont le titre contient `title` (ou la fenêtre `--screencap-window` par défaut) via KWin Scripting. Complète `find_element`/`eval_js` (indisponibles en kwin-eis, rendu natif) : convertit un rect DOM en coordonnées écran absolues pour un clic OS-level (EIS) hors du viewport webview. Nécessite KDE/Wayland ; `Unsupported` sinon."
    )]
    async fn window_geometry(
        &self,
        Parameters(p): Parameters<WindowGeometryParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let rect = self
            .window_geometry
            .geometry(p.title.as_deref())
            .await
            .map_err(port_to_mcp)?;
        Ok(json_success(&rect))
    }

    #[tool(
        description = "Lit le texte du presse-papiers de la session (OS-level via wl-paste sous Wayland). Renvoie `{ text }`. Fiable là où `navigator.clipboard.readText()` lève NotAllowedError (contexte automatisé) et en kwin-eis (pas de WebView). `Unsupported` hors Wayland."
    )]
    async fn clipboard_get(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let text = self.clipboard.get().await.map_err(port_to_mcp)?;
        Ok(json_success(&serde_json::json!({ "text": text })))
    }

    #[tool(
        description = "Place du texte dans le presse-papiers de la session (OS-level via wl-copy sous Wayland). Combiné à key_combo Ctrl+V, permet de saisir du texte long/Unicode sans frappe touche par touche. `Unsupported` hors Wayland."
    )]
    async fn clipboard_set(
        &self,
        Parameters(p): Parameters<ClipboardSetParams>,
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        self.clipboard.set(&p.text).await.map_err(port_to_mcp)?;
        Ok(CallToolResult::success(vec![Content::text("ok")]))
    }

    #[tool(
        description = "Liste les fenêtres/onglets de la cible : `[{ id, title, url, kind }]`. En CDP via Target.getTargets ; en kwin-eis via KWin Scripting (`url` vide, `kind` = resourceClass) ; via le pont SDK : liste vide (pas de visibilité inter-fenêtres)."
    )]
    async fn list_windows(&self) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        let windows = self
            .window_lister
            .list_windows()
            .await
            .map_err(port_to_mcp)?;
        Ok(json_success(&windows))
    }

    #[tool(
        description = "Attend que l'app cible publie un state non vide (signe qu'elle est prête), au plus `timeout_ms` ms (poll toutes les 200 ms, plafond 60000). Évite les captures/clics prématurés juste après le lancement. `isError` si le délai expire."
    )]
    async fn wait_ready(
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

#[tool_handler]
impl ServerHandler for DeveyeMcpServer {
    fn get_info(&self) -> ServerInfo {
        // `ServerInfo` est `#[non_exhaustive]` : on part du default et on
        // mute les champs qui nous intéressent.
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "DevEye MCP — observe et pilote une app vivante en dev. \
             Outils : screenshot, screenshot_region, screenshot_element, \
             assert_visual, click, double_click, right_click, hover, \
             click_selector, click_text, eval_js, type_text, press_key, \
             key_combo, navigate, scroll, handle_dialog, upload_file, \
             focus_window, window_geometry, clipboard_get, clipboard_set, dom, \
             state, assert_state, logs_tail, invoke_command, list_commands, \
             wait_for_selector, wait_for_text, find_element, assert_selector, \
             drag_drop, collect_console_errors, list_windows, wait_condition, \
             wait_for_log, wait_screenshot_region, wait_target_alive, \
             wait_ready, target_info. \
             Appeler target_info en premier pour connaitre les capacites de la \
             cible (eval_js/dom indisponibles en kwin-eis)."
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

/// A7 : decision pure d'`assert_selector`, extraite pour etre testable sans
/// I/O. Le garde precedent (`actual_count >= 1`) ecrasait toute demande
/// `expected_count: Some(0)` — l'assertion d'ABSENCE etait inexprimable, elle
/// echouait systematiquement meme quand elle etait vraie. `expected_count`
/// fixe desormais le critere de comptage a la place du `>= 1` implicite
/// (qui ne reste le defaut que si `expected_count` est absent) ; `text_ok`
/// (deja resolu par l'appelant : `true` si `expected_text` est absent) est
/// applique par-dessus.
fn selector_assertion_ok(
    actual_count: usize,
    expected_count: Option<usize>,
    text_ok: bool,
) -> bool {
    let count_ok = match expected_count {
        Some(n) => actual_count == n,
        None => actual_count >= 1,
    };
    count_ok && text_ok
}

/// B7 : filtre `entries` sur `keys` avant sérialisation par l'outil `state`.
/// `None` ou vide (aucun filtrage demandé) renvoie toutes les entrées ; les
/// clés absentes de `entries` sont silencieusement ignorées (pas d'erreur —
/// même logique que `collect_console_errors`/`levels`). Retourne des
/// références pour éviter de cloner les valeurs JSON quand un seul sous-
/// ensemble est demandé (et même sans filtrage, on ne clone pas `entries`).
fn filter_state_entries<'a>(
    entries: &'a std::collections::BTreeMap<String, serde_json::Value>,
    keys: Option<&[String]>,
) -> std::collections::BTreeMap<&'a String, &'a serde_json::Value> {
    match keys {
        Some(keys) if !keys.is_empty() => keys
            .iter()
            .filter_map(|k| entries.get_key_value(k))
            .collect(),
        _ => entries.iter().collect(),
    }
}

/// B1 : décide si un DOM sérialisé (longueur en octets) dépasse le garde-fou
/// [`DOM_MAX_BYTES`] ; renvoie le payload d'erreur à propager le cas échéant.
/// Extrait de l'outil `dom` pour être testable sans dépendre d'un
/// `AppInspector`.
fn dom_size_guard(serialized_len: usize) -> Option<serde_json::Value> {
    if serialized_len > DOM_MAX_BYTES {
        Some(serde_json::json!({
            "ok": false,
            "reason": "dom_too_large",
            "size_bytes": serialized_len,
            "max_bytes": DOM_MAX_BYTES,
            "hint": "DOM trop volumineux pour etre renvoye entier : cibler via \
                      find_element (un element), assert_selector (compte/texte) \
                      ou eval_js (extraction ciblee) plutot que de dumper tout l'arbre.",
        }))
    } else {
        None
    }
}

/// B4 : logique pure de troncature de `logs_tail` — garde les `limit`
/// événements les PLUS RÉCENTS (`poll()` renvoie l'ordre d'émission, donc les
/// plus récents sont en fin de vecteur) et signale si le total drainé
/// dépassait `limit`. Extrait pour être testé sans `LogSource`.
fn truncate_recent<T>(mut events: Vec<T>, limit: usize) -> (Vec<T>, bool, usize) {
    let total = events.len();
    let truncated = total > limit;
    if truncated {
        events = events.split_off(total - limit);
    }
    (events, truncated, total)
}

fn internal<E: std::fmt::Display>(err: E) -> McpError {
    McpError::internal_error(err.to_string(), None)
}

/// Convertit une `PortError` en `McpError` en distinguant qui est responsable :
///
/// - `Route` (commande logique inconnue) et `Interaction` (dispatch invalide)
///   sont imputables a l'appelant MCP (parametres incorrects) -> -32602
///   `invalid_params`, comme pour toute erreur de requete mal formee.
/// - tout le reste (Render, Inspect, Log, Network, Protocol, Console, Timeout,
///   Unsupported) reste une panne d'infra cote adapter/cible -> -32603
///   `internal_error`, code inchange.
///
/// Portage du banc d'essai Chronos du 28/06 (audit C2-c) : verifie en
/// conditions reelles que « route inconnue » y remontait -32602 au lieu du
/// -32603 aplati que produisait `internal` jusque-la.
fn port_to_mcp(err: PortError) -> McpError {
    match &err {
        PortError::Route { .. } | PortError::Interaction(_) => {
            McpError::invalid_params(err.to_string(), None)
        }
        _ => McpError::internal_error(err.to_string(), None),
    }
}

/// Dimensions `(width, height)` d'un PNG en ne lisant que l'en-tête (chunk
/// IHDR) via `ImageReader::into_dimensions`, sans décoder les pixels. Sert au
/// calcul d'échelle DPI de `screenshot_element`, où le PNG est ensuite décodé
/// (une seule fois) par `crop_png` pour le recadrage.
///
/// B6 : corrige le commentaire — et surtout l'implémentation — mensongers de
/// la version précédente, qui prétendait ne lire que l'en-tête mais appelait
/// en réalité `load_from_memory` (décodage pixel complet), doublant le coût
/// de `screenshot_element` qui décode déjà tout le PNG une fois via
/// `crop_png`.
fn png_dimensions(png: &[u8]) -> Result<(u32, u32), String> {
    image::ImageReader::new(std::io::Cursor::new(png))
        .with_guessed_format()
        .map_err(|e| format!("detection format PNG : {e}"))?
        .into_dimensions()
        .map_err(|e| format!("dimensions PNG : {e}"))
}

/// Recadre un PNG sur `(x, y, width, height)` et le re-encode en PNG.
/// La region est bornee aux dimensions reelles de l'image (un `width` trop
/// grand est tronque) ; une origine hors image ou une dimension nulle est une
/// erreur. Utilise par `screenshot_region` et `assert_visual`.
fn crop_png(png: &[u8], x: u32, y: u32, width: u32, height: u32) -> Result<Vec<u8>, String> {
    use image::GenericImageView;
    if width == 0 || height == 0 {
        return Err("region vide (width ou height = 0)".to_string());
    }
    let img = image::load_from_memory(png).map_err(|e| format!("decode PNG : {e}"))?;
    let (iw, ih) = img.dimensions();
    if x >= iw || y >= ih {
        return Err(format!(
            "region hors capture : origine ({x}, {y}) hors dimensions {iw}x{ih}"
        ));
    }
    let cw = width.min(iw - x);
    let ch = height.min(ih - y);
    let cropped = img.crop_imm(x, y, cw, ch);
    let mut out = Vec::new();
    cropped
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| format!("encode PNG : {e}"))?;
    Ok(out)
}

/// Resultat d'une comparaison visuelle (`assert_visual`).
struct VisualOutcome {
    /// Vrai si baseline et capture ont les memes dimensions.
    same_dims: bool,
    /// Ratio de pixels differents (0.0–1.0). `1.0` si les dimensions different.
    diff_ratio: f64,
    baseline_dims: (u32, u32),
    current_dims: (u32, u32),
}

/// Compare deux PNG pixel a pixel (RGBA). Des dimensions differentes => echec
/// (`same_dims=false`, `diff_ratio=1.0`) sans iterer. Sinon, ratio de pixels
/// non identiques sur le total.
fn visual_diff(baseline_png: &[u8], current_png: &[u8]) -> Result<VisualOutcome, String> {
    let a = image::load_from_memory(baseline_png)
        .map_err(|e| format!("decode baseline : {e}"))?
        .to_rgba8();
    let b = image::load_from_memory(current_png)
        .map_err(|e| format!("decode capture : {e}"))?
        .to_rgba8();
    let baseline_dims = a.dimensions();
    let current_dims = b.dimensions();
    if baseline_dims != current_dims {
        return Ok(VisualOutcome {
            same_dims: false,
            diff_ratio: 1.0,
            baseline_dims,
            current_dims,
        });
    }
    let total = (baseline_dims.0 as u64) * (baseline_dims.1 as u64);
    let diff = a
        .pixels()
        .zip(b.pixels())
        .filter(|(pa, pb)| pa != pb)
        .count() as u64;
    let diff_ratio = if total == 0 {
        0.0
    } else {
        diff as f64 / total as f64
    };
    Ok(VisualOutcome {
        same_dims: true,
        diff_ratio,
        baseline_dims,
        current_dims,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        crop_png, dom_size_guard, filter_state_entries, png_dimensions, selector_assertion_ok,
        truncate_recent, visual_diff, DOM_MAX_BYTES,
    };
    use deveye_core::{InteractionCommand, MouseButton, Position};

    /// Encode une image RGBA unie de `w`x`h` en PNG (helper de test).
    fn solid_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("encode png test");
        out
    }

    fn png_dims(png: &[u8]) -> (u32, u32) {
        use image::GenericImageView;
        image::load_from_memory(png).expect("decode").dimensions()
    }

    #[test]
    fn crop_png_reduces_to_region() {
        let src = solid_png(10, 10, [255, 0, 0, 255]);
        let cropped = crop_png(&src, 2, 3, 4, 5).expect("crop ok");
        assert_eq!(png_dims(&cropped), (4, 5));
    }

    #[test]
    fn crop_png_clamps_oversized_region() {
        let src = solid_png(10, 10, [0, 255, 0, 255]);
        // Origine (8,8), demande 10x10 => borne a 2x2 (10-8).
        let cropped = crop_png(&src, 8, 8, 10, 10).expect("crop ok");
        assert_eq!(png_dims(&cropped), (2, 2));
    }

    #[test]
    fn crop_png_rejects_origin_out_of_bounds() {
        let src = solid_png(10, 10, [0, 0, 255, 255]);
        assert!(crop_png(&src, 20, 0, 4, 4).is_err());
    }

    #[test]
    fn crop_png_rejects_zero_dimension() {
        let src = solid_png(10, 10, [0, 0, 255, 255]);
        assert!(crop_png(&src, 0, 0, 0, 4).is_err());
    }

    #[test]
    fn visual_diff_identical_is_zero() {
        let a = solid_png(4, 4, [10, 20, 30, 255]);
        let out = visual_diff(&a, &a).expect("diff ok");
        assert!(out.same_dims);
        assert_eq!(out.diff_ratio, 0.0);
    }

    #[test]
    fn visual_diff_dimension_mismatch_is_max() {
        let a = solid_png(4, 4, [0, 0, 0, 255]);
        let b = solid_png(8, 8, [0, 0, 0, 255]);
        let out = visual_diff(&a, &b).expect("diff ok");
        assert!(!out.same_dims);
        assert_eq!(out.diff_ratio, 1.0);
        assert_eq!(out.baseline_dims, (4, 4));
        assert_eq!(out.current_dims, (8, 8));
    }

    #[test]
    fn visual_diff_one_changed_pixel_ratio() {
        // 2x2 = 4 pixels ; un seul differe => ratio 0.25.
        let mut a_img = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255]));
        let mut b_img = a_img.clone();
        b_img.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
        let encode = |img: &mut image::RgbaImage| {
            let mut out = Vec::new();
            image::DynamicImage::ImageRgba8(img.clone())
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .unwrap();
            out
        };
        let a = encode(&mut a_img);
        let b = encode(&mut b_img);
        let out = visual_diff(&a, &b).expect("diff ok");
        assert!(out.same_dims);
        assert!(
            (out.diff_ratio - 0.25).abs() < 1e-9,
            "ratio={}",
            out.diff_ratio
        );
    }

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

    // ----- A7 : selector_assertion_ok -----

    #[test]
    fn selector_assertion_expected_count_zero_with_zero_elements_is_ok() {
        // Assertion d'ABSENCE : c'est precisement le cas que l'ancien garde
        // `actual_count >= 1` rendait inexprimable.
        assert!(selector_assertion_ok(0, Some(0), true));
    }

    #[test]
    fn selector_assertion_expected_count_zero_with_one_element_is_ko() {
        assert!(!selector_assertion_ok(1, Some(0), true));
    }

    #[test]
    fn selector_assertion_none_with_zero_elements_is_ko() {
        // Sans expected_count, le defaut reste "au moins un element present".
        assert!(!selector_assertion_ok(0, None, true));
    }

    #[test]
    fn selector_assertion_none_with_two_elements_is_ok() {
        assert!(selector_assertion_ok(2, None, true));
    }

    #[test]
    fn selector_assertion_text_ko_overrides_count_ok() {
        assert!(!selector_assertion_ok(1, Some(1), false));
        assert!(selector_assertion_ok(1, Some(1), true));
    }

    // ----- B6 : png_dimensions (lecture d'en-tete seule) -----

    #[test]
    fn png_dimensions_reads_header_without_full_decode() {
        let src = solid_png(7, 5, [1, 2, 3, 255]);
        assert_eq!(png_dimensions(&src).unwrap(), (7, 5));
    }

    #[test]
    fn png_dimensions_rejects_garbage() {
        assert!(png_dimensions(b"pas un png").is_err());
    }

    // ----- B7 : filter_state_entries -----

    fn sample_entries() -> std::collections::BTreeMap<String, serde_json::Value> {
        let mut entries = std::collections::BTreeMap::new();
        entries.insert("a".to_string(), serde_json::json!(1));
        entries.insert("b".to_string(), serde_json::json!(2));
        entries.insert("c".to_string(), serde_json::json!(3));
        entries
    }

    #[test]
    fn filter_state_entries_none_returns_everything() {
        let entries = sample_entries();
        let out = filter_state_entries(&entries, None);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn filter_state_entries_empty_keys_returns_everything() {
        let entries = sample_entries();
        let empty: Vec<String> = vec![];
        let out = filter_state_entries(&entries, Some(&empty));
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn filter_state_entries_keeps_only_requested_keys() {
        let entries = sample_entries();
        let keys = vec!["b".to_string(), "missing".to_string()];
        let out = filter_state_entries(&entries, Some(&keys));
        assert_eq!(out.len(), 1);
        assert_eq!(
            out.get(&"b".to_string()).copied(),
            Some(&serde_json::json!(2))
        );
    }

    // ----- B1 : dom_size_guard -----

    #[test]
    fn dom_size_guard_allows_small_payload() {
        assert!(dom_size_guard(100).is_none());
    }

    #[test]
    fn dom_size_guard_allows_exact_boundary() {
        assert!(dom_size_guard(DOM_MAX_BYTES).is_none());
    }

    #[test]
    fn dom_size_guard_blocks_oversized_payload() {
        let payload = dom_size_guard(DOM_MAX_BYTES + 1).expect("guard doit se declencher");
        assert_eq!(payload["reason"], "dom_too_large");
        assert_eq!(payload["size_bytes"], DOM_MAX_BYTES + 1);
        assert_eq!(payload["ok"], false);
    }

    // ----- B4 : truncate_recent -----

    #[test]
    fn truncate_recent_keeps_last_n_and_flags_truncation() {
        let (kept, truncated, total) = truncate_recent(vec![1, 2, 3, 4, 5], 2);
        assert_eq!(kept, vec![4, 5]);
        assert!(truncated);
        assert_eq!(total, 5);
    }

    #[test]
    fn truncate_recent_no_truncation_when_under_limit() {
        let (kept, truncated, total) = truncate_recent(vec![1, 2, 3], 10);
        assert_eq!(kept, vec![1, 2, 3]);
        assert!(!truncated);
        assert_eq!(total, 3);
    }

    #[test]
    fn truncate_recent_exact_limit_is_not_truncated() {
        let (kept, truncated, total) = truncate_recent(vec![1, 2, 3], 3);
        assert_eq!(kept, vec![1, 2, 3]);
        assert!(!truncated);
        assert_eq!(total, 3);
    }

    #[test]
    fn truncate_recent_zero_limit_drops_everything_but_counts_total() {
        let (kept, truncated, total) = truncate_recent(vec![1, 2, 3], 0);
        assert!(kept.is_empty());
        assert!(truncated);
        assert_eq!(total, 3);
    }
}
