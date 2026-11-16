//! A line diff for the edit preview: pure, std only. Shared common head and
//! tail lines are trimmed, then the rest is matched by longest common
//! subsequence. A change too large for that is shown as one block of removed
//! then added lines, which is still correct, only less fine-grained.

/// One line of a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    Same(String),
    Added(String),
    Removed(String),
}

/// Most cells the LCS table may hold (about 16 MiB as `u32`).
const MAX_CELLS: usize = 4_000_000;

/// The lines of `old` turned into the lines of `new`.
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let (a, b): (Vec<&str>, Vec<&str>) = (old.split('\n').collect(), new.split('\n').collect());
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (ma, mb) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut out = same(&a[..head]);
    out.extend(middle(ma, mb));
    out.extend(same(&a[a.len() - tail..]));
    out
}

fn same(lines: &[&str]) -> Vec<DiffLine> {
    lines
        .iter()
        .map(|l| DiffLine::Same((*l).to_owned()))
        .collect()
}

fn middle(a: &[&str], b: &[&str]) -> Vec<DiffLine> {
    let owned = |l: &&str| (*l).to_owned();
    let block = || {
        let removed = a.iter().map(|l| DiffLine::Removed(owned(l)));
        removed
            .chain(b.iter().map(|l| DiffLine::Added(owned(l))))
            .collect()
    };
    if a.is_empty() || b.is_empty() || (a.len() + 1).saturating_mul(b.len() + 1) > MAX_CELLS {
        return block();
    }
    // lcs[i][j]: length of the LCS of a[i..] and b[j..].
    let w = b.len() + 1;
    let mut lcs = vec![0u32; (a.len() + 1) * w];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i * w + j] = if a[i] == b[j] {
                lcs[(i + 1) * w + j + 1] + 1
            } else {
                lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push(DiffLine::Same(owned(&a[i])));
            (i, j) = (i + 1, j + 1);
        } else if j == b.len() || (i < a.len() && lcs[(i + 1) * w + j] >= lcs[i * w + j + 1]) {
            out.push(DiffLine::Removed(owned(&a[i])));
            i += 1;
        } else {
            out.push(DiffLine::Added(owned(&b[j])));
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use DiffLine::{Added as A, Removed as R, Same as S};

    fn s(x: &str) -> String {
        x.to_owned()
    }

    #[test]
    fn identical_text_is_all_same() {
        assert_eq!(diff_lines("a\nb", "a\nb"), [S(s("a")), S(s("b"))]);
    }

    #[test]
    fn a_changed_line_is_removed_then_added_between_its_neighbours() {
        assert_eq!(
            diff_lines("a\nb\nc", "a\nX\nc"),
            [S(s("a")), R(s("b")), A(s("X")), S(s("c"))]
        );
    }

    #[test]
    fn insertions_and_deletions_keep_matching_lines_aligned() {
        assert_eq!(
            diff_lines("a\nb\nc\nd", "a\nc\nd\ne"),
            [S(s("a")), R(s("b")), S(s("c")), S(s("d")), A(s("e"))]
        );
    }

    #[test]
    fn trailing_newline_changes_show_as_an_empty_last_line() {
        assert_eq!(diff_lines("a", "a\n"), [S(s("a")), A(s(""))]);
    }

    #[test]
    fn empty_to_text_and_back() {
        assert_eq!(diff_lines("", "x"), [R(s("")), A(s("x"))]);
        assert_eq!(diff_lines("x", ""), [R(s("x")), A(s(""))]);
    }

    #[test]
    fn a_change_too_big_for_the_table_still_diffs_as_one_block() {
        let old = (0..3000)
            .map(|i| format!("o{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new = (0..3000)
            .map(|i| format!("n{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let d = diff_lines(&old, &new);
        assert_eq!(d.len(), 6000);
        assert!(d[..3000].iter().all(|l| matches!(l, R(_))));
        assert!(d[3000..].iter().all(|l| matches!(l, A(_))));
    }
}
