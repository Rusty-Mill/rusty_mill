//! The account map of login mode (C26, Rex delta D9): which account each
//! principal may use. Pure: parsing and lookup, no I/O.
//!
//! One rule per line, `PRINCIPAL ACCOUNT`, separated by spaces or tabs;
//! `#` starts a comment, and blank lines are skipped. `PRINCIPAL` is
//! `uid:N`, `user:NAME` or `*` (anyone the others do not name); `ACCOUNT`
//! is an account name, or `=` for the principal's own account:
//!
//! ```text
//! uid:1000   alice
//! user:bob   =
//! *          guest
//! ```
//!
//! A file with no rules lets nobody in, and the shipped file has none. A
//! `uid:` rule beats a `user:` rule, which beats `*`; naming a principal
//! twice refuses the whole file. Root is reached only through a rule that
//! names its principal, never through `*` ([`Route::exact`]).

use std::collections::HashMap;
use std::fmt;

/// The longest account name taken, as `useradd` allows by default.
pub(crate) const MAX_ACCOUNT: usize = 32;

/// The parsed map.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AccountMap {
    uids: HashMap<u32, Target>,
    users: HashMap<String, Target>,
    anyone: Option<Target>,
}

/// Where a rule sends its principal.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Named(String),
    /// `=`: the principal's own account.
    Own,
}

/// The account a principal maps to, and whether a rule named the
/// principal (`exact`) or it came through `*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Route {
    pub account: String,
    pub exact: bool,
}

/// Why a file was refused, with its 1-based line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapError {
    pub line: usize,
    pub reason: String,
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for MapError {}

impl AccountMap {
    /// Parses a whole file; any bad line refuses all of it.
    pub(crate) fn parse(text: &str) -> Result<Self, MapError> {
        let mut map = Self::default();
        for (n, raw) in text.lines().enumerate() {
            let line = n.saturating_add(1);
            let fail = |reason: String| MapError { line, reason };
            let content = raw.split('#').next().unwrap_or_default();
            let fields: Vec<&str> = content.split_whitespace().collect();
            let [principal, account] = fields.as_slice() else {
                if fields.is_empty() {
                    continue;
                }
                return Err(fail("expected `PRINCIPAL ACCOUNT`".into()));
            };
            let target = target(account).map_err(fail)?;
            map.add(principal, target).map_err(fail)?;
        }
        Ok(map)
    }

    fn add(&mut self, principal: &str, target: Target) -> Result<(), String> {
        let fresh = if principal == "*" {
            self.anyone.replace(target).is_none()
        } else if let Some(uid) = principal.strip_prefix("uid:") {
            let uid = uid.parse().map_err(|_| format!("{principal}: not a uid"))?;
            self.uids.insert(uid, target).is_none()
        } else if let Some(user) = principal.strip_prefix("user:") {
            if !is_account_name(user) {
                return Err(format!("{principal}: not a user name"));
            }
            self.users.insert(user.to_owned(), target).is_none()
        } else {
            return Err(format!("{principal}: not `uid:N`, `user:NAME` or `*`"));
        };
        if fresh {
            Ok(())
        } else {
            Err(format!("{principal} has a rule already"))
        }
    }

    /// The account for the principal `uid`, whose account name is `user`
    /// (empty when it has none), or `None` if no rule lets it in.
    pub(crate) fn route(&self, uid: u32, user: &str) -> Option<Route> {
        let by_name = || (!user.is_empty()).then(|| self.users.get(user)).flatten();
        let (target, exact) = match self.uids.get(&uid).or_else(by_name) {
            Some(target) => (target, true),
            None => (self.anyone.as_ref()?, false),
        };
        let account = match target {
            Target::Named(account) => account.clone(),
            Target::Own if user.is_empty() => return None,
            Target::Own => user.to_owned(),
        };
        Some(Route { account, exact })
    }
}

fn target(account: &str) -> Result<Target, String> {
    if account == "=" {
        return Ok(Target::Own);
    }
    if !is_account_name(account) {
        return Err(format!("{account}: not an account name"));
    }
    Ok(Target::Named(account.to_owned()))
}

