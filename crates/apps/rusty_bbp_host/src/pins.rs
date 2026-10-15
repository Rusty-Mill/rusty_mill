//! Pre-run checks on an agents file: every launched role pins a model, every
//! pinned model has a price, and the two arms of a pair pin the same models.
//! A pin is `--model ID` or `--model=ID` on a `claude` command of a launcher,
//! found inside an `sh -c` script too. The checks
//! read the file, not the run: `bbp pairs` repeats them on what the harnesses
//! actually used.

use crate::moderator::{Launcher, Launchers};
use crate::tokens::Criteria;
use std::collections::{BTreeMap, BTreeSet};

/// Split a shell script into simple commands of unquoted words. Handles
/// quotes, backslashes, `#` comments and the command separators; not
/// substitutions or here-documents, which a launcher has no need for.
fn commands(script: &str) -> Vec<Vec<String>> {
    let (mut out, mut cmd, mut word) = (Vec::new(), Vec::new(), String::new());
    let mut in_word = false;
    let mut chars = script.chars().peekable();
    let end_word = |cmd: &mut Vec<String>, word: &mut String, in_word: &mut bool| {
        if std::mem::take(in_word) {
            cmd.push(std::mem::take(word));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                word.extend(chars.by_ref().take_while(|q| *q != '\''));
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => word.extend(chars.next()),
                        _ => word.push(q),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.extend(chars.next());
            }
            '#' if !in_word => {
                chars.by_ref().find(|q| *q == '\n');
                end_word(&mut cmd, &mut word, &mut in_word);
                out.push(std::mem::take(&mut cmd));
            }
            ';' | '&' | '|' | '\n' | '(' | ')' => {
                end_word(&mut cmd, &mut word, &mut in_word);
                out.push(std::mem::take(&mut cmd));
            }
            c if c.is_whitespace() => end_word(&mut cmd, &mut word, &mut in_word),
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    end_word(&mut cmd, &mut word, &mut in_word);
    out.push(cmd);
    out.retain(|c| !c.is_empty());
    out
}

/// Environment variables that change the model or the cache lifetime behind
/// the pin's back. The moderator sets the TTLs itself; a launcher may not.
const FORBIDDEN_ENV: [&str; 8] = [
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_PROMPT_CACHE_TTL",
    "CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL",
];

fn is_assignment(w: &str) -> Option<&str> {
    let (name, _) = w.split_once('=')?;
    let ok = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then_some(name)
}

fn base(w: &str) -> &str {
    w.rsplit('/').next().unwrap_or(w)
}

fn check_env(name: &str) -> Result<(), String> {
    if FORBIDDEN_ENV.contains(&name) {
        return Err(format!(
            "the launcher sets {name}, which the pin cannot see"
        ));
    }
    Ok(())
}

/// The model pins of one command, or an error for flags that route around a pin.
fn pins_of(cmd: &[String], found: &mut BTreeSet<String>) -> Result<(), String> {
    // Leading assignments, then wrappers that run the next word.
    let mut i = 0;
    while let Some(name) = cmd.get(i).and_then(|w| is_assignment(w)) {
        check_env(name)?;
        i += 1;
    }
    while let Some(w) = cmd.get(i) {
        match base(w) {
            "exec" | "command" | "nohup" | "time" | "env" | "export" => {
                i += 1;
                while let Some(name) = cmd.get(i).and_then(|w| is_assignment(w)) {
                    check_env(name)?;
                    i += 1;
                }
            }
            _ => break,
        }
    }
    let Some(program) = cmd.get(i) else {
        return Ok(());
    };
    let args = &cmd[i + 1..];
    match base(program) {
        "sh" | "bash" | "dash" | "zsh" => {
            if let Some(at) = args.iter().position(|a| a == "-c") {
                if let Some(script) = args.get(at + 1) {
                    return scan(script, found);
                }
            }
            Ok(())
        }
        "claude" => flags(args, found),
        _ => Ok(()),
    }
}

fn flags(args: &[String], found: &mut BTreeSet<String>) -> Result<(), String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v)),
            _ => (a.as_str(), None),
        };
        match flag {
            "--fallback-model" => {
                return Err("--fallback-model can switch the model mid-run".into())
            }
            "--agents" => return Err("--agents can define a subagent with its own model".into()),
            "--model" => {
                let v = inline
                    .or_else(|| it.next().map(String::as_str))
                    .unwrap_or("");
                if v.contains(['{', '}', '$']) {
                    return Err(format!("--model {v} is a placeholder, not a model name"));
                }
                if !v.is_empty() {
                    found.insert(v.to_owned());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn scan(script: &str, found: &mut BTreeSet<String>) -> Result<(), String> {
    commands(script).iter().try_for_each(|c| pins_of(c, found))
}

/// The model a launcher pins: `--model ID` or `--model=ID` on a `claude`
/// command, direct or inside an `sh -c` script. Prompt text, comments and other
/// commands do not count. `Ok(None)` when it pins none; an error when it pins
/// two different ones (the harness would take only one), names a placeholder,
/// or uses `--fallback-model`, `--agents` or a model-routing environment
/// variable.
pub fn pinned_model(l: &Launcher) -> Result<Option<String>, String> {
    let mut cmd = vec![l.program.clone()];
    cmd.extend(l.args.iter().cloned());
    let mut found = BTreeSet::new();
    pins_of(&cmd, &mut found)?;
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

    // ---- review probes on `pinned_model`

    #[test]
    fn a_model_flag_mentioned_inside_a_prompt_string_is_not_a_pin() {
        let l = launcher(
            "sh",
            &[
                "-c",
                "exec claude -p \"never pass --model foo here\" --tools Bash",
            ],
        );
        assert_eq!(pinned_model(&l).unwrap(), None);
    }

    #[test]
    fn a_prompt_that_mentions_the_flag_plus_a_real_pin_is_the_real_pin() {
        let l = launcher(
            "sh",
            &[
                "-c",
                "exec claude -p \"use --model sonnet if asked\" --model opus",
            ],
        );
        assert_eq!(pinned_model(&l).unwrap(), Some("opus".into()));
    }

    #[test]
    fn an_echo_or_a_comment_is_not_a_pin() {
        let echo = launcher("sh", &["-c", "echo --model m1; exec claude -p x"]);
        assert_eq!(pinned_model(&echo).unwrap(), None);
        let comment = launcher("sh", &["-c", "exec claude -p x # --model m2"]);
        assert_eq!(pinned_model(&comment).unwrap(), None);
    }

    #[test]
    fn an_unresolved_placeholder_is_not_a_model_name() {
        let l = launcher("claude", &["--model", "{role}", "-p", "x"]);
        assert!(pinned_model(&l).is_err());
        let l = launcher("claude", &["--model={turn}"]);
        assert!(pinned_model(&l).is_err());
    }

    #[test]
    fn a_fallback_model_or_a_subagent_definition_is_refused() {
        let fb = launcher("claude", &["--model", "m1", "--fallback-model", "m2"]);
        assert!(pinned_model(&fb).unwrap_err().contains("fallback"));
        let agents = launcher(
            "claude",
            &["--model", "m1", "--agents", "{\"h\":{\"model\":\"haiku\"}}"],
        );
        assert!(pinned_model(&agents).unwrap_err().contains("agents"));
    }

    #[test]
    fn a_launcher_may_not_route_the_model_or_the_cache_lifetime_by_environment() {
        for script in [
            "ANTHROPIC_MODEL=haiku exec claude --model m1",
            "export CLAUDE_CODE_SUBAGENT_MODEL=haiku; exec claude --model m1",
            "env CLAUDE_CODE_PROMPT_CACHE_TTL=5m claude --model m1",
            "CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL=5m claude --model m1",
        ] {
            let l = launcher("sh", &["-c", script]);
            assert!(pinned_model(&l).unwrap_err().contains("sets"), "{script}");
        }
        // The same words in a prompt are only text.
        let l = launcher("sh", &["-c", "claude --model m1 -p 'ANTHROPIC_MODEL=x'"]);
        assert_eq!(pinned_model(&l).unwrap(), Some("m1".into()));
    }

    #[test]
    fn a_pin_on_a_command_that_is_not_claude_is_not_a_pin() {
        let l = launcher("sh", &["-c", "grep --model m1 f; exec claude -p x"]);
        assert_eq!(pinned_model(&l).unwrap(), None);
    }
}
