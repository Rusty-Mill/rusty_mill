//! Pre-run checks on an agents file: every launched role pins a model, every
//! pinned model has a price, and the two arms of a pair pin the same models.
//! A pin is the `--model ID` or `--model=ID` argument of the launched program;
//! argument boundaries are kept, so prompt text that mentions the flag pins
//! nothing. A `sh -c` launcher is parsed with shell quoting and only its
//! `claude` commands count; a line that cannot be read literally is an error.
//! The checks read the file, not the run: `bbp pairs` repeats them on what the
//! harnesses actually used.

use crate::moderator::{Launcher, Launchers};
use crate::tokens::Criteria;
use std::collections::{BTreeMap, BTreeSet};

/// One word of a shell command line. `dynamic`: it contains an expansion
/// (`$`, a backtick) that is live, so its text is not what the shell will pass.
struct Word {
    text: String,
    dynamic: bool,
}

/// The command lines read so far, and the word being built.
#[derive(Default)]
struct Lexer {
    commands: Vec<Vec<Word>>,
    words: Vec<Word>,
    cur: String,
    /// A word is being built (an empty `""` is still a word).
    live: bool,
    dynamic: bool,
}

impl Lexer {
    fn end_word(&mut self) {
        if self.live {
            self.words.push(Word {
                text: std::mem::take(&mut self.cur),
                dynamic: self.dynamic,
            });
            self.live = false;
            self.dynamic = false;
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        if !self.words.is_empty() {
            self.commands.push(std::mem::take(&mut self.words));
        }
    }

    fn push(&mut self, c: char) {
        self.live = true;
        self.cur.push(c);
    }

    fn single_quoted(&mut self, chars: &mut std::str::Chars) -> Result<(), String> {
        self.live = true;
        for q in chars.by_ref() {
            if q == '\'' {
                return Ok(());
            }
            self.cur.push(q);
        }
        Err("unterminated single quote".into())
    }

    fn double_quoted(&mut self, chars: &mut std::str::Chars) -> Result<(), String> {
        self.live = true;
        while let Some(q) = chars.next() {
            match q {
                '"' => return Ok(()),
                '\\' => match chars.next() {
                    Some(e @ ('"' | '\\' | '$' | '`')) => self.cur.push(e),
                    Some('\n') => {}
                    Some(e) => {
                        self.cur.push('\\');
                        self.cur.push(e);
                    }
                    None => break,
                },
                q => {
                    self.dynamic |= matches!(q, '$' | '`');
                    self.cur.push(q);
                }
            }
        }
        Err("unterminated double quote".into())
    }
}

/// Split a shell command line into simple commands of words, the way a shell
/// would for the constructs launchers use: single and double quotes, backslash
/// escapes, and `;` `&` `|` newline between commands. Anything that cannot be
/// read without running the shell (an unterminated quote, a subshell) is an
/// error: an opaque launcher cannot be shown to pin a model.
fn shell_commands(script: &str) -> Result<Vec<Vec<Word>>, String> {
    let mut lx = Lexer::default();
    let mut chars = script.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => lx.single_quoted(&mut chars)?,
            '"' => lx.double_quoted(&mut chars)?,
            '\\' => match chars.next() {
                Some('\n') => {}
                Some(e) => lx.push(e),
                None => return Err("trailing backslash".into()),
            },
            '$' | '`' => {
                lx.dynamic = true;
                lx.push(c);
            }
            '(' | ')' if !lx.live => {
                return Err("a subshell is not read literally".into());
            }
            ';' | '&' | '|' | '\n' => lx.end_command(),
            c if c.is_whitespace() => lx.end_word(),
            c => lx.push(c),
        }
    }
    lx.end_command();
    Ok(lx.commands)
}

/// The command word of a simple command, past `NAME=value` assignments and
/// the wrappers that run the next word as is.
fn command_index(words: &[Word]) -> Option<usize> {
    words.iter().position(|w| {
        let assignment = w.text.split_once('=').is_some_and(|(n, _)| {
            !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_')
        });
        !assignment
            && !matches!(
                w.text.as_str(),
                "exec" | "env" | "command" | "nohup" | "time"
            )
    })
}

