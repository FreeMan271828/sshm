//! Launch interactive ssh sessions: in the current terminal, or in a new
//! terminal tab/window chosen in the settings.
//!
//! The "new terminal" medium follows the terminal the user actually runs:
//! Tabby (`tabby run`), WezTerm (`wezterm cli spawn`), Windows Terminal
//! (`wt -w 0 nt`), or a plain shell window. `Auto` detects the hosting
//! terminal from its environment variables, so pressing Enter inside Tabby or
//! WezTerm opens a tab there instead of somewhere else.

use std::io;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Mutex;

use crate::model::Host;
use crate::settings::Terminal;

/// Temporary helpers (askpass scripts, Tabby run scripts) that must outlive
/// the launch call: a new-tab session is started by another process *after*
/// this call returns. They contain no password; removed on exit.
static TEMP_FILES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Launch an interactive ssh session in the current terminal. The caller
/// (main.rs) has already suspended the TUI terminal.
pub(crate) fn run_ssh(host: &Host) -> io::Result<ExitStatus> {
    let mut cmd = Command::new("ssh");
    cmd.args(ssh_args(host));
    apply_askpass(&mut cmd, host)?;
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    // A nonzero exit code is the remote shell's status, not an error.
    cmd.status()
}

/// Open `host` in a new terminal tab/window. Returns the terminal that was
/// used, for the success toast.
pub(crate) fn open_in_new_terminal(host: &Host, terminal: Terminal) -> io::Result<&'static str> {
    let plan = plan(host, terminal)?;
    let mut cmd = Command::new(&plan.program);
    cmd.args(&plan.args);
    // Windows Terminal forwards the caller's environment to the new tab, so a
    // saved password can auto-fill there. Tabby/WezTerm build the child env
    // from their own process, so ssh simply prompts in those tabs.
    apply_askpass(&mut cmd, host)?;
    let mut child = cmd.spawn()?;
    // Reap in the background: the TUI must not block until the terminal closes.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(plan.label)
}

/// Remove helper scripts created for launches (called on exit).
pub fn cleanup_temp_files() {
    let mut files = match TEMP_FILES.lock() {
        Ok(files) => files,
        Err(poisoned) => poisoned.into_inner(),
    };
    for path in files.drain(..) {
        let _ = std::fs::remove_file(path);
    }
}

/// A concrete terminal invocation: program + argv.
#[derive(Debug)]
struct Plan {
    program: String,
    args: Vec<String>,
    /// Human label of the terminal ("Tabby", "WezTerm", "bash", ...).
    label: &'static str,
}

#[cfg(windows)]
fn plan(host: &Host, terminal: Terminal) -> io::Result<Plan> {
    windows::plan(host, terminal, windows::Env::detect())
}

#[cfg(not(windows))]
fn plan(host: &Host, terminal: Terminal) -> io::Result<Plan> {
    unix::plan(host, terminal, unix::Env::detect())
}

/// The terminal sshm itself runs in, judged from the environment its host
/// terminal injects. Pure so it can be tested without touching the process
/// environment.
fn host_terminal_from(
    term_program: Option<&str>,
    wezterm_pane: bool,
    wt_session: bool,
) -> Option<Terminal> {
    if wezterm_pane || term_program == Some("WezTerm") {
        return Some(Terminal::WezTerm);
    }
    if term_program == Some("Tabby") {
        return Some(Terminal::Tabby);
    }
    if wt_session {
        #[cfg(windows)]
        return Some(Terminal::WindowsTerminal);
    }
    None
}

fn detect_host_terminal() -> Option<Terminal> {
    let term_program = std::env::var("TERM_PROGRAM").ok();
    host_terminal_from(
        term_program.as_deref(),
        std::env::var_os("WEZTERM_PANE").is_some(),
        cfg!(windows) && std::env::var_os("WT_SESSION").is_some(),
    )
}

/// The ssh invocation for this host, argv only (no program name).
fn ssh_args(host: &Host) -> Vec<String> {
    let mut args = Vec::new();
    if host.port != 22 {
        args.push("-p".to_string());
        args.push(host.port.to_string());
    }
    args.push("-o".to_string());
    args.push("ConnectTimeout=8".to_string());
    args.push(destination(host));
    args
}

