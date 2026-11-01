//! sshm — terminal SSH connection manager.
//!
//! Modules:
//! - `model`: host records, sorting, filtering, validation
//! - `settings`: persisted preferences (open mode, default terminal)
//! - `storage`: config dir resolution + atomic JSON persistence
//! - `app`: UI state machine (list / form / help / settings, key handling)
//! - `ssh`: connection testing (russh) and ssh session launching
//! - `import`: ~/.ssh/config importer
//! - `ui`: ratatui rendering (list, form, dialogs, help, settings)

pub mod app;
pub mod import;
pub mod model;
pub mod settings;
pub mod ssh;
pub mod storage;
pub mod ui;
