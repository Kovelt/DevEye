//! Publication de l'arbre de widgets d'une app native (`deveye.ui/1`, ADR-0062).
//!
//! Une app sans DOM (egui, imgui, Skia…) décrit son interface par un
//! [`UiTree`] ; le SDK le sert sur la route de commande `ui.widgets`, que
//! `deveye-server` lit comme « DOM » des cibles sans navigateur.
//!
//! Le handler de route tourne sur le pool bloquant du SDK, jamais sur le thread
//! d'interface : l'app ne publie qu'un arbre déjà construit. Pour ne payer sa
//! construction que lorsqu'un agent regarde, elle teste [`is_observed`] :
//!
//! ```ignore
//! // à chaque frame, côté app
//! if deveye_sdk::ui::is_observed() {
//!     let mut tree = deveye_sdk::ui::builder(1280, 800).scale(1.5);
//!     // ... un widget par élément visible, rects en pixels de capture ...
//!     deveye_sdk::ui::publish(tree.build());
//! }
//! ```
//!
//! **Release : no-op.** Comme le reste du SDK, [`publish`] ne fait rien et
//! [`is_observed`] vaut toujours `false` hors `debug_assertions` : aucune route
//! n'est enregistrée et aucun port n'est ouvert (preuve :
//! `scripts/verify-sdk-release-noop.sh`).
//!
//! **Secrets.** La valeur d'un widget secret (`state.secret` ou rôle `password`)
//! n'est jamais publiée : [`publish`] la retire même pour un arbre construit à la
//! main, et les valeurs de type objet passent par le masquage par clés de
//! [`crate::Config::secret_keys`] (ADR-0064).

pub use deveye_core::ui::{
    UiIssue, UiIssueKind, UiReadError, UiRole, UiScroll, UiState, UiTree, UiTreeBuilder, UiWidget,
    UiWindow, UI_SCHEMA,
};

#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(debug_assertions)]
use std::sync::{Arc, RwLock};
#[cfg(debug_assertions)]
use std::time::{Duration, Instant};

/// Nom de la route de commande qui sert l'arbre.
pub const ROUTE: &str = "ui.widgets";

/// Capacité annoncée par `GET /devtools/info` dès que le helper est actif.
pub const CAPABILITY: &str = "ui.widgets/1";

/// Durée pendant laquelle un arbre est « observé » après la dernière lecture de
/// la route : l'app ne reconstruit son arbre que dans cette fenêtre.
#[cfg(debug_assertions)]
pub const OBSERVE_WINDOW: Duration = Duration::from_secs(2);

/// Attente maximale d'une première publication quand personne n'observait : une
/// app qui ne publie que sous `is_observed` a besoin d'une frame pour répondre.
#[cfg(debug_assertions)]
const FRESH_PUBLISH_WAIT: Duration = Duration::from_millis(500);

#[cfg(debug_assertions)]
const POLL_STEP: Duration = Duration::from_millis(10);

/// Constructeur d'arbre pour une fenêtre de `width` × `height` pixels.
pub fn builder(width: u32, height: u32) -> UiTreeBuilder {
    UiTree::builder(width, height)
}

/// Dernier arbre publié et état d'observation, partagés avec la route.
#[cfg(debug_assertions)]
pub(crate) struct UiSlot {
    tree: RwLock<Option<Arc<UiTree>>>,
    /// La route est enregistrée (le helper est utilisé par l'app).
    enabled: AtomicBool,
    publications: AtomicU64,
    /// Instant de la dernière lecture, en ms depuis `epoch` + 1 (0 = jamais).
    last_read: AtomicU64,
    epoch: Instant,
}

#[cfg(debug_assertions)]
impl Default for UiSlot {
    fn default() -> Self {
        Self {
            tree: RwLock::new(None),
            enabled: AtomicBool::new(false),
            publications: AtomicU64::new(0),
            last_read: AtomicU64::new(0),
            epoch: Instant::now(),
        }
    }
}

#[cfg(debug_assertions)]
impl UiSlot {
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64 + 1
    }

    fn observed(&self) -> bool {
        let last = self.last_read.load(Ordering::Acquire);
        last != 0 && self.now().saturating_sub(last) <= OBSERVE_WINDOW.as_millis() as u64
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }
}

/// Version release : aucun état, aucun code de serveur.
#[cfg(not(debug_assertions))]
#[derive(Default)]
pub(crate) struct UiSlot;

/// Enregistre la route au premier usage du helper. Appelée aussi par
/// [`is_observed`] : l'app n'attend pas d'avoir publié pour pouvoir être
/// observée, sinon personne ne demanderait jamais le premier arbre.
#[cfg(debug_assertions)]
fn ensure_route() {
    let slot = &crate::REGISTRY.ui;
    if slot.enabled.swap(true, Ordering::AcqRel) {
        return;
    }
    crate::expose_route(
        ROUTE,
        "Arbre de widgets de l'app (schéma deveye.ui/1)",
        serve,
    );
}

