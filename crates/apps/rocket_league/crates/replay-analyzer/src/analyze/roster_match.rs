//! Team-anchored roster identity matching.
//!
//! Pairs two rosters of the *same* match by player name when one side's names are
//! inconsistently mangled — ballchasing.com ASCII-folds (`Thómer`→`Thomer`),
//! strips characters (`357ß`→`357`), or occasionally shows a different platform
//! string, while the analyzer preserves the true in-replay name. Used to align the
//! analyzer's players with an external source: ground-truth stats
//! (`analyze::validate`) and the ranked corpus manifest (the `scoring` crate's
//! `calibrate`), so a mangled name still resolves to the right player.
//!
//! Two passes: exact name match, then the unique remaining slot per team (a forced
//! 1-1, so it is unambiguous whenever at most one name per team is mangled — the
//! common case in 2v2/3v3). A slot named `<unknown>` (a spurious or team-less
//! track) is never anchored in pass 2.

/// Sentinel name for an unidentified track — never team-anchored.
const UNKNOWN: &str = "<unknown>";

/// One roster slot: a team (RL teams are `0`/`1`; `None` = unbound) and a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RosterSlot<'a> {
    pub team: Option<i32>,
    pub name: &'a str,
}

impl<'a> RosterSlot<'a> {
    pub fn new(team: Option<i32>, name: &'a str) -> Self {
        Self { team, name }
    }
}

/// Pair `left` to `right` by identity, tolerant of mangled names, returning
/// `(left_index, right_index)` pairs.
///
/// Pass 1 matches exact names (the first unused `left` for each `right`). Pass 2
/// pairs the unique residual on each side *within a team* — unambiguous when at
/// most one name per team is mangled; a team with two mangled residuals is left
/// unpaired rather than guessed. `left` slots named `<unknown>` are excluded from
/// pass 2 (a spurious / team-less track is not a real player).
pub fn team_anchored_pairs(left: &[RosterSlot], right: &[RosterSlot]) -> Vec<(usize, usize)> {
    let mut lused = vec![false; left.len()];
    let mut rused = vec![false; right.len()];
    let mut pairs = Vec::new();

    // Pass 1: exact name match.
    for (ri, r) in right.iter().enumerate() {
        if let Some((li, _)) = left
            .iter()
            .enumerate()
            .find(|(li, l)| !lused[*li] && l.name == r.name)
        {
            lused[li] = true;
            rused[ri] = true;
            pairs.push((li, ri));
        }
    }

    // Pass 2: per team, pair the unique remaining slot on each side.
    for team in [0, 1] {
        let ls: Vec<usize> = (0..left.len())
            .filter(|&i| !lused[i] && left[i].team == Some(team) && left[i].name != UNKNOWN)
            .collect();
        let rs: Vec<usize> = (0..right.len())
            .filter(|&i| !rused[i] && right[i].team == Some(team))
            .collect();
        if let ([li], [ri]) = (ls.as_slice(), rs.as_slice()) {
            lused[*li] = true;
            rused[*ri] = true;
            pairs.push((*li, *ri));
        }
    }

    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(team: i32, name: &str) -> RosterSlot<'_> {
        RosterSlot::new(Some(team), name)
    }

    #[test]
    fn exact_names_match_directly() {
        let left = [slot(0, "Alice"), slot(1, "Bob")];
        let right = [slot(1, "Bob"), slot(0, "Alice")];
        let mut pairs = team_anchored_pairs(&left, &right);
        pairs.sort();
        // (left Alice -> right Alice@1), (left Bob -> right Bob@0)
        assert_eq!(pairs, vec![(0, 1), (1, 0)]);
    }

    #[test]
    fn mangled_name_resolves_via_unique_team_residual() {
        // Bób (ours) vs Bob (theirs): no exact match, but the unique team-0
        // residual on each side forces the pair.
        let left = [slot(0, "Alice"), slot(0, "Bób"), slot(1, "Carol")];
        let right = [slot(0, "Alice"), slot(0, "Bob"), slot(1, "Carol")];
        let mut pairs = team_anchored_pairs(&left, &right);
        pairs.sort();
        assert_eq!(pairs, vec![(0, 0), (1, 1), (2, 2)]);
    }

    #[test]
    fn unknown_is_never_anchored_and_two_mangled_residuals_stay_unpaired() {
        // Team 0: Alice exact + an <unknown> track (must not anchor to Bob).
        // Team 1: both names mangled (2 vs 2 residual) -> ambiguous, left unpaired.
        let left = [
            slot(0, "Alice"),
            slot(0, "<unknown>"),
            slot(1, "Çarol"),
            slot(1, "Davé"),
        ];
        let right = [
            slot(0, "Alice"),
            slot(0, "Bob"),
            slot(1, "Carol"),
            slot(1, "Dave"),
        ];
        let pairs = team_anchored_pairs(&left, &right);
        assert_eq!(pairs, vec![(0, 0)], "only the exact Alice match survives");
    }

    #[test]
    fn a_teamless_left_slot_is_not_anchored() {
        // A residual with no team can't be team-anchored even if it's the only one.
        let left = [RosterSlot::new(None, "Ghost")];
        let right = [slot(0, "Bob")];
        assert!(team_anchored_pairs(&left, &right).is_empty());
    }
}
