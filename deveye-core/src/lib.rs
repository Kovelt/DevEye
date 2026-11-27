//! DevEye core — domaine pur de l'outillage de dev.
//!
//! Ce crate isole le métier de DevEye (observer et piloter une app vivante)
//! de toute dépendance technique. Les seules crates utilisées sont `chrono`,
//! `serde`, `serde_json`, `thiserror` (et `schemars`, derrière la feature
//! optionnelle `schema`, désactivée par défaut).
//!
//! L'architecture d'ensemble est décrite dans `docs/architecture.md` du dépôt.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod command;
pub mod console_error;
pub mod log_event;
pub mod snapshot;
pub mod ui;
pub mod wait_condition;
pub mod window_info;
pub mod window_rect;

pub use command::{CommandDescriptor, InteractionCommand, MouseButton};
pub use console_error::{ConsoleError, ConsoleLevel};
pub use log_event::{LogEvent, LogLevel};
pub use snapshot::{DomTree, Position, Screenshot, Size, StateDump};
pub use ui::{
    UiIssue, UiIssueKind, UiReadError, UiRole, UiScroll, UiState, UiTree, UiTreeBuilder, UiWidget,
    UiWindow, UI_SCHEMA,
};
pub use wait_condition::{WaitOutcome, WaitPredicate};
pub use window_info::WindowInfo;
pub use window_rect::WindowRect;
