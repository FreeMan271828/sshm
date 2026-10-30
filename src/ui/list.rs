//! List screen: title bar, search bar, host table, test status strip, footer.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use crate::app::{App, TestCtx, TestState};
use crate::model::Host;
use crate::ssh::TestOutcome;
use crate::ui::{centered_rect, fmt_last_used, theme, truncate};

pub fn draw(app: &App, frame: &mut Frame) {
    let area = frame.area();
    let search_visible = app.searching || !app.filter.is_empty();
    let test_lines = test_strip_lines(app);
    let chunks = Layout::vertical([
        Constraint::Length(1),                         // title bar
        Constraint::Length(u16::from(search_visible)), // search bar
        Constraint::Min(0),                            // table / empty state
        Constraint::Length(test_lines.len() as u16),   // test status strip
        Constraint::Length(2),                         // footer
    ])
    .split(area);

    draw_title(app, frame, chunks[0]);
    if search_visible {
        draw_search(app, frame, chunks[1]);
    }
    draw_table(app, frame, chunks[2]);
    if !test_lines.is_empty() {
        draw_test_strip(frame, chunks[3], &test_lines);
    }
    draw_footer(app, frame, chunks[4]);
}

fn draw_title(app: &App, frame: &mut Frame, area: Rect) {
    let total = app.store.hosts.len();
    let valid = app.store.hosts.iter().filter(|h| h.valid).count();
    let right = format!(
        "排序: {}   有效 {valid} / 无效 {} / 共 {total}",
        app.store.sort_mode.label(),
        total - valid
    );
    let cols = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(crate::ui::disp_width(&right) as u16),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Span::styled("sshm — SSH 连接管理", theme::TITLE)),
        cols[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(right, theme::DIM)).alignment(Alignment::Right),
        cols[1],
    );
}

fn draw_search(app: &App, frame: &mut Frame, area: Rect) {
    let mut spans = vec![Span::styled("/", theme::ACCENT), Span::raw(&app.filter)];
    if app.searching {
        spans.push(Span::styled(" ", theme::CURSOR));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_table(app: &App, frame: &mut Frame, area: Rect) {
    let ids = app.visible_ids();
    if ids.is_empty() {
        let msg = Span::styled(
            "空列表 — 按 a 新增主机，按 i 导入 ~/.ssh/config",
            theme::DIM,
        );
        let rect = centered_rect(area, area.width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(msg)).alignment(Alignment::Center),
            rect,
        );
        return;
    }

    let header =
        Row::new(["名称", "主机", "账号", "分类", "状态", "备注", "最近使用"]).style(theme::KEY);
    let rows = ids.iter().filter_map(|id| app.store.get(id)).map(host_row);
    let table = Table::new(rows, table_widths(area.width))
        .header(header)
        .row_highlight_style(theme::SELECTED)
        .column_spacing(2);

    // `draw` must not mutate the app, so copy the scroll state (TableState is
    // Copy) and hand the copy to the stateful render.
    let mut state: TableState = app.table_state;
    frame.render_stateful_widget(table, area, &mut state);
}

/// Column widths: the contract's default set, with a compact fallback so a
/// narrow terminal (e.g. 80 columns) still shows every column.
fn table_widths(area_width: u16) -> Vec<Constraint> {
    if area_width < 110 {
        vec![
            Constraint::Length(12), // 名称
            Constraint::Length(18), // 主机
            Constraint::Length(10), // 账号
            Constraint::Length(8),  // 分类
            Constraint::Length(4),  // 状态
            Constraint::Fill(1),    // 备注 (rest)
            Constraint::Length(10), // 最近使用
        ]
    } else {
        vec![
            Constraint::Length(18), // 名称
            Constraint::Length(24), // 主机
            Constraint::Length(14), // 账号
            Constraint::Length(12), // 分类
            Constraint::Length(6),  // 状态
            Constraint::Fill(1),    // 备注 (rest)
            Constraint::Length(12), // 最近使用
        ]
    }
}

fn host_row(h: &Host) -> Row<'static> {
    let status = if h.valid {
        Span::styled("✓", theme::SUCCESS)
    } else {
        Span::styled("✗", theme::DIM)
    };
    Row::new(vec![
        Cell::from(h.display_name()),
        Cell::from(h.target()),
        Cell::from(h.username.clone()),
        Cell::from(h.category.clone()),
        Cell::from(Line::from(status)),
        Cell::from(truncate(&h.note, 48)),
        Cell::from(fmt_last_used(h.last_used_at)),
    ])
    .style(if h.valid { Style::new() } else { theme::DIM })
}

