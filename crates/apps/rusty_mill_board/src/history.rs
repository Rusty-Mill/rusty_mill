//! Settings history: an append-only record of what each settings change altered (#1100).
//!
//! `settings-audit.jsonl` says who called what and how it ended, never the values. This file adds
//! the values: one JSON line per changed section, with its old and new value. The values come from
//! the redacted `settings_get` view, so no token or key can reach it.
//!
//! Bounds, so a growing configuration cannot grow the file, a response or the work done while the
//! settings lock is held:
//! - the file keeps the newest [`MAX_ENTRIES`] entries and at most [`MAX_BYTES`] bytes; older
//!   entries are dropped when an append would pass either;
//! - an entry whose serialized form passes [`MAX_ENTRY_BYTES`] is kept without its values: `old`
//!   and `new` become `{"omitted_bytes": N}`, so who, when and which section are still recorded;
//! - a read returns the newest entries that fit in [`MAX_RESPONSE_BYTES`] (and `limit`), and says
//!   when it stopped short;
//! - a read looks at the last [`MAX_BYTES`] of the file only, however large the file is.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// How many entries the file keeps.
pub const MAX_ENTRIES: usize = 5000;

/// How many bytes the file keeps (newest entries first).
pub const MAX_BYTES: usize = 2 << 20;

/// The longest entry kept whole, in serialized bytes; a longer one loses its values.
pub const MAX_ENTRY_BYTES: usize = 64 << 10;

/// The most entry bytes one read returns.
pub const MAX_RESPONSE_BYTES: usize = 256 << 10;

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

/// `entry` as one line, with its values dropped if it passes [`MAX_ENTRY_BYTES`].
fn line(entry: &Entry) -> std::io::Result<String> {
    let json = |e: &Entry| serde_json::to_string(e).map_err(std::io::Error::other);
    let whole = json(entry)?;
    if whole.len() <= MAX_ENTRY_BYTES {
        return Ok(whole);
    }
    let omitted = |v: &Value| serde_json::json!({ "omitted_bytes": v.to_string().len() });
    json(&Entry {
        old: omitted(&entry.old),
        new: omitted(&entry.new),
        ..entry.clone()
    })
}

/// Append `entries` to the file at `path`, then keep only the newest [`MAX_ENTRIES`] entries
/// within [`MAX_BYTES`].
///
/// # Errors
/// The file could not be read or written.
pub fn append(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut lines = read_lines(path)?;
    for e in entries {
        lines.push(line(e)?);
    }
    let (mut from, mut bytes) = (lines.len().saturating_sub(MAX_ENTRIES), 0);
    // Walk back from the newest, so the budget is spent on the newest entries.
    let mut keep = lines.len() - from;
    for (n, l) in lines[from..].iter().rev().enumerate() {
        bytes += l.len() + 1;
        if bytes > MAX_BYTES {
            keep = n;
            break;
        }
    }
    from = lines.len() - keep;
    let mut text = lines[from..].join("\n");
    text.push('\n');
    rusty_atomic_file::write(path, text.as_bytes())
}

/// What a read found.
#[derive(Debug, PartialEq)]
pub struct Page {
    /// The newest entries, oldest first.
    pub entries: Vec<Entry>,
    /// Whether the response budget cut it short of `limit`.
    pub truncated: bool,
}

/// The newest `limit` entries (of `section`, when given) that fit in [`MAX_RESPONSE_BYTES`],
/// oldest first. A missing file is empty.
///
/// # Errors
/// The file exists but could not be read.
pub fn read(path: &Path, limit: usize, section: Option<&str>) -> std::io::Result<Page> {
    let (mut entries, mut bytes, mut truncated) = (Vec::new(), 0, false);
    for l in read_lines(path)?.iter().rev() {
        let Ok(e) = serde_json::from_str::<Entry>(l) else {
            continue;
        };
        if section.is_some_and(|s| e.section != s) {
            continue;
        }
        if entries.len() == limit {
            break;
        }
        bytes += l.len();
        if bytes > MAX_RESPONSE_BYTES {
            truncated = true;
            break;
        }
        entries.push(e);
    }
    entries.reverse();
    Ok(Page { entries, truncated })
}

