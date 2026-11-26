//! The real ports of `orch_bbp::builder` (#1318 S5): the worktree, `bbp
//! cgroup-open` and `jail.json`; `bbp slot-hold --slots` and `pidfd_getfd`
//! on its slot; the mux `own` connection; the ACP turn on orch's end of the
//! socketpair; the cgroup's own files for the end path; and the
//! `claude-builder` BBP seat for the D188 post.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use orch_bbp::acp::{Client, Turn, TurnError};
use orch_bbp::bounded::output_within;
use orch_bbp::builder::{self as seq, Outcome, Ports, Ran, Refused};
use orch_bbp::mux::{self, Ctl, Launch, Line, MuxError, Own};
use orch_jail::spec::{FileLimits, JailFile, JailSpec};
use orch_lens::builder::{self as lens_builder, Session};
use orch_lens::card::Card;
use orch_lens::LensConfig;
use platform::process::{Child, Command, EnvSpec, Signal, Stdio};
use platform_linux::{LinuxChild, LinuxFile, LinuxSpawner};
use rusty_bbp::ArtId;
use rusty_worktree::Repo;

use super::{prompt, replace, row, with_store, Options, AGENT, PROGRAM, SUBMIT};
use crate::lens::bbp::{run_id, Child as BbpChild, Session as Seat};
use crate::lens::run::board;

/// How long one step before the turn (`cgroup-open`, git) may take.
const STEP_WAIT: Duration = Duration::from_secs(120);
/// How long orch's own kill waits for the cgroup to empty.
const KILL_WAIT: Duration = Duration::from_secs(10);
/// How long the holder gets to exit once its stdin is closed and the
/// cgroup is empty.
const HOLDER_WAIT: Duration = Duration::from_secs(10);
/// How long a broken turn waits for `own` to say whether the mux went
/// away: its 25 s of reconnect tries, and a margin.
const VERDICT_WAIT: Duration = Duration::from_secs(30);
/// How often the turn's wait looks at the mux and the clock.
const TICK: Duration = Duration::from_millis(500);
/// System read roots, kept when they exist.
const SYSTEM: [&str; 5] = ["/usr", "/bin", "/lib", "/lib64", "/etc"];

/// Admit one session for `o.claim` and run it to its end.
pub fn run(o: &Options) -> Result<String, String> {
    let text = std::fs::read_to_string(&o.config)
        .map_err(|e| format!("cannot read {}: {e}", o.config.display()))?;
    let config = LensConfig::parse(&text).map_err(|e| format!("{}: {e}", o.config.display()))?;
    let base = Repo::open(&o.repo)?;
    let base = rev_parse(base.root(), &o.base)?;
    let board = board(&o.dir, &o.task)?;
    let child = BbpChild::spawn(&o.bbp, &o.dir, &o.task, AGENT)?;
    let mut seat = Seat::new(AGENT, child, &run_id()?).follow_every_domain();
    seat.refresh(&config)?;
    let card = seat
        .cards(&config)
        .into_iter()
        .find(|c| c.id == o.claim)
        .ok_or_else(|| {
            format!(
                "claim {} is not on the board in the config's domains",
                o.claim
            )
        })?;
    let limits = config.builders;
    let now = now_ms()?;
    let session = with_store(&o.state, |store| {
        let sessions = store
            .load_builder_sessions(&board)
            .map_err(|e| e.to_string())?;
        let admitted =
            lens_builder::admit(&limits, &card, &sessions, now).map_err(|e| e.to_string())?;
        store
            .save_builder_sessions(&board, &replace(sessions, &admitted))
            .map_err(|e| e.to_string())?;
        Ok(admitted)
    })?;
    let mut ports = Live::new(o, board, base, card, seat, &session, limits.wall_secs);
    let Outcome { session, notes } =
        seq::run(&mut ports, session, Duration::from_secs(limits.wall_secs))
            .map_err(|e| e.to_string())?;
    Ok(format!("{}\n", row(&session, &notes).to_json_string()))
}

/// Where one session's files live.
struct Paths {
    /// `<state>/builders/<claim>/<n>`: jail.json, its status file, acp.log.
    run: PathBuf,
    /// `<state>/wt/<claim>/<n>`: the only writable root.
    worktree: PathBuf,
    cgroup: PathBuf,
    /// The worktree's own git dir under the repository's, read before the
    /// agent runs; `.git` in the worktree is the agent's to rewrite.
    git_dir: Option<PathBuf>,
}

/// The ports, and what each step leaves for the next.
struct Live<'a> {
    o: &'a Options,
    board: String,
    base: String,
    card: Card,
    seat: Seat<BbpChild>,
    session: Session,
    paths: Paths,
    holder: Option<Holder>,
    own: Option<Own>,
    blocks: Vec<String>,
    acp: Option<UnixStream>,
    wall_secs: u64,
    /// The builder block's program exited during the turn.
    exited: bool,
}

