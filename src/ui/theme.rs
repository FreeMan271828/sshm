//! Central theme: every style used by the UI lives here, so no raw colors
//! are scattered across the render code.

use ratatui::style::{Color, Modifier, Style};

/// App title bar text: bold cyan.
pub const TITLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
/// Dimmed / secondary text (invalid rows, hints, empty states, footer cmd).
pub const DIM: Style = Style::new().fg(Color::DarkGray);
/// Selected row / focused element: black on cyan.
pub const SELECTED: Style = Style::new().fg(Color::Black).bg(Color::Cyan);
/// Accent text: keys in keybindings, prompts, headers.
pub const ACCENT: Style = Style::new().fg(Color::Cyan);
/// Success (valid hosts, ok test results).
pub const SUCCESS: Style = Style::new().fg(Color::Green);
/// Errors (validation messages, failed tests).
pub const ERROR: Style = Style::new().fg(Color::Red);
/// Warnings / non-fatal hints.
pub const WARN: Style = Style::new().fg(Color::Yellow);
/// Borders around boxes and panels.
pub const BORDER: Style = Style::new().fg(Color::Blue);
/// Keys in keybinding lists: bold cyan.
pub const KEY: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
/// Block cursor in input fields: black on white.
pub const CURSOR: Style = Style::new().fg(Color::Black).bg(Color::White);
/// Background of the focused form input value.
pub const INPUT: Style = Style::new().bg(Color::DarkGray);
/// Full-screen dark overlay behind dialogs.
pub const OVERLAY: Style = Style::new().bg(Color::Black).fg(Color::DarkGray);
