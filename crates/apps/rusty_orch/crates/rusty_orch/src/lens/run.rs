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

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use orch_core::AgentId;
use orch_lens::call::{Ctx, Models};
use orch_lens::card::Card;
use orch_lens::config::Tools;
use orch_lens::context::{prefix, Line};
use orch_lens::meter::Meter;
use orch_lens::round::{round, Board, Halt, View};
use orch_lens::schedule::State;
use orch_lens::{LensConfig, Model};
use orch_store::Store;

use super::assertions::{self, Grants};
use super::bbp::{run_id, Child, Rpc, Session};
use super::check::DEFAULT_ASSERTIONS;
use super::registry::Registry;
use crate::args::ArgsError;

/// Passes when none is named.
pub const DEFAULT_MAX_PASSES: u32 = 20;

pub const USAGE: &str = "usage: rusty_orch lens run <config.json> --bbp <bbp binary> --dir <bbp dir> --task <task>\n           --state <orch state dir> --experiment <name> [--rules <file>] [--assertions <file>]\n           [--repo <dir>] [--max-passes <n>]\n\nRun the configured lenses on one BBP claims task until a pass starts no job. Each lens\nreads and posts through its own `bbp mcp` session. Calls the configured models: their\nadmission follows the operator's assertions, as `lens check` reports.";

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
    })
}

/// One lens's way onto the board.
pub trait Seat {
    fn refresh(&mut self, config: &LensConfig) -> Result<(), String>;
    fn cards(&self, config: &LensConfig) -> Vec<Card>;
    fn lines(&mut self, config: &LensConfig) -> Result<Vec<Line>, String>;
    fn board(&mut self) -> &mut dyn Board;
}

impl<R: Rpc> Seat for Session<R> {
    fn refresh(&mut self, config: &LensConfig) -> Result<(), String> {
        Session::refresh(self, config)
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
}

/// The lenses to run: the config, and each lens's seat and prefix by id.
pub struct Lenses<'a> {
    pub config: &'a LensConfig,
    pub seats: &'a mut BTreeMap<String, Box<dyn Seat>>,
    pub prefixes: &'a BTreeMap<String, String>,
}

/// Run up to `max` passes over every lens until one starts no job. A lens
/// without a seat is an error. Each job's outcome is written to `log`.
/// `state` goes to `keep` after every lens's round, so a restart resumes
/// from it; a failed keep is an error.
pub fn passes(
    lenses: Lenses<'_>,
    state: &mut State,
    keep: &mut dyn FnMut(&State) -> Result<(), String>,
    metering: Metering<'_>,
    models: &mut dyn Models,
    max: u32,
    log: &mut String,
) -> Result<Ended, String> {
    let Lenses {
        config,
        seats,
        prefixes,
    } = lenses;
    let records = BTreeMap::new();
    for pass in 1..=max {
        let mut started = 0;
        for lens in &config.lenses {
            let seat = seats
                .get_mut(&lens.id)
                .ok_or_else(|| format!("no BBP session for lens {}", lens.id))?;
            seat.refresh(config)
                .map_err(|e| format!("{}: {e}", lens.id))?;
            let cards = seat.cards(config);
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
                prefixes,
                records: &records,
            };
            let ctx = Ctx {
                task: metering.task,
                providers: &config.providers,
                meter: &mut *metering.meter,
                save: &mut *metering.save,
            };
            let report = round(&one, &view, state, ctx, models, seat.board());
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
                return Ok(Ended::Halted(h));
            }
        }
        if started == 0 {
            return Ok(Ended::Quiet);
        }
    }
    Ok(Ended::MaxPasses)
}

/// The command's result: the log on a clean end (quiet, the pass cap, or a
/// task ceiling, which pauses the task by design), an error carrying the
/// log when the runner itself failed (an unsaved meter, a refused post, a
/// lens with no prefix).
pub fn outcome(ended: &Ended, mut log: String) -> Result<String, String> {
    let _ = writeln!(log, "ended: {ended:?}");
    match ended {
        Ended::Halted(Halt::Unsaved(_) | Halt::Board(_) | Halt::NoPrefix(_)) => Err(log),
        Ended::Quiet | Ended::MaxPasses | Ended::Halted(Halt::Task(_)) => Ok(log),
    }
}

/// The identity the scheduler's memory is saved under: the canonical BBP
/// store directory and the task id, NUL-separated (neither can hold one).
/// BBP task ids are scoped to their directory, so the id alone is not.
///
/// # Errors
///
/// A message when `dir` cannot be resolved.
pub fn board(dir: &Path, task: &str) -> Result<String, String> {
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", dir.display()))?;
    Ok(format!("{}\0{task}", dir.to_string_lossy()))
}

