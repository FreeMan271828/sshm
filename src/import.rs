//! Import hosts from `~/.ssh/config`.
//!
//! Contract frozen by Lead: `ImportReport` + `import_ssh_config(&mut Store)`,
//! plus `ParsedHost` / `parse_config_text` / `merge_parsed` for testability.
//!
//! # Grammar and documented edge-case decisions
//!
//! - Line-oriented parser, no external deps. CRLF is fine (`\r` is plain
//!   whitespace); lone-`\r` (classic Mac) line endings are not split on.
//! - An unquoted `#` starts a comment to end-of-line; inside double quotes
//!   it is literal. Double quotes group whitespace into one token; backslash
//!   and single quotes are literal (OpenSSH only treats `"` specially, so a
//!   single-quoted value like `'a b'` does NOT group — same as ssh).
//! - Keywords are case-insensitive (`hostname x` works), like OpenSSH.
//!   `=` is normalized in the keyword position: both `Port=22` and
//!   `Port = 22` work. `=` elsewhere is literal (hostnames cannot contain
//!   `=`, so this only diverges from OpenSSH on configs that are invalid
//!   anyway).
//! - Per block only `HostName` / `Port` / `User` are read. `IdentityFile`,
//!   `ProxyJump`, `ProxyCommand`, `ForwardAgent`, `StrictHostKeyChecking`,
//!   `Include` and anything unknown are ignored silently; `Include` counts
//!   nothing and warns nothing.
//! - Repeated directives: the last one wins. (OpenSSH keeps the first for
//!   some keywords; last-wins reads more naturally for an importer.)
//! - `Port` reuses `model::parse_port`: empty / non-numeric / 0 / > 65535
//!   all fall back to 22 without failing the import and without warning.
//! - `Host` names are kept as-is, only trimmed (ssh matches patterns
//!   case-insensitively; we don't lowercase). Names with `*`, `?` or `!` are
//!   wildcard patterns and are skipped one by one — the concrete names on the
//!   same `Host` line are still imported — each pattern counting once toward
//!   `skipped_wild` (a bare `Host *` is one name, so it counts once).
//!   `[`/`]` classes are not treated as wildcards (out of scope).
//! - Dedupe: a name is skipped as `skipped_dup` when a record that existed
//!   before its block has the same `host:port:username` (host compared
//!   case-insensitively, username exactly — same as `Store::contains_target`).
//!   Different names within one block are never duplicates of each other, so a
//!   multi-name `Host` line with one `HostName` still imports every name; a
//!   name repeated inside the same block (`Host dup dup`) imports once.
//! - A `Match` line closes the current block so the conditional directives
//!   under it don't leak into the previous `Host`. A `Host` nested inside a
//!   `Match` block is imported unconditionally (conditionals are out of
//!   scope and rare in real configs).
//! - Directives before the first `Host` block (globals) are ignored.
//! - Config location: Windows `%USERPROFILE%/.ssh/config` (falls back to
//!   `$HOME`), elsewhere `$HOME/.ssh/config` (falls back to `USERPROFILE`);
//!   the first existing candidate wins. A missing or unreadable file is a
//!   non-fatal `ImportReport.error`, never a panic.

use std::path::PathBuf;

use crate::model::{Host, Store, new_id, now_secs, parse_port};

/// Note stamped on every imported record.
const IMPORT_NOTE: &str = "来自 ~/.ssh/config";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Hosts actually added.
    pub added: usize,
    /// Skipped because host:port:user already exists in the store.
    pub skipped_dup: usize,
    /// Skipped wildcard patterns (`Host *.example.com` etc.).
    pub skipped_wild: usize,
    /// Non-fatal warning / fatal error description, if any.
    pub error: Option<String>,
}

/// One parsed `Host` block. `names` may still contain wildcard patterns —
/// `merge_parsed` decides what to skip and count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedHost {
    /// All names from the `Host` line, trimmed, in order.
    pub names: Vec<String>,
    /// `HostName` directive. `None`/empty = use the name itself.
    pub host_name: Option<String>,
    /// `Port` directive; invalid or missing → 22.
    pub port: u16,
    /// `User` directive. `None` = ssh default (empty username).
    pub user: Option<String>,
}

