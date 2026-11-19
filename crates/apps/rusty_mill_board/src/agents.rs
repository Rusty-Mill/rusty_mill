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
/// Longest nickname, in characters.
pub const MAX_NICKNAME: usize = 40;
/// How stale the *saved* `seen_ms` may get before a use is written back to the file. Stale
/// cleanup allows for this lag after a restart; it is below the 2-hour minimum stale age.
const SEEN_WRITE_MS: i64 = 3600 * 1000;

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
    /// A display name an admin gave it (never an identity: claims and history keep the id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    /// `seen_ms` as last written to the file (not itself stored).
    #[serde(skip)]
    saved_ms: i64,
    /// When the registry was loaded from the file, if the agent has not been used since (not
    /// stored). The file lags real use by at most [`SEEN_WRITE_MS`] while writes succeed.
    #[serde(skip)]
    loaded_ms: Option<i64>,
}

impl Agent {
    /// The latest its last use can have been. Exact for an agent used since the load; for one
    /// loaded and not used since, its saved last use plus the write-back lag, but never after
    /// the load. A use whose write failed (logged) can be missed: such an agent may be revoked
    /// and must register again, which beats stale cleanup finding nothing after every restart.
    fn last_use_upper_bound(&self) -> i64 {
        let known = self.seen_ms.max(self.created_ms);
        self.loaded_ms
            .map_or(known, |at| (known + SEEN_WRITE_MS).min(at).max(known))
    }
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
    /// Its nickname.
    pub nickname: Option<String>,
}

/// The registry, optionally kept in a file.
#[derive(Debug, Default)]
pub struct Registry {
    agents: BTreeMap<String, Agent>,
    path: Option<PathBuf>,
    /// A revocation reached memory but not the file: the next [`revoke_stale`](Self::revoke_stale)
    /// writes it even with nothing new to revoke.
    unsaved_revoke: bool,
}

