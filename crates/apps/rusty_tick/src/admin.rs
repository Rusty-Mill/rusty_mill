//! `rusty_tick user ...`: edit `users.json` (ADR-0002, step 4). A thin layer
//! over [`Registry`]; a running server sees the change within a second.

use crate::backend::USERS_FILE;
use crate::users::{Registry, UserKey};
use std::path::Path;

pub const USAGE: &str = "usage: rusty_tick user add KEY [LABEL] | list | revoke KEY TOKEN_ID | \
disable KEY | enable KEY   (with --data-dir DIR)";

/// Run one command, returning what to print (a token, a listing, or nothing).
/// `add` creates `users.json` in a directory that has none, unless the
/// directory already holds a single-user store: that store would stop being
/// served, so it is refused rather than hidden.
///
/// # Errors
///
/// A message for the operator: bad usage, an unknown user or token, an
/// unreadable `users.json`.
pub fn run(data_dir: &Path, args: &[String], now_ms: i64) -> Result<String, String> {
    let path = data_dir.join(USERS_FILE);
    let key = |k: &str| UserKey::parse(k).map_err(|e| e.to_string());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut registry = match (path.exists(), args.first()) {
        (true, _) => Registry::load(&path),
        (false, Some(&"add")) if data_dir.join("tasks.mmap").exists() => {
            return Err(format!(
            "{} holds a single-user store, which a users.json would hide; use another --data-dir",
            data_dir.display()
        ))
        }
        (false, Some(&"add")) => Ok(Registry::default()),
        (false, _) => return Err(format!("no {}; `user add` creates it", path.display())),
    }
    .map_err(|e| e.to_string())?;

    let out = match args.as_slice() {
        ["add", user, label @ ..] if label.len() <= 1 => {
            let user = key(user)?;
            // An existing user just gets another token.
            let _ = registry.add_user(&user);
            let token = registry.add_token(&user, label.first().unwrap_or(&""), now_ms);
            token.map_err(|e| e.to_string())?
        }
        ["list"] => registry
            .users()
            .map(|u| {
                let head = format!(
                    "{}{}\n",
                    u.key.as_str(),
                    if u.disabled { " (disabled)" } else { "" }
                );
                let tokens: String = u
                    .tokens
                    .iter()
                    .map(|t| format!("  {} {}\n", t.id, t.label))
                    .collect();
                head + &tokens
            })
            .collect(),
        ["revoke", user, id] => {
            registry
                .revoke(&key(user)?, id)
                .map_err(|e| e.to_string())?;
            String::new()
        }
        [verb @ ("disable" | "enable"), user] => {
            let disabled = *verb == "disable";
            registry
                .set_disabled(&key(user)?, disabled)
                .map_err(|e| e.to_string())?;
            String::new()
        }
        _ => return Err(USAGE.to_string()),
    };
    if args[0] != "list" {
        registry.save(&path).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_in(dir: &Path, line: &str) -> Result<String, String> {
        let args: Vec<String> = line.split_whitespace().map(String::from).collect();
        run(dir, &args, 7)
    }

    fn registry(dir: &Path) -> Registry {
        Registry::load(&dir.join(USERS_FILE)).unwrap()
    }

    #[test]
    fn add_creates_the_file_and_prints_a_token_that_works() {
        let dir = tempfile::tempdir().unwrap();
        let token = run_in(dir.path(), "add alice phone").unwrap();
        let user = UserKey::parse("alice").unwrap();
        assert_eq!(
            registry(dir.path()).authenticate(&token),
            Some(user.clone())
        );
        assert_eq!(
            registry(dir.path()).user(&user).unwrap().tokens[0].label,
            "phone"
        );
    }

    #[test]
    fn add_to_an_existing_user_adds_a_token_and_keeps_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let first = run_in(dir.path(), "add alice").unwrap();
        let second = run_in(dir.path(), "add alice laptop").unwrap();
        let registry = registry(dir.path());
        assert!(registry.authenticate(&first).is_some());
        assert!(registry.authenticate(&second).is_some());
        assert_eq!(registry.users().count(), 1);
    }

    #[test]
    fn list_shows_users_tokens_and_disabled() {
        let dir = tempfile::tempdir().unwrap();
        run_in(dir.path(), "add alice phone").unwrap();
        run_in(dir.path(), "add bob").unwrap();
        run_in(dir.path(), "disable bob").unwrap();
        let listing = run_in(dir.path(), "list").unwrap();
        let lines: Vec<&str> = listing.lines().collect();
        assert_eq!(lines[0], "alice");
        assert!(lines[1].starts_with("  ") && lines[1].ends_with(" phone"));
        assert_eq!(lines[2], "bob (disabled)");
    }

    #[test]
    fn revoke_disable_and_enable_change_what_authenticates() {
        let dir = tempfile::tempdir().unwrap();
        let token = run_in(dir.path(), "add alice").unwrap();
        let id = registry(dir.path()).users().next().unwrap().tokens[0]
            .id
            .clone();
        run_in(dir.path(), "disable alice").unwrap();
        assert_eq!(registry(dir.path()).authenticate(&token), None);
        run_in(dir.path(), "enable alice").unwrap();
        assert!(registry(dir.path()).authenticate(&token).is_some());
        run_in(dir.path(), &format!("revoke alice {id}")).unwrap();
        assert_eq!(registry(dir.path()).authenticate(&token), None);
    }

    #[test]
    fn mistakes_are_refused_and_change_nothing() {
        let dir = tempfile::tempdir().unwrap();
        // Nothing but `add` may create the file.
        assert!(run_in(dir.path(), "list").unwrap_err().contains("user add"));
        assert!(run_in(dir.path(), "disable alice").is_err());
        run_in(dir.path(), "add alice").unwrap();
        let before = std::fs::read(dir.path().join(USERS_FILE)).unwrap();
        for line in [
            "",
            "frob",
            "add",
            "add ../x",
            "add alice a b",
            "revoke alice",
            "revoke alice nope",
            "revoke bob nope",
            "disable bob",
            "enable",
            "list extra",
        ] {
            assert!(run_in(dir.path(), line).is_err(), "{line:?}");
        }
        assert_eq!(std::fs::read(dir.path().join(USERS_FILE)).unwrap(), before);
    }

    #[test]
    fn a_single_user_store_is_not_hidden_by_a_new_users_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tasks.mmap"), b"").unwrap();
        let err = run_in(dir.path(), "add alice").unwrap_err();
        assert!(err.contains("single-user store"), "{err}");
        assert!(!dir.path().join(USERS_FILE).exists());
    }
}