/// Merge ~/.ssh/config entries into the store.
///
/// - Parses `Host` blocks (skipping wildcard patterns `* ? !`).
/// - One `Host` line may name several hosts → one record per name.
/// - `HostName`/`Port`/`User` directives fill the record; missing values
///   fall back to the host name / 22 / local user.
/// - New records: `valid = true`, `password = None`, note = origin marker.
/// - Existing (host:port:user) entries are counted as `skipped_dup`.
/// - A missing or unreadable config file is a warning, not a hard error.
pub fn import_ssh_config(store: &mut Store) -> ImportReport {
    let Some(path) = ssh_config_path() else {
        return ImportReport {
            error: Some(format!(
                "未找到 ~/.ssh/config ({})",
                primary_ssh_config_path().display()
            )),
            ..ImportReport::default()
        };
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => merge_parsed(store, parse_config_text(&text)),
        Err(e) => ImportReport {
            error: Some(format!("读取 {} 失败: {e}", path.display())),
            ..ImportReport::default()
        },
    }
}

/// First existing candidate for `~/.ssh/config`, or `None` when neither
/// variable resolves to an existing file.
fn ssh_config_path() -> Option<PathBuf> {
    let keys: &[&str] = if cfg!(windows) {
        &["USERPROFILE", "HOME"]
    } else {
        &["HOME", "USERPROFILE"]
    };
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|dir| PathBuf::from(dir).join(".ssh").join("config"))
            .filter(|path| path.exists())
    })
}

/// Primary candidate path, used only for the "not found" error message.
fn primary_ssh_config_path() -> PathBuf {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var(key)
        .map(|dir| PathBuf::from(dir).join(".ssh").join("config"))
        .unwrap_or_else(|_| PathBuf::from("~/.ssh/config"))
}

/// Parse ssh config text into one `ParsedHost` per `Host` block.
///
/// Directives before any `Host` block are ignored, like global directives.
/// Wildcard names are kept in `names`; `merge_parsed` skips and counts them.
pub(crate) fn parse_config_text(text: &str) -> Vec<ParsedHost> {
    let mut hosts: Vec<ParsedHost> = Vec::new();
    let mut cur: Option<ParsedHost> = None;
    for raw in text.lines() {
        let tokens = tokenize(raw);
        let (Some(kw), args) = split_keyword(tokens) else {
            continue;
        };
        match kw.as_str() {
            "host" => {
                if let Some(h) = cur.take() {
                    hosts.push(h);
                }
                // `Host` with no names matches nothing in ssh too: skip.
                if args.is_empty() {
                    continue;
                }
                cur = Some(ParsedHost {
                    names: args,
                    host_name: None,
                    port: 22,
                    user: None,
                });
            }
            "hostname" => {
                if let Some(h) = cur.as_mut() {
                    h.host_name = args.first().cloned();
                }
            }
            "port" => {
                if let Some(h) = cur.as_mut() {
                    // `model::parse_port`: "" → 22, invalid/0/overflow → Err → 22.
                    h.port = args
                        .first()
                        .map(|s| parse_port(s).unwrap_or(22))
                        .unwrap_or(22);
                }
            }
            "user" => {
                if let Some(h) = cur.as_mut() {
                    h.user = args.first().cloned();
                }
            }
            // Conditional blocks can't be modeled: close the current block so
            // the directives under `Match` don't leak into the previous one.
            "match" => {
                if let Some(h) = cur.take() {
                    hosts.push(h);
                }
            }
            // include / identityfile / proxyjump / proxycommand /
            // forwardagent / stricthostkeychecking / unknown: ignored.
            _ => {}
        }
    }
    if let Some(h) = cur.take() {
        hosts.push(h);
    }
    hosts
}

