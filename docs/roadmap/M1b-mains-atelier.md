# M1b — Débloquer les MAINS sur Atelier (brancher la pompe eval)

## Objectif

Transformer DevEye de spectateur à pilote sur Atelier : brancher la pompe `take_eval_requests()` sous la garde double `cfg(all(feature="deveye", debug_assertions))`, ce qui active la voie eval/JS côté SDK et expose en MCP l'outil `right_click` manquant (le bouton droit existe dans le domaine core mais n'a pas d'outil dédié côté serveur MCP).

**Effort** : M (une demi-journée de code, un ADR, une ligne de test).
**Dépend de** : M0 (wire protocol SDK réconcilié, pompe `take_eval_requests()` existante dans le SDK).
**Statut décision** : Accepté — décision 1 de Morgan : brancher sous `cfg(all(feature="deveye", debug_assertions))`.

## Pourquoi (verrou levé)

Atelier est aujourd'hui en mode "yeux seulement". Le commentaire explicite dans le main de l'app cible (`crates/app-dioxus/src/main.rs`, lignes 54-55) le confirme :

```
// YEUX SEULEMENT : on ne branche PAS la pompe d'eval (`take_eval_requests`) → aucun
// `document::eval`/JS dans l'UI, donc règle invariante n°5 (ADR-0002) préservée.
```

La règle invariante n°5 du `CLAUDE.md` d'Atelier (ligne 25) dispose : **"UI : zéro API web-only. Pas d'`eval` JS, pas d'accès DOM navigateur, pas de dépendance JS — condition pour garder ouverte la migration vers Blitz (cf. ADR-0002)."**

Cette règle est valide en production et en usage normal. Elle ne concerne pas DevEye, qui est un outil de développement exclu de tout build release par `cfg(debug_assertions)`. Brancher la pompe sous la garde `cfg(all(feature="deveye", debug_assertions))` n'embarque aucun `eval` en release et ne rompt pas l'invariant : la migration vers Blitz reste ouverte.

Côté MCP, `MouseButton::Right` existe depuis l'origine dans `deveye-core/src/command.rs` (lignes 11-13) et est supporté par l'adapter CDP (`adapter-cdp-webview/src/interaction.rs`, ligne 129 : `MouseButton::Right => "right"`), mais aucun outil MCP dédié ne l'expose à l'agent — seul `click` (gauche par défaut via `InteractionCommand::click`) est disponible. Un agent qui veut ouvrir un menu contextuel doit en ce moment construire le payload JSON à la main via `eval_js`, ce qui est fragile.

## État actuel du code

**Côté Atelier** (`crates/app-dioxus/src/main.rs`, lignes 56-60) :

```rust
#[cfg(feature = "deveye")]
{
    deveye_sdk::init(deveye_sdk::Config::default());
    deveye_sdk::install_tracing_layer();
}
```

`take_eval_requests()` n'est pas appelée. Le bloc `#[cfg(feature = "deveye")]` protège déjà l'init, mais ne double pas la garde `debug_assertions` — c'est une imprécision à corriger (une feature peut être activée sur un build release accidentellement).

**Côté SDK** (`deveye/deveye-sdk/src/lib.rs`, lignes 191-198) :

```rust
pub fn take_eval_requests() -> tokio::sync::mpsc::UnboundedReceiver<EvalRequest> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let registry = REGISTRY.clone();
    runtime().spawn(async move {
        *registry.eval_tx.write().await = Some(tx);
    });
    rx
}
```

La fonction est prête. Elle crée le canal, enregistre l'émetteur dans le `REGISTRY` (champ `eval_tx`, ligne 103), et retourne le récepteur à l'app. Le handler `eval_handler` (`POST /devtools/eval`, lignes 215-274) lit ce canal ; si `eval_tx` est `None`, il retourne `503 SERVICE_UNAVAILABLE` avec `"aucune pompe d'évaluation enregistrée côté app"` (ligne 221) — c'est exactement le comportement observé quand on appelle `eval_js` via MCP sur Atelier en l'état.

**Côté MCP** (`adapter-mcp-server/src/lib.rs`) : dix-sept outils sont exposés (lignes 7-27). Parmi les outils d'interaction : `click` (gauche, ligne 236), `hover` (ligne 261), `scroll` (ligne 276), `resize` (ligne 295), `press_key` (ligne 309), `navigate` (ligne 323), `click_selector` (ligne 390), `click_text` (ligne 408). Aucun `right_click`.

**Côté domaine** (`deveye-core/src/command.rs`) : `MouseButton::Right` existe (ligne 11). `InteractionCommand::Click` accepte un champ `button: MouseButton` (lignes 27-30). La variante `Right` est couverte par `mouse_button_label` dans `interaction.rs` (ligne 129).

