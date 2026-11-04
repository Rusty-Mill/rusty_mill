//! The scheduler's memory as plain data, so the shell can store it between
//! runs (#594). [`State::saved`] takes it; [`State::restore`] rebuilds it
//! and re-checks every provider id.
//!
//! Jobs in flight are not kept: no job outlives its process. A job that
//! posted but crashed before its outcome was saved is asked again; the
//! meter has already charged it (see `Meter::restore`).

use std::collections::{BTreeMap, BTreeSet};

use orch_core::AgentId;
use rusty_bbp::MsgId;

use crate::schedule::{Pass, Progress, Proposals, Slot, State};

/// One blind or look-harder pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedPass {
    pub provider: String,
    pub posted: u64,
    pub found_counter: bool,
}

/// One lens's progress on one claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedProgress {
    pub lens: String,
    pub claim: u64,
    pub blind: Option<SavedPass>,
    pub look_harder: Option<SavedPass>,
    pub foreign_seen: u64,
}

/// One question's proposals. A `tried` slot is `Some(n)` for the `n`th
/// ordinary proposer and `None` for the rebel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedQuestion {
    pub claim: u64,
    pub used: Vec<String>,
    pub settled: Vec<u8>,
    pub tried: Vec<(Option<u8>, Vec<String>)>,
    pub rebel_landed: bool,
    pub rebel_failures: u32,
    pub attacks: Vec<(u64, u32)>,
}

/// The scheduler's memory for one task, without its jobs in flight.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
    pub progress: Vec<SavedProgress>,
    pub questions: Vec<SavedQuestion>,
}

impl State {
    /// This memory as plain data, dropping jobs in flight.
    pub fn saved(&self) -> Saved {
        Saved {
            progress: self
                .progress
                .iter()
                .map(|((lens, claim), p)| save_progress(lens, *claim, p))
                .collect(),
            questions: self
                .questions
                .iter()
                .map(|(claim, q)| save_question(*claim, q))
                .collect(),
        }
    }

    /// The memory `saved` was taken of, with nothing in flight.
    ///
    /// # Errors
    ///
    /// A message naming the first invalid provider id, out-of-range count
    /// or repeated entry.
    pub fn restore(saved: &Saved) -> Result<Self, String> {
        let mut state = Self::default();
        for p in &saved.progress {
            let key = (p.lens.clone(), MsgId(p.claim));
            if state.progress.insert(key, restore_progress(p)?).is_some() {
                return Err(format!("progress for {} on msg:{} twice", p.lens, p.claim));
            }
        }
        for q in &saved.questions {
            if state
                .questions
                .insert(MsgId(q.claim), restore_question(q)?)
                .is_some()
            {
                return Err(format!("question msg:{} twice", q.claim));
            }
        }
        Ok(state)
    }
}

fn save_pass(p: &Pass) -> SavedPass {
    SavedPass {
        provider: p.provider.as_str().to_owned(),
        posted: p.posted as u64,
        found_counter: p.found_counter,
    }
}

fn save_progress(lens: &str, claim: MsgId, p: &Progress) -> SavedProgress {
    SavedProgress {
        lens: lens.to_owned(),
        claim: claim.0,
        blind: p.blind.as_ref().map(save_pass),
        look_harder: p.look_harder.as_ref().map(save_pass),
        foreign_seen: p.foreign_seen as u64,
    }
}

fn ids<'a>(list: impl IntoIterator<Item = &'a AgentId>) -> Vec<String> {
    list.into_iter().map(|a| a.as_str().to_owned()).collect()
}

fn save_question(claim: MsgId, q: &Proposals) -> SavedQuestion {
    SavedQuestion {
        claim: claim.0,
        used: ids(&q.used),
        settled: q.settled.iter().copied().collect(),
        tried: q
            .tried
            .iter()
            .filter_map(|(slot, who)| {
                let n = match slot {
                    Slot::Propose(n) => Some(*n),
                    Slot::Rebel => None,
                    Slot::Review | Slot::LookHarder => return None,
                };
                Some((n, ids(who)))
            })
            .collect(),
        rebel_landed: q.rebel_landed,
        rebel_failures: q.rebel_failures,
        attacks: q.attacks.iter().map(|(c, n)| (c.0, *n)).collect(),
    }
}

fn agent(raw: &str) -> Result<AgentId, String> {
    AgentId::new(raw).map_err(|e| format!("provider {raw:?}: {e}"))
}

fn count(raw: u64) -> Result<usize, String> {
    usize::try_from(raw).map_err(|_| format!("count {raw} does not fit"))
}

