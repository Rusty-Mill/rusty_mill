//! Pre-run checks on an agents file: every launched role pins a model, every
//! pinned model has a price, and the two arms of a pair pin the same models.
//! A pin is the `--model ID` or `--model=ID` argument of the launched program;
//! argument boundaries are kept, so prompt text that mentions the flag pins
//! nothing. A `sh -c` launcher is parsed with shell quoting and comments; every
//! `claude` command in it must pin; the only other commands allowed are a short
//! list of setup commands, `git` and `set` in two and one argument forms only
//! (a command name alone is never taken as proof that nothing else starts: `git`
//! runs aliases and transports); assignments use names with no special meaning;
//! `$( ... )` substitutions hold setup commands only; and anything whose
//! effective execution cannot be established (a nested shell with or without a
//! path, `eval`, control flow, a computed command, a subshell or process
//! substitution, an unreadable quote) is an error.
//! The checks read the file, not the run: `bbp pairs` repeats them on what the
//! harnesses actually used.

use crate::moderator::{Launcher, Launchers};
use crate::tokens::Criteria;
use std::collections::{BTreeMap, BTreeSet};

/// One word of a shell command line. `dynamic`: it contains a live expansion
/// (`$`), so its text is not what the shell will pass. `subst`: the bodies of
/// its `$( ... )` command substitutions, which run before the command does.
struct Word {
    text: String,
    dynamic: bool,
    subst: Vec<String>,
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
    subst: Vec<String>,
}

impl Lexer {
    fn end_word(&mut self) {
        if self.live {
            self.words.push(Word {
                text: std::mem::take(&mut self.cur),
                dynamic: self.dynamic,
                subst: std::mem::take(&mut self.subst),
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

    /// After a `$`: a `$( ... )` substitution is kept as text and its body
    /// recorded; a nested one is not read. Any other `$` is a plain expansion.
    fn dollar(&mut self, chars: &mut std::str::Chars) -> Result<(), String> {
        self.live = true;
        self.dynamic = true;
        self.cur.push('$');
        if chars.clone().next() != Some('(') {
            return Ok(());
        }
        chars.next();
        let mut depth = 1;
        let mut body = String::new();
        for c in chars.by_ref() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                '`' => return Err("a backtick substitution is not read literally".into()),
                _ => {}
            }
            if depth == 0 {
                self.cur.push('(');
                self.cur.push_str(&body);
                self.cur.push(')');
                if body.contains("$(") {
                    return Err("a nested command substitution is not read literally".into());
                }
                self.subst.push(body);
                return Ok(());
            }
            body.push(c);
        }
        Err("unterminated command substitution".into())
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
                '`' => return Err("a backtick substitution is not read literally".into()),
                '$' => self.dollar(chars)?,
                '\\' => match chars.next() {
                    Some(e @ ('"' | '\\' | '$' | '`')) => self.cur.push(e),
                    Some('\n') => {}
                    Some(e) => {
                        self.cur.push('\\');
                        self.cur.push(e);
                    }
                    None => break,
                },
                q => self.cur.push(q),
            }
        }
        Err("unterminated double quote".into())
    }
}

/// Split a shell command line into simple commands of words, the way a shell
/// would for the constructs launchers use: single and double quotes, backslash
/// escapes, `#` comments, `$( ... )` substitutions (recorded, not run) and `;`
/// `&` `|` newline between commands. Anything that cannot be read without
/// running the shell (an unterminated quote, a subshell, a backtick or nested
/// substitution) is an error: an opaque launcher cannot be shown to pin a model.
fn shell_commands(script: &str) -> Result<Vec<Vec<Word>>, String> {
    let mut lx = Lexer::default();
    let mut chars = script.chars();
    while let Some(c) = chars.next() {
        match c {
            // A `#` that starts a word comments out the rest of the line.
            '#' if !lx.live => {
                let rest = chars.as_str();
                chars = rest[rest.find('\n').unwrap_or(rest.len())..].chars();
            }
            '\'' => lx.single_quoted(&mut chars)?,
            '"' => lx.double_quoted(&mut chars)?,
            '\\' => match chars.next() {
                Some('\n') => {}
                Some(e) => lx.push(e),
                None => return Err("trailing backslash".into()),
            },
            '$' => lx.dollar(&mut chars)?,
            '`' => return Err("a backtick substitution is not read literally".into()),
            '(' | ')' => {
                return Err("a subshell or process substitution is not read literally".into());
            }
            ';' | '&' | '|' | '\n' => lx.end_command(),
            c if c.is_whitespace() => lx.end_word(),
            c => lx.push(c),
        }
    }
    lx.end_command();
    Ok(lx.commands)
}

