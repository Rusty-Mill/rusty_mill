//! Repositories and their projects: the applications one board serves. In a monorepo a
//! project is the directories its work may touch; the board and the runner enforce them.
//!
//! One repository (a flat file):
//! ```json
//! {"rocket_league": ["crates/apps/rocket_league"],
//!  "rusty_orch":    ["crates/apps/rusty_orch", "docs/orch"]}
//! ```
//! Several repositories:
//! ```json
//! {"repos": {"rusty_mill": {"rocket_league": ["crates/apps/rocket_league"]},
//!            "super_board": {"skills": ["skills"]}}}
//! ```
//! Every repository also has a project named like itself that owns the paths no other
//! project of that repository does (the flat form's repository is called `default`). Project
//! names are unique across repositories, so a card needs only its project to be placed.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::domain::DEFAULT_PROJECT;

/// Longest repository or project name.
const MAX_NAME: usize = 40;
/// The key that introduces the several-repositories form.
const REPOS_KEY: &str = "repos";

/// project name -> the repository paths its cards may change (empty for the repository's default).
type Paths = BTreeMap<String, Vec<String>>;

/// Repository name -> its projects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Projects {
    repos: BTreeMap<String, Paths>,
}

impl Default for Projects {
    /// One repository, `default`, with only the `default` project.
    fn default() -> Self {
        Self {
            repos: BTreeMap::from([(
                DEFAULT_PROJECT.to_owned(),
                Paths::from([(DEFAULT_PROJECT.to_owned(), Vec::new())]),
            )]),
        }
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
    /// Parse and validate a projects file's JSON (either form above).
    ///
    /// # Errors
    /// On malformed JSON, a bad or duplicated name, a bad or duplicated path, or paths given
    /// for a repository's own default project (it owns whatever the others do not).
    pub fn parse(json: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("projects: {e}"))?;
        let many = value.as_object().is_some_and(|o| o.contains_key(REPOS_KEY));
        let bad = |e: serde_json::Error| format!("projects: {e}");
        if many {
            let mut object = value;
            let repos = object[REPOS_KEY].take();
            if object.as_object().is_some_and(|o| o.len() > 1) {
                return Err(format!(
                    "projects: with \"{REPOS_KEY}\" there is nothing else at the top level"
                ));
            }
            return Self::from_repos(serde_json::from_value(repos).map_err(bad)?);
        }
        let flat: Paths = serde_json::from_value(value).map_err(bad)?;
        Self::from_repos(BTreeMap::from([(DEFAULT_PROJECT.to_owned(), flat)]))
    }

