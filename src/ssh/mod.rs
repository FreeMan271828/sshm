//! SSH layer: async connection probing (russh) and launching sessions via the
//! system `ssh` client.
//!
//! Public API contract (frozen by Lead — change only after syncing with
//! `src/app.rs` and `src/main.rs`):
//! - `ProbeTarget` / `TestOutcome` / `Stage` / `AuthKind`
//! - `spawn_probe(target) -> mpsc::Receiver<TestOutcome>`
//! - `run_ssh(host) -> io::Result<ExitStatus>` (current terminal)
//! - `open_in_new_terminal(host, terminal) -> io::Result<&'static str>`
//! - `cleanup_temp_files()`
//! - `ssh_command_string(host) -> String`
//!
//! Layout: [`probe`] owns the connection-test pipeline (worker thread, tokio
//! runtime, stage classification, timeout, live tests); [`launch`] owns the
//! interactive session launch (current tab, new tab/window, askpass cleanup).

mod launch;
mod probe;

use std::io;
use std::sync::mpsc;

use crate::model::Host;
use crate::settings::Terminal;

/// What to test a connection against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeTarget {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// `None` = no password saved (key/agent only).
    pub password: Option<String>,
}

/// Which phase of a connection attempt failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Dns,
    Tcp,
    Handshake,
    Auth,
    Timeout,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Dns => "DNS 解析",
            Stage::Tcp => "TCP 连接",
            Stage::Handshake => "SSH 握手",
            Stage::Auth => "认证",
            Stage::Timeout => "超时",
        }
    }
}

/// How authentication succeeded (when the probe reached that far).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    /// Password auth accepted.
    Password,
    /// `none` auth accepted (server allows it).
    NoneAuth,
    /// Handshake succeeded but no password was saved and `none` was rejected;
    /// the user presumably authenticates with keys/agent.
    HandshakeOnly,
}

impl AuthKind {
    pub fn label(self) -> &'static str {
        match self {
            AuthKind::Password => "密码认证成功",
            AuthKind::NoneAuth => "免密(none)成功",
            AuthKind::HandshakeOnly => "握手成功(需密钥/交互认证)",
        }
    }
}

/// Result of a connection test.
#[derive(Debug, Clone)]
pub enum TestOutcome {
    Ok {
        banner: String,
        latency_ms: u64,
        auth: AuthKind,
    },
    Err {
        stage: Stage,
        message: String,
    },
}

impl TestOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, TestOutcome::Ok { .. })
    }

    /// One-line human summary, e.g. `✓ 密码认证成功 · 42ms` or
    /// `✗ TCP 连接: 连接被拒绝`.
    pub fn summary(&self) -> String {
        match self {
            TestOutcome::Ok {
                auth, latency_ms, ..
            } => {
                format!("✓ {} · {}ms", auth.label(), latency_ms)
            }
            TestOutcome::Err { stage, message } => {
                format!("✗ {}: {}", stage.label(), message)
            }
        }
    }

    /// Extra detail line for the UI (server banner / error hint), if any.
    pub fn detail(&self) -> Option<String> {
        match self {
            TestOutcome::Ok { banner, .. } => {
                let b = banner.trim();
                if b.is_empty() || b.len() > 120 {
                    None
                } else {
                    Some(format!("服务器: {}", b))
                }
            }
            TestOutcome::Err { .. } => None,
        }
    }
}

/// Kick off a connection probe on a worker thread with its own tokio runtime.
/// Results arrive on the returned receiver; the caller polls with `try_recv`.
pub fn spawn_probe(target: ProbeTarget) -> mpsc::Receiver<TestOutcome> {
    probe::spawn_probe(target)
}

/// Launch a real interactive ssh session via the system `ssh` binary.
/// The caller (main.rs) has already suspended the TUI terminal.
pub fn run_ssh(host: &Host) -> io::Result<std::process::ExitStatus> {
    launch::run_ssh(host)
}

/// Open an interactive ssh session in a new terminal tab/window. The TUI keeps
/// running; the returned label names the terminal that was launched (e.g.
/// `"Windows Terminal"`).
pub fn open_in_new_terminal(host: &Host, terminal: Terminal) -> io::Result<&'static str> {
    launch::open_in_new_terminal(host, terminal)
}

