//! Team workspaces: a roster, roles, and a shared pool of matches.
//!
//! A team is a named set of accounts, each a **coach** or a **player**. Any member
//! may upload a match to the team's pool; the pool is what the team reports are
//! computed from. Attribution from a replay's display names to roster members goes
//! through each member's *in-game name* (defaults to the account name), since the
//! canonical model has no stable platform id yet.
//!
//! Visibility is role-gated: a coach sees the whole roster plus a team rollup; a
//! player sees the rollup and only their own row.
//!
//! Roster file, one team per line (`#` comments and blank lines ignored):
//!
//! ```text
//! aces: coach:bailey, player:alice=Schutzein, player:bob=Nadir
//! ```
//!
//! This module is pure: it parses, checks and computes; the caller supplies the
//! pool of [`SessionRecord`]s.

use std::fmt;
use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::history::{habits, Focus, HabitReport, PlayerSnapshot, RecurringFault, SessionRecord};
use crate::store::AccountId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Coach,
    Player,
}

impl Role {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "coach" => Some(Self::Coach),
            "player" => Some(Self::Player),
            _ => None,
        }
    }
}

/// A validated team name. Uses the same charset as [`AccountId`] because the team
/// pool is stored in a directory named after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamId(AccountId);

impl TeamId {
    pub fn new(name: &str) -> Result<Self, TeamsError> {
        AccountId::new(name)
            .map(Self)
            .map_err(|e| TeamsError(format!("team name: {e}")))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The storage namespace of this team's match pool.
    pub fn namespace(&self) -> &AccountId {
        &self.0
    }
}

impl fmt::Display for TeamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub account: AccountId,
    pub role: Role,
    /// How replays identify this member: a display name, or a platform id such
    /// as `steam:7656…` (preferred — it survives renames).
    pub in_game: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    pub id: TeamId,
    /// Never empty and always contains at least one coach (enforced by parsing).
    pub members: Vec<Member>,
}

impl Team {
    pub fn member(&self, account: &AccountId) -> Option<&Member> {
        self.members.iter().find(|m| &m.account == account)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamsError(pub String);

impl fmt::Display for TeamsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "teams file: {}", self.0)
    }
}

impl std::error::Error for TeamsError {}

#[derive(Debug, Clone, Default)]
pub struct Teams(Vec<Team>);

impl Teams {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn from_file(path: &Path) -> Result<Self, TeamsError> {
        let text =
            fs::read_to_string(path).map_err(|e| TeamsError(format!("{}: {e}", path.display())))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, TeamsError> {
        let mut teams: Vec<Team> = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let team =
                parse_team(line).map_err(|e| TeamsError(format!("line {}: {}", i + 1, e.0)))?;
            if teams.iter().any(|t| t.id == team.id) {
                return Err(TeamsError(format!(
                    "line {}: duplicate team {}",
                    i + 1,
                    team.id
                )));
            }
            teams.push(team);
        }
        Ok(Self(teams))
    }

    pub fn get(&self, id: &str) -> Option<&Team> {
        self.0.iter().find(|t| t.id.as_str() == id)
    }

    /// Teams `account` belongs to, with their role in each.
    pub fn for_account<'a>(&'a self, account: &AccountId) -> Vec<(&'a Team, Role)> {
        self.0
            .iter()
            .filter_map(|t| t.member(account).map(|m| (t, m.role)))
            .collect()
    }

    /// Every rostered account must exist; a typo would otherwise be a silent
    /// dead seat. Returns the first unknown account.
    pub fn check_accounts(&self, known: impl Fn(&AccountId) -> bool) -> Result<(), TeamsError> {
        for team in &self.0 {
            if let Some(m) = team.members.iter().find(|m| !known(&m.account)) {
                return Err(TeamsError(format!(
                    "team {}: account {} is not in the accounts file",
                    team.id, m.account
                )));
            }
        }
        Ok(())
    }
}

fn parse_team(line: &str) -> Result<Team, TeamsError> {
    let (name, roster) = line
        .split_once(':')
        .ok_or_else(|| TeamsError("expected `team: role:account, ...`".into()))?;
    let id = TeamId::new(name.trim())?;
    let mut members: Vec<Member> = Vec::new();
    for entry in roster.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let member = parse_member(entry)?;
        if members.iter().any(|m| m.account == member.account) {
            return Err(TeamsError(format!(
                "{} listed twice in {id}",
                member.account
            )));
        }
        if members.iter().any(|m| m.in_game == member.in_game) {
            return Err(TeamsError(format!(
                "in-game name {:?} used twice in {id}",
                member.in_game
            )));
        }
        members.push(member);
    }
    if !members.iter().any(|m| m.role == Role::Coach) {
        return Err(TeamsError(format!("team {id} needs at least one coach")));
    }
    Ok(Team { id, members })
}

