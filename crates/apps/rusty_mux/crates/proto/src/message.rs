//! The messages and their field layouts.

use rusty_mux_core::{
    BlockId, ClientId, EntityId, Event, ExitStatus, Progress, SessionId, Size, WindowId,
};
use rusty_uuid::Uuid;
use rusty_wire::{Reader, Writer};

use crate::field::{In, Out};
use crate::schema::Value;
use crate::{Error, args, varint};

/// Optional protocol capabilities. Each peer sends its own; the session uses
/// the intersection. Continuation checkpoints require explicit opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Features(u64);

impl Features {
    pub const NONE: Self = Self(0);

    /// Reserved: rejected experimental RMRP replay capability.
    pub const REPLAY_SNAPSHOT: Self = Self(1);

    /// Reserved: incomplete RTCP v1 checkpoint capability.
    pub const CHECKPOINT_V1: Self = Self(1 << 1);

    /// Reserved: superseded fixed-width RTCP v2 checkpoint capability.
    pub const CHECKPOINT_V2: Self = Self(1 << 2);

    /// RTCP v3 (run-encoded) headless checkpoints transferred in ordered
    /// Checkpoint chunks.
    pub const CHECKPOINT_V3: Self = Self(1 << 3);

    /// Numbered events and resuming the stream after a reconnect: a
    /// [`Cursor`] in Hello and Welcome, and `seq` on every event.
    pub const EVENT_SEQ: Self = Self(1 << 4);

    /// [`Message::Sync`], a request that changes nothing.
    pub const SYNC: Self = Self(1 << 5);

    /// [`Message::List`], which reads the server's state without changing
    /// it.
    pub const INSPECT: Self = Self(1 << 6);
    /// [`Message::BlockCall`], answered with [`Message::Text`], or, for a
    /// method that writes to the block, only if it fails.
    pub const BLOCK_CALL: Self = Self(1 << 7);
    /// [`Event::BlockStatus`] events. A client without it never receives
    /// one, so its event numbers skip them.
    pub const STATUS: Self = Self(1 << 8);
    /// [`Event::PwdChanged`] and [`Event::TitleChanged`] events, likewise
    /// skipped for a client without it.
    pub const TERMINAL_EVENTS: Self = Self(1 << 9);
    /// [`Event::ProgramExited`], [`Event::Bell`], [`Event::ProgressChanged`]
    /// and [`Event::Notification`] events, likewise skipped for a client
    /// without it.
    pub const PROGRAM_EVENTS: Self = Self(1 << 10);
    /// [`Message::CreateBlock`]'s [`Launch`]: the program and working
    /// directory of the new block. A server without it ignores them and
    /// starts its default program, so a client must not send them then.
    pub const LAUNCH: Self = Self(1 << 11);
    /// [`Event::ProcessChanged`] events, skipped for a client without it
    /// as the other optional kinds are.
    pub const PROCESS: Self = Self(1 << 12);
    /// `terminal.format`'s `trim`, `unwrap` and `vt` options. A server
    /// without it refuses them as unknown parameters.
    pub const CAPTURE: Self = Self(1 << 13);
    /// Names for every kind: [`Message::Rename`] and the `*Renamed`
    /// events (skipped for a client without it); clients in the reply to
    /// [`Message::List`]; and [`Message::Disconnect`].
    pub const LABELS: Self = Self(1 << 14);

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The features both peers support.
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// A position in one server run's event stream: the run's `epoch` and the
/// `seq` of the last event delivered. Sequence numbers start at 1 in each
/// run, so `seq` 0 is the position before the first event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub epoch: Uuid,
    pub seq: u64,
}

/// What kind of failure a [`Message::Failed`] reports, for callers that act
/// on it. A code this peer does not know decodes as no code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    /// A session, window or block the request names does not exist.
    NotFound,
    /// The block's type has no method of the requested name.
    MethodNotFound,
}

/// What a new block runs: the server's default program unless `program`
/// names one, in the server's default directory unless `cwd` names one.
/// Negotiated with [`Features::LAUNCH`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Launch {
    pub program: Option<Program>,
    /// An absolute path; the server refuses a relative one.
    pub cwd: Option<String>,
}

/// A program to run and its arguments, without the program's own name.
/// The server looks a bare `command` up in its `PATH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub command: String,
    pub args: Vec<String>,
}

/// One entity in the reply to [`Message::List`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Session {
        session: SessionId,
        name: String,
    },
    Window {
        window: WindowId,
        session: SessionId,
        name: String,
    },
    Block {
        block: BlockId,
        window: WindowId,
        size: Size,
        /// The input lease's holder, if anyone holds it.
        holder: Option<ClientId>,
        /// The title the program last set; empty if it never set one.
        title: String,
        /// Empty until a client names the block.
        name: String,
    },
    /// Sent only with [`Features::LABELS`].
    Client {
        client: ClientId,
        name: String,
        /// The block whose output it receives, if any.
        attached: Option<BlockId>,
    },
}

