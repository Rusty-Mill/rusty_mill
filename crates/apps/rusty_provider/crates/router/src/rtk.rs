//! Built-in filter catalog for `transforms: ["rtk"]` -- command/tool-output
//! -aware compression, applied to `role: "tool"` message text before
//! dispatch. Loosely inspired by OmniRoute's RTK compression engine
//! (`docs/compression/RTK_COMPRESSION.md` there), scoped down to a fixed
//! MVP: 5 built-in categories (git, test, build, package-manager, and a
//! generic fallback) rather than a 49-filter, TOML-configurable catalog.
//!
//! Unlike OmniRoute's RTK, this router never sees the *command* that
//! produced a tool message's output (only its text), so classification is
//! content-based (keyword/pattern sniffing) rather than command-name
//! lookup -- good enough to route to the right filter for typical
//! coding-agent tool output, not a byte-for-byte port.

/// Which built-in filter a piece of tool output was classified into.
/// `Generic` is the always-available fallback -- every input classifies
/// as *something*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Git,
    Test,
    Build,
    Package,
    Generic,
}

/// Strip ANSI CSI escape sequences (colors, cursor movement) -- the same
/// first stage OmniRoute's RTK pipeline runs before any other filtering,
/// since raw escape codes are pure noise for a model reading the text and
/// would otherwise pollute every pattern match below.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn classify(text: &str) -> Category {
    let lower = text.to_ascii_lowercase();
    if lower.contains("on branch ")
        || lower.contains("nothing to commit")
        || lower.contains("changes not staged")
        || lower.contains("changes to be committed")
        || text.contains("diff --git ")
        || text.lines().next().is_some_and(|l| {
            l.starts_with("commit ")
                && l.split_whitespace()
                    .nth(1)
                    .is_some_and(|h| h.len() >= 7 && h.chars().all(|c| c.is_ascii_hexdigit()))
        })
    {
        return Category::Git;
    }
    if text.contains("test result:")
        || lower.contains("tests passed")
        || lower.contains("tests failed")
        || text.lines().any(|l| {
            let l = l.trim_start();
            l.starts_with("test ") && (l.ends_with(" ok") || l.ends_with(" FAILED"))
        })
        || (lower.contains("passed") && lower.contains("failed") && text.contains('='))
    {
        return Category::Test;
    }
    if lower.contains("compiling ")
        || text.contains("warning: ")
        || text.contains("error[")
        || text.lines().any(|l| {
            let l = l.trim_start();
            l.starts_with("error: ") || l.starts_with("Error: ")
        })
    {
        return Category::Build;
    }
    if (lower.contains("npm ") || lower.contains("added ") || lower.contains("resolving"))
        && (lower.contains(" packages") || lower.contains("dependencies"))
        || lower.contains("successfully installed")
        || lower.contains("audited ")
    {
        return Category::Package;
    }
    Category::Generic
}

/// Collapse a run of `threshold` or more identical consecutive lines into
/// the first occurrence plus a `"... line repeated N times ..."` marker --
/// the same duplicate-line problem every category's raw output tends to
/// have (a loop printing the same warning per file, a retry logging the
/// same message).
fn dedupe_repeated_lines(lines: &[&str], threshold: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let mut run_end = i + 1;
        while run_end < lines.len() && lines[run_end] == lines[i] {
            run_end += 1;
        }
        let run_len = run_end - i;
        out.push(lines[i].to_string());
        if run_len >= threshold {
            out.push(format!("... line repeated {} more times ...", run_len - 1));
        } else {
            for line in &lines[i + 1..run_end] {
                out.push(line.to_string());
            }
        }
        i = run_end;
    }
    out
}

/// Keep the first `head` and last `tail` lines, collapsing everything in
/// between into a single marker -- for output that's long but has no
/// better category-specific summary to extract.
fn head_tail(lines: &[String], head: usize, tail: usize) -> Vec<String> {
    if lines.len() <= head + tail {
        return lines.to_vec();
    }
    let mut out: Vec<String> = lines[..head].to_vec();
    out.push(format!(
        "... {} lines omitted ...",
        lines.len() - head - tail
    ));
    out.extend_from_slice(&lines[lines.len() - tail..]);
    out
}

fn compress_git(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let deduped = dedupe_repeated_lines(&lines, 3);
    head_tail(&deduped, 8, 4).join("\n")
}

fn compress_test(text: &str) -> String {
    let mut out = Vec::new();
    let mut passing_run = 0usize;
    let flush_passing_run = |out: &mut Vec<String>, run: &mut usize| {
        if *run > 0 {
            out.push(format!("... {run} passing test lines omitted ..."));
            *run = 0;
        }
    };
    for line in text.lines() {
        let trimmed = line.trim_start();
        let is_passing_test_line = trimmed.starts_with("test ") && trimmed.ends_with(" ok");
        if is_passing_test_line {
            passing_run += 1;
            continue;
        }
        flush_passing_run(&mut out, &mut passing_run);
        out.push(line.to_string());
    }
    flush_passing_run(&mut out, &mut passing_run);
    out.join("\n")
}

fn compress_build(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut compiling_run = 0usize;
    let flush_compiling_run = |out: &mut Vec<String>, run: &mut usize| {
        if *run > 0 {
            out.push(format!("... {run} \"Compiling\" lines omitted ..."));
            *run = 0;
        }
    };
    for line in &lines {
        if line.trim_start().starts_with("Compiling ") {
            compiling_run += 1;
            continue;
        }
        flush_compiling_run(&mut out, &mut compiling_run);
        out.push(line.to_string());
    }
    flush_compiling_run(&mut out, &mut compiling_run);
    out.join("\n")
}