/// `[user@]host` — the `user@` prefix is omitted when username is empty.
pub(crate) fn destination(host: &Host) -> String {
    if host.username.is_empty() {
        host.host.clone()
    } else {
        format!("{}@{}", host.username, host.host)
    }
}

/// `wezterm cli spawn --cwd <dir> -- ssh ...` — a new tab in the WezTerm
/// window that owns the current pane (`WEZTERM_PANE`), or a new window.
fn wezterm_plan(program: String, host: &Host) -> Plan {
    let mut args = vec!["cli".to_string(), "spawn".to_string()];
    if let Ok(cwd) = std::env::current_dir() {
        args.push("--cwd".to_string());
        args.push(cwd.to_string_lossy().into_owned());
    }
    args.push("--".to_string());
    args.push("ssh".to_string());
    args.extend(ssh_args(host));
    Plan {
        program,
        args,
        label: "WezTerm",
    }
}

/// Wire the saved password into ssh via askpass.
///
/// The password never appears on the command line or inside the script — it
/// travels exclusively through the `SSHM_ASKPASS_PW` environment variable.
fn apply_askpass(cmd: &mut Command, host: &Host) -> io::Result<()> {
    let Some(password) = &host.password else {
        return Ok(());
    };
    let script = write_askpass_script()?;
    cmd.env("SSH_ASKPASS", &script)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("SSHM_ASKPASS_PW", password);
    // DISPLAY makes ssh believe it is detached from the tty, so it consults
    // askpass instead of the (suspended) terminal.
    #[cfg(unix)]
    cmd.env("DISPLAY", ":0");
    Ok(())
}

/// Create the askpass script that echoes `SSHM_ASKPASS_PW`.
fn write_askpass_script() -> io::Result<PathBuf> {
    #[cfg(unix)]
    let (suffix, content) = (".sh", "#!/bin/sh\nprintf '%s' \"$SSHM_ASKPASS_PW\"\n");
    #[cfg(windows)]
    let (suffix, content) = (".cmd", "@echo off\necho %SSHM_ASKPASS_PW%\n");
    #[cfg(not(any(unix, windows)))]
    let (suffix, content) = (".sh", "#!/bin/sh\nprintf '%s' \"$SSHM_ASKPASS_PW\"\n");

    write_temp_file("sshm-askpass-", suffix, content)
}

/// Persist a small helper file for the lifetime of the process.
fn write_temp_file(prefix: &str, suffix: &str, content: &str) -> io::Result<PathBuf> {
    let file = tempfile::Builder::new()
        .prefix(prefix)
        .suffix(suffix)
        .tempfile()?;
    std::fs::write(file.path(), content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o700))?;
    }
    let (_file, path) = file.keep().map_err(|e| e.error)?;
    if let Ok(mut files) = TEMP_FILES.lock() {
        files.push(path.clone());
    }
    Ok(path)
}

fn missing_terminal(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("未找到 {what}，可在设置（o）中改用其他终端"),
    )
}

#[cfg(not(windows))]
fn unsupported_terminal(term: Terminal) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("当前平台不支持 {}，请在设置（o）中更换", term.label()),
    )
}

/// Is `program` an executable file on PATH? Used to pick a terminal without
/// spawning a probe process.
fn on_path(program: &str) -> bool {
    // `.exe` only: CreateProcess cannot start a .cmd/.bat by bare name, so
    // treating one as "available" would only produce a spawn error later.
    #[cfg(windows)]
    const EXTS: &[&str] = &["", ".exe"];
    #[cfg(not(windows))]
    const EXTS: &[&str] = &[""];

    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        EXTS.iter()
            .any(|ext| dir.join(format!("{program}{ext}")).is_file())
    })
}