/// One frame's content.
///
/// Requests from a client carry a `req` number that the server echoes in
/// [`Message::Done`] or [`Message::Failed`]. [`Message::Input`] is answered
/// only when it fails, so typing costs no reply traffic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    // Client to server.
    Hello {
        features: Features,
        /// With [`Features::EVENT_SEQ`]: the last event this client saw
        /// before reconnecting, to continue from there.
        resume: Option<Cursor>,
    },
    CreateSession {
        req: u64,
        name: String,
    },
    CreateWindow {
        req: u64,
        session: SessionId,
        name: String,
    },
    CreateBlock {
        req: u64,
        window: WindowId,
        size: Size,
        launch: Launch,
    },
    Attach {
        req: u64,
        block: BlockId,
    },
    Detach {
        req: u64,
    },
    AcquireLease {
        req: u64,
        block: BlockId,
    },
    ReleaseLease {
        req: u64,
        block: BlockId,
    },
    Input {
        req: u64,
        block: BlockId,
        data: Vec<u8>,
    },
    Resize {
        req: u64,
        block: BlockId,
        size: Size,
    },
    CloseBlock {
        req: u64,
        block: BlockId,
    },
    /// Changes nothing; its reply means every earlier request on this
    /// connection has been handled. Negotiated with [`Features::SYNC`].
    Sync {
        req: u64,
    },
    /// Every session, window and block, then, with [`Features::LABELS`],
    /// every client. Answered with one [`Message::Entry`] per entity, each
    /// parent before its children, then [`Message::Done`]. Negotiated with [`Features::INSPECT`].
    List {
        req: u64,
    },
    /// Runs `method`, one of the methods of the block's type such as
    /// `terminal.format`, with named arguments. Answered with
    /// [`Message::Text`], nothing for a method that writes to the block
    /// (as [`Message::Input`] is), or [`Message::Failed`] with
    /// [`Code::MethodNotFound`] if the type has no such method. Negotiated
    /// with [`Features::BLOCK_CALL`]; [`crate::schema`] lists the methods.
    /// At most [`crate::args::MAX_ARGS`] arguments decode.
    BlockCall {
        req: u64,
        block: BlockId,
        method: String,
        args: Vec<(String, Value)>,
    },
    /// Names any entity, replacing its old name. Negotiated with
    /// [`Features::LABELS`].
    Rename {
        req: u64,
        target: EntityId,
        name: String,
    },
    /// Ends another client's connection, as if it had closed it: it is
    /// detached and loses its leases. It may connect again. Negotiated
    /// with [`Features::LABELS`].
    Disconnect {
        req: u64,
        client: ClientId,
    },
    // Server to client.
    /// With [`Features::EVENT_SEQ`], `cursor` is where this client's event
    /// stream starts: events after it follow, the replayed ones first. It
    /// equals the Hello's `resume` when the server resumed the stream;
    /// otherwise events in between are lost and the client starts afresh.
    Welcome {
        features: Features,
        client: ClientId,
        cursor: Option<Cursor>,
    },
    /// The request succeeded; `created` is the new entity's id, if it made one.
    Done {
        req: u64,
        created: Option<Uuid>,
    },
    Failed {
        req: u64,
        reason: String,
        code: Option<Code>,
    },
    /// The reply to [`Message::BlockCall`].
    Text {
        req: u64,
        text: String,
    },
    /// One entity of the reply to [`Message::List`].
    Entry {
        req: u64,
        entry: Entry,
    },
    /// A model change; `seq` is its number with [`Features::EVENT_SEQ`].
    Event {
        seq: Option<u64>,
        event: Event,
    },
    /// Legacy drawable snapshot; not a raw-output continuation checkpoint.
    /// The Phase 1 server uses negotiated Checkpoint chunks for attach.
    Snapshot {
        block: BlockId,
        data: Vec<u8>,
    },
    /// A chunk of RTCP v3, negotiated with CHECKPOINT_V3. Offsets must be
    /// contiguous from zero and total is constant throughout one transfer.
    Checkpoint {
        block: BlockId,
        offset: u64,
        total: u64,
        data: Vec<u8>,
    },
    /// The snapshot is complete; raw output follows.
    Ready {
        block: BlockId,
    },
    /// Raw PTY bytes, teed unparsed to every attached client.
    Output {
        block: BlockId,
        data: Vec<u8>,
    },
}

