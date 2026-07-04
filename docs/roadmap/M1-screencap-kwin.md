# M1 — adapter-screencap-kwin : yeux fiables sous Wayland natif

> **Note de rectification (audit du 04/07/2026)** : la migration `xcap 0.0.14` → `0.9.6` planifiée dans ce document (section « Changements détaillés » → `Cargo.toml`) n'a été **exécutée que le 04/07/2026** (sprint R3, hygiène CI/packaging), pas au moment de M1. Entre les deux, `adapter-screencap-xcap` est resté sur `xcap 0.0.14` — c'est la source du warning `future-incompat` relevé dans la baseline de l'audit du 04/07. `deveye-adapters/adapter-screencap-xcap/Cargo.toml` porte désormais `xcap = "0.9.6"`. Le reste de ce document (contexte, risques, alternative KWin D-Bus) reste valide tel quel.

## Objectif

Implémenter le port `Renderer` via l'interface D-Bus `org.kde.KWin.ScreenShot2` (méthode `CaptureWindow` par UUID) afin de capturer des fenêtres graphiques sous KDE/Wayland natif sans passer par XWayland ni par CDP. Le renderer est sélectionné automatiquement selon la variable `XDG_SESSION_TYPE` au démarrage de `deveye-server`.

**Effort :** L — D-Bus asynchrone (zbus), pipe, décodage ARGB, intégration dans la composition de `main.rs`.
**Depend de :** M0 (wire protocol et port `Renderer` stabilisés).
**Statut decision :** tranche — KWin D-Bus prioritaire sur XWayland ; adapter-screencap-xcap conservé comme repli X11/hors-KDE.

---

## Pourquoi (verrou levé)

Sous une session Wayland native (le cas de la machine de dev : Fedora 44, `XDG_SESSION_TYPE=wayland`), le compositor interdit l'accès aux pixels d'une fenêtre tierce sans passer par un protocole dédié. `xcap` contourne la limitation en forçant l'app cible à tourner sous **XWayland** (pont de compatibilité X11), ce qui entraîne trois problèmes :

1. **Résolution fausse** : XWayland applique un facteur d'échelle implicite (HiDPI). Les coordonnées de clic deviennent décalées par rapport aux pixels capturés, rendant le futur adapter d'interaction (`adapter-interaction-kwin-eis`, M5) inutilisable.
2. **Latence doublée** : XWayland serialise les frames via le protocole X avant que `xcap` les lise. Sur une fenêtre Dioxus / WebKitGTK (Atelier, Chronos), la latence observable est 2-4× supérieure à une capture KWin directe.
3. **Instabilité** : `xcap 0.0.14` (version actuelle — `adapter-screencap-xcap/Cargo.toml:14`) produit sporadiquement des images noires sur Wayland (race condition connue entre le blit XWayland et le grab xcap) ; la PR xcap#73 n'est pas encore fusionnée en `0.0.14`.

La référence de code : `deveye-adapters/adapter-screencap-xcap/src/renderer.rs:36` — `Window::all()` itère sur les fenêtres X11 exposées par xcap, ce qui requiert XWayland actif. `deveye-server/src/main.rs:89` montre le titre par défaut codé en dur `"atelier"`, ce qui sera generalisé en M1 mais illustre la voie xcap actuelle.

---

## Etat actuel du code

| Artefact | Fichier | Lignes pertinentes |
|---|---|---|
| Trait `Renderer` | `deveye-ports/src/renderer.rs:19-27` | Deux méthodes asynchrones : `screenshot(&self) -> PortResult<Screenshot>` et `viewport_size(&self) -> PortResult<Size>` |
| `XcapRenderer` | `deveye-adapters/adapter-screencap-xcap/src/renderer.rs:23-74` | Sélection par sous-chaîne de titre, capture via `xcap 0.0.14`, encodage PNG ; appel dans `spawn_blocking` |
| Erreurs xcap | `deveye-adapters/adapter-screencap-xcap/src/error.rs:5-23` | `ScreencapError` : `WindowNotFound`, `Xcap`, `Encoding` ; mapping vers `PortError` |
| Composition du renderer | `deveye-server/src/main.rs:109-143` | `TargetMode::Sdk` instancie `XcapRenderer::new(xcap_title)` ; `TargetMode::Auto` l'enveloppe dans `FallbackRenderer` |
| Option CLI de titre | `deveye-server/src/main.rs:47-49` | `--screencap-window: Option<String>`, défaut `"atelier"` (ligne 89) |
| Dépendances workspace | `Cargo.toml:24-36` | `async-trait`, `tokio`, `thiserror`, `serde`, `serde_json`, `chrono`, `reqwest` — **pas de `zbus`, pas de `nix`** |

