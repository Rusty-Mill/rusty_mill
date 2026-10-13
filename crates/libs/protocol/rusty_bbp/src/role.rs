//! Roles as data. A [`Role`] is an id plus a capability set; a [`Roster`] is a
//! validated set of them supplied when a task opens. The protocol names no
//! role: every permission below is checked as a capability, never by id.

use crate::ids::RoleId;
use crate::record::{ArtifactKind, MessageKind};
use crate::state::State;
use rusty_serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// A verdict stage: the working state a verdict judges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Stage {
    Test,
    Review,
}

impl Stage {
    /// The stage a state judges, if it is one.
    pub fn of(state: State) -> Option<Stage> {
        match state {
            State::Test => Some(Stage::Test),
            State::Review => Some(Stage::Review),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Stage::Test => "test",
            Stage::Review => "review",
        }
    }
}

/// Which messages a role reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReadScope {
    /// Every message on the task.
    All,
    /// Human messages addressed to the role (or to everyone), plus human
    /// decisions and answers about the role's own messages.
    Scoped,
}

/// An identifier plus a capability set. See the spec's Role table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub id: RoleId,
    /// Position when several roles serve one state: the lowest wins. Unique in a roster.
    pub turn_order: u16,
    /// Message kinds the role may post. `decision` is never postable by an agent.
    pub post: Vec<MessageKind>,
    /// Artifact kinds the role may store (`spec`, `diff`, `candidate`).
    pub write: Vec<ArtifactKind>,
    pub read_artifacts: Vec<ArtifactKind>,
    pub read_messages: ReadScope,
    /// Stages the role may judge.
    pub verdict_at: Vec<Stage>,
    /// The role's `approve` at `review` is what the merge gate requires.
    pub final_decision: bool,
    /// The role may receive a yielded turn.
    pub consultable: bool,
}

impl Role {
    /// A role with no capabilities that reads every message and no artifact.
    pub fn new(id: &str, turn_order: u16) -> Role {
        Role {
            id: RoleId::from(id),
            turn_order,
            post: Vec::new(),
            write: Vec::new(),
            read_artifacts: Vec::new(),
            read_messages: ReadScope::All,
            verdict_at: Vec::new(),
            final_decision: false,
            consultable: false,
        }
    }

    pub fn post(mut self, kinds: &[MessageKind]) -> Role {
        self.post = kinds.to_vec();
        self
    }

    pub fn write(mut self, kinds: &[ArtifactKind]) -> Role {
        self.write = kinds.to_vec();
        self
    }

    pub fn read_artifacts(mut self, kinds: &[ArtifactKind]) -> Role {
        self.read_artifacts = kinds.to_vec();
        self
    }

    pub fn scoped(mut self) -> Role {
        self.read_messages = ReadScope::Scoped;
        self
    }

    pub fn verdict_at(mut self, stages: &[Stage]) -> Role {
        self.verdict_at = stages.to_vec();
        self
    }

    pub fn final_decision(mut self) -> Role {
        self.final_decision = true;
        self
    }

    pub fn consultable(mut self) -> Role {
        self.consultable = true;
        self
    }

    pub fn holds(&self, cap: Cap) -> bool {
        match cap {
            Cap::Post(k) => self.post.contains(&k),
            Cap::Write(k) => self.write.contains(&k),
            Cap::Read(k) => self.read_artifacts.contains(&k),
            Cap::Verdict(s) => self.verdict_at.contains(&s),
        }
    }

    /// The rejection detail for a missing capability.
    pub fn lacks(&self, cap: Cap) -> String {
        format!("role {} lacks {cap}", self.id)
    }
}

/// One capability a role can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    Post(MessageKind),
    Write(ArtifactKind),
    Read(ArtifactKind),
    Verdict(Stage),
}

impl fmt::Display for Cap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cap::Post(k) => write!(f, "post:{}", k.name()),
            Cap::Write(k) => write!(f, "write:{}", k.name()),
            Cap::Read(k) => write!(f, "read_artifacts:{}", k.name()),
            Cap::Verdict(s) => write!(f, "verdict_at:{}", s.name()),
        }
    }
}

/// What a working state needs from the role that serves it. The one table
/// that derives each state's default turn holder and validates the roster.
pub struct Requirement {
    pub state: State,
    /// Capabilities the serving role holds.
    pub caps: &'static [Cap],
    /// The serving role also holds `final_decision`.
    pub finalizes: bool,
}

pub const REQUIREMENTS: [Requirement; 4] = [
    Requirement {
        state: State::Planning,
        caps: &[
            Cap::Write(ArtifactKind::Spec),
            Cap::Post(MessageKind::RequestDecision),
        ],
        finalizes: false,
    },
    Requirement {
        state: State::Build,
        caps: &[
            Cap::Write(ArtifactKind::Diff),
            Cap::Write(ArtifactKind::Candidate),
        ],
        finalizes: false,
    },
    Requirement {
        state: State::Test,
        caps: &[Cap::Verdict(Stage::Test), Cap::Post(MessageKind::Verdict)],
        finalizes: false,
    },
    Requirement {
        state: State::Review,
        caps: &[Cap::Verdict(Stage::Review), Cap::Post(MessageKind::Verdict)],
        finalizes: true,
    },
];

