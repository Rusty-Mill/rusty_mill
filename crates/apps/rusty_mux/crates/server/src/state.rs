//! The server's state: the model, each block's PTY and grid, each client's queue.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use rusty_mux_core::{
    BlockId, ClientId, EntityId, Error, Event, ExitStatus, InputMode, Mux, Principal, Side, Size,
    Transport, WindowId,
};
use rusty_mux_proto::schema::{self, CallError, Value};
use rusty_mux_proto::theme::ThemeChange;
use rusty_mux_proto::{
    Code, Cursor, DatagramKey, Echo, Entry, Features, Launch, Message, Ownership, Takeover, encode,
};
use rusty_mux_status::{Parts, Status};
use rusty_term_core::{AnsiParser, Grid};
use rusty_tokio::sync::{Notify, oneshot};
use rusty_tokio::task::JoinHandle;
use rusty_uuid::Uuid;

use crate::Config;
use crate::checkpoint::validate_size;
use crate::datagram::Registry;
use crate::echo::Waiting;
use crate::history::History;
use crate::keys;
use crate::outbox::Outbox;
use crate::process;
use crate::pty::{self, Process, Pty};
use crate::spool::Spool;
use crate::store::{Started, Store};
use crate::terminal;

mod persist;
pub(crate) use persist::save_periodically;

pub(crate) type Shared = Rc<RefCell<State>>;

/// A lease silence released: the client, its block, and the cancel of
/// the connection that served it then.
pub(crate) type Released = (ClientId, BlockId, Rc<Notify>);

/// Bytes read from a PTY per output frame.
const CHUNK: usize = 16 << 10;

/// The largest reply to a block call or `list`, in encoded bytes: half a
/// client's output budget, so a reply alone never gets a client dropped.
const REPLY_MAX: usize = crate::outbox::BUDGET / 2;

/// Events kept for clients that reconnect.
const HISTORY: usize = 4096;

/// How often every block's foreground process group and echo mode are
/// checked, to see a job start or echo turned off that writes nothing;
/// output checks its own block at once.
const PROCESS_POLL: Duration = Duration::from_millis(500);

/// How long a client that can be taken over outlives its connection, so a
/// reconnect finds it, attachment, lease and all.
pub(crate) const ORPHAN_GRACE: Duration = Duration::from_secs(30);

/// What the server offers; a client gets the intersection with its own.
const OFFERED: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::SYNC)
    .union(Features::INSPECT)
    .union(Features::BLOCK_CALL)
    .union(Features::STATUS)
    .union(Features::TERMINAL_EVENTS)
    .union(Features::PROGRAM_EVENTS)
    .union(Features::LAUNCH)
    .union(Features::PROCESS)
    .union(Features::CAPTURE)
    .union(Features::LABELS)
    .union(Features::LAYOUT)
    .union(Features::TAKEOVER)
    .union(Features::ECHO_ACK)
    .union(Features::IDENTITY)
    .union(Features::MORE_PROGRAM_EVENTS)
    .union(Features::ACTIVE_WINDOW)
    .union(Features::CONFIG_RELOAD)
    .union(Features::TERMINAL_CONTROL)
    .union(Features::LAYOUT_EDIT)
    .union(Features::FOCUS)
    .union(Features::SESSION_NAMES)
    .union(Features::NAME_SESSION)
    .union(Features::TERMINAL_MORE)
    .union(Features::TERMINAL_SHELL)
    .union(Features::AGENT_STATUS)
    .union(Features::TERMINAL_HISTORY)
    .union(Features::OWNERSHIP)
    .union(Features::INPUT_GATE)
    .union(Features::LEASE_REQUESTS)
    .union(Features::SHUTDOWN)
    .union(Features::BACKGROUND_HISTORY)
    .union(Features::HOLD)
    .union(UNIX_ONLY);

/// Offered on Linux only: signal and release (C76) act on a Unix process
/// group, and orch, their one client, runs there.
#[cfg(target_os = "linux")]
const UNIX_ONLY: Features = Features::SIGNAL_RELEASE;
#[cfg(not(target_os = "linux"))]
const UNIX_ONLY: Features = Features::NONE;

/// An owner whose outbox holds more than this can't be told of more
/// input, so an owner-only block refuses it (C77): half the budget, so a
/// refusal comes well before the owner is dropped.
const INPUT_REPORT_LIMIT: usize = crate::outbox::BUDGET / 2;

/// Why a request that needs a block's program is refused once there is
/// none (C76).
#[cfg(target_os = "linux")]
const EXITED: &str = "exited: the block's program is no longer running";

/// The server's clock in unix ms, which grants are given in (C77).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Fresh randomness to pick a generated session name with.
fn name_seed() -> u64 {
    let bytes = Uuid::new_v4();
    let mut seed = [0; 8];
    seed.copy_from_slice(&bytes.as_bytes()[..8]);
    u64::from_le_bytes(seed)
}

/// Input to write to a PTY once the state is no longer borrowed.
pub(crate) struct PtyWrite {
    pub(crate) pty: Rc<Pty>,
    pub(crate) data: Vec<u8>,
    /// With [`Features::ECHO_ACK`]: the block and `Input` request to
    /// [`State::written`] once the write returns.
    pub(crate) key: Option<(BlockId, u64)>,
    /// On an owner-only block: the block and writer to report the bytes
    /// for, with [`State::report_input`], once the write returns (C77).
    pub(crate) report: Option<(BlockId, ClientId)>,
}

/// What a connection must finish before it reads its next request, so
/// requests on one connection complete in order (`sync` is a fence).
pub(crate) enum Pending {
    /// A PTY write.
    Write(PtyWrite),
    /// A `reload-config` reading off this thread; resolves once it has been
    /// applied or refused and answered.
    Reload(oneshot::Receiver<()>),
}

struct BlockRt {
    /// `None` for a read-only block: one restored after a restart whose
    /// command was not run again (C45). It shows its saved screen until
    /// closed and refuses anything that needs a program.
    live: Option<Live>,
    grid: Grid,
    parser: AnsiParser,
    /// What the program reported with OSC 7501 and the agent's hooks
    /// with `terminal.report-status`.
    status: Status,
    /// The working directory and title last announced.
    seen: terminal::Seen,
    /// The foreground process group last announced, at first the program's;
    /// `None` where the platform tells none.
    foreground: Option<process::Foreground>,
    /// Who echoes typing, as last announced.
    echo: Echo,
    /// Keys awaiting their echo ack.
    waiting: Waiting,
    /// How the program was started: for `terminal.shell` and the store.
    started: Started,
    /// The grid changed since the store last saved it.
    unsaved: bool,
    /// Where lines evicted from the grid's scrollback go, while the
    /// server keeps state (C70).
    spool: Option<Spool>,
    /// The last [`Message::InputBatch`] number sent to the owner (C77).
    input_seq: u64,
    /// Launched with `hold` (C78): the block stays, read-only, once its
    /// program exits.
    hold: bool,
    /// Its PTY was released (C76): read-only, with its program, if still
    /// running, left to its owner.
    released: bool,
}

/// A block's running program: its PTY and the task reading it.
struct Live {
    pty: Rc<Pty>,
    process: Process,
    /// Tells the reader a key was written, so it drains when its grace ends.
    wake: Rc<Notify>,
    reader: JoinHandle<()>,
}

/// What [`State::start`] starts a block's program with.
struct Start<'a> {
    block: BlockId,
    session: rusty_mux_core::SessionId,
    window: WindowId,
    size: Size,
    config: &'a Config,
    started: Started,
    /// Launched with an owner (`Launch.own`): on Windows its program gets
    /// no Job Object, so the mux never ends what it started (pack D182).
    owned: bool,
}

/// A started program whose block has no screen or reader yet.
struct Unscreened {
    block: BlockId,
    pty: Rc<Pty>,
    process: Process,
    foreground: Option<process::Foreground>,
    echo: Echo,
    started: Started,
}

impl Unscreened {
    /// The block, reading its PTY into `grid` from now on.
    fn screened(self, shared: &Shared, grid: Grid, parser: AnsiParser) -> BlockRt {
        let wake = Rc::new(Notify::new());
        let reader = rusty_tokio::task::spawn_local(pump(
            shared.clone(),
            self.block,
            self.pty.clone(),
            wake.clone(),
        ));
        let live = Live {
            pty: self.pty,
            process: self.process,
            wake,
            reader,
        };
        BlockRt {
            foreground: self.foreground,
            echo: self.echo,
            ..BlockRt::read_only(grid, parser, self.started)
        }
        .with(live)
    }
}

impl BlockRt {
    /// A block with no program, showing `grid`.
    fn read_only(mut grid: Grid, parser: AnsiParser, started: Started) -> Self {
        grid.collect_program_status = true;
        Self {
            live: None,
            grid,
            parser,
            status: Status::default(),
            seen: terminal::Seen::default(),
            foreground: None,
            echo: Echo::Unknown,
            waiting: Waiting::default(),
            started,
            unsaved: true,
            spool: None,
            input_seq: 0,
            hold: false,
            released: false,
        }
    }

    fn with(self, live: Live) -> Self {
        Self {
            live: Some(live),
            ..self
        }
    }

    /// The program, or a refusal saying the block is read-only.
    fn live(&self) -> Result<&Live, Refusal> {
        self.live.as_ref().ok_or_else(|| {
            if self.released {
                return "the block is read-only: its pty was released".into();
            }
            if self.hold {
                return "the block is read-only: its program has exited".into();
            }
            "the block is read-only: it was restored after a server restart and its command was not run again"
                .into()
        })
    }

    /// Keeps the history this grid evicts in `spool`, starting with the
    /// lines already waiting (a restore's marker pushes some out). Without a
    /// spool, or once writing fails, the grid stops collecting them: spilling
    /// is on exactly while `self.spool` is some.
    fn keep_history(&mut self, block: BlockId, spool: io::Result<Spool>) {
        let pending = self.grid.take_spilled();
        let kept = spool.and_then(|mut spool| spool.append(pending).map(|()| spool));
        match kept {
            Ok(spool) => {
                self.grid.spill_evicted(true);
                self.spool = Some(spool);
            }
            Err(e) => {
                eprintln!(
                    "rusty-mux-server: block {block}'s history past scrollback will not be kept: {e}"
                );
                self.stop_history();
            }
        }
    }

    /// Appends the lines the grid evicted to the spool. A spool that
    /// fails is dropped with a warning, and its history stops there.
    fn spill(&mut self, block: BlockId) {
        let Some(spool) = self.spool.as_mut() else {
            return self.stop_history();
        };
        let lines = self.grid.take_spilled();
        if lines.is_empty() {
            return;
        }
        if let Err(e) = spool.append(lines) {
            eprintln!("rusty-mux-server: block {block}'s history past scrollback stops here: {e}");
            self.stop_history();
        }
    }

    /// Drops the spool and stops the grid collecting lines for it.
    fn stop_history(&mut self) {
        self.spool = None;
        self.grid.spill_evicted(false);
    }

    fn pid(&self) -> Option<u32> {
        self.live.as_ref().and_then(|l| l.process.pid())
    }

    /// Stops reading (unless the reader is the caller), releases the PTY's
    /// output whatever other handles remain and closes this handle on the
    /// master. The program, for the caller to reap; `None` for a
    /// read-only block.
    fn release(self, abort_reader: bool) -> Option<Process> {
        let live = self.live?;
        if abort_reader {
            live.reader.abort();
        }
        live.pty.release();
        drop(live.pty);
        Some(live.process)
    }
}

pub(crate) struct State {
    /// What the server was started with, under the config file's section.
    base: Config,
    /// `base` with the config file's `server` section over it.
    config: Config,
    /// The listening socket's path, given to each block's program.
    socket: Option<PathBuf>,
    mux: Mux,
    blocks: HashMap<BlockId, BlockRt>,
    clients: HashMap<ClientId, Outbox>,
    features: HashMap<ClientId, Features>,
    /// Each `TAKEOVER` client's tokens.
    tokens: HashMap<ClientId, Tokens>,
    /// Clients whose connection ended within [`ORPHAN_GRACE`], each with
    /// that connection's cancel, so only its own expiry drops it.
    orphans: HashMap<ClientId, Rc<Notify>>,
    history: History,
    /// With `--udp`: the associations, offered with `DATAGRAM`.
    datagrams: Option<Registry>,
    /// How long silence and staleness keep a UDP client and its lease.
    lifetimes: Lifetimes,
    /// Orphans whose datagram stream overflowed its budget, each with the
    /// block whose lease was released after [`Lifetimes::lease`], to give
    /// back on takeover.
    stale: HashMap<ClientId, Option<BlockId>>,
    /// Leases silence released, each with its block and the connection
    /// that served the client then, until heard again or taken over.
    silenced: HashMap<ClientId, (BlockId, Rc<Notify>)>,
    /// The consoles a stopping server waits for.
    hosts: pty::Hosts,
    /// Set by [`State::stop`]: no block starts any more.
    stopping: bool,
    /// What `reload-config` reads the file with.
    pub(crate) loader: crate::settings::Loader,
    /// A reload is reading the file; one at a time, so a stalled
    /// filesystem holds one thread at most.
    reloading: bool,
    /// Where sessions are saved across restarts, with `Config::state_dir`.
    store: Option<Store>,
    /// The event number the store last saved the model at.
    saved_at: Option<u64>,
    /// Numbers each save, newest highest.
    generation: u64,
    /// The server's own account, set by [`State::serve_as`]: unrestricted
    /// on a server other principals share (D164).
    account: Option<Principal>,
    /// Set by [`State::stop_on_request`]: the stop the server's own
    /// account may start with `shutdown`. Taken by the first request.
    shutdown: Option<oneshot::Sender<()>>,
}

/// How long a UDP client outlives silence or an overflow: its lease for
/// `lease`, the client itself for `idle` (`--udp-idle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Lifetimes {
    pub(crate) idle: Duration,
    pub(crate) lease: Duration,
}

/// What became of a client when its connection ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Kept {
    /// Dropped, or another connection's now.
    Gone,
    /// Kept for [`ORPHAN_GRACE`], lease and all.
    Orphan,
    /// Its datagram stream overflowed: kept for these lifetimes.
    Stale(Lifetimes),
}

impl State {
    pub(crate) fn shared(config: Config, socket: Option<PathBuf>) -> Shared {
        Rc::new(RefCell::new(Self {
            base: config.clone(),
            config,
            socket,
            mux: Mux::new(),
            blocks: HashMap::new(),
            clients: HashMap::new(),
            features: HashMap::new(),
            tokens: HashMap::new(),
            orphans: HashMap::new(),
            history: History::new(Uuid::new_v4(), HISTORY),
            datagrams: None,
            lifetimes: Lifetimes {
                idle: ORPHAN_GRACE,
                lease: ORPHAN_GRACE,
            },
            stale: HashMap::new(),
            silenced: HashMap::new(),
            hosts: Default::default(),
            stopping: false,
            loader: crate::settings::reloader(),
            reloading: false,
            store: None,
            saved_at: None,
            generation: 0,
            account: None,
            shutdown: None,
        }))
    }

    /// Reads the config file at start, before serving: a bad one stops
    /// the server.
    pub(crate) fn load_config(&mut self) -> Result<(), String> {
        self.config = crate::settings::read(&self.base, crate::settings::When::Start)?;
        Ok(())
    }

    /// `reload-config`: reads the file off this thread and answers once
    /// read ([`Self::reloaded`]), in a task of its own, so the read lands
    /// even if the connection goes. The connection waits for it before its
    /// next request. A server started with no file refuses, as does a
    /// second reload while one is reading.
    fn reload_config(
        &mut self,
        shared: &Shared,
        client: ClientId,
        req: u64,
    ) -> Result<Outcome, Refusal> {
        if self.base.config_file.is_none() {
            return Err("the server was started with no config file".into());
        }
        if self.reloading {
            return Err("a reload is still reading the config file".into());
        }
        let cancel = self
            .clients
            .get(&client)
            .map(|out| out.cancel().clone())
            .ok_or("no such client")?;
        let read = crate::settings::load_apart(self.loader.clone(), self.base.clone())
            .map_err(|e| format!("reading the config file: {e}"))?;
        self.reloading = true;
        let (done, finished) = oneshot::channel();
        let weak = Rc::downgrade(shared);
        rusty_tokio::task::spawn_local(async move {
            let result = read
                .await
                .unwrap_or_else(|_| Err("reading the config file failed".into()));
            if let Some(shared) = weak.upgrade() {
                shared.borrow_mut().reloaded(client, &cancel, req, result);
            }
            // The connection may have gone; nothing waits then.
            let _ = done.send(());
        });
        Ok(Outcome::Later(finished))
    }

