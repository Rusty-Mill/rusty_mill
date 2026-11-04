//! One runner round: schedule the board's cards, call each job and post
//! what comes back. Pure over three ports the shell supplies: [`Models`],
//! the meter's save, and the [`Board`].
//!
//! - Jobs run one at a time, in schedule order; the next round sees what
//!   this one posted.
//! - An answer is posted as its lens, then recorded; an attack on a
//!   candidate also counts a pass for the fairness rule.
//! - A provider's failure (`Failed`, `Exhausted`) is recorded and the round
//!   goes on; the rules decide what to ask next.
//! - The runner's own failures end the round and record nothing, so the
//!   rules ask again once fixed: a task ceiling, a meter that could not be
//!   saved, a board that refused a post, a lens with no prefix. An
//!   experiment stop only drops that job: free providers go on.
//! - A proposal slot that no cleared provider can serve is settled.
//! - The run [`Policy`] picks which scheduled jobs run, and in what order;
//!   a shadow policy's pick for the same jobs is reported, not run (#596).

use std::collections::BTreeMap;

use rusty_bbp::{Direction, MsgId};

use crate::call::{call, Ctx, End, Models};
use crate::card::Card;
use crate::config::LensConfig;
use crate::context::{Line, Parts};
use crate::meter::Stop;
use crate::policy::Policy;
use crate::post::Post;
use crate::schedule::{schedule, Job, JobKey, Skipped, Slot, State};

/// Where answers go: the shared board.
pub trait Board {
    /// Post `posts` as lens `lens`, with its credential.
    fn post(&mut self, lens: &str, posts: &[Post]) -> Result<(), String>;
}

/// What the shell read for this round.
#[derive(Debug, Clone, Copy)]
pub struct View<'a> {
    pub cards: &'a [Card],
    /// Settled and related claims, newest first.
    pub lines: &'a [Line],
    /// Each lens's prefix ([`crate::context::prefix`]), by lens id.
    pub prefixes: &'a BTreeMap<String, String>,
    /// Each lens's track-record line, by lens id; a lens without one gets
    /// none.
    pub records: &'a BTreeMap<String, String>,
}

/// The policy a round runs, and one it only reports (#596 Phase A).
pub struct Policies<'a> {
    pub run: &'a mut dyn Policy,
    pub shadow: Option<&'a mut dyn Policy>,
}

/// Why a round ended early. Nothing is recorded for the job it ended on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halt {
    /// A task ceiling: the task pauses.
    Task(Stop),
    /// The meter could not be saved; the shell should stop.
    Unsaved(String),
    /// The board refused a post.
    Board(String),
    /// The shell gave no prefix for this lens.
    NoPrefix(String),
}

/// What a round did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Each job called, and how its call ended.
    pub ran: Vec<(JobKey, End)>,
    /// Jobs the rules allow but no provider can take.
    pub skipped: Vec<Skipped>,
    /// The jobs the shadow policy would have run, in its order.
    pub shadow: Option<Vec<JobKey>>,
    pub halt: Option<Halt>,
}

/// Run one round over `view`, updating `state` and the meter in `ctx`.
pub fn round(
    config: &LensConfig,
    view: &View<'_>,
    policies: Policies<'_>,
    state: &mut State,
    ctx: Ctx<'_>,
    models: &mut dyn Models,
    board: &mut dyn Board,
) -> Report {
    let Ctx {
        task,
        providers,
        meter,
        save,
    } = ctx;
    let (jobs, skipped) = schedule(config, view.cards, state);
    for s in &skipped {
        state.unservable(s);
    }
    let shadow = policies.shadow.map(|p| {
        keys(only_scheduled(
            &jobs,
            p.pick(view.cards, state, jobs.clone()),
        ))
    });
    let jobs = only_scheduled(&jobs, policies.run.pick(view.cards, state, jobs.clone()));
    let by_id: BTreeMap<MsgId, &Card> = view.cards.iter().map(|c| (c.id, c)).collect();
    let mut ran = Vec::new();
    for job in &jobs {
        let Some(card) = by_id.get(&job.key.claim) else {
            continue;
        };
        let Some(prefix) = view.prefixes.get(&job.key.lens) else {
            return report(ran, skipped, shadow, Halt::NoPrefix(job.key.lens.clone()));
        };
        let lens = rusty_bbp::PrincipalId(job.key.lens.clone());
        let parts = Parts {
            prefix,
            card,
            lens: &lens,
            phase: job.phase,
            lines: view.lines,
            record: view.records.get(&job.key.lens).map(String::as_str),
            suffix: job.suffix,
        };
        state.start(job);
        let ctx = Ctx {
            task,
            providers,
            meter: &mut *meter,
            save: &mut *save,
        };
        let called = call(job, &parts, ctx, models);
        let halt = settle(job, card, &called.end, state, board)
            .or_else(|| called.unsaved.clone().map(Halt::Unsaved));
        ran.push((job.key.clone(), called.end));
        if let Some(h) = halt {
            return report(ran, skipped, shadow, h);
        }
    }
    Report {
        ran,
        skipped,
        shadow,
        halt: None,
    }
}

