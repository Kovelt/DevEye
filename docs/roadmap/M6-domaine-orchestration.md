# M6 — Domaine enrichi et orchestration de scénarios

## Objectif

Ancrer dans `deveye-core` et `deveye-ports` les primitives qui rendent DevEye capable de chasser des bugs de façon reproductible : entité `Session`, port `WaitCondition` (attentes conditionnelles avec prédicats typés), port `ConsoleErrorSource` (drain des erreurs console CDP) et trois outils MCP corrélés (`wait_condition`, `collect_console_errors`, `assert_selector`).

- **Effort** : L
- **Dépend de** : M0 (wire protocol formalisé) ; M1b (pompe eval Atelier opérationnelle)
- **Statut décision** : tranché — Session est déjà prototypée dans Chronos (à promouvoir) ; WaitCondition et ConsoleErrorSource sont nouveaux

---

## Pourquoi (verrou levé)

Sans session, chaque appel MCP est apatride : Claude ne sait pas depuis combien de temps il observe l'app, ni si la page a rechargé entre deux captures. Sans `WaitCondition`, il doit poller manuellement avec `logs_wait` — fragilité documentée dans `adapter-mcp-server/src/lib.rs:453-493` (boucle de 250 ms sur `log_source.poll()`). Sans `ConsoleErrorSource`, les erreurs JS remontées par le navigateur (`console.error`, exceptions non capturées) sont invisibles à Claude alors qu'elles sont la première source d'information sur un bug de rendu Dioxus.

Le port `WaitCondition` lève le verrou principal : un scénario de chasse de bug ressemble à « clique, attends que le DOM contienne le sélecteur `.error-banner`, prends un screenshot » — séquence qui n'est pas exprimable aujourd'hui sans chaîne de promesses manuelles de l'agent. Le port `ConsoleErrorSource` lève le second verrou : sur une app Dioxus les paniques Rust remontent en `console.error` via `wasm-bindgen` (cible web) ou dans les logs CDP (cible desktop) ; les capturer automatiquement corrélées à une session permet de construire des assertions temporelles.

---

## État actuel du code

**`deveye-core`** (DevEye canonical, `~/Projets/DevEye/deveye-core/src/`) :
- `lib.rs:12-19` : publie `command`, `log_event`, `snapshot`. Pas de module `session`.
- `snapshot.rs` : `Screenshot`, `DomTree`, `StateDump`, `Size`, `Position` — dérivés `Clone, Debug, PartialEq, Serialize, Deserialize`.
- `log_event.rs:28-35` : `LogEvent { level, target, message, timestamp, fields }` avec `LogLevel` ordonné (`Trace < Debug < Info < Warn < Error`).
- `command.rs:22-52` : `InteractionCommand` (enum taggé `kebab-case`), `CommandDescriptor`.

**`deveye-ports`** (DevEye canonical, `~/Projets/DevEye/deveye-ports/src/`) :
- `lib.rs:9-23` : six ports publiés — `Renderer`, `AppInspector`, `InteractionSink`, `JsEvaluator`, `LogSource`, `CommandRouter`. Pas de `WaitCondition` ni de `ConsoleErrorSource`.
- `errors.rs:8-32` : `PortError { Render, Inspect, Interaction, Log, Route, Network, Unsupported }` ; alias `PortResult<T>`.
- `log_source.rs:24-28` : `LogSource::poll(&self) -> PortResult<Vec<LogEvent>>` — drainage pur, pas d'attente conditionnelle.

**Prototype Chronos** (`~/Projets/Chronos/crates/deveye/deveye-core/src/session.rs`) :
- `AppSession { id: String, kind: AppKind, started_at: DateTime<Utc> }` avec `AppSession::new(id, kind)` (ligne 38).
- `AppKind` : `DioxusDesktop | DioxusWeb | Tauri | GenericWebview | Unknown` (dérivé `Default = Unknown`, sérialisé `kebab-case`, ligne 22-35).
- Tests ligne 52-67 : vérifie la sérialisation `"dioxus-desktop"`, l'horodatage `started_at` borné par avant/après.
- Ce fichier est **absent de DevEye canonical** — il faut le promouvoir.

**`adapter-mcp-server`** (`~/Projets/DevEye/deveye-adapters/adapter-mcp-server/src/lib.rs`) :
- 17 outils existants (ligne 3-27 du module-level doc). Pas de `wait_condition`, `collect_console_errors`, `assert_selector`.
- `DeveyeMcpServer` (ligne 57-69) : `renderer`, `interaction`, `inspector`, `log_source`, `evaluator`, `command_router` — pas de champ pour les deux nouveaux ports.
- `logs_wait` (ligne 451-493) : boucle de 250 ms, plafond 30 s, `min_level` optionnel — modèle à suivre pour `wait_condition`.

