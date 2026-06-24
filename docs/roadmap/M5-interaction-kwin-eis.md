# M5 — adapter-interaction-kwin-eis : mains OS-level sous Wayland natif

## Objectif

Implémenter le port `InteractionSink` via l'injection d'événements d'entrée Wayland en s'appuyant sur le protocole EIS (Emulated Input System) exposé par KWin via D-Bus (`org.kde.KWin.EIS.RemoteDesktop`), afin de piloter n'importe quelle fenêtre graphique — y compris les cibles sans SDK et sans WebView comme atlas Uno desktop (rendu Skia) — sans passer par XWayland ni par un shim dans l'app cible.

- **Effort** : L
- **Dépend de** : M1 (adapter-screencap-kwin fournit les coordonnées de fenêtre pour convertir positions logiques -> coordonnées écran absolues), M0 (wire protocol stabilise les types partagées)
- **Statut décision** : M5 tranché comme jalon après M1 et M1b

## Pourquoi (verrou levé)

Sous Wayland, aucune app ne peut injecter des événements d'entrée dans une autre fenêtre par construction : le protocole interdit l'accès direct aux buffers d'entrée des surfaces tierces. Les solutions historiques (XTest, `xdotool`, `/dev/uinput` via `ydotool`) s'appuient toutes sur XWayland ou sur le chemin `uinput` du noyau qui produit des coordonnées absolues incorrectes en session Wayland multi-moniteur (issue connue dans ydotool, cf. [ydotool#250](https://github.com/ReimuNotMoe/ydotool/issues/250) — les coordonnées absolues sont calculées par rapport au framebuffer virtuel global, pas à la fenêtre cible).

La bonne interface Wayland-native est **EIS** (Emulated Input System), standard porté par libei et supporté par KDE Plasma depuis KWin 6.x : KWin ouvre un file descriptor EIS que le client utilise comme canal d'injection ; les événements arrivent dans le compositeur avec la même confiance qu'un périphérique physique. L'adapter `adapter-screencap-kwin` (M1) est nécessaire pour obtenir la géométrie de la fenêtre cible et convertir les positions logiques relatives au viewport en coordonnées absolues sur l'écran.

Les cibles sans SDK ni WebView — dont atlas Uno desktop (rendu Skia, aucune WebView, SDK Rust impossible à embarquer dans une app .NET) — n'ont aucun chemin d'interaction aujourd'hui : `JsInteraction` (voie SDK) et `CdpClient` (voie CDP) sont toutes deux inapplicables. M5 comble ce manque.

## État actuel du code

Le port `InteractionSink` est défini et stable :

```
deveye-ports/src/interaction.rs:20-22
```

```rust
#[async_trait]
pub trait InteractionSink: Send + Sync {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()>;
}
```

Les variantes de `InteractionCommand` sont définies dans :

```
deveye-core/src/command.rs:24-52
```

```rust
pub enum InteractionCommand {
    Click { position: Position, button: MouseButton },
    Hover { position: Position },
    Type { text: String },
    KeyPress { key: String },
    Navigate { route: String },
    Scroll { position: Position, delta_x: i32, delta_y: i32 },
    Resize { width: u32, height: u32 },
}
```

Le modèle de mapping de référence est `adapter-cdp-webview/src/interaction.rs`. Il illustre le pattern attendu : chaque variante de `InteractionCommand` est traduite en primitives du medium d'injection (ici CDP), les erreurs sont enveloppées dans `PortError::Interaction`, les variantes non applicables renvoient `PortError::Unsupported` :

```
deveye-adapters/adapter-cdp-webview/src/interaction.rs:10-103
```

Le binaire orchestrateur compose les adapters dans `deveye-server/src/main.rs`. Le `TargetMode` actuel (`Auto`, `Cdp`, `Sdk`) ne connaît pas encore `kwin-eis` :

```
deveye-server/src/main.rs:64-69
```

```rust
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum TargetMode {
    Auto,
    Cdp,
    Sdk,
}
```

L'argument `--target` est défini à la ligne 54-57. Il n'existe aucun crate `adapter-interaction-kwin-eis` dans le workspace (`Cargo.toml:3-14` liste 9 membres, aucun n'est kwin-eis).

Les types d'erreurs disponibles côté ports incluent `PortError::Interaction(String)` et `PortError::Unsupported(String)` (`deveye-ports/src/errors.rs:16-29`), les deux seront utilisés dans le nouvel adapter.

## Changements détaillés

### `deveye-adapters/adapter-interaction-kwin-eis/` (nouveau crate)

Créer le répertoire et les fichiers suivants.

**`Cargo.toml`**

```toml
[package]
name = "adapter-interaction-kwin-eis"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true

[dependencies]
deveye-core  = { path = "../../deveye-core" }
deveye-ports = { path = "../../deveye-ports" }
async-trait  = { workspace = true }
tokio        = { workspace = true }
thiserror    = { workspace = true }
# Interface D-Bus vers KWin EIS (zbus = implementation pure Rust, pas de libdbus-sys)
zbus         = { version = "4", default-features = false, features = ["tokio"] }
# FFI libei pour envoyer les evenements sur le fd EIS
libei         = "0.1"    # crate libei-rs ; a valider en cargo search libei

[target.'cfg(not(target_os = "linux"))'.dependencies]
# Rien : le crate ne compile et n'est inclus que sous Linux.

[dev-dependencies]
tokio = { workspace = true, features = ["rt", "macros"] }
```

> Note : si la crate `libei` n'est pas disponible sur crates.io au moment de l'implémentation, le fd EIS peut être manipulé directement via `std::os::unix::io::RawFd` et des syscalls `write(2)` avec les structures C de `libei.h` exposées par une petite couche FFI (`unsafe`, dans un sous-module `ffi.rs`). Dans ce cas, ajouter `libei-sys` ou un module `ffi` interne ; documenter le choix dans un commentaire de module.

**`src/lib.rs`**

```rust
//! Adapter `adapter-interaction-kwin-eis` — mains OS-level sous Wayland natif.
//!
//! Injecte des evenements d'entree dans n'importe quelle fenetre Wayland en
//! passant par le protocole EIS de KWin (`org.kde.KWin.EIS.RemoteDesktop`).
//!
//! # Flux
//!
//! 1. Appel D-Bus `connectToEIS` sur `org.kde.KWin.EIS.RemoteDesktop` →
//!    KWin renvoie un `UnixFd` (file descriptor EIS ouvert).
//! 2. Initialisation d'une session `libei` sur ce fd (capabilities :
//!    `EI_DEVICE_CAP_POINTER`, `EI_DEVICE_CAP_KEYBOARD`).
//! 3. A chaque `dispatch`, on traduit l'`InteractionCommand` en sequence
//!    d'evenements libei (pointer_motion_absolute / pointer_button /
//!    keyboard_key) et on les ecrit sur le fd.
//!
//! # Securite
//!
//! L'acces EIS est donne par KWin via D-Bus : il est soumis a la politique
//! de l'utilisateur courant (session active, pas de cross-session). Aucune
//! elevation de privilege n'est necessaire.
//!
//! # Variantes non supportees
//!
//! `Navigate` et `Resize` sont semantiquement impossibles OS-level (elles
//! supposent un routeur applicatif ou un protocole WebView) et renvoient
//! `PortError::Unsupported`.

#![forbid(unsafe_code)]
// Note : le bloc FFI libei (si libei-rs n'est pas dispo) sera isole dans
// `ffi.rs` et marque `#[allow(unsafe_code)]` au niveau du module uniquement.
#![deny(rust_2018_idioms)]

mod connection;
mod dispatch;
mod error;

pub use connection::KwinEisInteraction;
```

**`src/error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum EisError {
    #[error("D-Bus KWin EIS : {0}")]
    DBus(#[from] zbus::Error),
    #[error("libei : {0}")]
    Libei(String),
}

impl From<EisError> for deveye_ports::PortError {
    fn from(e: EisError) -> Self {
        deveye_ports::PortError::Interaction(e.to_string())
    }
}
```

**`src/connection.rs`**

```rust
use std::os::unix::io::OwnedFd;
use std::sync::Arc;

use tokio::sync::Mutex;
use zbus::Connection as ZbusConn;

use crate::error::EisError;

/// Proxy D-Bus vers `org.kde.KWin.EIS.RemoteDesktop`.
/// Interface cible : `org.kde.KWin.EIS.RemoteDesktop` sur le bus de session,
/// chemin canonique `/org/kde/KWin/EIS/RemoteDesktop`.
#[zbus::proxy(
    interface = "org.kde.KWin.EIS.RemoteDesktop",
    default_service = "org.kde.KWin",
    default_path = "/org/kde/KWin/EIS/RemoteDesktop"
)]
trait KwinEisProxy {
    /// Demande a KWin d'ouvrir une session EIS pour ce processus.
    /// Retourne un UnixFd (file descriptor EIS) sur lequel libei peut etre
    /// initialise.
    async fn connect_to_eis(&self) -> zbus::Result<zbus::zvariant::OwnedFd>;
}

/// Handle EIS actif : fd ouvert + contexte libei initialise.
pub(crate) struct EisSession {
    /// File descriptor EIS donne par KWin.
    pub fd: OwnedFd,
    // Le contexte libei (ei_t ou equivalent crate) vit ici.
    // Placeholder jusqu'a choix definitif de la crate FFI.
}

/// Adapter principal expose cote `pub`.
///
/// Initialisation paresseuse : la connexion D-Bus et l'appel `connectToEIS`
/// sont effectues au premier `dispatch`, pas au `new`. Ainsi un
/// `deveye-server` demarre sans erreur meme si KDE n'est pas disponible.
pub struct KwinEisInteraction {
    session: Arc<Mutex<Option<EisSession>>>,
    /// Coordonnees d'origine de la fenetre cible sur l'ecran absolu,
    /// fournies par adapter-screencap-kwin (M1) au moment de la composition.
    /// Permet de convertir une `Position` viewport-relative en coordonnees
    /// ecran absolues attendues par EIS.
    pub window_origin: Arc<Mutex<(i32, i32)>>,
}

impl KwinEisInteraction {
    /// Construit l'adapter. `window_origin` peut etre mis a jour dynamiquement
    /// par l'adapter screencap KWin entre deux appels de `dispatch`.
    pub fn new(window_origin: (i32, i32)) -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            window_origin: Arc::new(Mutex::new(window_origin)),
        }
    }

    /// Ouvre (ou reutilise) la session EIS via D-Bus.
    pub(crate) async fn session(&self) -> Result<EisSession, EisError> {
        // Connexion D-Bus de session
        let conn = ZbusConn::session().await?;
        let proxy = KwinEisProxyProxy::new(&conn).await?;
        let fd: zbus::zvariant::OwnedFd = proxy.connect_to_eis().await?;
        // Initialisation libei sur le fd — implementation dans dispatch.rs
        Ok(EisSession {
            fd: fd.into(),
        })
    }
}
```

**`src/dispatch.rs`**

Ce fichier contient l'implémentation du trait `InteractionSink`. Le modèle de mapping suit exactement le patron de `adapter-cdp-webview/src/interaction.rs:10-103`, adapté aux primitives libei.

```rust
use async_trait::async_trait;

