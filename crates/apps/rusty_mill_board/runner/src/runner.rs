//! The stage loop. Each tick finds cards each stage may take, claims them,
//! runs the stage's command and moves them on, reporting back as comments.
//!
//! The runner never holds `Triage` or `Merge` unless the operator gives its
//! role those capabilities: moving work into Ready and out of Review is a
//! human's call, and the stage table is the only thing it does on its own.

use std::collections::HashSet;
use std::thread;

use rusty_mill_board::domain::{Capability, Card, Column};
use uuid::Uuid;

use crate::config::Stage;
use crate::ports::{ApiError, BoardApi, Work};

/// Longest prompt handed to a command, in characters.
const PROMPT_CHARS: usize = 24_000;
/// Most earlier comments included in a prompt.
const PROMPT_COMMENTS: usize = 4;
/// Longest single comment quoted in a prompt, in characters.
const PROMPT_COMMENT_CHARS: usize = 1_500;

/// What happened to one card in one tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The command ran and the card moved on (or was released).
    Done {
        /// Stage name.
        stage: String,
        /// Card.
        card: Uuid,
        /// Exited 0.
        ok: bool,
    },
    /// Another actor took it, or a person moved it mid-run; nothing was changed.
    Skipped(Uuid),
    /// Over `max_attempts`; moved to Blocked.
    GaveUp(Uuid),
}

/// The one project a runner serves, and the paths besides its own that its work may change:
/// `shared` ones every project touches (change fragments, the lockfile). Which paths are the
/// project's own is the board's to say (`BoardApi::foreign`), from its whole registry.
#[derive(Debug, Clone)]
pub struct Scope {
    project: String,
    shared: Vec<String>,
}

impl Scope {
    /// Serve `project`; `shared` paths may be changed by any project.
    pub fn new(project: &str, shared: Vec<String>) -> Self {
        Self {
            project: project.to_owned(),
            shared,
        }
    }

    /// Whether `file` is one every project may change.
    fn is_shared(&self, file: &str) -> bool {
        self.shared.iter().any(|s| {
            file == s
                || file
                    .strip_prefix(s.as_str())
                    .is_some_and(|r| r.starts_with('/'))
        })
    }
}

/// The runner.
pub struct Runner<B, W> {
    board: B,
    work: W,
    stages: Vec<Stage>,
    actor: String,
    max_parallel: usize,
    env: Vec<(String, String)>,
    scope: Option<Scope>,
}

impl<B: BoardApi, W: Work> Runner<B, W> {
    /// `env` is added to every command's environment.
    pub fn new(
        board: B,
        work: W,
        stages: Vec<Stage>,
        actor: &str,
        max_parallel: usize,
        env: Vec<(String, String)>,
    ) -> Self {
        Self {
            board,
            work,
            stages,
            actor: actor.to_owned(),
            max_parallel: max_parallel.max(1),
            env,
            scope: None,
        }
    }

    /// Serve one project only: other projects' cards are never touched, and a stage whose
    /// work changes files outside the project's paths is discarded and fails.
    #[must_use]
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Check every stage's role exists and may make the moves the stage makes.
    ///
    /// # Errors
    /// A message naming the stage and the first problem found.
    pub fn validate(&self) -> Result<(), String> {
        let roles = self.board.roles().map_err(|e| e.to_string())?;
        let flow = self.board.workflow().map_err(|e| e.to_string())?;
        self.check_scope()?;
        for s in &self.stages {
            let caps = roles
                .get(&s.role)
                .ok_or_else(|| format!("stage {:?}: the board has no role {:?}", s.name, s.role))?;
            let work = s.work_column();
            let moves = s
                .from
                .iter()
                .filter(|f| Some(*f) != s.enter.as_ref())
                .filter_map(|f| s.enter.as_ref().map(|e| (f, e)))
                .chain(s.ok.as_ref().map(|c| (work, c)))
                .chain(s.fail.as_ref().map(|c| (work, c)));
            for (from, to) in moves {
                match flow.transition(from, to) {
                    Err(e) => return Err(format!("stage {:?}: {from} -> {to}: {e}", s.name)),
                    Ok(Some(ts)) if !ts.iter().any(|t| caps.contains(&t.cap)) => {
                        let need: Vec<String> = ts.iter().map(|t| format!("{:?}", t.cap)).collect();
                        return Err(format!(
                            "stage {:?}: role {:?} lacks {} for {from} -> {to}",
                            s.name,
                            s.role,
                            need.join(" or ")
                        ));
                    }
                    Ok(Some(t)) if t.gate && !caps.contains(&Capability::Admin) => {
                        return Err(format!(
                            "stage {:?}: {from} -> {to} is a gate (a human approval) and role {:?} lacks Admin",
                            s.name, s.role
                        ));
                    }
                    Ok(_) => {}
                }
            }
        }
        Ok(())
    }

