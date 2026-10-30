//! Help screen: scrollable keymap from `help_lines()`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::ui::{help_lines, theme};

pub fn draw(app: &App, frame: &mut Frame) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    // Title
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("帮助", theme::TITLE),
            Span::styled("  — sshm 快捷键", theme::DIM),
        ])),
        chunks[0],
    );

    // Keymap, scrolled by app.help_scroll (clamped; shows what fits).
    let lines = help_lines();
    let start = (app.help_scroll as usize).min(lines.len().saturating_sub(1));
    draw_keymap(frame, chunks[1], &lines[start..]);

    // Footer
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("j/k", theme::KEY),
            Span::raw(" 滚动  "),
            Span::styled("q", theme::KEY),
            Span::raw(" 返回"),
        ])),
        chunks[2],
    );
}

fn draw_keymap(frame: &mut Frame, area: Rect, lines: &[&'static str]) {
    for (offset, line) in lines.iter().take(area.height as usize).enumerate() {
        let y = area.y + offset as u16;
        // Section headers are styled; everything else is plain.
        let style = if is_section(line) {
            theme::ACCENT
        } else {
            ratatui::style::Style::new()
        };
        frame.render_widget(
            Paragraph::new(Span::styled(*line, style)),
            Rect::new(area.x, y, area.width, 1),
        );
    }
}

/// Section headers in help_lines() are the bare Chinese words without leading
/// spaces (列表 / 表单 / 对话框 / 排序说明 / 数据文件).
fn is_section(line: &str) -> bool {
    let trimmed = line.trim();
    !line.starts_with(' ')
        && matches!(
            trimmed,
            "列表" | "表单" | "对话框" | "排序说明" | "数据文件"
        )
}
