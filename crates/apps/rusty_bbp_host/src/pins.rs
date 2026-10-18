//! Pre-run checks on an agents file: every launched role pins a model, every
//! pinned model has a price, and the two arms of a pair pin the same models.
//! A pin is the `--model ID` or `--model=ID` argument of the launched program;
//! argument boundaries are kept, so prompt text that mentions the flag pins
//! nothing. A `sh -c` launcher is parsed with shell quoting and comments; every
//! `claude` command in it must pin; the only other commands allowed are `set`,
//! `cd`, `rm` and `git` (what the shipped coder launcher uses), `git` and `set`
//! in a few argument forms only, since `git` runs aliases and transports;
//! `exec` is the one wrapper; assignments use names with no special meaning;
//! `$( ... )` substitutions hold those commands only; and anything whose
//! effective execution cannot be established (a nested shell with or without a
//! path, `eval`, control flow, a computed command, a subshell or process
//! substitution, an unreadable quote) is an error.
//! The check guards against an accidentally unpinned harness, not a launcher
//! author who means harm; a human reads the launcher once before an evidence run.
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
    /// An unquoted `<` or `>` is in it, or it stands for a `|` that ends the
    /// command: the command's streams go somewhere else.
    redirect: bool,
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
    redirect: bool,
}

impl Lexer {
    fn end_word(&mut self) {
        if self.live {
            self.words.push(Word {
                text: std::mem::take(&mut self.cur),
                dynamic: self.dynamic,
                subst: std::mem::take(&mut self.subst),
                redirect: self.redirect,
            });
            self.live = false;
            self.dynamic = false;
            self.redirect = false;
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
            '<' | '>' => {
                lx.push(c);
                lx.redirect = true;
            }
            '|' => {
                // The command's output is piped away: keep that visible.
                lx.end_word();
                if !lx.words.is_empty() {
                    lx.words.push(Word {
                        text: "|".into(),
                        dynamic: false,
                        subst: Vec::new(),
                        redirect: true,
                    });
                }
                lx.end_command();
            }
            ';' | '&' | '\n' => lx.end_command(),
            c if c.is_whitespace() => lx.end_word(),
            c => lx.push(c),
        }
    }
    lx.end_command();
    Ok(lx.commands)
}

/// The commands a launcher may run besides `claude`: what the shipped coder
/// launcher uses, and nothing else. `git` and `set` only in the argument forms
/// `setup_ok` lists; `git` can run configured programs (`-c alias.x=!cmd`,
/// `--upload-pack`, `ext::` URLs), so a name on this list is never taken as proof
/// by itself. `rm` can delete or, with `cd`, reach any file the launcher can
/// write, and the check does not look at what it touches. Anything not listed is
/// refused (a nested shell, `eval`, `xargs`, control flow, `cp`, `mv`, `env`).
/// A launcher that needs another command is refused until it is added here, with
/// a reason.
const SETUP: [&str; 4] = ["set", "cd", "rm", "git"];

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
    /// Only assignments (or `exec` with nothing after it).
    Nothing,
    /// A listed setup command.
    Setup,
    /// `claude`, with its arguments starting at this word.
    Claude(usize),
}

