//! The alien lens's pass in `lens run --alien` (#708, D78, D99): after the
//! ordinary passes, one hunt per eligible claim whose current candidate
//! was not hunted yet, recorded in orch-store. The lens posts nothing
//! (D75): it reads through its own BBP session and keeps its rows.
//!
//! A claim skipped before any model call (the role may not read the
//! candidate, the diff is too long for one prompt, no provider is cleared
//! for its class, its probes cannot be set up, or no provider took the
//! first call) records nothing, so a later run tries it again; each skip
//! is one warning for stderr (D98). A ceiling ends the pass. A hunt that
//! made its first call is recorded whatever happened next, so its calls
//! are never spent twice.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use orch_core::AgentId;
use orch_lens::alien::{eligible, touches_rs, Claim, Trigger, MAX_DIFF, RUST};
use orch_lens::call::{Ctx, Models};
use orch_lens::card::Card;
use orch_lens::class::admit;
use orch_lens::config::{Alien, Tools};
use orch_lens::context::raw_prefix;
use orch_lens::hunt::{hunt, Hunted, Probes, Seat as Asking, Target, Unrecorded};
use orch_lens::schedule::{reviews_settled, State};
use orch_lens::LensConfig;
use orch_store::Store;
use rusty_bbp::{ArtId, MsgId};
use rusty_sandbox::Executor;
use rusty_worktree::Repo;

use super::bbp::{Child, Diffs, Session};
use super::probes::{Candidate, Dirs, NoNet, SandboxProbes, Toolchain};
use super::run::{Metering, Options, Seat};

/// The trigger until #596's value score exists (D75 Q1): irreversible
/// claims only, and so no control sample yet.
pub const TRIGGER: Trigger = Trigger {
    min_value: None,
    control_one_in: 10,
};

/// What the pass needs from outside: the hunt memory, the clock, a probe
/// runner per hunt, and where a hunt is saved.
pub trait Hunter {
    /// Was this candidate of this claim hunted already (D78)?
    fn hunted(&self, claim: MsgId, candidate: ArtId) -> Result<bool, String>;
    /// Now, in Unix milliseconds.
    fn now(&self) -> u64;
    /// The probe runner for one hunt of `claim` on `code`.
    fn probes(&mut self, claim: MsgId, code: Candidate) -> Result<Box<dyn Probes + '_>, String>;
    /// Save a hunt: its probe rows, then its hunt row, which marks the
    /// candidate hunted.
    fn record(&mut self, hunted: &Hunted) -> Result<(), String>;
}

/// The alien lens and what it reads besides its seat.
pub struct Alienist<'a> {
    pub config: &'a LensConfig,
    pub alien: &'a Alien,
    /// The ordinary lenses' memory, for "reviews settled" (§2).
    pub state: &'a State,
    /// The board rules and the alien lens's own prompt.
    pub rules: &'a str,
    pub prompt: &'a str,
}

