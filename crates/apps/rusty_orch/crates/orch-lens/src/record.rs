//! A lens's track record on one task: informational only (#599 Q2). It is
//! shown to the lens as one line and never weights a score.

use rusty_bbp::{ClaimStatus, Direction};

/// Counts for one lens on one task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Record {
    /// Evidence whose direction matched the claim's settled outcome.
    pub aligned: u32,
    /// Evidence whose direction went against it.
    pub misaligned: u32,
    /// Blind passes that found counter evidence.
    pub found_problems: u32,
    /// Blind passes where the look-harder re-ask found nothing either.
    pub found_nothing_twice: u32,
    /// Answers refused by the reply protocol.
    pub refused: u32,
}

impl Record {
    /// Score one of the lens's evidence items against how its claim settled.
    /// Open, escalated and superseded claims are not counted.
    pub fn settle(&mut self, direction: Direction, outcome: ClaimStatus) {
        let agreed = match (direction, outcome) {
            (Direction::Support, ClaimStatus::Accepted) => true,
            (Direction::Counter, ClaimStatus::Dropped | ClaimStatus::Void) => true,
            (Direction::Support, ClaimStatus::Dropped | ClaimStatus::Void) => false,
            (Direction::Counter, ClaimStatus::Accepted) => false,
            _ => return,
        };
        if agreed {
            self.aligned += 1;
        } else {
            self.misaligned += 1;
        }
    }

    /// The line put in the lens's context.
    pub fn line(&self) -> String {
        format!(
            "{} aligned, {} misaligned with settled outcomes; found problems {} times, nothing after looking harder {} times; {} replies refused",
            self.aligned, self.misaligned, self.found_problems, self.found_nothing_twice, self.refused
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settled_outcomes_count_and_open_ones_do_not() {
        let mut r = Record::default();
        r.settle(Direction::Support, ClaimStatus::Accepted);
        r.settle(Direction::Counter, ClaimStatus::Dropped);
        r.settle(Direction::Counter, ClaimStatus::Accepted);
        r.settle(Direction::Support, ClaimStatus::Open);
        r.settle(Direction::Support, ClaimStatus::Escalated);
        assert_eq!((r.aligned, r.misaligned), (2, 1));
        assert!(r.line().starts_with("2 aligned, 1 misaligned"));
    }
}