/// The command word of a simple command, past `NAME=value` assignments and the
/// `exec` wrapper (without options of its own). An assignment must use a plain name;
/// one that stands alone with a literal value is remembered in `vars`. An error
/// for a command word that is computed, not `claude` and not a listed setup
/// command, or hidden behind `exec`'s options: what runs cannot be
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
        if name == "exec" {
            if words.get(i + 1).is_some_and(|n| n.text.starts_with('-')) {
                return Err(format!("{name} with options is not read literally"));
            }
            continue;
        }
        if w.dynamic {
            return Err(format!("the command {:?} is computed", w.text));
        }
        if name == "claude" && w.text != "claude" {
            return Err(format!(
                "{:?} is not `claude`: a harness is started by name, never by path",
                w.text
            ));
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

/// One argument of a `claude` command as the program will receive it.
struct Arg {
    text: String,
    /// A live expansion: the text is not what the program will see.
    dynamic: bool,
    /// An unquoted redirect or pipe: its output goes elsewhere.
    redirect: bool,
}

/// The flags that route a model or a setting around the pin: a fallback model,
/// a subagent definition, or a settings file or string (whose `env` can set
/// `CLAUDE_CODE_SUBAGENT_MODEL` or the cache lifetime).
const ROUTING_FLAGS: [&str; 3] = ["--fallback-model", "--agents", "--settings"];

/// The flag part of one argument: `--x=v` is (`--x`, Some(`v`)).
fn flag_of(arg: &str) -> (&str, Option<&str>) {
    match arg.split_once('=') {
        Some((f, v)) if f.starts_with("--") => (f, Some(v)),
        _ => (arg, None),
    }
}

/// The arguments before a `--`: after it everything is prompt text.
fn flags(args: &[Arg]) -> &[Arg] {
    let end = args
        .iter()
        .position(|a| a.text == "--")
        .unwrap_or(args.len());
    &args[..end]
}

/// The models named by `--model` in one argv: `--model ID` (the next argument)
/// or `--model=ID`. Nothing is split: an argument that merely contains the text
/// is a prompt, not a flag, and what follows `--` is prompt. A flag that routes
/// a model or a setting around the pin is an error.
fn argv_models(args: &[Arg]) -> Result<Vec<String>, String> {
    let args = flags(args);
    let mut found = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        let (flag, inline) = flag_of(&arg.text);
        if ROUTING_FLAGS.contains(&flag) {
            return Err(format!("{flag} routes a model or a setting around the pin"));
        }
        if flag != "--model" {
            continue;
        }
        let (v, dynamic) = match inline {
            Some(v) => (v, arg.dynamic),
            None => args
                .get(i + 1)
                .map_or(("", false), |n| (n.text.as_str(), n.dynamic)),
        };
        if dynamic {
            return Err(format!("the value of --model ({v:?}) is not a literal"));
        }
        if !v.is_empty() {
            found.push(v.to_owned());
        }
    }
    Ok(found)
}

fn is_shell(program: &str) -> bool {
    let name = program.rsplit('/').next().unwrap_or(program);
    matches!(name, "sh" | "bash" | "dash" | "zsh" | "ksh")
}

/// The `claude` commands a launcher runs, each as its arguments. A direct
/// launcher is the program `claude` itself, by that name; anything else is an
/// error, since what it runs cannot be shown to pin or to print what the cap
/// reads. A shell launcher (`sh -c SCRIPT`) is parsed with shell quoting and
/// comments, and only `claude` commands in it count.
fn claude_commands(l: &Launcher) -> Result<Vec<Vec<Arg>>, String> {
    let plain = |a: &String| Arg {
        text: a.clone(),
        dynamic: false,
        redirect: false,
    };
    if !is_shell(&l.program) {
        if l.program != "claude" {
            return Err(format!(
                "the program is {:?}: only direct claude or sh -c is supported",
                l.program
            ));
        }
        return Ok(vec![l.args.iter().map(plain).collect()]);
    }
    // Accept only shell command mode. An operand before -c selects script
    // mode, so checking a later argument would validate a command never run.
    if l.args.len() != 2 || l.args[0] != "-c" {
        return Err("shell launcher must have exactly -c SCRIPT arguments".into());
    }
    let script = &l.args[1];
    let mut out = Vec::new();
    let mut vars = Vars::new();
    for words in shell_commands(script)? {
        check_substitutions(&words, &vars)?;
        let Runs::Claude(at) = runs(&words, &mut vars)? else {
            continue;
        };
        out.push(
            words[at + 1..]
                .iter()
                .map(|w| Arg {
                    text: w.text.clone(),
                    dynamic: w.dynamic,
                    redirect: w.redirect,
                })
                .collect(),
        );
    }
    Ok(out)
}

/// The model a launcher pins. `Ok(None)` when it pins none; an error when it
/// pins two different ones (the harness would take only one), when a `claude`
/// command routes a model or a setting around the pin, or when the launcher
/// cannot be read literally. Every `claude` command in a line must pin.
pub fn pinned_model(l: &Launcher) -> Result<Option<String>, String> {
    let per_command: Vec<Vec<String>> = claude_commands(l)?
        .iter()
        .map(|c| argv_models(c))
        .collect::<Result<_, _>>()?;
    // Each harness the line starts must pin; one that does not runs on the
    // default model whatever its neighbours pin.
    if per_command.iter().any(Vec::is_empty) && per_command.iter().any(|m| !m.is_empty()) {
        return Err("a claude command in the line pins no model".into());
    }
    let found: BTreeSet<String> = per_command.into_iter().flatten().collect();
    let mut it = found.into_iter();
    match (it.next(), it.next()) {
        (None, _) => Ok(None),
        (Some(m), None) => Ok(Some(m)),
        (Some(a), Some(b)) => Err(format!("pins both {a} and {b}")),
    }
}

