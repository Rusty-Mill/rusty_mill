//! One runner round: schedule the board's cards, call each job and post
//! what comes back. Pure over four ports the shell supplies: [`Models`],
//! the meter's save, the [`Board`] and the track record's write-ahead.
//!
//! - Jobs run one at a time, in schedule order; the next round sees what
//!   this one posted.
//! - An answer is posted as its lens, then recorded; an attack on a
//!   candidate also counts a pass for the fairness rule. A blind review's
//!   row is saved `pending` before its first post (D92 write-ahead).
//! - A provider's failure (`Failed`, `Exhausted`) is recorded and the round
//!   goes on; the rules decide what to ask next.
//! - The runner's own failures end the round and record nothing, so the
//!   rules ask again once fixed: a task ceiling, a meter that could not be
//!   saved, a board that refused a post, a lens with no prefix. An
//!   experiment stop only drops that job: free providers go on.
//! - A proposal slot that no cleared provider can serve is settled.

use std::collections::BTreeMap;

use orch_core::AgentId;
use orch_dispatch::ClassifiedError;
use rusty_bbp::{Direction, MsgId};

use crate::call::{call, Ctx, End, Models};
use crate::card::Card;
use crate::config::LensConfig;
use crate::context::{Line, Parts, Phase};
use crate::meter::Stop;
use crate::post::Post;
use crate::record::Review;
use crate::schedule::{schedule, Job, JobKey, Skipped, Slot, State};

/// Where answers go: the shared board.
pub trait Board {
    /// Post `posts` as lens `lens`, with its credential, and return their
    /// message ids in order.
    ///
    /// # Errors
    ///
    /// The failure, with the ids of the posts that landed before it.
    fn post(&mut self, lens: &str, posts: &[Post]) -> Result<Vec<MsgId>, PostError>;
}

/// A post that failed partway: what landed before it stays on the board,
/// so it stays the job's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostError {
    /// The ids the board answered with, in order.
    pub posted: Vec<MsgId>,
    /// The failing post's answer was lost: it may have landed, unknown.
    pub unsure: bool,
    pub error: String,
}

impl PostError {
    /// A refused post: nothing more landed than `posted`.
    pub fn refused(posted: Vec<MsgId>, error: impl Into<String>) -> Self {
        Self {
            posted,
            unsure: false,
            error: error.into(),
        }
    }
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

/// Why a round ended early. Nothing is recorded for the job it ended on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halt {
    /// A task ceiling: the task pauses.
    Task(Stop),
    /// The meter or a write-ahead row could not be saved; the shell should
    /// stop.
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
    /// Blind review jobs that ended, for the track record (#599).
    pub reviews: Vec<Review>,
    pub halt: Option<Halt>,
}

/// Saves a blind review's row before its first post; see [`Review::pending`].
pub type Ahead<'a> = &'a mut dyn FnMut(&Review) -> Result<(), String>;

