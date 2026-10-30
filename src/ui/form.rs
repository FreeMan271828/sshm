//! Form screen (add/edit host): centered box with one line per field.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::app::{App, Field, FormState, TestCtx, TestState};
use crate::ssh::TestOutcome;
use crate::ui::{centered_rect, disp_width, fit_cells, theme};

/// Box width; the 8-cell label column leaves ~55 cells for values at 66 wide.
const BOX_WIDTH: u16 = 66;

pub fn draw(app: &App, frame: &mut Frame) {
    let Some(form) = &app.form else { return };

    // Fixed rows: title, blank, 8 fields, footer.
    let content_rows = 1
        + 1
        + 8
        + 1
        + u16::from(form.was_invalid)
        + form_result_rows(app, form)
        + u16::from(form.error.is_some());
    let box_rect = centered_rect(frame.area(), BOX_WIDTH, content_rows + 2);

    frame.render_widget(Block::bordered().border_style(theme::BORDER), box_rect);

    let inner = Rect::new(
        box_rect.x + 1,
        box_rect.y + 1,
        box_rect.width.saturating_sub(2),
        box_rect.height.saturating_sub(2),
    );
    if inner.height == 0 {
        return;
    }

    let mut y = inner.y;
    // Title
    let title = if form.editing.is_some() {
        "编辑主机"
    } else {
        "新增主机"
    };
    frame.render_widget(
        Paragraph::new(Span::styled(title, theme::TITLE)),
        Rect::new(inner.x, y, inner.width, 1),
    );
    y += 1;

    // Fields
    for field in Field::ALL {
        render_field(frame, form, field, inner, y);
        y += 1;
    }

    // Footer
    render_footer(frame, inner, y);
    y += 1;

    // Hint
    if form.was_invalid && y < inner.y + inner.height {
        let text = fit_cells(
            "该配置原为无效，保存后默认标记为有效（按空格可取消）",
            inner.width as usize,
        );
        frame.render_widget(
            Paragraph::new(Span::styled(text, theme::WARN)),
            Rect::new(inner.x, y, inner.width, 1),
        );
        y += 1;
    }

    // Test result (form.last_test or Done{ctx: Form}), or "testing" spinner line.
    if let Some(outcome) = form_outcome(app, form) {
        for line in crate::ui::list::outcome_lines(outcome) {
            if y >= inner.y + inner.height {
                break;
            }
            frame.render_widget(
                Paragraph::new(truncate_line(&line, inner.width as usize)),
                Rect::new(inner.x, y, inner.width, 1),
            );
            y += 1;
        }
    } else if matches!(&app.test, TestState::Running) && y < inner.y + inner.height {
        frame.render_widget(
            Paragraph::new(Span::styled("⏳ 测试中…", theme::WARN)),
            Rect::new(inner.x, y, inner.width, 1),
        );
        y += 1;
    }

    // Error
    if let Some(err) = &form.error
        && y < inner.y + inner.height
    {
        frame.render_widget(
            Paragraph::new(Span::styled(
                fit_cells(err, inner.width as usize),
                theme::ERROR,
            )),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }
}

fn form_result_rows(app: &App, form: &FormState) -> u16 {
    match form_outcome(app, form) {
        Some(o) => crate::ui::list::outcome_lines(o).len() as u16,
        None => u16::from(matches!(&app.test, TestState::Running)),
    }
}

fn form_outcome<'a>(app: &'a App, form: &'a FormState) -> Option<&'a TestOutcome> {
    match &app.test {
        TestState::Done {
            ctx: TestCtx::Form,
            outcome,
        } => Some(outcome),
        _ => form.last_test.as_ref(),
    }
}

/// The value shown for one field: password masked unless revealed.
fn field_value(form: &FormState, field: Field) -> String {
    match field {
        Field::Alias => form.alias.clone(),
        Field::Host => form.host.clone(),
        Field::Port => form.port.clone(),
        Field::Username => form.username.clone(),
        Field::Password => {
            if form.reveal_password {
                form.password.clone()
            } else {
                "•".repeat(form.password.chars().count())
            }
        }
        Field::Note => form.note.clone(),
        Field::Category => form.category.clone(),
        Field::Valid => String::new(), // rendered as a toggle below
    }
}

fn render_field(frame: &mut Frame, form: &FormState, field: Field, inner: Rect, y: u16) {
    if y >= inner.y + inner.height {
        return;
    }
    let focused = form.focus == field;
    let (label_style, value_style) = if focused {
        (theme::SELECTED, theme::SELECTED)
    } else {
        (theme::DIM, Style::new())
    };

    // Label column: fixed 8 cells, padded by display width.
    let label = pad_disp(field.label(), 8);
    let mut spans = vec![Span::styled(label, label_style), Span::raw(" ")];

    if field == Field::Valid {
        spans.push(if form.valid {
            Span::styled("[✓] 有效", theme::SUCCESS)
        } else {
            Span::styled("[ ] 无效", theme::DIM)
        });
        if focused {
            spans.push(Span::styled(" ", theme::CURSOR));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(inner.x, y, inner.width, 1),
        );
        return;
    }

    // Value area: everything after 9 cells (label 8 + separator 1), minus the
    // cursor cell when focused. Tail-truncated so the cursor stays visible.
    let max_cells = (inner.width as usize)
        .saturating_sub(9)
        .saturating_sub(usize::from(focused));
    spans.push(Span::styled(
        fit_cells(&field_value(form, field), max_cells),
        value_style,
    ));
    if focused {
        spans.push(Span::styled(" ", theme::CURSOR));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(inner.x, y, inner.width, 1),
    );
}

fn render_footer(frame: &mut Frame, inner: Rect, y: u16) {
    if y >= inner.y + inner.height {
        return;
    }
    let spans = vec![
        Span::styled("Tab/↑↓", theme::KEY),
        Span::raw(" 切换字段  "),
        Span::styled("Enter", theme::KEY),
        Span::raw(" 下一个/保存  "),
        Span::styled("Ctrl+S", theme::KEY),
        Span::raw(" 保存  "),
        Span::styled("Ctrl+T", theme::KEY),
        Span::raw(" 测试连接  "),
        Span::styled("Ctrl+R", theme::KEY),
        Span::raw(" 显示密码  "),
        Span::styled("Esc", theme::KEY),
        Span::raw(" 取消"),
    ];
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(inner.x, y, inner.width, 1),
    );
}

/// Pad a label to a fixed display width using the ASCII/wide heuristic.
fn pad_disp(s: &str, width: usize) -> String {
    let mut out = String::from(s);
    let used = disp_width(s);
    if used < width {
        out.push_str(&" ".repeat(width - used));
    }
    out
}

/// Flatten a styled line back to one truncated styled line (for the result
/// strip inside the fixed-width form box).
fn truncate_line(line: &Line<'static>, max_cells: usize) -> Line<'static> {
    let style = line.spans.first().map(|s| s.style).unwrap_or_default();
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    Line::from(Span::styled(fit_cells(&text, max_cells), style))
}