/// The names in a tool list: `Agent,Task`, `Agent Task`.
fn tool_names(v: &str) -> Vec<&str> {
    v.split([',', ' ']).filter(|t| !t.is_empty()).collect()
}

/// The values of a list flag: after `--flag` up to the next flag, or `--flag=v`.
fn list_values(args: &[Arg], names: &[&str]) -> Option<Vec<String>> {
    let at = args
        .iter()
        .position(|a| names.contains(&flag_of(&a.text).0))?;
    let mut vals: Vec<String> = flag_of(&args[at].text)
        .1
        .map(str::to_owned)
        .into_iter()
        .collect();
    vals.extend(
        args[at + 1..]
            .iter()
            .take_while(|a| !a.text.starts_with('-'))
            .map(|a| a.text.clone()),
    );
    Some(vals)
}

/// Whether one `claude` command cannot start a subagent: a `--tools` list
/// without `Agent` and `Task`, or a `--disallowedTools` list with both.
fn no_subagents(args: &[Arg]) -> bool {
    let named = |v: Vec<String>| -> Vec<String> {
        v.iter()
            .flat_map(|s| tool_names(s).into_iter().map(str::to_owned))
            .collect()
    };
    if let Some(v) = list_values(args, &["--tools"]) {
        let t = named(v);
        return !t.iter().any(|n| n == "Agent" || n == "Task");
    }
    list_values(args, &["--disallowedTools", "--disallowed-tools"]).is_some_and(|v| {
        let t = named(v);
        ["Agent", "Task"].iter().all(|n| t.iter().any(|x| x == n))
    })
}

