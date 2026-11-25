//! LEL tab claims map, intent panel and stage strip (card #1416, slice 2).
//!
//! Pure view model over BBP `claim_card` / `claims_delta` entries (the
//! `rusty_bbp::claims::ClaimView` JSON), no UI and no I/O. Unknown fields are
//! ignored; `accept_at`, `human_touched`, `holdout_due` and `intent` default
//! when absent. The stage strip is derived from claim facts only (D55, LEL
//! D189): BBP records no stage.

use super::lel::Error;
use rusty_json::Value;

/// BBP claim status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Open,
    Accepted,
    Dropped,
    Void,
    Escalated,
    Superseded,
}

impl Status {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "Open" => Self::Open,
            "Accepted" => Self::Accepted,
            "Dropped" => Self::Dropped,
            "Void" => Self::Void,
            "Escalated" => Self::Escalated,
            "Superseded" => Self::Superseded,
            _ => return None,
        })
    }

    /// Open, accepted or escalated: still part of the app being built.
    fn live(self) -> bool {
        matches!(self, Self::Open | Self::Accepted | Self::Escalated)
    }
}

/// A claim on the map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapClaim {
    pub id: u64,
    pub text: String,
    pub domains: Vec<String>,
    pub status: Status,
    pub score: i64,
    /// The score this claim accepts at.
    pub accept_at: i64,
    /// Confidence as a whole percent.
    pub confidence: u32,
    pub spent: u64,
    pub budget: u64,
    pub deps: Vec<u64>,
    /// Candidate claims answering this one (a stuck question's options).
    pub candidates: Vec<u64>,
    pub human_touched: bool,
    pub holdout_due: bool,
    /// The intent version this claim was last accepted under, if ever.
    pub proven_at: Option<u32>,
}

/// Claims sharing a feature area (Q40 interim: derived, no BBP field).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Group {
    pub name: String,
    pub claims: Vec<MapClaim>,
}

/// The read-only intent panel (D56).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Intent {
    /// The brief text; `None` until a Mill Board endpoint serves it.
    pub brief: Option<String>,
    /// Intent version: 1 until the intent is first revised.
    pub version: u32,
}

/// The five LEL stages (D55).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Intent,
    Skeletons,
    UnderAttack,
    ShadowRun,
    LivingApp,
}

/// Where a stage stands. Shown as text plus glyph, never animated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StageState {
    Done,
    Current,
    Pending,
}

/// Everything the tab's centre shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Map {
    pub intent: Intent,
    pub stages: [(Stage, StageState); 5],
    pub groups: Vec<Group>,
    /// No claims yet: "new, gathering".
    pub gathering: bool,
}

/// Group name for a claim with neither a domain nor a known dependency.
const UNGROUPED: &str = "other";

/// Parses a `claim_card`/`claims_delta` reply and builds the map.
///
/// `brief` is the task brief text when known. An empty reply is a new task
/// that is still gathering, not an error.
///
/// # Errors
/// [`Error`] on text that is not JSON, not a list, or an entry with a missing
/// or malformed field (`index` is its position): one bad entry refuses the
/// read, so a half-drawn map never hides a claim.
pub(crate) fn parse(text: &str, brief: Option<&str>) -> Result<Map, Error> {
    let value = Value::parse(text).map_err(|e| Error::Json(e.to_string()))?;
    let items = value.as_array().ok_or(Error::NotAnArray)?;
    let mut claims = Vec::with_capacity(items.len());
    let mut version = 1;
    for (i, v) in items.iter().enumerate() {
        let (claim, current) = claim(i, v)?;
        version = version.max(current);
        claims.push(claim);
    }
    Ok(build(claims, brief, version))
}

