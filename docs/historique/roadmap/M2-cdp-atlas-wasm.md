# M2 — Route CDP pour les têtes WASM d'atlas (zéro modification d'atlas)

## Objectif

Rendre `adapter-cdp-webview` utilisable contre un Chromium externe (Brave, Chrome, Chromium) servant la tête `net10.0-browserwasm` d'une app Uno/WASM tierce, en remplaçant le filtre de target hard-codé `dioxus` par une `TargetFilter` configurable passée depuis la CLI.

**Effort** : S. **Dépend de** : M0 (wire protocol stable). **Statut décision** : tranché. Aucune modification de l'app cible n'est requise ni permise : le SDK Rust ne s'embarque pas dans une cible .NET.

## Pourquoi (verrou levé)

La fonction `target_matches` (`adapter-cdp-webview/src/client.rs:388-395`) accepte uniquement les pages dont l'URL ou le titre contient la chaîne `"dioxus"`, ou dont l'URL est exactement `"about:blank"`. Toute page WASM servie par un Chromium externe (par exemple `http://localhost:5000/`) est rejetée silencieusement : aucune session CDP n'est jamais posée, `wait_attached` expire après 10 secondes avec `"timeout en attente d'une page CDP attachable (10 s)"`, et tous les outils MCP échouent.

```
// adapter-cdp-webview/src/client.rs:388-395 (etat actuel)
fn target_matches(t: &TargetInfo) -> bool {
    if t.kind != "page" {
        return false;
    }
    let title = t.title.to_lowercase();
    let url = t.url.to_lowercase();
    title.contains("dioxus") || url.contains("dioxus") || url == "about:blank"
}
```

Ce filtre est une heuristique spécifique à Dioxus codée en dur, en contradiction directe avec la règle de découplage documentée dans `docs/architecture.md:243` : « les heuristiques spécifiques (ex. filtre de target par titre) doivent rester génériques ou configurables ». Le troubleshooting de `docs/usage.md:174` en prend même acte : « Ajuster `target_matches` dans `client.rs` si l'app cible a un autre title/URL ». M2 supprime la dette en la rendant configurable sans toucher au code.

La tête `net10.0-browserwasm` d'atlas est servie par `dotnet run --project src/Atlas.App -f net10.0-browserwasm` sur `http://localhost:5000` (`atlas/src/Atlas.App/Properties/launchSettings.json:16`). Le titre de la page est `"Atlas"` (`atlas/src/Atlas.App/Atlas.App.csproj:9` : `<ApplicationTitle>Atlas</ApplicationTitle>`). Ni l'URL ni le titre ne contiennent `"dioxus"`.

## État actuel du code

**`adapter-cdp-webview/src/client.rs`**

- `CdpClient::connect` (ligne 106) : bootstrap `GET /json/version`, connexion WS browser-level, activation `Target.setDiscoverTargets`. Le filtre cible est appliqué dans le callback `handle_event` (ligne 307).
- `handle_event` (ligne 307) : sur `Target.targetCreated` (ligne 313) et `Target.attachedToTarget` (ligne 324), appelle `target_matches(&p.target_info)`. Toute target qui ne matche pas est ignorée sans trace.
- `target_matches` (ligne 388-395) : filtre hard-codé `"dioxus"` / `"about:blank"`. Signature actuelle :
  ```rust
  fn target_matches(t: &TargetInfo) -> bool
  ```
  Pas de paramètre de configuration. La fonction n'a pas accès à quoi que ce soit d'extérieur à `TargetInfo`.
- `TargetInfo` (ligne 63-73) : struct dérivant `Debug, Deserialize`, champs `target_id`, `kind`, `title`, `url`. Stable, ne change pas dans ce jalon.
- `CdpInner` (ligne 38-48) : ne contient pas de filtre configurable. `spawn_reader` (ligne 257) prend `Arc<CdpInner>` ; `handle_event` prend `&Arc<CdpInner>`. Le filtre doit y être logé pour être accessible dans la tâche reader.

**`deveye-server/src/main.rs`**

- `struct Args` (ligne 36-61) : quatre champs actuels (`cdp_url`, `sdk_url`, `screencap_window`, `target`). Pas de champ pour un filtre de target CDP.
- `LazyCdp` (ligne 197-232) : encapsule `url: String` et `slot: tokio::sync::Mutex<Option<Arc<CdpClient>>>`. Appelle `CdpClient::connect(&self.url)` (ligne 219) sans transmettre de filtre.
- Valeur par défaut `cdp_url` : `"http://localhost:9222"` (ligne 39).
- La valeur codée en dur `"atelier"` dans `screencap_window` (ligne 89) est un anti-pattern que ce jalon doit éviter de reproduire pour le filtre CDP.

