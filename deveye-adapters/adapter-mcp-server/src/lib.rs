//! Adapter DevEye qui expose les ports `Renderer`, `AppInspector`,
//! `InteractionSink` et `LogSource` au format Model Context Protocol via
//! `rmcp` (transport stdio).
//!
//! Quarante-quatre outils stables, plus deux outils **expérimentaux** (`snapshot` et
//! `act`, hors promesse semver, ADR-0069 et ADR-0073), sont publiés à Claude (ou à
//! n'importe quel client MCP) :
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
//! - `key_hold` : touche maintenue pendant une durée bornée
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
//! - `logs_wait` : long-poll des logs — attend un log (de niveau >= `level_min`)
//!   jusqu'à un timeout, renvoie `{ events, timed_out, truncated, total }`
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
//! - `list_windows` : liste les fenêtres/onglets de la cible (CDP ; kwin-eis
//!   via KWin Scripting, sous KDE/Wayland seulement ; x11 via QueryTree ; vide
//!   en SDK)
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
//! - `observe` : outil composite — capture bornée, résumé de l'arbre, état,
//!   erreurs console et logs récents en un appel (ADR-0029) ; la section
//!   `elements` (références `snapshot`) s'y ajoute sur demande
//! - `snapshot` **(expérimental)** : arbre compact d'éléments avec références
//!   stables (`w:<id>` pour les widgets natifs), texte par défaut, JSON sur demande
//! - `act` **(expérimental)** : agit sur un élément par sa référence (position
//!   remesurée au moment de l'action, jamais de relocalisation silencieuse)
//!
//! **Sorties structurées (ADR-0067)** : 29 outils à petite sortie déclarent un
//! `outputSchema` et renvoient `structuredContent` en plus d'un texte inchangé
//! (voir `responses`) ; les autres restent en texte seul. `state` porte sa
//! fraîcheur et sa provenance dans `_meta`.
//!
//! **Annotations (ADR-0027)** : chaque outil porte les quatre indices MCP
//! (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`) ; un
//! test échoue si un outil n'est pas classé. **Protocole (ADR-0028)** : version
//! [`MCP_PROTOCOL_VERSION`], transport stdio seul.
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

use base64::engine::general_purpose::STANDARD as B64;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, ProtocolVersion, ServerCapabilities, ServerConfig},
    service::ServiceExt,
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::debug;

use imaging::{CaptureOptions, Region};
use responses::{
    outcome_result, output_schema, structured_failure, structured_ok, Ack, ClickAck, ErrorReason,
    FocusAck, FoundOutcome, ReadyOutcome, SelectorAssertion, StateAssertion, StateMismatch,
    ToolError,
};

use deveye_core::{
    InteractionCommand, LogLevel, Modifier, MouseButton, Position, StateDump, WaitPredicate,
    MAX_KEY_HOLD_MS,
};
use deveye_ports::{
    AppInspector, Clipboard, CommandRouter, ConsoleErrorSource, ElementInspector, InteractionSink,
    JsEvaluator, LogSource, PortError, PortResult, Renderer, WaitCondition, WindowGeometry,
    WindowLister, WindowManager,
};

// H1 : les outils `#[tool]` sont répartis par famille dans ces sous-modules
// (chacun `use super::*;` pour voir DeveyeMcpServer, les structs de
// parametres et les helpers ci-dessous) — lib.rs ne garde que la definition
// du serveur, les helpers communs et l'assemblage des routeurs (`new`).
mod imaging;
mod responses;
mod tools_capture;
mod tools_inspect;
mod tools_interaction;
mod tools_observe;
mod tools_perception;
mod tools_wait;

#[cfg(test)]
mod docs_tests;

#[cfg(test)]
mod contract_tests;

/// Version du protocole MCP annoncée à l'`initialize` et plafond de la
/// négociation (ADR-0028). Transport : stdio uniquement. Les clients plus
/// anciens (>= 2024-11-05) négocient à la baisse ; les annotations d'outils
/// (ADR-0027) existent depuis 2025-03-26 et sont ignorées des clients plus
/// anciens.
pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";

/// Nombre d'outils exposés. Source unique : le test de classification de ce
/// crate et le test de bout en bout du binaire (`deveye-server/tests/e2e_mcp.rs`)
/// l'exigent tous deux, de sorte qu'ajouter ou retirer un outil impose de
/// mettre à jour ce nombre, la classification et la documentation.
pub const EXPECTED_TOOL_COUNT: usize = 47;

/// Outils expérimentaux : hors de la promesse semver sur les noms, paramètres et
/// formes de réponse (ADR-0069). Leur description commence par
/// [`EXPERIMENTAL_PREFIX`] ; ils ne comptent pas dans les « 45 outils » annoncés
/// par la documentation.
pub const EXPERIMENTAL_TOOLS: &[&str] = &["snapshot", "act"];

/// Nombre d'outils stables : [`EXPECTED_TOOL_COUNT`] moins les expérimentaux.
pub const STABLE_TOOL_COUNT: usize = EXPECTED_TOOL_COUNT - EXPERIMENTAL_TOOLS.len();

/// Préfixe de la description d'un outil expérimental.
pub const EXPERIMENTAL_PREFIX: &str = "[expérimental]";

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

/// Identité du serveur : version du crate, commit de build et versions des deux
/// protocoles parlés (wire protocol du SDK, MCP). Exposée par `target_info` et
/// par le `serverInfo` de l'handshake MCP (ADR-0033).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ServerVersion {
    /// Version du binaire `deveye-server`.
    pub version: String,
    /// Commit court de la compilation ; `None` hors dépôt git (archive, crate publié).
    pub commit: Option<String>,
    /// Version `MAJEURE.MINEURE` du wire protocol du SDK que ce serveur implémente.
    pub sdk_protocol: String,
    /// Révision du protocole MCP annoncée à l'handshake.
    pub mcp_protocol: String,
}

impl ServerVersion {
    /// Identité par défaut (crate `adapter-mcp-server`, sans commit) ; le
    /// binaire la remplace par la sienne.
    pub fn new(version: impl Into<String>, commit: Option<String>, sdk_protocol: &str) -> Self {
        Self {
            version: version.into(),
            commit,
            sdk_protocol: sdk_protocol.to_string(),
            mcp_protocol: MCP_PROTOCOL_VERSION.to_string(),
        }
    }
}

/// Sonde du backend réellement utilisé, relue à chaque appel de `target_info`
/// (en mode `auto`, il change quand le CDP tombe puis revient).
pub type EffectiveBackendProbe = Arc<dyn Fn() -> String + Send + Sync>;

/// Description de la cible active et de ses capacités, renvoyee par l'outil
/// `target_info`. Construite par `deveye-server` (compose) selon le `--target`
/// choisi et la session détectée : permet a l'agent de savoir d'avance quels
/// outils répondront (ex. `eval_js`/`dom` absents en kwin-eis) sans tenter un
/// appel pour le découvrir. Seul `effective_backend` varie en cours de session.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct TargetInfo {
    /// Mode actif : "auto" | "cdp" | "sdk" | "kwin-eis" | "x11".
    pub target: String,
    /// Backend qui sert réellement les yeux et les mains a cet instant :
    /// "cdp", "sdk" (repli du mode `auto`), "kwin-eis", "x11", ou
    /// "indetermine" tant qu'aucun appel n'a tranche (mode `auto`).
    pub effective_backend: Option<String>,
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
    /// `list_windows` peut renvoyer des fenêtres ? CDP : onglets ; kwin-eis :
    /// fenêtres du workspace, **seulement sous KDE/Wayland** ; x11 : fenêtres
    /// du serveur privé ; faux en `sdk` (pas de visibilité inter-fenetres).
    pub list_windows: bool,
    /// `window_geometry` répond-il (KWin Scripting sous KDE/Wayland, ou serveur X privé) ?
    pub window_geometry: bool,
    /// `focus_window` répond-il (KWin avec `--screencap-window`, ou serveur X privé) ?
    pub focus_window: bool,
    /// Gestionnaire de fenetres derriere `list_windows`/`window_geometry`/
    /// `focus_window` : "kwin", "x11" ou "aucun".
    pub window_manager: String,
    /// `clipboard_get`/`clipboard_set` disponibles (Wayland ; jamais en x11) ?
    pub clipboard: bool,
    /// `snapshot` et `act` (expérimentaux) peuvent désigner des éléments sur cette
    /// cible ? Absent de la réponse quand c'est faux, pour garder la forme de la 1.1.
    #[serde(default, skip_serializing_if = "is_false")]
    pub elements: bool,
    /// Outils qui exigent un moteur JS (webview CDP ou pompe SDK) et répondent
    /// `Unsupported` quand `eval_js` est faux.
    pub js_dependent_tools: Vec<String>,
    /// Identité et versions du serveur.
    pub server: ServerVersion,
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
    /// Éléments désignables par référence (`snapshot`, `act`, expérimentaux).
    pub elements: Arc<dyn ElementInspector>,
    pub target_info: TargetInfo,
    /// Sonde du backend effectif (mode `auto`) ; `None` : celui de `target_info` fait foi.
    pub effective_backend: Option<EffectiveBackendProbe>,
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
    elements: Arc<dyn ElementInspector>,
    target_info: TargetInfo,
    effective_backend: Option<EffectiveBackendProbe>,
    /// Racine hors de laquelle `assert_visual` ne lit ni n'écrit (ADR-0049).
    baseline_root: std::path::PathBuf,
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

fn is_false(value: &bool) -> bool {
    !*value
}

