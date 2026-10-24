//! The server's state: the model, each block's PTY and grid, each client's queue.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rusty_mux_core::{BlockId, ClientId, Event, Mux, Size};
use rusty_mux_proto::{Features, Message, encode};
use rusty_term_core::{AnsiParser, Grid};
use rusty_tokio::task::JoinHandle;
use rusty_uuid::Uuid;

use crate::Config;
use crate::outbox::Outbox;
use crate::pty::{self, Process, Pty};

pub(crate) type Shared = Rc<RefCell<State>>;

/// Bytes read from a PTY per output frame.
const CHUNK: usize = 16 << 10;

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
    mux: Mux,
    blocks: HashMap<BlockId, BlockRt>,
    clients: HashMap<ClientId, Outbox>,
}

impl State {
    pub(crate) fn shared(config: Config) -> Shared {
        Rc::new(RefCell::new(Self {
            config,
            mux: Mux::new(),
            blocks: HashMap::new(),
            clients: HashMap::new(),
        }))
    }

    pub(crate) fn has_client(&self, client: ClientId) -> bool {
        self.clients.contains_key(&client)
    }

    /// Registers a client and welcomes it. Phase 1 supports no features, so
    /// the intersection is always empty.
    pub(crate) fn connect(&mut self, outbox: Outbox, _theirs: Features) -> ClientId {
        let client = self.mux.connect();
        self.clients.insert(client, outbox);
        self.send(
            client,
            &Message::Welcome {
                features: Features::NONE,
                client,
            },
        );
        self.flush();
        client
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
            Ok(Outcome::Write(write)) => Some(write),
            Err(reason) => {
                this.send(client, &Message::Failed { req, reason });
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
    ) -> (u64, Result<Outcome, String>) {
        use Message as M;
        let done = |r: Result<(), rusty_mux_core::Error>| {
            r.map(|()| Outcome::Done(None)).map_err(|e| e.to_string())
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
                        .map_err(|e| e.to_string()),
                )
            }
            M::CreateBlock { req, window, size } => (req, self.create_block(shared, window, size)),
            M::Attach { req, block } => {
                let result = self.mux.attach(client, block);
                (
                    req,
                    result
                        .map(|()| Outcome::Attached(block))
                        .map_err(|e| e.to_string()),
                )
            }
            M::Detach { req } => (req, done(self.mux.detach(client))),
            M::AcquireLease { req, block } => (req, done(self.mux.acquire_lease(client, block))),
            M::ReleaseLease { req, block } => (req, done(self.mux.release_lease(client, block))),
            M::Input { req, block, data } => (req, self.input(client, block, data)),
            M::Resize { req, block, size } => (req, self.resize(client, block, size)),
            M::CloseBlock { req, block } => (req, done(self.mux.close_block(block))),
            other => (0, Err(format!("not a client request: {other:?}"))),
        }
    }

    fn create_block(
        &mut self,
        shared: &Shared,
        window: rusty_mux_core::WindowId,
        size: Size,
    ) -> Result<Outcome, String> {
        if self.mux.window(window).is_none() {
            return Err(rusty_mux_core::Error::UnknownWindow(window).to_string());
        }
        let (pty, process) = pty::spawn(&self.config, size)
            .map_err(|e| format!("spawning the block's program: {e}"))?;
        let pty = Rc::new(pty);
        let block = self
            .mux
            .create_block(window, size)
            .map_err(|e| e.to_string())?;
        let reader = rusty_tokio::task::spawn_local(pump(shared.clone(), block, pty.clone()));
        let grid = Grid::new(size.cols().into(), size.rows().into());
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

    fn input(
        &mut self,
        client: ClientId,
        block: BlockId,
        data: Vec<u8>,
    ) -> Result<Outcome, String> {
        self.mux
            .authorize_input(client, block)
            .map_err(|e| e.to_string())?;
        let rt = self
            .blocks
            .get(&block)
            .ok_or_else(|| format!("block {block} has no PTY"))?;
        Ok(Outcome::Write((rt.pty.clone(), data)))
    }

    fn resize(&mut self, client: ClientId, block: BlockId, size: Size) -> Result<Outcome, String> {
        self.mux
            .resize_block(client, block, size)
            .map_err(|e| e.to_string())?;
        let rt = self
            .blocks
            .get_mut(&block)
            .ok_or_else(|| format!("block {block} has no PTY"))?;
        rt.grid.resize(size.cols().into(), size.rows().into());
        rt.pty
            .resize(size)
            .map_err(|e| format!("resizing the PTY: {e}"))?;
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
        let data = rt.grid.encode_snapshot();
        self.send(client, &Message::Snapshot { block, data });
        self.send(client, &Message::Ready { block });
    }

    fn send(&mut self, client: ClientId, message: &Message) {
        match encode(message) {
            Ok(frame) => self.send_frame(client, frame),
            Err(err) => {
                eprintln!("rusty-mux-server: encoding a message for client {client}: {err}")
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
                let frame = match encode(&Message::Event(event)) {
                    Ok(frame) => frame,
                    Err(err) => {
                        eprintln!("rusty-mux-server: encoding an event: {err}");
                        continue;
                    }
                };
                let clients: Vec<ClientId> = self.clients.keys().copied().collect();
                clients
                    .into_iter()
                    .for_each(|c| self.send_frame(c, frame.clone()));
            }
        }
    }
}

enum Outcome {
    Done(Option<Uuid>),
    Attached(BlockId),
    Write(PtyWrite),
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
