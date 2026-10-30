//! Headless render smoke tests: every screen and overlay is drawn into a
//! ratatui `TestBackend` and its buffer text is asserted. This is the closest
//! thing to a terminal screenshot we can run in CI — `draw` itself mutates
//! nothing, so these tests also prove it stays that way.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

use sshm::app::{App, Mode, TestCtx, TestState};
use sshm::model::{Host, Store};
use sshm::ssh::{AuthKind, Stage, TestOutcome};
use sshm::ui;

fn host_json(json: &str) -> Host {
    serde_json::from_str(json).expect("host fixture")
}

fn app_with(hosts: &[&str]) -> App {
    let mut store = Store::new();
    for raw in hosts {
        store.insert(host_json(raw));
    }
    App::new(store)
}

fn buffer_text(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in 0..area.height {
        // Wide glyphs occupy two cells: the continuation cell is blank in the
        // buffer, so advance by display width instead of one column.
        let mut x = 0u16;
        while x < area.width {
            let sym = buf[(x, y)].symbol();
            out.push_str(sym);
            let width = UnicodeWidthStr::width(sym).max(1) as u16;
            x += width;
        }
        out.push('\n');
    }
    out
}

fn render(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|frame| ui::draw(app, frame)).expect("draw");
    buffer_text(terminal.backend().buffer())
}

fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn list_shows_title_counts_table_and_ssh_preview() {
    let app = app_with(&[
        r#"{"id":"1","alias":"web-1","host":"10.0.0.1","username":"root","category":"生产"}"#,
        r#"{"id":"2","alias":"old-db","host":"10.0.0.2","username":"db","valid":false}"#,
    ]);
    let screen = render(&app, 100, 24);
    assert!(screen.contains("sshm"), "title present:\n{screen}");
    assert!(
        screen.contains("有效 1 / 无效 1 / 共 2"),
        "counts:\n{screen}"
    );
    assert!(screen.contains("web-1"));
    assert!(screen.contains("old-db"));
    assert!(screen.contains("生产"));
    // The footer previews the exact command Enter will run.
    assert!(
        screen.contains("ssh root@10.0.0.1"),
        "footer command:\n{screen}"
    );
}

#[test]
fn valid_rows_sort_before_invalid_rows_on_screen() {
    let app = app_with(&[
        r#"{"id":"1","alias":"aaa-invalid","host":"1.1.1.1","valid":false}"#,
        r#"{"id":"2","alias":"zzz-valid","host":"2.2.2.2"}"#,
    ]);
    let screen = render(&app, 100, 24);
    let valid_at = screen.find("zzz-valid").expect("valid row");
    let invalid_at = screen.find("aaa-invalid").expect("invalid row");
    assert!(
        valid_at < invalid_at,
        "valid must render above invalid:\n{screen}"
    );
}

#[test]
fn empty_list_renders_the_hint_line() {
    let app = app_with(&[]);
    let screen = render(&app, 100, 24);
    assert!(screen.contains("空列表"), "empty state:\n{screen}");
}

#[test]
fn form_renders_fields_focus_and_default_port() {
    let mut app = app_with(&[]);
    app.open_add();
    let screen = render(&app, 100, 30);
    assert!(screen.contains("新增主机"), "form title:\n{screen}");
    assert!(screen.contains("端口"));
    assert!(screen.contains("22"), "port default:\n{screen}");
    assert!(screen.contains("有效"));
}

