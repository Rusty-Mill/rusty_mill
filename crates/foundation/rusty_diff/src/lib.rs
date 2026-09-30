//! `rusty_diff`: Pure Rust implementation of Myers diff algorithm, unified diff formatting, and patch applier.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffOp<T> {
    Keep(T),
    Insert(T),
    Delete(T),
}

/// Upper bound on `old.len() + new.len()` for which the full Myers algorithm's
/// O(D^2)-memory edit-graph trace (one `Vec<isize>` frontier retained per edit
/// distance `0..=D`, where `D` can be as large as `old.len() + new.len()`) is
/// computed. Without this guard, `diff_myers` is reachable with attacker- or
/// user-controlled input of unbounded size (e.g. via `rgit diff` on any
/// substantially modified tracked file) and its memory usage grows
/// quadratically without limit, risking exhaustion on large or pathologically
/// dissimilar inputs. Beyond the cap, `diff_myers` falls back to a trivial
/// linear-memory diff (delete everything from `old`, then insert everything
/// from `new`) instead of the full algorithm, trading diff quality for a hard
/// memory ceiling on inputs that size alone already flags as abnormal.
pub const MAX_DIFF_INPUT_LEN: usize = 20_000;

/// Computes line-by-line or item-by-item diff using the classic Myers algorithm.
///
/// If `old.len() + new.len()` exceeds [`MAX_DIFF_INPUT_LEN`], returns a
/// trivial delete-all/insert-all diff instead of running the full algorithm;
/// see [`MAX_DIFF_INPUT_LEN`] for why.
pub fn diff_myers<T: PartialEq + Clone>(old: &[T], new: &[T]) -> Vec<DiffOp<T>> {
    let n = old.len();
    let m = new.len();

    if n == 0 && m == 0 {
        return Vec::new();
    }

    if n + m > MAX_DIFF_INPUT_LEN {
        return fallback(old, new);
    }

    let max_d = n + m;

    let mut v = vec![0isize; 2 * max_d + 1];
    let offset = max_d as isize;

    let mut trace: Vec<Frontier> = Vec::new();
    let mut cells = 0usize;

    for d in 0..=max_d {
        // Only the band step `d` reads, `[-d-1, d+1]`: the full `v` per
        // step made the trace `(n+m) * (2(n+m)+1)` cells, about 6.4 GB at
        // the input cap (design review 3.8).
        let lo = (offset - d as isize - 1).max(0) as usize;
        let hi = (offset + d as isize + 1).min(2 * max_d as isize) as usize;
        cells += hi - lo + 1;
        if cells > MAX_TRACE_CELLS {
            return fallback(old, new);
        }
        trace.push(Frontier {
            base: lo as isize,
            cells: v[lo..=hi].to_vec(),
        });
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
                return backtrack(&trace, old, new, offset);
            }
            k += 2;
        }
    }

    backtrack(&trace, old, new, offset)
}

/// Most edit-graph trace cells (`isize`s) one [`diff_myers`] call may
/// retain -- 128 MiB on 64-bit. The trace grows with the square of the edit
/// distance, so [`MAX_DIFF_INPUT_LEN`] alone still allowed gigabytes for
/// two dissimilar inputs at the cap (design review 3.8). Past this, the
/// linear delete-all/insert-all diff is returned instead.
pub const MAX_TRACE_CELLS: usize = 16 * 1024 * 1024;

/// One step's saved frontier: `cells[i]` is `v[base + i]`.
struct Frontier {
    base: isize,
    cells: Vec<isize>,
}

impl Frontier {
    fn at(&self, index: isize) -> isize {
        self.cells[(index - self.base) as usize]
    }
}

/// The linear-memory diff: delete all of `old`, then insert all of `new`.
fn fallback<T: Clone>(old: &[T], new: &[T]) -> Vec<DiffOp<T>> {
    let mut result = Vec::with_capacity(old.len() + new.len());
    result.extend(old.iter().cloned().map(DiffOp::Delete));
    result.extend(new.iter().cloned().map(DiffOp::Insert));
    result
}

