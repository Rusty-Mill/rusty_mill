//! Windows argv → command-line construction — the security boundary for
//! any Windows spawn path (RFC v2 §5.4; extraction map D3).
//!
//! Standalone since the convergence roadmap's Phase 1c: this module used
//! to live inside `platform-windows`, whose Windows-only modules (Dir,
//! Spawner, Job Objects, console raw mode, the curated `ffi`/`sys`
//! trees) a handback consumer (rush, rusty_naner) has no reason to pull
//! in just for safe command-line quoting. `platform-windows` now
//! re-exports this crate for its own internal use; nothing about its
//! consumers changed.
//!
//! `CreateProcessW` takes a single command-line string, not an argv array;
//! how that string is split back into argv depends on the *target*:
//!
//! - A normal executable's C runtime splits with the MSVCRT /
//!   `CommandLineToArgvW` convention. [`quote_arg_into`] implements the
//!   standard library's own algorithm for that convention (extracted from
//!   rush's `winjob::quote_arg`, where it is production-proven).
//! - A batch script (`.bat`/`.cmd`) is run by `cmd.exe`, which parses the
//!   line under **different** rules — quoting for MSVCRT and handing the
//!   result to cmd is the BatBadBut injection class. [`quote_batch_arg_into`]
//!   quotes under cmd's rules and **refuses** arguments cmd cannot
//!   represent safely, per this RFC's refuse-unrepresentable contract.
//!   This is deliberately stricter than what rush ships today (its
//!   background-spawn path quotes batch targets with MSVCRT rules); the
//!   intent is that rush adopts this module at extraction step 2+.
//!
//! The core operates on `&[u16]` wide units — the exact currency of
//! `CreateProcessW` and lossless for WTF-16, and host-independent so this
//! logic is unit-tested, Miri-checked, and fuzzable on every CI leg. The
//! Windows-only [`build_command_line`] wrapper converts from `OsStr` at
//! the WTF-16 boundary (`util::wide`).

#![forbid(unsafe_code)]

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const SPACE: u16 = b' ' as u16;
const TAB: u16 = b'\t' as u16;

/// How the spawned program will parse its command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A normal executable: MSVCRT / `CommandLineToArgvW` rules.
    Executable,
    /// A `.bat`/`.cmd` script: `cmd.exe` rules, refuse-unrepresentable.
    Batch,
}

/// Classify `program` by extension, case-insensitively: `.bat`/`.cmd` →
/// [`Target::Batch`], anything else → [`Target::Executable`]. The caller
/// must pass the *resolved* program (post-PATH/PATHEXT search) — deciding
/// on the unresolved name would miss `foo` resolving to `foo.bat`.
pub fn classify(program: &[u16]) -> Target {
    const BAT: &[u16; 4] = &[b'.' as u16, b'b' as u16, b'a' as u16, b't' as u16];
    const CMD: &[u16; 4] = &[b'.' as u16, b'c' as u16, b'm' as u16, b'd' as u16];
    if program.len() < 4 {
        return Target::Executable;
    }
    let tail = &program[program.len() - 4..];
    let lower: Vec<u16> = tail
        .iter()
        .map(|&u| {
            if (u16::from(b'A')..=u16::from(b'Z')).contains(&u) {
                u + 32
            } else {
                u
            }
        })
        .collect();
    if lower == *BAT || lower == *CMD {
        Target::Batch
    } else {
        Target::Executable
    }
}

fn refuse(what: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::InvalidInput, OsCode::None, what)
}

/// Append `arg`, quoted under the MSVCRT / `CommandLineToArgvW` rules, to
/// `out`. The algorithm is the standard library's own (also rush's):
///
/// - wrap in `"…"` if the arg is empty or contains space/tab/`"`;
/// - `n` backslashes before an embedded `"` become `2n+1` backslashes
///   (`n` to escape themselves, one more so the quote is escaped rather
///   than closing the argument);
/// - trailing backslashes before the closing quote are doubled, or they
///   would escape it instead of standing for themselves.
pub fn quote_arg_into(arg: &[u16], out: &mut Vec<u16>) {
    let quote = arg.is_empty() || arg.iter().any(|&u| u == SPACE || u == TAB || u == QUOTE);
    if quote {
        out.push(QUOTE);
    }
    let mut backslashes = 0usize;
    for &u in arg {
        if u == BACKSLASH {
            backslashes += 1;
        } else {
            if u == QUOTE {
                for _ in 0..=backslashes {
                    out.push(BACKSLASH);
                }
            }
            backslashes = 0;
        }
        out.push(u);
    }
    if quote {
        for _ in 0..backslashes {
            out.push(BACKSLASH);
        }
        out.push(QUOTE);
    }
}

