//! A Windows command line, as `CreateProcessW` and cmd.exe read it.
//!
//! The ConPTY backend hands its whole `shell` string to `CreateProcessW`.
//! Our own host spawns a `platform::process::Command`, which names the
//! program apart, so `shell = "wsl -d Ubuntu"` is split here first. The
//! rest goes on the command line verbatim (`Command::raw_args`): the
//! program reads it itself, and `cmd.exe` reads it differently from the
//! C runtime, so requoting it would change what runs.
//!
//! The launcher's own arguments (`rusty_term -- prog arg...`) are quoted
//! for whoever reads them ([`reader`]). cmd.exe doesn't take `\"` as an
//! escape, so C runtime quoting would let an argument close its quotes and
//! run `&` or `|` (BatBadBut, #1279). For cmd and batch files an argument
//! goes bare when cmd sees it as one plain word ([`plain_for_cmd`]), and
//! otherwise by cmd's rules (`winargv::quote_batch_arg_into`).

#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::path::{Path, PathBuf};

const QUOTE: u16 = b'"' as u16;
const CARET: u16 = b'^' as u16;
const PERCENT: u16 = b'%' as u16;
const SLASH: u16 = b'/' as u16;
const SPACE: u16 = b' ' as u16;
const TAB: u16 = b'\t' as u16;

/// Who parses a program's command line after its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reader {
    /// The program's C runtime, by the MSVCRT rules.
    Crt,
    /// cmd.exe itself.
    Cmd,
    /// A batch file, which `CreateProcessW` runs through `cmd /c`.
    Batch,
}

/// Who reads the command line of `program`, by its file name. A bare
/// `cmd` is cmd.exe, as `CreateProcessW` adds `.exe`.
pub(crate) fn reader(program: &str) -> Reader {
    let name = program
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    if name.ends_with(".bat") || name.ends_with(".cmd") {
        Reader::Batch
    } else if name == "cmd" || name == "cmd.exe" {
        Reader::Cmd
    } else {
        Reader::Crt
    }
}

/// Whether cmd.exe reads `arg` as one plain word: nothing it splits on,
/// quotes, expands, escapes or runs as an operator.
pub(crate) fn plain_for_cmd(arg: &str) -> bool {
    !arg.is_empty()
        && !arg
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "\"%!&|<>^(),;=".contains(c))
}

/// Whether cmd.exe ends `line` outside quotes. A quote toggles; outside
/// quotes a caret escapes the next character.
pub(crate) fn quotes_closed(line: &[u16]) -> bool {
    let mut quoted = false;
    let mut units = line.iter();
    while let Some(&u) = units.next() {
        if u == QUOTE {
            quoted = !quoted;
        } else if u == CARET && !quoted {
            units.next();
        }
    }
    !quoted
}

/// What the text before the launcher arguments lets them be, when cmd.exe
/// reads them (for itself or for a batch file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CmdRules {
    /// Whether cmd already has the `/c`, `/k` or `/r` that starts its
    /// command. Before it, an argument holding one would start a command.
    pub(crate) switched: bool,
    /// Why only plain words ([`plain_for_cmd`]) may follow, if so.
    pub(crate) plain_only: Option<&'static str>,
    /// Whether cmd expands `!var!` (`/v` or `/v:on`), inside quotes too.
    pub(crate) delayed: bool,
    /// Whether to [`keep_command_quotes`] once the arguments are on.
    pub(crate) wrap: bool,
}