// Field ids share one space across messages, so a field means the same thing everywhere.
const REQ: u32 = 1;
const SESSION: u32 = 2;
const WINDOW: u32 = 3;
const BLOCK: u32 = 4;
const CLIENT: u32 = 5;
const NAME: u32 = 6;
const COLS: u32 = 7;
const ROWS: u32 = 8;
const DATA: u32 = 9;
const FEATURES: u32 = 10;
const CREATED: u32 = 11;
const REASON: u32 = 12;
const KIND: u32 = 13;
const OFFSET: u32 = 15;
const TOTAL: u32 = 16;
const HOLDER: u32 = 14;
const EPOCH: u32 = 17;
const SEQ: u32 = 18;
// 19 was Capture's history, retired with it.
const TITLE: u32 = 20;
const CODE: u32 = 21;
const METHOD: u32 = 22;
const ARGS: u32 = 23;
/// A program's exit code, its two's-complement bits.
const EXIT: u32 = 24;
const SIGNAL: u32 = 25;
/// A progress state, numbered as OSC 9;4 numbers them.
const PROGRESS: u32 = 26;
const PERCENT: u32 = 27;
const BODY: u32 = 28;
const COMMAND: u32 = 29;
/// A program's arguments: each a varint length, then its UTF-8 bytes.
const ARGV: u32 = 30;
const CWD: u32 = 31;
/// A process id.
const PID: u32 = 32;
// IDs 1..=PID are recognized. Extend this bound when adding field IDs.
pub(crate) const FIELD_COUNT: usize = PID as usize;

mod ty {
    pub const HELLO: u8 = 0x01;
    pub const CREATE_SESSION: u8 = 0x02;
    pub const CREATE_WINDOW: u8 = 0x03;
    pub const CREATE_BLOCK: u8 = 0x04;
    pub const ATTACH: u8 = 0x05;
    pub const DETACH: u8 = 0x06;
    pub const ACQUIRE_LEASE: u8 = 0x07;
    pub const RELEASE_LEASE: u8 = 0x08;
    pub const INPUT: u8 = 0x09;
    pub const RESIZE: u8 = 0x0a;
    pub const CLOSE_BLOCK: u8 = 0x0b;
    pub const SYNC: u8 = 0x0c;
    // 0x0d was Capture, replaced by BlockCall's `terminal.format`.
    pub const LIST: u8 = 0x0e;
    pub const BLOCK_CALL: u8 = 0x0f;
    pub const RENAME: u8 = 0x10;
    pub const DISCONNECT: u8 = 0x11;
    pub const WELCOME: u8 = 0x81;
    pub const DONE: u8 = 0x82;
    pub const FAILED: u8 = 0x83;
    pub const EVENT: u8 = 0x84;
    pub const SNAPSHOT: u8 = 0x85;
    pub const READY: u8 = 0x86;
    pub const OUTPUT: u8 = 0x87;
    pub const CHECKPOINT: u8 = 0x88;
    pub const TEXT: u8 = 0x89;
    pub const ENTRY: u8 = 0x8a;
}

impl Message {
    pub(crate) fn encode_body(&self, out: &mut Out) -> u8 {
        use Message as M;
        match self {
            M::Hello { features, resume } => {
                put_cursor(out.varint(FEATURES, features.bits()), *resume);
                ty::HELLO
            }
            M::CreateSession { req, name } => {
                out.varint(REQ, *req).bytes(NAME, name.as_bytes());
                ty::CREATE_SESSION
            }
            M::CreateWindow { req, session, name } => {
                out.varint(REQ, *req)
                    .uuid(SESSION, session.uuid())
                    .bytes(NAME, name.as_bytes());
                ty::CREATE_WINDOW
            }
            M::CreateBlock {
                req,
                window,
                size,
                launch,
            } => {
                let out = out.varint(REQ, *req).uuid(WINDOW, window.uuid());
                put_size(out, *size);
                put_launch(out, launch);
                ty::CREATE_BLOCK
            }
            M::Attach { req, block } => req_block(out, *req, *block, ty::ATTACH),
            M::Detach { req } => {
                out.varint(REQ, *req);
                ty::DETACH
            }
            M::AcquireLease { req, block } => req_block(out, *req, *block, ty::ACQUIRE_LEASE),
            M::ReleaseLease { req, block } => req_block(out, *req, *block, ty::RELEASE_LEASE),
            M::Input { req, block, data } => {
                out.varint(REQ, *req)
                    .uuid(BLOCK, block.uuid())
                    .bytes(DATA, data);
                ty::INPUT
            }
            M::Resize { req, block, size } => {
                put_size(out.varint(REQ, *req).uuid(BLOCK, block.uuid()), *size);
                ty::RESIZE
            }
            M::CloseBlock { req, block } => req_block(out, *req, *block, ty::CLOSE_BLOCK),
            M::Sync { req } => {
                out.varint(REQ, *req);
                ty::SYNC
            }
            M::List { req } => {
                out.varint(REQ, *req);
                ty::LIST
            }
            M::BlockCall {
                req,
                block,
                method,
                args,
            } => {
                out.varint(REQ, *req)
                    .uuid(BLOCK, block.uuid())
                    .bytes(METHOD, method.as_bytes())
                    .bytes(ARGS, &args::encode(args));
                ty::BLOCK_CALL
            }
            M::Rename { req, target, name } => {
                put_target(out.varint(REQ, *req), *target).bytes(NAME, name.as_bytes());
                ty::RENAME
            }
            M::Disconnect { req, client } => {
                out.varint(REQ, *req).uuid(CLIENT, client.uuid());
                ty::DISCONNECT
            }
            M::Welcome {
                features,
                client,
                cursor,
            } => {
                let out = out
                    .varint(FEATURES, features.bits())
                    .uuid(CLIENT, client.uuid());
                put_cursor(out, *cursor);
                ty::WELCOME
            }
            M::Done { req, created } => {
                out.varint(REQ, *req);
                if let Some(id) = created {
                    out.uuid(CREATED, *id);
                }
                ty::DONE
            }
            M::Failed { req, reason, code } => {
                out.varint(REQ, *req).bytes(REASON, reason.as_bytes());
                if let Some(code) = code {
                    out.varint(CODE, code.wire());
                }
                ty::FAILED
            }
            M::Text { req, text } => {
                out.varint(REQ, *req).bytes(DATA, text.as_bytes());
                ty::TEXT
            }
            M::Entry { req, entry } => {
                put_entry(out.varint(REQ, *req), entry);
                ty::ENTRY
            }
            M::Event { seq, event } => {
                if let Some(seq) = seq {
                    out.varint(SEQ, *seq);
                }
                put_event(out, event);
                ty::EVENT
            }
            M::Snapshot { block, data } => {
                out.uuid(BLOCK, block.uuid()).bytes(DATA, data);
                ty::SNAPSHOT
            }
            M::Checkpoint {
                block,
                offset,
                total,
                data,
            } => {
                out.uuid(BLOCK, block.uuid())
                    .varint(OFFSET, *offset)
                    .varint(TOTAL, *total)
                    .bytes(DATA, data);
                ty::CHECKPOINT
            }
            M::Ready { block } => {
                out.uuid(BLOCK, block.uuid());
                ty::READY
            }
            M::Output { block, data } => {
                out.uuid(BLOCK, block.uuid()).bytes(DATA, data);
                ty::OUTPUT
            }
        }
    }