/// The cgroup holder and what orch keeps of it.
struct Holder {
    child: LinuxChild,
    /// Closing it tells the holder it may go once the cgroup is empty.
    stdin: Option<std::io::PipeWriter>,
    /// orch's copy of the slot, until `own` has it at fd 3.
    slot: Option<OwnedFd>,
}

impl<'a> Live<'a> {
    fn new(
        o: &'a Options,
        board: String,
        base: String,
        card: Card,
        seat: Seat<BbpChild>,
        session: &Session,
        wall_secs: u64,
    ) -> Self {
        let name = format!("{}/{}", session.claim.0, session.n);
        let paths = Paths {
            run: o.state.join("builders").join(&name),
            worktree: o.state.join("wt").join(&name),
            cgroup: o.cgroup_parent.join(format!(
                "builder-{}-{}-{}",
                session.claim.0, session.n, session.started
            )),
            git_dir: None,
        };
        Self {
            o,
            board,
            base,
            card,
            seat,
            session: session.clone(),
            paths,
            holder: None,
            own: None,
            blocks: Vec::new(),
            acp: None,
            wall_secs,
            exited: false,
        }
    }

    fn jail_path(&self) -> PathBuf {
        self.paths.run.join("jail.json")
    }

    fn acp_log(&self) -> PathBuf {
        self.paths.run.join("acp.log")
    }

    fn launch(&self, window: &str) -> Launch {
        Launch {
            window: window.to_owned(),
            program: self.o.jail.display().to_string(),
            args: vec![self.jail_path().display().to_string()],
            cwd: Some(self.paths.worktree.display().to_string()),
            env: Some(mux::exact_env(|k| std::env::var(k).ok())),
            fds: 2,
            hold: true,
            owner_only: true,
            meta: Some([
                AGENT.to_owned(),
                "builder".to_owned(),
                format!(
                    "{}/{}/{}",
                    self.o.task, self.session.claim.0, self.session.n
                ),
            ]),
        }
    }

    fn open_own(&mut self) -> Result<(), MuxError> {
        let (ours, theirs) =
            UnixStream::pair().map_err(|e| MuxError::Ctl(format!("socketpair: {e}")))?;
        let slot = self
            .holder
            .as_mut()
            .and_then(|h| h.slot.take())
            .ok_or_else(|| MuxError::Ctl("no slot held".into()))?;
        let ctl = Ctl {
            bin: self.o.mux_ctl.clone(),
            socket: self.o.mux_socket.clone(),
        };
        let window = ctl.window(&format!("lel-{}", self.o.task), "builders")?;
        let mut own = Own::spawn(&ctl, &[slot.as_fd(), theirs.as_fd()])?;
        // `own` holds fds 3 and 4 now; orch keeps neither copy.
        drop((slot, theirs));
        self.acp = Some(ours);
        let launched = own.launch(&self.launch(&window));
        let block = match launched {
            Ok(block) => block,
            Err(e) => {
                own.finish();
                return Err(e);
            }
        };
        self.blocks.push(block);
        let viewer = mux::viewer(&window, &self.acp_log().display().to_string());
        match own.launch(&viewer) {
            Ok(v) => self.blocks.push(v),
            Err(e) => eprintln!("rusty_orch builder: no acp.log viewer: {e}"),
        }
        self.own = Some(own);
        Ok(())
    }

