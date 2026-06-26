//! Ports DevEye — traits asynchrones consommés par le binaire orchestrateur
//! et le serveur MCP, implémentés par les crates `deveye-adapters/*`.
//!
//! 9 ports définis :
//! - [`CommandRouter`] — envoi de commandes à l'app cible
//! - [`ConsoleErrorSource`] — drain des erreurs console webview
//! - [`AppInspector`] — inspection de l'arbre de composants
//! - [`InteractionSink`] — injection d'événements clavier/souris
//! - [`JsEvaluator`] — évaluation JS dans le webview
//! - [`LogSource`] — collecte des logs applicatifs
//! - [`Renderer`] — capture d'écran / rendu visuel
//! - [`WaitCondition`] — attente de conditions asynchrones
//! - [`WindowLister`] — énumération des fenêtres de l'app cible
//!
//! Voir [`deveye.md`](../../../../../docs/architecture/deveye.md) pour le
//! détail de l'architecture.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod command_router;
pub mod console_error_source;
pub mod errors;
pub mod inspector;
pub mod interaction;
pub mod js_evaluator;
pub mod log_source;
pub mod renderer;
pub mod wait_condition;
pub mod window_lister;

pub use command_router::CommandRouter;
pub use console_error_source::ConsoleErrorSource;
pub use errors::{PortError, PortResult};
pub use inspector::AppInspector;
pub use interaction::InteractionSink;
pub use js_evaluator::JsEvaluator;
pub use log_source::LogSource;
pub use renderer::Renderer;
pub use wait_condition::WaitCondition;
pub use window_lister::WindowLister;