fn claim(index: usize, v: &Value) -> Result<(MapClaim, u32), Error> {
    let miss = |name| Error::Field { index, name };
    let num = |k: &'static str| v.get(k).and_then(Value::as_u64).ok_or(miss(k));
    let int = |k: &'static str| v.get(k).and_then(Value::as_i64).ok_or(miss(k));
    let ids = |k: &'static str| -> Result<Vec<u64>, Error> {
        v.get(k)
            .and_then(Value::as_array)
            .ok_or(miss(k))?
            .iter()
            .map(|x| x.as_u64().ok_or(miss(k)))
            .collect()
    };
    let flag = |k: &'static str| match v.get(k).filter(|x| !x.is_null()) {
        None => Ok(false),
        Some(x) => x.as_bool().ok_or(miss(k)),
    };
    let small = |k: &'static str| u32::try_from(num(k)?).map_err(|_| miss(k));
    let domains = v
        .get("domains")
        .and_then(Value::as_array)
        .ok_or(miss("domains"))?
        .iter()
        .map(|x| x.as_str().map(str::to_owned).ok_or(miss("domains")))
        .collect::<Result<_, _>>()?;
    let status = v
        .get("status")
        .and_then(Value::as_str)
        .and_then(Status::parse)
        .ok_or(miss("status"))?;
    // Absent until the intent is first revised; present and malformed refuses.
    let (current, proven_at) = match v.get("intent").filter(|x| !x.is_null()) {
        None => (1, None),
        Some(x) => {
            let n = |k| {
                x.get(k)
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
            };
            let proven = match x.get("proven_at").filter(|p| !p.is_null()) {
                None => None,
                Some(_) => Some(n("proven_at").ok_or(miss("intent"))?),
            };
            (n("current").ok_or(miss("intent"))?, proven)
        }
    };
    let claim = MapClaim {
        id: num("id")?,
        text: v
            .get("text")
            .and_then(Value::as_str)
            .ok_or(miss("text"))?
            .to_owned(),
        domains,
        status,
        score: int("score")?,
        accept_at: if v.get("accept_at").is_some() {
            int("accept_at")?
        } else {
            0
        },
        confidence: small("confidence")?,
        spent: num("spent")?,
        budget: num("budget")?,
        deps: ids("deps")?,
        candidates: ids("candidates")?,
        human_touched: flag("human_touched")?,
        holdout_due: flag("holdout_due")?,
        proven_at,
    };
    Ok((claim, current))
}

fn build(claims: Vec<MapClaim>, brief: Option<&str>, version: u32) -> Map {
    let brief = brief
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(str::to_owned);
    Map {
        stages: stages(&claims, brief.is_some()),
        gathering: claims.is_empty(),
        groups: group(claims),
        intent: Intent { brief, version },
    }
}

/// Groups by first domain; a claim with none follows its first known
/// dependency's group, else `other`. First-seen order, claims in id order.
fn group(claims: Vec<MapClaim>) -> Vec<Group> {
    let mut of: Vec<(u64, String)> = Vec::with_capacity(claims.len());
    let mut groups: Vec<Group> = Vec::new();
    for c in claims {
        let name = c
            .domains
            .first()
            .cloned()
            .or_else(|| {
                c.deps
                    .iter()
                    .find_map(|d| of.iter().find(|(id, _)| id == d).map(|(_, n)| n.clone()))
            })
            .unwrap_or_else(|| UNGROUPED.to_owned());
        of.push((c.id, name.clone()));
        match groups.iter_mut().find(|g| g.name == name) {
            Some(g) => g.claims.push(c),
            None => groups.push(Group {
                name,
                claims: vec![c],
            }),
        }
    }
    groups
}