    pub(crate) fn decode_body(message: u8, body: &[u8]) -> Result<Self, Error> {
        use Message as M;
        let f = In::parse(message, body)?;
        Ok(match message {
            ty::HELLO => M::Hello {
                features: Features(f.varint(FEATURES)?),
                resume: get_cursor(&f)?,
            },
            ty::CREATE_SESSION => M::CreateSession {
                req: f.varint(REQ)?,
                name: f.string(NAME)?,
            },
            ty::CREATE_WINDOW => M::CreateWindow {
                req: f.varint(REQ)?,
                session: session(&f, SESSION)?,
                name: f.string(NAME)?,
            },
            ty::CREATE_BLOCK => M::CreateBlock {
                req: f.varint(REQ)?,
                window: window(&f, WINDOW)?,
                size: get_size(&f)?,
                launch: get_launch(&f)?,
            },
            ty::ATTACH => M::Attach {
                req: f.varint(REQ)?,
                block: block(&f, BLOCK)?,
            },
            ty::DETACH => M::Detach {
                req: f.varint(REQ)?,
            },
            ty::ACQUIRE_LEASE => M::AcquireLease {
                req: f.varint(REQ)?,
                block: block(&f, BLOCK)?,
            },
            ty::RELEASE_LEASE => M::ReleaseLease {
                req: f.varint(REQ)?,
                block: block(&f, BLOCK)?,
            },
            ty::INPUT => M::Input {
                req: f.varint(REQ)?,
                block: block(&f, BLOCK)?,
                data: f.bytes(DATA)?.to_vec(),
            },
            ty::RESIZE => M::Resize {
                req: f.varint(REQ)?,
                block: block(&f, BLOCK)?,
                size: get_size(&f)?,
            },
            ty::SYNC => M::Sync {
                req: f.varint(REQ)?,
            },
            ty::CLOSE_BLOCK => M::CloseBlock {
                req: f.varint(REQ)?,
                block: block(&f, BLOCK)?,
            },
            ty::LIST => M::List {
                req: f.varint(REQ)?,
            },
            ty::BLOCK_CALL => M::BlockCall {
                req: f.varint(REQ)?,
                block: block(&f, BLOCK)?,
                method: f.string(METHOD)?,
                args: args::decode(f.bytes(ARGS)?).ok_or_else(|| f.invalid(ARGS))?,
            },
            ty::RENAME => M::Rename {
                req: f.varint(REQ)?,
                target: get_target(&f)?,
                name: f.string(NAME)?,
            },
            ty::DISCONNECT => M::Disconnect {
                req: f.varint(REQ)?,
                client: client(&f, CLIENT)?,
            },
            ty::WELCOME => M::Welcome {
                features: Features(f.varint(FEATURES)?),
                client: client(&f, CLIENT)?,
                cursor: get_cursor(&f)?,
            },
            ty::DONE => M::Done {
                req: f.varint(REQ)?,
                created: f.opt(CREATED, In::uuid)?,
            },
            ty::FAILED => M::Failed {
                req: f.varint(REQ)?,
                reason: f.string(REASON)?,
                code: f.opt(CODE, In::varint)?.and_then(Code::from_wire),
            },
            ty::TEXT => M::Text {
                req: f.varint(REQ)?,
                text: f.string(DATA)?,
            },
            ty::ENTRY => M::Entry {
                req: f.varint(REQ)?,
                entry: get_entry(&f)?,
            },
            ty::EVENT => M::Event {
                seq: f.opt(SEQ, In::varint)?,
                event: get_event(&f)?,
            },
            ty::SNAPSHOT => M::Snapshot {
                block: block(&f, BLOCK)?,
                data: f.bytes(DATA)?.to_vec(),
            },
            ty::CHECKPOINT => M::Checkpoint {
                block: block(&f, BLOCK)?,
                offset: f.varint(OFFSET)?,
                total: f.varint(TOTAL)?,
                data: f.bytes(DATA)?.to_vec(),
            },
            ty::READY => M::Ready {
                block: block(&f, BLOCK)?,
            },
            ty::OUTPUT => M::Output {
                block: block(&f, BLOCK)?,
                data: f.bytes(DATA)?.to_vec(),
            },
            other => return Err(Error::UnknownType(other)),
        })
    }
}