    /// A reload read `result`: a good config replaces the last and is
    /// announced, a bad one changes nothing. The answer goes to the
    /// connection that asked, unless another took its client over, where
    /// `req` means nothing.
    fn reloaded(
        &mut self,
        client: ClientId,
        cancel: &Rc<Notify>,
        req: u64,
        result: Result<Config, String>,
    ) {
        self.reloading = false;
        let answer = match result {
            Ok(config) => {
                self.config = config;
                self.mux.note_config_reloaded();
                Message::Done { req, created: None }
            }
            Err(reason) => Message::Failed {
                req,
                reason,
                code: None,
            },
        };
        if self.served_by(client, cancel) {
            self.send(client, &answer);
        }
        self.flush();
    }

    /// Whether nothing would notice the server go: no block, no client and
    /// no orphaned client a takeover could still claim (C69 idle exit).
    pub(crate) fn is_idle(&self) -> bool {
        self.blocks.is_empty() && self.clients.is_empty() && self.orphans.is_empty()
    }

    /// Serves as `account`, the server's own: it may disconnect any client
    /// and owns every session no client of another principal created.
    pub(crate) fn serve_as(&mut self, account: Principal) {
        self.account = Some(account);
    }

    /// Lets clients of the server's own account ([`State::serve_as`]) stop
    /// it with `shutdown`; the receiver resolves on the first such request.
    pub(crate) fn stop_on_request(&mut self) -> oneshot::Receiver<()> {
        let (stop, requested) = oneshot::channel();
        self.shutdown = Some(stop);
        requested
    }

    /// The server is stopping: starts no more blocks, and ends every
    /// block's program now, as dropping the block would, except that an
    /// owned block's program is only hung up (D87). The consoles to
    /// wait for, which still hold the ones closing when this was called.
    pub(crate) fn stop(&mut self) -> pty::Hosts {
        self.save_now();
        self.stopping = true;
        for (block, rt) in self.blocks.drain() {
            let owned = self.mux.block(block).is_some_and(|b| b.owner().is_some());
            match rt.release(true) {
                Some(process) if owned => process.abandon(),
                other => drop(other),
            }
        }
        std::mem::take(&mut self.hosts)
    }

    /// Offers `DATAGRAM` from now on, handing out associations of
    /// `registry`.
    pub(crate) fn offer_datagrams(&mut self, registry: Registry, lifetimes: Lifetimes) {
        self.datagrams = Some(registry);
        self.lifetimes = lifetimes;
    }

    /// The connection owning `cancel`, serving `client`, is a datagram
    /// stream.
    pub(crate) fn over_datagram(&mut self, client: ClientId, cancel: &Rc<Notify>) {
        if let Some(out) = self
            .clients
            .get_mut(&client)
            .filter(|o| o.belongs_to(cancel))
        {
            out.over_datagram();
        }
    }

    /// What this server offers: [`OFFERED`], and `DATAGRAM` with `--udp`.
    fn offered(&self) -> Features {
        match self.datagrams {
            Some(_) => OFFERED.union(Features::DATAGRAM),
            None => OFFERED,
        }
    }

    #[cfg(all(test, unix))]
    pub(crate) fn test_cwd(&self) -> &std::ffi::OsStr {
        &self.config.cwd
    }

    #[cfg(test)]
    pub(crate) fn lease_of(&self, block: BlockId) -> Option<ClientId> {
        self.mux.block(block)?.lease()
    }

    #[cfg(all(test, unix))]
    pub(crate) fn test_lifetimes(&mut self, lifetimes: Lifetimes) {
        self.lifetimes = lifetimes;
    }

    /// Makes `client` stale, as its datagram stream overflowing its budget
    /// would, without the output it takes to get there.
    #[cfg(all(test, unix))]
    pub(crate) fn test_go_stale(&mut self, client: ClientId) {
        self.go_stale(client);
    }

    #[cfg(all(test, unix))]
    pub(crate) fn is_stale(&self, client: ClientId) -> bool {
        self.stale.contains_key(&client)
    }

    #[cfg(test)]
    pub(crate) fn has_client(&self, client: ClientId) -> bool {
        self.clients.contains_key(&client)
    }

    /// Whether `client` is still served by the connection owning `cancel`,
    /// and not dropped or taken over.
    /// Whether the connection owning `cancel` still serves `client`, or
    /// left it a stale orphan: silence on its path still counts for it.
    pub(crate) fn kept_by(&self, client: ClientId, cancel: &Rc<Notify>) -> bool {
        let stale = self.stale.contains_key(&client) && self.orphaned_by(client, cancel);
        stale || self.served_by(client, cancel)
    }

    pub(crate) fn served_by(&self, client: ClientId, cancel: &Rc<Notify>) -> bool {
        self.clients
            .get(&client)
            .is_some_and(|out| out.belongs_to(cancel))
    }

    /// Registers a client of `principal`, whom the server verified, and
    /// negotiates features and, with `EVENT_SEQ`, replays the events after
    /// `resume` when they are all still held. With `TAKEOVER` and a valid
    /// `takeover` by the same principal, the connection continues that
    /// client instead: its id, name, attachment and lease stay, and
    /// replacing its outbox closes the old connection. Either way the
    /// client is now reached by `transport`.
    pub(crate) fn connect(
        &mut self,
        outbox: Outbox,
        principal: Principal,
        transport: Transport,
        theirs: Features,
        resume: Option<Cursor>,
        takeover: Option<Takeover>,
    ) -> ClientId {
        let features = theirs.intersection(self.offered());
        let continued = takeover.and_then(|t| self.takes_over(t, &principal, features));
        let client = match continued {
            Some(client) => {
                let moved = self.mux.reconnect(client, transport);
                in_step("reconnect", moved);
                client
            }
            None => self.mux.connect(principal, transport),
        };
        // The new connection numbers its requests afresh.
        self.forget_keys(client);
        // Waiting out an orphan's grace ends at its takeover.
        if let Some(ended) = self.orphans.remove(&client) {
            ended.notify_one();
        }
        self.silenced.remove(&client);
        let released = self.stale.remove(&client).flatten();
        self.clients.insert(client, outbox);
        self.features.insert(client, features);
        let token = features.contains(Features::TAKEOVER).then(Uuid::new_v4);
        // Presenting a valid token proves the client holds it, so it stays
        // good until the new one is proven; the token before it retires.
        let confirmed = continued.and(takeover).map(|t| t.token);
        match token {
            Some(current) => self.tokens.insert(client, Tokens { current, confirmed }),
            None => self.tokens.remove(&client),
        };
        let (cursor, missed) = if features.contains(Features::EVENT_SEQ) {
            let (cursor, missed) = self.missed(resume, features);
            (Some(cursor), missed)
        } else {
            (None, Vec::new())
        };
        self.send(
            client,
            &Message::Welcome {
                features,
                client,
                cursor,
                token,
                version: Some(rusty_mux_proto::VERSION.into()),
            },
        );
        missed.iter().for_each(|m| self.send(client, m));
        // A stale client's lease, released while it was away, comes back
        // on the connection that took it over.
        if let Some(block) = released {
            self.regain_lease(client, block);
        }
        // A taken-over client may still be attached; its new connection has
        // no screen yet, so it gets one before any live output.
        // It also learns who echoes there, as on an attach.
        if let Some(block) = continued.and_then(|c| self.mux.client(c)?.attached()) {
            self.send_snapshot(client, block);
            if !self.check_echo(block) {
                self.send_echo(client, block);
            }
        }
        self.flush();
        client
    }

    /// Where a client's stream starts, and the events it missed: from
    /// `resume` when possible, else from now with nothing to replay. Only
    /// events its `features` let it receive are replayed.
    fn missed(&self, resume: Option<Cursor>, features: Features) -> (Cursor, Vec<Message>) {
        let replay = resume.and_then(|from| Some((from, self.history.since(from)?)));
        let Some((from, events)) = replay else {
            return (self.history.cursor(), Vec::new());
        };
        let messages = events
            .filter(|(_, event)| receives(features, event))
            .map(|(seq, event)| Message::Event {
                seq: Some(*seq),
                event: event.clone(),
            })
            .collect();
        (from, messages)
    }

    /// The client `takeover` names, if it is connected or kept for a
    /// reconnect, the token is its current or confirmed one, the new
    /// connection is `principal`'s, as the client is, and it negotiated
    /// exactly the client's features.
    /// Only `TAKEOVER` clients hold a token. Equal features keep what the
    /// client is already attached to and subscribed for (a checkpoint
    /// version, event numbering) valid on the new connection. A token
    /// leaked to another principal continues nothing: that connection
    /// becomes a client of its own, and the token stays good.
    fn takes_over(
        &self,
        takeover: Takeover,
        principal: &Principal,
        features: Features,
    ) -> Option<ClientId> {
        let tokens = self.tokens.get(&takeover.client)?;
        let matches = |token: &Uuid| {
            rusty_crypto_key::constant_time_eq(token.as_bytes(), takeover.token.as_bytes())
        };
        let valid = matches(&tokens.current) | tokens.confirmed.as_ref().is_some_and(matches);
        let same = self.features.get(&takeover.client) == Some(&features);
        let known = self.clients.contains_key(&takeover.client)
            || self.orphans.contains_key(&takeover.client);
        let theirs = self
            .mux
            .client(takeover.client)
            .is_some_and(|c| c.principal().id() == principal.id());
        (valid && same && known && theirs).then_some(takeover.client)
    }

    /// Forgets a client whose connection, the one owning `cancel`, ended. A
    /// client the server already dropped for exceeding its budget is gone
    /// from every map; one another connection took over is not this
    /// connection's to forget. A client that can be taken over is kept
    /// instead, for [`ORPHAN_GRACE`], with its attachment and lease;
    /// whether it was, so the caller calls [`Self::expire`] after the
    /// grace. A client that means to leave detaches first.
    pub(crate) fn disconnect(&mut self, client: ClientId, cancel: &Rc<Notify>) -> Kept {
        if self.stale.contains_key(&client) && self.orphaned_by(client, cancel) {
            return Kept::Stale(self.lifetimes);
        }
        if !self.served_by(client, cancel) {
            return Kept::Gone;
        }
        if self.tokens.contains_key(&client) {
            self.clients.remove(&client);
            self.orphans.insert(client, cancel.clone());
            return Kept::Orphan;
        }
        self.drop_client(client);
        self.flush();
        Kept::Gone
    }

    /// Whether `client` is an orphan of the connection owning `cancel`.
    pub(crate) fn orphaned_by(&self, client: ClientId, cancel: &Rc<Notify>) -> bool {
        self.orphans
            .get(&client)
            .is_some_and(|ended| Rc::ptr_eq(ended, cancel))
    }

    /// A datagram client over its budget: ends its connection but keeps the
    /// client as a stale orphan, as lifetime B keeps a silent one. Its
    /// takeover sends the screen again, so nothing discarded is missed.
    fn go_stale(&mut self, client: ClientId) {
        let Some(out) = self.clients.remove(&client) else {
            return;
        };
        eprintln!("rusty-mux-server: client {client}: UDP output over budget, kept for a takeover");
        let cancel = out.cancel().clone();
        // A lease silence released on this connection comes back on the
        // takeover, as a stale one does.
        let released = self.silenced.get(&client);
        let held = released
            .filter(|(_, by)| Rc::ptr_eq(by, &cancel))
            .map(|(b, _)| *b);
        self.orphans.insert(client, cancel);
        self.stale.insert(client, held);
    }

    /// A stale orphan's lease time is up: releases its lease, kept for its
    /// takeover.
    pub(crate) fn release_stale(&mut self, client: ClientId, cancel: &Rc<Notify>) {
        if !self.orphaned_by(client, cancel) {
            return;
        }
        let Some(held) = self.stale.get_mut(&client) else {
            return;
        };
        let block = self.mux.client(client).and_then(|c| c.attached());
        let Some(block) =
            block.filter(|&b| self.mux.block(b).and_then(|b| b.lease()) == Some(client))
        else {
            return;
        };
        *held = Some(block);
        if let Err(err) = self.mux.release_lease(client, block) {
            eprintln!("rusty-mux-server: model out of step on a stale lease: {err}");
        }
        self.flush();
    }

    /// Releases the lease each of `clients` holds on its attached block:
    /// the path they are on went silent. Returns what was released, for
    /// [`Self::regain`].
    pub(crate) fn release_silent(&mut self, clients: &[(ClientId, Rc<Notify>)]) -> Vec<Released> {
        let held = |(client, cancel): &(ClientId, Rc<Notify>)| {
            let block = self.mux.client(*client)?.attached()?;
            let leased = self.mux.block(block)?.lease() == Some(*client);
            leased.then(|| (*client, block, cancel.clone()))
        };
        let released: Vec<_> = clients.iter().filter_map(held).collect();
        for (client, block, cancel) in &released {
            if let Err(err) = self.mux.release_lease(*client, *block) {
                eprintln!("rusty-mux-server: model out of step on a silent lease: {err}");
            }
            self.silenced.insert(*client, (*block, cancel.clone()));
            // Stale already: its takeover gets the lease back.
            if let Some(held) = self.stale.get_mut(client) {
                *held = Some(*block);
            }
        }
        self.flush();
        released
    }

    /// Gives each client the lease silence released, where the same
    /// connection still serves it, it is still attached there and the
    /// lease is still free. A client that went stale on that connection
    /// since got the lease recorded for its takeover; one taken over
    /// meanwhile is its new connection's to lease.
    pub(crate) fn regain(&mut self, released: &[Released]) {
        for (client, block, cancel) in released {
            let ours = self
                .silenced
                .get(client)
                .is_some_and(|(_, by)| Rc::ptr_eq(by, cancel));
            if ours {
                self.silenced.remove(client);
            }
            if self.served_by(*client, cancel) {
                self.regain_lease(*client, *block);
            }
        }
        self.flush();
    }

    /// Tells clients with `IDENTITY` that each of `clients` the same
    /// connection still serves is reached at a new address: its UDP
    /// association roamed. A client taken over elsewhere is not this
    /// association's any more.
    pub(crate) fn roamed(&mut self, clients: &[(ClientId, Rc<Notify>)]) {
        for (client, cancel) in clients {
            if self.served_by(*client, cancel) {
                let moved = self.mux.reconnect(*client, Transport::Datagram);
                in_step("roam", moved);
            }
        }
        self.flush();
    }

    /// Gives `client` the lease on `block` if it is still attached there
    /// and the lease is still free.
    fn regain_lease(&mut self, client: ClientId, block: BlockId) {
        let free = self.mux.block(block).is_some_and(|b| b.lease().is_none());
        let attached = self.mux.client(client).and_then(|c| c.attached()) == Some(block);
        if free && attached && self.mux.acquire_lease(client, block).is_err() {
            eprintln!("rusty-mux-server: model out of step regaining a lease");
        }
    }

    /// The grace of the connection owning `cancel` ended: drops `client`
    /// unless another connection took it over meanwhile.
    pub(crate) fn expire(&mut self, client: ClientId, cancel: &Rc<Notify>) {
        if !self.orphaned_by(client, cancel) {
            return;
        }
        self.drop_client(client);
        self.flush();
    }

    fn drop_client(&mut self, client: ClientId) {
        let orphan = self.orphans.remove(&client).is_some();
        self.stale.remove(&client);
        self.silenced.remove(&client);
        if self.clients.remove(&client).is_none() && !orphan {
            return;
        }
        self.features.remove(&client);
        self.tokens.remove(&client);
        self.forget_keys(client);
        if let Err(err) = self.mux.disconnect(client) {
            eprintln!("rusty-mux-server: model out of step on disconnect: {err}");
        }
    }