## Changements détaillés

### `Atelier/crates/app-dioxus/src/main.rs`

**Modification du bloc DevEye.** Renforcement de la garde (`debug_assertions` en plus de `feature`), et ajout de la pompe eval suivie d'une boucle de drain dans un thread dédié.

Le bloc actuel (lignes 56-60) :

```rust
#[cfg(feature = "deveye")]
{
    deveye_sdk::init(deveye_sdk::Config::default());
    deveye_sdk::install_tracing_layer();
}
```

Devient :

```rust
#[cfg(all(feature = "deveye", debug_assertions))]
{
    deveye_sdk::init(deveye_sdk::Config::default());
    deveye_sdk::install_tracing_layer();

    // MAINS : branche la pompe d'evaluation JS (ADR-00XX).
    // Permet a DevEye d'executer des expressions JS dans le webview Dioxus
    // via `POST /devtools/eval` -> MCP `eval_js`.
    // Guard double : feature AND debug_assertions -> zero embarquement en release.
    // Invariant n°5 (ADR-0002) preserve en production.
    let mut eval_rx = deveye_sdk::take_eval_requests();
    std::thread::Builder::new()
        .name("deveye-eval-pump".into())
        .spawn(move || {
            // Le webview Dioxus expose `document::eval` qui n'est accessible
            // que depuis le thread principal ou via les hooks Dioxus.
            // On tourne sur un thread OS dedié car `take_eval_requests`
            // retourne un receiver async ; on cree un runtime minimal.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime deveye-eval-pump");
            rt.block_on(async move {
                while let Some(req) = eval_rx.recv().await {
                    // Enveloppe de securite : try/catch JS pour eviter les
                    // exceptions non rattrapees qui bloqueraient le oneshot.
                    let wrapped = format!(
                        "(async () => {{ try {{ return ({expr}); }} \
                         catch (e) {{ return {{ __deveye_error: e.message }}; }} }})()",
                        expr = req.expression
                    );
                    // `dioxus::desktop::use_window()` n'est pas disponible hors
                    // composant. On passe par le canal WebView expose par Dioxus
                    // via `dioxus::desktop::DesktopContext`.
                    // NOTE : l'integration exacte depend de l'API Dioxus 0.6
                    // (cf. section Risques). Placeholder a adapter au contexte
                    // de la fenetre active.
                    let result: Result<serde_json::Value, String> =
                        Err("pompe non connectee au webview — voir ADR-00XX".into());
                    let _ = req.respond.send(result);
                }
            });
        })
        .expect("thread deveye-eval-pump");
}
```

**Remarque importante** : l'intégration précise avec le webview Dioxus 0.6 nécessite l'accès à un `DesktopContext` ou équivalent. Le placeholder ci-dessus montre la structure de la boucle ; la section Risques détaille l'API à utiliser.

### `Atelier/docs/adr/00XX-deveye-pompe-eval.md` (nouveau fichier)

Un ADR au format MADR doit être créé dans Atelier avant le code. Son contenu minimal :

```markdown
# ADR-00XX — DevEye : branchement de la pompe eval en mode debug

Statut : Accepté

## Contexte

DevEye est un outil dev-only (build debug uniquement). Atelier integre le SDK DevEye
sous `cfg(feature = "deveye")` pour exposer son state et ses logs a un agent Claude
via MCP. La pompe d'evaluation JS (`take_eval_requests`) n'etait pas branchee pour
preserver l'invariant n°5 (ADR-0002 : zero eval JS dans l'UI).

## Decision

Brancher `take_eval_requests()` sous la garde `cfg(all(feature="deveye", debug_assertions))`.

Justification : `cfg(debug_assertions)` garantit l'absence totale d'embarquement en
release. L'invariant n°5 ne concerne pas les outils de developpement exclu du binaire
de production — la migration vers Blitz reste ouverte.

## Consequences

- En build debug avec feature `deveye` : l'agent peut evaluer du JS dans le webview.
- En build release (tout profil) : aucun code DevEye embarque, invariant n°5 intact.
- L'outil MCP `eval_js` retourne 200 au lieu de 503 sur Atelier.
```

### `DevEye/deveye-adapters/adapter-mcp-server/src/lib.rs`

**Ajout de l'outil `right_click`.** Modélisé sur `click` (lignes 236-247). Ajouter après le bloc `click` existant (après la ligne 247) :

1. Nouveau struct de paramètres (à ajouter dans la section des structs, après `ClickParams`) :

```rust
#[derive(Debug, Deserialize, JsonSchema)]
struct RightClickParams {
    /// Abscisse en pixels logiques depuis le coin haut-gauche du viewport.
    x: i32,
    /// Ordonnée en pixels logiques depuis le coin haut-gauche du viewport.
    y: i32,
}
```