**`adapter-cdp-webview`** (`~/Projets/DevEye/deveye-adapters/adapter-cdp-webview/src/`) :
- `client.rs:307-358` : `handle_event` gère `Target.targetCreated`, `Target.attachedToTarget`, `Target.detachedFromTarget`, `Target.targetDestroyed`. Pas d'abonnement `Log.enable` / `Runtime.consoleAPICalled`.
- `js_evaluator.rs:36-67` : `Runtime.evaluate` avec `returnByValue: true`, `awaitPromise: true`. Modèle pour les prédicats DOM.
- `inspector.rs:23-41` : `dom_snapshot` via `DOM.getDocument { depth: 20 }`. Modèle pour `DomSelector`.
- `error.rs:5-37` : `CdpError { Connect, Send, Receive, Decode, Remote, Unexpected, Base64 }`.

**`deveye-server/src/main.rs`** :
- `Args` (ligne 32-61) : `--cdp-url`, `--sdk-url`, `--screencap-window`, `--target {auto|cdp|sdk}`, `--serve-mcp`. Pas de `--session-id`.
- `DeveyeMcpServer::new` (ligne 171-178) : six arguments. À étendre avec les deux nouveaux ports.

---

## Changements détaillés

### `deveye-core/src/session.rs` (fichier à créer)

Promouvoir le contenu de `Chronos/crates/deveye/deveye-core/src/session.rs` vers le dépôt DevEye canonical. Ajouter le champ `ended_at` absent du prototype.

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Identite d'une app branchee a DevEye a un moment donne.
///
/// `session_id` est choisi par l'orchestrateur (typiquement le nom du projet).
/// `ended_at` reste `None` tant que la session est active ; il est pose par le
/// binaire orchestrateur a la cloture (Ctrl-C ou perte de connexion).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub target: AppKind,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// Categorie d'app cible — repertoire des adapters disponibles.
///
/// Valeurs identiques a `AppKind` du prototype Chronos (session.rs:22-35)
/// mais promues ici comme source de verite canonique.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppKind {
    DioxusDesktop,
    DioxusWeb,
    Tauri,
    GenericWebview,
    #[default]
    Unknown,
}

impl Session {
    pub fn new(session_id: impl Into<String>, target: AppKind) -> Self {
        Self {
            session_id: session_id.into(),
            target,
            started_at: Utc::now(),
            ended_at: None,
        }
    }

    pub fn close(&mut self) {
        self.ended_at = Some(Utc::now());
    }

    pub fn is_active(&self) -> bool {
        self.ended_at.is_none()
    }
}
```

Tests à ajouter dans le même fichier (bloc `#[cfg(test)]`) :
- `new_sets_session_id_and_kind` : vérifie `session_id`, `target`, `ended_at == None`.
- `close_sets_ended_at` : `close()` puis `ended_at.is_some()`.
- `is_active_reflects_ended_at`.
- `kind_serializes_kebab_case` : `"dioxus-desktop"`, `"generic-webview"` (repris du prototype Chronos ligne 62-67 mais adapté au nouveau nom du champ).

### `deveye-core/src/wait_condition.rs` (fichier à créer)

Prédicats purs — zéro I/O. `WaitPredicate` est une enum sérialisée qui circule dans le wire protocol et dans les appels MCP ; elle doit rester dans le domaine.

```rust
use serde::{Deserialize, Serialize};

/// Predicat evalue periodiquement par le port [`WaitCondition`].
///
/// Chaque variante est expressible par l'agent Claude en JSON ; le port
/// se charge de la boucle et du timeout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WaitPredicate {
    /// L'element matchant le selecteur CSS est present dans le DOM.
    DomSelector { selector: String },
    /// La cle `key` dans le state SDK a pour valeur `expected` (comparaison JSON).
    StateKey {
        key: String,
        expected: serde_json::Value,
    },
    /// Deux screenshots consecutifs sont identiques a `threshold_ms` pres
    /// (le deuxieme est pris apres le delai ; si les octets PNG sont egaux,
    /// le predicat est satisfait — stabilite visuelle).
    ScreenshotStable { threshold_ms: u64 },
}

/// Resultat d'une attente conditionnelle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaitOutcome {
    /// Vrai si le predicat a ete satisfait avant le timeout.
    pub satisfied: bool,
    /// Duree effective d'attente en millisecondes.
    pub elapsed_ms: u64,
    /// Description textuelle pour l'agent (ce qui a ete observe si non satisfait).
    pub detail: String,
}
```

Pas de dépendance sur `chrono` ni `thiserror` ici — uniquement `serde` et `serde_json`, déjà dans `[workspace.dependencies]`.

