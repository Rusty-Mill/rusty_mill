//! Pre-run checks on an agents file: every launched role pins a model, every
//! pinned model has a price, and the two arms of a pair pin the same models.
//! A pin is `--model ID` or `--model=ID` anywhere in a launcher's program or
//! arguments (so it is found inside an `sh -c` command line too). The checks
//! read the file, not the run: `bbp pairs` repeats them on what the harnesses
//! actually used.

use crate::moderator::{Launcher, Launchers};
use crate::tokens::Criteria;
use std::collections::{BTreeMap, BTreeSet};

fn unquote(t: &str) -> &str {
    t.trim_matches(|c| matches!(c, '"' | '\'' | ';' | ','))
}

/// The model a launcher pins. `Ok(None)` when it pins none; an error when it
/// pins two different ones, since the harness would take only one.
pub fn pinned_model(l: &Launcher) -> Result<Option<String>, String> {
    let words: Vec<&str> = std::iter::once(l.program.as_str())
        .chain(l.args.iter().map(String::as_str))
        .flat_map(str::split_whitespace)
        .map(unquote)
        .collect();
    let mut found: BTreeSet<&str> = BTreeSet::new();
    for (i, w) in words.iter().enumerate() {
        if let Some(v) = w.strip_prefix("--model=") {
            found.insert(unquote(v));
        } else if *w == "--model" {
            if let Some(v) = words.get(i + 1) {
                found.insert(v);
            }
        }
    }
    found.remove("");
    let mut it = found.into_iter();
    match (it.next(), it.next()) {
        (None, _) => Ok(None),
        (Some(m), None) => Ok(Some(m.to_owned())),
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
}
