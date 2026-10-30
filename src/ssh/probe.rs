//! Connection probe: DNS + TCP + SSH handshake + auth, on a worker thread
//! with its own tokio runtime.
//!
//! Stage separation: the TCP connect is done explicitly with
//! `tokio::net::TcpStream::connect`, so io errors there (DNS lookup,
//! connection refused, timeout) can be classified precisely by
//! `classify_connect_error`. Anything that fails after a TCP connection was
//! established — `connect_stream` key exchange, or transport errors during
//! auth — is a `Stage::Handshake` failure. The original plan of using
//! `russh::client::connect` alone was dropped because it conflates both
//! layers: the same fake-server case surfaces as `IO(reset)`, `IO(eof)` or
//! `Disconnect` depending on timing, which is unclassifiable; with the split,
//! every connect_stream error is deterministically a handshake failure.

use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use russh::Disconnect;
use russh::client::{self, Config, Handler};
use russh::keys::PublicKeyOrCertificate;
use tokio::net::TcpStream;

use super::{AuthKind, ProbeTarget, Stage, TestOutcome};

/// Total budget for one probe: DNS + TCP + handshake + auth.
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// `TestOutcome::detail()` drops banners longer than 120 bytes; leave headroom.
const BANNER_MAX: usize = 112;
/// Raw protocol errors are long and the UI renders one line; cap the message.
const MESSAGE_MAX: usize = 200;

/// Classify a TCP-connect io error message into a probe stage.
///
/// Pure function so the mapping is unit-testable. Inputs come from
/// `std::io::Error::to_string()`, whose keywords are stable across platforms
/// (Windows: "failed to lookup address information", "actively refused it";
/// Unix: "Connection refused", "timed out").
pub(crate) fn classify_connect_error(msg: &str) -> Stage {
    let m = msg.to_lowercase();
    if m.contains("lookup") || m.contains("resolve") {
        Stage::Dns
    } else if m.contains("timed out") || m.contains("timeout") {
        Stage::Timeout
    } else {
        // Refused, unreachable, reset, ... — all TCP-layer failures.
        Stage::Tcp
    }
}

/// Facts collected from the server while the session runs.
#[derive(Debug, Clone, Default)]
struct CapturedInfo {
    /// Server identification string, e.g. `SSH-2.0-OpenSSH_9.6` (from kex).
    server_id: Option<String>,
    /// Pre-auth banner the server sent (may never arrive).
    auth_banner: Option<String>,
}

/// russh client handler for probes.
///
/// `check_server_key` unconditionally accepts the server key: a connection
/// manager trades MITM protection for convenience (the real `ssh` binary,
/// with its known_hosts checking, is the strict path). `kex_done` and
/// `auth_banner` capture display facts; russh moves the handler into its
/// session task, so the shared `info` cell carries them back out.
struct ProbeClient {
    info: Arc<Mutex<CapturedInfo>>,
}

impl Handler for ProbeClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn kex_done(
        &mut self,
        _shared_secret: Option<&[u8]>,
        _names: &russh::Names,
        session: &mut russh::client::Session,
    ) -> Result<(), Self::Error> {
        let id = String::from_utf8_lossy(session.remote_sshid())
            .trim()
            .to_string();
        if !id.is_empty()
            && let Ok(mut info) = self.info.lock()
        {
            info.server_id = Some(id);
        }
        Ok(())
    }

    async fn auth_banner(
        &mut self,
        banner: &str,
        _session: &mut russh::client::Session,
    ) -> Result<(), Self::Error> {
        if let Ok(mut info) = self.info.lock() {
            info.auth_banner = Some(banner.to_string());
        }
        Ok(())
    }
}

/// Kick off a probe on a dedicated thread and return its result channel.
pub(crate) fn spawn_probe(target: ProbeTarget) -> mpsc::Receiver<TestOutcome> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("sshm-probe".into())
        .spawn(move || {
            // A probe must always deliver exactly one outcome, even on panic.
            let outcome =
                std::panic::catch_unwind(move || run_probe_sync(target)).unwrap_or_else(|_| {
                    TestOutcome::Err {
                        stage: Stage::Handshake,
                        message: "探针线程内部错误".to_string(),
                    }
                });
            let _ = tx.send(outcome);
        })
        .expect("failed to spawn ssh probe thread");
    rx
}