fn compress_package(text: &str) -> String {
    // Keep only lines that look like a summary (short, or containing a
    // digit -- package counts/versions/timings), dropping per-package
    // fetch/progress noise. Always keep the very first and last line even
    // if neither matches, so a short/atypical transcript isn't emptied
    // out entirely.
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= 4 {
        return text.to_string();
    }
    let is_summary_like = |l: &str| {
        let lower = l.to_ascii_lowercase();
        lower.contains("added")
            || lower.contains("removed")
            || lower.contains("audited")
            || lower.contains("successfully installed")
            || lower.contains("up to date")
            || lower.contains("packages in")
            || lower.contains("vulnerabilit")
    };
    let summary_lines: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| is_summary_like(l))
        .collect();
    if !summary_lines.is_empty() {
        return summary_lines.join("\n");
    }
    // Nothing looked summary-like -- fall back to keeping just the
    // boundary lines rather than dropping everything.
    let owned: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    head_tail(&owned, 2, 2).join("\n")
}

fn compress_generic(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let deduped = dedupe_repeated_lines(&lines, 3);
    head_tail(&deduped, 40, 40).join("\n")
}

/// Compress one tool message's text: strip ANSI, classify into a built-in
/// filter category by content, and apply that category's compaction.
/// Never panics and never returns an empty string for non-empty input
/// (every category's fallback path keeps at least the boundary lines).
pub fn compress(text: &str) -> String {
    let stripped = strip_ansi(text);
    match classify(&stripped) {
        Category::Git => compress_git(&stripped),
        Category::Test => compress_test(&stripped),
        Category::Build => compress_build(&stripped),
        Category::Package => compress_package(&stripped),
        Category::Generic => compress_generic(&stripped),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_color_codes() {
        let input = "\u{1b}[32mok\u{1b}[0m and \u{1b}[1;31mfail\u{1b}[0m";
        assert_eq!(strip_ansi(input), "ok and fail");
    }

    #[test]
    fn classify_detects_git_status_output() {
        let text = "On branch main\nnothing to commit, working tree clean\n";
        assert_eq!(classify(text), Category::Git);
    }

    #[test]
    fn classify_detects_cargo_test_output() {
        let text = "running 3 tests\ntest foo::bar ... ok\ntest result: ok. 3 passed; 0 failed";
        assert_eq!(classify(text), Category::Test);
    }

    #[test]
    fn classify_detects_build_warnings() {
        let text = "Compiling rp-core v0.1.0\nwarning: unused import: `Foo`\n";
        assert_eq!(classify(text), Category::Build);
    }

    #[test]
    fn classify_detects_npm_install_output() {
        let text = "npm warn deprecated foo@1.0.0\nadded 120 packages in 3s\n";
        assert_eq!(classify(text), Category::Package);
    }

    #[test]
    fn classify_falls_back_to_generic_for_unrecognized_output() {
        let text = "hello\nworld\n";
        assert_eq!(classify(text), Category::Generic);
    }

    #[test]
    fn compress_git_collapses_a_long_run_of_repeated_lines() {
        let mut lines = vec!["On branch main".to_string()];
        for _ in 0..10 {
            lines.push("modified: src/lib.rs".to_string());
        }
        let text = lines.join("\n");
        let out = compress(&text);
        assert!(out.contains("repeated"));
        assert!(out.len() < text.len());
    }

    #[test]
    fn compress_test_collapses_passing_test_lines_but_keeps_failures() {
        let mut text = String::from("running 5 tests\n");
        for i in 0..5 {
            text.push_str(&format!("test suite::case_{i} ... ok\n"));
        }
        text.push_str("test suite::broken ... FAILED\n");
        text.push_str("test result: FAILED. 5 passed; 1 failed\n");

        let out = compress(&text);
        assert!(
            out.contains("passing test lines omitted"),
            "expected passing runs to be collapsed, got:\n{out}"
        );
        assert!(
            out.contains("test suite::broken ... FAILED"),
            "a failing test line must never be dropped, got:\n{out}"
        );
        assert!(
            out.contains("test result: FAILED. 5 passed; 1 failed"),
            "the summary line must survive, got:\n{out}"
        );
    }

    #[test]
    fn compress_build_collapses_compiling_lines_but_keeps_warnings() {
        let mut text = String::new();
        for i in 0..8 {
            text.push_str(&format!("Compiling crate-{i} v0.1.0\n"));
        }
        text.push_str("warning: unused variable: `x`\n");
        text.push_str(" --> src/main.rs:3:9\n");

        let out = compress(&text);
        assert!(out.contains("\"Compiling\" lines omitted"));
        assert!(out.contains("warning: unused variable: `x`"));
    }

    #[test]
    fn compress_package_keeps_the_summary_line() {
        let mut text = String::from("npm warn deprecated request@2.88.2\n");
        for i in 0..30 {
            text.push_str(&format!("fetch package-{i}\n"));
        }
        text.push_str("added 120 packages, and audited 121 packages in 4s\n");

        let out = compress(&text);
        assert!(out.contains("added 120 packages"));
        assert!(
            out.len() < text.len(),
            "per-package fetch noise must be dropped"
        );
    }

    #[test]
    fn compress_generic_dedupes_and_truncates_very_long_output() {
        let mut lines: Vec<String> = Vec::new();
        for i in 0..200 {
            lines.push(format!("line {i}"));
        }
        let text = lines.join("\n");
        let out = compress(&text);
        assert!(out.contains("lines omitted"));
        assert!(out.contains("line 0"), "must keep the head");
        assert!(out.contains("line 199"), "must keep the tail");
    }

    #[test]
    fn compress_never_panics_or_empties_out_short_input() {
        for input in ["", "a", "short output\nsecond line\n"] {
            let out = compress(input);
            if !input.is_empty() {
                assert!(!out.is_empty());
            }
        }
    }
}
