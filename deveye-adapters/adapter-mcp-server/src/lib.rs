//! Adapter DevEye qui expose les ports `Renderer`, `AppInspector`,
//! `InteractionSink` et `LogSource` au format Model Context Protocol via
//! `rmcp` (transport stdio).
//!
//! Quarante-deux outils sont publiés à Claude (ou à n'importe quel client MCP) :
//! - `screenshot` : capture le viewport en PNG, renvoie le base64
//! - `screenshot_region` : capture puis recadre sur (x, y, width, height)
//! - `screenshot_element` : capture recadrée sur le rect d'un sélecteur CSS
//! - `assert_visual` : compare le viewport (ou une région) à un PNG baseline
//!   (golden) ; crée la baseline si absente
//! - `click` : clic à `(x, y)` ; `button` optionnel (`left` par défaut,
//!   `right`, `middle` — E6) fait doublon assumé avec `right_click`
//! - `double_click` : double-clic gauche à `(x, y)`
//! - `right_click` : clic droit à `(x, y)` (ouvre les menus contextuels) —
//!   conservé pour compatibilité, équivalent à `click(button: "right")`
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
//! - `resize` : émule un viewport `width` x `height` (CDP ; `Unsupported` ailleurs)
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
//! **Convention d'erreur (E1)** : une capacité absente sur la cible active
//! (`PortError::Unsupported` — pas de WebView en kwin-eis pour `eval_js`/
//! `dom`/`state`, pas de session Wayland pour le presse-papiers, pas de
//! KDE/Wayland pour `focus_window`/`window_geometry`…) est un cas **normal**,
//! pas une panne : elle remonte comme un résultat `isError` structuré
//! `{ ok: false, reason: "unsupported", detail }`, jamais comme une erreur
//! JSON-RPC opaque (-32603). Cohérent avec `assert_*`/`wait_*`, où `isError`
//! signale déjà un échec sémantique plutôt qu'une panne de protocole.
//!
//! Les appels d'outils sont **sérialisés** (un seul à la fois, mutex) : des
//! interactions concurrentes pendant un re-render Dioxus touchaient le
//! mauvais nœud (leçon du 5 juin 2026).
//!
//! Voir `docs/architecture.md` pour le contexte global.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig},
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
    LogSource, PortError, PortResult, Renderer, WaitCondition, WindowGeometry, WindowLister,
    WindowManager,
};

// H1 : les outils `#[tool]` sont repartis par famille dans ces sous-modules
// (chacun `use super::*;` pour voir DeveyeMcpServer, les structs de
// parametres et les helpers ci-dessous) — lib.rs ne garde que la definition
// du serveur, les helpers communs et l'assemblage des routeurs (`new`).
mod tools_capture;
mod tools_inspect;
mod tools_interaction;
mod tools_wait;

/// E4 : timeout par defaut partage par tous les outils `wait_*` a boucle de
/// polling explicite, quand `timeout_ms` est omis par l'appelant. Une seule
/// constante nommee plutot que le litteral `5_000` recopie a chaque outil.
const WAIT_DEFAULT_MS: u64 = 5_000;

/// E4 : plafond des outils `wait_*` qui interrogent le rendu DOM de la page
/// (`wait_for_selector`, `wait_for_text`) — deliberement plus bas que
/// [`WAIT_TIMEOUT_MAX_MS`]. Une attente de rendu UI (element/texte qui
/// apparait) se joue normalement en quelques secondes ; les autres `wait_*`
/// couvrent aussi le cas d'un redemarrage complet de la cible (bien plus
/// lent, cf. `target-alive`). Nommer et partager les deux plafonds ici rend
/// cette difference (30 s vs 60 s) consciente plutot qu'un magic number
/// recopie deux fois chacun.
const WAIT_DOM_TIMEOUT_MAX_MS: u64 = 30_000;

/// Plafond partage par les outils `wait_*` a boucle de polling explicite
/// (`wait_condition`, `wait_for_log`, `wait_screenshot_region`,
/// `wait_target_alive` l'appliquent deja) : borne a 60 s le temps de blocage
/// cote agent MCP, quel que soit le `timeout_ms` demande. `wait_ready` (A5)
/// etait le seul sans plafond avant le sprint R1.
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
    /// Mode actif : "auto" | "cdp" | "sdk" | "kwin-eis" | "x11".
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