### `deveye-core/src/console_error.rs` (fichier à créer)

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Severite d'une entree console navigateur.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleLevel {
    Log,
    Info,
    Warning,
    Error,
}

/// Erreur (ou log console) emise par le webview de l'app cible.
///
/// `url` et `line` sont optionnels : CDP les fournit pour les messages issus
/// de fichiers JS references, mais pas pour les exceptions synchrones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsoleError {
    pub level: ConsoleLevel,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub timestamp: DateTime<Utc>,
}
```

### `deveye-core/src/lib.rs`

Ajouter les trois nouveaux modules au registre public :

```rust
// ajouts apres la ligne 15 (pub mod log_event;)
pub mod console_error;
pub mod session;
pub mod wait_condition;

// ajouts dans les pub use
pub use console_error::{ConsoleError, ConsoleLevel};
pub use session::{AppKind, Session};
pub use wait_condition::{WaitOutcome, WaitPredicate};
```

Ligne de référence : `lib.rs:13-19` (pub mod existants et pub use).

### `deveye-ports/src/wait_condition.rs` (fichier à créer)

```rust
use std::time::Duration;

use async_trait::async_trait;

use deveye_core::{WaitOutcome, WaitPredicate};

use crate::errors::PortResult;

/// Attente conditionnelle sur l'etat de l'app cible.
///
/// L'implementation boucle jusqu'a ce que `predicate` soit satisfait ou que
/// `timeout` soit ecoule, puis renvoie un [`WaitOutcome`].
/// Jamais une erreur pour un simple timeout — `WaitOutcome::satisfied = false`
/// suffit a distinguer les deux cas.
#[async_trait]
pub trait WaitCondition: Send + Sync {
    async fn wait_until(
        &self,
        predicate: WaitPredicate,
        timeout: Duration,
    ) -> PortResult<WaitOutcome>;
}
```

### `deveye-ports/src/console_error_source.rs` (fichier à créer)

```rust
use async_trait::async_trait;

use deveye_core::ConsoleError;

use crate::errors::PortResult;

/// Drain des erreurs console emises par le webview de l'app cible.
///
/// Appele `drain_console_errors` par analogie avec `LogSource::poll` :
/// chaque appel extrait et vide le buffer accumule depuis le dernier drain.
/// `Vec` vide = aucune nouvelle erreur.
///
/// L'implementation CDP active `Log.enable` et s'abonne a
/// `Runtime.consoleAPICalled` a la connexion ; l'implementation SDK
/// transmet les erreurs via un endpoint dedie.
#[async_trait]
pub trait ConsoleErrorSource: Send + Sync {
    async fn drain_console_errors(&self) -> PortResult<Vec<ConsoleError>>;
}
```

### `deveye-ports/src/errors.rs`

Ajouter une variante dédiée aux erreurs de console (les distinguer de `Log` qui est pour le tracing Rust) :

```rust
// apres la ligne 28 (#[error("réseau : {0}")])
#[error("console webview : {0}")]
Console(String),
```

### `deveye-ports/src/lib.rs`

```rust
// ajouts apres la ligne 13 (pub mod log_source;)
pub mod console_error_source;
pub mod wait_condition;

// ajouts dans les pub use
pub use console_error_source::ConsoleErrorSource;
pub use wait_condition::WaitCondition;
```

### `deveye-adapters/adapter-cdp-webview/src/console_error_source.rs` (fichier à créer)

L'implémentation CDP : active `Log.enable` à la connexion et bufférise les événements `Runtime.consoleAPICalled` dans un `Mutex<VecDeque<ConsoleError>>` partagé avec `CdpClient`.

Le buffer est à ajouter dans `CdpInner` (`client.rs:38-48`) :

```rust
// dans CdpInner, apres le champ `closed`
console_errors: Mutex<VecDeque<ConsoleError>>,
```

Initialisation dans `CdpClient::connect` après `spawn_reader` :

```rust
// appel Log.enable pour activer les messages Log.entryAdded
client
    .send_browser::<Value, _>("Log.enable", json!({}))
    .await
    .ok(); // non fatal si le navigateur ne supporte pas