/// Append `arg` quoted for a `cmd.exe`-parsed (batch) command line, or
/// refuse.
///
/// cmd has no general escape for its metacharacters inside arguments; the
/// only robust safe subset is: always wrap in `"…"`, double any embedded
/// `"` (cmd's own quote-escape), and refuse what cannot be made inert:
///
/// - NUL, CR, LF — unrepresentable in a command line at all (a newline
///   ends the cmd parse; injection by construction);
/// - `%` — cmd expands `%VAR%` *inside* double quotes, so an argument
///   carrying `%` can smuggle environment content; there is no
///   context-free escape for it. Refused conservatively (the RFC's
///   refuse-unrepresentable standard); relax only with evidence.
pub fn quote_batch_arg_into(arg: &[u16], out: &mut Vec<u16>) -> Result<()> {
    const NUL: u16 = 0;
    const CR: u16 = b'\r' as u16;
    const LF: u16 = b'\n' as u16;
    const PERCENT: u16 = b'%' as u16;
    if arg
        .iter()
        .any(|&u| u == NUL || u == CR || u == LF || u == PERCENT)
    {
        return Err(refuse("winargv: unrepresentable batch argument"));
    }
    out.push(QUOTE);
    for &u in arg {
        if u == QUOTE {
            out.push(QUOTE); // cmd escapes a quote by doubling it
        }
        out.push(u);
    }
    out.push(QUOTE);
    Ok(())
}

/// Build the full command line for `program` + `args` (wide units, no
/// terminating NUL — the spawn layer owns NUL-termination and mutability
/// requirements). `program` becomes the quoted first token; per-argument
/// quoting follows [`classify`]`(program)`. Any NUL unit anywhere is
/// refused — a command line cannot contain one.
pub fn build_command_line_wide(program: &[u16], args: &[&[u16]]) -> Result<Vec<u16>> {
    if program.is_empty() {
        return Err(refuse("winargv: empty program"));
    }
    if program.contains(&0) || args.iter().any(|a| a.contains(&0)) {
        return Err(refuse("winargv: embedded NUL"));
    }
    let target = classify(program);
    let mut out = Vec::new();
    quote_arg_into(program, &mut out);
    for arg in args {
        out.push(SPACE);
        match target {
            Target::Executable => quote_arg_into(arg, &mut out),
            Target::Batch => quote_batch_arg_into(arg, &mut out)?,
        }
    }
    Ok(out)
}

/// `OsStr` boundary wrapper for [`build_command_line_wide`] (RFC v2 §5.2:
/// the WTF-16 conversion happens in exactly one place, `util::wide`).
#[cfg(windows)]
pub fn build_command_line(
    program: &std::ffi::OsStr,
    args: &[&std::ffi::OsStr],
) -> Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let program_w: Vec<u16> = program.encode_wide().collect();
    let args_w: Vec<Vec<u16>> = args.iter().map(|a| a.encode_wide().collect()).collect();
    let arg_slices: Vec<&[u16]> = args_w.iter().map(Vec::as_slice).collect();
    build_command_line_wide(&program_w, &arg_slices).map_err(|e| e.with_path(program))
}