    /// Start the turn on its own thread; its answer comes on the receiver.
    fn start_turn(&mut self) -> Result<Receiver<Result<Turn, TurnError>>, String> {
        let ours = self.acp.as_ref().ok_or("no ACP socket")?;
        let reader = ours.try_clone().map_err(|e| format!("acp: {e}"))?;
        let writer = ours.try_clone().map_err(|e| format!("acp: {e}"))?;
        let log = File::create(self.acp_log()).map_err(|e| format!("acp.log: {e}"))?;
        let cwd = self.paths.worktree.clone();
        let text = prompt(&self.card, &cwd);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut client = Client::new(BufReader::new(reader), writer, log);
            let _ = tx.send(client.turn(&cwd, &text));
        });
        Ok(rx)
    }

    /// Wait for the turn, watching the mux and the wall clock.
    fn wait_turn(&mut self, rx: &Receiver<Result<Turn, TurnError>>, deadline_ms: u64) -> Ran {
        loop {
            match rx.recv_timeout(TICK) {
                Ok(Ok(turn)) => return Ran::Turn(turn),
                Ok(Err(e)) => return self.verdict(format!("{e:?}")),
                Err(RecvTimeoutError::Disconnected) => {
                    return Ran::Failed("turn thread died".into())
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            if let Some(own) = self.own.as_mut() {
                match own.next_event(Duration::ZERO) {
                    Err(MuxError::Lost(e)) => return Ran::Lost(e),
                    Ok(Some(line)) => {
                        self.exited |= exits(&line, self.blocks.first().map(String::as_str));
                    }
                    _ => {}
                }
            }
            if now_ms().map_or(true, |now| now >= deadline_ms) {
                return Ran::Wall;
            }
        }
    }

    /// A turn that broke off: lost when `own` says the mux went away
    /// (it retries a dropped connection for up to 25 s before `lost`),
    /// failed when the server reports the program exited or says nothing.
    fn verdict(&mut self, error: String) -> Ran {
        if self.exited {
            return Ran::Failed(error);
        }
        let builder = self.blocks.first().cloned();
        let Some(own) = self.own.as_mut() else {
            return Ran::Failed(error);
        };
        let until = Instant::now() + VERDICT_WAIT;
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            match own.next_event(left) {
                Err(MuxError::Lost(e)) => return Ran::Lost(e),
                Ok(Some(line)) if exits(&line, builder.as_deref()) => break,
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => break,
            }
        }
        Ran::Failed(error)
    }

    fn cgroup_empty(&self) -> Result<bool, String> {
        let path = self.paths.cgroup.join("cgroup.events");
        let events =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        populated(&events).map(|p| !p)
    }

    fn wait_empty(&self, wait: Duration) -> Result<bool, String> {
        let until = Instant::now() + wait;
        loop {
            if self.cgroup_empty()? {
                return Ok(true);
            }
            if Instant::now() >= until {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    fn post(&mut self) -> Result<Option<ArtId>, String> {
        let wt = &self.paths.worktree;
        if !wt.join(SUBMIT).is_file() {
            return Ok(None);
        }
        let git_dir = self.paths.git_dir.as_deref().ok_or("no worktree git dir")?;
        let diff = candidate_diff(git_dir, wt, &self.base)?;
        if diff.trim().is_empty() {
            return Ok(None);
        }
        let op = format!("builder-{}-{}", self.session.claim.0, self.session.n);
        self.seat
            .put_candidate(&op, &self.base, &diff, self.session.claim)
            .map(Some)
    }
}

impl Ports for Live<'_> {
    fn now_ms(&mut self) -> u64 {
        now_ms().unwrap_or(0)
    }

    fn save(&mut self, session: &Session) -> Result<(), String> {
        self.session = session.clone();
        with_store(&self.o.state, |store| {
            let sessions = store
                .load_builder_sessions(&self.board)
                .map_err(|e| e.to_string())?;
            store
                .save_builder_sessions(&self.board, &replace(sessions, session))
                .map_err(|e| e.to_string())
        })
    }

    fn prepare(&mut self, _: &Session) -> Result<(), String> {
        let p = &self.paths;
        std::fs::create_dir_all(&p.run).map_err(|e| format!("{}: {e}", p.run.display()))?;
        if let Some(parent) = p.worktree.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        refuse_gitlinks(&git(&self.o.repo, &["ls-tree", "-r", "-z", &self.base])?)?;
        let repo = Repo::open(&self.o.repo)?;
        let worktree = repo.add_detached(&p.worktree, &self.base)?;
        // Read now, before the agent can rewrite `.git` in its worktree.
        self.paths.git_dir = Some(admin_dir(&repo, &worktree)?);
        self.paths.worktree = worktree;
        bbp(
            &self.o.bbp,
            &["cgroup-open", "--task", &self.o.task, "--cgroup"],
            &self.o.dir,
            &self.paths.cgroup,
        )?;
        let status = self.paths.run.join("status");
        File::create(&status).map_err(|e| format!("{}: {e}", status.display()))?;
        let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_owned())?;
        let file = jail_file(&JailInputs {
            cgroup: &self.paths.cgroup,
            status: &status,
            worktree: &self.paths.worktree,
            home: Path::new(&home),
            extra: &self.o.read,
            // CPU seconds: the wall clock across a few cores' worth of build.
            cpu_secs: self.wall_secs.saturating_mul(4).max(1),
            env: mux::exact_env(|k| std::env::var(k).ok())
                .into_iter()
                .collect(),
        });
        JailSpec::validate(file.clone())?;
        let json = rusty_serde::json::to_string(&file).map_err(|e| e.to_string())?;
        std::fs::write(self.jail_path(), json).map_err(|e| format!("jail.json: {e}"))
    }

    fn hold(&mut self, deadline_ms: u64) -> Result<(), String> {
        let (child_in, stdin) = std::io::pipe().map_err(|e| format!("holder: {e}"))?;
        let (from_child, child_out) = std::io::pipe().map_err(|e| format!("holder: {e}"))?;
        let stdio = |fd: OwnedFd| Stdio::File(Box::new(LinuxFile::from(fd)));
        let mut cmd = Command::new(self.o.bbp.as_os_str(), ".").args([
            "slot-hold".into(),
            "--cgroup".into(),
            self.paths.cgroup.clone().into_os_string(),
            "--slots".into(),
            self.o.slots.clone().into_os_string(),
            "--deadline-ms".into(),
            deadline_ms.to_string().into(),
        ]);
        cmd.cwd = ".".into();
        cmd.env = EnvSpec::Inherit;
        cmd.stdin = stdio(OwnedFd::from(child_in));
        cmd.stdout = stdio(OwnedFd::from(child_out));
        let child = LinuxSpawner
            .spawn_passing(&cmd, &[])
            .map_err(|e| format!("holder: {e}"))?;
        drop(cmd);
        let slot = first_line(from_child, HOLDER_WAIT)
            .and_then(|first| slot_fd(&first))
            .and_then(|fd| child.dup_fd(fd).map_err(|e| format!("pidfd_getfd: {e}")));
        match slot {
            Ok(slot) => {
                self.holder = Some(Holder {
                    child,
                    stdin: Some(stdin),
                    slot: Some(slot),
                });
                Ok(())
            }
            Err(e) => {
                drop(stdin);
                let _ = child.kill_single(Signal::Kill);
                let _ = Box::new(child).wait();
                Err(format!("no slot for fd 3: {e}"))
            }
        }
    }

    fn open(&mut self) -> Result<(), Refused> {
        self.open_own().map_err(|e| match e {
            MuxError::Lost(e) => Refused::Lost(e),
            other => Refused::Failed(other.to_string()),
        })
    }

    fn turn(&mut self, deadline_ms: u64) -> Ran {
        let rx = match self.start_turn() {
            Ok(rx) => rx,
            Err(e) => return Ran::Failed(e),
        };
        let ran = self.wait_turn(&rx, deadline_ms);
        if let Some(acp) = &self.acp {
            // Ends a turn still reading, so its thread returns.
            let _ = acp.shutdown(std::net::Shutdown::Both);
        }
        ran
    }

    fn term(&mut self) -> Result<(), Refused> {
        let (Some(own), Some(block)) = (self.own.as_mut(), self.blocks.first()) else {
            return Ok(());
        };
        own.signal(block, "term").map_err(|e| match e {
            MuxError::Lost(e) => Refused::Lost(e),
            other => Refused::Failed(other.to_string()),
        })
    }

    fn empty_within(&mut self, grace: Duration) -> Result<bool, String> {
        self.wait_empty(grace)
    }

    fn kill(&mut self) -> Result<(), String> {
        let path = self.paths.cgroup.join("cgroup.kill");
        std::fs::write(&path, "1").map_err(|e| format!("{}: {e}", path.display()))?;
        match self.wait_empty(KILL_WAIT)? {
            true => Ok(()),
            false => Err(format!(
                "cgroup still populated {KILL_WAIT:?} after the kill"
            )),
        }
    }

    fn release(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        if let Some(mut own) = self.own.take() {
            for block in &self.blocks {
                if let Err(e) = own.release(block) {
                    errors.push(format!("release {block}: {e}"));
                }
            }
            own.finish();
        }
        if let Some(holder) = self.holder.take() {
            if let Err(e) = let_go(holder) {
                errors.push(e);
            }
        }
        match errors.is_empty() {
            true => Ok(()),
            false => Err(errors.join("; ")),
        }
    }

    fn submit(&mut self) -> Result<Option<ArtId>, String> {
        self.post()
    }
}

/// Close the holder's stdin and wait for it to go: it leaves once the
/// cgroup is empty.
fn let_go(mut holder: Holder) -> Result<(), String> {
    holder.stdin.take();
    let until = Instant::now() + HOLDER_WAIT;
    while Instant::now() < until {
        if matches!(holder.child.try_wait(), Ok(Some(_))) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!(
        "holder still running {HOLDER_WAIT:?} after its stdin closed; its deadline ends it"
    ))
}

/// The first line `from` writes, within `wait`: a holder that neither
/// prints its slot nor exits (one too old to know `--slots`) must not hang
/// the launch.
fn first_line(from: std::io::PipeReader, wait: Duration) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut first = String::new();
        let read = BufReader::new(from).read_line(&mut first);
        let _ = tx.send(read.map(|_| first).map_err(|e| e.to_string()));
    });
    rx.recv_timeout(wait)
        .map_err(|_| format!("the holder printed no slot within {wait:?}"))?
}