```

Dans `handle_event` (`client.rs:307`), ajouter les deux branches :

```rust
"Runtime.consoleAPICalled" => {
    // { type: "error"|"warning"|"log"|"info", args: [{value: "..."}], ... }
    let level_str = params
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("log");
    let text = params
        .get("args")
        .and_then(|a| a.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let level = console_level_from_str(level_str);
    let entry = ConsoleError {
        level,
        text,
        url: params.get("url").and_then(|v| v.as_str()).map(str::to_owned),
        line: params.get("lineNumber").and_then(|v| v.as_u64()).map(|n| n as u32),
        timestamp: Utc::now(),
    };
    inner.console_errors.lock().await.push_back(entry);
}
"Log.entryAdded" => {
    // Fallback pour les navigateurs qui ne supportent que Log.enable.
    let entry_val = params.get("entry").cloned().unwrap_or(Value::Null);
    let level_str = entry_val
        .get("level")
        .and_then(|v| v.as_str())
        .unwrap_or("log");
    if matches!(level_str, "error" | "warning") {
        let text = entry_val
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let entry = ConsoleError {
            level: console_level_from_str(level_str),
            text,
            url: entry_val.get("url").and_then(|v| v.as_str()).map(str::to_owned),
            line: entry_val.get("lineNumber").and_then(|v| v.as_u64()).map(|n| n as u32),
            timestamp: Utc::now(),
        };
        inner.console_errors.lock().await.push_back(entry);
    }
}
```

Fonction auxiliaire :

```rust
fn console_level_from_str(s: &str) -> ConsoleLevel {
    match s {
        "error" => ConsoleLevel::Error,
        "warning" | "warn" => ConsoleLevel::Warning,
        "info" => ConsoleLevel::Info,
        _ => ConsoleLevel::Log,
    }
}
```

Implémentation du port :

```rust
#[async_trait]
impl ConsoleErrorSource for CdpClient {
    async fn drain_console_errors(&self) -> PortResult<Vec<ConsoleError>> {
        let mut guard = self.inner.console_errors.lock().await;
        Ok(guard.drain(..).collect())
    }
}
```

### `deveye-adapters/adapter-cdp-webview/src/wait_condition.rs` (fichier à créer)

Implémentation du port `WaitCondition` sur `CdpClient`. La boucle utilise `JsEvaluator::evaluate` (déjà implémenté sur `CdpClient` dans `js_evaluator.rs:36-67`) et `AppInspector::state_snapshot` (via le SDK, composé par l'orchestrateur). Pour rester dans le crate sans couplage croisé, les prédicats `DomSelector` et `ScreenshotStable` sont implémentés directement via `CdpClient::call` ; `StateKey` délègue vers `GET /devtools/state` via le même mécanisme que `DioxusSignalsInspector`.

```rust
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;

use deveye_core::{WaitOutcome, WaitPredicate};
use deveye_ports::{PortResult, WaitCondition};

use crate::client::CdpClient;

const POLL_INTERVAL: Duration = Duration::from_millis(200);

#[async_trait]
impl WaitCondition for CdpClient {
    async fn wait_until(
        &self,
        predicate: WaitPredicate,
        timeout: Duration,
    ) -> PortResult<WaitOutcome> {
        let deadline = Instant::now() + timeout;
        let started = Utc::now();

        loop {
            let satisfied = check_predicate(self, &predicate).await?;
            if satisfied {
                let elapsed_ms = started.elapsed_millis_approx();
                return Ok(WaitOutcome {
                    satisfied: true,
                    elapsed_ms,
                    detail: "predicat satisfait".to_string(),
                });
            }
            if Instant::now() >= deadline {
                return Ok(WaitOutcome {
                    satisfied: false,
                    elapsed_ms: timeout.as_millis() as u64,
                    detail: format!("timeout apres {}ms", timeout.as_millis()),
                });
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

async fn check_predicate(client: &CdpClient, predicate: &WaitPredicate) -> PortResult<bool> {
    match predicate {
        WaitPredicate::DomSelector { selector } => {
            let expr = format!(
                "!!document.querySelector({})",
                serde_json::to_string(selector).unwrap_or_default()
            );
            let val = client.evaluate(&expr).await?;
            Ok(val.as_bool().unwrap_or(false))
        }
        WaitPredicate::StateKey { key, expected } => {
            // Lit le state via la pompe eval (document.__deveye_state__ si le
            // SDK l'expose en window, sinon POST /devtools/state — ici on
            // evaluerait un fetch, ce qui est couteux ; a affiner a M6b si
            // besoin. Pour le MVP : eval d'une variable JS posee par le SDK.
            let expr = format!(
                "(function(){{ const s = window.__deveye_state__ && window.__deveye_state__[{}]; \
                 return JSON.stringify(s) === JSON.stringify({}); }})()",
                serde_json::to_string(key).unwrap_or_default(),
                serde_json::to_string(expected).unwrap_or_default()
            );
            let val = client.evaluate(&expr).await?;
            Ok(val.as_bool().unwrap_or(false))
        }
        WaitPredicate::ScreenshotStable { threshold_ms } => {
            use deveye_ports::Renderer;
            let first = client.screenshot().await?;
            tokio::time::sleep(Duration::from_millis(*threshold_ms)).await;
            let second = client.screenshot().await?;
            Ok(first.bytes == second.bytes)
        }
    }
}
```

Note : `elapsed_millis_approx()` n'existe pas sur `chrono::DateTime` — utiliser `Instant::now() - start_instant` converti en `u64`. Adapter en conséquence à l'implémentation réelle.

### `deveye-adapters/adapter-cdp-webview/src/lib.rs`

Ajouter les deux modules et les ré-exports :

```rust
// apres la ligne "mod js_evaluator;"
mod console_error_source;
mod wait_condition;
```

Pas de `pub use` supplémentaire : les implémentations sont des `impl Trait for CdpClient`, visibles par les consommateurs via les traits.

### `deveye-adapters/adapter-mcp-server/src/lib.rs`

Trois nouveaux outils et deux champs dans `DeveyeMcpServer`.

**Nouveaux champs** (après `command_router`, ligne 65 de `lib.rs`) :

```rust
console_errors: Arc<dyn ConsoleErrorSource>,
wait_condition: Arc<dyn WaitCondition>,
```

**Signature `new` mise à jour** :

```rust
pub fn new(
    renderer: Arc<dyn Renderer>,
    interaction: Arc<dyn InteractionSink>,
    inspector: Arc<dyn AppInspector>,
    log_source: Arc<dyn LogSource>,
    evaluator: Arc<dyn JsEvaluator>,
    command_router: Arc<dyn CommandRouter>,
    console_errors: Arc<dyn ConsoleErrorSource>,
    wait_condition: Arc<dyn WaitCondition>,
) -> Self
```

**Nouveaux params** :

```rust
#[derive(Debug, Deserialize, JsonSchema)]
struct WaitConditionParams {
    /// Predicat a attendre : "dom-selector", "state-key" ou "screenshot-stable".
    /// Passe en JSON inline : { "kind": "dom-selector", "selector": ".btn" }
    predicate: serde_json::Value,
    /// Timeout en millisecondes (defaut 5000, plafond 60000).
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AssertSelectorParams {
    /// Selecteur CSS a verifier.
    selector: String,
    /// Texte attendu dans `innerText` du premier element matchant (optionnel).
    #[serde(default)]
    expected_text: Option<String>,
    /// Nombre d'elements attendus (optionnel ; si absent, verifie juste la presence).
    #[serde(default)]
    expected_count: Option<usize>,
}
```

**Trois nouveaux outils** dans le bloc `#[tool_router] impl DeveyeMcpServer` :

```rust
#[tool(
    description = "Attend que le predicat soit satisfait ou que le timeout expire. \
                   Renvoie { satisfied, elapsed_ms, detail }. \
                   Predicats : dom-selector (presence d'un element CSS), \
                   state-key (valeur dans le state SDK), \
                   screenshot-stable (pas de changement visuel pendant threshold_ms)."
)]
async fn wait_condition(
    &self,
    Parameters(p): Parameters<WaitConditionParams>,
) -> Result<CallToolResult, McpError> {
    const DEFAULT_MS: u64 = 5_000;
    const MAX_MS: u64 = 60_000;
    let predicate: WaitPredicate =
        serde_json::from_value(p.predicate).map_err(|e| {
            McpError::invalid_params(format!("predicat invalide : {e}"), None)
        })?;
    let timeout = Duration::from_millis(
        p.timeout_ms.unwrap_or(DEFAULT_MS).min(MAX_MS),
    );
    let outcome = self
        .wait_condition
        .wait_until(predicate, timeout)
        .await
        .map_err(to_mcp)?;
    let json =
        serde_json::to_string(&outcome).unwrap_or_else(|_| "{}".to_string());
    if outcome.satisfied {
        Ok(CallToolResult::success(vec![Content::text(json)]))
    } else {
        // Non-satisfait = resultat d'outil en erreur (isError) :
        // l'agent voit immediatement que l'attente a echoue.
        Ok(CallToolResult::error(vec![Content::text(json)]))
    }
}

#[tool(
    description = "Draine et renvoie les erreurs console emises par le webview \
                   (console.error, exceptions non capturees, Log.entryAdded). \
                   Chaque entree : { level, text, url?, line?, timestamp }."
)]
async fn collect_console_errors(&self) -> Result<CallToolResult, McpError> {
    let errors = self
        .console_errors
        .drain_console_errors()
        .await
        .map_err(to_mcp)?;
    let json = serde_json::to_string(&errors).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

#[tool(
    description = "Verifie la presence (et optionnellement le texte et le compte) \
                   d'un selecteur CSS dans le DOM. \
                   Renvoie { ok, actual_text?, actual_count, selector }. \
                   isError = true si ok = false (assertion echouee)."
)]
async fn assert_selector(
    &self,
    Parameters(p): Parameters<AssertSelectorParams>,
) -> Result<CallToolResult, McpError> {
    // Snippet unique : compte les elements et lit innerText du premier.
    let selector_json =
        serde_json::to_string(&p.selector).unwrap_or_default();
    let expr = format!(
        "(function(){{ \
         const nodes = document.querySelectorAll({selector_json}); \
         const count = nodes.length; \
         const text = count > 0 ? (nodes[0].innerText || nodes[0].textContent || '').trim() : null; \
         return {{ count, text }}; \
         }})()"
    );
    let val = self.evaluator.evaluate(&expr).await.map_err(to_mcp)?;
    let actual_count = val
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let actual_text = val
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    let mut ok = actual_count > 0;
    if ok {
        if let Some(ref expected) = p.expected_text {
            ok = actual_text.as_deref() == Some(expected.as_str());
        }
        if let Some(expected_count) = p.expected_count {
            ok = actual_count == expected_count;
        }
    }

    let payload = serde_json::json!({
        "ok": ok,
        "selector": p.selector,
        "actual_count": actual_count,
        "actual_text": actual_text,
    });
    let json = serde_json::to_string(&payload)
        .unwrap_or_else(|_| "{\"ok\":false}".to_string());
    if ok {
        Ok(CallToolResult::success(vec![Content::text(json)]))
    } else {
        Ok(CallToolResult::error(vec![Content::text(json)]))
    }
}
```

Mettre à jour `get_info` (ligne 524-537) pour lister les trois nouveaux outils dans `instructions`.

### `deveye-server/src/main.rs`

Ajouter `--session-id` à `Args` et câbler les deux nouveaux ports :

```rust
// dans Args
/// Identifiant de la session MCP (affiché dans les outils et les logs).
/// Si absent, utilise le nom du projet derive de --sdk-url.
#[arg(long)]
session_id: Option<String>,
```

Au moment de la construction du `DeveyeMcpServer`, passer les nouveaux ports. `CdpClient` implémentant désormais `ConsoleErrorSource` et `WaitCondition`, les variantes `Cdp` et `Auto` peuvent l'utiliser directement. Pour le mode `Sdk` (pont natif sans CDP), créer des stubs `NoopConsoleErrorSource` et `SdkWaitCondition` dans `deveye-server/src/main.rs` (structs locales, pas de crate supplémentaire) :

```rust
/// ConsoleErrorSource vide pour le mode sdk (pas de CDP pour intercepter les logs console).
struct NoopConsoleErrorSource;
#[async_trait]
impl ConsoleErrorSource for NoopConsoleErrorSource {
    async fn drain_console_errors(&self) -> PortResult<Vec<ConsoleError>> {
        Ok(Vec::new())
    }
}
```

`SdkWaitCondition` boucle en appelant `JsEvaluator::evaluate` du pont natif pour `DomSelector` et `StateKey`, et `Renderer::screenshot` (xcap) pour `ScreenshotStable`.

---

## Nouveaux artefacts

**Entités `deveye-core`** :
- `deveye_core::Session` — `session_id: String`, `target: AppKind`, `started_at: DateTime<Utc>`, `ended_at: Option<DateTime<Utc>>` ; méthodes `new`, `close`, `is_active`.
- `deveye_core::AppKind` — enum `DioxusDesktop | DioxusWeb | Tauri | GenericWebview | Unknown`.
- `deveye_core::WaitPredicate` — enum taggé `kebab-case` : `DomSelector { selector }`, `StateKey { key, expected }`, `ScreenshotStable { threshold_ms }`.
- `deveye_core::WaitOutcome` — `{ satisfied: bool, elapsed_ms: u64, detail: String }`.
- `deveye_core::ConsoleError` — `{ level: ConsoleLevel, text, url?, line?, timestamp }`.
- `deveye_core::ConsoleLevel` — `Log | Info | Warning | Error`.

**Ports `deveye-ports`** :
- `deveye_ports::WaitCondition` — trait `async fn wait_until(predicate: WaitPredicate, timeout: Duration) -> PortResult<WaitOutcome>`.
- `deveye_ports::ConsoleErrorSource` — trait `async fn drain_console_errors() -> PortResult<Vec<ConsoleError>>`.
- `PortError::Console(String)` — nouvelle variante.

**Implémentations `adapter-cdp-webview`** :
- `impl WaitCondition for CdpClient` — boucle 200 ms, prédicats `DomSelector` via `Runtime.evaluate`, `StateKey` via eval JS, `ScreenshotStable` via double screenshot PNG.
- `impl ConsoleErrorSource for CdpClient` — buffer `console_errors: Mutex<VecDeque<ConsoleError>>` dans `CdpInner` ; abonnement `Runtime.consoleAPICalled` + `Log.entryAdded` dans `handle_event`.

**Outils MCP** dans `adapter-mcp-server` :
- `wait_condition(predicate, timeout_ms?)` → `{ satisfied, elapsed_ms, detail }` ; `isError` si non satisfait.
- `collect_console_errors()` → `Vec<ConsoleError>`.
- `assert_selector(selector, expected_text?, expected_count?)` → `{ ok, selector, actual_count, actual_text? }` ; `isError` si `ok = false`.

---

## Critère de done

```bash
# Domaine et ports
cargo check -p deveye-core -p deveye-ports
cargo test  -p deveye-core -p deveye-ports
cargo clippy -p deveye-core -p deveye-ports -- -D warnings

# Adapter CDP
cargo test  -p adapter-cdp-webview
cargo clippy -p adapter-cdp-webview -- -D warnings

# Adapter MCP
cargo test  -p adapter-mcp-server
cargo clippy -p adapter-mcp-server -- -D warnings

# Workspace complet
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt   --all -- --check
```

Comportement observable en session MCP réelle (app Dioxus desktop lancée avec `--remote-debugging-port=9222`) :

1. `wait_condition { "predicate": { "kind": "dom-selector", "selector": ".page-home" }, "timeout_ms": 3000 }` → répond `{ satisfied: true, elapsed_ms: <300, detail: "predicat satisfait" }` en moins de 3 s si l'élément est présent.
2. `wait_condition { "predicate": { "kind": "dom-selector", "selector": ".inexistant" }, "timeout_ms": 500 }` → répond avec `isError = true` et `satisfied: false` après 500 ms.
3. `collect_console_errors` après avoir provoqué une `console.error("test")` via `eval_js` → renvoie au moins un `ConsoleError` de niveau `error`.
4. `assert_selector { "selector": "h1", "expected_text": "Atelier" }` → `{ ok: true, actual_count: 1, actual_text: "Atelier" }` sur la page d'accueil ; `isError = true` si le texte diffère.
5. `wait_condition { "predicate": { "kind": "screenshot-stable", "threshold_ms": 300 } }` → `satisfied: true` quand l'UI est au repos, `satisfied: false` pendant une animation.

---

## Risques et pièges

**1. Pureté de `deveye-core`**
`WaitPredicate` et `ConsoleError` n'introduisent que `serde`, `serde_json` et `chrono` — tous dans `[workspace.dependencies]` (`Cargo.toml` workspace). Vérifier qu'aucun `use tokio` ni `use reqwest` ne s'y glisse (`cargo check -p deveye-core` doit passer sans `tokio` dans les dépendances effectives du crate).

**2. Règle hexagonale — pas de couplage croisé entre adapters**
`adapter-cdp-webview` ne doit pas importer `adapter-dioxus-signals`. Pour `StateKey`, le prédicat CDP utilise `window.__deveye_state__` exposé par le SDK via `eval_js` — ce pont JS est suffisant. Si l'app ne pose pas la variable (app sans SDK), le prédicat retourne `false` proprement.

**3. `CdpInner` : taille du buffer console**
Ajouter `console_errors: tokio::sync::Mutex<std::collections::VecDeque<ConsoleError>>` dans `CdpInner` (`client.rs:38-48`). Borner la capacité à 512 entrées (politique drop-oldest) comme `LOG_BUFFER_CAPACITY` dans le SDK (`sdk/src/lib.rs:52`) pour éviter la croissance illimitée pendant des sessions longues.

**4. `Log.enable` vs `Runtime.consoleAPICalled` sur WebKitGTK**
Sur Linux (Atelier, Chronos — WebKitGTK), le port CDP expose `Runtime.consoleAPICalled` mais sa fiabilité dépend de la version de WebKitGTK. Vérifier avec `eval_js("console.error('ping')")` suivi de `collect_console_errors` pendant les tests d'intégration. Si `Runtime.consoleAPICalled` est absent, `Log.entryAdded` (enabled par `Log.enable`) prend le relais — les deux branches sont implémentées.

**5. `ScreenshotStable` et encodage PNG**
Deux PNG identiques visuellement peuvent avoir des octets différents si le codec insère un timestamp ou une entropie dans les métadonnées. Vérifier que `Page.captureScreenshot` CDP renvoie des PNG déterministes (en pratique oui pour Chromium-based ; à valider sur WebKitGTK). Alternative de repli : comparer seulement les `N` premiers octets après l'en-tête PNG (IHDR).

**6. `AppKind` : promotion depuis Chronos sans couplage**
`AppKind` est promu depuis `Chronos/crates/deveye/deveye-core/src/session.rs:22-35`. Le dépôt Chronos conserve sa propre copie jusqu'à ce qu'il bascule sur DevEye canonical comme dépendance externe. Pas de `path = "../../../DevEye/deveye-core"` dans `Chronos/Cargo.toml` avant que M0 ne stabilise le wire protocol et que la décision d'externalisation soit prise.

**7. Outils MCP : `isError` vs erreur de protocole**
Suivre le précédent établi par `eval_js` (`adapter-mcp-server/src/lib.rs:353`) : un prédicat non satisfait ou une assertion échouée est un `CallToolResult::error(...)` (isError MCP), pas une `Err(McpError)`. Claude distingue les deux : `isError` = l'outil a répondu mais le résultat est un échec sémantique ; `Err` = panne du serveur.

**8. Sécurité dev-only**
`ConsoleErrorSource` ne doit pas être exposé dans une build release. Le trait est dans `deveye-ports` (pas dans le SDK app), donc pas de risque direct d'embarquement. S'assurer que `deveye-server` n'est jamais compilé sans `debug_assertions` dans les pipelines CI — ajouter une assertion de compilation si besoin : `#[cfg(not(debug_assertions))] compile_error!("deveye-server ne doit pas etre compile en release")`.

---

## Checklist d'exécution

- [ ] Créer `deveye-core/src/session.rs` : `Session`, `AppKind` avec tests (`new`, `close`, `is_active`, sérialisation kebab-case).
- [ ] Créer `deveye-core/src/wait_condition.rs` : `WaitPredicate` (enum taggé), `WaitOutcome`. Tests de sérialisation pour les trois variantes.
- [ ] Créer `deveye-core/src/console_error.rs` : `ConsoleLevel`, `ConsoleError`. Tests : sérialisation `lowercase`, `skip_serializing_if` sur `url` et `line`.
- [ ] Mettre à jour `deveye-core/src/lib.rs` : ajouter `pub mod` et `pub use` pour les trois nouveaux modules.
- [ ] `cargo test -p deveye-core` vert.
- [ ] Créer `deveye-ports/src/wait_condition.rs` : trait `WaitCondition`.
- [ ] Créer `deveye-ports/src/console_error_source.rs` : trait `ConsoleErrorSource`.
- [ ] Ajouter `PortError::Console(String)` dans `deveye-ports/src/errors.rs`.
- [ ] Mettre à jour `deveye-ports/src/lib.rs` : `pub mod` et `pub use`.
- [ ] `cargo test -p deveye-ports` vert.
- [ ] Dans `adapter-cdp-webview/src/client.rs` : ajouter `console_errors: Mutex<VecDeque<ConsoleError>>` dans `CdpInner` (capacité 512) ; initialiser dans `CdpInner::default` ; appeler `Log.enable` après `setDiscoverTargets` dans `CdpClient::connect` ; ajouter les branches `Runtime.consoleAPICalled` et `Log.entryAdded` dans `handle_event`.
- [ ] Créer `adapter-cdp-webview/src/console_error_source.rs` : `impl ConsoleErrorSource for CdpClient`.
- [ ] Créer `adapter-cdp-webview/src/wait_condition.rs` : `impl WaitCondition for CdpClient` avec `check_predicate` pour les trois variantes.
- [ ] Mettre à jour `adapter-cdp-webview/src/lib.rs` : ajouter `mod console_error_source` et `mod wait_condition`.
- [ ] `cargo test -p adapter-cdp-webview` vert.
- [ ] Dans `adapter-mcp-server/src/lib.rs` : ajouter les deux champs à `DeveyeMcpServer`, mettre à jour `new`, ajouter `WaitConditionParams`, `AssertSelectorParams`, les trois outils, mettre à jour `get_info`.
- [ ] `cargo test -p adapter-mcp-server` vert.
- [ ] Dans `deveye-server/src/main.rs` : ajouter `--session-id` dans `Args` ; créer `NoopConsoleErrorSource` et `SdkWaitCondition` pour le mode `sdk` ; câbler les deux nouveaux ports dans `DeveyeMcpServer::new`.
- [ ] `cargo build -p deveye-server` vert.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` sans avertissement.
- [ ] `cargo fmt --all -- --check` sans diff.
- [ ] Test manuel : lancer l'app Dioxus cible, `deveye-server --serve-mcp --target cdp`, appeler `wait_condition`, `collect_console_errors`, `assert_selector` et vérifier les comportements décrits dans le critère de done.
- [ ] Mettre à jour `docs/architecture.md` : section « Les 6 ports » → « Les 8 ports » ; documenter `Session`, `WaitPredicate`, `ConsoleError`.
- [ ] Mettre à jour `docs/usage.md` : ajouter les trois nouveaux outils MCP avec exemples d'appel JSON.