/// Quote a string argument under POSIX shell rules (`'arg'`).
///
/// Safe against shell injection: if the argument contains single quotes,
/// it closes the quote, escapes the single quote as `'\''`, and reopens it.
/// If the argument contains no shell metacharacters, it is returned unquoted.
///
/// The odd one out in a crate that is otherwise Windows command-line
/// construction: this is the POSIX side of the same question, kept here
/// because `winargv` is already the workspace's home for "turn an
/// argument into something a shell won't reinterpret" and a second crate
/// for one function would be worse.
pub fn posix_quote_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "''".into();
    }
    if !arg.chars().any(|c| {
        matches!(
            c,
            ' ' | '\t'
                | '\n'
                | '\''
                | '"'
                | '\\'
                | '$'
                | '`'
                | '&'
                | '|'
                | ';'
                | '<'
                | '>'
                | '('
                | ')'
                | '{'
                | '}'
                | '*'
                | '?'
                | '['
                | ']'
                | '!'
                | '~'
                | '#'
        )
    }) {
        return arg.into();
    }
    let mut out = String::with_capacity(arg.len() + 4);
    out.push('\'');
    for c in arg.chars() {
        if c == '\'' {
            out.push_str(r"'\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn line(program: &str, args: &[&str]) -> Result<String> {
        let p = w(program);
        let a: Vec<Vec<u16>> = args.iter().map(|s| w(s)).collect();
        let slices: Vec<&[u16]> = a.iter().map(Vec::as_slice).collect();
        build_command_line_wide(&p, &slices)
            .map(|units| String::from_utf16(&units).expect("test inputs are UTF-16"))
    }

    // MSVCRT rules — the cases rush and std both pin.

    #[test]
    fn simple_words_stay_unquoted() {
        assert_eq!(
            line("prog.exe", &["hello", "world"]).unwrap(),
            "prog.exe hello world"
        );
    }

    #[test]
    fn spaces_force_quotes() {
        assert_eq!(
            line("prog.exe", &["hello world"]).unwrap(),
            r#"prog.exe "hello world""#
        );
    }

    #[test]
    fn empty_arg_is_quoted() {
        assert_eq!(line("prog.exe", &[""]).unwrap(), r#"prog.exe """#);
    }

    #[test]
    fn embedded_quote_gets_backslash_escape() {
        assert_eq!(
            line("prog.exe", &[r#"say "hi""#]).unwrap(),
            r#"prog.exe "say \"hi\"""#
        );
    }

    #[test]
    fn backslashes_before_quote_double_plus_one() {
        // 2 backslashes + quote -> 5 backslashes + quote (2n+1).
        assert_eq!(
            line("prog.exe", &[r#"a\\"b"#]).unwrap(),
            r#"prog.exe "a\\\\\"b""#
        );
    }

    #[test]
    fn trailing_backslashes_double_before_closing_quote() {
        assert_eq!(
            line("prog.exe", &[r"dir with space\"]).unwrap(),
            r#"prog.exe "dir with space\\""#
        );
    }

    #[test]
    fn backslashes_without_quotes_pass_through_untouched() {
        assert_eq!(line("prog.exe", &[r"C:\a\b"]).unwrap(), r"prog.exe C:\a\b");
    }

    #[test]
    fn program_with_space_is_quoted_as_first_token() {
        assert_eq!(
            line(r"C:\Program Files\x.exe", &["a"]).unwrap(),
            r#""C:\Program Files\x.exe" a"#
        );
    }

    // Batch (cmd.exe) rules.

    #[test]
    fn batch_args_are_always_quoted_and_quotes_doubled() {
        assert_eq!(
            line(r"C:\s.bat", &["plain", r#"has "quote""#]).unwrap(),
            r#"C:\s.bat "plain" "has ""quote""""#
        );
    }

    #[test]
    fn batch_extension_matching_is_case_insensitive() {
        let e = line(r"C:\S.CMD", &["100%"]).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
    }

    #[test]
    fn batch_refuses_percent_newline_and_cr() {
        for bad in ["100%", "a\nb", "a\rb"] {
            let e = line(r"C:\s.bat", &[bad]).unwrap_err();
            assert_eq!(e.kind, ErrorKind::InvalidInput, "should refuse {bad:?}");
        }
    }

    #[test]
    fn executables_accept_what_batch_refuses() {
        assert_eq!(line("prog.exe", &["100%"]).unwrap(), "prog.exe 100%");
    }

    #[test]
    fn cmdlike_names_without_the_extension_are_executables() {
        assert_eq!(classify(&w("cmd")), Target::Executable);
        assert_eq!(classify(&w("xbat")), Target::Executable);
        assert_eq!(classify(&w("a.bat")), Target::Batch);
        assert_eq!(classify(&w("A.CmD")), Target::Batch);
    }

    // Universal refusals.

    #[test]
    fn nul_is_refused_everywhere() {
        let p = w("prog.exe");
        let bad = vec![b'a' as u16, 0u16];
        let e = build_command_line_wide(&p, &[&bad]).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
        let e = build_command_line_wide(&[0u16], &[]).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
    }

    #[test]
    fn posix_quoting_roundtrips() {
        assert_eq!(posix_quote_arg("simple"), "simple");
        assert_eq!(posix_quote_arg("hello world"), "'hello world'");
        assert_eq!(posix_quote_arg("it's"), r#"'it'\''s'"#);
    }
}
