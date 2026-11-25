//! Settings history: an append-only record of what each settings change altered (#1100).
//!
//! `settings-audit.jsonl` says who called what and how it ended, never the values. This file adds
//! the values: one JSON line per changed section, with its old and new value. The values come from
//! the redacted `settings_get` view, so no token or key can reach it. The newest [`MAX_ENTRIES`]
//! entries are kept; older ones are dropped when the file is rewritten after an append.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// How many entries the file keeps.
pub const MAX_ENTRIES: usize = 5000;

/// Sections of the `settings_get` view that are settings (not counters, paths or server info).
const SECTIONS: [&str; 8] = [
    "agents",
    "registry",
    "roles",
    "github",
    "rules",
    "workflow",
    "project_status",
    "registered_agents",
];

/// One change: `section` went from `old` to `new`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// When (Unix ms).
    pub at_ms: i64,
    /// Who changed it.
    pub actor: String,
    /// Whether they came through a shared connector.
    pub via_connector: bool,
    /// The settings operation.
    pub op: String,
    /// What changed: one of the sections of the settings view.
    pub section: String,
    /// The value before.
    pub old: Value,
    /// The value after.
    pub new: Value,
}

/// The settings of a `settings_get` view, without what changes on its own: a registered agent's
/// last-seen time, or a running sync's state.
#[must_use]
pub fn snapshot(view: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    for key in SECTIONS {
        let Some(v) = view.get(key) else { continue };
        let mut v = v.clone();
        if let Some(agents) = v.as_array_mut().filter(|_| key == "registered_agents") {
            for a in agents.iter_mut().filter_map(Value::as_object_mut) {
                a.remove("created_ms");
                a.remove("seen_ms");
            }
        }
        if let Some(g) = v.as_object_mut().filter(|_| key == "github") {
            g.remove("running");
        }
        out.insert(key.to_owned(), v);
    }
    out
}

/// The sections that differ between two snapshots, as `(section, old, new)`.
#[must_use]
pub fn changes(
    before: &Map<String, Value>,
    after: &Map<String, Value>,
) -> Vec<(String, Value, Value)> {
    SECTIONS
        .iter()
        .filter_map(|k| {
            let (old, new) = (before.get(*k), after.get(*k));
            (old != new).then(|| {
                (
                    (*k).to_owned(),
                    old.cloned().unwrap_or(Value::Null),
                    new.cloned().unwrap_or(Value::Null),
                )
            })
        })
        .collect()
}

/// Append `entries` to the file at `path` and keep only the newest [`MAX_ENTRIES`].
///
/// # Errors
/// The file could not be read or written.
pub fn append(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut lines = read_lines(path)?;
    for e in entries {
        lines.push(serde_json::to_string(e).map_err(std::io::Error::other)?);
    }
    let from = lines.len().saturating_sub(MAX_ENTRIES);
    let mut text = lines[from..].join("\n");
    text.push('\n');
    rusty_atomic_file::write(path, text.as_bytes())
}

/// The newest `limit` entries (of `section`, when given), oldest first. A missing file is empty.
///
/// # Errors
/// The file exists but could not be read.
pub fn read(path: &Path, limit: usize, section: Option<&str>) -> std::io::Result<Vec<Entry>> {
    let mut out: Vec<Entry> = read_lines(path)?
        .iter()
        .rev()
        .filter_map(|l| serde_json::from_str::<Entry>(l).ok())
        .filter(|e| section.is_none_or(|s| e.section == s))
        .take(limit)
        .collect();
    out.reverse();
    Ok(out)
}

fn read_lines(path: &Path) -> std::io::Result<Vec<String>> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(n: i64) -> Entry {
        Entry {
            at_ms: n,
            actor: "a".into(),
            via_connector: false,
            op: "settings_rules_set".into(),
            section: if n % 2 == 0 { "rules" } else { "roles" }.into(),
            old: json!(n),
            new: json!(n + 1),
        }
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mill_history_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir.join("h.jsonl")
    }

    #[test]
    fn only_changed_sections_are_reported_and_noise_is_ignored() {
        let a = snapshot(&json!({
            "rules": {"movement": true}, "roles": {"x": []}, "paths": {"a": 1},
            "registered_agents": [{"id": "i", "seen_ms": 1, "created_ms": 1}],
            "github": {"repo": "o/r", "running": false},
        }));
        let b = snapshot(&json!({
            "rules": {"movement": false}, "roles": {"x": []}, "paths": {"a": 2},
            "registered_agents": [{"id": "i", "seen_ms": 99, "created_ms": 1}],
            "github": {"repo": "o/r", "running": true},
        }));
        assert_eq!(
            changes(&a, &b),
            vec![(
                "rules".into(),
                json!({"movement": true}),
                json!({"movement": false})
            )]
        );
        assert!(changes(&a, &a).is_empty());
    }

    #[test]
    fn a_section_that_appears_has_a_null_old_value() {
        let a = snapshot(&json!({}));
        let b = snapshot(&json!({"github": {"repo": "o/r"}}));
        assert_eq!(
            changes(&a, &b),
            vec![("github".into(), Value::Null, json!({"repo": "o/r"}))]
        );
    }

    #[test]
    fn entries_read_back_oldest_first_and_filter_by_section() {
        let p = tmp("read");
        assert!(read(&p, 10, None).expect("missing is empty").is_empty());
        append(&p, &(1..=4).map(entry).collect::<Vec<_>>()).expect("append");
        let all = read(&p, 10, None).expect("read");
        assert_eq!(
            all.iter().map(|e| e.at_ms).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        let last2 = read(&p, 2, None).expect("read");
        assert_eq!(last2.iter().map(|e| e.at_ms).collect::<Vec<_>>(), [3, 4]);
        let rules = read(&p, 10, Some("rules")).expect("read");
        assert_eq!(rules.iter().map(|e| e.at_ms).collect::<Vec<_>>(), [2, 4]);
    }

    #[test]
    fn the_oldest_entries_are_dropped_past_the_cap() {
        let p = tmp("cap");
        let n = MAX_ENTRIES as i64;
        append(&p, &(1..=n).map(entry).collect::<Vec<_>>()).expect("fill");
        append(&p, &[entry(n + 1), entry(n + 2)]).expect("overflow");
        let all = read(&p, usize::MAX, None).expect("read");
        assert_eq!(all.len(), MAX_ENTRIES);
        assert_eq!(all.first().map(|e| e.at_ms), Some(3));
        assert_eq!(all.last().map(|e| e.at_ms), Some(n + 2));
    }
}
