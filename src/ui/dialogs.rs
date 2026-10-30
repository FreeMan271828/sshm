//! Dialog overlays (confirmations, info) and the toast. Rendered LAST, on top
//! of a full-area dim overlay.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::{App, Dialog};
use crate::ui::{centered_rect, disp_width, theme, wrap_line};

pub fn draw_dialog(app: &App, frame: &mut Frame, dialog: &Dialog) {
    let area = frame.area();
    // Dim everything behind the dialog.
    frame.render_widget(Clear, area);
    frame.render_widget(Block::new().style(theme::OVERLAY), area);

    match dialog {
        Dialog::ConfirmDelete { id } => draw_confirm_delete(app, frame, id),
        Dialog::ConfirmDiscard => draw_confirm_discard(frame),
        Dialog::Info { title, body } => draw_info(frame, title, body),
    }
}

fn draw_confirm_delete(app: &App, frame: &mut Frame, id: &str) {
    let name = app
        .store
        .get(id)
        .map(|h| h.display_name())
        .unwrap_or_else(|| id.to_string());
    let rect = centered_rect(frame.area(), 50, 5);
    frame.render_widget(Block::bordered().border_style(theme::ERROR), rect);
    let inner = Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2);

    put(frame, inner, 0, Line::from(Span::raw("删除该主机？")));
    put(
        frame,
        inner,
        1,
        Line::from(Span::styled(name, theme::SELECTED)),
    );
    put(
        frame,
        inner,
        2,
        Line::from(vec![
            Span::styled("y", theme::KEY),
            Span::raw("=确认  其他=取消"),
        ]),
    );
}

fn draw_confirm_discard(frame: &mut Frame) {
    let rect = centered_rect(frame.area(), 50, 4);
    frame.render_widget(Block::bordered().border_style(theme::WARN), rect);
    let inner = Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2);

    put(frame, inner, 0, Line::from(Span::raw("放弃未保存的修改？")));
    put(
        frame,
        inner,
        1,
        Line::from(vec![
            Span::styled("y", theme::KEY),
            Span::raw("=放弃 其他=继续编辑"),
        ]),
    );
}

fn draw_info(frame: &mut Frame, title: &str, body: &[String]) {
    const W: u16 = 66;
    let max_inner_w = W as usize - 2;

    // Wrap each body line to the box width, then compute the box height.
    let mut lines: Vec<String> = Vec::new();
    for line in body {
        lines.extend(wrap_line(line, max_inner_w));
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    // title + blank + body + footer
    let content_rows = (2 + lines.len() + 1) as u16;
    let max_rows = frame.area().height.saturating_sub(2).max(3);
    let rect = centered_rect(frame.area(), W, content_rows.min(max_rows));

    frame.render_widget(Block::bordered().border_style(theme::BORDER), rect);
    let inner = Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2);

    put(
        frame,
        inner,
        0,
        Line::from(Span::styled(title.to_owned(), theme::TITLE)),
    );
    let mut y = 1u16;
    for line in lines.into_iter() {
        if y >= inner.height {
            break;
        }
        put(frame, inner, y, Line::from(Span::raw(line)));
        y += 1;
    }
    if y < inner.height {
        put(
            frame,
            inner,
            y,
            Line::from(Span::styled("任意键关闭", theme::DIM)),
        );
    }
}

/// Bottom-right floating toast, 1-3 lines, above the footer.
pub fn draw_toast(frame: &mut Frame, msg: &str) {
    let area = frame.area();
    let width = 40u16.min(area.width.saturating_sub(2));
    let max_cells = width as usize - 2;
    let mut lines = wrap_line(msg, max_cells);
    lines.truncate(3);
    let h = lines.len() as u16 + 2;
    let w = lines
        .iter()
        .map(|l| disp_width(l))
        .max()
        .unwrap_or(0)
        .min(max_cells) as u16
        + 4;
    // Above the footer (list footer is 2 rows high; on other screens this just
    // floats a little higher, which is fine).
    let x = area.x + area.width.saturating_sub(w).saturating_sub(1);
    let y = area.y + area.height.saturating_sub(h).saturating_sub(2).max(area.y);
    let h = h.min(area.height);
    let rect = Rect::new(x.max(area.x), y, w.min(area.width), h);

    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(Text::from(
            lines
                .into_iter()
                .map(Line::from)
                .collect::<Vec<Line<'static>>>(),
        ))
        .block(Block::bordered().border_style(theme::DIM))
        .style(theme::DIM),
        rect,
    );
}

/// Render one line at row `y` inside `inner`.
fn put(frame: &mut Frame, inner: Rect, y: u16, line: Line<'static>) {
    if y >= inner.height {
        return;
    }
    frame.render_widget(
        Paragraph::new(line),
        Rect::new(inner.x, inner.y + y, inner.width, 1),
    );
}
