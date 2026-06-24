# M4 — Outils MCP pour scénarios et chasse aux bugs

## Objectif

Enrichir la surface MCP de DevEye avec six nouveaux outils (`wait_for_selector`, `find_element`, `collect_console_errors`, `list_windows`, `drag_drop`, et les filtres sur `logs_wait`) afin de permettre des scénarios d'agent robustes, sans `sleep` arbitraire ni évaluation JS brute pour des besoins récurrents.

**Effort** : M  
**Dépend de** : M0 (wire protocol SDK stable), M1b (pompe eval Atelier opérationnelle, pour les tests en mode SDK)  
**Statut décision** : non encore engagé — roadmap validée, implémentation à démarrer

---

## Pourquoi (verrou levé)

Les 17 outils actuels (`adapter-mcp-server/src/lib.rs`, lignes 5-26) couvrent bien les interactions de base et les lectures d'état, mais laissent trois verrous ouverts lors d'un scénario typique :

**Verrou 1 — absence d'attente sémantique.** Après un `click` ou un `invoke_command`, l'agent ne sait pas quand l'effet est visible dans le DOM. L'unique solution disponible est `logs_wait` (ligne 451-493), qui attend un log tracing. Si l'action produit un rendu silencieux (sans log), l'agent doit recourir à `eval_js` avec un snippet `querySelector` + `getBoundingClientRect` rédigé manuellement à chaque fois. Cela rend les scénarios fragiles et verbeux.

**Verrou 2 — aucune introspection structurée d'un élément.** `dom` (ligne 428-432) renvoie l'arbre complet, ce qui est coûteux et oblige l'agent à en extraire manuellement les propriétés pertinentes (visibilité, état désactivé, rectangle, attributs). Il n'existe pas d'outil qui réponde en une frappe à la question « cet élément est-il présent et cliquable ? ».

**Verrou 3 — les erreurs JS/console sont invisibles.** Le canal `logs_tail`/`logs_wait` est alimenté par le buffer tracing du SDK (`adapter-logs-tracing`, via `deveye-sdk`). Les erreurs remontées par le moteur JS lui-même (`window.onerror`, `unhandledrejection`, `console.error`) n'y apparaissent pas. En mode CDP, les événements `Runtime.exceptionThrown` et `Log.entryAdded` existent mais ne sont pas écoutés. Un crash JS de la page cible reste donc invisible à l'agent.

**Verrou 4 — drag-drop absent.** `InteractionCommand` (deveye-core/src/command.rs, lignes 24-51) ne modélise pas le drag-drop. Les tests de réordonnancement ou de colonnes Kanban sont impossibles sans une séquence manuelle `mousePressed`/`mouseMoved`/`mouseReleased` via `eval_js` brut.

**Verrou 5 — `logs_wait` filtre uniquement par niveau.** La structure `LogsWaitParams` (lib.rs, lignes 152-162) n'expose que `timeout_ms` et `min_level`. Impossible d'attendre un message précis (ex. « formulaire sauvegardé ») ou un log émis par un module particulier (`target`).

---

## État actuel du code

### adapter-mcp-server/src/lib.rs

17 outils enregistrés via `#[tool_router]` (ligne 164). Le modèle à suivre pour ajouter un outil est : déclarer une struct `*Params` dérivant `Deserialize + JsonSchema` (lignes 71-162), puis ajouter une méthode `async fn nom_outil(&self, Parameters(p): Parameters<NomParams>) -> Result<CallToolResult, McpError>` décorée de `#[tool(description = "...")]` dans le bloc `impl DeveyeMcpServer` (lignes 165-519).

Les outils qui composent `JsEvaluator` sans passer par `InteractionSink` (ex. `click_selector`, lignes 390-406 ; `click_text`, lignes 408-425 ; `eval_js`, lignes 337-355) montrent le patron type : construire un snippet JS, appeler `self.evaluator.evaluate(&snippet).await`, interpréter le `Value` retourné.

La fonction auxiliaire `click_resolved` (lignes 362-388) illustre comment combiner évaluateur et sink en un outil composite.

`logs_wait` (lignes 451-493) montre le patron de polling avec deadline : boucle `loop` sur `self.log_source.poll()`, sleep de `POLL_INTERVAL_MS = 250 ms`, sortie sur match ou timeout.

### deveye-core/src/command.rs