/// The scheduler's own job for each picked key, in the policy's order and
/// each once. Only the key of a pick is read, so a policy cannot make a
/// job the rules did not, nor change one they did (its phase, providers,
/// tools, allowed posts or suffix).
fn only_scheduled(scheduled: &[Job], picked: Vec<Job>) -> Vec<Job> {
    let mut by_key: BTreeMap<&JobKey, &Job> = scheduled.iter().map(|j| (&j.key, j)).collect();
    picked
        .iter()
        .filter_map(|p| by_key.remove(&p.key))
        .cloned()
        .collect()
}

fn keys(jobs: Vec<Job>) -> Vec<JobKey> {
    jobs.into_iter().map(|j| j.key).collect()
}

fn report(
    ran: Vec<(JobKey, End)>,
    skipped: Vec<Skipped>,
    shadow: Option<Vec<JobKey>>,
    halt: Halt,
) -> Report {
    Report {
        ran,
        skipped,
        shadow,
        halt: Some(halt),
    }
}

/// Post and record one job's end; `Some` ends the round.
fn settle(
    job: &Job,
    card: &Card,
    end: &End,
    state: &mut State,
    board: &mut dyn Board,
) -> Option<Halt> {
    match end {
        End::Answered { provider, posts } => {
            if let Err(e) = post(board, &job.key.lens, posts) {
                state.abandon(job);
                return Some(Halt::Board(e));
            }
            state.answered(job, *provider, posts.len(), counters(posts));
            if let (Slot::Review | Slot::LookHarder, Some(q)) = (job.key.slot, card.answers) {
                state.attacked(q, card.id);
            }
            None
        }
        End::Failed { .. } | End::Exhausted => {
            state.failed(job);
            None
        }
        End::Stopped(Stop::Experiment) => {
            state.abandon(job);
            None
        }
        End::Stopped(stop) => {
            state.abandon(job);
            Some(Halt::Task(*stop))
        }
        End::Unsaved(e) => {
            state.abandon(job);
            Some(Halt::Unsaved(e.clone()))
        }
    }
}

/// An empty answer posts nothing.
fn post(board: &mut dyn Board, lens: &str, posts: &[Post]) -> Result<(), String> {
    if posts.is_empty() {
        return Ok(());
    }
    board.post(lens, posts)
}