/// The models named by `--model` in one argv, where each item is one argument
/// as the program will receive it: `--model ID` (the next argument) or
/// `--model=ID`. Nothing is split: an argument that merely contains the text
/// is a prompt, not a flag.
fn argv_models<'a>(argv: impl Iterator<Item = (&'a str, bool)>) -> Result<Vec<String>, String> {
    let argv: Vec<(&str, bool)> = argv.collect();
    let mut found = Vec::new();
    for (i, (arg, _)) in argv.iter().enumerate() {
        let value = if let Some(v) = arg.strip_prefix("--model=") {
            Some((v, argv[i].1))
        } else if *arg == "--model" {
            argv.get(i + 1).copied()
        } else {
            None
        };
        if let Some((v, dynamic)) = value {
            if dynamic {
                return Err(format!("the value of --model ({v:?}) is not a literal"));
            }
            if !v.is_empty() {
                found.push(v.to_owned());
            }
        }
    }
    Ok(found)
}

fn is_shell(program: &str) -> bool {
    let name = program.rsplit('/').next().unwrap_or(program);
    matches!(name, "sh" | "bash" | "dash" | "zsh" | "ksh")
}

/// The model a launcher pins. `Ok(None)` when it pins none; an error when it
/// pins two different ones (the harness would take only one) or when it is a
/// shell line that cannot be read literally.
///
/// A direct launcher is its argument list: argument boundaries are kept. A
/// shell launcher (`sh -c SCRIPT`) is parsed with shell quoting, and only the
/// arguments of a `claude` command in it count, so `--model` inside a quoted
/// prompt, or in another command, pins nothing.
pub fn pinned_model(l: &Launcher) -> Result<Option<String>, String> {
    let models = match l.args.iter().position(|a| a == "-c") {
        Some(i) if is_shell(&l.program) => {
            let script = l.args.get(i + 1).map_or("", String::as_str);
            let mut all = Vec::new();
            for words in shell_commands(script)? {
                let Some(at) = command_index(&words) else {
                    continue;
                };
                if words[at].text.rsplit('/').next() != Some("claude") {
                    continue;
                }
                all.extend(argv_models(
                    words[at + 1..].iter().map(|w| (w.text.as_str(), w.dynamic)),
                )?);
            }
            all
        }
        _ => argv_models(l.args.iter().map(|a| (a.as_str(), false)))?,
    };
    let found: BTreeSet<String> = models.into_iter().collect();
    let mut it = found.into_iter();
    match (it.next(), it.next()) {
        (None, _) => Ok(None),
        (Some(m), None) => Ok(Some(m)),
        (Some(a), Some(b)) => Err(format!("pins both {a} and {b}")),
    }
}

/// Role to model for every role in the agents file. Every role must pin one.
pub fn pins(l: &Launchers) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut missing = Vec::new();
    for (role, launcher) in l.iter() {
        match pinned_model(launcher).map_err(|e| format!("role {role}: {e}"))? {
            Some(m) => {
                out.insert(role.to_owned(), m);
            }
            None => missing.push(role),
        }
    }
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(format!(
            "roles without a pinned model (--model ID): {}",
            missing.join(", ")
        ))
    }
}

/// [`pins`], and every pinned model has a price in `criteria`.
pub fn check_run(l: &Launchers, criteria: &Criteria) -> Result<BTreeMap<String, String>, String> {
    let p = pins(l)?;
    let unpriced: BTreeSet<&str> = p
        .values()
        .map(String::as_str)
        .filter(|m| !criteria.models.contains_key(*m))
        .collect();
    if unpriced.is_empty() {
        Ok(p)
    } else {
        Err(format!(
            "pinned models with no price in the criteria file: {unpriced:?}"
        ))
    }
}