/// Coordonnées seules — `hover`/`right_click`/`double_click` (le bouton y est
/// implicite ou fixe). Distincte de [`ClickParams`] (E6 : `click` seul a un
/// paramètre `button`) pour ne pas faire apparaître un champ sans effet dans
/// le schéma MCP des trois autres outils.
#[derive(Debug, Deserialize, JsonSchema)]
struct PositionParams {
    /// Abscisse en pixels logiques depuis le coin haut-gauche du viewport.
    x: i32,
    /// Ordonnée en pixels logiques depuis le coin haut-gauche du viewport.
    y: i32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ClickParams {
    /// Abscisse en pixels logiques depuis le coin haut-gauche du viewport.
    x: i32,
    /// Ordonnée en pixels logiques depuis le coin haut-gauche du viewport.
    y: i32,
    /// E6 : bouton de souris, insensible à la casse — `"left"` (défaut),
    /// `"right"`, `"middle"`. `MouseButton::Middle` existe côté deveye-core
    /// depuis l'origine mais n'était exposé par aucun outil ; `"right"` fait
    /// doublon assumé avec `right_click` (conservé pour compatibilité et
    /// découvrabilité), voie jugée la moins perturbante plutôt qu'un outil
    /// `middle_click` dédié.
    #[serde(default)]
    button: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TypeParams {
    /// Texte UTF-8 à insérer au focus courant.
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PressKeyParams {
    /// Nom de la touche à presser (keydown + keyup). En CDP **et** en sdk
    /// (pont JS du SDK, qui synthétise aussi un `KeyboardEvent` DOM), noms
    /// W3C `KeyboardEvent.key` (`Enter`, `Tab`, `Escape`, `ArrowDown`,
    /// `F5`…). En kwin-eis (mains OS-level, pas de DOM), noms XKB (`Return`,
    /// `Down`…). Les noms communs (`Tab`, `Escape`, `F5`, `space`) passent
    /// sur les trois (E7).
    key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct KeyComboParams {
    /// Modificateurs maintenus pendant la frappe (insensibles à la casse) :
    /// `ctrl`/`control`, `shift`, `alt`, `meta`/`super`/`cmd`.
    modifiers: Vec<String>,
    /// Touche principale. En CDP **et** en sdk (pont JS, `KeyboardEvent`
    /// synthétique), nom W3C (`KeyZ`, `KeyS`, `Tab`…) ou caractère (`z`,
    /// `s`) ; en kwin-eis (mains OS-level), nom XKB (`z`, `s`, `Tab`…) (E7).
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
struct ResizeParams {
    /// Largeur émulée du viewport, en pixels CSS (1 à 16384).
    width: u32,
    /// Hauteur émulée du viewport, en pixels CSS (1 à 16384).
    height: u32,
}

/// Plus grande dimension émulable : au-delà, le moteur de rendu ne peut plus
/// allouer de surface de capture (limite des textures GPU courantes).
const RESIZE_MAX_DIMENSION: u32 = 16_384;

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

/// H1 : bloc `impl` restant en lib.rs — construction du serveur (`new`),
/// point d'entree stdio et les deux helpers d'interaction partages par
/// plusieurs familles (`acquire_gate` par capture/inspect/interaction,
/// `dispatch_ok` par interaction). Volontairement SANS `#[tool_router]` :
/// aucune fn `#[tool]` ici, seule `tool_router()` (fn ordinaire, pas la
/// macro) assemble les quatre routeurs de famille (`capture_router`,
/// `interaction_router`, `inspect_router`, `wait_router` — generes par leur
/// propre `#[tool_router(..., vis = "pub(crate)")]` dans `tools_*.rs`) via
/// `Add` (`ToolRouter<S>: Add`), pour que `#[tool_handler]` ci-dessous
/// (par defaut : `Self::tool_router()`) continue de la trouver sans avoir a
/// preciser explicitement `router = ...`.
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

    /// Assemble les quatre routeurs de famille en un seul (`ToolRouter<S>`
    /// implemente `Add`). Nomme `tool_router` (et non generee par la macro
    /// `#[tool_router]`, absente de ce bloc) pour rester la fn que
    /// `#[tool_handler]` appelle par defaut (`router = Self::tool_router()`).
    fn tool_router() -> ToolRouter<Self> {
        Self::capture_router()
            + Self::interaction_router()
            + Self::inspect_router()
            + Self::wait_router()
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
    /// pas tous, cf. audit §A8.
    async fn acquire_gate(&self, outil: &'static str) -> GateGuard<'_> {
        let inner = self.gate.lock().await;
        debug!(outil, "gate acquis");
        GateGuard {
            _inner: inner,
            outil,
            acquired_at: Instant::now(),
        }
    }

    /// H3 : factorise le triptyque commun a la majorite des outils
    /// d'interaction « fire-and-forget » (right_click, double_click,
    /// type_text, press_key, key_combo, navigate, hover, drag_drop,
    /// handle_dialog, upload_file — 10 des 12 sites identifies par l'audit ;
    /// `click` garde son propre corps pour l'instrumentation `acquire_gate`
    /// dediee, `scroll` pour la resolution d'ancrage sous le meme gate) :
    /// acquiert le gate (non instrumente : le bruit de tracer ces 10 outils
    /// n'apporterait rien de plus qu'un `click`/`screenshot`/`eval_js` deja
    /// trace, cf. A8), dispatche, puis convertit le resultat via
    /// `ok_or_unsupported` (E1 : Unsupported -> isError structure ; E3 :
    /// succes JSON `{ok:true}` au lieu du texte libre `"ok"`).
    async fn dispatch_ok(&self, cmd: InteractionCommand) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        ok_or_unsupported(self.interaction.dispatch(cmd).await)
    }
}

#[tool_handler]
impl ServerHandler for DeveyeMcpServer {
    fn get_info(&self) -> ServerConfig {
        // `ServerConfig` est `#[non_exhaustive]` : on part du default et on
        // mute les champs qui nous intéressent.
        let mut info = ServerConfig::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "DevEye MCP — observe et pilote une app vivante en dev. \
             Outils : screenshot, screenshot_region, screenshot_element, \
             assert_visual, click, double_click, right_click, hover, \
             click_selector, click_text, eval_js, type_text, press_key, \
             key_combo, navigate, scroll, resize, handle_dialog, upload_file, \
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
    CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
    )])
}

/// Idem mais renvoie un [`CallToolResult`] erreur (`isError = true`).
/// Utilise pour les echecs semantiques (element absent H1, timeout L13) qui ne
/// doivent pas etre confondus avec un succes par le client MCP.
fn json_error<T: Serialize>(value: &T) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(
        serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
    )])
}

