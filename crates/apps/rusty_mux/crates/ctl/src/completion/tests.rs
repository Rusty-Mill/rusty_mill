use super::*;
use crate::cli::COMMANDS;

#[test]
fn every_command_has_words_and_every_word_is_in_its_help() {
    let names: Vec<&str> = COMMAND_WORDS.iter().map(|(n, _)| *n).collect();
    let helped: Vec<&str> = COMMANDS.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, helped);
    for ((name, words), (_, help)) in COMMAND_WORDS.iter().zip(COMMANDS) {
        for word in words.iter().filter(|w| **w != "--") {
            assert!(help.contains(word), "{name}: `{word}` not in its help");
        }
    }
}

#[test]
fn bare_flags_are_flags_some_command_takes() {
    for flag in BARE_FLAGS {
        let used = GLOBAL.contains(flag) || COMMAND_WORDS.iter().any(|(_, w)| w.contains(flag));
        assert!(used, "{flag}");
    }
}

#[test]
fn no_flag_is_both_a_switch_and_valued() {
    let spec = spec();
    for method in schema::CATALOG {
        for p in method.params {
            let flag = format!("--{}", p.name);
            let valued = spec.value_flags.contains(&flag);
            assert_eq!(valued, p.ty != Ty::Flag, "{}: {flag}", method.name);
        }
    }
    for flag in BARE_FLAGS {
        assert!(!spec.value_flags.iter().any(|v| v == flag), "{flag}");
    }
}

#[test]
fn spec_has_every_method_with_its_flags() {
    let spec = spec();
    for method in schema::CATALOG {
        assert!(spec.top.iter().any(|w| w == method.name), "{}", method.name);
        let (_, words) = spec
            .commands
            .iter()
            .find(|(n, _)| *n == method.name)
            .expect("listed");
        assert_eq!(words.len(), method.params.len(), "{}", method.name);
    }
    let (_, help) = spec
        .commands
        .iter()
        .find(|(n, _)| *n == "help")
        .expect("help");
    assert!(help.iter().any(|w| w == "terminal.format"));
}

#[test]
fn no_word_needs_quoting() {
    let spec = spec();
    let words = spec
        .top
        .iter()
        .chain(spec.commands.iter().flat_map(|(_, w)| w));
    for word in words {
        assert!(
            word.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._".contains(c)),
            "{word}"
        );
    }
}

#[test]
fn every_shell_names_every_command() {
    for shell in [
        Shell::Bash,
        Shell::Zsh,
        Shell::Fish,
        Shell::Nu,
        Shell::PowerShell,
    ] {
        let text = script(shell);
        for method in schema::CATALOG {
            assert!(text.contains(method.name), "{shell:?}: {}", method.name);
        }
        assert!(text.contains("rusty-mux-ctl"), "{shell:?}");
    }
}

#[test]
fn shells_parse_by_name() {
    assert_eq!("bash".parse(), Ok(Shell::Bash));
    assert_eq!("pwsh".parse(), Ok(Shell::PowerShell));
    assert!("csh".parse::<Shell>().is_err());
}