    /// Applies one request. Returns input for the caller to write to a PTY
    /// after releasing the borrow.
    pub(crate) fn request(shared: &Shared, client: ClientId, message: Message) -> Option<Pending> {
        let mut this = shared.borrow_mut();
        // A request proves the client got this connection's Welcome, and
        // with it the current token.
        if let Some(tokens) = this.tokens.get_mut(&client) {
            tokens.confirmed = Some(tokens.current);
        }
        // Grants end on time before anything relies on them (C77).
        this.mux.expire_grants(now_ms());
        let (req, outcome) = this.apply(shared, client, message);
        let write = match outcome {
            Ok(Outcome::Done(created)) => {
                this.send(client, &Message::Done { req, created });
                None
            }
            Ok(Outcome::Attached(block)) => {
                this.send(client, &Message::Done { req, created: None });
                this.send_snapshot(client, block);
                if !this.check_echo(block) {
                    this.send_echo(client, block);
                }
                None
            }
            Ok(Outcome::Reply(frames)) => {
                frames.into_iter().for_each(|f| this.send_frame(client, f));
                None
            }
            Ok(Outcome::Write(write)) => Some(Pending::Write(write)),
            Ok(Outcome::Later(done)) => Some(Pending::Reload(done)),
            Err(Refusal { reason, code }) => {
                this.send(client, &Message::Failed { req, reason, code });
                None
            }
        };
        this.flush();
        write
    }

    fn apply(
        &mut self,
        shared: &Shared,
        client: ClientId,
        message: Message,
    ) -> (u64, Result<Outcome, Refusal>) {
        use Message as M;
        let done = |r: Result<(), rusty_mux_core::Error>| {
            r.map(|()| Outcome::Done(None)).map_err(Refusal::from)
        };
        match message {
            M::CreateSession { req, name } => {
                let result = match name.is_empty() && self.has(client, Features::SESSION_NAMES) {
                    true => self.mux.create_generated_session(name_seed()),
                    false => self.mux.create_session(name),
                }
                .and_then(|session| self.record_creator(session, client).map(|()| session));
                (
                    req,
                    result
                        .map(|s| Outcome::Done(Some(s.uuid())))
                        .map_err(Refusal::from),
                )
            }
            M::CreateWindow { req, session, name } => {
                let result = self.mux.create_window(session, name);
                (
                    req,
                    result
                        .map(|w| Outcome::Done(Some(w.uuid())))
                        .map_err(Refusal::from),
                )
            }
            M::CreateBlock {
                req,
                window,
                size,
                launch,
            } => {
                // As the catalog's contract: a launch only on a connection
                // that negotiated it.
                if let Err(refusal) = self.check_launch(client, &launch) {
                    return (req, Err(refusal));
                }
                (
                    req,
                    self.create_block(shared, client, window, size, &launch, None),
                )
            }
            M::Split {
                req,
                block,
                side,
                launch,
            } => (
                req,
                self.require(client, Features::LAYOUT, "split")
                    .and_then(|()| self.check_launch(client, &launch))
                    .and_then(|()| self.owns(client, [block]))
                    .and_then(|()| self.split(shared, client, block, side, &launch)),
            ),
            M::Zoom { req, block } => (
                req,
                self.require(client, Features::LAYOUT, "zoom")
                    .and_then(|()| done(self.mux.zoom(block))),
            ),
            M::Unzoom { req, window } => (
                req,
                self.require(client, Features::LAYOUT, "unzoom")
                    .and_then(|()| done(self.mux.unzoom(window))),
            ),
            M::SelectWindow { req, window } => (
                req,
                self.require(client, Features::ACTIVE_WINDOW, "select-window")
                    .and_then(|()| done(self.mux.select_window(window))),
            ),
            M::ResizeSplit {
                req,
                block,
                axis,
                share,
            } => (
                req,
                self.owns(client, [block]).and_then(|()| {
                    self.edit(client, "resize-split", |mux| {
                        mux.resize_split(block, axis, share)
                    })
                }),
            ),
            M::SwapBlocks { req, block, other } => (
                req,
                self.owns(client, [block, other]).and_then(|()| {
                    self.edit(client, "swap-blocks", |mux| mux.swap_blocks(block, other))
                }),
            ),
            M::MoveBlock {
                req,
                block,
                beside,
                side,
            } => (
                req,
                self.owns(client, [Some(block), beside].into_iter().flatten())
                    .and_then(|()| {
                        self.edit(client, "move-block", |mux| {
                            mux.move_block(block, beside, side)
                        })
                    }),
            ),
            M::DetachBlock { req, block } => (
                req,
                self.owns(client, [block]).and_then(|()| {
                    self.edit(client, "detach-block", |mux| mux.detach_block(block))
                }),
            ),
            M::MoveWindow { req, window, index } => (
                req,
                self.edit(client, "move-window", |mux| mux.move_window(window, index)),
            ),
            M::FocusBlock { req, block } => (
                req,
                self.require(client, Features::FOCUS, "focus-block")
                    .and_then(|()| done(self.mux.focus_block(block))),
            ),
            M::MoveFocus {
                req,
                window,
                toward,
            } => (
                req,
                self.require(client, Features::FOCUS, "move-focus")
                    .and_then(|()| done(self.mux.move_focus(window, toward))),
            ),
            M::ReloadConfig { req } => (
                req,
                self.require(client, Features::CONFIG_RELOAD, "reload-config")
                    .and_then(|()| self.reload_config(shared, client, req)),
            ),
            M::Attach { req, block } => {
                if let Err(refusal) = self.require(client, Features::CHECKPOINT_V6, "attach") {
                    return (req, Err(refusal));
                }
                if self.clients.get(&client).is_some_and(|out| out.attaching()) {
                    return (req, Err("checkpoint transfer already in progress".into()));
                }
                // Its output ended and its program is being reaped: nothing
                // is left to send a checkpoint from.
                if self.mux.block(block).is_some() && !self.blocks.contains_key(&block) {
                    return (req, Err(format!("block {block} is closing").into()));
                }
                let result = self.mux.attach(client, block);
                (
                    req,
                    result
                        .map(|()| Outcome::Attached(block))
                        .map_err(Refusal::from),
                )
            }
            M::Detach { req } => {
                if let Some(out) = self.clients.get(&client) {
                    out.end_history();
                }
                (req, done(self.mux.detach(client)))
            }
            M::AcquireLease { req, block } => (req, done(self.mux.acquire_lease(client, block))),
            M::ReleaseLease { req, block } => (req, done(self.mux.release_lease(client, block))),
            M::Input { req, block, data } => (req, self.input(client, req, block, data)),
            M::Resize { req, block, size } => (req, self.resize(client, block, size)),
            M::CloseBlock { req, block } => (
                req,
                done(
                    self.mux
                        .authorize_close(client, block)
                        .and_then(|()| self.mux.close_block(block)),
                ),
            ),
            M::Sync { req } => (req, Ok(Outcome::Done(None))),
            M::BlockCall {
                req,
                block,
                method,
                args,
            } => (
                req,
                self.require(client, Features::BLOCK_CALL, "a block call")
                    .and_then(|()| self.block_call(client, req, block, &method, args)),
            ),
            M::List { req } => (
                req,
                self.require(client, Features::INSPECT, "list")
                    .and_then(|()| self.list(client, req)),
            ),
            M::Whoami { req } => (
                req,
                self.require(client, Features::IDENTITY, "whoami")
                    .and_then(|()| self.whoami(client, req)),
            ),
            M::Rename { req, target, name } => (
                req,
                self.require(client, Features::LABELS, "rename")
                    .and_then(|()| match target {
                        EntityId::Block(block) => self.owns(client, [block]),
                        _ => Ok(()),
                    })
                    .and_then(|()| done(self.mux.rename(target, name))),
            ),
            M::NameSession { req, session, name } => (
                req,
                self.require(client, Features::NAME_SESSION, "name-session")
                    .and_then(|()| done(self.mux.name_session(session, name))),
            ),
            M::Disconnect {
                req,
                client: target,
            } => (
                req,
                self.require(client, Features::LABELS, "disconnect")
                    .and_then(|()| self.disconnect_other(client, target)),
            ),
            M::GrantLease {
                req,
                block,
                client: grantee,
                until,
            } => (
                req,
                self.require(client, Features::INPUT_GATE, "grant-lease")
                    .and_then(|()| done(self.mux.grant(client, block, grantee, until, now_ms()))),
            ),
            M::RevokeLease {
                req,
                block,
                client: grantee,
            } => (
                req,
                self.require(client, Features::INPUT_GATE, "revoke-lease")
                    .and_then(|()| done(self.mux.revoke(client, block, grantee))),
            ),
            M::RequestLease { req, block } => (
                req,
                self.require(client, Features::LEASE_REQUESTS, "request-lease")
                    .and_then(|()| self.request_lease(client, block)),
            ),
            M::Shutdown { req } => (
                req,
                self.require(client, Features::SHUTDOWN, "shutdown")
                    .and_then(|()| self.shutdown(client)),
            ),
            #[cfg(target_os = "linux")]
            M::SignalBlock { req, block, signal } => (
                req,
                self.require(client, Features::SIGNAL_RELEASE, "signal-block")
                    .and_then(|()| self.signal_block(client, block, signal)),
            ),
            #[cfg(target_os = "linux")]
            M::ReleaseBlock { req, block } => (
                req,
                self.require(client, Features::SIGNAL_RELEASE, "release-block")
                    .and_then(|()| self.release_block(shared, client, block)),
            ),
            #[cfg(not(target_os = "linux"))]
            M::SignalBlock { req, .. } | M::ReleaseBlock { req, .. } => (
                req,
                Err("signal and release are supported on Linux only".into()),
            ),
            M::OpenDatagram { req } => (
                req,
                self.require(client, Features::DATAGRAM, "a datagram association")
                    .and_then(|()| self.open_datagram(client, req)),
            ),
            other => (0, Err(format!("not a client request: {other:?}").into())),
        }
    }

    /// A new association for the client to move its connections onto,
    /// whose streams are the client's principal's. The key travels only in
    /// this reply, inside the client's connection.
    fn open_datagram(&self, client: ClientId, req: u64) -> Result<Outcome, Refusal> {
        let registry = self.datagrams.as_ref().ok_or("no UDP listener")?;
        let principal = self
            .mux
            .client(client)
            .ok_or("unknown client")?
            .principal()
            .clone();
        let (port, id, key) = registry.offer(Instant::now(), principal)?;
        reply(
            "open datagram",
            &[Message::Datagram {
                req,
                port,
                id,
                key: DatagramKey(key),
            }],
        )
    }

    /// Fails unless `client` negotiated `feature`, which `what` needs.
    /// A layout edit, for a client that negotiated them.
    fn edit(
        &mut self,
        client: ClientId,
        what: &str,
        edit: impl FnOnce(&mut Mux) -> Result<(), rusty_mux_core::Error>,
    ) -> Result<Outcome, Refusal> {
        self.require(client, Features::LAYOUT_EDIT, what)
            .and_then(|()| {
                edit(&mut self.mux)
                    .map(|()| Outcome::Done(None))
                    .map_err(Refusal::from)
            })
    }

    /// Sends `signal` to `block`'s program group, for its owner (C76).
    #[cfg(target_os = "linux")]
    fn signal_block(
        &self,
        client: ClientId,
        block: BlockId,
        signal: rusty_mux_proto::BlockSignal,
    ) -> Result<Outcome, Refusal> {
        self.owns(client, [block])?;
        let live = self.running(block)?;
        live.process
            .signal(signal)
            .map_err(|e| format!("signalling the block's program: {e}"))?;
        Ok(Outcome::Done(None))
    }

    /// Closes `block`'s PTY and waits for its program, never killing it,
    /// for its owner (C76). The block is read-only from now on; once the
    /// program exits, [`Self::exited`] keeps a held block and closes any
    /// other, as for a program that exits by itself.
    #[cfg(target_os = "linux")]
    fn release_block(
        &mut self,
        shared: &Shared,
        client: ClientId,
        block: BlockId,
    ) -> Result<Outcome, Refusal> {
        self.owns(client, [block])?;
        self.running(block)?;
        let rt = self
            .blocks
            .get_mut(&block)
            .ok_or(rusty_mux_core::Error::UnknownBlock(block))?;
        let live = rt.live.take().ok_or(EXITED)?;
        // Saved and restored read-only from now on (D72).
        rt.started.read_only = true;
        rt.unsaved = true;
        rt.released = true;
        live.reader.abort();
        live.pty.release();
        drop(live.pty);
        let shared = shared.clone();
        rusty_tokio::task::spawn_local(async move {
            let status = live.process.wait().await;
            shared.borrow_mut().exited(block, status);
        });
        Ok(Outcome::Done(None))
    }

    /// `block`'s running program, or why there is none.
    #[cfg(target_os = "linux")]
    fn running(&self, block: BlockId) -> Result<&Live, Refusal> {
        let rt = self
            .blocks
            .get(&block)
            .ok_or(rusty_mux_core::Error::UnknownBlock(block))?;
        rt.live.as_ref().ok_or_else(|| EXITED.into())
    }

    /// Refuses unless `client` may change each of `blocks`: nobody else
    /// owns it and it is not orphaned (C46). An unknown block is left for
    /// the request itself to report.
    fn owns(
        &self,
        client: ClientId,
        blocks: impl IntoIterator<Item = BlockId>,
    ) -> Result<(), Refusal> {
        blocks
            .into_iter()
            .try_for_each(|block| match self.mux.authorize_owner(client, block) {
                Err(rusty_mux_core::Error::UnknownBlock(_)) => Ok(()),
                result => result,
            })
            .map_err(Refusal::from)
    }

    /// Refuses a launch `client` did not negotiate: any at all without
    /// `LAUNCH`, an owned one without `OWNERSHIP`.
    fn check_launch(&self, client: ClientId, launch: &Launch) -> Result<(), Refusal> {
        if *launch != Launch::default() {
            self.require(client, Features::LAUNCH, "a launch")?;
        }
        if launch.own {
            self.require(client, Features::OWNERSHIP, "an owned launch")?;
        }
        if launch.hold {
            self.require(client, Features::HOLD, "a held launch")?;
        }
        if launch.input == InputMode::OwnerOnly {
            self.require(client, Features::INPUT_GATE, "an owner-only launch")?;
            if !launch.own {
                return Err("bad_launch(input): an owner-only block must be owned".into());
            }
        }
        Ok(())
    }

    /// Whether `client` negotiated `feature`.
    fn has(&self, client: ClientId, feature: Features) -> bool {
        self.features
            .get(&client)
            .is_some_and(|f| f.contains(feature))
    }

    fn require(&self, client: ClientId, feature: Features, what: &str) -> Result<(), Refusal> {
        if self.has(client, feature) {
            return Ok(());
        }
        Err(format!("{what} requires feature bits {:#x}", feature.bits()).into())
    }

