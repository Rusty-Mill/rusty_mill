//! `rusty_orch lens run`: drive one claims task with the configured lenses
//! until the board goes quiet (#594).
//!
//! Each pass refreshes every lens's view through its own BBP session
//! ([`super::bbp`]) and runs one [`round`] for that lens alone, so a lens
//! is only ever shown what its session can read. Passes repeat until one
//! starts no job, a round halts, or `--max-passes` is reached (a provider
//! that keeps failing transiently would otherwise be retried forever).
//!
//! The meter is restored from, and saved to, the orch state directory
//! before every call. The scheduler's memory is restored at start and
//! saved after every lens's round, under the board's identity: the
//! canonical BBP store directory and the task id ([`board`]), so a task of
//! the same name in another BBP directory starts with nothing remembered.
//! A crash mid-round asks that round's jobs again.
//!
//! Each blind review job is saved for its lens's track record (#599) after
//! its round, and each pass starts by reading every lens's record into the
//! line its context shows. A record that can't be read is logged and shown
//! to nobody: it is informational and never holds a run up.
//!
//! The #598 guards ([`Guard`]): every post is tainted with the class of the
//! context its provider was shown, the table is saved after each round and
//! applied to every card and line, so a tainted claim goes only to
//! providers cleared for it. The cost breaker judges each lens's fresh
//! cards against the window it opened; its window lives in memory and
//! starts again with each run (PLAN S8: no breaker table). An enforced
//! trip, and a task ceiling, ask BBP to pause the task (`bbp pause`).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use orch_core::AgentId;
use orch_lens::breaker::{judge, Breaker, Mark, Outcome};
use orch_lens::call::{Ctx, Models};
use orch_lens::card::Card;
use orch_lens::compose::{prefix_key, Limits, PrefixKey};
use orch_lens::config::Tools;
use orch_lens::context::{prefix, Line};
use orch_lens::meter::{Meter, Stop};
use orch_lens::record::{Review, Tally};
use orch_lens::round::{round, Board, Halt, View};
use orch_lens::schedule::State;
use orch_lens::taint::Taints;
use orch_lens::{LensConfig, Model};
use orch_store::Store;
use rusty_bbp::ArtId;

use super::assertions::{self, Grants};
use super::bbp::{run_id, Child, Diffs, Rpc, Session};
use super::catalog::Catalog;
use super::check::DEFAULT_ASSERTIONS;
use super::hunts::{Kit, Ordinary};
use super::registry::Registry;
use crate::args::ArgsError;

/// Passes when none is named.
pub const DEFAULT_MAX_PASSES: u32 = 20;

pub const USAGE: &str = "usage: rusty_orch lens run <config.json> --bbp <bbp binary> --dir <bbp dir> --task <task>\n           --state <orch state dir> --experiment <name> [--rules <file>] [--assertions <file>]\n           [--repo <dir>] [--max-passes <n>] [--alien]\n\nRun the configured lenses on one BBP claims task until a pass starts no job. Each lens\nreads and posts through its own `bbp mcp` session. Calls the configured models: their\nadmission follows the operator's assertions, as `lens check` reports.\n\n--alien  then run the config's alien lens (#708): one hunt per eligible claim not hunted\n         yet, with each probe built and run sandboxed, offline and with no network, in a\n         scratch tree of --repo under --state. Posts nothing; `lens alien report` reads\n         the rows. Needs `rustc` and the dependencies in the cargo home already.";

/// What to run.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub config: PathBuf,
    pub bbp: PathBuf,
    pub dir: PathBuf,
    pub task: String,
    pub state: PathBuf,
    pub experiment: String,
    pub rules: Option<PathBuf>,
    pub assertions: PathBuf,
    pub repo: PathBuf,
    pub max_passes: u32,
    /// Run the alien lens's pass after the ordinary passes (#708).
    pub alien: bool,
}

/// Parse `run <config> --bbp … --dir … --task … --state … --experiment …`.
pub fn options(argv: impl IntoIterator<Item = String>, cwd: PathBuf) -> Result<Options, ArgsError> {
    let mut args = argv.into_iter();
    match args.next().as_deref() {
        Some("run") => {}
        Some("-h" | "--help") | None => return Err(ArgsError::Help),
        Some(other) => return Err(ArgsError::Usage(format!("unknown lens command {other:?}"))),
    }
    let mut config = None;
    let mut alien = false;
    let mut flags: BTreeMap<&'static str, String> = BTreeMap::new();
    const FLAGS: [&str; 9] = [
        "--bbp",
        "--dir",
        "--task",
        "--state",
        "--experiment",
        "--rules",
        "--assertions",
        "--repo",
        "--max-passes",
    ];
    while let Some(arg) = args.next() {
        if arg == "-h" || arg == "--help" {
            return Err(ArgsError::Help);
        }
        if arg == "--alien" {
            if alien {
                return Err(ArgsError::Usage("duplicate --alien".to_owned()));
            }
            alien = true;
            continue;
        }
        if let Some(flag) = FLAGS.iter().find(|f| **f == arg) {
            let value = args
                .next()
                .filter(|v| !v.is_empty() && !v.starts_with('-'))
                .ok_or_else(|| ArgsError::Usage(format!("{arg} needs a value")))?;
            if flags.insert(flag, value).is_some() {
                return Err(ArgsError::Usage(format!("duplicate {arg}")));
            }
            continue;
        }
        if arg.starts_with('-') || config.is_some() {
            return Err(ArgsError::Usage(format!("unexpected argument {arg:?}")));
        }
        config = Some(PathBuf::from(arg));
    }
    let mut take = |f: &str| flags.remove(f);
    let required =
        |v: Option<String>, f: &str| v.ok_or_else(|| ArgsError::Usage(format!("{f} is required")));
    let max_passes = match take("--max-passes") {
        None => DEFAULT_MAX_PASSES,
        Some(n) => n.parse().ok().filter(|n| *n > 0).ok_or_else(|| {
            ArgsError::Usage(format!("--max-passes {n:?} is not a positive number"))
        })?,
    };
    Ok(Options {
        config: config.ok_or_else(|| ArgsError::Usage("a lens config is required".into()))?,
        bbp: required(take("--bbp"), "--bbp")?.into(),
        dir: required(take("--dir"), "--dir")?.into(),
        task: required(take("--task"), "--task")?,
        state: required(take("--state"), "--state")?.into(),
        experiment: required(take("--experiment"), "--experiment")?,
        rules: take("--rules").map(PathBuf::from),
        assertions: take("--assertions")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_ASSERTIONS)),
        repo: take("--repo").map(PathBuf::from).unwrap_or(cwd),
        max_passes,
        alien,
    })
}

/// One lens's way onto the board.
pub trait Seat {
    fn refresh(&mut self, config: &LensConfig) -> Result<(), String>;
    /// The task's current brief, as of the last refresh (#598: a revised
    /// intent names a new one).
    fn brief_id(&self) -> Option<ArtId>;
    /// The current brief's text.
    fn brief(&mut self) -> Result<String, String>;
    /// Use `taints` for every card and line built from now on. `lenses`
    /// are the lens ids the run's state has jobs for, so a lens since
    /// removed from the config still counts as one (D139).
    fn taint(&mut self, taints: &Taints, lenses: &BTreeSet<String>);
    /// The task revision this seat's view is current to: of two views of
    /// one claim, the higher revision's is the newer.
    fn rev(&self) -> u64;
    fn cards(&self, config: &LensConfig) -> Vec<Card>;
    fn lines(&mut self, config: &LensConfig) -> Result<Vec<Line>, String>;
    fn board(&mut self) -> &mut dyn Board;
    /// Code candidate `id`'s base and diffs, as far as the role may read
    /// them (D98).
    fn candidate(&mut self, id: ArtId) -> Result<Diffs, String>;
}

impl<R: Rpc> Seat for Session<R> {
    fn refresh(&mut self, config: &LensConfig) -> Result<(), String> {
        Session::refresh(self, config)
    }
    fn brief_id(&self) -> Option<ArtId> {
        Session::brief_id(self)
    }
    fn brief(&mut self) -> Result<String, String> {
        Session::brief(self)
    }
    fn taint(&mut self, taints: &Taints, lenses: &BTreeSet<String>) {
        Session::taint(self, taints, lenses);
    }
    fn rev(&self) -> u64 {
        Session::rev(self)
    }
    fn cards(&self, config: &LensConfig) -> Vec<Card> {
        Session::cards(self, config)
    }
    fn lines(&mut self, config: &LensConfig) -> Result<Vec<Line>, String> {
        Session::lines(self, config)
    }
    fn board(&mut self) -> &mut dyn Board {
        self
    }
    fn candidate(&mut self, id: ArtId) -> Result<Diffs, String> {
        Session::candidate_diffs(self, id)
    }
}

impl Models for Registry {
    fn get(&mut self, id: AgentId, tools: Tools) -> Option<&mut dyn Model> {
        Registry::get(self, id, tools)
    }
}

/// The meter and where it is saved.
pub struct Metering<'a> {
    pub task: &'a str,
    pub meter: &'a mut Meter,
    pub save: &'a mut dyn FnMut(&Meter) -> Result<(), String>,
}

/// How the passes ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// A pass started no job.
    Quiet,
    /// `--max-passes` passes ran.
    MaxPasses,
    /// A round halted.
    Halted(Halt),
    /// The enforced cost breaker tripped and the task was paused.
    Paused(String),
}

/// A pause the runner asks BBP for (`bbp pause`, #598 C-R12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pause {
    /// `breaker`, `spend_ceiling` or `call_ceiling`.
    pub reason: &'static str,
    pub detail: String,
    /// The same op replays; a new run's ops are new, so a resumed task
    /// still over its ceiling is paused again.
    pub op: String,
}

