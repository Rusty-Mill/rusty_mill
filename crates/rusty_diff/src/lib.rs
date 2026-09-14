//! `rusty_diff`: Pure Rust implementation of Myers diff algorithm, unified diff formatting, and patch applier.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffOp<T> {
    Keep(T),
    Insert(T),
    Delete(T),
}

/// Maximum combined input length (`old.len() + new.len()`) accepted by
/// [`diff_myers`].
///
/// The classic Myers algorithm implemented here records one snapshot of the
/// entire edit-graph frontier (`v`, length `2 * (n + m) + 1`) per
/// edit-distance step explored. For two inputs that share no common
/// subsequence, the number of steps explored approaches `n + m`, so the
/// total trace memory is `O((n + m)^2)` with no inherent upper bound (e.g.
/// two independently generated ~50k-line files can demand multiple
/// gigabytes). Rather than let that grow unbounded, inputs whose combined
/// length exceeds this cap are rejected up front; the worst case under the
/// cap is bounded to roughly `16 * MAX_COMBINED_LEN^2` bytes.
pub const MAX_COMBINED_LEN: usize = 10_000;

/// Computes line-by-line or item-by-item diff using the classic Myers algorithm.
///
/// Returns an error instead of diffing if `old.len() + new.len()` exceeds
/// [`MAX_COMBINED_LEN`], to avoid the unbounded `O((n + m)^2)` memory this
/// implementation's trace would otherwise require. See [`MAX_COMBINED_LEN`].
pub fn diff_myers<T: PartialEq + Clone>(old: &[T], new: &[T]) -> Result<Vec<DiffOp<T>>, String> {
    let n = old.len();
    let m = new.len();

    if n == 0 && m == 0 {
        return Ok(Vec::new());
    }

    if n + m > MAX_COMBINED_LEN {
        return Err(format!(
            "diff_myers: combined input length {} (old.len()={}, new.len()={}) exceeds \
             the maximum of {}; this implementation's trace is O((n+m)^2) in the worst \
             case and would risk an out-of-memory abort",
            n + m,
            n,
            m,
            MAX_COMBINED_LEN,
        ));
    }

    let max_d = n + m;

    let mut v = vec![0isize; 2 * max_d + 1];
    let offset = max_d as isize;

    let mut trace = Vec::new();

    for d in 0..=max_d {
        trace.push(v.clone());
        let mut k = -(d as isize);
        while k <= (d as isize) {
            let idx = (k + offset) as usize;
            let mut x = if k == -(d as isize)
                || (k != (d as isize)
                    && v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize])
            {
                v[(k + 1 + offset) as usize]
            } else {
                v[(k - 1 + offset) as usize] + 1
            };
            let mut y = x - k;

            while (x as usize) < n && (y as usize) < m && old[x as usize] == new[y as usize] {
                x += 1;
                y += 1;
            }

            v[idx] = x;

            if x as usize >= n && y as usize >= m {
                return Ok(backtrack(&trace, old, new, offset));
            }
            k += 2;
        }
    }

    Ok(backtrack(&trace, old, new, offset))
}

fn backtrack<T: PartialEq + Clone>(
    trace: &[Vec<isize>],
    old: &[T],
    new: &[T],
    offset: isize,
) -> Vec<DiffOp<T>> {
    let mut x = old.len() as isize;
    let mut y = new.len() as isize;
    let mut result = Vec::new();

    for (d, v) in trace.iter().enumerate().rev() {
        let d = d as isize;
        let k = x - y;

        let prev_k =
            if k == -d || (k != d && v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize]) {
                k + 1
            } else {
                k - 1
            };

        let prev_x = v[(prev_k + offset) as usize];
        let prev_y = prev_x - prev_k;

        while x > prev_x && y > prev_y {
            result.push(DiffOp::Keep(old[(x - 1) as usize].clone()));
            x -= 1;
            y -= 1;
        }

        if d > 0 {
            if x == prev_x {
                result.push(DiffOp::Insert(new[(y - 1) as usize].clone()));
                y -= 1;
            } else {
                result.push(DiffOp::Delete(old[(x - 1) as usize].clone()));
                x -= 1;
            }
        }
    }

    result.reverse();
    result
}