**Tests existants (`client.rs:397-561`)** : sept tests unitaires couvrent `target_matches` sur les cas Dioxus et `about:blank`. Ils devront être mis à jour pour passer le filtre en paramètre.

## Changements détaillés

### `deveye-adapters/adapter-cdp-webview/src/client.rs`

**1. Nouvelle struct `TargetFilter` (à ajouter avant `CdpInner`)**

`TargetFilter` est publique (utilisée par `deveye-server`), immuable, `Clone + Debug`. Elle encode les trois modes utiles :
- sous-chaîne dans l'URL de la page (`url_contains`) ;
- sous-chaîne dans le titre de la page (`title_contains`) ;
- accepter les pages `about:blank` transitoires (`accept_blank`).

La valeur par défaut reproduit le comportement actuel pour les apps Dioxus, sans rien casser.

```rust
/// Filtre de selection de la page CDP cible.
///
/// Toutes les conditions presentes (non-None) sont evaluees en OU sur le meme
/// target. `accept_blank` est un predicat supplementaire independant.
/// La valeur par defaut (`Default::default()`) reproduit le comportement
/// historique Dioxus : URL ou titre contenant "dioxus", ou "about:blank".
#[derive(Clone, Debug)]
pub struct TargetFilter {
    /// Sous-chaine (insensible a la casse) que doit contenir l'URL de la page.
    pub url_contains: Option<String>,
    /// Sous-chaine (insensible a la casse) que doit contenir le titre de la page.
    pub title_contains: Option<String>,
    /// Accepte les pages `about:blank` (transitoire wry au boot).
    pub accept_blank: bool,
}

impl Default for TargetFilter {
    fn default() -> Self {
        Self {
            url_contains: Some("dioxus".to_string()),
            title_contains: Some("dioxus".to_string()),
            accept_blank: true,
        }
    }
}
```

**2. Ajout de `filter: TargetFilter` dans `CdpInner`**

```rust
struct CdpInner {
    next_id: AtomicU64,
    cmd_tx: mpsc::UnboundedSender<Message>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>,
    session: Mutex<Option<SessionInfo>>,
    attached: Notify,
    closed: AtomicBool,
    filter: TargetFilter,   // <-- nouveau champ
}
```

**3. Modification de `CdpClient::connect` pour accepter un filtre**

La signature devient :

```rust
pub async fn connect(http_url: &str, filter: TargetFilter) -> Result<Self, CdpError>
```

Dans le corps, à la création de `CdpInner` :

```rust
let inner = Arc::new(CdpInner {
    next_id: AtomicU64::new(1),
    cmd_tx,
    pending: Mutex::new(HashMap::new()),
    session: Mutex::new(None),
    attached: Notify::new(),
    closed: AtomicBool::new(false),
    filter,   // <-- propagation
});
```

**4. Remplacement de `target_matches` (libre) par une méthode sur `TargetFilter`**

La fonction libre `target_matches` disparaît. Elle est remplacée par une méthode sur `TargetFilter`, ce qui supprime le couplage implicite et rend les tests triviaux :

```rust
impl TargetFilter {
    fn matches(&self, t: &TargetInfo) -> bool {
        if t.kind != "page" {
            return false;
        }
        let title = t.title.to_lowercase();
        let url = t.url.to_lowercase();
        if self.accept_blank && url == "about:blank" {
            return true;
        }
        let url_ok = self
            .url_contains
            .as_deref()
            .map(|s| url.contains(s))
            .unwrap_or(false);
        let title_ok = self
            .title_contains
            .as_deref()
            .map(|s| title.contains(s))
            .unwrap_or(false);
        url_ok || title_ok
    }
}
```

**5. Mise à jour de `handle_event`**

Les deux appels à `target_matches` deviennent des appels à `inner.filter.matches(...)` :

```rust
// dans handle_event, branche "Target.targetCreated"
if !inner.filter.matches(&p.target_info) {
    return;
}

// dans handle_event, branche "Target.attachedToTarget"
if !inner.filter.matches(&p.target_info) {
    return;
}
```

La signature de `handle_event` et `spawn_reader` ne change pas (`&Arc<CdpInner>` contient désormais le filtre).

**6. Mise à jour des tests unitaires**

