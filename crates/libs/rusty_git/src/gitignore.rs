//! A minimal, real `.gitignore` matcher — reads the repository root's
//! `.gitignore` (only; no per-directory nested `.gitignore` files, no
//! `.git/info/exclude`, no global gitignore, no `!negation`) and matches
//! simple names, `*`-glob patterns, and directory-only (`name/`) patterns.
//!
//! Existing without this, `status`/`add` walked entire build-output
//! directories (`target/`, hundreds of MB, thousands of files) on every
//! call — both wrong (real git ignores them) and, on any repo with
//! meaningful build artifacts, prohibitively slow. This is the fix for
//! that; see this module's tests for what it does and doesn't cover.

use std::fs;
use std::path::Path;

struct Pattern {
    /// The glob text, without a trailing `/`.
    glob: String,
    /// Only matches directories (the gitignore trailing-`/` rule).
    dir_only: bool,
}

/// A parsed `.gitignore`.
pub struct Gitignore {
    patterns: Vec<Pattern>,
}

impl Gitignore {
    /// Loads `git_dir`'s sibling working tree's root `.gitignore`, or an
    /// empty (matches-nothing) set if none exists.
    pub fn load(work_tree: &Path) -> Self {
        let text = fs::read_to_string(work_tree.join(".gitignore")).unwrap_or_default();
        let patterns = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let dir_only = line.ends_with('/');
                let glob = line.trim_end_matches('/').to_string();
                Pattern { glob, dir_only }
            })
            .collect();
        Gitignore { patterns }
    }

    /// Whether `name` (a single path component's basename, not a full
    /// path) should be skipped. `is_dir` gates directory-only patterns.
    pub fn matches(&self, name: &str, is_dir: bool) -> bool {
        self.patterns.iter().any(|p| {
            if p.dir_only && !is_dir {
                return false;
            }
            glob_match(&p.glob, name)
        })
    }
}

/// A small, `*`-only glob matcher (no `?`, no `[...]`, no `**`) — covers
/// the overwhelming majority of real-world `.gitignore` entries
/// (`target/`, `*.log`, `Cargo.lock`-style exact names).
fn glob_match(pattern: &str, name: &str) -> bool {
    // Memoized on (pattern_pos, name_pos) so an adversarial pattern with
    // several `*` wildcards against a long non-matching name can't
    // backtrack exponentially (O(n^k) for k wildcards, unbounded time)
    // reached via any repo's `.gitignore` processed during `add`/`status`.
    // Bounding the table to O(pattern_len * name_len) states makes this
    // O(n*m) worst case instead.
    fn matches(
        pattern: &[u8],
        name: &[u8],
        memo: &mut std::collections::HashMap<(usize, usize), bool>,
    ) -> bool {
        let key = (pattern.len(), name.len());
        if let Some(&cached) = memo.get(&key) {
            return cached;
        }
        let result = match pattern.first() {
            None => name.is_empty(),
            Some(b'*') => (0..=name.len()).any(|i| matches(&pattern[1..], &name[i..], memo)),
            Some(&c) => name.first() == Some(&c) && matches(&pattern[1..], &name[1..], memo),
        };
        memo.insert(key, result);
        result
    }
    let mut memo = std::collections::HashMap::new();
    matches(pattern.as_bytes(), name.as_bytes(), &mut memo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_gitignore(dir: &Path, contents: &str) {
        fs::write(dir.join(".gitignore"), contents).unwrap();
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rusty_git_gitignore_test_{name}_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_gitignore_matches_nothing() {
        let dir = temp_dir("missing");
        let ig = Gitignore::load(&dir);
        assert!(!ig.matches("target", true));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_only_pattern_matches_only_directories() {
        let dir = temp_dir("dironly");
        write_gitignore(&dir, "target/\n");
        let ig = Gitignore::load(&dir);
        assert!(ig.matches("target", true));
        assert!(!ig.matches("target", false));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn glob_pattern_matches_suffix() {
        let dir = temp_dir("glob");
        write_gitignore(&dir, "*.log\n");
        let ig = Gitignore::load(&dir);
        assert!(ig.matches("debug.log", false));
        assert!(!ig.matches("debug.txt", false));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exact_name_pattern() {
        let dir = temp_dir("exact");
        write_gitignore(&dir, "Cargo.lock\n");
        let ig = Gitignore::load(&dir);
        assert!(ig.matches("Cargo.lock", false));
        assert!(!ig.matches("Cargo.toml", false));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let dir = temp_dir("comments");
        write_gitignore(&dir, "# comment\n\ntarget/\n");
        let ig = Gitignore::load(&dir);
        assert!(ig.matches("target", true));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn adversarial_multi_wildcard_pattern_matches_quickly_instead_of_exponentially_backtracking() {
        // Pre-fix, the naive recursive `matches` backtracks a `*` with no
        // memoization: an adversarial pattern with several `*` wildcards
        // against a long non-matching name is O(n^k) in the wildcard
        // count k, and hangs for seconds-to-minutes at these sizes.
        // Reachable via any repo's `.gitignore` processed during
        // `rgit add`/`status`.
        let pattern = format!("{}!", "*a".repeat(20));
        let name = "a".repeat(35);
        let start = std::time::Instant::now();
        assert!(!glob_match(&pattern, &name));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "glob_match took too long -- exponential backtracking suspected"
        );
    }
}