/// First `name` found on PATH, else the first existing candidate path.
fn find_program(names: &[&str], candidates: &[PathBuf]) -> Option<String> {
    for name in names {
        if on_path(name) {
            return Some((*name).to_string());
        }
    }
    candidates
        .iter()
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

/// POSIX single-quote one shell token (`'it'\''s'`), so hosts with shell
/// metacharacters cannot turn into a second command.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(windows)]
mod windows {
    use super::{
        Host, Plan, Terminal, detect_host_terminal, find_program, missing_terminal, on_path,
        sh_quote, ssh_args, wezterm_plan,
    };
    use std::path::PathBuf;

    /// Resolved programs + hosting terminal. Kept separate from [`plan`] so
    /// argv construction is testable without the machine's real PATH.
    #[derive(Debug, Clone)]
    pub(super) struct Env {
        pub wt: bool,
        pub tabby: Option<String>,
        pub wezterm: Option<String>,
        pub host: Option<Terminal>,
    }

    impl Env {
        pub(super) fn detect() -> Self {
            Self {
                wt: on_path("wt"),
                tabby: find_tabby(),
                wezterm: find_wezterm(),
                host: detect_host_terminal(),
            }
        }
    }

    pub(super) fn plan(host: &Host, terminal: Terminal, env: Env) -> std::io::Result<Plan> {
        let term = resolve(terminal, &env);
        match term {
            Terminal::Tabby => {
                let program = env
                    .tabby
                    .clone()
                    .ok_or_else(|| missing_terminal("Tabby（tabby 命令）"))?;
                // `tabby run` parses argv with yargs: leading-dash ssh flags
                // would be eaten as Tabby options, and `--` does not route
                // them into the command positional. Wrapping the ssh command
                // in a quoted `cmd /d /k` string keeps every token positional.
                Ok(Plan {
                    program,
                    args: vec![
                        "run".to_string(),
                        "cmd".to_string(),
                        "/d".to_string(),
                        "/k".to_string(),
                        cmd_command(host),
                    ],
                    label: "Tabby",
                })
            }
            Terminal::WezTerm => {
                let program = env
                    .wezterm
                    .clone()
                    .ok_or_else(|| missing_terminal("WezTerm（wezterm 命令）"))?;
                Ok(wezterm_plan(program, host))
            }
            Terminal::WindowsTerminal => {
                if !env.wt {
                    return Err(missing_terminal("wt.exe（Windows Terminal）"));
                }
                // `-w 0` = most recently used window, `nt` = new tab, `--`
                // stops `wt` from parsing the ssh argv as its own options.
                let mut args: Vec<String> = ["-w", "0", "nt", "--title"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                args.push(host.display_name());
                args.push("--".to_string());
                args.push("ssh".to_string());
                args.extend(ssh_args(host));
                Ok(Plan {
                    program: "wt".to_string(),
                    args,
                    label: "Windows Terminal",
                })
            }
            Terminal::PowerShell => {
                let shell = if on_path("pwsh") {
                    "pwsh"
                } else {
                    "powershell"
                };
                Ok(shell_window(
                    shell,
                    vec![
                        "-NoLogo".to_string(),
                        "-NoExit".to_string(),
                        "-Command".to_string(),
                        ps_command(host),
                    ],
                    "PowerShell",
                ))
            }
            Terminal::Cmd => Ok(shell_window(
                "cmd",
                vec!["/d".to_string(), "/k".to_string(), cmd_command(host)],
                "cmd",
            )),
            Terminal::Bash => {
                if !on_path("bash") {
                    return Err(missing_terminal("bash（Git Bash / WSL）"));
                }
                Ok(shell_window(
                    "bash",
                    vec!["-c".to_string(), bash_command(host)],
                    "bash",
                ))
            }
            Terminal::Auto | Terminal::Zsh => unreachable!("resolved above"),
        }
    }

    /// Auto: the hosting terminal when its CLI is available, else Windows
    /// Terminal, else a PowerShell window.
    fn resolve(terminal: Terminal, env: &Env) -> Terminal {
        if terminal != Terminal::Auto {
            return terminal;
        }
        let host_available = match env.host {
            Some(Terminal::Tabby) => env.tabby.is_some(),
            Some(Terminal::WezTerm) => env.wezterm.is_some(),
            Some(Terminal::WindowsTerminal) => env.wt,
            _ => false,
        };
        if host_available && let Some(host) = env.host {
            return host;
        }
        if env.wt {
            Terminal::WindowsTerminal
        } else {
            Terminal::PowerShell
        }
    }

    /// A shell in its own console window.
    ///
    /// `start` is the only safe way to do this: a plain
    /// `CreateProcess(CREATE_NEW_CONSOLE)` child inherits the TUI's std
    /// handles and draws all over the list.
    fn shell_window(program: &str, args: Vec<String>, label: &'static str) -> Plan {
        let mut full = vec![
            "/d".to_string(),
            "/c".to_string(),
            "start".to_string(),
            String::new(), // window title
            program.to_string(),
        ];
        full.extend(args);
        Plan {
            program: "cmd".to_string(),
            args: full,
            label,
        }
    }

    fn find_tabby() -> Option<String> {
        let mut candidates = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(local)
                    .join("Programs")
                    .join("Tabby")
                    .join("Tabby.exe"),
            );
        }
        for base in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(dir) = std::env::var_os(base) {
                candidates.push(PathBuf::from(dir).join("Tabby").join("Tabby.exe"));
            }
        }
        find_program(&["tabby", "Tabby"], &candidates)
    }

    fn find_wezterm() -> Option<String> {
        let mut candidates = Vec::new();
        if let Some(dir) = std::env::var_os("ProgramFiles") {
            candidates.push(PathBuf::from(dir).join("WezTerm").join("wezterm.exe"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(local)
                    .join("Programs")
                    .join("wezterm")
                    .join("wezterm.exe"),
            );
        }
        find_program(&["wezterm"], &candidates)
    }

    /// PowerShell command string. Every token is single-quoted so a hostile
    /// host string cannot terminate the command and start another one.
    fn ps_command(host: &Host) -> String {
        let mut parts = vec![ps_quote("ssh")];
        parts.extend(ssh_args(host).iter().map(|arg| ps_quote(arg)));
        parts.join(" ")
    }

    fn ps_quote(s: &str) -> String {
        // Inside single quotes PowerShell only treats '' as an escaped quote.
        format!("'{}'", s.replace('\'', "''"))
    }

    /// cmd.exe command string. Tokens are quoted only when they could change
    /// meaning, so the common case stays a plain `ssh -p 2222 root@host`.
    fn cmd_command(host: &Host) -> String {
        let mut parts = vec!["ssh".to_string()];
        parts.extend(ssh_args(host).iter().map(|arg| cmd_quote(arg)));
        parts.join(" ")
    }

    fn cmd_quote(s: &str) -> String {
        let needs_quotes = s.is_empty()
            || s.chars()
                .any(|c| c.is_whitespace() || "\"&|<>^()".contains(c));
        if needs_quotes {
            format!("\"{}\"", s.replace('"', ""))
        } else {
            s.to_string()
        }
    }

    /// bash command string (Git Bash / WSL). The trailing `exec bash -i` keeps
    /// the tab alive with a prompt after the ssh session ends, matching the
    /// `-NoExit` / `/k` behaviour of the other shells.
    fn bash_command(host: &Host) -> String {
        let mut parts = vec![sh_quote("ssh")];
        parts.extend(ssh_args(host).iter().map(|arg| sh_quote(arg)));
        format!("{}; exec bash -i", parts.join(" "))
    }
}