fn backtrack<T: PartialEq + Clone>(
    trace: &[Frontier],
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

        let prev_k = if k == -d || (k != d && v.at(k - 1 + offset) < v.at(k + 1 + offset)) {
            k + 1
        } else {
            k - 1
        };

        let prev_x = v.at(prev_k + offset);
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
pub fn format_unified_diff(
    old_name: &str,
    new_name: &str,
    old_text: &str,
    new_text: &str,
) -> String {
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();

    let ops = diff_myers(&old_lines, &new_lines);

    let mut out = String::new();
    out.push_str(&format!("--- {}\n", old_name));
    out.push_str(&format!("+++ {}\n", new_name));

    if ops.is_empty() {
        return out;
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

    out
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

        let ops = diff_myers(&old, &new);
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
        let ops = diff_myers(&old, &new);
        assert_eq!(ops, Vec::<DiffOp<&str>>::new());
    }

    #[test]
    fn test_diff_myers_empty_old_nonempty_new() {
        let old: Vec<&str> = vec![];
        let new = vec!["a", "b"];
        let ops = diff_myers(&old, &new);
        assert_eq!(ops, vec![DiffOp::Insert("a"), DiffOp::Insert("b")]);
    }

    #[test]
    fn test_diff_myers_nonempty_old_empty_new() {
        let old = vec!["a", "b"];
        let new: Vec<&str> = vec![];
        let ops = diff_myers(&old, &new);
        assert_eq!(ops, vec![DiffOp::Delete("a"), DiffOp::Delete("b")]);
    }

    #[test]
    fn test_diff_myers_over_cap_uses_bounded_fallback_not_quadratic_trace() {
        // Regression for unbounded O(D^2) trace memory in `diff_myers`
        // (`trace.push(v.clone())` once per edit distance, reachable via
        // `rgit diff` on any sufficiently modified tracked file). `old` and
        // `new` here are *identical*, so a full Myers run (no size guard)
        // would find zero edits and return an all-`Keep` result almost
        // instantly, without ever growing the trace. The size guard must
        // instead trigger purely on `old.len() + new.len()` exceeding
        // `MAX_DIFF_INPUT_LEN` and fall back to the linear delete-all/
        // insert-all diff regardless of content. Asserting that exact
        // delete-then-insert shape (rather than all-`Keep`) proves the
        // size-guarded fallback ran instead of the real algorithm — which is
        // the mechanism that keeps memory bounded no matter how large or
        // pathologically dissimilar `old`/`new` are, instead of growing
        // without bound as input size increases.
        let half = MAX_DIFF_INPUT_LEN;
        let old: Vec<usize> = (0..half).collect();
        let new: Vec<usize> = (0..half).collect();
        assert!(old.len() + new.len() > MAX_DIFF_INPUT_LEN);

        let start = std::time::Instant::now();
        let ops = diff_myers(&old, &new);
        let elapsed = start.elapsed();

        assert_eq!(ops.len(), old.len() + new.len());
        assert!(
            ops[..old.len()]
                .iter()
                .all(|op| matches!(op, DiffOp::Delete(_))),
            "expected the fallback's delete-all prefix, got a real Myers trace instead"
        );
        assert!(
            ops[old.len()..]
                .iter()
                .all(|op| matches!(op, DiffOp::Insert(_))),
            "expected the fallback's insert-all suffix, got a real Myers trace instead"
        );
        assert!(
            elapsed.as_secs() < 5,
            "fallback path must be O(n + m) and complete quickly, took {elapsed:?}"
        );
    }

    #[test]
    fn test_diff_myers_at_cap_still_uses_full_algorithm() {
        // Sanity check on the guard's boundary: input sized at (not over)
        // `MAX_DIFF_INPUT_LEN` must still take the real Myers path and
        // produce a genuine diff (with `Keep`s), not the fallback.
        let old: Vec<usize> = (0..MAX_DIFF_INPUT_LEN / 2 - 1).collect();
        let mut new = old.clone();
        new.push(999_999); // one extra element beyond the shared prefix
        assert!(old.len() + new.len() <= MAX_DIFF_INPUT_LEN);

        let ops = diff_myers(&old, &new);
        assert!(ops.iter().any(|op| matches!(op, DiffOp::Keep(_))));
        assert!(matches!(ops.last(), Some(DiffOp::Insert(999_999))));
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

        let patch = format_unified_diff("a.txt", "b.txt", old_text, new_text);
        assert!(patch.contains("\n---kept\n"));
        assert!(patch.contains("\n+++added\n"));

        let patched = apply_patch(old_text, &patch).expect("patch application failed");
        assert_eq!(patched, new_text);
    }

    #[test]
    fn test_format_and_apply_patch() {
        let old_text = "line 1\nline 2\nline 3";
        let new_text = "line 1\nline 2 modified\nline 3";

        let patch = format_unified_diff("a.txt", "b.txt", old_text, new_text);
        assert!(patch.contains("+line 2 modified"));
        assert!(patch.contains("-line 2"));

        let patched = apply_patch(old_text, &patch).expect("patch application failed");
        assert_eq!(patched, new_text);
    }

    /// Design review 3.8: two dissimilar inputs under `MAX_DIFF_INPUT_LEN`
    /// could still build a multi-gigabyte trace. The cell budget stops it:
    /// fully disjoint inputs at the cap need an edit distance of the whole
    /// input, far past the budget, so the linear fallback is returned.
    #[test]
    fn test_diff_myers_disjoint_inputs_at_cap_stay_within_the_trace_budget() {
        let half = MAX_DIFF_INPUT_LEN / 2;
        let old: Vec<usize> = (0..half).collect();
        let new: Vec<usize> = (half..2 * half).collect();
        let start = std::time::Instant::now();
        let ops = diff_myers(&old, &new);
        assert_eq!(ops.len(), 2 * half);
        assert!(ops[..half].iter().all(|op| matches!(op, DiffOp::Delete(_))));
        assert!(start.elapsed().as_secs() < 5, "took {:?}", start.elapsed());
    }

    /// The banded trace still reconstructs a minimal diff.
    #[test]
    fn test_diff_myers_banded_trace_matches_a_known_edit_script() {
        let old: Vec<char> = "ABCABBA".chars().collect();
        let new: Vec<char> = "CBABAC".chars().collect();
        let ops = diff_myers(&old, &new);
        let edits = ops
            .iter()
            .filter(|op| !matches!(op, DiffOp::Keep(_)))
            .count();
        assert_eq!(edits, 5, "Myers' paper example has D = 5: {ops:?}");
        let rebuilt: String = ops
            .iter()
            .filter_map(|op| match op {
                DiffOp::Keep(c) | DiffOp::Insert(c) => Some(*c),
                DiffOp::Delete(_) => None,
            })
            .collect();
        assert_eq!(rebuilt, "CBABAC");
    }

    /// The banded trace's diffs rebuild both inputs exactly, over many
    /// small pseudo-random inputs (a tiny LCG; no dependency).
    #[test]
    fn test_diff_myers_banded_trace_round_trips_random_inputs() {
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |bound: u64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 33) % bound
        };
        for _ in 0..500 {
            let old: Vec<u64> = (0..next(12)).map(|_| next(4)).collect();
            let new: Vec<u64> = (0..next(12)).map(|_| next(4)).collect();
            let ops = diff_myers(&old, &new);
            let side = |keep_deleted: bool| -> Vec<u64> {
                ops.iter()
                    .filter_map(|op| match op {
                        DiffOp::Keep(v) => Some(*v),
                        DiffOp::Delete(v) if keep_deleted => Some(*v),
                        DiffOp::Insert(v) if !keep_deleted => Some(*v),
                        _ => None,
                    })
                    .collect()
            };
            assert_eq!(side(true), old, "{old:?} -> {new:?}");
            assert_eq!(side(false), new, "{old:?} -> {new:?}");
        }
    }
}