/// Coordonnées seules — `hover`/`right_click`/`double_click` (le bouton y est
/// implicite ou fixe). Distincte de [`ClickParams`] (E6 : `click` seul a un
/// paramètre `button`) pour ne pas faire apparaître un champ sans effet dans
/// le schéma MCP des trois autres outils ; tous quatre portent `modifiers`.
#[derive(Debug, Deserialize, JsonSchema)]
struct PositionParams {
    /// Abscisse en pixels logiques depuis le coin haut-gauche du viewport.
    x: i32,
    /// Ordonnée en pixels logiques depuis le coin haut-gauche du viewport.
    y: i32,
    /// Modificateurs maintenus pendant le geste (insensibles à la casse) :
    /// `ctrl`/`control`, `shift`, `alt`, `meta`/`super`/`cmd`. Un nom inconnu est
    /// refusé (`invalid_params`) sans émettre le geste. Absent ou vide : geste nu.
    #[serde(default)]
    modifiers: Option<Vec<String>>,
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
    /// Modificateurs maintenus pendant le geste (insensibles à la casse) :
    /// `ctrl`/`control`, `shift`, `alt`, `meta`/`super`/`cmd`. Un nom inconnu est
    /// refusé (`invalid_params`) sans émettre le geste. Absent ou vide : geste nu.
    #[serde(default)]
    modifiers: Option<Vec<String>>,
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
    /// Modificateurs maintenus pendant le geste (insensibles à la casse) :
    /// `ctrl`/`control`, `shift`, `alt`, `meta`/`super`/`cmd`. Un nom inconnu est
    /// refusé (`invalid_params`) sans émettre le geste. Absent ou vide : geste nu.
    #[serde(default)]
    modifiers: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ClickTextParams {
    /// Texte exact (après `trim`) de l'élément à cliquer.
    text: String,
    /// Modificateurs maintenus pendant le geste (insensibles à la casse) :
    /// `ctrl`/`control`, `shift`, `alt`, `meta`/`super`/`cmd`. Un nom inconnu est
    /// refusé (`invalid_params`) sans émettre le geste. Absent ou vide : geste nu.
    #[serde(default)]
    modifiers: Option<Vec<String>>,
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
    /// Modificateurs maintenus pendant le geste (insensibles à la casse) :
    /// `ctrl`/`control`, `shift`, `alt`, `meta`/`super`/`cmd`. Un nom inconnu est
    /// refusé (`invalid_params`) sans émettre le geste. Absent ou vide : geste nu.
    #[serde(default)]
    modifiers: Option<Vec<String>>,
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
    /// Modificateurs maintenus pendant le geste (insensibles à la casse) :
    /// `ctrl`/`control`, `shift`, `alt`, `meta`/`super`/`cmd`. Un nom inconnu est
    /// refusé (`invalid_params`) sans émettre le geste. Absent ou vide : geste nu.
    #[serde(default)]
    modifiers: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct KeyHoldParams {
    /// Touche à maintenir (par exemple `Shift_L`). Noms comme `press_key` : XKB
    /// en kwin-eis, noms X11 en x11.
    key: String,
    /// Durée du maintien en millisecondes, de 0 à 10 000.
    duration_ms: u32,
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

/// `screenshot` : aucune sélection, seulement les options de coût (ADR-0029).
#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ScreenshotParams {
    #[serde(flatten)]
    capture: CaptureOptions,
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
    #[serde(flatten)]
    capture: CaptureOptions,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AssertVisualParams {
    /// Chemin du PNG de référence (baseline), relatif à la racine des baselines
    /// (`DEVEYE_BASELINE_DIR`, sinon le répertoire courant du serveur) ; un
    /// chemin absolu n'est accepté que dans cette racine, sans `..`, avec
    /// l'extension `.png`. S'il n'existe pas, il est créé à partir de la
    /// capture courante (`{ created: true }`) ; sinon la capture est comparée
    /// pixel à pixel à ce fichier.
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
    /// Rectangles à ignorer (horloge, curseur, animation), en pixels de l'image
    /// comparée — relatifs à la région si `x/y/width/height` sont fournis. Les
    /// pixels masqués sont exclus de la comparaison **et** du dénominateur de
    /// `diff_ratio`.
    #[serde(default)]
    ignore_regions: Option<Vec<Region>>,
    /// Si `true`, remplace la baseline par la capture courante (même si elle
    /// existe) au lieu de comparer : à n'utiliser qu'après avoir validé le
    /// changement visuel. Défaut `false`.
    #[serde(default)]
    update_baseline: Option<bool>,
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
    #[serde(flatten)]
    capture: CaptureOptions,
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

#[derive(Debug, Deserialize, JsonSchema)]
struct LogsWaitParams {
    /// Niveau minimal qui met fin à l'attente (`trace`/`debug`/`info`/`warn`/
    /// `error`, insensible à la casse). Absent : le premier log, quel que soit
    /// son niveau, met fin à l'attente.
    #[serde(default)]
    level_min: Option<String>,
    /// Délai maximal en millisecondes (défaut 5000, plafond 30000).
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Nombre maximal d'événements les plus RÉCENTS renvoyés (défaut 200), comme
    /// `logs_tail` ; `total` et `truncated` signalent ce qui a été omis.
    #[serde(default)]
    limit: Option<usize>,
}

/// Plafond de `logs_wait` : au-delà, le client MCP abandonne souvent l'appel.
const LOGS_WAIT_MAX_MS: u64 = 30_000;

/// Intervalle de sondage du buffer de logs par `logs_wait`.
const LOGS_WAIT_POLL_MS: u64 = 250;

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
            elements: ports.elements,
            target_info: ports.target_info,
            effective_backend: ports.effective_backend,
            baseline_root: tools_capture::default_baseline_root(),
            gate: Arc::new(tokio::sync::Mutex::new(())),
            tool_router: Self::tool_router(),
        }
    }

    /// Remplace la racine de confinement des baselines d'`assert_visual`
    /// (par défaut `DEVEYE_BASELINE_DIR`, sinon le répertoire courant).
    #[must_use]
    pub fn with_baseline_root(mut self, root: impl Into<std::path::PathBuf>) -> Self {
        self.baseline_root = root.into();
        self
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
            + Self::observe_router()
            + Self::perception_router()
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

    /// Valide les coordonnées d'un geste avant tout dispatch (ADR-0021).
    ///
    /// - une coordonnée **négative** est refusée sans aucune I/O, sur toutes
    ///   les cibles ;
    /// - la borne haute est vérifiée **au mieux** : si le renderer fournit la
    ///   taille du viewport (non nulle), un point `x >= largeur` ou
    ///   `y >= hauteur` est refusé ; s'il ne sait pas la fournir, le geste
    ///   passe plutôt que de bloquer un geste légitime.
    ///
    /// Dans les deux cas l'erreur est `invalid_params` : une faute d'appel que
    /// l'agent corrige, pas une panne. Sans cette garde, un clic hors fenêtre
    /// en `kwin-eis` ou `x11` atteint une autre application sans erreur.
    async fn ensure_in_viewport(&self, points: &[(i32, i32)]) -> Result<(), McpError> {
        if let Some(&(x, y)) = points.iter().find(|&&(x, y)| x < 0 || y < 0) {
            return Err(McpError::invalid_params(
                format!("coordonnées hors viewport : ({x}, {y}) — les valeurs négatives sont interdites"),
                None,
            ));
        }
        if let Ok(size) = self.renderer.viewport_size().await {
            if size.width > 0 && size.height > 0 {
                let (w, h) = (i64::from(size.width), i64::from(size.height));
                if let Some(&(x, y)) = points
                    .iter()
                    .find(|&&(x, y)| i64::from(x) >= w || i64::from(y) >= h)
                {
                    return Err(McpError::invalid_params(
                        format!("coordonnées hors viewport : ({x}, {y}) dépasse {w}x{h}"),
                        None,
                    ));
                }
            }
        }
        Ok(())
    }

    /// Comme [`Self::dispatch_ok`], pour un geste ancré à des points : les
    /// coordonnées sont validées sous le gate, avant le dispatch.
    async fn dispatch_at_ok(
        &self,
        cmd: InteractionCommand,
        points: &[(i32, i32)],
    ) -> Result<CallToolResult, McpError> {
        let _serial = self.gate.lock().await;
        self.ensure_in_viewport(points).await?;
        ok_or_unsupported(self.interaction.dispatch(cmd).await)
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
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        // Identité annoncée a l'handshake : sans elle, le client voit celle de la
        // bibliotheque rmcp et non celle de deveye-server (ADR-0033).
        info.server_info = rmcp::model::Implementation::new(
            "deveye-server",
            self.target_info.server.version.clone(),
        );
        info.instructions = Some(
            "DevEye MCP — observe et pilote une app vivante en dev. \
             Outils : screenshot, screenshot_region, screenshot_element, \
             assert_visual, click, double_click, right_click, hover, \
             click_selector, click_text, eval_js, type_text, press_key, \
             key_combo, key_hold, navigate, scroll, resize, handle_dialog, upload_file, \
             focus_window, window_geometry, clipboard_get, clipboard_set, dom, \
             state, assert_state, logs_tail, logs_wait, invoke_command, list_commands, \
             wait_for_selector, wait_for_text, find_element, assert_selector, \
             drag_drop, collect_console_errors, list_windows, wait_condition, \
             wait_for_log, wait_screenshot_region, wait_target_alive, \
             wait_ready, target_info, observe ; expérimentaux : snapshot, act. \
             Appeler target_info en premier pour connaitre les capacites de la \
             cible (eval_js/dom indisponibles en kwin-eis). observe resume \
             l'app en un appel (capture reduite, arbre, état, console, logs). \
             snapshot liste les éléments avec des références, act agit dessus."
                .to_string(),
        );
        info
    }

    /// ADR-0028 : on n'annonce que les révisions du protocole qu'on a testées,
    /// jusqu'à [`MCP_PROTOCOL_VERSION`] ; la révision suivante du SDK (sans
    /// `initialize`, par requête) n'est pas implémentée ici.
    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [ProtocolVersion]> {
        std::borrow::Cow::Borrowed(ProtocolVersion::known_up_to(&ProtocolVersion::V_2025_11_25))
    }
}

/// Sérialise `value` en texte JSON. Un échec (impossible pour les types
/// utilisés ici) est rendu **visiblement** par un objet d'erreur, jamais par un
/// `"null"` qui passerait pour un succès vide (ADR-0024).
pub(crate) fn to_json_text<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| {
        serde_json::json!({
            "ok": false,
            "reason": "serialization_error",
            "message": e.to_string(),
        })
        .to_string()
    })
}

/// Serialise `value` en texte JSON et emballe dans un [`CallToolResult`] succes.
/// La serialisation ne peut echouer sur les types utilises ici (tous implementent
/// `Serialize` correctement) ; un échec serait rendu visible (`to_json_text`). N14.
fn json_success<T: Serialize>(value: &T) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(to_json_text(value))])
}

/// Idem mais renvoie un [`CallToolResult`] erreur (`isError = true`).
/// Utilise pour les echecs semantiques (element absent H1, timeout L13) qui ne
/// doivent pas etre confondus avec un succes par le client MCP.
fn json_error<T: Serialize>(value: &T) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(to_json_text(value))])
}

/// E3 : sortie succes minimale des outils « fire-and-forget » (click,
/// type_text, scroll...) — JSON structure `{ "ok": true }` plutot que le
/// texte libre `"ok"` d'avant ce sprint, incoherent avec le reste de la
/// surface (assert_*, find_element, wait_*...) qui renvoie deja du JSON.
fn ok_result() -> CallToolResult {
    structured_ok(&Ack::done())
}

/// E1 : payload `isError` structure pour `PortError::Unsupported` — une
/// capacite absente sur la cible active (webview en kwin-eis, session
/// Wayland absente, KDE/Wayland absent...) est un cas NORMAL, pas une
/// panne : un resultat semantique, coherent avec le contrat `isError` deja
/// applique par les `assert_*`/`wait_*` — plutot que l'erreur JSON-RPC
/// opaque (-32603 `internal_error`) qu'aplatissait `port_to_mcp` jusqu'ici.
fn unsupported_error(detail: impl Into<String>) -> CallToolResult {
    ToolError::new(ErrorReason::Unsupported, detail).into_result()
}

/// Payload `isError` d'une exception levée par le JS evalue dans la cible
/// (`PortError::Script`) : un résultat que l'agent doit lire et corriger (son
/// expression est fausse), pas une panne du serveur ni de la liaison. Porte le
/// message et, quand le moteur la fournit, la trace (ADR-0023).
fn script_error(message: String, stack: Option<String>) -> CallToolResult {
    let mut payload = serde_json::json!({
        "ok": false,
        "reason": "js_exception",
        "message": message,
    });
    if let Some(stack) = stack {
        payload["stack"] = serde_json::Value::String(stack);
    }
    json_error(&payload)
}

/// Bloc texte de métadonnées joint, en **second** bloc, à la sortie des outils
/// d'observation (`screenshot`, `dom`, `state`) : le premier bloc garde sa forme
/// d'origine (ADR-0025), les clients qui n'en lisent qu'un n'y voient rien changer.
fn meta_block(meta: &serde_json::Value) -> ContentBlock {
    ContentBlock::text(meta.to_string())
}

