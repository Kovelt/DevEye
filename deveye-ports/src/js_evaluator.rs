use async_trait::async_trait;
use serde_json::Value;

use crate::errors::PortResult;

/// Exécution d'une expression JavaScript dans le contexte de la page cible.
///
/// L'adapter principal (`adapter-cdp-webview`) traduit l'appel en
/// `Runtime.evaluate` avec `returnByValue: true` + `awaitPromise: true`, ce
/// qui rend l'API utilisable depuis des snippets `async`.
///
/// Cas d'usage typiques :
/// - lire un `getBoundingClientRect()` pour cliquer précisément ;
/// - déclencher un `.click()` directement par sélecteur (plus robuste que des
///   coordonnées pixel) ;
/// - sonder un état frontend (focus, validation de formulaire, etc.).
///
/// Volontairement permissif côté sécurité : DevEye est un outil de **dev
/// local** (cf. `cfg(debug_assertions)` côté SDK), donc l'évaluation arbitraire
/// est acceptable.
#[async_trait]
pub trait JsEvaluator: Send + Sync {
    async fn evaluate(&self, expression: &str) -> PortResult<Value>;
}