/// [`CmdRules`] for `line`, whose cmd parameters start at `params` (after
/// the program; 0 when `line` is cmd's raw text or a batch line).
///
/// Only plain words may follow text that leaves a quote open, holds a `%`
/// (cmd expands `%var%` before it reads quotes, and the value may hold
/// one), or whose command was written for cmd's quote stripping: it starts
/// with `""` or follows `/s`. Such a shell keeps its text as written, so
/// its command isn't wrapped, and a quote in an argument would become the
/// line's last one, which cmd drops. Delayed expansion through the
/// registry (`DelayedExpansion`) can't be seen here.
pub(crate) fn cmd_rules(line: &[u16], params: usize, reader: Reader) -> CmdRules {
    let shell = &line[params..];
    let start = command_start(shell);
    let options = &shell[..start.unwrap_or(shell.len())];
    let stripped = reader == Reader::Cmd
        && start.is_some_and(|i| {
            shell.get(i) == Some(&QUOTE)
                && (shell.get(i + 1) == Some(&QUOTE) || has_switch(options, b's'))
        });
    let plain_only = if !quotes_closed(line) {
        Some("the shell leaves a quote open")
    } else if line.contains(&PERCENT) {
        Some("the shell expands a %variable%, which may hold a quote")
    } else if stripped {
        Some("the shell's command is written for cmd's quote stripping")
    } else {
        None
    };
    CmdRules {
        switched: reader == Reader::Batch || start.is_some(),
        plain_only,
        delayed: reader == Reader::Cmd && delayed_expansion(options),
        wrap: reader == Reader::Cmd && !stripped,
    }
}

/// Whether `options` holds the switch `/<letter>`, any case.
fn has_switch(options: &[u16], letter: u8) -> bool {
    options
        .windows(2)
        .any(|w| w[0] == SLASH && w[1] | 0x20 == u16::from(letter))
}

/// Whether cmd's `options` turn delayed expansion on: a `/v` that isn't
/// `/v:off`.
fn delayed_expansion(options: &[u16]) -> bool {
    let off: Vec<u16> = ":off".encode_utf16().collect();
    options.windows(2).enumerate().any(|(i, w)| {
        w[0] == SLASH
            && w[1] | 0x20 == u16::from(b'v')
            && !options[i + 2..]
                .get(..off.len())
                .is_some_and(|rest| rest.iter().zip(&off).all(|(&u, &o)| u | 0x20 == o))
    })
}

/// Wraps cmd.exe's command in one more pair of quotes when it starts with
/// a quote, as `CreateProcessW` does for a batch file. Unless rule 1 of
/// `cmd /?` holds (exactly two quotes, and so on), cmd drops a leading
/// quote and the line's last one, which would turn the arguments' quoting
/// inside out. The command follows the first `/c`, `/k` or `/r` in cmd's
/// parameters, which start at `params` in `line`, after the program.
pub(crate) fn keep_command_quotes(line: &mut Vec<u16>, params: usize) {
    let Some(start) = command_start(&line[params..]).map(|i| params + i) else {
        return;
    };
    if line.get(start) == Some(&QUOTE) {
        line.insert(start, QUOTE);
        line.push(QUOTE);
    }
}

/// Whether `text` holds a `/c`, `/k` or `/r` anywhere. cmd takes the first
/// one in its parameters as the switch that starts its command, even inside
/// quotes or glued to a word (`"x /c y"`, `x/c y`; measured on Windows).
pub(crate) fn has_command_switch(text: &[u16]) -> bool {
    command_start(text).is_some()
}

/// Whether `arg` is exactly a switch that starts cmd's command.
pub(crate) fn is_command_switch(arg: &str) -> bool {
    ["/c", "/k", "/r"]
        .iter()
        .any(|s| arg.eq_ignore_ascii_case(s))
}

/// Where the command starts in cmd's `params`: after its `/c`, `/k` or
/// `/r` switch and any blanks.
fn command_start(params: &[u16]) -> Option<usize> {
    let switch = params
        .windows(2)
        .position(|w| w[0] == SLASH && matches!(w[1] | 0x20, 0x63 | 0x6B | 0x72))?;
    let after = switch + 2;
    let blanks = params[after..]
        .iter()
        .take_while(|&&u| u == SPACE || u == TAB)
        .count();
    Some(after + blanks)
}

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