use deveye_core::{InteractionCommand, MouseButton, Position};
use deveye_ports::{InteractionSink, PortError, PortResult};

use crate::connection::KwinEisInteraction;
use crate::error::EisError;

/// Mapping InteractionCommand -> sequence d'evenements EIS/libei.
///
/// Variantes supportees OS-level :
///   Click  -> pointer_motion_absolute + button_press + button_release
///   Hover  -> pointer_motion_absolute
///   Type   -> sequence de keyboard_key (keycode XKB par char UTF-32)
///   KeyPress -> keyboard_key (press + release)
///   Scroll -> pointer_scroll (axe vertical et/ou horizontal)
///
/// Variantes non supportees (semantique applicative sans acces WebView) :
///   Navigate -> PortError::Unsupported
///   Resize   -> PortError::Unsupported
#[async_trait]
impl InteractionSink for KwinEisInteraction {
    async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
        // Recuperation de l'origine fenetre pour conversion de coordonnees.
        let (ox, oy) = *self.window_origin.lock().await;

        match command {
            InteractionCommand::Click { position, button } => {
                let abs = to_absolute(position, ox, oy);
                // 1. Mouvement absolu (declenche les effets de hover/mouseover
                //    cote framework, comme le fait CDP avec mouseMoved + 60 ms
                //    de delai — cf. adapter-cdp-webview/src/interaction.rs:21-24).
                eis_pointer_motion_absolute(abs).await
                    .map_err(PortError::from)?;
                // 2. Pression puis relachement du bouton.
                eis_button(abs, button, true).await
                    .map_err(PortError::from)?;
                eis_button(abs, button, false).await
                    .map_err(PortError::from)?;
            }
            InteractionCommand::Hover { position } => {
                let abs = to_absolute(position, ox, oy);
                eis_pointer_motion_absolute(abs).await
                    .map_err(PortError::from)?;
            }
            InteractionCommand::Type { text } => {
                // Chaque caractere Unicode est converti en keycode XKB via
                // xkbcommon puis envoye comme paire key_down / key_up.
                // Alternative de depannage sans xkbcommon : coller via le
                // presse-papiers Wayland (wl-clipboard), moins fiable.
                for ch in text.chars() {
                    eis_type_char(ch).await
                        .map_err(PortError::from)?;
                }
            }
            InteractionCommand::KeyPress { key } => {
                // `key` est un nom de touche XKB (ex. "Return", "Tab",
                // "Escape") ; xkbcommon le resout en keycode.
                eis_key_name(&key, true).await
                    .map_err(PortError::from)?;
                eis_key_name(&key, false).await
                    .map_err(PortError::from)?;
            }
            InteractionCommand::Scroll { position, delta_x, delta_y } => {
                let abs = to_absolute(position, ox, oy);
                eis_scroll(abs, delta_x, delta_y).await
                    .map_err(PortError::from)?;
            }
            InteractionCommand::Navigate { route: _ } => {
                return Err(PortError::Unsupported(
                    "Navigate necessite un routeur applicatif (SDK ou CDP) \
                     — non disponible en mode kwin-eis".to_string(),
                ));
            }
            InteractionCommand::Resize { .. } => {
                return Err(PortError::Unsupported(
                    "Resize necessite un acces WebView — non disponible en \
                     mode kwin-eis".to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// Convertit une position viewport-relative en coordonnees ecran absolues.
/// L'origine (ox, oy) est fournie par adapter-screencap-kwin (M1).
fn to_absolute(pos: Position, ox: i32, oy: i32) -> (i32, i32) {
    (pos.x + ox, pos.y + oy)
}

// Stubs en attente de l'implementation libei reelle.
// Remplacer par les appels libei-rs / FFI dans l'ordre de l'implementation.
async fn eis_pointer_motion_absolute(_abs: (i32, i32)) -> Result<(), EisError> {
    todo!("libei : ei_device_pointer_motion_absolute")
}

async fn eis_button(
    _abs: (i32, i32),
    _button: MouseButton,
    _pressed: bool,
) -> Result<(), EisError> {
    todo!("libei : ei_device_button_button")
}

async fn eis_type_char(_ch: char) -> Result<(), EisError> {
    todo!("libei + xkbcommon : resolution keycode depuis char Unicode")
}

async fn eis_key_name(_key: &str, _pressed: bool) -> Result<(), EisError> {
    todo!("libei + xkbcommon : resolution keycode depuis nom XKB")
}

async fn eis_scroll(_abs: (i32, i32), _dx: i32, _dy: i32) -> Result<(), EisError> {
    todo!("libei : ei_device_scroll_discrete ou ei_device_scroll_delta")
}
```

### `Cargo.toml` (workspace)

Ajouter le nouveau membre à la liste `members` (`Cargo.toml:3-14`) :

```toml
members = [
    "deveye-core",
    "deveye-ports",
    "deveye-adapters/adapter-cdp-webview",
    "deveye-adapters/adapter-screencap-xcap",
    "deveye-adapters/adapter-dioxus-signals",
    "deveye-adapters/adapter-logs-tracing",
    "deveye-adapters/adapter-commands-http",
    "deveye-adapters/adapter-mcp-server",
+   "deveye-adapters/adapter-interaction-kwin-eis",
    "deveye-sdk",
    "deveye-server",
]
```

### `deveye-server/src/main.rs`

**1. Ajouter la variante `KwinEis` dans `TargetMode`** (`main.rs:64-69`) :

```rust
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum TargetMode {
    Auto,
    Cdp,
    Sdk,
    KwinEis,   // nouveau : mains OS-level uniquement, yeux fournis par M1
}
```

**2. Ajouter l'import** en tête des `use` :

```rust
use adapter_interaction_kwin_eis::KwinEisInteraction;
```

**3. Étendre le branchement de composition** dans le `match args.target` (`main.rs:109-143`). Le mode `KwinEis` fournit uniquement les mains via EIS ; les yeux restent `XcapRenderer` (pont natif M1) ; DOM et évaluateur JS sont indisponibles (retournent `PortError::Unsupported`) :

```rust
TargetMode::KwinEis => {
    // Les coordonnees d'origine fenetre sont fournies par le renderer xcap (M1).
    // A terme : le renderer expose son origine via un trait partage ; en
    // attendant, (0, 0) est un placeholder — l'origine est mise a jour
    // dynamiquement dans `KwinEisInteraction::window_origin`.
    let eis_interaction = Arc::new(KwinEisInteraction::new((0, 0)));
    (
        Arc::new(XcapRenderer::new(xcap_title)),
        eis_interaction as Arc<dyn InteractionSink>,
        Arc::new(UnsupportedInspector),   // pas de DOM sans WebView
        Arc::new(UnsupportedEvaluator),   // pas d'eval JS sans WebView
    )
}
```

`UnsupportedInspector` et `UnsupportedEvaluator` sont des structs locaux à `main.rs` implémentant les ports concernés en renvoyant systématiquement `PortError::Unsupported`. Ils suivent le même pattern que `FallbackRenderer` (lignes 278-297) mais renvoient toujours l'erreur sans primaire.

**4. Ajouter l'option CLI `--interaction`** (complémentaire à `--target`, optionnelle, pour surcharger l'adapter d'interaction indépendamment des yeux) :

```rust
/// Adapter d'interaction a forcer (independant des yeux).
/// Si absent, l'adapter est derive du --target.
/// Valeurs : `kwin-eis`, `js-sdk`, `cdp`.
#[arg(long)]
interaction: Option<InteractionAdapter>,

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum InteractionAdapter {
    KwinEis,
    JsSdk,
    Cdp,
}
```

Cet argument permet par exemple de combiner des yeux CDP (WebView Atelier) avec des mains EIS (si le CDP d'interaction est flaky) sans changer le mode global.

### `deveye-server/Cargo.toml`

Ajouter la dépendance au nouveau crate :

```toml
[dependencies]
# ...existant...
adapter-interaction-kwin-eis = { path = "../deveye-adapters/adapter-interaction-kwin-eis" }
```

### `CLAUDE.md` (PC-Dada, `~/CLAUDE.md`)

Ajouter dans la section **Environnement de développement** :

```
- **Dépendances adapter-interaction-kwin-eis (M5 DevEye)** : `libei-devel` (librairie EIS Wayland, requis pour la compilation du crate) et `xkbcommon-devel` (résolution des keycodes XKB pour l'injection clavier). Installation : `SUDO_ASKPASS=/usr/bin/ksshaskpass sudo -A dnf install libei-devel xkbcommon-devel`. Sans ces paquets, le build du workspace DevEye échoue sur `libei-sys`/`xkbcommon-sys`.
```

## Nouveaux artefacts

**Nouveau crate** : `deveye-adapters/adapter-interaction-kwin-eis`

- Scope Conventional Commits : `adapter-eis`
- Exporte publiquement : `KwinEisInteraction`
- Trait implémenté : `deveye_ports::InteractionSink`
- Dépendances internes : `deveye-core`, `deveye-ports` uniquement (règle hexagonale respectée — aucune dépendance inter-adapter)
- Dépendances système : `libei-devel`, `xkbcommon-devel` (packages Fedora, section dnf)

**Variantes de `InteractionCommand` supportées vs. non supportées** :

| Variante | Statut | Primitive EIS |
|---|---|---|
| `Click` | supporté | `pointer_motion_absolute` + `button_press` + `button_release` |
| `Hover` | supporté | `pointer_motion_absolute` |
| `Type` | supporté | séquence `keyboard_key` (xkbcommon -> keycode) |
| `KeyPress` | supporté | `keyboard_key` press + release |
| `Scroll` | supporté | `scroll_discrete` ou `scroll_delta` |
| `Navigate` | `PortError::Unsupported` | sémantique applicative, impossible OS-level |
| `Resize` | `PortError::Unsupported` | sémantique WebView, impossible OS-level |

**Nouvel outil MCP** : aucun outil MCP nouveau. L'outil `interact` existant dans `adapter-mcp-server` accepte déjà `InteractionCommand` sérialisé ; le changement est transparent pour l'agent (seul l'adapter sous-jacent change selon `--target`).

## Critère de done

```bash
# 1. Compilation propre du workspace entier (y compris le nouveau crate)
cargo check --workspace

# 2. Aucun avertissement clippy
cargo clippy --workspace --all-targets -- -D warnings

# 3. Tests unitaires passes (domaine + adapters)
cargo test --workspace

# 4. Verification de la composition (smoke test sans KWin reel)
cargo run -p deveye-server -- --target kwin-eis --serve-mcp
# Attendu : le serveur demarre sans erreur, le log indique
# "adapter-interaction-kwin-eis charge, session EIS paresseuse"

# 5. Appel MCP depuis Claude : outil `interact` avec une commande Click
# Response attendue : la commande est dispatchee ; si KWin EIS est disponible
# (session KDE Plasma active), le clic est visible sur la cible.
# Si KWin est absent : PortError::Interaction avec message D-Bus explicite.

# 6. Appel MCP avec Navigate -> reponse PortError::Unsupported explicite
# (pas de panic, pas de hang).

# 7. Sur la machine PC-Dada (Fedora 44 KDE Plasma Wayland) :
SUDO_ASKPASS=/usr/bin/ksshaskpass sudo -A dnf install libei-devel xkbcommon-devel
cargo build -p adapter-interaction-kwin-eis
# Build vert = prerequis systeme OK
```

## Risques et pièges

**Wayland / KDE EIS**
- L'interface D-Bus `org.kde.KWin.EIS.RemoteDesktop` existe depuis KWin 6.x (Plasma 6). Sous KDE Plasma 5 (Fedora 38 et antérieur) elle est absente : l'appel `connectToEIS` échoue avec `org.freedesktop.DBus.Error.ServiceUnknown`. L'adapter doit renvoyer `PortError::Interaction` avec un message explicite, jamais `panic!`.
- La session doit être active côté utilisateur (pas de cross-session). L'adapter ouvre toujours le bus de session (`ZbusConn::session()`), jamais le bus système.
- Le fd EIS renvoyé par KWin est valable pour la durée de la session D-Bus associée. Si la connexion zbus est droppée, le fd devient invalide. Gérer le cycle de vie dans `EisSession`.

**Crate libei-rs**
- Au 24/06/2026, la disponibilité d'une crate `libei` stable sur crates.io est à vérifier avant d'ouvrir l'implémentation (`cargo search libei`). Si absente, recourir à une couche FFI manuelle (`bindgen` sur `libei.h`) dans un sous-module `unsafe` isolé, avec `#[allow(unsafe_code)]` au niveau du module uniquement (la directive `#![forbid(unsafe_code)]` en tête du crate devra alors être retirée et remplacée par `#![deny(unsafe_code)]` + exception locale documentée).
- Variante de dépannage uniquement (à ne pas utiliser en production de l'adapter) : `ydotool` via `uinput`. Raison du rejet : les coordonnées absolues de `ydotool` sont calculées dans le référentiel du framebuffer virtuel global (`/dev/uinput` ne connaît pas les géométries des fenêtres Wayland), ce qui les rend systématiquement fausses en configuration multi-moniteur ou quand la fenêtre cible n'est pas positionnée en (0, 0) — issue ydotool#250. Acceptable uniquement pour un test one-shot en moniteur unique.

**Dépendance à M1 pour les coordonnées**
- L'adapter doit recevoir l'origine de la fenêtre cible en coordonnées écran absolues depuis l'adapter screencap KWin (M1). Sans M1, `window_origin` reste (0, 0) et les clics sont émis en coordonnées relatives au coin haut-gauche du moniteur principal, pas de la fenêtre. Implémenter M1 avant de valider M5 en end-to-end.
- La mise à jour de `window_origin` entre deux appels de `dispatch` suppose un mécanisme de partage (par exemple un `Arc<Mutex<(i32, i32)>>` passé aux deux adapters au moment de la composition dans `deveye-server/src/main.rs`). Ce couplage de données en runtime doit rester dans `main.rs` (composition), jamais dans les crates d'adapters eux-mêmes (règle hexagonale : un adapter ne dépend pas d'un autre adapter).

**Règle hexagonale**
- `adapter-interaction-kwin-eis` ne doit dépendre que de `deveye-core` et `deveye-ports`. Aucune dépendance vers `adapter-screencap-kwin`, `adapter-cdp-webview` ou `deveye-sdk`. Le partage de `window_origin` se fait par passage de donnée primitive (`(i32, i32)` ou `Arc<Mutex<(i32, i32)>>`), pas par import de crate adapter.
- Le crate ne doit contenir aucun nom d'application cible en dur (ni "atlas", ni "atelier", ni "chronos"). Les heuristiques de titre de fenêtre restent dans `main.rs` via l'argument CLI `--screencap-window`.

**Sécurité dev-only**
- M5 ne touche pas au SDK (`deveye-sdk`) : aucun risque d'embarquement en release. L'adapter agit exclusivement côté DevEye.
- L'accès EIS est limité à la session KDE de l'utilisateur courant : pas d'élévation, pas d'accès réseau, pas de bind de port. La contrainte `127.0.0.1` s'applique au serveur MCP (`deveye-server`), pas à l'adapter EIS qui ne fait que des appels D-Bus locaux.

**Conversion Type/KeyPress**
- `xkbcommon` est nécessaire pour résoudre les keycodes à partir de noms XKB (`"Return"`, `"Tab"`) ou de codepoints Unicode. Sans lui, seule l'injection de touches par scancode brut est possible (fastidieux, fragile). Ajouter `xkbcommon-devel` aux prérequis système (documenter dans `CLAUDE.md` PC-Dada).
- La saisie de texte Unicode (`Type`) est plus robuste via le protocole `zwp_text_input_v3` de Wayland (qui prend un string directement) que via une séquence de keycodes. À évaluer selon ce que libei expose : si `ei_device_type_text` (ou équivalent) est disponible, le préférer à la boucle `eis_type_char`.

## Checklist d'exécution

- [ ] Vérifier la disponibilité de `libei` sur crates.io (`cargo search libei`) et choisir entre la crate et une couche FFI `bindgen`
- [ ] Installer les prérequis système : `SUDO_ASKPASS=/usr/bin/ksshaskpass sudo -A dnf install libei-devel xkbcommon-devel`
- [ ] Mettre à jour `CLAUDE.md` (PC-Dada) avec la ligne `libei-devel` + `xkbcommon-devel` dans la section Environnement de développement
- [ ] Créer le répertoire `deveye-adapters/adapter-interaction-kwin-eis/src/`
- [ ] Écrire `Cargo.toml` du nouveau crate (dépendances `deveye-core`, `deveye-ports`, `zbus`, `libei` ou `libei-sys`)
- [ ] Écrire `src/lib.rs`, `src/error.rs`, `src/connection.rs`, `src/dispatch.rs` avec les stubs `todo!()` pour les primitives libei
- [ ] Ajouter `adapter-interaction-kwin-eis` dans `Cargo.toml` workspace (`members`)
- [ ] Ajouter la dépendance dans `deveye-server/Cargo.toml`
- [ ] Ajouter la variante `KwinEis` dans `TargetMode` (`deveye-server/src/main.rs:64-69`)
- [ ] Ajouter l'import `use adapter_interaction_kwin_eis::KwinEisInteraction;` dans `main.rs`
- [ ] Implémenter la branche `TargetMode::KwinEis` dans le `match` de composition (`main.rs:109-143`) avec `UnsupportedInspector` et `UnsupportedEvaluator`
- [ ] Ajouter l'argument CLI optionnel `--interaction` (`InteractionAdapter`) dans `Args`
- [ ] `cargo check --workspace` vert (stubs compilés)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` vert
- [ ] Remplacer les stubs `todo!()` par les appels libei réels (pointer, button, keyboard, scroll)
- [ ] Implémenter `eis_type_char` via `zwp_text_input_v3` ou boucle xkbcommon selon ce que libei expose
- [ ] Tester en session KDE Plasma Wayland sur PC-Dada : clic sur une fenêtre cible (atlas Uno desktop ou terminal)
- [ ] Valider que `Navigate` et `Resize` renvoient `PortError::Unsupported` et non un `panic!`
- [ ] Valider que le serveur MCP démarre sans erreur même quand KDE n'est pas disponible (session SSH sans affichage)
- [ ] `cargo test --workspace` vert
- [ ] Commit : `feat(adapter-eis): implémente InteractionSink via KWin EIS pour les mains Wayland OS-level`