/// E3 : sortie succes minimale des outils « fire-and-forget » (click,
/// type_text, scroll...) — JSON structure `{ "ok": true }` plutot que le
/// texte libre `"ok"` d'avant ce sprint, incoherent avec le reste de la
/// surface (assert_*, find_element, wait_*...) qui renvoie deja du JSON.
fn ok_result() -> CallToolResult {
    json_success(&serde_json::json!({ "ok": true }))
}

/// E1 : payload `isError` structure pour `PortError::Unsupported` — une
/// capacite absente sur la cible active (webview en kwin-eis, session
/// Wayland absente, KDE/Wayland absent...) est un cas NORMAL, pas une
/// panne : un resultat semantique, coherent avec le contrat `isError` deja
/// applique par les `assert_*`/`wait_*` — plutot que l'erreur JSON-RPC
/// opaque (-32603 `internal_error`) qu'aplatissait `port_to_mcp` jusqu'ici.
fn unsupported_error(detail: impl Into<String>) -> CallToolResult {
    json_error(&serde_json::json!({
        "ok": false,
        "reason": "unsupported",
        "detail": detail.into(),
    }))
}

/// E1 : deroule un `PortResult<T>` en distinguant « capacite absente sur la
/// cible active » (`PortError::Unsupported`, cf. [`unsupported_error`]) de
/// toute autre panne de port (mappee par [`port_to_mcp`]). Sur `Ok`, renvoie
/// la valeur telle quelle ; sur erreur, renvoie l'issue MCP a faire remonter
/// directement (soit un resultat isError structure, soit une erreur
/// JSON-RPC classique) — a consommer par
/// `match unsupported_aware(...) { Ok(v) => v, Err(issue) => return issue }`.
/// Impossible d'exprimer ce filtrage avec `?` seul : les deux issues
/// d'echec n'ont pas le meme type de retour cote outil MCP (`Ok(CallToolResult)`
/// vs `Err(McpError)`), d'ou ce `Result` imbrique explicite.
fn unsupported_aware<T>(result: PortResult<T>) -> Result<T, Result<CallToolResult, McpError>> {
    match result {
        Ok(v) => Ok(v),
        Err(PortError::Unsupported(detail)) => Err(Ok(unsupported_error(detail))),
        Err(e) => Err(Err(port_to_mcp(e))),
    }
}

/// E1/E3/H3 : variante de [`unsupported_aware`] pour les ports qui ne
/// produisent pas de valeur utile (`PortResult<()>` — `InteractionSink::dispatch`,
/// `Clipboard::set`) : succes -> [`ok_result`], `Unsupported` -> [`unsupported_error`],
/// autre erreur -> [`port_to_mcp`]. Utilise par [`DeveyeMcpServer::dispatch_ok`]
/// et par les outils dont le corps differe legerement (`click`, `scroll`,
/// `clipboard_set`).
fn ok_or_unsupported(result: PortResult<()>) -> Result<CallToolResult, McpError> {
    match unsupported_aware(result) {
        Ok(()) => Ok(ok_result()),
        Err(issue) => issue,
    }
}