L'adapter `adapter-screencap-xcap` est le seul renderer de type OS-level. Il n'existe pas encore d'adapter KWin D-Bus. La logique de sélection automatique dans `main.rs` ne consulte pas `XDG_SESSION_TYPE`.

---

## Changements détaillés

### `Cargo.toml` (workspace)

**Ajout de deux dépendances partagées** dans `[workspace.dependencies]` :

```toml
# D-Bus asynchrone pour l'interface KWin ScreenShot2.
# zbus est la crate Rust de référence pour D-Bus : async-native (tokio),
# générateur de proxy automatique via #[proxy], zero unsafe dans le code
# consommateur. Aucune alternative mature en Rust pur sans FFI libdbus.
zbus = { version = "4", default-features = false, features = ["tokio"] }

# Primitives POSIX (O_CLOEXEC, pipe2) nécessaires pour créer un pipe anonyme
# passé à KWin via le file descriptor. nix est plus sûre que libc brute
# (wrapping des erreurs POSIX en Result) et est déjà une dépendance indirecte
# via xcap sur Linux. On l'explose en dépendance directe pour le contrôle.
nix = { version = "0.29", default-features = false, features = ["fs", "unistd"] }
```

**Justification de l'ajout (règle CLAUDE.md)** : `zbus` est non substituable — l'interface `org.kde.KWin.ScreenShot2` n'expose pas d'autre protocole (pas de socket HTTP, pas d'API fichier). `nix` est non substituable pour `pipe2(O_CLOEXEC | O_NONBLOCK)` de manière sûre sans `unsafe libc` direct. Les deux crates sont scope-limitées au seul crate `adapter-screencap-kwin` (feature-gate workspace) et n'affectent pas les autres adapters.

**Mise à jour de xcap dans `adapter-screencap-xcap/Cargo.toml`** : `xcap = "0.0.14"` -> `xcap = "0.9.6"`. La version `0.9.6` corrige la race condition de capture noire sous XWayland (xcap PR#73 intégrée) et aligne l'API sur les versions >= 0.3 (la signature de `Window::capture_image()` reste identique).

### Nouveau crate `deveye-adapters/adapter-screencap-kwin/`

Ce crate est ajouté au workspace dans `Cargo.toml:members`.

**Structure** :

```
deveye-adapters/adapter-screencap-kwin/
  Cargo.toml
  src/
    lib.rs
    error.rs
    proxy.rs      ← proxy D-Bus généré par zbus #[proxy]
    renderer.rs   ← implémentation du trait Renderer
```

#### `adapter-screencap-kwin/Cargo.toml`

```toml
[package]
name = "adapter-screencap-kwin"
version = "0.1.0"
edition = "2021"
rust-version = "1.81"
license = "Proprietary"
authors = ["NuvoraCode <contact@nuvoracode.fr>"]
description = "Adapter DevEye : capture de fenetre Wayland via KWin D-Bus ScreenShot2."

[dependencies]
deveye-core  = { path = "../../deveye-core" }
deveye-ports = { path = "../../deveye-ports" }
async-trait  = { workspace = true }
thiserror    = { workspace = true }
tokio        = { workspace = true, features = ["sync", "rt", "io-util"] }
zbus         = { workspace = true }
nix          = { workspace = true }
image        = { version = "0.25", default-features = false, features = ["png"] }
```

#### `adapter-screencap-kwin/src/lib.rs`

```rust
//! Adapter DevEye : capture de fenêtre via `org.kde.KWin.ScreenShot2` (D-Bus).
//!
//! Cible : KDE Plasma sous Wayland natif (`XDG_SESSION_TYPE=wayland`).
//! Ne fonctionne pas sous Gnome, XFCE, ou en session X11 pure (utiliser
//! `adapter-screencap-xcap` dans ces cas).
//!
//! Aucune dépendance à une app cible précise : la sélection de fenêtre se fait
//! par sous-chaîne du titre de la fenêtre (`wm-caption`), configurable à
//! l'appel.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

mod error;
mod proxy;
mod renderer;

pub use error::KwinScreencapError;
pub use renderer::KwinRenderer;
```

#### `adapter-screencap-kwin/src/error.rs`

```rust
use deveye_ports::PortError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum KwinScreencapError {
    #[error("D-Bus KWin : {0}")]
    Dbus(String),

    #[error("aucune fenetre dont le titre contient {0:?} (ScreenShot2 windowId inconnu)")]
    WindowNotFound(String),

    #[error("lecture du pipe KWin : {0}")]
    Pipe(String),

    #[error("décodage ARGB->PNG : {0}")]
    Encoding(String),

    #[error("KWin ScreenShot2 indisponible sur cette session : {0}")]
    Unavailable(String),
}

impl From<KwinScreencapError> for PortError {
    fn from(err: KwinScreencapError) -> Self {
        match err {
            KwinScreencapError::WindowNotFound(_) => PortError::Unsupported(err.to_string()),
            KwinScreencapError::Unavailable(_)    => PortError::Unsupported(err.to_string()),
            _                                     => PortError::Render(err.to_string()),
        }
    }
}
```

Le mapping `WindowNotFound` et `Unavailable` vers `PortError::Unsupported` est cohérent avec la convention de `adapter-screencap-xcap/src/error.rs:18-19` : une absence de cible ou de support est une limite de l'adapter, pas une erreur de rendu.

#### `adapter-screencap-kwin/src/proxy.rs`

Le proxy D-Bus est déclaré via la macro `zbus::proxy`. L'interface `org.kde.KWin.ScreenShot2` est documentée dans le code source de KWin (`src/plugins/screenshot/screenshotdbusinterface2.cpp`).

```rust
use zbus::proxy;

/// Proxy D-Bus pour `org.kde.KWin.ScreenShot2`.
///
/// L'objet est exposé sur le bus de session à l'adresse
/// `org.kde.KWin` / `/org/kde/KWin/ScreenShot2`.
///
/// On n'expose ici que les deux méthodes utilisées par DevEye :
/// - `capture_window` : capture une fenêtre par son UUID interne KWin
///   (entier `u64` dans l'API D-Bus, appelé "windowId" dans la doc KWin).
/// - `find_window` (via `queryWindowInfo` KWin) : résolution du UUID à partir
///   du titre. Voir commentaire dans `renderer.rs` — on passe par une méthode
///   auxiliaire plutôt qu'un second proxy pour garder le code groupé.
#[proxy(
    interface = "org.kde.KWin.ScreenShot2",
    default_service = "org.kde.KWin",
    default_path = "/org/kde/KWin/ScreenShot2"
)]
pub trait ScreenShot2 {
    /// Capture la fenêtre identifiée par `window_id` (UUID KWin, u64).
    ///
    /// `options` est un `a{sv}` (dictionnaire variant D-Bus) ; DevEye passe
    /// `{}` (dictionnaire vide) pour obtenir le format par défaut (ARGB8888).
    ///
    /// `pipe_fd` est le côté écriture d'un pipe anonyme créé par DevEye.
    /// KWin écrit les pixels bruts dans ce fd puis le ferme. DevEye lit
    /// l'autre extrémité jusqu'à EOF pour obtenir les bytes bruts.
    ///
    /// Retourne la taille en pixels `(width, height)` de l'image capturée.
    async fn capture_window(
        &self,
        window_id: u64,
        options: std::collections::HashMap<String, zbus::zvariant::Value<'_>>,
        pipe_fd: zbus::zvariant::Fd<'_>,
    ) -> zbus::Result<(u32, u32)>;
}

/// Proxy D-Bus pour `org.kde.KWin.Script` (interface `queryWindowInfo`).
///
/// On l'utilise uniquement pour la résolution UUID : KWin ne fournit pas de
/// méthode `findWindowByTitle` directe sur ScreenShot2. La stratégie retenue
/// est d'itérer sur les clients KWin via `org.kde.KWin` / `getClients` puis
/// `queryWindowInfo` par ID jusqu'à correspondance de titre.
///
/// Alternative possible : `org.kde.KWin.Scripting` (scripting KWin en JS).
/// Non retenue : dépendance au moteur de script KWin, surface d'API instable.
#[proxy(
    interface = "org.kde.KWin",
    default_service = "org.kde.KWin",
    default_path = "/KWin"
)]
pub trait KWin {
    /// Renvoie la liste des IDs (u64) de toutes les fenêtres gérées par KWin.
    async fn get_clients(&self) -> zbus::Result<Vec<u64>>;

    /// Renvoie les métadonnées d'une fenêtre (titre, classe, géométrie…)
    /// sous forme de dictionnaire `a{sv}`.
    async fn query_window_info(
        &self,
        window_id: u64,
    ) -> zbus::Result<std::collections::HashMap<String, zbus::zvariant::OwnedValue>>;
}
```

#### `adapter-screencap-kwin/src/renderer.rs`

C'est le coeur du crate. Le flux complet est :

1. Connexion au bus de session D-Bus (une connexion partagée par `Clone`).
2. Résolution du `window_id` (UUID KWin `u64`) par sous-chaîne du titre via `KWinProxy::get_clients` + `query_window_info`.
3. Création d'un pipe anonyme POSIX (`nix::unistd::pipe2`).
4. Appel `ScreenShot2Proxy::capture_window(window_id, {}, write_fd)`.
5. Fermeture du `write_fd` côté DevEye (sinon `read_fd` ne voit jamais EOF).
6. Lecture asynchrone du `read_fd` jusqu'à EOF : bytes bruts au format ARGB8888 (32 bits par pixel, ordre B-G-R-A sur x86 little-endian).
7. Conversion ARGB -> `image::RgbaImage` (réinterprétation des canaux : BGRA -> RGBA).
8. Encodage PNG vers `Vec<u8>`.
9. Construction de `Screenshot::new_png`.

```rust
use std::collections::HashMap;
use std::io::Cursor;
use std::os::unix::io::{FromRawFd, OwnedFd};

use async_trait::async_trait;
use image::{ImageFormat, RgbaImage};
use nix::fcntl::OFlag;
use nix::unistd::pipe2;
use tokio::io::AsyncReadExt;
use zbus::Connection;

use deveye_core::{Screenshot, Size};
use deveye_ports::{PortError, PortResult, Renderer};

use crate::error::KwinScreencapError;
use crate::proxy::{KWinProxy, ScreenShot2Proxy};

/// Capture une fenêtre KDE/Wayland via `org.kde.KWin.ScreenShot2`.
///
/// Le `title_substring` est cherché de façon insensible à la casse dans la
/// clé `"caption"` des métadonnées KWin. On itère sur tous les clients KWin
/// à chaque appel (liste petite, < 50 entrées en pratique) — pas de cache
/// interne pour rester sans état et survivre aux ouvertures/fermetures de
/// fenêtres entre deux appels d'outil MCP.
#[derive(Clone, Debug)]
pub struct KwinRenderer {
    title_substring: String,
}

impl KwinRenderer {
    pub fn new(title_substring: impl Into<String>) -> Self {
        Self {
            title_substring: title_substring.into(),
        }
    }

    /// Résout le `window_id` KWin (u64) correspondant à la première fenêtre
    /// dont le titre (`"caption"`) contient `title_substring` (insensible à
    /// la casse). Retourne `KwinScreencapError::WindowNotFound` si aucune ne
    /// correspond.
    async fn find_window_id(&self, conn: &Connection) -> Result<u64, KwinScreencapError> {
        let kwin = KWinProxy::new(conn)
            .await
            .map_err(|e| KwinScreencapError::Dbus(e.to_string()))?;

        let ids = kwin
            .get_clients()
            .await
            .map_err(|e| KwinScreencapError::Dbus(format!("get_clients : {e}")))?;

        let needle = self.title_substring.to_lowercase();

        for id in ids {
            let info = kwin
                .query_window_info(id)
                .await
                .map_err(|e| KwinScreencapError::Dbus(format!("query_window_info({id}) : {e}")))?;

            if let Some(caption) = info.get("caption") {
                // OwnedValue -> &str via TryFrom.
                let title: String = caption
                    .downcast_ref::<str>()
                    .unwrap_or_default()
                    .to_string();
                if title.to_lowercase().contains(&needle) {
                    return Ok(id);
                }
            }
        }

        Err(KwinScreencapError::WindowNotFound(
            self.title_substring.clone(),
        ))
    }
}

#[async_trait]
impl Renderer for KwinRenderer {
    async fn screenshot(&self) -> PortResult<Screenshot> {
        // Connexion au bus de session D-Bus. La création par appel est légère
        // (zbus réutilise le socket de session sous-jacent via l'env
        // DBUS_SESSION_BUS_ADDRESS). Un pool de connexions n'est pas nécessaire
        // à ce stade (capture synchrone par l'outil MCP).
        let conn = Connection::session()
            .await
            .map_err(|e| PortError::Render(format!("D-Bus session : {e}")))?;

        let window_id = self
            .find_window_id(&conn)
            .await
            .map_err(PortError::from)?;

        // Pipe anonyme : KWin écrit les pixels dans write_fd, DevEye lit
        // depuis read_fd. O_CLOEXEC : pas de fuite du fd aux sous-processus
        // (règle de sécurité). O_NONBLOCK sur read_fd pour la lecture async
        // tokio ; write_fd reste bloquant (KWin écrit de façon synchrone).
        let (read_fd, write_fd) = pipe2(OFlag::O_CLOEXEC)
            .map_err(|e| PortError::Render(format!("pipe2 : {e}")))?;

        let ss2 = ScreenShot2Proxy::new(&conn)
            .await
            .map_err(|e| PortError::Render(format!("proxy ScreenShot2 : {e}")))?;

        // zbus::zvariant::Fd emprunte le RawFd ; le OwnedFd write_fd reste
        // valide pendant toute la durée de l'appel D-Bus.
        let write_raw = write_fd.as_raw_fd();
        let options: HashMap<String, zbus::zvariant::Value<'_>> = HashMap::new();

        // KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1 doit être positionné dans
        // l'environnement de KWin au démarrage de la session (voir section
        // « Sécurité dev-only / prérequis système »). Sans cette variable,
        // KWin refuse capture_window depuis un processus non privilégié.
        let (width, height) = ss2
            .capture_window(
                window_id,
                options,
                zbus::zvariant::Fd::from(write_raw),
            )
            .await
            .map_err(|e| PortError::Render(format!("CaptureWindow : {e}")))?;

        // Fermeture immédiate du write_fd côté DevEye : KWin garde sa propre
        // copie du fd jusqu'à fin d'écriture ; sans cette fermeture la lecture
        // ci-dessous ne voit jamais EOF.
        drop(write_fd);

        // Lecture asynchrone du pipe.
        // SAFETY : read_fd est un fd POSIX valide, non dupliqué ailleurs.
        let mut async_read = unsafe {
            tokio::fs::File::from_raw_fd(read_fd.into_raw_fd())
        };
        let mut raw_bytes: Vec<u8> = Vec::with_capacity((width * height * 4) as usize);
        async_read
            .read_to_end(&mut raw_bytes)
            .await
            .map_err(|e| PortError::Render(format!("lecture pipe : {e}")))?;

        // KWin produit du BGRA (little-endian ARGB32). image::RgbaImage attend
        // du RGBA. On réinterprète canal par canal : swap B<->R.
        if raw_bytes.len() != (width * height * 4) as usize {
            return Err(PortError::Render(format!(
                "taille inattendue du pipe KWin : {} octets (attendu {}x{}x4={})",
                raw_bytes.len(),
                width,
                height,
                width * height * 4
            )));
        }
        for chunk in raw_bytes.chunks_exact_mut(4) {
            chunk.swap(0, 2); // B <-> R
        }

        let img = RgbaImage::from_raw(width, height, raw_bytes).ok_or_else(|| {
            PortError::Render("RgbaImage::from_raw : dimensions incohérentes".to_string())
        })?;

        let viewport = Size::new(width, height);
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .map_err(|e| PortError::Render(format!("encodage PNG : {e}")))?;

        Ok(Screenshot::new_png(buf, viewport))
    }

    async fn viewport_size(&self) -> PortResult<Size> {
        let conn = Connection::session()
            .await
            .map_err(|e| PortError::Render(format!("D-Bus session : {e}")))?;

        let window_id = self
            .find_window_id(&conn)
            .await
            .map_err(PortError::from)?;

        let kwin = KWinProxy::new(&conn)
            .await
            .map_err(|e| PortError::Render(format!("proxy KWin : {e}")))?;

        let info = kwin
            .query_window_info(window_id)
            .await
            .map_err(|e| PortError::Render(format!("query_window_info : {e}")))?;

        // Les clés "width" et "height" sont des u32 dans le dict KWin.
        let w: u32 = info
            .get("width")
            .and_then(|v| v.downcast_ref::<u32>().ok().copied())
            .ok_or_else(|| PortError::Render("clé 'width' absente de queryWindowInfo".to_string()))?;
        let h: u32 = info
            .get("height")
            .and_then(|v| v.downcast_ref::<u32>().ok().copied())
            .ok_or_else(|| PortError::Render("clé 'height' absente de queryWindowInfo".to_string()))?;

        Ok(Size::new(w, h))
    }
}
```

### `deveye-server/src/main.rs`

Trois modifications :

**1. Import du nouveau renderer :**

```rust
// Ajouter en tête des imports adapter
use adapter_screencap_kwin::KwinRenderer;
```

**2. Logique de sélection automatique du renderer OS-level :**

La sélection est effectuée une fois au démarrage dans une fonction extraite, avant la composition du `Pilotage`. Elle consulte `XDG_SESSION_TYPE` et `WAYLAND_DISPLAY` (cohérent avec la règle CLAUDE.md : pas de nom d'app, tout configurable via CLI ou env) :

```rust
/// Sélectionne le renderer OS-level adapté à la session courante.
///
/// Règle :
/// - `XDG_SESSION_TYPE=wayland` ET `WAYLAND_DISPLAY` présent => KWin D-Bus
///   (session KDE/Wayland native, le seul contexte où ScreenShot2 fonctionne).
/// - sinon => xcap (session X11, XWayland, ou session Wayland non-KDE).
///
/// Le choix peut être forcé via `--screencap-backend kwin|xcap` (voir `Args`).
fn build_os_renderer(
    title: &str,
    backend: ScreencapBackend,
) -> Arc<dyn Renderer> {
    match backend {
        ScreencapBackend::Kwin => Arc::new(KwinRenderer::new(title)),
        ScreencapBackend::Xcap => Arc::new(XcapRenderer::new(title)),
        ScreencapBackend::Auto => {
            let is_wayland = std::env::var("XDG_SESSION_TYPE")
                .map(|v| v.to_lowercase() == "wayland")
                .unwrap_or(false);
            let has_wayland_display = std::env::var("WAYLAND_DISPLAY").is_ok();
            if is_wayland && has_wayland_display {
                eprintln!(
                    "deveye-server : session Wayland détectée — renderer KWin D-Bus (ScreenShot2)."
                );
                Arc::new(KwinRenderer::new(title))
            } else {
                eprintln!(
                    "deveye-server : session non-Wayland — renderer xcap (X11 / XWayland)."
                );
                Arc::new(XcapRenderer::new(title))
            }
        }
    }
}
```

**3. Nouvel enum et argument CLI :**

```rust
/// Backend de capture OS-level (indépendant du backend CDP).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ScreencapBackend {
    /// Détection automatique selon `XDG_SESSION_TYPE` (défaut recommandé).
    Auto,
    /// KWin D-Bus `ScreenShot2` — Wayland natif KDE uniquement.
    Kwin,
    /// xcap — X11, XWayland, ou Wayland non-KDE.
    Xcap,
}
```

Ajout dans `Args` :

```rust
/// Backend de capture OS-level pour les yeux en mode `sdk` ou repli `auto`.
/// `auto` sélectionne KWin sous Wayland KDE, xcap sinon.
#[arg(long, value_enum, default_value_t = ScreencapBackend::Auto)]
screencap_backend: ScreencapBackend,
```

L'instanciation `XcapRenderer::new(xcap_title)` aux lignes 115 et 128 de `main.rs` est remplacée par `build_os_renderer(&xcap_title, args.screencap_backend)`.

---

## Nouveaux artefacts

**Crate :** `deveye-adapters/adapter-screencap-kwin`

- Implémente `deveye-ports::Renderer` (trait de `deveye-ports/src/renderer.rs:19`).
- Dépend uniquement de `deveye-core`, `deveye-ports`, `zbus`, `nix`, `tokio`, `image`, `async-trait`, `thiserror`.
- Aucune dépendance vers un autre adapter, aucune dépendance vers une app cible.

**Dépendances workspace nouvelles :**

| Crate | Version | Usage |
|---|---|---|
| `zbus` | 4.x | Proxy D-Bus asynchrone (tokio) pour `org.kde.KWin.ScreenShot2` |
| `nix` | 0.29 | `pipe2(O_CLOEXEC)` sans `unsafe libc` direct |

**Mise à jour :**

| Crate | Avant | Après |
|---|---|---|
| `xcap` (dans `adapter-screencap-xcap/Cargo.toml`) | `0.0.14` | `0.9.6` |

**Outil MCP :** aucun nouvel outil MCP. Le changement est transparent pour le client MCP : l'outil `screenshot` existant (exposé par `adapter-mcp-server`) continue d'appeler `Renderer::screenshot()` — il ne sait pas quel adapter est derrière.

**Option CLI nouvelle :** `--screencap-backend {auto|kwin|xcap}` dans `deveye-server`.

---

## Critere de done

```bash
# 1. Compilation sans erreur ni warning clippy
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings

# 2. Tests unitaires (mapping d'erreurs, pipe size check)
cargo test -p adapter-screencap-kwin

# 3. Tests de non-régression xcap (mapping d'erreurs conservé)
cargo test -p adapter-screencap-xcap

# 4. Test d'intégration manuel (session Wayland KDE active)
# Lancer une app graphique quelconque (titre contenant "Atelier" ou configurable)
# puis :
DEVEYE_PORT=31337 cargo run -p deveye-server -- \
    --screencap-backend kwin \
    --screencap-window "atelier" \
    --serve-mcp
# => l'outil MCP `screenshot` renvoie un PNG non-noir de la fenêtre cible
# => l'outil MCP `viewport_size` renvoie des dimensions cohérentes avec la fenêtre

# 5. Test de repli auto (session X11 simulée)
XDG_SESSION_TYPE=x11 cargo run -p deveye-server -- \
    --target sdk \
    --screencap-window "atelier" \
    --serve-mcp
# => log "renderer xcap (X11 / XWayland)" au démarrage

# 6. Test de repli plein-ecran (fenetre cible introuvable par titre)
# => PortError::Unsupported retourné proprement, pas de panic

# 7. Vérification de la contrainte hexagonale
grep -r "adapter-screencap-kwin" deveye-core deveye-ports
# => aucun résultat (le crate ne remonte pas dans les couches internes)
```

Comportement observable en session de dev : après M1, `cargo run -p deveye-server -- --target sdk --serve-mcp` sur la machine Fedora 44 / Wayland sélectionne automatiquement `KwinRenderer` sans argument supplémentaire. Le screenshot retourné par l'outil MCP est pixel-perfect (même résolution que la fenêtre, pas de facteur d'échelle XWayland).

---

## Risques et pieges

**1. Variable `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1`**

Sans elle, KWin 5.27+ refuse `CaptureWindow` depuis un processus non privilégié avec `org.kde.KWin.ScreenShot2.Error.PermissionDenied`. Cette variable est une feature KWin de **développement** : elle doit être positionnée dans l'environnement de la session KDE, pas seulement dans le shell qui lance `deveye-server` (car KWin hérite de l'environnement de session, pas du shell fils). Méthode recommandée :

```
# ~/.config/plasma-workspace/env/deveye-kwin.sh
export KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1
```

Rendre exécutable, relancer la session. Vérification : `grep -r KWIN_SCREENSHOT /proc/$(pgrep kwin_wayland)/environ` doit retourner la variable.

Alternative plus propre à terme : utiliser `xdg-desktop-portal-kde` (portal `org.freedesktop.portal.Screenshot`) qui passe par une confirmation utilisateur sans variable de debug. Non retenue pour M1 car elle exige un confirmDialog à chaque capture — incompatible avec un agent autonome.

**2. Format pixel BGRA versus RGBA**

KWin `ScreenShot2` produit du `ARGB32` (little-endian, donc ordre mémoire BGRA). `image::RgbaImage` attend RGBA. L'oubli du swap `B<->R` produit un PNG avec canaux inversés (rouge et bleu échangés) sans erreur détectable au compile-time. Le test unitaire doit inclure un pixel rouge pur `[255, 0, 0, 255]` injecté en BGRA `[0, 0, 255, 255]` et vérifier le résultat PNG.

**3. EOF du pipe et double-fermeture**

Le `write_fd` doit être fermé **du côté DevEye** immédiatement après l'appel D-Bus, avant la lecture. KWin conserve sa propre copie du fd jusqu'à fin d'écriture. Si DevEye oublie de fermer `write_fd`, `read_to_end` ne termine jamais (deadlock). Pièges additionnels : `tokio::fs::File::from_raw_fd` prend possession du fd (ne pas appeler `close` manuellement après) ; `OwnedFd` de nix drop automatiquement à la fin du bloc.

**4. Règle hexagonale**

`adapter-screencap-kwin` ne doit figurer dans aucun `[dependencies]` de `deveye-core` ou `deveye-ports`. Seul `deveye-server` l'importe. Vérifiable par le `grep` de la checklist ci-dessus.

**5. Compatibilité KWin**

`org.kde.KWin.ScreenShot2` est apparu dans KWin 5.20 (Plasma 5.20, oct. 2020). Fedora 44 embarque KWin 6.x (Plasma 6). L'API est stable entre 5.20 et 6.x. En revanche, les clés retournées par `queryWindowInfo` ont changé : dans KWin 6 la clé du titre est `"caption"` (validé) ; dans des versions antérieures elle pouvait être `"title"`. Si `"caption"` est absente, loguer un warning et continuer l'itération (ne pas panic).

**6. Session non-KDE Wayland**

Sous GNOME/Wayland, `XDG_SESSION_TYPE=wayland` est vrai mais `org.kde.KWin.ScreenShot2` est absent. La détection auto (`XDG_SESSION_TYPE=wayland && WAYLAND_DISPLAY`) sélectionnerait incorrectement `KwinRenderer`, qui échouerait au premier appel avec `KwinScreencapError::Dbus`. La détection doit être enrichie : tester l'existence du service D-Bus `org.kde.KWin` (`zbus::Connection::session()` + `dbus_proxy.name_has_owner("org.kde.KWin")`) avant de sélectionner KWin. En cas d'absence, replier sur xcap avec un log explicite. Ce raffinement peut être fait dans `build_os_renderer` sans toucher à la logique de composition du `Pilotage`.

**7. Pas de repli plein-ecran implicite**

Le cahier des charges mentionne un "repli plein-écran". Ce repli doit rester **explicite** (option CLI `--screencap-backend xcap`) et ne jamais être déclenché silencieusement en cas d'erreur D-Bus : un repli silencieux vers un screenshot plein écran (incluant d'autres fenêtres) est une fuite de contexte visuel non désirée. Les erreurs `KwinScreencapError::Dbus` doivent remonter proprement au client MCP comme `PortError::Render`, pas être absorbées par un fallback interne à l'adapter.

