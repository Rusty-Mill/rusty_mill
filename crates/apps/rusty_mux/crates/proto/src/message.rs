//! The messages and their field layouts.

use rusty_mux_core::{BlockId, ClientId, Event, SessionId, Size, WindowId};
use rusty_uuid::Uuid;

use crate::Error;
use crate::field::{In, Out};

/// Optional protocol capabilities. Each peer sends its own; the session uses
/// the intersection. Replay checkpoints require explicit opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Features(u64);

impl Features {
    pub const NONE: Self = Self(0);

    /// Snapshot payload is the versioned RMRP replay checkpoint (see mux docs).
    pub const REPLAY_SNAPSHOT: Self = Self(1);

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    /// The features both peers support.
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
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
    // Server to client.
    Welcome {
        features: Features,
        client: ClientId,
    },
    /// The request succeeded; `created` is the new entity's id, if it made one.
    Done {
        req: u64,
        created: Option<Uuid>,
    },
    Failed {
        req: u64,
        reason: String,
    },
    Event(Event),
    /// The block's terminal state, sent on attach before any output.
    Snapshot {
        block: BlockId,
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
const HOLDER: u32 = 14;
// IDs 1..=HOLDER are recognized. Extend this bound when adding field IDs.
pub(crate) const FIELD_COUNT: usize = HOLDER as usize;

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
    pub const WELCOME: u8 = 0x81;
    pub const DONE: u8 = 0x82;
    pub const FAILED: u8 = 0x83;
    pub const EVENT: u8 = 0x84;
    pub const SNAPSHOT: u8 = 0x85;
    pub const READY: u8 = 0x86;
    pub const OUTPUT: u8 = 0x87;
}

impl Message {
    pub(crate) fn encode_body(&self, out: &mut Out) -> u8 {
        use Message as M;
        match self {
            M::Hello { features } => {
                out.varint(FEATURES, features.bits());
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
            M::CreateBlock { req, window, size } => {
                put_size(out.varint(REQ, *req).uuid(WINDOW, window.uuid()), *size);
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
            M::Welcome { features, client } => {
                out.varint(FEATURES, features.bits())
                    .uuid(CLIENT, client.uuid());
                ty::WELCOME
            }
            M::Done { req, created } => {
                out.varint(REQ, *req);
                if let Some(id) = created {
                    out.uuid(CREATED, *id);
                }
                ty::DONE
            }
            M::Failed { req, reason } => {
                out.varint(REQ, *req).bytes(REASON, reason.as_bytes());
                ty::FAILED
            }
            M::Event(event) => {
                put_event(out, event);
                ty::EVENT
            }
            M::Snapshot { block, data } => {
                out.uuid(BLOCK, block.uuid()).bytes(DATA, data);
                ty::SNAPSHOT
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
            ty::CLOSE_BLOCK => M::CloseBlock {
                req: f.varint(REQ)?,
                block: block(&f, BLOCK)?,
            },
            ty::WELCOME => M::Welcome {
                features: Features(f.varint(FEATURES)?),
                client: client(&f, CLIENT)?,
            },
            ty::DONE => M::Done {
                req: f.varint(REQ)?,
                created: f.opt(CREATED, In::uuid)?,
            },
            ty::FAILED => M::Failed {
                req: f.varint(REQ)?,
                reason: f.string(REASON)?,
            },
            ty::EVENT => M::Event(get_event(&f)?),
            ty::SNAPSHOT => M::Snapshot {
                block: block(&f, BLOCK)?,
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
        _ => return Err(f.invalid(KIND)),
    })
}