/// The #598 guards: the cost breaker, the pause it and the task ceilings
/// ask for, and the taint table with where it is saved.
pub struct Guard<'a> {
    pub breaker: Breaker,
    /// This run's id, which names each pause op.
    pub run: &'a str,
    pub pause: &'a mut dyn FnMut(&Pause) -> Result<(), String>,
    pub taints: &'a mut Taints,
    pub save_taints: &'a mut dyn FnMut(&Taints) -> Result<(), String>,
}

impl Pause {
    /// The pause for `reason` in run `run` at `calls` calls.
    fn new(run: &str, reason: &'static str, detail: String, calls: u32) -> Self {
        let op = format!("orch-{run}-{reason}-{calls}");
        Self { reason, detail, op }
    }
}

impl Guard<'_> {
    fn pause(&mut self, reason: &'static str, detail: String, calls: u32) -> Result<(), String> {
        (self.pause)(&Pause::new(self.run, reason, detail, calls))
            .map_err(|e| format!("task not paused ({reason}): {e}"))
    }

    /// Raise the table by a round's posts and save it when it grew.
    fn raise(&mut self, posted: &[(rusty_bbp::MsgId, orch_lens::DataClass)]) -> Result<(), String> {
        if posted.is_empty() {
            return Ok(());
        }
        for (id, class) in posted {
            self.taints.raise(*id, *class);
        }
        (self.save_taints)(self.taints).map_err(|e| format!("taint table not saved: {e}"))
    }
}

/// The BBP pause reason for a task ceiling.
fn ceiling_reason(stop: Stop) -> Option<&'static str> {
    match stop {
        Stop::TaskSpend => Some("spend_ceiling"),
        Stop::TaskCalls => Some("call_ceiling"),
        Stop::Experiment => None,
    }
}

/// Where blind review jobs go and each lens's track record comes from.
pub struct Records<'a> {
    pub save: &'a mut dyn FnMut(&Review) -> Result<(), String>,
    pub read: &'a mut dyn FnMut() -> Result<BTreeMap<String, Tally>, String>,
}

/// The lenses to run: the config, and each lens's seat and prompt by id.
pub struct Lenses<'a> {
    pub config: &'a LensConfig,
    pub seats: &'a mut BTreeMap<String, Box<dyn Seat>>,
    pub prompts: &'a Prompts,
}

/// What every lens prefix is built from besides the intent.
pub struct Prompts {
    /// The board rules, shared by every lens.
    pub rules: String,
    /// Each lens's own prompt, by lens id.
    pub lens: BTreeMap<String, String>,
    /// #723: the Factory catalog when the config turns composition on.
    pub catalog: Option<Catalog>,
}

/// Each lens's prefix, rebuilt whenever its seat sees a new brief (#598):
/// after an intent revision no call goes out under the old intent.
#[derive(Default)]
struct Prefixes {
    text: BTreeMap<String, String>,
    built_on: BTreeMap<String, Option<ArtId>>,
    /// The brief each lens's prefix was built on, for composed prefixes.
    intent: BTreeMap<String, String>,
}

impl Prefixes {
    fn update(&mut self, lens: &str, seat: &mut dyn Seat, prompts: &Prompts) -> Result<(), String> {
        let Some(prompt) = prompts.lens.get(lens) else {
            return Ok(()); // the round halts with NoPrefix
        };
        let id = seat.brief_id();
        if self.built_on.get(lens) == Some(&id) {
            return Ok(());
        }
        let intent = seat.brief()?;
        self.text
            .insert(lens.to_owned(), prefix(&intent, &prompts.rules, prompt));
        self.built_on.insert(lens.to_owned(), id);
        self.intent.insert(lens.to_owned(), intent);
        Ok(())
    }

    /// #723: one composed prefix per distinct domain set among `cards`,
    /// built on the same brief as the lens's flat prefix. Empty when
    /// composition is off. Each file dropped to fit is logged.
    fn composed(
        &self,
        lens: &str,
        cards: &[Card],
        prompts: &Prompts,
        log: &mut String,
    ) -> Result<BTreeMap<PrefixKey, String>, String> {
        let (Some(catalog), Some(prompt), Some(intent)) = (
            prompts.catalog.as_ref(),
            prompts.lens.get(lens),
            self.intent.get(lens),
        ) else {
            return Ok(BTreeMap::new());
        };
        let mut out = BTreeMap::new();
        for card in cards {
            let key = prefix_key(lens, &card.domains);
            if out.contains_key(&key) {
                continue;
            }
            let built = catalog.build(prompt, &key.1)?;
            for d in &built.composed.dropped {
                let _ = writeln!(log, "{lens} [{}]: {d} left out to fit", key.1.join(", "));
            }
            out.insert(key, prefix(intent, &prompts.rules, &built.composed.prompt));
        }
        Ok(out)
    }
}

/// The task-wide claim set the breaker marks: every seat's cards merged
/// by claim id, the newest revision's view of each claim winning. Lenses
/// follow different domains, so one lens's view alone would count claims
/// new to it as progress, and a stale seat's older score as a move.
fn task_cards(seats: &BTreeMap<String, Box<dyn Seat>>, config: &LensConfig) -> Vec<Card> {
    let mut by_rev: Vec<&dyn Seat> = seats.values().map(|s| s.as_ref()).collect();
    by_rev.sort_by_key(|s| s.rev());
    let mut all: BTreeMap<_, Card> = BTreeMap::new();
    for seat in by_rev {
        all.extend(seat.cards(config).into_iter().map(|c| (c.id, c)));
    }
    all.into_values().collect()
}

/// Every lens id the run's state has a job for, kept or in flight.
pub(super) fn lens_history(state: &State) -> BTreeSet<String> {
    let kept = state.progress.keys().map(|(lens, _)| lens);
    let running = state.in_flight.iter().map(|k| &k.lens);
    kept.chain(running).cloned().collect()
}

/// Run up to `max` passes over every lens until one starts no job. A lens
/// without a seat is an error. Each job's outcome is written to `log`.
/// `state` goes to `keep` after every lens's round, so a restart resumes
/// from it; a failed keep is an error. The round's review rows are saved
/// first, so no job is kept as done without its row.
///
/// The cost breaker is judged before each lens's round, never inside one:
/// a round over many claims can run past `window_calls` before the next
/// judgement, and the run's last round is not judged after it ends. The
/// meter's task ceilings, checked before every call, are the only hard cap.
#[allow(clippy::too_many_arguments)]
pub fn passes(
    lenses: Lenses<'_>,
    state: &mut State,
    keep: &mut dyn FnMut(&State) -> Result<(), String>,
    records: Records<'_>,
    metering: Metering<'_>,
    mut guard: Guard<'_>,
    models: &mut dyn Models,
    max: u32,
    log: &mut String,
) -> Result<Ended, String> {
    let Lenses {
        config,
        seats,
        prompts,
    } = lenses;
    let mut prefixes = Prefixes::default();
    let mut window: Option<Mark> = None;
    // Every view filled before the breaker's first mark, so no lens's
    // claims first appear mid-window as progress.
    for (id, seat) in seats.iter_mut() {
        seat.refresh(config).map_err(|e| format!("{id}: {e}"))?;
    }
    for pass in 1..=max {
        let tracks: BTreeMap<String, String> = match (records.read)() {
            Ok(t) => t.iter().map(|(l, t)| (l.clone(), t.line())).collect(),
            Err(e) => {
                let _ = writeln!(log, "pass {pass} track record not read: {e}");
                BTreeMap::new()
            }
        };
        let mut started = 0;
        for lens in &config.lenses {
            let seat = seats
                .get_mut(&lens.id)
                .ok_or_else(|| format!("no BBP session for lens {}", lens.id))?;
            seat.refresh(config)
                .map_err(|e| format!("{}: {e}", lens.id))?;
            seat.taint(guard.taints, &lens_history(state));
            prefixes
                .update(&lens.id, seat.as_mut(), prompts)
                .map_err(|e| format!("{}: {e}", lens.id))?;
            let cards = seat.cards(config);
            let composed = prefixes
                .composed(&lens.id, &cards, prompts, log)
                .map_err(|e| format!("{}: {e}", lens.id))?;
            let calls = metering.meter.calls(metering.task);
            let seen = task_cards(seats, config);
            let now = Mark::of(calls, metering.meter.task_spent(metering.task), &seen);
            let start = window.get_or_insert_with(|| now.clone());
            match judge(&guard.breaker, start, &now) {
                Outcome::Open => {}
                Outcome::Healthy => window = Some(now),
                Outcome::WouldTrip(detail) => {
                    let _ = writeln!(log, "pass {pass} breaker would trip: {detail}");
                    window = Some(now);
                }
                Outcome::Pause(detail) => {
                    guard.pause("breaker", detail.clone(), calls)?;
                    let _ = writeln!(log, "pass {pass} paused: {detail}");
                    return Ok(Ended::Paused(detail));
                }
            }
            let seat = seats
                .get_mut(&lens.id)
                .ok_or_else(|| format!("no BBP session for lens {}", lens.id))?;
            let lines = seat
                .lines(config)
                .map_err(|e| format!("{}: {e}", lens.id))?;
            let one = LensConfig {
                lenses: vec![lens.clone()],
                ..config.clone()
            };
            let view = View {
                cards: &cards,
                lines: &lines,
                prefixes: &prefixes.text,
                composed: &composed,
                records: &tracks,
            };
            let ctx = Ctx {
                task: metering.task,
                providers: &config.providers,
                meter: &mut *metering.meter,
                save: &mut *metering.save,
            };
            let report = round(
                &one,
                &view,
                state,
                ctx,
                models,
                seat.board(),
                &mut *records.save,
            );
            // Rows before state: a job kept as done must have its row, and a
            // row saved for a job that then reruns is replaced by its key.
            for r in &report.reviews {
                (records.save)(r).map_err(|e| format!("track record not saved: {e}"))?;
            }
            // Taints before state too: a job kept as done has tainted its posts.
            guard.raise(&report.taints)?;
            keep(state).map_err(|e| format!("lens state not saved: {e}"))?;
            started += report.ran.len();
            for (key, end) in &report.ran {
                let _ = writeln!(
                    log,
                    "pass {pass} {} {} {:?}: {end:?}",
                    key.lens, key.claim, key.slot
                );
            }
            for s in &report.skipped {
                let _ = writeln!(
                    log,
                    "pass {pass} {} {} {:?}: skipped, {}",
                    s.key.lens, s.key.claim, s.key.slot, s.reason
                );
            }
            if let Some(h) = report.halt {
                if let Halt::Task(stop) = &h {
                    if let Some(reason) = ceiling_reason(*stop) {
                        let calls = metering.meter.calls(metering.task);
                        guard.pause(reason, stop.to_string(), calls)?;
                    }
                }
                return Ok(Ended::Halted(h));
            }
        }
        if started == 0 {
            return Ok(Ended::Quiet);
        }
    }
    Ok(Ended::MaxPasses)
}

