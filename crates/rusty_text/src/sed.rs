//! A real (POSIX-subset) sed engine: multi-command scripts, addresses
//! (line number, `$`, `/regex/`, and ranges), `s///` with backreferences
//! (`\1`, `&`), `p`/`d`/`q`/`=`, and `-n` (suppress default auto-print).
//!
//! Known, deliberate gaps (not implemented): hold space (`h`/`H`/`g`/`G`/
//! `x`), `a`/`i`/`c` text insertion, branching (`b`/`t`/`:label`), and
//! multi-line pattern-space commands (`N`/`D`/`P`). This covers the
//! substitution/filtering workflow that accounts for the large majority of
//! real-world sed usage.

use rusty_regx::Regex;

/// Where a command applies.
enum Address {
    /// A specific 1-based line number.
    Line(usize),
    /// The last line of input.
    Last,
    /// Any line matching this pattern.
    Regex(Regex),
}

impl Address {
    fn matches(&self, line_num: usize, line: &str, is_last: bool) -> bool {
        match self {
            Address::Line(n) => *n == line_num,
            Address::Last => is_last,
            Address::Regex(re) => re.is_match(line),
        }
    }
}

/// A command's address restriction: every line, one address, or a range
/// between two addresses (inclusive, real sed semantics: once the start
/// address matches, every following line is included up to and including
/// the line the end address matches).
enum AddressSpec {
    Every,
    One(Address),
    Range(Address, Address),
}

/// One piece of a parsed `s///` replacement string.
enum ReplPart {
    Literal(String),
    /// `&` or `\0`: the whole match.
    WholeMatch,
    /// `\N`: capture group `N`.
    Group(usize),
}

fn parse_replacement(rep: &str) -> Vec<ReplPart> {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut chars = rep.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '&' => {
                if !literal.is_empty() {
                    parts.push(ReplPart::Literal(std::mem::take(&mut literal)));
                }
                parts.push(ReplPart::WholeMatch);
            }
            '\\' => match chars.peek() {
                Some(d) if d.is_ascii_digit() => {
                    let n = d.to_digit(10).unwrap() as usize;
                    chars.next();
                    if !literal.is_empty() {
                        parts.push(ReplPart::Literal(std::mem::take(&mut literal)));
                    }
                    if n == 0 {
                        parts.push(ReplPart::WholeMatch);
                    } else {
                        parts.push(ReplPart::Group(n));
                    }
                }
                Some('&') => {
                    literal.push('&');
                    chars.next();
                }
                Some('\\') => {
                    literal.push('\\');
                    chars.next();
                }
                _ => literal.push('\\'),
            },
            other => literal.push(other),
        }
    }
    if !literal.is_empty() {
        parts.push(ReplPart::Literal(literal));
    }
    parts
}

fn render_replacement(parts: &[ReplPart], caps: &rusty_regx::Captures) -> String {
    let mut out = String::new();
    for part in parts {
        match part {
            ReplPart::Literal(s) => out.push_str(s),
            ReplPart::WholeMatch => out.push_str(caps.get(0).unwrap_or("")),
            ReplPart::Group(n) => out.push_str(caps.get(*n).unwrap_or("")),
        }
    }
    out
}

/// A parsed `s/pattern/replacement/flags` command.
struct Substitution {
    pattern: Regex,
    replacement: Vec<ReplPart>,
    /// Replace every match from `start_occurrence` onward.
    global: bool,
    /// 1-based index of the first occurrence to replace (default 1).
    start_occurrence: usize,
    /// Also print the resulting line once, regardless of `-n` (sed's `p` flag).
    print: bool,
}

impl Substitution {
    fn apply(&self, line: &str) -> (String, bool) {
        let mut out = String::new();
        let mut last_end = 0;
        let mut occurrence = 0;
        let mut changed = false;

        for caps in self.pattern.captures_iter(line) {
            occurrence += 1;
            if occurrence < self.start_occurrence {
                continue;
            }
            let (start, end) = caps.span(0).unwrap();
            out.push_str(&line[last_end..start]);
            out.push_str(&render_replacement(&self.replacement, &caps));
            last_end = end;
            changed = true;
            if !self.global {
                break;
            }
        }
        out.push_str(&line[last_end..]);
        (out, changed)
    }
}