/// Handler de la route : le payload est ignoré (ADR-0065).
#[cfg(debug_assertions)]
fn serve(_payload: serde_json::Value) -> serde_json::Value {
    let slot = &crate::REGISTRY.ui;
    let was_observed = slot.observed();
    slot.last_read.store(slot.now(), Ordering::Release);
    if !was_observed {
        let before = slot.publications.load(Ordering::Acquire);
        let deadline = Instant::now() + FRESH_PUBLISH_WAIT;
        while slot.publications.load(Ordering::Acquire) == before && Instant::now() < deadline {
            std::thread::sleep(POLL_STEP);
        }
    }
    let tree = slot
        .tree
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let Some(tree) = tree else {
        return serde_json::json!({
            "error": "aucun arbre publié : l'app n'a pas encore appelé deveye_sdk::ui::publish"
        });
    };
    let mut value = match serde_json::to_value(&*tree) {
        Ok(value) => value,
        Err(e) => return serde_json::json!({ "error": format!("arbre non sérialisable : {e}") }),
    };
    mask_values_by_key(&mut value);
    value
}

/// Masque par clés les valeurs structurées des widgets (seconde ligne après le
/// retrait des widgets secrets).
#[cfg(debug_assertions)]
fn mask_values_by_key(tree: &mut serde_json::Value) {
    let extra = crate::REGISTRY
        .secret_keys
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(widgets) = tree.get_mut("widgets").and_then(|w| w.as_array_mut()) else {
        return;
    };
    for widget in widgets {
        if let Some(value) = widget.get_mut("value") {
            crate::redact::redact(value, &extra);
        }
    }
}

/// Publie `tree` comme arbre courant de l'app. Remplace le précédent ; le
/// premier appel enregistre la route `ui.widgets`.
///
/// Retire la valeur des widgets secrets, horodate l'arbre s'il ne l'est pas
/// (`published_at_ms`) et numérote `generation` s'il vaut 0. No-op en release.
pub fn publish(tree: UiTree) {
    #[cfg(not(debug_assertions))]
    let _ = tree;

    #[cfg(debug_assertions)]
    {
        ensure_route();
        let slot = &crate::REGISTRY.ui;
        let mut tree = tree;
        tree.strip_secret_values();
        if tree.published_at_ms.is_none() {
            tree.published_at_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_millis() as u64);
        }
        if tree.generation == 0 {
            tree.generation = slot.publications.load(Ordering::Acquire) + 1;
        }
        *slot
            .tree
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(tree));
        slot.publications.fetch_add(1, Ordering::AcqRel);
    }
}

/// Vrai si la route `ui.widgets` a été lue dans les [`OBSERVE_WINDOW`]
/// dernières secondes : l'app ne construit son arbre que dans ce cas. Toujours
/// `false` en release. Le premier appel enregistre la route.
#[cfg(debug_assertions)]
pub fn is_observed() -> bool {
    ensure_route();
    crate::REGISTRY.ui.observed()
}

/// Vrai si la route `ui.widgets` a été lue récemment. Toujours `false` en
/// release : aucune route n'existe.
#[cfg(not(debug_assertions))]
#[inline(always)]
pub const fn is_observed() -> bool {
    false
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;
    use serde_json::json;

    // Les tests partagent le registre global : un seul test sériel.
    #[test]
    fn publish_serve_and_observe() {
        assert!(!is_observed());
        // Route enregistrée, rien publié : erreur explicite après l'attente.
        let empty = serve(json!({}));
        assert!(empty["error"].is_string(), "{empty}");
        assert!(is_observed());

        let tree = builder(800, 600)
            .scale(2.0)
            .widget(
                UiWidget::new("login", UiRole::TEXTBOX)
                    .with_rect([10, 10, 200, 30])
                    .with_value("alice"),
            )
            .widget(
                UiWidget::new("pw", UiRole::PASSWORD)
                    .with_rect([10, 50, 200, 30])
                    .with_value("hunter2"),
            )
            .widget(
                UiWidget::new("meta", UiRole::LABEL)
                    .with_rect([0, 0, 10, 10])
                    .with_value(json!({ "user": "alice", "api_key": "k" })),
            )
            .build();
        // Arbre construit sans le builder : publish retire quand même le secret.
        let mut raw = tree.clone();
        raw.widgets[1].value = Some(json!("hunter2"));
        publish(raw);

        let value = serve(json!({}));
        assert_eq!(value["schema"], UI_SCHEMA);
        assert!(value["published_at_ms"].is_u64());
        assert_eq!(value["generation"], 1);
        assert_eq!(value["widgets"][0]["value"], "alice");
        assert!(value["widgets"][1].get("value").is_none(), "{value}");
        assert_eq!(value["widgets"][2]["value"]["user"], "alice");
        assert_eq!(value["widgets"][2]["value"]["api_key"], crate::REDACTED);
        let parsed = UiTree::from_value(&value).expect("arbre relu");
        assert!(parsed.validate().iter().all(|i| !i.is_error()));
        assert!(crate::REGISTRY.ui.is_enabled());
    }
}