/// The command's result: the log on a clean end (quiet, the pass cap, a
/// task ceiling or an enforced breaker trip, which pause the task by
/// design), an error carrying the
/// log when the runner itself failed (an unsaved meter, a refused post, a
/// lens with no prefix).
pub fn outcome(ended: &Ended, mut log: String) -> Result<String, String> {
    let _ = writeln!(log, "ended: {ended:?}");
    match ended {
        Ended::Halted(Halt::Unsaved(_) | Halt::Board(_) | Halt::NoPrefix(_)) => Err(log),
        Ended::Quiet | Ended::MaxPasses | Ended::Halted(Halt::Task(_)) | Ended::Paused(_) => {
            Ok(log)
        }
    }
}

/// The identity the scheduler's memory is saved under: the canonical BBP
/// store directory and the task id, NUL-separated (neither can hold one).
/// BBP task ids are scoped to their directory, so the id alone is not.
///
/// # Errors
///
/// A message when `dir` cannot be resolved, or when its canonical path is
/// not UTF-8: a lossy string could name two directories, so such a
/// directory is refused before any state is read or any provider asked.
pub fn board(dir: &Path, task: &str) -> Result<String, String> {
    let canonical = dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", dir.display()))?;
    let text = canonical
        .to_str()
        .ok_or_else(|| format!("BBP directory {} is not UTF-8", canonical.display()))?;
    Ok(format!("{text}\0{task}"))
}

/// The scheduler's memory saved for `board`, or a fresh one.
fn load_state(store: &Store, board: &str) -> Result<State, String> {
    match store.load_lens_state(board).map_err(|e| e.to_string())? {
        Some(saved) => State::restore(&saved).map_err(|e| format!("lens state: {e}")),
        None => Ok(State::default()),
    }
}

/// Run `o`: open the store and the sessions, build the providers, read
/// the prompts, then [`passes`] and, with `--alien`, the alien lens's
/// pass ([`Kit::sweep`]). Returns the log and how it ended.
pub fn run(o: &Options) -> Result<String, String> {
    let text = std::fs::read_to_string(&o.config)
        .map_err(|e| format!("cannot read {}: {e}", o.config.display()))?;
    let config = LensConfig::parse(&text).map_err(|e| format!("{}: {e}", o.config.display()))?;
    let base = o.config.parent().map(PathBuf::from).unwrap_or_default();
    let rules = match &o.rules {
        Some(p) => {
            std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?
        }
        None => String::new(),
    };
    let mut log = String::new();
    let grants = match assertions::load(&o.assertions) {
        Ok(g) => g,
        Err(e) => {
            let _ = writeln!(log, "assertions ignored: {e}");
            Grants::none()
        }
    };
    let mut registry = Registry::build(&config, &o.repo, &grants)?;
    let store = RefCell::new(Store::open(&o.state).map_err(|e| e.to_string())?);
    let board = board(&o.dir, &o.task)?;
    let mut state = load_state(&store.borrow(), &board)?;
    let loaded = store
        .borrow()
        .load_meter(&o.experiment)
        .map_err(|e| e.to_string())?;
    let mut meter = match loaded {
        Some(snap) => Meter::restore(config.ceilings, &snap),
        None => Meter::new(config.ceilings, 0),
    };
    let mut seats: BTreeMap<String, Box<dyn Seat>> = BTreeMap::new();
    let catalog = if config.compose {
        Some(Catalog::open(&base, &o.repo, Limits::default())?)
    } else {
        None
    };
    let mut prompts = Prompts {
        rules,
        lens: BTreeMap::new(),
        catalog,
    };
    let run = run_id()?;
    let _ = writeln!(log, "run {run}");
    for lens in &config.lenses {
        let path = base.join(&lens.prompt);
        let prompt = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        prompts.lens.insert(lens.id.clone(), prompt);
        let child = Child::spawn(&o.bbp, &o.dir, &o.task, &lens.id)?;
        seats.insert(
            lens.id.clone(),
            Box::new(Session::new(&lens.id, child, &run)),
        );
    }
    let mut alien = if o.alien {
        Some(Kit::open(&config, o, &board, &base, &run)?)
    } else {
        None
    };
    let experiment = o.experiment.clone();
    let mut save = |m: &Meter| {
        store
            .borrow_mut()
            .save_meter(&experiment, &m.snapshot())
            .map_err(|e| e.to_string())
    };
    let mut keep = |s: &State| {
        store
            .borrow_mut()
            .save_lens_state(&board, &s.saved())
            .map_err(|e| e.to_string())
    };
    let mut save_review = |r: &Review| {
        store
            .borrow_mut()
            .save_lens_review(&board, r)
            .map_err(|e| e.to_string())
    };
    let mut taints = store
        .borrow()
        .load_taints(&board)
        .map_err(|e| format!("taint table: {e}"))?;
    let mut save_taints = |t: &Taints| {
        store
            .borrow_mut()
            .save_taints(&board, t)
            .map_err(|e| e.to_string())
    };
    let mut pause = |p: &Pause| pause_task(&o.bbp, &o.dir, &o.task, p);
    let guard = Guard {
        breaker: config.breaker,
        run: &run,
        pause: &mut pause,
        taints: &mut taints,
        save_taints: &mut save_taints,
    };
    let mut read_records = || {
        let rows = store
            .borrow()
            .lens_reviews(&board)
            .map_err(|e| e.to_string())?;
        super::record::read(&o.bbp, &o.dir, &o.task, &rows)
    };
    let records = Records {
        save: &mut save_review,
        read: &mut read_records,
    };
    let metering = Metering {
        task: &o.task,
        meter: &mut meter,
        save: &mut save,
    };
    let lenses = Lenses {
        config: &config,
        seats: &mut seats,
        prompts: &prompts,
    };
    let ended = passes(
        lenses,
        &mut state,
        &mut keep,
        records,
        metering,
        guard,
        &mut registry,
        o.max_passes,
        &mut log,
    )?;
    if let Some(kit) = alien.as_mut() {
        if !matches!(ended, Ended::Quiet | Ended::MaxPasses) {
            let _ = writeln!(log, "alien: not run after {ended:?}");
            return outcome(&ended, log);
        }
        let ordinary = Ordinary {
            config: &config,
            state: &state,
            rules: &prompts.rules,
            taints: &taints,
        };
        let metering = Metering {
            task: &o.task,
            meter: &mut meter,
            save: &mut save,
        };
        let stopped = match kit.sweep(ordinary, &store, metering, &mut registry, &mut log) {
            Ok(stopped) => stopped,
            Err(e) => {
                let _ = writeln!(log, "alien: {e}");
                return Err(log);
            }
        };
        // A task ceiling pauses the task, as it does in `passes`.
        let ceiling = stopped.and_then(|s| Some((s, ceiling_reason(s)?)));
        if let Some((stop, reason)) = ceiling {
            let p = Pause::new(&run, reason, format!("alien: {stop}"), meter.calls(&o.task));
            if let Err(e) = pause_task(&o.bbp, &o.dir, &o.task, &p) {
                let _ = writeln!(log, "alien: task not paused ({reason}): {e}");
                return Err(log);
            }
            let _ = writeln!(log, "alien: paused: {}", p.detail);
        }
    }
    outcome(&ended, log)
}

