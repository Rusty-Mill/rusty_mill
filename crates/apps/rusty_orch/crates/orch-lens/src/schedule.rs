//! The scheduler: which jobs to run now. Pure: board cards and the runner's
//! own memory in, jobs out. The rules are checks here, not prompt text:
//!
//! - **Blind first.** A lens's first job on a claim is blind. It gets an
//!   informed job only after its blind outcome is recorded (and, when the
//!   blind pass posted, after that post is on the card), and then only when
//!   someone else has added evidence since it last looked.
//! - **Look harder.** A blind pass that found no counter gets exactly one
//!   re-ask, on a different provider. Its answer stands.
//! - **Several candidates, one rebel.** A question gets blind proposals from
//!   the Architect's first two proposers and its rebel before any attack on
//!   its candidates. Attacks wait for both ordinary proposal slots to
//!   settle, for two candidates, and for the rebel's candidate (or two
//!   rebel failures; an empty rebel answer is a failure).
//! - **Single candidate.** While a question has fewer than two candidates,
//!   none of them gets a job: proposing goes on, support does not.
//! - **Fair attacks.** A candidate is attacked only while it has no more
//!   attack passes than its least-attacked open sibling.
//! - **One at a time.** At most one job per (lens, claim, slot) in flight.

use std::collections::{BTreeMap, BTreeSet};

use orch_core::AgentId;
use rusty_bbp::{ClaimStatus, MsgId, PrincipalId};

use crate::card::Card;
use crate::config::{Lens, LensConfig};
use crate::context::Phase;
use crate::post::{Allowed, ClaimRule, EvidenceRule};
use crate::tier::route;

/// Claims carrying this domain are questions: proposers answer them.
pub const QUESTION_DOMAIN: &str = "question";
/// Appended to a look-harder re-ask.
pub const LOOK_HARDER: &str = "You found no counter evidence on your first pass. Look harder: check the failure paths, the limits and the assumptions. If there is still nothing, reply with no posts.";
/// Appended to the rebel proposer's question.
pub const REBEL: &str =
    "You are the rebel proposer: propose an approach that departs from the obvious prior art.";
/// A rebel that failed this often no longer holds attacks back.
pub const REBEL_TRIES: u32 = 2;
/// Ordinary proposal slots per question; config requires this many
/// distinct proposers on a proposing lens.
const ORDINARY: usize = 2;

/// What a job is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slot {
    /// Evidence on a claim, blind or informed.
    Review,
    /// The one look-harder re-ask.
    LookHarder,
    /// A proposal for a question, from the `n`th proposer.
    Propose(u8),
    /// The rebel proposal for a question.
    Rebel,
}

/// One job's identity, for the one-in-flight rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct JobKey {
    pub lens: String,
    pub claim: MsgId,
    pub slot: Slot,
}

/// One model call to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub key: JobKey,
    pub phase: Phase,
    /// Cleared providers in fallback order.
    pub providers: Vec<AgentId>,
    pub allowed: Allowed,
    /// Appended to the context ([`LOOK_HARDER`] or [`REBEL`]).
    pub suffix: Option<&'static str>,
    /// Foreign evidence on the card when the job was made.
    pub foreign_seen: usize,
}

/// A job the rules allow but no provider can take. For a proposal slot
/// this is permanent; the runner passes it to [`State::unservable`] so the
/// question can move on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub key: JobKey,
    pub reason: &'static str,
}

/// How a blind or look-harder pass went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pass {
    pub provider: AgentId,
    pub posted: usize,
    pub found_counter: bool,
}

/// What the runner remembers about one lens on one claim.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub blind: Option<Pass>,
    pub look_harder: Option<Pass>,
    /// Foreign evidence count at the lens's last look.
    pub foreign_seen: usize,
}

/// What the runner remembers about one question.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Proposals {
    /// Providers that proposed a candidate, in order. A fallback never
    /// reuses one.
    pub used: Vec<AgentId>,
    /// Ordinary proposal slots that answered or failed for good.
    pub settled: BTreeSet<u8>,
    /// The provider each in-flight proposal slot runs on.
    pub running: BTreeMap<Slot, AgentId>,
    /// Providers that failed a slot; that slot does not ask them again.
    pub tried: BTreeMap<Slot, BTreeSet<AgentId>>,
    pub rebel_landed: bool,
    pub rebel_failures: u32,
    /// Attack passes per candidate.
    pub attacks: BTreeMap<MsgId, u32>,
}