fn req_block(out: &mut Out, req: u64, block: BlockId, message: u8) -> u8 {
    out.varint(REQ, req).uuid(BLOCK, block.uuid());
    message
}

fn put_size(out: &mut Out, size: Size) {
    out.varint(COLS, size.cols().into())
        .varint(ROWS, size.rows().into());
}

fn get_size(f: &In<'_>) -> Result<Size, Error> {
    Size::new(f.u16(COLS)?, f.u16(ROWS)?).map_err(|_| f.invalid(COLS))
}

fn put_launch(out: &mut Out, launch: &Launch) {
    if let Some(Program { command, args }) = &launch.program {
        let mut argv = Writer::new();
        for arg in args {
            varint::write(&mut argv, arg.len() as u64);
            argv.write_bytes(arg.as_bytes());
        }
        out.bytes(COMMAND, command.as_bytes())
            .bytes(ARGV, &argv.into_vec());
    }
    if let Some(cwd) = &launch.cwd {
        out.bytes(CWD, cwd.as_bytes());
    }
}

/// Arguments only with a command.
fn get_launch(f: &In<'_>) -> Result<Launch, Error> {
    let command = f.opt(COMMAND, In::string)?;
    let argv = f.opt(ARGV, In::bytes)?;
    let program = match (command, argv) {
        (None, None) => None,
        (None, Some(_)) => return Err(f.invalid(ARGV)),
        (Some(command), argv) => Some(Program {
            command,
            args: get_argv(argv.unwrap_or_default()).ok_or_else(|| f.invalid(ARGV))?,
        }),
    };
    Ok(Launch {
        program,
        cwd: f.opt(CWD, In::string)?,
    })
}

fn get_argv(bytes: &[u8]) -> Option<Vec<String>> {
    let mut r = Reader::new(bytes);
    let mut args = Vec::new();
    while !r.is_empty() {
        let len = usize::try_from(varint::read(&mut r).ok()?).ok()?;
        args.push(String::from_utf8(r.read_bytes(len).ok()?.to_vec()).ok()?);
    }
    Some(args)
}

fn put_cursor(out: &mut Out, cursor: Option<Cursor>) {
    if let Some(Cursor { epoch, seq }) = cursor {
        out.uuid(EPOCH, epoch).varint(SEQ, seq);
    }
}

/// Both cursor fields or neither.
fn get_cursor(f: &In<'_>) -> Result<Option<Cursor>, Error> {
    if f.opt(EPOCH, In::uuid)?.is_none() && f.opt(SEQ, In::varint)?.is_none() {
        return Ok(None);
    }
    Ok(Some(Cursor {
        epoch: f.uuid(EPOCH)?,
        seq: f.varint(SEQ)?,
    }))
}

fn session(f: &In<'_>, id: u32) -> Result<SessionId, Error> {
    f.uuid(id).map(SessionId::from_uuid)
}

fn window(f: &In<'_>, id: u32) -> Result<WindowId, Error> {
    f.uuid(id).map(WindowId::from_uuid)
}

fn block(f: &In<'_>, id: u32) -> Result<BlockId, Error> {
    f.uuid(id).map(BlockId::from_uuid)
}

fn client(f: &In<'_>, id: u32) -> Result<ClientId, Error> {
    f.uuid(id).map(ClientId::from_uuid)
}

/// An entity of any kind, by the field its id travels in.
fn put_target(out: &mut Out, target: EntityId) -> &mut Out {
    match target {
        EntityId::Session(id) => out.uuid(SESSION, id.uuid()),
        EntityId::Window(id) => out.uuid(WINDOW, id.uuid()),
        EntityId::Block(id) => out.uuid(BLOCK, id.uuid()),
        EntityId::Client(id) => out.uuid(CLIENT, id.uuid()),
    }
}

