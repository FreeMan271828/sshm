//! End-to-end CLI tests: spawn the real `sshm` binary and assert on its
//! stdout/stderr, exit code, and the JSON/file side effects. These cover the
//! contract that unit tests inside the crate cannot: argv parsing, the
//! `--config` override, and the `list`/`import` commands as users run them.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sshm::model::{Host, SortMode, Store};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_sshm")
}

fn run(args: &[&str], config: &Path, envs: &[(&str, &Path)]) -> Output {
    let mut cmd = Command::new(bin());
    cmd.arg("--config").arg(config);
    for a in args {
        cmd.arg(a);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().expect("failed to spawn sshm")
}

fn run_args(args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("failed to spawn sshm")
}

fn host(id: &str, name: &str, valid: bool, last: Option<i64>) -> Host {
    Host {
        id: id.into(),
        alias: name.into(),
        host: format!("10.0.0.{id}"),
        port: 22,
        username: "root".into(),
        password: Some("hunter2".into()),
        note: String::new(),
        category: "prod".into(),
        valid,
        last_used_at: last,
        use_count: 0,
        created_at: 0,
        updated_at: 0,
    }
}

fn write_store(dir: &Path, hosts: Vec<Host>) {
    let mut store = Store::new();
    for h in hosts {
        store.insert(h);
    }
    sshm::storage::save_to(dir, &store).expect("save fixtures");
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn list_on_missing_config_is_empty_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(&["list"], dir.path(), &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("还没有主机"), "got: {}", stdout(&out));
}

#[test]
fn list_json_never_leaks_passwords() {
    let dir = tempfile::tempdir().unwrap();
    write_store(dir.path(), vec![host("1", "web", true, None)]);

    let out = run(&["list", "--json"], dir.path(), &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(!text.contains("hunter2"), "password leaked: {text}");
    assert!(!text.contains("password"), "password field leaked: {text}");
    assert!(text.contains("\"alias\": \"web\""), "got: {text}");
}

#[test]
fn list_puts_valid_before_invalid_in_both_modes() {
    let dir = tempfile::tempdir().unwrap();
    // "bad" is the most recently used *and* alphabetically first, so only the
    // valid-first invariant can push it last in both modes.
    write_store(
        dir.path(),
        vec![
            host("3", "bad", false, Some(9_000)),
            host("2", "mid", true, Some(1_000)),
            host("1", "aaa", true, Some(2_000)),
        ],
    );

    for sort in ["default", "recent"] {
        let out = run(&["list", "--sort", sort], dir.path(), &[]);
        assert!(out.status.success(), "{sort}: {}", stderr(&out));
        let text = stdout(&out);
        let bad = text.find("bad").expect("bad row present");
        let aaa = text.find("aaa").expect("aaa row present");
        let mid = text.find("mid").expect("mid row present");
        assert!(bad > aaa && bad > mid, "{sort} put invalid first: {text}");
    }

    // Recent mode must order the two valid entries by last use (aaa > mid).
    let text = stdout(&run(&["list", "--sort", "recent"], dir.path(), &[]));
    assert!(
        text.find("aaa").unwrap() < text.find("mid").unwrap(),
        "recent order wrong: {text}"
    );
}

#[test]
fn unknown_argument_is_a_usage_error() {
    let out = run_args(&["--definitely-not-a-flag"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("未知参数"), "got: {}", stderr(&out));
}

#[test]
fn help_lists_the_commands() {
    let out = run_args(&["--help"]);
    assert!(out.status.success());
    let text = stdout(&out);
    for needle in ["sshm list", "sshm import", "--config", "--sort"] {
        assert!(text.contains(needle), "missing {needle} in help: {text}");
    }
}

#[test]
fn tui_without_a_terminal_refuses_instead_of_garbling() {
    // Child stdin is /dev/null (not a tty) in this harness.
    let out = run_args(&["tui"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("需要交互式终端"),
        "got: {}",
        stderr(&out)
    );
}

#[test]
fn import_merges_ssh_config_and_is_idempotent() {
    let home = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
    std::fs::write(
        home.path().join(".ssh").join("config"),
        "\
# demo config
Host web
    HostName 192.168.1.10
    User deploy
    Port 2222

Host db *.example.com
    User postgres
",
    )
    .unwrap();

    let envs = [("HOME", home.path()), ("USERPROFILE", home.path())];
    let out = run(&["import"], config.path(), &envs);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("新增 2"), "got: {text}");
    assert!(text.contains("通配符跳过 1"), "got: {text}");

    // Second run adds nothing.
    let again = stdout(&run(&["import"], config.path(), &envs));
    assert!(again.contains("新增 0"), "import not idempotent: {again}");

    // The records really landed: web on its custom port, db on 22.
    let json = stdout(&run(&["list", "--json"], config.path(), &[]));
    assert!(json.contains("\"host\": \"192.168.1.10\""), "{json}");
    assert!(json.contains("\"port\": 2222"), "{json}");
    assert!(json.contains("\"alias\": \"db\""), "{json}");
    assert!(json.contains("\"port\": 22"), "{json}");
    assert!(
        !json.contains("password"),
        "imported password field: {json}"
    );
}

#[test]
fn import_without_ssh_config_is_an_error_exit() {
    let home = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let envs = [("HOME", home.path()), ("USERPROFILE", home.path())];
    let out = run(&["import"], config.path(), &envs);
    assert!(!out.status.success(), "missing config must fail");
    assert!(stderr(&out).contains("未找到"), "got: {}", stderr(&out));
    assert!(
        stdout(&out).is_empty(),
        "no cheerful '导入完成' on failure: {}",
        stdout(&out)
    );
}

#[test]
fn config_flag_isolates_the_database() {
    let dir = tempfile::tempdir().unwrap();
    write_store(dir.path(), vec![host("1", "only-here", true, None)]);
    let text = stdout(&run(&["list"], dir.path(), &[]));
    assert!(text.contains("only-here"));

    // An empty directory starts empty — no write to the user's real config.
    let empty = tempfile::tempdir().unwrap();
    assert!(stdout(&run(&["list"], empty.path(), &[])).contains("还没有主机"));
    let _ = PathBuf::from(dir.path());
}

#[test]
fn import_preserves_saved_settings() {
    use sshm::settings::{OpenMode, Terminal};

    let home = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
    std::fs::write(
        home.path().join(".ssh").join("config"),
        "Host web\n    HostName 192.168.1.10\n",
    )
    .unwrap();

    let mut store = Store::new();
    store.settings.open_mode = OpenMode::CurrentTab;
    store.settings.terminal = Terminal::Cmd;
    sshm::storage::save_to(config.path(), &store).unwrap();

    let envs = [("HOME", home.path()), ("USERPROFILE", home.path())];
    let out = run(&["import"], config.path(), &envs);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    // `import` rewrites hosts.json: settings must survive that write.
    let reloaded = sshm::storage::load_from(config.path()).unwrap();
    assert_eq!(reloaded.settings, store.settings);
    assert_eq!(reloaded.hosts.len(), 1);
}

#[test]
fn sort_mode_is_accepted_by_the_store_roundtrip() {
    // Guards the CLI/SortMode contract: `--sort` values map 1:1 to SortMode.
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::new();
    store.insert(host("1", "web", true, None));
    store.sort_mode = SortMode::Recent;
    sshm::storage::save_to(dir.path(), &store).unwrap();
    assert!(
        run(&["list", "--sort", "recent"], dir.path(), &[])
            .status
            .success()
    );
    assert!(
        run(&["list", "--sort", "default"], dir.path(), &[])
            .status
            .success()
    );
    assert!(
        !run(&["list", "--sort", "bogus"], dir.path(), &[])
            .status
            .success()
    );
}