**8. Sécurité dev-only**

`adapter-screencap-kwin` n'est pas un composant du SDK embarqué dans l'app cible — il vit uniquement dans `deveye-server`. Pas de garde `cfg(debug_assertions)` nécessaire ici. La contrainte de bind `127.0.0.1` ne le concerne pas (pas d'écoute réseau). La variable `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1` est une porte de dev ; documenter explicitement qu'elle ne doit jamais être activée sur une machine de production.

---

## Checklist d'execution

- [ ] Lire et valider les signatures réelles de `org.kde.KWin.ScreenShot2` via `busctl introspect org.kde.KWin /org/kde/KWin/ScreenShot2` sur la machine de dev (Fedora 44 / KWin 6.x) — confirmer le type exact de `window_id` (`u64` ou `u32`), le format du dict `options`, et les clés de `queryWindowInfo`.
- [ ] Ajouter `zbus = { version = "4", ... }` et `nix = { version = "0.29", ... }` dans `Cargo.toml` workspace `[workspace.dependencies]`.
- [ ] Créer le répertoire `deveye-adapters/adapter-screencap-kwin/` et son `Cargo.toml`.
- [ ] Ajouter `"deveye-adapters/adapter-screencap-kwin"` dans `Cargo.toml:members`.
- [ ] Ecrire `src/lib.rs`, `src/error.rs`, `src/proxy.rs`.
- [ ] Ecrire `src/renderer.rs` (flux pipe complet : find_window_id, pipe2, capture_window, swap BGRA->RGBA, PNG).
- [ ] Mettre à jour `xcap = "0.0.14"` -> `xcap = "0.9.6"` dans `deveye-adapters/adapter-screencap-xcap/Cargo.toml`.
- [ ] Vérifier que l'API `xcap 0.9.6` est compatible avec `adapter-screencap-xcap/src/renderer.rs` (signature de `Window::capture_image()` inchangée ou adaptée).
- [ ] Ajouter `ScreencapBackend` (enum) et `--screencap-backend` dans `deveye-server/src/main.rs:Args`.
- [ ] Extraire `build_os_renderer(title, backend)` dans `main.rs` ; remplacer les deux instanciations `XcapRenderer::new` (lignes 115 et 128) par son appel.
- [ ] Ajouter le test unitaire BGRA->RGBA dans `adapter-screencap-kwin/src/renderer.rs` (pixel rouge en entrée).
- [ ] Ajouter le test unitaire de mapping d'erreur dans `adapter-screencap-kwin/src/error.rs` (modèle : `adapter-screencap-xcap/src/error.rs:25-39`).
- [ ] Positionner `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1` dans `~/.config/plasma-workspace/env/deveye-kwin.sh` sur la machine de dev et relancer la session KDE.
- [ ] Valider `cargo check --workspace` sans erreur.
- [ ] Valider `cargo clippy --workspace --all-targets -- -D warnings` sans warning.
- [ ] Valider `cargo test --workspace`.
- [ ] Test d'intégration manuel : lancer une app cible, appeler l'outil MCP `screenshot`, vérifier que le PNG est non-noir et à la bonne résolution.
- [ ] Vérifier `grep -r "adapter-screencap-kwin" deveye-core deveye-ports` => aucun résultat.
- [ ] Commettre : `feat(adapter-screencap): implémente le renderer KWin ScreenShot2 (Wayland natif)`.