#[test]
fn editing_an_invalid_host_shows_the_revive_hint() {
    let mut app = app_with(&[r#"{"id":"1","alias":"old","host":"1.2.3.4","valid":false}"#]);
    app.selected = 0;
    app.open_edit();
    let screen = render(&app, 110, 30);
    assert!(screen.contains("编辑主机"));
    assert!(
        screen.contains("该配置原为无效"),
        "spec: editing an invalid host defaults to valid and says so:\n{screen}"
    );
}

#[test]
fn help_screen_renders_keymap_and_scrolls() {
    let mut app = app_with(&[]);
    app.mode = Mode::Help;
    let top = render(&app, 100, 24);
    assert!(top.contains("帮助"));
    assert!(top.contains("列表"));

    app.help_scroll = 999; // clamped to the last line
    let bottom = render(&app, 100, 24);
    assert_ne!(top, bottom, "scrolling must change what is rendered");
    assert!(
        bottom.contains("SSHM_CONFIG_DIR"),
        "tail of keymap:\n{bottom}"
    );

    app.help_scroll = 31;
    let middle = render(&app, 100, 24);
    assert!(
        middle.contains("对话框") || middle.contains("排序说明"),
        "mid keymap:\n{middle}"
    );
}

#[test]
fn delete_dialog_is_rendered_over_the_list() {
    let mut app = app_with(&[r#"{"id":"1","alias":"web-1","host":"10.0.0.1"}"#]);
    app.delete_selected();
    let screen = render(&app, 100, 24);
    assert!(screen.contains("删除该主机？"), "confirm dialog:\n{screen}");
    assert!(screen.contains("web-1"), "dialog names the host:\n{screen}");
}

#[test]
fn discard_dialog_is_rendered_from_the_form() {
    let mut app = app_with(&[r#"{"id":"1","alias":"web-1","host":"10.0.0.1"}"#]);
    app.selected = 0;
    app.open_edit();
    app.form.as_mut().unwrap().note = "改了点东西".into();
    press(&mut app, KeyCode::Esc);
    let screen = render(&app, 100, 30);
    assert!(
        screen.contains("放弃未保存的修改？"),
        "discard dialog:\n{screen}"
    );
}

#[test]
fn info_dialog_renders_its_body() {
    let mut app = app_with(&[]);
    app.dialog = Some(sshm::app::Dialog::Info {
        title: "导入完成".into(),
        body: vec!["新增 3".into(), "重复 1".into()],
    });
    let screen = render(&app, 100, 24);
    assert!(screen.contains("导入完成"));
    assert!(screen.contains("新增 3"));
}

#[test]
fn test_strip_shows_a_finished_probe_result() {
    let mut app = app_with(&[r#"{"id":"1","alias":"web-1","host":"10.0.0.1"}"#]);
    app.test = TestState::Done {
        ctx: TestCtx::List,
        outcome: TestOutcome::Ok {
            banner: "SSH-2.0-OpenSSH_9.6".into(),
            latency_ms: 42,
            auth: AuthKind::Password,
        },
    };
    let screen = render(&app, 110, 24);
    assert!(screen.contains("42ms"), "test strip:\n{screen}");
    assert!(screen.contains("SSH-2.0-OpenSSH_9.6"));
}

#[test]
fn failed_probe_shows_stage_and_message() {
    let mut app = app_with(&[r#"{"id":"1","alias":"web-1","host":"10.0.0.1"}"#]);
    app.test = TestState::Done {
        ctx: TestCtx::List,
        outcome: TestOutcome::Err {
            stage: Stage::Tcp,
            message: "连接被拒绝".into(),
        },
    };
    let screen = render(&app, 110, 24);
    assert!(screen.contains("TCP 连接"));
    assert!(screen.contains("连接被拒绝"));
}

#[test]
fn toast_is_rendered_above_the_footer() {
    let mut app = app_with(&[]);
    app.toast("已保存");
    let screen = render(&app, 100, 24);
    assert!(screen.contains("已保存"), "toast:\n{screen}");
}

#[test]
fn tiny_terminals_never_panic() {
    for (w, h) in [(20u16, 6u16), (40, 10), (80, 12)] {
        let mut app =
            app_with(&[r#"{"id":"1","alias":"a-very-long-alias-name","host":"1.2.3.4"}"#]);
        render(&app, w, h);

        app.mode = Mode::Help;
        render(&app, w, h);

        app.mode = Mode::Form;
        app.form = Some(sshm::app::FormState::new(None));
        render(&app, w, h);

        app.dialog = Some(sshm::app::Dialog::ConfirmDelete { id: "1".into() });
        render(&app, w, h);

        app.dialog = Some(sshm::app::Dialog::Info {
            title: "导入完成".into(),
            body: vec!["很长的正文要换行，看看在极窄终端里会不会炸掉".into()],
        });
        render(&app, w, h);
    }
}