/// Formats a unified diff string between two text strings.
///
/// Returns an error if the underlying [`diff_myers`] call does (see
/// [`MAX_COMBINED_LEN`]).
pub fn format_unified_diff(
    old_name: &str,
    new_name: &str,
    old_text: &str,
    new_text: &str,
) -> Result<String, String> {
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();

    let ops = diff_myers(&old_lines, &new_lines)?;

    let mut out = String::new();
    out.push_str(&format!("--- {}\n", old_name));
    out.push_str(&format!("+++ {}\n", new_name));

    if ops.is_empty() {
        return Ok(out);
    }

    out.push_str(&format!(
        "@@ -1,{} +1,{} @@\n",
        old_lines.len(),
        new_lines.len()
    ));

    for op in ops {
        match op {
            DiffOp::Keep(line) => out.push_str(&format!(" {}\n", line)),
            DiffOp::Delete(line) => out.push_str(&format!("-{}\n", line)),
            DiffOp::Insert(line) => out.push_str(&format!("+{}\n", line)),
        }
    }

    Ok(out)
}

/// Applies a unified diff patch to an original text string.
pub fn apply_patch(original: &str, patch: &str) -> Result<String, String> {
    let orig_lines: Vec<&str> = original.lines().collect();
    let mut result_lines: Vec<&str> = Vec::new();
    let mut orig_idx: usize = 0;

    let patch_lines: Vec<&str> = patch.lines().collect();

    // Before the first "@@" hunk header we are in the file-header region, where
    // "---"/"+++" lines (and anything else) are ignored. Once a hunk header is
    // seen, every subsequent line is a hunk body line whose operation is decided
    // solely by its first byte, until the next "@@" header (or end of patch).
    let mut in_header = true;
    let mut i = 0;
    while i < patch_lines.len() {
        let line = patch_lines[i];

        if in_header {
            if line.starts_with("@@") {
                in_header = false;
            } else {
                i += 1;
                continue;
            }
        }

        let (old_start, old_count, _new_start, new_count) = parse_hunk_header(line)?;
        let hunk_start_idx = old_start.saturating_sub(1);

        if hunk_start_idx < orig_idx {
            return Err(format!(
                "Hunk starting at original line {} overlaps content already applied up to line {}",
                old_start,
                orig_idx + 1
            ));
        }
        if hunk_start_idx > orig_lines.len() {
            return Err(format!(
                "Hunk starting at original line {} is beyond the end of the original ({} lines)",
                old_start,
                orig_lines.len()
            ));
        }

        // Copy the untouched prefix before this hunk's declared start.
        while orig_idx < hunk_start_idx {
            result_lines.push(orig_lines[orig_idx]);
            orig_idx += 1;
        }

        i += 1;
        let mut seen_old = 0usize;
        let mut seen_new = 0usize;
        while i < patch_lines.len() && !patch_lines[i].starts_with("@@") {
            let body_line = patch_lines[i];
            match body_line.as_bytes().first() {
                Some(b'+') => {
                    result_lines.push(&body_line[1..]);
                    seen_new += 1;
                }
                Some(b'-') => {
                    let rest = &body_line[1..];
                    if orig_idx < orig_lines.len() && orig_lines[orig_idx] == rest {
                        orig_idx += 1;
                        seen_old += 1;
                    } else {
                        return Err(format!(
                            "Patch mismatch at line {}: expected '{}'",
                            orig_idx + 1,
                            rest
                        ));
                    }
                }
                Some(b' ') => {
                    let rest = &body_line[1..];
                    if orig_idx < orig_lines.len() && orig_lines[orig_idx] == rest {
                        result_lines.push(rest);
                        orig_idx += 1;
                        seen_old += 1;
                        seen_new += 1;
                    } else {
                        return Err(format!(
                            "Patch mismatch at line {}: expected '{}'",
                            orig_idx + 1,
                            rest
                        ));
                    }
                }
                None => {
                    // Blank hunk-body line with no prefix character represents an
                    // empty context line.
                    if orig_idx < orig_lines.len() && orig_lines[orig_idx].is_empty() {
                        result_lines.push("");
                        orig_idx += 1;
                        seen_old += 1;
                        seen_new += 1;
                    } else {
                        return Err(format!(
                            "Patch mismatch at line {}: expected empty context line",
                            orig_idx + 1
                        ));
                    }
                }
                Some(other) => {
                    return Err(format!(
                        "Malformed hunk body line (unexpected prefix '{}'): '{}'",
                        *other as char, body_line
                    ));
                }
            }
            i += 1;
        }

        if seen_old != old_count || seen_new != new_count {
            return Err(format!(
                "Hunk header declared -{},{} +{},{} but body has {} old-side and {} new-side lines",
                old_start, old_count, _new_start, new_count, seen_old, seen_new
            ));
        }
    }

    // Append any remaining original lines not touched by any hunk.
    while orig_idx < orig_lines.len() {
        result_lines.push(orig_lines[orig_idx]);
        orig_idx += 1;
    }

    Ok(result_lines.join("\n"))
}

