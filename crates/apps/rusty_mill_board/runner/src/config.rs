//! Stage definitions: which column a stage works, as which role, running
//! which command, and where the card goes afterwards.

use std::collections::HashSet;

use rusty_mill_board::domain::Column;
use serde::Deserialize;

/// One stage of the pipeline.
///
/// ```json
/// {"name": "build", "from": ["ready", "building"], "enter": "building", "role": "builder",
///  "argv": ["claude", "-p", "{prompt}"], "ok": "qa", "fail": "blocked"}
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    /// Short name; comments are prefixed `[name]` and attempts are counted by it.
    pub name: String,
    /// Columns whose free (or own) cards this stage picks up.
    pub from: Vec<Column>,
    /// Column to move a card into before working it, if not already there.
    #[serde(default)]
    pub enter: Option<Column>,
    /// Board role the runner acts as for this stage.
    pub role: String,
    /// Program and arguments, run without a shell. Placeholders in each
    /// element: `{prompt}`, `{id}`, `{title}`, `{worktree}`.
    pub argv: Vec<String>,
    /// Column after success; `None` releases the card where it is.
    #[serde(default)]
    pub ok: Option<Column>,
    /// Column after failure; `None` releases the card where it is.
    #[serde(default)]
    pub fail: Option<Column>,
    /// Wall-clock limit for the command.
    #[serde(default = "default_wall_secs")]
    pub wall_secs: u64,
    /// Runs allowed per card before it is moved to Blocked.
    #[serde(default = "default_attempts")]
    pub max_attempts: u32,
    /// Allow internet sockets (an agent CLI reaching its model). Off by default.
    #[serde(default)]
    pub network: bool,
}

fn default_wall_secs() -> u64 {
    1800
}

fn default_attempts() -> u32 {
    3
}

impl Stage {
    /// The column the card is in while this stage works it.
    pub fn work_column(&self) -> &Column {
        self.enter.as_ref().unwrap_or(&self.from[0])
    }
}

/// Parse and validate a stage file (a JSON array).
///
/// # Errors
/// On malformed JSON, an unknown field, or a stage that cannot work
/// (blank name, no source column, empty command, zero limits, duplicate name).
pub fn parse(json: &str) -> Result<Vec<Stage>, String> {
    let stages: Vec<Stage> = serde_json::from_str(json).map_err(|e| format!("stages: {e}"))?;
    if stages.is_empty() {
        return Err("stages: at least one stage is required".into());
    }
    let mut names = HashSet::new();
    for s in &stages {
        let bad = |why: &str| format!("stage {:?}: {why}", s.name);
        if s.name.trim().is_empty() || s.name.contains(['[', ']']) {
            return Err(bad("name must be non-empty without brackets"));
        }
        if !names.insert(s.name.as_str()) {
            return Err(bad("duplicate name"));
        }
        if s.from.is_empty() {
            return Err(bad("`from` needs at least one column"));
        }
        if s.argv.first().is_none_or(|p| p.trim().is_empty()) {
            return Err(bad("`argv` needs a program"));
        }
        if s.wall_secs == 0 || s.max_attempts == 0 {
            return Err(bad("wall_secs and max_attempts must be at least 1"));
        }
    }
    Ok(stages)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = r#"[{"name":"build","from":["ready","building"],"enter":"building","role":"builder","argv":["agent","{prompt}"],"ok":"qa","fail":"blocked"}]"#;

    #[test]
    fn parses_with_defaults() {
        let s = &parse(OK).expect("valid")[0];
        assert_eq!((s.wall_secs, s.max_attempts, s.network), (1800, 3, false));
        assert_eq!(s.work_column(), &Column::BUILDING);
    }

    #[test]
    fn work_column_defaults_to_first_source() {
        let s =
            parse(r#"[{"name":"t","from":["qa"],"role":"qa","argv":["make"]}]"#).expect("valid");
        assert_eq!(s[0].work_column(), &Column::QA);
    }

    #[test]
    fn rejects_bad_stages() {
        for (json, why) in [
            ("[]", "at least one"),
            (
                r#"[{"name":"a","from":["ready"],"role":"r","argv":["x"],"typo":1}]"#,
                "unknown field",
            ),
            (
                r#"[{"name":" ","from":["ready"],"role":"r","argv":["x"]}]"#,
                "name",
            ),
            (
                r#"[{"name":"a[1]","from":["ready"],"role":"r","argv":["x"]}]"#,
                "name",
            ),
            (
                r#"[{"name":"a","from":[],"role":"r","argv":["x"]}]"#,
                "from",
            ),
            (
                r#"[{"name":"a","from":["ready"],"role":"r","argv":[]}]"#,
                "argv",
            ),
            (
                r#"[{"name":"a","from":["ready"],"role":"r","argv":["x"],"max_attempts":0}]"#,
                "at least 1",
            ),
            (
                r#"[{"name":"a","from":["No Such"],"role":"r","argv":["x"]}]"#,
                "column id",
            ),
            (
                r#"[{"name":"a","from":["ready"],"role":"r","argv":["x"]},{"name":"a","from":["qa"],"role":"r","argv":["x"]}]"#,
                "duplicate",
            ),
        ] {
            let err = parse(json).expect_err(json);
            assert!(err.contains(why), "{json} -> {err}");
        }
    }
}
