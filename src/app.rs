//! Application state machine: modes, forms, dialogs, key handling, actions.
//!
//! `App` is the single source of truth for UI state. `main.rs` owns the
//! terminal, event loop, and side effects (save, launch, test threads);
//! `ui/` only renders `&App` and `ui::draw` never mutates it.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;

use crate::model::{Host, Store, new_id, now_secs, parse_port, validate};
use crate::ssh::{ProbeTarget, TestOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    List,
    Form,
    Help,
}

#[derive(Debug, Clone)]
pub enum Dialog {
    ConfirmDelete { id: String },
    ConfirmDiscard,
    Info { title: String, body: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Alias,
    Host,
    Port,
    Username,
    Password,
    Note,
    Category,
    Valid,
}

impl Field {
    pub const ALL: [Field; 8] = [
        Field::Alias,
        Field::Host,
        Field::Port,
        Field::Username,
        Field::Password,
        Field::Note,
        Field::Category,
        Field::Valid,
    ];

    pub fn next(self) -> Field {
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Field {
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Field::Alias => "别名",
            Field::Host => "IP/主机",
            Field::Port => "端口",
            Field::Username => "账号",
            Field::Password => "密码",
            Field::Note => "备注",
            Field::Category => "分类",
            Field::Valid => "是否有效",
        }
    }

