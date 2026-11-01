//! sshm entry point: CLI parsing + TUI event loop + side effects
//! (persistence, ssh launch, connection-test threads).

use std::io::IsTerminal;
use std::sync::mpsc::Receiver;

use crossterm::event::{self, Event, KeyEventKind};
use sshm::app::{Action, App, TestCtx};
use sshm::import;
use sshm::model::{Host, SortMode};
use sshm::settings::OpenMode;
use sshm::ssh::{self, TestOutcome};
use sshm::storage;
use sshm::ui;

const HELP: &str = "\
sshm — 终端 SSH 连接管理器

用法:
  sshm                    启动交互式 TUI
  sshm list [选项]        打印排序后的主机列表
  sshm import             从 ~/.ssh/config 导入并打印报告

选项:
  --sort <default|recent>  list 的排序模式 (默认: default)
  --json                   list 以 JSON 输出 (不含密码)
  --config <目录>          配置目录 (默认按平台, 也可用环境变量 SSHM_CONFIG_DIR)
  -h, --help               显示帮助
  -V, --version            显示版本

排序不变量: 无论哪种模式, 有效的记录永远排在无效的前面。
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = dispatch(&args);
    // Helper scripts (askpass, Tabby run wrappers) live for the process
    // lifetime; remove them on every exit path.
    ssh::cleanup_temp_files();
    match result {
        Ok(true) => {}
        Ok(false) => {
            print!("{HELP}");
        }
        Err(msg) => {
            eprintln!("sshm: {msg}");
            std::process::exit(1);
        }
    }
}

/// Returns Ok(true) when the command was handled and the process may exit,
/// Ok(false) to print help.
fn dispatch(args: &[String]) -> Result<bool, String> {
    if args.is_empty() {
        return run_tui();
    }
    let mut cmd: Option<&str> = None;
    let mut sort = SortMode::Default;
    let mut json = false;
    let mut config: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "list" | "import" | "tui" => {
                if cmd.is_some() {
                    return Err(format!("多余的命令: {a}"));
                }
                cmd = Some(a);
            }
            "--sort" => {
                i += 1;
                let v = args.get(i).ok_or("--sort 需要一个参数")?;
                sort = match v.as_str() {
                    "default" => SortMode::Default,
                    "recent" => SortMode::Recent,
                    other => return Err(format!("未知排序模式: {other}")),
                };
            }
            "--json" => json = true,
            "--config" => {
                i += 1;
                config = Some(args.get(i).ok_or("--config 需要一个参数")?.to_string());
            }
            "-h" | "--help" => return Ok(false),
            "-V" | "--version" => {
                println!("sshm {}", env!("CARGO_PKG_VERSION"));
                return Ok(true);
            }
            other => return Err(format!("未知参数: {other}")),
        }
        i += 1;
    }
    if let Some(dir) = config {
        storage::set_config_dir(dir);
    }

    match cmd {
        Some("list") => run_list(sort, json),
        Some("import") => run_import(),
        Some("tui") | None => run_tui(),
        _ => unreachable!(),
    }
}

/// JSON view of a host for `sshm list --json`: every public field, **no
/// password**. Keeping the fields explicit (instead of serializing `Host` and
/// stripping a key) preserves declaration order and makes the omission
/// impossible to reintroduce by accident.
#[derive(serde::Serialize)]
struct HostJson<'a> {
    id: &'a str,
    alias: &'a str,
    host: &'a str,
    port: u16,
    username: &'a str,
    note: &'a str,
    category: &'a str,
    valid: bool,
    last_used_at: Option<i64>,
    use_count: u32,
    created_at: i64,
    updated_at: i64,
}

impl<'a> From<&'a Host> for HostJson<'a> {
    fn from(h: &'a Host) -> Self {
        Self {
            id: &h.id,
            alias: &h.alias,
            host: &h.host,
            port: h.port,
            username: &h.username,
            note: &h.note,
            category: &h.category,
            valid: h.valid,
            last_used_at: h.last_used_at,
            use_count: h.use_count,
            created_at: h.created_at,
            updated_at: h.updated_at,
        }
    }
}