/// The arms' pins agree: a role in both pins the same model, and a role only
/// the treatment launches (the extra author) pins a model the control also uses.
pub fn check_arms(
    control: &BTreeMap<String, String>,
    treatment: &BTreeMap<String, String>,
) -> Result<(), String> {
    let control_models: BTreeSet<&String> = control.values().collect();
    for (role, m) in treatment {
        match control.get(role) {
            Some(c) if c != m => {
                return Err(format!(
                    "role {role} pins {c} in the control and {m} in the treatment"
                ))
            }
            None if !control_models.contains(m) => {
                return Err(format!(
                    "extra role {role} pins {m}, which the control never uses"
                ))
            }
            _ => {}
        }
    }
    // A control role the treatment drops is not an error: the treatment may
    // launch fewer roles only if the agents files say so, and `bbp pairs`
    // sees what ran.
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::tokens::Prices;

    fn launcher(program: &str, args: &[&str]) -> Launcher {
        Launcher {
            program: program.into(),
            args: args.iter().map(|a| (*a).into()).collect(),
            builtin_tools: false,
        }
    }

    fn criteria(models: &[&str]) -> Criteria {
        let p = Prices {
            input: 1.0,
            output: 1.0,
            cache_creation_5m: 1.0,
            cache_creation_1h: 1.0,
            cache_read: 1.0,
        };
        Criteria {
            prices_as_of: "t".into(),
            models: models.iter().map(|m| ((*m).to_owned(), p)).collect(),
            min_commit: None,
        }
    }

    #[test]
    fn a_pin_is_found_as_two_words_one_word_or_inside_a_shell_command() {
        let two = launcher("claude", &["-p", "x", "--model", "m1"]);
        assert_eq!(pinned_model(&two).unwrap(), Some("m1".into()));
        let one = launcher("claude", &["--model=m2", "-p", "x"]);
        assert_eq!(pinned_model(&one).unwrap(), Some("m2".into()));
        let sh = launcher(
            "sh",
            &[
                "-c",
                "set -e; exec claude -p \"do it\" --model m3 --tools Bash",
            ],
        );
        assert_eq!(pinned_model(&sh).unwrap(), Some("m3".into()));
        let quoted = launcher("sh", &["-c", "exec claude --model 'm4' -p x"]);
        assert_eq!(pinned_model(&quoted).unwrap(), Some("m4".into()));
    }

    #[test]
    fn no_pin_is_none_and_two_different_pins_are_an_error() {
        assert_eq!(
            pinned_model(&launcher("claude", &["-p", "x"])).unwrap(),
            None
        );
        // `--model` with nothing after it pins nothing.
        assert_eq!(
            pinned_model(&launcher("claude", &["--model"])).unwrap(),
            None
        );
        let two = launcher("sh", &["-c", "claude --model a; claude --model b"]);
        assert!(pinned_model(&two).unwrap_err().contains("both"));
        // The same model named twice is one pin.
        let same = launcher("sh", &["-c", "claude --model a && claude --model a"]);
        assert_eq!(pinned_model(&same).unwrap(), Some("a".into()));
    }

    #[test]
    fn every_launched_role_must_pin_and_the_error_names_the_ones_that_do_not() {
        let l = Launchers::default()
            .with("planner", launcher("claude", &["--model", "m1"]))
            .with("coder", launcher("claude", &["-p", "x"]))
            .with("tester", launcher("claude", &[]));
        let err = pins(&l).unwrap_err();
        assert!(
            err.contains("coder") && err.contains("tester") && !err.contains("planner"),
            "{err}"
        );
    }

    #[test]
    fn a_pinned_model_without_a_price_is_refused() {
        let l = Launchers::default()
            .with("planner", launcher("claude", &["--model", "m1"]))
            .with("coder", launcher("claude", &["--model", "m2"]));
        assert!(check_run(&l, &criteria(&["m1", "m2"])).is_ok());
        let err = check_run(&l, &criteria(&["m1"])).unwrap_err();
        assert!(err.contains("m2") && err.contains("no price"), "{err}");
    }

    fn map(p: &[(&str, &str)]) -> BTreeMap<String, String> {
        p.iter()
            .map(|(r, m)| ((*r).to_owned(), (*m).to_owned()))
            .collect()
    }

    #[test]
    fn arms_must_pin_the_same_model_per_role_and_an_extra_role_a_model_the_control_uses() {
        let control = map(&[("planner", "m1"), ("coder", "m2")]);
        let same_plus_extra = map(&[("planner", "m1"), ("coder", "m2"), ("planner2", "m1")]);
        assert!(check_arms(&control, &same_plus_extra).is_ok());
        let differs = map(&[("planner", "m9"), ("coder", "m2")]);
        let err = check_arms(&control, &differs).unwrap_err();
        assert!(
            err.contains("planner") && err.contains("m1") && err.contains("m9"),
            "{err}"
        );
        let extra_new = map(&[("planner", "m1"), ("planner2", "m9")]);
        assert!(check_arms(&control, &extra_new)
            .unwrap_err()
            .contains("extra role"));
    }

    #[test]
    fn prompt_text_that_mentions_the_flag_is_not_a_pin() {
        // The argument is one word of argv, whatever it contains.
        let direct = launcher("claude", &["-p", "Explain --model claude-opus-5-5"]);
        assert_eq!(pinned_model(&direct).unwrap(), None);
        let alone = launcher("claude", &["-p", "--model claude-opus-5-5"]);
        assert_eq!(pinned_model(&alone).unwrap(), None);
        // And in a shell command line, a quoted word stays one word.
        for script in [
            r#"exec claude -p "Explain --model m1" --tools Bash"#,
            "exec claude -p 'Explain --model m1' --tools Bash",
            r#"exec claude -p "say \"--model m1\" aloud""#,
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert_eq!(pinned_model(&sh).unwrap(), None, "{script}");
        }
        // A real flag after prompt text that names another model is the pin.
        let real = launcher("claude", &["-p", "once used --model old", "--model", "new"]);
        assert_eq!(pinned_model(&real).unwrap(), Some("new".into()));
        let real_sh = launcher(
            "sh",
            &["-c", r#"exec claude -p "was --model old" --model new"#],
        );
        assert_eq!(pinned_model(&real_sh).unwrap(), Some("new".into()));
    }

    #[test]
    fn only_a_claude_command_in_a_shell_line_can_pin() {
        let echo = launcher("sh", &["-c", "echo --model m1; exec claude -p x"]);
        assert_eq!(pinned_model(&echo).unwrap(), None);
        let env = launcher("sh", &["-c", "A=1 exec claude --model m1 -p x"]);
        assert_eq!(pinned_model(&env).unwrap(), Some("m1".into()));
        let nested = launcher("sh", &["-c", "exec sh -c 'claude --model m1'"]);
        assert_eq!(pinned_model(&nested).unwrap(), None, "opaque, so no pin");
    }

    #[test]
    fn a_shell_line_that_cannot_be_read_literally_is_refused() {
        for script in [
            "exec claude --model \"m1",         // unterminated quote
            "exec claude --model $(pick) -p x", // substitution as the value
            "exec claude --model \"$M\" -p x",  // variable as the value
            "exec claude --model `pick` -p x",  // backticks
            "(exec claude --model m1)",         // subshell
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert!(pinned_model(&sh).is_err(), "{script}");
        }
        // The same constructs inside a quoted prompt are only text.
        let prompt = launcher(
            "sh",
            &[
                "-c",
                r#"exec claude -p "at $(git rev-parse HEAD) in $PWD" --model m1"#,
            ],
        );
        assert_eq!(pinned_model(&prompt).unwrap(), Some("m1".into()));
    }

    #[test]
    fn the_shipped_agents_file_parses_and_pins_nothing() {
        let file =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/proving/agents.json");
        let agents = Launchers::read_file(&file).unwrap();
        for (role, l) in agents.iter() {
            assert_eq!(pinned_model(l).unwrap(), None, "{role}");
        }
    }
}