enum CommandKind {
    Substitute(Substitution),
    Delete,
    Print,
    Quit,
    PrintLineNumber,
}

struct SedCommand {
    address: AddressSpec,
    kind: CommandKind,
}

/// A parsed, ready-to-run sed script.
pub struct SedScript {
    commands: Vec<SedCommand>,
}

/// Per-command range state, tracked across lines during a run. Opaque to
/// callers: construct it with [`SedScript::new_state`], not directly.
#[derive(Default)]
pub struct RangeState {
    in_range: bool,
}

struct Cursor<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> Cursor<'a> {
    fn new(s: &'a str) -> Self {
        Cursor { chars: s.chars().peekable() }
    }

    fn skip_separators(&mut self) {
        while matches!(self.chars.peek(), Some(c) if c.is_whitespace() || *c == ';') {
            self.chars.next();
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn next(&mut self) -> Option<char> {
        self.chars.next()
    }

    /// Reads up to (and consuming) the next unescaped `delim`, unescaping
    /// `\<delim>` to a literal `<delim>` but leaving every other backslash
    /// escape untouched (regex/replacement engines interpret those
    /// themselves).
    fn read_until_delim(&mut self, delim: char) -> Result<String, String> {
        let mut out = String::new();
        loop {
            match self.next() {
                None => return Err(format!("unterminated command, expected '{delim}'")),
                Some(c) if c == delim => return Ok(out),
                Some('\\') => match self.next() {
                    Some(d) if d == delim => out.push(d),
                    Some(d) => {
                        out.push('\\');
                        out.push(d);
                    }
                    None => return Err("unterminated escape at end of script".to_string()),
                },
                Some(c) => out.push(c),
            }
        }
    }
}

impl SedScript {
    /// Parses a script that may contain multiple commands, separated by
    /// `;` or newlines.
    pub fn parse(script: &str) -> Result<Self, String> {
        let mut cursor = Cursor::new(script);
        let mut commands = Vec::new();

        loop {
            cursor.skip_separators();
            if cursor.peek().is_none() {
                break;
            }
            commands.push(Self::parse_command(&mut cursor)?);
        }

        if commands.is_empty() {
            return Err("empty sed script".to_string());
        }
        Ok(SedScript { commands })
    }

    fn parse_address(cursor: &mut Cursor) -> Result<Option<Address>, String> {
        match cursor.peek() {
            Some(c) if c.is_ascii_digit() => {
                let mut num = String::new();
                while matches!(cursor.peek(), Some(c) if c.is_ascii_digit()) {
                    num.push(cursor.next().unwrap());
                }
                Ok(Some(Address::Line(num.parse().map_err(|_| "bad line number".to_string())?)))
            }
            Some('$') => {
                cursor.next();
                Ok(Some(Address::Last))
            }
            Some('/') => {
                cursor.next();
                let pat = cursor.read_until_delim('/')?;
                let re = Regex::new(&pat).map_err(|e| format!("bad address regex: {e:?}"))?;
                Ok(Some(Address::Regex(re)))
            }
            _ => Ok(None),
        }
    }

    fn parse_command(cursor: &mut Cursor) -> Result<SedCommand, String> {
        let first = Self::parse_address(cursor)?;
        let address = match first {
            None => AddressSpec::Every,
            Some(a1) => {
                cursor.skip_separators_no_semicolon();
                if cursor.peek() == Some(',') {
                    cursor.next();
                    cursor.skip_separators_no_semicolon();
                    let a2 = Self::parse_address(cursor)?.ok_or("expected address after ','")?;
                    AddressSpec::Range(a1, a2)
                } else {
                    AddressSpec::One(a1)
                }
            }
        };

        cursor.skip_separators_no_semicolon();
        let letter = cursor.next().ok_or("expected a command letter")?;
        let kind = match letter {
            's' => {
                let delim = cursor.next().ok_or("expected delimiter after 's'")?;
                let pattern_str = cursor.read_until_delim(delim)?;
                let replacement_str = cursor.read_until_delim(delim)?;

                let mut flags = String::new();
                while matches!(cursor.peek(), Some(c) if !c.is_whitespace() && c != ';') {
                    flags.push(cursor.next().unwrap());
                }

                let mut global = false;
                let mut print = false;
                let mut start_occurrence = 1usize;
                let mut case_insensitive = false;
                let mut num_buf = String::new();
                for c in flags.chars() {
                    match c {
                        'g' => global = true,
                        'p' => print = true,
                        'i' | 'I' => case_insensitive = true,
                        d if d.is_ascii_digit() => num_buf.push(d),
                        other => return Err(format!("unknown s/// flag '{other}'")),
                    }
                }
                if !num_buf.is_empty() {
                    start_occurrence = num_buf.parse().map_err(|_| "bad occurrence number".to_string())?;
                }

                let pattern = if case_insensitive {
                    Regex::new_ci(&pattern_str)
                } else {
                    Regex::new(&pattern_str)
                }
                .map_err(|e| format!("bad pattern: {e:?}"))?;

                CommandKind::Substitute(Substitution {
                    pattern,
                    replacement: parse_replacement(&replacement_str),
                    global,
                    start_occurrence,
                    print,
                })
            }
            'd' => CommandKind::Delete,
            'p' => CommandKind::Print,
            'q' => CommandKind::Quit,
            '=' => CommandKind::PrintLineNumber,
            other => return Err(format!("unsupported sed command '{other}'")),
        };

        Ok(SedCommand { address, kind })
    }

    /// Runs this script over `lines`, calling `emit` for each line of
    /// output (in order). `suppress_auto_print` is sed's `-n` flag.
    /// Returns `true` if a `q` command was hit (caller should stop feeding
    /// more input).
    pub fn run_line(
        &self,
        state: &mut Vec<RangeState>,
        line_num: usize,
        is_last: bool,
        line: &str,
        suppress_auto_print: bool,
        mut emit: impl FnMut(&str),
    ) -> bool {
        if state.len() < self.commands.len() {
            state.resize_with(self.commands.len(), RangeState::default);
        }

        let mut current = line.to_string();
        let mut deleted = false;
        let mut quit = false;

        for (i, cmd) in self.commands.iter().enumerate() {
            let applies = match &cmd.address {
                AddressSpec::Every => true,
                AddressSpec::One(addr) => addr.matches(line_num, &current, is_last),
                AddressSpec::Range(start, end) => {
                    let range_state = &mut state[i];
                    if !range_state.in_range {
                        if start.matches(line_num, &current, is_last) {
                            range_state.in_range = true;
                            if end.matches(line_num, &current, is_last) {
                                range_state.in_range = false;
                            }
                            true
                        } else {
                            false
                        }
                    } else {
                        if end.matches(line_num, &current, is_last) {
                            range_state.in_range = false;
                        }
                        true
                    }
                }
            };
            if !applies {
                continue;
            }

            match &cmd.kind {
                CommandKind::Substitute(subst) => {
                    let (result, changed) = subst.apply(&current);
                    current = result;
                    if changed && subst.print {
                        emit(&current);
                    }
                }
                CommandKind::Delete => {
                    deleted = true;
                    break;
                }
                CommandKind::Print => emit(&current),
                CommandKind::Quit => quit = true,
                CommandKind::PrintLineNumber => emit(&line_num.to_string()),
            }
        }

        if !deleted && !suppress_auto_print {
            emit(&current);
        }
        quit
    }

    /// Fresh per-run range-tracking state, sized for this script.
    pub fn new_state(&self) -> Vec<RangeState> {
        self.commands.iter().map(|_| RangeState::default()).collect()
    }
}

impl<'a> Cursor<'a> {
    /// Skips whitespace only (not `;`) — used between an address and its
    /// command letter, or around a range's `,`, where a literal `;` should
    /// not be silently swallowed as a separator.
    fn skip_separators_no_semicolon(&mut self) {
        while matches!(self.chars.peek(), Some(c) if c.is_whitespace()) {
            self.chars.next();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_all(script: &str, input: &[&str], suppress: bool) -> Vec<String> {
        let parsed = SedScript::parse(script).unwrap();
        let mut state = parsed.new_state();
        let mut out = Vec::new();
        for (i, line) in input.iter().enumerate() {
            let is_last = i + 1 == input.len();
            let quit = parsed.run_line(&mut state, i + 1, is_last, line, suppress, |l| out.push(l.to_string()));
            if quit {
                break;
            }
        }
        out
    }

    #[test]
    fn basic_substitution_replaces_first_match_only() {
        let out = run_all("s/foo/bar/", &["foo foo"], false);
        assert_eq!(out, vec!["bar foo"]);
    }

    #[test]
    fn global_flag_replaces_every_match() {
        let out = run_all("s/foo/bar/g", &["foo foo foo"], false);
        assert_eq!(out, vec!["bar bar bar"]);
    }

    #[test]
    fn nth_occurrence_replaces_only_that_one() {
        let out = run_all("s/foo/bar/2", &["foo foo foo"], false);
        assert_eq!(out, vec!["foo bar foo"]);
    }

    #[test]
    fn nth_with_global_replaces_that_one_and_all_after() {
        let out = run_all("s/foo/bar/2g", &["foo foo foo foo"], false);
        assert_eq!(out, vec!["foo bar bar bar"]);
    }

    #[test]
    fn backreferences_in_replacement_work() {
        let out = run_all(r"s/(\w+) (\w+)/\2 \1/", &["hello world"], false);
        assert_eq!(out, vec!["world hello"]);
    }

    #[test]
    fn ampersand_inserts_whole_match() {
        let out = run_all("s/foo/[&]/", &["a foo b"], false);
        assert_eq!(out, vec!["a [foo] b"]);
    }

    #[test]
    fn case_insensitive_flag_matches_regardless_of_case() {
        let out = run_all("s/foo/bar/i", &["FOO"], false);
        assert_eq!(out, vec!["bar"]);
    }

    #[test]
    fn custom_delimiter_is_supported() {
        let out = run_all("s#/usr/bin#/opt/bin#", &["/usr/bin/rgit"], false);
        assert_eq!(out, vec!["/opt/bin/rgit"]);
    }

    #[test]
    fn line_address_restricts_the_command() {
        let out = run_all("2s/x/y/", &["x", "x", "x"], false);
        assert_eq!(out, vec!["x", "y", "x"]);
    }

    #[test]
    fn last_line_address_matches_only_the_final_line() {
        let out = run_all("$s/x/y/", &["x", "x", "x"], false);
        assert_eq!(out, vec!["x", "x", "y"]);
    }

    #[test]
    fn regex_address_restricts_to_matching_lines() {
        let out = run_all("/foo/s/x/y/", &["x", "foo x", "x"], false);
        assert_eq!(out, vec!["x", "foo y", "x"]);
    }

    #[test]
    fn range_address_covers_every_line_in_between_inclusive() {
        let out = run_all("2,4d", &["1", "2", "3", "4", "5"], false);
        assert_eq!(out, vec!["1", "5"]);
    }

    #[test]
    fn regex_range_address_works() {
        let out = run_all("/start/,/end/d", &["a", "start", "b", "end", "c"], false);
        assert_eq!(out, vec!["a", "c"]);
    }

    #[test]
    fn delete_command_drops_matching_lines() {
        let out = run_all("/foo/d", &["foo", "bar", "foo"], false);
        assert_eq!(out, vec!["bar"]);
    }

    #[test]
    fn suppress_auto_print_only_shows_explicit_p() {
        let out = run_all("/foo/p", &["foo", "bar"], true);
        assert_eq!(out, vec!["foo"]);
    }

    #[test]
    fn quit_stops_processing_further_lines() {
        let out = run_all("2q", &["a", "b", "c"], false);
        assert_eq!(out, vec!["a", "b"]);
    }

    #[test]
    fn print_line_number_command() {
        let out = run_all("=", &["a", "b"], true);
        assert_eq!(out, vec!["1", "2"]);
    }

    #[test]
    fn multiple_commands_separated_by_semicolons() {
        let out = run_all("s/a/b/; s/c/d/", &["a c"], false);
        assert_eq!(out, vec!["b d"]);
    }

    #[test]
    fn substitute_pattern_containing_the_command_separator_is_not_mis_split() {
        // The ';' inside the pattern must not be treated as a command
        // separator -- verifies the parser reads delimited fields, not a
        // naive split on ';'.
        let out = run_all(r"s/a;b/x/", &["a;b c"], false);
        assert_eq!(out, vec!["x c"]);
    }
}
