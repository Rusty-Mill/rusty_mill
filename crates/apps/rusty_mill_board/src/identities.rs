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
use serde::{Deserialize, Serialize};

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

/// One credential as the file holds it.
#[derive(Serialize)]
struct EntryOut<'a> {
    token: &'a str,
    actor: &'a str,
    role: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    projects: &'a Option<Vec<String>>,
}

/// Token -> identity.
#[derive(Clone)]
pub struct Credentials(Vec<(String, Identity)>);

/// Read a file that holds secrets. On Unix it must not be readable by group or others.
///
/// # Errors
/// If it cannot be read or is group/world-accessible.
pub fn read_private(path: &Path) -> Result<String, String> {
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
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// An agent name: plain characters, so it reads well in claims, logs and the UI.
fn valid_actor(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.len() <= 40
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A fresh random token: 24 bytes of OS randomness as hex.
///
/// # Errors
/// If the OS cannot supply random bytes.
pub fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 24];
    rusty_rand::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

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

    /// Wrap for sharing between the web layer and the settings.
    pub fn shared(self) -> std::sync::Arc<std::sync::RwLock<Self>> {
        std::sync::Arc::new(std::sync::RwLock::new(self))
    }

    /// Read a credentials file. On Unix it must not be readable by group or others.
    ///
    /// # Errors
    /// If it cannot be read, is group/world-accessible, or does not validate.
    pub fn load(path: &Path, roles: &Roles, projects: &Projects) -> Result<Self, String> {
        Self::parse(&read_private(path)?, roles, projects)
    }

    /// Everyone who holds a credential, without the tokens.
    pub fn list(&self) -> impl Iterator<Item = &Identity> {
        self.0.iter().map(|(_, i)| i)
    }

    /// Whether `actor` holds a credential.
    pub fn contains(&self, actor: &str) -> bool {
        self.0.iter().any(|(_, i)| i.actor == actor)
    }

    /// Add a credential for a new agent; returns its token, which is not kept anywhere else.
    ///
    /// # Errors
    /// If the name is invalid or taken (the role and projects are the caller's to validate).
    pub fn add(&mut self, identity: Identity) -> Result<String, String> {
        if !valid_actor(&identity.actor) {
            return Err(format!(
                "agent name {:?} must be 1-40 of letters, digits . _ - (starting with a letter or digit)",
                identity.actor
            ));
        }
        if self.contains(&identity.actor) {
            return Err(format!("{:?} already has a credential", identity.actor));
        }
        let token = new_token()?;
        self.0.push((token.clone(), identity));
        Ok(token)
    }

    /// Change an agent's role and/or project limit. `projects: Some(None)` lifts the limit.
    ///
    /// # Errors
    /// If there is no such agent.
    pub fn update(
        &mut self,
        actor: &str,
        role: Option<String>,
        projects: Option<Option<Vec<String>>>,
    ) -> Result<(), String> {
        let (_, i) = self
            .0
            .iter_mut()
            .find(|(_, i)| i.actor == actor)
            .ok_or_else(|| format!("no agent {actor:?}"))?;
        if let Some(role) = role {
            i.role = role;
        }
        if let Some(projects) = projects {
            i.projects = projects;
        }
        Ok(())
    }

    /// Give an agent a new token (the old one stops working); returns it.
    ///
    /// # Errors
    /// If there is no such agent.
    pub fn rotate(&mut self, actor: &str) -> Result<String, String> {
        let token = new_token()?;
        let (old, _) = self
            .0
            .iter_mut()
            .find(|(_, i)| i.actor == actor)
            .ok_or_else(|| format!("no agent {actor:?}"))?;
        old.clone_from(&token);
        Ok(token)
    }

    /// Take an agent's credential away.
    ///
    /// # Errors
    /// If there is no such agent.
    pub fn remove(&mut self, actor: &str) -> Result<(), String> {
        let before = self.0.len();
        self.0.retain(|(_, i)| i.actor != actor);
        (self.0.len() < before)
            .then_some(())
            .ok_or_else(|| format!("no agent {actor:?}"))
    }

    /// The file's JSON, tokens included: write it only with owner-only permissions.
    ///
    /// # Errors
    /// If serialisation fails.
    pub fn to_json(&self) -> Result<String, String> {
        let entries: Vec<_> = self
            .0
            .iter()
            .map(|(token, i)| EntryOut {
                token,
                actor: &i.actor,
                role: &i.role,
                projects: &i.projects,
            })
            .collect();
        serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())
    }

    /// Save to `path`, atomically and readable by the owner only.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        rusty_atomic_file::write_private(path, self.to_json()?.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// A credentials set holding one admin with a fresh token, for a board that has none yet.
    ///
    /// # Errors
    /// If the OS cannot supply random bytes.
    pub fn first_admin(role: &str) -> Result<(Self, String), String> {
        let mut creds = Self(Vec::new());
        let token = creds.add(Identity {
            actor: "admin".into(),
            role: role.into(),
            projects: None,
        })?;
        Ok((creds, token))
    }

    /// Whose token this is. Compares against every entry, so timing does not
    /// reveal which entry matched or how much of a token was right.
    /// The current identity of `actor`, if exactly one credential names them:
    /// what an OAuth grant made by that actor acts as. `None` when none does,
    /// and when several do (a file may give one actor two tokens with
    /// different roles or projects), so an actor name never picks between
    /// credentials of different authority.
    pub fn by_actor(&self, actor: &str) -> Option<&Identity> {
        let mut named = self.0.iter().map(|(_, i)| i).filter(|i| i.actor == actor);
        let only = named.next()?;
        named.next().is_none().then_some(only)
    }

    /// The identity a token belongs to.
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