`InteractionCommand` (lignes 22-51) est une enum taguée `#[serde(tag = "kind", rename_all = "kebab-case")]`. Les variantes existantes : `Click`, `Hover`, `Type`, `KeyPress`, `Navigate`, `Scroll`, `Resize`. Pas de variante `DragDrop`.

`MouseButton` (lignes 6-13) : `Left`, `Right`, `Middle`.

### deveye-ports/src/js_evaluator.rs

Trait `JsEvaluator` (ligne 22) : une seule méthode `async fn evaluate(&self, expression: &str) -> PortResult<Value>`. Assez pour tous les nouveaux outils qui n'ont besoin que d'interroger ou manipuler le DOM via JS.

### deveye-ports/src/interaction.rs

Trait `InteractionSink` (ligne 20) : `async fn dispatch(&self, command: InteractionCommand) -> PortResult<()>`. `drag_drop` nécessite une nouvelle variante de `InteractionCommand` et son implémentation CDP dans `adapter-cdp-webview/src/interaction.rs`.

### adapter-cdp-webview/src/interaction.rs

Implémente `InteractionSink for CdpClient` (lignes 10-103). Le patron de dispatch bas niveau est la fonction `mouse_event` (lignes 106-123) qui appelle `client.call("Input.dispatchMouseEvent", payload).await`. Chaque variante de `InteractionCommand` est un bras du `match`. `DragDrop` sera un nouveau bras.

### adapter-cdp-webview/src/js_evaluator.rs

Implémente `JsEvaluator for CdpClient` (lignes 35-67) via `Runtime.evaluate` avec `returnByValue: true, awaitPromise: true, userGesture: true`. Les exceptions JS sont remontées comme `PortError` à partir de `exceptionDetails` (lignes 50-63).

### deveye-core/src/log_event.rs

`LogEvent` (lignes 27-35) : champs `level: LogLevel`, `target: String`, `message: String`, `timestamp: DateTime<Utc>`, `fields: serde_json::Value`. Le filtre `filter_message_contains` et `filter_target` s'appliquent sur `message` et `target`.

---

## Changements détaillés

### 1. deveye-core/src/command.rs

Ajouter la variante `DragDrop` à `InteractionCommand` et son constructeur.

```rust
// Apres la variante Resize (ligne 51), dans l'enum InteractionCommand :

/// Glisser-déposer de `from` vers `to` en maintenant le bouton gauche pressé.
/// Génère la séquence : mouseMoved(from) -> mousePressed(from) ->
/// [mouseMoved intermédiaires] -> mouseReleased(to).
DragDrop {
    from: Position,
    to: Position,
},
```

Constructeur à ajouter dans le bloc `impl InteractionCommand` après `fn resize` (ligne 88) :

```rust
pub fn drag_drop(from: Position, to: Position) -> Self {
    Self::DragDrop { from, to }
}
```

Test à ajouter dans `mod tests` :

```rust
#[test]
fn drag_drop_serializes_with_kind_tag() {
    let cmd = InteractionCommand::drag_drop(Position::new(10, 20), Position::new(100, 200));
    let v = serde_json::to_value(&cmd).unwrap();
    assert_eq!(
        v,
        json!({
            "kind": "drag-drop",
            "from": { "x": 10, "y": 20 },
            "to":   { "x": 100, "y": 200 }
        })
    );
}
```

### 2. adapter-cdp-webview/src/interaction.rs

Ajouter le bras `DragDrop` dans le `match command` de `dispatch` (après `InteractionCommand::Resize`, ligne 87). L'implémentation envoie 5 événements CDP pour simuler un drag fidèle :

```rust
InteractionCommand::DragDrop { from, to } => {
    // 1. Deplacer vers la source sans press (hover pre-drag)
    mouse_event(self, "mouseMoved", from, "left").await?;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    // 2. Presser le bouton
    mouse_event(self, "mousePressed", from, "left").await?;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    // 3. Glissement en deux etapes intermediaires
    let mid = Position::new((from.x + to.x) / 2, (from.y + to.y) / 2);
    mouse_event(self, "mouseMoved", mid, "left").await?;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    mouse_event(self, "mouseMoved", to, "left").await?;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    // 4. Relacher
    mouse_event(self, "mouseReleased", to, "left").await?;
}
```

Note sur `mouse_event` : la fonction existante (lignes 106-123) passe déjà `button: &str` comme paramètre dans le payload, ce qui suffit — pas de modification de la signature.

