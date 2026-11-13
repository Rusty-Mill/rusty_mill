//! Registered agents: a unique, board-issued identity for each agent on a shared connector.
//!
//! A connector sign-in (OAuth) says *what* is connecting and fixes the role and projects.
//! `agent_register` gives one agent on it an id (`<connector>#<6 hex>`) and a secret key; the
//! connector's writes then carry both, so every claim, comment and move names one agent that
//! nobody else can pose as. The `#` is not a character an actor name or a step-1 label may
//! hold, so no bearer-token holder can write under a registered id. Entries idle for [`IDLE_MS`] expire; a connector holds at most
//! [`MAX_PER_CONNECTOR`] live agents. The file is private (it holds the keys).

use std::collections::BTreeMap;
use std::path::PathBuf;

use rusty_crypto_key::constant_time_eq;
use serde::{Deserialize, Serialize};

use crate::identities::new_token;

/// Live agents one connector may hold.
pub const MAX_PER_CONNECTOR: usize = 64;
/// An agent idle this long (30 days) is forgotten.
pub const IDLE_MS: i64 = 30 * 24 * 3600 * 1000;
/// How stale `seen_ms` may get before a use is written back to the file.
const SEEN_WRITE_MS: i64 = 24 * 3600 * 1000;

/// What one registered agent may do instead of its connector's role and projects. A field left
/// `None` falls back to the connector's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    /// Its own role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Its own project limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projects: Option<Vec<String>>,
}

/// One registered agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// The connector credential it registered through.
    pub connector: String,
    /// Its secret key.
    pub key: String,
    /// Registered at (Unix ms).
    pub created_ms: i64,
    /// Last used (Unix ms).
    pub seen_ms: i64,
    /// Its own role and projects, if narrowed or set by an admin.
    #[serde(flatten)]
    pub grant: Grant,
}

/// A registered agent as listed: everything but its key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentInfo {
    /// Its id.
    pub id: String,
    /// The connector it registered through.
    pub connector: String,
    /// Registered at (Unix ms).
    pub created_ms: i64,
    /// Last used (Unix ms).
    pub seen_ms: i64,
    /// Its own role and projects.
    #[serde(flatten)]
    pub grant: Grant,
}

/// The registry, optionally kept in a file.
#[derive(Debug, Default)]
pub struct Registry {
    agents: BTreeMap<String, Agent>,
    path: Option<PathBuf>,
}

impl Registry {
    /// Open the registry kept in `path` (empty if the file is missing).
    ///
    /// # Errors
    /// If the file exists but cannot be read or parsed.
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let agents = match std::fs::read_to_string(&path) {
            Ok(text) => {
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Ok(Self {
            agents,
            path: Some(path),
        })
    }

    /// Register a new agent on `connector` with `grant`: its id and key.
    ///
    /// # Errors
    /// If the connector is full of live agents, randomness fails, or the file cannot be written.
    pub fn register(
        &mut self,
        connector: &str,
        grant: Grant,
        now: i64,
    ) -> Result<(String, String), String> {
        self.expire(now);
        let live = self
            .agents
            .values()
            .filter(|a| a.connector == connector)
            .count();
        if live >= MAX_PER_CONNECTOR {
            return Err(format!(
                "{connector} already has {MAX_PER_CONNECTOR} registered agents; revoke one in Settings"
            ));
        }
        let id = format!("{connector}#{}", &new_token()?[..6]);
        let key = new_token()?;
        let agent = Agent {
            connector: connector.to_owned(),
            key: key.clone(),
            created_ms: now,
            seen_ms: now,
            grant,
        };
        if self.agents.insert(id.clone(), agent).is_some() {
            return Err("agent id collided; try again".into());
        }
        self.save()?;
        Ok((id, key))
    }

    /// The grant of `id` if it is a live agent of `connector` holding `key`; a hit counts as use.
    pub fn verify(&mut self, connector: &str, id: &str, key: &str, now: i64) -> Option<Grant> {
        let agent = self.agents.get_mut(id)?;
        let ok = agent.connector == connector
            && now - agent.seen_ms <= IDLE_MS
            && constant_time_eq(agent.key.as_bytes(), key.as_bytes());
        if !ok {
            return None;
        }
        let stale = now - agent.seen_ms > SEEN_WRITE_MS;
        agent.seen_ms = now;
        let grant = agent.grant.clone();
        if stale && let Err(e) = self.save() {
            rusty_log::warn!(error = %e, "could not record agent use");
        }
        Some(grant)
    }

    /// Replace agent `id`'s own role and projects (`Grant::default()` returns it to its
    /// connector's); whether it exists.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn set_grant(&mut self, id: &str, grant: Grant) -> Result<bool, String> {
        let Some(agent) = self.agents.get_mut(id) else {
            return Ok(false);
        };
        agent.grant = grant;
        self.save()?;
        Ok(true)
    }

