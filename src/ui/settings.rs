//! Settings screen: where to open sessions and which terminal to use.
//!
//! Every change is persisted immediately by the event loop (`Action::Save`),
//! so there is no draft state and no save button — the screen only renders
//! `app.store.settings` and the focused row.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::app::{App, SettingsField};
use crate::settings::OpenMode;
use crate::ui::{centered_rect, fit_cells, pad_disp, theme};

/// Box width; leaves ~68 cells for label + value + hints.
const BOX_WIDTH: u16 = 72;
const LABEL_WIDTH: usize = 10;
/// Blank + two setting rows + two hints + blank + save note + key footer + blank.
const INNER_ROWS: u16 = 9;

pub fn draw(app: &App, frame: &mut Frame) {
    let settings = app.store.settings;
    let box_rect = centered_rect(frame.area(), BOX_WIDTH, INNER_ROWS + 2);
    frame.render_widget(
        Block::bordered()
            .border_style(theme::BORDER)
            .title(Span::styled(" 设置 ", theme::TITLE)),
        box_rect,
    );

    let inner = Rect::new(
        box_rect.x + 1,
        box_rect.y + 1,
        box_rect.width.saturating_sub(2),
        box_rect.height.saturating_sub(2),
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut y = inner.y;
    y += 1; // breathing room under the title

    put(
        frame,
        inner,
        y,
        setting_row(
            SettingsField::OpenMode.label(),
            settings.open_mode.label(),
            app.settings_focus == SettingsField::OpenMode,
        ),
    );
    y += 1;
    put(
        frame,
        inner,
        y,
        hint_line(settings.open_mode.detail(), inner.width),
    );
    y += 2; // blank line between the two settings

    put(
        frame,
        inner,
        y,
        setting_row(
            SettingsField::Terminal.label(),
            settings.terminal.label(),
            app.settings_focus == SettingsField::Terminal,
        ),
    );
    y += 1;
    let terminal_detail = match settings.open_mode {
        OpenMode::CurrentTab => "当前标签页模式下不生效（ssh 直接占用本终端）",
        OpenMode::NewTab => settings.terminal.detail(),
    };
    put(frame, inner, y, hint_line(terminal_detail, inner.width));
    y += 2;

    put(
        frame,
        inner,
        y,
        Line::from(Span::styled(
            "修改会立即保存到 hosts.json（settings 段）",
            theme::DIM,
        )),
    );
    y += 1;
    put(
        frame,
        inner,
        y,
        Line::from(vec![
            Span::styled("Tab/↑↓", theme::KEY),
            Span::raw(" 切换项目  "),
            Span::styled("←/→/空格", theme::KEY),
            Span::raw(" 修改  "),
            Span::styled("Esc/q", theme::KEY),
            Span::raw(" 返回"),
        ]),
    );
}

/// One setting row: label, then `◀ value ▶` with the arrows as affordances.
fn setting_row(label: &str, value: &str, focused: bool) -> Line<'static> {
    let label_style = if focused { theme::ACCENT } else { theme::DIM };
    let value_style = if focused {
        theme::SELECTED
    } else {
        Style::new()
    };
    Line::from(vec![
        Span::styled(pad_disp(label, LABEL_WIDTH), label_style),
        Span::raw("  "),
        Span::styled("◀", theme::KEY),
        Span::styled(format!(" {value} "), value_style),
        Span::styled("▶", theme::KEY),
    ])
}

/// Dim explanation line, indented two cells and clipped to the box.
fn hint_line(text: &str, width: u16) -> Line<'static> {
    let indent = 2usize;
    let max = (width as usize).saturating_sub(indent);
    Line::from(vec![
        Span::raw(" ".repeat(indent)),
        Span::styled(fit_cells(text, max), theme::DIM),
    ])
}

/// Render one line at absolute row `y`, clipped to `inner`.
fn put(frame: &mut Frame, inner: Rect, y: u16, line: Line<'static>) {
    if y >= inner.y + inner.height {
        return;
    }
    frame.render_widget(Paragraph::new(line), Rect::new(inner.x, y, inner.width, 1));
}