#[cfg(not(windows))]
mod unix {
    use super::{
        Host, Plan, Terminal, detect_host_terminal, find_program, io, missing_terminal, on_path,
        sh_quote, ssh_args, unsupported_terminal, wezterm_plan, write_temp_file,
    };
    use std::path::PathBuf;

    /// Resolved programs + hosting terminal (see the Windows equivalent).
    #[derive(Debug, Clone)]
    pub(super) struct Env {
        pub tabby: Option<String>,
        pub wezterm: Option<String>,
        pub host: Option<Terminal>,
    }

    impl Env {
        pub(super) fn detect() -> Self {
            Self {
                tabby: find_tabby(),
                wezterm: find_wezterm(),
                host: detect_host_terminal(),
            }
        }
    }

    pub(super) fn plan(host: &Host, terminal: Terminal, env: Env) -> std::io::Result<Plan> {
        let term = match terminal {
            Terminal::Auto => match env.host {
                Some(Terminal::Tabby) if env.tabby.is_some() => Terminal::Tabby,
                Some(Terminal::WezTerm) if env.wezterm.is_some() => Terminal::WezTerm,
                _ => Terminal::Auto,
            },
            other => other,
        };

        match term {
            Terminal::Tabby => {
                let program = env
                    .tabby
                    .clone()
                    .ok_or_else(|| missing_terminal("Tabby（tabby 命令）"))?;
                Ok(tabby_plan(program, host)?)
            }
            Terminal::WezTerm => {
                let program = env
                    .wezterm
                    .clone()
                    .ok_or_else(|| missing_terminal("WezTerm（wezterm 命令）"))?;
                Ok(wezterm_plan(program, host))
            }
            Terminal::Auto | Terminal::Bash | Terminal::Zsh => emulator_plan(host, term),
            other => Err(unsupported_terminal(other)),
        }
    }