/// E6 : parse le parametre `button` optionnel de `click` (insensible a la
/// casse) en `MouseButton` ; absent ou chaine vide => `Left` (comportement
/// inchange, retro-compatible). Une valeur non reconnue est une erreur
/// protocolaire explicite (-32602 `invalid_params`) plutot qu'un silencieux
/// retour au bouton gauche.
fn parse_mouse_button(button: Option<&str>) -> Result<MouseButton, McpError> {
    match button.map(str::to_lowercase).as_deref() {
        None | Some("") | Some("left") => Ok(MouseButton::Left),
        Some("right") => Ok(MouseButton::Right),
        Some("middle") => Ok(MouseButton::Middle),
        Some(other) => Err(McpError::invalid_params(
            format!("button invalide : {other:?} (attendu left/right/middle)"),
            None,
        )),
    }
}

/// Valide les dimensions de `resize` avant tout dispatch : une dimension
/// invalide est une faute d'appel (`invalid_params`), quelle que soit la cible
/// (même celles qui répondraient `Unsupported`). `0` x `0` est le retrait de
/// l'émulation ; un seul côté nul ou une dimension démesurée est refusé.
fn validate_resize(width: u32, height: u32) -> Result<(), McpError> {
    let reset = width == 0 && height == 0;
    let in_range = |v: u32| (1..=RESIZE_MAX_DIMENSION).contains(&v);
    if reset || (in_range(width) && in_range(height)) {
        Ok(())
    } else {
        Err(McpError::invalid_params(
            format!(
                "dimensions invalides {width}x{height} : chaque côté doit être compris entre 1 et \
                 {RESIZE_MAX_DIMENSION} (0x0 retire l'émulation)"
            ),
            None,
        ))
    }
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
///   et `Unsupported` s'il atteint directement cette fonction) reste une
///   panne d'infra cote adapter/cible -> -32603 `internal_error`, code
///   inchange.
///
/// E1 : sur les outils ou `Unsupported` est un cas normal (capacite absente
/// sur la cible active — eval_js/dom/state/clipboard/focus_window/
/// window_geometry/handle_dialog/upload_file/navigate...), l'appelant
/// intercepte `Unsupported` **avant** `port_to_mcp` via [`unsupported_aware`]
/// / [`ok_or_unsupported`] pour le router vers un resultat isError structure
/// plutot que cette erreur protocolaire — cette fonction ne voit alors
/// jamais la variante pour ces outils-la.
///
/// Verifie en conditions reelles qu'une « route inconnue » remonte -32602 et
/// non le -32603 aplati que produisait `internal` auparavant.
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

/// F-dispatch : le dispatch MCP (gate + routage vers les ports + conversion
/// du resultat) etait quasi non teste malgre son role central. Module dedie
/// avec des mocks legers des ports (les seams existent deja via les traits
/// `deveye_ports`) et un `DeveyeMcpServer` de test construit dessus.
/// Couvre aussi E1 (`PortError::Unsupported` -> resultat isError structure,
/// plutot qu'erreur JSON-RPC opaque) sur les traits mockes ici.
#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use async_trait::async_trait;
    use deveye_core::{
        CommandDescriptor, ConsoleError, DomTree, LogEvent, Screenshot, Size, StateDump,
        WaitOutcome, WindowInfo, WindowRect,
    };
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    // ------------------------------------------------------------------
    // Mocks « noop » — un par port, juste assez pour construire un
    // DeveyeMcpServer complet. Chaque test ne remplace que les ports qui
    // l'interessent via `server_with`.
    // ------------------------------------------------------------------

    struct NoopRenderer;
    #[async_trait]
    impl Renderer for NoopRenderer {
        async fn screenshot(&self) -> PortResult<Screenshot> {
            Ok(Screenshot::new_png(vec![0u8; 4], Size::new(2, 2)))
        }
        async fn viewport_size(&self) -> PortResult<Size> {
            Ok(Size::new(2, 2))
        }
    }

    struct NoopInteraction;
    #[async_trait]
    impl InteractionSink for NoopInteraction {
        async fn dispatch(&self, _command: InteractionCommand) -> PortResult<()> {
            Ok(())
        }
    }

    struct NoopInspector;
    #[async_trait]
    impl AppInspector for NoopInspector {
        async fn dom_snapshot(&self) -> PortResult<DomTree> {
            Ok(DomTree::new(serde_json::json!({})))
        }
        async fn state_snapshot(&self) -> PortResult<StateDump> {
            Ok(StateDump::empty())
        }
    }

    struct NoopLogSource;
    #[async_trait]
    impl LogSource for NoopLogSource {
        async fn poll(&self) -> PortResult<Vec<LogEvent>> {
            Ok(Vec::new())
        }
    }

    struct NoopEvaluator;
    #[async_trait]
    impl JsEvaluator for NoopEvaluator {
        async fn evaluate(&self, _expression: &str) -> PortResult<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }
    }

    struct NoopCommandRouter;
    #[async_trait]
    impl CommandRouter for NoopCommandRouter {
        async fn invoke(
            &self,
            route: &str,
            _payload: serde_json::Value,
        ) -> PortResult<serde_json::Value> {
            Err(PortError::Route {
                route: route.to_string(),
                message: "aucune commande enregistree (mock)".to_string(),
            })
        }
        async fn list(&self) -> PortResult<Vec<CommandDescriptor>> {
            Ok(Vec::new())
        }
    }

    struct NoopWindowLister;
    #[async_trait]
    impl WindowLister for NoopWindowLister {
        async fn list_windows(&self) -> PortResult<Vec<WindowInfo>> {
            Ok(Vec::new())
        }
    }

    struct NoopConsoleErrors;
    #[async_trait]
    impl ConsoleErrorSource for NoopConsoleErrors {
        async fn drain_console_errors(&self) -> PortResult<Vec<ConsoleError>> {
            Ok(Vec::new())
        }
    }

    struct NoopWaitCondition;
    #[async_trait]
    impl WaitCondition for NoopWaitCondition {
        async fn wait_until(
            &self,
            _predicate: WaitPredicate,
            _timeout: Duration,
        ) -> PortResult<WaitOutcome> {
            Ok(WaitOutcome::new(true, 0, "mock"))
        }
    }

    struct NoopWindowManager;
    #[async_trait]
    impl WindowManager for NoopWindowManager {
        async fn activate(&self, _title_substring: Option<&str>) -> PortResult<String> {
            Ok("mock".to_string())
        }
    }

    struct NoopWindowGeometry;
    #[async_trait]
    impl WindowGeometry for NoopWindowGeometry {
        async fn geometry(&self, _title_substring: Option<&str>) -> PortResult<WindowRect> {
            Ok(WindowRect::new(0, 0, 0, 0))
        }
    }

    struct NoopClipboard;
    #[async_trait]
    impl Clipboard for NoopClipboard {
        async fn get(&self) -> PortResult<String> {
            Ok(String::new())
        }
        async fn set(&self, _text: &str) -> PortResult<()> {
            Ok(())
        }
    }

    fn base_ports() -> DeveyePorts {
        DeveyePorts {
            renderer: Arc::new(NoopRenderer),
            interaction: Arc::new(NoopInteraction),
            inspector: Arc::new(NoopInspector),
            log_source: Arc::new(NoopLogSource),
            evaluator: Arc::new(NoopEvaluator),
            command_router: Arc::new(NoopCommandRouter),
            window_lister: Arc::new(NoopWindowLister),
            console_errors: Arc::new(NoopConsoleErrors),
            wait_condition: Arc::new(NoopWaitCondition),
            window_manager: Arc::new(NoopWindowManager),
            window_geometry: Arc::new(NoopWindowGeometry),
            clipboard: Arc::new(NoopClipboard),
            target_info: TargetInfo {
                target: "test".to_string(),
                eyes: "mock".to_string(),
                hands: "mock".to_string(),
                eval_js: true,
                dom: true,
                state: true,
                console_errors: true,
                list_windows: true,
                notes: String::new(),
            },
        }
    }

    /// Construit un serveur de test a partir de `base_ports()`, en laissant
    /// `customize` remplacer les ports qu'un test veut observer/controler.
    fn server_with(customize: impl FnOnce(&mut DeveyePorts)) -> DeveyeMcpServer {
        let mut ports = base_ports();
        customize(&mut ports);
        DeveyeMcpServer::new(ports)
    }

    /// Extrait et parse en JSON le premier bloc `ContentBlock::text` d'un
    /// `CallToolResult` — tous les outils testes ici renvoient du JSON (E3).
    fn body_json(result: &CallToolResult) -> serde_json::Value {
        let text = result.content[0]
            .as_text()
            .expect("contenu texte attendu")
            .text
            .clone();
        serde_json::from_str(&text).expect("JSON attendu")
    }

    // ------------------------------------------------------------------
    // (a)/(b) click_selector : element absent (isError, dispatch jamais
    // appele) / element trouve (clic au centre, dispatch verifie par spy).
    // ------------------------------------------------------------------

    /// Evaluateur renvoyant une reponse JSON fixe, quel que soit le
    /// snippet — suffisant en test (pas de moteur JS reel derriere).
    struct FixedEval(serde_json::Value);
    #[async_trait]
    impl JsEvaluator for FixedEval {
        async fn evaluate(&self, _expression: &str) -> PortResult<serde_json::Value> {
            Ok(self.0.clone())
        }
    }

    /// Evaluateur qui echoue toujours avec `PortError::Unsupported` — simule
    /// une cible sans WebView (kwin-eis).
    struct UnsupportedEval(String);
    #[async_trait]
    impl JsEvaluator for UnsupportedEval {
        async fn evaluate(&self, _expression: &str) -> PortResult<serde_json::Value> {
            Err(PortError::Unsupported(self.0.clone()))
        }
    }

    /// Espion `InteractionSink` : enregistre chaque commande dispatchee, pour
    /// verifier (a) qu'elle n'a JAMAIS ete appelee, ou (b) qu'elle l'a ete
    /// avec les bons parametres.
    #[derive(Clone, Default)]
    struct DispatchSpy {
        calls: Arc<Mutex<Vec<InteractionCommand>>>,
    }
    #[async_trait]
    impl InteractionSink for DispatchSpy {
        async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
            self.calls.lock().unwrap().push(command);
            Ok(())
        }
    }

    #[tokio::test]
    async fn click_selector_is_error_when_element_absent_and_never_dispatches() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| {
            ports.evaluator = Arc::new(FixedEval(serde_json::json!({
                "ok": false,
                "reason": "not_found",
            })));
            ports.interaction = Arc::new(spy.clone());
        });
        let result = server
            .click_selector(Parameters(ClickSelectorParams {
                selector: ".missing".to_string(),
            }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        assert!(
            spy.calls.lock().unwrap().is_empty(),
            "interaction.dispatch ne doit jamais etre appele si l'element est absent"
        );
    }

    #[tokio::test]
    async fn click_selector_clicks_at_the_found_center() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| {
            ports.evaluator = Arc::new(FixedEval(serde_json::json!({
                "ok": true,
                "x": 42,
                "y": 84,
            })));
            ports.interaction = Arc::new(spy.clone());
        });
        let result = server
            .click_selector(Parameters(ClickSelectorParams {
                selector: ".present".to_string(),
            }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(false));
        let calls = spy.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            InteractionCommand::Click {
                position: Position::new(42, 84),
                button: MouseButton::Left,
            }
        );
    }

    #[test]
    fn parse_mouse_button_maps_all_cases() {
        // Absent / vide / "left" (toutes casses) => Left (retro-compat).
        assert_eq!(parse_mouse_button(None).unwrap(), MouseButton::Left);
        assert_eq!(parse_mouse_button(Some("")).unwrap(), MouseButton::Left);
        assert_eq!(parse_mouse_button(Some("left")).unwrap(), MouseButton::Left);
        assert_eq!(parse_mouse_button(Some("LEFT")).unwrap(), MouseButton::Left);
        assert_eq!(
            parse_mouse_button(Some("Right")).unwrap(),
            MouseButton::Right
        );
        assert_eq!(
            parse_mouse_button(Some("MIDDLE")).unwrap(),
            MouseButton::Middle
        );
        // Valeur inconnue => erreur protocolaire (invalid_params), pas un
        // silencieux repli sur Left.
        assert!(parse_mouse_button(Some("scroll")).is_err());
    }

    #[tokio::test]
    async fn click_dispatches_with_requested_button() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| {
            ports.interaction = Arc::new(spy.clone());
        });
        let result = server
            .click(Parameters(ClickParams {
                x: 10,
                y: 20,
                button: Some("right".to_string()),
            }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(false));
        let calls = spy.calls.lock().unwrap();
        assert_eq!(
            calls[0],
            InteractionCommand::Click {
                position: Position::new(10, 20),
                button: MouseButton::Right,
            }
        );
    }

    #[tokio::test]
    async fn click_with_invalid_button_is_protocol_error() {
        let server = server_with(|_| {});
        let err = server
            .click(Parameters(ClickParams {
                x: 0,
                y: 0,
                button: Some("nope".to_string()),
            }))
            .await;
        assert!(
            err.is_err(),
            "un button invalide doit etre une erreur -32602"
        );
    }

    // ------------------------------------------------------------------
    // E1 : PortError::Unsupported -> isError structure (pas une erreur
    // JSON-RPC opaque), sur les 3 traits mockes ici.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn eval_js_unsupported_becomes_structured_is_error() {
        let server = server_with(|ports| {
            ports.evaluator = Arc::new(UnsupportedEval(
                "eval JS indisponible en mode kwin-eis".to_string(),
            ));
        });
        let result = server
            .eval_js(Parameters(EvalParams {
                expression: "1 + 1".to_string(),
            }))
            .await
            .expect("E1 : Unsupported devient un isError semantique, pas une erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        let body = body_json(&result);
        assert_eq!(body["reason"], "unsupported");
        assert_eq!(body["detail"], "eval JS indisponible en mode kwin-eis");
    }

    struct UnsupportedInspectorMock(String);
    #[async_trait]
    impl AppInspector for UnsupportedInspectorMock {
        async fn dom_snapshot(&self) -> PortResult<DomTree> {
            Err(PortError::Unsupported(self.0.clone()))
        }
        async fn state_snapshot(&self) -> PortResult<StateDump> {
            Err(PortError::Unsupported(self.0.clone()))
        }
    }

    #[tokio::test]
    async fn dom_unsupported_becomes_structured_is_error() {
        let server = server_with(|ports| {
            ports.inspector = Arc::new(UnsupportedInspectorMock(
                "DOM indisponible en mode kwin-eis".to_string(),
            ));
        });
        let result = server.dom().await.expect("E1 : pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(body_json(&result)["reason"], "unsupported");
    }

    struct UnsupportedInteraction(String);
    #[async_trait]
    impl InteractionSink for UnsupportedInteraction {
        async fn dispatch(&self, _command: InteractionCommand) -> PortResult<()> {
            Err(PortError::Unsupported(self.0.clone()))
        }
    }

    #[tokio::test]
    async fn navigate_unsupported_in_kwin_eis_becomes_structured_is_error() {
        let server = server_with(|ports| {
            ports.interaction = Arc::new(UnsupportedInteraction(
                "Navigate necessite un routeur applicatif".to_string(),
            ));
        });
        let result = server
            .navigate(Parameters(NavigateParams {
                route: "Agenda".to_string(),
            }))
            .await
            .expect("E1 : pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(body_json(&result)["reason"], "unsupported");
    }

    // ------------------------------------------------------------------
    // resize : dispatch de la commande, validation, Unsupported structure.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn resize_dispatches_the_requested_dimensions() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| {
            ports.interaction = Arc::new(spy.clone());
        });
        let result = server
            .resize(Parameters(ResizeParams {
                width: 375,
                height: 812,
            }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(false));
        assert_eq!(
            spy.calls.lock().unwrap()[0],
            InteractionCommand::resize(375, 812)
        );
    }

    #[tokio::test]
    async fn resize_zero_by_zero_is_forwarded_to_clear_the_emulation() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| {
            ports.interaction = Arc::new(spy.clone());
        });
        server
            .resize(Parameters(ResizeParams {
                width: 0,
                height: 0,
            }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(
            spy.calls.lock().unwrap()[0],
            InteractionCommand::resize(0, 0)
        );
    }

    #[tokio::test]
    async fn resize_with_invalid_dimensions_is_invalid_params_and_never_dispatches() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| {
            ports.interaction = Arc::new(spy.clone());
        });
        for (width, height) in [(0, 600), (800, 0), (RESIZE_MAX_DIMENSION + 1, 600)] {
            let err = server
                .resize(Parameters(ResizeParams { width, height }))
                .await
                .expect_err("dimensions invalides");
            assert_eq!(
                err.code,
                rmcp::model::ErrorCode::INVALID_PARAMS,
                "{width}x{height}"
            );
        }
        assert!(spy.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn resize_unsupported_target_becomes_structured_is_error() {
        let server = server_with(|ports| {
            ports.interaction = Arc::new(UnsupportedInteraction(
                "Resize necessite un moteur de rendu pilotable".to_string(),
            ));
        });
        let result = server
            .resize(Parameters(ResizeParams {
                width: 800,
                height: 600,
            }))
            .await
            .expect("E1 : pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(body_json(&result)["reason"], "unsupported");
    }

    // ------------------------------------------------------------------
    // (c) assert_state rapporte les mismatches.
    // ------------------------------------------------------------------

    struct FixedState(BTreeMap<String, serde_json::Value>);
    #[async_trait]
    impl AppInspector for FixedState {
        async fn dom_snapshot(&self) -> PortResult<DomTree> {
            Ok(DomTree::new(serde_json::json!({})))
        }
        async fn state_snapshot(&self) -> PortResult<StateDump> {
            Ok(StateDump::new(self.0.clone()))
        }
    }

    #[tokio::test]
    async fn assert_state_reports_mismatches() {
        let mut entries = BTreeMap::new();
        entries.insert("a".to_string(), serde_json::json!(1));
        entries.insert("b".to_string(), serde_json::json!(2));
        let server = server_with(|ports| {
            ports.inspector = Arc::new(FixedState(entries));
        });
        let result = server
            .assert_state(Parameters(AssertStateParams {
                expected: serde_json::json!({ "a": 1, "b": 99, "c": 5 }),
            }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        let body = body_json(&result);
        assert_eq!(body["ok"], false);
        let mismatches = body["mismatches"].as_array().expect("tableau attendu");
        assert_eq!(mismatches.len(), 2, "b et c divergent, a est correct");
        let keys: std::collections::BTreeSet<&str> = mismatches
            .iter()
            .map(|m| m["key"].as_str().expect("cle string"))
            .collect();
        assert_eq!(
            keys,
            ["b", "c"]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
        );
    }

    // ------------------------------------------------------------------
    // (d) le gate serialise : 2 appels concurrents (sur deux ports/outils
    // differents) ne sont jamais dans leur section critique en meme temps.
    // ------------------------------------------------------------------

    /// Compteur de sections critiques simultanement en cours. Si le gate ne
    /// serialisait pas vraiment, deux appels concurrents feraient monter
    /// `max_observed` a 2 (chacun incremente `current` avant son `sleep`,
    /// sans attendre l'autre).
    struct ConcurrencyProbe {
        current: AtomicUsize,
        max_observed: AtomicUsize,
    }
    impl ConcurrencyProbe {
        fn new() -> Self {
            Self {
                current: AtomicUsize::new(0),
                max_observed: AtomicUsize::new(0),
            }
        }
        async fn enter_then_wait(&self) {
            let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_observed.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.current.fetch_sub(1, Ordering::SeqCst);
        }
    }

    struct ProbedRenderer(Arc<ConcurrencyProbe>);
    #[async_trait]
    impl Renderer for ProbedRenderer {
        async fn screenshot(&self) -> PortResult<Screenshot> {
            self.0.enter_then_wait().await;
            Ok(Screenshot::new_png(vec![0u8; 4], Size::new(2, 2)))
        }
        async fn viewport_size(&self) -> PortResult<Size> {
            Ok(Size::new(2, 2))
        }
    }

    struct ProbedInspector(Arc<ConcurrencyProbe>);
    #[async_trait]
    impl AppInspector for ProbedInspector {
        async fn dom_snapshot(&self) -> PortResult<DomTree> {
            self.0.enter_then_wait().await;
            Ok(DomTree::new(serde_json::json!({})))
        }
        async fn state_snapshot(&self) -> PortResult<StateDump> {
            Ok(StateDump::empty())
        }
    }

    #[tokio::test]
    async fn gate_serializes_two_concurrent_tool_calls() {
        let probe = Arc::new(ConcurrencyProbe::new());
        let server = server_with(|ports| {
            ports.renderer = Arc::new(ProbedRenderer(probe.clone()));
            ports.inspector = Arc::new(ProbedInspector(probe.clone()));
        });
        // `screenshot` passe par `acquire_gate` (instrumente), `dom` par un
        // `gate.lock()` direct : les deux tiennent le MEME mutex, preuve que
        // la serialisation couvre bien tous les outils et pas seulement ceux
        // instrumentes.
        let s1 = server.clone();
        let s2 = server.clone();
        let (r1, r2) = tokio::join!(async move { s1.screenshot().await }, async move {
            s2.dom().await
        });
        assert!(r1.is_ok());
        assert!(r2.is_ok());
        assert_eq!(
            probe.max_observed.load(Ordering::SeqCst),
            1,
            "le gate (max_concurrent==1) doit serialiser : jamais deux outils \
             a la fois dans leur section critique, meme sur deux ports differents"
        );
    }

    // ------------------------------------------------------------------
    // (e) wait_ready : succes immediat / timeout.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn wait_ready_succeeds_immediately_when_state_non_empty() {
        let mut entries = BTreeMap::new();
        entries.insert("ready".to_string(), serde_json::json!(true));
        let server = server_with(|ports| {
            ports.inspector = Arc::new(FixedState(entries));
        });
        let result = server
            .wait_ready(Parameters(WaitReadyParams { timeout_ms: 1_000 }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(false));
        assert_eq!(body_json(&result)["ready"], true);
    }

    #[tokio::test]
    async fn wait_ready_times_out_when_state_stays_empty() {
        let server = server_with(|ports| {
            ports.inspector = Arc::new(FixedState(BTreeMap::new()));
        });
        // timeout_ms=0 : des le premier tour de boucle, le state est vide
        // (pas pret) ET le budget (0 ms) est deja depasse -> retour
        // immediat sans jamais atteindre le `sleep(200ms)` de la boucle de
        // polling (test rapide et deterministe, pas de vrai delai).
        let result = server
            .wait_ready(Parameters(WaitReadyParams { timeout_ms: 0 }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(body_json(&result)["ready"], false);
    }
}