/// Run one hunt for each eligible claim `seat` shows. Hunt results go to
/// `log`, skips to `warnings`.
///
/// # Errors
///
/// A message when the seat or the hunt memory fails, a hunt cannot be
/// saved, or the meter cannot be.
pub fn sweep(
    lens: Alienist<'_>,
    seat: &mut dyn Seat,
    hunter: &mut dyn Hunter,
    metering: Metering<'_>,
    models: &mut dyn Models,
    log: &mut String,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let Alienist {
        config,
        alien,
        state,
        rules,
        prompt,
    } = lens;
    let at = |e: String| format!("{}: {e}", alien.id);
    seat.refresh(config).map_err(at)?;
    let prefix = raw_prefix(&seat.brief().map_err(at)?, rules, prompt);
    let Metering { task, meter, save } = metering;
    for card in seat.cards(config) {
        let Some(candidate) = card.current else {
            continue;
        };
        let claim = Claim {
            task,
            id: card.id,
            rust_candidate: true, // read below, once the rest holds
            reviews_settled: reviews_settled(config, &card, state),
            value: None,
            reversible: card.reversible,
            hunted: hunter.hunted(card.id, candidate)?,
        };
        if !eligible(&claim, TRIGGER) {
            continue;
        }
        let skip = |why: String| format!("alien: claim {} skipped: {why}", card.id.0);
        let providers = cleared(config, alien, &card);
        if providers.is_empty() {
            warnings.push(skip(format!("no provider is cleared for {:?}", card.class)));
            continue;
        }
        let code = match seat.candidate(candidate).map_err(at)? {
            Diffs::Read(code) => code,
            Diffs::Forbidden(why) => {
                warnings.push(skip(why));
                continue;
            }
        };
        let diff = joined(&code.diffs);
        if !touches_rs(&diff) {
            continue;
        }
        if diff.len() > MAX_DIFF {
            warnings.push(skip(format!(
                "its diffs are {} bytes, over {MAX_DIFF} (Q18)",
                diff.len()
            )));
            continue;
        }
        if !full_commit(&code.base) {
            warnings.push(skip(format!(
                "base {:?} is not a full commit id",
                code.base
            )));
            continue;
        }
        let notes: Vec<String> = card.evidence.iter().map(|i| i.note.clone()).collect();
        let target = Target {
            claim: card.id,
            candidate: candidate.0,
            claim_text: &card.text,
            diff: &diff,
            notes: &notes,
            class: card.class,
            started: hunter.now(),
        };
        let asking = Asking {
            providers: &providers,
            tools: Tools::ReadOnly,
            prefix: &prefix,
        };
        let ctx = Ctx {
            task,
            providers: &config.providers,
            meter: &mut *meter,
            save: &mut *save,
        };
        let result = match hunter.probes(card.id, code) {
            Ok(mut probes) => hunt(&target, asking, RUST, ctx, models, probes.as_mut()),
            Err(e) => {
                warnings.push(skip(format!("probes not set up: {e}")));
                continue;
            }
        };
        match result {
            Ok(hunted) => {
                hunter.record(&hunted)?;
                logged(log, &hunted);
            }
            Err(Unrecorded::Stopped(stop)) => {
                let _ = writeln!(log, "alien: stopped before claim {}: {stop}", card.id.0);
                return Ok(());
            }
            Err(Unrecorded::Exhausted(passed)) => {
                warnings.push(skip(format!("no provider took the call: {passed:?}")));
            }
            Err(Unrecorded::Unsaved(e)) => return Err(format!("meter not saved: {e}")),
        }
    }
    Ok(())
}

/// The alien lens's providers cleared for the card's class, in order.
fn cleared(config: &LensConfig, alien: &Alien, card: &Card) -> Vec<AgentId> {
    alien
        .providers
        .iter()
        .copied()
        .filter(|id| {
            config
                .providers
                .get(id)
                .is_some_and(|p| admit(card.class, p.max_class))
        })
        .collect()
}