/// What a session's `jail.json` is made from.
pub struct JailInputs<'a> {
    pub cgroup: &'a Path,
    pub status: &'a Path,
    pub worktree: &'a Path,
    pub home: &'a Path,
    pub extra: &'a [PathBuf],
    pub cpu_secs: u64,
    pub env: BTreeMap<String, String>,
}

/// The builder's `jail.json` (D107, D166, D187, D189): the worktree is the
/// only write root; the system roots that exist, `~/.claude` and `extra`
/// are read roots; no token; `sh` puts ACP on the agent's stdio and closes
/// fd 4 so the agent's tools can't reach it.
pub fn jail_file(i: &JailInputs<'_>) -> JailFile {
    let mut read: Vec<String> = SYSTEM
        .iter()
        .filter(|p| Path::new(p).exists())
        .map(|p| (*p).to_owned())
        .collect();
    read.push(i.home.join(".claude").display().to_string());
    read.extend(i.extra.iter().map(|p| p.display().to_string()));
    let worktree = i.worktree.display().to_string();
    JailFile {
        cgroup: i.cgroup.display().to_string(),
        status: i.status.display().to_string(),
        cwd: worktree.clone(),
        read,
        write: vec![worktree],
        limits: FileLimits {
            cpu_secs: i.cpu_secs,
            memory_bytes: 16 << 30,
            file_bytes: 4 << 30,
            open_files: 4096,
            processes: 512,
        },
        env: i.env.clone(),
        token: None,
        program: "/bin/sh".into(),
        args: vec!["-c".into(), format!("exec {PROGRAM} <&4 >&4 4>&-")],
    }
}