Les tests du module `tests` qui appelaient `target_matches(&t)` directement sont remplacés par des appels à `TargetFilter::default().matches(&t)` (ou un filtre spécifique au test). La fonction `test_inner` doit passer `filter: TargetFilter::default()` à `CdpInner`. Exemple :

```rust
fn test_inner() -> (Arc<CdpInner>, mpsc::UnboundedReceiver<Message>) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let inner = Arc::new(CdpInner {
        next_id: AtomicU64::new(1),
        cmd_tx,
        pending: Mutex::new(HashMap::new()),
        session: Mutex::new(None),
        attached: Notify::new(),
        closed: AtomicBool::new(false),
        filter: TargetFilter::default(),
    });
    (inner, cmd_rx)
}
```

Ajouter deux tests supplémentaires :

```rust
#[test]
fn filter_url_contains_matches_wasm_page() {
    let filter = TargetFilter {
        url_contains: Some("localhost:5000".to_string()),
        title_contains: None,
        accept_blank: false,
    };
    let t = TargetInfo {
        target_id: "x".into(),
        kind: "page".into(),
        title: "Atlas".into(),
        url: "http://localhost:5000/".into(),
    };
    assert!(filter.matches(&t));
}

#[test]
fn filter_title_contains_matches_atlas_page() {
    let filter = TargetFilter {
        url_contains: None,
        title_contains: Some("atlas".to_string()),
        accept_blank: false,
    };
    let t = TargetInfo {
        target_id: "x".into(),
        kind: "page".into(),
        title: "Atlas".into(),
        url: "http://localhost:5000/".into(),
    };
    assert!(filter.matches(&t));
}
```

**7. Re-export dans `lib.rs`**

```rust
pub use client::{CdpClient, TargetFilter};
```

### `deveye-server/src/main.rs`

**1. Deux nouveaux arguments CLI dans `struct Args`**

```rust
/// Sous-chaine (insensible a la casse) que doit contenir l'URL de la page CDP
/// cible. Si absente et si --cdp-target-filter-title est aussi absent,
/// le filtre par defaut "dioxus" (comportement historique) est utilise.
#[arg(long)]
cdp_target_filter_url: Option<String>,

/// Sous-chaine (insensible a la casse) que doit contenir le titre de la page
/// CDP cible.
#[arg(long)]
cdp_target_filter_title: Option<String>,

/// Accepte les pages "about:blank" comme cible CDP (transitoire wry au boot).
/// Active par defaut quand le filtre est celui par defaut (Dioxus).
/// Desactive automatiquement des qu'un filtre explicite est fourni, sauf
/// si ce flag est passe explicitement.
#[arg(long, default_value_t = false)]
cdp_accept_blank: bool,
```

**2. Construction du `TargetFilter` à partir des args**

À placer juste après `let args = Args::parse();`, avant la création de `LazyCdp` :

```rust
let cdp_filter = if args.cdp_target_filter_url.is_none()
    && args.cdp_target_filter_title.is_none()
{
    // Aucun filtre explicite : comportement Dioxus par defaut (retro-compatible).
    TargetFilter::default()
} else {
    TargetFilter {
        url_contains: args.cdp_target_filter_url.clone(),
        title_contains: args.cdp_target_filter_title.clone(),
        accept_blank: args.cdp_accept_blank,
    }
};
```

**3. Propagation du filtre dans `LazyCdp`**

`LazyCdp` reçoit un champ `filter: TargetFilter` et le transmet à `CdpClient::connect` :

```rust
struct LazyCdp {
    url: String,
    filter: TargetFilter,
    slot: tokio::sync::Mutex<Option<Arc<CdpClient>>>,
}

impl LazyCdp {
    fn new(url: String, filter: TargetFilter, initial: Option<Arc<CdpClient>>) -> Self {
        Self {
            url,
            filter,
            slot: tokio::sync::Mutex::new(initial),
        }
    }

    async fn client(&self) -> PortResult<Arc<CdpClient>> {
        let mut slot = self.slot.lock().await;
        if let Some(client) = slot.as_ref() {
            if !client.is_closed() {
                return Ok(client.clone());
            }
        }
        match CdpClient::connect(&self.url, self.filter.clone()).await {
            Ok(client) => {
                let client = Arc::new(client);
                *slot = Some(client.clone());
                Ok(client)
            }
            Err(e) => Err(PortError::Network(format!(
                "CDP injoignable sur {} ({e}) — lancer le navigateur cible \
                 puis rappeler l'outil",
                self.url
            ))),
        }
    }
}
```

