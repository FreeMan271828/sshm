//! Launch interactive ssh sessions through the system `ssh` binary.

use std::io;
use std::process::{Command, ExitStatus, Stdio};

use crate::model::Host;

/// Launch an interactive ssh session. The caller (main.rs) has already
/// suspended the TUI terminal.
pub(crate) fn run_ssh(host: &Host) -> io::Result<ExitStatus> {
    let mut cmd = build_ssh_command(host);

    // Password support via askpass. The password never appears on the command
    // line or inside the script file — it travels exclusively through the
    // SSHM_ASKPASS_PW environment variable, so quotes/spaces/unicode can't
    // break it. On OpenSSH builds that ignore askpass, ssh falls back to an
    // interactive prompt — graceful degradation.
    let _askpass = match &host.password {
        Some(password) => {
            let script = write_askpass_script(password)?;
            cmd.env("SSH_ASKPASS", script.path())
                .env("SSH_ASKPASS_REQUIRE", "force")
                .env("SSHM_ASKPASS_PW", password);
            // DISPLAY makes ssh believe it is detached from the tty, so it
            // consults askpass instead of the (suspended) terminal.
            #[cfg(unix)]
            cmd.env("DISPLAY", ":0");
            Some(script)
        }
        None => None,
    };

    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    // `status()` keeps the askpass temp file alive for the whole session and
    // cleans it up afterwards (`_askpass` drops at scope end). A nonzero exit
    // code is the remote shell's status, not an error.
    cmd.status()
}

/// The ssh invocation for this host, argv only (no askpass env) so it can be
/// inspected in unit tests without executing anything.
fn build_ssh_command(host: &Host) -> Command {
    let mut cmd = Command::new("ssh");
    if host.port != 22 {
        cmd.arg("-p").arg(host.port.to_string());
    }
    cmd.arg("-o").arg("ConnectTimeout=8");
    cmd.arg(destination(host));
    cmd
}

/// `[user@]host` — the `user@` prefix is omitted when username is empty.
pub(crate) fn destination(host: &Host) -> String {
    if host.username.is_empty() {
        host.host.clone()
    } else {
        format!("{}@{}", host.username, host.host)
    }
}

/// Create the askpass script that echoes `SSHM_ASKPASS_PW`.
///
/// The password argument is deliberately ignored: embedding it would
/// reintroduce quoting/escaping problems. The script is a fixed, tiny echo
/// of the env var, so the password lives only in the process environment.
fn write_askpass_script(_password: &str) -> io::Result<tempfile::NamedTempFile> {
    #[cfg(unix)]
    let (suffix, content) = (".sh", "#!/bin/sh\nprintf '%s' \"$SSHM_ASKPASS_PW\"\n");
    #[cfg(windows)]
    let (suffix, content) = (".cmd", "@echo off\necho %SSHM_ASKPASS_PW%\n");
    #[cfg(not(any(unix, windows)))]
    let (suffix, content) = (".sh", "#!/bin/sh\nprintf '%s' \"$SSHM_ASKPASS_PW\"\n");

    let file = tempfile::Builder::new()
        .prefix("sshm-askpass-")
        .suffix(suffix)
        .tempfile()?;
    std::fs::write(file.path(), content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(port: u16, username: &str) -> Host {
        Host {
            id: "x".into(),
            alias: String::new(),
            host: "10.0.0.5".into(),
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

    #[test]
    fn destination_handles_user_and_empty_user() {
        assert_eq!(destination(&host(22, "root")), "root@10.0.0.5");
        assert_eq!(destination(&host(22, "")), "10.0.0.5");
    }

    #[test]
    fn askpass_script_never_contains_the_password() {
        // Blue team: quotes, spaces, unicode, shell metacharacters — none of
        // it may end up in the script file.
        let nasty = "p@ss \"word\" '中文' $back`tick &|<>%;";
        let script = write_askpass_script(nasty).unwrap();
        let content = std::fs::read_to_string(script.path()).unwrap();
        assert!(!content.contains(nasty));
        assert!(!content.contains("p@ss"));
        assert!(!content.contains("中文"));
        assert!(content.contains("SSHM_ASKPASS_PW"));
    }

    #[cfg(unix)]
    #[test]
    fn askpass_script_is_executable_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let script = write_askpass_script("pw").unwrap();
        let mode = std::fs::metadata(script.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[cfg(windows)]
    #[test]
    fn askpass_script_is_a_cmd_on_windows() {
        let script = write_askpass_script("pw").unwrap();
        assert_eq!(
            script.path().extension().and_then(|e| e.to_str()),
            Some("cmd")
        );
    }

    #[test]
    fn ssh_command_argv_covers_port_and_user_combos() {
        let cmd = build_ssh_command(&host(2222, "root"));
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            ["-p", "2222", "-o", "ConnectTimeout=8", "root@10.0.0.5"]
        );

        let cmd = build_ssh_command(&host(22, "root"));
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["-o", "ConnectTimeout=8", "root@10.0.0.5"]);

        let cmd = build_ssh_command(&host(22, ""));
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["-o", "ConnectTimeout=8", "10.0.0.5"]);
    }
}