    /// `tabby run <script>`: yargs would eat ssh's leading-dash flags, so the
    /// command goes into a small executable script instead.
    fn tabby_plan(program: String, host: &Host) -> std::io::Result<Plan> {
        let script = write_temp_file(
            "sshm-tabby-",
            ".sh",
            &format!("#!/bin/sh\n{}\n", shell_command(host)),
        )?;
        Ok(Plan {
            program,
            args: vec!["run".to_string(), script.to_string_lossy().into_owned()],
            label: "Tabby",
        })
    }

    fn shell_for(terminal: Terminal) -> std::io::Result<String> {
        match terminal {
            Terminal::Auto => Ok(std::env::var("SHELL")
                .ok()
                .filter(|shell| !shell.trim().is_empty())
                .unwrap_or_else(|| "/bin/sh".to_string())),
            Terminal::Bash => Ok("bash".to_string()),
            Terminal::Zsh => Ok("zsh".to_string()),
            other => Err(unsupported_terminal(other)),
        }
    }

    /// The shell command that runs ssh with safe quoting.
    fn shell_command(host: &Host) -> String {
        let mut parts = vec!["exec".to_string(), sh_quote("ssh")];
        parts.extend(ssh_args(host).iter().map(|arg| sh_quote(arg)));
        parts.join(" ")
    }

    #[cfg(target_os = "macos")]
    fn emulator_plan(host: &Host, terminal: Terminal) -> std::io::Result<Plan> {
        let shell = shell_for(terminal)?;
        let inner = format!(
            "{} -l -c {}",
            sh_quote(&shell),
            sh_quote(&shell_command(host))
        );
        let escaped = applescript_escape(&inner);
        // `do script ... in front window` adds a tab; without a window we fall
        // back to the plain form, which opens one.
        let script = format!(
            "tell application \"Terminal\"\n\
             if (count of windows) > 0 then\n\
             do script \"{escaped}\" in front window\n\
             else\n\
             do script \"{escaped}\"\n\
             end if\n\
             end tell"
        );
        Ok(Plan {
            program: "osascript".to_string(),
            args: vec!["-e".to_string(), script],
            label: "Terminal",
        })
    }

