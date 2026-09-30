//! `rusty_tick user ...`: edit `users.json` (ADR-0002, step 4). A thin layer
//! over [`Registry`]; a running server sees the change within a second.

use crate::backend::{LOCK_FILE, USERS_DIR, USERS_FILE};
use crate::users::{Registry, UserKey};
use rusty_multimodal_db_engine::dir_lock::{DirLock, DirLockError};
use std::path::Path;

pub const USAGE: &str = "usage: rusty_tick user add KEY [LABEL] | list | revoke KEY TOKEN_ID | \
disable KEY | enable KEY | adopt KEY   (with --data-dir DIR)";

/// Held while a command edits `users.json`, so two commands cannot both read,
/// change and rewrite it and lose one change.
const EDIT_LOCK: &str = "users.lock";

/// Run one command, returning what to print (a token, a listing, or nothing).
/// `add` creates `users.json` in a directory that has none, unless the
/// directory already holds a single-user store: that store would stop being
/// served, so it is refused rather than hidden.
///
/// # Errors
///
/// A message for the operator: bad usage, an unknown user or token, an
/// unreadable `users.json`, another `user` command running.
pub fn run(data_dir: &Path, args: &[String], now_ms: i64) -> Result<String, String> {
    let path = data_dir.join(USERS_FILE);
    let key = |k: &str| UserKey::parse(k).map_err(|e| e.to_string());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let _editing = match args.first() {
        Some(&("add" | "revoke" | "disable" | "enable" | "adopt")) => Some(lock(
            data_dir,
            EDIT_LOCK,
            "another `user` command is running",
        )?),
        _ => None,
    };
    let mut registry = match (path.exists(), args.first()) {
        (true, _) => Registry::load(&path),
        (false, Some(&"add")) if data_dir.join("tasks.mmap").exists() => {
            return Err(format!(
            "{} holds a single-user store, which a users.json would hide; use another --data-dir",
            data_dir.display()
        ))
        }
        (false, Some(&("add" | "adopt"))) => Ok(Registry::default()),
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
        ["adopt", user] => {
            let user = key(user)?;
            // Holding the store's lock proves no server is using it.
            let _store = lock(
                data_dir,
                LOCK_FILE,
                "the server is running here; stop it first",
            )?;
            let target = data_dir.join(USERS_DIR).join(user.as_str());
            if !data_dir.join("tasks.mmap").exists() {
                return Err(format!("no single-user store in {}", data_dir.display()));
            }
            if target.exists() {
                return Err(format!("{} already exists", target.display()));
            }
            let _ = registry.add_user(&user);
            let token = registry
                .add_token(&user, "adopted", now_ms)
                .map_err(|e| e.to_string())?;
            move_store(data_dir, &target)?;
            token
        }
        _ => return Err(USAGE.to_string()),
    };
    if args[0] != "list" {
        registry.save(&path).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Take `name`'s lock in `dir`, or say `busy` if someone holds it.
fn lock(dir: &Path, name: &str, busy: &str) -> Result<DirLock, String> {
    DirLock::acquire(dir, name).map_err(|e| match e {
        DirLockError::Held(_) => busy.to_string(),
        e => e.to_string(),
    })
}

/// Rename everything in `from` that belongs to the store (all but the users
/// file, the users directory and the lock files) into `to`.
fn move_store(from: &Path, to: &Path) -> Result<(), String> {
    let io = |what: &Path, e: std::io::Error| format!("{}: {e}", what.display());
    std::fs::create_dir_all(to).map_err(|e| io(to, e))?;
    let keep = [
        USERS_FILE,
        "users.json.tmp",
        USERS_DIR,
        LOCK_FILE,
        EDIT_LOCK,
    ];
    for entry in std::fs::read_dir(from).map_err(|e| io(from, e))? {
        let entry = entry.map_err(|e| io(from, e))?;
        if keep.iter().any(|k| entry.file_name() == *k) {
            continue;
        }
        let dest = to.join(entry.file_name());
        std::fs::rename(entry.path(), &dest).map_err(|e| io(&dest, e))?;
    }
    Ok(())
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
    fn a_second_command_at_the_same_time_is_refused_not_merged() {
        let dir = tempfile::tempdir().unwrap();
        run_in(dir.path(), "add alice").unwrap();
        let _running = DirLock::acquire(dir.path(), EDIT_LOCK).unwrap();
        for line in ["add bob", "disable alice", "adopt bob"] {
            let err = run_in(dir.path(), line).unwrap_err();
            assert!(err.contains("another `user` command"), "{line}: {err}");
        }
        // Reading needs no lock.
        assert!(run_in(dir.path(), "list").unwrap().starts_with("alice"));
    }

    /// A directory with a fake single-user store: two files and a lock.
    fn single_user_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for name in ["tasks.mmap", "tasks.mmap.log", "lists.mmap", LOCK_FILE] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        dir
    }

    #[test]
    fn adopt_moves_the_store_into_the_user_and_mints_a_token() {
        let dir = single_user_dir();
        let token = run_in(dir.path(), "adopt alice").unwrap();
        let home = dir.path().join(USERS_DIR).join("alice");
        for name in ["tasks.mmap", "tasks.mmap.log", "lists.mmap"] {
            assert_eq!(std::fs::read_to_string(home.join(name)).unwrap(), name);
            assert!(!dir.path().join(name).exists(), "{name} was left behind");
        }
        assert!(registry(dir.path()).authenticate(&token).is_some());
        // The lock file stays: it belongs to the directory, not the store.
        assert!(dir.path().join(LOCK_FILE).exists());
    }

    #[test]
    fn adopt_refuses_what_it_cannot_do_safely_and_moves_nothing() {
        // A server holds the directory.
        let dir = single_user_dir();
        let server = DirLock::acquire(dir.path(), LOCK_FILE).unwrap();
        assert!(run_in(dir.path(), "adopt alice")
            .unwrap_err()
            .contains("stop it"));
        drop(server);
        // The user already has a directory.
        std::fs::create_dir_all(dir.path().join(USERS_DIR).join("alice")).unwrap();
        assert!(run_in(dir.path(), "adopt alice")
            .unwrap_err()
            .contains("already exists"));
        assert!(dir.path().join("tasks.mmap").exists());
        // Nothing to adopt.
        let empty = tempfile::tempdir().unwrap();
        assert!(run_in(empty.path(), "adopt alice")
            .unwrap_err()
            .contains("no single-user store"));
        assert!(!empty.path().join(USERS_FILE).exists());
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
