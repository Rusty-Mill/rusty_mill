//! The open-time `untested` check for Windows-only intent lines (Let Em
//! Loose #600, card #1405; design §2, D56, D63, D178). Pure: the frozen
//! intent's text and the task's Windows mapping file in, a report out.
//!
//! Linux claims runs can't judge behaviour only Windows shows (mapped
//! drives, UNC paths, the desktop GUI). Each such line is listed in a
//! mapping file in the task's frozen inputs, keyed by the line's text, and
//! names what covers it instead: a signed golden case (`golden:<case>`) or
//! the Windows shadow run (`shadow`), or nothing yet. A listed line with no
//! cover is `untested` for the Linux runs.
//!
//! The intent itself is never marked (it is frozen; any change is a new
//! version) and no line is recognised by its wording. A map entry that
//! names no line of the intent is refused, so a stale or mistyped map fails
//! closed instead of silently testing nothing.
//!
//! The mapping file is JSON:
//!
//! ```json
//! {"lines": [
//!   {"line": "NAS shares are scanned as mounted paths (mapped drives or UNC paths)",
//!    "covered_by": "golden:unc-paths"},
//!   {"line": "platform: Windows first", "covered_by": "shadow"},
//!   {"line": "a desktop GUI", "covered_by": null}
//! ]}
//! ```

use std::collections::BTreeSet;

use rusty_json::{Map, Value};

/// What judges a Windows-only line in place of the Linux runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cover {
    /// A golden case Nano signs off (#1238).
    Golden(String),
    /// The read-only Windows shadow run (#1321).
    Shadow,
}

impl Cover {
    fn parse(word: &str) -> Result<Self, String> {
        if word == "shadow" {
            return Ok(Self::Shadow);
        }
        match word.strip_prefix("golden:") {
            Some(case) if !case.trim().is_empty() => Ok(Self::Golden(case.trim().to_owned())),
            _ => Err(format!(
                "covered_by {word:?}: \"golden:<case>\", \"shadow\" or null required"
            )),
        }
    }

    /// The word the mapping file uses.
    pub fn word(&self) -> String {
        match self {
            Self::Golden(case) => format!("golden:{case}"),
            Self::Shadow => "shadow".to_owned(),
        }
    }
}

/// One mapped line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The intent line's text, as [`normal`] gives it.
    pub line: String,
    pub cover: Option<Cover>,
}

/// The check's result, in the mapping file's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Windows-only lines with no cover: untested by the Linux runs.
    pub untested: Vec<String>,
    /// Windows-only lines and what covers each.
    pub covered: Vec<(String, Cover)>,
}

/// Parse and check the mapping file.
///
/// # Errors
///
/// A message naming the first bad field or repeated line.
pub fn parse_map(text: &str) -> Result<Vec<Entry>, String> {
    let v = Value::parse(text).map_err(|e| format!("mapping file is not JSON: {e}"))?;
    let top = v
        .as_object()
        .ok_or_else(|| "mapping file: object required".to_owned())?;
    if let Some(k) = top.keys().find(|k| k.as_str() != "lines") {
        return Err(format!("mapping file: unknown field {k:?}"));
    }
    let lines = top
        .get("lines")
        .and_then(Value::as_array)
        .ok_or_else(|| "lines: array required".to_owned())?;
    let entries = lines
        .iter()
        .enumerate()
        .map(|(i, e)| entry(e).map_err(|err| format!("lines[{i}].{err}")))
        .collect::<Result<Vec<_>, _>>()?;
    let mut seen = BTreeSet::new();
    match entries.iter().find(|e| !seen.insert(e.line.as_str())) {
        Some(e) => Err(format!("line {:?} is mapped twice", e.line)),
        None => Ok(entries),
    }
}

fn entry(v: &Value) -> Result<Entry, String> {
    let m: &Map = v.as_object().ok_or_else(|| "object required".to_owned())?;
    if let Some(k) = m
        .keys()
        .find(|k| !["line", "covered_by"].contains(&k.as_str()))
    {
        return Err(format!("unknown field {k:?}"));
    }
    let line = m
        .get("line")
        .and_then(Value::as_str)
        .map(normal)
        .filter(|l| !l.is_empty())
        .ok_or_else(|| "line: non-empty string required".to_owned())?;
    let cover = match m.get("covered_by") {
        None => return Err("covered_by: required (null when nothing covers it)".to_owned()),
        Some(Value::Null) => None,
        Some(c) => Some(
            c.as_str()
                .ok_or_else(|| "covered_by: string or null required".to_owned())
                .and_then(Cover::parse)?,
        ),
    };
    Ok(Entry { line, cover })
}

