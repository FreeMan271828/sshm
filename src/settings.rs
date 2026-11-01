//! Persisted user preferences (`hosts.json` → `settings`).
//!
//! Two global knobs:
//! - [`OpenMode`]: reuse the current terminal or open a new tab/window.
//! - [`Terminal`]: which terminal hosts a new session.
//!
//! Unknown values in a hand-edited file degrade to the defaults instead of
//! failing the whole load: a typo in a preference must never make the app
//! refuse to start (and thus never clobber the host list).

use serde::{Deserialize, Deserializer, Serialize};

/// Where an interactive ssh session runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenMode {
    /// Keep this terminal: the TUI suspends while ssh owns the screen.
    CurrentTab,
    /// Open a new terminal tab/window and keep the host list usable.
    #[default]
    NewTab,
}

impl OpenMode {
    /// Display order; the default comes first.
    pub const ALL: [OpenMode; 2] = [OpenMode::NewTab, OpenMode::CurrentTab];

    pub fn key(self) -> &'static str {
        match self {
            OpenMode::NewTab => "new_tab",
            OpenMode::CurrentTab => "current_tab",
        }
    }

    /// Parse a persisted key. `None` for anything unknown.
    pub fn from_key(raw: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|mode| mode.key() == raw)
    }

    pub fn label(self) -> &'static str {
        match self {
            OpenMode::NewTab => "新建标签页",
            OpenMode::CurrentTab => "当前标签页",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            OpenMode::NewTab => "在新终端页签中运行 ssh，列表保持可用",
            OpenMode::CurrentTab => "暂停 TUI，ssh 直接占用当前终端；退出后回到列表",
        }
    }

    pub fn cycle(self, forward: bool) -> Self {
        cycle(&Self::ALL, self, forward)
    }
}

/// Which terminal program hosts a new session.
///
/// The variant list is platform independent so a synced config still parses;
/// [`Terminal::choices`] exposes only the variants that make sense locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Terminal {
    /// Follow the terminal sshm itself is running in, else pick a sensible
    /// local default.
    #[default]
    Auto,
    /// Tabby — opens a tab in the running Tabby window.
    Tabby,
    /// WezTerm — opens a tab in the running WezTerm window (`wezterm cli`).
    #[serde(rename = "wezterm")]
    WezTerm,
    /// Windows Terminal (`wt`) — a tab in the current window.
    WindowsTerminal,
    /// PowerShell (`pwsh` when installed, else Windows PowerShell).
    #[serde(rename = "powershell")]
    PowerShell,
    /// cmd.exe.
    Cmd,
    /// bash: Git Bash / WSL on Windows, the system bash elsewhere.
    Bash,
    /// zsh (unix only).
    Zsh,
}

#[cfg(windows)]
const CHOICES: &[Terminal] = &[
    Terminal::Auto,
    Terminal::Tabby,
    Terminal::WezTerm,
    Terminal::WindowsTerminal,
    Terminal::PowerShell,
    Terminal::Cmd,
    Terminal::Bash,
];

#[cfg(not(windows))]
const CHOICES: &[Terminal] = &[
    Terminal::Auto,
    Terminal::Tabby,
    Terminal::WezTerm,
    Terminal::Bash,
    Terminal::Zsh,
];

impl Terminal {
    /// Selectable values on this platform, in display order.
    pub fn choices() -> &'static [Terminal] {
        CHOICES
    }

    pub fn key(self) -> &'static str {
        match self {
            Terminal::Auto => "auto",
            Terminal::Tabby => "tabby",
            Terminal::WezTerm => "wezterm",
            Terminal::WindowsTerminal => "windows_terminal",
            Terminal::PowerShell => "powershell",
            Terminal::Cmd => "cmd",
            Terminal::Bash => "bash",
            Terminal::Zsh => "zsh",
        }
    }

    pub fn from_key(raw: &str) -> Option<Self> {
        CHOICES.iter().copied().find(|term| term.key() == raw)
    }

    pub fn label(self) -> &'static str {
        match self {
            Terminal::Auto => "自动",
            Terminal::Tabby => "Tabby",
            Terminal::WezTerm => "WezTerm",
            Terminal::WindowsTerminal => "Windows Terminal",
            Terminal::PowerShell => "PowerShell",
            Terminal::Cmd => "cmd",
            Terminal::Bash => "bash",
            Terminal::Zsh => "zsh",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Terminal::Auto => {
                if cfg!(windows) {
                    "优先跟随当前所在终端（Tabby / WezTerm / WT），否则 PowerShell"
                } else {
                    "优先跟随当前所在终端（Tabby / WezTerm），否则自动选终端模拟器"
                }
            }
            Terminal::Tabby => "在运行中的 Tabby 窗口新开标签页（会弹一次确认）",
            Terminal::WezTerm => "在运行中的 WezTerm 窗口新开标签页（wezterm cli spawn）",
            Terminal::WindowsTerminal => "wt -w 0 nt：在当前 Windows Terminal 窗口新开标签页",
            Terminal::PowerShell => "在新 PowerShell 窗口中运行 ssh（pwsh 优先）",
            Terminal::Cmd => "在新 cmd 窗口中运行 ssh",
            Terminal::Bash => {
                if cfg!(windows) {
                    "在新 bash 窗口中运行 ssh（Git Bash / WSL）"
                } else {
                    "在新终端窗口中用 bash 运行 ssh"
                }
            }
            Terminal::Zsh => "在新终端窗口中用 zsh 运行 ssh",
        }
    }

    pub fn cycle(self, forward: bool) -> Self {
        cycle(CHOICES, self, forward)
    }
}