/// The stage rule of LEL D189, over one snapshot.
fn stages(claims: &[MapClaim], has_brief: bool) -> [(Stage, StageState); 5] {
    use StageState::{Current, Done, Pending};
    let live = || claims.iter().filter(|c| c.status.live());
    let candidates = live().any(|c| !c.candidates.is_empty());
    let settled = live().any(|c| c.status == Status::Accepted)
        && !live().any(|c| matches!(c.status, Status::Open | Status::Escalated));
    let intent = match (claims.is_empty(), has_brief) {
        (true, _) => Current,
        (false, true) => Done,
        (false, false) => Pending,
    };
    let skeletons = match (candidates, live().next().is_some()) {
        (true, _) => Done,
        (false, true) => Current,
        (false, false) => Pending,
    };
    let attack = match (settled, candidates) {
        (true, _) => Done,
        (false, true) => Current,
        (false, false) => Pending,
    };
    // Nothing records a shadow run, so it is never done (LEL D189, Q65).
    let shadow = if settled { Current } else { Pending };
    [
        (Stage::Intent, intent),
        (Stage::Skeletons, skeletons),
        (Stage::UnderAttack, attack),
        (Stage::ShadowRun, shadow),
        (Stage::LivingApp, Pending),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use StageState::{Current, Done, Pending};

    const FIXTURE: &str = include_str!("testdata/lel_claims.json");

    fn states(m: &Map) -> Vec<StageState> {
        m.stages.iter().map(|s| s.1).collect()
    }

    fn one(status: &str, candidates: &str) -> String {
        format!(
            r#"[{{"id":1,"text":"t","domains":[],"status":"{status}","score":0,"confidence":50,
            "spent":0,"budget":1,"deps":[],"candidates":{candidates}}}]"#
        )
    }

    #[test]
    fn reads_the_golden_claims() {
        let m = parse(FIXTURE, Some("Build a client")).unwrap();
        let all: Vec<&MapClaim> = m.groups.iter().flat_map(|g| &g.claims).collect();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].status, Status::Accepted);
        assert_eq!(
            (all[0].score, all[0].accept_at, all[0].confidence),
            (12, 8, 95)
        );
        assert!(all[1].human_touched && all[1].holdout_due);
        assert_eq!(all[2].candidates, [4, 5]);
        assert_eq!(all[2].proven_at, Some(2));
        assert_eq!(m.intent.version, 3);
        assert_eq!(m.intent.brief.as_deref(), Some("Build a client"));
        assert!(!m.gathering);
    }

    #[test]
    fn groups_by_domain_then_dependency() {
        let m = parse(FIXTURE, None).unwrap();
        let names: Vec<&str> = m.groups.iter().map(|g| g.name.as_str()).collect();
        // Claim 3 sits under its first domain, `ui`; claim 2 follows dep 1.
        assert_eq!(names, ["net", "ui"]);
        assert_eq!(m.groups[0].claims.len(), 2);
    }

    #[test]
    fn a_claim_with_no_domain_or_known_dep_is_ungrouped() {
        let m = parse(&one("Open", "[]"), None).unwrap();
        assert_eq!(m.groups[0].name, "other");
    }

    #[test]
    fn defaults_when_fields_are_absent() {
        let m = parse(&one("Open", "[]"), None).unwrap();
        let c = &m.groups[0].claims[0];
        assert_eq!(c.accept_at, 0);
        assert!(!c.human_touched && !c.holdout_due);
        assert_eq!((c.proven_at, m.intent.version), (None, 1));
    }

    #[test]
    fn empty_is_new_and_gathering() {
        let m = parse("[]", Some("brief")).unwrap();
        assert!(m.gathering && m.groups.is_empty());
        assert_eq!(states(&m), [Current, Pending, Pending, Pending, Pending]);
    }

    #[test]
    fn a_blank_brief_is_none() {
        assert_eq!(parse("[]", Some("  \n")).unwrap().intent.brief, None);
    }

    #[test]
    fn stage_skeleton_only() {
        let m = parse(&one("Open", "[]"), Some("b")).unwrap();
        assert_eq!(states(&m), [Done, Current, Pending, Pending, Pending]);
    }

    #[test]
    fn stage_mid_attack() {
        let m = parse(&one("Open", "[2]"), Some("b")).unwrap();
        assert_eq!(states(&m), [Done, Done, Current, Pending, Pending]);
    }

    #[test]
    fn an_escalated_claim_blocks_the_attack() {
        let two = format!(
            "[{},{}]",
            one("Accepted", "[]").trim_matches(['[', ']']),
            one("Escalated", "[2]")
                .trim_matches(['[', ']'])
                .replace("\"id\":1", "\"id\":2")
        );
        let m = parse(&two, Some("b")).unwrap();
        assert_eq!(states(&m), [Done, Done, Current, Pending, Pending]);
    }

    #[test]
    fn all_settled_leaves_shadow_run_current_and_never_done() {
        let m = parse(&one("Accepted", "[]"), Some("b")).unwrap();
        assert_eq!(states(&m), [Done, Current, Done, Current, Pending]);
    }

    #[test]
    fn dropped_claims_do_not_count() {
        let m = parse(&one("Dropped", "[2]"), Some("b")).unwrap();
        assert_eq!(states(&m), [Done, Pending, Pending, Pending, Pending]);
    }

    #[test]
    fn claims_without_a_brief_leave_intent_pending() {
        let m = parse(&one("Open", "[]"), None).unwrap();
        assert_eq!(m.stages[0].1, Pending);
    }

    #[test]
    fn errors_are_named() {
        assert!(matches!(parse("nope", None), Err(Error::Json(_))));
        assert_eq!(parse("{}", None), Err(Error::NotAnArray));
        for (bad, name) in [
            (one("Weird", "[]"), "status"),
            (one("Open", "3"), "candidates"),
            (one("Open", r#"["x"]"#), "candidates"),
            (r#"[{"id":1}]"#.to_owned(), "domains"),
        ] {
            assert_eq!(
                parse(&bad, None),
                Err(Error::Field { index: 0, name }),
                "{bad}"
            );
        }
        let bad_intent =
            one("Open", "[]").replace("\"deps\"", "\"intent\":{\"current\":\"x\"},\"deps\"");
        assert_eq!(
            parse(&bad_intent, None),
            Err(Error::Field {
                index: 0,
                name: "intent"
            })
        );
    }
}