/// Exactly one of the four id fields.
fn get_target(f: &In<'_>) -> Result<EntityId, Error> {
    let found = [
        f.opt(SESSION, session)?.map(EntityId::Session),
        f.opt(WINDOW, window)?.map(EntityId::Window),
        f.opt(BLOCK, block)?.map(EntityId::Block),
        f.opt(CLIENT, client)?.map(EntityId::Client),
    ];
    let mut present = found.into_iter().flatten();
    match (present.next(), present.next()) {
        (Some(target), None) => Ok(target),
        _ => Err(f.invalid(SESSION)),
    }
}

mod kind {
    pub const SESSION_CREATED: u64 = 1;
    pub const SESSION_CLOSED: u64 = 2;
    pub const WINDOW_CREATED: u64 = 3;
    pub const WINDOW_CLOSED: u64 = 4;
    pub const BLOCK_CREATED: u64 = 5;
    pub const BLOCK_RESIZED: u64 = 6;
    pub const BLOCK_CLOSED: u64 = 7;
    pub const CLIENT_CONNECTED: u64 = 8;
    pub const CLIENT_DISCONNECTED: u64 = 9;
    pub const ATTACHED: u64 = 10;
    pub const DETACHED: u64 = 11;
    pub const LEASE_CHANGED: u64 = 12;
    pub const BLOCK_STATUS: u64 = 13;
    pub const PWD_CHANGED: u64 = 14;
    pub const TITLE_CHANGED: u64 = 15;
    pub const PROGRAM_EXITED: u64 = 16;
    pub const BELL: u64 = 17;
    pub const PROGRESS_CHANGED: u64 = 18;
    pub const NOTIFICATION: u64 = 19;
    pub const PROCESS_CHANGED: u64 = 20;
    pub const SESSION_RENAMED: u64 = 21;
    pub const WINDOW_RENAMED: u64 = 22;
    pub const BLOCK_RENAMED: u64 = 23;
    pub const CLIENT_RENAMED: u64 = 24;
}

fn put_event(out: &mut Out, event: &Event) {
    use Event as E;
    match *event {
        E::SessionCreated { session } => {
            out.varint(KIND, kind::SESSION_CREATED)
                .uuid(SESSION, session.uuid());
        }
        E::SessionClosed { session } => {
            out.varint(KIND, kind::SESSION_CLOSED)
                .uuid(SESSION, session.uuid());
        }
        E::WindowCreated { window, session } => {
            out.varint(KIND, kind::WINDOW_CREATED)
                .uuid(WINDOW, window.uuid())
                .uuid(SESSION, session.uuid());
        }
        E::WindowClosed { window } => {
            out.varint(KIND, kind::WINDOW_CLOSED)
                .uuid(WINDOW, window.uuid());
        }
        E::BlockCreated {
            block,
            window,
            size,
        } => {
            let o = out
                .varint(KIND, kind::BLOCK_CREATED)
                .uuid(BLOCK, block.uuid())
                .uuid(WINDOW, window.uuid());
            put_size(o, size);
        }
        E::BlockResized { block, size } => {
            put_size(
                out.varint(KIND, kind::BLOCK_RESIZED)
                    .uuid(BLOCK, block.uuid()),
                size,
            );
        }
        E::BlockClosed { block } => {
            out.varint(KIND, kind::BLOCK_CLOSED)
                .uuid(BLOCK, block.uuid());
        }
        E::ClientConnected { client } => {
            out.varint(KIND, kind::CLIENT_CONNECTED)
                .uuid(CLIENT, client.uuid());
        }
        E::ClientDisconnected { client } => {
            out.varint(KIND, kind::CLIENT_DISCONNECTED)
                .uuid(CLIENT, client.uuid());
        }
        E::Attached { client, block } => {
            out.varint(KIND, kind::ATTACHED)
                .uuid(CLIENT, client.uuid())
                .uuid(BLOCK, block.uuid());
        }
        E::Detached { client, block } => {
            out.varint(KIND, kind::DETACHED)
                .uuid(CLIENT, client.uuid())
                .uuid(BLOCK, block.uuid());
        }
        E::LeaseChanged { block, holder } => {
            out.varint(KIND, kind::LEASE_CHANGED)
                .uuid(BLOCK, block.uuid());
            if let Some(holder) = holder {
                out.uuid(HOLDER, holder.uuid());
            }
        }
        E::BlockStatus { block } => {
            out.varint(KIND, kind::BLOCK_STATUS)
                .uuid(BLOCK, block.uuid());
        }
        E::PwdChanged { block } => {
            out.varint(KIND, kind::PWD_CHANGED)
                .uuid(BLOCK, block.uuid());
        }
        E::TitleChanged { block } => {
            out.varint(KIND, kind::TITLE_CHANGED)
                .uuid(BLOCK, block.uuid());
        }
        E::ProgramExited { block, status } => {
            let o = out
                .varint(KIND, kind::PROGRAM_EXITED)
                .uuid(BLOCK, block.uuid());
            match status {
                ExitStatus::Code(code) => o.varint(EXIT, code.cast_unsigned().into()),
                ExitStatus::Signal(signal) => o.varint(SIGNAL, signal.cast_unsigned().into()),
            };
        }
        E::Bell { block } => {
            out.varint(KIND, kind::BELL).uuid(BLOCK, block.uuid());
        }
        E::ProgressChanged { block, progress } => {
            put_progress(
                out.varint(KIND, kind::PROGRESS_CHANGED)
                    .uuid(BLOCK, block.uuid()),
                progress,
            );
        }
        E::Notification {
            block,
            ref title,
            ref body,
        } => {
            out.varint(KIND, kind::NOTIFICATION)
                .uuid(BLOCK, block.uuid())
                .bytes(TITLE, title.as_bytes())
                .bytes(BODY, body.as_bytes());
        }
        E::ProcessChanged {
            block,
            pid,
            ref name,
        } => {
            out.varint(KIND, kind::PROCESS_CHANGED)
                .uuid(BLOCK, block.uuid())
                .varint(PID, pid.into())
                .bytes(NAME, name.as_bytes());
        }
        E::SessionRenamed { session, ref name } => {
            renamed(out, kind::SESSION_RENAMED, EntityId::Session(session), name);
        }
        E::WindowRenamed { window, ref name } => {
            renamed(out, kind::WINDOW_RENAMED, EntityId::Window(window), name);
        }
        E::BlockRenamed { block, ref name } => {
            renamed(out, kind::BLOCK_RENAMED, EntityId::Block(block), name);
        }
        E::ClientRenamed { client, ref name } => {
            renamed(out, kind::CLIENT_RENAMED, EntityId::Client(client), name);
        }
    }
}