/// A line as compared: trimmed, without a leading YAML list dash. That fits
/// the frozen intent's YAML; `*` bullets, numbered items or a tab after the
/// dash are not stripped, and the same text in two sections is one line.
/// Whether a `golden:` case exists is checked by the golden run (#1238),
/// not here.
pub fn normal(line: &str) -> String {
    let t = line.trim();
    t.strip_prefix("- ").unwrap_or(t).trim().to_owned()
}

/// Check `map` against the frozen intent's text.
///
/// # Errors
///
/// When an entry names no line of the intent, quoting the first such entry.
pub fn check(intent: &str, map: &[Entry]) -> Result<Report, String> {
    let lines: BTreeSet<String> = intent.lines().map(normal).collect();
    if let Some(e) = map.iter().find(|e| !lines.contains(&e.line)) {
        return Err(format!("mapped line {:?} is not in the intent", e.line));
    }
    let mut report = Report::default();
    for e in map {
        match &e.cover {
            None => report.untested.push(e.line.clone()),
            Some(c) => report.covered.push((e.line.clone(), c.clone())),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTENT: &str = "id: dedup-manager\nplatform: Windows first\nalways:\n  - NAS shares are scanned as mounted paths (mapped drives or UNC paths)\n  - removed copies go to a quarantine Nano reviews and purges later\nexamples:\n  - the same files on drive D: and on a mapped NAS share -> one group across both\n";

    fn map(entries: &str) -> String {
        format!(r#"{{"lines": [{entries}]}}"#)
    }

    #[test]
    fn listed_lines_without_cover_are_untested() {
        let m = parse_map(&map(
            r#"{"line": "platform: Windows first", "covered_by": "shadow"},
               {"line": "- NAS shares are scanned as mounted paths (mapped drives or UNC paths)", "covered_by": "golden:unc-paths"},
               {"line": "the same files on drive D: and on a mapped NAS share -> one group across both", "covered_by": null}"#,
        ))
        .unwrap();
        let r = check(INTENT, &m).unwrap();
        assert_eq!(
            r.untested,
            vec!["the same files on drive D: and on a mapped NAS share -> one group across both"]
        );
        assert_eq!(
            r.covered,
            vec![
                ("platform: Windows first".to_owned(), Cover::Shadow),
                (
                    "NAS shares are scanned as mounted paths (mapped drives or UNC paths)"
                        .to_owned(),
                    Cover::Golden("unc-paths".to_owned())
                ),
            ]
        );
    }

    #[test]
    fn unlisted_lines_are_not_windows_only() {
        let r = check(INTENT, &parse_map(&map("")).unwrap()).unwrap();
        assert_eq!(r, Report::default(), "no wording heuristic marks a line");
    }

    #[test]
    fn a_line_not_in_the_intent_fails_closed() {
        let m = parse_map(&map(
            r#"{"line": "platform: Linux first", "covered_by": null}"#,
        ))
        .unwrap();
        let e = check(INTENT, &m).unwrap_err();
        assert!(e.contains("not in the intent"), "{e}");
    }

    #[test]
    fn a_partial_line_does_not_match() {
        let m = parse_map(&map(r#"{"line": "NAS shares", "covered_by": null}"#)).unwrap();
        assert!(check(INTENT, &m).is_err());
    }

    #[test]
    fn map_refusals_name_the_field() {
        let bad = |text: &str, expected: &str| {
            let e = parse_map(text).unwrap_err();
            assert!(e.contains(expected), "{expected:?} not in {e:?}");
        };
        bad("[", "not JSON");
        bad("[]", "object required");
        bad(r#"{"entries": []}"#, "unknown field \"entries\"");
        bad("{}", "lines: array required");
        bad(
            &map(r#"{"covered_by": null}"#),
            "lines[0].line: non-empty string",
        );
        bad(
            &map(r#"{"line": "  ", "covered_by": null}"#),
            "lines[0].line",
        );
        bad(&map(r#"{"line": "x"}"#), "lines[0].covered_by: required");
        bad(&map(r#"{"line": "x", "covered_by": 3}"#), "string or null");
        bad(
            &map(r#"{"line": "x", "covered_by": "golden:"}"#),
            "golden:<case>",
        );
        bad(
            &map(r#"{"line": "x", "covered_by": "linux"}"#),
            "golden:<case>",
        );
        bad(
            &map(r#"{"line": "x", "covered_by": null, "why": 1}"#),
            "unknown field \"why\"",
        );
        bad(
            &map(r#"{"line": "x", "covered_by": null}, {"line": "- x", "covered_by": "shadow"}"#),
            "mapped twice",
        );
    }

    #[test]
    fn cover_words_round_trip() {
        for w in ["shadow", "golden:unc-paths"] {
            assert_eq!(Cover::parse(w).unwrap().word(), w);
        }
        assert_eq!(
            Cover::parse("golden: unc-paths ").unwrap(),
            Cover::Golden("unc-paths".to_owned())
        );
    }
}