Pour `collect_console_errors` en mode CDP, ajouter une méthode publique sur `CdpClient` dans `adapter-cdp-webview/src/client.rs` ou dans un nouveau fichier `adapter-cdp-webview/src/console_errors.rs` :

```rust
// Dans adapter-cdp-webview — méthode d'activation du listener d'exceptions JS.
// Appelée une seule fois au démarrage de la session (après attach).
// Active Runtime.enable + Log.enable pour recevoir les événements
// Runtime.exceptionThrown et Log.entryAdded dans le reader background.
pub async fn enable_console_capture(&self) -> Result<(), CdpError> {
    let _: Value = self.call("Runtime.enable", json!({})).await?;
    let _: Value = self.call("Log.enable",     json!({})).await?;
    Ok(())
}

// Buffer partagé des erreurs JS collectées par le reader background.
// Dans CdpInner, ajouter :
console_errors: Mutex<Vec<ConsoleError>>,

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsoleError {
    pub source: String,   // "js-exception" | "console-error" | "network"
    pub message: String,
    pub url: Option<String>,
    pub line: Option<u32>,
    pub timestamp: DateTime<Utc>,
}

// Dans handle_event (ligne 307), ajouter deux bras :
"Runtime.exceptionThrown" => { /* deserialise params.exceptionDetails, pousse dans console_errors */ }
"Log.entryAdded"          => { /* filtre entry.level == "error", pousse dans console_errors */ }

// Méthode de drain (analogie avec LogSource::poll) :
pub async fn drain_console_errors(&self) -> Vec<ConsoleError> {
    std::mem::take(&mut *self.inner.console_errors.lock().await)
}
```

### 3. adapter-mcp-server/src/lib.rs

**Structs de paramètres à ajouter** (dans la section des structs, après `LogsWaitParams`, ligne 162) :

