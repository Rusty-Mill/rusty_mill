//! Filename globbing — hand-rolled, no external crate.
//!
//! Two layers:
//!   * [`match_component`] matches one path component against a pattern using
//!     `*`, `?`, and `[…]` (with ranges and `!`/`^` negation). A backslash
//!     escapes the next character so quoted metacharacters can be passed
//!     through literally.
//!   * [`glob`] walks the filesystem component-by-component, so `src/*.rs` and
//!     `*/*.rs` work. Unmatched patterns return nothing; the caller falls back
//!     to the literal word (POSIX no-match behaviour).
//!
//! Like a POSIX shell, a leading `.` in a filename is only matched when the
//! pattern's component itself begins with a literal `.` — so `*` skips dotfiles.

use std::fs;
use std::path::{Path, PathBuf};

/// Expand `pattern` against the filesystem, returning matching paths sorted
/// lexically. An empty result means "no match" — the caller keeps the literal.
pub fn glob(pattern: &str) -> Vec<String> {
    let (base, prefix, rest) = if let Some(r) = pattern.strip_prefix('/') {
        (PathBuf::from("/"), String::from("/"), r)
    } else {
        (PathBuf::from("."), String::new(), pattern)
    };

    let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    walk(&base, &segs, 0, &prefix, &mut out);
    out.sort();
    out.dedup();
    out
}

fn walk(dir: &Path, segs: &[&str], i: usize, prefix: &str, out: &mut Vec<String>) {
    let seg = segs[i];
    let is_last = i + 1 == segs.len();

    // A component with no metacharacters is a literal path step: no need to
    // scan the directory, just check it exists / descend into it.
    if !has_meta(seg) {
        let name = unescape(seg);
        let child = dir.join(&name);
        let display = format!("{prefix}{name}");
        if is_last {
            if child.exists() {
                out.push(display);
            }
        } else if child.is_dir() {
            walk(&child, segs, i + 1, &format!("{display}/"), out);
        }
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let pattern_is_dotted = seg.starts_with('.');

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && !pattern_is_dotted {
            continue;
        }
        if !match_component(seg, &name) {
            continue;
        }
        let display = format!("{prefix}{name}");
        if is_last {
            out.push(display);
        } else if entry.path().is_dir() {
            walk(&entry.path(), segs, i + 1, &format!("{display}/"), out);
        }
    }
}

/// Does any unescaped `*`, `?`, or `[` appear in this component?
fn has_meta(seg: &str) -> bool {
    let mut chars = seg.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '*' | '?' | '[' => return true,
            _ => {}
        }
    }
    false
}

