//! Persistence layer: config dir resolution, atomic JSON load/save,
//! corruption backup, and unix file permissions.
//!
//! Every entry point takes an explicit directory (`load_from` / `save_to`)
//! so tests never touch process-global state; `load` / `save` are thin
//! wrappers around [`config_dir`].

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::model::{Host, Store};

/// On-disk envelope. `version` guards future migrations.
#[derive(Debug, Serialize, Deserialize)]
pub struct Db {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub hosts: Vec<Host>,
}

impl Db {
    pub fn from_store(store: &Store) -> Self {
        Self {
            version: 1,
            hosts: store.hosts.clone(),
        }
    }

    pub fn into_store(self) -> Store {
        Store {
            hosts: self.hosts,
            sort_mode: crate::model::SortMode::Default,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("读取配置目录失败: {0}")]
    Dir(#[source] std::io::Error),
    #[error("解析 {path} 失败: {msg}")]
    Parse { path: PathBuf, msg: String },
    #[error("写入 {path} 失败: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Process-level directory override, set once from `--config`.
/// (Rust 2024 made `std::env::set_var` unsafe, so CLI overrides go through
/// this instead of mutating the environment.)
static CONFIG_DIR_OVERRIDE: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

fn override_cell() -> &'static RwLock<Option<PathBuf>> {
    CONFIG_DIR_OVERRIDE.get_or_init(|| RwLock::new(None))
}

/// Force the config directory for the rest of the process.
pub fn set_config_dir(dir: impl Into<PathBuf>) {
    if let Ok(mut guard) = override_cell().write() {
        *guard = Some(dir.into());
    }
}

fn config_dir_override() -> Option<PathBuf> {
    override_cell().read().ok().and_then(|guard| guard.clone())
}

/// Resolve the sshm config directory.
///
/// Priority: `--config` override > `SSHM_CONFIG_DIR` env var > platform default
/// (Windows `%APPDATA%`, macOS `~/Library/Application Support`,
/// elsewhere `$XDG_CONFIG_HOME` or `~/.config`), each under `sshm`.
pub fn config_dir() -> Result<PathBuf, StorageError> {
    if let Some(dir) = config_dir_override() {
        return Ok(dir);
    }
    if let Ok(dir) = std::env::var("SSHM_CONFIG_DIR")
        && !dir.trim().is_empty()
    {
        return Ok(PathBuf::from(dir));
    }
    #[cfg(target_os = "windows")]
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("."))
        });
    #[cfg(target_os = "macos")]
    let base = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("Library")
        .join("Application Support");
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|_| PathBuf::from("."));
    Ok(base.join("sshm"))
}

pub fn hosts_path() -> Result<PathBuf, StorageError> {
    Ok(config_dir()?.join("hosts.json"))
}

/// Load the store from the resolved config dir.
pub fn load() -> Result<Store, StorageError> {
    load_from(&config_dir()?)
}

/// Save the store to the resolved config dir.
pub fn save(store: &Store) -> Result<(), StorageError> {
    save_to(&config_dir()?, store)
}

/// Load from an explicit directory. A missing file is an empty store (not an
/// error); a corrupt file is a hard error so the caller never silently
/// clobbers it.
pub fn load_from(dir: &Path) -> Result<Store, StorageError> {
    let path = dir.join("hosts.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let db: Db = serde_json::from_str(&text).map_err(|e| StorageError::Parse {
                path: path.clone(),
                msg: e.to_string(),
            })?;
            Ok(db.into_store())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::new()),
        Err(e) => Err(StorageError::Dir(e)),
    }
}

/// Save into an explicit directory, atomically: write to a temp file in the
/// same directory, fsync, then rename over the target. Before overwriting, a
/// corrupt existing file is preserved as `hosts.json.corrupt-<ts>.bak`.
pub fn save_to(dir: &Path, store: &Store) -> Result<(), StorageError> {
    std::fs::create_dir_all(dir).map_err(StorageError::Dir)?;
    let path = dir.join("hosts.json");

    // Guard: if the current file exists but can't be parsed, don't destroy it.
    if let Ok(existing) = std::fs::read_to_string(&path)
        && serde_json::from_str::<Db>(&existing).is_err()
    {
        let ts = crate::model::now_secs();
        let backup = dir.join(format!("hosts.json.corrupt-{ts}.bak"));
        let _ = std::fs::rename(&path, &backup);
    }

    let text =
        serde_json::to_string_pretty(&Db::from_store(store)).map_err(|e| StorageError::Parse {
            path: path.clone(),
            msg: e.to_string(),
        })?;

    let tmp = dir.join(format!("hosts.json.tmp-{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| StorageError::Write {
            path: tmp.clone(),
            source: e,
        })?;
        f.write_all(text.as_bytes())
            .map_err(|e| StorageError::Write {
                path: tmp.clone(),
                source: e,
            })?;
        f.write_all(b"\n").ok();
        f.sync_all().map_err(|e| StorageError::Write {
            path: tmp.clone(),
            source: e,
        })?;
    }
    restrict_permissions(&tmp);
    std::fs::rename(&tmp, &path).map_err(|e| StorageError::Write {
        path: tmp.clone(),
        source: e,
    })
}

/// Keep saved passwords out of other users' reach on unix.
#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perm = meta.permissions();
        perm.set_mode(0o600);
        let _ = std::fs::set_permissions(path, perm);
    }
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Host, SortMode, now_secs};

    fn sample_host(id: &str) -> Host {
        Host {
            id: id.into(),
            alias: String::new(),
            host: "10.0.0.1".into(),
            port: 22,
            username: "root".into(),
            password: Some("secret".into()),
            note: String::new(),
            category: String::new(),
            valid: true,
            last_used_at: None,
            use_count: 0,
            created_at: now_secs(),
            updated_at: now_secs(),
        }
    }

    #[test]
    fn roundtrip_preserves_hosts_and_password() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::new();
        store.insert(sample_host("1"));
        save_to(dir.path(), &store).unwrap();
        let loaded = load_from(dir.path()).unwrap();
        assert_eq!(loaded.hosts, store.hosts);
        assert_eq!(loaded.hosts[0].password.as_deref(), Some("secret"));
        // sort mode is not persisted (session preference) — defaults.
        assert_eq!(loaded.sort_mode, SortMode::Default);
    }

    #[test]
    fn missing_file_is_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = load_from(dir.path()).unwrap();
        assert!(store.hosts.is_empty());
    }

    #[test]
    fn corrupt_file_is_error_not_silent_clobber() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hosts.json"), "{ not json").unwrap();
        assert!(load_from(dir.path()).is_err());
        // Saving still works and backs the corrupt file up.
        let store = Store::new();
        save_to(dir.path(), &store).unwrap();
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(entries.iter().any(|n| n.starts_with("hosts.json.corrupt-")));
        assert!(load_from(dir.path()).is_ok());
    }

    #[test]
    fn config_dir_ends_with_sshm() {
        // No override set here (tests never call set_config_dir): the
        // platform default must be namespaced so we never litter the base dir.
        let dir = config_dir().unwrap();
        assert_eq!(dir.file_name().unwrap(), "sshm");
    }

    #[test]
    fn hosts_path_is_hosts_json() {
        assert_eq!(hosts_path().unwrap().file_name().unwrap(), "hosts.json");
    }
}
