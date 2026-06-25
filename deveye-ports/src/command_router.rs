use async_trait::async_trait;

use deveye_core::CommandDescriptor;

use crate::errors::PortResult;

/// Invocation d'une route logique côté app cible — au-delà des simples
/// interactions UI.
///
/// Exemples d'usage :
/// - `invoke("set-page", { "page": "Agenda" })` → l'app cible mute son
///   signal `Page` directement, sans passer par un clic dans la sidebar.
/// - `invoke("mock-adapter", { "port": "HolidayProvider", "fixture": "Alsace2026" })`
///   → l'app cible remplace son adapter par une fixture spécifique pour les
///   tests.
///
/// L'adapter par défaut est exposé par `deveye-sdk` côté app : chaque appel
/// à `expose_route(name, |payload| { ... })` enregistre une closure qui
/// sera appelée par cet `invoke`.
#[async_trait]
pub trait CommandRouter: Send + Sync {
    /// Invoque la commande nommée `route` avec son `payload`, et retourne la
    /// valeur JSON produite par le handler côté app.
    async fn invoke(
        &self,
        route: &str,
        payload: serde_json::Value,
    ) -> PortResult<serde_json::Value>;

    /// Liste les commandes exposées par l'app cible (découverte), triées par
    /// `name`. Liste vide si l'app n'a enregistré aucune route.
    async fn list(&self) -> PortResult<Vec<CommandDescriptor>>;
}