    /// Max input length for text fields.
    pub fn max_len(self) -> usize {
        match self {
            Field::Port => 5,
            Field::Password => 512,
            _ => 128,
        }
    }
}

/// Form state for add/edit. `port` is kept as a string so typing is free;
/// parsed with `model::parse_port` on save.
#[derive(Debug, Clone)]
pub struct FormState {
    /// `Some(id)` = editing existing host, `None` = adding new.
    pub editing: Option<String>,
    /// Was the edited host invalid before? Drives the "defaults back to
    /// valid on save" hint required by spec.
    pub was_invalid: bool,
    pub alias: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: String,
    pub note: String,
    pub category: String,
    pub valid: bool,
    pub focus: Field,
    pub reveal_password: bool,
    pub error: Option<String>,
    /// Result of the last "test connection" run from inside this form.
    pub last_test: Option<TestOutcome>,
}

impl FormState {
    pub fn new(editing: Option<&Host>) -> Self {
        let (editing, was_invalid, alias, host, port, username, password, note, category) =
            match editing {
                Some(h) => (
                    Some(h.id.clone()),
                    !h.valid,
                    h.alias.clone(),
                    h.host.clone(),
                    h.port.to_string(),
                    h.username.clone(),
                    h.password.clone().unwrap_or_default(),
                    h.note.clone(),
                    h.category.clone(),
                ),
                None => (
                    None,
                    false,
                    String::new(),
                    String::new(),
                    // Spec: port defaults to 22. Prefill it so the default is
                    // visible in the form instead of hiding behind an empty box.
                    "22".to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
            };
        Self {
            editing,
            was_invalid,
            alias,
            host,
            port,
            username,
            password,
            note,
            category,
            // Spec: a newly added or newly edited host defaults to valid —
            // including editing a previously invalid one.
            valid: true,
            focus: Field::Host,
            reveal_password: false,
            error: None,
            last_test: None,
        }
    }

    /// Does the form differ from a pristine state? Used for discard confirm.
    pub fn is_dirty(&self, store: &Store) -> bool {
        match &self.editing {
            Some(id) => match store.get(id) {
                Some(h) => {
                    self.alias != h.alias
                        || self.host != h.host
                        || self.port != h.port.to_string()
                        || self.username != h.username
                        || self.password != h.password.clone().unwrap_or_default()
                        || self.note != h.note
                        || self.category != h.category
                        || self.valid != h.valid
                }
                None => true,
            },
            None => {
                !(self.alias.is_empty()
                    && self.host.is_empty()
                    && self.port == "22"
                    && self.username.is_empty()
                    && self.password.is_empty()
                    && self.note.is_empty()
                    && self.category.is_empty()
                    && self.valid)
            }
        }
    }

    pub fn probe_target(&self) -> Result<ProbeTarget, String> {
        let port = parse_port(&self.port)?;
        validate(&self.host, port, &self.username, &self.alias)?;
        Ok(ProbeTarget {
            host: self.host.trim().to_string(),
            port,
            username: self.username.trim().to_string(),
            password: if self.password.is_empty() {
                None
            } else {
                Some(self.password.clone())
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestCtx {
    List,
    Form,
}

#[derive(Debug, Clone)]
pub enum TestState {
    Idle,
    Running,
    Done { ctx: TestCtx, outcome: TestOutcome },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    /// Save the store to disk.
    Save,
    /// Launch an ssh session for this host (main suspends the terminal).
    Launch(Host),
    /// Kick off an async connection test (main owns the receiver).
    StartTest(TestCtx, ProbeTarget),
    /// Import hosts from ~/.ssh/config (main runs the import).
    Import,
    /// Reload the store from disk, discarding in-memory changes.
    RefreshFromDisk,
    Quit,
}

#[derive(Debug)]
pub struct App {
    pub store: Store,
    pub mode: Mode,
    /// Index into the *visible* (sorted+filtered) list.
    pub selected: usize,
    pub table_state: TableState,
    pub filter: String,
    pub searching: bool,
    pub form: Option<FormState>,
    pub dialog: Option<Dialog>,
    pub toast: Option<(String, Instant)>,
    pub test: TestState,
    pub running: bool,
    pub help_scroll: u16,
    pub last_launched: Option<String>,
}

impl App {
    pub fn new(store: Store) -> Self {
        let mut app = Self {
            store,
            mode: Mode::List,
            selected: 0,
            table_state: TableState::default(),
            filter: String::new(),
            searching: false,
            form: None,
            dialog: None,
            toast: None,
            test: TestState::Idle,
            running: true,
            help_scroll: 0,
            last_launched: None,
        };
        app.sync_selection();
        app
    }

    // ---- list accessors -------------------------------------------------

    pub fn visible_ids(&self) -> Vec<String> {
        self.store.visible_ids(&self.filter)
    }

    pub fn visible_len(&self) -> usize {
        self.store.visible_ids(&self.filter).len()
    }

    pub fn selected_id(&self) -> Option<String> {
        self.visible_ids().get(self.selected).cloned()
    }

    pub fn selected_host(&self) -> Option<Host> {
        self.selected_id()
            .and_then(|id| self.store.get(&id).cloned())
    }

    /// Re-anchor selection to a valid index and update the table state.
    fn sync_selection(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            self.selected = 0;
            self.table_state.select(None);
            return;
        }
        if self.selected >= len {
            self.selected = len - 1;
        }
        self.table_state.select(Some(self.selected));
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub fn tick(&mut self) {
        if let Some((_, at)) = &self.toast
            && at.elapsed().as_secs() >= 5
        {
            self.toast = None;
        }
    }

    // ---- sort / filter --------------------------------------------------

    pub fn toggle_sort(&mut self) {
        self.store.sort_mode = self.store.sort_mode.next();
        self.toast(format!("排序: {}", self.store.sort_mode.label()));
    }

    fn set_filter(&mut self, f: String) {
        self.filter = f;
        self.selected = 0;
        self.sync_selection();
    }

    // ---- form ------------------------------------------------------------

    pub fn open_add(&mut self) {
        self.form = Some(FormState::new(None));
        self.mode = Mode::Form;
    }

    pub fn open_edit(&mut self) {
        let host = self.selected_host();
        self.form = Some(FormState::new(host.as_ref()));
        self.mode = Mode::Form;
    }

    pub fn close_form(&mut self) {
        self.form = None;
        self.mode = Mode::List;
        self.sync_selection();
    }

    /// Validate + commit the form into the store. Returns the saved host.
    pub fn apply_form(&mut self) -> Result<Host, String> {
        let form = self.form.clone().ok_or("没有打开的编辑表单")?;
        let port = parse_port(&form.port)?;
        validate(&form.host, port, &form.username, &form.alias)?;

        let now = now_secs();
        let (id, created_at, last_used_at, use_count) = match &form.editing {
            Some(id) => {
                let old = self.store.get(id).ok_or("编辑的主机不存在")?;
                (id.clone(), old.created_at, old.last_used_at, old.use_count)
            }
            None => (new_id(), now, None, 0),
        };

        let host = Host {
            id,
            alias: form.alias.trim().to_string(),
            host: form.host.trim().to_string(),
            port,
            username: form.username.trim().to_string(),
            password: if form.password.is_empty() {
                None
            } else {
                Some(form.password.clone())
            },
            note: form.note.trim().to_string(),
            category: form.category.trim().to_string(),
            valid: form.valid,
            last_used_at,
            use_count,
            created_at,
            updated_at: now,
        };

        let id = host.id.clone();
        if form.editing.is_some() {
            self.store.replace(host);
        } else {
            self.store.insert(host);
        }
        self.form = None;
        self.mode = Mode::List;
        self.sync_selection();
        Ok(self.store.get(&id).cloned().expect("just saved host"))
    }

    // ---- mutations (each caller persists after) --------------------------

    pub fn delete_selected(&mut self) -> Option<String> {
        let id = self.selected_id()?;
        self.dialog = Some(Dialog::ConfirmDelete { id });
        None
    }

    pub fn confirm_delete(&mut self) -> Option<String> {
        let Dialog::ConfirmDelete { id } = self.dialog.clone()? else {
            return None;
        };
        self.dialog = None;
        if self.store.remove(&id) {
            self.sync_selection();
            Some(id)
        } else {
            None
        }
    }

    pub fn toggle_valid_selected(&mut self) -> Option<(String, bool)> {
        let id = self.selected_id()?;
        if let Some(h) = self.store.get_mut(&id) {
            h.valid = !h.valid;
            h.updated_at = now_secs();
            Some((id, h.valid))
        } else {
            None
        }
    }

    pub fn record_use(&mut self, id: &str) {
        self.store.record_use(id);
        self.last_launched = Some(id.to_string());
    }

    pub fn apply_test_outcome(&mut self, ctx: TestCtx, outcome: TestOutcome) {
        if let Some(form) = self.form.as_mut() {
            form.last_test = Some(outcome.clone());
        }
        self.test = TestState::Done { ctx, outcome };
    }

    // ---- key handling -----------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        // Ctrl+C is the escape hatch even while a dialog is up.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Action::Quit;
        }
        // A modal dialog owns the keyboard regardless of the underlying mode:
        // `ConfirmDiscard` is raised from the form, `ConfirmDelete` from the list.
        if self.dialog.is_some() {
            return self.on_key_dialog(key);
        }
        match &self.mode {
            Mode::List => self.on_key_list(key),
            Mode::Form => self.on_key_form(key),
            Mode::Help => self.on_key_help(key),
        }
    }

    fn on_key_dialog(&mut self, key: KeyEvent) -> Action {
        let Some(dialog) = self.dialog.clone() else {
            return Action::None;
        };
        let confirm = matches!(
            key.code,
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter
        );
        self.dialog = None;
        match dialog {
            Dialog::ConfirmDelete { .. } => {
                if confirm {
                    if self.confirm_delete().is_some() {
                        self.toast("已删除");
                        self.sync_selection();
                    }
                    Action::Save
                } else {
                    Action::None
                }
            }
            Dialog::ConfirmDiscard => {
                if confirm {
                    self.close_form();
                }
                Action::None
            }
            Dialog::Info { .. } => Action::None,
        }
    }

    fn on_key_list(&mut self, key: KeyEvent) -> Action {
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.set_filter(String::new());
                }
                KeyCode::Enter => {
                    self.searching = false;
                }
                KeyCode::Char(c) => {
                    let mut f = self.filter.clone();
                    f.push(c);
                    self.set_filter(f);
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.set_filter(self.filter.clone());
                }
                _ => {}
            }
            return Action::None;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
            KeyCode::Char('/') => {
                self.searching = true;
                Action::None
            }
            KeyCode::Char('s') => {
                self.toggle_sort();
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_selection(1);
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_selection(-1);
                Action::None
            }
            KeyCode::Char('g') => {
                self.selected = 0;
                self.sync_selection();
                Action::None
            }
            KeyCode::Char('G') => {
                self.selected = self.visible_len().saturating_sub(1);
                self.sync_selection();
                Action::None
            }
            KeyCode::PageDown => {
                self.move_selection(10);
                Action::None
            }
            KeyCode::PageUp => {
                self.move_selection(-10);
                Action::None
            }
            KeyCode::Enter => match self.selected_host() {
                Some(host) => Action::Launch(host),
                None => Action::None,
            },
            KeyCode::Char('a') => {
                self.open_add();
                Action::None
            }
            KeyCode::Char('e') => {
                self.open_edit();
                Action::None
            }
            KeyCode::Char('d') => {
                self.delete_selected();
                Action::None
            }
            KeyCode::Char('x') => {
                if let Some((id, valid)) = self.toggle_valid_selected() {
                    self.toast(format!(
                        "{} 已标记为{}",
                        id_short(&id),
                        if valid { "有效" } else { "无效" }
                    ));
                    return Action::Save;
                }
                Action::None
            }
            KeyCode::Char('t') => match self.selected_host() {
                Some(host) => self.start_test(TestCtx::List, &host),
                None => Action::None,
            },
            KeyCode::Char('i') => Action::Import,
            KeyCode::Char('r') => Action::RefreshFromDisk,
            KeyCode::Char('?') | KeyCode::F(1) => {
                self.mode = Mode::Help;
                self.help_scroll = 0;
                Action::None
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
            _ => Action::None,
        }
    }

    fn start_test(&mut self, ctx: TestCtx, host: &Host) -> Action {
        let target = ProbeTarget {
            host: host.host.clone(),
            port: host.port,
            username: host.username.clone(),
            password: host.password.clone(),
        };
        self.test = TestState::Running;
        Action::StartTest(ctx, target)
    }

    fn move_selection(&mut self, delta: i32) {
        let len = self.visible_len();
        if len == 0 {
            return;
        }
        let cur = self.selected as i64;
        let next = (cur + delta as i64).rem_euclid(len as i64);
        self.selected = next as usize;
        self.sync_selection();
    }

    fn on_key_form(&mut self, key: KeyEvent) -> Action {
        let Some(form) = self.form.as_mut() else {
            self.mode = Mode::List;
            return Action::None;
        };

        match key.code {
            KeyCode::Esc => {
                if form.is_dirty(&self.store) {
                    self.dialog = Some(Dialog::ConfirmDiscard);
                } else {
                    self.close_form();
                }
                Action::None
            }
            KeyCode::Tab => {
                form.error = None;
                form.focus = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    form.focus.prev()
                } else {
                    form.focus.next()
                };
                Action::None
            }
            KeyCode::Enter => {
                if form.focus == Field::Valid {
                    return self.save_form();
                }
                form.error = None;
                form.focus = form.focus.next();
                Action::None
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => self.save_form(),
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                form.reveal_password = !form.reveal_password;
                Action::None
            }
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                match form.probe_target() {
                    Ok(target) => {
                        form.error = None;
                        form.last_test = None;
                        self.test = TestState::Running;
                        Action::StartTest(TestCtx::Form, target)
                    }
                    Err(e) => {
                        form.error = Some(format!("无法测试: {e}"));
                        Action::None
                    }
                }
            }
            KeyCode::Char(' ') | KeyCode::Char('x') if form.focus == Field::Valid => {
                form.valid = !form.valid;
                Action::None
            }
            KeyCode::Up => {
                form.error = None;
                form.focus = form.focus.prev();
                Action::None
            }
            KeyCode::Down => {
                form.error = None;
                form.focus = form.focus.next();
                Action::None
            }
            KeyCode::Backspace => {
                let value = form.field_mut(form.focus);
                value.pop();
                Action::None
            }
            KeyCode::Char(c) => {
                let focus = form.focus;
                if focus == Field::Port {
                    if c.is_ascii_digit() && form.port.len() < Field::Port.max_len() {
                        form.port.push(c);
                    }
                } else {
                    let cap = focus.max_len();
                    let value = form.field_mut(focus);
                    if value.chars().count() < cap {
                        value.push(c);
                    }
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    fn save_form(&mut self) -> Action {
        match self.apply_form() {
            Ok(host) => {
                self.toast(format!("已保存 {}", host.display_name()));
                Action::Save
            }
            Err(e) => {
                if let Some(form) = self.form.as_mut() {
                    form.error = Some(e);
                }
                Action::None
            }
        }
    }

    fn on_key_help(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::F(1) => {
                self.mode = Mode::List;
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.help_scroll = self.help_scroll.saturating_add(1);
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.help_scroll = self.help_scroll.saturating_sub(1);
                Action::None
            }
            _ => Action::None,
        }
    }
}

impl FormState {
    pub fn field_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Alias => &mut self.alias,
            Field::Host => &mut self.host,
            Field::Port => &mut self.port,
            Field::Username => &mut self.username,
            Field::Password => &mut self.password,
            Field::Note => &mut self.note,
            Field::Category => &mut self.category,
            Field::Valid => &mut self.password, // unreachable; Valid is a bool
        }
    }
}

pub fn id_short(id: &str) -> String {
    if id.len() > 8 {
        id[..8].to_string()
    } else {
        id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SortMode;

    fn host(id: &str, valid: bool, last: Option<i64>, count: u32) -> Host {
        Host {
            id: id.into(),
            alias: String::new(),
            host: format!("host-{id}"),
            port: 22,
            username: "root".into(),
            password: None,
            note: String::new(),
            category: String::new(),
            valid,
            last_used_at: last,
            use_count: count,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn app_with(hosts: Vec<Host>) -> App {
        let mut store = Store::new();
        for h in hosts {
            store.insert(h);
        }
        App::new(store)
    }

    #[test]
    fn invalid_edit_defaults_to_valid() {
        let mut app = app_with(vec![host("1", false, None, 0)]);
        app.selected = 0;
        app.open_edit();
        let form = app.form.as_ref().unwrap();
        assert!(
            form.valid,
            "spec: editing an invalid config defaults to valid"
        );
        assert!(form.was_invalid);
    }

    #[test]
    fn new_form_defaults_to_valid() {
        let mut app = app_with(vec![]);
        app.open_add();
        assert!(app.form.as_ref().unwrap().valid);
    }

    #[test]
    fn apply_form_inserts_and_replaces() {
        let mut app = app_with(vec![host("1", true, None, 0)]);
        app.selected = 0;
        app.open_edit();
        {
            let f = app.form.as_mut().unwrap();
            f.host = "192.168.1.9".into();
            f.port = "2222".into();
        }
        let saved = app.apply_form().unwrap();
        assert_eq!(saved.host, "192.168.1.9");
        assert_eq!(saved.port, 2222);
        assert_eq!(app.store.hosts.len(), 1);
        assert_eq!(app.store.get("1").unwrap().host, "192.168.1.9");
    }

    #[test]
    fn apply_form_rejects_bad_input_and_keeps_form() {
        let mut app = app_with(vec![]);
        app.open_add();
        {
            let f = app.form.as_mut().unwrap();
            f.host = String::new();
            f.port = "0".into();
        }
        assert!(app.apply_form().is_err());
        assert!(app.form.is_some(), "form stays open with error");
    }

    #[test]
    fn toggle_valid_and_sort_invariant() {
        let mut app = app_with(vec![
            host("1", true, None, 0),
            host("2", false, None, 0),
            host("3", true, None, 0),
        ]);
        app.selected = 0;
        let (id, valid) = app.toggle_valid_selected().unwrap();
        assert_eq!(id, "1");
        assert!(!valid);
        // The remaining valid host sorts first no matter what; among the now
        // invalid ones the default (name) order decides.
        let ids = app.visible_ids();
        assert_eq!(ids, vec!["3".to_string(), "1".to_string(), "2".to_string()]);
        // Flip it back: "1" is valid again, so the two valid entries come
        // first and the still-invalid "2" drops to the bottom.
        app.selected = 1;
        app.toggle_valid_selected();
        assert_eq!(app.visible_ids(), vec!["1", "3", "2"]);
        // Reviving the last invalid entry restores plain name order.
        app.selected = 2;
        app.toggle_valid_selected();
        assert_eq!(app.visible_ids(), vec!["1", "2", "3"]);
    }

    #[test]
    fn recent_sort_puts_latest_use_first() {
        let mut app = app_with(vec![
            host("a", true, Some(100), 1),
            host("b", true, Some(300), 1),
            host("c", true, None, 0),
        ]);
        app.store.sort_mode = SortMode::Recent;
        let ids = app.visible_ids();
        assert_eq!(ids, vec!["b", "a", "c"]);
    }

    #[test]
    fn filter_shrinks_visible_and_resets_selection() {
        let mut app = app_with(vec![host("a", true, None, 0), host("b", true, None, 0)]);
        app.store.get_mut("a").unwrap().note = "数据库".into();
        app.set_filter("数据".into());
        assert_eq!(app.visible_ids(), vec!["a"]);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn selection_moves_wrap() {
        let mut app = app_with(vec![host("a", true, None, 0), host("b", true, None, 0)]);
        app.move_selection(1);
        assert_eq!(app.selected_id().unwrap(), "b");
        app.move_selection(1);
        assert_eq!(app.selected_id().unwrap(), "a");
        app.move_selection(-1);
        assert_eq!(app.selected_id().unwrap(), "b");
    }

    #[test]
    fn delete_flow_confirms_then_removes() {
        let mut app = app_with(vec![host("1", true, None, 0)]);
        app.selected = 0;
        app.delete_selected();
        assert!(matches!(app.dialog, Some(Dialog::ConfirmDelete { .. })));
        let removed = app.confirm_delete();
        assert_eq!(removed, Some("1".to_string()));
        assert!(app.store.hosts.is_empty());
    }

    #[test]
    fn password_empty_string_becomes_none() {
        let mut app = app_with(vec![]);
        app.open_add();
        {
            let f = app.form.as_mut().unwrap();
            f.host = "h".into();
            f.password = String::new();
        }
        let saved = app.apply_form().unwrap();
        assert_eq!(saved.password, None);
    }

    fn press(app: &mut App, code: KeyCode) -> Action {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn dirty_form_esc_asks_before_discarding_and_y_works() {
        let mut app = app_with(vec![host("1", true, None, 0)]);
        app.selected = 0;
        app.open_edit();
        app.form.as_mut().unwrap().note = "改动".into();

        assert_eq!(press(&mut app, KeyCode::Esc), Action::None);
        assert!(matches!(app.dialog, Some(Dialog::ConfirmDiscard)));

        // The dialog must consume keys: 'y' closes the form instead of typing
        // a 'y' into the focused field.
        press(&mut app, KeyCode::Char('y'));
        assert!(app.dialog.is_none());
        assert!(app.form.is_none());
        assert_eq!(app.mode, Mode::List);
        assert_eq!(app.store.get("1").unwrap().note, "");
    }

    #[test]
    fn discarding_a_dirty_form_can_be_cancelled() {
        let mut app = app_with(vec![host("1", true, None, 0)]);
        app.selected = 0;
        app.open_edit();
        app.form.as_mut().unwrap().note = "改动".into();
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('n'));
        assert!(app.dialog.is_none());
        assert!(
            app.form.is_some(),
            "any key other than y/Enter keeps editing"
        );
        assert_eq!(app.mode, Mode::Form);
    }

    #[test]
    fn ctrl_c_quits_even_with_a_dialog_open() {
        let mut app = app_with(vec![host("1", true, None, 0)]);
        app.delete_selected();
        let quit = app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(quit, Action::Quit);
    }
}
