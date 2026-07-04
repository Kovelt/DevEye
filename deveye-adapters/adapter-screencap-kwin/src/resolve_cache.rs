//! Cache de résolution "sous-chaîne de titre -> (UUID KWin, géométrie écran)",
//! process-wide (via `static`).
//!
//! [`crate::KwinRenderer`] LIT et ÉCRIT ce cache (chemin chaud : les captures
//! répétées d'une boucle d'attente résolvent la même fenêtre ; l'échec de
//! `CaptureWindow` sur un UUID mort auto-invalide l'entrée). [`crate::KwinWindowGeometry`]
//! ÉCRIT seulement (il résout toujours frais — ses coordonnées servent à
//! décider côté agent, une entrée périmée y serait un bug silencieux) mais
//! peuple le cache pour qu'un `screenshot` proche en profite. Chacun résout via
//! son propre script KWin éphémère ; le partage se fait ici, la génération du
//! JS et l'interprétation du résultat restent spécifiques (cf. [`crate::kwin_bridge`]).
//!
//! [`crate::KwinWindowManager::activate`] n'y participe volontairement PAS en
//! lecture : son script combine résolution ET action (rendre la fenêtre
//! active), donc consulter le cache reviendrait à parfois ne plus l'activer.
//! Il invalide l'entrée après une activation réussie (une fenêtre qui vient
//! d'être activée/déminimisée peut avoir changé de position).
//!
//! Un aller-retour D-Bus KWin (`loadScript` + `start` + attente callback +
//! `unloadScript`, cf. [`crate::kwin_bridge::run_kwin_script`]) coûte
//! plusieurs millisecondes ; une fenêtre cible ne change quasiment jamais en
//! cours de session MCP, d'où un TTL court plutôt qu'une invalidation
//! uniquement événementielle (KWin ne pousse aucun signal de changement ici).

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use deveye_core::WindowRect;

/// Durée de vie d'une entrée de cache. Volontairement TRÈS court : le cache ne
/// sert qu'à fusionner les résolutions redondantes d'une rafale (une boucle
/// `wait_screenshot_region` re-capture toutes les ~200 ms → même fenêtre
/// résolue en boucle). 500 ms borne la fenêtre pendant laquelle une donnée
/// périmée pourrait être servie (fenêtre fermée-rouverte au même titre, ou
/// déplacée). Le chemin `screenshot` s'auto-guérit de toute façon (échec de
/// `CaptureWindow` sur un UUID mort → invalidation) ; les consommateurs de
/// coordonnées que l'agent exploite pour décider (`window_geometry`) ne lisent
/// PAS le cache (résolution toujours fraîche, cf. `KwinWindowGeometry`).
pub(crate) const RESOLVE_CACHE_TTL: Duration = Duration::from_millis(500);

/// Résolution mise en cache pour une sous-chaîne de titre donnée.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedWindow {
    /// `internalId` KWin (utilisé par `ScreenShot2.CaptureWindow`).
    pub uuid: String,
    /// `frameGeometry` complète (position + taille écran).
    pub rect: WindowRect,
}

struct Entry {
    value: ResolvedWindow,
    at: Instant,
}

/// Clé = sous-chaîne de titre normalisée (minuscules), cohérent avec la
/// recherche insensible à la casse faite côté script KWin.
static CACHE: LazyLock<Mutex<HashMap<String, Entry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn normalize(title_substring: &str) -> String {
    title_substring.to_lowercase()
}

/// Renvoie l'entrée en cache pour `title_substring` si présente et encore
/// fraîche (< [`RESOLVE_CACHE_TTL`]). Le verrou n'est tenu que le temps du
/// lookup dans la `HashMap` — jamais à travers un `await` D-Bus (cf. piège R1).
pub(crate) async fn get(title_substring: &str) -> Option<ResolvedWindow> {
    let key = normalize(title_substring);
    let guard = CACHE.lock().await;
    let entry = guard.get(&key)?;
    if entry.at.elapsed() > RESOLVE_CACHE_TTL {
        return None;
    }
    Some(entry.value.clone())
}

/// Enregistre/remplace l'entrée pour `title_substring`.
pub(crate) async fn put(title_substring: &str, value: ResolvedWindow) {
    let key = normalize(title_substring);
    CACHE.lock().await.insert(
        key,
        Entry {
            value,
            at: Instant::now(),
        },
    );
}

/// Purge l'entrée pour `title_substring` (résolution en échec — WindowNotFound
/// / erreur D-Bus —, ou UUID caché devenu invalide en aval, ex. `CaptureWindow`
/// qui échoue sur une fenêtre entre-temps fermée).
pub(crate) async fn invalidate(title_substring: &str) {
    let key = normalize(title_substring);
    CACHE.lock().await.remove(&key);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> WindowRect {
        WindowRect::new(1, 2, 3, 4)
    }

    #[tokio::test]
    async fn put_then_get_hits_within_ttl() {
        let key = "resolve-cache-test-hit";
        put(
            key,
            ResolvedWindow {
                uuid: "u1".into(),
                rect: rect(),
            },
        )
        .await;
        let got = get(key).await.expect("entree presente juste apres put");
        assert_eq!(got.uuid, "u1");
        assert_eq!(got.rect, rect());
        invalidate(key).await;
    }

    #[tokio::test]
    async fn invalidate_removes_entry() {
        let key = "resolve-cache-test-invalidate";
        put(
            key,
            ResolvedWindow {
                uuid: "u2".into(),
                rect: rect(),
            },
        )
        .await;
        invalidate(key).await;
        assert!(get(key).await.is_none());
    }

    #[tokio::test]
    async fn get_absent_key_is_none() {
        assert!(get("resolve-cache-test-never-put").await.is_none());
    }

    #[tokio::test]
    async fn key_lookup_is_case_insensitive() {
        let key = "Resolve-Cache-Test-Case";
        put(
            key,
            ResolvedWindow {
                uuid: "u3".into(),
                rect: rect(),
            },
        )
        .await;
        assert!(get("resolve-cache-test-case").await.is_some());
        invalidate(key).await;
    }
}