fn parse_member(entry: &str) -> Result<Member, TeamsError> {
    let (role, rest) = entry
        .split_once(':')
        .ok_or_else(|| TeamsError(format!("{entry:?}: expected role:account")))?;
    let role = Role::parse(role.trim())
        .ok_or_else(|| TeamsError(format!("{entry:?}: role must be `coach` or `player`")))?;
    let (account, in_game) = match rest.split_once('=') {
        Some((a, n)) => (a.trim(), Some(n.trim())),
        None => (rest.trim(), None),
    };
    let account = AccountId::new(account).map_err(|e| TeamsError(e.to_string()))?;
    let in_game = match in_game {
        Some("") => return Err(TeamsError(format!("{entry:?}: empty in-game name"))),
        Some(n) => n.to_string(),
        None => account.as_str().to_string(),
    };
    Ok(Member {
        account,
        role,
        in_game,
    })
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

/// One roster row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MemberReport {
    pub account: String,
    pub role: Role,
    pub in_game: String,
    pub matches: usize,
    pub wins: usize,
    pub losses: usize,
    pub focus: Option<Focus>,
    pub recurring_fault: Option<RecurringFault>,
}

/// What `GET /api/teams/{team}` returns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TeamReport {
    pub team: String,
    /// The viewer's own role — the UI uses it to explain what it is showing.
    pub viewer_role: Role,
    /// Matches in the shared pool.
    pub sessions: usize,
    /// The roster as one entity: the team's win-vs-loss habits. `None` when no
    /// rostered player appears in any pooled match.
    pub rollup: Option<HabitReport>,
    /// Coaches: every member. Players: only themselves.
    pub members: Vec<MemberReport>,
}

/// Build the report `viewer` is allowed to see, or `None` if they are not on the
/// team. `pool` is the team's shared matches, oldest first.
pub fn team_report(team: &Team, viewer: &AccountId, pool: &[SessionRecord]) -> Option<TeamReport> {
    let viewer_role = team.member(viewer)?.role;
    let members = team
        .members
        .iter()
        .filter(|m| viewer_role == Role::Coach || &m.account == viewer)
        .map(|m| member_report(m, pool))
        .collect();
    Some(TeamReport {
        team: team.id.to_string(),
        viewer_role,
        sessions: pool.len(),
        rollup: rollup(team, pool),
        members,
    })
}

fn member_report(m: &Member, pool: &[SessionRecord]) -> MemberReport {
    let report = habits(&m.in_game, pool);
    MemberReport {
        account: m.account.to_string(),
        role: m.role,
        in_game: m.in_game.clone(),
        matches: report.as_ref().map_or(0, |r| r.matches),
        wins: report.as_ref().map_or(0, |r| r.wins),
        losses: report.as_ref().map_or(0, |r| r.losses),
        focus: report.as_ref().and_then(|r| r.focus.clone()),
        recurring_fault: report.and_then(|r| r.recurring_fault),
    }
}

