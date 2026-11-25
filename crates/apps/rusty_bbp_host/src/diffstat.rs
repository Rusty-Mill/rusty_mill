//! Per-file line counts of stored candidate diffs, for the inbox's `acted`
//! rows (#1400). Pure text parsing of unified/git diffs: no git process and no
//! checkout, so it works on exactly what the log's diff artifacts hold.

use rusty_serde::Serialize;
use std::collections::BTreeMap;

/// Files listed by name; the rest are only counted in `more`.
pub const TOP: usize = 10;
/// A listed path is cut to this many characters.
pub const PATH_MAX: usize = 200;

/// One file's changed lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileStat {
    pub path: String,
    pub added: u64,
    pub removed: u64,
}

/// What a candidate's diffs change, capped for display.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Diffstat {
    /// Files touched, listed or not.
    pub files: u64,
    pub added: u64,
    pub removed: u64,
    /// The [`TOP`] files with the most changed lines, ties by path.
    pub top: Vec<FileStat>,
    /// Files touched beyond `top`.
    pub more: u64,
}

/// The stat of `patches`, applied in order. A file touched by several diffs is
/// one file with summed counts. `None` when no patch names a file.
pub fn of(patches: &[&[u8]]) -> Option<Diffstat> {
    let mut files: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for p in patches {
        count(&String::from_utf8_lossy(p), &mut files);
    }
    if files.is_empty() {
        return None;
    }
    let (added, removed) = files.values().fold((0, 0), |a, f| (a.0 + f.0, a.1 + f.1));
    let total = files.len() as u64;
    let mut rows: Vec<FileStat> = files
        .into_iter()
        .map(|(path, (added, removed))| FileStat {
            path: path.chars().take(PATH_MAX).collect(),
            added,
            removed,
        })
        .collect();
    rows.sort_by(|a, b| (b.added + b.removed).cmp(&(a.added + a.removed)));
    rows.truncate(TOP);
    Some(Diffstat {
        files: total,
        added,
        removed,
        more: total - rows.len() as u64,
        top: rows,
    })
}