/// The scheduler's memory saved for `board`, or a fresh one.
fn load_state(store: &Store, board: &str) -> Result<State, String> {
    match store.load_lens_state(board).map_err(|e| e.to_string())? {
        Some(saved) => State::restore(&saved).map_err(|e| format!("lens state: {e}")),
        None => Ok(State::default()),
    }
}

/// Run `o`: open the store and the sessions, build the providers, read
/// the prompts, then [`passes`]. Returns the log and how it ended.
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
    let mut prefixes = BTreeMap::new();
    let run = run_id()?;
    let _ = writeln!(log, "run {run}");
    for lens in &config.lenses {
        let child = Child::spawn(&o.bbp, &o.dir, &o.task, &lens.id)?;
        let mut session = Session::new(&lens.id, child, &run);
        let intent = session.brief().map_err(|e| format!("{}: {e}", lens.id))?;
        let path = base.join(&lens.prompt);
        let prompt = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        prefixes.insert(lens.id.clone(), prefix(&intent, &rules, &prompt));
        seats.insert(lens.id.clone(), Box::new(session));
    }
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
    let metering = Metering {
        task: &o.task,
        meter: &mut meter,
        save: &mut save,
    };
    let lenses = Lenses {
        config: &config,
        seats: &mut seats,
        prefixes: &prefixes,
    };
    let ended = passes(
        lenses,
        &mut state,
        &mut keep,
        metering,
        &mut registry,
        o.max_passes,
        &mut log,
    )?;
    outcome(&ended, log)
}

#[cfg(test)]
mod tests {
    use orch_dispatch::{AgentError, ClassifiedError};
    use orch_lens::meter::Stop;
    use orch_lens::{Answer, Post, Question, Usage};
    use rusty_bbp::{ClaimStatus, Direction, EvidenceBody, EvidenceKind, MsgId, Stakes};

    use super::*;

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
         "models": {"routine": ["codex"], "contested": ["claude"]}},
        {"id": "operator", "prompt": "p", "domains": ["*"],
         "may_post": {"evidence": ["counter"]},
         "models": {"routine": ["codex"], "contested": ["claude"]}}
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
            stakes: Stakes::Medium,
            reversible: true,
            answers: None,
            deps: vec![],
            spent: 0,
            budget: 16,
            evidence: vec![],
            candidates: vec![],
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
    }

    impl FakeSeat {
        fn new(cards: Vec<Card>) -> Self {
            Self {
                cards,
                views: vec![],
                refreshes: 0,
                posts: vec![],
                refresh_error: None,
            }
        }
    }

    impl Board for FakeSeat {
        fn post(&mut self, _: &str, posts: &[Post]) -> Result<(), String> {
            self.posts.extend_from_slice(posts);
            Ok(())
        }
    }

    impl Seat for FakeSeat {
        fn refresh(&mut self, _: &LensConfig) -> Result<(), String> {
            self.refreshes += 1;
            self.refresh_error.clone().map_or(Ok(()), Err)
        }
        fn cards(&self, config: &LensConfig) -> Vec<Card> {
            let built = self
                .views
                .iter()
                .map(|(v, b)| Card::from_view(v, b, config));
            self.cards.iter().cloned().chain(built).collect()
        }
        fn lines(&mut self, _: &LensConfig) -> Result<Vec<Line>, String> {
            Ok(vec![])
        }
        fn board(&mut self) -> &mut dyn Board {
            self
        }
    }

    /// Every provider gives the same reply; `asked` is who was asked what.
    struct Echo(
        Result<Vec<Post>, ClassifiedError>,
        u32,
        Vec<(AgentId, String)>,
        Option<AgentId>,
    );

    impl Model for Echo {
        fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
            self.1 += 1;
            if let Some(id) = self.3 {
                self.2.push((id, q.body.clone()));
            }
            self.0.clone().map(|posts| Answer {
                posts,
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
        Echo(reply, 0, vec![], None)
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
        let cfg = config();
        let mut boxed: BTreeMap<String, Box<dyn Seat>> = seats
            .into_iter()
            .map(|(l, s)| (l.to_owned(), Box::new(s) as Box<dyn Seat>))
            .collect();
        let prefixes = cfg
            .lenses
            .iter()
            .map(|l| (l.id.clone(), prefix("intent", "", &l.id)))
            .collect();
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
            prefixes: &prefixes,
        };
        let mut log = String::new();
        let ended = passes(lenses, state, keep, metering, echo, max, &mut log);
        (ended, log)
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
}