/// Strip backslash escapes, yielding the literal text of a component.
fn unescape(seg: &str) -> String {
    let mut out = String::new();
    let mut chars = seg.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Match a single path component against a glob pattern.
pub fn match_component(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = name.chars().collect();
    matches(&p, 0, &s, 0)
}

/// One indivisible piece of a compiled pattern: a run of `*` (matches any
/// number of characters, including zero), `?` (exactly one character), a
/// `[...]` class, or one literal character. Compiling the pattern up front
/// lets `matches` walk it with plain indices instead of re-parsing bracket
/// classes and backslash escapes on every backtrack.
enum Atom {
    Star,
    Any,
    Class(Class),
    Literal(char),
}

impl Atom {
    fn matches(&self, ch: char) -> bool {
        match self {
            Atom::Star => unreachable!("Star is matched as a run, not per-char"),
            Atom::Any => true,
            Atom::Class(class) => class.matches(ch),
            Atom::Literal(c) => *c == ch,
        }
    }
}

/// Compile `p` into a flat sequence of atoms.
fn compile(p: &[char]) -> Vec<Atom> {
    let mut atoms = Vec::new();
    let mut i = 0;
    while i < p.len() {
        match p[i] {
            '*' => {
                atoms.push(Atom::Star);
                i += 1;
            }
            '?' => {
                atoms.push(Atom::Any);
                i += 1;
            }
            '[' => match parse_class(p, i) {
                Some((class, npi)) => {
                    atoms.push(Atom::Class(class));
                    i = npi;
                }
                // Unterminated `[` is a literal bracket.
                None => {
                    atoms.push(Atom::Literal('['));
                    i += 1;
                }
            },
            '\\' => {
                let lit = if i + 1 < p.len() { p[i + 1] } else { '\\' };
                atoms.push(Atom::Literal(lit));
                i += if i + 1 < p.len() { 2 } else { 1 };
            }
            c => {
                atoms.push(Atom::Literal(c));
                i += 1;
            }
        }
    }
    atoms
}

/// Iterative two-pointer wildcard match (the standard `*`-glob algorithm):
/// track the most recently seen `*` (`star_at`) and how far into the string
/// it has been allowed to consume so far (`star_from`). On a mismatch,
/// backtrack by re-trying the `*` against one more character instead of
/// recursing — this keeps the whole match `O(atoms.len() * s.len())` even on
/// adversarial patterns with many `*` segments, unlike naive backtracking
/// recursion which is exponential in the number of `*`s.
fn matches(p: &[char], pi: usize, s: &[char], si: usize) -> bool {
    let atoms = compile(&p[pi..]);
    let s = &s[si..];
    let (m, n) = (s.len(), atoms.len());
    let mut i = 0; // position in s
    let mut j = 0; // position in atoms
    let mut star_at: Option<usize> = None;
    let mut star_from = 0usize;

    while i < m {
        if j < n && !matches!(atoms[j], Atom::Star) && atoms[j].matches(s[i]) {
            i += 1;
            j += 1;
        } else if j < n && matches!(atoms[j], Atom::Star) {
            star_at = Some(j);
            star_from = i;
            j += 1;
        } else if let Some(sj) = star_at {
            j = sj + 1;
            star_from += 1;
            i = star_from;
        } else {
            return false;
        }
    }
    while j < n && matches!(atoms[j], Atom::Star) {
        j += 1;
    }
    j == n
}

struct Class {
    negate: bool,
    ranges: Vec<(char, char)>,
}

impl Class {
    fn matches(&self, ch: char) -> bool {
        let inside = self.ranges.iter().any(|&(lo, hi)| ch >= lo && ch <= hi);
        inside ^ self.negate
    }
}

/// Parse a `[...]` class starting at `start` (the `[`). Returns the class and
/// the index just past the closing `]`, or `None` if there is no closing `]`.
fn parse_class(p: &[char], start: usize) -> Option<(Class, usize)> {
    let mut i = start + 1;
    let mut negate = false;
    if i < p.len() && (p[i] == '!' || p[i] == '^') {
        negate = true;
        i += 1;
    }

    let mut ranges = Vec::new();
    let mut first = true;
    while i < p.len() {
        // A `]` is only the terminator if it isn't the very first class member.
        if p[i] == ']' && !first {
            return Some((Class { negate, ranges }, i + 1));
        }
        first = false;
        if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
            ranges.push((p[i], p[i + 2]));
            i += 3;
        } else {
            ranges.push((p[i], p[i]));
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_matches_within_component() {
        assert!(match_component("*.rs", "lexer.rs"));
        assert!(match_component("*", "anything"));
        assert!(match_component("a*c", "abbbc"));
        assert!(!match_component("*.rs", "lexer.txt"));
    }

    #[test]
    fn question_matches_one_char() {
        assert!(match_component("?.rs", "a.rs"));
        assert!(!match_component("?.rs", "ab.rs"));
    }

    #[test]
    fn char_classes() {
        assert!(match_component("[abc].rs", "a.rs"));
        assert!(match_component("[a-z].rs", "m.rs"));
        assert!(!match_component("[a-z].rs", "M.rs"));
        assert!(match_component("[!0-9]*", "abc"));
        assert!(!match_component("[!0-9]*", "9bc"));
    }

    #[test]
    fn escaped_metachars_are_literal() {
        assert!(match_component("a\\*b", "a*b"));
        assert!(!match_component("a\\*b", "axb"));
    }

    #[test]
    fn unterminated_class_is_literal_bracket() {
        assert!(match_component("[abc", "[abc"));
    }

    #[test]
    fn glob_against_known_files() {
        // Run from the crate root, these files are stable fixtures. (Vendored
        // into the Nexus workspace, this crate has no per-crate `Cargo.lock`, so
        // assert the manifest is matched rather than an exact lockfile-dependent
        // list.)
        let m = glob("Cargo.*");
        assert!(m.contains(&"Cargo.toml".to_string()));

        assert_eq!(glob("src/lexer.rs"), vec!["src/lexer.rs"]);
        assert!(glob("src/*.rs").contains(&"src/glob.rs".to_string()));
        assert!(glob("no-such-file-*.zzz").is_empty());
    }

    #[test]
    fn adversarial_star_pattern_matches_in_bounded_time() {
        // ~30 `a*` segments against a non-matching string of similar length
        // is the classic catastrophic-backtracking trigger for naive
        // recursive `*` matching (exponential in the number of `*`s). The
        // iterative two-pointer algorithm is O(pattern * string), so this
        // must return well under a second even though it never matches.
        use std::time::{Duration, Instant};

        let pattern = "a*".repeat(30);
        let subject = "a".repeat(29) + "b"; // one char short of matching, and
        // ends in a character the pattern can never place, forcing every
        // `*` to exhaust its backtracking budget.

        let start = Instant::now();
        let result = match_component(&pattern, &subject);
        let elapsed = start.elapsed();

        assert!(!result);
        assert!(
            elapsed < Duration::from_secs(1),
            "matching took {elapsed:?}, exponential backtracking regressed"
        );
    }
}