/// Build a single-threaded runtime and block on the async probe.
fn run_probe_sync(target: ProbeTarget) -> TestOutcome {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            return fail(Stage::Handshake, format!("探测线程启动失败: {e}"));
        }
    };
    rt.block_on(probe_async(target))
}

/// The probe pipeline under one overall deadline.
async fn probe_async(target: ProbeTarget) -> TestOutcome {
    match tokio::time::timeout(PROBE_TIMEOUT, probe_connect_and_auth(&target)).await {
        Ok(outcome) => outcome,
        Err(_) => fail(
            Stage::Timeout,
            format!("{} 秒内未完成", PROBE_TIMEOUT.as_secs()),
        ),
    }
}

async fn probe_connect_and_auth(target: &ProbeTarget) -> TestOutcome {
    let started = Instant::now();

    // Step 1: TCP connect (DNS resolution happens inside).
    let stream = match TcpStream::connect((target.host.as_str(), target.port)).await {
        Ok(stream) => stream,
        Err(e) => {
            let message = e.to_string();
            return fail(classify_connect_error(&message), message);
        }
    };

    // Step 2: SSH handshake / key exchange on the established socket.
    let config = Arc::new(Config::default());
    let info = Arc::new(Mutex::new(CapturedInfo::default()));
    let mut handle = match client::connect_stream(
        config,
        stream,
        ProbeClient {
            info: Arc::clone(&info),
        },
    )
    .await
    {
        Ok(handle) => handle,
        Err(e) => return fail(Stage::Handshake, format!("握手失败: {e}")),
    };

    // Step 3: authentication.
    let auth_outcome: Result<AuthKind, TestOutcome> = match &target.password {
        Some(password) => match handle
            .authenticate_password(target.username.as_str(), password.as_str())
            .await
        {
            Ok(result) if result.success() => Ok(AuthKind::Password),
            Ok(_) => Err(fail(Stage::Auth, "服务器拒绝密码认证")),
            Err(e) => Err(fail(Stage::Handshake, format!("认证请求失败: {e}"))),
        },
        None => match handle.authenticate_none(target.username.as_str()).await {
            Ok(result) if result.success() => Ok(AuthKind::NoneAuth),
            // Server rejected `none`: handshake worked and the user presumably
            // has keys/agent, so this is still a successful probe.
            Ok(_) => Ok(AuthKind::HandshakeOnly),
            Err(e) => Err(fail(Stage::Handshake, format!("认证请求失败: {e}"))),
        },
    };

    // Step 4: always tear the session down, on every path — including auth
    // failure, so no server is left with a half-open russh session.
    let _ = handle
        .disconnect(Disconnect::ByApplication, "probe complete", "")
        .await;

    match auth_outcome {
        Ok(auth) => TestOutcome::Ok {
            banner: banner_from(&info, target),
            latency_ms: started.elapsed().as_millis() as u64,
            auth,
        },
        Err(outcome) => outcome,
    }
}

/// Prefer the server's version string, then its auth banner, then synthesize.
fn banner_from(info: &Arc<Mutex<CapturedInfo>>, target: &ProbeTarget) -> String {
    let captured = info.lock().map(|g| g.clone()).unwrap_or_default();
    if let Some(id) = captured.server_id.as_deref().filter(|s| !s.is_empty()) {
        return truncate_chars(id, BANNER_MAX);
    }
    if let Some(banner) = captured
        .auth_banner
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return truncate_chars(banner, BANNER_MAX);
    }
    format!("{}:{} 握手成功", target.host, target.port)
}

fn fail(stage: Stage, message: impl Into<String>) -> TestOutcome {
    TestOutcome::Err {
        stage,
        message: one_line(&message.into()),
    }
}

