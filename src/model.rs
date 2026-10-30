//! Domain model: `Host` records, sorting, filtering, validation.

use serde::{Deserialize, Serialize};

/// One saved SSH endpoint.
///
/// Field order in the JSON file matches this struct. Keep names stable:
/// older saves must keep loading (serde defaults below guard missing keys).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    /// Stable unique id (uuid v4 hex, generated at creation).
    pub id: String,
    /// Optional human alias. Display falls back to `user@host` when empty.
    #[serde(default)]
    pub alias: String,
    /// IP address or hostname.
    pub host: String,
    /// SSH port.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Login username. May be empty (then the local user / ssh default applies).
    #[serde(default)]
    pub username: String,
    /// Plaintext password. `None` = no password saved (key/agent auth).
    #[serde(default)]
    pub password: Option<String>,
    /// Free-form note.
    #[serde(default)]
    pub note: String,
    /// Grouping category, may be empty.
    #[serde(default)]
    pub category: String,
    /// Whether the entry is valid/usable. Invalid entries always sort last.
    #[serde(default = "default_valid")]
    pub valid: bool,
    /// Unix seconds of last successful connect/launch. `None` = never used.
    #[serde(default)]
    pub last_used_at: Option<i64>,
    /// How many times this entry was launched.
    #[serde(default)]
    pub use_count: u32,
    /// Unix seconds at creation.
    #[serde(default)]
    pub created_at: i64,
    /// Unix seconds at last edit.
    #[serde(default)]
    pub updated_at: i64,
}

fn default_port() -> u16 {
    22
}
fn default_valid() -> bool {
    true
}

impl Host {
    /// The display name for list views: alias, else `user@host`, else bare host.
    pub fn display_name(&self) -> String {
        if !self.alias.trim().is_empty() {
            return self.alias.trim().to_string();
        }
        if self.username.is_empty() {
            self.host.clone()
        } else {
            format!("{}@{}", self.username, self.host)
        }
    }

    /// `host:port` target, with `:22` elided.
    pub fn target(&self) -> String {
        if self.port == 22 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// Normalized lowercase key used for case-insensitive ordering/filtering.
    pub fn search_key(&self) -> String {
        format!(
            "{} {} {} {} {} {} {}",
            self.alias,
            self.host,
            self.port,
            self.username,
            self.note,
            self.category,
            self.display_name()
        )
        .to_lowercase()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortMode {
    #[default]
    Default,
    Recent,
}

impl SortMode {
    pub fn next(self) -> Self {
        match self {
            SortMode::Default => SortMode::Recent,
            SortMode::Recent => SortMode::Default,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Default => "默认",
            SortMode::Recent => "最近使用",
        }
    }
}

/// Stable, pure ordering used everywhere (TUI list, `sshm list` CLI, tests).
///
/// Invariant: **valid entries always sort before invalid ones**, in every mode.
pub fn sorted_view(hosts: &[Host], mode: SortMode) -> Vec<&Host> {
    let mut v: Vec<&Host> = hosts.iter().collect();
    v.sort_by(|a, b| {
        b.valid.cmp(&a.valid).then_with(|| match mode {
            SortMode::Default => default_cmp(a, b),
            SortMode::Recent => recent_cmp(a, b),
        })
    });
    v
}

fn default_cmp(a: &Host, b: &Host) -> std::cmp::Ordering {
    let cat_a = if a.category.trim().is_empty() {
        "~"
    } else {
        a.category.trim()
    };
    let cat_b = if b.category.trim().is_empty() {
        "~"
    } else {
        b.category.trim()
    };
    cat_a
        .to_lowercase()
        .cmp(&cat_b.to_lowercase())
        .then_with(|| {
            a.display_name()
                .to_lowercase()
                .cmp(&b.display_name().to_lowercase())
        })
        .then_with(|| a.host.cmp(&b.host))
        .then_with(|| a.id.cmp(&b.id))
}

fn recent_cmp(a: &Host, b: &Host) -> std::cmp::Ordering {
    // Descending by last use; never-used sinks to the bottom; ties broken by
    // use count (desc), then name (asc) for stability.
    b.last_used_at
        .unwrap_or(i64::MIN)
        .cmp(&a.last_used_at.unwrap_or(i64::MIN))
        .then_with(|| b.use_count.cmp(&a.use_count))
        .then_with(|| {
            a.display_name()
                .to_lowercase()
                .cmp(&b.display_name().to_lowercase())
        })
        .then_with(|| a.id.cmp(&b.id))
}

/// Case-insensitive substring match over every searchable field.
pub fn matches_filter(host: &Host, needle: &str) -> bool {
    let n = needle.trim().to_lowercase();
    if n.is_empty() {
        return true;
    }
    host.search_key().contains(&n)
}

/// Validate a candidate host record. Returns `Err(human message)` on the
/// first failing field, in form order.
pub fn validate(host: &str, port: u16, username: &str, alias: &str) -> Result<(), String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("主机不能为空".into());
    }
    if host.len() > 253 {
        return Err("主机名过长".into());
    }
    if host.chars().any(char::is_whitespace) {
        return Err("主机不能包含空白字符".into());
    }
    if port == 0 {
        return Err("端口必须在 1-65535 之间".into());
    }
    if username.trim().chars().any(char::is_whitespace) {
        return Err("账号不能包含空白字符".into());
    }
    if alias.trim().len() > 64 {
        return Err("别名过长".into());
    }
    Ok(())
}

/// Parse a port string the same way the form does. `""` = default 22.
pub fn parse_port(s: &str) -> Result<u16, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(22);
    }
    s.parse::<u16>()
        .map_err(|_| "端口必须是 1-65535 的数字".to_string())
        .and_then(|p| {
            if p == 0 {
                Err("端口必须在 1-65535 之间".into())
            } else {
                Ok(p)
            }
        })
}