/// `bbp pause` for `p`. BBP answers a refusal (a task no longer open) as
/// JSON on a zero exit, which is fine: the run ends either way.
fn pause_task(bbp: &Path, dir: &Path, task: &str, p: &Pause) -> Result<(), String> {
    let out = std::process::Command::new(bbp)
        .arg("pause")
        .arg("--dir")
        .arg(dir)
        .args(["--task", task, "--reason", p.reason, "--op", &p.op])
        .args(["--detail", &p.detail])
        .output()
        .map_err(|e| format!("cannot run {}: {e}", bbp.display()))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use orch_dispatch::{AgentError, ClassifiedError};
    use orch_lens::meter::Stop;
    use orch_lens::round::PostError;
    use orch_lens::{Answer, Post, Question, Usage};
    use rusty_bbp::{ClaimStatus, Direction, EvidenceBody, EvidenceKind, MsgId, Stakes};

    use super::*;

    /// Open the test store, waiting out a lock that a child process forked
    /// by another test may briefly inherit before it execs.
    fn open_store(d: &Path) -> Store {
        for _ in 0..100 {
            match Store::open(d) {
                Err(orch_store::StoreError::Locked(_)) => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                r => return r.expect("store"),
            }
        }
        panic!("{} stayed locked", d.display());
    }

    const CONFIG: &str = r#"{
      "providers": {
        "codex": {"kind": "codex-cli", "context_tokens": 32768,
                  "price": {"input": 0, "output": 0, "cached_input": 0}},
        "claude": {"kind": "claude-cli", "context_tokens": 32768,
                  "price": {"input": 0, "output": 0, "cached_input": 0}}
      },
      "ceilings": {"experiment_usd": 1, "task_usd": 1, "task_calls": 10},
      "lenses": [
        {"id": "adversary", "prompt": "p", "domains": ["*"],
         "may_post": {"evidence": ["counter"]},
         "models": {"routine": ["codex", "claude"], "contested": ["claude"]}},
        {"id": "operator", "prompt": "p", "domains": ["*"],
         "may_post": {"evidence": ["counter"]},
         "models": {"routine": ["codex", "claude"], "contested": ["claude"]}}
      ]
    }"#;

    fn config() -> LensConfig {
        LensConfig::parse(CONFIG).expect("config")
    }

    fn card(id: u64) -> Card {
        Card {
            id: MsgId(id),
            text: "Hash by size first".to_owned(),
            domains: vec!["dedup".to_owned()],
            status: ClaimStatus::Open,
            score: 0,
            accept_at: 8,
            epoch: 0,
            stakes: Stakes::Medium,
            reversible: true,
            answers: None,
            deps: vec![],
            spent: 0,
            budget: 16,
            evidence: vec![],
            candidates: vec![],
            current: None,
            class: orch_lens::DataClass::Internal,
        }
    }

    /// A board that shows fixed cards and records posts. Cards given as a
    /// BBP view are built the way a session builds them.
    struct FakeSeat {
        cards: Vec<Card>,
        views: Vec<(rusty_bbp::ClaimView, rusty_bbp::ClaimBody)>,
        refreshes: u32,
        posts: Vec<Post>,
        refresh_error: Option<String>,
        /// The brief, and from which refresh on it is revised (and the
        /// cards reopened) to the second one.
        briefs: [(Option<ArtId>, &'static str); 2],
        revise_at: Option<u32>,
        taints: Taints,
    }

    impl FakeSeat {
        fn new(cards: Vec<Card>) -> Self {
            Self {
                cards,
                views: vec![],
                refreshes: 0,
                posts: vec![],
                refresh_error: None,
                briefs: [(Some(ArtId(1)), "intent"), (Some(ArtId(9)), "intent v2")],
                revise_at: None,
                taints: Taints::default(),
            }
        }
    }

    impl FakeSeat {
        fn current(&self) -> (Option<ArtId>, &'static str) {
            let revised = self.revise_at.is_some_and(|at| self.refreshes >= at);
            self.briefs[usize::from(revised)]
        }
    }

    impl Board for FakeSeat {
        fn post(&mut self, _: &str, posts: &[Post]) -> Result<Vec<MsgId>, PostError> {
            let first = 100 + self.posts.len() as u64;
            self.posts.extend_from_slice(posts);
            Ok((first..first + posts.len() as u64).map(MsgId).collect())
        }
    }

    impl Seat for FakeSeat {
        fn refresh(&mut self, _: &LensConfig) -> Result<(), String> {
            self.refreshes += 1;
            if self.revise_at == Some(self.refreshes) {
                for c in &mut self.cards {
                    c.epoch += 1;
                }
            }
            self.refresh_error.clone().map_or(Ok(()), Err)
        }
        fn brief_id(&self) -> Option<ArtId> {
            self.current().0
        }
        fn brief(&mut self) -> Result<String, String> {
            Ok(self.current().1.to_owned())
        }
        fn taint(&mut self, taints: &Taints, _: &BTreeSet<String>) {
            self.taints = taints.clone();
        }
        fn rev(&self) -> u64 {
            u64::from(self.refreshes)
        }
        fn cards(&self, config: &LensConfig) -> Vec<Card> {
            let built = self
                .views
                .iter()
                .map(|(v, b)| Card::from_view(v, b, config));
            let mut cards: Vec<Card> = self.cards.iter().cloned().chain(built).collect();
            for c in &mut cards {
                self.taints.apply(c);
            }
            cards
        }
        fn lines(&mut self, _: &LensConfig) -> Result<Vec<Line>, String> {
            Ok(vec![])
        }
        fn board(&mut self) -> &mut dyn Board {
            self
        }
        fn candidate(&mut self, id: ArtId) -> Result<Diffs, String> {
            Err(format!("no candidate {id} in this test"))
        }
    }

    /// Every provider gives the same reply; `asked` is who was asked what.
    struct Echo(
        Result<Vec<Post>, ClassifiedError>,
        u32,
        Vec<(AgentId, String)>,
        Option<AgentId>,
        Vec<String>,
    );

    impl Model for Echo {
        fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
            self.1 += 1;
            self.4.push(q.prefix.clone());
            if let Some(id) = self.3 {
                self.2.push((id, q.body.clone()));
            }
            self.0.clone().map(|posts| Answer {
                reply: orch_lens::Reply::Posts(posts),
                usage: Usage::default(),
            })
        }
    }

    impl Models for Echo {
        fn get(&mut self, id: AgentId, _: Tools) -> Option<&mut dyn Model> {
            self.3 = Some(id);
            Some(self)
        }
    }

    fn echo(reply: Result<Vec<Post>, ClassifiedError>) -> Echo {
        Echo(reply, 0, vec![], None, vec![])
    }

    fn counter() -> Post {
        Post::Evidence {
            body: EvidenceBody {
                claim: MsgId(7),
                direction: Direction::Counter,
                kind: EvidenceKind::Opinion,
                strength: 1,
                note: "x".to_owned(),
            },
            evidence: vec![],
        }
    }

    fn go(
        seats: Vec<(&str, FakeSeat)>,
        echo: &mut Echo,
        task_calls: u32,
        max: u32,
    ) -> (Result<Ended, String>, String) {
        go_from(
            &mut State::default(),
            &mut |_| Ok(()),
            seats,
            echo,
            task_calls,
            max,
        )
    }

    /// [`go`] from `state`, keeping it through `keep`.
    fn go_from(
        state: &mut State,
        keep: &mut dyn FnMut(&State) -> Result<(), String>,
        seats: Vec<(&str, FakeSeat)>,
        echo: &mut Echo,
        task_calls: u32,
        max: u32,
    ) -> (Result<Ended, String>, String) {
        let mut save_review = |_: &Review| Ok(());
        let mut read_records = || Ok(BTreeMap::new());
        let records = Records {
            save: &mut save_review,
            read: &mut read_records,
        };
        go_with(state, keep, records, seats, echo, task_calls, max)
    }

    /// [`go_from`] with the track record's store.
    fn go_with(
        state: &mut State,
        keep: &mut dyn FnMut(&State) -> Result<(), String>,
        records: Records<'_>,
        seats: Vec<(&str, FakeSeat)>,
        echo: &mut Echo,
        task_calls: u32,
        max: u32,
    ) -> (Result<Ended, String>, String) {
        let mut pause = |_: &Pause| Ok(());
        let mut taints = Taints::default();
        let mut save_taints = |_: &Taints| Ok(());
        let guard = Guard {
            breaker: orch_lens::breaker::Breaker::default(),
            run: "r",
            pause: &mut pause,
            taints: &mut taints,
            save_taints: &mut save_taints,
        };
        go_guarded(state, keep, records, seats, echo, task_calls, max, guard)
    }

    /// [`go_with`] under `guard`.
    #[allow(clippy::too_many_arguments)]
    fn go_guarded(
        state: &mut State,
        keep: &mut dyn FnMut(&State) -> Result<(), String>,
        records: Records<'_>,
        seats: Vec<(&str, FakeSeat)>,
        echo: &mut Echo,
        task_calls: u32,
        max: u32,
        guard: Guard<'_>,
    ) -> (Result<Ended, String>, String) {
        let cfg = config();
        let mut boxed: BTreeMap<String, Box<dyn Seat>> = seats
            .into_iter()
            .map(|(l, s)| (l.to_owned(), Box::new(s) as Box<dyn Seat>))
            .collect();
        let prompts = Prompts {
            rules: String::new(),
            lens: cfg
                .lenses
                .iter()
                .map(|l| (l.id.clone(), l.id.clone()))
                .collect(),
            catalog: None,
        };
        let mut ceilings = cfg.ceilings;
        ceilings.task_calls = task_calls;
        let mut meter = Meter::new(ceilings, 0);
        let mut save = |_: &Meter| Ok(());
        let metering = Metering {
            task: "t",
            meter: &mut meter,
            save: &mut save,
        };
        let lenses = Lenses {
            config: &cfg,
            seats: &mut boxed,
            prompts: &prompts,
        };
        let mut log = String::new();
        let ended = passes(
            lenses, state, keep, records, metering, guard, echo, max, &mut log,
        );
        (ended, log)
    }

    /// What a guarded run asked for and saved.
    #[derive(Default)]
    struct Guarded {
        pauses: Vec<Pause>,
        taints: Taints,
        saves: u32,
    }

    /// Run `seats` under `breaker`, from `taints`, recording into a
    /// [`Guarded`].
    fn guarded(
        seats: Vec<(&str, FakeSeat)>,
        echo: &mut Echo,
        task_calls: u32,
        max: u32,
        breaker: orch_lens::breaker::Breaker,
        taints: Taints,
    ) -> (Result<Ended, String>, String, Guarded) {
        let mut g = Guarded {
            taints,
            ..Guarded::default()
        };
        let mut pauses = Vec::new();
        let mut saves = 0;
        let mut pause = |p: &Pause| {
            pauses.push(p.clone());
            Ok(())
        };
        let mut save_taints = |_: &Taints| {
            saves += 1;
            Ok(())
        };
        let guard = Guard {
            breaker,
            run: "r",
            pause: &mut pause,
            taints: &mut g.taints,
            save_taints: &mut save_taints,
        };
        let mut save_review = |_: &Review| Ok(());
        let mut read_records = || Ok(BTreeMap::new());
        let records = Records {
            save: &mut save_review,
            read: &mut read_records,
        };
        let (ended, log) = go_guarded(
            &mut State::default(),
            &mut |_| Ok(()),
            records,
            seats,
            echo,
            task_calls,
            max,
            guard,
        );
        g.pauses = pauses;
        g.saves = saves;
        (ended, log, g)
    }

    fn enforcing(window_calls: u32) -> orch_lens::breaker::Breaker {
        orch_lens::breaker::Breaker {
            window_calls,
            mode: orch_lens::breaker::Mode::Enforce,
            ..orch_lens::breaker::Breaker::default()
        }
    }

    #[test]
    fn an_enforced_breaker_pauses_a_task_whose_claims_do_not_move() {
        let mut e = echo(Ok(vec![counter()]));
        let (ended, log, g) = guarded(two_seats(), &mut e, 10, 5, enforcing(2), Taints::default());
        let Ok(Ended::Paused(detail)) = ended else {
            panic!("paused: {ended:?}\n{log}")
        };
        assert!(detail.contains("no claim settled"), "{detail}");
        assert_eq!(g.pauses.len(), 1);
        let p = &g.pauses[0];
        assert_eq!(p.reason, "breaker");
        assert_eq!(p.detail, detail);
        assert!(p.op.starts_with("orch-r-breaker-"), "{}", p.op);
        assert!(outcome(&Ended::Paused(detail), log).is_ok());
    }

    /// Lenses that follow different domains see different claims. The
    /// breaker marks every seat's claims together, so one lens's claims are
    /// not progress when the next lens's turn comes (#758 review).
    #[test]
    fn lenses_with_disjoint_views_share_one_breaker_mark() {
        let scored = |id: u64| Card {
            score: 4,
            ..card(id)
        };
        let seats = vec![
            ("adversary", FakeSeat::new(vec![scored(7)])),
            ("operator", FakeSeat::new(vec![scored(8)])),
        ];
        let mut e = echo(Ok(vec![counter()]));
        // The window closes as the second lens starts: the first lens's
        // claim must not read as moved from zero then.
        let (ended, log, g) = guarded(seats, &mut e, 10, 5, enforcing(1), Taints::default());
        assert!(matches!(ended, Ok(Ended::Paused(_))), "{ended:?}\n{log}");
        assert_eq!(g.pauses.len(), 1);
    }

    /// Of two seats' views of one claim, the newer revision's wins, so a
    /// seat that has not refreshed yet can't put back an older score.
    #[test]
    fn the_breaker_takes_each_claims_newest_view() {
        let at = |score: i64, refreshes: u32| {
            let mut seat = FakeSeat::new(vec![Card { score, ..card(7) }]);
            seat.refreshes = refreshes;
            Box::new(seat) as Box<dyn Seat>
        };
        let cfg = config();
        for (fresh, stale) in [("a", "c"), ("c", "a")] {
            let seats: BTreeMap<String, Box<dyn Seat>> =
                BTreeMap::from([(fresh.to_owned(), at(0, 2)), (stale.to_owned(), at(4, 1))]);
            let merged = task_cards(&seats, &cfg);
            assert_eq!(merged.len(), 1);
            assert_eq!(merged[0].score, 0, "{fresh} is newer");
        }
    }

    #[test]
    fn an_observing_breaker_logs_and_the_run_goes_on() {
        let observe = orch_lens::breaker::Breaker {
            window_calls: 2,
            ..orch_lens::breaker::Breaker::default()
        };
        let mut e = echo(Ok(vec![counter()]));
        let (ended, log, g) = guarded(two_seats(), &mut e, 10, 5, observe, Taints::default());
        assert!(matches!(ended, Ok(Ended::Quiet)), "{ended:?}\n{log}");
        assert!(log.contains("breaker would trip"), "{log}");
        assert!(g.pauses.is_empty());
    }

    #[test]
    fn a_task_ceiling_pauses_the_task() {
        let mut e = echo(Ok(vec![counter()]));
        let (ended, _, g) = guarded(
            two_seats(),
            &mut e,
            1,
            5,
            orch_lens::breaker::Breaker::default(),
            Taints::default(),
        );
        assert!(matches!(
            ended,
            Ok(Ended::Halted(Halt::Task(Stop::TaskCalls)))
        ));
        assert_eq!(g.pauses.len(), 1);
        assert_eq!(g.pauses[0].reason, "call_ceiling");
    }

    #[test]
    fn a_failed_pause_is_an_error() {
        let mut pause = |_: &Pause| Err("bbp gone".to_owned());
        let mut taints = Taints::default();
        let mut save_taints = |_: &Taints| Ok(());
        let guard = Guard {
            breaker: enforcing(1),
            run: "r",
            pause: &mut pause,
            taints: &mut taints,
            save_taints: &mut save_taints,
        };
        let mut save_review = |_: &Review| Ok(());
        let mut read_records = || Ok(BTreeMap::new());
        let records = Records {
            save: &mut save_review,
            read: &mut read_records,
        };
        let mut e = echo(Ok(vec![counter()]));
        let (ended, _) = go_guarded(
            &mut State::default(),
            &mut |_| Ok(()),
            records,
            two_seats(),
            &mut e,
            10,
            5,
            guard,
        );
        let err = ended.expect_err("a pause that failed stops the run");
        assert!(err.contains("bbp gone"), "{err}");
    }

    #[test]
    fn posts_are_tainted_and_the_table_is_saved() {
        let mut e = echo(Ok(vec![counter()]));
        let (ended, log, g) = guarded(
            two_seats(),
            &mut e,
            10,
            5,
            orch_lens::breaker::Breaker::default(),
            Taints::default(),
        );
        assert!(matches!(ended, Ok(Ended::Quiet)), "{ended:?}\n{log}");
        assert!(g.saves > 0);
        assert_eq!(
            g.taints.of(MsgId(100)),
            Some(orch_lens::DataClass::Internal)
        );
    }

    #[test]
    fn a_tainted_claim_goes_to_no_provider_below_its_class() {
        let mut e = echo(Ok(vec![counter()]));
        let tainted = Taints::from_entries([(7, orch_lens::DataClass::Sensitive)]);
        let (_, log, _) = guarded(
            two_seats(),
            &mut e,
            10,
            5,
            orch_lens::breaker::Breaker::default(),
            tainted,
        );
        assert_eq!(e.1, 0, "no cloud provider is asked: {log}");
        assert!(log.contains("skipped"), "{log}");
    }

    #[test]
    fn blind_reviews_are_saved_and_each_lens_sees_its_record() {
        let mut saved: Vec<Review> = Vec::new();
        let mut save_review = |r: &Review| {
            saved.push(r.clone());
            Ok(())
        };
        let mut reads = 0;
        let mut read_records = || {
            reads += 1;
            let mut t = Tally::default();
            t.add(orch_lens::record::Scored::Points {
                points: 3,
                pivotal: false,
            });
            Ok(BTreeMap::from([("adversary".to_owned(), t)]))
        };
        let records = Records {
            save: &mut save_review,
            read: &mut read_records,
        };
        let mut e = echo(Ok(vec![counter()]));
        let (ended, log) = go_with(
            &mut State::default(),
            &mut |_| Ok(()),
            records,
            two_seats(),
            &mut e,
            10,
            1,
        );
        assert!(ended.is_ok(), "{log}");
        assert_eq!(reads, 1, "read once per pass");
        let line = "record: 3 points over 1 blind reviews (1 scored, 0 pivotal)";
        let bodies: Vec<&String> = e.2.iter().map(|(_, b)| b).collect();
        assert_eq!(
            bodies.iter().filter(|b| b.contains(line)).count(),
            1,
            "{bodies:?}"
        );
        // Each job's write-ahead, then its row once posted.
        let (ahead, rows): (Vec<Review>, Vec<Review>) = saved.into_iter().partition(|r| r.pending);
        assert_eq!(ahead.len(), 2, "{ahead:?}");
        assert!(ahead.iter().all(|r| r.items.is_empty()));
        let mut lenses: Vec<&str> = rows.iter().map(|r| r.lens.as_str()).collect();
        lenses.sort_unstable();
        assert_eq!(lenses, ["adversary", "operator"]);
        assert!(rows
            .iter()
            .all(|r| r.claim == MsgId(7) && r.items.len() == 1));
    }

    #[test]
    fn an_unreadable_record_is_logged_and_the_run_goes_on() {
        let mut save_review = |_: &Review| Ok(());
        let mut read_records = || Err("bbp why: no such task".to_owned());
        let records = Records {
            save: &mut save_review,
            read: &mut read_records,
        };
        let mut e = echo(Ok(vec![counter()]));
        let (ended, log) = go_with(
            &mut State::default(),
            &mut |_| Ok(()),
            records,
            two_seats(),
            &mut e,
            10,
            1,
        );
        assert!(ended.is_ok(), "{log}");
        assert!(
            log.contains("track record not read: bbp why: no such task"),
            "{log}"
        );
        assert_eq!(e.1, 2, "both lenses still asked");
    }

    #[test]
    fn a_failed_row_save_keeps_no_state_so_the_job_reruns() {
        // The write-ahead lands; the row after the posts doesn't.
        let mut save_review = |r: &Review| {
            if r.pending {
                return Ok(());
            }
            Err("disk full".to_owned())
        };
        let mut read_records = || Ok(BTreeMap::new());
        let records = Records {
            save: &mut save_review,
            read: &mut read_records,
        };
        let mut kept = 0;
        let mut keep = |_: &State| {
            kept += 1;
            Ok(())
        };
        let mut e = echo(Ok(vec![counter()]));
        let (ended, log) = go_with(
            &mut State::default(),
            &mut keep,
            records,
            two_seats(),
            &mut e,
            10,
            1,
        );
        let err = ended.expect_err("a lost row stops the run");
        assert!(err.contains("track record not saved: disk full"), "{err}");
        assert_eq!(kept, 0, "state not kept past an unsaved row: {log}");
    }

    /// A crash between the row saves and the state keep: the restart from
    /// the last kept state reruns the jobs, and the reopened store holds each
    /// job once, with every item it posted.
    #[test]
    fn a_crash_before_the_keep_reruns_and_records_each_job_once() {
        let d = std::env::temp_dir().join(format!("orch-lens-rerun-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let run = |keep: &mut dyn FnMut(&State) -> Result<(), String>| {
            let mut store = open_store(&d);
            let mut save_review =
                |r: &Review| store.save_lens_review("b", r).map_err(|e| e.to_string());
            let mut read_records = || Ok(BTreeMap::new());
            let records = Records {
                save: &mut save_review,
                read: &mut read_records,
            };
            let mut e = echo(Ok(vec![counter()]));
            go_with(
                &mut State::default(),
                keep,
                records,
                two_seats(),
                &mut e,
                10,
                1,
            )
            .0
        };
        let crashed = run(&mut |_| Err("killed".to_owned()));
        assert!(crashed.is_err());
        assert!(run(&mut |_| Ok(())).is_ok());
        let rows = open_store(&d).lens_reviews("b").expect("rows");
        let lenses: Vec<&str> = rows.iter().map(|r| r.lens.as_str()).collect();
        assert_eq!(lenses, ["adversary", "operator"], "{rows:?}");
        assert!(rows.iter().all(|r| !r.items.is_empty()), "{rows:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A crash after the first lens posts but before its row is saved: the
    /// store holds only its write-ahead. The restart asks it again, and the
    /// first attempt's post is in no row, so that job ends unsure and is
    /// never scored (D92 write-ahead). The other lens, never reached before
    /// the crash, is not.
    #[test]
    fn a_crash_after_posting_leaves_the_rerun_job_unsure() {
        let d = std::env::temp_dir().join(format!("orch-lens-ahead-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let run = |crash: bool| {
            let mut store = open_store(&d);
            let mut save_review = |r: &Review| {
                if crash && !r.pending {
                    return Err("killed".to_owned());
                }
                store.save_lens_review("b", r).map_err(|e| e.to_string())
            };
            let mut read_records = || Ok(BTreeMap::new());
            let records = Records {
                save: &mut save_review,
                read: &mut read_records,
            };
            let mut e = echo(Ok(vec![counter()]));
            go_with(
                &mut State::default(),
                &mut |_| Ok(()),
                records,
                two_seats(),
                &mut e,
                10,
                1,
            )
            .0
        };
        assert!(run(true).is_err());
        let rows = open_store(&d).lens_reviews("b").expect("rows");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(rows[0].pending && !rows[0].unsure, "{rows:?}");
        assert!(run(false).is_ok());
        let rows = open_store(&d).lens_reviews("b").expect("rows");
        let state: Vec<(&str, bool, bool, usize)> = rows
            .iter()
            .map(|r| (r.lens.as_str(), r.pending, r.unsure, r.items.len()))
            .collect();
        assert_eq!(
            state,
            [("adversary", false, true, 1), ("operator", false, false, 1)],
            "the first post is unknown"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A crash after a post that settled the claim, before its row is
    /// saved: the restart sees the claim settled, so the job is never asked
    /// again. Its row stays pending, so it stays in the denominator and is
    /// never scored; here the lost item crossed the line, and the pending
    /// row counts as pivotal where a row without the marker would not.
    #[test]
    fn a_crash_after_a_settling_post_keeps_the_job_pending_and_unscored() {
        use orch_lens::record::{score, Change, Item, Rule, Scored, Step, Trail};
        let d = std::env::temp_dir().join(format!("orch-lens-settled-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let run = |crash: bool, status: ClaimStatus| {
            let mut store = open_store(&d);
            let mut save_review = |r: &Review| {
                if crash && !r.pending {
                    return Err("killed".to_owned());
                }
                store.save_lens_review("b", r).map_err(|e| e.to_string())
            };
            let mut read_records = || Ok(BTreeMap::new());
            let records = Records {
                save: &mut save_review,
                read: &mut read_records,
            };
            let settled = Card { status, ..card(7) };
            let mut e = echo(Ok(vec![counter()]));
            let ended = go_with(
                &mut State::default(),
                &mut |_| Ok(()),
                records,
                vec![
                    ("adversary", FakeSeat::new(vec![settled.clone()])),
                    ("operator", FakeSeat::new(vec![settled])),
                ],
                &mut e,
                10,
                1,
            )
            .0;
            (ended, e.1)
        };
        assert!(run(true, ClaimStatus::Open).0.is_err());
        let (ended, asked) = run(false, ClaimStatus::Dropped);
        assert!(ended.is_ok(), "{ended:?}");
        assert_eq!(asked, 0, "a settled claim's job isn't asked again");
        let rows = open_store(&d).lens_reviews("b").expect("rows");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(rows[0].pending && rows[0].items.is_empty(), "{rows:?}");
        let item = |msg: u64, author: &str, delta: i64| {
            Change::Item(Item {
                msg: MsgId(msg),
                author: author.into(),
                direction: Direction::Counter,
                grounded: true,
                cites: vec![],
                delta,
                scoring: true,
            })
        };
        let changes = [
            item(2, "a", -6),
            item(41, "adversary", -3),
            Change::Status(Rule::Line {
                at: -8,
                authors: 2,
                min_authors: 2,
                holdout: None,
            }),
        ];
        let mut so_far = 0;
        let steps = changes
            .into_iter()
            .map(|change| {
                if let Change::Item(i) = &change {
                    so_far += i.delta;
                }
                let status = match change {
                    Change::Status(_) => ClaimStatus::Dropped,
                    _ => ClaimStatus::Open,
                };
                Step {
                    change,
                    status,
                    score: so_far,
                    epoch: 0,
                }
            })
            .collect();
        let trail = Trail { steps };
        assert_eq!(score(&rows[0], &trail), Scored::Unscored { pivotal: true });
        let unmarked = Review {
            pending: false,
            ..rows[0].clone()
        };
        assert_eq!(
            score(&unmarked, &trail),
            Scored::Unscored { pivotal: false }
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A known prefix plus an unknown landed post, then a retry that posts
    /// nothing: an empty answer or a refusal. The retry writes ahead over
    /// the interrupted attempt's pending row, so the job stays unsure with
    /// the ids it had, whatever the retry ends with.
    #[test]
    fn a_retry_that_posts_nothing_keeps_an_interrupted_job_unsure() {
        let retries = [
            ("empty", Ok(vec![])),
            (
                "refused",
                Err(ClassifiedError::Permanent(AgentError("bad".into()))),
            ),
        ];
        for (name, retry) in retries {
            let d = std::env::temp_dir()
                .join(format!("orch-lens-recover-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            let run = |crash: bool, reply: Result<Vec<Post>, ClassifiedError>| {
                let mut store = open_store(&d);
                let mut save_review = |r: &Review| {
                    if crash && !r.pending {
                        return Err("killed".to_owned());
                    }
                    store.save_lens_review("b", r).map_err(|e| e.to_string())
                };
                let mut read_records = || Ok(BTreeMap::new());
                let records = Records {
                    save: &mut save_review,
                    read: &mut read_records,
                };
                let mut e = echo(reply);
                go_with(
                    &mut State::default(),
                    &mut |_| Ok(()),
                    records,
                    two_seats(),
                    &mut e,
                    10,
                    1,
                )
                .0
            };
            assert!(run(false, Ok(vec![counter()])).is_ok(), "known prefix");
            assert!(run(true, Ok(vec![counter()])).is_err(), "lost post");
            assert!(run(false, retry).is_ok(), "{name}");
            let rows = open_store(&d).lens_reviews("b").expect("rows");
            let state: Vec<(&str, bool, bool, usize)> = rows
                .iter()
                .map(|r| (r.lens.as_str(), r.pending, r.unsure, r.items.len()))
                .collect();
            assert_eq!(
                state,
                [("adversary", false, true, 1), ("operator", false, false, 1)],
                "{name}: the lost post keeps the job unsure"
            );
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    fn two_seats() -> Vec<(&'static str, FakeSeat)> {
        vec![
            ("adversary", FakeSeat::new(vec![card(7)])),
            ("operator", FakeSeat::new(vec![card(7)])),
        ]
    }

    #[test]
    fn a_restart_from_the_kept_memory_does_not_ask_again() {
        let mut echo = echo(Ok(vec![counter()]));
        let mut kept = Vec::new();
        let mut keep = |s: &State| {
            kept.push(s.saved());
            Ok(())
        };
        let (ended, _) = go_from(
            &mut State::default(),
            &mut keep,
            two_seats(),
            &mut echo,
            10,
            20,
        );
        assert_eq!(ended, Ok(Ended::Quiet));
        assert_eq!(echo.1, 2);
        assert_eq!(kept.len(), 4, "kept after every lens's round, two passes");
        let last = kept.last().expect("kept");
        assert_eq!(last.progress.len(), 2, "both blind passes remembered");
        let mut restored = State::restore(last).expect("restore");
        let (ended, log) = go_from(
            &mut restored,
            &mut |_| Ok(()),
            two_seats(),
            &mut echo,
            10,
            20,
        );
        assert_eq!(ended, Ok(Ended::Quiet));
        assert_eq!(echo.1, 2, "no lens asked again: {log}");
    }

    #[test]
    fn a_same_named_task_in_another_bbp_directory_starts_fresh() {
        let root = std::env::temp_dir().join(format!("orch-lens-board-{}", std::process::id()));
        let (a, b, orch) = (root.join("a"), root.join("b"), root.join("orch"));
        for d in [&a, &b] {
            std::fs::create_dir_all(d).expect("dir");
        }
        let (board_a, board_b) = (
            board(&a, "task-1").expect("a"),
            board(&b, "task-1").expect("b"),
        );
        assert_ne!(board_a, board_b);
        let relative = a.join("..").join("a");
        assert_eq!(board(&relative, "task-1").expect("same dir"), board_a);
        let store = RefCell::new(Store::open(&orch).expect("store"));
        let mut echo = echo(Ok(vec![counter()]));
        let mut keep = |s: &State| {
            store
                .borrow_mut()
                .save_lens_state(&board_a, &s.saved())
                .map_err(|e| e.to_string())
        };
        let (ended, _) = go_from(
            &mut State::default(),
            &mut keep,
            two_seats(),
            &mut echo,
            10,
            20,
        );
        assert_eq!(ended, Ok(Ended::Quiet));
        assert_eq!(echo.1, 2);

        // Board B, same task id and claim ids: nothing of A's carries over.
        let mut fresh = load_state(&store.borrow(), &board_b).expect("load b");
        assert_eq!(fresh, State::default());
        let (_, _) = go_from(&mut fresh, &mut |_| Ok(()), two_seats(), &mut echo, 10, 20);
        assert_eq!(echo.1, 4, "B makes its own blind passes");

        // Restarting A resumes where it stopped.
        let mut resumed = load_state(&store.borrow(), &board_a).expect("load a");
        assert_eq!(resumed.progress.len(), 2);
        let (_, _) = go_from(
            &mut resumed,
            &mut |_| Ok(()),
            two_seats(),
            &mut echo,
            10,
            20,
        );
        assert_eq!(echo.1, 4, "A asks nothing again");
        drop(store);
        std::fs::remove_dir_all(&root).expect("clean up");
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_whose_path_is_not_utf8_is_refused() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let root = std::env::temp_dir().join(format!("orch-lens-utf8-{}", std::process::id()));
        let named =
            |last: u8| root.join(OsString::from_vec(vec![b'b', b'o', b'a', b'r', b'd', last]));
        let (x80, x81) = (named(0x80), named(0x81));
        for d in [&x80, &x81] {
            std::fs::create_dir_all(d).expect("dir");
        }
        assert_ne!(
            x80.canonicalize().expect("x80"),
            x81.canonicalize().expect("x81")
        );
        for d in [&x80, &x81] {
            let e = board(d, "task-1").expect_err("lossy path refused");
            assert!(e.contains("not UTF-8"), "{e}");
        }
        std::fs::remove_dir_all(&root).expect("clean up");
    }

    #[test]
    fn a_claim_reopened_between_runs_is_reviewed_again() {
        let mut echo = echo(Ok(vec![counter()]));
        let mut kept = None;
        let mut keep = |s: &State| {
            kept = Some(s.saved());
            Ok(())
        };
        let (ended, _) = go_from(
            &mut State::default(),
            &mut keep,
            two_seats(),
            &mut echo,
            10,
            20,
        );
        assert_eq!(ended, Ok(Ended::Quiet));
        assert_eq!(echo.1, 2);
        let mut restored = State::restore(&kept.expect("kept")).expect("restore");
        let mut reopened = card(7);
        reopened.epoch = 1;
        let seats = vec![
            ("adversary", FakeSeat::new(vec![reopened.clone()])),
            ("operator", FakeSeat::new(vec![reopened])),
        ];
        let (ended, log) = go_from(&mut restored, &mut |_| Ok(()), seats, &mut echo, 10, 20);
        assert_eq!(ended, Ok(Ended::Quiet));
        assert_eq!(echo.1, 4, "each lens looks at the new epoch blind: {log}");
    }

    #[test]
    fn a_revised_intent_rebuilds_the_prefix_before_the_next_call() {
        let mut echo = echo(Ok(vec![counter()]));
        // Revised (and the claim reopened) at the second pass's refresh,
        // the third: the run refreshes every seat once before pass 1.
        let seats = ["adversary", "operator"].map(|lens| {
            let mut seat = FakeSeat::new(vec![card(7)]);
            seat.revise_at = Some(3);
            (lens, seat)
        });
        let (ended, log) = go(seats.into(), &mut echo, 10, 20);
        assert_eq!(ended, Ok(Ended::Quiet), "{log}");
        assert_eq!(
            echo.4.len(),
            4,
            "each lens asked blind on each epoch: {log}"
        );
        for (i, p) in echo.4.iter().enumerate() {
            let want = if i < 2 { "intent" } else { "intent v2" };
            assert!(
                p.starts_with(&format!("# Intent\n{want}\n")),
                "call {i} carries the intent of its pass: {p}"
            );
        }
    }

    fn seats_with(cards: &[Card]) -> Vec<(&'static str, FakeSeat)> {
        vec![
            ("adversary", FakeSeat::new(cards.to_vec())),
            ("operator", FakeSeat::new(cards.to_vec())),
        ]
    }

    /// Both passes done on `before` (blind found nothing, the look-harder
    /// re-ask found nothing) and kept; then a restart onto `after`. Gives
    /// the asks before and after, and the restart's log.
    fn restart_after_both_passes(before: &[Card], after: &[Card]) -> (u32, u32, String) {
        let mut echo = echo(Ok(vec![]));
        let mut kept = None;
        let mut keep = |s: &State| {
            kept = Some(s.saved());
            Ok(())
        };
        let (ended, _) = go_from(
            &mut State::default(),
            &mut keep,
            seats_with(before),
            &mut echo,
            10,
            20,
        );
        assert_eq!(ended, Ok(Ended::Quiet));
        let first = echo.1;
        let saved = kept.expect("kept");
        assert!(saved
            .progress
            .iter()
            .all(|p| p.blind.is_some() && p.look_harder.is_some()));
        let mut restored = State::restore(&saved).expect("restore");
        let (ended, log) = go_from(
            &mut restored,
            &mut |_| Ok(()),
            seats_with(after),
            &mut echo,
            10,
            20,
        );
        assert_eq!(ended, Ok(Ended::Quiet));
        (first, echo.1 - first, log)
    }

    #[test]
    fn a_reopen_after_both_passes_gets_both_passes_again_and_other_claims_keep_theirs() {
        let both = [card(7), card(8)];
        let (first, again, log) = restart_after_both_passes(&both, &both);
        assert_eq!(first, 8, "blind and look-harder, two lenses, two claims");
        assert_eq!(again, 0, "same epoch resumes with nothing to do: {log}");
        let mut reopened = card(7);
        reopened.epoch = 1;
        let (_, again, log) = restart_after_both_passes(&both, &[reopened, card(8)]);
        assert_eq!(again, 4, "claim 7's new epoch, both passes per lens: {log}");
        assert_eq!(log.matches("msg:7").count(), 4, "{log}");
        assert_eq!(
            log.matches("msg:8").count(),
            0,
            "claim 8 keeps its work: {log}"
        );
    }

    /// Question 3 with candidates 10 and 11, proposals settled.
    fn question_board(epoch_10: u32) -> (Vec<Card>, State) {
        let mut q = card(3);
        q.domains = vec![orch_lens::schedule::QUESTION_DOMAIN.to_owned()];
        q.candidates = vec![MsgId(10), MsgId(11)];
        let candidate = |n: u64, epoch: u32| {
            let mut c = card(n);
            c.answers = Some(MsgId(3));
            c.epoch = epoch;
            c
        };
        let mut st = State::default();
        let p = st.questions.entry(MsgId(3)).or_default();
        p.settled = [0, 1].into();
        p.rebel_landed = true;
        (vec![q, candidate(10, epoch_10), candidate(11, 0)], st)
    }

    #[test]
    fn a_reopened_candidate_is_not_held_back_by_its_old_attack_count() {
        let mut echo = echo(Ok(vec![]));
        let (cards, mut st) = question_board(0);
        let mut kept = None;
        let mut keep = |s: &State| {
            kept = Some(s.saved());
            Ok(())
        };
        let (ended, _) = go_from(&mut st, &mut keep, seats_with(&cards), &mut echo, 10, 20);
        assert_eq!(ended, Ok(Ended::Quiet));
        assert_eq!(
            echo.1, 8,
            "blind and look-harder, two lenses, two candidates"
        );
        let mut saved = kept.expect("kept");
        // Make the old counts unequal: 10 is ahead of 11.
        let attacks = &mut saved.questions[0].attacks;
        attacks.iter_mut().find(|a| a.0 == 10).expect("10").2 += 1;

        let mut same = State::restore(&saved).expect("restore");
        let (ended, log) = go_from(
            &mut same,
            &mut |_| Ok(()),
            seats_with(&cards),
            &mut echo,
            10,
            20,
        );
        assert_eq!(ended, Ok(Ended::Quiet));
        assert_eq!(echo.1, 8, "same epoch: nothing to do: {log}");

        let (reopened, _) = question_board(1);
        let mut restored = State::restore(&saved).expect("restore");
        let (ended, log) = go_from(
            &mut restored,
            &mut |_| Ok(()),
            seats_with(&reopened),
            &mut echo,
            10,
            20,
        );
        assert_eq!(ended, Ok(Ended::Quiet));
        assert_eq!(
            echo.1, 12,
            "10's new epoch: blind and look-harder per lens: {log}"
        );
        assert_eq!(log.matches("msg:10 Review").count(), 2, "{log}");
        assert_eq!(log.matches("msg:10 LookHarder").count(), 2, "{log}");
        assert_eq!(log.matches("msg:11").count(), 0, "{log}");
    }

    #[test]
    fn memory_that_cannot_be_kept_is_an_error() {
        let mut echo = echo(Ok(vec![counter()]));
        let (ended, _) = go_from(
            &mut State::default(),
            &mut |_| Err("disk full".to_owned()),
            two_seats(),
            &mut echo,
            10,
            20,
        );
        assert_eq!(ended, Err("lens state not saved: disk full".to_owned()));
        assert_eq!(echo.1, 1, "stops after the first round");
    }

    #[test]
    fn passes_run_until_quiet() {
        let mut echo = echo(Ok(vec![counter()]));
        let seats = vec![
            ("adversary", FakeSeat::new(vec![card(7)])),
            ("operator", FakeSeat::new(vec![card(7)])),
        ];
        let (ended, log) = go(seats, &mut echo, 10, 20);
        assert_eq!(ended, Ok(Ended::Quiet));
        assert_eq!(echo.1, 2, "one blind pass per lens, then quiet: {log}");
        assert_eq!(log.lines().count(), 2, "{log}");
    }

    #[test]
    fn a_failing_provider_stops_at_max_passes() {
        let mut echo = echo(Err(ClassifiedError::Transient(AgentError("busy".into()))));
        let seats = vec![
            ("adversary", FakeSeat::new(vec![card(7)])),
            ("operator", FakeSeat::new(vec![card(7)])),
        ];
        let (ended, _) = go(seats, &mut echo, 100, 3);
        assert_eq!(ended, Ok(Ended::MaxPasses));
        assert_eq!(echo.1, 6, "two lenses, three passes");
    }

    #[test]
    fn a_halted_round_ends_the_run() {
        let mut echo = echo(Ok(vec![]));
        let seats = vec![
            ("adversary", FakeSeat::new(vec![card(7)])),
            ("operator", FakeSeat::new(vec![card(7)])),
        ];
        let (ended, _) = go(seats, &mut echo, 1, 20);
        assert_eq!(ended, Ok(Ended::Halted(Halt::Task(Stop::TaskCalls))));
        assert_eq!(echo.1, 1);
    }

    #[test]
    fn a_missing_seat_or_a_failed_refresh_is_an_error() {
        let mut echo = echo(Ok(vec![]));
        let (ended, _) = go(
            vec![("adversary", FakeSeat::new(vec![]))],
            &mut echo,
            10,
            20,
        );
        assert_eq!(ended, Err("no BBP session for lens operator".to_owned()));
        let mut bad = FakeSeat::new(vec![]);
        bad.refresh_error = Some("bbp mcp exited".to_owned());
        let seats = vec![("adversary", bad), ("operator", FakeSeat::new(vec![]))];
        let (ended, _) = go(seats, &mut echo, 10, 20);
        assert_eq!(ended, Err("adversary: bbp mcp exited".to_owned()));
    }

    /// A split claim at score +5 with four evidence items, accepting at
    /// `accept_at`: under half of a 20-point line, not under half of 8.
    fn split_view(accept_at: i64) -> (rusty_bbp::ClaimView, rusty_bbp::ClaimBody) {
        let item = |m: u64, author: &str, direction: Direction| rusty_bbp::claims::EvidenceView {
            msg: MsgId(m),
            author: rusty_bbp::PrincipalId(author.to_owned()),
            direction,
            kind: EvidenceKind::Cited,
            strength: 1,
            note: String::new(),
            delta: 0,
            epoch: 0,
            scoring: true,
        };
        let view = rusty_bbp::ClaimView {
            id: MsgId(7),
            text: "Hash by size first".to_owned(),
            domains: vec!["dedup".to_owned()],
            status: ClaimStatus::Open,
            score: 5,
            accept_at,
            confidence: 50,
            epoch: 0,
            spent: 4,
            budget: 16,
            evidence: vec![
                item(20, "a", Direction::Support),
                item(21, "b", Direction::Support),
                item(22, "c", Direction::Counter),
                item(23, "d", Direction::Support),
            ],
            deps: vec![],
            candidates: vec![],
            sole_candidate: false,
            current: None,
            failing: vec![],
            tests_changed: vec![],
            endorsement: None,
            holdout_due: false,
            intent: None,
            human_touched: false,
        };
        let body = rusty_bbp::ClaimBody {
            text: view.text.clone(),
            domains: view.domains.clone(),
            deps: vec![],
            answers: None,
            supersedes: None,
            stakes: Stakes::Medium,
            reversible: true,
        };
        (view, body)
    }

    fn asked_with(accept_at: i64) -> Vec<(AgentId, String)> {
        let mut seat = FakeSeat::new(vec![]);
        seat.views.push(split_view(accept_at));
        let mut e = echo(Ok(vec![]));
        let seats = vec![("adversary", seat), ("operator", FakeSeat::new(vec![]))];
        let (ended, log) = go(seats, &mut e, 10, 1);
        assert!(ended.is_ok(), "{ended:?} {log}");
        e.2
    }

    #[test]
    fn the_cards_accept_line_sets_the_tier_and_the_card() {
        let id = |s: &str| AgentId::new(s).expect("id");
        let default = asked_with(8);
        assert_eq!(default[0].0, id("codex"), "routine under the default 8");
        assert!(default[0].1.contains("(accepts at +8)"), "{}", default[0].1);
        let strict = asked_with(20);
        assert_eq!(strict[0].0, id("claude"), "contested under the task's 20");
        assert!(strict[0].1.contains("(accepts at +20)"), "{}", strict[0].1);
    }

    #[test]
    fn the_command_fails_on_the_runners_own_halts() {
        let ok = |e: Ended| outcome(&e, "log\n".to_owned());
        assert!(ok(Ended::Quiet).is_ok_and(|l| l.starts_with("log\n")));
        assert!(ok(Ended::MaxPasses).is_ok());
        assert!(ok(Ended::Halted(Halt::Task(Stop::TaskCalls))).is_ok());
        for h in [
            Halt::Unsaved("disk full".to_owned()),
            Halt::Board("refused".to_owned()),
            Halt::NoPrefix("operator".to_owned()),
        ] {
            let err = ok(Ended::Halted(h.clone())).expect_err("a failure");
            assert!(err.starts_with("log\n"), "the log is kept: {err}");
            assert!(err.contains(&format!("{h:?}")), "{err}");
        }
    }

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn options_need_the_task_and_take_defaults() {
        let o = options(
            argv("run c.json --bbp /b --dir /d --task t1 --state /s --experiment dedup"),
            PathBuf::from("/cwd"),
        )
        .expect("options");
        assert_eq!(o.task, "t1");
        assert_eq!(o.max_passes, DEFAULT_MAX_PASSES);
        assert_eq!(o.assertions, PathBuf::from(DEFAULT_ASSERTIONS));
        assert_eq!(o.repo, PathBuf::from("/cwd"));
        assert_eq!(o.rules, None);
        assert!(!o.alien, "the alien lens is off by default");
        let on = options(
            argv("run c.json --alien --bbp /b --dir /d --task t1 --state /s --experiment e"),
            PathBuf::new(),
        )
        .expect("options");
        assert!(on.alien);
        let twice = options(argv("run c.json --alien --alien"), PathBuf::new());
        assert!(matches!(twice, Err(ArgsError::Usage(m)) if m.contains("duplicate --alien")));
        let missing = options(
            argv("run c.json --bbp /b --dir /d --state /s --experiment e"),
            PathBuf::new(),
        );
        assert!(matches!(missing, Err(ArgsError::Usage(m)) if m.contains("--task")));
        let dup = options(argv("run c.json --task a --task b"), PathBuf::new());
        assert!(matches!(dup, Err(ArgsError::Usage(m)) if m.contains("duplicate")));
        let zero = options(
            argv("run c.json --bbp /b --dir /d --task t --state /s --experiment e --max-passes 0"),
            PathBuf::new(),
        );
        assert!(matches!(zero, Err(ArgsError::Usage(m)) if m.contains("--max-passes")));
        assert!(matches!(
            options(argv("check c.json"), PathBuf::new()),
            Err(ArgsError::Usage(_))
        ));
    }

    #[test]
    fn composed_prefixes_differ_by_claim_domain_from_one_catalog() {
        let root = std::env::temp_dir().join(format!("orch-run-compose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (factory, repo) = (root.join("factory"), root.join("repo"));
        for (dir, rel, text) in [
            (
                &factory,
                "domains.json",
                r#"{"rust": {"knowledge": ["k/rust.md"]}, "tty": {"knowledge": ["k/tty.md"]}}"#,
            ),
            (&factory, "k/rust.md", "Ownership first."),
            (&factory, "k/tty.md", "VT is a state machine."),
            (&repo, "DOMAIN.md", "Dedup 20 TB."),
        ] {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
            std::fs::write(path, text).expect("write");
        }
        let prompts = Prompts {
            rules: "rules".to_owned(),
            lens: BTreeMap::from([("adversary".to_owned(), "find faults".to_owned())]),
            catalog: Some(Catalog::open(&factory, &repo, Limits::default()).expect("opens")),
        };
        let mut prefixes = Prefixes::default();
        prefixes
            .intent
            .insert("adversary".to_owned(), "the brief".to_owned());
        let on = |id: u64, domains: &[&str]| Card {
            domains: domains.iter().map(|d| (*d).to_owned()).collect(),
            ..card(id)
        };
        let cards = [on(1, &["rust"]), on(2, &["tty"]), on(3, &["rust"])];
        let mut log = String::new();
        let got = prefixes
            .composed("adversary", &cards, &prompts, &mut log)
            .expect("composes");
        assert_eq!(got.len(), 2, "one prefix per domain set");
        let rust = &got[&prefix_key("adversary", &["rust".to_owned()])];
        let tty = &got[&prefix_key("adversary", &["tty".to_owned()])];
        assert_ne!(rust, tty);
        assert!(rust.contains("Ownership first.") && rust.contains("# Project\nDedup 20 TB."));
        assert!(rust.starts_with("# Intent\nthe brief\n"));
        assert!(tty.contains("VT is a state machine.") && !tty.contains("Ownership"));
        let off = Prompts {
            catalog: None,
            ..prompts
        };
        assert!(prefixes
            .composed("adversary", &cards, &off, &mut log)
            .expect("ok")
            .is_empty());
    }
}
