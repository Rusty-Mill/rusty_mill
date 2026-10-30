//! Split a Windows command line into a program and its arguments, the way
//! `CreateProcessW` and the MSVC runtime read it back.
//!
//! The ConPTY backend hands its whole `shell` string to `CreateProcessW`.
//! Our own host takes the program and each argument apart
//! (`platform::process::Command`), so `shell = "wsl -d Ubuntu"` is split
//! here first, by the same rules that would have read it.

use std::iter::Peekable;
use std::str::Chars;

/// The words of `line`: the program, then each argument. Empty when `line`
/// holds only whitespace.
///
/// The program follows `CreateProcessW`'s rule: quoted up to the next
/// quote, or else up to the first space or tab, with no escapes. The
/// arguments follow the MSVC runtime's: `2n` backslashes before a quote
/// give `n` backslashes and the quote toggles quoting; `2n + 1` give `n`
/// and a literal quote; `""` inside quotes is a literal quote; other
/// backslashes are literal.
pub(crate) fn split_command_line(line: &str) -> Vec<String> {
    let mut chars = line.chars().peekable();
    skip_blanks(&mut chars);
    let Some(program) = next_program(&mut chars) else {
        return Vec::new();
    };
    let mut words = vec![program];
    loop {
        skip_blanks(&mut chars);
        if chars.peek().is_none() {
            return words;
        }
        words.push(next_arg(&mut chars));
    }
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

fn skip_blanks(chars: &mut Peekable<Chars>) {
    while chars.next_if(|&c| is_blank(c)).is_some() {}
}

fn next_program(chars: &mut Peekable<Chars>) -> Option<String> {
    chars.peek()?;
    if chars.next_if_eq(&'"').is_some() {
        return Some(chars.by_ref().take_while(|&c| c != '"').collect());
    }
    Some(chars.by_ref().take_while(|&c| !is_blank(c)).collect())
}

fn next_arg(chars: &mut Peekable<Chars>) -> String {
    let mut arg = String::new();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => push_backslashes(chars, &mut arg),
            '"' if quoted && chars.next_if_eq(&'"').is_some() => arg.push('"'),
            '"' => quoted = !quoted,
            c if is_blank(c) && !quoted => break,
            c => arg.push(c),
        }
    }
    arg
}

/// One backslash has been read: read the rest of the run and apply the
/// quote rule if a quote follows it.
fn push_backslashes(chars: &mut Peekable<Chars>, arg: &mut String) {
    let mut run = 1;
    while chars.next_if_eq(&'\\').is_some() {
        run += 1;
    }
    if chars.peek() != Some(&'"') {
        arg.extend(std::iter::repeat_n('\\', run));
        return;
    }
    arg.extend(std::iter::repeat_n('\\', run / 2));
    if run % 2 == 1 {
        chars.next();
        arg.push('"');
    }
}

#[cfg(test)]
mod tests {
    use super::split_command_line as split;

    #[test]
    fn a_bare_program_is_one_word() {
        assert_eq!(split("pwsh"), ["pwsh"]);
        assert_eq!(split("  cmd.exe  "), ["cmd.exe"]);
    }

    #[test]
    fn arguments_split_on_spaces_and_tabs() {
        assert_eq!(split("wsl -d\tUbuntu"), ["wsl", "-d", "Ubuntu"]);
        assert_eq!(
            split("cmd /K clink inject"),
            ["cmd", "/K", "clink", "inject"]
        );
    }

    #[test]
    fn a_quoted_program_keeps_its_spaces_and_backslashes() {
        assert_eq!(
            split(r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo"#),
            [r"C:\Program Files\PowerShell\7\pwsh.exe", "-NoLogo"]
        );
        // The program has no escapes, even before a quote.
        assert_eq!(split(r#""C:\dir\" x"#), [r"C:\dir\", "x"]);
    }

    #[test]
    fn quoted_arguments_keep_their_spaces() {
        assert_eq!(
            split(r#"cmd /c "echo hi there""#),
            ["cmd", "/c", "echo hi there"]
        );
        assert_eq!(split(r#"x a"b c"d"#), ["x", "ab cd"]);
        assert_eq!(split(r#"x """#), ["x", ""]);
    }

    #[test]
    fn backslashes_escape_only_before_a_quote() {
        assert_eq!(split(r"x C:\a\b"), ["x", r"C:\a\b"]);
        assert_eq!(split(r#"x a\"b"#), ["x", r#"a"b"#]);
        assert_eq!(split(r#"x a\\"b c""#), ["x", r"a\b c"]);
        assert_eq!(split(r#"x "a\\\"b""#), ["x", r#"a\"b"#]);
        assert_eq!(split(r#"x "C:\dir\\""#), ["x", r"C:\dir\"]);
    }

    #[test]
    fn a_doubled_quote_inside_quotes_is_a_quote() {
        assert_eq!(split(r#"x "a""b""#), ["x", r#"a"b"#]);
    }

    #[test]
    fn an_unclosed_quote_runs_to_the_end() {
        assert_eq!(split(r#"x "a b"#), ["x", "a b"]);
        assert_eq!(split(r#""C:\a b"#), [r"C:\a b"]);
    }

    #[test]
    fn blank_input_has_no_words() {
        assert!(split("").is_empty());
        assert!(split(" \t ").is_empty());
    }
}