    /// Remove agent `id`; whether it existed.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn revoke(&mut self, id: &str) -> Result<bool, String> {
        let found = self.agents.remove(id).is_some();
        if found {
            self.save()?;
        }
        Ok(found)
    }

    /// Every agent without its key.
    pub fn list(&self) -> Vec<AgentInfo> {
        self.agents
            .iter()
            .map(|(id, a)| AgentInfo {
                id: id.clone(),
                connector: a.connector.clone(),
                created_ms: a.created_ms,
                seen_ms: a.seen_ms,
                grant: a.grant.clone(),
            })
            .collect()
    }

    fn expire(&mut self, now: i64) {
        self.agents.retain(|_, a| now - a.seen_ms <= IDLE_MS);
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let json = serde_json::to_string_pretty(&self.agents).map_err(|e| e.to_string())?;
        rusty_atomic_file::write_private(path, json.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_agent_verifies_only_with_its_key_and_connector() {
        let mut r = Registry::default();
        let (id, key) = r
            .register("claude-cloud", Grant::default(), 0)
            .expect("register");
        assert!(id.starts_with("claude-cloud#") && id.len() == "claude-cloud#".len() + 6);
        assert!(r.verify("claude-cloud", &id, &key, 1).is_some());
        assert!(r.verify("claude-cloud", &id, "wrong", 1).is_none());
        assert!(r.verify("other", &id, &key, 1).is_none());
        assert!(
            r.verify("claude-cloud", "claude-cloud#nope00", &key, 1)
                .is_none()
        );
    }

    #[test]
    fn ids_are_unique() {
        let mut r = Registry::default();
        let a = r.register("c", Grant::default(), 0).expect("a").0;
        let b = r.register("c", Grant::default(), 0).expect("b").0;
        assert_ne!(a, b);
    }

    #[test]
    fn an_idle_agent_expires_and_use_keeps_it_alive() {
        let mut r = Registry::default();
        let (idle, k1) = r.register("c", Grant::default(), 0).expect("idle");
        let (busy, k2) = r.register("c", Grant::default(), 0).expect("busy");
        assert!(r.verify("c", &busy, &k2, IDLE_MS - 1).is_some());
        assert!(r.verify("c", &busy, &k2, 2 * IDLE_MS - 2).is_some());
        assert!(r.verify("c", &idle, &k1, 2 * IDLE_MS - 2).is_none());
    }

    #[test]
    fn a_connector_is_capped_and_expiry_frees_room() {
        let mut r = Registry::default();
        for _ in 0..MAX_PER_CONNECTOR {
            r.register("c", Grant::default(), 0).expect("fits");
        }
        assert!(r.register("c", Grant::default(), 1).is_err());
        assert!(
            r.register("d", Grant::default(), 1).is_ok(),
            "another connector is unaffected"
        );
        assert!(
            r.register("c", Grant::default(), IDLE_MS + 1).is_ok(),
            "idle agents expired"
        );
    }

    #[test]
    fn revoke_removes_one_and_the_file_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("agents-{}", new_token().expect("rand")));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("agents.json");
        let mut r = Registry::open(path.clone()).expect("open");
        let (a, ka) = r.register("c", Grant::default(), 0).expect("a");
        let (b, kb) = r.register("c", Grant::default(), 0).expect("b");
        assert!(r.revoke(&a).expect("revoke"));
        assert!(!r.revoke(&a).expect("again"));
        let mut again = Registry::open(path).expect("reopen");
        assert!(again.verify("c", &a, &ka, 1).is_none());
        assert!(again.verify("c", &b, &kb, 1).is_some());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_grant_is_kept_returned_replaced_and_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("agents-{}", new_token().expect("rand")));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("agents.json");
        let mut r = Registry::open(path.clone()).expect("open");
        let qa = Grant {
            role: Some("qa".into()),
            projects: Some(vec!["rusty_term".into()]),
        };
        let (id, key) = r.register("c", qa.clone(), 0).expect("register");
        assert_eq!(r.verify("c", &id, &key, 1), Some(qa));
        let reviewer = Grant {
            role: Some("reviewer".into()),
            projects: None,
        };
        assert!(r.set_grant(&id, reviewer.clone()).expect("set"));
        assert!(!r.set_grant("c#nope00", reviewer.clone()).expect("unknown"));
        let mut again = Registry::open(path).expect("reopen");
        assert_eq!(again.verify("c", &id, &key, 2), Some(reviewer));
        assert!(again.set_grant(&id, Grant::default()).expect("clear"));
        assert_eq!(again.verify("c", &id, &key, 3), Some(Grant::default()));
        std::fs::remove_dir_all(dir).ok();
    }
}
