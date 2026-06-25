//! DevEye core — domaine pur de l'outillage de dev.
//!
//! Ce crate isole le métier de DevEye (observer et piloter une app vivante)
//! de toute dépendance technique. Les seules crates utilisées sont `chrono`,
//! `serde`, `serde_json`, `thiserror`.
//!
//! Voir [`deveye.md`](../../../../../docs/architecture/deveye.md) (côté
//! Chronos) pour le contexte stratégique et le plan d'implémentation.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod command;
pub mod log_event;
pub mod session;
pub mod snapshot;
pub mod window_info;

pub use command::{CommandDescriptor, InteractionCommand, MouseButton};
pub use log_event::{LogEvent, LogLevel};
pub use session::{AppKind, AppSession};
pub use snapshot::{DomTree, Position, Screenshot, Size, StateDump};
pub use window_info::WindowInfo;
