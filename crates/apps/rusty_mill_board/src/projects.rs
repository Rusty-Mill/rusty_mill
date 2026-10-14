//! Projects: the applications one board serves. In a monorepo they are the
//! directories work may touch; the board and the runner enforce them.
//!
//! ```json
//! {"rocket_league": ["crates/apps/rocket_league"],
//!  "rusty_orch":    ["crates/apps/rusty_orch", "docs/orch"]}
//! ```
//! The `default` project always exists and owns every path no other project does.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::domain::DEFAULT_PROJECT;

/// Longest project name.
const MAX_NAME: usize = 40;

/// Project name -> the repository paths its cards may change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Projects(BTreeMap<String, Vec<String>>);

impl Default for Projects {
    /// Only the `default` project.
    fn default() -> Self {
        Self(BTreeMap::from([(DEFAULT_PROJECT.to_owned(), Vec::new())]))
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// A path relative to the repository root, without a trailing slash.
fn clean_path(raw: &str) -> Result<String, String> {
    let path = raw.trim_end_matches('/');
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.split('/').any(|part| matches!(part, "" | "." | ".."));
    if bad {
        return Err(format!(
            "path {raw:?} must be relative, inside the repository"
        ));
    }
    Ok(path.to_owned())
}

impl Projects {
    /// Parse and validate a projects file's JSON.
    ///
    /// # Errors
    /// On malformed JSON, a bad name, a bad or duplicated path, or paths given
    /// for `default` (it owns whatever the others do not).
    pub fn parse(json: &str) -> Result<Self, String> {
        let raw: BTreeMap<String, Vec<String>> =
            serde_json::from_str(json).map_err(|e| format!("projects: {e}"))?;
        Self::from_map(raw)
    }

    /// Build from a name -> paths map, with the same validation as [`Projects::parse`].
    ///
    /// # Errors
    /// See [`Projects::parse`].
    pub fn from_map(raw: BTreeMap<String, Vec<String>>) -> Result<Self, String> {
        let mut projects = Self::default();
        let mut owner: BTreeMap<String, String> = BTreeMap::new();
        for (name, paths) in raw {
            if !valid_name(&name) {
                return Err(format!(
                    "projects: name {name:?} must be 1-{MAX_NAME} of a-z 0-9 _ -"
                ));
            }
            if name == DEFAULT_PROJECT {
                return Err("projects: `default` owns every path no other project does".into());
            }
            if paths.is_empty() {
                return Err(format!("projects: {name:?} needs at least one path"));
            }
            let mut cleaned = Vec::new();
            for p in &paths {
                let p = clean_path(p).map_err(|e| format!("projects: {name:?}: {e}"))?;
                if let Some(other) = owner.insert(p.clone(), name.clone()) {
                    return Err(format!(
                        "projects: {p:?} is claimed by {other:?} and {name:?}"
                    ));
                }
                cleaned.push(p);
            }
            projects.0.insert(name, cleaned);
        }
        Ok(projects)
    }

    /// Read a projects file.
    ///
    /// # Errors
    /// If it cannot be read or does not validate.
    pub fn load(path: &Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&json)
    }

    /// Rebuild from what `board_projects` reports (which lists `default` with no paths).
    ///
    /// # Errors
    /// See [`Projects::parse`].
    pub fn from_listing(mut listed: BTreeMap<String, Vec<String>>) -> Result<Self, String> {
        listed.remove(DEFAULT_PROJECT);
        Self::from_map(listed)
    }

    /// Whether `name` is a project.
    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// Project names, `default` included.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// The project's own paths (empty for `default`).
    pub fn paths(&self, name: &str) -> &[String] {
        self.0.get(name).map_or(&[], Vec::as_slice)
    }

    /// The project that owns the repository file `file` (a `/`-separated path
    /// from the root): the one whose path contains it, else `default`.
    pub fn owner_of(&self, file: &str) -> &str {
        self.0
            .iter()
            .find(|(_, paths)| {
                paths.iter().any(|p| {
                    file == p
                        || file
                            .strip_prefix(p.as_str())
                            .is_some_and(|r| r.starts_with('/'))
                })
            })
            .map_or(DEFAULT_PROJECT, |(name, _)| name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_alone_when_there_is_no_file() {
        let p = Projects::default();
        assert_eq!(p.names().collect::<Vec<_>>(), [DEFAULT_PROJECT]);
        assert_eq!(p.owner_of("anything/at/all.rs"), DEFAULT_PROJECT);
    }

    #[test]
    fn a_file_belongs_to_the_project_whose_path_contains_it() {
        let p = Projects::parse(r#"{"rl": ["crates/apps/rocket_league/"], "orch": ["crates/apps/rusty_orch", "docs/orch"]}"#)
            .expect("parse");
        assert_eq!(p.owner_of("crates/apps/rocket_league/src/lib.rs"), "rl");
        assert_eq!(p.owner_of("docs/orch/adr.md"), "orch");
        assert_eq!(p.owner_of("crates/apps/rusty_orch"), "orch");
        // A sibling that merely shares a prefix is not inside.
        assert_eq!(
            p.owner_of("crates/apps/rocket_league_old/x.rs"),
            DEFAULT_PROJECT
        );
        assert_eq!(p.owner_of("Cargo.toml"), DEFAULT_PROJECT);
        assert_eq!(p.paths("rl"), ["crates/apps/rocket_league"]);
    }

    #[test]
    fn bad_files_are_refused() {
        for (json, why) in [
            ("[]", "projects:"),
            (r#"{"Bad Name": ["a"]}"#, "name"),
            (r#"{"": ["a"]}"#, "name"),
            (r#"{"default": ["a"]}"#, "default"),
            (r#"{"x": []}"#, "at least one path"),
            (r#"{"x": ["/etc"]}"#, "relative"),
            (r#"{"x": ["a/../b"]}"#, "relative"),
            (r#"{"x": ["a//b"]}"#, "relative"),
            (r#"{"x": ["."]}"#, "relative"),
            (r#"{"x": ["a"], "y": ["a/"]}"#, "claimed by"),
        ] {
            let e = Projects::parse(json).expect_err(json);
            assert!(e.contains(why), "{json}: {e}");
        }
    }
}