/// Merge parsed blocks into the store.
///
/// Dedupe uses the exact `contains_target` comparison (host case-folded,
/// port equal, username exact) but windowed to records that existed before
/// the **current block**: `Host web1 web2 web3` with one `HostName` must
/// yield one record per name even though they share an endpoint, while a
/// second `Host` block (or a re-import) pointing at that endpoint is a
/// duplicate.
pub(crate) fn merge_parsed(store: &mut Store, parsed: Vec<ParsedHost>) -> ImportReport {
    let mut report = ImportReport::default();
    for block in parsed {
        // Wildcard patterns are skipped name by name, never whole-block: in
        // `Host prod *.example.com` the pattern can only be expanded by ssh,
        // but `prod` is a concrete alias and must be imported. Each pattern
        // itself counts once toward `skipped_wild`.
        let before = store.hosts.len();
        // Names repeated inside one block (`Host dup dup`) would otherwise all
        // insert, because the dedupe window deliberately excludes this block.
        let mut seen_in_block: Vec<(String, String, u16, String)> = Vec::new();
        for name in &block.names {
            if is_wildcard(name) {
                report.skipped_wild += 1;
                continue;
            };
            let host = block
                .host_name
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| name.clone());
            let username = block.user.clone().unwrap_or_default();
            let duplicate = store.hosts[..before].iter().any(|h| {
                h.host.eq_ignore_ascii_case(&host) && h.port == block.port && h.username == username
            }) || seen_in_block.iter().any(|(a, h, p, u)| {
                a.eq_ignore_ascii_case(name)
                    && h.eq_ignore_ascii_case(&host)
                    && *p == block.port
                    && *u == username
            });
            if duplicate {
                report.skipped_dup += 1;
                continue;
            }
            seen_in_block.push((name.clone(), host.clone(), block.port, username.clone()));
            store.insert(Host {
                id: new_id(),
                alias: name.clone(),
                host,
                port: block.port,
                username,
                password: None,
                note: IMPORT_NOTE.to_string(),
                category: String::new(),
                valid: true,
                last_used_at: None,
                use_count: 0,
                created_at: now_secs(),
                updated_at: now_secs(),
            });
            report.added += 1;
        }
    }
    report
}

/// OpenSSH pattern metacharacters. `[`/`]` classes are out of contract.
fn is_wildcard(name: &str) -> bool {
    name.chars().any(|c| c == '*' || c == '?' || c == '!')
}