**4. Passage du filtre aux points d'instanciation de `LazyCdp`**

Dans la section de connexion initiale :

```rust
let initial = if args.target == TargetMode::Sdk {
    None
} else {
    match CdpClient::connect(&args.cdp_url, cdp_filter.clone()).await {
        Ok(client) => Some(Arc::new(client)),
        Err(e) => {
            eprintln!(
                "deveye-server : CDP injoignable au demarrage ({e}) — \
                 reconnexion automatique au premier appel d'outil."
            );
            None
        }
    }
};
```

Dans la composition des adapters :

```rust
// TargetMode::Cdp
let lazy = Arc::new(LazyCdp::new(args.cdp_url.clone(), cdp_filter.clone(), initial));

// TargetMode::Auto
let lazy = Arc::new(LazyCdp::new(args.cdp_url.clone(), cdp_filter, initial));
```

**5. Import à ajouter**

```rust
use adapter_cdp_webview::{CdpClient, TargetFilter};
```

### `docs/usage.md`

Ajouter une nouvelle section après la section « Linux — piloter l'app desktop sans CDP » :

**Titre** : `## Piloter une app WASM via un Chromium externe`

Contenu : explication du mode `--target cdp` avec filtre explicite, commande de lancement du navigateur, exemple de `.mcp.json`. Voir la section « Nouveaux artefacts » pour le modèle exact.

Mettre à jour la ligne de troubleshooting `docs/usage.md:174` :

```
| `timeout en attente d'une page CDP attachable (10 s)` | Aucune page ne matche le filtre configuré | Verifier que le navigateur est ouvert sur la bonne URL ; passer `--cdp-target-filter-url <sous-chaine>` ou `--cdp-target-filter-title <sous-chaine>` |
```

### `docs/architecture.md`

Mettre à jour la description du point 4 de la section « Robustesse CDP » (`architecture.md:226`) :

```
4. Filtre target configurable (`TargetFilter`) : `type=page` ET (URL ou titre contient la sous-chaine configuree, OU `about:blank` si `accept_blank`). Par defaut : sous-chaine "dioxus" + `about:blank` (retro-compatible Dioxus). Passe depuis la CLI via `--cdp-target-filter-url` / `--cdp-target-filter-title`.
```

## Nouveaux artefacts

### `TargetFilter` (struct publique dans `adapter-cdp-webview`)

```rust
// adapter-cdp-webview/src/client.rs
#[derive(Clone, Debug)]
pub struct TargetFilter {
    pub url_contains: Option<String>,
    pub title_contains: Option<String>,
    pub accept_blank: bool,
}
```

Exportée dans `adapter-cdp-webview/src/lib.rs` :

```rust
pub use client::{CdpClient, TargetFilter};
```

### Commande de lancement du navigateur (script, non commité dans DevEye)

Pour servir une app WASM en mode CDP, il faut un Chromium avec `--remote-debugging-port` et un profil isolé (pour éviter les conflits avec un navigateur personnel déjà ouvert) :

```bash
brave --remote-debugging-port=9222 \
      --user-data-dir=/tmp/deveye-brave \
      http://localhost:5000
```

Le `--user-data-dir` est obligatoire : sans lui, si Brave est déjà ouvert, l'instance existante absorbe la commande et ignore `--remote-debugging-port`. Le port 9222 est la valeur par défaut de `--cdp-url` dans `deveye-server`.

### Exemple de `.mcp.json` pour une app WASM tierce (à placer à la racine du projet client)

L'exemple ci-dessous cible une tête WASM servie sur `localhost:5000` en filtrant sur l'URL. Aucun nom d'app n'est codé en dur dans DevEye : la sous-chaîne est une valeur purement opératrice.

```json
{
  "mcpServers": {
    "deveye": {
      "type": "stdio",
      "command": "~/Projets/DevEye/target/release/deveye-server",
      "args": [
        "--cdp-url", "http://localhost:9222",
        "--cdp-target-filter-url", "localhost:5000",
        "--target", "cdp",
        "--serve-mcp"
      ]
    }
  }
}
```

Variante par titre (si l'URL est générique mais le titre est distinctif) :

```json
"args": [
  "--cdp-url", "http://localhost:9222",
  "--cdp-target-filter-title", "atlas",
  "--target", "cdp",
  "--serve-mcp"
]
```

Les deux flags peuvent être combinés (relation OU). `--sdk-url` est omis : le SDK Rust n'est pas embarquable dans une cible .NET, `state` et `logs_tail` ne sont pas disponibles sur cette cible (voir section Risques).

### Ordre de lancement pour une tête WASM

```bash
# Terminal 1 — app WASM (dotnet run, DX CLI, ou tout autre outillage)
dotnet run --project src/MonApp -f net10.0-browserwasm
# attendre que le serveur soit pret (message "Now listening on: http://localhost:5000")