/// The descriptor number `slot-hold --slots` printed as its first line.
pub fn slot_fd(line: &str) -> Result<i32, String> {
    line.trim()
        .parse()
        .ok()
        .filter(|fd: &i32| *fd >= 0)
        .ok_or_else(|| {
            format!(
                "the holder printed {:?}, not a descriptor (every slot held?)",
                line.trim()
            )
        })
}

/// `populated` from a `cgroup.events` file.
pub fn populated(events: &str) -> Result<bool, String> {
    events
        .lines()
        .find_map(|l| l.strip_prefix("populated "))
        .and_then(|v| match v.trim() {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        })
        .ok_or_else(|| format!("cgroup.events has no populated line: {events:?}"))
}

fn now_ms() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = StdCommand::new("git");
    cmd.arg("-C").arg(dir).args(args);
    git_out(cmd, args)
}

/// True for `block`'s `program-exited`: the mux is up, so a broken turn
/// is the agent's failure, not a lost mux. The viewer's exit doesn't count.
fn exits(line: &Line, block: Option<&str>) -> bool {
    let Line::Event { kind, body } = line else {
        return false;
    };
    kind == "program-exited"
        && block.is_some()
        && body.get("block").and_then(rusty_json::Value::as_str) == block
}

/// The most entries [`nested_repo`] looks at before it gives up.
const WALK_ENTRIES: usize = 1_000_000;

/// The first nested `.git` (file or directory) below `root`, its own
/// `.git` aside. `git add` steps into a nested repository and reads its
/// config (a `filter.*.clean` there runs as orch), so any one refuses
/// the diff. Symlinks are not followed; a symlink at a submodule path is
/// [`refuse_gitlinks`]'s. Bounded by [`WALK_ENTRIES`] and [`STEP_WAIT`].
fn nested_repo(root: &Path) -> Result<Option<PathBuf>, String> {
    let until = Instant::now() + STEP_WAIT;
    let mut seen = 0usize;
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            seen += 1;
            if seen > WALK_ENTRIES || Instant::now() >= until {
                return Err(format!(
                    "worktree walk passed {WALK_ENTRIES} entries or {STEP_WAIT:?}: no diff taken"
                ));
            }
            let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
            let path = entry.path();
            if entry.file_name() == ".git" {
                if dir != root {
                    return Ok(Some(path));
                }
                continue;
            }
            let kind = entry
                .file_type()
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if kind.is_dir() {
                dirs.push(path);
            }
        }
    }
    Ok(None)
}

/// The first gitlink (mode 160000) in an `ls-tree -r -z` listing refuses
/// the base. At a submodule path git reads whatever repository it finds
/// (a nested one runs its filters as orch), and a symlink there can point
/// anywhere, so builders take no base with submodules.
fn refuse_gitlinks(listing: &str) -> Result<(), String> {
    match tree_gitlink(listing) {
        Some(path) => Err(format!(
            "the base has a submodule at {path}: builders take no base with submodules"
        )),
        None => Ok(()),
    }
}

/// The path of the first mode-160000 entry in an `ls-tree -r -z` listing,
/// where each entry is `<mode> <type> <oid>\t<path>`.
fn tree_gitlink(listing: &str) -> Option<&str> {
    listing
        .split('\0')
        .filter_map(|entry| entry.split_once('\t'))
        .find(|(head, _)| head.starts_with("160000 "))
        .map(|(_, path)| path)
}