impl Registry {
    /// Open the registry kept in `path` (empty if the file is missing); `now` is the load time.
    ///
    /// # Errors
    /// If the file exists but cannot be read or parsed.
    pub fn open(path: PathBuf, now: i64) -> Result<Self, String> {
        let agents = match std::fs::read_to_string(&path) {
            Ok(text) => {
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut agents: BTreeMap<String, Agent> = agents;
        for a in agents.values_mut() {
            a.saved_ms = a.seen_ms;
            a.loaded_ms = Some(now);
        }
        Ok(Self {
            agents,
            path: Some(path),
            unsaved_revoke: false,
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
            nickname: None,
            saved_ms: now,
            loaded_ms: None,
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
        let stale = now - agent.saved_ms > SEEN_WRITE_MS;
        agent.seen_ms = now;
        agent.loaded_ms = None;
        let grant = agent.grant.clone();
        if stale {
            match self.save() {
                Ok(()) => self.mark_saved(),
                Err(e) => rusty_log::warn!(error = %e, "could not record agent use"),
            }
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

    /// Set or clear agent `id`'s nickname; whether it exists.
    ///
    /// # Errors
    /// If the nickname is not 1 to 40 printable characters, or the file cannot be written.
    pub fn set_nickname(&mut self, id: &str, nickname: Option<String>) -> Result<bool, String> {
        let nickname = nickname
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty());
        if let Some(n) = &nickname
            && (n.chars().count() > MAX_NICKNAME || n.chars().any(char::is_control))
        {
            return Err(format!(
                "a nickname is up to {MAX_NICKNAME} printable characters"
            ));
        }
        let Some(agent) = self.agents.get_mut(id) else {
            return Ok(false);
        };
        agent.nickname = nickname;
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

    /// Ids of agents last used (or, if never used, created) more than `age_ms` before `now`.
    pub fn stale(&self, now: i64, age_ms: i64) -> Vec<String> {
        self.agents
            .iter()
            .filter(|(_, a)| now - a.last_use_upper_bound() > age_ms)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// A revocation reached memory but not the file; [`revoke_stale`](Self::revoke_stale) flushes it.
    pub fn has_unsaved_revoke(&self) -> bool {
        self.unsaved_revoke
    }

    /// Remove the [`stale`](Self::stale) agents with one write: all of them, or only those of
    /// `only` that are still stale (an agent used since a preview is kept). The ids removed.
    ///
    /// # Errors
    /// If the file cannot be written. Revocation fails closed: the agents are already gone from
    /// memory (as with [`revoke`](Self::revoke)), and the file may or may not hold them until a
    /// later write succeeds, so the admin should retry.
    pub fn revoke_stale(
        &mut self,
        now: i64,
        age_ms: i64,
        only: Option<&[String]>,
    ) -> Result<Vec<String>, String> {
        let mut ids = self.stale(now, age_ms);
        if let Some(only) = only {
            ids.retain(|id| only.contains(id));
        }
        if ids.is_empty() && !self.unsaved_revoke {
            return Ok(ids);
        }
        for id in &ids {
            self.agents.remove(id);
        }
        self.unsaved_revoke = true;
        self.save()
            .map_err(|e| format!("{e} (revoked in memory; run it again to make it durable)"))?;
        self.unsaved_revoke = false;
        self.mark_saved();
        Ok(ids)
    }

    fn mark_saved(&mut self) {
        for a in self.agents.values_mut() {
            a.saved_ms = a.seen_ms;
        }
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
                nickname: a.nickname.clone(),
            })
            .collect()
    }

    /// Every nickname: agent id to nickname.
    pub fn nicknames(&self) -> std::collections::BTreeMap<String, String> {
        self.agents
            .iter()
            .filter_map(|(id, a)| Some((id.clone(), a.nickname.clone()?)))
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
        let mut r = Registry::open(path.clone(), 0).expect("open");
        let (a, ka) = r.register("c", Grant::default(), 0).expect("a");
        let (b, kb) = r.register("c", Grant::default(), 0).expect("b");
        assert!(r.revoke(&a).expect("revoke"));
        assert!(!r.revoke(&a).expect("again"));
        let mut again = Registry::open(path, 0).expect("reopen");
        assert!(again.verify("c", &a, &ka, 1).is_none());
        assert!(again.verify("c", &b, &kb, 1).is_some());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn stale_means_unused_for_more_than_the_age_and_revoke_stale_keeps_the_rest() {
        const DAY: i64 = 24 * 3600 * 1000;
        let mut r = Registry::default();
        let (old, _) = r.register("c", Grant::default(), 0).expect("old");
        let (edge, ke) = r.register("c", Grant::default(), 0).expect("edge");
        let (used, ku) = r.register("c", Grant::default(), 0).expect("used");
        let now = 3 * DAY;
        assert!(r.verify("c", &edge, &ke, now - DAY).is_some());
        assert!(r.verify("c", &used, &ku, now - 1).is_some());
        // Exactly a day old is not "more than a day"; one ms older is.
        assert_eq!(r.stale(now, DAY), vec![old.clone()]);
        assert_eq!(r.stale(now + 1, DAY).len(), 2);
        assert!(
            r.stale(now, DAY).contains(&old),
            "a dry run removes nothing"
        );
        assert_eq!(r.list().len(), 3, "a dry run removes nothing");
        let gone = r.revoke_stale(now + 1, DAY, None).expect("revoke");
        assert_eq!(gone.len(), 2);
        assert!(r.verify("c", &used, &ku, now + 2).is_some());
        assert!(r.verify("c", &edge, &ke, now + 2).is_none());
        assert!(r.revoke_stale(now + 2, DAY, None).expect("none").is_empty());
    }

    #[test]
    fn only_the_previewed_agents_are_revoked_and_a_since_used_one_is_kept() {
        const DAY: i64 = 24 * 3600 * 1000;
        let mut r = Registry::default();
        let (a, _) = r.register("c", Grant::default(), 0).expect("a");
        let (b, kb) = r.register("c", Grant::default(), 0).expect("b");
        let (c, _) = r.register("c", Grant::default(), 0).expect("c");
        let now = DAY + 1;
        let preview = r.stale(now, DAY);
        assert_eq!(preview.len(), 3);
        assert!(
            r.verify("c", &b, &kb, now).is_some(),
            "used after the preview"
        );
        let only = [a.clone(), b.clone()];
        let gone = r.revoke_stale(now + 1, DAY, Some(&only)).expect("revoke");
        assert_eq!(gone, vec![a]);
        assert!(r.list().iter().any(|x| x.id == b) && r.list().iter().any(|x| x.id == c));
    }

    #[test]
    fn continuous_use_is_written_back_so_a_restart_does_not_make_it_look_stale() {
        const H: i64 = 3600 * 1000;
        let dir = std::env::temp_dir().join(format!("agents-{}", new_token().expect("rand")));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("agents.json");
        let mut r = Registry::open(path.clone(), 0).expect("open");
        let (id, key) = r.register("c", Grant::default(), 0).expect("register");
        for h in 1..=72 {
            assert!(r.verify("c", &id, &key, h * H).is_some());
        }
        let again = Registry::open(path, 0).expect("reopen");
        assert!(
            again.stale(72 * H + 1, 24 * H).is_empty(),
            "used hourly for 3 days"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_lagging_file_never_makes_a_used_agent_look_stale_after_a_restart() {
        const H: i64 = 3600 * 1000;
        let dir = std::env::temp_dir().join(format!("agents-{}", new_token().expect("rand")));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("agents.json");
        let mut r = Registry::open(path.clone(), 0).expect("open");
        let (id, key) = r.register("c", Grant::default(), 0).expect("register");
        // Used at exactly one hour: not written back (the lag is strict), so the file says 0.
        assert!(r.verify("c", &id, &key, H).is_some());
        let mut again = Registry::open(path, H + 1).expect("restart");
        assert!(
            again.stale(3 * H, 2 * H).is_empty(),
            "not over 2h since its last use"
        );
        assert_eq!(
            again.stale(3 * H + 2, 2 * H),
            vec![id.clone()],
            "idle over 2h"
        );
        assert!(again.verify("c", &id, &key, 3 * H).is_some());
        assert!(
            again.stale(3 * H + 2, 2 * H).is_empty(),
            "exact again once used"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_restart_does_not_hide_agents_idle_long_before_it() {
        const H: i64 = 3600 * 1000;
        let dir = std::env::temp_dir().join(format!("agents-{}", new_token().expect("rand")));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("agents.json");
        let mut r = Registry::open(path.clone(), 0).expect("open");
        let (idle, _) = r.register("c", Grant::default(), 0).expect("idle");
        let (used, ku) = r.register("c", Grant::default(), 0).expect("used");
        for h in 1..=72 {
            assert!(r.verify("c", &used, &ku, h * H).is_some());
        }
        let again = Registry::open(path, 72 * H + 1).expect("restart");
        assert_eq!(
            again.stale(72 * H + 2, 24 * H),
            vec![idle],
            "idle for 3 days, found right after the restart; the hourly user is kept"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_failed_use_save_is_bounded_only_by_the_load() {
        const H: i64 = 3600 * 1000;
        let dir = std::env::temp_dir().join(format!("agents-{}", new_token().expect("rand")));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("agents.json");
        let mut r = Registry::open(path.clone(), 0).expect("open");
        let (id, key) = r.register("c", Grant::default(), 0).expect("register");
        // Writes fail while the agent is used at 5h; the file keeps saying 0. Accepted risk
        // (Nano, 2026-11-19): after a restart it counts as last used at 1h.
        r.path = Some(dir.join("gone").join("agents.json"));
        assert!(r.verify("c", &id, &key, 5 * H).is_some());
        let again = Registry::open(path, 6 * H).expect("restart");
        assert!(again.stale(3 * H, 2 * H).is_empty());
        assert_eq!(again.stale(3 * H + 1, 2 * H), vec![id]);
        // Loaded just after registering: never counted as used after the load.
        let dir2 = dir.join("b");
        std::fs::create_dir_all(&dir2).expect("dir2");
        let mut b = Registry::open(dir2.join("agents.json"), 0).expect("open b");
        let (fresh, _) = b.register("c", Grant::default(), 0).expect("fresh");
        let b = Registry::open(dir2.join("agents.json"), H / 2).expect("reload b");
        assert!(b.stale(H / 2 + 2 * H, 2 * H).is_empty());
        assert_eq!(b.stale(H / 2 + 2 * H + 1, 2 * H), vec![fresh]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_failed_write_still_revokes_in_memory_and_says_to_retry() {
        let dir = std::env::temp_dir().join(format!("agents-{}", new_token().expect("rand")));
        std::fs::create_dir_all(&dir).expect("dir");
        let mut r = Registry::open(dir.join("agents.json"), 0).expect("open");
        let (id, key) = r.register("c", Grant::default(), 0).expect("register");
        std::fs::remove_dir_all(&dir).expect("remove dir");
        let e = r.revoke_stale(10, 5, None).expect_err("write fails");
        assert!(e.contains("run it again"), "{e}");
        assert!(r.verify("c", &id, &key, 11).is_none(), "fails closed");
        assert!(r.has_unsaved_revoke());
        // Writable again: a retry with nothing new to revoke still makes it durable.
        std::fs::create_dir_all(&dir).expect("restore dir");
        assert!(r.revoke_stale(12, 5, None).expect("retry").is_empty());
        assert!(!r.has_unsaved_revoke());
        let mut again = Registry::open(dir.join("agents.json"), 0).expect("reopen");
        assert!(
            again.verify("c", &id, &key, 13).is_none(),
            "gone from the file"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_grant_is_kept_returned_replaced_and_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("agents-{}", new_token().expect("rand")));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("agents.json");
        let mut r = Registry::open(path.clone(), 0).expect("open");
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
        let mut again = Registry::open(path, 0).expect("reopen");
        assert_eq!(again.verify("c", &id, &key, 2), Some(reviewer));
        assert!(again.set_grant(&id, Grant::default()).expect("clear"));
        assert_eq!(again.verify("c", &id, &key, 3), Some(Grant::default()));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_nickname_is_trimmed_validated_listed_and_cleared() {
        let mut r = Registry::default();
        let (id, _) = r.register("c", Grant::default(), 0).expect("register");
        assert!(r.set_nickname(&id, Some("  QA bot ".into())).expect("set"));
        assert_eq!(r.nicknames().get(&id).map(String::as_str), Some("QA bot"));
        assert_eq!(r.list()[0].nickname.as_deref(), Some("QA bot"));
        assert!(r.set_nickname(&id, Some("x".repeat(41))).is_err());
        assert!(r.set_nickname(&id, Some("bad\nname".into())).is_err());
        assert!(
            !r.set_nickname("c#nope00", Some("x".into()))
                .expect("unknown")
        );
        assert!(
            r.set_nickname(&id, Some("  ".into()))
                .expect("blank clears")
        );
        assert!(r.nicknames().is_empty());
    }
}