fn counters(posts: &[Post]) -> usize {
    posts
        .iter()
        .filter(
            |p| matches!(p, Post::Evidence { body, .. } if body.direction == Direction::Counter),
        )
        .count()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use orch_core::AgentId;
    use orch_dispatch::{AgentError, ClassifiedError};
    use rusty_bbp::{EvidenceBody, EvidenceKind};

    use super::*;
    use crate::card::fixtures::card;
    use crate::config::tests::STARTER;
    use crate::config::Tools;
    use crate::meter::{Ceilings, Meter};
    use crate::policy::{EventDriven, ValueDriven};
    use crate::port::{Answer, Model, Question, Reply, Usage};
    use crate::schedule::schedule;

    const USD: u64 = 1_000_000;
    const LENSES: [&str; 4] = ["adversary", "architect", "operator", "research"];

    fn config() -> LensConfig {
        LensConfig::parse(STARTER).expect("config")
    }

    fn plain(n: u64) -> Card {
        let mut c = card(vec![]);
        c.id = MsgId(n);
        c.answers = None;
        c
    }

    fn counter(claim: u64) -> Post {
        Post::Evidence {
            body: EvidenceBody {
                claim: MsgId(claim),
                direction: Direction::Counter,
                kind: EvidenceKind::Cited,
                strength: 2,
                note: "fails on empty files".to_owned(),
            },
            evidence: Vec::new(),
        }
    }

    /// Every provider gives the same reply; records who was asked how.
    struct Echo {
        reply: Result<Vec<Post>, ClassifiedError>,
        asked: Vec<(AgentId, Tools, Question)>,
        current: Option<(AgentId, Tools)>,
    }

    impl Echo {
        fn new(reply: Result<Vec<Post>, ClassifiedError>) -> Self {
            Self {
                reply,
                asked: Vec::new(),
                current: None,
            }
        }
    }

    impl Model for Echo {
        fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
            if let Some((id, tools)) = self.current {
                self.asked.push((id, tools, q.clone()));
            }
            self.reply.clone().map(|posts| Answer {
                reply: Reply::Posts(posts),
                usage: Usage {
                    input_tokens: 100,
                    output_tokens: 10,
                    cached_input_tokens: 0,
                },
            })
        }
    }

    impl Models for Echo {
        fn get(&mut self, id: AgentId, tools: Tools) -> Option<&mut dyn Model> {
            self.current = Some((id, tools));
            Some(self)
        }
    }

    #[derive(Default)]
    struct Posted {
        posts: Vec<(String, Vec<Post>)>,
        refuse: bool,
    }

    impl Board for Posted {
        fn post(&mut self, lens: &str, posts: &[Post]) -> Result<(), String> {
            if self.refuse {
                return Err("board is read-only".to_owned());
            }
            self.posts.push((lens.to_owned(), posts.to_vec()));
            Ok(())
        }
    }

    fn prefixes() -> BTreeMap<String, String> {
        LENSES
            .iter()
            .map(|l| ((*l).to_owned(), format!("# Your lens\n{l}\n")))
            .collect()
    }

    struct Out {
        report: Report,
        state: State,
        meter: Meter,
        echo: Echo,
        board: Posted,
    }

    fn go(cards: &[Card], state: State, task_calls: u32, echo: Echo, board: Posted) -> Out {
        go_with(cards, state, task_calls, echo, board, &prefixes())
    }

    fn go_with(
        cards: &[Card],
        state: State,
        task_calls: u32,
        echo: Echo,
        board: Posted,
        prefixes: &BTreeMap<String, String>,
    ) -> Out {
        let policies = Policies {
            run: &mut EventDriven,
            shadow: None,
        };
        go_policies(cards, state, task_calls, echo, board, prefixes, policies)
    }

    fn go_policies(
        cards: &[Card],
        mut state: State,
        task_calls: u32,
        mut echo: Echo,
        mut board: Posted,
        prefixes: &BTreeMap<String, String>,
        policies: Policies<'_>,
    ) -> Out {
        let cfg = config();
        let ceilings = Ceilings {
            experiment: 200 * USD,
            task: 25 * USD,
            task_calls,
        };
        let mut meter = Meter::new(ceilings, 0);
        let records = BTreeMap::new();
        let view = View {
            cards,
            lines: &[],
            prefixes,
            records: &records,
        };
        let ctx = Ctx {
            task: "task-1",
            providers: &cfg.providers,
            meter: &mut meter,
            save: &mut |_| Ok(()),
        };
        let report = round(
            &cfg, &view, policies, &mut state, ctx, &mut echo, &mut board,
        );
        Out {
            report,
            state,
            meter,
            echo,
            board,
        }
    }

    fn lenses_of<'a>(it: impl Iterator<Item = &'a str>) -> BTreeSet<&'a str> {
        it.collect()
    }

    #[test]
    fn answers_are_posted_as_their_lens_and_recorded() {
        let out = go(
            &[plain(7)],
            State::default(),
            400,
            Echo::new(Ok(vec![counter(7)])),
            Posted::default(),
        );
        assert_eq!(out.report.halt, None);
        let reviewers = ["adversary", "operator", "research"];
        assert_eq!(
            lenses_of(out.board.posts.iter().map(|(l, _)| l.as_str())),
            reviewers.into()
        );
        assert!(out.board.posts.iter().all(|(_, p)| p == &[counter(7)]));
        for lens in reviewers {
            let p = &out.state.progress[&(lens.to_owned(), MsgId(7))];
            assert!(p.blind.is_some_and(|b| b.found_counter && b.posted == 1));
        }
        assert!(out.state.in_flight.is_empty());
        assert_eq!(out.meter.snapshot().tasks, [("task-1".to_owned(), 0, 3)]);
    }

    #[test]
    fn each_call_gets_its_lens_prefix_and_tools() {
        let out = go(
            &[plain(7)],
            State::default(),
            400,
            Echo::new(Ok(vec![])),
            Posted::default(),
        );
        assert!(out.board.posts.is_empty(), "an empty answer posts nothing");
        for (_, tools, q) in &out.echo.asked {
            let lens = LENSES
                .iter()
                .find(|l| q.prefix.contains(&format!("\n{l}\n")))
                .expect("a lens prefix");
            let want = if *lens == "research" {
                Tools::ReadOnly
            } else {
                Tools::None
            };
            assert_eq!(*tools, want, "{lens}");
        }
        assert_eq!(out.echo.asked.len(), 3);
    }

    #[test]
    fn a_provider_failure_is_recorded_and_the_round_goes_on() {
        let out = go(
            &[plain(7)],
            State::default(),
            400,
            Echo::new(Err(ClassifiedError::Permanent(AgentError("bad".into())))),
            Posted::default(),
        );
        assert_eq!(out.report.halt, None);
        assert_eq!(out.report.ran.len(), 3);
        assert!(out
            .report
            .ran
            .iter()
            .all(|(_, e)| matches!(e, End::Failed { .. })));
        assert!(out.state.in_flight.is_empty());
        assert!(out.state.progress.is_empty(), "no blind pass recorded");
    }

    #[test]
    fn a_board_refusal_ends_the_round_and_records_nothing() {
        let board = Posted {
            refuse: true,
            ..Posted::default()
        };
        let out = go(
            &[plain(7)],
            State::default(),
            400,
            Echo::new(Ok(vec![counter(7)])),
            board,
        );
        assert_eq!(
            out.report.halt,
            Some(Halt::Board("board is read-only".to_owned()))
        );
        assert_eq!(out.report.ran.len(), 1);
        assert!(out.state.in_flight.is_empty());
        assert!(out.state.progress.is_empty());
    }

    #[test]
    fn a_task_ceiling_ends_the_round() {
        let out = go(
            &[plain(7)],
            State::default(),
            1,
            Echo::new(Ok(vec![counter(7)])),
            Posted::default(),
        );
        assert_eq!(out.report.halt, Some(Halt::Task(Stop::TaskCalls)));
        assert_eq!(out.report.ran.len(), 2);
        assert_eq!(out.board.posts.len(), 1);
        assert!(out.state.in_flight.is_empty());
        assert_eq!(out.state.progress.len(), 1, "only the answered job");
    }

    #[test]
    fn a_lens_without_a_prefix_ends_the_round() {
        let out = go_with(
            &[plain(7)],
            State::default(),
            400,
            Echo::new(Ok(vec![])),
            Posted::default(),
            &BTreeMap::new(),
        );
        assert!(matches!(out.report.halt, Some(Halt::NoPrefix(_))));
        assert!(out.report.ran.is_empty());
        assert!(out.state.in_flight.is_empty());
    }

    #[test]
    fn an_attack_on_a_candidate_counts_a_pass() {
        let mut q = plain(3);
        q.domains = vec!["question".to_owned()];
        q.candidates = vec![MsgId(10), MsgId(11)];
        let cand = |n| {
            let mut c = plain(n);
            c.answers = Some(MsgId(3));
            c
        };
        let mut st = State::default();
        let p = st.questions.entry(MsgId(3)).or_default();
        p.settled = [0, 1].into();
        p.rebel_landed = true;
        let out = go(
            &[q, cand(10), cand(11)],
            st,
            400,
            Echo::new(Ok(vec![])),
            Posted::default(),
        );
        assert_eq!(out.report.halt, None);
        let attacks = &out.state.questions[&MsgId(3)].attacks;
        assert!(attacks[&MsgId(10)] > 0);
        assert_eq!(attacks[&MsgId(10)], attacks[&MsgId(11)]);
    }

    #[test]
    fn the_run_policy_picks_and_the_shadow_is_only_reported() {
        let mut low = plain(7);
        low.stakes = rusty_bbp::Stakes::Low;
        let cards = [low, plain(8)];
        let mut value = ValueDriven::new(std::num::NonZeroUsize::MIN);
        let policies = Policies {
            run: &mut value,
            shadow: Some(&mut EventDriven),
        };
        let out = go_policies(
            &cards,
            State::default(),
            400,
            Echo::new(Ok(vec![])),
            Posted::default(),
            &prefixes(),
            policies,
        );
        assert!(!out.report.ran.is_empty());
        assert!(out.report.ran.iter().all(|(k, _)| k.claim == MsgId(8)));
        let shadow = out.report.shadow.expect("a shadow pick");
        assert!(shadow.iter().any(|k| k.claim == MsgId(7)));
        assert!(shadow.len() > out.report.ran.len());
    }

    #[test]
    fn no_shadow_reports_none() {
        let out = go(
            &[plain(7)],
            State::default(),
            400,
            Echo::new(Ok(vec![])),
            Posted::default(),
        );
        assert_eq!(out.report.shadow, None);
    }

    /// Returns every job tampered with, reversed, with a duplicate and an
    /// unknown key added.
    struct Tamper;

    impl Policy for Tamper {
        fn pick(&mut self, _: &[Card], _: &State, jobs: Vec<Job>) -> Vec<Job> {
            let mut out: Vec<Job> = jobs
                .into_iter()
                .rev()
                .map(|mut j| {
                    j.phase = crate::context::Phase::Informed;
                    j.tools = Tools::ReadOnly;
                    j.allowed = crate::post::Allowed {
                        evidence: None,
                        claims: None,
                    };
                    j.providers.clear();
                    j.suffix = Some("tampered");
                    j
                })
                .collect();
            if let Some(first) = out.first().cloned() {
                out.push(first.clone());
                let mut unknown = first;
                unknown.key.claim = MsgId(99);
                out.push(unknown);
            }
            out
        }
    }

    #[test]
    fn a_pick_keeps_only_the_schedulers_jobs() {
        let cfg = config();
        let cards = [plain(7)];
        let scheduled = schedule(&cfg, &cards, &State::default()).0;
        assert!(scheduled.len() > 1);
        let picked = Tamper.pick(&cards, &State::default(), scheduled.clone());
        let kept = only_scheduled(&scheduled, picked);
        let mut want = scheduled.clone();
        want.reverse();
        assert_eq!(kept, want, "scheduler's jobs, policy's order, once each");
    }

    #[test]
    fn a_tampering_policy_runs_and_shadows_only_scheduled_jobs() {
        let cards = [plain(7)];
        let scheduled = schedule(&config(), &cards, &State::default()).0;
        let mut shadow = Tamper;
        let policies = Policies {
            run: &mut Tamper,
            shadow: Some(&mut shadow),
        };
        let out = go_policies(
            &cards,
            State::default(),
            400,
            Echo::new(Ok(vec![])),
            Posted::default(),
            &prefixes(),
            policies,
        );
        let mut want: Vec<JobKey> = scheduled.iter().map(|j| j.key.clone()).collect();
        want.reverse();
        let ran: Vec<JobKey> = out.report.ran.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(ran, want);
        assert_eq!(out.report.shadow, Some(want));
        assert_eq!(out.echo.asked.len(), scheduled.len());
        for (agent, tools, q) in &out.echo.asked {
            assert!(!q.body.contains("tampered"));
            let job = scheduled
                .iter()
                .find(|j| q.prefix.contains(&format!("\n{}\n", j.key.lens)))
                .expect("a scheduled lens");
            assert!(job.providers.contains(agent));
            assert_eq!(*tools, job.tools);
            assert_eq!(q.shape, crate::port::Shape::Posts(job.allowed.clone()));
        }
    }
}