impl Requirement {
    fn holds_caps(&self, r: &Role) -> bool {
        self.caps.iter().all(|c| r.holds(*c))
    }

    /// Does `r` serve this state?
    pub fn served_by(&self, r: &Role) -> bool {
        self.holds_caps(r) && (!self.finalizes || r.final_decision)
    }

    fn describe(&self) -> String {
        let mut parts: Vec<String> = self.caps.iter().map(|c| c.to_string()).collect();
        if self.finalizes {
            parts.push("final_decision".into());
        }
        parts.join(" + ")
    }
}

/// Why a roster was refused. Each variant names the check and the role or state involved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RosterError {
    /// Not 1 to 32 of `[a-z0-9_-]` starting with a letter.
    BadId {
        id: RoleId,
    },
    /// `human`, `moderator` and `runner` are protocol principals.
    ReservedId {
        id: RoleId,
    },
    DuplicateId {
        id: RoleId,
    },
    DuplicateTurnOrder {
        order: u16,
        first: RoleId,
        second: RoleId,
    },
    /// No role serves a working state: nobody could leave it.
    Unservable {
        state: State,
        needs: String,
    },
    /// Some role could judge `review` but none holds `final_decision`.
    NoFinalizer,
}

impl fmt::Display for RosterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RosterError::BadId { id } => write!(
                f,
                "role id {id:?} is not 1 to 32 of [a-z0-9_-] starting with a letter"
            ),
            RosterError::ReservedId { id } => write!(f, "role id {id} is reserved"),
            RosterError::DuplicateId { id } => write!(f, "role id {id} appears twice"),
            RosterError::DuplicateTurnOrder {
                order,
                first,
                second,
            } => write!(f, "roles {first} and {second} share turn_order {order}"),
            RosterError::Unservable { state, needs } => {
                write!(f, "no role serves {state:?}; it needs {needs}")
            }
            RosterError::NoFinalizer => write!(
                f,
                "no finalizer: a review judge must also hold final_decision"
            ),
        }
    }
}

impl std::error::Error for RosterError {}

const RESERVED: [&str; 3] = ["human", "moderator", "runner"];

fn valid_token(id: &str) -> bool {
    let mut chars = id.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    first_ok
        && id.len() <= 32
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A validated set of roles. Immutable once built.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roster {
    roles: Vec<Role>,
}

impl Roster {
    /// Validate and build. See the spec's Roster section for the checks.
    pub fn new(roles: Vec<Role>) -> Result<Roster, RosterError> {
        let mut ids: BTreeMap<&RoleId, ()> = BTreeMap::new();
        let mut orders: BTreeMap<u16, &RoleId> = BTreeMap::new();
        for r in &roles {
            if !valid_token(&r.id.0) {
                return Err(RosterError::BadId { id: r.id.clone() });
            }
            if RESERVED.contains(&r.id.0.as_str()) {
                return Err(RosterError::ReservedId { id: r.id.clone() });
            }
            if ids.insert(&r.id, ()).is_some() {
                return Err(RosterError::DuplicateId { id: r.id.clone() });
            }
        }
        for r in &roles {
            if let Some(first) = orders.insert(r.turn_order, &r.id) {
                return Err(RosterError::DuplicateTurnOrder {
                    order: r.turn_order,
                    first: first.clone(),
                    second: r.id.clone(),
                });
            }
        }
        for q in &REQUIREMENTS {
            if roles.iter().any(|r| q.served_by(r)) {
                continue;
            }
            return Err(if q.finalizes && roles.iter().any(|r| q.holds_caps(r)) {
                RosterError::NoFinalizer
            } else {
                RosterError::Unservable {
                    state: q.state,
                    needs: q.describe(),
                }
            });
        }
        Ok(Roster { roles })
    }

    pub fn roles(&self) -> &[Role] {
        &self.roles
    }

    pub fn get(&self, id: &RoleId) -> Option<&Role> {
        self.roles.iter().find(|r| &r.id == id)
    }

    /// The roles that serve `state`, in `turn_order`. The first is the default
    /// holder; the rest may hold concurrent turns when `max_live_turns > 1`.
    pub fn serving(&self, state: State) -> Vec<&RoleId> {
        let Some(q) = REQUIREMENTS.iter().find(|q| q.state == state) else {
            return Vec::new();
        };
        let mut roles: Vec<&Role> = self.roles.iter().filter(|r| q.served_by(r)).collect();
        roles.sort_by_key(|r| r.turn_order);
        roles.into_iter().map(|r| &r.id).collect()
    }

    /// The role that holds the default turn in `state`: the serving role with
    /// the lowest `turn_order`.
    pub fn default_holder(&self, state: State) -> Option<&RoleId> {
        self.serving(state).into_iter().next()
    }

    /// Derived yield rule: in a working state a role may be yielded to when it
    /// is `consultable` and is not that state's default holder.
    pub fn yieldable_in(&self, state: State, target: &RoleId) -> bool {
        let working = REQUIREMENTS.iter().any(|q| q.state == state);
        working
            && self.get(target).is_some_and(|r| r.consultable)
            && self.default_holder(state) != Some(target)
    }

    /// Does `role` hold `final_decision`?
    pub fn is_final(&self, role: &RoleId) -> bool {
        self.get(role).is_some_and(|r| r.final_decision)
    }
}