/// The runner's memory for one task.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    pub progress: BTreeMap<(String, MsgId), Progress>,
    pub questions: BTreeMap<MsgId, Proposals>,
    pub in_flight: BTreeSet<JobKey>,
}

/// The jobs to start now, and those the rules allow but nobody can serve.
pub fn schedule(config: &LensConfig, cards: &[Card], state: &State) -> (Vec<Job>, Vec<Skipped>) {
    let by_id: BTreeMap<MsgId, &Card> = cards.iter().map(|c| (c.id, c)).collect();
    let mut jobs = Vec::new();
    let mut skipped = Vec::new();
    for card in cards.iter().filter(|c| c.status == ClaimStatus::Open) {
        for lens in config
            .lenses
            .iter()
            .filter(|l| card.domains.iter().any(|d| l.follows(d)))
        {
            let wanted = if is_question(card) {
                proposals(config, lens, card, state)
            } else {
                let cleared = route(config, lens, card);
                review(lens, card, &cleared, &by_id, state)
                    .into_iter()
                    .collect()
            };
            for (slot, phase, providers, suffix) in wanted {
                let key = JobKey {
                    lens: lens.id.clone(),
                    claim: card.id,
                    slot,
                };
                if state.in_flight.contains(&key) {
                    continue;
                }
                if providers.is_empty() {
                    skipped.push(Skipped {
                        key,
                        reason: "no cleared provider",
                    });
                    continue;
                }
                jobs.push(Job {
                    allowed: allowed(lens, card, slot),
                    foreign_seen: foreign(card, &lens.id),
                    key,
                    phase,
                    providers,
                    suffix,
                });
            }
        }
    }
    (jobs, skipped)
}