2. Nouvelle méthode d'outil dans le bloc `#[tool_router] impl DeveyeMcpServer`, après la méthode `click` (après la ligne 247) :

```rust
#[tool(
    description = "Effectue un clic droit (contextmenu) sur le viewport de l'app cible aux coordonnées absolues fournies. Declenche mousedown/mouseup avec button=right, ce qui ouvre les menus contextuels des UI qui les ecoutent."
)]
async fn right_click(
    &self,
    Parameters(p): Parameters<RightClickParams>,
) -> Result<CallToolResult, McpError> {
    self.ensure_in_viewport(p.x, p.y).await?;
    let cmd = InteractionCommand::Click {
        position: Position::new(p.x, p.y),
        button: deveye_core::MouseButton::Right,
    };
    self.interaction.dispatch(cmd).await.map_err(to_mcp)?;
    Ok(CallToolResult::success(vec![Content::text("ok")]))
}
```

3. Mettre à jour la description des outils dans `get_info` (ligne 533) en ajoutant `right_click` à la liste.

4. Mettre à jour le commentaire d'en-tête du module (ligne 7 : "Dix-sept outils" devient "Dix-huit outils") et ajouter la ligne de description de `right_click` dans la liste de la doc.

### `DevEye/deveye-adapters/adapter-mcp-server/src/lib.rs` — tests

Ajouter dans le module `tests` un test de non-régression sur `right_click` :

```rust
#[test]
fn right_click_uses_mouse_button_right() {
    use deveye_core::{InteractionCommand, MouseButton, Position};
    let cmd = InteractionCommand::Click {
        position: Position::new(100, 200),
        button: MouseButton::Right,
    };
    // Verifie que la serialisation est coherente avec ce qu'attend interaction.rs.
    let v = serde_json::to_value(&cmd).unwrap();
    assert_eq!(v["button"], "right");
}
```

## Nouveaux artefacts

**Outil MCP `right_click`** — expose `MouseButton::Right` déjà présent dans le domaine.

- Nom MCP : `right_click`
- Paramètres : `{ x: i32, y: i32 }`
- Retour : `"ok"` (texte) en succès
- Chemin de la commande domaine : `InteractionCommand::Click { position, button: MouseButton::Right }`
- Chemin CDP : `Input.dispatchMouseEvent` avec `button: "right"` via `mouse_button_label` dans `adapter-cdp-webview/src/interaction.rs` (ligne 129)

**ADR Atelier** (fichier à créer) : `Atelier/docs/adr/00XX-deveye-pompe-eval.md`

Aucun nouveau crate, aucun nouveau port, aucun nouveau trait.

## Critère de done

```bash
# Compilation sans warning ni erreur dans les deux dépôts
cargo check --workspace                                # dans DevEye/
cargo clippy --workspace --all-targets -- -D warnings  # dans DevEye/

cargo check -p atelier-app-dioxus --features deveye   # dans Atelier/
cargo clippy -p atelier-app-dioxus --features deveye -- -D warnings

# Tests
cargo test --workspace                                 # dans DevEye/

# Comportement observable
# 1. Lancer Atelier en debug avec la feature :
DEVEYE_PORT=31337 cargo run -p atelier-app-dioxus --features deveye

# 2. Vérifier que le SDK répond :
curl -s http://127.0.0.1:31337/devtools/state | jq .

# 3. Vérifier que la pompe est enregistrée (ne retourne plus 503) :
curl -s -X POST http://127.0.0.1:31337/devtools/eval \
  -H 'Content-Type: application/json' \
  -d '{"expression":"1+1"}' | jq .
# Attendu : { "value": 2 } — et non { "error": "aucune pompe d'évaluation..." }

# 4. Via MCP : l'outil right_click est annoncé dans la liste des outils
# (dans Claude Code avec DevEye configuré en .mcp.json)
# Tool list doit contenir "right_click".

# 5. Appel right_click fonctionnel sur Atelier :
# right_click(x=400, y=300) -> "ok", menu contextuel visible en screenshot.
```

## Risques et pièges

**Intégration pompe / webview Dioxus 0.6.**
`take_eval_requests()` retourne un `UnboundedReceiver<EvalRequest>`. Chaque requête doit être exécutée dans le webview via l'API Dioxus desktop. Sous Dioxus 0.6 (crate `dioxus-desktop`), l'accès programmatique au webview hors composant se fait typiquement via `DesktopContext` obtenu dans un `use_effect` ou via le canal interne de wry. L'API exacte est à vérifier dans le code de `dioxus-desktop` 0.6 : la méthode `eval` peut n'être accessible que depuis le thread de la boucle d'événements. Si c'est le cas, la pompe doit poster les requêtes dans le canal d'événements de wry (ou utiliser `DesktopService::send_event`) plutôt que de les exécuter depuis un thread OS séparé. L'ADR doit consigner ce choix.

