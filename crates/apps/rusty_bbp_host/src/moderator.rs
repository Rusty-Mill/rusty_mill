//! The moderator loop: `bbp mod`. The core decides who holds the turn; this
//! loop makes it happen. Each iteration it re-reads the store, fires
//! deadlines, starts the runner for a pending selected run, launches one
//! agent harness for a newly granted turn with a `bbp mcp` binding in its
//! environment, and reaps a harness whose turn has ended. Human gates are
//! states with no turn, so the loop simply waits through them.
//!
//! The harness never sees a token: it gets `BBP_DIR`, `BBP_TASK`,
//! `BBP_PRINCIPAL` and `BBP_TURN`, and talks to the per-turn `bbp mcp`
//! server, which derives the token under the directory's master secret.

use crate::runner::{self, Confinement, Unconfined};
use crate::{now, open_driver};
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Proc, Stdio};
use std::time::{Duration, Instant};

/// How to start one role's agent harness. Every argument may use the
/// placeholders `{dir}`, `{task}`, `{principal}`, `{turn}`, `{role}`,
/// `{bbp}` (this binary) and `{mcp_config}` (a Claude Code MCP config file
/// for the turn's `bbp mcp` server).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launcher {
    pub program: String,
    pub args: Vec<String>,
}

/// One launcher per role; a role without one is left to its deadline.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launchers {
    #[rusty_serde(default)]
    pub planner: Option<Launcher>,
    #[rusty_serde(default)]
    pub coder: Option<Launcher>,
    #[rusty_serde(default)]
    pub tester: Option<Launcher>,
    #[rusty_serde(default)]
    pub reviewer: Option<Launcher>,
}

impl Launchers {
    pub fn read_file(file: &Path) -> Result<Launchers, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))
    }

    pub fn for_role(&self, role: Role) -> Option<&Launcher> {
        match role {
            Role::Planner => self.planner.as_ref(),
            Role::Coder => self.coder.as_ref(),
            Role::Tester => self.tester.as_ref(),
            Role::Reviewer => self.reviewer.as_ref(),
        }
    }
}

pub struct Config {
    pub dir: PathBuf,
    pub task: TaskId,
    /// The `bbp` binary the harness and its MCP config call; `bbp mod` passes
    /// its own path.
    pub bbp: PathBuf,
    /// Local path of the task's repository, for the runner.
    pub repo: PathBuf,
    /// One checkout per run goes under here.
    pub work: PathBuf,
    pub launchers: Launchers,
    pub confinement: Confinement,
    /// How long to sleep when nothing is ready.
    pub poll: Duration,
    /// Give up after this long; the task is left where it stands.
    pub max_wall: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub state: State,
    pub turns_launched: u64,
    pub runs: u64,
}

struct Harness {
    turn: TurnId,
    child: Child,
}

/// Run the loop until the task is closed or cancelled, or `max_wall` passes.
pub fn run(cfg: &Config) -> Result<Outcome, String> {
    let mut d = open_driver(&cfg.dir, &cfg.task)?;
    let started = Instant::now();
    let mut harness: Option<Harness> = None;
    let mut launched: HashSet<TurnId> = HashSet::new();
    let mut ran: HashSet<RunId> = HashSet::new();
    loop {
        d.sync().map_err(|e| format!("{e:?}"))?;
        d.dispatch(&Command::Tick, now())
            .map_err(|e| format!("{e:?}"))?;
        let state = d.state.state;
        if matches!(state, State::Closed | State::Cancelled) {
            kill(&mut harness);
            return Ok(Outcome {
                state,
                turns_launched: launched.len() as u64,
                runs: ran.len() as u64,
            });
        }
        if started.elapsed() > cfg.max_wall {
            kill(&mut harness);
            return Err(format!("moderator: wall limit reached in state {state:?}"));
        }

        // A selected run with no report yet: the runner's job, once per run.
        let pending = (state == State::Test)
            .then(|| d.state.selected_run())
            .flatten()
            .filter(|r| r.status.is_none() && !ran.contains(&r.id))
            .map(|r| r.id);
        if let Some(run) = pending {
            ran.insert(run);
            match run_once(cfg) {
                Ok(r) => eprintln!("bbp mod: run {} served: {r:?}", run.0),
                Err(e) => eprintln!("bbp mod: run {} not served: {e}", run.0),
            }
            continue;
        }

        let live = d.state.turn.as_ref().map(|t| (t.id, t.role));

        // Reap: a harness that exited without ending its turn forfeits it;
        // one whose turn ended under it is killed.
        if let Some(h) = harness.as_mut() {
            match h.child.try_wait() {
                Ok(Some(status)) => {
                    eprintln!("bbp mod: harness for turn {} exited: {status}", h.turn.0);
                    let turn = h.turn;
                    harness = None;
                    if live.map(|(id, _)| id) == Some(turn) {
                        d.dispatch(&Command::AbortTurn, now())
                            .map_err(|e| format!("{e:?}"))?;
                        continue;
                    }
                }
                Ok(None) if live.map(|(id, _)| id) != Some(h.turn) => {
                    eprintln!("bbp mod: turn {} ended; stopping its harness", h.turn.0);
                    kill(&mut harness);
                }
                Ok(None) => {}
                Err(e) => return Err(format!("harness wait: {e}")),
            }
        }

        // Launch: one harness per granted turn.
        if harness.is_none() {
            if let Some((turn, role)) = live {
                if !launched.contains(&turn) {
                    launched.insert(turn);
                    match cfg.launchers.for_role(role) {
                        Some(l) => {
                            let principal = d
                                .state
                                .assigned
                                .get(&role)
                                .cloned()
                                .ok_or_else(|| format!("{role:?} has no principal"))?;
                            let child = spawn(cfg, l, role, &principal, turn)?;
                            harness = Some(Harness { turn, child });
                            continue;
                        }
                        None => eprintln!(
                            "bbp mod: no launcher for {role:?}; turn {} waits for its deadline",
                            turn.0
                        ),
                    }
                }
            }
        }
        std::thread::sleep(cfg.poll);
    }
}

