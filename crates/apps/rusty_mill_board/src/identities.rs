//! Per-model credentials: a bearer token per actor, each bound to one role.
//!
//! ```json
//! [{"token": "<random, 16+ chars>", "actor": "claude-code", "role": "builder"},
//!  {"token": "<random>",            "actor": "codex",       "role": "reviewer"}]
//! ```
//! A model that holds a token cannot act as any other actor or role. An optional
//! `"projects": ["rocket_league"]` limits it to those projects: it cannot see, take
//! or change cards of any other.

use std::collections::HashSet;
use std::path::Path;

use rusty_crypto_key::constant_time_eq;
use serde::Deserialize;

use crate::api::Identity;
use crate::board::Roles;
use crate::projects::Projects;

/// Shortest accepted token, in characters.
const MIN_TOKEN: usize = 16;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    token: String,
    actor: String,
    role: String,
    projects: Option<Vec<String>>,
}

/// Token -> identity.
pub struct Credentials(Vec<(String, Identity)>);

impl Credentials {
    /// Parse and validate a credentials file's JSON against the board's roles and projects.
    ///
    /// # Errors
    /// On malformed JSON, a short or duplicate token, a blank actor, an
    /// unknown role or project, an empty project list, or an empty list.
    pub fn parse(json: &str, roles: &Roles, projects: &Projects) -> Result<Self, String> {
        let entries: Vec<Entry> =
            serde_json::from_str(json).map_err(|e| format!("identities: {e}"))?;
        if entries.is_empty() {
            return Err("identities: at least one entry is required".into());
        }
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for e in entries {
            let bad = |why: &str| format!("identity {:?}: {why}", e.actor);
            if e.actor.trim().is_empty() {
                return Err("identities: actor must not be blank".into());
            }
            if e.token.chars().count() < MIN_TOKEN {
                return Err(bad(&format!(
                    "token must be at least {MIN_TOKEN} characters"
                )));
            }
            if !seen.insert(e.token.clone()) {
                return Err(bad("token is used twice"));
            }
            if !roles.0.contains_key(&e.role) {
                return Err(bad(&format!("unknown role {:?}", e.role)));
            }
            if let Some(scope) = &e.projects {
                if scope.is_empty() {
                    return Err(bad("projects is empty; omit it for all projects"));
                }
                if let Some(unknown) = scope.iter().find(|p| !projects.contains(p)) {
                    return Err(bad(&format!("unknown project {unknown:?}")));
                }
            }
            out.push((
                e.token,
                Identity {
                    actor: e.actor,
                    role: e.role,
                    projects: e.projects,
                },
            ));
        }
        Ok(Self(out))
    }

    /// Read a credentials file. On Unix it must not be readable by group or others.
    ///
    /// # Errors
    /// If it cannot be read, is group/world-accessible, or does not validate.
    pub fn load(path: &Path, roles: &Roles, projects: &Projects) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .permissions()
                .mode();
            if mode & 0o077 != 0 {
                return Err(format!(
                    "{} holds secrets: chmod 600 it (mode is {:o})",
                    path.display(),
                    mode & 0o777
                ));
            }
        }
        let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&json, roles, projects)
    }

    /// Whose token this is. Compares against every entry, so timing does not
    /// reveal which entry matched or how much of a token was right.
    pub fn identify(&self, token: &str) -> Option<&Identity> {
        let mut found = None;
        for (candidate, identity) in &self.0 {
            if constant_time_eq(candidate.as_bytes(), token.as_bytes()) {
                found = Some(identity);
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOK: &str = "0123456789abcdef0123";

    fn entry(token: &str, role: &str) -> String {
        format!(r#"[{{"token":"{token}","actor":"claude","role":"{role}"}}]"#)
    }

    #[test]
    fn identifies_by_token() {
        let c = Credentials::parse(
            &entry(TOK, "builder"),
            &Roles::default(),
            &Projects::default(),
        )
        .expect("valid");
        assert_eq!(
            c.identify(TOK),
            Some(&Identity {
                actor: "claude".into(),
                role: "builder".into(),
                projects: None,
            })
        );
        assert_eq!(c.identify("0123456789abcdef0124"), None);
        assert_eq!(c.identify(""), None);
    }

    #[test]
    fn rejects_bad_files() {
        let roles = Roles::default();
        for (json, why) in [
            ("[]".to_owned(), "at least one"),
            (entry("short", "builder"), "at least 16"),
            (entry(TOK, "ghost"), "unknown role"),
            (
                r#"[{"token":"0123456789abcdef0123","actor":" ","role":"qa"}]"#.to_owned(),
                "blank",
            ),
            (
                format!(
                    r#"[{{"token":"{TOK}","actor":"a","role":"qa"}},{{"token":"{TOK}","actor":"b","role":"qa"}}]"#
                ),
                "twice",
            ),
            (
                r#"[{"token":"0123456789abcdef0123","actor":"a","role":"qa","extra":1}]"#
                    .to_owned(),
                "unknown field",
            ),
        ] {
            let e = Credentials::parse(&json, &roles, &Projects::default())
                .err()
                .expect(&json);
            assert!(e.contains(why), "{json} -> {e}");
        }
    }

    #[test]
    fn projects_must_exist_and_are_kept() {
        let known = Projects::parse(r#"{"rl": ["crates/apps/rocket_league"]}"#).expect("projects");
        let with = |projects: &str| {
            format!(r#"[{{"token":"{TOK}","actor":"c","role":"builder","projects":{projects}}}]"#)
        };
        let ok = Credentials::parse(&with(r#"["rl","default"]"#), &Roles::default(), &known)
            .expect("known projects");
        assert_eq!(
            ok.identify(TOK).and_then(|i| i.projects.clone()),
            Some(vec!["rl".to_owned(), "default".to_owned()])
        );
        for (json, why) in [
            (with(r#"["nope"]"#), "unknown project"),
            (with("[]"), "is empty"),
        ] {
            let e = Credentials::parse(&json, &Roles::default(), &known)
                .err()
                .expect(&json);
            assert!(e.contains(why), "{json} -> {e}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn load_refuses_a_world_readable_file() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("mill_ident_{}.json", std::process::id()));
        std::fs::write(&path, entry(TOK, "builder")).expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let e = Credentials::load(&path, &Roles::default(), &Projects::default())
            .err()
            .expect("refused");
        assert!(e.contains("chmod 600"), "{e}");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        assert!(Credentials::load(&path, &Roles::default(), &Projects::default()).is_ok());
        std::fs::remove_file(&path).ok();
    }
}