    /// Runs block method `method` on `block`, after checking its arguments
    /// against the catalog.
    fn block_call(
        &mut self,
        client: ClientId,
        req: u64,
        block: BlockId,
        method: &str,
        args: Vec<(String, Value)>,
    ) -> Result<Outcome, Refusal> {
        let rt = self
            .blocks
            .get(&block)
            .ok_or(rusty_mux_core::Error::UnknownBlock(block))?;
        let unknown = || Refusal {
            reason: format!("terminal blocks have no method `{method}`"),
            code: Some(Code::MethodNotFound),
        };
        let found = schema::block_method(method).ok_or_else(unknown)?;
        // Moves each value under its parameter's name, so nothing is copied
        // before the catalog has checked it; an unknown name stops here.
        let mut named = vec![("block", Value::Id(block.uuid()))];
        for (name, value) in args {
            let param = found
                .params
                .iter()
                .find(|p| p.name == name)
                .ok_or_else(|| format!("{method}: {}", CallError::UnknownParam(name)))?;
            named.push((param.name, value));
        }
        found
            .request(req, &named)
            .map_err(|e| format!("{method}: {e}"))?;
        self.require(
            client,
            found.needs_for(named.iter().map(|(name, _)| *name)),
            method,
        )?;
        let text = match found.name {
            "terminal.format" => terminal::format(&rt.grid, capture(&named), REPLY_MAX)
                .ok_or_else(|| format!("{method} reply exceeds {REPLY_MAX} bytes"))?,
            "terminal.history" => history(rt.spool.as_ref(), &rt.grid, &named)?,
            "terminal.send-keys" => {
                self.mux
                    .authorize_input(client, block)
                    .map_err(Refusal::from)?;
                let keys = match named.iter().find(|(name, _)| *name == "key") {
                    Some((_, Value::Words(keys))) => keys,
                    _ => return Err(format!("{method}: missing key").into()),
                };
                let data = keys::encode(keys, rt.grid.app_cursor_keys)
                    .map_err(|e| format!("{method}: {e}"))?;
                let pty = rt.live()?.pty.clone();
                let report = self.reporting(client, block)?;
                return Ok(Outcome::Write(PtyWrite {
                    pty,
                    data,
                    key: None,
                    report,
                }));
            }
            "terminal.pwd" => terminal::pwd(&rt.grid, rt.pid())
                .ok_or("the program's working directory is unknown")?,
            "terminal.title" => rt.grid.title.clone(),
            "terminal.status" => rt.status.shown().bodies().join("\n"),
            "terminal.process" => {
                let live = rt.live()?;
                let pid = live.process.pid().ok_or("the program has exited")?;
                process::text(&live.pty.processes(pid))
            }
            "terminal.size" => format!("{}x{}", rt.grid.cols, rt.grid.rows),
            "terminal.reset" => {
                self.mux
                    .authorize_input(client, block)
                    .map_err(Refusal::from)?;
                // A client without the bit was built before CAN ended an OSC
                // string: its copy would act on what the program left
                // unfinished and miss the RIS, or both.
                let older = self.attached(block).into_iter().any(|c| {
                    !(self.features.get(&c)).is_some_and(|f| f.contains(Features::TERMINAL_CONTROL))
                });
                if older {
                    return Err(format!(
                        "{method}: a client attached to this block is too old to reset \
                         its terminal safely; detach it or update it"
                    )
                    .into());
                }
                self.inject(block, terminal::RESET);
                return Ok(Outcome::Done(None));
            }
            "terminal.modes" => terminal::modes(&rt.grid),
            "terminal.report-status" => return self.report_status(block, &named),
            "terminal.shell" => {
                // A held or restored block's program is gone (C78).
                let activity = match &rt.live {
                    None => "exited",
                    Some(live) => {
                        let pid = live.process.pid().ok_or("the program has exited")?;
                        process::activity(pid, live.pty.foreground())
                    }
                };
                terminal::shell(&rt.grid, &rt.started.program, activity)
            }
            "terminal.set-theme" | "terminal.clear" => {
                self.mux
                    .authorize_input(client, block)
                    .map_err(Refusal::from)?;
                // Bytes added inside the program's unfinished sequence would
                // end it early and leave its rest on the screen as text.
                if !rt.parser.ground() {
                    return Err(format!(
                        "{method}: the program left an escape sequence unfinished; \
                         try again, or reset the terminal"
                    )
                    .into());
                }
                let bytes = match found.name {
                    "terminal.clear" => terminal::CLEAR.to_vec(),
                    _ => terminal::theme(
                        &ThemeChange::from_args(&named).map_err(|e| format!("{method}: {e}"))?,
                    ),
                };
                self.inject(block, &bytes);
                return Ok(Outcome::Done(None));
            }
            _ => return Err(unknown()),
        };
        reply(method, &[Message::Text { req, text }])
    }

    /// Records the principal of `client`, which created `session`, as its
    /// creator (D164).
    fn record_creator(
        &mut self,
        session: rusty_mux_core::SessionId,
        client: ClientId,
    ) -> Result<(), rusty_mux_core::Error> {
        let Some(principal) = self.mux.client(client).map(|c| c.principal().clone()) else {
            return Ok(());
        };
        self.mux.set_creator(session, principal)
    }

    /// Ends `target`'s connection for `client`, as the budget does a slow
    /// client's. A client cannot end its own, since its reply would be lost
    /// with the connection, nor an owner's (C46). Otherwise the server's
    /// own account may end any; another principal only its own principal's
    /// connections and those attached to a session it created (D161, D164).
    fn disconnect_other(&mut self, client: ClientId, target: ClientId) -> Result<Outcome, Refusal> {
        if target == client {
            return Err("a client cannot disconnect itself; close the connection instead".into());
        }
        if !self.clients.contains_key(&target) && !self.orphans.contains_key(&target) {
            return Err(rusty_mux_core::Error::UnknownClient(target).into());
        }
        // An owner's connection is its owner's to end (C46).
        if self.mux.owns_any(target) {
            return Err(
                format!("client {target} owns blocks; only it may end its connection").into(),
            );
        }
        if !self.may_disconnect(client, target) {
            return Err(format!(
                "client {target} belongs to another account; only it, its session's \
                 creator or the server's own account may disconnect it"
            )
            .into());
        }
        self.drop_client(target);
        Ok(Outcome::Done(None))
    }

    /// Whether `client`'s principal may end `target`'s connection: it is
    /// the server's own account, `target`'s principal, or the creator of
    /// the session `target` is attached to.
    fn may_disconnect(&self, client: ClientId, target: ClientId) -> bool {
        let (Some(asker), Some(other)) = (self.mux.client(client), self.mux.client(target)) else {
            return false;
        };
        let asker = asker.principal().id();
        if self.account.as_ref().is_some_and(|a| a.id() == asker) || other.principal().id() == asker
        {
            return true;
        }
        self.mux
            .session_of_client(target)
            .and_then(|session| session.creator())
            .is_some_and(|creator| creator.id() == asker)
    }

    /// Every session, window and block, in creation order, each parent
    /// before its children; then, for a `client` with
    /// [`Features::LABELS`], every client.
    fn list(&self, client: ClientId, req: u64) -> Result<Outcome, Refusal> {
        let mut entries = Vec::new();
        for session in self.mux.sessions() {
            entries.push(Entry::Session {
                session: session.id(),
                name: session.name().to_owned(),
                active: session.active(),
                generated: session.generated(),
            });
            for window in session.windows().iter().filter_map(|w| self.mux.window(*w)) {
                entries.push(Entry::Window {
                    window: window.id(),
                    session: session.id(),
                    name: window.name().to_owned(),
                    layout: window.layout().cloned(),
                    zoomed: window.zoomed(),
                    focused: window.focused(),
                });
                for block in window.blocks().iter().filter_map(|b| self.mux.block(*b)) {
                    entries.push(Entry::Block {
                        block: block.id(),
                        window: window.id(),
                        size: block.size(),
                        holder: block.lease(),
                        title: self
                            .blocks
                            .get(&block.id())
                            .map(|rt| rt.grid.title.clone())
                            .unwrap_or_default(),
                        name: block.name().to_owned(),
                        owner: block.owner().map(|o| match o.client() {
                            Some(client) => Ownership::Client(client),
                            None => Ownership::Orphaned,
                        }),
                        input: block.input(),
                        exited: self
                            .blocks
                            .get(&block.id())
                            .is_some_and(|rt| rt.live.is_none()),
                    });
                }
            }
        }
        if self
            .features
            .get(&client)
            .is_some_and(|f| f.contains(Features::LABELS))
        {
            entries.extend(self.mux.clients().map(client_entry));
        }
        let mut messages: Vec<Message> = entries
            .into_iter()
            .map(|entry| Message::Entry { req, entry })
            .collect();
        messages.push(Message::Done { req, created: None });
        reply("list", &messages)
    }

    /// The caller's own client entry.
    fn whoami(&self, client: ClientId, req: u64) -> Result<Outcome, Refusal> {
        let me = self
            .mux
            .client(client)
            .ok_or(rusty_mux_core::Error::UnknownClient(client))?;
        let messages = [
            Message::Entry {
                req,
                entry: client_entry(me),
            },
            Message::Done { req, created: None },
        ];
        reply("whoami", &messages)
    }

    /// Starts a block beside `target`, sized as its part of `target`.
    fn split(
        &mut self,
        shared: &Shared,
        client: ClientId,
        target: BlockId,
        side: Side,
        launch: &Launch,
    ) -> Result<Outcome, Refusal> {
        let size = self.mux.split_size(target, side)?;
        let window = self
            .mux
            .block(target)
            .ok_or(rusty_mux_core::Error::UnknownBlock(target))?
            .window();
        self.create_block(shared, client, window, size, launch, Some((target, side)))
    }

    /// Starts a block in `window`, placed by the model, or beside the
    /// block `beside` names; owned by `client` if `launch` says so.
    fn create_block(
        &mut self,
        shared: &Shared,
        client: ClientId,
        window: WindowId,
        size: Size,
        launch: &Launch,
        beside: Option<(BlockId, Side)>,
    ) -> Result<Outcome, Refusal> {
        if self.stopping {
            return Err(String::from("the server is stopping").into());
        }
        validate_size(size)?;
        let config = launched(&self.config, launch)?;
        let session = self
            .mux
            .window(window)
            .ok_or(rusty_mux_core::Error::UnknownWindow(window))?
            .session();
        // The id is chosen first so the program starts knowing it.
        let block = BlockId::from_uuid(Uuid::new_v4());
        let started = Started {
            shell: launch.program.is_none(),
            read_only: false,
            program: config.program.to_string_lossy().into_owned(),
            args: config
                .args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect(),
            cwd: config.cwd.to_string_lossy().into_owned(),
        };
        let rt = self.start(Start {
            block,
            session,
            window,
            size,
            config: &config,
            started,
            owned: launch.own,
        })?;
        match beside {
            None => self.mux.add_block(block, window, size)?,
            Some((target, side)) => self.mux.split_block(block, target, side, size)?,
        }
        if launch.own {
            in_step("own", self.mux.own(client, block, launch.input));
        }
        let (grid, parser) = fresh_screen(
            crate::checkpoint::grid(size),
            self.config.grapheme_clustering,
        );
        let mut rt = rt.screened(shared, grid, parser);
        rt.hold = launch.hold;
        self.adopt(block, rt);
        Ok(Outcome::Done(Some(block.uuid())))
    }

    /// Holds `rt` as `block`'s, spooling the history its grid evicts
    /// while the server keeps state (C70).
    fn adopt(&mut self, block: BlockId, mut rt: BlockRt) {
        match &self.store {
            Some(store) => rt.keep_history(block, store.spool(block)),
            None => rt.stop_history(),
        }
        self.blocks.insert(block, rt);
    }

    /// Starts `start.block`'s program, for the caller to give a screen
    /// ([`Unscreened::screened`]) once the model holds the block.
    fn start(&mut self, start: Start<'_>) -> Result<Unscreened, Refusal> {
        let Start {
            block,
            session,
            window,
            size,
            config,
            started,
            owned,
        } = start;
        let env = self.block_env(session, window, block);
        let (pty, process) = pty::spawn(config, size, &env, owned)
            .map_err(|e| format!("spawning the block's program: {e}"))?;
        self.hosts.track(&process);
        let pty = Rc::new(pty);
        // Named after the program as started, so a block does not announce
        // its own program once it has exec'd.
        let foreground = process.pid().map(|pid| process::Foreground {
            pid,
            name: process::started_name(pid, &config.program),
        });
        let echo = pty.echo();
        Ok(Unscreened {
            block,
            pty,
            process,
            foreground,
            echo,
            started,
        })
    }

    /// Where a block's program is, for `rusty-mux-ctl` run inside it.
    fn block_env(
        &self,
        session: rusty_mux_core::SessionId,
        window: WindowId,
        block: BlockId,
    ) -> Vec<(&'static str, OsString)> {
        let mut env = vec![
            ("RUSTY_MUX_SESSION", session.to_string().into()),
            ("RUSTY_MUX_WINDOW", window.to_string().into()),
            ("RUSTY_MUX_BLOCK", block.to_string().into()),
        ];
        if let Some(socket) = &self.socket {
            env.push(("RUSTY_MUX_SOCKET", socket.clone().into_os_string()));
        }
        env
    }

    /// Input for the PTY, after announcing an echo mode change, which then
    /// precedes anything the input makes the program print.
    fn input(
        &mut self,
        client: ClientId,
        req: u64,
        block: BlockId,
        data: Vec<u8>,
    ) -> Result<Outcome, Refusal> {
        self.mux
            .authorize_input(client, block)
            .map_err(Refusal::from)?;
        let pty = self
            .blocks
            .get(&block)
            .ok_or_else(|| format!("block {block} has no PTY"))?
            .live()?
            .pty
            .clone();
        let report = self.reporting(client, block)?;
        self.check_echo(block);
        let key = self.echoes(client).then_some((block, req));
        Ok(Outcome::Write(PtyWrite {
            pty,
            data,
            key,
            report,
        }))
    }

    /// For input `client` may write to `block`: what to report it as, on an
    /// owner-only block, or a refusal when the owner can't be told now,
    /// being in its reconnect grace or behind on its output (C77). Input is
    /// never delivered without being reported.
    fn reporting(
        &self,
        client: ClientId,
        block: BlockId,
    ) -> Result<Option<(BlockId, ClientId)>, Refusal> {
        let Some(target) = self.mux.block(block) else {
            return Ok(None);
        };
        if target.input() != InputMode::OwnerOnly {
            return Ok(None);
        }
        let owner = target.owner().and_then(|o| o.client());
        let reachable = owner
            .and_then(|o| self.clients.get(&o))
            .is_some_and(|out| out.queued() <= INPUT_REPORT_LIMIT);
        if !reachable {
            return Err(format!(
                "input_unrecorded: block {block}'s owner can't be told of input now"
            )
            .into());
        }
        Ok(Some((block, client)))
    }

    /// `shutdown` (C36b): starts the clean stop for a client of the server's
    /// own account. A stop ends every session, so no lesser owner may ask.
    fn shutdown(&mut self, client: ClientId) -> Result<Outcome, Refusal> {
        let asker = self.mux.client(client).ok_or("unknown client")?.principal();
        let Some(account) = self.account.as_ref().filter(|_| self.shutdown.is_some()) else {
            return Err(
                "this server is not stopped by request; it is stopping already, or \
                        its owner stops it"
                    .into(),
            );
        };
        if asker.id() != account.id() {
            return Err(format!("only {} may stop this server", account.id()).into());
        }
        if let Some(stop) = self.shutdown.take() {
            // The receiver is serve_for's; gone only once it has stopped.
            let _ = stop.send(());
        }
        Ok(Outcome::Done(None))
    }

    /// Tells `block`'s owner that `client` asks for its lease (Q26): one
    /// [`Message::LeaseRequested`], to the owner only. The owner decides.
    fn request_lease(&mut self, client: ClientId, block: BlockId) -> Result<Outcome, Refusal> {
        let target = self.mux.block(block).ok_or(Error::UnknownBlock(block))?;
        if target.input() != InputMode::OwnerOnly {
            return Err(format!("block {block} takes input from anyone: acquire its lease").into());
        }
        let owner = target
            .owner()
            .and_then(|o| o.client())
            .ok_or(Error::Orphaned(block))?;
        if owner == client {
            return Err(format!("block {block} is yours: acquire its lease").into());
        }
        // As C77's input reports: an owner behind on its output isn't sent
        // more, so a flood of asks can't push it over its budget.
        let reachable = self
            .clients
            .get(&owner)
            .is_some_and(|out| out.queued() <= INPUT_REPORT_LIMIT);
        if !reachable || !self.has(owner, Features::LEASE_REQUESTS) {
            return Err(
                format!("owner_unreachable: block {block}'s owner can't be asked now").into(),
            );
        }
        let principal = self
            .mux
            .client(client)
            .ok_or(Error::UnknownClient(client))?
            .principal()
            .clone();
        let ask = Message::LeaseRequested {
            block,
            client,
            principal,
        };
        self.send(owner, &ask);
        self.flush();
        Ok(Outcome::Done(None))
    }

    /// Tells `block`'s owner of `data`, which `from` just wrote to its
    /// program (C77): one [`Message::InputBatch`], numbered per block.
    pub(crate) fn report_input(&mut self, block: BlockId, from: ClientId, data: Vec<u8>) {
        let Some(owner) = self.mux.block(block).and_then(|b| b.owner()?.client()) else {
            return;
        };
        let Some(rt) = self.blocks.get_mut(&block) else {
            return;
        };
        rt.input_seq += 1;
        let batch = Message::InputBatch {
            block,
            client: from,
            seq: rt.input_seq,
            data,
        };
        self.send(owner, &batch);
        self.flush();
    }

    /// Whether `client` negotiated [`Features::ECHO_ACK`].
    fn echoes(&self, client: ClientId) -> bool {
        self.features
            .get(&client)
            .is_some_and(|f| f.contains(Features::ECHO_ACK))
    }

    /// `client`'s input `req` to `block`, sent on the connection owning
    /// `cancel`, was written at `at`: its ack is due a grace later, at a
    /// drain or from the reader's timer. Not if another connection took
    /// the client over while the write was under way: request numbers
    /// start afresh there, so the ack would claim a request it never sent.
    pub(crate) fn written(
        &mut self,
        client: ClientId,
        cancel: &Rc<Notify>,
        (block, req): (BlockId, u64),
        at: Instant,
    ) {
        if !self.served_by(client, cancel) {
            return;
        }
        let Some(rt) = self.blocks.get_mut(&block) else {
            return;
        };
        rt.waiting.written(client, req, at);
        if let Some(live) = &rt.live {
            live.wake.notify_one();
        }
    }

