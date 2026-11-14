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
//! Version 2, for a project that spans repositories or a repository with facts of its own
//! (see `docs/repos-and-agent-permissions-design.md`):
//! ```json
//! {"version": 2,
//!  "repos": {"rusty_mill": {"github": "rusty-mill/rusty_mill"}, "pythrust": {}},
//!  "projects": {"tooling": {"repos": {"rusty_mill": [".github"], "pythrust": ["ci"]},
//!                           "default_repo": "rusty_mill"}}}
//! ```
//! A file only becomes version 2 when it must (see [`Projects::file_json`]), so an older
//! binary can still read every file that does not use the new features.
//! Every repository also has a project named like itself that owns the paths no other
//! project of that repository does (the flat form's repository is called `default`). Project
//! names are unique across repositories, so a card needs only its project to be placed; a
//! project that spans repositories has a `default_repo` for what needs a single one.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::domain::{DEFAULT_PROJECT, ProjectStatus};

/// Longest repository or project name.
const MAX_NAME: usize = 40;
/// The key that introduces the several-repositories form.
const REPOS_KEY: &str = "repos";

/// project name -> the repository paths its cards may change (empty for the repository's default).
type Paths = BTreeMap<String, Vec<String>>;

/// Each project's status, kept apart from the registry (its own settings file), so the
/// projects file keeps its format. Only projects that are not active are listed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Statuses(BTreeMap<String, ProjectStatus>);

impl Statuses {
    /// Parse a status file's JSON: `{"project": "paused" | "experiment" | "archived", ...}`.
    ///
    /// # Errors
    /// On malformed JSON or an unknown status.
    pub fn parse(json: &str) -> Result<Self, String> {
        let raw: BTreeMap<String, ProjectStatus> =
            serde_json::from_str(json).map_err(|e| format!("project status: {e}"))?;
        Ok(Self(
            raw.into_iter()
                .filter(|(_, s)| *s != ProjectStatus::Active)
                .collect(),
        ))
    }

    /// `project`'s status: [`ProjectStatus::Active`] unless listed.
    pub fn of(&self, project: &str) -> ProjectStatus {
        self.0.get(project).copied().unwrap_or_default()
    }

    /// These statuses with `project` set to `status`.
    #[must_use]
    pub fn with(&self, project: &str, status: ProjectStatus) -> Self {
        let mut next = self.0.clone();
        match status {
            ProjectStatus::Active => next.remove(project),
            other => next.insert(project.to_owned(), other),
        };
        Self(next)
    }
}

/// What a repository carries besides its projects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoInfo {
    /// `owner/name` on GitHub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
}

/// One project: the paths it owns in each repository it touches.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Project {
    /// repository -> paths (empty for a repository's own project).
    edges: Paths,
    /// The repository to use when only one can be named; always one of `edges`.
    default_repo: String,
}

/// A project as written in a version 2 file.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectFile {
    repos: Paths,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_repo: Option<String>,
}

/// A version 2 file.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FileV2 {
    version: u32,
    #[serde(default)]
    repos: BTreeMap<String, RepoInfo>,
    #[serde(default)]
    projects: BTreeMap<String, ProjectFile>,
}

/// The registry: repositories, and projects with the paths they own in each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projects {
    repos: BTreeMap<String, RepoInfo>,
    projects: BTreeMap<String, Project>,
}

impl Serialize for Projects {
    /// The `board_projects` listing: repository -> its projects -> their paths there.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.listing().serialize(s)
    }
}