fn renamed(out: &mut Out, kind: u64, target: EntityId, name: &str) {
    put_target(out.varint(KIND, kind), target).bytes(NAME, name.as_bytes());
}

fn put_progress(out: &mut Out, progress: Progress) {
    let (state, percent) = match progress {
        Progress::Cleared => (0, None),
        Progress::Normal(p) => (1, Some(p)),
        Progress::Error(p) => (2, Some(p)),
        Progress::Indeterminate => (3, None),
        Progress::Paused(p) => (4, Some(p)),
    };
    out.varint(PROGRESS, state);
    if let Some(percent) = percent {
        out.varint(PERCENT, percent.into());
    }
}

/// A percentage is at most 100; a state that has one needs it.
fn get_progress(f: &In<'_>) -> Result<Progress, Error> {
    let percent = || {
        u8::try_from(f.varint(PERCENT)?)
            .ok()
            .filter(|p| *p <= 100)
            .ok_or_else(|| f.invalid(PERCENT))
    };
    Ok(match f.varint(PROGRESS)? {
        0 => Progress::Cleared,
        1 => Progress::Normal(percent()?),
        2 => Progress::Error(percent()?),
        3 => Progress::Indeterminate,
        4 => Progress::Paused(percent()?),
        _ => return Err(f.invalid(PROGRESS)),
    })
}

/// An exit code or a signal, whichever is present; a value is 32 bits.
fn get_exit(f: &In<'_>) -> Result<ExitStatus, Error> {
    let bits = |id| {
        u32::try_from(f.varint(id)?)
            .map(u32::cast_signed)
            .map_err(|_| f.invalid(id))
    };
    match f.opt(EXIT, |_, id| bits(id))? {
        Some(code) => Ok(ExitStatus::Code(code)),
        None => bits(SIGNAL).map(ExitStatus::Signal),
    }
}