/// Where `CreateProcessW` finds `program` before it searches PATH, as a
/// full path. A name with a directory part is looked up from our current
/// directory; a bare name in rusty_term's own directory, the current
/// directory, System32, then the Windows directory. A name with no
/// extension gets `.exe`, and is never tried bare, as in `CreateProcessW`:
/// an extensionless file must not hide the program on PATH. `None` leaves
/// the name to `Command`'s PATH search.
#[cfg(windows)]
pub(crate) fn locate(program: &str) -> Option<PathBuf> {
    let bare = !program.contains(['\\', '/', ':']);
    let dirs = if bare {
        search_dirs()
    } else {
        vec![PathBuf::new()]
    };
    locate_in(program, &dirs)
}

/// [`locate`] over the given places.
#[cfg(windows)]
pub(crate) fn locate_in(program: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| with_exe(dir.join(program)))
        .find(|candidate| candidate.is_file())
        .and_then(|found| std::path::absolute(found).ok())
}

/// `path`, with `.exe` added when it has no extension.
#[cfg(windows)]
fn with_exe(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        return path;
    }
    let mut name = path.into_os_string();
    name.push(OsStr::new(".exe"));
    PathBuf::from(name)
}

/// The places `CreateProcessW` searches for a bare name before PATH.
#[cfg(windows)]
fn search_dirs() -> Vec<PathBuf> {
    let own = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let windows = std::env::var_os("SystemRoot").map(PathBuf::from);
    own.into_iter()
        .chain(std::env::current_dir().ok())
        .chain(windows.as_ref().map(|dir| dir.join("System32")))
        .chain(windows)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        CmdRules, Reader, cmd_rules, has_command_switch, is_command_switch, keep_command_quotes,
        plain_for_cmd, quotes_closed, reader, split_program,
    };

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

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// `line` after [`keep_command_quotes`], cmd's parameters at `params`.
    fn kept(line: &str, params: usize) -> String {
        let mut line = wide(line);
        keep_command_quotes(&mut line, params);
        String::from_utf16(&line).expect("utf-16")
    }

    #[test]
    fn cmd_and_batch_files_are_read_by_cmd() {
        assert_eq!(reader("cmd"), Reader::Cmd);
        assert_eq!(reader(r"C:\Windows\System32\CMD.EXE"), Reader::Cmd);
        assert_eq!(reader("C:/tools/x.Bat"), Reader::Batch);
        assert_eq!(reader(r"C:\a b\t.cmd"), Reader::Batch);
        assert_eq!(reader("pwsh"), Reader::Crt);
        assert_eq!(reader(r"C:\cmd\x.exe"), Reader::Crt);
        assert_eq!(reader("cmd.com"), Reader::Crt);
    }

    #[test]
    fn plain_words_are_only_those_cmd_cannot_misread() {
        for arg in ["/c", "dir", r"C:\a\b.txt", "-x", "naïve", "a/c"] {
            assert!(plain_for_cmd(arg), "{arg}");
        }
        for arg in [
            "", "a b", "a\tb", "a\"b", "%x%", "!x!", "a&b", "a|b", "<", ">", "^", "(", ")", ",",
            ";", "=", "a\nb", "a\0b", "a\u{a0}b",
        ] {
            assert!(!plain_for_cmd(arg), "{arg:?}");
        }
    }

    #[test]
    fn quotes_close_as_cmd_counts_them() {
        assert!(quotes_closed(&wide(r#"cmd /c echo a"b c"d"#)));
        assert!(!quotes_closed(&wide(r#"cmd /c echo ""#)));
        // A caret outside quotes escapes a quote; inside, it is plain.
        assert!(quotes_closed(&wide(r#"echo ^""#)));
        assert!(!quotes_closed(&wide(r#"echo ^" ""#)));
        assert!(quotes_closed(&wide(r#"echo "^" x"#)));
    }

    #[test]
    fn a_command_starting_with_a_quote_is_wrapped() {
        assert_eq!(kept(r#"cmd /c "x y" "a&b""#, 3), r#"cmd /c ""x y" "a&b"""#);
        assert_eq!(kept(r#"cmd /K  "x""#, 3), r#"cmd /K  ""x"""#);
        assert_eq!(kept(r#"cmd /r"x""#, 3), r#"cmd /r""x"""#);
        assert_eq!(kept(r#"cmd /s /c "x""#, 3), r#"cmd /s /c ""x"""#);
    }

    #[test]
    fn other_commands_are_left_alone() {
        assert_eq!(kept(r#"cmd /c echo "a&b""#, 3), r#"cmd /c echo "a&b""#);
        assert_eq!(kept(r#"cmd "a&b""#, 3), r#"cmd "a&b""#);
        assert_eq!(kept(r#"cmd /d "a&b""#, 3), r#"cmd /d "a&b""#);
        // A `/c` in the program's own path is not cmd's switch.
        let line = r#"C:/cmd/cmd.exe "a&b""#;
        assert_eq!(kept(line, 14), line);
    }

    #[test]
    fn command_switches_are_found_anywhere() {
        for text in ["/c", "x/C y", r#""a /k b""#, "/r"] {
            assert!(has_command_switch(&wide(text)), "{text}");
        }
        for text in ["/d /q", "c:\\x", "-c"] {
            assert!(!has_command_switch(&wide(text)), "{text}");
        }
        assert!(is_command_switch("/C") && is_command_switch("/k"));
        assert!(!is_command_switch("/cdir") && !is_command_switch("x/c"));
    }

    fn rules(line: &str, params: usize, reader: Reader) -> CmdRules {
        cmd_rules(&wide(line), params, reader)
    }

    #[test]
    fn a_plain_shell_lets_quoted_arguments_follow() {
        let r = rules("cmd /c echo", 3, Reader::Cmd);
        assert_eq!(
            r,
            CmdRules {
                switched: true,
                plain_only: None,
                delayed: false,
                wrap: true
            }
        );
        assert!(!rules("cmd", 3, Reader::Cmd).switched);
        let batch = rules(r#""C:\a b\t.cmd" -x"#, 0, Reader::Batch);
        assert!(batch.switched && !batch.wrap && batch.plain_only.is_none());
        // A quoted command not written for stripping is wrapped (clink).
        let clink = rules(r#"cmd /k "C:\a b\clink.bat" inject"#, 3, Reader::Cmd);
        assert!(clink.wrap && clink.plain_only.is_none());
    }

    #[test]
    fn a_shell_written_for_quote_stripping_takes_plain_words_unwrapped() {
        for line in [
            r#"cmd /c ""C:\a b\x.exe" -v""#,
            r#"cmd /s /c "prog a""#,
            r#"cmd /S/K "x""#,
        ] {
            let r = rules(line, 3, Reader::Cmd);
            assert!(!r.wrap && r.plain_only.is_some(), "{line}");
        }
        // `/s` with an unquoted command strips nothing.
        assert!(rules("cmd /s /c echo", 3, Reader::Cmd).plain_only.is_none());
    }

    #[test]
    fn a_percent_or_open_quote_in_the_shell_takes_plain_words() {
        assert!(
            rules("cmd /c echo %X%", 3, Reader::Cmd)
                .plain_only
                .is_some()
        );
        assert!(
            rules(r#"C:\t.cmd %X%"#, 0, Reader::Batch)
                .plain_only
                .is_some()
        );
        assert!(
            rules(r#"cmd /c echo ""#, 3, Reader::Cmd)
                .plain_only
                .is_some()
        );
    }

    #[test]
    fn delayed_expansion_follows_v_before_the_command() {
        for (line, on) in [
            ("cmd /v:on /c echo", true),
            ("cmd /V /c echo", true),
            ("cmd /v:OFF /c echo", false),
            ("cmd /c findstr /v x", false),
            ("cmd /c echo", false),
        ] {
            assert_eq!(rules(line, 3, Reader::Cmd).delayed, on, "{line}");
        }
    }
}