/// Delete helper scripts kept alive for the process (askpass, Tabby run
/// scripts). Call once on exit.
pub fn cleanup_temp_files() {
    launch::cleanup_temp_files()
}

/// `user@host`, or bare `host` when no username is configured.
fn destination(host: &Host) -> String {
    let user = host.username.trim();
    let host = host.host.trim();
    if user.is_empty() {
        host.to_string()
    } else {
        format!("{user}@{host}")
    }
}

/// The shell command this host maps to, for display/tests, e.g.
/// `ssh -p 2222 root@10.0.0.5`, or `ssh host` on the default port with no
/// username.
pub fn ssh_command_string(host: &Host) -> String {
    let mut command = String::from("ssh");
    if host.port != 22 {
        command.push_str(&format!(" -p {}", host.port));
    }
    command.push(' ');
    command.push_str(&destination(host));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_host(host: &str, port: u16, username: &str) -> Host {
        Host {
            id: "test".into(),
            alias: String::new(),
            host: host.into(),
            port,
            username: username.into(),
            password: None,
            note: String::new(),
            category: String::new(),
            valid: true,
            last_used_at: None,
            use_count: 0,
            created_at: 0,
            updated_at: 0,
        }
    }

    // --- ssh_command_string: the four combos --------------------------------

    #[test]
    fn command_user_and_non_default_port() {
        let h = sample_host("10.0.0.5", 2222, "root");
        assert_eq!(ssh_command_string(&h), "ssh -p 2222 root@10.0.0.5");
    }

    #[test]
    fn command_user_and_default_port() {
        let h = sample_host("10.0.0.1", 22, "root");
        assert_eq!(ssh_command_string(&h), "ssh root@10.0.0.1");
    }

    #[test]
    fn command_no_user_and_default_port() {
        let h = sample_host("10.0.0.1", 22, "");
        assert_eq!(ssh_command_string(&h), "ssh 10.0.0.1");
    }

    #[test]
    fn command_no_user_and_non_default_port() {
        let h = sample_host("10.0.0.1", 22022, "");
        assert_eq!(ssh_command_string(&h), "ssh -p 22022 10.0.0.1");
    }

    // --- TestOutcome contract ----------------------------------------------

    #[test]
    fn summary_formats_ok_outcome() {
        let outcome = TestOutcome::Ok {
            banner: "SSH-2.0-OpenSSH_9.6".into(),
            latency_ms: 42,
            auth: AuthKind::Password,
        };
        assert_eq!(outcome.summary(), "✓ 密码认证成功 · 42ms");
        assert_eq!(
            outcome.detail().as_deref(),
            Some("服务器: SSH-2.0-OpenSSH_9.6")
        );
        assert!(outcome.is_ok());
    }

    #[test]
    fn summary_formats_err_outcome() {
        let outcome = TestOutcome::Err {
            stage: Stage::Tcp,
            message: "连接被拒绝".into(),
        };
        assert_eq!(outcome.summary(), "✗ TCP 连接: 连接被拒绝");
        assert_eq!(outcome.detail(), None);
        assert!(!outcome.is_ok());
    }

    #[test]
    fn detail_hides_empty_and_oversized_banner() {
        let empty = TestOutcome::Ok {
            banner: "   ".into(),
            latency_ms: 1,
            auth: AuthKind::NoneAuth,
        };
        assert_eq!(empty.detail(), None);

        let huge = TestOutcome::Ok {
            banner: "x".repeat(121),
            latency_ms: 1,
            auth: AuthKind::HandshakeOnly,
        };
        assert_eq!(huge.detail(), None);
    }

    #[test]
    fn stage_and_auth_labels_are_short() {
        for stage in [
            Stage::Dns,
            Stage::Tcp,
            Stage::Handshake,
            Stage::Auth,
            Stage::Timeout,
        ] {
            assert!(!stage.label().is_empty());
        }
        for auth in [
            AuthKind::Password,
            AuthKind::NoneAuth,
            AuthKind::HandshakeOnly,
        ] {
            assert!(!auth.label().is_empty());
        }
    }
}
