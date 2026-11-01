//! Rendering layer. `draw` renders the whole screen from `&App` and never
//! mutates it. Owned by the UI implementer; contract frozen by Lead.
//!
//! Screens: list (table + search bar + footer), form (add/edit + inline test
//! result), help, plus dialog overlays and toast.

pub mod dialogs;
pub mod form;
pub mod help;
pub mod list;
pub mod settings;
pub mod theme;

use ratatui::Frame;
use ratatui::layout::Rect;

use chrono::TimeZone;

use crate::app::{App, Mode};

/// Render the current screen, then any dialog overlay and the toast on top.
/// Never mutates `app` — all scroll/selection state is copied out.
pub fn draw(app: &App, frame: &mut Frame) {
    match app.mode {
        Mode::List => list::draw(app, frame),
        Mode::Form => form::draw(app, frame),
        Mode::Help => help::draw(app, frame),
        Mode::Settings => settings::draw(app, frame),
    }
    if let Some((msg, _)) = &app.toast {
        dialogs::draw_toast(frame, msg);
    }
    // Dialogs render last, dimming everything beneath them.
    if let Some(dialog) = &app.dialog {
        dialogs::draw_dialog(app, frame, dialog);
    }
}

/// Static keymap help shown on the help screen (scrollable via `app.help_scroll`).
pub fn help_lines() -> &'static [&'static str] {
    &[
        "列表",
        "  j / ↓            向下移动选择",
        "  k / ↑            向上移动选择",
        "  g / G            跳到第一条 / 最后一条",
        "  PgUp / PgDn      上翻 / 下翻 10 行",
        "  Enter            连接所选主机（按设置在新标签页 / 当前标签页打开）",
        "  a                新增主机",
        "  e                编辑所选主机",
        "  d                删除所选主机（需确认）",
        "  x                切换所选主机 有效 / 无效",
        "  t                测试所选主机的连接",
        "  /                搜索（过滤列表；Esc 清空退出，Enter 退出）",
        "  s                切换排序：默认 ↔ 最近使用",
        "  i                从 ~/.ssh/config 导入主机",
        "  r                从磁盘重新加载（丢弃未保存的修改）",
        "  o                设置：打开位置 / 默认终端",
        "  ? / F1           打开本帮助",
        "  q / Esc / Ctrl+C 退出",
        "",
        "表单",
        "  Tab / Shift+Tab  下一个 / 上一个字段",
        "  ↑ / ↓            上一个 / 下一个字段",
        "  Enter            下一个字段；在最后一项时保存",
        "  Ctrl+S           保存",
        "  Ctrl+T           测试当前表单的连接",
        "  Ctrl+R           显示 / 隐藏密码",
        "  空格 / x         在“是否有效”字段切换 有效 / 无效",
        "  Backspace        删除上一个字符",
        "  Esc              取消（有未保存修改时先确认）",
        "",
        "对话框",
        "  y / Enter        确认（删除主机 / 放弃修改）",
        "  其他任意键        取消",
        "  信息框            任意键关闭",
        "",
        "排序说明",
        "  默认：      分类 → 名称 排序",
        "  最近使用：  最近连接过的排在前面",
        "  不变式：    有效主机永远排在无效主机之前（两种模式都如此）",
        "",
        "设置（o）",
        "  打开位置    新建标签页：新终端页签运行 ssh，列表继续可用（默认）",
        "              当前标签页：暂停 TUI，ssh 占用本终端，退出后回到列表",
        "  默认终端    自动：跟随当前终端（Tabby / WezTerm / Windows Terminal）",
        "              Windows 可选：Tabby / WezTerm / Windows Terminal / PowerShell / cmd / bash",
        "              Unix 可选：Tabby / WezTerm / bash / zsh",
        "  保存        修改立即写入 hosts.json 的 settings 段",
        "",
        "数据文件",
        "  主机数据保存在平台配置目录下的 sshm/hosts.json：",
        "  Windows：  %APPDATA%\\sshm\\hosts.json",
        "  macOS：    ~/Library/Application Support/sshm/hosts.json",
        "  Linux：    $XDG_CONFIG_HOME/sshm/hosts.json（默认 ~/.config/sshm/）",
        "  可用环境变量 SSHM_CONFIG_DIR 覆盖该目录",
    ]
}

/// Format a `last_used_at` unix timestamp as relative time, or `-` when `None`.
/// <60s "刚刚", <60m "N分钟前", <24h "N小时前", <30d "N天前", else local `%Y-%m-%d`.
pub(crate) fn fmt_last_used(ts: Option<i64>) -> String {
    let Some(ts) = ts else { return "-".into() };
    let diff = crate::model::now_secs().saturating_sub(ts);
    if diff < 60 {
        "刚刚".into()
    } else if diff < 3_600 {
        format!("{}分钟前", diff / 60)
    } else if diff < 86_400 {
        format!("{}小时前", diff / 3_600)
    } else if diff < 2_592_000 {
        format!("{}天前", diff / 86_400)
    } else {
        // chrono here is built without the `alloc` feature (Cargo.toml is
        // Lead-owned), so `DateTime::format("%Y-%m-%d")` is unavailable.
        // `DateTime<Local>`'s Display prints "YYYY-MM-DD HH:MM:SS +OFFSET" —
        // take the first 10 chars, which is exactly the local %Y-%m-%d date.
        match chrono::Local.timestamp_opt(ts, 0) {
            chrono::MappedLocalTime::Single(dt) => dt.to_string().chars().take(10).collect(),
            _ => "-".into(),
        }
    }
}