/// Fold one patch into `files`, keyed by the new-side path (the old side for a
/// deletion). Hunk lengths from each `@@` header decide which lines count, so a
/// removed line that starts with `--` is not mistaken for a file header.
fn count(patch: &str, files: &mut BTreeMap<String, (u64, u64)>) {
    let (mut cur, mut gone): (Option<String>, Option<String>) = (None, None);
    let (mut old, mut new) = (0u64, 0u64);
    for line in patch.lines() {
        if old > 0 || new > 0 {
            let slot = cur.as_ref().and_then(|p| files.get_mut(p));
            match (line.as_bytes().first(), slot) {
                (Some(b'+'), Some(s)) => {
                    s.0 += 1;
                    new = new.saturating_sub(1);
                }
                (Some(b'-'), Some(s)) => {
                    s.1 += 1;
                    old = old.saturating_sub(1);
                }
                (Some(b'\\'), _) => {}
                _ => {
                    old = old.saturating_sub(1);
                    new = new.saturating_sub(1);
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = rest.rsplit_once(" b/").map_or(rest, |(_, b)| b);
            files.entry(path.to_owned()).or_default();
            cur = Some(path.to_owned());
        } else if let Some(p) = line.strip_prefix("--- ") {
            gone = p.strip_prefix("a/").map(str::to_owned);
        } else if let Some(p) = line.strip_prefix("+++ ") {
            // `+++ /dev/null` is a deletion: the file is named by its old path.
            let path = p.strip_prefix("b/").map(str::to_owned).or(gone.take());
            if let Some(path) = path {
                files.entry(path.clone()).or_default();
                cur = Some(path);
            }
        } else if let Some(h) = line.strip_prefix("@@ -") {
            (old, new) = hunk(h);
        }
    }
}

/// `(old, new)` line counts of a hunk header's text after `@@ -`.
fn hunk(h: &str) -> (u64, u64) {
    let len = |s: &str| {
        s.split_once(',')
            .map_or(1, |(_, n)| n.trim().parse().unwrap_or(1))
    };
    let mut parts = h.split_whitespace();
    let old = parts.next().map_or(1, len);
    let new = parts
        .next()
        .and_then(|s| s.strip_prefix('+'))
        .map_or(1, len);
    (old, new)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    const TWO: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n keep\n-old\n+new\n+extra\n keep2\ndiff --git a/b.txt b/b.txt\nnew file mode 100644\n--- /dev/null\n+++ b/b.txt\n@@ -0,0 +1,2 @@\n+x\n+y\n";

    #[test]
    fn counts_added_and_removed_per_file() {
        let s = of(&[TWO.as_bytes()]).unwrap();
        assert_eq!((s.files, s.added, s.removed, s.more), (2, 4, 1, 0));
        assert_eq!(
            s.top[0],
            FileStat {
                path: "src/a.rs".into(),
                added: 2,
                removed: 1
            }
        );
    }

    #[test]
    fn a_removed_line_that_looks_like_a_header_is_a_line() {
        let p = "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1,2 +1,1 @@\n--- not a header\n keep\n";
        let s = of(&[p.as_bytes()]).unwrap();
        assert_eq!((s.files, s.added, s.removed), (1, 0, 1));
    }

    #[test]
    fn plain_unified_diffs_without_git_headers_are_counted_per_file() {
        let p = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+c\n--- a/o\n+++ /dev/null\n@@ -1 +0,0 @@\n-d\n";
        let s = of(&[p.as_bytes()]).unwrap();
        let by = |n: &str| {
            s.top
                .iter()
                .find(|f| f.path == n)
                .map(|f| (f.added, f.removed))
        };
        assert_eq!(
            (by("x"), by("n"), by("o")),
            (Some((1, 1)), Some((1, 0)), Some((0, 1)))
        );
    }

    #[test]
    fn diffs_are_summed_per_file_in_one_stat() {
        let s = of(&[TWO.as_bytes(), TWO.as_bytes()]).unwrap();
        assert_eq!((s.files, s.added, s.removed), (2, 8, 2));
    }

    #[test]
    fn binary_and_rename_only_files_count_with_zero_lines() {
        let p = "diff --git a/i.png b/i.png\nBinary files a/i.png and b/i.png differ\ndiff --git a/o b/n\nsimilarity index 100%\nrename from o\nrename to n\n";
        let s = of(&[p.as_bytes()]).unwrap();
        assert_eq!((s.files, s.added, s.removed), (2, 0, 0));
    }

    #[test]
    fn a_deleted_file_is_named_by_its_old_path() {
        let p = "diff --git a/gone b/gone\ndeleted file mode 100644\n--- a/gone\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n";
        let s = of(&[p.as_bytes()]).unwrap();
        assert_eq!(s.top[0].path, "gone");
        assert_eq!((s.added, s.removed), (0, 2));
    }

    #[test]
    fn the_list_is_capped_and_ordered_by_size_then_path() {
        let mut p = String::new();
        for i in 0..12 {
            let n = if i == 11 { 5 } else { 1 };
            p += &format!(
                "diff --git a/f{i:02} b/f{i:02}\n--- a/f{i:02}\n+++ b/f{i:02}\n@@ -0,0 +1,{n} @@\n"
            );
            p += &"+x\n".repeat(n);
        }
        let s = of(&[p.as_bytes()]).unwrap();
        assert_eq!((s.files, s.top.len(), s.more), (12, TOP, 2));
        assert_eq!(s.top[0].path, "f11");
        assert_eq!(s.top[1].path, "f00");
    }

    #[test]
    fn a_long_path_is_cut_and_an_empty_patch_has_no_stat() {
        let long = "p".repeat(300);
        let p = format!(
            "diff --git a/{long} b/{long}\n--- a/{long}\n+++ b/{long}\n@@ -0,0 +1 @@\n+x\n"
        );
        assert_eq!(of(&[p.as_bytes()]).unwrap().top[0].path.len(), PATH_MAX);
        assert!(of(&[b"" as &[u8]]).is_none());
    }
}