    /// The board's projects can change while this runner lives: before taking work and before
    /// committing it, check the runner's scope still fits. A runner without a project needs a
    /// board with one project; one with a project needs it to exist and be this credential's.
    fn check_scope(&self) -> Result<(), String> {
        let err = |e: ApiError| e.to_string();
        match &self.scope {
            None if self.board.project_count().map_err(err)? > 1 => Err(
                "the board has several projects: pass --project NAME so this runner only \
                 works (and may only change) one application"
                    .into(),
            ),
            None => Ok(()),
            Some(scope) => self
                .board
                .foreign(&scope.project, &[])
                .map(drop)
                .map_err(err),
        }
    }

    /// One pass: up to `max_parallel` cards, each worked to the end of its stage.
    /// Later stages and later columns are served first, oldest card first.
    ///
    /// # Errors
    /// If the board cannot be listed.
    pub fn tick(&self) -> Result<Vec<Step>, ApiError> {
        self.check_scope()
            .map_err(|message| ApiError { status: 0, message })?;
        let mut jobs: Vec<(&Stage, Card)> = Vec::new();
        let mut taken = HashSet::new();
        // Finish work in progress before starting new: later stages first, and
        // within a stage the later column first (rework before fresh cards).
        for stage in self.stages.iter().rev() {
            for column in stage.from.iter().rev() {
                for card in self.board.list(column)? {
                    let in_scope = self
                        .scope
                        .as_ref()
                        .is_none_or(|s| s.project == card.project);
                    let ours = in_scope && card.claim.as_deref().is_none_or(|c| c == self.actor);
                    if ours && taken.insert(card.id) {
                        jobs.push((stage, card));
                    }
                }
            }
        }
        jobs.truncate(self.max_parallel);
        Ok(thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .into_iter()
                .map(|(s, c)| scope.spawn(move || self.job(s, c)))
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        }))
    }

    fn say(&self, stage: &Stage, id: Uuid, text: &str) {
        if let Err(e) = self.board.comment(&stage.role, id, text) {
            rusty_log::warn!(card = %id, error = %e, "could not comment");
        }
    }

    fn job(&self, stage: &Stage, card: Card) -> Step {
        let id = card.id;
        match self.work_card(stage, card) {
            Ok(r) => r,
            Err(e) if e.is_contention() => Step::Skipped(id),
            Err(e) => {
                rusty_log::warn!(card = %id, stage = %stage.name, error = %e, "stage failed");
                Step::Skipped(id)
            }
        }
    }

    fn work_card(&self, stage: &Stage, mut card: Card) -> Result<Step, ApiError> {
        let (role, id) = (stage.role.as_str(), card.id);
        if let Some(enter) = stage.enter.as_ref().filter(|e| **e != card.column) {
            card = self.board.move_card(role, id, enter)?;
        }
        if card.claim.as_deref() != Some(&self.actor) {
            card = self.board.claim(role, id)?;
        }
        let attempt = attempts(&card, &self.actor, &stage.name) + 1;
        if attempt > stage.max_attempts {
            self.say(
                stage,
                id,
                &format!(
                    "[{}] gave up after {} attempts; needs a person",
                    stage.name, stage.max_attempts
                ),
            );
            self.board.move_card(role, id, &Column::BLOCKED)?;
            return Ok(Step::GaveUp(id));
        }

        let ran = self.run_stage(stage, &card);
        let (ok, report) = match &ran {
            Ok((ok, text)) => (*ok, text.clone()),
            Err(why) => (false, format!("error: {why}")),
        };
        self.say(
            stage,
            id,
            &format!("[{}] attempt {attempt}: {report}", stage.name),
        );

        let now = self.board.get(id)?;
        if now.column != card.column || now.claim.as_deref() != Some(&self.actor) {
            return Ok(Step::Skipped(id));
        }
        // An infrastructure error is not the card's fault: release it where it
        // is. The attempt is counted, so a persistent fault ends in Blocked.
        let dest = match (&ran, ok) {
            (Err(_), _) => None,
            (Ok(_), true) => stage.ok.as_ref(),
            (Ok(_), false) => stage.fail.as_ref(),
        };
        match dest {
            Some(column) => self.board.move_card(role, id, column)?,
            None => self.board.release(role, id)?,
        };
        Ok(Step::Done {
            stage: stage.name.clone(),
            card: id,
            ok,
        })
    }

    /// Run the command; `Ok((exited_zero, report))`, or `Err` for a fault of ours.
    fn run_stage(&self, stage: &Stage, card: &Card) -> Result<(bool, String), String> {
        let dir = self.work.prepare(card)?;
        let argv = expand(&stage.argv, card, &dir.display().to_string());
        let mut env = self.env.clone();
        env.push(("MILL_CARD_ID".into(), card.id.to_string()));
        env.push(("MILL_STAGE".into(), stage.name.clone()));
        let outcome = self.work.run(stage, &argv, &dir, &env)?;
        let mut report = outcome.summary;
        if outcome.ok
            && let Some(stray) = self.stray_changes(&dir)?
        {
            self.work.discard(&dir)?;
            return Ok((
                false,
                format!(
                    "{report}; changed files outside project {:?}, all uncommitted work discarded: {}",
                    self.scope.as_ref().map_or("", |s| s.project.as_str()),
                    stray.join(", ")
                ),
            ));
        }
        if outcome.ok {
            let message = format!("mill: {} [{}]", card.title, stage.name);
            if self.work.commit(&dir, &message)? {
                report.push_str("; committed");
            }
        }
        if !outcome.tail.trim().is_empty() {
            report.push('\n');
            report.push_str(outcome.tail.trim_end());
        }
        Ok((outcome.ok, report))
    }
}