    #[cfg(not(target_os = "macos"))]
    fn emulator_plan(host: &Host, terminal: Terminal) -> std::io::Result<Plan> {
        let shell = shell_for(terminal)?;
        let emu = find_emulator().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "未找到可用的终端模拟器（gnome-terminal / konsole / xterm…），可在设置（o）中改用当前标签页",
            )
        })?;
        let mut args: Vec<String> = emu.prefix.iter().map(|s| s.to_string()).collect();
        args.push(shell);
        args.push("-l".to_string());
        args.push("-c".to_string());
        args.push(shell_command(host));
        Ok(Plan {
            program: emu.program.to_string(),
            args,
            label: emu.label,
        })
    }

    #[cfg(not(target_os = "macos"))]
    struct Emulator {
        program: &'static str,
        /// Emulator flags that must come before the shell command.
        prefix: &'static [&'static str],
        label: &'static str,
    }

    #[cfg(not(target_os = "macos"))]
    const EMULATORS: &[Emulator] = &[
        Emulator {
            program: "gnome-terminal",
            prefix: &["--tab", "--"],
            label: "gnome-terminal",
        },
        Emulator {
            program: "ptyxis",
            prefix: &["--tab", "--"],
            label: "ptyxis",
        },
        Emulator {
            program: "konsole",
            prefix: &["--new-tab", "-e"],
            label: "konsole",
        },
        Emulator {
            program: "xfce4-terminal",
            prefix: &["--tab", "-x"],
            label: "xfce4-terminal",
        },
        Emulator {
            program: "kitty",
            prefix: &["--detach"],
            label: "kitty",
        },
        Emulator {
            program: "alacritty",
            prefix: &["-e"],
            label: "alacritty",
        },
        Emulator {
            program: "wezterm",
            prefix: &["start", "--"],
            label: "wezterm",
        },
        Emulator {
            program: "x-terminal-emulator",
            prefix: &["-e"],
            label: "x-terminal-emulator",
        },
        Emulator {
            program: "xterm",
            prefix: &["-e"],
            label: "xterm",
        },
    ];

    #[cfg(not(target_os = "macos"))]
    fn find_emulator() -> Option<&'static Emulator> {
        EMULATORS.iter().find(|emu| on_path(emu.program))
    }

    #[cfg(target_os = "macos")]
    fn applescript_escape(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }

    fn find_tabby() -> Option<String> {
        let mut candidates = Vec::new();
        candidates.push(PathBuf::from(
            "/Applications/Tabby.app/Contents/MacOS/Tabby",
        ));
        find_program(&["tabby", "Tabby"], &candidates)
    }

    fn find_wezterm() -> Option<String> {
        let candidates = vec![PathBuf::from(
            "/Applications/WezTerm.app/Contents/MacOS/wezterm",
        )];
        find_program(&["wezterm"], &candidates)
    }
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
    fn ssh_args_cover_port_and_user_combos() {
        assert_eq!(
            ssh_args(&host(2222, "root")),
            ["-p", "2222", "-o", "ConnectTimeout=8", "root@10.0.0.5"]
        );
        assert_eq!(
            ssh_args(&host(22, "root")),
            ["-o", "ConnectTimeout=8", "root@10.0.0.5"]
        );
        assert_eq!(
            ssh_args(&host(22, "")),
            ["-o", "ConnectTimeout=8", "10.0.0.5"]
        );
    }

    #[test]
    fn host_terminal_detection_prefers_wezterm_then_tabby() {
        assert_eq!(
            host_terminal_from(Some("WezTerm"), false, false),
            Some(Terminal::WezTerm)
        );
        assert_eq!(
            host_terminal_from(None, true, false),
            Some(Terminal::WezTerm)
        );
        assert_eq!(
            host_terminal_from(Some("Tabby"), false, false),
            Some(Terminal::Tabby)
        );
        assert_eq!(host_terminal_from(None, false, false), None);
        assert_eq!(host_terminal_from(Some("vscode"), false, false), None);
        #[cfg(windows)]
        assert_eq!(
            host_terminal_from(Some("Windows Terminal"), false, true),
            Some(Terminal::WindowsTerminal)
        );
        #[cfg(not(windows))]
        assert_eq!(host_terminal_from(None, false, true), None);
    }

    #[test]
    fn askpass_script_never_contains_the_password() {
        // Blue team: quotes, spaces, unicode, shell metacharacters — none of
        // it may end up in the script file.
        let nasty = "p@ss \"word\" '中文' $back`tick &|<>%;";
        let mut host = host(22, "root");
        host.password = Some(nasty.to_string());
        let script = write_askpass_script().unwrap();
        let content = std::fs::read_to_string(&script).unwrap();
        assert!(!content.contains(nasty));
        assert!(!content.contains("p@ss"));
        assert!(!content.contains("中文"));
        assert!(content.contains("SSHM_ASKPASS_PW"));
        let _ = std::fs::remove_file(&script);
    }

    #[cfg(windows)]
    #[test]
    fn askpass_script_is_a_cmd_on_windows() {
        let script = write_askpass_script().unwrap();
        assert_eq!(
            script.extension().and_then(|e| e.to_str()),
            Some("cmd"),
            "{script:?}"
        );
        let _ = std::fs::remove_file(&script);
    }

    #[cfg(unix)]
    #[test]
    fn askpass_script_is_executable_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let script = write_askpass_script().unwrap();
        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        let _ = std::fs::remove_file(&script);
    }

    #[cfg(windows)]
    mod windows_plan {
        use super::*;
        use crate::ssh::launch::windows::{Env, plan};

        fn env(wt: bool, tabby: bool, wezterm: bool, host: Option<Terminal>) -> Env {
            Env {
                wt,
                tabby: tabby.then(|| "tabby".to_string()),
                wezterm: wezterm.then(|| "wezterm".to_string()),
                host,
            }
        }

        #[test]
        fn auto_opens_a_tab_in_the_hosting_tabby() {
            let p = plan(
                &host(22, "root"),
                Terminal::Auto,
                env(true, true, true, Some(Terminal::Tabby)),
            )
            .unwrap();
            assert_eq!(p.label, "Tabby");
            assert_eq!(p.program, "tabby");
            assert_eq!(
                p.args,
                [
                    "run",
                    "cmd",
                    "/d",
                    "/k",
                    "ssh -o ConnectTimeout=8 root@10.0.0.5"
                ]
            );
        }

        #[test]
        fn auto_opens_a_tab_in_the_hosting_wezterm() {
            let p = plan(
                &host(22, "root"),
                Terminal::Auto,
                env(true, false, true, Some(Terminal::WezTerm)),
            )
            .unwrap();
            assert_eq!(p.label, "WezTerm");
            assert_eq!(p.program, "wezterm");
            assert_eq!(&p.args[..3], ["cli", "spawn", "--cwd"]);
            assert_eq!(
                p.args[3],
                std::env::current_dir().unwrap().display().to_string()
            );
            assert_eq!(
                &p.args[p.args.len() - 5..],
                ["--", "ssh", "-o", "ConnectTimeout=8", "root@10.0.0.5"]
            );
        }

        #[test]
        fn auto_falls_back_to_wt_when_the_hosting_cli_is_missing() {
            // Tabby is hosting, but the `tabby` command was not found.
            let p = plan(
                &host(22, "root"),
                Terminal::Auto,
                env(true, false, false, Some(Terminal::Tabby)),
            )
            .unwrap();
            assert_eq!(p.label, "Windows Terminal");
            assert_eq!(p.program, "wt");
            assert_eq!(&p.args[..3], ["-w", "0", "nt"]);
        }

        #[test]
        fn auto_prefers_wt_when_the_host_is_unknown() {
            let p = plan(
                &host(22, "root"),
                Terminal::Auto,
                env(true, true, true, None),
            )
            .unwrap();
            assert_eq!(p.label, "Windows Terminal");
            assert_eq!(p.program, "wt");
        }

        #[test]
        fn auto_falls_back_to_a_powershell_window_without_wt() {
            let p = plan(
                &host(22, "root"),
                Terminal::Auto,
                env(false, false, false, None),
            )
            .unwrap();
            assert_eq!(p.label, "PowerShell");
            assert_eq!(p.program, "cmd");
            assert_eq!(&p.args[..3], ["/d", "/c", "start"]);
            assert_eq!(
                p.args[4],
                if super::on_path("pwsh") {
                    "pwsh"
                } else {
                    "powershell"
                }
            );
        }

        #[test]
        fn windows_terminal_tab_runs_ssh_directly() {
            let p = plan(
                &host(22, "root"),
                Terminal::WindowsTerminal,
                env(true, false, false, None),
            )
            .unwrap();
            assert_eq!(
                p.args,
                [
                    "-w",
                    "0",
                    "nt",
                    "--title",
                    "root@10.0.0.5",
                    "--",
                    "ssh",
                    "-o",
                    "ConnectTimeout=8",
                    "root@10.0.0.5"
                ]
            );
        }

        #[test]
        fn tabby_and_wezterm_require_their_cli() {
            let err = plan(
                &host(22, "root"),
                Terminal::Tabby,
                env(false, false, false, None),
            )
            .unwrap_err();
            assert!(err.to_string().contains("Tabby"), "{err}");

            let err = plan(
                &host(22, "root"),
                Terminal::WezTerm,
                env(false, false, false, None),
            )
            .unwrap_err();
            assert!(err.to_string().contains("WezTerm"), "{err}");
        }

        #[test]
        fn shell_presets_open_their_own_window() {
            let ps = plan(
                &host(22, "root"),
                Terminal::PowerShell,
                env(true, true, true, None),
            )
            .unwrap();
            assert_eq!(ps.label, "PowerShell");
            assert_eq!(ps.program, "cmd");
            assert_eq!(&ps.args[..4], ["/d", "/c", "start", ""]);
            assert_eq!(
                ps.args[4],
                if super::on_path("pwsh") {
                    "pwsh"
                } else {
                    "powershell"
                }
            );

            let cmd = plan(
                &host(22, "root"),
                Terminal::Cmd,
                env(true, true, true, None),
            )
            .unwrap();
            assert_eq!(cmd.args[4], "cmd");
            assert_eq!(
                &cmd.args[5..],
                ["/d", "/k", "ssh -o ConnectTimeout=8 root@10.0.0.5"]
            );

            if super::on_path("bash") {
                let bash = plan(
                    &host(22, "root"),
                    Terminal::Bash,
                    env(true, true, true, None),
                )
                .unwrap();
                assert_eq!(bash.args[4], "bash");
                assert_eq!(
                    bash.args[6],
                    "'ssh' '-o' 'ConnectTimeout=8' 'root@10.0.0.5'; exec bash -i"
                );
            }
        }

        #[test]
        fn saved_password_never_reaches_the_command_line() {
            let mut h = host(22, "root");
            h.password = Some("hunter2".into());
            let env = env(true, true, true, Some(Terminal::Tabby));
            for term in [
                Terminal::Auto,
                Terminal::Tabby,
                Terminal::WezTerm,
                Terminal::WindowsTerminal,
                Terminal::PowerShell,
                Terminal::Cmd,
                Terminal::Bash,
            ] {
                let p = plan(&h, term, env.clone()).unwrap();
                for arg in &p.args {
                    assert!(!arg.contains("hunter2"), "{term:?}: {arg}");
                    assert!(!arg.contains("SSHM_ASKPASS_PW"), "{term:?}: {arg}");
                }
            }
        }

        #[test]
        fn hostile_host_strings_stay_quoted() {
            let mut h = host(22, "root");
            h.username = "a'b".into();
            h.host = "x&y".into();
            h.port = 2222;

            let ps = plan(&h, Terminal::PowerShell, env(true, false, false, None)).unwrap();
            assert_eq!(
                ps.args[8],
                "'ssh' '-p' '2222' '-o' 'ConnectTimeout=8' 'a''b@x&y'"
            );

            // The Tabby wrapper goes through cmd, so it uses cmd quoting.
            let tabby = plan(&h, Terminal::Tabby, env(true, true, false, None)).unwrap();
            assert_eq!(tabby.args[4], "ssh -p 2222 -o ConnectTimeout=8 \"a'b@x&y\"");
        }
    }

    #[cfg(not(windows))]
    mod unix_plan {
        use super::*;
        use crate::ssh::launch::unix::{Env, plan};

        fn env(tabby: bool, wezterm: bool, host: Option<Terminal>) -> Env {
            Env {
                tabby: tabby.then(|| "tabby".to_string()),
                wezterm: wezterm.then(|| "wezterm".to_string()),
                host,
            }
        }

        #[test]
        fn auto_opens_a_tab_in_the_hosting_tabby() {
            let p = plan(
                &host(22, "root"),
                Terminal::Auto,
                env(true, false, Some(Terminal::Tabby)),
            )
            .unwrap();
            assert_eq!(p.label, "Tabby");
            assert_eq!(p.program, "tabby");
            assert_eq!(p.args[0], "run");
            let script = std::fs::read_to_string(&p.args[1]).unwrap();
            assert!(script.starts_with("#!/bin/sh"), "{script}");
            assert!(
                script.contains("exec 'ssh' '-o' 'ConnectTimeout=8' 'root@10.0.0.5'"),
                "{script}"
            );
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p.args[1]).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }

        #[test]
        fn auto_opens_a_tab_in_the_hosting_wezterm() {
            let p = plan(
                &host(22, "root"),
                Terminal::Auto,
                env(false, true, Some(Terminal::WezTerm)),
            )
            .unwrap();
            assert_eq!(p.label, "WezTerm");
            assert_eq!(&p.args[..3], ["cli", "spawn", "--cwd"]);
            assert_eq!(
                &p.args[p.args.len() - 5..],
                ["--", "ssh", "-o", "ConnectTimeout=8", "root@10.0.0.5"]
            );
        }

        #[test]
        fn explicit_tabby_without_the_cli_is_an_error() {
            let err =
                plan(&host(22, "root"), Terminal::Tabby, env(false, false, None)).unwrap_err();
            assert!(err.to_string().contains("Tabby"), "{err}");
        }

        #[test]
        fn windows_only_terminals_are_rejected() {
            let err = plan(
                &host(22, "root"),
                Terminal::WindowsTerminal,
                env(false, false, None),
            )
            .unwrap_err();
            assert!(err.to_string().contains("不支持"), "{err}");
        }

        #[test]
        fn auto_falls_back_to_a_terminal_emulator() {
            // Only assert the shape when an emulator exists on the test host.
            if let Ok(p) = plan(&host(22, "root"), Terminal::Auto, env(false, false, None)) {
                let joined = p.args.join(" ");
                assert!(joined.contains("-l -c"), "{joined}");
                assert!(joined.contains("exec 'ssh'"), "{joined}");
            }
        }
    }
}
