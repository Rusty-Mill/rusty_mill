//! Thin task adapter: BBP owns protocol, storage, moderation and execution.
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
mod config;
mod lease;
mod supervisor;
pub use config::{Agent, Assignment, Config, Sandbox, Slots};
pub use supervisor::{StdSupervisor, Supervised, Supervisor};

use orch_cli::{CommandRunner, ExecError, StdCommand};
use orch_core::board::{Board, Entry, EntryKind};
use orch_core::task::Task;
use orch_core::{Ref, Text};
use orch_dispatch::{
    AgentError, ClassifiedError, Fence, GateContext, HostHandle, Output, Progress, Relayed,
    TaskHost,
};
use rusty_bbp::{Code, PrincipalKind, Response, Sha256, State, TaskView};
use rusty_serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// Subscription CLIs authenticate themselves; inherited API/push credentials
/// must not select a different billing path or reach spawned workloads.
pub const SCRUB: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "BBP_DIR",
    "BBP_TASK",
    "BBP_PRINCIPAL",
    "BBP_TURN",
];
/// How often a cancel re-reads the task when another writer moved its
/// revision in between.
/// The claim file each task's manifest holds beside its agents file.
pub(crate) const CLAIM: &str = "claim.json";
const CANCEL_ATTEMPTS: usize = 5;
/// BBP's end grace when the configuration leaves it unset.
const BBP_END_GRACE_SECS: u64 = 60;
/// How long a cancel waits, beyond the end grace BBP gives a harness to
/// print its result, for everything the task started to be gone.
const CANCEL_MARGIN: Duration = Duration::from_secs(30);

fn transient(e: impl std::fmt::Display) -> ClassifiedError {
    ClassifiedError::Transient(AgentError(e.to_string()))
}
fn permanent(e: impl std::fmt::Display) -> ClassifiedError {
    ClassifiedError::Permanent(AgentError(e.to_string()))
}
fn unix_now() -> Result<u64, ClassifiedError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(transient)
}
fn text(s: &str) -> Result<Text, ClassifiedError> {
    Text::new(s).ok_or_else(|| permanent("empty host field"))
}

fn request_display(message: &rusty_bbp::Message) -> Result<String, ClassifiedError> {
    let body = match &message.draft.body {
        rusty_bbp::Body::Prose(body) => body.clone(),
        body => rusty_serde::json::to_string(body).map_err(permanent)?,
    };
    Ok(format!(
        "Request msg:{} from {}:\n{body}",
        message.id.0, message.from
    ))
}

