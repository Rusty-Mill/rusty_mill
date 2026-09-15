//! Generation-scoped operation identity (`RM-ASYNC-OP-0001`/`0002`):
//! every submitted operation gets a provider-unique, generation-scoped
//! id and exactly one terminal completion. This module defines the
//! identity type only — what "submitted" and "terminal" mean is a
//! domain concern that belongs in `platform-async`, not here.

use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonically increasing generation counter, scoped to one engine
/// instance. Guards against a completion from a torn-down and recreated
/// engine being mistaken for a live operation's result — the same
/// "reject stale reuse" discipline `RM-ASYNC-REG-0001` applies to
/// resource registration, applied here to operation identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(u64);

impl Generation {
    pub const fn initial() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl Default for Generation {
    fn default() -> Self {
        Self::initial()
    }
}

/// A provider-unique, generation-scoped operation identity
/// (`RM-ASYNC-OP-0001`). Two ids are equal only if they share both the
/// same engine generation and the same sequence number within it — an
/// id from a prior generation never aliases one from the current
/// generation, even if the sequence numbers collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OperationId {
    generation: Generation,
    sequence: u64,
}

impl OperationId {
    pub const fn new(generation: Generation, sequence: u64) -> Self {
        Self {
            generation,
            sequence,
        }
    }

    pub const fn generation(self) -> Generation {
        self.generation
    }

    pub const fn sequence(self) -> u64 {
        self.sequence
    }
}

/// Issues generation-scoped, provider-unique [`OperationId`]s for one
/// engine instance. Not `Clone`: an engine owns exactly one issuer, so
/// two issuers can never hand out colliding ids for the same
/// generation.
#[derive(Debug)]
pub struct OperationIdIssuer {
    generation: Generation,
    next_sequence: AtomicU64,
}

impl OperationIdIssuer {
    pub const fn new(generation: Generation) -> Self {
        Self {
            generation,
            next_sequence: AtomicU64::new(0),
        }
    }

    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Issue the next id in this issuer's generation. Wraps only after
    /// 2^64 operations in one engine generation's lifetime, which no
    /// supported workload reaches.
    pub fn issue(&self) -> OperationId {
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        OperationId::new(self.generation, sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_within_a_generation_are_unique_and_increasing() {
        let issuer = OperationIdIssuer::new(Generation::initial());
        let a = issuer.issue();
        let b = issuer.issue();
        assert_ne!(a, b);
        assert_eq!(a.generation(), b.generation());
        assert!(b.sequence() > a.sequence());
    }

    #[test]
    fn ids_across_generations_never_alias() {
        let gen0 = OperationIdIssuer::new(Generation::initial());
        let gen1 = OperationIdIssuer::new(Generation::initial().next());
        let a = gen0.issue();
        let b = gen1.issue();
        // Same sequence number (both 0), different generation: must
        // not compare equal — this is the stale-completion guard.
        assert_eq!(a.sequence(), b.sequence());
        assert_ne!(a, b);
    }
}