/// The path of the first entry with mode 160000 on either side in a
/// `diff --raw -z --no-renames` listing, where each `:<old> <new> …`
/// header is followed by its one path, so a path can't pass for a header.
fn staged_gitlink(raw: &str) -> Option<&str> {
    let mut fields = raw.split('\0');
    while let (Some(head), Some(path)) = (fields.next(), fields.next()) {
        // A field that isn't a header means the listing isn't what was
        // asked for: refuse rather than guess.
        let Some(modes) = head.strip_prefix(':') else {
            return Some(head);
        };
        if modes.split(' ').take(2).any(|m| m == "160000") {
            return Some(path);
        }
    }
    None
}

/// git on `work_tree` through orch's own record of its git dir, with
/// hooks, fsmonitor and external diff and textconv drivers off: nothing
/// the agent wrote in the worktree (a new `.git`, attributes) can make
/// orch run a command outside the jail.
fn sealed_git(git_dir: &Path, work_tree: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = StdCommand::new("git");
    // Inherited `GIT_*` goes first, so the sealed values below stay.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(name);
        }
    }
    cmd.arg("--git-dir")
        .arg(git_dir)
        .arg("--work-tree")
        .arg(work_tree)
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(["-c", "core.untrackedCache=false", "-c", "diff.external="])
        .current_dir(work_tree)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        // Nor the operator's global config: a `filter.lfs` there would let
        // the agent's `.gitattributes` start `git-lfs` as orch.
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd.args(args);
    git_out(cmd, args)
}

/// The worktree's diff against `base`, refused for a base with a
/// submodule (see [`refuse_gitlinks`]; prepare checked it too) and while
/// any nested repository is present (see [`nested_repo`]).
fn candidate_diff(git_dir: &Path, work_tree: &Path, base: &str) -> Result<String, String> {
    refuse_gitlinks(&sealed_git(
        git_dir,
        work_tree,
        &["ls-tree", "-r", "-z", base],
    )?)?;
    if let Some(nested) = nested_repo(work_tree)? {
        return Err(format!(
            "nested repository at {}: no diff taken",
            nested.display()
        ));
    }
    staged_diff(git_dir, work_tree, base)
}

/// The candidate's diff: everything in `work_tree` but `.lel/`, staged,
/// against `base`.
fn staged_diff(git_dir: &Path, work_tree: &Path, base: &str) -> Result<String, String> {
    sealed_git(
        git_dir,
        work_tree,
        &["add", "-A", "--", ".", ":(exclude).lel"],
    )?;
    // Backstop: nothing staged may be a gitlink either.
    let staged = sealed_git(
        git_dir,
        work_tree,
        &["diff", "--cached", "--raw", "-z", "--no-renames", base],
    )?;
    if let Some(path) = staged_gitlink(&staged) {
        return Err(format!("a submodule was staged at {path}: no diff taken"));
    }
    sealed_git(
        git_dir,
        work_tree,
        &[
            "diff",
            "--cached",
            "--binary",
            "--no-ext-diff",
            "--no-textconv",
            base,
        ],
    )
}

/// The git dir `git worktree add` made for `worktree`: the one its `.git`
/// names, which must sit under the repository's `worktrees/`.
fn admin_dir(repo: &Repo, worktree: &Path) -> Result<PathBuf, String> {
    let dotgit = std::fs::read_to_string(worktree.join(".git"))
        .map_err(|e| format!("{}/.git: {e}", worktree.display()))?;
    let named = dotgit
        .trim()
        .strip_prefix("gitdir: ")
        .ok_or_else(|| format!("{}/.git is not a worktree link", worktree.display()))?;
    let admin = Path::new(named)
        .canonicalize()
        .map_err(|e| format!("{named}: {e}"))?;
    let root = repo
        .git_dir()
        .join("worktrees")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    match admin.starts_with(&root) {
        true => Ok(admin),
        false => Err(format!(
            "{} is not under {}",
            admin.display(),
            root.display()
        )),
    }
}

fn git_out(cmd: StdCommand, args: &[&str]) -> Result<String, String> {
    let out = output_within(cmd, STEP_WAIT).map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("git {}: {e}", args.join(" ")))
}

fn rev_parse(repo: &Path, rev: &str) -> Result<String, String> {
    Ok(git(
        repo,
        &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
    )?
    .trim()
    .to_owned())
}