/// What an evidence run needs of every harness a launcher starts, beyond its
/// pin: it prints `--output-format stream-json` to its log, unredirected and
/// unpiped (the cap reads that log), loads no settings source (`--setting-sources ""`), and
/// cannot start a subagent (which would run on a model nobody pinned).
pub fn check_harness(l: &Launcher) -> Result<(), String> {
    let commands = claude_commands(l)?;
    if commands.len() != 1 {
        return Err(format!(
            "the launcher starts {} claude commands; an evidence launcher is exactly one session, since sessions share one log and the last result accounts for the last session only",
            commands.len()
        ));
    }
    for args in &commands {
        // The shell redirects whatever follows `--` too: it is prompt text to
        // `claude`, not to the shell.
        if args.iter().any(|a| a.redirect) {
            return Err("a claude command's output is redirected or piped, so the log the cap reads would not hold it".into());
        }
        let args = flags(args);
        // A flag whose value the shell computes is not read: `--tools "$t"`
        // may hold anything.
        for (i, a) in args.iter().enumerate() {
            let (flag, inline) = flag_of(&a.text);
            let checked = [
                "--tools",
                "--disallowedTools",
                "--disallowed-tools",
                "--output-format",
                "--setting-sources",
            ];
            if !checked.contains(&flag) {
                continue;
            }
            let computed = if inline.is_some() {
                a.dynamic
            } else {
                args[i + 1..]
                    .iter()
                    .take_while(|n| !n.text.starts_with('-'))
                    .any(|n| n.dynamic)
            };
            if computed {
                return Err(format!(
                    "the value of {flag} is computed by the shell, not a literal: it cannot be read"
                ));
            }
        }
        let format = args
            .iter()
            .enumerate()
            .find_map(|(i, a)| match flag_of(&a.text) {
                ("--output-format", Some(v)) => Some(v),
                ("--output-format", None) => args.get(i + 1).map(|n| n.text.as_str()),
                _ => None,
            });
        if format != Some("stream-json") {
            return Err(format!(
                "--output-format is {format:?}; the cap and the criteria read stream-json"
            ));
        }
        // A settings file's `env` overrides the process environment (captured:
        // a project's `.claude/settings.json` set a 5m cache TTL over the
        // moderator's 1h), so no settings source loads at all.
        let sources = args
            .iter()
            .enumerate()
            .find_map(|(i, a)| match flag_of(&a.text) {
                ("--setting-sources", Some(v)) => Some(v),
                ("--setting-sources", None) => args.get(i + 1).map(|n| n.text.as_str()),
                _ => None,
            });
        if sources != Some("") {
            return Err(format!(
                "--setting-sources is {sources:?}; pass `--setting-sources \"\"` so no settings file's `env` can override the pinned cache lifetime or model"
            ));
        }
        if !no_subagents(args) {
            return Err("subagents are not restricted: pass `--disallowedTools Agent,Task` (or a `--tools` list without them), since a subagent runs on a model nobody pinned".into());
        }
    }
    Ok(())
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

/// [`pins`], every launcher passes [`check_harness`], and every pinned model
/// has a price in `criteria`.
pub fn check_run(l: &Launchers, criteria: &Criteria) -> Result<BTreeMap<String, String>, String> {
    let p = pins(l)?;
    for (role, launcher) in l.iter() {
        check_harness(launcher).map_err(|e| format!("role {role}: {e}"))?;
    }
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
        let with = |m: &str| {
            launcher(
                "claude",
                &[
                    "--model",
                    m,
                    "--output-format",
                    "stream-json",
                    "--tools",
                    "Bash",
                    "--setting-sources",
                    "",
                ],
            )
        };
        let l = Launchers::default()
            .with("planner", with("m1"))
            .with("coder", with("m2"));
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
        let setup = launcher("sh", &["-c", "set -e; rm -rf w; cd w; exec claude -p x"]);
        assert_eq!(pinned_model(&setup).unwrap(), None);
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
        // The exec wrapper and the setup commands the shipped launcher uses are fine.
        let ok = launcher(
            "sh",
            &[
                "-c",
                "set -e; cd /tmp; rm -rf w; A=1 B=2 exec claude --model m1 -p x",
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
            r#"rm -rf "$(claude -p task)"; exec claude --model m1 -p next"#,
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
            r#"exec claude --model m1 -p "at $(git rev-parse HEAD) in $PWD""#,
            "cd $(git rev-parse HEAD); exec claude --model m1 -p x",
        ] {
            let sh = launcher("sh", &["-c", script]);
            assert_eq!(pinned_model(&sh).unwrap(), Some("m1".into()), "{script}");
        }
    }

    #[test]
    fn a_harness_is_started_by_name_never_by_path() {
        // Any file named `claude` would do, and a line can write one.
        for script in [
            "exec /usr/local/bin/claude --model m1 -p x",
            "/usr/local/bin/claude -p a; exec claude --model m1 -p b",
            "exec ./claude -p x",
        ] {
            let l = launcher("sh", &["-c", script]);
            assert!(
                pinned_model(&l).unwrap_err().contains("by name"),
                "{script}"
            );
        }
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
            r#"D=$(git rev-parse HEAD); git clone -q /a "$D"; exec claude --model m1 -p x"#,
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

    // ---- second review (findings 3, 4 and the cap's input)

    fn refused(l: &Launcher) -> bool {
        pinned_model(l).is_err()
    }

    #[test]
    fn a_direct_launcher_may_not_route_models_by_flag() {
        let fb = launcher(
            "claude",
            &["--model", "m1", "--fallback-model", "m2", "-p", "x"],
        );
        assert!(refused(&fb), "--fallback-model");
        let fb = launcher("claude", &["--model", "m1", "--fallback-model=m2"]);
        assert!(refused(&fb), "--fallback-model=");
        let ag = launcher(
            "claude",
            &["--model", "m1", "--agents", "{\"a\":{\"model\":\"m9\"}}"],
        );
        assert!(refused(&ag), "--agents");
        let st = launcher(
            "claude",
            &[
                "--model",
                "m1",
                "--settings",
                "{\"env\":{\"CLAUDE_CODE_SUBAGENT_MODEL\":\"m9\"}}",
            ],
        );
        assert!(refused(&st), "--settings");
        let st = launcher("claude", &["--model", "m1", "--settings=x.json"]);
        assert!(refused(&st), "--settings=");
        let sh = launcher(
            "sh",
            &["-c", "exec claude --model m1 --settings x.json -p y"],
        );
        assert!(refused(&sh), "--settings in a script");
    }

    #[test]
    fn a_direct_launcher_may_not_set_the_environment() {
        for l in [
            launcher(
                "env",
                &["CLAUDE_CODE_SUBAGENT_MODEL=m9", "claude", "--model", "m1"],
            ),
            launcher(
                "env",
                &["CLAUDE_CODE_PROMPT_CACHE_TTL=5m", "claude", "--model", "m1"],
            ),
        ] {
            assert!(refused(&l), "{l:?}");
        }
    }

    #[test]
    fn the_program_must_be_claude() {
        for l in [
            launcher("sh", &["./wrapper.sh", "--model", "m1"]),
            launcher("python3", &["-c", "import os", "--model", "m1"]),
            launcher("/usr/bin/true", &["--model", "m1"]),
            launcher("/tmp/evil/claude", &["--model", "m1"]),
            launcher("./claude", &["--model", "m1"]),
            launcher(
                "sh",
                &["-c", "cp /tmp/evil ./claude; exec ./claude --model m1"],
            ),
            launcher("sh", &["-c", "exec /tmp/evil/claude --model m1"]),
        ] {
            assert!(refused(&l), "{l:?}");
        }
        assert_eq!(
            pinned_model(&launcher("claude", &["--model", "m1"])).unwrap(),
            Some("m1".into())
        );
    }

    #[test]
    fn what_follows_a_double_dash_is_prompt_not_flags() {
        let direct = launcher("claude", &["-p", "--", "--model", "m1"]);
        assert_eq!(pinned_model(&direct).unwrap(), None);
        let sh = launcher("sh", &["-c", "exec claude -p -- --model m1"]);
        assert_eq!(pinned_model(&sh).unwrap(), None);
        // A real pin before the `--` still counts, and flags after it are text.
        let both = launcher(
            "claude",
            &["--model", "m1", "-p", "--", "--fallback-model", "x"],
        );
        assert_eq!(pinned_model(&both).unwrap(), Some("m1".into()));
    }

    /// A launcher with everything an evidence run needs.
    fn harness_ok() -> Launcher {
        launcher(
            "claude",
            &[
                "-p",
                "x",
                "--model",
                "m1",
                "--output-format",
                "stream-json",
                "--verbose",
                "--disallowedTools",
                "Agent,Task",
                "--setting-sources",
                "",
            ],
        )
    }

    #[test]
    fn a_harness_with_stream_json_and_no_subagents_passes() {
        assert_eq!(check_harness(&harness_ok()), Ok(()));
        let eq = launcher(
            "claude",
            &[
                "--model=m1",
                "--output-format=stream-json",
                "--tools",
                "Bash,Read",
                "--setting-sources=",
            ],
        );
        assert_eq!(
            check_harness(&eq),
            Ok(()),
            "an allow-list with no Agent or Task"
        );
        let sh = launcher(
            "sh",
            &["-c", "set -e; exec claude -p x --model m1 --output-format stream-json --verbose --disallowedTools 'Agent Task' --setting-sources ''"],
        );
        assert_eq!(check_harness(&sh), Ok(()));
    }

    #[test]
    fn a_harness_must_print_stream_json_to_its_log() {
        let no_format = launcher(
            "claude",
            &["--model", "m1", "--disallowedTools", "Agent,Task"],
        );
        assert!(check_harness(&no_format).is_err(), "no --output-format");
        let text = launcher(
            "claude",
            &[
                "--model",
                "m1",
                "--output-format",
                "text",
                "--disallowedTools",
                "Agent,Task",
            ],
        );
        assert!(check_harness(&text).is_err(), "--output-format text");
        let redirect = launcher(
            "sh",
            &["-c", "exec claude --model m1 --output-format stream-json --disallowedTools Agent,Task > /tmp/x"],
        );
        assert!(check_harness(&redirect).is_err(), "redirect");
        let piped = launcher(
            "sh",
            &["-c", "claude --model m1 --output-format stream-json --disallowedTools Agent,Task | tee /tmp/x"],
        );
        assert!(check_harness(&piped).is_err(), "pipe");
        let err = launcher(
            "sh",
            &["-c", "exec claude --model m1 --output-format stream-json --disallowedTools Agent,Task 2>/dev/null"],
        );
        assert!(check_harness(&err).is_err(), "stderr redirect");
    }

    #[test]
    fn a_harness_must_not_be_able_to_start_subagents() {
        let open = launcher(
            "claude",
            &[
                "--model",
                "m1",
                "--output-format",
                "stream-json",
                "--setting-sources",
                "",
            ],
        );
        assert!(check_harness(&open).unwrap_err().contains("subagent"));
        let half = launcher(
            "claude",
            &[
                "--model",
                "m1",
                "--output-format",
                "stream-json",
                "--disallowedTools",
                "Task",
                "--setting-sources",
                "",
            ],
        );
        assert!(check_harness(&half).is_err(), "Agent is still allowed");
        let allow = launcher(
            "claude",
            &[
                "--model",
                "m1",
                "--output-format",
                "stream-json",
                "--tools",
                "Bash,Agent",
                "--setting-sources",
                "",
            ],
        );
        assert!(check_harness(&allow).is_err(), "Agent in the allow-list");
    }

    #[test]
    fn check_run_applies_the_harness_checks() {
        let good = Launchers::default().with("planner", harness_ok());
        assert!(check_run(&good, &criteria(&["m1"])).is_ok());
        let bad =
            Launchers::default().with("planner", launcher("claude", &["--model", "m1", "-p", "x"]));
        let err = check_run(&bad, &criteria(&["m1"])).unwrap_err();
        assert!(err.contains("planner"), "{err}");
    }

    // ---- round two: no setting sources at all, not even the user's

    #[test]
    fn a_harness_loads_no_settings_at_all() {
        let base = |sources: &[&str]| {
            let mut a = vec![
                "--model",
                "m1",
                "--output-format",
                "stream-json",
                "--disallowedTools",
                "Agent,Task",
            ];
            a.extend(sources);
            launcher("claude", &a)
        };
        for ok in [&["--setting-sources", ""][..], &["--setting-sources="]] {
            assert_eq!(check_harness(&base(ok)), Ok(()), "{ok:?}");
        }
        for bad in [
            &[][..],
            &["--setting-sources", "user"],
            &["--setting-sources=user"],
            &["--setting-sources", "project"],
            &["--setting-sources", "user,project"],
        ] {
            let err = check_harness(&base(bad)).unwrap_err();
            assert!(err.contains("setting"), "{bad:?}: {err}");
        }
        let sh = launcher(
            "sh",
            &["-c", "exec claude --model m1 --output-format stream-json --disallowedTools Agent,Task --setting-sources ''"],
        );
        assert_eq!(check_harness(&sh), Ok(()));
    }

    // ---- review comments on #763 (baileyrd, fa4a8a8a)

    fn full(extra: &str) -> Launcher {
        launcher(
            "sh",
            &[
                "-c",
                &format!("exec claude --model m1 --output-format stream-json --setting-sources '' {extra} -p task"),
            ],
        )
    }

    #[test]
    fn an_evidence_launcher_is_one_harness_session() {
        // Two sequential sessions write one log, and the last result accounts
        // for the last session only.
        let two = launcher(
            "sh",
            &[
                "-c",
                "claude --model m1 --output-format stream-json --setting-sources '' --disallowedTools Agent,Task -p warmup; exec claude --model m1 --output-format stream-json --setting-sources '' --disallowedTools Agent,Task -p task",
            ],
        );
        assert!(
            pinned_model(&two).is_ok(),
            "both pin: the pin check alone allows it"
        );
        let err = check_harness(&two).unwrap_err();
        assert!(err.contains("one"), "{err}");
        assert_eq!(check_harness(&full("--disallowedTools Agent,Task")), Ok(()));
    }

    #[test]
    fn a_computed_tool_list_or_format_is_not_read_as_a_literal() {
        for (what, line) in [
            ("tools", "t=Agent,Task; exec claude --model m1 --output-format stream-json --setting-sources '' --tools \"$t\" -p task"),
            ("disallowed", "t=Agent,Task; exec claude --model m1 --output-format stream-json --setting-sources '' --disallowedTools \"$t\" -p task"),
            ("format", "f=stream-json; exec claude --model m1 --output-format \"$f\" --setting-sources '' --disallowedTools Agent,Task -p task"),
            ("sources", "s=''; exec claude --model m1 --output-format stream-json --setting-sources \"$s\" --disallowedTools Agent,Task -p task"),
        ] {
            let l = launcher("sh", &["-c", line]);
            let err = check_harness(&l).unwrap_err();
            assert!(err.contains("computed"), "{what}: {err}");
        }
    }

    // ---- third review, finding 3: `-c` after a script file is the script's argument

    #[test]
    fn a_dash_c_after_a_script_file_is_an_argument_of_that_script() {
        let line = "exec claude --model m1 --output-format stream-json --setting-sources '' --disallowedTools Agent,Task -p task";
        for (what, l) in [
            ("sh wrapper", launcher("sh", &["./wrapper.sh", "-c", line])),
            (
                "bash -e wrapper",
                launcher("bash", &["-e", "./wrapper.sh", "-c", line]),
            ),
            (
                "sh -- wrapper",
                launcher("sh", &["--", "./wrapper.sh", "-c", line]),
            ),
            (
                "sh -o errexit",
                launcher("sh", &["-o", "errexit", "-c", line]),
            ),
        ] {
            let err = pinned_model(&l).unwrap_err();
            assert!(err.contains("-c"), "{what}: {err}");
            assert!(check_harness(&l).is_err(), "{what}");
        }
        // Only exactly `-c SCRIPT` is read; options before `-c` are refused too.
        let plain = launcher("sh", &["-c", line]);
        assert_eq!(pinned_model(&plain).unwrap(), Some("m1".into()));
        assert_eq!(check_harness(&plain), Ok(()));
        for l in [
            launcher("sh", &["-e", "-c", line]),
            launcher("bash", &["-eu", "-c", line]),
        ] {
            assert!(pinned_model(&l).is_err(), "{l:?}");
        }
    }

    #[test]
    fn shell_script_operand_before_c_is_refused() {
        let command = "exec claude --model m1 --output-format stream-json --setting-sources '' --disallowedTools Agent,Task -p task";
        assert!(check_harness(&launcher("sh", &["-c", command])).is_ok());
        let bad = launcher("sh", &["/tmp/wrapper.sh", "-c", command]);
        assert!(check_harness(&bad).is_err());
        assert!(pinned_model(&bad).is_err());
    }

    // ---- third review, finding 4: a redirect after `--` still redirects

    #[test]
    fn a_redirect_or_pipe_is_seen_wherever_it_is_on_the_command() {
        let flags = "--model m1 --output-format stream-json --setting-sources '' --disallowedTools Agent,Task";
        let good = |tail: &str| launcher("sh", &["-c", &format!("exec claude {flags} {tail}")]);
        // Nothing else is wrong with any of these: only the redirect or pipe.
        assert_eq!(check_harness(&good("-p task")), Ok(()));
        assert_eq!(check_harness(&good("-p -- 'do it'")), Ok(()));
        for (what, tail) in [
            ("before --", "-p task > /tmp/x"),
            ("after --", "-p -- 'do it' > /tmp/x"),
            ("stderr after --", "-p -- 'do it' 2>/dev/null"),
            ("append after --", "-p -- 'do it' >> /tmp/x"),
            // `cd` is an allowed command, so only the pipe can be at fault.
            ("pipe after --", "-p -- 'do it' | cd /tmp"),
            ("pipe before --", "-p task | cd /tmp"),
            ("input redirect after --", "-p -- 'do it' < /tmp/in"),
        ] {
            let err = check_harness(&good(tail)).unwrap_err();
            assert!(err.contains("redirected or piped"), "{what}: {err}");
        }
        // `&>` splits at the `&`: the redirect becomes a command of its own,
        // which is refused as an unsupported command.
        assert!(check_harness(&good("-p -- 'do it' &> /tmp/x")).is_err());
    }

    // ---- third review, finding 5: `--tools default` is every tool

    #[test]
    fn a_tools_list_that_says_default_allows_the_agent_tool() {
        // `claude --help`: `--tools` takes tool names, "" for none, and "default"
        // for all of them, the Agent tool included.
        let with = |tools: &[&str]| {
            let mut a = vec![
                "--model",
                "m1",
                "--output-format",
                "stream-json",
                "--setting-sources",
                "",
            ];
            a.extend(tools);
            launcher("claude", &a)
        };
        for ok in [
            &["--tools", "Bash,Read"][..],
            &["--tools", ""],
            &["--tools=Bash"],
        ] {
            assert_eq!(check_harness(&with(ok)), Ok(()), "{ok:?}");
        }
        for bad in [
            &["--tools", "default"][..],
            &["--tools=default"],
            &["--tools", "Bash,default"],
            &["--tools", "Bash default"],
            &["--tools", "DEFAULT"],
        ] {
            let err = check_harness(&with(bad)).unwrap_err();
            assert!(err.contains("subagent"), "{bad:?}: {err}");
        }
    }
}