/// Treat the roster as one player: per pooled match, merge every rostered
/// member's snapshot into one, then reuse the single-player habit analysis.
/// Merging (rather than scoring each member separately) keeps a match counted
/// once even when several members played it.
fn rollup(team: &Team, pool: &[SessionRecord]) -> Option<HabitReport> {
    let names: Vec<&str> = team.members.iter().map(|m| m.in_game.as_str()).collect();
    let label = team.id.as_str();
    let merged: Vec<SessionRecord> = pool
        .iter()
        .filter_map(|r| {
            let mine: Vec<&PlayerSnapshot> = r
                .players
                .iter()
                .filter(|p| names.iter().any(|n| p.matches(n)))
                .collect();
            let first = mine.first()?;
            let values: Vec<f32> = mine.iter().filter_map(|p| p.value).collect();
            Some(SessionRecord {
                key: r.key.clone(),
                label: r.label.clone(),
                saved_at: r.saved_at,
                players: vec![PlayerSnapshot {
                    player: label.to_string(),
                    platform_id: None,
                    team: first.team,
                    won: first.won,
                    value: (!values.is_empty())
                        .then(|| values.iter().sum::<f32>() / values.len() as f32),
                    majors: mine.iter().map(|p| p.majors).sum(),
                    minors: mine.iter().map(|p| p.minors).sum(),
                    top_minor_fault: None,
                    dimensions: mine.iter().flat_map(|p| p.dimensions.clone()).collect(),
                    ..Default::default()
                }],
                ..Default::default()
            })
        })
        .collect();
    habits(label, &merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::DimensionSnapshot;

    const ROSTER: &str = "aces: coach:bailey, player:alice=Schutzein, player:bob";

    fn acct(n: &str) -> AccountId {
        AccountId::new(n).unwrap()
    }

    fn team() -> Team {
        Teams::parse(ROSTER).unwrap().get("aces").unwrap().clone()
    }

    #[test]
    fn parses_roles_and_in_game_names() {
        let t = team();
        assert_eq!(t.members.len(), 3);
        assert_eq!(t.member(&acct("alice")).unwrap().in_game, "Schutzein");
        assert_eq!(
            t.member(&acct("bob")).unwrap().in_game,
            "bob",
            "defaults to account"
        );
        assert_eq!(t.member(&acct("bailey")).unwrap().role, Role::Coach);
    }

    #[test]
    fn rejects_bad_rosters() {
        let cases = [
            ("aces", "expected `team:"),
            ("aces: player:a", "at least one coach"),
            ("aces: coach:a, coach:a", "listed twice"),
            ("aces: coach:a=X, player:b=X", "used twice"),
            ("aces: boss:a", "role must be"),
            ("aces: coach:a=", "empty in-game"),
            ("../x: coach:a", "team name"),
            ("aces: coach:a\naces: coach:b", "duplicate team"),
        ];
        for (text, needle) in cases {
            let e = Teams::parse(text).unwrap_err().to_string();
            assert!(e.contains(needle), "{text:?} -> {e}");
        }
    }

    #[test]
    fn membership_lookup_and_account_check() {
        let teams = Teams::parse(ROSTER).unwrap();
        assert_eq!(teams.for_account(&acct("alice"))[0].1, Role::Player);
        assert!(teams.for_account(&acct("stranger")).is_empty());
        assert!(teams.check_accounts(|_| true).is_ok());
        let e = teams.check_accounts(|a| a.as_str() != "bob").unwrap_err();
        assert!(e.to_string().contains("bob"));
    }

    fn snap(name: &str, won: bool, shadow: f32) -> PlayerSnapshot {
        PlayerSnapshot {
            player: name.into(),
            platform_id: None,
            team: 0,
            won: Some(won),
            value: Some(shadow),
            majors: 0,
            minors: 0,
            top_minor_fault: None,
            dimensions: vec![DimensionSnapshot {
                label: "Shadow".into(),
                value: shadow,
                opportunities: 10,
            }],
            ..Default::default()
        }
    }

    /// 2 wins (Shadow 80) and 2 losses (Shadow 50) with both rostered players
    /// plus an unrostered opponent in every match.
    fn pool() -> Vec<SessionRecord> {
        (0..4)
            .map(|i| {
                let won = i < 2;
                let s = if won { 80.0 } else { 50.0 };
                SessionRecord {
                    key: format!("k{i}"),
                    label: format!("m{i}"),
                    saved_at: i,
                    players: vec![
                        snap("Schutzein", won, s),
                        snap("bob", won, s),
                        snap("Opp", !won, 10.0),
                    ],
                    ..Default::default()
                }
            })
            .collect()
    }

    #[test]
    fn coach_sees_everyone_and_a_rollup_habit() {
        let r = team_report(&team(), &acct("bailey"), &pool()).unwrap();
        assert_eq!(r.viewer_role, Role::Coach);
        assert_eq!(r.sessions, 4);
        assert_eq!(r.members.len(), 3);
        let alice = r.members.iter().find(|m| m.account == "alice").unwrap();
        assert_eq!((alice.matches, alice.wins, alice.losses), (4, 2, 2));
        // The coach has no in-game presence in the pool.
        let coach = r.members.iter().find(|m| m.account == "bailey").unwrap();
        assert_eq!(coach.matches, 0);
        // Rollup counts each match once (4), not once per member (8).
        let roll = r.rollup.unwrap();
        assert_eq!((roll.matches, roll.wins, roll.losses), (4, 2, 2));
        assert_eq!(roll.focus.unwrap().dimension, "Shadow");
    }

    #[test]
    fn player_sees_only_their_own_row() {
        let r = team_report(&team(), &acct("alice"), &pool()).unwrap();
        assert_eq!(r.viewer_role, Role::Player);
        assert_eq!(r.members.len(), 1);
        assert_eq!(r.members[0].account, "alice");
        assert!(r.rollup.is_some(), "the team rollup is visible to players");
    }

    #[test]
    fn non_members_get_nothing() {
        assert!(team_report(&team(), &acct("stranger"), &pool()).is_none());
    }

    #[test]
    fn empty_pool_reports_zeroes_without_a_rollup() {
        let r = team_report(&team(), &acct("bailey"), &[]).unwrap();
        assert_eq!(r.sessions, 0);
        assert!(r.rollup.is_none());
        assert!(r
            .members
            .iter()
            .all(|m| m.matches == 0 && m.focus.is_none()));
    }

    #[test]
    fn a_roster_entry_can_be_a_platform_id_that_survives_renames() {
        let team = Teams::parse("aces: coach:bailey, player:alice=steam:42")
            .unwrap()
            .get("aces")
            .unwrap()
            .clone();
        let mut pool = pool();
        for (i, rec) in pool.iter_mut().enumerate() {
            // Alice plays under a different name every match, same platform id.
            rec.players[0].player = format!("alias{i}");
            rec.players[0].platform_id = Some("steam:42".into());
        }
        let r = team_report(&team, &acct("bailey"), &pool).unwrap();
        let alice = r.members.iter().find(|m| m.account == "alice").unwrap();
        assert_eq!(alice.matches, 4, "all four matches despite four names");
        assert_eq!(r.rollup.unwrap().matches, 4);
    }
}