impl Default for Projects {
    /// One repository, `default`, with only the `default` project.
    fn default() -> Self {
        Self::build(BTreeMap::new(), BTreeMap::new()).unwrap_or_else(|_| Self {
            repos: BTreeMap::new(),
            projects: BTreeMap::new(),
        })
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// `owner/name`, no spaces.
fn valid_github(slug: &str) -> bool {
    let mut parts = slug.split('/');
    let part = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
    };
    slug.len() <= 100 && part(parts.next()) && part(parts.next()) && parts.next().is_none()
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
    /// Parse and validate a projects file's JSON (any form above).
    ///
    /// # Errors
    /// On malformed JSON, a bad or duplicated name, a bad or duplicated path, or paths given
    /// for a repository's own default project (it owns whatever the others do not).
    pub fn parse(json: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("projects: {e}"))?;
        let has = |key: &str| value.as_object().is_some_and(|o| o.contains_key(key));
        let bad = |e: serde_json::Error| format!("projects: {e}");
        if has("projects") || has("version") {
            let file: FileV2 = serde_json::from_value(value).map_err(bad)?;
            return Self::from_file(file);
        }
        if has(REPOS_KEY) {
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

    fn from_file(file: FileV2) -> Result<Self, String> {
        if file.version != 2 {
            return Err(format!("projects: unknown version {}", file.version));
        }
        let mut projects = BTreeMap::new();
        for (name, p) in file.projects {
            let edges = p.repos;
            let default_repo = match (p.default_repo, edges.len()) {
                (_, 0) => return Err(format!("projects: {name:?} needs at least one repository")),
                (Some(r), _) if edges.contains_key(&r) => r,
                (Some(r), _) => {
                    return Err(format!(
                        "projects: {name:?}: default_repo {r:?} is not one of its repositories"
                    ));
                }
                (None, 1) => edges.keys().next().cloned().unwrap_or_default(),
                (None, _) => {
                    return Err(format!(
                        "projects: {name:?} spans several repositories and needs a default_repo"
                    ));
                }
            };
            projects.insert(
                name,
                Project {
                    edges,
                    default_repo,
                },
            );
        }
        Self::build(file.repos, projects)
    }

    /// Build from repository -> projects, with the validation of [`Projects::parse`].
    ///
    /// # Errors
    /// See [`Projects::parse`].
    pub fn from_repos(raw: BTreeMap<String, Paths>) -> Result<Self, String> {
        let mut repos = BTreeMap::new();
        let mut projects: BTreeMap<String, Project> = BTreeMap::new();
        for (repo, listed) in raw {
            for (name, paths) in listed {
                if let Some(other) = projects.get(&name) {
                    return Err(format!(
                        "projects: name {name:?} is used in repositories {:?} and {repo:?}; project names are unique across repositories",
                        other.default_repo
                    ));
                }
                projects.insert(
                    name,
                    Project {
                        edges: Paths::from([(repo.clone(), paths)]),
                        default_repo: repo.clone(),
                    },
                );
            }
            repos.insert(repo, RepoInfo::default());
        }
        Self::build(repos, projects)
    }

    /// Validate `repos` and `projects` (without the implicit project of each repository) and add
    /// the implicit projects and the `default` repository.
    fn build(
        mut repos: BTreeMap<String, RepoInfo>,
        mut projects: BTreeMap<String, Project>,
    ) -> Result<Self, String> {
        repos.entry(DEFAULT_PROJECT.to_owned()).or_default();
        for (repo, info) in &repos {
            if !valid_name(repo) || repo == REPOS_KEY {
                return Err(format!(
                    "projects: repository name {repo:?} must be 1-{MAX_NAME} of a-z 0-9 _ - (and not {REPOS_KEY:?})"
                ));
            }
            if info.github.as_deref().is_some_and(|g| !valid_github(g)) {
                return Err(format!(
                    "projects: repository {repo:?}: github must be owner/name"
                ));
            }
        }
        let mut owner: BTreeMap<(String, String), String> = BTreeMap::new(); // (repo, path) -> project
        for (name, project) in &mut projects {
            if repos.contains_key(name) {
                return Err(format!(
                    "projects: {name:?} is repository {name:?}'s own project: it owns every path no other project does"
                ));
            }
            if !valid_name(name) || name == REPOS_KEY {
                return Err(format!(
                    "projects: name {name:?} must be 1-{MAX_NAME} of a-z 0-9 _ - (and not {REPOS_KEY:?})"
                ));
            }
            for (repo, paths) in &mut project.edges {
                if !repos.contains_key(repo) {
                    return Err(format!("projects: {name:?}: unknown repository {repo:?}"));
                }
                if paths.is_empty() {
                    return Err(format!("projects: {name:?} needs at least one path"));
                }
                for p in paths.iter_mut() {
                    *p = clean_path(p).map_err(|e| format!("projects: {name:?}: {e}"))?;
                    if let Some(other) = owner.insert((repo.clone(), p.clone()), name.clone()) {
                        return Err(format!(
                            "projects: {p:?} is claimed by {other:?} and {name:?}"
                        ));
                    }
                }
            }
        }
        for repo in repos.keys() {
            projects.insert(
                repo.clone(),
                Project {
                    edges: Paths::from([(repo.clone(), Vec::new())]),
                    default_repo: repo.clone(),
                },
            );
        }
        Ok(Self { repos, projects })
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
        let nested = |a: &str, b: &str| a.strip_prefix(b).is_some_and(|r| r.starts_with('/'));
        for raw in paths {
            let p = clean_path(raw).map_err(|e| format!("projects: {name:?}: {e}"))?;
            let taken = self.projects.iter().find(|(_, theirs)| {
                theirs
                    .edges
                    .get(repo)
                    .is_some_and(|ps| ps.iter().any(|q| p == *q || nested(&p, q) || nested(q, &p)))
            });
            if let Some((other, _)) = taken {
                return Err(format!("projects: {p:?} overlaps project {other:?}"));
            }
        }
        let mut repos = self.repos.clone();
        repos.entry(repo.to_owned()).or_default();
        let mut projects = self.explicit();
        projects.insert(
            name.to_owned(),
            Project {
                edges: Paths::from([(repo.to_owned(), paths.to_vec())]),
                default_repo: repo.to_owned(),
            },
        );
        Self::build(repos, projects)
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
    /// with no paths). A project listed under two repositories is refused.
    ///
    /// # Errors
    /// See [`Projects::parse`].
    pub fn from_listing(mut listed: BTreeMap<String, Paths>) -> Result<Self, String> {
        for (repo, projects) in &mut listed {
            projects.remove(repo);
        }
        Self::from_repos(listed)
    }

    /// The projects that are not a repository's own.
    fn explicit(&self) -> BTreeMap<String, Project> {
        self.projects
            .iter()
            .filter(|(name, _)| !self.repos.contains_key(*name))
            .map(|(name, p)| (name.clone(), p.clone()))
            .collect()
    }

    /// Repository -> project -> the paths it owns there (a repository's own project: none).
    fn listing(&self) -> BTreeMap<&str, Paths> {
        let mut out: BTreeMap<&str, Paths> = self
            .repos
            .keys()
            .map(|r| (r.as_str(), Paths::new()))
            .collect();
        for (name, project) in &self.projects {
            for (repo, paths) in &project.edges {
                if let Some(listed) = out.get_mut(repo.as_str()) {
                    listed.insert(name.clone(), paths.clone());
                }
            }
        }
        out
    }

    /// The JSON a projects file holds for this registry. The older form (a repository's own
    /// project left out) when it can say everything, so a binary that predates version 2 still
    /// reads the file; version 2 only once a project spans repositories or a repository has facts.
    ///
    /// # Errors
    /// If serialisation fails.
    pub fn file_json(&self) -> Result<String, String> {
        let explicit = self.explicit();
        let older = explicit.values().all(|p| p.edges.len() == 1)
            && self.repos.values().all(|r| r.github.is_none());
        let value = if older {
            let mut repos: BTreeMap<&str, BTreeMap<&str, &Vec<String>>> = self
                .repos
                .keys()
                .map(|r| (r.as_str(), BTreeMap::new()))
                .collect();
            for (name, p) in &explicit {
                for (repo, paths) in &p.edges {
                    repos.entry(repo).or_default().insert(name, paths);
                }
            }
            serde_json::json!({ REPOS_KEY: repos })
        } else {
            let projects = explicit.into_iter().map(|(name, p)| {
                let multi = p.edges.len() > 1;
                let file = ProjectFile {
                    repos: p.edges,
                    default_repo: multi.then_some(p.default_repo),
                };
                (name, file)
            });
            serde_json::to_value(FileV2 {
                version: 2,
                repos: self.repos.clone(),
                projects: projects.collect(),
            })
            .map_err(|e| e.to_string())?
        };
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
    }

    /// Whether `name` is a project.
    pub fn contains(&self, name: &str) -> bool {
        self.projects.contains_key(name)
    }

    /// Every project name, across repositories.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.projects.keys().map(String::as_str)
    }

    /// The repositories and the projects that own paths in each (in name order).
    pub fn repos(&self) -> impl Iterator<Item = (&str, impl Iterator<Item = &str>)> {
        self.repos.keys().map(|repo| {
            let mine = self
                .projects
                .iter()
                .filter(move |(_, p)| p.edges.contains_key(repo))
                .map(|(name, _)| name.as_str());
            (repo.as_str(), mine)
        })
    }

    /// The repository `project` belongs to (its `default_repo` if it spans several).
    pub fn repo_of(&self, project: &str) -> Option<&str> {
        self.projects.get(project).map(|p| p.default_repo.as_str())
    }

    /// Every repository `project` owns paths in.
    pub fn repos_of(&self, project: &str) -> impl Iterator<Item = &str> {
        self.projects
            .get(project)
            .into_iter()
            .flat_map(|p| p.edges.keys().map(String::as_str))
    }

    /// The project's own paths in its default repository (empty for a repository's own project,
    /// or an unknown name).
    pub fn paths(&self, project: &str) -> &[String] {
        self.projects
            .get(project)
            .and_then(|p| p.edges.get(&p.default_repo))
            .map_or(&[], Vec::as_slice)
    }

    /// The project's own paths in `repo` (empty if it has none there).
    pub fn paths_in(&self, project: &str, repo: &str) -> &[String] {
        self.projects
            .get(project)
            .and_then(|p| p.edges.get(repo))
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
        let inside = |p: &String| {
            file == p
                || file
                    .strip_prefix(p.as_str())
                    .is_some_and(|r| r.starts_with('/'))
        };
        self.projects
            .iter()
            .find(|(_, project)| {
                project
                    .edges
                    .get(repo)
                    .is_some_and(|ps| ps.iter().any(inside))
            })
            .map_or(repo, |(name, _)| name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_default_to_active_and_list_only_the_rest() {
        let s = Statuses::parse(r#"{"a": "paused", "b": "active"}"#).expect("parse");
        assert_eq!(s.of("a"), ProjectStatus::Paused);
        assert_eq!(s.of("b"), ProjectStatus::Active);
        assert_eq!(s.of("unlisted"), ProjectStatus::Active);
        let s = s
            .with("a", ProjectStatus::Active)
            .with("c", ProjectStatus::Experiment);
        assert_eq!(
            serde_json::to_string(&s).expect("json"),
            r#"{"c":"experiment"}"#
        );
        assert!(Statuses::parse(r#"{"a": "gone"}"#).is_err());
    }

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
                "own project",
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

    const SPAN: &str = r#"{"version": 2,
        "repos": {"mill": {"github": "rusty-mill/rusty_mill"}, "py": {}},
        "projects": {"tooling": {"repos": {"mill": [".github"], "py": ["ci"]}, "default_repo": "mill"},
                     "term": {"repos": {"mill": ["crates/term"]}}}}"#;

    #[test]
    fn a_project_may_span_repositories() {
        let p = Projects::parse(SPAN).expect("parse");
        assert_eq!(p.repo_of("tooling"), Some("mill"));
        assert_eq!(p.repo_of("term"), Some("mill"));
        assert_eq!(p.repos_of("tooling").collect::<Vec<_>>(), ["mill", "py"]);
        assert_eq!(p.paths("tooling"), [".github"]);
        assert_eq!(p.paths_in("tooling", "py"), ["ci"]);
        assert_eq!(p.owner_of("py", "ci/x.yml"), "tooling");
        assert_eq!(p.owner_of("py", "other"), "py");
        assert_eq!(p.owner_of("mill", ".github/w.yml"), "tooling");
        let listing: Vec<_> = p.repos().map(|(r, ps)| (r, ps.count())).collect();
        assert_eq!(listing, [("default", 1), ("mill", 3), ("py", 2)]);
    }

    #[test]
    fn version_two_round_trips_and_older_files_stay_older() {
        let p = Projects::parse(SPAN).expect("parse");
        let json = p.file_json().expect("json");
        assert!(json.contains("\"version\": 2"), "{json}");
        assert_eq!(Projects::parse(&json).expect("again"), p);
        let plain = Projects::parse(r#"{"repos": {"a": {"x": ["src"]}}}"#).expect("parse");
        let json = plain.file_json().expect("json");
        assert!(
            !json.contains("version"),
            "an older binary must read it: {json}"
        );
        assert_eq!(Projects::parse(&json).expect("again"), plain);
    }

    #[test]
    fn version_two_files_are_validated() {
        let wrap = |p: &str| {
            format!(r#"{{"version": 2, "repos": {{"a": {{}}, "b": {{}}}}, "projects": {{{p}}}}}"#)
        };
        for (project, why) in [
            (
                r#""x": {"repos": {"a": ["p"], "b": ["q"]}}"#,
                "default_repo",
            ),
            (
                r#""x": {"repos": {"a": ["p"], "b": ["q"]}, "default_repo": "c"}"#,
                "not one of",
            ),
            (r#""x": {"repos": {"zz": ["p"]}}"#, "unknown repository"),
            (r#""x": {"repos": {}}"#, "at least one repository"),
            (r#""x": {"repos": {"a": []}}"#, "at least one path"),
            (r#""a": {"repos": {"a": ["p"]}}"#, "own project"),
            (
                r#""x": {"repos": {"a": ["p"]}}, "y": {"repos": {"a": ["p/"]}}"#,
                "claimed by",
            ),
        ] {
            let e = Projects::parse(&wrap(project)).expect_err(project);
            assert!(e.contains(why), "{project}: {e}");
        }
        assert!(
            Projects::parse(r#"{"version": 3}"#)
                .expect_err("v3")
                .contains("version")
        );
        let bad = r#"{"version": 2, "repos": {"a": {"github": "not a slug"}}}"#;
        assert!(Projects::parse(bad).expect_err("slug").contains("github"));
    }

    #[test]
    fn with_project_keeps_spanning_projects_and_repo_facts() {
        let p = Projects::parse(SPAN).expect("parse");
        let more = p
            .with_project("py", "docs", &["docs".to_owned()])
            .expect("add");
        assert_eq!(more.paths_in("tooling", "py"), ["ci"]);
        assert_eq!(more.repo_of("docs"), Some("py"));
        assert!(
            p.with_project("py", "clash", &["ci/sub".to_owned()])
                .is_err()
        );
        assert_eq!(
            more.file_json()
                .map(|j| j.contains("rusty-mill/rusty_mill")),
            Ok(true)
        );
    }
}
