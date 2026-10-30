//! Split the program off a Windows command line, the way `CreateProcessW`
//! reads it.
//!
//! The ConPTY backend hands its whole `shell` string to `CreateProcessW`.
//! Our own host spawns a `platform::process::Command`, which names the
//! program apart, so `shell = "wsl -d Ubuntu"` is split here first. The
//! rest goes on the command line verbatim (`Command::raw_args`): the
//! program reads it itself, and `cmd.exe` reads it differently from the
//! C runtime, so requoting it would change what runs.

/// `line` as (program, the rest), the rest without its leading blanks.
/// `None` when `line` holds only spaces and tabs.
///
/// Quoted, the program runs to the next quote, with no escapes. Unquoted,
/// it is the shortest prefix ending at a space or tab (or the whole line)
/// for which `is_program` holds, so `C:\Program Files\x.exe -a` finds
/// `x.exe`; when none does, it is the first word.
pub(crate) fn split_program(line: &str, is_program: impl Fn(&str) -> bool) -> Option<(&str, &str)> {
    let line = line.trim_start_matches(is_blank);
    if line.is_empty() {
        return None;
    }
    let (program, rest) = match line.strip_prefix('"') {
        Some(quoted) => quoted.split_once('"').unwrap_or((quoted, "")),
        None => line.split_at(unquoted_end(line, is_program)),
    };
    Some((program, rest.trim_start_matches(is_blank)))
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Where the unquoted program at the start of `line` ends.
fn unquoted_end(line: &str, is_program: impl Fn(&str) -> bool) -> usize {
    let first = line.find(is_blank).unwrap_or(line.len());
    line.match_indices(is_blank)
        .map(|(i, _)| i)
        .chain(std::iter::once(line.len()))
        .find(|&i| is_program(&line[..i]))
        .unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::split_program;

    /// Split with no program on disk: the program is the first word.
    fn split(line: &str) -> Option<(&str, &str)> {
        split_program(line, |_| false)
    }

    #[test]
    fn a_bare_program_has_no_rest() {
        assert_eq!(split("pwsh"), Some(("pwsh", "")));
        assert_eq!(split("  cmd.exe  "), Some(("cmd.exe", "")));
    }

    #[test]
    fn the_rest_is_kept_verbatim() {
        assert_eq!(split("wsl -d\tUbuntu"), Some(("wsl", "-d\tUbuntu")));
        assert_eq!(
            split(r#"cmd /c echo a"b c"d"#),
            Some(("cmd", r#"/c echo a"b c"d"#))
        );
    }

    #[test]
    fn a_quoted_program_keeps_its_spaces_and_backslashes() {
        assert_eq!(
            split(r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo"#),
            Some((r"C:\Program Files\PowerShell\7\pwsh.exe", "-NoLogo"))
        );
        // The program has no escapes, even before a quote.
        assert_eq!(split(r#""C:\dir\" x"#), Some((r"C:\dir\", "x")));
    }

    #[test]
    fn an_unclosed_quote_runs_to_the_end() {
        assert_eq!(split(r#""C:\a b"#), Some((r"C:\a b", "")));
    }

    #[test]
    fn an_unquoted_program_is_the_shortest_prefix_that_exists() {
        let pwsh = r"C:\Program Files\PowerShell\7\pwsh.exe";
        let exists = |p: &str| p == pwsh;
        let line = format!("{pwsh} -NoLogo");
        assert_eq!(split_program(&line, exists), Some((pwsh, "-NoLogo")));
        assert_eq!(split_program(pwsh, exists), Some((pwsh, "")));
        // The shortest wins, as CreateProcessW tries `C:\Program` first.
        let short = |p: &str| p == r"C:\Program" || p == pwsh;
        assert_eq!(
            split_program(&line, short),
            Some((r"C:\Program", r"Files\PowerShell\7\pwsh.exe -NoLogo"))
        );
    }

    #[test]
    fn with_no_program_found_the_first_word_is_the_program() {
        assert_eq!(
            split(r"C:\Program Files\x.exe -a"),
            Some((r"C:\Program", r"Files\x.exe -a"))
        );
    }

    #[test]
    fn blank_input_has_no_program() {
        assert_eq!(split(""), None);
        assert_eq!(split(" \t "), None);
    }
}