    /// When the reader should next drain `block` for an ack.
    fn echo_due(&self, block: BlockId) -> Option<Instant> {
        self.blocks.get(&block)?.waiting.due()
    }

    /// A read of `block` at `at` found nothing: acks the keys whose grace
    /// ended by then, after every Output frame read before it.
    fn drained(&mut self, block: BlockId, at: Instant) {
        self.check_echo(block);
        let Some(rt) = self.blocks.get_mut(&block) else {
            return;
        };
        for (client, req) in rt.waiting.drained(at) {
            self.send(client, &Message::EchoAck { block, req });
        }
    }

    /// Reads who echoes on `block`'s terminal and, if that changed, tells
    /// every attached client with [`Features::ECHO_ACK`]. Whether it changed.
    fn check_echo(&mut self, block: BlockId) -> bool {
        let Some(rt) = self.blocks.get_mut(&block) else {
            return false;
        };
        let Some(live) = &rt.live else {
            return false;
        };
        let mode = live.pty.echo();
        if std::mem::replace(&mut rt.echo, mode) == mode {
            return false;
        }
        for client in self.attached(block) {
            self.send_echo(client, block);
        }
        true
    }

    /// Tells `client`, if it negotiated [`Features::ECHO_ACK`], who echoes
    /// on `block`'s terminal.
    fn send_echo(&mut self, client: ClientId, block: BlockId) {
        let Some(rt) = self.blocks.get(&block) else {
            return;
        };
        let mode = rt.echo;
        if self.echoes(client) {
            self.send(client, &Message::EchoMode { block, mode });
        }
    }

    /// Drops `client`'s keys on every block.
    fn forget_keys(&mut self, client: ClientId) {
        self.blocks
            .values_mut()
            .for_each(|rt| rt.waiting.forget(client));
    }

    /// The clients attached to `block`.
    fn attached(&self, block: BlockId) -> Vec<ClientId> {
        self.clients
            .keys()
            .copied()
            .filter(|&c| self.mux.client(c).and_then(|c| c.attached()) == Some(block))
            .collect()
    }

    fn resize(&mut self, client: ClientId, block: BlockId, size: Size) -> Result<Outcome, Refusal> {
        validate_size(size)?;
        self.mux
            .authorize_input(client, block)
            .map_err(Refusal::from)?;
        let rt = self
            .blocks
            .get(&block)
            .ok_or_else(|| format!("block {block} has no PTY"))?;
        // A read-only block's grid resizes alone.
        if let Some(live) = &rt.live {
            live.pty
                .resize(size)
                .map_err(|e| format!("resizing the PTY: {e}"))?;
        }
        self.mux
            .resize_block(client, block, size)
            .map_err(Refusal::from)?;
        let rt = self
            .blocks
            .get_mut(&block)
            .ok_or_else(|| format!("block {block} has no PTY"))?;
        rt.grid.resize(size.cols().into(), size.rows().into());
        rt.unsaved = true;
        Ok(Outcome::Done(None))
    }

    /// Applies an agent hook's report (`terminal.report-status`) to
    /// `block`'s status and announces what that changed in what is shown.
    fn report_status(
        &mut self,
        block: BlockId,
        named: &[(&str, Value)],
    ) -> Result<Outcome, Refusal> {
        let text = |name: &str| {
            named.iter().find_map(|(n, v)| match v {
                Value::Text(t) if *n == name => Some(t.as_str()),
                _ => None,
            })
        };
        let parts = Parts {
            id: text("id"),
            state: text("state").unwrap_or_default(),
            kind: text("kind"),
            app: text("app"),
            msg: text("msg"),
        };
        let report = parts
            .report()
            .ok_or("terminal.report-status: the fields are not a valid OSC 7501 report")?;
        let rt = self
            .blocks
            .get_mut(&block)
            .ok_or(rusty_mux_core::Error::UnknownBlock(block))?;
        let before = rt.status.shown().states();
        rt.status.hook(report, Instant::now());
        let status = terminal::absorbed(&before, &rt.status.shown().states());
        announce_status(&mut self.mux, block, status);
        Ok(Outcome::Done(None))
    }

    /// Feeds PTY output to the block's grid and tees it to attached clients.
    /// Returns the terminal's replies to queries, for the PTY.
    fn output(&mut self, block: BlockId, data: &[u8]) -> Vec<u8> {
        // A mode change, such as echo turned off before a password prompt,
        // goes ahead of the output printed after it.
        self.check_echo(block);
        let Some(rt) = self.blocks.get_mut(&block) else {
            return Vec::new();
        };
        rt.parser.advance(&mut rt.grid, data);
        rt.spill(block);
        rt.unsaved = true;
        let replies = rt.parser.take_responses();
        let status = terminal::absorb_status(&mut rt.grid, &mut rt.status, Instant::now());
        announce_status(&mut self.mux, block, status);
        let (pwd, title) = rt.seen.changes(&rt.grid);
        if pwd {
            in_step("pwd", self.mux.note_pwd(block));
        }
        if title {
            in_step("title", self.mux.note_title(block));
        }
        if std::mem::take(&mut rt.grid.bell) {
            in_step("bell", self.mux.note_bell(block));
        }
        if let Some(progress) = rt.seen.progress(&rt.grid) {
            in_step("progress", self.mux.note_progress(block, progress));
        }
        for (title, body) in rt.grid.notifications.drain(..) {
            in_step(
                "notification",
                self.mux.note_notification(block, title, body),
            );
        }
        for selection in terminal::take_clipboard(&mut rt.grid) {
            in_step("clipboard", self.mux.note_clipboard(block, selection));
        }
        // What a terminal host would relay, such as the bell; clients
        // already have the raw output.
        rt.grid.take_host_out();
        self.check_process(block, false);
        let frame = encode(&Message::Output {
            block,
            data: data.to_vec(),
        });
        let attached = self.attached(block);
        match frame {
            Ok(frame) => attached
                .into_iter()
                .for_each(|c| self.send_frame(c, frame.clone())),
            Err(err) => eprintln!("rusty-mux-server: encoding output of block {block}: {err}"),
        }
        self.flush();
        replies
    }

    /// Feeds `bytes` of the server's own to `block`'s terminal and to every
    /// client attached to it, as if its program had written them, so all
    /// copies of the terminal stay alike. They must ask nothing: the
    /// program would read an answer as input.
    fn inject(&mut self, block: BlockId, bytes: &[u8]) {
        let replies = self.output(block, bytes);
        if !replies.is_empty() {
            eprintln!(
                "rusty-mux-server: block {block} answered the server's own bytes; \
                 {} bytes of answer dropped",
                replies.len()
            );
        }
    }

    /// Announces the foreground process group of `block` if another took
    /// its terminal since the last check, or, on a `refresh`, if its
    /// program's name changed ([`process::Foreground::observe`]).
    fn check_process(&mut self, block: BlockId, refresh: bool) {
        let Some(rt) = self.blocks.get_mut(&block) else {
            return;
        };
        let foreground = rt.live.as_ref().and_then(|l| l.pty.foreground());
        let (Some(pid), Some(seen)) = (foreground, rt.foreground.as_mut()) else {
            return;
        };
        if !seen.observe(pid, refresh, process::lookup) {
            return;
        }
        let name = seen.name.clone();
        in_step("process", self.mux.note_process(block, pid, name));
    }

    /// [`Self::check_process`], names refreshed, and [`Self::check_echo`]
    /// for every block, then broadcasts.
    fn check_processes(&mut self) {
        let blocks: Vec<BlockId> = self.blocks.keys().copied().collect();
        for block in blocks {
            self.check_process(block, true);
            self.check_echo(block);
        }
        self.flush();
    }

    /// The block's output ended: stop serving its PTY, without aborting the
    /// reader that is calling this. Its program, to reap; `None` if a
    /// client closed the block first. A held block stays, read-only, with
    /// its screen and scrollback (C78).
    fn hang_up(&mut self, block: BlockId) -> Option<Process> {
        if let Some(rt) = self.blocks.get_mut(&block)
            && rt.hold
        {
            let live = rt.live.take()?;
            // Saved and restored read-only from now on (D72).
            rt.started.read_only = true;
            rt.unsaved = true;
            live.pty.release();
            return Some(live.process);
        }
        self.blocks.remove(&block).and_then(|rt| rt.release(false))
    }

    /// The block's program ended: announce how, if known, and close the
    /// block, unless a client closed it while the program was reaped or it
    /// is held (C78).
    fn exited(&mut self, block: BlockId, status: Option<ExitStatus>) {
        if let Some(status) = status
            && self.mux.note_exit(block, status).is_err()
        {
            return;
        }
        let held = self.blocks.get(&block).is_some_and(|rt| rt.hold);
        if held {
            return self.flush();
        }
        if self.mux.close_block(block).is_ok() {
            self.flush();
        }
    }

    fn send_snapshot(&mut self, client: ClientId, block: BlockId) {
        let Some(rt) = self.blocks.get(&block) else {
            return;
        };
        // With C03 the checkpoint carries the screen only and the history
        // follows Ready, newest first.
        let encoded = if self.has(client, Features::BACKGROUND_HISTORY) {
            rusty_term_core::checkpoint::encode_split(
                &rt.grid,
                &rt.parser,
                crate::checkpoint::CHUNK_BYTES,
            )
        } else {
            rusty_term_core::checkpoint::encode(&rt.grid, &rt.parser).map(|data| (data, Vec::new()))
        };
        let (data, history) = match encoded {
            Ok(encoded) => encoded,
            Err(err) => {
                eprintln!("rusty-mux-server: encoding checkpoint: {err}");
                self.drop_client(client);
                return;
            }
        };
        if !self
            .clients
            .get(&client)
            .is_some_and(|out| out.checkpoint(block, data, history))
        {
            self.drop_client(client);
        }
    }

    fn send(&mut self, client: ClientId, message: &Message) {
        match encode(message) {
            Ok(frame) => self.send_frame(client, frame),
            Err(err) => {
                eprintln!("rusty-mux-server: encoding a message for client {client}: {err}");
                self.drop_client(client);
            }
        }
    }

    /// Queues a frame; a client over budget is dropped.
    fn send_frame(&mut self, client: ClientId, frame: Vec<u8>) {
        let Some(outbox) = self.clients.get(&client) else {
            return;
        };
        if !outbox.push(frame) {
            if outbox.is_datagram() && self.tokens.contains_key(&client) {
                return self.go_stale(client);
            }
            eprintln!("rusty-mux-server: dropping client {client}: output queue over budget");
            self.drop_client(client);
        }
    }

    /// Deletes closed `block`'s spool by its path, whether or not a handle
    /// to it is left. A stopping server closes no block, so keeps them all.
    fn forget_history(&self, block: BlockId) {
        if let Some(Err(e)) = self.store.as_ref().map(|s| s.remove_spool(block)) {
            eprintln!("rusty-mux-server: removing block {block}'s history: {e}");
        }
    }

    /// Broadcasts queued model events and releases closed blocks' PTYs,
    /// until a pass queues nothing new.
    fn flush(&mut self) {
        loop {
            let events = self.mux.take_events();
            if events.is_empty() {
                return;
            }
            for event in events {
                if let Event::BlockClosed { block } = event {
                    // Its program may have hung up, or its spool failed, first.
                    if let Some(rt) = self.blocks.remove(&block) {
                        close(rt);
                    }
                    self.forget_history(block);
                }
                let seq = self.history.push(event.clone());
                let (Some(plain), Some(numbered)) =
                    (event_frame(None, &event), event_frame(Some(seq), &event))
                else {
                    continue;
                };
                let clients: Vec<(ClientId, bool)> = self
                    .features
                    .iter()
                    .filter(|(_, f)| receives(**f, &event))
                    .map(|(c, f)| (*c, f.contains(Features::EVENT_SEQ)))
                    .collect();
                for (client, numbers) in clients {
                    let frame = if numbers { &numbered } else { &plain };
                    self.send_frame(client, frame.clone());
                }
            }
        }
    }
}

/// Lets a closed block's program go, to be reaped, and its spool's files.
fn close(mut rt: BlockRt) {
    drop(rt.spool.take());
    if let Some(process) = rt.release(true) {
        rusty_tokio::task::spawn_local(process.reap());
    }
}

/// The server's `config` with what `launch` asks for instead. A relative
/// working directory would depend on where the server started, so it is
/// refused, as is one that is not a directory.
fn launched(config: &Config, launch: &Launch) -> Result<Config, Refusal> {
    let mut config = config.clone();
    if let Some(cwd) = &launch.cwd {
        let path = std::path::Path::new(cwd);
        if !path.is_absolute() {
            return Err(format!("cwd `{cwd}` is not an absolute path").into());
        }
        if !path.is_dir() {
            return Err(format!("cwd `{cwd}` is not a directory").into());
        }
        config.cwd = cwd.into();
    }
    if let Some(program) = &launch.program {
        config.program = program.command.clone().into();
        config.args = program.args.iter().map(Into::into).collect();
    }
    Ok(config)
}

/// Whether a client with `features` is sent `event`: a status event only
/// with [`Features::STATUS`], a pwd or title event only with
/// [`Features::TERMINAL_EVENTS`], so older clients never see a kind they
/// can't decode.
fn receives(features: Features, event: &Event) -> bool {
    let needs = match event {
        Event::BlockStatus { .. } => Features::STATUS,
        Event::PwdChanged { .. } | Event::TitleChanged { .. } => Features::TERMINAL_EVENTS,
        Event::ProgramExited { .. }
        | Event::Bell { .. }
        | Event::ProgressChanged { .. }
        | Event::Notification { .. } => Features::PROGRAM_EVENTS,
        Event::ProcessChanged { .. } => Features::PROCESS,
        Event::SessionRenamed { .. }
        | Event::WindowRenamed { .. }
        | Event::BlockRenamed { .. }
        | Event::ClientRenamed { .. } => Features::LABELS,
        Event::LayoutChanged { .. } => Features::LAYOUT,
        Event::ClientConnectionChanged { .. } => Features::IDENTITY,
        Event::ClipboardWritten { .. } | Event::StatusRemoved { .. } => {
            Features::MORE_PROGRAM_EVENTS
        }
        Event::ActiveWindowChanged { .. } => Features::ACTIVE_WINDOW,
        Event::ConfigReloaded => Features::CONFIG_RELOAD,
        Event::WindowMoved { .. } => Features::LAYOUT_EDIT,
        Event::FocusChanged { .. } => Features::FOCUS,
        Event::GrantChanged { .. } => Features::INPUT_GATE,
        _ => Features::NONE,
    };
    features.contains(needs)
}

/// A principal for tests, which build a state without a peer check.
/// A new block's screen: `grid` and a fresh parser, with DEC mode 2027
/// (grapheme clustering) set when `clustering`. Set through the parser, so
/// DECRQM reports it, a program can reset it, and the checkpoint carries it
/// to every client.
pub(crate) fn fresh_screen(mut grid: Grid, clustering: bool) -> (Grid, AnsiParser) {
    let mut parser = AnsiParser::new();
    if clustering {
        parser.advance(&mut grid, b"\x1b[?2027h");
    }
    (grid, parser)
}

#[cfg(test)]
pub(crate) fn test_owner() -> Principal {
    Principal::unix(1000, Some("tester".into())).expect("valid")
}

/// A client's entry in a listing: with its principal and transport, which
/// a client without [`Features::IDENTITY`] skips.
fn client_entry(client: &rusty_mux_core::Client) -> Entry {
    Entry::Client {
        client: client.id(),
        name: client.name().to_owned(),
        attached: client.attached(),
        principal: Some(client.principal().clone()),
        transport: Some(client.transport()),
    }
}

/// Reports a model call that fails only if the server's state and the
/// model's disagree.
/// Announces the status records that ended, then a state change, if any.
fn announce_status(mux: &mut Mux, block: BlockId, status: terminal::Absorbed) {
    for id in status.removed {
        in_step("status removed", mux.note_status_removed(block, id));
    }
    if status.changed {
        in_step("status", mux.note_status(block));
    }
}

fn in_step(what: &str, result: Result<(), rusty_mux_core::Error>) {
    if let Err(err) = result {
        eprintln!("rusty-mux-server: model out of step on {what}: {err}");
    }
}

