//! The server's state: the model, each block's PTY and grid, each client's queue.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use rusty_mux_core::{BlockId, ClientId, Event, ExitStatus, Mux, Side, Size, WindowId};
use rusty_mux_proto::schema::{self, CallError, Value};
use rusty_mux_proto::{Code, Cursor, Entry, Features, Launch, Message, encode};
use rusty_mux_status::Records;
use rusty_term_core::{AnsiParser, Grid};
use rusty_tokio::task::JoinHandle;
use rusty_uuid::Uuid;

use crate::Config;
use crate::checkpoint::validate_size;
use crate::history::History;
use crate::keys;
use crate::outbox::Outbox;
use crate::process;
use crate::pty::{self, Process, Pty};
use crate::terminal;

pub(crate) type Shared = Rc<RefCell<State>>;

/// Bytes read from a PTY per output frame.
const CHUNK: usize = 16 << 10;

/// The largest reply to a block call or `list`, in encoded bytes: half a
/// client's output budget, so a reply alone never gets a client dropped.
const REPLY_MAX: usize = crate::outbox::BUDGET / 2;

/// Events kept for clients that reconnect.
const HISTORY: usize = 4096;

/// How often every block's foreground process group is checked, to see a
/// job start that writes nothing; output checks its own block at once.
const PROCESS_POLL: Duration = Duration::from_millis(500);

/// What the server offers; a client gets the intersection with its own.
const OFFERED: Features = Features::CHECKPOINT_V5
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
    .union(Features::LAYOUT);

/// Input to write to a PTY once the state is no longer borrowed.
pub(crate) type PtyWrite = (Rc<Pty>, Vec<u8>);

struct BlockRt {
    pty: Rc<Pty>,
    process: Process,
    grid: Grid,
    parser: AnsiParser,
    /// What the program reported with OSC 7501.
    status: Records,
    /// The working directory and title last announced.
    seen: terminal::Seen,
    /// The foreground process group last announced, at first the program's;
    /// `None` where the platform tells none.
    foreground: Option<process::Foreground>,
    reader: JoinHandle<()>,
}

impl BlockRt {
    /// Stops reading (unless the reader is the caller), releases the PTY's
    /// output whatever other handles remain and closes this handle on the
    /// master. The program, for the caller to reap.
    fn release(self, abort_reader: bool) -> Process {
        if abort_reader {
            self.reader.abort();
        }
        self.pty.release();
        drop(self.pty);
        self.process
    }
}

pub(crate) struct State {
    config: Config,
    /// The listening socket's path, given to each block's program.
    socket: Option<PathBuf>,
    mux: Mux,
    blocks: HashMap<BlockId, BlockRt>,
    clients: HashMap<ClientId, Outbox>,
    features: HashMap<ClientId, Features>,
    history: History,
}

impl State {
    pub(crate) fn shared(config: Config, socket: Option<PathBuf>) -> Shared {
        Rc::new(RefCell::new(Self {
            config,
            socket,
            mux: Mux::new(),
            blocks: HashMap::new(),
            clients: HashMap::new(),
            features: HashMap::new(),
            history: History::new(Uuid::new_v4(), HISTORY),
        }))
    }

    pub(crate) fn has_client(&self, client: ClientId) -> bool {
        self.clients.contains_key(&client)
    }

