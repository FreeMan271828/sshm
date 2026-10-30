//! sshm — terminal SSH connection manager.
//!
//! Modules:
//! - `model`: host records, sorting, filtering, validation
//! - `storage`: config dir resolution + atomic JSON persistence
//! - `app`: UI state machine (list / form / help, dialogs, key handling)
//! - `ssh`: connection testing (russh) and ssh session launching
//! - `import`: ~/.ssh/config importer
//! - `ui`: ratatui rendering (list, form, dialogs, help)

pub mod app;
pub mod import;
pub mod model;
pub mod ssh;
pub mod storage;
pub mod ui;