/// Collapse newlines/spaces and cap the length.
fn one_line(text: &str) -> String {
    truncate_chars(
        &text.split_whitespace().collect::<Vec<_>>().join(" "),
        MESSAGE_MAX,
    )
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    fn target(host: &str, port: u16) -> ProbeTarget {
        ProbeTarget {
            host: host.into(),
            port,
            username: "root".into(),
            password: None,
        }
    }

    #[test]
    fn classify_connect_error_maps_stages() {
        assert_eq!(
            classify_connect_error(
                "failed to lookup address information: No such host is known. (os error 11001)"
            ),
            Stage::Dns
        );
        assert_eq!(
            classify_connect_error("could not resolve hostname"),
            Stage::Dns
        );
        assert_eq!(
            classify_connect_error("Connection refused (os error 61)"),
            Stage::Tcp
        );
        assert_eq!(
            classify_connect_error(
                "No connection could be made because the target machine actively refused it. (os error 10061)"
            ),
            Stage::Tcp
        );
        assert_eq!(
            classify_connect_error("connection timed out"),
            Stage::Timeout
        );
        assert_eq!(classify_connect_error("Network is unreachable"), Stage::Tcp);
    }

    #[test]
    fn helpers_collapse_and_truncate() {
        assert_eq!(one_line("a\nb   c"), "a b c");
        assert_eq!(truncate_chars("abcdef", 3), "abc…");
        assert_eq!(truncate_chars("abc", 3), "abc");
        // Multi-byte safe: never splits a char boundary.
        assert_eq!(truncate_chars("中文测试", 2), "中文…");
    }

    #[test]
    fn banner_prefers_server_id_then_auth_banner_then_synth() {
        let probe_target = target("10.0.0.5", 2222);
        let empty = Arc::new(Mutex::new(CapturedInfo::default()));
        assert_eq!(banner_from(&empty, &probe_target), "10.0.0.5:2222 握手成功");

        let with_id = Arc::new(Mutex::new(CapturedInfo {
            server_id: Some("SSH-2.0-OpenSSH_9.6".into()),
            auth_banner: Some("welcome".into()),
        }));
        assert_eq!(banner_from(&with_id, &probe_target), "SSH-2.0-OpenSSH_9.6");

        let with_banner = Arc::new(Mutex::new(CapturedInfo {
            server_id: None,
            auth_banner: Some("  欢迎使用\r\n".into()),
        }));
        assert_eq!(banner_from(&with_banner, &probe_target), "欢迎使用");
    }

    /// A fake server that completes TCP, sends a valid-looking SSH version
    /// string, then hangs up: the probe must fail fast with `Handshake`,
    /// never hang.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fake_server_yields_handshake_stage() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let _ = sock.write_all(b"SSH-2.0-fake\r\n").await;
            // sock drops here: connection closed mid-kex.
        });

        let rx = spawn_probe(target("127.0.0.1", port));
        let outcome = rx
            .recv_timeout(Duration::from_secs(15))
            .expect("probe hung instead of reporting a handshake failure");
        match outcome {
            TestOutcome::Err { stage, .. } => assert_eq!(stage, Stage::Handshake),
            other => panic!("expected handshake error, got {other:?}"),
        }
        server.await.unwrap();
    }

    /// Nothing listening on the port: refused connect, classified as TCP,
    /// and it must be fast.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn closed_port_yields_tcp_stage_quickly() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // port is free (and closed) again.

        let rx = spawn_probe(target("127.0.0.1", port));
        let started = Instant::now();
        let outcome = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("probe hung on a closed port");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "refused connect should fail fast"
        );
        match outcome {
            TestOutcome::Err { stage, .. } => assert_eq!(stage, Stage::Tcp),
            other => panic!("expected tcp error, got {other:?}"),
        }
    }

    /// Server accepts TCP but never speaks SSH: the 8s budget must fire.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "takes ~8s: verifies the probe timeout end to end"]
    async fn silent_server_yields_timeout_stage() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (_sock, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let rx = spawn_probe(target("127.0.0.1", port));
        let started = Instant::now();
        let outcome = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("probe did not finish");
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_secs(7),
            "timeout fired too early: {elapsed:?}"
        );
        match outcome {
            TestOutcome::Err { stage, .. } => assert_eq!(stage, Stage::Timeout),
            other => panic!("expected timeout, got {other:?}"),
        }
        server.abort();
    }
}