fn run_list(sort: SortMode, json: bool) -> Result<bool, String> {
    let mut store = storage::load().map_err(|e| e.to_string())?;
    store.sort_mode = sort;
    let hosts: Vec<&Host> = sshm::model::sorted_view(&store.hosts, sort);
    if json {
        // Never dump saved passwords into terminals/logs/pipes: `list --json`
        // is for scripting, and a password field there ends up in shell
        // history and CI logs. The field is omitted, not blanked.
        let public: Vec<HostJson<'_>> = hosts.iter().map(|h| HostJson::from(*h)).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&public).map_err(|e| e.to_string())?
        );
        return Ok(true);
    }
    if hosts.is_empty() {
        println!("(空 — 配置文件里还没有主机)");
        return Ok(true);
    }
    for h in hosts {
        println!(
            "{:<20} {:<22} {:<16} {:<12} {:<6} {}",
            h.display_name(),
            h.target(),
            h.username,
            if h.category.is_empty() {
                "-"
            } else {
                &h.category
            },
            if h.valid { "有效" } else { "无效" },
            if h.note.is_empty() { "" } else { &h.note }
        );
    }
    Ok(true)
}

fn run_import() -> Result<bool, String> {
    let mut store = storage::load().map_err(|e| e.to_string())?;
    let report = import::import_ssh_config(&mut store);
    if report.added > 0 {
        storage::save(&store).map_err(|e| e.to_string())?;
    }
    // A fatal source error (missing/unreadable config) is an error exit, not a
    // cheerful "导入完成 新增 0" — scripts need to see the difference.
    if let Some(err) = &report.error {
        return Err(err.clone());
    }
    println!(
        "导入完成: 新增 {} · 重复跳过 {} · 通配符跳过 {}",
        report.added, report.skipped_dup, report.skipped_wild
    );
    Ok(true)
}

fn run_tui() -> Result<bool, String> {
    if !std::io::stdin().is_terminal() {
        return Err("需要交互式终端才能启动 TUI".into());
    }
    let store = storage::load().map_err(|e| e.to_string())?;
    let mut app = App::new(store);

    let mut terminal = ratatui::init();
    let result = run_event_loop(&mut app, &mut terminal);
    ratatui::restore();
    match result {
        Ok(()) => Ok(true),
        Err(e) => Err(format!("TUI 运行失败: {e}")),
    }
}