fn event_frame(seq: Option<u64>, event: &Event) -> Option<Vec<u8>> {
    let message = Message::Event {
        seq,
        event: event.clone(),
    };
    encode(&message)
        .map_err(|err| eprintln!("rusty-mux-server: encoding an event: {err}"))
        .ok()
}

/// A `TAKEOVER` client's tokens. `current` is the one its last Welcome
/// carried, and `confirmed` the one it last proved it holds, by presenting
/// it in a takeover or by sending a request after the Welcome that carried
/// it. A takeover whose Welcome never arrived leaves the client holding
/// only `confirmed`, so both are accepted; once the client proves
/// `current`, the older token stops working.
#[derive(Debug, Clone, Copy)]
struct Tokens {
    current: Uuid,
    confirmed: Option<Uuid>,
}

/// Why a request was refused, sent back as [`Message::Failed`].
#[derive(Debug)]
struct Refusal {
    reason: String,
    code: Option<Code>,
}

impl From<rusty_mux_core::Error> for Refusal {
    fn from(err: rusty_mux_core::Error) -> Self {
        use rusty_mux_core::Error as E;
        let code = match err {
            E::UnknownSession(_)
            | E::UnknownWindow(_)
            | E::UnknownBlock(_)
            | E::UnknownClient(_) => Some(Code::NotFound),
            _ => None,
        };
        Self {
            reason: err.to_string(),
            code,
        }
    }
}

impl From<String> for Refusal {
    fn from(reason: String) -> Self {
        Self { reason, code: None }
    }
}

impl From<&str> for Refusal {
    fn from(reason: &str) -> Self {
        reason.to_owned().into()
    }
}

enum Outcome {
    Done(Option<Uuid>),
    Attached(BlockId),
    /// Encoded messages answering the request, in order.
    Reply(Vec<Vec<u8>>),
    Write(PtyWrite),
    /// Answered later, by the task the request started; resolves once it
    /// has.
    Later(oneshot::Receiver<()>),
}

/// The line count `param` holds, 0 when left out.
fn lines(named: &[(&str, Value)], param: &str) -> u32 {
    named
        .iter()
        .find_map(|(name, value)| match value {
            Value::Lines(n) if *name == param => Some(*n),
            _ => None,
        })
        .unwrap_or_default()
}

/// `terminal.format`'s options from its arguments, which the catalog
/// checked: a flag left out is off.
fn capture(named: &[(&str, Value)]) -> terminal::Capture {
    let flag = |param| {
        named
            .iter()
            .any(|(name, value)| *name == param && *value == Value::Flag(true))
    };
    terminal::Capture {
        // Required, so always there.
        history: lines(named, "history"),
        trim: flag("trim"),
        unwrap: flag("unwrap"),
        vt: flag("vt"),
    }
}

/// `terminal.history`: the spooled lines `named` asks for, as
/// `terminal.format` would show them in `grid`, the block's.
fn history(spool: Option<&Spool>, grid: &Grid, named: &[(&str, Value)]) -> Result<String, Refusal> {
    let spool = spool.ok_or(
        "no history is kept past this block's scrollback: the server keeps no state, \
         or writing it failed",
    )?;
    let capture = capture(named);
    let read = spool
        .read(lines(named, "skip").into(), capture.history.into())
        .map_err(|e| format!("reading the history: {e}"))?;
    format_history(&read, grid, capture, REPLY_MAX)
}

/// Spilled lines `read`, oldest first, as `terminal.format` shows them
/// in `like`, the grid that spilled them, with `capture`, refused past
/// `max` bytes.
fn format_history(
    read: &[Vec<u8>],
    like: &Grid,
    capture: terminal::Capture,
    max: usize,
) -> Result<String, Refusal> {
    let Some(above) = read.len().checked_sub(1) else {
        return Ok(String::new());
    };
    let read: Vec<&[u8]> = read.iter().map(Vec::as_slice).collect();
    let grid = rusty_term_core::checkpoint::spilled_grid(&read, like)
        .map_err(|e| format!("the history is damaged: {e}"))?;
    let capture = terminal::Capture {
        history: u32::try_from(above).unwrap_or(u32::MAX),
        ..capture
    };
    terminal::format(&grid, capture, max)
        .ok_or_else(|| format!("terminal.history reply exceeds {max} bytes").into())
}

/// Encodes the messages answering a `what` request, all or nothing: a reply
/// over [`REPLY_MAX`] is refused before any of it is sent.
fn reply(what: &str, messages: &[Message]) -> Result<Outcome, Refusal> {
    let mut size = 0;
    let mut frames = Vec::with_capacity(messages.len());
    for message in messages {
        let frame = encode(message).map_err(|e| format!("{what}: {e}"))?;
        size += frame.len();
        if size > REPLY_MAX {
            return Err(format!("{what} reply exceeds {REPLY_MAX} bytes").into());
        }
        frames.push(frame);
    }
    Ok(Outcome::Reply(frames))
}

/// Checks every block's foreground process group and echo mode each
/// [`PROCESS_POLL`], until the server's state is dropped. Holding it
/// weakly, the poll never keeps a stopped server's state alive.
pub(crate) async fn watch_processes(state: Weak<RefCell<State>>) {
    loop {
        rusty_tokio::time::sleep(PROCESS_POLL).await;
        let Some(shared) = state.upgrade() else {
            return;
        };
        shared.borrow_mut().check_processes();
    }
}

/// Reads a block's PTY until its program exits. After output arrives, or
/// when a key's echo grace ends, it reads until the PTY is empty, then acks
/// the keys whose grace that covers.
async fn pump(shared: Shared, block: BlockId, pty: Rc<Pty>, wake: Rc<Notify>) {
    let mut buf = vec![0; CHUNK];
    'run: loop {
        let due = shared.borrow().echo_due(block);
        let grace = async {
            match due {
                Some(due) => {
                    rusty_tokio::time::sleep_until(rusty_tokio::time::Instant::from_std(due)).await;
                }
                None => std::future::pending().await,
            }
        };
        let read = rusty_tokio::select! {
            biased;
            read = pty.read(&mut buf) => Some(read),
            () = wake.notified() => None,
            () = grace => None,
        };
        if let Some(read) = read
            && !forward(&shared, block, &pty, read, &buf).await
        {
            break;
        }
        loop {
            let at = Instant::now();
            let read = match pty.try_read(&mut buf) {
                Ok(Some(n)) => Ok(n),
                Ok(None) => {
                    shared.borrow_mut().drained(block, at);
                    break;
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => Err(err),
            };
            if !forward(&shared, block, &pty, read, &buf).await {
                break 'run;
            }
        }
    }
    let process = shared.borrow_mut().hang_up(block);
    let status = match process {
        Some(process) => process.reap().await,
        None => None,
    };
    shared.borrow_mut().exited(block, status);
}

/// Feeds one read's output to the block and writes the terminal's replies
/// to queries back. Whether to go on reading: not at the end of the output
/// or after an error.
async fn forward(
    shared: &Shared,
    block: BlockId,
    pty: &Pty,
    read: std::io::Result<usize>,
    buf: &[u8],
) -> bool {
    let n = match read {
        Ok(0) => return false,
        Ok(n) => n,
        Err(err) => {
            eprintln!("rusty-mux-server: reading block {block}: {err}");
            return false;
        }
    };
    let replies = shared.borrow_mut().output(block, &buf[..n]);
    if !replies.is_empty()
        && let Err(err) = pty.write_all(&replies).await
    {
        eprintln!("rusty-mux-server: answering block {block}: {err}");
        return false;
    }
    // A flooding PTY is always readable; yield so input and client writes
    // get their turn between chunks.
    rusty_tokio::task::yield_now().await;
    true
}

#[cfg(all(test, unix))]
impl State {
    pub(crate) fn test_client(&self) -> ClientId {
        *self.clients.keys().next().unwrap()
    }
    pub(crate) fn test_token(&self, client: ClientId) -> Uuid {
        self.tokens[&client].current
    }
    pub(crate) fn test_checkpoint(&self, client: ClientId, data: Vec<u8>) {
        assert!(self.clients[&client].checkpoint(
            BlockId::from_uuid(Uuid::nil()),
            data,
            Vec::new()
        ));
    }
    pub(crate) fn test_send(&mut self, client: ClientId, data: Vec<u8>) {
        self.send_frame(client, data);
    }
    pub(crate) fn test_queue(&self, client: ClientId) -> Rc<std::cell::Cell<usize>> {
        self.clients[&client].test_queue()
    }
}

// These start `/bin/sh`.
#[cfg(all(test, unix))]
mod regression {
    use super::*;
    use crate::ConsoleHost;
    use crate::checkpoint::Receiver;
    use crate::outbox::{Inbox, outbox};
    use rusty_tokio::{Builder, LocalOptions};

    fn runtime() -> rusty_tokio::LocalRuntime {
        Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .unwrap()
    }

    /// Dropping the state ends the poll; nothing else need stop it.
    #[test]
    fn the_process_poll_ends_with_the_state() {
        runtime().block_on(async {
            let shared = State::shared(
                Config {
                    program: "/bin/sh".into(),
                    args: vec![],
                    cwd: "/".into(),
                    console_host: ConsoleHost::default(),
                    state_dir: None,
                    grapheme_clustering: false,
                    config_file: None,
                },
                None,
            );
            let poll = rusty_tokio::task::spawn_local(watch_processes(Rc::downgrade(&shared)));
            // One pass over an empty state first.
            rusty_tokio::time::sleep(PROCESS_POLL * 2).await;
            assert!(!poll.is_finished());
            drop(shared);
            rusty_tokio::time::timeout(PROCESS_POLL * 4, poll)
                .await
                .expect("the poll outlived the state")
                .unwrap();
        });
    }

    fn fixture() -> (Shared, ClientId, BlockId, Inbox) {
        fixture_with(false)
    }

    /// [`fixture`] with the config's `grapheme_clustering` set to `clustering`.
    fn fixture_with(clustering: bool) -> (Shared, ClientId, BlockId, Inbox) {
        let shared = State::shared(
            Config {
                program: "/bin/sh".into(),
                args: vec![],
                cwd: "/".into(),
                console_host: ConsoleHost::default(),
                state_dir: None,
                grapheme_clustering: clustering,
                config_file: None,
            },
            None,
        );
        let (out, inbox, _) = outbox();
        let mut state = shared.borrow_mut();
        let client = state.connect(
            out,
            test_owner(),
            Transport::Socket,
            Features::CHECKPOINT_V6,
            None,
            None,
        );
        let session = state.mux.create_session("s").unwrap();
        let window = state.mux.create_window(session, "w").unwrap();
        let Outcome::Done(Some(id)) = state
            .create_block(
                &shared,
                client,
                window,
                Size::new(80, 24).unwrap(),
                &Launch::default(),
                None,
            )
            .unwrap()
        else {
            panic!()
        };
        drop(state);
        (shared, client, BlockId::from_uuid(id), inbox)
    }

    /// A PTY write that finishes after a takeover belongs to the old
    /// connection, whose request numbers mean nothing on the new one.
    #[test]
    fn a_write_finishing_after_a_takeover_is_not_acked() {
        runtime().block_on(async {
            let (shared, _, block, _inbox) = fixture();
            let features = Features::CHECKPOINT_V6
                .union(Features::TAKEOVER)
                .union(Features::ECHO_ACK);
            let (out, _old_inbox, old) = outbox();
            let client = shared.borrow_mut().connect(
                out,
                test_owner(),
                Transport::Socket,
                features,
                None,
                None,
            );
            let token = shared.borrow().tokens[&client].current;
            let (out, _new_inbox, new) = outbox();
            let takeover = Some(Takeover { client, token });
            let taken = shared.borrow_mut().connect(
                out,
                test_owner(),
                Transport::Socket,
                features,
                None,
                takeover,
            );
            assert_eq!(taken, client);

            let now = Instant::now();
            shared.borrow_mut().written(client, &old, (block, 7), now);
            assert_eq!(shared.borrow().echo_due(block), None, "the old key");
            shared.borrow_mut().written(client, &new, (block, 1), now);
            assert!(shared.borrow().echo_due(block).is_some(), "the new key");
        });
    }

    /// A lease silence released goes with the client when it then goes
    /// stale on the same connection, so a takeover from anywhere gives it
    /// back, not only one on the silent association.
    #[test]
    fn a_lease_released_by_silence_comes_back_on_a_stale_takeover() {
        runtime().block_on(async {
            let (shared, _, block, _inbox) = fixture();
            let features = Features::CHECKPOINT_V6.union(Features::TAKEOVER);
            let (out, _old_inbox, old) = outbox();
            let mut state = shared.borrow_mut();
            let client = state.connect(out, test_owner(), Transport::Socket, features, None, None);
            state.mux.attach(client, block).unwrap();
            state.mux.acquire_lease(client, block).unwrap();
            let token = state.tokens[&client].current;

            state.release_silent(&[(client, old.clone())]);
            assert_eq!(state.lease_of(block), None);
            state.go_stale(client);
            let (out, _new_inbox, _) = outbox();
            let takeover = Some(Takeover { client, token });
            assert_eq!(
                state.connect(
                    out,
                    test_owner(),
                    Transport::Socket,
                    features,
                    None,
                    takeover
                ),
                client
            );
            assert_eq!(state.lease_of(block), Some(client));
        });
    }

    /// A takeover must come from the client's own principal: a valid
    /// token presented by anyone else continues nothing, takes no lease,
    /// and leaves the token good for its owner (C26).
    #[test]
    fn a_takeover_by_another_principal_starts_a_client_of_its_own() {
        runtime().block_on(async {
            let (shared, _, block, _inbox) = fixture();
            let features = Features::CHECKPOINT_V6.union(Features::TAKEOVER);
            let mut state = shared.borrow_mut();
            let (out, _owner_inbox, _) = outbox();
            let client = state.connect(out, test_owner(), Transport::Socket, features, None, None);
            state.mux.attach(client, block).unwrap();
            state.mux.acquire_lease(client, block).unwrap();
            let takeover = Some(Takeover {
                client,
                token: state.tokens[&client].current,
            });

            let stranger = Principal::unix(1001, Some("other".into())).unwrap();
            let (out, _stranger_inbox, _) = outbox();
            let theirs = state.connect(
                out,
                stranger.clone(),
                Transport::Datagram,
                features,
                None,
                takeover,
            );
            assert_ne!(theirs, client);
            assert_eq!(state.mux.client(theirs).unwrap().principal(), &stranger);
            assert_eq!(state.mux.client(theirs).unwrap().attached(), None);
            assert_eq!(state.lease_of(block), Some(client));
            let mine = state.mux.client(client).unwrap();
            assert_eq!(mine.transport(), Transport::Socket);

            let (out, _new_inbox, _) = outbox();
            let again = state.connect(
                out,
                test_owner(),
                Transport::Socket,
                features,
                None,
                takeover,
            );
            assert_eq!(again, client, "the token still works for its owner");
        });
    }

    /// Attaches to `block` and returns the client's grid and parser as the
    /// checkpoint leaves them at Ready.
    async fn attached(
        shared: &Shared,
        client: ClientId,
        block: BlockId,
        inbox: &mut Inbox,
    ) -> (Grid, AnsiParser) {
        State::request(shared, client, Message::Attach { req: 78, block });
        let mut receiver = Receiver::default();
        loop {
            match rusty_mux_proto::decode(&inbox.next().await.unwrap())
                .unwrap()
                .unwrap()
                .0
            {
                Message::Checkpoint {
                    offset,
                    total,
                    data,
                    ..
                } => receiver.push(offset, total, &data).unwrap(),
                Message::Ready { .. } => break receiver.finish().unwrap(),
                _ => {}
            }
        }
    }

    /// DECRQM's reply for mode 2027 on a client's copy.
    fn mode_2027(grid: &mut Grid, parser: &mut AnsiParser) -> Vec<u8> {
        parser.advance(grid, b"\x1b[?2027$p");
        parser.take_responses()
    }

    #[test]
    fn a_new_block_starts_in_mode_2027_when_the_config_says_so() {
        runtime().block_on(async {
            for (clustering, reply) in [(false, &b"\x1b[?2027;2$y"[..]), (true, b"\x1b[?2027;1$y")]
            {
                let (shared, client, block, mut inbox) = fixture_with(clustering);
                let (mut grid, mut parser) = attached(&shared, client, block, &mut inbox).await;
                assert_eq!(
                    mode_2027(&mut grid, &mut parser),
                    reply,
                    "clustering {clustering}"
                );
            }
        });
    }