/// Payload `isError` quand le SDK de l'app est injoignable (`PortError::Network`
/// sur le state) : distinct d'un SDK joignable qui n'a publié aucun signal
/// (succès avec `signals: 0`) — ADR-0025.
fn sdk_unavailable_error(detail: String) -> CallToolResult {
    ToolError::new(ErrorReason::SdkUnavailable, detail).into_result()
}

/// Comme [`unsupported_aware`], pour un `state_snapshot` : une coupure de
/// liaison avec le SDK devient un résultat `sdk_unavailable` structuré plutôt
/// qu'une erreur protocolaire opaque.
fn state_aware(
    result: PortResult<StateDump>,
) -> Result<StateDump, Result<CallToolResult, McpError>> {
    match result {
        Err(PortError::Network(detail)) => Err(Ok(sdk_unavailable_error(detail))),
        other => unsupported_aware(other),
    }
}

/// E1 : deroule un `PortResult<T>` en distinguant « capacite absente sur la
/// cible active » (`PortError::Unsupported`, cf. [`unsupported_error`]) et
/// « exception JS de l'expression evaluee » (`PortError::Script`, cf.
/// [`script_error`]) de toute autre panne de port (mappée par [`port_to_mcp`]). Sur `Ok`, renvoie
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
        Err(PortError::Script { message, stack }) => Err(Ok(script_error(message, stack))),
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

/// Parse le paramètre `modifiers` optionnel d'un outil de pointeur. Un nom
/// inconnu est une erreur protocolaire explicite (-32602 `invalid_params`) :
/// jamais un geste émis sans le modificateur demandé.
fn parse_modifier_names(names: Option<&[String]>) -> Result<Vec<Modifier>, McpError> {
    deveye_core::parse_modifiers(names.unwrap_or_default())
        .map_err(|e| McpError::invalid_params(format!("modifiers invalide : {e}"), None))
}

/// Enveloppe `command` d'un décorateur `WithModifiers` quand des modificateurs
/// sont demandés ; sans modificateur, la commande part inchangée (aucun
/// changement de comportement pour les appels existants).
fn decorate(
    command: InteractionCommand,
    modifiers: &[Modifier],
) -> Result<InteractionCommand, McpError> {
    if modifiers.is_empty() {
        return Ok(command);
    }
    InteractionCommand::with_modifiers(modifiers.to_vec(), command)
        .map_err(|e| McpError::invalid_params(e.to_string(), None))
}