/// The lines in the last [`MAX_BYTES`] of the file (plus one entry's room): a larger file, by
/// hand or from an older build, is read from its tail and never in full.
fn read_lines(path: &Path) -> std::io::Result<Vec<String>> {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let window = (MAX_BYTES + MAX_ENTRY_BYTES) as u64;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(window);
    file.seek(SeekFrom::Start(start))?;
    let mut raw = Vec::new();
    file.take(window).read_to_end(&mut raw)?;
    let text = String::from_utf8_lossy(&raw);
    let mut lines = text.lines();
    if start > 0 {
        lines.next(); // a line cut by the window
    }
    Ok(lines.filter(|l| !l.is_empty()).map(str::to_owned).collect())
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
        assert!(
            read(&p, 10, None)
                .expect("missing is empty")
                .entries
                .is_empty()
        );
        append(&p, &(1..=4).map(entry).collect::<Vec<_>>()).expect("append");
        let all = read(&p, 10, None).expect("read").entries;
        assert_eq!(
            all.iter().map(|e| e.at_ms).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        let last2 = read(&p, 2, None).expect("read").entries;
        assert_eq!(last2.iter().map(|e| e.at_ms).collect::<Vec<_>>(), [3, 4]);
        let rules = read(&p, 10, Some("rules")).expect("read").entries;
        assert_eq!(rules.iter().map(|e| e.at_ms).collect::<Vec<_>>(), [2, 4]);
    }

    #[test]
    fn the_oldest_entries_are_dropped_past_the_cap() {
        let p = tmp("cap");
        let n = MAX_ENTRIES as i64;
        append(&p, &(1..=n).map(entry).collect::<Vec<_>>()).expect("fill");
        append(&p, &[entry(n + 1), entry(n + 2)]).expect("overflow");
        let ids = kept(&p);
        assert_eq!(ids.len(), MAX_ENTRIES);
        assert_eq!(ids.first(), Some(&3));
        assert_eq!(ids.last(), Some(&(n + 2)));
    }

    /// The `at_ms` of every entry the file holds, oldest first.
    fn kept(p: &std::path::Path) -> Vec<i64> {
        std::fs::read_to_string(p)
            .expect("file")
            .lines()
            .map(|l| serde_json::from_str::<Entry>(l).expect("entry").at_ms)
            .collect()
    }

    fn sized(n: i64, bytes: usize) -> Entry {
        Entry {
            old: json!("o".repeat(bytes)),
            new: json!("n".repeat(bytes)),
            ..entry(n)
        }
    }

    #[test]
    fn the_file_stays_within_its_byte_budget_and_keeps_the_newest() {
        let p = tmp("bytes");
        // 40 KB entries (under the entry cap): about 25 fit in 2 MiB.
        for n in 1..=100 {
            append(&p, &[sized(n, 20_000)]).expect("append");
        }
        let size = std::fs::metadata(&p).expect("file").len() as usize;
        assert!(size <= MAX_BYTES, "{size} bytes");
        let ids = kept(&p);
        assert_eq!(ids.last(), Some(&100));
        assert!(ids.len() > 20, "{}", ids.len());
        assert!(ids.windows(2).all(|w| w[1] == w[0] + 1), "no gaps: {ids:?}");
    }

    #[test]
    fn an_entry_past_the_entry_cap_keeps_who_and_when_but_not_its_values() {
        let p = tmp("oversized");
        append(&p, &[sized(1, MAX_ENTRY_BYTES), entry(2)]).expect("append");
        let all = read(&p, 10, None).expect("read").entries;
        assert_eq!(all.len(), 2);
        let (big, small) = (&all[0], &all[1]);
        assert_eq!((big.actor.as_str(), big.section.as_str()), ("a", "roles"));
        assert_eq!(big.old, json!({"omitted_bytes": MAX_ENTRY_BYTES + 2}));
        assert_eq!(big.new, json!({"omitted_bytes": MAX_ENTRY_BYTES + 2}));
        assert_eq!(small.old, json!(2), "a small entry is untouched");
    }

    #[test]
    fn an_entry_just_under_the_cap_is_kept_whole() {
        let p = tmp("under");
        append(&p, &[sized(1, 30_000)]).expect("append");
        assert_eq!(
            read(&p, 1, None).expect("read").entries[0].old,
            json!("o".repeat(30_000))
        );
    }

    #[test]
    fn a_read_stops_at_the_response_budget_and_says_so() {
        let p = tmp("response");
        for n in 1..=20 {
            append(&p, &[sized(n, 20_000)]).expect("append");
        }
        let page = read(&p, 500, None).expect("read");
        let bytes: usize = page
            .entries
            .iter()
            .map(|e| serde_json::to_string(e).expect("json").len())
            .sum();
        assert!(bytes <= MAX_RESPONSE_BYTES, "{bytes} bytes");
        assert!(page.truncated);
        assert_eq!(
            page.entries.last().map(|e| e.at_ms),
            Some(20),
            "the newest are kept"
        );
        assert!(!read(&p, 2, None).expect("read").truncated);
    }

    #[test]
    fn a_file_far_past_the_budget_is_read_from_its_tail() {
        let p = tmp("tail");
        let junk: String = (1..=3000)
            .map(|n| serde_json::to_string(&sized(n, 5_000)).expect("json") + "\n")
            .collect();
        assert!(junk.len() > 4 * MAX_BYTES);
        std::fs::write(&p, junk).expect("write");
        let page = read(&p, 5, None).expect("read");
        assert_eq!(page.entries.last().map(|e| e.at_ms), Some(3000));
        assert_eq!(page.entries.len(), 5);
        append(&p, &[entry(3001)]).expect("append trims the oversized file");
        assert!(std::fs::metadata(&p).expect("file").len() as usize <= MAX_BYTES);
    }
}
