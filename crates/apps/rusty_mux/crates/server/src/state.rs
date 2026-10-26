//! The server's state: the model, each block's PTY and grid, each client's queue.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use rusty_mux_core::{BlockId, ClientId, Event, Mux, Size};
use rusty_mux_proto::{Code, Cursor, Entry, Features, Message, encode};
use rusty_term_core::{AnsiParser, Grid};
use rusty_tokio::task::JoinHandle;
use rusty_uuid::Uuid;

use crate::Config;
use crate::checkpoint::validate_size;
use crate::history::History;
use crate::outbox::Outbox;
use crate::pty::{self, Process, Pty};

pub(crate) type Shared = Rc<RefCell<State>>;

/// Bytes read from a PTY per output frame.
const CHUNK: usize = 16 << 10;

/// The largest reply to `capture` or `list`, in encoded bytes: half a
/// client's output budget, so a reply alone never gets a client dropped.
const REPLY_MAX: usize = crate::outbox::BUDGET / 2;

/// Events kept for clients that reconnect.
const HISTORY: usize = 4096;

/// What the server offers; a client gets the intersection with its own.
const OFFERED: Features = Features::CHECKPOINT_V3
    .union(Features::EVENT_SEQ)
    .union(Features::SYNC)
    .union(Features::INSPECT);

/// Input to write to a PTY once the state is no longer borrowed.
pub(crate) type PtyWrite = (Rc<Pty>, Vec<u8>);

struct BlockRt {
    pty: Rc<Pty>,
    process: Process,
    grid: Grid,
    parser: AnsiParser,
    reader: JoinHandle<()>,
}

impl BlockRt {
    /// Stops reading (unless the reader is the caller), closes this handle
    /// on the master, and reaps the program in the background.
    fn release(self, abort_reader: bool) {
        if abort_reader {
            self.reader.abort();
        }
        drop(self.pty);
        rusty_tokio::task::spawn_local(self.process.reap());
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
            let (cursor, missed) = self.missed(resume);
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
    /// `resume` when possible, else from now with nothing to replay.
    fn missed(&self, resume: Option<Cursor>) -> (Cursor, Vec<Message>) {
        let replay = resume.and_then(|from| Some((from, self.history.since(from)?)));
        let Some((from, events)) = replay else {
            return (self.history.cursor(), Vec::new());
        };
        let messages = events
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
                let session = self.mux.create_session(name);
                (req, Ok(Outcome::Done(Some(session.uuid()))))
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
            M::CreateBlock { req, window, size } => (req, self.create_block(shared, window, size)),
            M::Attach { req, block } => {
                if let Err(refusal) = self.require(client, Features::CHECKPOINT_V3, "attach") {
                    return (req, Err(refusal));
                }
                if self.clients.get(&client).is_some_and(|out| out.attaching()) {
                    return (req, Err("checkpoint transfer already in progress".into()));
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
            M::Capture {
                req,
                block,
                history,
            } => (
                req,
                self.require(client, Features::INSPECT, "capture")
                    .and_then(|()| self.capture(req, block, history)),
            ),
            M::List { req } => (
                req,
                self.require(client, Features::INSPECT, "list")
                    .and_then(|()| self.list(req)),
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

    /// The last `history` scrollback lines and the screen, as one Text.
    fn capture(&self, req: u64, block: BlockId, history: u32) -> Result<Outcome, Refusal> {
        let rt = self
            .blocks
            .get(&block)
            .ok_or(rusty_mux_core::Error::UnknownBlock(block))?;
        let scrollback = rt.grid.scrollback_len();
        let from = scrollback.saturating_sub(usize::try_from(history).unwrap_or(usize::MAX));
        let mut text = String::new();
        for abs in from..scrollback + rt.grid.rows {
            if abs > from {
                text.push('\n');
            }
            text.push_str(&rt.grid.line_text(abs).unwrap_or_default());
            // Stop building early; `reply` checks the exact size.
            if text.len() > REPLY_MAX {
                break;
            }
        }
        reply("capture", &[Message::Text { req, text }])
    }

    /// Every session, window and block, in creation order, each parent
    /// before its children.
    fn list(&self, req: u64) -> Result<Outcome, Refusal> {
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
                    });
                }
            }
        }
        let mut messages: Vec<Message> = entries
            .into_iter()
            .map(|entry| Message::Entry { req, entry })
            .collect();
        messages.push(Message::Done { req, created: None });
        reply("list", &messages)
    }

    fn create_block(
        &mut self,
        shared: &Shared,
        window: rusty_mux_core::WindowId,
        size: Size,
    ) -> Result<Outcome, Refusal> {
        validate_size(size)?;
        let session = self
            .mux
            .window(window)
            .ok_or(rusty_mux_core::Error::UnknownWindow(window))?
            .session();
        // The id is chosen first so the program starts knowing it.
        let block = BlockId::from_uuid(Uuid::new_v4());
        let env = self.block_env(session, window, block);
        let (pty, process) = pty::spawn(&self.config, size, &env)
            .map_err(|e| format!("spawning the block's program: {e}"))?;
        let pty = Rc::new(pty);
        self.mux.add_block(block, window, size)?;
        let reader = rusty_tokio::task::spawn_local(pump(shared.clone(), block, pty.clone()));
        let grid = crate::checkpoint::grid(size);
        self.blocks.insert(
            block,
            BlockRt {
                pty,
                process,
                grid,
                parser: AnsiParser::new(),
                reader,
            },
        );
        Ok(Outcome::Done(Some(block.uuid())))
    }

    /// Where a block's program is, for `rusty-mux-ctl` run inside it.
    fn block_env(
        &self,
        session: rusty_mux_core::SessionId,
        window: rusty_mux_core::WindowId,
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

    /// The program exited: close its block.
    fn exited(&mut self, block: BlockId) {
        // Released first, without aborting the reader that is calling this.
        if let Some(rt) = self.blocks.remove(&block) {
            rt.release(false);
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
                    rt.release(true);
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
            E::UnknownSession(_) | E::UnknownWindow(_) | E::UnknownBlock(_) => Some(Code::NotFound),
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
    shared.borrow_mut().exited(block);
}

#[cfg(test)]
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

#[cfg(test)]
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
        let client = state.connect(out, Features::CHECKPOINT_V3, None);
        let session = state.mux.create_session("s");
        let window = state.mux.create_window(session, "w").unwrap();
        let Outcome::Done(Some(id)) = state
            .create_block(&shared, window, Size::new(80, 24).unwrap())
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
                assert!(state.create_block(&shared, window, size).is_err());
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