    #[test]
    fn a_clustered_block_attaches_to_the_same_cells() {
        runtime().block_on(async {
            let (shared, client, block, mut inbox) = fixture_with(true);
            // A ZWJ sequence split across the attach point.
            shared.borrow_mut().output(block, "👨".as_bytes());
            let (mut grid, mut parser) = attached(&shared, client, block, &mut inbox).await;
            let rest = "\u{200d}💻".as_bytes();
            shared.borrow_mut().output(block, rest);
            parser.advance(&mut grid, rest);
            let state = shared.borrow();
            let server = &state.blocks[&block].grid;
            assert_eq!(grid.encode_snapshot(), server.encode_snapshot());
            assert_eq!(server.cursor, (2, 0), "one cluster, two columns");
        });
    }

    #[test]
    fn a_program_can_reset_the_configured_mode_2027() {
        runtime().block_on(async {
            let (shared, client, block, mut inbox) = fixture_with(true);
            shared.borrow_mut().output(block, b"\x1b[?2027l");
            let (mut grid, mut parser) = attached(&shared, client, block, &mut inbox).await;
            assert_eq!(mode_2027(&mut grid, &mut parser), b"\x1b[?2027;2$y");
        });
    }

    #[test]
    fn attach_continues_exact_server_state() {
        runtime().block_on(async {
            for (prefix, suffix) in [
                (&b"\x1b[31m"[..], &b"X"[..]),
                (&b"\x1b[?25l"[..], &b"X\x1b[?25$p"[..]),
                (&b"\x1b["[..], &b"31mX"[..]),
                (&b"\xe2\x82"[..], &b"\xac"[..]),
                (&b"\x1b]2;part"[..], &b"ial\x07X"[..]),
                (&b"\x1b(0"[..], &b"qx\x1b(BX"[..]),
                (&b"\x1b[?2004h\x1b[?1h\x1b[?1003h"[..], &b"X"[..]),
                (&b"\x1b[?1049h\x1b[31mA"[..], &b"\x1b[?1049lX"[..]),
            ] {
                let (shared, client, block, mut inbox) = fixture();
                shared.borrow_mut().output(block, prefix);
                State::request(&shared, client, Message::Attach { req: 77, block });
                let mut receiver = Receiver::default();
                let (mut grid, mut parser) = loop {
                    match rusty_mux_proto::decode(&inbox.next().await.unwrap())
                        .unwrap()
                        .unwrap()
                        .0
                    {
                        Message::Checkpoint {
                            offset,
                            total,
                            data,
                            ..
                        } => receiver.push(offset, total, &data).unwrap(),
                        Message::Ready { .. } => break receiver.finish().unwrap(),
                        _ => {}
                    }
                };
                let replies = shared.borrow_mut().output(block, suffix);
                parser.advance(&mut grid, suffix);
                assert_eq!(parser.take_responses(), replies);
                let state = shared.borrow();
                let server = &state.blocks[&block].grid;
                assert_eq!(
                    grid.encode_snapshot(),
                    server.encode_snapshot(),
                    "prefix {prefix:?}"
                );
                assert_eq!(grid.cursor_visible, server.cursor_visible);
                assert_eq!(grid.bracketed_paste, server.bracketed_paste);
                assert_eq!(grid.app_cursor_keys, server.app_cursor_keys);
                assert_eq!(grid.mouse_modes, server.mouse_modes);
                drop(state);
                State::request(&shared, client, Message::CloseBlock { req: 99, block });
            }
        });
    }

    #[test]
    fn legacy_attach_and_encoding_failure_never_send_ready() {
        runtime().block_on(async {
            let (shared, client, block, mut inbox) = fixture();
            for features in [
                Features::NONE,
                Features::REPLAY_SNAPSHOT,
                Features::CHECKPOINT_V1,
                Features::CHECKPOINT_V2,
                Features::CHECKPOINT_V3,
                Features::CHECKPOINT_V4,
                Features::CHECKPOINT_V5,
            ] {
                shared.borrow_mut().features.insert(client, features);
                State::request(&shared, client, Message::Attach { req: 10, block });
            }
            assert_eq!(shared.borrow().mux.client(client).unwrap().attached(), None);
            // Force a frame encoding failure. Subsequent sends (including
            // Ready) must find no client rather than emit an incomplete attach.
            shared.borrow_mut().send(
                client,
                &Message::Snapshot {
                    block,
                    data: vec![0; rusty_mux_proto::MAX_FRAME as usize],
                },
            );
            shared.borrow_mut().send(client, &Message::Ready { block });
            assert!(!shared.borrow().has_client(client));
            let mut failed = false;
            while let Some(frame) = inbox.next().await {
                let message = rusty_mux_proto::decode(&frame).unwrap().unwrap().0;
                assert!(!matches!(
                    message,
                    Message::Ready { .. } | Message::Snapshot { .. }
                ));
                if matches!(message, Message::Failed { req: 10, .. }) {
                    failed = true;
                }
            }
            assert!(failed);
            shared.borrow_mut().mux.close_block(block).unwrap();
            shared.borrow_mut().flush();
        });
    }

    #[test]
    fn invalid_checkpoint_closes_without_chunks_or_ready() {
        runtime().block_on(async {
            let (shared, client, block, mut inbox) = fixture();
            shared
                .borrow_mut()
                .blocks
                .get_mut(&block)
                .unwrap()
                .grid
                .cursor = (usize::MAX, 0);
            State::request(&shared, client, Message::Attach { req: 12, block });
            assert!(!shared.borrow().has_client(client));
            while let Some(frame) = inbox.next().await {
                assert!(!matches!(
                    rusty_mux_proto::decode(&frame).unwrap().unwrap().0,
                    Message::Checkpoint { .. } | Message::Ready { .. }
                ));
            }
            shared.borrow_mut().mux.close_block(block).unwrap();
            shared.borrow_mut().flush();
        });
    }

    #[test]
    fn sustained_output_and_repeated_attach_preserve_normal_history() {
        runtime().block_on(async {
            let (shared, client, block, mut inbox) = fixture();
            // More than the rejected lifetime replay cap; only current Grid
            // state is retained. Fill the normal 10,000-row history as well.
            // Varied text, so the run-encoded checkpoint still spans many chunks.
            let bytes: Vec<u8> = (0..16u32 << 10)
                .map(|i| b'!' + (i * 7 % 94) as u8)
                .collect();
            for _ in 0..320 {
                shared.borrow_mut().output(block, &bytes);
            }
            assert_eq!(shared.borrow().blocks[&block].grid.scrollback_len(), 10_000);
            for req in 1..=3 {
                State::request(&shared, client, Message::Attach { req, block });
                // Continuation output arrives while the checkpoint is still
                // queued: it must follow Ready and apply exactly once.
                shared.borrow_mut().output(block, b"TAIL");
                let mut receiver = Receiver::default();
                let mut chunks = 0;
                let mut declared = 0;
                let (mut grid, mut parser) = loop {
                    let frame = inbox.next().await.unwrap();
                    assert!(frame.len() <= crate::checkpoint::CHUNK_BYTES + 128);
                    match rusty_mux_proto::decode(&frame).unwrap().unwrap().0 {
                        Message::Checkpoint {
                            offset,
                            total,
                            data,
                            ..
                        } => {
                            chunks += 1;
                            declared = total;
                            receiver.push(offset, total, &data).unwrap();
                        }
                        Message::Ready { .. } => break receiver.finish().unwrap(),
                        Message::Output { .. } => panic!("output preceded Ready"),
                        _ => {}
                    }
                };
                assert!(declared > 8 * crate::checkpoint::CHUNK_BYTES as u64);
                assert!(chunks > 8);
                loop {
                    if let Message::Output { data, .. } =
                        rusty_mux_proto::decode(&inbox.next().await.unwrap())
                            .unwrap()
                            .unwrap()
                            .0
                    {
                        assert_eq!(data, b"TAIL");
                        parser.advance(&mut grid, &data);
                        break;
                    }
                }
                assert_eq!(grid.scrollback_len(), 10_000);
                assert_eq!(
                    grid.encode_snapshot(),
                    shared.borrow().blocks[&block].grid.encode_snapshot()
                );
                State::request(&shared, client, Message::Detach { req: req + 10 });
            }
            State::request(&shared, client, Message::CloseBlock { req: 99, block });
        });
    }

    /// Reads until the reply to `req`.
    async fn reply_to(inbox: &mut Inbox, req: u64) -> Message {
        loop {
            let frame = inbox.next().await.expect("a reply");
            let message = rusty_mux_proto::decode(&frame).unwrap().unwrap().0;
            if let Message::Done { req: r, .. } | Message::Failed { req: r, .. } = message
                && r == req
            {
                return message;
            }
        }
    }

    /// A state whose reloads wait for the returned sender's go, and a
    /// `CONFIG_RELOAD` client of it.
    fn stalled_reloads() -> (Shared, ClientId, Inbox, std::sync::mpsc::Sender<()>) {
        let base = Config {
            program: "/bin/sh".into(),
            args: vec![],
            cwd: "/".into(),
            console_host: ConsoleHost::default(),
            state_dir: None,
            grapheme_clustering: false,
            config_file: Some("/stalled.json".into()),
        };
        let shared = State::shared(base, None);
        let (loader, go) = crate::settings::stalled();
        shared.borrow_mut().loader = loader;
        let (client, inbox) = reload_client(&shared);
        (shared, client, inbox, go)
    }

    fn reload_client(shared: &Shared) -> (ClientId, Inbox) {
        let (out, inbox, _) = outbox();
        let client = shared.borrow_mut().connect(
            out,
            test_owner(),
            Transport::Socket,
            Features::EVENT_SEQ.union(Features::CONFIG_RELOAD),
            None,
            None,
        );
        (client, inbox)
    }

    /// A `LABELS` client of the principal with `uid`.
    fn client_of(shared: &Shared, uid: u32) -> (ClientId, Inbox) {
        let (out, inbox, _) = outbox();
        let principal = Principal::unix(uid, Some(format!("user{uid}"))).expect("valid");
        let client = shared.borrow_mut().connect(
            out,
            principal,
            Transport::Socket,
            Features::LABELS,
            None,
            None,
        );
        (client, inbox)
    }

    /// Asks `asker` to disconnect `target`: `Ok` on Done, else the reason.
    async fn disconnect(
        shared: &Shared,
        asker: &mut (ClientId, Inbox),
        target: ClientId,
        req: u64,
    ) -> Result<(), String> {
        State::request(
            shared,
            asker.0,
            Message::Disconnect {
                req,
                client: target,
            },
        );
        match reply_to(&mut asker.1, req).await {
            Message::Done { .. } => Ok(()),
            Message::Failed { reason, .. } => Err(reason),
            other => panic!("{other:?}"),
        }
    }

    /// Arc 1 (D161, D164): on a server several principals share, a client
    /// may disconnect its own principal's connections and those attached to
    /// a session it created; the server's own account may disconnect any.
    #[test]
    fn only_a_session_creator_its_own_principal_or_the_server_disconnects() {
        runtime().block_on(async {
            let (shared, ..) = stalled_reloads();
            shared.borrow_mut().serve_as(test_owner());
            let mut alice = client_of(&shared, 1001);
            let bob = client_of(&shared, 1002);
            let carol = client_of(&shared, 1003);
            let mut carol_too = client_of(&shared, 1003);
            let mut server = client_of(&shared, 1000);

            State::request(
                &shared,
                alice.0,
                Message::CreateSession {
                    req: 1,
                    name: "a".into(),
                },
            );
            assert!(matches!(
                reply_to(&mut alice.1, 1).await,
                Message::Done { .. }
            ));
            {
                let mut state = shared.borrow_mut();
                let session = state.mux.sessions().next().expect("a session").id();
                let window = state.mux.create_window(session, "w").expect("window");
                let size = Size::new(80, 24).expect("size");
                let block = state.mux.create_block(window, size).expect("block");
                state.mux.attach(bob.0, block).expect("attach");
            }

            let refused = disconnect(&shared, &mut alice, carol.0, 2).await;
            assert!(refused.is_err_and(|r| r.contains("another account")));
            assert_eq!(disconnect(&shared, &mut alice, bob.0, 3).await, Ok(()));
            assert_eq!(
                disconnect(&shared, &mut carol_too, carol.0, 4).await,
                Ok(())
            );
            let refused = disconnect(&shared, &mut carol_too, alice.0, 5).await;
            assert!(refused.is_err_and(|r| r.contains("another account")));
            assert_eq!(disconnect(&shared, &mut server, alice.0, 6).await, Ok(()));
        });
    }

    /// A `SHUTDOWN` client of `principal`.
    fn shutdown_client(shared: &Shared, principal: Principal) -> (ClientId, Inbox) {
        let (out, inbox, _) = outbox();
        let client = shared.borrow_mut().connect(
            out,
            principal,
            Transport::Socket,
            Features::SHUTDOWN,
            None,
            None,
        );
        (client, inbox)
    }

    /// C36b: only the server's own account may stop it, once; anyone else
    /// is refused and the stop does not start.
    #[test]
    fn only_the_servers_own_account_stops_it_and_only_once() {
        runtime().block_on(async {
            let (shared, ..) = stalled_reloads();
            shared.borrow_mut().serve_as(test_owner());
            let requested = shared.borrow_mut().stop_on_request();
            let stranger = Principal::unix(1001, Some("other".into())).expect("valid");
            let (other, mut others) = shutdown_client(&shared, stranger);
            let (owner, mut owners) = shutdown_client(&shared, test_owner());

            State::request(&shared, other, Message::Shutdown { req: 1 });
            let refused = reply_to(&mut others, 1).await;
            assert!(
                matches!(&refused, Message::Failed { reason, .. } if reason.contains("only")),
                "{refused:?}"
            );
            assert!(
                shared.borrow().shutdown.is_some(),
                "a refusal starts no stop"
            );

            State::request(&shared, owner, Message::Shutdown { req: 2 });
            assert!(matches!(
                reply_to(&mut owners, 2).await,
                Message::Done { .. }
            ));
            assert_eq!(requested.await, Ok(()), "the owner's request starts it");

            State::request(&shared, owner, Message::Shutdown { req: 3 });
            let again = reply_to(&mut owners, 3).await;
            assert!(matches!(again, Message::Failed { .. }), "{again:?}");
        });
    }

    /// A reload reads off the server's thread: while the file stalls,
    /// another client is answered, its reload is refused rather than
    /// queued, and the stalled one lands, announced, once the read returns.
    #[test]
    fn a_stalled_reload_holds_up_no_other_client_and_only_one_runs() {
        runtime().block_on(async {
            let (shared, client, mut inbox, go) = stalled_reloads();
            let (other, mut others) = reload_client(&shared);
            let pending = State::request(&shared, client, Message::ReloadConfig { req: 1 });
            let Some(Pending::Reload(mut done)) = pending else {
                panic!("the connection must wait for the reload");
            };
            State::request(&shared, other, Message::ReloadConfig { req: 2 });
            let second = reply_to(&mut others, 2).await;
            assert!(
                matches!(&second, Message::Failed { reason, .. } if reason.contains("still reading")),
                "{second:?}"
            );
            State::request(&shared, other, Message::Sync { req: 3 });
            assert!(matches!(reply_to(&mut others, 3).await, Message::Done { .. }));
            assert_eq!(shared.borrow().config.cwd, "/");
            let early = rusty_tokio::time::timeout(Duration::from_millis(50), &mut done).await;
            assert!(early.is_err(), "the reload finished before its read");

            go.send(()).unwrap();
            assert!(matches!(reply_to(&mut inbox, 1).await, Message::Done { .. }));
            done.await.expect("finished");
            let frame = inbox.next().await.expect("the event");
            assert!(matches!(
                rusty_mux_proto::decode(&frame).unwrap().unwrap().0,
                Message::Event {
                    event: Event::ConfigReloaded,
                    ..
                }
            ));
            assert_eq!(shared.borrow().config.cwd, "/tmp");
            assert!(!shared.borrow().reloading);
        });
    }