/// `key_hold` : touche non vide et durée sous le plafond, avant tout dispatch.
fn validate_key_hold(key: &str, duration_ms: u32) -> Result<InteractionCommand, McpError> {
    InteractionCommand::key_hold(key, duration_ms).map_err(|e| {
        McpError::invalid_params(
            format!("{e} (durée de 0 à {MAX_KEY_HOLD_MS} ms, touche non vide)"),
            None,
        )
    })
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

/// Littéral de chaîne JavaScript (entre guillemets, échappé) pour `s`, à
/// insérer dans un snippet évalué dans la cible. S'appuie sur l'affichage de
/// `serde_json::Value`, qui ne peut pas échouer pour une chaîne en mémoire :
/// aucun `expect` ni valeur de repli silencieuse (ADR-0024).
fn js_string(s: &str) -> String {
    serde_json::Value::from(s).to_string()
}

/// Certains clients MCP sérialisent un argument objet en **chaîne** JSON
/// (`"{\"page\":\"Agenda\"}"` au lieu de `{"page":"Agenda"}`). Une chaîne qui
/// contient un objet ou un tableau JSON valide est donc relue comme tel ; toute
/// autre valeur (dont une chaîne ordinaire, un nombre ou un booléen écrit en
/// chaîne) est transmise inchangée, pour ne pas altérer un argument voulu comme
/// texte.
fn reparse_stringified_args(args: serde_json::Value) -> serde_json::Value {
    if let serde_json::Value::String(text) = &args {
        if let Ok(parsed @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) =
            serde_json::from_str::<serde_json::Value>(text)
        {
            return parsed;
        }
    }
    args
}

/// Interprète un niveau de log textuel (`trace`, `debug`, `info`, `warn`,
/// `error`, insensible à la casse) en réutilisant la désérialisation serde de
/// [`LogLevel`], donc sans table de correspondance à tenir à jour. Un niveau
/// inconnu est une faute d'appel (`invalid_params`), pas une panne interne.
fn parse_log_level(s: &str) -> Result<LogLevel, McpError> {
    serde_json::from_value::<LogLevel>(serde_json::Value::String(s.to_ascii_lowercase())).map_err(
        |_| {
            McpError::invalid_params(
                format!("niveau de log inconnu : {s:?} (attendu trace/debug/info/warn/error)"),
                None,
            )
        },
    )
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

#[cfg(test)]
mod tests {
    use super::{
        crop_png, dom_size_guard, filter_state_entries, png_dimensions, selector_assertion_ok,
        truncate_recent, DOM_MAX_BYTES,
    };
    use crate::imaging::visual_diff;
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
        let out = visual_diff(&a, &a, &[]).expect("diff ok");
        assert!(out.same_dims);
        assert_eq!(out.diff_ratio, 0.0);
    }

    #[test]
    fn visual_diff_dimension_mismatch_is_max() {
        let a = solid_png(4, 4, [0, 0, 0, 255]);
        let b = solid_png(8, 8, [0, 0, 0, 255]);
        let out = visual_diff(&a, &b, &[]).expect("diff ok");
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
        let out = visual_diff(&a, &b, &[]).expect("diff ok");
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
    use super::tools_observe::ObserveParams;
    use super::*;
    use async_trait::async_trait;
    use deveye_core::{
        CommandDescriptor, ConsoleError, DomTree, LogEvent, Screenshot, Size, StateDump,
        WaitOutcome, WindowInfo, WindowRect,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    // ------------------------------------------------------------------
    // Mocks « noop » — un par port, juste assez pour construire un
    // DeveyeMcpServer complet. Chaque test ne remplace que les ports qui
    // l'interessent via `server_with`.
    // ------------------------------------------------------------------

    /// PNG 2x2 valide : les outils de capture lisent désormais l'en-tête de
    /// l'image pour rapporter sa taille et son échelle (ADR-0029).
    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([9, 9, 9, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("encodage PNG de test");
        out
    }

    struct NoopRenderer;
    #[async_trait]
    impl Renderer for NoopRenderer {
        async fn screenshot(&self) -> PortResult<Screenshot> {
            Ok(Screenshot::new_png(tiny_png(), Size::new(2, 2)))
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

    struct NoopElements;
    #[async_trait]
    impl ElementInspector for NoopElements {
        async fn snapshot(
            &self,
            _query: &deveye_core::elements::SnapshotQuery,
        ) -> deveye_ports::ElementResult<deveye_core::elements::ElementSnapshot> {
            Err(deveye_ports::ElementError::Unsupported {
                hint: "mock".to_string(),
            })
        }
        async fn locate(
            &self,
            _reference: &str,
        ) -> deveye_ports::ElementResult<deveye_core::elements::ElementLocation> {
            Err(deveye_ports::ElementError::Unsupported {
                hint: "mock".to_string(),
            })
        }
    }

    pub(crate) fn base_ports() -> DeveyePorts {
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
            elements: Arc::new(NoopElements),
            target_info: TargetInfo {
                target: "test".to_string(),
                eyes: "mock".to_string(),
                hands: "mock".to_string(),
                eval_js: true,
                dom: true,
                state: true,
                console_errors: true,
                list_windows: true,
                window_geometry: true,
                focus_window: true,
                window_manager: "test".to_string(),
                clipboard: true,
                elements: false,
                js_dependent_tools: vec!["eval_js".to_string()],
                effective_backend: Some("test".to_string()),
                server: ServerVersion::new("9.9.9", Some("abc1234".to_string()), "1.0"),
                notes: String::new(),
            },
            effective_backend: None,
        }
    }

    /// Construit un serveur de test a partir de `base_ports()`, en laissant
    /// `customize` remplacer les ports qu'un test veut observer/controler.
    fn server_with(customize: impl FnOnce(&mut DeveyePorts)) -> DeveyeMcpServer {
        let mut ports = base_ports();
        customize(&mut ports);
        // Les baselines de test vivent dans le répertoire temporaire.
        DeveyeMcpServer::new(ports).with_baseline_root(std::env::temp_dir())
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

    /// Évaluateur dont le JS lève toujours une exception.
    struct ThrowingEval {
        message: String,
        stack: Option<String>,
    }
    #[async_trait]
    impl JsEvaluator for ThrowingEval {
        async fn evaluate(&self, _expression: &str) -> PortResult<serde_json::Value> {
            Err(PortError::Script {
                message: self.message.clone(),
                stack: self.stack.clone(),
            })
        }
    }

    /// Évaluateur dont la liaison est coupée (panne, pas exception).
    struct BrokenLinkEval;
    #[async_trait]
    impl JsEvaluator for BrokenLinkEval {
        async fn evaluate(&self, _expression: &str) -> PortResult<serde_json::Value> {
            Err(PortError::Network("connexion refusée".to_string()))
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

    fn mods(names: &[&str]) -> Option<Vec<String>> {
        Some(names.iter().map(|n| (*n).to_string()).collect())
    }

    #[tokio::test]
    async fn modifiers_decorate_the_pointer_gestures_of_the_eight_tools() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| {
            ports.interaction = Arc::new(spy.clone());
            ports.evaluator =
                Arc::new(FixedEval(serde_json::json!({ "ok": true, "x": 5, "y": 6 })));
        });
        let m = || mods(&["Shift", "control"]);
        let pos = |modifiers| PositionParams {
            x: 1,
            y: 1,
            modifiers,
        };
        server
            .click(Parameters(ClickParams {
                x: 1,
                y: 1,
                button: None,
                modifiers: m(),
            }))
            .await
            .unwrap();
        server.right_click(Parameters(pos(m()))).await.unwrap();
        server.double_click(Parameters(pos(m()))).await.unwrap();
        server.hover(Parameters(pos(m()))).await.unwrap();
        server
            .drag_drop(Parameters(DragDropParams {
                from_x: 1,
                from_y: 1,
                to_x: 1,
                to_y: 1,
                modifiers: m(),
            }))
            .await
            .unwrap();
        server
            .scroll(Parameters(ScrollParams {
                x: Some(1),
                y: Some(1),
                dx: None,
                dy: Some(100),
                modifiers: m(),
            }))
            .await
            .unwrap();
        server
            .click_selector(Parameters(ClickSelectorParams {
                selector: "#a".into(),
                modifiers: m(),
            }))
            .await
            .unwrap();
        server
            .click_text(Parameters(ClickTextParams {
                text: "a".into(),
                modifiers: m(),
            }))
            .await
            .unwrap();
        let calls = spy.calls.lock().unwrap();
        assert_eq!(calls.len(), 8);
        for call in calls.iter() {
            match call {
                InteractionCommand::WithModifiers { modifiers, command } => {
                    assert_eq!(modifiers, &vec![Modifier::Shift, Modifier::Ctrl]);
                    assert!(command.accepts_modifiers(), "{command:?}");
                }
                other => panic!("commande non decoree : {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn absent_or_empty_modifiers_keep_the_bare_command() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| ports.interaction = Arc::new(spy.clone()));
        for modifiers in [None, Some(vec![])] {
            server
                .hover(Parameters(PositionParams {
                    x: 1,
                    y: 1,
                    modifiers,
                }))
                .await
                .unwrap();
        }
        let calls = spy.calls.lock().unwrap();
        assert!(calls
            .iter()
            .all(|c| matches!(c, InteractionCommand::Hover { .. })));
        assert_eq!(calls.len(), 2);
    }

    #[tokio::test]
    async fn an_unknown_modifier_is_invalid_params_and_nothing_is_dispatched() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| {
            ports.interaction = Arc::new(spy.clone());
            ports.evaluator =
                Arc::new(FixedEval(serde_json::json!({ "ok": true, "x": 5, "y": 6 })));
        });
        let invalid = rmcp::model::ErrorCode::INVALID_PARAMS;
        let bad = || mods(&["ctrl", "hyper"]);
        let pos = || PositionParams {
            x: 1,
            y: 1,
            modifiers: bad(),
        };
        let errors = [
            server
                .click(Parameters(ClickParams {
                    x: 1,
                    y: 1,
                    button: None,
                    modifiers: bad(),
                }))
                .await
                .unwrap_err(),
            server.right_click(Parameters(pos())).await.unwrap_err(),
            server.double_click(Parameters(pos())).await.unwrap_err(),
            server.hover(Parameters(pos())).await.unwrap_err(),
            server
                .drag_drop(Parameters(DragDropParams {
                    from_x: 1,
                    from_y: 1,
                    to_x: 1,
                    to_y: 1,
                    modifiers: bad(),
                }))
                .await
                .unwrap_err(),
            server
                .scroll(Parameters(ScrollParams {
                    x: None,
                    y: None,
                    dx: None,
                    dy: Some(1),
                    modifiers: bad(),
                }))
                .await
                .unwrap_err(),
            server
                .click_selector(Parameters(ClickSelectorParams {
                    selector: "#a".into(),
                    modifiers: bad(),
                }))
                .await
                .unwrap_err(),
            server
                .click_text(Parameters(ClickTextParams {
                    text: "a".into(),
                    modifiers: bad(),
                }))
                .await
                .unwrap_err(),
        ];
        for e in &errors {
            assert_eq!(e.code, invalid, "{e:?}");
        }
        assert!(spy.calls.lock().unwrap().is_empty(), "rien ne doit partir");
    }

    #[tokio::test]
    async fn key_hold_dispatches_within_the_cap_and_refuses_beyond() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| ports.interaction = Arc::new(spy.clone()));
        let ok = server
            .key_hold(Parameters(KeyHoldParams {
                key: "Shift_L".into(),
                duration_ms: 300,
            }))
            .await
            .unwrap();
        assert_eq!(ok.is_error, Some(false));
        let invalid = rmcp::model::ErrorCode::INVALID_PARAMS;
        for (key, duration_ms) in [
            ("Shift_L", 20_000),
            ("Shift_L", MAX_KEY_HOLD_MS + 1),
            ("  ", 10),
        ] {
            let err = server
                .key_hold(Parameters(KeyHoldParams {
                    key: key.into(),
                    duration_ms,
                }))
                .await
                .unwrap_err();
            assert_eq!(err.code, invalid, "{key} {duration_ms}");
        }
        let calls = spy.calls.lock().unwrap();
        assert_eq!(
            calls.as_slice(),
            [InteractionCommand::key_hold("Shift_L", 300).unwrap()]
        );
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
                modifiers: None,
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
                modifiers: None,
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

    /// Renderer dont la taille de viewport est fixée par le test (`None` : la
    /// cible ne sait pas la fournir).
    struct SizedRenderer(Option<Size>);
    #[async_trait]
    impl Renderer for SizedRenderer {
        async fn screenshot(&self) -> PortResult<Screenshot> {
            Ok(Screenshot::new_png(tiny_png(), Size::new(2, 2)))
        }
        async fn viewport_size(&self) -> PortResult<Size> {
            self.0
                .ok_or_else(|| PortError::Render("taille inconnue".to_string()))
        }
    }

    fn click_params(x: i32, y: i32) -> ClickParams {
        ClickParams {
            x,
            y,
            button: None,
            modifiers: None,
        }
    }

    #[tokio::test]
    async fn click_with_negative_coordinates_is_invalid_params_and_never_dispatches() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| {
            ports.interaction = Arc::new(spy.clone());
            ports.renderer = Arc::new(SizedRenderer(Some(Size::new(800, 600))));
        });
        for (x, y) in [(-1, 10), (10, -1), (-5, -5)] {
            let err = server
                .click(Parameters(click_params(x, y)))
                .await
                .expect_err("coordonnées négatives");
            assert_eq!(
                err.code,
                rmcp::model::ErrorCode::INVALID_PARAMS,
                "({x}, {y})"
            );
        }
        assert!(spy.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn gestures_beyond_the_viewport_are_invalid_params_when_its_size_is_known() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| {
            ports.interaction = Arc::new(spy.clone());
            ports.renderer = Arc::new(SizedRenderer(Some(Size::new(800, 600))));
        });
        let invalid = rmcp::model::ErrorCode::INVALID_PARAMS;
        let position = |x, y| PositionParams {
            x,
            y,
            modifiers: None,
        };
        // Dernier pixel valide : 799 x 599 ; 800 ou 600 est déjà dehors.
        assert_eq!(
            server
                .click(Parameters(click_params(800, 10)))
                .await
                .unwrap_err()
                .code,
            invalid
        );
        assert_eq!(
            server
                .hover(Parameters(position(10, 600)))
                .await
                .unwrap_err()
                .code,
            invalid
        );
        assert_eq!(
            server
                .right_click(Parameters(position(900, 900)))
                .await
                .unwrap_err()
                .code,
            invalid
        );
        assert_eq!(
            server
                .double_click(Parameters(position(-3, 4)))
                .await
                .unwrap_err()
                .code,
            invalid
        );
        let drag = DragDropParams {
            from_x: 10,
            from_y: 10,
            to_x: 801,
            to_y: 10,
            modifiers: None,
        };
        assert_eq!(
            server.drag_drop(Parameters(drag)).await.unwrap_err().code,
            invalid
        );
        let scroll = ScrollParams {
            x: Some(10),
            y: Some(700),
            dx: None,
            dy: Some(100),
            modifiers: None,
        };
        assert_eq!(
            server.scroll(Parameters(scroll)).await.unwrap_err().code,
            invalid
        );
        assert!(spy.calls.lock().unwrap().is_empty());

        // Les bords valides passent.
        server
            .click(Parameters(click_params(799, 599)))
            .await
            .expect("dernier pixel valide");
        assert_eq!(spy.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn gestures_pass_when_the_viewport_size_is_unknown() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| {
            ports.interaction = Arc::new(spy.clone());
            ports.renderer = Arc::new(SizedRenderer(None));
        });
        server
            .click(Parameters(click_params(5000, 5000)))
            .await
            .expect("taille inconnue : on laisse passer");
        assert_eq!(spy.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn click_dispatches_with_requested_button() {
        let spy = DispatchSpy::default();
        let server = server_with(|ports| {
            ports.interaction = Arc::new(spy.clone());
            ports.renderer = Arc::new(SizedRenderer(Some(Size::new(800, 600))));
        });
        let result = server
            .click(Parameters(ClickParams {
                x: 10,
                y: 20,
                button: Some("right".to_string()),
                modifiers: None,
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
                modifiers: None,
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
    async fn eval_js_exception_becomes_is_error_with_message_and_stack() {
        let server = server_with(|ports| {
            ports.evaluator = Arc::new(ThrowingEval {
                message: "ReferenceError: inconnu is not defined".to_string(),
                stack: Some(
                    "ReferenceError: inconnu is not defined\n    at <anonymous>:1:1".to_string(),
                ),
            });
        });
        let result = server
            .eval_js(Parameters(EvalParams {
                expression: "inconnu".to_string(),
            }))
            .await
            .expect("G37 : une exception JS n'est pas une erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        let body = body_json(&result);
        assert_eq!(body["ok"], false);
        assert_eq!(body["reason"], "js_exception");
        assert_eq!(body["message"], "ReferenceError: inconnu is not defined");
        assert!(body["stack"].as_str().unwrap().contains("at <anonymous>"));
    }

    #[tokio::test]
    async fn eval_js_exception_without_stack_omits_the_field() {
        let server = server_with(|ports| {
            ports.evaluator = Arc::new(ThrowingEval {
                message: "boom".to_string(),
                stack: None,
            });
        });
        let result = server
            .eval_js(Parameters(EvalParams {
                expression: "throw 'boom'".to_string(),
            }))
            .await
            .expect("pas d'erreur protocolaire");
        let body = body_json(&result);
        assert_eq!(body["message"], "boom");
        assert!(body.get("stack").is_none());
    }

    #[tokio::test]
    async fn eval_js_link_failure_stays_a_protocol_error() {
        // Une vraie panne de liaison n'est pas une exception JS : elle reste
        // une erreur du serveur (-32603), distincte du résultat d'outil.
        let server = server_with(|ports| {
            ports.evaluator = Arc::new(BrokenLinkEval);
        });
        let err = server
            .eval_js(Parameters(EvalParams {
                expression: "1".to_string(),
            }))
            .await
            .expect_err("panne de liaison");
        assert_eq!(err.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
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
    // G42 : captured_at, state vide vs SDK indisponible, type_text sans focus.
    // ------------------------------------------------------------------

    /// Extrait le second bloc texte (métadonnées) d'un résultat.
    fn meta_json(result: &CallToolResult) -> serde_json::Value {
        let text = result.content[1]
            .as_text()
            .expect("second bloc texte attendu")
            .text
            .clone();
        serde_json::from_str(&text).expect("JSON attendu")
    }

    fn is_rfc3339(value: &serde_json::Value) -> bool {
        // Forme `AAAA-MM-JJTHH:MM:SS[.fff]Z` (pas de dépendance à chrono ici).
        value.as_str().is_some_and(|s| {
            let b = s.as_bytes();
            b.len() >= 20 && b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && s.ends_with('Z')
        })
    }

    #[tokio::test]
    async fn screenshot_carries_captured_at_and_viewport_in_a_second_block() {
        let server = server_with(|ports| {
            ports.renderer = Arc::new(SizedRenderer(Some(Size::new(800, 600))));
        });
        let result = server
            .screenshot(Parameters(ScreenshotParams::default()))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.content.len(), 2);
        assert!(result.content[0].as_image().is_some(), "image d'abord");
        let meta = meta_json(&result);
        assert!(is_rfc3339(&meta["captured_at"]), "{meta}");
        assert_eq!(meta["viewport"], json!({ "width": 2, "height": 2 }));
        assert_eq!(meta["image"]["scale"], 1.0, "{meta}");
        assert_eq!(meta["image"]["format"], "png");
    }

    #[tokio::test]
    async fn dom_keeps_its_tree_first_and_adds_captured_at() {
        let result = server_with(|_| {})
            .dom()
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.content.len(), 2);
        assert_eq!(body_json(&result), json!({}));
        assert!(is_rfc3339(&meta_json(&result)["captured_at"]));
    }

    /// Inspecteur dont `state_snapshot` renvoie un résultat fixé par le test.
    struct StateResult(Mutex<Option<PortResult<StateDump>>>);
    impl StateResult {
        fn new(result: PortResult<StateDump>) -> Self {
            Self(Mutex::new(Some(result)))
        }
    }
    #[async_trait]
    impl AppInspector for StateResult {
        async fn dom_snapshot(&self) -> PortResult<DomTree> {
            Ok(DomTree::new(json!({})))
        }
        async fn state_snapshot(&self) -> PortResult<StateDump> {
            self.0
                .lock()
                .unwrap()
                .take()
                .expect("un seul appel attendu")
        }
    }

    #[tokio::test]
    async fn state_with_signals_reports_their_count_and_captured_at() {
        let mut entries = BTreeMap::new();
        entries.insert("page".to_string(), json!("Agenda"));
        entries.insert("count".to_string(), json!(3));
        let server = server_with(|ports| {
            ports.inspector = Arc::new(StateResult::new(Ok(StateDump::new(entries))));
        });
        let result = server
            .state(Parameters(StateParams { keys: None }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(false));
        assert_eq!(body_json(&result), json!({ "count": 3, "page": "Agenda" }));
        let meta = meta_json(&result);
        assert_eq!(meta["signals"], 2);
        assert!(is_rfc3339(&meta["captured_at"]));
        assert!(meta.get("note").is_none());
    }

    #[tokio::test]
    async fn state_with_no_signal_says_the_sdk_is_reachable_but_empty() {
        let server = server_with(|ports| {
            ports.inspector = Arc::new(StateResult::new(Ok(StateDump::empty())));
        });
        let result = server
            .state(Parameters(StateParams { keys: None }))
            .await
            .expect("pas d'erreur protocolaire");
        // Succès (pas isError) : le SDK répond, il n'a simplement rien publié.
        assert_eq!(result.is_error, Some(false));
        assert_eq!(body_json(&result), json!({}));
        let meta = meta_json(&result);
        assert_eq!(meta["signals"], 0);
        assert!(meta["note"].as_str().unwrap().contains("SDK joignable"));
    }

    #[tokio::test]
    async fn state_with_an_unreachable_sdk_is_a_structured_is_error() {
        let server = server_with(|ports| {
            ports.inspector = Arc::new(StateResult::new(Err(PortError::Network(
                "GET http://127.0.0.1:31337/devtools/state : connexion refusée".to_string(),
            ))));
        });
        let result = server
            .state(Parameters(StateParams { keys: None }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        let body = body_json(&result);
        assert_eq!(body["reason"], "sdk_unavailable");
        assert!(body["detail"]
            .as_str()
            .unwrap()
            .contains("connexion refusée"));
    }

    #[tokio::test]
    async fn assert_state_with_an_unreachable_sdk_is_sdk_unavailable_not_a_mismatch() {
        let server = server_with(|ports| {
            ports.inspector = Arc::new(StateResult::new(Err(PortError::Network(
                "connexion refusée".to_string(),
            ))));
        });
        let result = server
            .assert_state(Parameters(AssertStateParams {
                expected: json!({ "page": "Agenda" }),
            }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(body_json(&result)["reason"], "sdk_unavailable");
    }

    struct FailingInteraction(PortError);
    #[async_trait]
    impl InteractionSink for FailingInteraction {
        async fn dispatch(&self, _command: InteractionCommand) -> PortResult<()> {
            Err(match &self.0 {
                PortError::Interaction(m) => PortError::Interaction(m.clone()),
                _ => PortError::Unsupported("autre".to_string()),
            })
        }
    }

    #[tokio::test]
    async fn type_text_without_editable_focus_is_a_structured_is_error() {
        let server = server_with(|ports| {
            ports.interaction = Arc::new(FailingInteraction(PortError::Interaction(
                "aucun champ éditable au focus".to_string(),
            )));
        });
        let result = server
            .type_text(Parameters(TypeParams {
                text: "bonjour".to_string(),
            }))
            .await
            .expect("pas d'erreur protocolaire : un résultat à corriger");
        assert_eq!(result.is_error, Some(true));
        let body = body_json(&result);
        assert_eq!(body["ok"], false);
        assert_eq!(body["reason"], "interaction_failed");
        assert!(body["detail"].as_str().unwrap().contains("éditable"));
    }

    #[tokio::test]
    async fn type_text_still_reports_unsupported_and_success_as_before() {
        let spy = DispatchSpy::default();
        let ok = server_with(|ports| {
            ports.interaction = Arc::new(spy.clone());
        })
        .type_text(Parameters(TypeParams {
            text: "salut".to_string(),
        }))
        .await
        .expect("pas d'erreur protocolaire");
        assert_eq!(ok.is_error, Some(false));
        assert_eq!(
            spy.calls.lock().unwrap()[0],
            InteractionCommand::type_text("salut")
        );

        let unsupported = server_with(|ports| {
            ports.interaction =
                Arc::new(FailingInteraction(PortError::Unsupported("x".to_string())));
        })
        .type_text(Parameters(TypeParams {
            text: "salut".to_string(),
        }))
        .await
        .expect("pas d'erreur protocolaire");
        assert_eq!(body_json(&unsupported)["reason"], "unsupported");
    }

    // ------------------------------------------------------------------
    // js_string / invoke_command : arguments stringifies.
    // ------------------------------------------------------------------

    #[test]
    fn js_string_escapes_quotes_backslashes_and_line_breaks() {
        assert_eq!(js_string("a\"b"), r#""a\"b""#);
        assert_eq!(js_string("a\\b"), r#""a\\b""#);
        assert_eq!(js_string("l1\nl2"), r#""l1\nl2""#);
        // Un sélecteur hostile reste une simple chaîne littérale.
        let hostile = "x\"); alert(1); (\"";
        let literal = js_string(hostile);
        let reparsed: String = serde_json::from_str(&literal).expect("littéral JSON valide");
        assert_eq!(reparsed, hostile);
    }

    #[test]
    fn reparse_stringified_args_unwraps_json_objects_and_arrays() {
        assert_eq!(
            reparse_stringified_args(json!("{\"page\":\"Agenda\",\"n\":2}")),
            json!({ "page": "Agenda", "n": 2 })
        );
        assert_eq!(
            reparse_stringified_args(json!("[1, 2, 3]")),
            json!([1, 2, 3])
        );
    }

    #[test]
    fn reparse_stringified_args_leaves_other_values_untouched() {
        for value in [
            json!(null),
            json!({ "deja": "objet" }),
            json!("simple texte"),
            // Une chaîne qui ressemble à un scalaire reste du texte voulu.
            json!("42"),
            json!("true"),
            json!("{pas du json"),
        ] {
            assert_eq!(reparse_stringified_args(value.clone()), value);
        }
    }

    /// Routeur qui enregistre le payload reçu.
    #[derive(Clone, Default)]
    struct PayloadSpy {
        seen: Arc<Mutex<Vec<serde_json::Value>>>,
    }
    #[async_trait]
    impl CommandRouter for PayloadSpy {
        async fn invoke(
            &self,
            _route: &str,
            payload: serde_json::Value,
        ) -> PortResult<serde_json::Value> {
            self.seen.lock().unwrap().push(payload);
            Ok(json!({ "ok": true }))
        }
        async fn list(&self) -> PortResult<Vec<CommandDescriptor>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn invoke_command_reparses_a_stringified_object_argument() {
        let spy = PayloadSpy::default();
        let server = server_with(|ports| {
            ports.command_router = Arc::new(spy.clone());
        });
        server
            .invoke_command(Parameters(InvokeCommandParams {
                name: "set-page".to_string(),
                args: json!("{\"page\":\"Agenda\"}"),
            }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(spy.seen.lock().unwrap()[0], json!({ "page": "Agenda" }));
    }

    // ------------------------------------------------------------------
    // logs_wait / parse_log_level.
    // ------------------------------------------------------------------

    /// Source de logs scriptée : chaque `poll` rend le lot suivant (vide une
    /// fois le script épuisé).
    struct ScriptedLogs(Mutex<std::collections::VecDeque<Vec<LogEvent>>>);
    impl ScriptedLogs {
        fn new(batches: Vec<Vec<LogEvent>>) -> Self {
            Self(Mutex::new(batches.into()))
        }
    }
    #[async_trait]
    impl LogSource for ScriptedLogs {
        async fn poll(&self) -> PortResult<Vec<LogEvent>> {
            Ok(self.0.lock().unwrap().pop_front().unwrap_or_default())
        }
    }

    fn log(level: LogLevel, message: &str) -> LogEvent {
        LogEvent::new(level, "test", message)
    }

    fn logs_wait_params(level_min: Option<&str>, timeout_ms: u64) -> LogsWaitParams {
        LogsWaitParams {
            level_min: level_min.map(str::to_string),
            timeout_ms: Some(timeout_ms),
            limit: None,
        }
    }

    #[test]
    fn parse_log_level_is_case_insensitive_and_rejects_unknown_levels() {
        assert_eq!(parse_log_level("warn").unwrap(), LogLevel::Warn);
        assert_eq!(parse_log_level("ERROR").unwrap(), LogLevel::Error);
        assert_eq!(parse_log_level("Info").unwrap(), LogLevel::Info);
        let err = parse_log_level("verbose").unwrap_err();
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn logs_wait_returns_as_soon_as_a_log_arrives() {
        let server = server_with(|ports| {
            ports.log_source = Arc::new(ScriptedLogs::new(vec![vec![log(
                LogLevel::Info,
                "demarre",
            )]]));
        });
        let result = server
            .logs_wait(Parameters(logs_wait_params(None, 2_000)))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(false));
        let body = body_json(&result);
        assert_eq!(body["timed_out"], false);
        assert_eq!(body["events"].as_array().unwrap().len(), 1);
        assert_eq!(body["events"][0]["message"], "demarre");
    }

    #[tokio::test]
    async fn logs_wait_filters_by_level_but_keeps_the_events_below() {
        let server = server_with(|ports| {
            ports.log_source = Arc::new(ScriptedLogs::new(vec![
                vec![log(LogLevel::Info, "bruit")],
                vec![log(LogLevel::Error, "boom")],
            ]));
        });
        let result = server
            .logs_wait(Parameters(logs_wait_params(Some("warn"), 5_000)))
            .await
            .expect("pas d'erreur protocolaire");
        let body = body_json(&result);
        assert_eq!(body["timed_out"], false);
        let messages: Vec<&str> = body["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["message"].as_str().unwrap())
            .collect();
        // Le log « info » drainé en chemin n'est pas perdu.
        assert_eq!(messages, ["bruit", "boom"]);
    }

    #[tokio::test]
    async fn logs_wait_times_out_when_nothing_matches() {
        let server = server_with(|ports| {
            ports.log_source = Arc::new(ScriptedLogs::new(vec![vec![log(
                LogLevel::Debug,
                "trop bas",
            )]]));
        });
        let result = server
            .logs_wait(Parameters(logs_wait_params(Some("error"), 100)))
            .await
            .expect("pas d'erreur protocolaire");
        let body = body_json(&result);
        assert_eq!(body["timed_out"], true);
        assert_eq!(body["events"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn logs_wait_reports_truncated_when_events_exceed_the_limit() {
        let server = server_with(|ports| {
            ports.log_source = Arc::new(ScriptedLogs::new(vec![vec![
                log(LogLevel::Info, "a"),
                log(LogLevel::Info, "b"),
                log(LogLevel::Info, "c"),
            ]]));
        });
        let mut params = logs_wait_params(None, 2_000);
        params.limit = Some(2);
        let result = server
            .logs_wait(Parameters(params))
            .await
            .expect("pas d'erreur protocolaire");
        let body = body_json(&result);
        assert_eq!(body["truncated"], true);
        assert_eq!(body["total"], 3);
        let messages: Vec<&str> = body["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["message"].as_str().unwrap())
            .collect();
        assert_eq!(messages, ["b", "c"], "les plus récents sont conservés");
    }

    /// Un lot puis une panne : le lot déjà drainé ne doit pas être perdu.
    struct FailAfterFirst(Mutex<Option<Vec<LogEvent>>>);
    #[async_trait]
    impl LogSource for FailAfterFirst {
        async fn poll(&self) -> PortResult<Vec<LogEvent>> {
            match self.0.lock().unwrap().take() {
                Some(batch) => Ok(batch),
                None => Err(PortError::Log("source morte".to_string())),
            }
        }
    }

    #[tokio::test]
    async fn logs_wait_keeps_drained_events_when_a_later_poll_fails() {
        let server = server_with(|ports| {
            ports.log_source = Arc::new(FailAfterFirst(Mutex::new(Some(vec![log(
                LogLevel::Info,
                "deja drainé",
            )]))));
        });
        let result = server
            .logs_wait(Parameters(logs_wait_params(Some("error"), 5_000)))
            .await
            .expect("les logs drainés sont renvoyés, pas perdus");
        let body = body_json(&result);
        assert_eq!(body["events"][0]["message"], "deja drainé");
        assert!(body["poll_error"]
            .as_str()
            .unwrap()
            .contains("source morte"));
        assert_eq!(body["timed_out"], false);
    }

    #[tokio::test]
    async fn logs_wait_fails_plainly_when_the_first_poll_fails() {
        let server = server_with(|ports| {
            ports.log_source = Arc::new(FailAfterFirst(Mutex::new(None)));
        });
        let err = server
            .logs_wait(Parameters(logs_wait_params(None, 100)))
            .await
            .expect_err("rien de collecté : erreur franche");
        assert!(err.message.contains("source morte"));
    }

    #[tokio::test]
    async fn logs_wait_with_invalid_level_is_invalid_params() {
        let server = server_with(|_| {});
        let err = server
            .logs_wait(Parameters(logs_wait_params(Some("verbose"), 100)))
            .await
            .expect_err("level_min invalide");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn wait_for_log_with_invalid_level_is_invalid_params() {
        let server = server_with(|_| {});
        let err = server
            .wait_for_log(Parameters(WaitForLogParams {
                contains: "x".to_string(),
                level_min: Some("verbose".to_string()),
                timeout_ms: Some(100),
            }))
            .await
            .expect_err("level_min invalide");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
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
    // différents) ne sont jamais dans leur section critique en même temps.
    // ------------------------------------------------------------------

    /// Compteur de sections critiques simultanement en cours. Si le gate ne
    /// sérialisait pas vraiment, deux appels concurrents feraient monter
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
            Ok(Screenshot::new_png(tiny_png(), Size::new(2, 2)))
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
        // la sérialisation couvre bien tous les outils et pas seulement ceux
        // instrumentes.
        let s1 = server.clone();
        let s2 = server.clone();
        let (r1, r2) = tokio::join!(
            async move { s1.screenshot(Parameters(ScreenshotParams::default())).await },
            async move { s2.dom().await }
        );
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

    // ------------------------------------------------------------------
    // Annotations d'outils et version de protocole (ADR-0027, ADR-0028).
    // ------------------------------------------------------------------

    /// Classification attendue de CHAQUE outil : (nom, read_only, destructive,
    /// idempotent, open_world). Ajouter un outil impose de le classer ici, donc
    /// de réfléchir à son risque (règle dans ADR-0027 et docs/usage.md).
    const CLASSIFICATION: &[(&str, bool, bool, bool, bool)] = &[
        // Observation pure.
        ("screenshot", true, false, true, false),
        ("screenshot_region", true, false, true, false),
        ("dom", true, false, true, false),
        ("state", true, false, true, false),
        ("assert_state", true, false, true, false),
        ("find_element", true, false, true, false),
        ("assert_selector", true, false, true, false),
        ("list_commands", true, false, true, false),
        ("target_info", true, false, true, false),
        ("list_windows", true, false, true, false),
        ("window_geometry", true, false, true, false),
        // Lit les données de l'hôte (presse-papiers réel) : monde ouvert (ADR-0050).
        ("clipboard_get", true, false, true, true),
        ("wait_for_selector", true, false, true, false),
        ("wait_for_text", true, false, true, false),
        ("wait_screenshot_region", true, false, true, false),
        ("wait_target_alive", true, false, true, false),
        ("wait_ready", true, false, true, false),
        // Observation qui draine un buffer de l'observateur (non idempotente).
        ("logs_tail", true, false, false, false),
        ("logs_wait", true, false, false, false),
        ("collect_console_errors", true, false, false, false),
        ("wait_for_log", true, false, false, false),
        ("wait_condition", true, false, false, false),
        ("observe", true, false, false, false),
        // Expérimentaux (ADR-0073) : `snapshot` observe sans rien changer ; `act`
        // écrit sans rien détruire, deux appels identiques peuvent différer.
        ("snapshot", true, false, true, false),
        ("act", false, false, false, false),
        // Modifie l'app de façon non destructive.
        ("screenshot_element", false, false, true, false),
        ("hover", false, false, true, false),
        ("resize", false, false, true, false),
        ("focus_window", false, false, true, false),
        ("click", false, false, false, false),
        ("right_click", false, false, false, false),
        ("double_click", false, false, false, false),
        ("click_selector", false, false, false, false),
        ("click_text", false, false, false, false),
        ("type_text", false, false, false, false),
        ("press_key", false, false, false, false),
        ("drag_drop", false, false, false, false),
        ("handle_dialog", false, false, false, false),
        ("scroll", false, false, false, false),
        // Potentiellement destructif (écrase, quitte, exécute du code arbitraire).
        ("assert_visual", false, true, false, false),
        ("key_combo", false, true, false, false),
        // Appui maintenu : touche tenue jusqu'à 10 s, effet non borné par DevEye (ADR-0075).
        ("key_hold", false, true, false, false),
        ("navigate", false, true, true, false),
        ("upload_file", false, true, true, false),
        ("clipboard_set", false, true, true, false),
        ("eval_js", false, true, false, true),
        ("invoke_command", false, true, false, true),
    ];

    #[test]
    fn every_tool_is_annotated_and_classified() {
        let tools = DeveyeMcpServer::tool_router().list_all();
        let mut seen = Vec::new();
        for tool in &tools {
            let name = tool.name.as_ref();
            let a = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("l'outil `{name}` n'a aucune annotation (ADR-0027)"));
            let hints = (
                a.read_only_hint,
                a.destructive_hint,
                a.idempotent_hint,
                a.open_world_hint,
            );
            let (Some(ro), Some(de), Some(id), Some(ow)) = hints else {
                panic!("l'outil `{name}` doit poser les QUATRE indices explicitement : {hints:?}");
            };
            assert!(
                !(ro && de),
                "`{name}` ne peut être à la fois lecture seule et destructif"
            );
            let expected = CLASSIFICATION
                .iter()
                .find(|(n, ..)| *n == name)
                .unwrap_or_else(|| panic!("`{name}` absent de CLASSIFICATION : le classer"));
            assert_eq!(
                (ro, de, id, ow),
                (expected.1, expected.2, expected.3, expected.4),
                "classification de `{name}`"
            );
            seen.push(name.to_string());
        }
        for (name, ..) in CLASSIFICATION {
            assert!(
                seen.iter().any(|s| s == name),
                "CLASSIFICATION cite `{name}`, qui n'est plus un outil"
            );
        }
        assert_eq!(tools.len(), CLASSIFICATION.len());
        assert_eq!(tools.len(), EXPECTED_TOOL_COUNT);
    }

    #[test]
    fn capture_tools_expose_the_cost_options_in_their_input_schema() {
        let tools = DeveyeMcpServer::tool_router().list_all();
        for name in [
            "screenshot",
            "screenshot_region",
            "screenshot_element",
            "observe",
        ] {
            let tool = tools
                .iter()
                .find(|t| t.name.as_ref() == name)
                .unwrap_or_else(|| panic!("outil {name} absent"));
            let props = tool
                .input_schema
                .get("properties")
                .and_then(|p| p.as_object())
                .unwrap_or_else(|| panic!("{name} : schéma sans propriétés"));
            for opt in ["max_width", "format", "quality"] {
                assert!(props.contains_key(opt), "{name} sans `{opt}` : {props:?}");
            }
        }
    }

    /// ADR-0033 : l'handshake annonce deveye-server et sa version, pas rmcp.
    #[test]
    fn server_info_announces_the_server_identity() {
        let info = server_with(|_| {}).get_info();
        assert_eq!(info.server_info.name, "deveye-server");
        assert_eq!(info.server_info.version, "9.9.9");
    }

    /// `target_info` porte les capacités complètes, les versions et relit le
    /// backend effectif a chaque appel (repli du mode auto, ADR-0034).
    #[tokio::test]
    async fn target_info_exposes_versions_capabilities_and_live_backend() {
        let state = Arc::new(std::sync::Mutex::new("cdp".to_string()));
        let probe_state = state.clone();
        let server = server_with(move |p| {
            p.effective_backend = Some(Arc::new(move || {
                probe_state
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_else(|e| e.into_inner().clone())
            }));
        });
        let first = body_json(&server.target_info().await.expect("target_info"));
        assert_eq!(first["effective_backend"], "cdp");
        assert_eq!(first["server"]["version"], "9.9.9");
        assert_eq!(first["server"]["commit"], "abc1234");
        assert_eq!(first["server"]["sdk_protocol"], "1.0");
        assert_eq!(first["server"]["mcp_protocol"], MCP_PROTOCOL_VERSION);
        for key in [
            "window_geometry",
            "focus_window",
            "window_manager",
            "clipboard",
            "js_dependent_tools",
        ] {
            assert!(first.get(key).is_some(), "champ `{key}` absent : {first}");
        }
        *state.lock().expect("verrou") = "sdk".to_string();
        let second = body_json(&server.target_info().await.expect("target_info"));
        assert_eq!(second["effective_backend"], "sdk");
    }

    fn init_request(version: ProtocolVersion) -> rmcp::model::InitializeRequestParams {
        rmcp::model::InitializeRequestParams::new(
            rmcp::model::ClientCapabilities::default(),
            rmcp::model::Implementation::new("test", "0"),
        )
        .with_protocol_version(version)
    }

    #[test]
    fn protocol_version_is_pinned_and_negotiated_down_from_newer_clients() {
        assert_eq!(MCP_PROTOCOL_VERSION, ProtocolVersion::V_2025_11_25.as_str());
        let server = server_with(|_| {});
        assert_eq!(
            server.get_info().protocol_version,
            ProtocolVersion::V_2025_11_25
        );
        for (asked, answered) in [
            (ProtocolVersion::V_2025_11_25, ProtocolVersion::V_2025_11_25),
            (ProtocolVersion::V_2025_06_18, ProtocolVersion::V_2025_06_18),
            // Un client plus récent que notre plafond est ramené au plafond.
            (ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25),
        ] {
            let result = server
                .negotiate_initialize(&init_request(asked.clone()))
                .expect("négociation");
            assert_eq!(
                result.protocol_version, answered,
                "client demandant {asked}"
            );
        }
        assert!(!server
            .supported_protocol_versions()
            .contains(&ProtocolVersion::V_2026_07_28));
    }

    // ------------------------------------------------------------------
    // Options de coût des captures (ADR-0029).
    // ------------------------------------------------------------------

    fn png_of(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba(rgba))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("encodage PNG de test");
        out
    }

    /// Renderer dont le PNG est modifiable par le test entre deux captures.
    #[derive(Clone)]
    struct SwitchRenderer(Arc<Mutex<Vec<u8>>>);
    impl SwitchRenderer {
        fn new(png: Vec<u8>) -> Self {
            Self(Arc::new(Mutex::new(png)))
        }
        fn set(&self, png: Vec<u8>) {
            *self.0.lock().unwrap() = png;
        }
    }
    #[async_trait]
    impl Renderer for SwitchRenderer {
        async fn screenshot(&self) -> PortResult<Screenshot> {
            let png = self.0.lock().unwrap().clone();
            let (w, h) = png_dimensions(&png).expect("PNG de test");
            Ok(Screenshot::new_png(png, Size::new(w, h)))
        }
        async fn viewport_size(&self) -> PortResult<Size> {
            let (w, h) = png_dimensions(&self.0.lock().unwrap()).expect("PNG de test");
            Ok(Size::new(w, h))
        }
    }

    fn capture_options(
        max_width: Option<u32>,
        format: Option<&str>,
        quality: Option<u32>,
    ) -> CaptureOptions {
        serde_json::from_value(json!({
            "max_width": max_width, "format": format, "quality": quality
        }))
        .expect("options de capture")
    }

    #[tokio::test]
    async fn screenshot_max_width_reports_size_and_scale() {
        let server = server_with(|p| {
            p.renderer = Arc::new(SwitchRenderer::new(png_of(400, 200, [1, 2, 3, 255])));
        });
        let result = server
            .screenshot(Parameters(ScreenshotParams {
                capture: capture_options(Some(100), None, None),
            }))
            .await
            .expect("pas d'erreur protocolaire");
        let image = result.content[0].as_image().expect("image");
        let bytes = base64::Engine::decode(&B64, &image.data).expect("base64");
        assert_eq!(png_dimensions(&bytes).unwrap(), (100, 50));
        let meta = meta_json(&result);
        assert_eq!(meta["image"]["width"], 100);
        assert_eq!(meta["image"]["source_width"], 400);
        assert_eq!(meta["image"]["scale"], 0.25);
        assert_eq!(meta["viewport"], json!({ "width": 400, "height": 200 }));
    }

    #[tokio::test]
    async fn screenshot_jpeg_has_the_jpeg_mime_type() {
        let server = server_with(|p| {
            p.renderer = Arc::new(SwitchRenderer::new(png_of(64, 64, [200, 10, 10, 255])));
        });
        let result = server
            .screenshot(Parameters(ScreenshotParams {
                capture: capture_options(None, Some("jpeg"), Some(70)),
            }))
            .await
            .expect("pas d'erreur protocolaire");
        let image = result.content[0].as_image().expect("image");
        assert_eq!(image.mime_type, "image/jpeg");
        assert_eq!(meta_json(&result)["image"]["format"], "jpeg");
    }

    #[tokio::test]
    async fn invalid_capture_options_are_invalid_params_before_any_capture() {
        let server = server_with(|_| {});
        for bad in [
            capture_options(Some(0), None, None),
            capture_options(None, Some("bmp"), None),
            capture_options(None, Some("jpeg"), Some(500)),
        ] {
            let err = server
                .screenshot(Parameters(ScreenshotParams { capture: bad }))
                .await
                .expect_err("options invalides");
            assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        }
    }

    #[tokio::test]
    async fn screenshot_region_carries_origin_and_scale() {
        let server = server_with(|p| {
            p.renderer = Arc::new(SwitchRenderer::new(png_of(400, 200, [1, 2, 3, 255])));
        });
        let result = server
            .screenshot_region(Parameters(ScreenshotRegionParams {
                x: 40,
                y: 20,
                width: 200,
                height: 100,
                capture: capture_options(Some(50), None, None),
            }))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.content.len(), 2);
        let meta = meta_json(&result);
        assert_eq!(meta["image"]["origin"], json!({ "x": 40, "y": 20 }));
        assert_eq!(meta["image"]["source_width"], 200);
        assert_eq!(meta["image"]["width"], 50);
        assert_eq!(meta["image"]["scale"], 0.25);
    }

    #[tokio::test]
    async fn screenshot_element_carries_origin_and_scale() {
        let server = server_with(|p| {
            p.renderer = Arc::new(SwitchRenderer::new(png_of(400, 200, [1, 2, 3, 255])));
            p.evaluator = Arc::new(FixedEval(json!({
                "found": true, "x": 10.0, "y": 20.0, "width": 100.0, "height": 50.0
            })));
        });
        let result = server
            .screenshot_element(Parameters(ScreenshotElementParams {
                selector: "#x".to_string(),
                capture: capture_options(Some(50), None, None),
            }))
            .await
            .expect("pas d'erreur protocolaire");
        let meta = meta_json(&result);
        assert_eq!(meta["image"]["origin"], json!({ "x": 10, "y": 20 }));
        assert_eq!(meta["image"]["scale"], 0.5);
    }

    // ------------------------------------------------------------------
    // assert_visual enrichi (ADR-0030).
    // ------------------------------------------------------------------

    fn scratch_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("deveye-av-{}-{name}", std::process::id()))
    }

    fn visual_params(path: &std::path::Path) -> AssertVisualParams {
        serde_json::from_value(json!({ "baseline_path": path.to_str().unwrap() }))
            .expect("params assert_visual")
    }

    fn png_with_pixels(w: u32, h: u32, lit: &[(u32, u32)]) -> Vec<u8> {
        let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));
        for &(x, y) in lit {
            img.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
        }
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("encodage PNG de test");
        out
    }

    #[tokio::test]
    async fn assert_visual_failure_returns_the_diff_image_and_bbox() {
        let path = scratch_path("fail.png");
        let _ = std::fs::remove_file(&path);
        let renderer = SwitchRenderer::new(png_with_pixels(20, 10, &[]));
        let server = server_with(|p| p.renderer = Arc::new(renderer.clone()));
        // 1re passe : crée la baseline.
        let created = server
            .assert_visual(Parameters(visual_params(&path)))
            .await
            .expect("création");
        assert_eq!(body_json(&created)["created"], true);
        // 2e passe : capture différente => échec avec image de différence.
        renderer.set(png_with_pixels(20, 10, &[(3, 4), (5, 6)]));
        let mut params = visual_params(&path);
        params.threshold = Some(0.0);
        let result = server
            .assert_visual(Parameters(params))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(result.content.len(), 2, "JSON puis image de différence");
        let payload = body_json(&result);
        assert_eq!(payload["ok"], false);
        assert_eq!(payload["diff_pixels"], 2);
        assert_eq!(payload["compared_pixels"], 200);
        assert_eq!(
            payload["diff_bbox"],
            json!({ "x": 3, "y": 4, "width": 3, "height": 3 })
        );
        assert_eq!(payload["diff_image"]["scale"], 1.0);
        let image = result.content[1].as_image().expect("image de différence");
        let bytes = base64::Engine::decode(&B64, &image.data).expect("base64");
        let img = image::load_from_memory(&bytes).expect("PNG").to_rgba8();
        assert_eq!(img.get_pixel(3, 4).0, [255, 0, 0, 255]);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn assert_visual_ignore_regions_make_a_moving_zone_pass() {
        let path = scratch_path("mask.png");
        let _ = std::fs::remove_file(&path);
        let renderer = SwitchRenderer::new(png_with_pixels(20, 10, &[]));
        let server = server_with(|p| p.renderer = Arc::new(renderer.clone()));
        server
            .assert_visual(Parameters(visual_params(&path)))
            .await
            .expect("création");
        // Un pixel change dans la zone « horloge » (en haut à droite).
        renderer.set(png_with_pixels(20, 10, &[(18, 1)]));
        let mut params = visual_params(&path);
        params.threshold = Some(0.0);
        let unmasked = server
            .assert_visual(Parameters(params))
            .await
            .expect("sans masque");
        assert_eq!(unmasked.is_error, Some(true));

        let mut params = visual_params(&path);
        params.threshold = Some(0.0);
        params.ignore_regions = Some(vec![Region {
            x: 15,
            y: 0,
            width: 5,
            height: 3,
        }]);
        let masked = server
            .assert_visual(Parameters(params))
            .await
            .expect("avec masque");
        assert_eq!(masked.is_error, Some(false));
        let payload = body_json(&masked);
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["compared_pixels"], 185);
        assert_eq!(payload["ignored_regions"], 1);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn assert_visual_fully_masked_comparison_is_inconclusive_not_ok() {
        let path = scratch_path("fullmask.png");
        let _ = std::fs::remove_file(&path);
        let renderer = SwitchRenderer::new(png_with_pixels(20, 10, &[]));
        let server = server_with(|p| p.renderer = Arc::new(renderer.clone()));
        server
            .assert_visual(Parameters(visual_params(&path)))
            .await
            .expect("création");
        renderer.set(png_with_pixels(20, 10, &[(3, 4)]));
        let mut params = visual_params(&path);
        params.ignore_regions = Some(vec![Region {
            x: 0,
            y: 0,
            width: 20,
            height: 10,
        }]);
        let result = server
            .assert_visual(Parameters(params))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        let payload = body_json(&result);
        assert_eq!(payload["ok"], false);
        assert_eq!(payload["reason"], "fully_masked");
        assert_eq!(payload["compared_pixels"], 0);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn assert_visual_refuses_paths_outside_the_baseline_root() {
        let server = server_with(|_| {});
        let outside = std::env::current_dir()
            .unwrap()
            .join("deveye-outside-root.png");
        for bad in [
            outside.to_str().unwrap(),
            "../x.png",
            "/etc/hosts",
            "notes.txt",
        ] {
            let params: AssertVisualParams =
                serde_json::from_value(json!({ "baseline_path": bad, "update_baseline": true }))
                    .expect("params");
            let err = server
                .assert_visual(Parameters(params))
                .await
                .expect_err("chemin refusé");
            assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{bad}");
        }
        assert!(!outside.exists(), "rien ne doit être écrit hors racine");
    }

    #[tokio::test]
    async fn assert_visual_update_refuses_to_overwrite_a_non_png_file() {
        let path = scratch_path("not-a-png.png");
        std::fs::write(&path, b"mot de passe").expect("fichier de test");
        let server = server_with(|_| {});
        let mut params = visual_params(&path);
        params.update_baseline = Some(true);
        let err = server
            .assert_visual(Parameters(params))
            .await
            .expect_err("écrasement refusé");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert_eq!(std::fs::read(&path).unwrap(), b"mot de passe");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn assert_visual_update_baseline_replaces_the_reference() {
        let path = scratch_path("update.png");
        let _ = std::fs::remove_file(&path);
        let renderer = SwitchRenderer::new(png_with_pixels(8, 8, &[]));
        let server = server_with(|p| p.renderer = Arc::new(renderer.clone()));
        server
            .assert_visual(Parameters(visual_params(&path)))
            .await
            .expect("création");
        let changed = png_with_pixels(8, 8, &[(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)]);
        renderer.set(changed.clone());
        let mut params = visual_params(&path);
        params.update_baseline = Some(true);
        let result = server
            .assert_visual(Parameters(params))
            .await
            .expect("mise à jour");
        assert_eq!(result.is_error, Some(false));
        let payload = body_json(&result);
        assert_eq!(payload["updated"], true);
        assert_eq!(payload["created"], false);
        assert_eq!(std::fs::read(&path).expect("baseline"), changed);
        // La nouvelle référence est comparée comme identique.
        let again = server
            .assert_visual(Parameters(visual_params(&path)))
            .await
            .expect("comparaison");
        assert_eq!(body_json(&again)["ok"], true);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn assert_visual_dimension_mismatch_has_no_diff_image() {
        let path = scratch_path("dims.png");
        let _ = std::fs::remove_file(&path);
        let renderer = SwitchRenderer::new(png_with_pixels(8, 8, &[]));
        let server = server_with(|p| p.renderer = Arc::new(renderer.clone()));
        server
            .assert_visual(Parameters(visual_params(&path)))
            .await
            .expect("création");
        renderer.set(png_with_pixels(10, 8, &[]));
        let result = server
            .assert_visual(Parameters(visual_params(&path)))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(result.content.len(), 1);
        assert_eq!(body_json(&result)["same_dims"], false);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn assert_visual_empty_mask_is_invalid_params_and_never_captures() {
        let path = scratch_path("badmask.png");
        let mut params = visual_params(&path);
        params.ignore_regions = Some(vec![Region {
            x: 0,
            y: 0,
            width: 0,
            height: 4,
        }]);
        let err = server_with(|_| {})
            .assert_visual(Parameters(params))
            .await
            .expect_err("masque vide");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(!path.exists());
    }

    // ------------------------------------------------------------------
    // Outil composite observe (ADR-0029).
    // ------------------------------------------------------------------

    /// Inspecteur avec un DOM et un state fixés.
    struct SceneInspector {
        dom: serde_json::Value,
        state: BTreeMap<String, serde_json::Value>,
    }
    #[async_trait]
    impl AppInspector for SceneInspector {
        async fn dom_snapshot(&self) -> PortResult<DomTree> {
            Ok(DomTree::new(self.dom.clone()))
        }
        async fn state_snapshot(&self) -> PortResult<StateDump> {
            Ok(StateDump::new(self.state.clone()))
        }
    }

    /// Source console qui rend ses entrées une seule fois (drain).
    struct DrainingConsole(Mutex<Vec<ConsoleError>>);
    #[async_trait]
    impl ConsoleErrorSource for DrainingConsole {
        async fn drain_console_errors(&self) -> PortResult<Vec<ConsoleError>> {
            Ok(std::mem::take(&mut *self.0.lock().unwrap()))
        }
    }

    struct UnsupportedRenderer;
    #[async_trait]
    impl Renderer for UnsupportedRenderer {
        async fn screenshot(&self) -> PortResult<Screenshot> {
            Err(PortError::Unsupported("pas de capture ici".to_string()))
        }
        async fn viewport_size(&self) -> PortResult<Size> {
            Err(PortError::Unsupported("pas de capture ici".to_string()))
        }
    }

    struct BrokenLogs;
    #[async_trait]
    impl LogSource for BrokenLogs {
        async fn poll(&self) -> PortResult<Vec<LogEvent>> {
            Err(PortError::Network("SDK coupé".to_string()))
        }
    }

    fn scene_server() -> DeveyeMcpServer {
        let mut state = BTreeMap::new();
        state.insert("page".to_string(), json!("Agenda"));
        state.insert("blob".to_string(), json!("x".repeat(500)));
        server_with(|p| {
            p.renderer = Arc::new(SwitchRenderer::new(png_of(2_000, 1_000, [5, 5, 5, 255])));
            p.inspector = Arc::new(SceneInspector {
                dom: json!({
                    "nodeName": "#document",
                    "children": [{ "nodeName": "BODY", "children": [
                        { "nodeName": "BUTTON", "attributes": ["id", "ok"] }
                    ]}]
                }),
                state,
            });
            p.console_errors = Arc::new(DrainingConsole(Mutex::new(vec![
                ConsoleError::new(deveye_core::ConsoleLevel::Info, "bruit"),
                ConsoleError::new(deveye_core::ConsoleLevel::Error, "TypeError: x"),
                ConsoleError::new(deveye_core::ConsoleLevel::Warning, "déprécié"),
            ])));
            p.log_source = Arc::new(ScriptedLogs(Mutex::new(
                vec![vec![
                    log(LogLevel::Info, "démarré"),
                    log(LogLevel::Error, "échec"),
                ]]
                .into(),
            )));
        })
    }

    fn observe_json(result: &CallToolResult) -> serde_json::Value {
        let last = result.content.last().expect("bloc texte");
        serde_json::from_str(&last.as_text().expect("texte").text).expect("JSON")
    }

    #[tokio::test]
    async fn observe_composes_every_section_in_one_call() {
        let result = scene_server()
            .observe(Parameters(ObserveParams::default()))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(false));
        assert!(result.content[0].as_image().is_some(), "image d'abord");
        let body = observe_json(&result);
        let s = &body["sections"];
        // Capture réduite à 1024 px par défaut (source 2000 px).
        assert_eq!(s["screenshot"]["image"]["width"], 1024);
        assert_eq!(s["screenshot"]["image"]["source_width"], 2000);
        assert!((s["screenshot"]["image"]["scale"].as_f64().unwrap() - 0.512).abs() < 1e-9);
        // Arbre résumé.
        assert_eq!(s["dom"]["total_nodes"], 3);
        assert!(s["dom"]["outline"].as_str().unwrap().contains("button#ok"));
        // État : valeur longue aperçue, pas dumpée.
        assert_eq!(s["state"]["signals"], 2);
        let blob = s["state"]["entries"]["blob"].as_str().unwrap();
        assert!(blob.chars().count() <= 121, "aperçu borné");
        // Console : error + warning seulement, le reste compté.
        assert_eq!(s["console"]["total"], 2);
        assert_eq!(s["console"]["ignored_lower_levels"], 1);
        // Logs.
        assert_eq!(s["logs"]["total"], 2);
        assert_eq!(body["skipped"], json!([]));
    }

    #[tokio::test]
    async fn observe_degrades_each_unavailable_section_independently() {
        let server = server_with(|p| {
            p.renderer = Arc::new(UnsupportedRenderer);
            p.inspector = Arc::new(UnsupportedInspectorMock("pas de WebView".to_string()));
            p.log_source = Arc::new(BrokenLogs);
        });
        let result = server
            .observe(Parameters(ObserveParams::default()))
            .await
            .expect("pas d'erreur protocolaire");
        assert_eq!(result.is_error, Some(false), "jamais d'échec global");
        assert_eq!(result.content.len(), 1, "pas d'image sans capture");
        let s = observe_json(&result)["sections"].clone();
        for name in ["screenshot", "dom", "state"] {
            assert_eq!(s[name]["available"], false, "{name}");
            assert_eq!(s[name]["reason"], "unsupported", "{name}");
            assert!(
                s[name]["detail"]
                    .as_str()
                    .unwrap()
                    .starts_with("indisponible pour cette cible"),
                "{name}: {}",
                s[name]
            );
        }
        assert_eq!(s["logs"]["available"], false);
        assert_eq!(s["logs"]["reason"], "unreachable");
        // La console, saine, répond malgré tout.
        assert_eq!(s["console"]["available"], true);
    }

    #[tokio::test]
    async fn observe_include_and_exclude_select_sections() {
        let server = scene_server();
        let only_logs = server
            .observe(Parameters(ObserveParams {
                include: Some(vec!["logs".to_string()]),
                ..ObserveParams::default()
            }))
            .await
            .expect("include");
        assert_eq!(only_logs.content.len(), 1, "pas de capture demandée");
        let body = observe_json(&only_logs);
        assert_eq!(body["sections"].as_object().unwrap().len(), 1);
        assert_eq!(
            body["skipped"],
            json!(["screenshot", "dom", "state", "console"])
        );

        let no_shot = scene_server()
            .observe(Parameters(ObserveParams {
                exclude: Some(vec!["Screenshot".to_string(), "dom".to_string()]),
                ..ObserveParams::default()
            }))
            .await
            .expect("exclude");
        assert!(no_shot.content[0].as_text().is_some());
        let body = observe_json(&no_shot);
        assert!(body["sections"].get("screenshot").is_none());
        assert_eq!(body["skipped"], json!(["screenshot", "dom"]));
    }

    #[tokio::test]
    async fn observe_invalid_params_never_drain_anything() {
        let server = scene_server();
        for bad in [
            ObserveParams {
                include: Some(vec!["reseau".to_string()]),
                ..ObserveParams::default()
            },
            ObserveParams {
                include: Some(vec!["logs".to_string()]),
                exclude: Some(vec!["logs".to_string()]),
                ..ObserveParams::default()
            },
        ] {
            let err = server.observe(Parameters(bad)).await.expect_err("invalide");
            assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        }
        // Les buffers n'ont pas été touchés par les appels refusés.
        let result = server
            .observe(Parameters(ObserveParams::default()))
            .await
            .expect("appel valide");
        assert_eq!(observe_json(&result)["sections"]["logs"]["total"], 2);
        // Et un second appel les trouve drainés.
        let again = server
            .observe(Parameters(ObserveParams::default()))
            .await
            .expect("second appel");
        assert_eq!(observe_json(&again)["sections"]["logs"]["total"], 0);
        assert_eq!(observe_json(&again)["sections"]["console"]["total"], 0);
    }

    #[tokio::test]
    async fn observe_bounds_events_to_the_most_recent() {
        let server = scene_server();
        let result = server
            .observe(Parameters(ObserveParams {
                include: Some(vec!["logs".to_string()]),
                max_events: Some(1),
                ..ObserveParams::default()
            }))
            .await
            .expect("observe");
        let logs = &observe_json(&result)["sections"]["logs"];
        assert_eq!(logs["total"], 2);
        assert_eq!(logs["truncated"], true);
        assert_eq!(logs["events"].as_array().unwrap().len(), 1);
        assert_eq!(logs["events"][0]["message"], "échec");
    }

    #[tokio::test]
    async fn observe_explicit_max_width_overrides_the_default() {
        let result = scene_server()
            .observe(Parameters(ObserveParams {
                include: Some(vec!["screenshot".to_string()]),
                capture: capture_options(Some(200), Some("jpeg"), None),
                ..ObserveParams::default()
            }))
            .await
            .expect("observe");
        let image = &observe_json(&result)["sections"]["screenshot"]["image"];
        assert_eq!(image["width"], 200);
        assert_eq!(image["format"], "jpeg");
        assert_eq!(
            result.content[0].as_image().unwrap().mime_type,
            "image/jpeg"
        );
    }
}
