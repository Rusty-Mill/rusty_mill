//! Role → agent routing. A card never names an agent (ADR-0001); this
//! table does, and the review rule keeps reviewer ≠ author.

use std::collections::HashSet;
use std::fmt;

use orch_core::task::{Agent, Role};

/// Routing as configured, before validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingConfig {
    pub research: Agent,
    pub design: Agent,
    pub implement: Agent,
    pub triage: Agent,
    /// Reviewer preference, most preferred first. The first agent that is
    /// not the target's author gets the review.
    pub reviewers: Vec<Agent>,
}

/// Why a routing config was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingError {
    /// Fewer than two distinct reviewers: some author could never be reviewed.
    TooFewReviewers { distinct: usize },
}

impl fmt::Display for RoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewReviewers { distinct } => write!(
                f,
                "review routing needs at least two distinct agents, got {distinct}"
            ),
        }
    }
}

impl std::error::Error for RoutingError {}

/// A validated routing table. Build with [`Routing::try_from`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routing(RoutingConfig);

impl TryFrom<RoutingConfig> for Routing {
    type Error = RoutingError;

    fn try_from(config: RoutingConfig) -> Result<Self, Self::Error> {
        let distinct = config.reviewers.iter().collect::<HashSet<_>>().len();
        if distinct < 2 {
            return Err(RoutingError::TooFewReviewers { distinct });
        }
        Ok(Self(config))
    }
}

impl Routing {
    /// The agent for a work role; `None` for [`Role::Review`], which needs
    /// the target's author (see [`Routing::reviewer`]).
    pub fn work(&self, role: Role) -> Option<Agent> {
        match role {
            Role::Research => Some(self.0.research),
            Role::Design => Some(self.0.design),
            Role::Implement => Some(self.0.implement),
            Role::Triage => Some(self.0.triage),
            Role::Review { .. } => None,
        }
    }

    /// The most preferred reviewer that is not `author`. Always `Some` for a
    /// validated table, since two distinct reviewers cannot both be `author`.
    pub fn reviewer(&self, author: Agent) -> Option<Agent> {
        self.0.reviewers.iter().copied().find(|a| *a != author)
    }
}