/// The commands a launcher may run besides `claude`: setup that starts no
/// program. `git` and `set` only in the argument forms `setup_ok` lists; `git`
/// can run configured programs (`-c alias.x=!cmd`, `--upload-pack`, `ext::`
/// URLs), so a name on this list is never taken as proof by itself. Anything
/// not listed is refused (a nested shell, `eval`, `xargs`, control flow).
const SETUP: [&str; 15] = [
    "set", "cd", "rm", "mkdir", "git", "cp", "mv", "echo", "printf", "true", "false", "sleep",
    "ls", "cat", "pwd",
];

/// Literal values assigned earlier in the line, by variable name.
type Vars = BTreeMap<String, String>;

/// A command word without its directory: `/bin/sh` is `sh`.
fn base(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// A variable name an assignment may use: a lowercase name, or one uppercase
/// letter. The names the shell, git, the dynamic loader and the like give a
/// meaning (`PATH`, `IFS`, `BASH_ENV`, `LD_PRELOAD`, `GIT_SSH_COMMAND`) are all
/// longer and upper case, so a line cannot change what runs by assigning one.
fn plain_name(n: &str) -> bool {
    let mut c = n.chars();
    match c.next() {
        Some(f) if f.is_ascii_uppercase() => n.len() == 1,
        Some(f) if f.is_ascii_lowercase() => {
            c.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        }
        _ => false,
    }
}

/// `NAME=value` as one word, when `NAME` reads as a variable name.
fn assignment(w: &Word) -> Option<(&str, &str)> {
    let (n, v) = w.text.split_once('=')?;
    (!n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_')).then_some((n, v))
}

/// The value of a command operand that can be known: a literal word, or
/// `$name` / `${name}` of a variable assigned a literal earlier in the line. It
/// must not look like an option or a transport (`ext::...`).
fn operand(w: &Word, vars: &Vars) -> Result<String, String> {
    let v = if w.dynamic {
        let name = w
            .text
            .strip_prefix('$')
            .map(|n| n.trim_start_matches('{').trim_end_matches('}'))
            .ok_or_else(|| format!("the operand {:?} is computed", w.text))?;
        vars.get(name).cloned().ok_or_else(|| {
            format!(
                "the operand {:?} is not a literal assigned in the line",
                w.text
            )
        })?
    } else {
        w.text.clone()
    };
    if v.is_empty() || v.starts_with('-') || v.contains("::") {
        return Err(format!("the operand {v:?} is not a plain path"));
    }
    Ok(v)
}

/// The argument forms a listed setup command may take. Only `git` and `set` can
/// start programs or change what runs, and they are the only ones restricted:
/// `git clone [-q] SRC DEST` and `git rev-parse HEAD`, `set -e` / `-u` forms.
fn setup_ok(name: &str, args: &[Word], vars: &Vars) -> Result<(), String> {
    // A computed word reads as `$`: a literal word cannot, since any `$` makes it computed.
    let shape: Vec<&str> = args
        .iter()
        .map(|w| if w.dynamic { "$" } else { w.text.as_str() })
        .collect();
    let operands = |ws: &[Word]| ws.iter().try_for_each(|w| operand(w, vars).map(drop));
    match (name, shape.as_slice()) {
        ("git", ["rev-parse", "HEAD"]) => Ok(()),
        ("git", ["clone", "-q" | "--quiet", _, _]) => operands(&args[2..]),
        ("git", ["clone", _, _]) => operands(&args[1..]),
        ("git", _) => {
            Err("only `git clone [-q] SRC DEST` and `git rev-parse HEAD` are read".into())
        }
        ("set", flags) => flags
            .iter()
            .all(|f| {
                f.len() > 1 && f.starts_with('-') && f[1..].chars().all(|c| matches!(c, 'e' | 'u'))
            })
            .then_some(())
            .ok_or_else(|| "only `set -e` and `set -u` are read".into()),
        _ => Ok(()),
    }
}

/// What a simple command runs.
enum Runs {
    /// Only assignments (or a wrapper with nothing after it).
    Nothing,
    /// A listed setup command.
    Setup,
    /// `claude`, with its arguments starting at this word.
    Claude(usize),
}

/// The command word of a simple command, past `NAME=value` assignments and the
/// wrappers that run the next word as is (`exec`, `command`, `nohup`, `time`,
/// `env`, without options of their own). An assignment must use a plain name;
/// one that stands alone with a literal value is remembered in `vars`. An error
/// for a command word that is computed, not `claude` and not a listed setup
/// command, or hidden behind a wrapper's options: what runs cannot be
/// established.
fn runs(words: &[Word], vars: &mut Vars) -> Result<Runs, String> {
    let mut pending: Vec<(String, String)> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        if let Some((n, v)) = assignment(w) {
            if !plain_name(n) {
                return Err(format!("assigning {n} is not read literally"));
            }
            if !w.dynamic {
                pending.push((n.to_owned(), v.to_owned()));
            }
            continue;
        }
        let name = base(&w.text);
        if matches!(name, "exec" | "env" | "command" | "nohup" | "time") {
            if words.get(i + 1).is_some_and(|n| n.text.starts_with('-')) {
                return Err(format!("{name} with options is not read literally"));
            }
            continue;
        }
        if w.dynamic {
            return Err(format!("the command {:?} is computed", w.text));
        }
        return match name {
            "claude" => Ok(Runs::Claude(i)),
            n if SETUP.contains(&n) => {
                setup_ok(n, &words[i + 1..], vars)?;
                Ok(Runs::Setup)
            }
            n => Err(format!(
                "the command {n:?} is not a supported setup command"
            )),
        };
    }
    // Only assignments: they now hold for the rest of the line.
    vars.extend(pending);
    Ok(Runs::Nothing)
}

/// Every command substitution in a command's words runs before it does, so its
/// body must itself be setup commands only: `$(claude ...)` starts a harness.
fn check_substitutions(words: &[Word], vars: &Vars) -> Result<(), String> {
    for body in words.iter().flat_map(|w| &w.subst) {
        for inner in shell_commands(body)? {
            check_substitutions(&inner, vars)?;
            if !matches!(
                runs(&inner, &mut vars.clone())?,
                Runs::Setup | Runs::Nothing
            ) {
                return Err(format!(
                    "the substitution $({body}) starts a harness or an unknown command"
                ));
            }
        }
    }
    Ok(())
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
/// shell launcher (`sh -c SCRIPT`) is parsed with shell quoting and comments,
/// and only the arguments of a `claude` command in it count, so `--model`
/// inside a quoted prompt, a comment or another command pins nothing. Every
/// `claude` command in the line must pin.
pub fn pinned_model(l: &Launcher) -> Result<Option<String>, String> {
    let models = match l.args.iter().position(|a| a == "-c") {
        Some(i) if is_shell(&l.program) => {
            let script = l.args.get(i + 1).map_or("", String::as_str);
            let mut per_command: Vec<Vec<String>> = Vec::new();
            let mut vars = Vars::new();
            for words in shell_commands(script)? {
                check_substitutions(&words, &vars)?;
                let Runs::Claude(at) = runs(&words, &mut vars)? else {
                    continue;
                };
                per_command.push(argv_models(
                    words[at + 1..].iter().map(|w| (w.text.as_str(), w.dynamic)),
                )?);
            }
            // Each harness the line starts must pin; one that does not runs on
            // the default model whatever its neighbours pin.
            if per_command.iter().any(Vec::is_empty) && per_command.iter().any(|m| !m.is_empty()) {
                return Err("a claude command in the line pins no model".into());
            }
            per_command.concat()
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
        assert!(pinned_model(&nested).is_err(), "a nested shell is opaque");
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

    #[test]
    fn every_claude_command_in_a_line_must_pin_not_just_one_of_them() {
        let warm = launcher(
            "sh",
            &["-c", "claude --model m1 -p warmup; exec claude -p task"],
        );
        let err = pinned_model(&warm).unwrap_err();
        assert!(err.contains("pins no model"), "{err}");
        for script in [
            "exec claude -p task && claude --model m1 -p again",
            "claude -p a | claude --model m1 -p b",
            "claude --model m1 -p a\nclaude -p b",
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert!(pinned_model(&sh).is_err(), "{script}");
        }
        // Every one pinned, to the same model: one pin. To two: an error.
        let same = launcher(
            "sh",
            &["-c", "claude --model m1 -p a; exec claude --model m1 -p b"],
        );
        assert_eq!(pinned_model(&same).unwrap(), Some("m1".into()));
        let two = launcher(
            "sh",
            &["-c", "claude --model m1 -p a; exec claude --model m2 -p b"],
        );
        assert!(pinned_model(&two).unwrap_err().contains("both"));
    }

    #[test]
    fn a_shell_comment_is_not_executed() {
        // The shell drops everything after an unquoted `#` that starts a word.
        let commented = launcher(
            "sh",
            &["-c", "exec claude -p task # note; claude --model m1"],
        );
        assert_eq!(pinned_model(&commented).unwrap(), None);
        let all = launcher("sh", &["-c", "# claude --model m1\nexec claude -p task"]);
        assert_eq!(pinned_model(&all).unwrap(), None);
        // A `#` inside a word or a quote is text, and a later line runs.
        let word = launcher("sh", &["-c", "exec claude -p a#b --model m1"]);
        assert_eq!(pinned_model(&word).unwrap(), Some("m1".into()));
        let quoted = launcher("sh", &["-c", "exec claude -p '# x' --model m1 # tail"]);
        assert_eq!(pinned_model(&quoted).unwrap(), Some("m1".into()));
        let next = launcher("sh", &["-c", "# off\nexec claude --model m1 -p x"]);
        assert_eq!(pinned_model(&next).unwrap(), Some("m1".into()));
    }

    #[test]
    fn syntax_whose_effective_execution_cannot_be_established_is_refused() {
        for script in [
            "claude --model m1 -p x; sh -c 'claude -p y'", // a nested shell
            "claude --model m1 -p x; eval \"$CMD\"",       // eval
            "if true; then claude --model m1 -p x; fi",    // control flow
            "while true; do claude --model m1 -p x; done",
            "$(echo claude) --model m1 -p x", // a computed command
            "env -i claude --model m1 -p x",  // a wrapper with options
            "xargs claude --model m1",
            "source ./launch.sh",
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert!(pinned_model(&sh).is_err(), "{script}");
        }
        // The supported wrappers and the usual setup commands are fine.
        let ok = launcher(
            "sh",
            &[
                "-c",
                "set -e; cd /tmp; rm -rf w; A=1 env B=2 exec claude --model m1 -p x",
            ],
        );
        assert_eq!(pinned_model(&ok).unwrap(), Some("m1".into()));
    }

    #[test]
    fn a_nested_shell_is_refused_with_or_without_a_path() {
        for script in [
            r#"claude --model m1 -p warmup; /bin/sh -c "claude -p task""#,
            "claude --model m1 -p warmup; /usr/bin/env bash -c 'claude -p task'",
            "claude --model m1 -p warmup; ./launch.sh",
            "claude --model m1 -p warmup; /opt/x/xargs claude",
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert!(pinned_model(&sh).is_err(), "{script}");
        }
    }

    #[test]
    fn a_command_substitution_is_checked_wherever_it_hides() {
        // In an assignment, an argument of a setup command, a quoted prompt of
        // a pinned harness: the substitution runs before the command does.
        for script in [
            r#"X="$(claude -p task)"; exec claude --model m1 -p next"#,
            "X=$(claude -p task); exec claude --model m1 -p next",
            r#"echo "$(claude -p task)"; exec claude --model m1 -p next"#,
            r#"exec claude --model m1 -p "$(claude -p task)""#,
            "X=$(echo $(claude -p task)); exec claude --model m1 -p next", // nested
            "X=$(sh -c 'claude -p task'); exec claude --model m1 -p next",
            "X=`claude -p task`; exec claude --model m1 -p next",
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert!(pinned_model(&sh).is_err(), "{script}");
        }
        // A substitution of setup commands only is fine, in any of those places.
        for script in [
            r#"X="$(git rev-parse HEAD)"; exec claude --model m1 -p "at $X""#,
            r#"exec claude --model m1 -p "at $(git rev-parse HEAD) in $(pwd)""#,
            "cd $(pwd); exec claude --model m1 -p x",
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert_eq!(pinned_model(&sh).unwrap(), Some("m1".into()), "{script}");
        }
    }

    #[test]
    fn a_claude_given_by_path_is_still_a_claude_command() {
        let pinned = launcher("sh", &["-c", "exec /usr/local/bin/claude --model m1 -p x"]);
        assert_eq!(pinned_model(&pinned).unwrap(), Some("m1".into()));
        let partial = launcher(
            "sh",
            &[
                "-c",
                "/usr/local/bin/claude -p a; exec claude --model m1 -p b",
            ],
        );
        assert!(pinned_model(&partial).is_err());
        let bare = launcher("/bin/sh", &["-c", "exec ./claude -p x"]);
        assert_eq!(pinned_model(&bare).unwrap(), None);
    }

    #[test]
    fn git_can_run_configured_programs_so_only_two_forms_of_it_are_read() {
        for script in [
            // The reported case: a git alias that runs an unpinned harness.
            r#"git -c alias.bbp="!claude -p task" bbp; exec claude --model m1 -p next"#,
            r#"X="$(git -c alias.bbp='!claude -p task' bbp)"; exec claude --model m1 -p next"#,
            // Other ways git starts a program.
            "git clone --upload-pack='claude -p x' a b; exec claude --model m1 -p x",
            "git clone -q ext::sh a b; exec claude --model m1 -p x",
            "git clone -q a ext::sh; exec claude --model m1 -p x",
            "git clone -q -- a b; exec claude --model m1 -p x",
            "git log --ext-diff; exec claude --model m1 -p x",
            "git status; exec claude --model m1 -p x",
            "git rev-parse --git-dir; exec claude --model m1 -p x",
            // An operand from the environment, or assigned something unsafe.
            r#"git clone -q /a "$DEST"; exec claude --model m1 -p x"#,
            r#"D=--upload-pack=x; git clone -q /a "$D"; exec claude --model m1 -p x"#,
            r#"D=$(pwd); git clone -q /a "$D"; exec claude --model m1 -p x"#,
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert!(pinned_model(&sh).is_err(), "{script}");
        }
        for script in [
            "git clone /tmp/src /tmp/dest; exec claude --model m1 -p x",
            "git clone -q /tmp/src /tmp/dest; exec claude --model m1 -p x",
            r#"D=/tmp/dest; rm -rf "$D"; git clone --quiet /tmp/src "$D"; cd "$D"; exec claude --model m1 -p x"#,
            "exec claude --model m1 -p \"at $(git rev-parse HEAD)\"",
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert_eq!(pinned_model(&sh).unwrap(), Some("m1".into()), "{script}");
        }
    }

    #[test]
    fn a_line_cannot_change_what_runs_by_assigning_a_variable_that_means_something() {
        for script in [
            "PATH=/tmp/evil exec claude --model m1 -p x",
            "PATH=/tmp/evil; exec claude --model m1 -p x",
            "export PATH=/tmp/evil; exec claude --model m1 -p x",
            "BASH_ENV=/tmp/x exec claude --model m1 -p x",
            "GIT_SSH_COMMAND=x; exec claude --model m1 -p x",
            "LD_PRELOAD=/tmp/x exec claude --model m1 -p x",
            "IFS=x; exec claude --model m1 -p x",
            "_=x; exec claude --model m1 -p x",
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert!(pinned_model(&sh).is_err(), "{script}");
        }
        for script in [
            "w=/tmp/x; A=1 exec claude --model m1 -p x",
            "tmp_dir=/tmp/x; exec claude --model m1 -p x",
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert_eq!(pinned_model(&sh).unwrap(), Some("m1".into()), "{script}");
        }
    }

    #[test]
    fn a_process_substitution_or_subshell_hides_nothing() {
        for script in [
            "cat <(claude -p task); exec claude --model m1 -p x",
            "echo hi > >(claude -p task); exec claude --model m1 -p x",
            "(claude -p task); exec claude --model m1 -p x",
            "exec claude --model m1 -p x; f() { claude -p task; }",
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert!(pinned_model(&sh).is_err(), "{script}");
        }
    }

    #[test]
    fn set_is_read_only_in_its_plain_forms() {
        for ok in ["set -e", "set -eu", "set -u"] {
            let sh = launcher("sh", &["-c", &format!("{ok}; exec claude --model m1 -p x")]);
            assert_eq!(pinned_model(&sh).unwrap(), Some("m1".into()), "{ok}");
        }
        for bad in [
            "set -x",
            "set -o errexit",
            "set -- a",
            "set +e",
            "set $FLAGS",
        ] {
            let sh = launcher(
                "sh",
                &["-c", &format!("{bad}; exec claude --model m1 -p x")],
            );
            assert!(pinned_model(&sh).is_err(), "{bad}");
        }
    }
}