/// `bbp <args> <path> --dir <dir>`, failing on a non-zero exit.
fn bbp(bin: &Path, args: &[&str], dir: &Path, path: &Path) -> Result<(), String> {
    let mut cmd = StdCommand::new(bin);
    cmd.args(args).arg(path).arg("--dir").arg(dir);
    let out = output_within(cmd, STEP_WAIT).map_err(|e| format!("{}: {e}", bin.display()))?;
    if !out.status.success() {
        return Err(format!(
            "bbp {}: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, script: &str) {
        let ok = StdCommand::new("sh")
            .args(["-c", script])
            .current_dir(dir)
            .status()
            .expect("sh runs")
            .success();
        assert!(ok, "{script}");
    }

    /// A repo with a submodule at `sub`, its worktree and admin dir.
    fn with_submodule(root: &Path) -> (PathBuf, String, PathBuf) {
        let _ = std::fs::remove_dir_all(root);
        std::fs::create_dir_all(root).expect("temp dir");
        let c = "git -c user.name=t -c user.email=t@t -c protocol.file.allow=always";
        sh(
            root,
            &format!(
                "git init -q lib && cd lib && echo l > l && git add l && {c} commit -qm l && cd .. && \
                 git init -q repo && cd repo && echo a > a && git add a && \
                 {c} submodule -q add ../lib sub && {c} commit -qm a"
            ),
        );
        let repo = Repo::open(&root.join("repo")).expect("repo");
        let base = rev_parse(&root.join("repo"), "HEAD").expect("base");
        let wt = repo
            .add_detached(&root.join("wt"), &base)
            .expect("worktree");
        let admin = admin_dir(&repo, &wt).expect("admin dir");
        (wt, base, admin)
    }

    #[test]
    fn a_nested_repo_at_a_submodule_path_is_refused_before_git_runs() {
        let root = std::env::temp_dir().join(format!("orch-nested-{}", std::process::id()));
        let (wt, base, admin) = with_submodule(&root);
        // The agent makes `sub` a repo whose clean filter runs its script.
        let marker = root.join("ran");
        sh(
            &root,
            &format!(
                "printf '#!/bin/sh\\ntouch {m}\\ncat\\n' > hook && chmod +x hook && \
                 cd wt/sub && git init -q && git fetch -q ../../lib && git checkout -q FETCH_HEAD && \
                 git config filter.e.clean \"$PWD/../../hook\" && \
                 echo '* filter=e' > .gitattributes && git add .gitattributes && rm {m} && touch l .gitattributes",
                m = marker.display()
            ),
        );
        let err = candidate_diff(&admin, &wt, &base).expect_err("refused");
        assert!(err.contains("submodule at sub"), "{err}");
        assert!(!marker.exists(), "the nested repo's filter ran");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_symlink_at_a_submodule_path_is_refused_before_git_runs() {
        let root = std::env::temp_dir().join(format!("orch-gitlink-{}", std::process::id()));
        let (wt, base, admin) = with_submodule(&root);
        // The review's case: the agent points its own `.git` at a repo
        // whose fsmonitor runs its script and makes `sub` a symlink back to
        // the worktree. It is refused on the base's gitlink before git
        // touches the worktree, whether or not this git would run the hook.
        let marker = root.join("ran");
        sh(
            &root,
            &format!(
                "git init -q evil && printf '#!/bin/sh\\ntouch {m}\\n' > hook && chmod +x hook && \
                 git -C evil config core.fsmonitor \"$PWD/hook\" && \
                 rm wt/.git && echo \"gitdir: $PWD/evil/.git\" > wt/.git && \
                 rm -rf wt/sub && ln -s . wt/sub",
                m = marker.display()
            ),
        );
        let err = candidate_diff(&admin, &wt, &base).expect_err("refused");
        assert!(err.contains("submodule at sub"), "{err}");
        assert!(!marker.exists(), "the planted fsmonitor ran");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_staged_gitlink_shows_in_the_raw_diff() {
        let raw = ":000000 160000 0000 1111 A\x00sub\x00:100644 100644 2222 3333 M\x00a\x00";
        assert_eq!(staged_gitlink(raw), Some("sub"));
        assert_eq!(staged_gitlink(":100644 100644 2 3 M\x00a\x00"), None);
        assert_eq!(
            staged_gitlink(":160000 000000 1 0 D\x00gone\x00"),
            Some("gone")
        );
        // A path that looks like a header or an ls-tree entry is only a path.
        let odd = ":000000 100644 0 1 A\x00160000 notes.txt\x00:000000 100644 0 2 A\x00:000000 160000\x00";
        assert_eq!(staged_gitlink(odd), None);
        assert_eq!(tree_gitlink("100644 blob 0a\t160000 notes.txt\x00"), None);
    }

    #[test]
    fn a_listing_without_gitlinks_passes() {
        let listing = "100644 blob 0a\ta\x00040000 tree 1b\td\0";
        assert_eq!(refuse_gitlinks(listing), Ok(()));
        let err =
            refuse_gitlinks("100644 blob 0a\ta\x00160000 commit 2c\tx y\0").expect_err("gitlink");
        assert!(err.contains("submodule at x y"), "{err}");
    }

    #[test]
    fn a_nested_repo_elsewhere_refuses_the_diff() {
        let root = std::env::temp_dir().join(format!("orch-nested2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        sh(
            &root,
            "git init -q repo && cd repo && echo a > a && git add a && \
             git -c user.name=t -c user.email=t@t commit -qm a",
        );
        let repo = Repo::open(&root.join("repo")).expect("repo");
        let base = rev_parse(&root.join("repo"), "HEAD").expect("base");
        let wt = repo
            .add_detached(&root.join("wt"), &base)
            .expect("worktree");
        let admin = admin_dir(&repo, &wt).expect("admin dir");
        sh(&wt, "mkdir -p d && git init -q d/inner");
        let err = candidate_diff(&admin, &wt, &base).expect_err("refused");
        assert!(err.contains("nested repository"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_worktree_s_own_git_is_not_a_nested_repo() {
        let root = std::env::temp_dir().join(format!("orch-own-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b")).expect("dirs");
        std::fs::write(root.join(".git"), "gitdir: x").expect("own .git");
        assert_eq!(nested_repo(&root).expect("walk"), None);
        std::fs::write(root.join("a/b/.git"), "gitdir: y").expect("nested .git");
        assert_eq!(
            nested_repo(&root).expect("walk"),
            Some(root.join("a/b/.git"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_diff_ignores_a_git_dir_the_agent_planted() {
        let root = std::env::temp_dir().join(format!("orch-sealed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        sh(
            &root,
            "git init -q repo && cd repo && echo a > a && git add a && \
             git -c user.name=t -c user.email=t@t commit -qm a",
        );
        let repo = Repo::open(&root.join("repo")).expect("repo");
        let base = rev_parse(&root.join("repo"), "HEAD").expect("base");
        let wt = repo
            .add_detached(&root.join("wt"), &base)
            .expect("worktree");
        let admin = admin_dir(&repo, &wt).expect("admin dir");
        // The agent swaps `.git` for a repo whose fsmonitor runs its script.
        let marker = root.join("ran");
        sh(
            &root,
            &format!(
                "git init -q evil && printf '#!/bin/sh\\ntouch {m}\\n' > hook && chmod +x hook && \
                 git -C evil config core.fsmonitor \"$PWD/hook\" && \
                 rm wt/.git && echo \"gitdir: $PWD/evil/.git\" > wt/.git && echo b > wt/a",
                m = marker.display()
            ),
        );
        let diff = candidate_diff(&admin, &wt, &base).expect("diff");
        assert!(diff.contains("+b"), "{diff}");
        assert!(!marker.exists(), "the planted fsmonitor ran");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_jail_file_holds_the_v0_contract() {
        let extra = [PathBuf::from("/opt/node")];
        let env: BTreeMap<String, String> = [("PATH".to_owned(), "/usr/bin".to_owned())].into();
        let file = jail_file(&JailInputs {
            cgroup: Path::new("/sys/fs/cgroup/lel-store-ab/builder-7-1-10"),
            status: Path::new("/s/builders/7/1/status"),
            worktree: Path::new("/s/wt/7/1"),
            home: Path::new("/home/nano"),
            extra: &extra,
            cpu_secs: 10_800,
            env,
        });
        assert_eq!(file.write, ["/s/wt/7/1"]);
        assert_eq!(file.cwd, "/s/wt/7/1");
        assert!(file.read.contains(&"/home/nano/.claude".to_owned()));
        assert!(file.read.contains(&"/opt/node".to_owned()));
        assert!(file.read.contains(&"/usr".to_owned()));
        assert_eq!(file.token, None, "no token in v0 (D187)");
        assert_eq!(file.program, "/bin/sh");
        assert_eq!(file.args[1], "exec claude-code-acp <&4 >&4 4>&-");
        JailSpec::validate(file).expect("lel-jail accepts it");
    }

    #[test]
    fn the_holder_line_is_a_descriptor_or_a_refusal() {
        assert_eq!(slot_fd("5\n"), Ok(5));
        assert!(slot_fd("").is_err(), "every slot held: nothing printed");
        assert!(slot_fd("-1").is_err());
        assert!(slot_fd("slot 5").is_err());
    }

    #[test]
    fn a_silent_holder_is_given_up_on() {
        use std::io::Write;
        let (r, mut w) = std::io::pipe().expect("pipe");
        let wait = Duration::from_millis(100);
        assert!(first_line(r, wait).unwrap_err().contains("no slot"));
        let (r, w2) = std::io::pipe().expect("pipe");
        let mut w2 = w2;
        writeln!(w2, "5").expect("write");
        assert_eq!(first_line(r, wait), Ok("5\n".to_owned()));
        let _ = w.flush();
    }

    #[test]
    fn cgroup_events_read_populated() {
        assert_eq!(populated("populated 1\nfrozen 0\n"), Ok(true));
        assert_eq!(populated("frozen 0\npopulated 0\n"), Ok(false));
        assert!(populated("frozen 0\n").is_err());
        assert!(populated("populated x\n").is_err());
    }
}