fn run_once(cfg: &Config) -> Result<Response, String> {
    match cfg.confinement {
        Confinement::Unconfined => runner::run_once(
            &cfg.dir,
            &cfg.task,
            &cfg.repo,
            &cfg.work,
            &Unconfined,
            Confinement::Unconfined,
        ),
        Confinement::Sandboxed => {
            let exec = runner::sandboxed(&cfg.dir.join("sandbox-state"))?;
            runner::run_once(
                &cfg.dir,
                &cfg.task,
                &cfg.repo,
                &cfg.work,
                &exec,
                Confinement::Sandboxed,
            )
        }
    }
}

fn kill(h: &mut Option<Harness>) {
    if let Some(mut h) = h.take() {
        let _ = h.child.kill();
        let _ = h.child.wait();
    }
}

fn json_str(s: &str) -> String {
    rusty_serde::json::to_string(&s).unwrap_or_else(|_| "\"\"".into())
}

/// Write the Claude Code MCP config for one turn and return its path.
fn mcp_config(
    cfg: &Config,
    bbp: &Path,
    principal: &PrincipalId,
    turn: TurnId,
) -> Result<PathBuf, String> {
    let dir = cfg.dir.join("mcp");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("turn-{}.json", turn.0));
    let text = format!(
        "{{\"mcpServers\":{{\"bbp\":{{\"command\":{},\"args\":[\"mcp\"],\"env\":{{\"BBP_DIR\":{},\"BBP_TASK\":{},\"BBP_PRINCIPAL\":{},\"BBP_TURN\":\"{}\"}}}}}}}}\n",
        json_str(&bbp.to_string_lossy()),
        json_str(&cfg.dir.to_string_lossy()),
        json_str(&cfg.task.0),
        json_str(&principal.0),
        turn.0
    );
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path)
}

fn spawn(
    cfg: &Config,
    l: &Launcher,
    role: Role,
    principal: &PrincipalId,
    turn: TurnId,
) -> Result<Child, String> {
    let bbp = &cfg.bbp;
    let mcp = mcp_config(cfg, bbp, principal, turn)?;
    let role_name = crate::admin::role_name(role);
    let fill = |a: &str| {
        a.replace("{dir}", &cfg.dir.to_string_lossy())
            .replace("{task}", &cfg.task.0)
            .replace("{principal}", &principal.0)
            .replace("{turn}", &turn.0.to_string())
            .replace("{role}", role_name)
            .replace("{bbp}", &bbp.to_string_lossy())
            .replace("{mcp_config}", &mcp.to_string_lossy())
    };
    let logs = cfg.dir.join("agents");
    std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    let log = std::fs::File::create(logs.join(format!("turn-{}.log", turn.0)))
        .map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    eprintln!("bbp mod: launching {role_name} for turn {}", turn.0);
    Proc::new(fill(&l.program))
        .args(l.args.iter().map(|a| fill(a)))
        .env("BBP_DIR", &cfg.dir)
        .env("BBP_TASK", &cfg.task.0)
        .env("BBP_PRINCIPAL", &principal.0)
        .env("BBP_TURN", turn.0.to_string())
        .env("BBP_ROLE", role_name)
        .env("BBP_BIN", bbp)
        .env("BBP_MCP_CONFIG", &mcp)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", l.program))
}