/// Status strip lines for the list screen: only list-context results show here.
fn test_strip_lines(app: &App) -> Vec<Line<'static>> {
    match &app.test {
        TestState::Idle => vec![],
        TestState::Running => vec![Line::from(Span::styled("⏳ 测试中…", theme::WARN))],
        TestState::Done {
            ctx: TestCtx::List,
            outcome,
        } => outcome_lines(outcome),
        TestState::Done {
            ctx: TestCtx::Form, ..
        } => vec![],
    }
}

pub(crate) fn outcome_lines(outcome: &TestOutcome) -> Vec<Line<'static>> {
    let style = if outcome.is_ok() {
        theme::SUCCESS
    } else {
        theme::ERROR
    };
    let mut lines = vec![Line::from(Span::styled(outcome.summary(), style))];
    if let Some(detail) = outcome.detail() {
        lines.push(Line::from(Span::styled(detail, theme::DIM)));
    }
    lines
}

fn draw_test_strip(frame: &mut Frame, area: Rect, lines: &[Line<'static>]) {
    for (offset, line) in lines.iter().take(area.height as usize).enumerate() {
        let y = area.y + offset as u16;
        frame.render_widget(
            Paragraph::new(line.clone()),
            Rect::new(area.x, y, area.width, 1),
        );
    }
}

fn draw_footer(app: &App, frame: &mut Frame, area: Rect) {
    let keys = [
        ("a", "新增"),
        ("e", "编辑"),
        ("d", "删除"),
        ("x", "有效/无效"),
        ("t", "测试"),
        ("/", "搜索"),
        ("s", "排序"),
        ("i", "导入"),
        ("r", "重载"),
        ("?", "帮助"),
        ("q", "退出"),
    ];
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(keys.len() * 3);
    for (i, (key, label)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(*key, theme::KEY));
        spans.push(Span::raw(format!(" {label}")));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x, area.y, area.width, 1),
    );

    if let Some(host) = app.selected_host() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!("$ {}", ssh_command_preview(&host)),
                theme::DIM,
            )),
            Rect::new(area.x, area.y + 1, area.width, 1),
        );
    }
}

/// The ssh command shown in the footer. `ssh::ssh_command_string` was a
/// `todo!()` stub when this renderer was written and has since landed; the
/// catch_unwind guard is kept as cheap insurance so a panic in a future
/// ssh-layer refactor can never take down the render loop — it degrades to a
/// simple `ssh [user@]target` preview instead. The thread-local flag avoids
/// re-panicking (and stderr noise) on every frame once triggered.
fn ssh_command_preview(host: &Host) -> String {
    use std::cell::Cell;

    thread_local! {
        static SSH_CMD_STUB_PANICKED: Cell<bool> = const { Cell::new(false) };
    }
    if SSH_CMD_STUB_PANICKED.get() {
        return fallback_ssh_preview(host);
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::ssh::ssh_command_string(host)
    })) {
        Ok(cmd) => cmd,
        Err(_) => {
            SSH_CMD_STUB_PANICKED.set(true);
            fallback_ssh_preview(host)
        }
    }
}

fn fallback_ssh_preview(host: &Host) -> String {
    if host.username.is_empty() {
        format!("ssh {}", host.target())
    } else {
        format!("ssh {}@{}", host.username, host.target())
    }
}