fn restore_pass(p: &SavedPass) -> Result<Pass, String> {
    Ok(Pass {
        provider: agent(&p.provider)?,
        posted: count(p.posted)?,
        found_counter: p.found_counter,
    })
}

fn restore_progress(p: &SavedProgress) -> Result<Progress, String> {
    Ok(Progress {
        blind: p.blind.as_ref().map(restore_pass).transpose()?,
        look_harder: p.look_harder.as_ref().map(restore_pass).transpose()?,
        foreign_seen: count(p.foreign_seen)?,
    })
}

fn restore_question(q: &SavedQuestion) -> Result<Proposals, String> {
    let mut tried = BTreeMap::new();
    for (n, who) in &q.tried {
        let slot = n.map_or(Slot::Rebel, Slot::Propose);
        let who: BTreeSet<AgentId> = who.iter().map(|a| agent(a)).collect::<Result<_, _>>()?;
        if tried.insert(slot, who).is_some() {
            return Err(format!(
                "question msg:{} lists slot {slot:?} twice",
                q.claim
            ));
        }
    }
    let mut attacks = BTreeMap::new();
    for (c, n) in &q.attacks {
        if attacks.insert(MsgId(*c), *n).is_some() {
            return Err(format!(
                "question msg:{} lists candidate msg:{c} twice",
                q.claim
            ));
        }
    }
    Ok(Proposals {
        used: q.used.iter().map(|a| agent(a)).collect::<Result<_, _>>()?,
        settled: q.settled.iter().copied().collect(),
        running: BTreeMap::new(),
        tried,
        rebel_landed: q.rebel_landed,
        rebel_failures: q.rebel_failures,
        attacks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::JobKey;

    fn id(s: &str) -> AgentId {
        AgentId::new(s).expect("id")
    }

    /// A memory with every field set, one job in flight and one proposal
    /// running.
    fn busy() -> State {
        let mut s = State::default();
        s.progress.insert(
            ("adversary".to_owned(), MsgId(7)),
            Progress {
                blind: Some(Pass {
                    provider: id("claude"),
                    posted: 0,
                    found_counter: false,
                }),
                look_harder: Some(Pass {
                    provider: id("local"),
                    posted: 2,
                    found_counter: true,
                }),
                foreign_seen: 3,
            },
        );
        let mut q = Proposals {
            used: vec![id("claude"), id("codex")],
            settled: [0, 1].into(),
            rebel_landed: false,
            rebel_failures: 1,
            attacks: [(MsgId(11), 2), (MsgId(12), 1)].into(),
            ..Proposals::default()
        };
        q.tried.insert(Slot::Propose(1), [id("local")].into());
        q.tried.insert(Slot::Rebel, [id("codex")].into());
        q.running.insert(Slot::Rebel, id("claude"));
        s.questions.insert(MsgId(3), q);
        s.in_flight.insert(JobKey {
            lens: "architect".to_owned(),
            claim: MsgId(3),
            slot: Slot::Rebel,
        });
        s
    }

    #[test]
    fn a_restored_memory_is_the_saved_one_with_nothing_in_flight() {
        let s = busy();
        let back = State::restore(&s.saved()).expect("restore");
        let mut want = s;
        want.in_flight.clear();
        for q in want.questions.values_mut() {
            q.running.clear();
        }
        assert_eq!(back, want);
        assert!(back.in_flight.is_empty());
    }

    #[test]
    fn an_empty_memory_round_trips() {
        let back = State::restore(&State::default().saved()).expect("restore");
        assert_eq!(back, State::default());
    }

    #[test]
    fn a_bad_provider_id_is_refused() {
        let mut saved = busy().saved();
        saved.questions[0].used.push("Not An Id".to_owned());
        let e = State::restore(&saved).expect_err("bad id");
        assert!(e.contains("Not An Id"), "{e}");
    }

    #[test]
    fn a_repeated_entry_is_refused() {
        let mut saved = busy().saved();
        saved.progress.push(saved.progress[0].clone());
        assert!(State::restore(&saved).is_err(), "progress twice");
        let mut saved = busy().saved();
        saved.questions.push(saved.questions[0].clone());
        assert!(State::restore(&saved).is_err(), "question twice");
        let mut saved = busy().saved();
        let first = saved.questions[0].tried[0].clone();
        saved.questions[0].tried.push(first);
        assert!(State::restore(&saved).is_err(), "slot twice");
        let mut saved = busy().saved();
        saved.questions[0].attacks.push((11, 5));
        assert!(State::restore(&saved).is_err(), "candidate twice");
    }
}