/// The user's global preferences, stored beside the host list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Where Enter opens a session. Default: a new tab.
    #[serde(default, deserialize_with = "de_open_mode")]
    pub open_mode: OpenMode,
    /// Which terminal hosts a new session. Default: auto-detect.
    #[serde(default, deserialize_with = "de_terminal")]
    pub terminal: Terminal,
}

impl Settings {
    /// Short one-line summary, used by tests/toasts.
    pub fn summary(self) -> String {
        format!("{} · {}", self.open_mode.label(), self.terminal.label())
    }
}

fn de_open_mode<'de, D>(deserializer: D) -> Result<OpenMode, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    Ok(OpenMode::from_key(&raw).unwrap_or_default())
}

fn de_terminal<'de, D>(deserializer: D) -> Result<Terminal, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    Ok(Terminal::from_key(&raw).unwrap_or_default())
}

fn cycle<T: Copy + PartialEq>(all: &[T], current: T, forward: bool) -> T {
    if all.is_empty() {
        return current;
    }
    match all.iter().position(|item| *item == current) {
        Some(index) if forward => all[(index + 1) % all.len()],
        Some(index) => all[(index + all.len() - 1) % all.len()],
        // A value that does not exist on this platform (e.g. a Windows
        // terminal in a config copied to Linux) restarts at the first choice.
        None => all[0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_new_tab_and_auto_terminal() {
        let settings = Settings::default();
        assert_eq!(settings.open_mode, OpenMode::NewTab);
        assert_eq!(settings.terminal, Terminal::Auto);
        assert_eq!(settings.summary(), "新建标签页 · 自动");
    }

    #[test]
    fn missing_settings_section_uses_defaults() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn roundtrip_uses_stable_snake_case_keys() {
        let settings = Settings {
            open_mode: OpenMode::CurrentTab,
            terminal: Terminal::WindowsTerminal,
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("\"open_mode\":\"current_tab\""), "{json}");
        assert!(json.contains("\"terminal\":\"windows_terminal\""), "{json}");
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);

        // Every variant key must round-trip exactly, including the ones serde's
        // snake_case would otherwise spell differently (`power_shell`).
        for term in Terminal::choices() {
            let raw = serde_json::to_string(term).unwrap();
            assert_eq!(raw, format!("\"{}\"", term.key()), "{term:?}");
            assert_eq!(serde_json::from_str::<Terminal>(&raw).unwrap(), *term);
        }
    }

    #[test]
    fn unknown_values_degrade_to_defaults_instead_of_failing() {
        let settings: Settings =
            serde_json::from_str(r#"{"open_mode":"sideways","terminal":"teletype"}"#).unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn from_key_roundtrips_every_selectable_value() {
        for mode in OpenMode::ALL {
            assert_eq!(OpenMode::from_key(mode.key()), Some(mode));
        }
        for term in Terminal::choices() {
            assert_eq!(Terminal::from_key(term.key()), Some(*term));
            assert!(!term.label().is_empty());
            assert!(!term.detail().is_empty());
        }
        assert_eq!(Terminal::from_key("nope"), None);
    }

    #[test]
    fn cycle_wraps_in_both_directions() {
        let first = OpenMode::ALL[0];
        let last = OpenMode::ALL[OpenMode::ALL.len() - 1];
        assert_eq!(last.cycle(true), first);
        assert_eq!(first.cycle(false), last);

        let terms = Terminal::choices();
        assert_eq!(terms[terms.len() - 1].cycle(true), terms[0]);
        assert_eq!(terms[0].cycle(false), terms[terms.len() - 1]);
    }

    #[test]
    fn a_terminal_from_another_platform_restarts_at_the_first_choice() {
        // Whichever variant is absent locally must not panic or stay stuck.
        let foreign = if cfg!(windows) {
            Terminal::Zsh
        } else {
            Terminal::WindowsTerminal
        };
        let cycled = foreign.cycle(true);
        assert_eq!(cycled, Terminal::choices()[0]);
    }

    #[test]
    fn settings_deserialize_with_defaults_for_missing_fields() {
        let settings: Settings = serde_json::from_str(r#"{"open_mode":"current_tab"}"#).unwrap();
        assert_eq!(settings.open_mode, OpenMode::CurrentTab);
        assert_eq!(settings.terminal, Terminal::Auto);
    }
}