fn run_event_loop(
    app: &mut App,
    terminal: &mut ratatui::DefaultTerminal,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut pending_test: Option<(TestCtx, Receiver<TestOutcome>)> = None;
    // Test hook: `SSHM_TUI_EXIT_AFTER_MS=1200` renders the real TUI and exits,
    // so a smoke test can prove init/draw/restore work on a real console.
    let deadline = std::env::var("SSHM_TUI_EXIT_AFTER_MS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms));

    while app.running {
        terminal.draw(|frame| ui::draw(app, frame))?;

        if let Some(at) = deadline
            && std::time::Instant::now() >= at
        {
            app.running = false;
            break;
        }

        // Drain test results first (they change what the next frame shows).
        if let Some((ctx, rx)) = &pending_test {
            let mut got = false;
            while let Ok(outcome) = rx.try_recv() {
                app.apply_test_outcome(*ctx, outcome);
                got = true;
            }
            if got {
                pending_test = None;
            }
        }

        if event::poll(std::time::Duration::from_millis(50))? {
            let ev = event::read()?;
            match ev {
                Event::Key(key) => {
                    // Ignore key release/repeat events on Windows.
                    if key.kind == KeyEventKind::Release || key.kind == KeyEventKind::Repeat {
                        continue;
                    }
                    let action = app.on_key(key);
                    if let Err(e) = handle_action(app, &mut pending_test, action) {
                        app.toast(format!("操作失败: {e}"));
                    }
                }
                Event::Mouse(_) => {}
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        app.tick();
    }
    Ok(())
}

fn handle_action(
    app: &mut App,
    pending_test: &mut Option<(TestCtx, Receiver<TestOutcome>)>,
    action: Action,
) -> Result<(), String> {
    match action {
        Action::None => {}
        Action::Save => {
            storage::save(&app.store).map_err(|e| e.to_string())?;
        }
        Action::Launch(host) => {
            app.record_use(&host.id);
            storage::save(&app.store).map_err(|e| e.to_string())?;
            let name = host.display_name();
            match app.store.settings.open_mode {
                OpenMode::CurrentTab => {
                    // Suspend the TUI so the ssh session owns the terminal.
                    ratatui::restore();
                    let result = ssh::run_ssh(&host);
                    let _ = ratatui::init();
                    match result {
                        Ok(status) => app.toast(format!("{} 会话结束 ({status})", name)),
                        Err(e) => app.toast(format!("启动 ssh 失败: {e}")),
                    }
                }
                OpenMode::NewTab => {
                    // The TUI keeps running: the session lives in another tab.
                    match ssh::open_in_new_terminal(&host, app.store.settings.terminal) {
                        Ok(terminal) => app.toast(format!("{} 已在新终端打开（{terminal}）", name)),
                        Err(e) => app.toast(format!("打开新终端失败: {e}")),
                    }
                }
            }
        }
        Action::StartTest(ctx, target) => {
            let rx = ssh::spawn_probe(target);
            *pending_test = Some((ctx, rx));
        }
        Action::Import => {
            let report = import::import_ssh_config(&mut app.store);
            let body = vec![
                format!("新增: {}", report.added),
                format!("重复跳过: {}", report.skipped_dup),
                format!("通配符跳过: {}", report.skipped_wild),
            ];
            app.dialog = Some(sshm::app::Dialog::Info {
                title: "导入 ~/.ssh/config".into(),
                body,
            });
            if let Some(err) = &report.error {
                app.toast(format!("导入警告: {err}"));
            }
            if report.added > 0 {
                storage::save(&app.store).map_err(|e| e.to_string())?;
                app.toast(format!("导入完成, 新增 {}", report.added));
            } else {
                app.toast("没有可导入的新主机");
            }
        }
        Action::RefreshFromDisk => {
            let mut loaded = storage::load().map_err(|e| e.to_string())?;
            loaded.sort_mode = app.store.sort_mode;
            app.store = loaded;
            app.toast("已从磁盘重载");
        }
        Action::Quit => {
            app.running = false;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode as KC, KeyEvent, KeyModifiers};
    use sshm::app::Mode;
    use sshm::model::Host;

    fn key(code: KC, ctrl: bool) -> KeyEvent {
        KeyEvent::new(
            code,
            if ctrl {
                KeyModifiers::CONTROL
            } else {
                KeyModifiers::NONE
            },
        )
    }

    fn sample() -> App {
        let mut store = sshm::model::Store::new();
        store.insert(Host {
            id: "1".into(),
            alias: "web".into(),
            host: "10.0.0.1".into(),
            port: 22,
            username: "root".into(),
            password: None,
            note: String::new(),
            category: "prod".into(),
            valid: true,
            last_used_at: None,
            use_count: 0,
            created_at: 0,
            updated_at: 0,
        });
        App::new(store)
    }

    #[test]
    fn quit_key_works() {
        let mut app = sample();
        let a = app.on_key(key(KC::Char('q'), false));
        assert_eq!(a, Action::Quit);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = sample();
        let a = app.on_key(key(KC::Char('c'), true));
        assert_eq!(a, Action::Quit);
    }

    #[test]
    fn slash_enters_search_esc_exits_and_clears() {
        let mut app = sample();
        app.on_key(key(KC::Char('/'), false));
        assert!(app.searching);
        app.on_key(key(KC::Char('w'), false));
        app.on_key(key(KC::Char('e'), false));
        assert_eq!(app.filter, "we");
        app.on_key(key(KC::Esc, false));
        assert!(!app.searching);
        assert!(app.filter.is_empty());
    }

    #[test]
    fn a_opens_add_form_with_valid_default() {
        let mut app = sample();
        app.on_key(key(KC::Char('a'), false));
        assert_eq!(app.mode, Mode::Form);
        assert!(app.form.as_ref().unwrap().valid);
        assert!(app.form.as_ref().unwrap().editing.is_none());
    }

    #[test]
    fn default_port_22_when_field_empty() {
        let mut app = sample();
        app.on_key(key(KC::Char('a'), false));
        {
            let f = app.form.as_mut().unwrap();
            f.host = "example.com".into();
            f.username = "root".into();
        }
        let saved = app.apply_form().unwrap();
        assert_eq!(saved.port, 22);
    }

    #[test]
    fn sort_toggle_flips_mode() {
        let mut app = sample();
        let before = app.store.sort_mode;
        app.on_key(key(KC::Char('s'), false));
        assert_ne!(app.store.sort_mode, before);
        app.on_key(key(KC::Char('s'), false));
        assert_eq!(app.store.sort_mode, before);
    }

    #[test]
    fn launch_action_carries_selected_host() {
        let mut app = sample();
        let a = app.on_key(key(KC::Enter, false));
        assert!(matches!(a, Action::Launch(h) if h.host == "10.0.0.1"));
    }

    #[test]
    fn probe_target_from_edit_form() {
        let mut app = sample();
        app.on_key(key(KC::Char('e'), false));
        let t = app.form.as_ref().unwrap().probe_target().unwrap();
        assert_eq!(t.host, "10.0.0.1");
        assert_eq!(t.port, 22);
        assert_eq!(t.password, None);
    }

    #[test]
    fn help_mode_opens_and_closes() {
        let mut app = sample();
        app.on_key(key(KC::Char('?'), false));
        assert_eq!(app.mode, Mode::Help);
        app.on_key(key(KC::Esc, false));
        assert_eq!(app.mode, Mode::List);
    }
}
