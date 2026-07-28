//! A real (subset) awk engine: `BEGIN`/`END` blocks, `/regex/` and
//! expression patterns, field variables (`$0`.. `$NF`), `NR`/`NF`/`FS`/
//! `OFS`, arithmetic/relational/logical expressions, string
//! concatenation, `~`/`!~` matching, field/variable assignment, `print`,
//! and `if`/`else`.
//!
//! Known, deliberate gaps (not implemented): user-defined functions,
//! arrays, `for`/`while` loops, `getline`, `printf`, and multi-char `FS`
//! as a real ERE (treated as a literal substring split instead — see
//! [`interp::Interp`]'s module doc). This covers the filter/report
//! workflow that accounts for most real-world one-liner awk usage.

mod ast;
mod interp;
mod lexer;
mod parser;

use ast::Program;
use interp::Interp;

/// A parsed, ready-to-run awk program.
pub struct AwkProgram {
    program: Program,
}

impl AwkProgram {
    /// Parses an awk program's source text.
    pub fn parse(src: &str) -> Result<Self, String> {
        let tokens = lexer::Lexer::new(src).tokenize()?;
        let program = parser::Parser::new(tokens).parse_program()?;
        Ok(AwkProgram { program })
    }

    /// Runs this program: `BEGIN` rules once, then `lines` fed through the
    /// main rules one record at a time, then `END` rules once. `field_sep`
    /// is awk's `FS` (`" "` for the default whitespace-splitting behavior).
    /// Calls `emit` once per output line, in order.
    pub fn run<'a>(&self, lines: impl Iterator<Item = &'a str>, field_sep: &str, mut emit: impl FnMut(&str)) {
        let mut interp = Interp::new(field_sep);
        interp.run_begin(&self.program, &mut emit);
        for line in lines {
            interp.set_record(line);
            interp.run_main_rules(&self.program, &mut emit);
        }
        interp.run_end(&self.program, &mut emit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(script: &str, input: &[&str], fs: &str) -> Vec<String> {
        let prog = AwkProgram::parse(script).unwrap();
        let mut out = Vec::new();
        prog.run(input.iter().copied(), fs, |l| out.push(l.to_string()));
        out
    }

    #[test]
    fn default_action_prints_the_whole_record() {
        let out = run("1", &["hello world"], " ");
        assert_eq!(out, vec!["hello world"]);
    }

    #[test]
    fn print_specific_fields() {
        let out = run("{print $1, $3}", &["apple banana cherry durian"], " ");
        assert_eq!(out, vec!["apple cherry"]);
    }

    #[test]
    fn nr_variable_tracks_record_number() {
        let out = run("{print NR, $0}", &["a", "b"], " ");
        assert_eq!(out, vec!["1 a", "2 b"]);
    }

    #[test]
    fn nf_variable_tracks_field_count() {
        let out = run("{print NF}", &["a b c", "x"], " ");
        assert_eq!(out, vec!["3", "1"]);
    }

    #[test]
    fn regex_pattern_filters_matching_lines() {
        let out = run("/foo/", &["foo bar", "baz qux", "has foo in it"], " ");
        assert_eq!(out, vec!["foo bar", "has foo in it"]);
    }

    #[test]
    fn relational_pattern_on_a_field() {
        let out = run(r#"$1=="foo"{print $2}"#, &["foo bar", "baz qux"], " ");
        assert_eq!(out, vec!["bar"]);
    }

    #[test]
    fn nr_relational_pattern() {
        let out = run("NR==2", &["a", "b", "c"], " ");
        assert_eq!(out, vec!["b"]);
    }

    #[test]
    fn begin_and_end_blocks_run_exactly_once_each() {
        let out = run(r#"BEGIN{print "start"} {print NR} END{print "done"}"#, &["a", "b"], " ");
        assert_eq!(out, vec!["start", "1", "2", "done"]);
    }

    #[test]
    fn arithmetic_on_numeric_fields() {
        let out = run("{print $1+$2}", &["3 4"], " ");
        assert_eq!(out, vec!["7"]);
    }

    #[test]
    fn string_concatenation_of_fields() {
        let out = run("{print $1 $2}", &["foo bar"], " ");
        assert_eq!(out, vec!["foobar"]);
    }

    #[test]
    fn field_assignment_rebuilds_the_record_with_ofs() {
        let out = run(r#"{$1="X"; print}"#, &["a b c"], " ");
        assert_eq!(out, vec!["X b c"]);
    }

    #[test]
    fn custom_field_separator() {
        let out = run("{print $2}", &["a:b:c"], ":");
        assert_eq!(out, vec!["b"]);
    }

    #[test]
    fn if_else_statement() {
        let out = run(r#"{if ($1 > 5) print "big"; else print "small"}"#, &["10", "2"], " ");
        assert_eq!(out, vec!["big", "small"]);
    }

    #[test]
    fn logical_and_or() {
        let out = run(r#"$1 > 1 && $1 < 5 {print "in range"}"#, &["0", "3", "10"], " ");
        assert_eq!(out, vec!["in range"]);
    }

    #[test]
    fn match_and_not_match_operators() {
        let out = run(r#"$0 ~ /foo/ {print "yes"}"#, &["foobar", "baz"], " ");
        assert_eq!(out, vec!["yes"]);

        let out = run(r#"$0 !~ /foo/ {print "no-foo"}"#, &["foobar", "baz"], " ");
        assert_eq!(out, vec!["no-foo"]);
    }

    #[test]
    fn string_vs_numeric_comparison_rules() {
        // Numeric fields compare numerically ("9" < "10" numerically).
        let out = run(r#"$1 < $2 {print "less"}"#, &["9 10"], " ");
        assert_eq!(out, vec!["less"]);
    }

    #[test]
    fn user_defined_variable_persists_across_records() {
        let out = run(r#"{total = total + $1} END{print total}"#, &["1", "2", "3"], " ");
        assert_eq!(out, vec!["6"]);
    }

    #[test]
    fn compound_assignment_operators() {
        let out = run(r#"{total += $1} END{print total}"#, &["1", "2", "3"], " ");
        assert_eq!(out, vec!["6"]);

        let out = run(r#"{n = 10; n -= 3; n *= 2; n /= 2; print n}"#, &["x"], " ");
        assert_eq!(out, vec!["7"]);
    }
}