    /// A read that never returns does not keep the server from stopping:
    /// its thread is never joined.
    #[test]
    fn a_stalled_reload_does_not_hold_up_a_stop() {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            runtime().block_on(async {
                let (shared, client, _inbox, go) = stalled_reloads();
                State::request(&shared, client, Message::ReloadConfig { req: 1 });
                let hosts = shared.borrow_mut().stop();
                assert!(hosts.shut_down(pty::SHUTDOWN).await);
                // Kept alive, so the read never gets its cue.
                std::mem::forget(go);
            });
            done_tx.send(()).unwrap();
        });
        done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the runtime ended while the read stalled");
    }

    #[test]
    fn a_block_being_reaped_refuses_attach_and_may_still_be_closed() {
        runtime().block_on(async {
            let (shared, client, block, mut inbox) = fixture();
            let process = shared.borrow_mut().hang_up(block).expect("program");
            State::request(&shared, client, Message::Attach { req: 7, block });
            let reply = reply_to(&mut inbox, 7).await;
            assert!(
                matches!(&reply, Message::Failed { reason, .. } if reason.contains("closing")),
                "{reply:?}"
            );
            assert_eq!(shared.borrow().mux.client(client).unwrap().attached(), None);

            // A client closes it before the reap ends: no exit is announced.
            State::request(&shared, client, Message::CloseBlock { req: 8, block });
            assert!(shared.borrow().mux.block(block).is_none());
            let before = shared.borrow().history.cursor();
            shared.borrow_mut().exited(block, Some(ExitStatus::Code(0)));
            assert_eq!(shared.borrow().history.cursor(), before);
            drop(process);
        });
    }

    #[test]
    fn an_exit_is_announced_before_the_close() {
        runtime().block_on(async {
            let (shared, _, block, _inbox) = fixture();
            shared.borrow_mut().flush();
            let process = shared.borrow_mut().hang_up(block).expect("program");
            let before = shared.borrow().history.cursor();
            shared.borrow_mut().exited(block, Some(ExitStatus::Code(3)));
            let state = shared.borrow();
            let events: Vec<&Event> = state
                .history
                .since(before)
                .expect("held")
                .map(|(_, e)| e)
                .collect();
            assert_eq!(
                events.first(),
                Some(&&Event::ProgramExited {
                    block,
                    status: ExitStatus::Code(3)
                })
            );
            // Then the close, and the window's layout without the block.
            assert_eq!(events.get(1), Some(&&Event::BlockClosed { block }));
            drop(state);
            drop(process);
        });
    }

    #[test]
    fn a_status_query_is_answered_in_order_with_other_replies() {
        runtime().block_on(async {
            let (shared, client, block, _inbox) = fixture();
            let replies = shared
                .borrow_mut()
                .output(block, b"\x1b]7501;?\x1b\\\x1b[c");
            assert!(
                replies.starts_with(b"\x1b]7501;?\x1b\\\x1b["),
                "{replies:?}"
            );
            assert!(
                shared.borrow().blocks[&block]
                    .status
                    .shown()
                    .bodies()
                    .is_empty()
            );
            State::request(&shared, client, Message::CloseBlock { req: 99, block });
        });
    }

    #[test]
    fn dimensions_fail_before_mutation() {
        runtime().block_on(async {
            let (shared, client, block, _inbox) = fixture();
            let window = shared.borrow().mux.block(block).unwrap().window();
            shared.borrow_mut().mux.attach(client, block).unwrap();
            shared
                .borrow_mut()
                .mux
                .acquire_lease(client, block)
                .unwrap();
            for (cols, rows) in [(4097, 1), (1, 4097), (65535, 65535), (257, 256), (500, 100)] {
                let size = Size::new(cols, rows).unwrap();
                let mut state = shared.borrow_mut();
                assert!(
                    state
                        .create_block(&shared, client, window, size, &Launch::default(), None)
                        .is_err()
                );
                assert!(state.resize(client, block, size).is_err());
                assert_eq!(state.blocks.len(), 1);
                assert_eq!(state.blocks[&block].grid.cols, 80);
                assert_eq!(
                    state.mux.block(block).unwrap().size(),
                    Size::new(80, 24).unwrap()
                );
            }
            shared.borrow_mut().mux.detach(client).unwrap();
            State::request(&shared, client, Message::CloseBlock { req: 99, block });
        });
    }

    #[test]
    fn an_orphan_expires_only_by_its_own_connection() {
        runtime().block_on(async {
            let shared = State::shared(
                Config {
                    program: "/bin/sh".into(),
                    args: vec![],
                    cwd: "/".into(),
                    console_host: ConsoleHost::default(),
                    state_dir: None,
                    grapheme_clustering: false,
                    config_file: None,
                },
                None,
            );
            let mut state = shared.borrow_mut();
            let (out, _inbox, cancel) = outbox();
            let client = state.connect(
                out,
                test_owner(),
                Transport::Socket,
                Features::TAKEOVER,
                None,
                None,
            );
            assert_eq!(
                state.disconnect(client, &cancel),
                Kept::Orphan,
                "kept for a reconnect"
            );
            assert!(state.mux.client(client).is_some());

            // A later connection's expiry is not this one's.
            let (_, _, other) = outbox();
            state.expire(client, &other);
            assert!(state.mux.client(client).is_some());
            state.expire(client, &cancel);
            assert!(
                state.mux.client(client).is_none(),
                "dropped after the grace"
            );

            let (out, _inbox, cancel) = outbox();
            let plain = state.connect(
                out,
                test_owner(),
                Transport::Socket,
                Features::NONE,
                None,
                None,
            );
            assert_eq!(
                state.disconnect(plain, &cancel),
                Kept::Gone,
                "no token, no grace"
            );
            assert!(state.mux.client(plain).is_none());
        });
    }

    #[test]
    fn a_reply_over_the_limit_is_refused_whole() {
        let text = |req, len| Message::Text {
            req,
            text: "x".repeat(len),
        };
        let half = REPLY_MAX / 2;
        assert!(matches!(
            reply("t", &[text(1, half - 64), text(2, half - 64)]),
            Ok(Outcome::Reply(frames)) if frames.len() == 2
        ));
        let refused = reply("t", &[text(1, half), text(2, half)]);
        assert!(matches!(refused, Err(why) if why.reason.contains("exceeds")));
    }

    /// A read-only block whose 2-row grid keeps no scrollback, so every
    /// line that scrolls off spills, as a revived one's does.
    fn spilling_block() -> BlockRt {
        let mut grid = Grid::new(10, 2);
        grid.set_scrollback_max(0);
        grid.spill_evicted(true);
        let started = Started {
            shell: true,
            read_only: true,
            program: "sh".into(),
            args: vec![],
            cwd: "/".into(),
        };
        BlockRt::read_only(grid, AnsiParser::new(), started)
    }

    /// Prints `n` lines into `rt`'s grid, as `output` does.
    fn print(rt: &mut BlockRt, block: BlockId, n: usize) {
        for i in 0..n {
            rt.parser
                .advance(&mut rt.grid, format!("line {i}\r\n").as_bytes());
            rt.spill(block);
        }
    }

    /// A spool directory, gone before anything is written to it.
    fn vanished_spool(name: &str) -> Spool {
        let dir = std::env::temp_dir().join(format!("rusty-mux-{name}-{}", std::process::id()));
        let spool = Spool::open(dir.clone()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        spool
    }

    #[test]
    fn a_spool_that_cannot_open_stops_the_grid_collecting_lines() {
        let block = BlockId::from_uuid(Uuid::nil());
        let mut rt = spilling_block();
        rt.parser.advance(&mut rt.grid, b"a\r\nb\r\nc\r\n");
        rt.keep_history(block, Err(io::Error::other("no room")));
        print(&mut rt, block, 50);
        assert!(rt.spool.is_none());
        assert!(rt.grid.take_spilled().is_empty(), "nothing queued");
        assert_eq!(rt.grid.line_text(0).as_deref(), Some("line 49"));
    }

    #[test]
    fn a_spool_that_fails_to_write_is_dropped_and_collects_nothing_more() {
        let block = BlockId::from_uuid(Uuid::nil());
        // The lines a restore's marker pushed out fail to write.
        let mut rt = spilling_block();
        rt.parser.advance(&mut rt.grid, b"a\r\nb\r\nc\r\n");
        rt.keep_history(block, Ok(vanished_spool("pending")));
        assert!(rt.spool.is_none());
        print(&mut rt, block, 50);
        assert!(rt.grid.take_spilled().is_empty(), "nothing queued");
        // Or later output does.
        let mut rt = spilling_block();
        rt.keep_history(block, Ok(vanished_spool("later")));
        assert!(rt.spool.is_some(), "nothing to write yet");
        print(&mut rt, block, 50);
        assert!(rt.spool.is_none());
        assert!(rt.grid.take_spilled().is_empty(), "nothing queued");
        assert_eq!(rt.grid.line_text(0).as_deref(), Some("line 49"));
    }

    /// Every line `text` leaves a grid `cols` wide, spilled: a 2-row grid
    /// keeping no scrollback, the cursor's row pushed off at the end.
    fn spilled(cols: usize, text: &str) -> Vec<Vec<u8>> {
        let mut grid = Grid::new(cols, 2);
        grid.set_scrollback_max(0);
        grid.spill_evicted(true);
        let mut parser = AnsiParser::new();
        parser.advance(&mut grid, text.as_bytes());
        parser.advance(&mut grid, b"\r\n\r\n");
        grid.take_spilled()
    }

    fn capture(unwrap: bool, trim: bool, vt: bool) -> terminal::Capture {
        terminal::Capture {
            history: u32::MAX,
            trim,
            unwrap,
            vt,
        }
    }

    #[test]
    fn history_unwraps_narrow_rows_beside_a_wide_one_without_padding() {
        // `abcd` wraps into `ef` at 4 columns; an unrelated row is 400 wide.
        let mut read = spilled(4, "\x1b[31mabcdef\x1b[0m");
        read.extend(spilled(400, &"w".repeat(400)));
        let wide = "w".repeat(400);
        let text = |c| format_history(&read, &Grid::new(1, 1), c, REPLY_MAX).unwrap();
        let plain = text(capture(true, true, false));
        assert_eq!(plain.lines().collect::<Vec<_>>(), ["abcdef", wide.as_str()]);
        let rows = text(capture(false, true, false));
        assert_eq!(
            rows.lines().collect::<Vec<_>>(),
            ["abcd", "ef", wide.as_str()]
        );
        let vt = text(capture(true, true, true));
        let first = vt.lines().next().unwrap();
        assert!(
            first.contains("abcdef") && first.contains("\x1b[0m"),
            "{first:?}"
        );
        assert!(!first.contains("  "), "no padding: {first:?}");
        // Without trim the blank rows the cursor left stay.
        let untrimmed = text(capture(true, false, false));
        assert!(untrimmed.starts_with("abcdef\n"), "{untrimmed:?}");
    }

    #[test]
    fn a_long_wrapped_history_run_is_refused_before_it_is_built() {
        // One logical line of 2,000 cells, wrapped every 20.
        let read = spilled(20, &"x".repeat(2_000));
        let fresh = Grid::new(1, 1);
        assert!(format_history(&read, &fresh, capture(true, true, false), 4_000).is_ok());
        let refused = format_history(&read, &fresh, capture(true, true, false), 500).unwrap_err();
        assert!(refused.reason.contains("exceeds 500"), "{}", refused.reason);
        // Row by row, the reply is the same 2,099 bytes.
        assert!(format_history(&read, &fresh, capture(false, true, false), 500).is_err());
        assert!(format_history(&read, &fresh, capture(false, true, false), 2_200).is_ok());
    }

    #[test]
    fn a_styled_wrapped_history_run_is_refused_while_it_is_written() {
        // 200 cells changing colour every cell: few enough cells to pass the
        // run's bound, far too many SGR bytes for the limit.
        let styled: String = (0..200).map(|i| format!("\x1b[3{}mx", 1 + i % 2)).collect();
        let read = spilled(20, &styled);
        let fresh = Grid::new(1, 1);
        let vt = capture(true, true, true);
        assert!(format_history(&read, &fresh, capture(true, true, false), 300).is_ok());
        let refused = format_history(&read, &fresh, vt, 300).unwrap_err();
        assert!(refused.reason.contains("exceeds 300"), "{}", refused.reason);
        assert!(format_history(&read, &fresh, vt, REPLY_MAX).is_ok());
    }

    #[test]
    fn history_shows_the_blocks_themed_default_colours() {
        let mut grid = Grid::new(10, 2);
        grid.set_scrollback_max(0);
        grid.spill_evicted(true);
        let mut parser = AnsiParser::new();
        // OSC 11 themes the default background; the text keeps the default.
        parser.advance(&mut grid, b"\x1b]11;rgb:12/34/56\x1b\\plain\r\n\r\n\r\n");
        let read = grid.take_spilled();
        let vt = capture(false, true, true);
        let themed = format_history(&read, &grid, vt, REPLY_MAX).unwrap();
        assert!(
            themed.starts_with("\x1b[0;48;2;18;52;86mplain"),
            "{themed:?}"
        );
        let fresh = format_history(&read, &Grid::new(1, 1), vt, REPLY_MAX).unwrap();
        assert!(fresh.starts_with("\x1b[0mplain"), "{fresh:?}");
    }

    #[test]
    fn themed_history_reads_as_the_block_showed_it_blanks_included() {
        // OSC 10 and 11 theme the defaults; a themed blank row sits between
        // text rows, and one row has a colour of its own.
        let input: &[u8] = b"\x1b]10;rgb:aa/bb/cc\x1b\\\x1b]11;rgb:12/34/56\x1b\\\
            top\r\n\x1b[2K\r\n\x1b[31mred\x1b[0m\r\nend\r\n\r\n\r\n";
        let mut spilling = Grid::new(10, 2);
        spilling.set_scrollback_max(0);
        spilling.spill_evicted(true);
        let mut twin = Grid::new(10, 2);
        for grid in [&mut spilling, &mut twin] {
            AnsiParser::new().advance(grid, input);
        }
        let read = spilling.take_spilled();
        assert_eq!(read.len(), 5);
        for (trim, vt) in [(false, false), (false, true), (true, false), (true, true)] {
            let c = capture(false, trim, vt);
            let history = format_history(&read, &spilling, c, REPLY_MAX).unwrap();
            // The twin kept the same rows in its scrollback, the screen after.
            let live = terminal::format(&twin, c, REPLY_MAX).unwrap();
            let rows = |t: &str| t.lines().take(4).map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(rows(&history), rows(&live), "trim {trim}, vt {vt}");
        }
        let vt = format_history(&read, &spilling, capture(false, true, true), REPLY_MAX).unwrap();
        let blank = vt.lines().nth(1).unwrap();
        assert!(blank.contains("48;2;18;52;86"), "themed blank kept: {vt:?}");
    }

    #[test]
    fn a_narrow_last_history_row_is_not_widened_under_a_theme() {
        // OSC 10/11 theme the defaults. A 400-column row, a themed blank row
        // 4 wide, then a 4-column row, the last spilled.
        let theme = "\x1b]10;rgb:aa/bb/cc\x1b\\\x1b]11;rgb:12/34/56\x1b\\";
        let mut read = spilled(400, &format!("{theme}{}", "w".repeat(400)));
        read.extend(spilled(4, &format!("{theme}\x1b[2K\r\nab")));
        let mut like = Grid::new(4, 2);
        AnsiParser::new().advance(&mut like, theme.as_bytes());
        for trim in [false, true] {
            let c = capture(false, trim, true);
            let vt = format_history(&read, &like, c, REPLY_MAX).unwrap();
            let rows: Vec<&str> = vt.lines().collect();
            assert_eq!(rows.len(), 3, "trim {trim}: {vt:?}");
            // Each row at its own width: themed blanks are cells, not padding.
            assert_eq!(
                rows[1].matches(' ').count(),
                4,
                "themed blank: {:?}",
                rows[1]
            );
            assert!(rows[2].ends_with("ab  \x1b[0m"), "last row: {:?}", rows[2]);
        }
        let plain = format_history(&read, &like, capture(false, true, false), REPLY_MAX).unwrap();
        assert_eq!(plain.lines().last(), Some("ab"));
    }
}

#[cfg(test)]
mod offered {
    use super::*;

    /// Every feature offered has words for the older-server notice (C36a),
    /// so a new bit can't drop out of it.
    #[test]
    fn every_offered_feature_is_named() {
        for bit in 0..64 {
            let feature = Features::from_bits(1 << bit);
            if OFFERED.contains(feature) {
                assert_eq!(feature.names().len(), 1, "bit {bit} has no name");
            }
        }
    }
}