    /// Registers a client, negotiates features and, with `EVENT_SEQ`,
    /// replays the events after `resume` when they are all still held.
    pub(crate) fn connect(
        &mut self,
        outbox: Outbox,
        theirs: Features,
        resume: Option<Cursor>,
    ) -> ClientId {
        let client = self.mux.connect();
        self.clients.insert(client, outbox);
        let features = theirs.intersection(OFFERED);
        self.features.insert(client, features);
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
            },
        );
        missed.iter().for_each(|m| self.send(client, m));
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

    /// Forgets a client whose connection ended. A client the server already
    /// dropped for exceeding its budget is gone from both maps.
    pub(crate) fn disconnect(&mut self, client: ClientId) {
        self.drop_client(client);
        self.flush();
    }

    fn drop_client(&mut self, client: ClientId) {
        if self.clients.remove(&client).is_none() {
            return;
        }
        self.features.remove(&client);
        if let Err(err) = self.mux.disconnect(client) {
            eprintln!("rusty-mux-server: model out of step on disconnect: {err}");
        }
    }

    /// Applies one request. Returns input for the caller to write to a PTY
    /// after releasing the borrow.
    pub(crate) fn request(shared: &Shared, client: ClientId, message: Message) -> Option<PtyWrite> {
        let mut this = shared.borrow_mut();
        let (req, outcome) = this.apply(shared, client, message);
        let write = match outcome {
            Ok(Outcome::Done(created)) => {
                this.send(client, &Message::Done { req, created });
                None
            }
            Ok(Outcome::Attached(block)) => {
                this.send(client, &Message::Done { req, created: None });
                this.send_snapshot(client, block);
                None
            }
            Ok(Outcome::Reply(frames)) => {
                frames.into_iter().for_each(|f| this.send_frame(client, f));
                None
            }
            Ok(Outcome::Write(write)) => Some(write),
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
                let result = self.mux.create_session(name);
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
                if launch != Launch::default()
                    && let Err(refusal) = self.require(client, Features::LAUNCH, "a launch")
                {
                    return (req, Err(refusal));
                }
                (req, self.create_block(shared, window, size, &launch, None))
            }
            M::Split {
                req,
                block,
                side,
                launch,
            } => {
                let launched = match launch == Launch::default() {
                    true => Ok(()),
                    false => self.require(client, Features::LAUNCH, "a launch"),
                };
                (
                    req,
                    self.require(client, Features::LAYOUT, "split")
                        .and(launched)
                        .and_then(|()| self.split(shared, block, side, &launch)),
                )
            }
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
            M::Attach { req, block } => {
                if let Err(refusal) = self.require(client, Features::CHECKPOINT_V5, "attach") {
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
            M::Detach { req } => (req, done(self.mux.detach(client))),
            M::AcquireLease { req, block } => (req, done(self.mux.acquire_lease(client, block))),
            M::ReleaseLease { req, block } => (req, done(self.mux.release_lease(client, block))),
            M::Input { req, block, data } => (req, self.input(client, block, data)),
            M::Resize { req, block, size } => (req, self.resize(client, block, size)),
            M::CloseBlock { req, block } => (req, done(self.mux.close_block(block))),
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
            M::Rename { req, target, name } => (
                req,
                self.require(client, Features::LABELS, "rename")
                    .and_then(|()| done(self.mux.rename(target, name))),
            ),
            M::Disconnect {
                req,
                client: target,
            } => (
                req,
                self.require(client, Features::LABELS, "disconnect")
                    .and_then(|()| self.disconnect_other(client, target)),
            ),
            other => (0, Err(format!("not a client request: {other:?}").into())),
        }
    }

    /// Fails unless `client` negotiated `feature`, which `what` needs.
    fn require(&self, client: ClientId, feature: Features, what: &str) -> Result<(), Refusal> {
        if self
            .features
            .get(&client)
            .is_some_and(|f| f.contains(feature))
        {
            return Ok(());
        }
        Err(format!("{what} requires feature bits {:#x}", feature.bits()).into())
    }

    /// Runs block method `method` on `block`, after checking its arguments
    /// against the catalog.
    fn block_call(
        &self,
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
                return Ok(Outcome::Write((rt.pty.clone(), data)));
            }
            "terminal.pwd" => terminal::pwd(&rt.grid, rt.process.pid())
                .ok_or("the program's working directory is unknown")?,
            "terminal.title" => rt.grid.title.clone(),
            "terminal.status" => rt.status.bodies().join("\n"),
            "terminal.process" => {
                let pid = rt.process.pid().ok_or("the program has exited")?;
                process::text(&process::chain(pid, rt.pty.foreground(), process::lookup))
            }
            _ => return Err(unknown()),
        };
        reply(method, &[Message::Text { req, text }])
    }

    /// Ends `target`'s connection for `client`, as the budget does a slow
    /// client's. Any client may end any other's: everyone who can reach
    /// the socket is trusted alike. A client cannot end its own, since its
    /// reply would be lost with the connection.
    fn disconnect_other(&mut self, client: ClientId, target: ClientId) -> Result<Outcome, Refusal> {
        if target == client {
            return Err("a client cannot disconnect itself; close the connection instead".into());
        }
        if !self.clients.contains_key(&target) {
            return Err(rusty_mux_core::Error::UnknownClient(target).into());
        }
        self.drop_client(target);
        Ok(Outcome::Done(None))
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
            });
            for window in session.windows().iter().filter_map(|w| self.mux.window(*w)) {
                entries.push(Entry::Window {
                    window: window.id(),
                    session: session.id(),
                    name: window.name().to_owned(),
                    layout: window.layout().cloned(),
                    zoomed: window.zoomed(),
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
                    });
                }
            }
        }
        if self
            .features
            .get(&client)
            .is_some_and(|f| f.contains(Features::LABELS))
        {
            entries.extend(self.mux.clients().map(|c| Entry::Client {
                client: c.id(),
                name: c.name().to_owned(),
                attached: c.attached(),
            }));
        }
        let mut messages: Vec<Message> = entries
            .into_iter()
            .map(|entry| Message::Entry { req, entry })
            .collect();
        messages.push(Message::Done { req, created: None });
        reply("list", &messages)
    }

    /// Starts a block beside `target`, sized as its part of `target`.
    fn split(
        &mut self,
        shared: &Shared,
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
        self.create_block(shared, window, size, launch, Some((target, side)))
    }

    /// Starts a block in `window`, placed by the model, or beside the
    /// block `beside` names.
    fn create_block(
        &mut self,
        shared: &Shared,
        window: WindowId,
        size: Size,
        launch: &Launch,
        beside: Option<(BlockId, Side)>,
    ) -> Result<Outcome, Refusal> {
        validate_size(size)?;
        let config = launched(&self.config, launch)?;
        let session = self
            .mux
            .window(window)
            .ok_or(rusty_mux_core::Error::UnknownWindow(window))?
            .session();
        // The id is chosen first so the program starts knowing it.
        let block = BlockId::from_uuid(Uuid::new_v4());
        let env = self.block_env(session, window, block);
        let (pty, process) = pty::spawn(&config, size, &env)
            .map_err(|e| format!("spawning the block's program: {e}"))?;
        let pty = Rc::new(pty);
        // Named after the program as started, so a block does not announce
        // its own program once it has exec'd.
        let foreground = process.pid().map(|pid| process::Foreground {
            pid,
            name: std::path::Path::new(&config.program)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        });
        match beside {
            None => self.mux.add_block(block, window, size)?,
            Some((target, side)) => self.mux.split_block(block, target, side, size)?,
        }
        let reader = rusty_tokio::task::spawn_local(pump(shared.clone(), block, pty.clone()));
        let mut grid = crate::checkpoint::grid(size);
        grid.collect_program_status = true;
        self.blocks.insert(
            block,
            BlockRt {
                pty,
                process,
                grid,
                parser: AnsiParser::new(),
                status: Records::default(),
                seen: terminal::Seen::default(),
                foreground,
                reader,
            },
        );
        Ok(Outcome::Done(Some(block.uuid())))
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

    fn input(
        &mut self,
        client: ClientId,
        block: BlockId,
        data: Vec<u8>,
    ) -> Result<Outcome, Refusal> {
        self.mux
            .authorize_input(client, block)
            .map_err(Refusal::from)?;
        let rt = self
            .blocks
            .get(&block)
            .ok_or_else(|| format!("block {block} has no PTY"))?;
        Ok(Outcome::Write((rt.pty.clone(), data)))
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
        rt.pty
            .resize(size)
            .map_err(|e| format!("resizing the PTY: {e}"))?;
        self.mux
            .resize_block(client, block, size)
            .map_err(Refusal::from)?;
        let rt = self
            .blocks
            .get_mut(&block)
            .ok_or_else(|| format!("block {block} has no PTY"))?;
        rt.grid.resize(size.cols().into(), size.rows().into());
        Ok(Outcome::Done(None))
    }

    /// Feeds PTY output to the block's grid and tees it to attached clients.
    /// Returns the terminal's replies to queries, for the PTY.
    fn output(&mut self, block: BlockId, data: &[u8]) -> Vec<u8> {
        let Some(rt) = self.blocks.get_mut(&block) else {
            return Vec::new();
        };
        rt.parser.advance(&mut rt.grid, data);
        let replies = rt.parser.take_responses();
        if terminal::absorb_status(&mut rt.grid, &mut rt.status) {
            in_step("status", self.mux.note_status(block));
        }
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
        // What a terminal host would relay, such as the bell; clients
        // already have the raw output.
        rt.grid.take_host_out();
        self.check_process(block, false);
        let frame = encode(&Message::Output {
            block,
            data: data.to_vec(),
        });
        let attached: Vec<ClientId> = self
            .clients
            .keys()
            .copied()
            .filter(|&c| self.mux.client(c).and_then(|c| c.attached()) == Some(block))
            .collect();
        match frame {
            Ok(frame) => attached
                .into_iter()
                .for_each(|c| self.send_frame(c, frame.clone())),
            Err(err) => eprintln!("rusty-mux-server: encoding output of block {block}: {err}"),
        }
        self.flush();
        replies
    }

    /// Announces the foreground process group of `block` if another took
    /// its terminal since the last check, or, on a `refresh`, if its
    /// program's name changed ([`process::Foreground::observe`]).
    fn check_process(&mut self, block: BlockId, refresh: bool) {
        let Some(rt) = self.blocks.get_mut(&block) else {
            return;
        };
        let (Some(pid), Some(seen)) = (rt.pty.foreground(), rt.foreground.as_mut()) else {
            return;
        };
        if !seen.observe(pid, refresh, process::lookup) {
            return;
        }
        let name = seen.name.clone();
        in_step("process", self.mux.note_process(block, pid, name));
    }

    /// [`Self::check_process`] for every block, names refreshed, then
    /// broadcasts.
    fn check_processes(&mut self) {
        let blocks: Vec<BlockId> = self.blocks.keys().copied().collect();
        blocks.into_iter().for_each(|b| self.check_process(b, true));
        self.flush();
    }

    /// The block's output ended: stop serving its PTY, without aborting the
    /// reader that is calling this. Its program, to reap; `None` if a
    /// client closed the block first.
    fn hang_up(&mut self, block: BlockId) -> Option<Process> {
        self.blocks.remove(&block).map(|rt| rt.release(false))
    }

    /// The block's program ended: announce how, if known, and close the
    /// block, unless a client closed it while the program was reaped.
    fn exited(&mut self, block: BlockId, status: Option<ExitStatus>) {
        if let Some(status) = status
            && self.mux.note_exit(block, status).is_err()
        {
            return;
        }
        if self.mux.close_block(block).is_ok() {
            self.flush();
        }
    }

    fn send_snapshot(&mut self, client: ClientId, block: BlockId) {
        let Some(rt) = self.blocks.get(&block) else {
            return;
        };
        let data = match rusty_term_core::checkpoint::encode(&rt.grid, &rt.parser) {
            Ok(data) => data,
            Err(err) => {
                eprintln!("rusty-mux-server: encoding checkpoint: {err}");
                self.drop_client(client);
                return;
            }
        };
        if !self
            .clients
            .get(&client)
            .is_some_and(|out| out.checkpoint(block, data))
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
            eprintln!("rusty-mux-server: dropping client {client}: output queue over budget");
            self.drop_client(client);
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
                if let Event::BlockClosed { block } = event
                    && let Some(rt) = self.blocks.remove(&block)
                {
                    rusty_tokio::task::spawn_local(rt.release(true).reap());
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
        _ => Features::NONE,
    };
    features.contains(needs)
}

/// Reports a model call that fails only if the server's state and the
/// model's disagree.
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
        history: named
            .iter()
            .find_map(|(name, value)| match (*name, value) {
                ("history", Value::Lines(n)) => Some(*n),
                _ => None,
            })
            .unwrap_or_default(),
        trim: flag("trim"),
        unwrap: flag("unwrap"),
        vt: flag("vt"),
    }
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