type Wanted = (Slot, Phase, Vec<AgentId>, Option<&'static str>);

/// Is `card` a question for proposers?
pub fn is_question(card: &Card) -> bool {
    card.answers.is_none() && card.domains.iter().any(|d| d == QUESTION_DOMAIN)
}

fn cleared_for(config: &LensConfig, card: &Card, p: AgentId) -> bool {
    config
        .providers
        .get(&p)
        .is_some_and(|prov| crate::class::admit(card.class, prov.max_class))
}

/// Proposal jobs a lens owes a question: its first two proposers and its
/// rebel, each slot once. Each job gets exactly one provider, so no two
/// proposals of a question ever share a model: none a provider that already
/// proposed a candidate, none a provider another slot is running on or was
/// just given. A slot that only waits on a running provider gets no job; a
/// slot no unused cleared provider can ever serve gets an empty provider
/// list, which [`schedule`] reports as [`Skipped`].
fn proposals(config: &LensConfig, lens: &Lens, question: &Card, state: &State) -> Vec<Wanted> {
    if !lens.claims || lens.proposers.is_empty() {
        return Vec::new();
    }
    let seen = state
        .questions
        .get(&question.id)
        .cloned()
        .unwrap_or_default();
    let mut slots: Vec<(Slot, AgentId, Option<&'static str>)> = (0u8..)
        .zip(lens.proposers.iter().take(ORDINARY))
        .filter(|(n, _)| !seen.settled.contains(n))
        .map(|(n, want)| (Slot::Propose(n), *want, None))
        .collect();
    if let Some(rebel) = lens.rebel {
        if !seen.rebel_landed && seen.rebel_failures < REBEL_TRIES {
            slots.push((Slot::Rebel, rebel, Some(REBEL)));
        }
    }
    let mut taken: BTreeSet<AgentId> = seen.running.values().copied().collect();
    let none = BTreeSet::new();
    let mut wanted = Vec::new();
    for (slot, want, suffix) in slots {
        if seen.running.contains_key(&slot) {
            continue;
        }
        let eligible: Vec<AgentId> = std::iter::once(want)
            .chain(lens.proposers.iter().copied())
            .chain(lens.rebel)
            .filter(|p| {
                !seen.used.contains(p)
                    && !seen.tried.get(&slot).unwrap_or(&none).contains(p)
                    && cleared_for(config, question, *p)
            })
            .collect();
        if eligible.is_empty() {
            wanted.push((slot, Phase::Blind, Vec::new(), suffix));
            continue;
        }
        let Some(pick) = eligible.into_iter().find(|p| !taken.contains(p)) else {
            continue;
        };
        taken.insert(pick);
        wanted.push((slot, Phase::Blind, vec![pick], suffix));
    }
    wanted
}

/// The one review job, if any, a lens owes a claim, over the claim's
/// cleared providers for its tier.
fn review(
    lens: &Lens,
    card: &Card,
    cleared: &[AgentId],
    by_id: &BTreeMap<MsgId, &Card>,
    state: &State,
) -> Option<Wanted> {
    if lens.evidence.is_empty() {
        return None;
    }
    if let Some(q) = card.answers {
        if !attack_ready(card, q, by_id, state) {
            return None;
        }
    }
    let me = PrincipalId(lens.id.clone());
    let p = state
        .progress
        .get(&(lens.id.clone(), card.id))
        .cloned()
        .unwrap_or_default();
    let Some(blind) = p.blind else {
        return Some((Slot::Review, Phase::Blind, cleared.to_vec(), None));
    };
    let others: Vec<AgentId> = cleared
        .iter()
        .copied()
        .filter(|m| *m != blind.provider)
        .collect();
    // With no other cleared provider the re-ask is skipped, not repeated
    // on the same model.
    if !blind.found_counter && p.look_harder.is_none() && !others.is_empty() {
        return Some((Slot::LookHarder, Phase::Blind, others, Some(LOOK_HARDER)));
    }
    let posted = blind.posted + p.look_harder.map_or(0, |l| l.posted);
    if posted > 0 && !card.has_evidence_by(&me) {
        return None;
    }
    (foreign(card, &lens.id) > p.foreign_seen)
        .then(|| (Slot::Review, Phase::Informed, cleared.to_vec(), None))
}

/// May candidate `card` of question `q` be attacked now?
fn attack_ready(card: &Card, q: MsgId, by_id: &BTreeMap<MsgId, &Card>, state: &State) -> bool {
    let Some(question) = by_id.get(&q) else {
        return false;
    };
    let seen = state.questions.get(&q).cloned().unwrap_or_default();
    let rebel_settled = seen.rebel_landed || seen.rebel_failures >= REBEL_TRIES;
    if seen.settled.len() < ORDINARY {
        return false;
    }
    let open: Vec<MsgId> = question
        .candidates
        .iter()
        .copied()
        .filter(|c| by_id.get(c).is_some_and(|c| c.status == ClaimStatus::Open))
        .collect();
    if question.candidates.len() < 2 || !rebel_settled {
        return false;
    }
    let passes = |c: &MsgId| seen.attacks.get(c).copied().unwrap_or(0);
    let least = open.iter().map(passes).min().unwrap_or(0);
    passes(&card.id) <= least
}

fn foreign(card: &Card, lens: &str) -> usize {
    card.evidence.iter().filter(|i| i.author.0 != lens).count()
}

fn allowed(lens: &Lens, card: &Card, slot: Slot) -> Allowed {
    match slot {
        Slot::Propose(_) | Slot::Rebel => Allowed {
            evidence: None,
            claims: Some(ClaimRule {
                answers: Some(card.id),
            }),
        },
        Slot::Review | Slot::LookHarder => Allowed {
            evidence: Some(EvidenceRule {
                claim: card.id,
                directions: lens.evidence.clone(),
            }),
            claims: lens.claims.then_some(ClaimRule { answers: None }),
        },
    }
}

impl State {
    /// Mark a job started.
    pub fn start(&mut self, job: &Job) {
        self.in_flight.insert(job.key.clone());
        if let (Slot::Propose(_) | Slot::Rebel, Some(p)) = (job.key.slot, job.providers.first()) {
            self.questions
                .entry(job.key.claim)
                .or_default()
                .running
                .insert(job.key.slot, *p);
        }
    }

    /// Settle a proposal slot that no unused cleared provider can serve:
    /// an ordinary slot settles without a candidate, the rebel stops trying.
    pub fn unservable(&mut self, skipped: &Skipped) {
        let q = self.questions.entry(skipped.key.claim).or_default();
        match skipped.key.slot {
            Slot::Propose(n) => {
                q.settled.insert(n);
            }
            Slot::Rebel => q.rebel_failures = q.rebel_failures.max(REBEL_TRIES),
            Slot::Review | Slot::LookHarder => {}
        }
    }

    /// Record a job that got an answer from `provider` with `posts` posts,
    /// `counters` of them counter evidence.
    pub fn answered(&mut self, job: &Job, provider: AgentId, posts: usize, counters: usize) {
        self.in_flight.remove(&job.key);
        let key = &job.key;
        let pass = Pass {
            provider,
            posted: posts,
            found_counter: counters > 0,
        };
        match key.slot {
            Slot::Review | Slot::LookHarder => {
                let p = self
                    .progress
                    .entry((key.lens.clone(), key.claim))
                    .or_default();
                match (key.slot, job.phase) {
                    (Slot::LookHarder, _) => p.look_harder = Some(pass),
                    (_, Phase::Blind) => p.blind = Some(pass),
                    (_, Phase::Informed) => {}
                }
                p.foreign_seen = job.foreign_seen;
            }
            Slot::Propose(n) => {
                let q = self.questions.entry(key.claim).or_default();
                q.running.remove(&key.slot);
                q.settled.insert(n);
                if posts > 0 {
                    q.used.push(provider);
                }
            }
            Slot::Rebel => {
                let q = self.questions.entry(key.claim).or_default();
                q.running.remove(&key.slot);
                if posts > 0 {
                    q.used.push(provider);
                    q.rebel_landed = true;
                } else {
                    // No candidate: a failed try, retried within REBEL_TRIES.
                    q.rebel_failures += 1;
                }
            }
        }
    }

    /// Record a job that no provider could answer. A proposal slot does
    /// not ask that provider again: an ordinary slot moves to an unused
    /// provider and settles only once none is left (see
    /// [`State::unservable`]); the rebel also spends one of its tries.
    pub fn failed(&mut self, job: &Job) {
        self.in_flight.remove(&job.key);
        if matches!(job.key.slot, Slot::Review | Slot::LookHarder) {
            return;
        }
        let q = self.questions.entry(job.key.claim).or_default();
        q.running.remove(&job.key.slot);
        q.tried
            .entry(job.key.slot)
            .or_default()
            .extend(job.providers.iter().copied());
        if job.key.slot == Slot::Rebel {
            q.rebel_failures += 1;
        }
    }

    /// Count an attack pass on a candidate of `question`.
    pub fn attacked(&mut self, question: MsgId, candidate: MsgId) {
        *self
            .questions
            .entry(question)
            .or_default()
            .attacks
            .entry(candidate)
            .or_default() += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::fixtures::{card, item};
    use crate::class::DataClass;
    use crate::config::tests::STARTER;
    use rusty_bbp::Direction;

    fn config() -> LensConfig {
        LensConfig::parse(STARTER).expect("config")
    }

    fn id(s: &str) -> AgentId {
        AgentId::new(s).expect("id")
    }

    fn plain(n: u64) -> Card {
        let mut c = card(vec![]);
        c.id = MsgId(n);
        c.answers = None;
        c
    }

    fn jobs_for<'a>(jobs: &'a [Job], lens: &str) -> Vec<&'a Job> {
        jobs.iter().filter(|j| j.key.lens == lens).collect()
    }

    #[test]
    fn every_reviewing_lens_starts_blind() {
        let (jobs, skipped) = schedule(&config(), &[plain(7)], &State::default());
        assert!(skipped.is_empty());
        let lenses: BTreeSet<&str> = jobs.iter().map(|j| j.key.lens.as_str()).collect();
        assert_eq!(lenses, ["adversary", "operator", "research"].into());
        assert!(jobs
            .iter()
            .all(|j| j.phase == Phase::Blind && j.key.slot == Slot::Review));
        let adv = jobs_for(&jobs, "adversary")[0];
        assert_eq!(adv.providers, [id("codex"), id("local")]);
        assert!(adv.allowed.claims.is_none());
    }

    #[test]
    fn a_job_in_flight_is_not_repeated() {
        let cfg = config();
        let (jobs, _) = schedule(&cfg, &[plain(7)], &State::default());
        let mut st = State::default();
        for j in &jobs {
            st.start(j);
        }
        assert!(schedule(&cfg, &[plain(7)], &st).0.is_empty());
    }

    fn after_blind(found_counter: bool, posted: usize) -> (State, Job) {
        let (jobs, _) = schedule(&config(), &[plain(7)], &State::default());
        let job = jobs_for(&jobs, "adversary")[0].clone();
        let mut st = State::default();
        st.start(&job);
        st.answered(&job, id("codex"), posted, usize::from(found_counter));
        (st, job)
    }

    #[test]
    fn nothing_found_gets_one_look_harder_on_another_provider() {
        let (mut st, _) = after_blind(false, 0);
        let (jobs, _) = schedule(&config(), &[plain(7)], &st);
        let again = jobs_for(&jobs, "adversary");
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].key.slot, Slot::LookHarder);
        assert_eq!(again[0].phase, Phase::Blind);
        assert!(!again[0].providers.contains(&id("codex")));
        assert_eq!(again[0].suffix, Some(LOOK_HARDER));
        let j = again[0].clone();
        st.start(&j);
        st.answered(&j, id("local"), 0, 0);
        let (jobs, _) = schedule(&config(), &[plain(7)], &st);
        assert!(
            jobs_for(&jobs, "adversary").is_empty(),
            "never a second re-ask"
        );
    }

    #[test]
    fn a_counter_found_gets_no_re_ask_and_informed_waits_for_the_post() {
        let (st, _) = after_blind(true, 1);
        let mut c = plain(7);
        c.evidence = vec![item(
            20,
            "operator",
            Direction::Support,
            DataClass::Internal,
        )];
        let (jobs, _) = schedule(&config(), &[c.clone()], &st);
        assert!(
            jobs_for(&jobs, "adversary").is_empty(),
            "the blind post is not on the card yet"
        );
        c.evidence.push(item(
            21,
            "adversary",
            Direction::Counter,
            DataClass::Internal,
        ));
        let (jobs, _) = schedule(&config(), &[c], &st);
        let informed = jobs_for(&jobs, "adversary");
        assert_eq!(informed.len(), 1);
        assert_eq!(informed[0].phase, Phase::Informed);
    }

    #[test]
    fn informed_runs_only_on_new_foreign_evidence() {
        let (mut st, _) = after_blind(true, 1);
        let mut c = plain(7);
        c.evidence = vec![item(
            21,
            "adversary",
            Direction::Counter,
            DataClass::Internal,
        )];
        let (jobs, _) = schedule(&config(), &[c.clone()], &st);
        assert!(
            jobs_for(&jobs, "adversary").is_empty(),
            "nothing new from others"
        );
        c.evidence.push(item(
            22,
            "operator",
            Direction::Support,
            DataClass::Internal,
        ));
        let (jobs, _) = schedule(&config(), &[c.clone()], &st);
        let j = jobs_for(&jobs, "adversary")[0].clone();
        st.start(&j);
        st.answered(&j, id("codex"), 0, 0);
        let (jobs, _) = schedule(&config(), &[c], &st);
        assert!(jobs_for(&jobs, "adversary").is_empty(), "seen it already");
    }

    fn question(n: u64, candidates: &[u64]) -> Card {
        let mut q = plain(n);
        q.domains = vec![QUESTION_DOMAIN.to_owned()];
        q.candidates = candidates.iter().copied().map(MsgId).collect();
        q
    }

    fn candidate(n: u64, q: u64) -> Card {
        let mut c = plain(n);
        c.answers = Some(MsgId(q));
        c
    }

    #[test]
    fn a_question_gets_two_proposers_and_the_rebel_blind() {
        let (jobs, _) = schedule(&config(), &[question(3, &[])], &State::default());
        let arch = jobs_for(&jobs, "architect");
        let slots: Vec<Slot> = arch.iter().map(|j| j.key.slot).collect();
        assert_eq!(slots, [Slot::Propose(0), Slot::Propose(1), Slot::Rebel]);
        assert_eq!(arch[0].providers[0], id("claude"));
        assert_eq!(arch[1].providers[0], id("local"));
        assert_eq!(arch[2].providers, [id("codex")]);
        assert!(arch.iter().all(|j| j.providers.len() == 1), "no fallback");
        assert_eq!(arch[2].suffix, Some(REBEL));
        assert!(arch.iter().all(|j| j.allowed.claims
            == Some(ClaimRule {
                answers: Some(MsgId(3))
            })));
        assert!(
            jobs.iter().all(|j| j.key.lens == "architect"),
            "nobody reviews a question"
        );
    }

    #[test]
    fn a_proposal_fallback_never_reuses_a_model() {
        let mut st = State::default();
        let q = st.questions.entry(MsgId(3)).or_default();
        q.used = vec![id("claude")];
        q.settled = [0].into();
        let (jobs, _) = schedule(&config(), &[question(3, &[10])], &st);
        let arch = jobs_for(&jobs, "architect");
        let slots: Vec<Slot> = arch.iter().map(|j| j.key.slot).collect();
        assert_eq!(slots, [Slot::Propose(1), Slot::Rebel]);
        assert!(arch.iter().all(|j| !j.providers.contains(&id("claude"))));
    }

    #[test]
    fn candidates_wait_for_two_and_the_rebel() {
        let cfg = config();
        let mut st = State::default();
        let one = [question(3, &[10]), candidate(10, 3)];
        let (jobs, _) = schedule(&cfg, &one, &st);
        assert!(
            jobs.iter().all(|j| j.key.claim == MsgId(3)),
            "a sole candidate gets no job"
        );

        let two = [question(3, &[10, 11]), candidate(10, 3), candidate(11, 3)];
        let (jobs, _) = schedule(&cfg, &two, &st);
        assert!(
            jobs.iter().all(|j| j.key.claim == MsgId(3)),
            "the rebel has not landed"
        );

        let q = st.questions.entry(MsgId(3)).or_default();
        q.rebel_failures = REBEL_TRIES;
        q.settled = [0].into();
        let (jobs, _) = schedule(&cfg, &two, &st);
        assert!(
            jobs.iter().all(|j| j.key.claim == MsgId(3)),
            "ordinary slot 1 has not settled"
        );

        st.questions.entry(MsgId(3)).or_default().settled = [0, 1].into();
        let (jobs, _) = schedule(&cfg, &two, &st);
        assert!(jobs.iter().any(|j| j.key.claim == MsgId(10)));
        assert!(jobs.iter().any(|j| j.key.claim == MsgId(11)));
    }

    fn arch_jobs(cfg: &LensConfig, cards: &[Card], st: &State) -> Vec<Job> {
        schedule(cfg, cards, st)
            .0
            .into_iter()
            .filter(|j| j.key.lens == "architect")
            .collect()
    }

    fn slot_job(jobs: &[Job], slot: Slot) -> Job {
        jobs.iter()
            .find(|j| j.key.slot == slot)
            .cloned()
            .expect("job for slot")
    }

    #[test]
    fn an_answered_slot_is_not_asked_again_out_of_order() {
        let cfg = config();
        let q = [question(3, &[])];
        let mut st = State::default();
        let jobs = arch_jobs(&cfg, &q, &st);
        for j in &jobs {
            st.start(j);
        }
        st.answered(&slot_job(&jobs, Slot::Propose(1)), id("local"), 1, 0);
        assert!(
            arch_jobs(&cfg, &q, &st).is_empty(),
            "slot 1 answered; slot 0 and the rebel are still running"
        );
        st.answered(&slot_job(&jobs, Slot::Rebel), id("codex"), 1, 0);
        st.failed(&slot_job(&jobs, Slot::Propose(0)));
        assert!(arch_jobs(&cfg, &q, &st).is_empty(), "every slot settled");
    }

    #[test]
    fn attacks_wait_for_the_second_ordinary_proposer() {
        let cfg = config();
        let mut st = State::default();
        let jobs = arch_jobs(&cfg, &[question(3, &[])], &st);
        st.answered(&slot_job(&jobs, Slot::Propose(0)), id("claude"), 1, 0);
        st.answered(&slot_job(&jobs, Slot::Rebel), id("codex"), 1, 0);
        let two = [question(3, &[10, 11]), candidate(10, 3), candidate(11, 3)];
        let (jobs, _) = schedule(&cfg, &two, &st);
        assert!(
            jobs.iter().all(|j| j.key.claim == MsgId(3)),
            "ordinary 1 is still out"
        );
        st.answered(
            &slot_job(&arch_jobs(&cfg, &two, &st), Slot::Propose(1)),
            id("local"),
            1,
            0,
        );
        let (jobs, _) = schedule(&cfg, &two, &st);
        assert!(jobs.iter().any(|j| j.key.claim == MsgId(10)));
    }

    #[test]
    fn an_empty_rebel_answer_is_a_failed_try_and_cannot_strand_the_question() {
        let cfg = config();
        let mut st = State::default();
        let q = [question(3, &[])];
        let jobs = arch_jobs(&cfg, &q, &st);
        st.answered(&slot_job(&jobs, Slot::Propose(0)), id("claude"), 1, 0);
        st.answered(&slot_job(&jobs, Slot::Propose(1)), id("local"), 1, 0);
        st.answered(&slot_job(&jobs, Slot::Rebel), id("codex"), 0, 0);
        let seen = &st.questions[&MsgId(3)];
        assert_eq!(seen.rebel_failures, 1);
        assert!(!seen.used.contains(&id("codex")), "no candidate, no use");

        let retry = slot_job(&arch_jobs(&cfg, &q, &st), Slot::Rebel);
        assert_eq!(retry.providers[0], id("codex"));
        st.answered(&retry, id("codex"), 0, 0);
        let two = [question(3, &[10, 11]), candidate(10, 3), candidate(11, 3)];
        let (jobs, _) = schedule(&cfg, &two, &st);
        assert!(jobs.iter().any(|j| j.key.claim == MsgId(10)));
        assert!(jobs.iter().any(|j| j.key.claim == MsgId(11)));
    }

    #[test]
    fn a_failed_ordinary_slot_moves_on_then_settles() {
        let cfg = config();
        let q = [question(3, &[])];
        let mut st = State::default();
        let jobs = arch_jobs(&cfg, &q, &st);
        for j in &jobs {
            st.start(j);
        }
        // local fails slot 1; claude (slot 0) and codex (rebel) still run.
        st.failed(&slot_job(&jobs, Slot::Propose(1)));
        assert!(
            arch_jobs(&cfg, &q, &st).is_empty(),
            "every other model is busy"
        );

        // claude proposes, so it is used; the rebel fails on codex.
        st.answered(&slot_job(&jobs, Slot::Propose(0)), id("claude"), 1, 0);
        st.failed(&slot_job(&jobs, Slot::Rebel));
        let retry = slot_job(&arch_jobs(&cfg, &q, &st), Slot::Propose(1));
        assert_eq!(retry.providers, [id("codex")], "an untried, unused model");

        st.start(&retry);
        st.failed(&retry);
        let (jobs, skipped) = schedule(&cfg, &q, &st);
        assert!(!jobs.iter().any(|j| j.key.slot == Slot::Propose(1)));
        let slot1 = skipped
            .iter()
            .find(|s| s.key.slot == Slot::Propose(1))
            .expect("slot 1 exhausted");
        st.unservable(slot1);
        assert!(st.questions[&MsgId(3)].settled.contains(&1));
    }

    #[test]
    fn concurrent_proposals_never_share_a_model() {
        let cfg = config();
        let mut q = question(3, &[]);
        q.class = DataClass::Sensitive;
        let mut st = State::default();
        let (jobs, skipped) = schedule(&cfg, std::slice::from_ref(&q), &st);
        let arch: Vec<&Job> = jobs_for(&jobs, "architect");
        assert_eq!(arch.len(), 1, "only local is cleared; the rest wait");
        assert_eq!(arch[0].providers, [id("local")]);
        assert!(skipped.is_empty(), "waiting is not unservable");
        st.start(arch[0]);
        let (again, skipped) = schedule(&cfg, std::slice::from_ref(&q), &st);
        assert!(jobs_for(&again, "architect").is_empty());
        assert!(skipped.is_empty());

        // Local proposes a candidate: no unused cleared provider is left.
        st.answered(arch[0], id("local"), 1, 0);
        let (again, skipped) = schedule(&cfg, std::slice::from_ref(&q), &st);
        assert!(jobs_for(&again, "architect").is_empty());
        let slots: Vec<Slot> = skipped.iter().map(|s| s.key.slot).collect();
        assert_eq!(slots, [Slot::Propose(1), Slot::Rebel]);
        for s in &skipped {
            st.unservable(s);
        }
        let (again, skipped) = schedule(&cfg, std::slice::from_ref(&q), &st);
        assert!(again.is_empty() && skipped.is_empty(), "terminal");
        let seen = &st.questions[&MsgId(3)];
        assert_eq!(seen.settled, [0, 1].into());
        assert_eq!(seen.rebel_failures, REBEL_TRIES);
    }

    #[test]
    fn unservable_slots_open_attacks_on_two_sole_provider_candidates() {
        let cfg = config();
        let sensitive = |mut c: Card| {
            c.class = DataClass::Sensitive;
            c
        };
        let mut st = State::default();
        let q = [sensitive(question(3, &[]))];
        let first = arch_jobs(&cfg, &q, &st);
        st.start(&first[0]);
        st.answered(&first[0], id("local"), 2, 0);
        let board = [
            sensitive(question(3, &[10, 11])),
            sensitive(candidate(10, 3)),
            sensitive(candidate(11, 3)),
        ];
        let (jobs, skipped) = schedule(&cfg, &board, &st);
        assert!(jobs.iter().all(|j| j.key.claim == MsgId(3)), "gate shut");
        for s in &skipped {
            st.unservable(s);
        }
        let (jobs, _) = schedule(&cfg, &board, &st);
        assert!(jobs.iter().any(|j| j.key.claim == MsgId(10)));
        assert!(jobs.iter().any(|j| j.key.claim == MsgId(11)));
    }

    #[test]
    fn attacks_stay_within_one_pass_of_each_other() {
        let cfg = config();
        let mut st = State::default();
        let q = st.questions.entry(MsgId(3)).or_default();
        q.rebel_landed = true;
        q.settled = [0, 1].into();
        st.attacked(MsgId(3), MsgId(10));
        let two = [question(3, &[10, 11]), candidate(10, 3), candidate(11, 3)];
        let (jobs, _) = schedule(&cfg, &two, &st);
        assert!(
            !jobs.iter().any(|j| j.key.claim == MsgId(10)),
            "10 is ahead"
        );
        assert!(jobs.iter().any(|j| j.key.claim == MsgId(11)));
        st.attacked(MsgId(3), MsgId(11));
        let (jobs, _) = schedule(&cfg, &two, &st);
        assert!(jobs.iter().any(|j| j.key.claim == MsgId(10)));
    }

    #[test]
    fn a_sensitive_claim_without_a_cleared_provider_is_skipped() {
        let mut c = plain(7);
        c.class = DataClass::Sensitive;
        let (jobs, skipped) = schedule(&config(), &[c], &State::default());
        assert!(jobs
            .iter()
            .all(|j| j.providers.iter().all(|p| p.as_str().starts_with("local"))));
        assert!(skipped
            .iter()
            .any(|s| s.key.lens == "research" && s.reason == "no cleared provider"));
    }

    #[test]
    fn closed_claims_get_nothing() {
        let mut c = plain(7);
        c.status = ClaimStatus::Accepted;
        assert!(schedule(&config(), &[c], &State::default()).0.is_empty());
    }
}