/// Publish `bytes` as `dir/name` unless it already exists, never replacing
/// it: written whole to a private temporary, then hard-linked into place.
fn publish_once(dir: &std::path::Path, name: &str, bytes: &[u8]) -> Result<(), ClassifiedError> {
    let tmp = dir.join(format!(
        "{name}-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(transient)?
            .as_nanos()
    ));
    rusty_atomic_file::write_private(&tmp, bytes).map_err(transient)?;
    let published = std::fs::hard_link(&tmp, dir.join(name));
    let _ = std::fs::remove_file(tmp);
    match published {
        Ok(()) => {
            // Match rusty_atomic_file's platform contract: directory
            // fsync is supported on Unix, not std File::open on Windows.
            #[cfg(unix)]
            std::fs::File::open(dir)
                .and_then(|d| d.sync_all())
                .map_err(transient)?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(transient(e)),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Handle {
    #[rusty_serde(with = "config::path_json")]
    dir: PathBuf,
    task: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Frozen {
    config: Config,
    brief: String,
    profiles: String,
    /// The lease paths the pool was confined to at the first admission;
    /// absent for a task frozen unleased, or before leases existed.
    #[rusty_serde(default)]
    lease: Option<LeasePaths>,
}
/// The paths of a hosting lease, as granted. Its end is not frozen: a later
/// generation may renew it, never change where it reaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LeasePaths {
    read: Vec<String>,
    write: Vec<String>,
}
impl LeasePaths {
    fn of(fence: &Fence) -> Option<Self> {
        fence.lease.as_ref().map(|l| Self {
            read: l.read.clone(),
            write: l.write.clone(),
        })
    }
}
/// `bbp mod --claim`, written beside the agents file as [`CLAIM`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Claim {
    parent: Option<ClaimParent>,
    priority_secs: u64,
    write: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ClaimParent {
    id: String,
    agents: u32,
}
/// Who holds one admitted generation of a hosted task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Holder {
    assignment: String,
    holder: String,
}
#[derive(Debug, Clone, Serialize)]
struct Launcher {
    program: String,
    args: Vec<String>,
    builtin_tools: bool,
    sandbox: Option<Sandbox>,
}

/// A provider-neutral adapter over finite CLI commands and a detached supervisor.
#[derive(Debug)]
pub struct BbpHost<C = StdCommand, S = StdSupervisor> {
    config: Config,
    namespace: String,
    context: String,
    command: C,
    supervisor: S,
}
impl BbpHost {
    /// Real commands and sandboxed Linux supervision. Namespace identifies the
    /// goal bytes, never the process, so uncertain opens are adopted after crashes.
    pub fn new(config: Config, namespace: String, context: String) -> Result<Self, String> {
        if !cfg!(target_os = "linux") {
            return Err("BBP sandboxed hosting requires Linux".into());
        }
        Self::with_seams(config, namespace, context, StdCommand, StdSupervisor)
    }
}
impl<C: CommandRunner, S: Supervisor> BbpHost<C, S> {
    /// Injectable finite-process and moderator seams for contract/recovery tests.
    pub fn with_seams(
        config: Config,
        namespace: String,
        context: String,
        command: C,
        supervisor: S,
    ) -> Result<Self, String> {
        let config = config.absolute()?;
        config.validate()?;
        Ok(Self {
            config,
            namespace,
            context,
            command,
            supervisor,
        })
    }
    fn handle(&self, h: &HostHandle) -> Result<Handle, ClassifiedError> {
        let json =
            h.0.as_str()
                .strip_prefix("bbp:")
                .ok_or_else(|| permanent("invalid host handle"))?;
        let handle: Handle = rusty_serde::json::from_str(json).map_err(permanent)?;
        if !handle.dir.is_absolute() || handle.task.is_empty() {
            return Err(permanent("invalid host location"));
        }
        Ok(handle)
    }
    fn manifest(h: &Handle) -> PathBuf {
        h.dir.join("orch").join(Sha256::of(h.task.as_bytes()).hex())
    }
    fn load(h: &Handle) -> Result<Frozen, ClassifiedError> {
        let bytes =
            std::fs::read_to_string(Self::manifest(h).join("manifest.json")).map_err(transient)?;
        rusty_serde::json::from_str(&bytes).map_err(permanent)
    }
    fn invoke(
        &self,
        config: &Config,
        h: &Handle,
        sub: &str,
        args: Vec<String>,
    ) -> Result<String, ClassifiedError> {
        let mut argv = vec![
            config.binary.to_string_lossy().into_owned(),
            sub.into(),
            "--dir".into(),
            h.dir.to_string_lossy().into_owned(),
            "--task".into(),
            h.task.clone(),
        ];
        argv.extend(args);
        let exit = self
            .command
            .run_scrubbed(&argv, &[], Duration::from_secs(30), SCRUB)
            .map_err(|e| match e {
                ExecError::Spawn(e) => ClassifiedError::Unavailable(AgentError(e)),
                e => transient(e),
            })?;
        if exit.status != 0 {
            return Err(transient(format!(
                "bbp {sub} exited {}: {}",
                exit.status,
                orch_cli::excerpt(&exit.stderr)
            )));
        }
        String::from_utf8(exit.stdout).map_err(permanent)
    }
    fn response(
        &self,
        config: &Config,
        h: &Handle,
        sub: &str,
        args: Vec<String>,
    ) -> Result<Response, ClassifiedError> {
        rusty_serde::json::from_str(&self.invoke(config, h, sub, args)?).map_err(permanent)
    }
    fn show(&self, config: &Config, h: &Handle) -> Result<Option<TaskView>, ClassifiedError> {
        rusty_serde::json::from_str(&self.invoke(config, h, "show", vec![])?).map_err(permanent)
    }
    fn accepted(response: Response) -> Result<(), ClassifiedError> {
        match response {
            Response::Ok | Response::Posted(_) | Response::Stored(_) => Ok(()),
            Response::Rejected(r) => Err(permanent(format!("BBP {:?}: {}", r.code, r.detail))),
            _ => Err(permanent("unexpected BBP response")),
        }
    }
    /// Freeze the task's configuration once, its pool confined to the
    /// lease of the fence that first freezes it.
    fn freeze(
        &self,
        h: &Handle,
        task: &Task,
        board: &Board,
        fence: &Fence,
    ) -> Result<Frozen, ClassifiedError> {
        let dir = Self::manifest(h);
        std::fs::create_dir_all(&dir).map_err(transient)?;
        let path = dir.join("manifest.json");
        if path.exists() {
            return Self::load(h);
        }
        let config = match &fence.lease {
            Some(lease) => lease::confine(&self.config, lease).map_err(permanent)?,
            None => self.config.clone(),
        };
        let profiles = std::fs::read_to_string(&self.config.profiles).map_err(transient)?;
        let brief = format!("{}\n\n{}", self.context, orch_cli::render(task, board, ""));
        let frozen = Frozen {
            config,
            brief,
            profiles,
            lease: LeasePaths::of(fence),
        };
        let bytes = rusty_serde::json::to_string(&frozen).map_err(permanent)?;
        publish_once(&dir, "manifest.json", bytes.as_bytes())?;
        Self::load(h)
    }
    /// Admit `fence` for this task, or refuse it. Each admitted generation
    /// is a file published once and never rewritten, so the highest
    /// generation only grows whatever the interleaving of processes, and two
    /// holders racing for one generation find the same winner.
    fn admit(h: &Handle, fence: &Fence) -> Result<(), ClassifiedError> {
        let dir = Self::manifest(h).join("fence");
        std::fs::create_dir_all(&dir).map_err(transient)?;
        let mine = Holder {
            assignment: fence.assignment.as_str().into(),
            holder: fence.holder.as_str().into(),
        };
        let foreign = |owner: &Holder| {
            permanent(format!(
                "hosted task belongs to assignment {}",
                owner.assignment
            ))
        };
        let (highest, owner) = match Self::highest(&dir)? {
            Some((_, owner)) if owner.assignment != mine.assignment => return Err(foreign(&owner)),
            Some((generation, owner)) if generation >= fence.generation => (generation, owner),
            _ => {
                let name = format!("{:020}", fence.generation);
                let bytes = rusty_serde::json::to_string(&mine).map_err(permanent)?;
                publish_once(&dir, &name, bytes.as_bytes())?;
                (fence.generation, Self::holder(&dir.join(&name))?)
            }
        };
        if owner.assignment != mine.assignment {
            return Err(foreign(&owner));
        }
        if highest > fence.generation {
            return Err(ClassifiedError::Unavailable(AgentError(format!(
                "superseded: generation {highest} now holds this task"
            ))));
        }
        if owner != mine {
            return Err(ClassifiedError::Unavailable(AgentError(format!(
                "generation {highest} is held by another process"
            ))));
        }
        Ok(())
    }
    fn highest(dir: &std::path::Path) -> Result<Option<(u64, Holder)>, ClassifiedError> {
        let mut highest: Option<u64> = None;
        for entry in std::fs::read_dir(dir).map_err(transient)? {
            let name = entry.map_err(transient)?.file_name();
            let Some(name) = name.to_str() else { continue };
            // Publication temporaries are not generations.
            if name.ends_with(".tmp") {
                continue;
            }
            let generation: u64 = name
                .parse()
                .map_err(|_| permanent(format!("unexpected fence file {name}")))?;
            highest = highest.max(Some(generation));
        }
        highest
            .map(|g| Ok((g, Self::holder(&dir.join(format!("{g:020}")))?)))
            .transpose()
    }
    /// Refuse a fence whose lease has ended, so a lapsed holder stops
    /// driving the task without failing it, and one whose lease reaches
    /// other paths than the task's pool was confined to: changing access
    /// needs a task started under the new lease.
    fn leased(frozen: &Frozen, fence: &Fence) -> Result<(), ClassifiedError> {
        if let Some(lease) = &fence.lease {
            if !lease.live(unix_now()?) {
                return Err(ClassifiedError::Unavailable(AgentError(
                    "the hosting lease has ended".into(),
                )));
            }
        }
        if frozen.lease != LeasePaths::of(fence) {
            return Err(permanent(
                "the hosted task was confined under another lease; \
                 changing access needs a task started under the new one",
            ));
        }
        Ok(())
    }
    /// The absolute end, in unix seconds, every process the fence starts
    /// must be gone by: its lease's.
    fn deadline(fence: &Fence) -> Option<u64> {
        fence.lease.as_ref().and_then(|l| l.expires)
    }
    fn holder(path: &std::path::Path) -> Result<Holder, ClassifiedError> {
        let bytes = std::fs::read_to_string(path).map_err(transient)?;
        rusty_serde::json::from_str(&bytes).map_err(permanent)
    }
    /// What this task asks of fair admission: its goal as the parent whose
    /// ceiling it shares, the goal's priority, and the write paths of the
    /// lease it was frozen under.
    fn claim(&self, frozen: &Frozen) -> Claim {
        let slots = frozen.config.slots.as_ref();
        Claim {
            parent: slots.and_then(|s| s.ceiling).map(|agents| ClaimParent {
                id: self.namespace.clone(),
                agents,
            }),
            priority_secs: slots.map_or(0, |s| s.priority_secs),
            write: frozen
                .lease
                .as_ref()
                .map(|l| l.write.clone())
                .unwrap_or_default(),
        }
    }
    fn materialize(&self, h: &Handle, frozen: &Frozen) -> Result<PathBuf, ClassifiedError> {
        let dir = Self::manifest(h);
        let config = &frozen.config;
        let mut launchers = BTreeMap::new();
        for role in &config.roster {
            let assignment = config
                .assignments
                .get(&role.id.0)
                .ok_or_else(|| permanent("missing frozen assignment"))?;
            let agent = config
                .pool
                .get(&assignment.agent)
                .ok_or_else(|| permanent("missing frozen agent"))?;
            launchers.insert(
                role.id.0.clone(),
                Launcher {
                    program: agent.program.clone(),
                    args: agent.args.clone(),
                    builtin_tools: assignment.builtin_tools,
                    sandbox: agent.sandbox.clone(),
                },
            );
        }
        for (name, bytes) in [
            ("brief.md", frozen.brief.clone()),
            ("profiles.json", frozen.profiles.clone()),
            (
                "roster.json",
                rusty_serde::json::to_string(&config.roster).map_err(permanent)?,
            ),
            (
                "agents.json",
                rusty_serde::json::to_string(&launchers).map_err(permanent)?,
            ),
            (
                CLAIM,
                rusty_serde::json::to_string(&self.claim(frozen)).map_err(permanent)?,
            ),
        ] {
            rusty_atomic_file::write_private(&dir.join(name), bytes.as_bytes())
                .map_err(transient)?;
        }
        Ok(dir.join("agents.json"))
    }
    fn ensure(
        &mut self,
        h: &Handle,
        frozen: &Frozen,
        fence: &Fence,
    ) -> Result<(), ClassifiedError> {
        let agents = self.materialize(h, frozen)?;
        self.supervisor
            .ensure(&frozen.config, &h.task, &agents, Self::deadline(fence))
            .map_err(transient)?;
        Ok(())
    }
    /// Record the cancel in BBP, retrying when the revision moved under it.
    /// The op is derived from the revision, so a retry after a lost reply
    /// replays rather than cancels twice.
    fn request_cancel(
        &self,
        h: &Handle,
        config: &Config,
        words: &[&str],
        reason: &Text,
    ) -> Result<(), ClassifiedError> {
        for _ in 0..CANCEL_ATTEMPTS {
            let view = self
                .show(config, h)?
                .ok_or_else(|| transient("hosted task missing"))?;
            if view.card.state.terminal() {
                return Ok(());
            }
            let rev = view.card.rev.0.to_string();
            let op = Sha256::of(format!("{}:cancel:{rev}:{reason}", h.task).as_bytes()).hex();
            let mut args = vec![
                "--fence".into(),
                rev.clone(),
                "--rev".into(),
                rev,
                "--op".into(),
                format!("orch-{op}"),
                "cancel".into(),
            ];
            args.extend(words.iter().map(|w| (*w).to_owned()));
            match self.response(config, h, "human", args)? {
                Response::Rejected(r) if matches!(r.code, Code::StaleRev | Code::Terminal) => {}
                response => return Self::accepted(response),
            }
        }
        Err(transient("the task kept changing; cancel not recorded"))
    }
    /// Wait until `bbp quiesce` finds nothing of the ended task running: no
    /// moderator, no runner, no harness group. It kills the groups of a
    /// moderator that died, so this holds without one.
    fn await_quiesced(&self, config: &Config, h: &Handle) -> Result<(), ClassifiedError> {
        let grace = config.end_grace_secs.unwrap_or(BBP_END_GRACE_SECS);
        let deadline = std::time::Instant::now() + Duration::from_secs(grace) + CANCEL_MARGIN;
        loop {
            match self.invoke(config, h, "quiesce", vec![])?.trim() {
                "true" => return Ok(()),
                "false" => {}
                other => return Err(permanent(format!("bbp quiesce answered {other:?}"))),
            }
            if std::time::Instant::now() >= deadline {
                return Err(transient("the task's processes still run after cancel"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    /// Where `task`'s BBP task lives: derived from the goal's bytes, the
    /// board's goal and the task id, so it is found again without a handle.
    fn located(&self, task: &Task, board: &Board) -> Handle {
        let id =
            Sha256::of(format!("{}:{}:{}", self.namespace, board.goal(), task.id()).as_bytes())
                .hex();
        Handle {
            dir: self.config.dir.clone(),
            task: format!("orch-{id}"),
        }
    }
    /// `bbp open` the frozen task unless it exists. Starts nothing.
    fn open_once(&self, config: &Config, h: &Handle) -> Result<(), ClassifiedError> {
        if self.show(config, h)?.is_some() {
            return Ok(());
        }
        let dir = Self::manifest(h);
        let response = self.response(
            config,
            h,
            "open",
            vec![
                "--repo".into(),
                config.repo.to_string_lossy().into_owned(),
                "--brief".into(),
                dir.join("brief.md").to_string_lossy().into_owned(),
                "--roster".into(),
                dir.join("roster.json").to_string_lossy().into_owned(),
                "--profiles".into(),
                dir.join("profiles.json").to_string_lossy().into_owned(),
                "--live-turns".into(),
                config.live_turns.to_string(),
            ],
        )?;
        match response {
            Response::Rejected(r) if r.code == Code::AlreadyOpen => Ok(()),
            r => Self::accepted(r),
        }
    }
    fn rejected_answer(
        &self,
        h: &Handle,
        config: &Config,
        reason: &str,
    ) -> Result<Relayed, ClassifiedError> {
        let view = self
            .show(config, h)?
            .ok_or_else(|| transient("hosted task missing"))?;
        let mut progress = Self::progress(view)?;
        if let Progress::Gate(gate) = &mut progress {
            gate.prompt = text(&format!("Answer rejected: {reason}\n{}", gate.prompt))?;
        }
        Ok(Relayed::Rejected(progress))
    }

    fn progress(view: TaskView) -> Result<Progress, ClassifiedError> {
        let c = &view.card;
        let mut ids = Vec::new();
        let prompt = match c.state {
            State::Closed => {
                let (candidate, revision) = view
                    .receipt
                    .ok_or_else(|| permanent("closed task has no receipt"))?;
                return Ok(Progress::Closed {
                    outputs: vec![
                        Output {
                            kind: EntryKind::Artifact,
                            body: text("BBP candidate")?,
                            refs: vec![Ref::Url(text(&format!("art:{}", candidate.0))?)],
                            supersedes: None,
                        },
                        Output {
                            kind: EntryKind::Artifact,
                            body: text("recorded merge receipt")?,
                            refs: vec![Ref::Commit(text(&revision)?)],
                            supersedes: None,
                        },
                    ],
                });
            }
            State::Cancelled => {
                return Ok(Progress::Cancelled {
                    reason: text(
                        view.cancelled
                            .as_deref()
                            .ok_or_else(|| permanent("cancelled without reason"))?,
                    )?,
                })
            }
            State::PlanGate => {
                let art = view
                    .proposal
                    .ok_or_else(|| permanent("plan gate has no proposal"))?;
                ids.push(text(&format!("art:{}", art.0))?);
                let request = view
                    .request
                    .as_ref()
                    .map(request_display)
                    .transpose()?
                    .unwrap_or_default();
                format!(
                    "approve-plan art:{} | reject planning REASON\n{request}",
                    art.0
                )
            }
            State::MergeGate | State::Approved => {
                let art = c.candidate.ok_or_else(|| permanent("missing candidate"))?;
                let (run, _, _) = c.run.ok_or_else(|| permanent("missing selected run"))?;
                ids.push(text(&format!("art:{}", art.0))?);
                ids.push(text(&format!("run:{}", run.0))?);
                if c.state == State::Approved {
                    format!("Candidate art:{} approved on run:{}. Merge externally, then receipt art:{} REVISION. Approval is not a receipt.", art.0, run.0, art.0)
                } else {
                    format!(
                        "approve-merge art:{} run:{} | reject build REASON",
                        art.0, run.0
                    )
                }
            }
            State::Escalated => {
                let mut prompt = format!(
                    "Escalated: {:?}. resume STATE | extend FIELD LIMIT | cancel REASON",
                    view.escalation
                );
                if let Some(request) = &view.request {
                    ids.push(text(&format!("msg:{}", request.id.0))?);
                    prompt.push_str(&format!("\n{}\nSettle the outstanding request before resuming: answer msg:{} TEXT | decision msg:{} accept|reject NOTE", request_display(request)?, request.id.0, request.id.0));
                }
                prompt
            }
            _ => match view.request {
                Some(request) => {
                    ids.push(text(&format!("msg:{}", request.id.0))?);
                    format!(
                        "{}\nanswer msg:{} TEXT | decision msg:{} accept|reject NOTE",
                        request_display(&request)?,
                        request.id.0,
                        request.id.0
                    )
                }
                None => return Ok(Progress::Working),
            },
        };
        Ok(Progress::Gate(GateContext {
            prompt: text(&prompt)?,
            rev: c.rev.0,
            ids,
        }))
    }
}
impl<C: CommandRunner + std::fmt::Debug, S: Supervisor> TaskHost for BbpHost<C, S> {
    fn open(
        &mut self,
        task: &Task,
        board: &Board,
        fence: &Fence,
    ) -> Result<HostHandle, ClassifiedError> {
        let h = self.located(task, board);
        Self::admit(&h, fence)?;
        let frozen = self.freeze(&h, task, board, fence)?;
        Self::leased(&frozen, fence)?;
        let config = &frozen.config;
        let agents = self.materialize(&h, &frozen)?;
        self.open_once(config, &h)?;
        let view = self
            .show(config, &h)?
            .ok_or_else(|| transient("opened task is missing"))?;
        if view.repo != config.repo.to_string_lossy() || view.roster != config.roster {
            return Err(permanent("existing task differs from frozen configuration"));
        }
        if !view.card.state.terminal() {
            for role in &config.roster {
                let assignment = config
                    .assignments
                    .get(&role.id.0)
                    .ok_or_else(|| permanent("missing assignment"))?;
                let agent = config
                    .pool
                    .get(&assignment.agent)
                    .ok_or_else(|| permanent("missing agent"))?;
                let principal = format!("orch-{}", role.id.0);
                if let Some(p) = view.principals.iter().find(
                    |p| matches!(&p.kind, PrincipalKind::Agent { role: r, .. } if r == &role.id),
                ) {
                    if p.id.0 != principal
                        || !matches!(&p.kind, PrincipalKind::Agent { vendor, .. } if vendor == &agent.vendor)
                    {
                        return Err(permanent("existing assignment differs from frozen pool"));
                    }
                    continue;
                }
                Self::accepted(self.response(
                    config,
                    &h,
                    "assign",
                    vec![
                        "--role".into(),
                        role.id.0.clone(),
                        "--principal".into(),
                        principal,
                        "--vendor".into(),
                        agent.vendor.clone(),
                    ],
                )?)?;
            }
            self.supervisor
                .ensure(config, &h.task, &agents, Self::deadline(fence))
                .map_err(transient)?;
        }
        Ok(HostHandle(text(&format!(
            "bbp:{}",
            rusty_serde::json::to_string(&h).map_err(permanent)?
        ))?))
    }
    fn poll(&mut self, handle: &HostHandle, fence: &Fence) -> Result<Progress, ClassifiedError> {
        let h = self.handle(handle)?;
        Self::admit(&h, fence)?;
        let frozen = Self::load(&h)?;
        Self::leased(&frozen, fence)?;
        let view = self
            .show(&frozen.config, &h)?
            .ok_or_else(|| transient("hosted task missing"))?;
        if !view.card.state.terminal() {
            self.ensure(&h, &frozen, fence)?;
        }
        Self::progress(view)
    }
    fn relay(
        &mut self,
        handle: &HostHandle,
        gate: &Entry,
        answer: &Text,
        fence: &Fence,
    ) -> Result<Relayed, ClassifiedError> {
        let h = self.handle(handle)?;
        Self::admit(&h, fence)?;
        let frozen = Self::load(&h)?;
        Self::leased(&frozen, fence)?;
        let refs: Vec<_> = gate
            .content()
            .refs
            .iter()
            .filter_map(|r| match r {
                Ref::Url(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        let rev = refs
            .iter()
            .find_map(|r| r.strip_prefix("host:rev:"))
            .ok_or_else(|| permanent("question lacks revision fence"))?;
        let words: Vec<_> = answer.as_str().split_whitespace().collect();
        let Some(verb) = words.first().copied() else {
            return self.rejected_answer(&h, &frozen.config, "empty human action");
        };
        // No flags may enter BBP's flag parser from an answer, even in free text.
        if words.iter().any(|s| s.starts_with("--")) {
            return self.rejected_answer(&h, &frozen.config, "flags are not human answers");
        }
        let subjects: &[(&str, usize)] = match verb {
            "approve-plan" | "receipt" | "rerun" => &[("art:", 1)],
            "approve-merge" => &[("art:", 1), ("run:", 2)],
            "answer" | "decision" => &[("msg:", 1)],
            "reject" | "resume" | "cancel" | "extend" => &[],
            _ => return self.rejected_answer(&h, &frozen.config, "unsupported human answer verb"),
        };
        for (prefix, position) in subjects {
            let Some(subject) = words.get(*position) else {
                return self.rejected_answer(&h, &frozen.config, "missing subject");
            };
            if !subject.starts_with(prefix) || !refs.contains(subject) {
                return self.rejected_answer(
                    &h,
                    &frozen.config,
                    "answer names a subject outside the displayed gate",
                );
            }
        }
        // Stable op permits a crash after BBP applied the answer but before Orch
        // checkpointed it. The host replays it before checking the old fence.
        let op = Sha256::of(format!("{}:{}:{}", h.task, gate.id(), answer).as_bytes()).hex();
        let mut args = vec![
            "--fence".into(),
            rev.into(),
            "--rev".into(),
            rev.into(),
            "--op".into(),
            format!("orch-{op}"),
        ];
        args.extend(words.into_iter().map(str::to_owned));
        match self.response(&frozen.config, &h, "human", args)? {
            Response::Rejected(r)
                if matches!(
                    r.code,
                    Code::StaleRev
                        | Code::StaleSubject
                        | Code::StaleCandidate
                        | Code::StaleRun
                        | Code::WrongState
                        | Code::Terminal
                ) =>
            {
                Ok(Relayed::Stale(self.poll(handle, fence)?))
            }
            Response::Rejected(r)
                if !matches!(r.code, Code::Forbidden | Code::OpConflict | Code::NotOpen) =>
            {
                self.rejected_answer(
                    &h,
                    &frozen.config,
                    &format!("BBP {:?}: {}", r.code, r.detail),
                )
            }
            response => {
                Self::accepted(response)?;
                Ok(Relayed::Applied)
            }
        }
    }
    /// Cancel the task's BBP task, found from its handle or derived as
    /// `open` derives it. One never opened is opened now and cancelled at
    /// once, with no principal assigned and no moderator started, so a
    /// superseded holder's late `open` finds it ended and starts nothing.
    /// A cancel only takes authority away, so it is not refused for an
    /// ended or different lease.
    fn cancel(
        &mut self,
        task: &Task,
        board: &Board,
        reason: &Text,
        fence: &Fence,
    ) -> Result<Progress, ClassifiedError> {
        // No flags may enter BBP's flag parser from a reason.
        let words: Vec<_> = reason.as_str().split_whitespace().collect();
        if words.iter().any(|s| s.starts_with("--")) {
            return Err(permanent("a cancel reason may not contain flags"));
        }
        let h = match orch_dispatch::hosted_handle(board, task.id()) {
            Some(handle) => self.handle(&handle)?,
            None => self.located(task, board),
        };
        Self::admit(&h, fence)?;
        let frozen = self.freeze(&h, task, board, fence)?;
        self.materialize(&h, &frozen)?;
        self.open_once(&frozen.config, &h)?;
        self.request_cancel(&h, &frozen.config, &words, reason)?;
        self.await_quiesced(&frozen.config, &h)?;
        let view = self
            .show(&frozen.config, &h)?
            .ok_or_else(|| transient("hosted task missing"))?;
        Self::progress(view)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_e2e;

#[cfg(all(test, target_os = "linux"))]
mod tests_confined;