impl<B: BoardApi, W: Work> Runner<B, W> {
    /// The changed files this runner's project may not change, if there are any.
    fn stray_changes(&self, dir: &std::path::Path) -> Result<Option<Vec<String>>, String> {
        self.check_scope()?;
        let Some(scope) = &self.scope else {
            return Ok(None);
        };
        let files: Vec<String> = self
            .work
            .changed(dir)?
            .into_iter()
            .filter(|f| !scope.is_shared(f))
            .collect();
        let stray = self
            .board
            .foreign(&scope.project, &files)
            .map_err(|e| e.to_string())?;
        Ok((!stray.is_empty()).then_some(stray))
    }
}

/// Runs of `stage` already recorded on the card by `actor`.
fn attempts(card: &Card, actor: &str, stage: &str) -> u32 {
    let prefix = format!("[{stage}] attempt");
    let n = card
        .comments
        .iter()
        .filter(|c| c.by == actor && c.text.starts_with(&prefix))
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The text a command is asked to act on: the card, then the latest notes
/// (a failed test's output, a reviewer's remarks) so a retry sees why.
pub fn prompt(card: &Card) -> String {
    let mut text = format!("# {}\n\n{}\n", card.title, card.body);
    let notes: Vec<_> = card.comments.iter().rev().take(PROMPT_COMMENTS).collect();
    if !notes.is_empty() {
        text.push_str("\n## Earlier notes (newest first)\n");
        for c in notes {
            let note: String = c.text.chars().take(PROMPT_COMMENT_CHARS).collect();
            text.push_str(&format!("\n- {}: {note}\n", c.by));
        }
    }
    text.chars().take(PROMPT_CHARS).collect()
}

/// Fill `{prompt}`, `{id}`, `{title}` and `{worktree}` in each argument.
fn expand(argv: &[String], card: &Card, worktree: &str) -> Vec<String> {
    let prompt = prompt(card);
    argv.iter()
        .map(|a| {
            a.replace("{prompt}", &prompt)
                .replace("{id}", &card.id.to_string())
                .replace("{title}", &card.title)
                .replace("{worktree}", worktree)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};

    use rusty_mill_board::api::Shared;
    use rusty_mill_board::board::{Board, Roles as BoardRoles};
    use rusty_mill_board::domain::Column;
    use rusty_mill_board::store::MemStore;
    use rusty_mill_board::workflow::Workflow;

    use super::*;
    use crate::config::parse;
    use crate::ports::{Outcome, Roles};

    /// A clock that never repeats, so card order is deterministic.
    fn tick_clock() -> i64 {
        static NOW: AtomicI64 = AtomicI64::new(1);
        NOW.fetch_add(1, Ordering::Relaxed)
    }

    /// The real board domain, called in-process as the runner would over HTTP.
    struct Local(Arc<Shared>);

    fn err(e: rusty_mill_board::domain::BoardError) -> ApiError {
        use rusty_mill_board::domain::BoardError::*;
        let status = match e {
            NotFound(_) => 404,
            Conflict | Claimed(_) => 409,
            Forbidden(_) | NotClaimant | OutOfScope(_) => 403,
            Terminal | NoSuchMove { .. } | Invalid(_) | Refused(_) => 422,
            Storage(_) => 500,
        };
        ApiError {
            status,
            message: e.to_string(),
        }
    }

    impl BoardApi for Local {
        fn list(&self, column: &Column) -> Result<Vec<Card>, ApiError> {
            self.0.list(Some(column)).map_err(err)
        }
        fn get(&self, id: Uuid) -> Result<Card, ApiError> {
            self.0.get(id).map_err(err)
        }
        fn workflow(&self) -> Result<Workflow, ApiError> {
            Ok(self.0.workflow())
        }
        fn roles(&self) -> Result<Roles, ApiError> {
            Ok(self.0.roles().0.clone())
        }
        fn project_count(&self) -> Result<usize, ApiError> {
            Ok(self.0.projects().names().count())
        }
        fn foreign(&self, project: &str, files: &[String]) -> Result<Vec<String>, ApiError> {
            Ok(self.0.projects().not_owned_by(project, files))
        }
        fn move_card(&self, role: &str, id: Uuid, to: &Column) -> Result<Card, ApiError> {
            self.0
                .move_card(
                    id,
                    to,
                    &self.0.roles().actor("mill-runner", role).map_err(err)?,
                )
                .map_err(err)
        }
        fn claim(&self, role: &str, id: Uuid) -> Result<Card, ApiError> {
            self.0
                .claim(id, &self.0.roles().actor("mill-runner", role).map_err(err)?)
                .map_err(err)
        }
        fn release(&self, role: &str, id: Uuid) -> Result<Card, ApiError> {
            self.0
                .release(id, &self.0.roles().actor("mill-runner", role).map_err(err)?)
                .map_err(err)
        }
        fn comment(&self, role: &str, id: Uuid, text: &str) -> Result<Card, ApiError> {
            self.0
                .comment(
                    id,
                    &self.0.roles().actor("mill-runner", role).map_err(err)?,
                    text,
                )
                .map_err(err)
        }
    }

    /// Scripted workspace: outcomes by stage name, in order; records prompts.
    #[derive(Default)]
    struct Fake {
        script: Mutex<Vec<(String, Result<bool, String>)>>,
        prompts: Mutex<Vec<String>>,
        prepare_fails: bool,
        before_run: Mutex<Option<Box<dyn Fn() + Send>>>,
        /// Files the scripted command "changed", reported by `changed`.
        changed: Mutex<Vec<String>>,
        discards: std::sync::atomic::AtomicUsize,
        commits: std::sync::atomic::AtomicUsize,
    }

    impl Fake {
        fn then(self, stage: &str, result: Result<bool, &str>) -> Self {
            self.script
                .lock()
                .expect("lock")
                .push((stage.into(), result.map_err(str::to_owned)));
            self
        }
    }

    impl Work for Fake {
        fn prepare(&self, _: &Card) -> Result<PathBuf, String> {
            if self.prepare_fails {
                Err("disk full".into())
            } else {
                Ok(PathBuf::from("/work/x"))
            }
        }
        fn run(
            &self,
            stage: &Stage,
            argv: &[String],
            _: &Path,
            _: &[(String, String)],
        ) -> Result<Outcome, String> {
            self.prompts.lock().expect("lock").push(argv.join(" "));
            if let Some(hook) = self.before_run.lock().expect("lock").as_ref() {
                hook();
            }
            let mut script = self.script.lock().expect("lock");
            let at = script
                .iter()
                .position(|(s, _)| *s == stage.name)
                .ok_or("no scripted outcome")?;
            match script.remove(at).1 {
                Ok(ok) => Ok(Outcome {
                    ok,
                    summary: format!("exit {}", i32::from(!ok)),
                    tail: if ok {
                        String::new()
                    } else {
                        "FAILED test_x".into()
                    },
                }),
                Err(e) => Err(e),
            }
        }
        fn changed(&self, _: &Path) -> Result<Vec<String>, String> {
            Ok(self.changed.lock().expect("lock").clone())
        }
        fn discard(&self, _: &Path) -> Result<(), String> {
            self.discards.fetch_add(1, Ordering::Relaxed);
            self.changed.lock().expect("lock").clear();
            Ok(())
        }
        fn commit(&self, _: &Path, _: &str) -> Result<bool, String> {
            self.commits.fetch_add(1, Ordering::Relaxed);
            Ok(true)
        }
    }

    const STAGES: &str = r#"[
      {"name":"build","from":["ready","building"],"enter":"building","role":"builder","argv":["agent","{prompt}"],"ok":"qa","fail":"blocked","max_attempts":2},
      {"name":"test","from":["qa"],"role":"qa","argv":["make","test"],"ok":"review","fail":"building"}
    ]"#;

    fn setup(fake: Fake, parallel: usize) -> (Runner<Local, Fake>, Vec<Uuid>) {
        let board: Arc<Shared> = Arc::new(Board::with_clock(
            Box::new(MemStore::default()),
            BoardRoles::default(),
            tick_clock,
        ));
        let human = board.roles().actor("nano", "human").expect("role");
        let ids = (1..=3)
            .map(|i| {
                let c = board
                    .add(&human, &format!("card {i}"), "do it", vec![])
                    .expect("add");
                board
                    .move_card(c.id, &Column::READY, &human)
                    .expect("ready");
                c.id
            })
            .collect();
        let runner = Runner::new(
            Local(board),
            fake,
            parse(STAGES).expect("stages"),
            "mill-runner",
            parallel,
            vec![],
        );
        (runner, ids)
    }

    fn column(r: &Runner<Local, Fake>, id: Uuid) -> Column {
        r.board.get(id).expect("card").column
    }

    #[test]
    fn happy_path_runs_a_card_from_ready_to_review() {
        let fake = Fake::default()
            .then("build", Ok(true))
            .then("test", Ok(true));
        let (r, ids) = setup(fake, 1);
        r.validate().expect("valid");

        assert_eq!(
            r.tick().expect("tick"),
            [Step::Done {
                stage: "build".into(),
                card: ids[0],
                ok: true
            }]
        );
        assert_eq!(column(&r, ids[0]), Column::QA);
        r.tick().expect("tick"); // the Column::QA card is served before the fresh Column::READY ones
        let card = r.board.get(ids[0]).expect("card");
        assert_eq!(card.column, Column::REVIEW, "{:?}", card.comments);
        assert_eq!(card.claim, None);
        assert!(
            card.comments[0]
                .text
                .starts_with("[build] attempt 1: exit 0; committed")
        );
        assert!(card.comments[1].text.starts_with("[test] attempt 1"));
        // The prompt carried the card.
        assert!(r.work.prompts.lock().expect("lock")[0].contains("# card 1"));
    }

    #[test]
    fn failed_tests_send_the_card_back_with_the_output_in_the_next_prompt() {
        let fake = Fake::default()
            .then("build", Ok(true))
            .then("test", Ok(false))
            .then("build", Ok(true))
            .then("test", Ok(true));
        let (r, ids) = setup(fake, 1);
        r.tick().expect("build");
        r.tick().expect("test fails");
        assert_eq!(column(&r, ids[0]), Column::BUILDING);
        r.tick().expect("rebuild"); // rework in Column::BUILDING comes before fresh Column::READY cards
        let prompts = r.work.prompts.lock().expect("lock").clone();
        assert!(
            prompts[2].contains("FAILED test_x"),
            "retry prompt must include the failure: {}",
            prompts[2]
        );
    }

    #[test]
    fn gives_up_after_max_attempts() {
        let fake = Fake::default()
            .then("build", Ok(true))
            .then("test", Ok(false))
            .then("build", Ok(true))
            .then("test", Ok(false));
        let (r, ids) = setup(fake, 1);
        for _ in 0..4 {
            r.tick().expect("tick");
        }
        // Third build attempt exceeds max_attempts = 2.
        let steps = r.tick().expect("tick");
        assert!(steps.contains(&Step::GaveUp(ids[0])), "{steps:?}");
        let card = r.board.get(ids[0]).expect("card");
        assert_eq!(card.column, Column::BLOCKED);
        assert!(
            card.comments
                .last()
                .expect("comment")
                .text
                .contains("gave up after 2 attempts")
        );
    }

    #[test]
    fn infrastructure_errors_release_the_card_and_count_as_attempts() {
        let fake = Fake {
            prepare_fails: true,
            ..Fake::default()
        };
        let (r, ids) = setup(fake, 1);
        r.tick().expect("tick");
        let card = r.board.get(ids[0]).expect("card");
        assert_eq!(
            (card.column, card.claim.clone()),
            (Column::BUILDING, None),
            "released, not moved"
        );
        assert!(card.comments[0].text.contains("error: disk full"));
        r.tick().expect("tick");
        let steps = r.tick().expect("tick");
        assert!(
            steps.contains(&Step::GaveUp(ids[0])),
            "persistent fault must end in Blocked: {steps:?}"
        );
    }

    #[test]
    fn skips_cards_claimed_by_others_and_resumes_its_own() {
        let (r, ids) = setup(Fake::default().then("build", Ok(true)), 3);
        let other = r.board.0.roles().actor("someone", "builder").expect("role");
        r.board
            .0
            .move_card(ids[0], &Column::BUILDING, &other)
            .expect("move");
        r.board.0.claim(ids[0], &other).expect("claim");
        // ids[1] is claimed by the runner itself, as if it crashed mid-run.
        let me = r
            .board
            .0
            .roles()
            .actor("mill-runner", "builder")
            .expect("role");
        r.board
            .0
            .move_card(ids[1], &Column::BUILDING, &me)
            .expect("move");
        r.board.0.claim(ids[1], &me).expect("claim");

        let steps = r.tick().expect("tick");
        let touched: Vec<_> = steps
            .iter()
            .filter_map(|s| {
                if let Step::Done { card, .. } = s {
                    Some(*card)
                } else {
                    None
                }
            })
            .collect();
        assert!(touched.contains(&ids[1]), "own claim resumed");
        assert!(
            !touched.contains(&ids[0]),
            "someone else's claim left alone"
        );
        assert_eq!(
            r.board.0.get(ids[0]).expect("card").claim.as_deref(),
            Some("someone")
        );
    }

    #[test]
    fn a_person_moving_the_card_mid_run_wins() {
        let (r, ids) = setup(Fake::default().then("build", Ok(true)), 1);
        let (board, id) = (Arc::clone(&r.board.0), ids[0]);
        // While the command runs, a person (holding Triage) drags the card to Blocked.
        *r.work.before_run.lock().expect("lock") = Some(Box::new(move || {
            let human = board.roles().actor("nano", "human").expect("role");
            board
                .move_card(id, &Column::BLOCKED, &human)
                .expect("override");
        }));

        let steps = r.tick().expect("tick");
        assert_eq!(steps, [Step::Skipped(id)]);
        let card = r.board.get(id).expect("card");
        assert_eq!(
            (card.column, card.claim),
            (Column::BLOCKED, None),
            "the runner must not move it on"
        );
    }

    #[test]
    fn max_parallel_bounds_a_tick() {
        let fake = Fake::default()
            .then("build", Ok(true))
            .then("build", Ok(true))
            .then("build", Ok(true));
        let (r, ids) = setup(fake, 2);
        let steps = r.tick().expect("tick");
        assert_eq!(steps.len(), 2);
        let done = ids
            .iter()
            .filter(|id| column(&r, **id) == Column::QA)
            .count();
        assert_eq!(done, 2);
    }

    #[test]
    fn validate_catches_missing_roles_and_capabilities() {
        let unknown =
            r#"[{"name":"a","from":["ready"],"enter":"building","role":"ghost","argv":["x"]}]"#;
        let weak = r#"[{"name":"a","from":["ready"],"enter":"building","role":"qa","argv":["x"],"ok":"qa"}]"#;
        let nonsense =
            r#"[{"name":"a","from":["backlog"],"enter":"done","role":"human","argv":["x"]}]"#;
        let missing =
            r#"[{"name":"a","from":["ready"],"enter":"design","role":"human","argv":["x"]}]"#;
        for (json, why) in [
            (unknown, "no role"),
            (weak, "lacks Build"),
            (nonsense, "no move"),
            (missing, "no move ready -> design"),
        ] {
            let board: Arc<Shared> = Arc::new(Board::new(
                Box::new(MemStore::default()),
                BoardRoles::default(),
            ));
            let r = Runner::new(
                Local(board),
                Fake::default(),
                parse(json).expect("stages"),
                "r",
                1,
                vec![],
            );
            let e = r.validate().expect_err(json);
            assert!(e.contains(why), "{json} -> {e}");
        }
    }

    #[test]
    fn validate_refuses_a_stage_that_would_pass_a_gate_without_admin() {
        let flow = Workflow::default();
        let transitions = flow
            .transitions()
            .iter()
            .cloned()
            .map(|t| rusty_mill_board::workflow::Transition {
                gate: t.from == Column::BUILDING && t.to == Column::QA,
                ..t
            })
            .collect();
        let gated = Workflow::new(flow.columns().to_vec(), transitions).expect("valid");
        let stages = r#"[{"name":"b","from":["ready"],"enter":"building","role":"builder","argv":["x"],"ok":"qa"}]"#;
        let board: Arc<Shared> = Arc::new(
            Board::new(
                Box::new(MemStore::default()) as Box<dyn rusty_mill_board::store::Store>,
                BoardRoles::default(),
            )
            .with_workflow(gated),
        );
        let r = Runner::new(
            Local(board),
            Fake::default(),
            parse(stages).expect("stages"),
            "r",
            1,
            vec![],
        );
        let e = r.validate().expect_err("gate without admin");
        assert!(e.contains("lacks Admin"), "{e}");
    }

    // ---- projects: a runner serves one application and may change only its paths

    fn projects() -> rusty_mill_board::projects::Projects {
        rusty_mill_board::projects::Projects::parse(
            r#"{"rl": ["crates/apps/rocket_league"], "orch": ["crates/apps/rusty_orch"]}"#,
        )
        .expect("projects")
    }

    /// A board with a Ready card in each of `rl`, `orch` and `default`.
    fn setup_projects(fake: Fake) -> (Runner<Local, Fake>, Vec<(String, Uuid)>) {
        let board: Arc<Shared> = Arc::new(
            Board::with_clock(
                Box::new(MemStore::default()) as Box<dyn rusty_mill_board::store::Store>,
                BoardRoles::default(),
                tick_clock,
            )
            .with_projects(projects()),
        );
        let human = board.roles().actor("nano", "human").expect("role");
        let cards = ["rl", "orch", "default"]
            .iter()
            .map(|p| {
                let c = board.add_in(&human, p, p, "do it", vec![]).expect("add");
                board
                    .move_card(c.id, &Column::READY, &human)
                    .expect("ready");
                ((*p).to_owned(), c.id)
            })
            .collect();
        let runner = Runner::new(
            Local(board),
            fake,
            parse(STAGES).expect("stages"),
            "mill-runner",
            3,
            vec![],
        );
        (runner, cards)
    }

    fn scope(project: &str) -> Scope {
        Scope::new(project, vec!["changes".into(), "Cargo.lock".into()])
    }

    #[test]
    fn a_board_with_several_projects_needs_a_scoped_runner() {
        let (r, _) = setup_projects(Fake::default());
        let e = r.validate().expect_err("unscoped");
        assert!(e.contains("--project"), "{e}");
        let (r, _) = setup_projects(Fake::default());
        assert!(r.with_scope(scope("rl")).validate().is_ok());
    }

    #[test]
    fn a_scoped_runner_works_only_its_own_projects_cards() {
        let (r, cards) = setup_projects(
            Fake::default()
                .then("build", Ok(true))
                .then("build", Ok(true)),
        );
        let r = r.with_scope(scope("orch"));
        let steps = r.tick().expect("tick");
        let orch = cards.iter().find(|(p, _)| p == "orch").expect("card").1;
        assert_eq!(
            steps,
            [Step::Done {
                stage: "build".into(),
                card: orch,
                ok: true
            }]
        );
        for (project, id) in &cards {
            let expected = if project == "orch" {
                Column::QA
            } else {
                Column::READY
            };
            assert_eq!(column(&r, *id), expected, "{project}");
        }
    }

    #[test]
    fn work_outside_the_project_is_discarded_and_not_committed() {
        let fake = Fake::default().then("build", Ok(true));
        *fake.changed.lock().expect("lock") = vec![
            "crates/apps/rocket_league/src/lib.rs".into(),
            "crates/apps/rusty_orch/src/lib.rs".into(),
        ];
        let (r, cards) = setup_projects(fake);
        let r = r.with_scope(scope("rl"));
        let rl = cards.iter().find(|(p, _)| p == "rl").expect("card").1;
        r.tick().expect("tick");
        assert_eq!(
            r.work.commits.load(Ordering::Relaxed),
            0,
            "nothing was committed"
        );
        assert_eq!(r.work.discards.load(Ordering::Relaxed), 1);
        let card = r.board.get(rl).expect("card");
        assert_eq!(
            card.column,
            Column::BLOCKED,
            "the build stage's failure column"
        );
        let note = &card.comments[0].text;
        assert!(
            note.contains("outside project") && note.contains("crates/apps/rusty_orch/src/lib.rs"),
            "{note}"
        );
        assert!(
            !note.contains("rocket_league/src/lib.rs"),
            "only the stray files are named: {note}"
        );
    }

    #[test]
    fn work_inside_the_project_and_shared_paths_is_committed() {
        let fake = Fake::default().then("build", Ok(true));
        *fake.changed.lock().expect("lock") = vec![
            "crates/apps/rocket_league/src/lib.rs".into(),
            "changes/2026-10-14-rl.md".into(),
            "Cargo.lock".into(),
        ];
        let (r, cards) = setup_projects(fake);
        let r = r.with_scope(scope("rl"));
        let rl = cards.iter().find(|(p, _)| p == "rl").expect("card").1;
        r.tick().expect("tick");
        assert_eq!(r.work.commits.load(Ordering::Relaxed), 1);
        assert_eq!(r.work.discards.load(Ordering::Relaxed), 0);
        assert_eq!(column(&r, rl), Column::QA);
    }

    #[test]
    fn a_path_moved_to_another_project_mid_stage_is_not_committed() {
        let fake = Fake::default().then("build", Ok(true));
        *fake.changed.lock().expect("lock") = vec!["crates/apps/rocket_league/src/lib.rs".into()];
        let (r, cards) = setup_projects(fake);
        let r = r.with_scope(scope("rl"));
        // While the stage runs, settings give rocket_league's path to the other project.
        let board = r.board.0.clone();
        *r.work.before_run.lock().expect("lock") = Some(Box::new(move || {
            let moved = rusty_mill_board::projects::Projects::parse(
                r#"{"rl": ["crates/apps/rl_new"], "orch": ["crates/apps/rusty_orch", "crates/apps/rocket_league"]}"#,
            )
            .expect("projects");
            board.set_projects(moved, |_| Ok(())).expect("moved");
        }));
        r.tick().expect("tick");
        assert_eq!(
            r.work.commits.load(Ordering::Relaxed),
            0,
            "stale scope committed"
        );
        assert_eq!(r.work.discards.load(Ordering::Relaxed), 1);
        let rl = cards.iter().find(|(p, _)| p == "rl").expect("card").1;
        assert_eq!(column(&r, rl), Column::BLOCKED);
    }

    #[test]
    fn an_unscoped_runner_stops_when_the_board_gains_projects() {
        let board: Arc<Shared> = Arc::new(Board::with_clock(
            Box::new(MemStore::default()) as Box<dyn rusty_mill_board::store::Store>,
            BoardRoles::default(),
            tick_clock,
        ));
        let human = board.roles().actor("nano", "human").expect("role");
        let card = board.add(&human, "t", "do it", vec![]).expect("add");
        board
            .move_card(card.id, &Column::READY, &human)
            .expect("ready");
        let r = Runner::new(
            Local(board.clone()),
            Fake::default().then("build", Ok(true)),
            parse(STAGES).expect("stages"),
            "mill-runner",
            1,
            vec![],
        );
        r.validate().expect("one project: no scope needed");
        board
            .set_projects(projects(), |_| Ok(()))
            .expect("more projects");
        let e = r.tick().expect_err("now it needs a project");
        assert!(e.message.contains("several projects"), "{e}");
        assert_eq!(column(&r, card.id), Column::READY, "no work was taken");
    }

    #[test]
    fn shared_paths_are_whole_path_components() {
        let s = scope("rl");
        for shared in ["changes/x.md", "changes", "Cargo.lock"] {
            assert!(s.is_shared(shared), "{shared}");
        }
        for own in ["changes.md", "Cargo.lock.bak", "crates/a.rs"] {
            assert!(!s.is_shared(own), "{own}");
        }
    }

    #[test]
    fn prompt_includes_recent_notes_and_is_bounded() {
        let board: Shared = Board::new(Box::new(MemStore::default()), BoardRoles::default());
        let h = board.roles().actor("n", "human").expect("role");
        let mut card = board
            .add(&h, "T", &"b".repeat(30_000), vec![])
            .expect("add");
        for i in 0..6 {
            card = board
                .comment(card.id, &h, &format!("note {i}"))
                .expect("comment");
        }
        let p = prompt(&card);
        assert_eq!(p.chars().count(), PROMPT_CHARS);
        let short = board.add(&h, "S", "x", vec![]).expect("add");
        let short = board.comment(short.id, &h, "n1").expect("c");
        let short = board.comment(short.id, &h, "n2").expect("c");
        let p = prompt(&short);
        assert!(
            p.contains("n2") && p.find("n2") < p.find("n1"),
            "newest first: {p}"
        );
    }
}
