//! DevEye core — domaine pur de l'outillage de dev.
//!
//! Ce crate isole le métier de DevEye (observer et piloter une app vivante)
//! de toute dépendance technique. Les seules crates utilisées sont `chrono`,
//! `serde`, `serde_json`, `thiserror`.
//!
//! Le contexte stratégique et le plan d'implémentation de DevEye vivent dans le
//! repo Chronos (`crates/deveye`), source canonique dont ce dépôt est l'export
//! miroir (`git subtree split`).

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod command;
pub mod console_error;
pub mod log_event;
pub mod session;
pub mod snapshot;
pub mod wait_condition;
pub mod window_info;

pub use command::{CommandDescriptor, InteractionCommand, MouseButton};
pub use console_error::{ConsoleError, ConsoleLevel};
pub use log_event::{LogEvent, LogLevel};
pub use session::{AppKind, AppSession};
pub use snapshot::{DomTree, Position, Screenshot, Size, StateDump};
pub use wait_condition::{WaitOutcome, WaitPredicate};
pub use window_info::WindowInfo;