/// Split one line into tokens. Double quotes group whitespace (and a `#`)
/// into one token; an unquoted `#` starts a comment to end-of-line. An
/// unterminated quote makes the rest of the line literal.
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for c in line.chars() {
        if !in_quotes && c == '#' {
            break;
        }
        if c == '"' {
            in_quotes = !in_quotes;
            continue;
        }
        if c.is_whitespace() && !in_quotes {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
            continue;
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// Split a token list into `(keyword, arguments)`. The keyword is
/// lowercased (ssh keywords are case-insensitive). `Port=22` attaches the
/// value to the keyword; a lone `=` token right after the keyword
/// (`Port = 22`) is dropped.
fn split_keyword(mut tokens: Vec<String>) -> (Option<String>, Vec<String>) {
    // Blank/comment-only lines tokenize to nothing; `drain(..1)` would panic.
    if tokens.is_empty() {
        return (None, Vec::new());
    }
    let first = tokens.remove(0);
    let (kw, attached) = match first.split_once('=') {
        Some((k, v)) if !k.is_empty() => (k.to_ascii_lowercase(), Some(v.to_string())),
        _ => (first.to_ascii_lowercase(), None),
    };
    let mut args = tokens;
    if let Some(v) = attached {
        if !v.is_empty() {
            args.insert(0, v);
        }
    } else if args.first().map(String::as_str) == Some("=") {
        args.remove(0);
    }
    (Some(kw), args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge_text(store: &mut Store, text: &str) -> ImportReport {
        merge_parsed(store, parse_config_text(text))
    }

    #[test]
    fn multi_name_host_line_creates_one_record_per_name() {
        let mut store = Store::new();
        let r = merge_text(
            &mut store,
            "Host web1 web2 web3\n    HostName 10.0.0.1\n    User root\n",
        );
        assert_eq!(
            r,
            ImportReport {
                added: 3,
                skipped_dup: 0,
                skipped_wild: 0,
                error: None
            }
        );
        assert_eq!(store.hosts.len(), 3);
        for (i, alias) in ["web1", "web2", "web3"].iter().enumerate() {
            let h = &store.hosts[i];
            assert_eq!(&h.alias, alias);
            assert_eq!(h.host, "10.0.0.1");
            assert_eq!(h.port, 22);
            assert_eq!(h.username, "root");
            assert_eq!(h.password, None);
            assert_eq!(h.note, IMPORT_NOTE);
            assert!(h.valid);
            assert_eq!(h.use_count, 0);
            assert_eq!(h.last_used_at, None);
            assert_eq!(h.category, "");
        }
    }

    #[test]
    fn hostname_port_user_override_and_defaults() {
        let mut store = Store::new();
        let text = "\
Host a
    HostName 192.168.1.10
    Port 2222
    User admin
Host b
    Port abc
Host c
";
        let r = merge_text(&mut store, text);
        assert_eq!(r.added, 3);
        let a = &store.hosts[0];
        assert_eq!(a.host, "192.168.1.10");
        assert_eq!(a.port, 2222);
        assert_eq!(a.username, "admin");
        let b = &store.hosts[1];
        assert_eq!(b.host, "b"); // missing HostName → name used
        assert_eq!(b.port, 22); // Port "abc" → 22
        assert_eq!(b.username, "");
        let c = &store.hosts[2];
        assert_eq!(c.host, "c");
        assert_eq!(c.port, 22);
        assert_eq!(c.username, "");
    }

    #[test]
    fn wildcard_block_skipped_and_counted() {
        let mut store = Store::new();
        let r = merge_text(
            &mut store,
            "Host *.example.com\n    HostName x\nHost concrete\n    HostName y\n",
        );
        assert_eq!(r.added, 1);
        assert_eq!(r.skipped_wild, 1);
        assert_eq!(store.hosts.len(), 1);
        assert_eq!(store.hosts[0].alias, "concrete");
    }

    #[test]
    fn mixed_concrete_wildcard_line_imports_the_concrete_name() {
        let mut store = Store::new();
        let r = merge_text(
            &mut store,
            "Host prod *.example.com\n    HostName 1.2.3.4\n",
        );
        assert_eq!(r.added, 1); // `prod` is concrete → imported
        assert_eq!(r.skipped_wild, 1); // one wildcard name → counted once
        assert_eq!(store.hosts.len(), 1);
        assert_eq!(store.hosts[0].alias, "prod");
        assert_eq!(store.hosts[0].host, "1.2.3.4");
    }

    #[test]
    fn multiple_wildcard_names_each_count() {
        let mut store = Store::new();
        let r = merge_text(&mut store, "Host *.a.com *.b.com\n");
        assert_eq!(r.added, 0);
        assert_eq!(r.skipped_wild, 2);
    }

    #[test]
    fn bare_star_counts_once() {
        let mut store = Store::new();
        let r = merge_text(&mut store, "Host *\n    HostName foo\n");
        assert_eq!(r.added, 0);
        assert_eq!(r.skipped_wild, 1);
    }

    #[test]
    fn second_import_is_all_duplicates() {
        let mut store = Store::new();
        let text = "Host a\n    HostName 10.0.0.1\n    User u\nHost b\n    HostName 10.0.0.2\n";
        let first = merge_text(&mut store, text);
        assert_eq!(first.added, 2);
        let second = merge_text(&mut store, text);
        assert_eq!(second.added, 0);
        assert_eq!(second.skipped_dup, 2);
        assert_eq!(store.hosts.len(), 2);
    }

    #[test]
    fn duplicates_within_one_run_are_skipped() {
        let mut store = Store::new();
        let text =
            "Host x\n    HostName 1.2.3.4\n    User u\nHost y\n    HostName 1.2.3.4\n    User u\n";
        let r = merge_text(&mut store, text);
        assert_eq!(r.added, 1);
        assert_eq!(r.skipped_dup, 1);
    }

    #[test]
    fn name_repeated_inside_one_block_is_inserted_once() {
        let mut store = Store::new();
        let r = merge_text(&mut store, "Host dup dup\n    HostName 10.0.0.9\n");
        assert_eq!(r.added, 1);
        assert_eq!(r.skipped_dup, 1);
        assert_eq!(store.hosts.len(), 1);
        assert_eq!(store.hosts[0].alias, "dup");
    }

    #[test]
    fn dedupe_host_is_case_insensitive() {
        let mut store = Store::new();
        merge_text(&mut store, "Host a\n    HostName SERVER\n    User u\n");
        let r = merge_text(&mut store, "Host b\n    HostName server\n    User u\n");
        assert_eq!(r.added, 0);
        assert_eq!(r.skipped_dup, 1);
    }

    #[test]
    fn comments_blanks_and_unknown_directives_ignored() {
        let mut store = Store::new();
        let text = "\
# top-level comment
Host srv # inline comment
    # inner comment
    HostName 10.0.0.2
    IdentityFile ~/.ssh/id_ed25519
    ProxyJump jump.example.com
    ProxyCommand ssh -W %h:%p jump
    ForwardAgent yes
    StrictHostKeyChecking no
    UnknownDirective whatever

";
        let r = merge_text(&mut store, text);
        assert_eq!(r.added, 1);
        assert_eq!(store.hosts[0].host, "10.0.0.2");
    }

    #[test]
    fn include_lines_ignored_and_count_nothing() {
        let mut store = Store::new();
        let r = merge_text(
            &mut store,
            "Include config.d/*.conf\nHost srv\n    HostName 10.0.0.2\n",
        );
        assert_eq!(
            r,
            ImportReport {
                added: 1,
                skipped_dup: 0,
                skipped_wild: 0,
                error: None
            }
        );
    }

    #[test]
    fn empty_and_blank_input_yields_nothing() {
        let mut store = Store::new();
        assert_eq!(merge_text(&mut store, ""), ImportReport::default());
        assert_eq!(
            merge_text(&mut store, "\n\n   \n# only comments\n"),
            ImportReport::default()
        );
    }

    #[test]
    fn crlf_case_insensitive_keywords_and_equals_forms() {
        let mut store = Store::new();
        let text = "HOST win\r\n    hostname=10.0.0.5\r\n    port = 2200\r\n    user=bob\r\n";
        let r = merge_text(&mut store, text);
        assert_eq!(r.added, 1);
        let h = &store.hosts[0];
        assert_eq!(h.alias, "win");
        assert_eq!(h.host, "10.0.0.5");
        assert_eq!(h.port, 2200);
        assert_eq!(h.username, "bob");
    }

    #[test]
    fn quoted_values_group_whitespace() {
        let mut store = Store::new();
        let text = "Host q\n    HostName \"my host\"\n    User \"my user\"\n";
        let r = merge_text(&mut store, text);
        assert_eq!(r.added, 1);
        assert_eq!(store.hosts[0].host, "my host");
        assert_eq!(store.hosts[0].username, "my user");
    }

    #[test]
    fn hash_inside_quotes_is_not_a_comment() {
        let mut store = Store::new();
        let r = merge_text(&mut store, "Host h\n    User \"ro#ot\"\n");
        assert_eq!(r.added, 1);
        assert_eq!(store.hosts[0].username, "ro#ot");
    }

    #[test]
    fn directives_before_any_host_block_ignored() {
        let parsed = parse_config_text("HostName orphan\nPort 1234\nUser ghost\n");
        assert!(parsed.is_empty());
    }

    #[test]
    fn match_block_closes_current_block() {
        let mut store = Store::new();
        let text = "\
Host a
    HostName 1.1.1.1
Match host b
    HostName 2.2.2.2
Host c
    HostName 3.3.3.3
";
        let r = merge_text(&mut store, text);
        assert_eq!(r.added, 2);
        assert_eq!(store.hosts[0].host, "1.1.1.1"); // not leaked 2.2.2.2
        assert_eq!(store.hosts[1].host, "3.3.3.3");
    }

    #[test]
    fn last_duplicate_directive_wins() {
        let mut store = Store::new();
        let text = "\
Host dup
    HostName first
    Port 22
    User u1
    HostName second
    Port 2200
    User u2
";
        let r = merge_text(&mut store, text);
        assert_eq!(r.added, 1);
        let h = &store.hosts[0];
        assert_eq!(h.host, "second");
        assert_eq!(h.port, 2200);
        assert_eq!(h.username, "u2");
    }

    #[test]
    fn host_without_names_produces_nothing() {
        let parsed = parse_config_text("Host\nHost a\n    HostName x\n");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].names, vec!["a".to_string()]);
    }

    #[test]
    fn port_zero_and_overflow_treated_as_invalid() {
        let mut store = Store::new();
        let r = merge_text(
            &mut store,
            "Host p0\n    Port 0\nHost big\n    Port 70000\n",
        );
        assert_eq!(r.added, 2);
        assert_eq!(store.hosts[0].port, 22);
        assert_eq!(store.hosts[1].port, 22);
    }

    #[test]
    fn names_and_hostname_kept_as_is() {
        let mut store = Store::new();
        let r = merge_text(&mut store, "Host MiXeD\n    HostName CaseHost\n");
        assert_eq!(r.added, 1);
        assert_eq!(store.hosts[0].alias, "MiXeD");
        assert_eq!(store.hosts[0].host, "CaseHost");
    }
}