/// Parses a unified-diff hunk header of the form `@@ -start,count +start,count @@`,
/// where `,count` may be omitted (implying a count of 1).
fn parse_hunk_header(line: &str) -> Result<(usize, usize, usize, usize), String> {
    let rest = line
        .strip_prefix("@@")
        .ok_or_else(|| format!("Malformed hunk header: '{}'", line))?;
    let end = rest
        .find("@@")
        .ok_or_else(|| format!("Malformed hunk header: '{}'", line))?;
    let coords = rest[..end].trim();
    let mut parts = coords.split_whitespace();
    let old_part = parts
        .next()
        .ok_or_else(|| format!("Malformed hunk header: '{}'", line))?;
    let new_part = parts
        .next()
        .ok_or_else(|| format!("Malformed hunk header: '{}'", line))?;

    let (old_start, old_count) = parse_hunk_range(old_part, '-', line)?;
    let (new_start, new_count) = parse_hunk_range(new_part, '+', line)?;

    Ok((old_start, old_count, new_start, new_count))
}

fn parse_hunk_range(part: &str, sigil: char, full_line: &str) -> Result<(usize, usize), String> {
    let stripped = part
        .strip_prefix(sigil)
        .ok_or_else(|| format!("Malformed hunk header: '{}'", full_line))?;
    if let Some((start, count)) = stripped.split_once(',') {
        let start = start
            .parse::<usize>()
            .map_err(|_| format!("Malformed hunk header: '{}'", full_line))?;
        let count = count
            .parse::<usize>()
            .map_err(|_| format!("Malformed hunk header: '{}'", full_line))?;
        Ok((start, count))
    } else {
        let start = stripped
            .parse::<usize>()
            .map_err(|_| format!("Malformed hunk header: '{}'", full_line))?;
        Ok((start, 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_myers() {
        let old = vec!["apple", "banana", "cherry"];
        let new = vec!["apple", "durian", "cherry"];

        let ops = diff_myers(&old, &new).expect("diff_myers failed");
        assert_eq!(
            ops,
            vec![
                DiffOp::Keep("apple"),
                DiffOp::Delete("banana"),
                DiffOp::Insert("durian"),
                DiffOp::Keep("cherry")
            ]
        );
    }

    #[test]
    fn test_diff_myers_empty_empty_no_panic() {
        let old: Vec<&str> = vec![];
        let new: Vec<&str> = vec![];
        let ops = diff_myers(&old, &new).expect("diff_myers failed");
        assert_eq!(ops, Vec::<DiffOp<&str>>::new());
    }

    #[test]
    fn test_diff_myers_empty_old_nonempty_new() {
        let old: Vec<&str> = vec![];
        let new = vec!["a", "b"];
        let ops = diff_myers(&old, &new).expect("diff_myers failed");
        assert_eq!(ops, vec![DiffOp::Insert("a"), DiffOp::Insert("b")]);
    }

    #[test]
    fn test_diff_myers_nonempty_old_empty_new() {
        let old = vec!["a", "b"];
        let new: Vec<&str> = vec![];
        let ops = diff_myers(&old, &new).expect("diff_myers failed");
        assert_eq!(ops, vec![DiffOp::Delete("a"), DiffOp::Delete("b")]);
    }

    #[test]
    fn test_diff_myers_rejects_oversized_combined_input() {
        // Two inputs whose combined length clearly exceeds `MAX_COMBINED_LEN`, and
        // which share no common subsequence (every element is unique to its own
        // side). Pre-fix, this forced the Myers trace to grow to `O((n+m)^2)` --
        // here that would mean tens of millions of `isize` clones. Post-fix, the
        // guard at the top of `diff_myers` rejects the input up front, before any
        // `O((n+m)^2)` allocation is attempted, so this stays fast regardless of
        // how large `MAX_COMBINED_LEN` is calibrated to be.
        let half = MAX_COMBINED_LEN / 2 + 1;
        let old: Vec<String> = (0..half).map(|i| format!("old-{}", i)).collect();
        let new: Vec<String> = (0..half).map(|i| format!("new-{}", i)).collect();

        let err = diff_myers(&old, &new).expect_err("oversized disjoint input must be rejected");
        assert!(
            err.contains(&MAX_COMBINED_LEN.to_string()),
            "error should mention the cap: {}",
            err
        );
    }

    #[test]
    fn test_apply_patch_hunk_starts_at_nonzero_line() {
        // Hand-crafted patch (not emitted by format_unified_diff) whose hunk starts
        // at original line 4 and targets the second occurrence of a repeated line.
        let original = "x\nb\ny\nb\nz";
        let patch = "--- a.txt\n+++ b.txt\n@@ -4,1 +4,1 @@\n-b\n+B\n";

        let patched = apply_patch(original, patch).expect("patch application failed");
        assert_eq!(patched, "x\nb\ny\nB\nz");
    }

    #[test]
    fn test_apply_patch_multi_hunk() {
        let original = "a\nb\nc\nd\ne\nf\ng\nh";
        let patch = "--- a.txt\n+++ b.txt\n@@ -2,1 +2,1 @@\n-b\n+B\n@@ -6,1 +6,1 @@\n-f\n+F\n";

        let patched = apply_patch(original, patch).expect("patch application failed");
        assert_eq!(patched, "a\nB\nc\nd\ne\nF\ng\nh");
    }

    #[test]
    fn test_apply_patch_rejects_inconsistent_hunk_counts() {
        let original = "a\nb\nc";
        // Header declares a single old-side and new-side line, but the body has two.
        let patch = "--- a.txt\n+++ b.txt\n@@ -2,1 +2,1 @@\n-b\n-c\n+B\n";

        assert!(apply_patch(original, patch).is_err());
    }

    #[test]
    fn test_round_trip_plus_plus_minus_minus_content() {
        // Deleted/inserted content that itself begins with "--"/"++" must not be
        // mistaken for a file header once inside a hunk body.
        let old_text = "header\n--kept\nfooter";
        let new_text = "header\n++added\nfooter";

        let patch = format_unified_diff("a.txt", "b.txt", old_text, new_text)
            .expect("format_unified_diff failed");
        assert!(patch.contains("\n---kept\n"));
        assert!(patch.contains("\n+++added\n"));

        let patched = apply_patch(old_text, &patch).expect("patch application failed");
        assert_eq!(patched, new_text);
    }

    #[test]
    fn test_format_and_apply_patch() {
        let old_text = "line 1\nline 2\nline 3";
        let new_text = "line 1\nline 2 modified\nline 3";

        let patch = format_unified_diff("a.txt", "b.txt", old_text, new_text)
            .expect("format_unified_diff failed");
        assert!(patch.contains("+line 2 modified"));
        assert!(patch.contains("-line 2"));

        let patched = apply_patch(old_text, &patch).expect("patch application failed");
        assert_eq!(patched, new_text);
    }
}