    /// Build from repository -> projects, with the validation of [`Projects::parse`].
    ///
    /// # Errors
    /// See [`Projects::parse`].
    pub fn from_repos(raw: BTreeMap<String, Paths>) -> Result<Self, String> {
        let mut repos = Self::default().repos;
        let mut seen: BTreeMap<String, String> = BTreeMap::new(); // project -> its repository
        for (repo, projects) in raw {
            if !valid_name(&repo) || repo == REPOS_KEY {
                return Err(format!(
                    "projects: repository name {repo:?} must be 1-{MAX_NAME} of a-z 0-9 _ - (and not {REPOS_KEY:?})"
                ));
            }
            let mut mine = Paths::from([(repo.clone(), Vec::new())]);
            let mut owner: BTreeMap<String, String> = BTreeMap::new(); // path -> project
            for (name, paths) in projects {
                if name == repo {
                    return Err(format!(
                        "projects: {name:?} is repository {repo:?}'s own project: it owns every path no other project does"
                    ));
                }
                if !valid_name(&name) || name == REPOS_KEY {
                    return Err(format!(
                        "projects: name {name:?} must be 1-{MAX_NAME} of a-z 0-9 _ - (and not {REPOS_KEY:?})"
                    ));
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
                mine.insert(name, cleaned);
            }
            repos.insert(repo, mine);
        }
        for (repo, projects) in &repos {
            for name in projects.keys() {
                if let Some(other) = seen.insert(name.clone(), repo.clone()) {
                    return Err(format!(
                        "projects: name {name:?} is used in repositories {other:?} and {repo:?}; project names are unique across repositories"
                    ));
                }
            }
        }
        Ok(Self { repos })
    }

    /// This registry plus project `name` owning `paths` in repository `repo` (a new repository
    /// if `repo` is not one yet). Purely additive: it takes no path any project already names,
    /// so it never changes what an existing project owns beyond its repository's own project.
    ///
    /// # Errors
    /// If `name` exists, a path overlaps (equals, contains or sits inside) another project's
    /// path in `repo`, or the result fails [`Projects::parse`]'s validation.
    pub fn with_project(&self, repo: &str, name: &str, paths: &[String]) -> Result<Self, String> {
        if self.contains(name) {
            return Err(format!("projects: {name:?} already exists"));
        }
        let mut listed = self.repos.clone();
        let projects = listed.entry(repo.to_owned()).or_default();
        let nested = |a: &str, b: &str| a.strip_prefix(b).is_some_and(|r| r.starts_with('/'));
        for raw in paths {
            let p = clean_path(raw).map_err(|e| format!("projects: {name:?}: {e}"))?;
            let taken = projects.iter().find(|(_, theirs)| {
                theirs
                    .iter()
                    .any(|q| p == *q || nested(&p, q) || nested(q, &p))
            });
            if let Some((other, _)) = taken {
                return Err(format!("projects: {p:?} overlaps project {other:?}"));
            }
        }
        projects.insert(name.to_owned(), paths.to_vec());
        Self::from_listing(listed)
    }

    /// Read a projects file.
    ///
    /// # Errors
    /// If it cannot be read or does not validate.
    pub fn load(path: &Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&json)
    }

    /// Rebuild from what `board_projects` reports (which lists each repository's own project
    /// with no paths).
    ///
    /// # Errors
    /// See [`Projects::parse`].
    pub fn from_listing(mut listed: BTreeMap<String, Paths>) -> Result<Self, String> {
        for (repo, projects) in &mut listed {
            projects.remove(repo);
        }
        Self::from_repos(listed)
    }