# Terminal 2 — Chromium isole en mode CDP
brave --remote-debugging-port=9222 \
      --user-data-dir=/tmp/deveye-brave \
      http://localhost:5000

# Terminal 3 — Claude Code dans le projet client
claude
```

## Critère de done

```bash
# Compilation sans avertissement
cargo check --workspace

# Lint strict
cargo clippy --workspace --all-targets -- -D warnings

# Tests unitaires (incluant les deux nouveaux tests TargetFilter)
cargo test -p adapter-cdp-webview

# Retro-compatibilite : le filtre par defaut se comporte comme avant
cargo test -p adapter-cdp-webview -- target_matches
# -> les tests historiques (renommes en filter_*) passent

# Smoke test integration manuelle (app WASM en cours sur :5000, Brave sur :9222)
cargo run -p deveye-server -- \
  --cdp-url http://localhost:9222 \
  --cdp-target-filter-url localhost:5000 \
  --target cdp \
  --serve-mcp
# -> dans Claude Code : "prends un screenshot" doit retourner une image de la page WASM

# Verification retro Dioxus (app Dioxus desktop sur :9222, aucun flag filtre)
cargo run -p deveye-server -- \
  --cdp-url http://localhost:9222 \
  --target cdp \
  --serve-mcp
# -> screenshot doit fonctionner sans --cdp-target-filter-url (filtre par defaut)

# Verification de non-regression : les tests d'integration existants passent
cargo test --workspace
```

Comportement observable attendu :
- Avec `--cdp-target-filter-url localhost:5000` et une page WASM sur `http://localhost:5000/` : `Target.attachedToTarget` est reçu dans les 5 secondes après `CdpClient::connect`, la session est posée, `screenshot` renvoie une image de la page.
- Sans aucun flag de filtre : comportement identique à avant ce jalon sur une app Dioxus.
- Avec un filtre qui ne matche aucune page ouverte : `wait_attached` expire après 10 secondes avec le message `"timeout en attente d'une page CDP attachable (10 s)"` (comportement inchangé).

## Risques et pièges