**Invariant n°5 Atelier (règle non-négociable).**
La garde `cfg(all(feature="deveye", debug_assertions))` est obligatoire — `cfg(feature="deveye")` seul ne suffit pas car une feature peut théoriquement être activée sur un profil release custom. Vérifier que le `Cargo.toml` d'Atelier ne déclare pas `deveye` dans les features par défaut (`default = []` doit exclure `deveye`).

**Règles hexagonales DevEye.**
`right_click` dans `adapter-mcp-server` ne doit pas nommer Atelier ni aucune app cible. Le paramétrage est générique (`x`, `y`). L'outil passe par `InteractionCommand::Click { button: MouseButton::Right }` -> `InteractionSink::dispatch` -> `CdpClient` : le sens des dépendances est respecté (`adapter-mcp-server` -> `deveye-ports` -> `deveye-core`, jamais l'inverse).

**Sécurité dev-only (SDK).**
La pompe n'est active que sous `debug_assertions`. Le SDK bind `127.0.0.1` uniquement (vérifié dans `deveye-sdk/src/lib.rs`, ligne 150 : `SocketAddr::from(([127, 0, 0, 1], config.port))`). Aucune surface d'attaque supplémentaire en release.

**Timeout eval.**
Le handler `eval_handler` impose un timeout de 10 secondes (ligne 244 du SDK : `tokio::time::timeout(Duration::from_secs(10), rx)`). Si le webview est bloqué ou si la pompe ne lit pas assez vite, l'agent recevra `504 GATEWAY_TIMEOUT`. S'assurer que la boucle de la pompe ne bloque pas sur des opérations lentes.

**Alternative rejetée.**
Rester en mode "yeux seulement" avait été envisagé pour éviter tout risque sur l'invariant n°5. Cette alternative est **rejetée par décision explicite de Morgan** : la garde double `cfg(all(feature="deveye", debug_assertions))` apporte les garanties équivalentes sans sacrifice fonctionnel pour l'agent.

**Dioxus 0.7 vs 0.6.**
Atelier est sur Dioxus 0.6. Chronos (M2) sera sur 0.7. L'API `eval` / `DesktopContext` peut différer entre les deux versions. Ne pas mutualiser la pompe avant validation indépendante sur chaque app.

## Checklist d'exécution

- [ ] Lire l'ADR-0002 d'Atelier pour confirmer que la garde `cfg(debug_assertions)` satisfait l'exception.
- [ ] Rédiger et faire valider l'ADR Atelier (`docs/adr/00XX-deveye-pompe-eval.md`) avant de toucher au code.
- [ ] Vérifier dans `Cargo.toml` d'Atelier que la feature `deveye` n'est pas dans `default`.
- [ ] Inspecter l'API `dioxus-desktop` 0.6 pour identifier la méthode d'accès au webview hors composant (évaluer `DesktopContext`, `DesktopService::send_event`, ou channel wry direct) ; consigner le choix dans l'ADR.
- [ ] Modifier la garde dans `Atelier/crates/app-dioxus/src/main.rs` : `#[cfg(feature = "deveye")]` -> `#[cfg(all(feature = "deveye", debug_assertions))]`.
- [ ] Appeler `deveye_sdk::take_eval_requests()` dans ce bloc et connecter le receiver à la boucle de la pompe avec l'API webview identifiée.
- [ ] Tester `curl -X POST /devtools/eval -d '{"expression":"1+1"}'` — attendu : `{ "value": 2 }`.
- [ ] Ajouter le struct `RightClickParams` dans `adapter-mcp-server/src/lib.rs`.
- [ ] Ajouter la méthode `right_click` dans le bloc `#[tool_router] impl DeveyeMcpServer`.
- [ ] Mettre à jour le commentaire d'en-tête du module (18 outils) et la chaîne `instructions` dans `get_info`.
- [ ] Ajouter le test unitaire `right_click_uses_mouse_button_right` dans le module `tests`.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` dans DevEye : zéro warning.
- [ ] `cargo test --workspace` dans DevEye : tous les tests passent.
- [ ] Vérifier en MCP réel que `right_click(x=..., y=...)` est listé et fonctionnel (`screenshot` après = menu visible).
- [ ] Commit Atelier : `feat(app-dioxus): branche la pompe eval deveye sous cfg(all(feature="deveye", debug_assertions))`
- [ ] Commit DevEye : `feat(adapter-mcp): ajoute l'outil right_click (MouseButton::Right)`