/// The diffs as one text, each ending in a newline so no two lines join.
fn joined(diffs: &[String]) -> String {
    let mut out = String::new();
    for d in diffs {
        out.push_str(d);
        if !d.is_empty() && !d.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// A full SHA-1 or SHA-256 commit id in lowercase hex, which git cannot
/// read as an option or a ref name.
fn full_commit(base: &str) -> bool {
    matches!(base.len(), 40 | 64) && base.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn logged(log: &mut String, h: &Hunted) {
    let row = &h.hunt;
    let _ = writeln!(
        log,
        "alien: claim {} candidate {}: {} suspects, {} probes, {} tokens",
        row.claim, row.candidate, row.suspects, row.probes, row.tokens
    );
    for p in &h.probes {
        let _ = writeln!(
            log,
            "alien: claim {} probe {} {}: {}",
            p.claim,
            p.site,
            p.mechanism,
            p.outcome.name()
        );
    }
    for n in &h.notes {
        let _ = writeln!(log, "alien: claim {}: {n}", row.claim);
    }
}

/// What the ordinary passes leave the alien pass: the config, the
/// scheduler's memory and the board rules.
pub struct Ordinary<'a> {
    pub config: &'a LensConfig,
    pub state: &'a State,
    pub rules: &'a str,
}

/// What `lens run --alien` opens before any pass, so a missing piece fails
/// the run before anything is spent.
pub struct Kit {
    alien: Alien,
    prompt: String,
    seat: Session<Child>,
    exec: NoNet,
    repo: Repo,
    toolchain: Toolchain,
    /// `<state>/alien`.
    root: PathBuf,
}

impl Kit {
    /// Check the config and the task id, read the alien prompt (relative to
    /// `base`, the config's directory), remove leftover probe trees (D97),
    /// find the toolchain and the repository, and open the lens's session
    /// in run `run`.
    ///
    /// # Errors
    ///
    /// A message naming the piece that is missing.
    pub fn open(config: &LensConfig, o: &Options, base: &Path, run: &str) -> Result<Self, String> {
        let alien = config
            .alien
            .clone()
            .ok_or("--alien needs an \"alien\" entry in the lens config")?;
        if !one_component(&o.task) {
            return Err(format!(
                "--alien: task {:?} cannot name a directory",
                o.task
            ));
        }
        let path = base.join(&alien.prompt);
        let prompt = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let root = o.state.join("alien");
        let leftover = root.join("probes");
        if leftover.symlink_metadata().is_ok() {
            std::fs::remove_dir_all(&leftover)
                .map_err(|e| format!("{}: {e}", leftover.display()))?;
        }
        let toolchain = Toolchain::detect()?;
        let repo = Repo::open(&o.repo)?;
        let exec = NoNet::current(&root.join("sandbox"))?;
        let child = Child::spawn(&o.bbp, &o.dir, &o.task, &alien.id)?;
        let seat = Session::new(&alien.id, child, run);
        Ok(Self {
            alien,
            prompt,
            seat,
            exec,
            repo,
            toolchain,
            root,
        })
    }

    /// [`sweep`] with the store and the sandbox; skips go to stderr.
    ///
    /// # Errors
    ///
    /// As [`sweep`].
    pub fn sweep(
        &mut self,
        ordinary: Ordinary<'_>,
        store: &RefCell<Store>,
        metering: Metering<'_>,
        models: &mut dyn Models,
        log: &mut String,
    ) -> Result<(), String> {
        let Ordinary {
            config,
            state,
            rules,
        } = ordinary;
        let mut shelf = Shelf {
            store,
            task: metering.task,
            exec: &self.exec,
            repo: &self.repo,
            toolchain: &self.toolchain,
            root: &self.root,
        };
        let lens = Alienist {
            config,
            alien: &self.alien,
            state,
            rules,
            prompt: &self.prompt,
        };
        let mut warnings = Vec::new();
        let swept = sweep(
            lens,
            &mut self.seat,
            &mut shelf,
            metering,
            models,
            log,
            &mut warnings,
        );
        for w in &warnings {
            eprintln!("rusty_orch: {w}");
        }
        swept
    }
}

/// Is `task` one plain path component, so `<state>/alien/probes/<task>`
/// stays under the state dir?
fn one_component(task: &str) -> bool {
    let mut parts = Path::new(task).components();
    matches!(
        (parts.next(), parts.next()),
        (Some(Component::Normal(_)), None)
    )
}

/// The real [`Hunter`]: orch-store and the sandbox.
struct Shelf<'a, E> {
    store: &'a RefCell<Store>,
    task: &'a str,
    exec: &'a E,
    repo: &'a Repo,
    toolchain: &'a Toolchain,
    root: &'a Path,
}

impl<E: Executor> Hunter for Shelf<'_, E>
where
    E::Error: std::fmt::Display,
{
    fn hunted(&self, claim: MsgId, candidate: ArtId) -> Result<bool, String> {
        let hunts = self
            .store
            .borrow()
            .hunts(Some(self.task))
            .map_err(|e| e.to_string())?;
        Ok(hunts
            .iter()
            .any(|h| h.claim == claim.0 && h.candidate == candidate.0))
    }

    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    fn probes(&mut self, claim: MsgId, code: Candidate) -> Result<Box<dyn Probes + '_>, String> {
        let dirs = Dirs {
            probes: self.root.join("probes").join(self.task),
            target: self
                .root
                .join("target")
                .join(self.task)
                .join(claim.0.to_string()),
        };
        let probes = SandboxProbes::new(self.exec, self.repo, self.toolchain, code, dirs)?;
        Ok(Box::new(probes))
    }

    fn record(&mut self, hunted: &Hunted) -> Result<(), String> {
        let mut store = self.store.borrow_mut();
        for row in &hunted.probes {
            store
                .save_probe(row)
                .map_err(|e| format!("probe row not saved: {e}"))?;
        }
        store
            .save_hunt(&hunted.hunt)
            .map_err(|e| format!("hunt row not saved: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use orch_dispatch::ClassifiedError;
    use orch_lens::alien::{Outcome, Run};
    use orch_lens::context::Line;
    use orch_lens::hunt::{Probe, Runs};
    use orch_lens::meter::Meter;
    use orch_lens::round::Board;
    use orch_lens::schedule::{Pass, Progress};
    use orch_lens::{Answer, DataClass, Model, Post, Question, Reply, Usage};
    use rusty_bbp::{ClaimStatus, Stakes};

    use super::*;

    const CONFIG: &str = r#"{
      "providers": {
        "codex": {"kind": "codex-cli", "context_tokens": 200000,
                  "price": {"input": 0, "output": 0, "cached_input": 0}}
      },
      "ceilings": {"experiment_usd": 1, "task_usd": 1, "task_calls": 10},
      "lenses": [
        {"id": "adversary", "prompt": "p", "domains": ["*"],
         "may_post": {"evidence": ["counter"]},
         "models": {"routine": ["codex"], "contested": ["codex"]}}
      ],
      "alien": {"id": "alien", "prompt": "a", "providers": ["codex"]}
    }"#;
    const RS: &str = "--- a/crates/a/src/lib.rs\n+++ b/crates/a/src/lib.rs\n@@ -1 +1 @@\n-1\n+2";
    const BASE: &str = "0123456789abcdef0123456789abcdef01234567";
    const SUSPECT: &str = r#"{"suspects": [{"site": "a::f", "mechanism": "rs-cast", "sketch": "s", "coherence": 3}]}"#;
    const PROBE: &str = r#"{"probe": "fn t() {}"}"#;

    fn config() -> LensConfig {
        LensConfig::parse(CONFIG).expect("config")
    }

    /// An irreversible claim with a code candidate.
    fn card(id: u64) -> Card {
        Card {
            id: MsgId(id),
            text: "Hash by size first".to_owned(),
            domains: vec!["dedup".to_owned()],
            status: ClaimStatus::Open,
            score: 0,
            accept_at: 12,
            epoch: 0,
            stakes: Stakes::High,
            reversible: false,
            answers: None,
            deps: vec![],
            spent: 0,
            budget: 16,
            evidence: vec![],
            candidates: vec![],
            current: Some(ArtId(50 + id)),
            class: DataClass::Internal,
        }
    }

    /// The memory after every review lens's blind pass found a counter.
    fn settled(cards: &[Card]) -> State {
        let mut state = State::default();
        for c in cards {
            let p = Progress {
                blind: Some(Pass {
                    provider: AgentId::new("codex").expect("id"),
                    posted: 1,
                    found_counter: true,
                }),
                epoch: c.epoch,
                ..Progress::default()
            };
            state.progress.insert(("adversary".to_owned(), c.id), p);
        }
        state
    }

    struct FakeSeat {
        cards: Vec<Card>,
        code: Result<Diffs, String>,
        reads: Vec<ArtId>,
    }

    impl Board for FakeSeat {
        fn post(&mut self, _: &str, _: &[Post]) -> Result<(), String> {
            Err("the alien lens posts nothing".to_owned())
        }
    }

    impl Seat for FakeSeat {
        fn refresh(&mut self, _: &LensConfig) -> Result<(), String> {
            Ok(())
        }
        fn brief_id(&self) -> Option<ArtId> {
            Some(ArtId(1))
        }
        fn brief(&mut self) -> Result<String, String> {
            Ok("intent".to_owned())
        }
        fn cards(&self, _: &LensConfig) -> Vec<Card> {
            self.cards.clone()
        }
        fn lines(&mut self, _: &LensConfig) -> Result<Vec<Line>, String> {
            Ok(vec![])
        }
        fn board(&mut self) -> &mut dyn Board {
            self
        }
        fn candidate(&mut self, id: ArtId) -> Result<Diffs, String> {
            self.reads.push(id);
            self.code.clone()
        }
    }

    fn seat(cards: Vec<Card>, diff: &str) -> FakeSeat {
        FakeSeat {
            cards,
            code: Ok(Diffs::Read(Candidate {
                base: BASE.to_owned(),
                diffs: vec![diff.to_owned()],
            })),
            reads: vec![],
        }
    }

    /// Fails on the candidate and passes on base: a regression.
    struct Regress(u32);

    impl Probes for Regress {
        fn run(&mut self, _: &Probe<'_>) -> Result<Runs, String> {
            self.0 += 1;
            Ok(Runs {
                candidate: Run::Failed,
                base: Some(Run::Passed),
            })
        }
    }

    #[derive(Default)]
    struct FakeHunter {
        hunted: Vec<(MsgId, ArtId)>,
        unready: Option<String>,
        unsaved: Option<String>,
        saved: Vec<Hunted>,
        set_up: Vec<(MsgId, Candidate)>,
    }

    impl Hunter for FakeHunter {
        fn hunted(&self, claim: MsgId, candidate: ArtId) -> Result<bool, String> {
            Ok(self.hunted.contains(&(claim, candidate)))
        }
        fn now(&self) -> u64 {
            1_000
        }
        fn probes(
            &mut self,
            claim: MsgId,
            code: Candidate,
        ) -> Result<Box<dyn Probes + '_>, String> {
            self.set_up.push((claim, code));
            match &self.unready {
                Some(e) => Err(e.clone()),
                None => Ok(Box::new(Regress(0))),
            }
        }
        fn record(&mut self, hunted: &Hunted) -> Result<(), String> {
            if let Some(e) = &self.unsaved {
                return Err(e.clone());
            }
            self.saved.push(hunted.clone());
            Ok(())
        }
    }

    /// Replies from a script; records each question.
    struct Script(VecDeque<&'static str>, Vec<Question>);

    impl Model for Script {
        fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
            self.1.push(q.clone());
            let text = self.0.pop_front().expect("scripted reply");
            Ok(Answer {
                reply: Reply::Raw(text.to_owned()),
                usage: Usage::default(),
            })
        }
    }

    impl Models for Script {
        fn get(&mut self, _: AgentId, _: Tools) -> Option<&mut dyn Model> {
            Some(self)
        }
    }

    fn script(replies: &[&'static str]) -> Script {
        Script(replies.iter().copied().collect(), vec![])
    }

    struct Swept {
        result: Result<(), String>,
        log: String,
        warnings: Vec<String>,
    }

    fn go(
        state: &State,
        seat: &mut FakeSeat,
        hunter: &mut FakeHunter,
        models: &mut Script,
        task_calls: u32,
    ) -> Swept {
        let cfg = config();
        let alien = cfg.alien.clone().expect("alien");
        let lens = Alienist {
            config: &cfg,
            alien: &alien,
            state,
            rules: "rules",
            prompt: "hunt",
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
        let (mut log, mut warnings) = (String::new(), vec![]);
        let result = sweep(
            lens,
            seat,
            hunter,
            metering,
            models,
            &mut log,
            &mut warnings,
        );
        Swept {
            result,
            log,
            warnings,
        }
    }

    #[test]
    fn an_eligible_claim_is_hunted_and_its_rows_recorded() {
        let cards = vec![card(7)];
        let mut seat = seat(cards.clone(), RS);
        let mut hunter = FakeHunter::default();
        let mut models = script(&[SUSPECT, PROBE]);
        let s = go(&settled(&cards), &mut seat, &mut hunter, &mut models, 10);
        assert_eq!(s.result, Ok(()));
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert_eq!(seat.reads, [ArtId(57)]);
        assert_eq!(hunter.set_up.len(), 1);
        assert_eq!(hunter.set_up[0].1.base, BASE);
        assert_eq!(hunter.saved.len(), 1);
        let h = &hunter.saved[0];
        assert_eq!(
            (h.hunt.task.as_str(), h.hunt.claim, h.hunt.candidate),
            ("t", 7, 57)
        );
        assert_eq!(
            (h.hunt.suspects, h.hunt.probes, h.hunt.started),
            (1, 1, 1_000)
        );
        assert_eq!(h.probes[0].outcome, Outcome::Regression);
        let q = &models.1[0];
        assert!(q
            .prefix
            .starts_with("# Intent\nintent\n\n# Board rules\nrules\n"));
        assert!(q.prefix.contains("# Your lens\nhunt\n"));
        assert!(
            !q.prefix.contains("# Reply"),
            "no post protocol: {}",
            q.prefix
        );
        assert!(q.body.contains(RS));
        assert!(s
            .log
            .contains("alien: claim 7 candidate 57: 1 suspects, 1 probes"));
        assert!(s
            .log
            .contains("alien: claim 7 probe a::f rs-cast: regression"));
    }

    #[test]
    fn claims_that_are_not_eligible_are_left_unread() {
        let mut reversible = card(1);
        reversible.reversible = true;
        let mut no_candidate = card(2);
        no_candidate.current = None;
        let hunted = card(3);
        let unsettled = card(4);
        let cards = vec![reversible.clone(), no_candidate, hunted.clone(), unsettled];
        let state = settled(&[reversible, hunted]);
        let mut seat = seat(cards, RS);
        let mut hunter = FakeHunter {
            hunted: vec![(MsgId(3), ArtId(53))],
            ..FakeHunter::default()
        };
        let mut models = script(&[]);
        let s = go(&state, &mut seat, &mut hunter, &mut models, 10);
        assert_eq!(s.result, Ok(()));
        assert!(seat.reads.is_empty());
        assert!(hunter.saved.is_empty() && models.1.is_empty() && s.warnings.is_empty());
    }

    #[test]
    fn a_candidate_touching_no_rust_is_skipped_quietly() {
        let cards = vec![card(7)];
        let mut seat = seat(cards.clone(), "--- a/README.md\n+++ b/README.md\n");
        let mut hunter = FakeHunter::default();
        let mut models = script(&[]);
        let s = go(&settled(&cards), &mut seat, &mut hunter, &mut models, 10);
        assert_eq!(s.result, Ok(()));
        assert_eq!(seat.reads.len(), 1);
        assert!(hunter.saved.is_empty() && hunter.set_up.is_empty() && s.warnings.is_empty());
    }

    /// Each skip warns once, records nothing and asks no model.
    fn skipped(seat: &mut FakeSeat, cards: &[Card], hunter: &mut FakeHunter, want: &str) {
        let mut models = script(&[]);
        let s = go(&settled(cards), seat, hunter, &mut models, 10);
        assert_eq!(s.result, Ok(()));
        assert_eq!(s.warnings.len(), 1, "{:?}", s.warnings);
        assert!(
            s.warnings[0].contains(want),
            "{want:?} not in {:?}",
            s.warnings
        );
        assert!(hunter.saved.is_empty() && models.1.is_empty());
    }

    #[test]
    fn skips_before_any_call_warn_and_record_nothing() {
        let cards = vec![card(7)];
        let mut forbidden = seat(cards.clone(), RS);
        forbidden.code = Ok(Diffs::Forbidden("role lacks read(diff)".to_owned()));
        skipped(
            &mut forbidden,
            &cards,
            &mut FakeHunter::default(),
            "role lacks read(diff)",
        );
        let big = format!("{RS}\n+{}", "x".repeat(MAX_DIFF));
        skipped(
            &mut seat(cards.clone(), &big),
            &cards,
            &mut FakeHunter::default(),
            "(Q18)",
        );
        let mut bad_base = seat(cards.clone(), RS);
        bad_base.code = Ok(Diffs::Read(Candidate {
            base: "--upload-pack=x".to_owned(),
            diffs: vec![RS.to_owned()],
        }));
        skipped(
            &mut bad_base,
            &cards,
            &mut FakeHunter::default(),
            "not a full commit id",
        );
        let mut unready = FakeHunter {
            unready: Some("no rustc".to_owned()),
            ..FakeHunter::default()
        };
        skipped(
            &mut seat(cards.clone(), RS),
            &cards,
            &mut unready,
            "probes not set up: no rustc",
        );
        let mut secret = card(7);
        secret.class = DataClass::Sensitive;
        let secret = vec![secret];
        let mut s = seat(secret.clone(), RS);
        skipped(
            &mut s,
            &secret,
            &mut FakeHunter::default(),
            "no provider is cleared",
        );
        assert!(
            s.reads.is_empty(),
            "nothing read for a claim nobody can hunt"
        );
    }

    #[test]
    fn a_ceiling_ends_the_pass_with_nothing_recorded() {
        let cards = vec![card(7), card(8)];
        let mut seat = seat(cards.clone(), RS);
        let mut hunter = FakeHunter::default();
        let mut models = script(&[]);
        let s = go(&settled(&cards), &mut seat, &mut hunter, &mut models, 0);
        assert_eq!(s.result, Ok(()));
        assert!(s.log.contains("alien: stopped before claim 7"), "{}", s.log);
        assert_eq!(seat.reads, [ArtId(57)], "claim 8 is not read");
        assert!(hunter.saved.is_empty() && models.1.is_empty());
    }

    #[test]
    fn a_hunt_that_cannot_be_saved_fails_the_pass() {
        let cards = vec![card(7)];
        let mut hunter = FakeHunter {
            unsaved: Some("disk full".to_owned()),
            ..FakeHunter::default()
        };
        let mut models = script(&[r#"{"suspects": []}"#]);
        let s = go(
            &settled(&cards),
            &mut seat(cards.clone(), RS),
            &mut hunter,
            &mut models,
            10,
        );
        assert_eq!(s.result, Err("disk full".to_owned()));
    }

    #[test]
    fn opening_refuses_a_config_without_the_lens_or_a_task_that_names_a_dir() {
        let o = |task: &str| Options {
            config: "c.json".into(),
            bbp: "/nonexistent/bbp".into(),
            dir: "/nonexistent/dir".into(),
            task: task.to_owned(),
            state: "/nonexistent/state".into(),
            experiment: "e".to_owned(),
            rules: None,
            assertions: "a.json".into(),
            repo: "/nonexistent/repo".into(),
            max_passes: 1,
            alien: true,
        };
        let mut without = config();
        without.alien = None;
        let e = Kit::open(&without, &o("t"), Path::new("/"), "r").err();
        assert!(e.is_some_and(|e| e.contains("needs an \"alien\" entry")));
        let e = Kit::open(&config(), &o("../t"), Path::new("/"), "r").err();
        assert!(e.is_some_and(|e| e.contains("cannot name a directory")));
    }

    #[test]
    fn helpers_check_paths_commits_and_joins() {
        assert!(one_component("task-1"));
        for bad in ["", ".", "..", "a/b", "/abs", "a/.."] {
            assert!(!one_component(bad), "{bad:?}");
        }
        assert!(full_commit(BASE));
        assert!(full_commit(&"a".repeat(64)));
        for bad in ["", "HEAD", "main", &BASE.to_uppercase(), &BASE[..39], "-x"] {
            assert!(!full_commit(bad), "{bad:?}");
        }
        assert_eq!(joined(&["a".into(), "b\n".into(), "c".into()]), "a\nb\nc\n");
    }
}
