//! `rusty_diff`: Pure Rust implementation of Myers diff algorithm, unified diff formatting, and patch applier.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffOp<T> {
    Keep(T),
    Insert(T),
    Delete(T),
}

/// Computes line-by-line or item-by-item diff using the classic Myers algorithm.
pub fn diff_myers<T: PartialEq + Clone>(old: &[T], new: &[T]) -> Vec<DiffOp<T>> {
    let n = old.len();
    let m = new.len();
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
                return backtrack(&trace, old, new, offset);
            }
            k += 2;
        }
    }

    backtrack(&trace, old, new, offset)
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
    let mut result_lines = Vec::new();
    let mut orig_idx = 0;

    let patch_lines: Vec<&str> = patch.lines().collect();

    for line in patch_lines {
        if line.starts_with("---") || line.starts_with("+++") || line.starts_with("@@") {
            continue;
        }

        if let Some(rest) = line.strip_prefix('+') {
            result_lines.push(rest);
        } else if let Some(rest) = line.strip_prefix('-') {
            if orig_idx < orig_lines.len() && orig_lines[orig_idx] == rest {
                orig_idx += 1;
            } else {
                return Err(format!(
                    "Patch mismatch at line {}: expected '{}'",
                    orig_idx + 1,
                    rest
                ));
            }
        } else if let Some(rest) = line.strip_prefix(' ') {
            if orig_idx < orig_lines.len() && orig_lines[orig_idx] == rest {
                result_lines.push(rest);
                orig_idx += 1;
            } else {
                return Err(format!(
                    "Patch mismatch at line {}: expected '{}'",
                    orig_idx + 1,
                    rest
                ));
            }
        }
    }

    // Append any remaining original lines if not consumed
    while orig_idx < orig_lines.len() {
        result_lines.push(orig_lines[orig_idx]);
        orig_idx += 1;
    }

    Ok(result_lines.join("\n"))
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
    fn test_format_and_apply_patch() {
        let old_text = "line 1\nline 2\nline 3";
        let new_text = "line 1\nline 2 modified\nline 3";

        let patch = format_unified_diff("a.txt", "b.txt", old_text, new_text);
        assert!(patch.contains("+line 2 modified"));
        assert!(patch.contains("-line 2"));

        let patched = apply_patch(old_text, &patch).expect("patch application failed");
        assert_eq!(patched, new_text);
    }
}