**Aucun SDK disponible sur cible .NET.** `state`, `logs_tail`, `logs_wait`, `invoke_command`, `list_commands` renvoient une erreur `PortError::Network` (le `sdk_url` par défaut `localhost:31337` n'écoute pas). C'est attendu et documenté : seules les capacités CDP (`screenshot`, `click*`, `type_text`, `press_key`, `scroll`, `dom`, `eval_js`, `navigate`) sont disponibles. Pas d'erreur silencieuse : l'outil renvoie un message explicite.

**Conflit de Chromium.** Si Brave (ou Chrome) est déjà ouvert sans `--user-data-dir` dédié, `--remote-debugging-port=9222` est ignoré par l'instance existante et `/json/version` est injoignable. Solution : toujours passer `--user-data-dir=/tmp/deveye-brave` (ou tout chemin éphémère).

**Page de chargement transitoire (`about:blank`).** Pour une cible WASM avec `--cdp-target-filter-url localhost:5000`, passer `accept_blank: false` (comportement automatique quand un filtre explicite est fourni). Si `accept_blank` restait à `true`, `deveye-server` pourrait s'attacher à la page `about:blank` initiale avant que le WASM soit chargé, puis perdre la session au premier rechargement. La logique de construction du filtre dans `main.rs` désactive `accept_blank` dès qu'un filtre explicite est présent, sauf si `--cdp-accept-blank` est passé explicitement.

**Plusieurs onglets ouverts.** Si le Chromium externe a plusieurs onglets et que deux correspondent au filtre, `deveye-server` s'attache au premier `Target.targetCreated` qui matche. Fournir un filtre assez discriminant (URL précise, ou titre unique) pour éviter toute ambiguïté.

**Découplage nom d'app.** Ne jamais coder en dur une sous-chaîne spécifique à une app cible dans le code DevEye (`"atlas"`, `"localhost:5000"`, etc.). Ces valeurs sont des opérateurs de configuration, transmises exclusivement via la CLI et le `.mcp.json` de l'app consommatrice. La règle 4 du CLAUDE.md DevEye s'applique sans exception.

**Règle hexagonale.** `TargetFilter` vit dans `adapter-cdp-webview` (un adapter), pas dans `deveye-ports` ni dans `deveye-core`. Elle n'est pas un concept du domaine mais un détail de configuration CDP. `deveye-server` (orchestrateur) peut y avoir accès : la dépendance `deveye-server -> adapter-cdp-webview` existe déjà (cf. `deveye-server/Cargo.toml`).

**Compatibilité `args` CLI.** Les deux nouveaux flags (`--cdp-target-filter-url`, `--cdp-target-filter-title`) sont optionnels et absents par défaut. Tout `.mcp.json` existant qui ne les mentionne pas continue de fonctionner avec le filtre Dioxus par défaut. Aucune migration requise.

**Wayland / Linux.** Ce jalon ne dépend pas du display server : CDP est un protocole réseau local, indépendant du backend graphique. L'app WASM tourne dans un Chromium externe (Brave), le screencap xcap n'est pas utilisé (mode `--target cdp`). Pas de régression attendue sur la machine de dev Fedora/Wayland.

## Checklist d'exécution

- [ ] Lire `adapter-cdp-webview/src/client.rs` en entier avant de modifier (s'assurer que `handle_event` et `spawn_reader` sont bien compris).
- [ ] Définir `TargetFilter` (struct + `Default` + méthode `matches`) dans `client.rs`, avant `CdpInner`.
- [ ] Ajouter `filter: TargetFilter` dans `CdpInner`.
- [ ] Modifier la signature de `CdpClient::connect` : `pub async fn connect(http_url: &str, filter: TargetFilter) -> Result<Self, CdpError>`.
- [ ] Propager `filter` dans la construction de `CdpInner` dans `connect`.
- [ ] Remplacer les deux appels `target_matches(...)` dans `handle_event` par `inner.filter.matches(...)`.
- [ ] Supprimer la fonction libre `target_matches`.
- [ ] Re-exporter `TargetFilter` dans `lib.rs` : `pub use client::{CdpClient, TargetFilter};`.
- [ ] Mettre à jour `test_inner` dans le module `tests` pour passer `filter: TargetFilter::default()`.
- [ ] Mettre à jour les sept tests existants : remplacer `target_matches(&t)` par `TargetFilter::default().matches(&t)`.
- [ ] Ajouter les deux tests `filter_url_contains_matches_wasm_page` et `filter_title_contains_matches_atlas_page`.
- [ ] `cargo test -p adapter-cdp-webview` : tous les tests passent.
- [ ] Dans `deveye-server/src/main.rs` : ajouter `use adapter_cdp_webview::{CdpClient, TargetFilter};`.
- [ ] Ajouter les trois nouveaux champs dans `struct Args` (`cdp_target_filter_url`, `cdp_target_filter_title`, `cdp_accept_blank`).
- [ ] Ajouter la logique de construction de `cdp_filter` après `Args::parse()`.
- [ ] Ajouter `filter: TargetFilter` dans `LazyCdp`, propager à `new()` et à `CdpClient::connect(...)` dans `client()`.
- [ ] Mettre à jour les deux points d'instanciation de `LazyCdp` (modes `Cdp` et `Auto`) pour passer `cdp_filter.clone()` / `cdp_filter`.
- [ ] Mettre à jour l'appel initial `CdpClient::connect(&args.cdp_url, cdp_filter.clone())`.
- [ ] `cargo check --workspace` : pas d'erreur de compilation.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` : zéro warning.
- [ ] `cargo test --workspace` : tous les tests passent.
- [ ] Mettre à jour `docs/architecture.md:226` (description du filtre target).
- [ ] Mettre à jour `docs/usage.md:174` (ligne troubleshooting timeout).
- [ ] Ajouter la section « Piloter une app WASM via un Chromium externe » dans `docs/usage.md` avec la commande Brave et le modèle de `.mcp.json`.
- [ ] Vérification manuelle : smoke test avec un Chromium sur une page WASM — `screenshot` MCP retourne une image.
- [ ] Vérification manuelle rétro-compat : smoke test Dioxus sans flags de filtre — toujours fonctionnel.
- [ ] Commit : `feat(adapter-cdp): remplace le filtre target hard-codé par TargetFilter configurable`.
- [ ] Commit documentation : `docs: décrit le pilotage WASM via Chromium externe (M2)`.