    /// The JSON a projects file holds for this registry (each repository's own project left out).
    ///
    /// # Errors
    /// If serialisation fails.
    pub fn file_json(&self) -> Result<String, String> {
        let repos: BTreeMap<&String, BTreeMap<&String, &Vec<String>>> = self
            .repos
            .iter()
            .map(|(repo, projects)| {
                (
                    repo,
                    projects.iter().filter(|(name, _)| *name != repo).collect(),
                )
            })
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({ REPOS_KEY: repos }))
            .map_err(|e| e.to_string())
    }

    /// Whether `name` is a project.
    pub fn contains(&self, name: &str) -> bool {
        self.repo_of(name).is_some()
    }

    /// Every project name, across repositories.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.repos
            .values()
            .flat_map(|p| p.keys().map(String::as_str))
    }

    /// The repositories and their projects (each repository's own project first is not
    /// guaranteed; they are in name order).
    pub fn repos(&self) -> impl Iterator<Item = (&str, impl Iterator<Item = &str>)> {
        self.repos
            .iter()
            .map(|(repo, projects)| (repo.as_str(), projects.keys().map(String::as_str)))
    }

    /// The repository `project` belongs to.
    pub fn repo_of(&self, project: &str) -> Option<&str> {
        self.repos
            .iter()
            .find(|(_, projects)| projects.contains_key(project))
            .map(|(repo, _)| repo.as_str())
    }

    /// The project's own paths (empty for a repository's own project, or an unknown name).
    pub fn paths(&self, project: &str) -> &[String] {
        self.repos
            .values()
            .find_map(|projects| projects.get(project))
            .map_or(&[], Vec::as_slice)
    }

    /// The `files` (repository paths) that `project` does not own: those of another project
    /// of its repository. Empty for an unknown project.
    pub fn not_owned_by(&self, project: &str, files: &[String]) -> Vec<String> {
        let Some(repo) = self.repo_of(project) else {
            return Vec::new();
        };
        let foreign = |f: &&String| self.owner_of(repo, f) != project;
        files.iter().filter(foreign).cloned().collect()
    }

    /// The project of `repo` that owns the repository file `file` (a `/`-separated path from
    /// the root): the one whose path contains it, else the repository's own project.
    pub fn owner_of<'a>(&'a self, repo: &'a str, file: &str) -> &'a str {
        let Some(projects) = self.repos.get(repo) else {
            return repo;
        };
        projects
            .iter()
            .find(|(_, paths)| {
                paths.iter().any(|p| {
                    file == p
                        || file
                            .strip_prefix(p.as_str())
                            .is_some_and(|r| r.starts_with('/'))
                })
            })
            .map_or(repo, |(name, _)| name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(p: &Projects) -> Vec<&str> {
        let mut n: Vec<_> = p.names().collect();
        n.sort_unstable();
        n
    }

    #[test]
    fn default_alone_when_there_is_no_file() {
        let p = Projects::default();
        assert_eq!(names(&p), [DEFAULT_PROJECT]);
        assert_eq!(
            p.owner_of(DEFAULT_PROJECT, "anything/at/all.rs"),
            DEFAULT_PROJECT
        );
        assert_eq!(p.repo_of(DEFAULT_PROJECT), Some(DEFAULT_PROJECT));
    }

    #[test]
    fn a_flat_file_is_one_repository_called_default() {
        let p = Projects::parse(r#"{"rl": ["crates/apps/rocket_league/"], "orch": ["crates/apps/rusty_orch", "docs/orch"]}"#)
            .expect("parse");
        assert_eq!(names(&p), ["default", "orch", "rl"]);
        assert_eq!(p.repo_of("rl"), Some("default"));
        assert_eq!(
            p.owner_of("default", "crates/apps/rocket_league/src/lib.rs"),
            "rl"
        );
        assert_eq!(p.owner_of("default", "docs/orch/adr.md"), "orch");
        assert_eq!(p.owner_of("default", "crates/apps/rusty_orch"), "orch");
        // A sibling that merely shares a prefix is not inside.
        assert_eq!(
            p.owner_of("default", "crates/apps/rocket_league_old/x.rs"),
            DEFAULT_PROJECT
        );
        assert_eq!(p.owner_of("default", "Cargo.toml"), DEFAULT_PROJECT);
        assert_eq!(p.paths("rl"), ["crates/apps/rocket_league"]);
    }

    #[test]
    fn several_repositories_each_own_the_rest_of_their_paths() {
        let p = Projects::parse(
            r#"{"repos": {"rusty_mill": {"rl": ["crates/apps/rocket_league"]},
                          "super_board": {"skills": ["skills"], "hooks": ["hooks"]}}}"#,
        )
        .expect("parse");
        assert_eq!(
            names(&p),
            [
                "default",
                "hooks",
                "rl",
                "rusty_mill",
                "skills",
                "super_board"
            ]
        );
        assert_eq!(p.repo_of("rl"), Some("rusty_mill"));
        assert_eq!(p.repo_of("skills"), Some("super_board"));
        assert_eq!(
            p.repo_of("super_board"),
            Some("super_board"),
            "a repository's own project"
        );
        // Paths are per repository.
        assert_eq!(p.owner_of("super_board", "skills/x.md"), "skills");
        assert_eq!(p.owner_of("rusty_mill", "skills/x.md"), "rusty_mill");
        assert_eq!(
            p.owner_of("rusty_mill", "crates/apps/rocket_league/a.rs"),
            "rl"
        );
        let listing: Vec<_> = p.repos().map(|(r, ps)| (r, ps.count())).collect();
        assert_eq!(
            listing,
            [("default", 1), ("rusty_mill", 2), ("super_board", 3)]
        );
    }

    #[test]
    fn the_same_path_may_appear_in_different_repositories() {
        assert!(
            Projects::parse(r#"{"repos": {"a": {"x": ["src"]}, "b": {"y": ["src"]}}}"#).is_ok()
        );
    }

    #[test]
    fn bad_files_are_refused() {
        for (json, why) in [
            ("[]", "projects:"),
            (r#"{"Bad Name": ["a"]}"#, "name"),
            (r#"{"": ["a"]}"#, "name"),
            (r#"{"default": ["a"]}"#, "own project"),
            (r#"{"x": []}"#, "at least one path"),
            (r#"{"x": ["/etc"]}"#, "relative"),
            (r#"{"x": ["a/../b"]}"#, "relative"),
            (r#"{"x": ["a//b"]}"#, "relative"),
            (r#"{"x": ["."]}"#, "relative"),
            (r#"{"x": ["a"], "y": ["a/"]}"#, "claimed by"),
            (
                r#"{"repos": {"a": {"x": ["p"]}}, "extra": 1}"#,
                "nothing else",
            ),
            (r#"{"repos": {"Bad": {}}}"#, "repository name"),
            (r#"{"repos": {"a": {"a": ["p"]}}}"#, "own project"),
            (
                r#"{"repos": {"a": {"x": ["p"]}, "b": {"x": ["q"]}}}"#,
                "unique across",
            ),
            (
                r#"{"repos": {"a": {"b": ["p"]}, "b": {"x": ["q"]}}}"#,
                "unique across",
            ),
            (r#"{"repos": {"repos": {}}}"#, "repository name"),
            (r#"{"repos": {"a": {"repos": ["p"]}}}"#, "name"),
        ] {
            let e = Projects::parse(json).expect_err(json);
            assert!(e.contains(why), "{json}: {e}");
        }
    }

    #[test]
    fn the_file_form_round_trips() {
        let p = Projects::parse(r#"{"repos": {"a": {"x": ["src"], "y": ["docs"]}, "b": {}}}"#)
            .expect("parse");
        let json = p.file_json().expect("json");
        assert!(
            !json.contains("\"a\": []"),
            "a repository's own project is implied, not written: {json}"
        );
        assert_eq!(Projects::parse(&json).expect("again"), p);
    }

    #[test]
    fn a_listing_round_trips() {
        let p =
            Projects::parse(r#"{"repos": {"rusty_mill": {"rl": ["crates/apps/rocket_league"]}}}"#)
                .expect("parse");
        let wire = serde_json::to_value(&p).expect("json");
        let listed: BTreeMap<String, Paths> = serde_json::from_value(wire).expect("map");
        assert_eq!(Projects::from_listing(listed).expect("rebuild"), p);
    }

    #[test]
    fn not_owned_by_names_the_files_of_sibling_projects_only() {
        let p = Projects::parse(
            r#"{"repos": {"a": {"x": ["src"]}, "b": {"y": ["src"], "z": ["docs"]}}}"#,
        )
        .expect("projects");
        let files = |v: &[&str]| v.iter().map(|f| (*f).to_owned()).collect::<Vec<_>>();
        let all = files(&["src/lib.rs", "docs/a.md", "README.md", "srcx/a.rs"]);
        // The same path belongs to a different project in each repository.
        assert_eq!(
            p.not_owned_by("x", &all),
            files(&["docs/a.md", "README.md", "srcx/a.rs"])
        );
        assert_eq!(
            p.not_owned_by("y", &all),
            files(&["docs/a.md", "README.md", "srcx/a.rs"])
        );
        // A repository's own project owns what its projects do not, and only there.
        assert_eq!(p.not_owned_by("a", &all), files(&["src/lib.rs"]));
        assert_eq!(
            p.not_owned_by("b", &all),
            files(&["src/lib.rs", "docs/a.md"])
        );
        assert!(p.not_owned_by("nope", &all).is_empty());
    }
}
