//! The planner: observed state in, [`Plan`] out. Never touches the disk.

use crate::model::{Entry, LinkMethod, Tool};
use std::path::PathBuf;

/// What an adapter found at an entry's target path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observed {
    /// Nothing there.
    Absent,
    /// A link or copy carrying our marker, pointing at this entry's source.
    ManagedCurrent,
    /// Carries our marker but points elsewhere or is stale.
    ManagedStale,
    /// Something we cannot prove we own. Never replaced or removed.
    Unmanaged,
}

/// One step of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Link {
        from: PathBuf,
        to: PathBuf,
        method: LinkMethod,
    },
    Unlink {
        path: PathBuf,
    },
    /// Declined, with the reason shown to the user. Applying a plan never
    /// touches a refused path.
    Refuse {
        path: PathBuf,
        reason: &'static str,
    },
}

/// An ordered list of actions. Empty means nothing to do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub actions: Vec<Action>,
}

impl Plan {
    pub fn is_noop(&self) -> bool {
        self.actions.is_empty()
    }

    /// True if any step was refused; the caller should explain, not apply.
    pub fn has_refusals(&self) -> bool {
        self.actions
            .iter()
            .any(|a| matches!(a, Action::Refuse { .. }))
    }
}

const UNMANAGED: &str =
    "exists and is not managed by Ai Env Manager; refusing to replace or remove it";

fn one(action: Action) -> Plan {
    Plan {
        actions: vec![action],
    }
}

/// Plan linking `entry` into `tool` with `method`.
pub fn plan_enable(entry: &Entry, tool: &Tool, observed: Observed, method: LinkMethod) -> Plan {
    let to = tool.target(entry);
    match observed {
        Observed::ManagedCurrent => Plan::default(),
        Observed::Absent => one(Action::Link {
            from: entry.source.clone(),
            to,
            method,
        }),
        // Ours but wrong: replace in two explicit steps so the plan shows both.
        Observed::ManagedStale => Plan {
            actions: vec![
                Action::Unlink { path: to.clone() },
                Action::Link {
                    from: entry.source.clone(),
                    to,
                    method,
                },
            ],
        },
        Observed::Unmanaged => one(Action::Refuse {
            path: to,
            reason: UNMANAGED,
        }),
    }
}

/// Plan removing `entry` from `tool`.
pub fn plan_disable(entry: &Entry, tool: &Tool, observed: Observed) -> Plan {
    let path = tool.target(entry);
    match observed {
        Observed::Absent => Plan::default(),
        Observed::ManagedCurrent | Observed::ManagedStale => one(Action::Unlink { path }),
        Observed::Unmanaged => one(Action::Refuse {
            path,
            reason: UNMANAGED,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Error, Kind, Os};

    fn fixture() -> (Entry, Tool) {
        let entry = Entry {
            id: "code-review".parse().unwrap(),
            kind: Kind::Skill,
            source: PathBuf::from("/src/skills/code-review"),
        };
        let tool = Tool {
            id: "claude-code".parse().unwrap(),
            skills_dir: PathBuf::from("/home/u/.claude/skills"),
        };
        (entry, tool)
    }

    #[test]
    fn enable_absent_links() {
        let (e, t) = fixture();
        let p = plan_enable(&e, &t, Observed::Absent, LinkMethod::Symlink);
        assert_eq!(
            p.actions,
            vec![Action::Link {
                from: "/src/skills/code-review".into(),
                to: "/home/u/.claude/skills/code-review".into(),
                method: LinkMethod::Symlink
            }]
        );
    }

    #[test]
    fn enable_current_is_noop() {
        let (e, t) = fixture();
        assert!(plan_enable(&e, &t, Observed::ManagedCurrent, LinkMethod::Symlink).is_noop());
    }

    #[test]
    fn enable_stale_unlinks_then_links() {
        let (e, t) = fixture();
        let p = plan_enable(&e, &t, Observed::ManagedStale, LinkMethod::Copy);
        assert!(matches!(
            p.actions[..],
            [
                Action::Unlink { .. },
                Action::Link {
                    method: LinkMethod::Copy,
                    ..
                }
            ]
        ));
    }

    #[test]
    fn enable_over_unmanaged_is_refused() {
        let (e, t) = fixture();
        let p = plan_enable(&e, &t, Observed::Unmanaged, LinkMethod::Symlink);
        assert!(p.has_refusals());
        assert!(!p
            .actions
            .iter()
            .any(|a| matches!(a, Action::Link { .. } | Action::Unlink { .. })));
    }

    #[test]
    fn disable_managed_unlinks_and_absent_is_noop() {
        let (e, t) = fixture();
        assert!(matches!(
            plan_disable(&e, &t, Observed::ManagedCurrent).actions[..],
            [Action::Unlink { .. }]
        ));
        assert!(matches!(
            plan_disable(&e, &t, Observed::ManagedStale).actions[..],
            [Action::Unlink { .. }]
        ));
        assert!(plan_disable(&e, &t, Observed::Absent).is_noop());
    }

    #[test]
    fn disable_unmanaged_is_refused_never_unlinked() {
        let (e, t) = fixture();
        let p = plan_disable(&e, &t, Observed::Unmanaged);
        assert!(p.has_refusals());
        assert!(!p.actions.iter().any(|a| matches!(a, Action::Unlink { .. })));
    }

    #[test]
    fn ids_reject_traversal_and_junk() {
        for bad in ["", ".", "..", ".hidden", "a/b", "a\\b", "a b", "../x", "é"] {
            assert_eq!(
                bad.parse::<crate::EntryId>(),
                Err(Error::InvalidId(bad.to_owned())),
                "{bad:?}"
            );
        }
        assert!("code-review_2.0".parse::<crate::EntryId>().is_ok());
    }

    #[test]
    fn kinds_parse_and_unknown_fails() {
        assert_eq!("claude-md".parse::<Kind>(), Ok(Kind::ClaudeMd));
        assert_eq!("agent".parse::<Kind>(), Ok(Kind::Agent));
        assert_eq!(
            "plugin".parse::<Kind>(),
            Err(Error::UnknownKind("plugin".into()))
        );
    }

    #[test]
    fn windows_chain_has_junction_between_symlink_and_copy() {
        assert_eq!(
            LinkMethod::fallback_chain(Os::Windows),
            &[LinkMethod::Symlink, LinkMethod::Junction, LinkMethod::Copy]
        );
        assert_eq!(
            LinkMethod::fallback_chain(Os::Unix),
            &[LinkMethod::Symlink, LinkMethod::Copy]
        );
    }
}
