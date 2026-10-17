//! Provider-neutral no-progress protection, replayed from the existing BBP log.
//! Only an explicit reset revision is host metadata; no protocol events are copied.
use crate::open_driver;
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Consecutive forfeits/timeouts allowed without a role publishing an artifact or message.
pub const NO_PROGRESS_LIMIT: u64 = 3;

/// The moderator's replayed launcher health, independent of provider output formats.
#[derive(Debug, Serialize, Deserialize)]
pub struct Health {
    /// BBP revision inspected; supply it to the explicit reset command.
    pub rev: u64,
    /// Consecutive no-progress forfeits per configured role.
    pub consecutive: BTreeMap<String, u64>,
    /// Role whose launch loop is paused, if any.
    pub paused: Option<String>,
}
impl Health {
    /// Refuse further automatic launches until the owner repairs and acknowledges the fault.
    pub fn check(&self) -> Result<(), String> {
        match &self.paused {
            Some(role) => Err(format!("moderator paused: role {role} forfeited {NO_PROGRESS_LIMIT} turns without publishing an artifact or message; inspect its agent logs, repair the launcher, then use bbp mod-health --reset yes --rev {} (with this task's --dir and --task)", self.rev)),
            None => Ok(()),
        }
    }
}

fn reset_path(dir: &Path, task: &TaskId) -> PathBuf {
    dir.join("mod")
        .join(format!("{}.retry", Sha256::of(task.0.as_bytes()).hex()))
}

fn fold(events: &[Event], after: u64) -> BTreeMap<String, u64> {
    let mut principals = BTreeMap::new();
    let mut live = BTreeMap::<u64, (String, bool)>::new();
    let mut counts = BTreeMap::<String, u64>::new();
    for (index, event) in events.iter().enumerate() {
        let recent = index as u64 + 1 > after;
        let published = match event {
            Event::Assigned { role, principal } => {
                principals.insert(principal.id.0.clone(), role.0.clone());
                None
            }
            Event::TurnGranted { role, turn, .. } => {
                // A turn granted before an owner reset belongs to the old
                // attempt. Recovery may abort it, but must not charge a new streak.
                live.insert(turn.0, (role.0.clone(), !recent));
                None
            }
            Event::ArtifactStored(a) => principals.get(&a.author.0).cloned(),
            Event::MessageAppended(m) => m.from_role.as_ref().map(|r| r.0.clone()),
            Event::TurnEnded { turn, cause } => {
                if let Some((role, published)) = live.remove(&turn.0) {
                    if recent && !published && matches!(cause, TurnEnd::Aborted | TurnEnd::Deadline)
                    {
                        *counts.entry(role).or_default() += 1;
                    }
                }
                None
            }
            _ => None,
        };
        if let Some(role) = published {
            if recent {
                counts.remove(&role);
            }
            for (holder, published) in live.values_mut() {
                if *holder == role {
                    *published = true;
                }
            }
        }
    }
    counts
}

pub(crate) fn inspect(d: &Driver<FsStore>, dir: &Path, task: &TaskId) -> Result<Health, String> {
    let path = reset_path(dir, task);
    let after = match std::fs::read_to_string(&path) {
        Ok(text) => text
            .trim()
            .parse::<u64>()
            .map_err(|e| format!("moderator reset marker: {e}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(format!("moderator reset marker: {error}")),
    };
    if after > d.state.rev.0 {
        return Err("moderator reset marker is ahead of the task revision".into());
    }
    let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
    let consecutive = fold(&events, after);
    let paused = (!d.state.state.terminal())
        .then(|| {
            consecutive
                .iter()
                .find(|(_, count)| **count >= NO_PROGRESS_LIMIT)
                .map(|(role, _)| role.clone())
        })
        .flatten();
    Ok(Health {
        rev: d.state.rev.0,
        consecutive,
        paused,
    })
}

/// Read the health projection without starting a moderator or provider.
pub fn read(dir: &Path, task: &TaskId) -> Result<Health, String> {
    inspect(&open_driver(dir, task)?, dir, task)
}

pub(crate) fn acknowledge(dir: &Path, task: &TaskId, rev: Rev) -> Result<(), String> {
    rusty_atomic_file::write_private(&reset_path(dir, task), rev.0.to_string().as_bytes())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    fn grant(role: &str, turn: u64) -> Event {
        Event::TurnGranted {
            role: role.into(),
            turn: TurnId(turn),
            kind: TurnKind::Default,
            deadline: Time(999),
            return_to: None,
            slice: None,
        }
    }
    fn abort(turn: u64) -> Event {
        Event::TurnEnded {
            turn: TurnId(turn),
            cause: TurnEnd::Aborted,
        }
    }
    #[test]
    fn replay_counts_role_failures_across_restarts_but_not_reads_or_other_roles() {
        let mut events = vec![];
        for turn in 1..=3 {
            events.extend([
                grant("custom-writer", turn),
                Event::ReadDeduped {
                    turn: TurnId(turn),
                    art: ArtId(1),
                },
                abort(turn),
            ]);
        }
        events.extend([grant("different-role", 4), abort(4)]);
        assert_eq!(fold(&events, 0).get("custom-writer"), Some(&3));
        assert_eq!(fold(&events, 0).get("different-role"), Some(&1));
        assert!(fold(&events, events.len() as u64).is_empty());
    }
    #[test]
    fn reset_does_not_count_cleanup_of_a_turn_granted_before_acknowledgement() {
        let events = vec![grant("writer", 1), abort(1), grant("writer", 2), abort(2)];
        assert_eq!(fold(&events, 1).get("writer"), Some(&1));
    }

    #[test]
    fn a_published_message_resets_only_its_role_and_revocation_is_not_failure() {
        let mut events = vec![grant("writer", 1), abort(1), grant("writer", 2)];
        events.push(Event::MessageAppended(Message {
            id: MsgId(1),
            from: "agent".into(),
            from_role: Some("writer".into()),
            draft: Draft::new(MessageKind::Finding, "progress"),
        }));
        events.extend([
            abort(2),
            grant("writer", 3),
            Event::TurnEnded {
                turn: TurnId(3),
                cause: TurnEnd::Revoked,
            },
        ]);
        assert!(!fold(&events, 0).contains_key("writer"));
    }
}