fn get_event(f: &In<'_>) -> Result<Event, Error> {
    use Event as E;
    Ok(match f.varint(KIND)? {
        kind::SESSION_CREATED => E::SessionCreated {
            session: session(f, SESSION)?,
        },
        kind::SESSION_CLOSED => E::SessionClosed {
            session: session(f, SESSION)?,
        },
        kind::WINDOW_CREATED => E::WindowCreated {
            window: window(f, WINDOW)?,
            session: session(f, SESSION)?,
        },
        kind::WINDOW_CLOSED => E::WindowClosed {
            window: window(f, WINDOW)?,
        },
        kind::BLOCK_CREATED => E::BlockCreated {
            block: block(f, BLOCK)?,
            window: window(f, WINDOW)?,
            size: get_size(f)?,
        },
        kind::BLOCK_RESIZED => E::BlockResized {
            block: block(f, BLOCK)?,
            size: get_size(f)?,
        },
        kind::BLOCK_CLOSED => E::BlockClosed {
            block: block(f, BLOCK)?,
        },
        kind::CLIENT_CONNECTED => E::ClientConnected {
            client: client(f, CLIENT)?,
        },
        kind::CLIENT_DISCONNECTED => E::ClientDisconnected {
            client: client(f, CLIENT)?,
        },
        kind::ATTACHED => E::Attached {
            client: client(f, CLIENT)?,
            block: block(f, BLOCK)?,
        },
        kind::DETACHED => E::Detached {
            client: client(f, CLIENT)?,
            block: block(f, BLOCK)?,
        },
        kind::LEASE_CHANGED => E::LeaseChanged {
            block: block(f, BLOCK)?,
            holder: f.opt(HOLDER, client)?,
        },
        kind::BLOCK_STATUS => E::BlockStatus {
            block: block(f, BLOCK)?,
        },
        kind::PWD_CHANGED => E::PwdChanged {
            block: block(f, BLOCK)?,
        },
        kind::TITLE_CHANGED => E::TitleChanged {
            block: block(f, BLOCK)?,
        },
        kind::PROGRAM_EXITED => E::ProgramExited {
            block: block(f, BLOCK)?,
            status: get_exit(f)?,
        },
        kind::BELL => E::Bell {
            block: block(f, BLOCK)?,
        },
        kind::PROGRESS_CHANGED => E::ProgressChanged {
            block: block(f, BLOCK)?,
            progress: get_progress(f)?,
        },
        kind::NOTIFICATION => E::Notification {
            block: block(f, BLOCK)?,
            title: f.string(TITLE)?,
            body: f.string(BODY)?,
        },
        kind::PROCESS_CHANGED => E::ProcessChanged {
            block: block(f, BLOCK)?,
            pid: u32::try_from(f.varint(PID)?).map_err(|_| f.invalid(PID))?,
            name: f.string(NAME)?,
        },
        kind::SESSION_RENAMED => E::SessionRenamed {
            session: session(f, SESSION)?,
            name: f.string(NAME)?,
        },
        kind::WINDOW_RENAMED => E::WindowRenamed {
            window: window(f, WINDOW)?,
            name: f.string(NAME)?,
        },
        kind::BLOCK_RENAMED => E::BlockRenamed {
            block: block(f, BLOCK)?,
            name: f.string(NAME)?,
        },
        kind::CLIENT_RENAMED => E::ClientRenamed {
            client: client(f, CLIENT)?,
            name: f.string(NAME)?,
        },
        _ => return Err(f.invalid(KIND)),
    })
}

mod entry {
    pub const SESSION: u64 = 1;
    pub const WINDOW: u64 = 2;
    pub const BLOCK: u64 = 3;
    pub const CLIENT: u64 = 4;
}

fn put_entry(out: &mut Out, entry: &Entry) {
    match entry {
        Entry::Session { session, name } => {
            out.varint(KIND, entry::SESSION)
                .uuid(SESSION, session.uuid())
                .bytes(NAME, name.as_bytes());
        }
        Entry::Window {
            window,
            session,
            name,
        } => {
            out.varint(KIND, entry::WINDOW)
                .uuid(WINDOW, window.uuid())
                .uuid(SESSION, session.uuid())
                .bytes(NAME, name.as_bytes());
        }
        Entry::Block {
            block,
            window,
            size,
            holder,
            title,
            name,
        } => {
            let o = out
                .varint(KIND, entry::BLOCK)
                .uuid(BLOCK, block.uuid())
                .uuid(WINDOW, window.uuid())
                .bytes(TITLE, title.as_bytes())
                .bytes(NAME, name.as_bytes());
            if let Some(holder) = holder {
                o.uuid(HOLDER, holder.uuid());
            }
            put_size(o, *size);
        }
        Entry::Client {
            client,
            name,
            attached,
        } => {
            let o = out
                .varint(KIND, entry::CLIENT)
                .uuid(CLIENT, client.uuid())
                .bytes(NAME, name.as_bytes());
            if let Some(block) = attached {
                o.uuid(BLOCK, block.uuid());
            }
        }
    }
}

fn get_entry(f: &In<'_>) -> Result<Entry, Error> {
    Ok(match f.varint(KIND)? {
        entry::SESSION => Entry::Session {
            session: session(f, SESSION)?,
            name: f.string(NAME)?,
        },
        entry::WINDOW => Entry::Window {
            window: window(f, WINDOW)?,
            session: session(f, SESSION)?,
            name: f.string(NAME)?,
        },
        entry::BLOCK => Entry::Block {
            block: block(f, BLOCK)?,
            window: window(f, WINDOW)?,
            size: get_size(f)?,
            holder: f.opt(HOLDER, client)?,
            title: f.string(TITLE)?,
            // An older server sends no name.
            name: f.opt(NAME, In::string)?.unwrap_or_default(),
        },
        entry::CLIENT => Entry::Client {
            client: client(f, CLIENT)?,
            name: f.string(NAME)?,
            attached: f.opt(BLOCK, block)?,
        },
        _ => return Err(f.invalid(KIND)),
    })
}

impl Code {
    const fn wire(self) -> u64 {
        match self {
            Self::NotFound => 1,
            Self::MethodNotFound => 2,
        }
    }

    const fn from_wire(value: u64) -> Option<Self> {
        match value {
            1 => Some(Self::NotFound),
            2 => Some(Self::MethodNotFound),
            _ => None,
        }
    }
}