/// A portable account name: a lowercase letter or `_`, then lowercase
/// letters, digits, `_`, `.` or `-`, at most [`MAX_ACCOUNT`] bytes. Being
/// a path component of the account's runtime directory, it never holds
/// `/` or starts with `.`.
pub(crate) fn is_account_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let first_ok = bytes
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b == b'_');
    first_ok
        && name.len() <= MAX_ACCOUNT
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
}

/// Whether `route` may reach the account whose uid is `account_uid`:
/// root (uid 0) only through a rule naming the principal.
pub(crate) fn admits(route: &Route, account_uid: u32) -> bool {
    account_uid != 0 || route.exact
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(map: &AccountMap, uid: u32, user: &str) -> Option<(String, bool)> {
        map.route(uid, user).map(|r| (r.account, r.exact))
    }

    #[test]
    fn an_empty_file_lets_nobody_in() {
        for text in ["", "\n\n", "# nobody yet\n   # still nobody\n"] {
            let map = AccountMap::parse(text).expect(text);
            assert_eq!(map, AccountMap::default());
            assert_eq!(route(&map, 1000, "alice"), None);
            assert_eq!(route(&map, 0, "root"), None);
        }
    }

    #[test]
    fn a_uid_rule_beats_a_user_rule_which_beats_anyone() {
        let map = AccountMap::parse(
            "uid:1000 deploy   # alice deploys\nuser:alice alice\nuser:bob =\n*\tguest\n",
        )
        .unwrap();
        assert_eq!(route(&map, 1000, "alice"), Some(("deploy".into(), true)));
        assert_eq!(route(&map, 1001, "alice"), Some(("alice".into(), true)));
        assert_eq!(route(&map, 1002, "bob"), Some(("bob".into(), true)));
        assert_eq!(route(&map, 1003, "carol"), Some(("guest".into(), false)));
        assert_eq!(route(&map, 1004, ""), Some(("guest".into(), false)));
    }

    #[test]
    fn own_account_needs_a_name() {
        let map = AccountMap::parse("* =").unwrap();
        assert_eq!(route(&map, 1000, "alice"), Some(("alice".into(), false)));
        assert_eq!(route(&map, 1001, ""), None);
    }

    #[test]
    fn root_comes_only_through_a_rule_that_names_it() {
        let anyone = AccountMap::parse("* =\n").unwrap();
        let via_star = anyone.route(0, "root").unwrap();
        assert!(!admits(&via_star, 0), "* = let root in");
        let to_root = AccountMap::parse("* root\n").unwrap();
        assert!(!admits(&to_root.route(1000, "alice").unwrap(), 0));
        let named = AccountMap::parse("uid:0 root\nuser:alice root\n").unwrap();
        assert!(admits(&named.route(0, "root").unwrap(), 0));
        assert!(admits(&named.route(1000, "alice").unwrap(), 0));
        // Anyone else may still reach an ordinary account through `*`.
        assert!(admits(&via_star, 1000));
    }

    #[test]
    fn a_bad_line_refuses_the_whole_file() {
        for (text, line) in [
            ("alice", 1),
            ("uid:1 a b", 1),
            ("uid:x alice", 1),
            ("uid:-1 alice", 1),
            ("group:wheel alice", 1),
            ("user:Alice alice", 1),
            ("user: alice", 1),
            ("uid:1 Alice", 1),
            ("uid:1 ../etc", 1),
            ("uid:1 .hidden", 1),
            ("uid:1 a/b", 1),
            ("uid:1 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 1),
            ("uid:1 alice\n\nuid:1 bob", 3),
            ("user:a a\nuser:a b", 2),
            ("* a\n* b", 2),
        ] {
            let err = AccountMap::parse(text).expect_err(text);
            assert_eq!(err.line, line, "{text:?}: {err}");
        }
    }

    #[test]
    fn account_names_are_portable() {
        for good in ["a", "_svc", "deploy-1", "a.b", &"a".repeat(MAX_ACCOUNT)] {
            assert!(is_account_name(good), "{good}");
        }
        for bad in [
            "",
            "-a",
            "1a",
            ".a",
            "A",
            "a/b",
            "a b",
            "é",
            &"a".repeat(MAX_ACCOUNT + 1),
        ] {
            assert!(!is_account_name(bad), "{bad}");
        }
    }
}