/// Run one round over `view`, updating `state` and the meter in `ctx`.
pub fn round(
    config: &LensConfig,
    view: &View<'_>,
    state: &mut State,
    ctx: Ctx<'_>,
    models: &mut dyn Models,
    board: &mut dyn Board,
    ahead: Ahead<'_>,
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
    let by_id: BTreeMap<MsgId, &Card> = view.cards.iter().map(|c| (c.id, c)).collect();
    let mut ran = Vec::new();
    let mut reviews = Vec::new();
    for job in &jobs {
        let Some(card) = by_id.get(&job.key.claim) else {
            continue;
        };
        let Some(prefix) = view.prefixes.get(&job.key.lens) else {
            return report(ran, skipped, reviews, Halt::NoPrefix(job.key.lens.clone()));
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
        let (row, halted) = settle(job, card, &called.end, state, board, &mut *ahead);
        reviews.extend(row);
        let halt = halted.or_else(|| called.unsaved.clone().map(Halt::Unsaved));
        ran.push((job.key.clone(), called.end));
        if let Some(h) = halt {
            return report(ran, skipped, reviews, h);
        }
    }
    Report {
        ran,
        skipped,
        reviews,
        halt: None,
    }
}

fn report(
    ran: Vec<(JobKey, End)>,
    skipped: Vec<Skipped>,
    reviews: Vec<Review>,
    halt: Halt,
) -> Report {
    Report {
        ran,
        skipped,
        reviews,
        halt: Some(halt),
    }
}

/// The track-record row of a blind review job that ended with `items`
/// posted; `None` for any other job.
fn review(job: &Job, items: Vec<MsgId>) -> Option<Review> {
    let blind = match job.key.slot {
        Slot::Review => job.phase == Phase::Blind,
        Slot::LookHarder => true,
        Slot::Propose(_) | Slot::Rebel => false,
    };
    blind.then(|| Review {
        lens: job.key.lens.clone(),
        claim: job.key.claim,
        epoch: job.epoch,
        look_harder: job.key.slot == Slot::LookHarder,
        items,
        unsure: false,
        pending: false,
    })
}

/// Post and record one job's end: a blind review's track-record row, if
/// any, and a halt that ends the round. The row holds what the job posted,
/// or nothing for a reply the protocol refused, so refusing is not free. A
/// post that failed partway halts (see [`partial`]); so does a write-ahead
/// that could not be saved, before anything is posted.
fn settle(
    job: &Job,
    card: &Card,
    end: &End,
    state: &mut State,
    board: &mut dyn Board,
    ahead: Ahead<'_>,
) -> (Option<Review>, Option<Halt>) {
    match end {
        End::Answered { provider, posts } => {
            if let Err(e) = write_ahead(job, posts, ahead) {
                state.abandon(job);
                return (None, Some(Halt::Unsaved(e)));
            }
            let ids = match post(board, &job.key.lens, posts) {
                Ok(ids) => ids,
                Err(e) => return partial(job, card, *provider, posts, e, state),
            };
            state.answered(job, *provider, posts.len(), counters(posts));
            if let (Slot::Review | Slot::LookHarder, Some(q)) = (job.key.slot, card.answers) {
                state.attacked(q, card.id, card.epoch);
            }
            (review(job, ids), None)
        }
        End::Failed { error, .. } => {
            state.failed(job);
            let refused = matches!(error, ClassifiedError::Permanent(_));
            (refused.then(|| review(job, Vec::new())).flatten(), None)
        }
        End::Exhausted => {
            state.failed(job);
            (None, None)
        }
        End::Stopped(Stop::Experiment) => {
            state.abandon(job);
            (None, None)
        }
        End::Stopped(stop) => {
            state.abandon(job);
            (None, Some(Halt::Task(*stop)))
        }
        End::Unsaved(e) => {
            state.abandon(job);
            (None, Some(Halt::Unsaved(e.clone())))
        }
    }
}

/// A post that failed partway. Nothing on the board: the job is abandoned
/// and asked again. Something landed, or may have (a lost answer): the job
/// is answered with what landed and never asked again, since a retry would
/// post fresh evidence beside it. Its row keeps the landed posts and
/// whether one is unknown.
fn partial(
    job: &Job,
    card: &Card,
    provider: AgentId,
    posts: &[Post],
    e: PostError,
    state: &mut State,
) -> (Option<Review>, Option<Halt>) {
    let halt = Some(Halt::Board(e.error));
    if e.posted.is_empty() && !e.unsure {
        state.abandon(job);
        return (None, halt);
    }
    let landed = &posts[..e.posted.len().min(posts.len())];
    state.answered(job, provider, landed.len(), counters(landed));
    if let (Slot::Review | Slot::LookHarder, Some(q)) = (job.key.slot, card.answers) {
        state.attacked(q, card.id, card.epoch);
    }
    let row = review(job, e.posted).map(|r| Review {
        unsure: e.unsure,
        ..r
    });
    (row, halt)
}

/// Save a blind review's row `pending` before its posts, so a crash or a
/// failed save after them leaves the job unscored, never scored on part of
/// its items. An empty answer posts nothing and needs none.
fn write_ahead(job: &Job, posts: &[Post], ahead: Ahead<'_>) -> Result<(), String> {
    let Some(row) = review(job, Vec::new()).filter(|_| !posts.is_empty()) else {
        return Ok(());
    };
    ahead(&Review {
        pending: true,
        ..row
    })
    .map_err(|e| format!("track record not saved: {e}"))
}

/// An empty answer posts nothing.
fn post(board: &mut dyn Board, lens: &str, posts: &[Post]) -> Result<Vec<MsgId>, PostError> {
    if posts.is_empty() {
        return Ok(Vec::new());
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
    use crate::port::{Answer, Model, Question, Reply, Usage};

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
        /// Refuse after this many posts have landed in one call.
        refuse_after: Option<usize>,
        /// The write-ahead save fails.
        ahead_fails: bool,
    }

    impl Board for Posted {
        fn post(&mut self, lens: &str, posts: &[Post]) -> Result<Vec<MsgId>, PostError> {
            if self.refuse {
                return Err(PostError::refused(Vec::new(), "board is read-only"));
            }
            if let Some(n) = self.refuse_after.filter(|n| *n < posts.len()) {
                let first = 100 + self.posts.iter().map(|(_, p)| p.len() as u64).sum::<u64>();
                self.posts.push((lens.to_owned(), posts[..n].to_vec()));
                return Err(PostError::refused(
                    (first..first + n as u64).map(MsgId).collect(),
                    "rejected",
                ));
            }
            let first = 100 + self.posts.iter().map(|(_, p)| p.len() as u64).sum::<u64>();
            self.posts.push((lens.to_owned(), posts.to_vec()));
            Ok((first..first + posts.len() as u64).map(MsgId).collect())
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
        /// The write-ahead rows saved, in order.
        ahead: Vec<Review>,
    }

    fn go(cards: &[Card], state: State, task_calls: u32, echo: Echo, board: Posted) -> Out {
        go_with(cards, state, task_calls, echo, board, &prefixes())
    }

    fn go_with(
        cards: &[Card],
        mut state: State,
        task_calls: u32,
        mut echo: Echo,
        mut board: Posted,
        prefixes: &BTreeMap<String, String>,
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
        let fails = board.ahead_fails;
        let mut ahead = Vec::new();
        let mut save_ahead = |r: &Review| {
            if fails {
                return Err("disk full".to_owned());
            }
            ahead.push(r.clone());
            Ok(())
        };
        let report = round(
            &cfg,
            &view,
            &mut state,
            ctx,
            &mut echo,
            &mut board,
            &mut save_ahead,
        );
        Out {
            report,
            state,
            meter,
            echo,
            board,
            ahead,
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
    fn blind_reviews_are_kept_for_the_track_record() {
        let out = go(
            &[plain(7)],
            State::default(),
            400,
            Echo::new(Ok(vec![counter(7)])),
            Posted::default(),
        );
        let mut rows = out.report.reviews.clone();
        rows.sort_by(|a, b| a.lens.cmp(&b.lens));
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.claim == MsgId(7) && !r.look_harder));
        let mut items: Vec<MsgId> = rows.iter().flat_map(|r| r.items.clone()).collect();
        items.sort();
        assert_eq!(items, [MsgId(100), MsgId(101), MsgId(102)], "one post each");

        // A reply the protocol refused is kept with nothing posted; a
        // provider that couldn't answer is not kept.
        let refused = go(
            &[plain(7)],
            State::default(),
            400,
            Echo::new(Err(ClassifiedError::Permanent(AgentError("bad".into())))),
            Posted::default(),
        );
        assert_eq!(refused.report.reviews.len(), 3);
        assert!(refused.report.reviews.iter().all(|r| r.items.is_empty()));
        let down = go(
            &[plain(7)],
            State::default(),
            400,
            Echo::new(Err(ClassifiedError::Transient(AgentError("busy".into())))),
            Posted::default(),
        );
        assert!(down.report.reviews.is_empty());
    }

    #[test]
    fn each_blind_review_is_saved_pending_before_it_posts() {
        let out = go(
            &[plain(7)],
            State::default(),
            400,
            Echo::new(Ok(vec![counter(7)])),
            Posted::default(),
        );
        assert_eq!(out.ahead.len(), 3, "one write-ahead per blind review");
        assert!(out.ahead.iter().all(|r| r.pending && r.items.is_empty()));
        assert!(
            out.report.reviews.iter().all(|r| !r.pending),
            "the save after the posts clears it"
        );
    }

    #[test]
    fn a_failed_write_ahead_posts_nothing_and_halts() {
        let board = Posted {
            ahead_fails: true,
            ..Posted::default()
        };
        let out = go(
            &[plain(7)],
            State::default(),
            400,
            Echo::new(Ok(vec![counter(7)])),
            board,
        );
        assert!(out.board.posts.is_empty(), "nothing posted without a row");
        assert!(out.report.reviews.is_empty());
        assert!(matches!(
            out.report.halt,
            Some(Halt::Unsaved(ref e)) if e.contains("track record not saved")
        ));
    }

    #[test]
    fn a_post_that_fails_partway_keeps_its_landed_items_and_halts() {
        let board = Posted {
            refuse_after: Some(1),
            ..Posted::default()
        };
        let out = go(
            &[plain(7)],
            State::default(),
            400,
            Echo::new(Ok(vec![counter(7), counter(7)])),
            board,
        );
        assert!(
            matches!(&out.report.halt, Some(Halt::Board(e)) if e == "rejected"),
            "{:?}",
            out.report.halt
        );
        assert_eq!(
            out.report.reviews.len(),
            1,
            "the round stops at the first job"
        );
        assert_eq!(out.report.reviews[0].items, [MsgId(100)]);
        // The job's first post is on the board: it is answered, not asked
        // again, so a later round can't post fresh evidence beside it.
        let lens = out.report.reviews[0].lens.clone();
        let again = go(
            &[plain(7)],
            out.state,
            400,
            Echo::new(Ok(vec![counter(7)])),
            Posted::default(),
        );
        assert!(
            again.report.ran.iter().all(|(k, _)| k.lens != lens),
            "{:?}",
            again.report.ran
        );
        assert_eq!(again.report.ran.len(), 2, "the other lenses still run");
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
        assert!(attacks[&MsgId(10)].passes > 0);
        assert_eq!(attacks[&MsgId(10)], attacks[&MsgId(11)]);
    }
}