/// Pad a label to a fixed display width using the ASCII/wide heuristic.
pub(crate) fn pad_disp(s: &str, width: usize) -> String {
    let mut out = String::from(s);
    let used = disp_width(s);
    if used < width {
        out.push_str(&" ".repeat(width - used));
    }
    out
}

/// Char-count truncation with a trailing ellipsis. `unicode-width` is not a
/// direct dependency (Cargo.toml is Lead-owned), so this truncates by chars —
/// acceptable per the rendering contract; the Table still clips by width.
pub(crate) fn truncate(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s.to_string();
    }
    match max_chars {
        0 => String::new(),
        1 => "…".to_string(),
        n => {
            let mut out: String = chars[..n - 1].iter().collect();
            out.push('…');
            out
        }
    }
}

/// Display width in terminal cells (ASCII = 1, everything else = 2). This
/// matches ratatui's UnicodeWidthStr treatment of the text this UI renders,
/// without pulling `unicode-width` in as a direct dependency.
pub(crate) fn disp_width(s: &str) -> usize {
    s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum()
}

/// Head-truncate by display width, keeping the tail (for values inside boxes).
pub(crate) fn fit_cells(s: &str, max_cells: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    let mut dropped = false;
    for c in s.chars() {
        let w = if c.is_ascii() { 1 } else { 2 };
        if used + w > max_cells {
            dropped = true;
            break;
        }
        used += w;
        out.push(c);
    }
    if dropped {
        // Rebuild with a leading ellipsis, keeping as much tail as fits.
        let mut tail = String::new();
        let mut used = 1; // reserve one cell for …
        for c in s.chars().rev() {
            let w = if c.is_ascii() { 1 } else { 2 };
            if used + w > max_cells {
                break;
            }
            used += w;
            tail.push(c);
        }
        let mut result = String::from("…");
        result.extend(tail.chars().rev());
        result
    } else {
        out
    }
}

/// Greedy cell-width wrap for dialog/toast bodies. Breaks long words only when
/// a single word exceeds the line width.
pub(crate) fn wrap_line(s: &str, max_cells: usize) -> Vec<String> {
    if max_cells == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    for raw in s.split('\n') {
        let mut cur = String::new();
        let mut used = 0;
        for word in raw.split_inclusive(' ') {
            let w = disp_width(word);
            if used + w > max_cells && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                used = 0;
            }
            // If the word itself exceeds the width, hard-break it.
            if w > max_cells {
                if !cur.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                }
                let mut chunk = String::new();
                let mut cw = 0;
                for c in word.chars() {
                    let cwidth = if c.is_ascii() { 1 } else { 2 };
                    if cw + cwidth > max_cells {
                        lines.push(std::mem::take(&mut chunk));
                        cw = 0;
                    }
                    cw += cwidth;
                    chunk.push(c);
                }
                cur = chunk;
                used = cw;
            } else {
                cur.push_str(word);
                used += w;
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// A rect of `w × h` centered inside `area`, clamped so it always fits.
pub(crate) fn centered_rect(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_last_used_relative_buckets() {
        let now = crate::model::now_secs();
        assert_eq!(fmt_last_used(None), "-");
        assert_eq!(fmt_last_used(Some(now - 30)), "刚刚");
        assert_eq!(fmt_last_used(Some(now - 90)), "1分钟前");
        assert_eq!(fmt_last_used(Some(now - 300)), "5分钟前");
        assert_eq!(fmt_last_used(Some(now - 5_000)), "1小时前");
        assert_eq!(fmt_last_used(Some(now - 3 * 86_400)), "3天前");
        // A timestamp in the future degrades gracefully (saturating diff).
        assert_eq!(fmt_last_used(Some(now + 60)), "刚刚");
    }

    #[test]
    fn fmt_last_used_old_timestamps_render_local_date() {
        let now = crate::model::now_secs();
        let ts = now - 40 * 86_400;
        let out = fmt_last_used(Some(ts));
        assert_eq!(out.len(), 10, "expected YYYY-MM-DD, got {out:?}");
        assert_eq!(out.as_bytes()[4], b'-');
        assert_eq!(out.as_bytes()[7], b'-');
        // Mirror the implementation via chrono's own Display to confirm the
        // date part matches the local calendar date.
        let mirror: String = match chrono::Local.timestamp_opt(ts, 0) {
            chrono::MappedLocalTime::Single(dt) => dt.to_string().chars().take(10).collect(),
            _ => "-".into(),
        };
        assert_eq!(out, mirror);
    }

    #[test]
    fn truncate_respects_char_count_with_cjk() {
        assert_eq!(truncate("数据库连接管理", 4), "数据库…");
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("", 3), "");
        assert_eq!(truncate("你好", 1), "…");
        assert_eq!(truncate("你好世界", 0), "");
        // No byte-slicing panics on multi-byte chars.
        assert_eq!(truncate("a💾b💾c", 3), "a💾…");
    }

    #[test]
    fn help_lines_cover_main_keys_and_modes() {
        let joined = help_lines().join("\n");
        assert!(!help_lines().is_empty());
        for needle in [
            "新增主机",
            "编辑",
            "删除",
            "测试",
            "搜索",
            "导入",
            "排序",
            "最近使用",
            "Ctrl+S",
            "Ctrl+T",
            "Ctrl+R",
            "Tab",
            "Esc",
            "hosts.json",
            "有效主机永远排在无效主机之前",
        ] {
            assert!(
                joined.contains(needle),
                "help_lines() should mention {needle:?}"
            );
        }
    }
}