```rust
#[derive(Debug, Deserialize, JsonSchema)]
struct WaitForSelectorParams {
    /// Sélecteur CSS à attendre (ex. `#save-btn:not([disabled])`).
    selector: String,
    /// Timeout en millisecondes (défaut 5000, plafond 30000).
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Si true, attend également que l'élément soit visible
    /// (getBoundingClientRect().width > 0 && height > 0).
    #[serde(default)]
    visible: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FindElementParams {
    /// Sélecteur CSS de l'élément à inspecter.
    selector: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DragDropParams {
    /// Abscisse de départ en pixels logiques.
    from_x: i32,
    /// Ordonnée de départ en pixels logiques.
    from_y: i32,
    /// Abscisse d'arrivée en pixels logiques.
    to_x: i32,
    /// Ordonnée d'arrivée en pixels logiques.
    to_y: i32,
}

// LogsWaitParams enrichi (remplace l'existant, lignes 152-162) :
#[derive(Debug, Deserialize, JsonSchema)]
struct LogsWaitParams {
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    min_level: Option<String>,
    /// Si fourni, ne débloquer que si au moins un log provient d'un target
    /// dont le champ `target` contient cette sous-chaîne (insensible à la casse).
    #[serde(default)]
    filter_target: Option<String>,
    /// Si fourni, ne débloquer que si au moins un log dont le champ `message`
    /// contient cette sous-chaîne (insensible à la casse).
    #[serde(default)]
    filter_message_contains: Option<String>,
}
```

**Outils à ajouter dans le bloc `#[tool_router] impl DeveyeMcpServer`** :

#### wait_for_selector

```rust
#[tool(
    description = "Attend qu'un sélecteur CSS soit présent dans le DOM (et optionnellement visible). \
                   Renvoie { found: bool, elapsed_ms: u64 }. Sans sleep arbitraire côté agent."
)]
async fn wait_for_selector(
    &self,
    Parameters(p): Parameters<WaitForSelectorParams>,
) -> Result<CallToolResult, McpError> {
    const DEFAULT_TIMEOUT_MS: u64 = 5_000;
    const MAX_TIMEOUT_MS: u64 = 30_000;
    const POLL_MS: u64 = 100;

    let check_visible = p.visible.unwrap_or(false);
    let budget = Duration::from_millis(
        p.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).min(MAX_TIMEOUT_MS),
    );
    let deadline = Instant::now() + budget;
    let selector_json = serde_json::to_string(&p.selector).unwrap_or_default();

    let snippet = if check_visible {
        format!(
            "(function(){{ \
               const el = document.querySelector({s}); \
               if (!el) return false; \
               const r = el.getBoundingClientRect(); \
               return r.width > 0 && r.height > 0; \
             }})()",
            s = selector_json
        )
    } else {
        format!("!!document.querySelector({s})", s = selector_json)
    };

    let start = Instant::now();
    loop {
        match self.evaluator.evaluate(&snippet).await {
            Ok(v) if v.as_bool().unwrap_or(false) => {
                let elapsed = start.elapsed().as_millis() as u64;
                let payload = serde_json::json!({ "found": true, "elapsed_ms": elapsed });
                return Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string(&payload).unwrap_or_default(),
                )]));
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            let elapsed = start.elapsed().as_millis() as u64;
            let payload = serde_json::json!({ "found": false, "elapsed_ms": elapsed });
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string(&payload).unwrap_or_default(),
            )]));
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
}
```

**Mode CDP vs SDK** : utilise `JsEvaluator::evaluate`, donc fonctionne en mode CDP et en mode SDK (la pompe eval de M1b expose également ce trait). En mode screencap pur, retourne `PortError::Unsupported` traduit en `McpError::invalid_request` par `to_mcp` (lib.rs, ligne 549).

#### find_element

```rust
#[tool(
    description = "Inspecte le premier élément matchant un sélecteur CSS et renvoie ses propriétés : \
                   found, tag, text, rect {x,y,width,height}, visible, disabled, attributes."
)]
async fn find_element(
    &self,
    Parameters(p): Parameters<FindElementParams>,
) -> Result<CallToolResult, McpError> {
    let selector_json = serde_json::to_string(&p.selector).unwrap_or_default();
    let snippet = format!(
        r#"(function(){{
          const el = document.querySelector({s});
          if (!el) return {{ found: false }};
          const r = el.getBoundingClientRect();
          const attrs = {{}};
          for (const a of el.attributes) {{ attrs[a.name] = a.value; }}
          return {{
            found:    true,
            tag:      el.tagName.toLowerCase(),
            text:     (el.innerText || el.textContent || '').trim().slice(0, 200),
            rect:     {{ x: r.left, y: r.top, width: r.width, height: r.height }},
            visible:  r.width > 0 && r.height > 0,
            disabled: el.disabled === true || el.hasAttribute('disabled'),
            attributes: attrs,
          }};
        }})()"#,
        s = selector_json
    );
    match self.evaluator.evaluate(&snippet).await {
        Ok(value) => {
            let json = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
            Ok(CallToolResult::success(vec![Content::text(json)]))
        }
        Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
    }
}
```

**Mode CDP vs SDK** : même remarque que `wait_for_selector` — via `JsEvaluator`, compatible CDP et SDK.

#### collect_console_errors

```rust
#[tool(
    description = "Draine et renvoie les erreurs JS collectées depuis le dernier appel : \
                   exceptions non capturées (Runtime.exceptionThrown en mode CDP, \
                   window.onerror injecté en mode SDK) et entrées console.error/Log.entryAdded. \
                   Renvoie un tableau JSON de { source, message, url, line, timestamp }."
)]
async fn collect_console_errors(&self) -> Result<CallToolResult, McpError> {
    // En mode CDP : drain du buffer CdpClient::drain_console_errors()
    //   (nécessite enable_console_capture() au démarrage de la session).
    // En mode SDK : eval d'un snippet qui lit window.__deveye_console_errors__
    //   (tableau alimenté par le listener window.onerror injecté une seule fois
    //   au premier appel, puis vidé à chaque drain).
    //
    // L'implémentation concrète s'appuie sur un nouveau port optionnel
    // ConsoleErrorSource ou sur une branche dans l'impl CDP de LogSource.
    // Voir section "Nouveaux artefacts".
    todo!("implementation M4")
}
```

**Mode CDP** : appelle `CdpClient::drain_console_errors()` (méthode à ajouter — voir section précédente). `enable_console_capture()` doit être appelé une fois lors de l'attach de la session, dans `handle_event` au bras `"Target.attachedToTarget"` (client.rs, ligne 319).

**Mode SDK** : évalue le snippet suivant, injecté une seule fois via `eval_js` au premier appel :

```javascript
(function() {
  if (!window.__deveye_init_console__) {
    window.__deveye_console_errors__ = [];
    window.addEventListener('error', function(e) {
      window.__deveye_console_errors__.push({
        source: 'js-exception', message: e.message,
        url: e.filename, line: e.lineno,
        timestamp: new Date().toISOString()
      });
    });
    window.addEventListener('unhandledrejection', function(e) {
      window.__deveye_console_errors__.push({
        source: 'unhandled-rejection',
        message: String(e.reason), url: null, line: null,
        timestamp: new Date().toISOString()
      });
    });
    window.__deveye_init_console__ = true;
  }
  const errs = window.__deveye_console_errors__.splice(0);
  return errs;
})()
```

#### list_windows

```rust
#[tool(
    description = "Liste les fenêtres (targets CDP) ouvertes : id, titre, url, type. \
                   En mode CDP : Target.getTargets. \
                   En mode SDK : renvoie une liste vide documentée (le SDK ne connaît pas les autres fenêtres)."
)]
async fn list_windows(&self) -> Result<CallToolResult, McpError> {
    // En mode CDP : self.cdp_client.send_browser("Target.getTargets", json!({}))
    // puis filtrer kind == "page", renvoyer [{ id, title, url }].
    // En mode SDK / screencap : renvoyer [] avec un champ "note" explicatif.
    todo!("implementation M4")
}
```

**Mode CDP** : appelle `CdpClient::send_browser::<Value, _>("Target.getTargets", json!({}))`. La méthode `send_browser` est privée (`fn`, ligne 182 de client.rs) ; elle doit être rendue `pub(crate)` ou enveloppée dans une méthode publique `pub async fn list_targets(&self) -> Result<Vec<TargetSummary>, CdpError>`.

**Mode SDK** : pas de visibilité inter-fenêtres ; renvoie `[{"note":"list_windows non disponible en mode SDK"}]` plutôt qu'une erreur, pour que l'agent continue son scénario.

#### drag_drop

```rust
#[tool(
    description = "Glisse-dépose depuis (from_x, from_y) vers (to_x, to_y) en mode CDP \
                   (mousedown/mousemove/mouseup natifs). \
                   En mode SDK : simule via PointerEvent JS (compatibilité réduite selon le framework). \
                   Vérifie les coordonnées dans le viewport avant dispatch."
)]
async fn drag_drop(
    &self,
    Parameters(p): Parameters<DragDropParams>,
) -> Result<CallToolResult, McpError> {
    self.ensure_in_viewport(p.from_x, p.from_y).await?;
    self.ensure_in_viewport(p.to_x, p.to_y).await?;
    let cmd = InteractionCommand::drag_drop(
        Position::new(p.from_x, p.from_y),
        Position::new(p.to_x, p.to_y),
    );
    self.interaction.dispatch(cmd).await.map_err(to_mcp)?;
    Ok(CallToolResult::success(vec![Content::text("ok")]))
}
```

**Mode CDP** : via `InteractionCommand::DragDrop` -> bras CDP dans `adapter-cdp-webview/src/interaction.rs` (séquence `mouseMoved` / `mousePressed` / `mouseMoved` intermédiaires / `mouseReleased`).

**Mode SDK (fallback JS)** : si `InteractionSink::dispatch` reçoit `DragDrop` dans un contexte SDK sans CDP, l'implémentation doit envoyer la séquence via `JsEvaluator::evaluate` avec `PointerEvent` :

```javascript
(function(fx, fy, tx, ty) {
  const el = document.elementFromPoint(fx, fy);
  if (!el) return;
  const opts = (x, y) => ({
    bubbles: true, cancelable: true, clientX: x, clientY: y, pointerId: 1, isPrimary: true
  });
  el.dispatchEvent(new PointerEvent('pointerdown', opts(fx, fy)));
  el.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, clientX: fx, clientY: fy }));
  const target = document.elementFromPoint(tx, ty);
  (target || el).dispatchEvent(new PointerEvent('pointermove', opts(tx, ty)));
  (target || el).dispatchEvent(new PointerEvent('pointerup', opts(tx, ty)));
  (target || el).dispatchEvent(new MouseEvent('mouseup', { bubbles: true, clientX: tx, clientY: ty }));
})(from_x, from_y, to_x, to_y)
```

Le support effectif dépend du framework (les widgets Dioxus 0.6/0.7 écoutant les événements `ondragstart`/`ondrop` natifs ne répondront pas à `PointerEvent` — documenté dans les risques).

#### Modification de logs_wait (filtres supplémentaires)

Dans la méthode `logs_wait` existante (lignes 451-493 de lib.rs), modifier le calcul de `matched` pour intégrer les deux nouveaux filtres :

```rust
// Remplacer (ligne 476-479) :
let matched = match min_level {
    Some(level) => batch.iter().any(|e| e.level >= level),
    None => !batch.is_empty(),
};

// Par :
let matched = {
    let candidates: Vec<&LogEvent> = match min_level {
        Some(level) => batch.iter().filter(|e| e.level >= level).collect(),
        None => batch.iter().collect(),
    };
    let candidates = match &p.filter_target {
        Some(t) => {
            let t_lower = t.to_ascii_lowercase();
            candidates.into_iter()
                .filter(|e| e.target.to_ascii_lowercase().contains(&t_lower))
                .collect::<Vec<_>>()
        }
        None => candidates,
    };
    let candidates = match &p.filter_message_contains {
        Some(m) => {
            let m_lower = m.to_ascii_lowercase();
            candidates.into_iter()
                .filter(|e| e.message.to_ascii_lowercase().contains(&m_lower))
                .collect::<Vec<_>>()
        }
        None => candidates,
    };
    !candidates.is_empty()
};
```

Le champ `collected` reste alimenté de tous les événements (sans filtre), pour que l'agent voie le contexte complet ; seule la condition de déblocage est filtrée.

---

## Nouveaux artefacts

### Port optionnel ConsoleErrorSource (si l'on ne veut pas étendre CdpClient directement)

Si Morgan préfère ne pas surcharger `CdpClient` d'une responsabilité supplémentaire, un port léger peut être défini dans `deveye-ports/src/console_error_source.rs` :

```rust
// deveye-ports/src/console_error_source.rs
use async_trait::async_trait;
use deveye_core::ConsoleError;
use crate::errors::PortResult;

#[async_trait]
pub trait ConsoleErrorSource: Send + Sync {
    /// Drain atomique : renvoie les erreurs collectées depuis le dernier appel
    /// et vide le buffer interne.
    async fn drain(&self) -> PortResult<Vec<ConsoleError>>;
}
```

Ce port serait implémenté :
- par `CdpClient` (écoute CDP `Runtime.exceptionThrown` + `Log.entryAdded`)
- par un `SdkConsoleErrorSource` qui délègue à `JsEvaluator` pour exécuter le snippet `window.__deveye_console_errors__.splice(0)` après injection unique du listener

`DeveyeMcpServer` recevrait un `Arc<dyn ConsoleErrorSource>` supplémentaire dans son constructeur `new`.

**Alternative (plus simple, recommandée pour M4)** : ne pas créer de nouveau port. Implémenter `collect_console_errors` directement dans `adapter-mcp-server` en branchant sur le type concret via `Any`, ou en exposant une méthode sur un wrapper newtype. Décision à prendre selon la préférence de Morgan.

### Entité ConsoleError dans deveye-core

```rust
// deveye-core/src/console_error.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsoleError {
    /// "js-exception" | "unhandled-rejection" | "console-error" | "network"
    pub source: String,
    pub message: String,
    pub url: Option<String>,
    pub line: Option<u32>,
    pub timestamp: DateTime<Utc>,
}
```

À exporter depuis `deveye-core/src/lib.rs` et inclure dans `[workspace.dependencies]` si nécessaire.

### Outils MCP créés (noms de référence pour le `.mcp.json`)

| Nom de l'outil | Paramètres MCP | Mode CDP | Mode SDK |
|---|---|---|---|
| `wait_for_selector` | `selector` (str), `timeout_ms` (u64?), `visible` (bool?) | oui | oui |
| `find_element` | `selector` (str) | oui | oui |
| `collect_console_errors` | aucun | oui (CDP events) | oui (window.onerror injecté) |
| `list_windows` | aucun | oui (Target.getTargets) | liste vide documentée |
| `drag_drop` | `from_x`, `from_y`, `to_x`, `to_y` (i32) | oui (CDP natif) | partiel (PointerEvent JS) |
| `logs_wait` (enrichi) | + `filter_target` (str?), `filter_message_contains` (str?) | oui | oui |

---

## Critère de done

```bash
# Compilation sans avertissement
cargo check --workspace

# Lint strict
cargo clippy --workspace --all-targets -- -D warnings

# Tests unitaires (incluant les nouveaux tests de sérialisation DragDrop)
cargo test --workspace

# Vérification manuelle en mode CDP (Atelier ou Chronos en mode debug) :
# 1. wait_for_selector : après un clic qui déclenche un rendu asynchrone,
#    vérifier que l'outil retourne { found: true, elapsed_ms: <200 }
#    avant la prochaine action — sans sleep côté agent.

# 2. find_element : appel sur un sélecteur connu et sur un sélecteur absent.
#    { found: false } si absent, { found: true, disabled: true/false, ... } sinon.

# 3. collect_console_errors : provoquer une exception JS via eval_js
#    ("throw new Error('test M4')"), puis appeler collect_console_errors.
#    Le tableau renvoyé doit contenir au moins une entrée avec source="js-exception".

# 4. list_windows : vérifier que la liste contient au moins le target
#    de l'app cible ouverte (type="page", url contenant "dioxus").

# 5. drag_drop : tester sur un élément dragable de l'app cible.
#    En l'absence d'élément dragable, vérifier au moins que la séquence
#    CDP est émise sans erreur (logs_tail ne remonte pas d'erreur CDP).

# 6. logs_wait avec filtres : émettre un log tracing depuis l'app cible
#    avec target="mon::module" et message="sauvegarde ok", puis vérifier
#    que logs_wait avec filter_target="mon::module" débloque immédiatement
#    et que logs_wait avec filter_message_contains="ABSENT" épuise le timeout.

# Vérification de la description visible par l'agent dans un client MCP :
# les 6 nouveaux outils doivent apparaître dans la liste renvoyée par
# l'initialisation MCP (champ tools de la réponse initialize).
```

---

## Risques et pièges

**1. Hexagonal — ne pas coupler les nouveaux outils à l'implémentation CDP.**  
`wait_for_selector` et `find_element` s'appuient uniquement sur `JsEvaluator` (deveye-ports/src/js_evaluator.rs, ligne 22). `drag_drop` passe par `InteractionSink` (deveye-ports/src/interaction.rs, ligne 20). `collect_console_errors` nécessite un choix architectural (nouveau port vs délégation interne — voir section Nouveaux artefacts). Dans tous les cas, `adapter-mcp-server` ne doit jamais importer `adapter-cdp-webview` directement.

**2. `list_windows` et la visibilité multi-target.**  
`CdpClient::send_browser` est actuellement privée (client.rs, ligne 182). La rendre accessible sans briser l'encapsulation nécessite soit de l'exposer en `pub(crate)` et d'envelopper l'appel dans la méthode publique `list_targets`, soit de définir un nouveau port `WindowLister`. Le choix du port est préférable pour rester dans les règles hexagonales.

**3. Drag-drop Wayland / WebKitGTK.**  
En mode CDP sur Atelier (WebKitGTK 2.46+, Dioxus 0.6), la séquence `Input.dispatchMouseEvent` passe par le moteur de rendu WebKit. Le support du drag-drop natif (`ondragstart`/`ondrop` HTML5) via CDP n'est pas garanti sous WebKitGTK — c'est une limitation connue du protocole CDP sur moteurs non-Chromium. Tester avec `PointerEvent` JS en fallback si les événements CDP ne produisent pas d'effet. Pour les apps Chronos (Dioxus 0.7), même restriction.

**4. `collect_console_errors` — injection unique du listener en mode SDK.**  
Le guard `window.__deveye_init_console__` protège contre la ré-injection du listener à chaque appel. Vérifier que le snippet est bien évalué *après* le chargement du DOM (le timing dépend de la pompe eval de M1b). Si l'injection a lieu avant `DOMContentLoaded`, les événements émis pendant le chargement peuvent être manqués.

**5. `LogsWaitParams` enrichi — compatibilité des clients existants.**  
Les deux nouveaux champs `filter_target` et `filter_message_contains` sont `#[serde(default)]` : les clients existants qui n'envoient pas ces champs continuent de fonctionner avec le comportement actuel (aucun filtre). Pas de rupture de compatibilité.

**6. Sécurité dev-only — pas d'exposition hors debug.**  
Les outils `collect_console_errors` et `wait_for_selector` injectent du JS dans la page cible. Ils ne doivent jamais être accessibles en build release. Le SDK reste sous `cfg(debug_assertions)` (règle 1 de la section sécurité du CLAUDE.md). Le serveur MCP lui-même bind sur `127.0.0.1` uniquement et doit conserver la garde `Host` anti DNS-rebinding introduite à M0.

**7. Taille du champ `attributes` dans `find_element`.**  
Sur des éléments avec de nombreux attributs (ex. composants Dioxus générés), le dictionnaire `attributes` peut devenir volumineux. Limiter à 50 attributs dans le snippet JS si nécessaire, ou tronquer les valeurs longues.

**8. Filtre `target_matches` dans client.rs (ligne 388-395).**  
`list_windows` via `Target.getTargets` retourne tous les targets, y compris les `service_worker` et les pages système. Filtrer côté Rust avant de sérialiser la réponse, en réutilisant la logique de `target_matches` ou en exposant une variante configurable.

---

## Checklist d'exécution

- [ ] **Lire** `deveye-core/src/command.rs` pour valider l'absence de variante `DragDrop` existante avant de l'ajouter.
- [ ] **Ajouter** la variante `InteractionCommand::DragDrop { from, to }` et le constructeur `drag_drop` dans `deveye-core/src/command.rs`.
- [ ] **Ajouter** le test `drag_drop_serializes_with_kind_tag` dans `deveye-core/src/command.rs`.
- [ ] **Ajouter** `ConsoleError` dans `deveye-core/src/console_error.rs` et l'exporter depuis `deveye-core/src/lib.rs`.
- [ ] **Trancher** avec Morgan : nouveau port `ConsoleErrorSource` ou délégation interne ? (impact sur le constructeur de `DeveyeMcpServer`)
- [ ] **Modifier** `CdpClient` dans `adapter-cdp-webview/src/client.rs` : ajouter `console_errors: Mutex<Vec<ConsoleError>>` dans `CdpInner`, implémenter `enable_console_capture()`, `drain_console_errors()`, et les deux bras `Runtime.exceptionThrown` / `Log.entryAdded` dans `handle_event`.
- [ ] **Appeler** `enable_console_capture()` dans le bras `"Target.attachedToTarget"` de `handle_event` (client.rs, ligne 319).
- [ ] **Rendre** `send_browser` accessible (pub ou méthode wrapper `list_targets`) dans `adapter-cdp-webview/src/client.rs`.
- [ ] **Ajouter** le bras `InteractionCommand::DragDrop` dans `adapter-cdp-webview/src/interaction.rs`.
- [ ] **Ajouter** les structs `WaitForSelectorParams`, `FindElementParams`, `DragDropParams` dans `adapter-mcp-server/src/lib.rs`.
- [ ] **Enrichir** `LogsWaitParams` avec `filter_target` et `filter_message_contains` dans `adapter-mcp-server/src/lib.rs`.
- [ ] **Adapter** la logique de `matched` dans `logs_wait` pour appliquer les nouveaux filtres.
- [ ] **Implémenter** `wait_for_selector` dans le bloc `#[tool_router] impl DeveyeMcpServer`.
- [ ] **Implémenter** `find_element` dans le bloc `#[tool_router] impl DeveyeMcpServer`.
- [ ] **Implémenter** `collect_console_errors` dans le bloc `#[tool_router] impl DeveyeMcpServer`.
- [ ] **Implémenter** `list_windows` dans le bloc `#[tool_router] impl DeveyeMcpServer`.
- [ ] **Implémenter** `drag_drop` dans le bloc `#[tool_router] impl DeveyeMcpServer`.
- [ ] **Mettre à jour** la description dans `get_info()` (lib.rs, ligne 528) pour lister les 6 nouveaux outils.
- [ ] **Mettre à jour** le commentaire de module en tête de `adapter-mcp-server/src/lib.rs` (lignes 1-27) pour refléter les 23 outils.
- [ ] **Vérifier** `cargo check --workspace` sans erreur.
- [ ] **Vérifier** `cargo clippy --workspace --all-targets -- -D warnings` sans avertissement.
- [ ] **Vérifier** `cargo test --workspace` : tous les tests passent, y compris `drag_drop_serializes_with_kind_tag`.
- [ ] **Tester** manuellement `wait_for_selector`, `find_element`, `collect_console_errors` sur Atelier en mode CDP.
- [ ] **Tester** `drag_drop` sur un élément interactif et vérifier le comportement en mode SDK fallback.
- [ ] **Mettre à jour** `docs/architecture.md` pour refléter les 23 outils et le nouveau type `ConsoleError`.