/// Checks every block's foreground process group each [`PROCESS_POLL`],
/// until the server's state is dropped. Holding it weakly, the poll never
/// keeps a stopped server's state alive.
pub(crate) async fn watch_processes(state: Weak<RefCell<State>>) {
    loop {
        rusty_tokio::time::sleep(PROCESS_POLL).await;
        let Some(shared) = state.upgrade() else {
            return;
        };
        shared.borrow_mut().check_processes();
    }
}

/// Reads a block's PTY until its program exits.
async fn pump(shared: Shared, block: BlockId, pty: Rc<Pty>) {
    let mut buf = vec![0; CHUNK];
    loop {
        let n = match pty.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) => {
                eprintln!("rusty-mux-server: reading block {block}: {err}");
                break;
            }
        };
        let replies = shared.borrow_mut().output(block, &buf[..n]);
        if !replies.is_empty()
            && let Err(err) = pty.write_all(&replies).await
        {
            eprintln!("rusty-mux-server: answering block {block}: {err}");
            break;
        }
        // A flooding PTY is always readable; yield so input and client
        // writes get their turn between chunks.
        rusty_tokio::task::yield_now().await;
    }
    let process = shared.borrow_mut().hang_up(block);
    let status = match process {
        Some(process) => process.reap().await,
        None => None,
    };
    shared.borrow_mut().exited(block, status);
}

#[cfg(all(test, unix))]
impl State {
    pub(crate) fn test_client(&self) -> ClientId {
        *self.clients.keys().next().unwrap()
    }
    pub(crate) fn test_checkpoint(&self, client: ClientId, data: Vec<u8>) {
        assert!(self.clients[&client].checkpoint(BlockId::from_uuid(Uuid::nil()), data));
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
        let shared = State::shared(
            Config {
                program: "/bin/sh".into(),
                args: vec![],
                cwd: "/".into(),
            },
            None,
        );
        let (out, inbox, _) = outbox();
        let mut state = shared.borrow_mut();
        let client = state.connect(out, Features::CHECKPOINT_V5, None);
        let session = state.mux.create_session("s").unwrap();
        let window = state.mux.create_window(session, "w").unwrap();
        let Outcome::Done(Some(id)) = state
            .create_block(
                &shared,
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
            assert!(shared.borrow().blocks[&block].status.bodies().is_empty());
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
                        .create_block(&shared, window, size, &Launch::default(), None)
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
}