/// Unix seconds now.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `Host` storage container with the one place that owns insert/remove/use.
#[derive(Debug, Clone, Default)]
pub struct Store {
    pub hosts: Vec<Host>,
    pub sort_mode: SortMode,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: &str) -> Option<&Host> {
        self.hosts.iter().find(|h| h.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Host> {
        self.hosts.iter_mut().find(|h| h.id == id)
    }

    pub fn insert(&mut self, mut host: Host) {
        let now = now_secs();
        host.created_at = now;
        host.updated_at = now;
        self.hosts.push(host);
    }

    /// Replace an existing host by id; returns false when id is unknown.
    pub fn replace(&mut self, host: Host) -> bool {
        match self.get_mut(&host.id) {
            Some(slot) => {
                *slot = host;
                slot.updated_at = now_secs();
                true
            }
            None => false,
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.hosts.len();
        self.hosts.retain(|h| h.id != id);
        self.hosts.len() != before
    }

    /// Mark a host as used now (launched or connected).
    pub fn record_use(&mut self, id: &str) -> bool {
        match self.get_mut(id) {
            Some(h) => {
                h.last_used_at = Some(now_secs());
                h.use_count = h.use_count.saturating_add(1);
                h.updated_at = now_secs();
                true
            }
            None => false,
        }
    }

    /// Ids in display order (same ordering as `sorted_view`).
    pub fn sorted_ids(&self) -> Vec<String> {
        sorted_view(&self.hosts, self.sort_mode)
            .into_iter()
            .map(|h| h.id.clone())
            .collect()
    }

    /// Ids in display order, filtered by needle.
    pub fn visible_ids(&self, needle: &str) -> Vec<String> {
        self.sorted_ids()
            .into_iter()
            .filter(|id| {
                self.get(id)
                    .map(|h| matches_filter(h, needle))
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Duplicate check used by import: same host:port:username.
    pub fn contains_target(&self, host: &str, port: u16, username: &str) -> bool {
        self.hosts
            .iter()
            .any(|h| h.host.eq_ignore_ascii_case(host) && h.port == port && h.username == username)
    }
}

/// New unique id (uuid v4 hex without dashes).
pub fn new_id() -> String {
    uuid_simple()
}

// Tiny uuid v4 generator: avoids pulling in the uuid crate just for ids.
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // xorshift64* seeded with time + address entropy.
    let addr = std::sync::atomic::AtomicU64::new(0);
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let c = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut s = t ^ c.wrapping_mul(0x9E3779B97F4A7C15) ^ (&addr as *const _ as u64);
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let hex = |v: u64| format!("{:016x}", v);
    let a = next();
    let b = next();
    let bytes = format!("{}{}", hex(a), hex(b));
    // Set version (4) and variant bits.
    let mut chars: Vec<char> = bytes.chars().collect();
    chars[12] = '4';
    chars[16] = match chars[16] {
        '8' | '9' | 'a' | 'b' => chars[16],
        _ => '8',
    };
    chars.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(id: &str, host: &str, valid: bool, last: Option<i64>, count: u32, cat: &str) -> Host {
        Host {
            id: id.into(),
            alias: String::new(),
            host: host.into(),
            port: 22,
            username: "root".into(),
            password: None,
            note: String::new(),
            category: cat.into(),
            valid,
            last_used_at: last,
            use_count: count,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn valid_always_before_invalid_default_mode() {
        let hosts = vec![
            h("a", "z.invalid", false, Some(100), 5, "x"),
            h("b", "a.ok", true, None, 0, "z"),
            h("c", "b.ok", true, None, 0, "a"),
        ];
        let v = sorted_view(&hosts, SortMode::Default);
        let ids: Vec<_> = v.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["c", "b", "a"]);
    }

    #[test]
    fn recent_mode_orders_by_last_use_desc() {
        let hosts = vec![
            h("old", "old", true, Some(100), 9, ""),
            h("never", "never", true, None, 0, ""),
            h("new", "new", true, Some(200), 1, ""),
        ];
        let v = sorted_view(&hosts, SortMode::Recent);
        let ids: Vec<_> = v.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["new", "old", "never"]);
    }

    #[test]
    fn invalid_first_mode_invariant_holds_in_recent() {
        let hosts = vec![
            h("bad", "bad", false, Some(999), 99, ""),
            h("good", "good", true, None, 0, ""),
        ];
        let v = sorted_view(&hosts, SortMode::Recent);
        assert_eq!(v[0].id, "good");
        assert_eq!(v[1].id, "bad");
    }

    #[test]
    fn default_mode_sorts_by_category_then_name() {
        let mut hosts = vec![
            h("b1", "h1", true, None, 0, "b"),
            h("a2", "h2", true, None, 0, "a"),
            h("a1", "h3", true, None, 0, "a"),
            h("e1", "h4", true, None, 0, ""),
        ];
        // Give them explicit aliases so the tie-break inside category "a" is
        // decided by name, not by the generated `root@hN` display name.
        hosts.iter_mut().for_each(|x| x.alias = x.id.clone());
        let v = sorted_view(&hosts, SortMode::Default);
        let ids: Vec<_> = v.iter().map(|x| x.id.as_str()).collect();
        // category a first (a1, a2), then b, then empty-category e1 last.
        assert_eq!(ids, ["a1", "a2", "b1", "e1"]);
    }

    #[test]
    fn filter_matches_all_fields_case_insensitive() {
        let mut host = h("x", "10.0.0.5", true, None, 0, "prod");
        host.note = "数据库".into();
        host.username = "Admin".into();
        assert!(matches_filter(&host, "admin"));
        assert!(matches_filter(&host, "10.0.0"));
        assert!(matches_filter(&host, "prod"));
        assert!(matches_filter(&host, "数据库"));
        assert!(matches_filter(&host, "22"));
        assert!(!matches_filter(&host, "nothing"));
        assert!(matches_filter(&host, ""));
    }

    #[test]
    fn port_parsing_defaults_and_rejects() {
        assert_eq!(parse_port("").unwrap(), 22);
        assert_eq!(parse_port(" 2222 ").unwrap(), 2222);
        assert!(parse_port("0").is_err());
        assert!(parse_port("70000").is_err());
        assert!(parse_port("abc").is_err());
    }

    #[test]
    fn validation_checks() {
        assert!(validate("", 22, "u", "").is_err());
        assert!(validate("host", 0, "u", "").is_err());
        assert!(validate("ho st", 22, "u", "").is_err());
        assert!(validate("host", 22, "u u", "").is_err());
        assert!(validate("host", 22, "u", "").is_ok());
        assert!(validate("host", 22, "", "").is_ok());
    }

    #[test]
    fn replace_and_remove_and_record_use() {
        let mut store = Store::new();
        let mut host = h("1", "h", true, None, 0, "");
        store.insert(host.clone());
        host.note = "edited".into();
        assert!(store.replace(host));
        assert_eq!(store.get("1").unwrap().note, "edited");
        assert!(store.record_use("1"));
        assert_eq!(store.get("1").unwrap().use_count, 1);
        assert!(store.get("1").unwrap().last_used_at.is_some());
        assert!(store.remove("1"));
        assert!(!store.remove("1"));
    }

    #[test]
    fn ids_are_unique_and_stable_format() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
