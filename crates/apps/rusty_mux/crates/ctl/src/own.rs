//! `own`: one long-lived connection that owns every block it launches
//! (C46), driven by one JSON request per line on stdin and answering with
//! one JSON line per answer or owner event on stdout (pack D118).
//!
//! Requests:
//! - `{"id":ID,"op":"launch","window":UUID,"cols":N,"rows":N,"program":P,"args":[..],"cwd":DIR}`,
//!   where only `window` is required; the answer carries the new `block`.
//! - `{"id":ID,"op":"close","block":UUID}`.
//!
//! Answers are `{"id":ID,"ok":true,…}` or `{"id":ID,"ok":false,"error":…}`.
//! Owner events are `{"event":KIND,…}` for `program-exited`,
//! `lease-changed` and `block-closed` on the blocks it owns, `reconnected`
//! after a reconnect inside the server's grace, and `lost` when the
//! connection, and with it the ownership, is gone; `own` then exits with
//! [`Exit::Unreachable`]. A `launch` in flight when the connection drops
//! may have started its program, so it ends `own` as `lost` rather than
//! being answered as failed. The end of stdin closes the connection: the
//! blocks orphan once the server's grace runs out.
//!
//! The takeover token stays inside this process and is never printed.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

#[cfg(windows)]
use platform_windows::WindowsUnixStream as UnixStream;
use rusty_json::{Map, Value};
use rusty_mux_core::{BlockId, ClientId, Event, Size, WindowId};
use rusty_mux_proto::{Cursor, Features, Launch, Message, Program, Takeover, encode};
use rusty_uuid::Uuid;

use crate::conn::{Conn, connect};
use crate::stop::Stop;
use crate::{Exit, Failure, render};

/// What an owner connection needs from the server.
const NEEDED: Features = Features::LAUNCH
    .union(Features::OWNERSHIP)
    .union(Features::TAKEOVER)
    .union(Features::EVENT_SEQ)
    .union(Features::PROGRAM_EVENTS);

/// How long a dropped connection is retried: inside the server's 30 s
/// grace (D44), leaving room for the last try.
const RECONNECT: Duration = Duration::from_secs(25);

/// The pause between reconnect tries.
const RETRY: Duration = Duration::from_millis(250);

/// A new block's size when a launch names none.
const DEFAULT_SIZE: (u16, u16) = (80, 24);

/// What the main loop handles, one at a time and in order.
enum Inbound {
    Line(String),
    /// Stdin ended or failed.
    End,
    /// A message on the connection of this generation.
    Frame(u64, Message),
    /// The connection of this generation ended.
    Dropped(u64),
}

/// One request read from stdin.
#[derive(Debug, PartialEq)]
enum Request {
    Launch {
        window: WindowId,
        size: Size,
        launch: Launch,
    },
    Close {
        block: BlockId,
    },
}

/// A request sent and not yet answered, with the caller's id.
struct Pending {
    id: Value,
    launch: bool,
}

/// The current connection: the write half, and what taking it over needs.
struct Link {
    stream: UnixStream,
    client: ClientId,
    token: Uuid,
    generation: u64,
}

/// Runs `own` against the server at `socket`, reading requests from
/// `input` and writing answers and events to `out`, until `input` ends
/// ([`Exit::Ok`]) or the ownership is lost ([`Exit::Unreachable`]).
pub(crate) fn run(
    socket: &Path,
    input: impl BufRead + Send + 'static,
    out: &mut dyn Write,
) -> Result<Exit, Failure> {
    let (tx, rx) = mpsc::channel();
    let (link, cursor) = open(socket, None, None, 0, &tx)?;
    read_lines(input, tx.clone());
    let mut owner = Owner {
        socket,
        tx,
        out,
        link,
        cursor,
        pending: HashMap::new(),
        owned: HashSet::new(),
        next_req: 1,
    };
    // The loop holds a sender, so the channel never disconnects.
    while let Ok(inbound) = rx.recv() {
        match inbound {
            Inbound::Line(line) => owner.request(&line)?,
            Inbound::End => {
                owner.close();
                return Ok(Exit::Ok);
            }
            Inbound::Frame(generation, message) if generation == owner.link.generation => {
                owner.answer(message)?;
            }
            Inbound::Dropped(generation) if generation == owner.link.generation => {
                if !owner.reconnect()? {
                    owner.print(&event("lost", []))?;
                    return Ok(Exit::Unreachable);
                }
            }
            // From a connection already replaced.
            Inbound::Frame(..) | Inbound::Dropped(_) => {}
        }
    }
    Ok(Exit::Ok)
}

struct Owner<'a> {
    socket: &'a Path,
    tx: Sender<Inbound>,
    out: &'a mut dyn Write,
    link: Link,
    /// The last event seen, to resume from after a reconnect.
    cursor: Option<Cursor>,
    pending: HashMap<u64, Pending>,
    owned: HashSet<BlockId>,
    next_req: u64,
}

impl Owner<'_> {
    fn print(&mut self, line: &str) -> Result<(), Failure> {
        writeln!(self.out, "{line}")
            .and_then(|()| self.out.flush())
            .map_err(|e| Failure::Failed(format!("writing output: {e}")))
    }

    /// Sends the request on `line`, or answers it as refused if it does
    /// not parse.
    fn request(&mut self, line: &str) -> Result<(), Failure> {
        if line.trim().is_empty() {
            return Ok(());
        }
        let (id, request) = match parse(line) {
            Ok(parsed) => parsed,
            Err((id, why)) => return self.print(&failed(id, &why)),
        };
        let req = self.next_req;
        self.next_req += 1;
        let (message, launch) = match request {
            Request::Launch {
                window,
                size,
                launch,
            } => (
                Message::CreateBlock {
                    req,
                    window,
                    size,
                    launch,
                },
                true,
            ),
            Request::Close { block } => (Message::CloseBlock { req, block }, false),
        };
        self.pending.insert(req, Pending { id, launch });
        // A failed write shows as the connection dropping, which the
        // reader reports; the request is then pending like any other.
        let _ = send(&mut self.link.stream, &message);
        Ok(())
    }

    /// Prints the answer or owner event `message` carries, if any.
    fn answer(&mut self, message: Message) -> Result<(), Failure> {
        match message {
            Message::Done { req, created } => {
                let Some(pending) = self.pending.remove(&req) else {
                    return Ok(());
                };
                let block = created.filter(|_| pending.launch);
                if let Some(block) = block {
                    self.owned.insert(BlockId::from_uuid(block));
                }
                let block = block.map(|b| ("block", Value::from(b.to_string())));
                self.print(&answered(pending.id, block))
            }
            Message::Failed { req, reason, .. } => match self.pending.remove(&req) {
                Some(pending) => self.print(&failed(pending.id, &reason)),
                None => Ok(()),
            },
            Message::Event { seq, event } => {
                if let (Some(seq), Some(cursor)) = (seq, self.cursor.as_mut()) {
                    cursor.seq = seq;
                }
                self.owner_event(&event)
            }
            _ => Ok(()),
        }
    }

    /// Prints `event` if it concerns a block this connection owns.
    fn owner_event(&mut self, event: &Event) -> Result<(), Failure> {
        let (block, closed) = match *event {
            Event::ProgramExited { block, .. } | Event::LeaseChanged { block, .. } => {
                (block, false)
            }
            Event::BlockClosed { block } => (block, true),
            _ => return Ok(()),
        };
        let owned = match closed {
            true => self.owned.remove(&block),
            false => self.owned.contains(&block),
        };
        if !owned {
            return Ok(());
        }
        self.print(&render::owner_event_line(event))
    }

    /// Takes the client over on a new connection within the grace.
    /// `false` when the ownership is lost: a launch was in flight, or no
    /// connection took the client over in time.
    fn reconnect(&mut self) -> Result<bool, Failure> {
        if self.pending.values().any(|p| p.launch) {
            return Ok(false);
        }
        let lost: Vec<Pending> = self.pending.drain().map(|(_, p)| p).collect();
        for pending in lost {
            self.print(&failed(pending.id, "connection lost"))?;
        }
        let takeover = Takeover {
            client: self.link.client,
            token: self.link.token,
        };
        let generation = self.link.generation + 1;
        let deadline = Instant::now() + RECONNECT;
        while Instant::now() < deadline {
            let Ok((link, cursor)) = open(
                self.socket,
                Some(takeover),
                self.cursor,
                generation,
                &self.tx,
            ) else {
                std::thread::sleep(RETRY);
                continue;
            };
            if link.client != takeover.client {
                // A client of its own: the old one, and what it owned, is gone.
                let _ = link.stream.shutdown(Shutdown::Both);
                return Ok(false);
            }
            let resumed = cursor.is_some() && cursor == self.cursor;
            self.link = link;
            self.cursor = cursor;
            self.print(&event("reconnected", [("resumed", Value::Bool(resumed))]))?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Ends the connection; the server keeps the client for its grace.
    fn close(&mut self) {
        let _ = self.link.stream.shutdown(Shutdown::Both);
    }
}

/// Connects, says Hello, and starts a thread passing the connection's
/// messages to `tx` as `generation`'s. The link and the Welcome's cursor.
fn open(
    socket: &Path,
    takeover: Option<Takeover>,
    resume: Option<Cursor>,
    generation: u64,
    tx: &Sender<Inbound>,
) -> Result<(Link, Option<Cursor>), Failure> {
    let stream = connect(socket, &Stop::default())?;
    let mut conn = Conn::new(stream.try_clone()?, Stop::default());
    conn.send(&Message::Hello {
        features: NEEDED,
        resume,
        takeover,
    })?;
    let Message::Welcome {
        features,
        client,
        cursor,
        token,
    } = conn.recv()?
    else {
        return Err(Failure::Failed("expected Welcome".into()));
    };
    if !features.contains(NEEDED) {
        return Err(Failure::Failed(format!(
            "the server cannot own blocks: needs feature bits {:#x}",
            NEEDED.bits()
        )));
    }
    let token = token.ok_or_else(|| Failure::Failed("the server sent no takeover token".into()))?;
    let tx = tx.clone();
    std::thread::spawn(move || {
        loop {
            match conn.recv() {
                Ok(message) => {
                    if tx.send(Inbound::Frame(generation, message)).is_err() {
                        return;
                    }
                }
                Err(_) => {
                    let _ = tx.send(Inbound::Dropped(generation));
                    return;
                }
            }
        }
    });
    let link = Link {
        stream,
        client,
        token,
        generation,
    };
    Ok((link, cursor))
}

/// Passes each line of `input` to `tx`, then [`Inbound::End`].
fn read_lines(input: impl BufRead + Send + 'static, tx: Sender<Inbound>) {
    std::thread::spawn(move || {
        for line in input.lines() {
            let Ok(line) = line else {
                break;
            };
            if tx.send(Inbound::Line(line)).is_err() {
                return;
            }
        }
        let _ = tx.send(Inbound::End);
    });
}

fn send(stream: &mut UnixStream, message: &Message) -> io::Result<()> {
    let frame = encode(message).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    stream.write_all(&frame)
}

/// The request on `line` and its id, or why it is refused with the id
/// (`null` when it has none).
fn parse(line: &str) -> Result<(Value, Request), (Value, String)> {
    let value = Value::from_json_str(line).map_err(|e| (Value::Null, format!("not JSON: {e}")))?;
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    let request = request(&value).map_err(|why| (id.clone(), why))?;
    Ok((id, request))
}

fn request(value: &Value) -> Result<Request, String> {
    match value.get("op").and_then(Value::as_str) {
        Some("launch") => Ok(Request::Launch {
            window: WindowId::from_uuid(uuid(value, "window")?),
            size: size(value)?,
            launch: launch(value)?,
        }),
        Some("close") => Ok(Request::Close {
            block: BlockId::from_uuid(uuid(value, "block")?),
        }),
        Some(op) => Err(format!("unknown op `{op}`")),
        None => Err("no `op`".into()),
    }
}

fn uuid(value: &Value, key: &str) -> Result<Uuid, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{key}` must be an id"))?
        .parse()
        .map_err(|_| format!("`{key}` is not an id"))
}

fn size(value: &Value) -> Result<Size, String> {
    let cells = |key: &str, default: u16| match value.get(key) {
        None => Ok(default),
        Some(n) => n
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .ok_or_else(|| format!("`{key}` must be a number of cells")),
    };
    let (cols, rows) = (
        cells("cols", DEFAULT_SIZE.0)?,
        cells("rows", DEFAULT_SIZE.1)?,
    );
    Size::new(cols, rows).map_err(|e| e.to_string())
}

/// An owned launch of the request's program, if it names one.
fn launch(value: &Value) -> Result<Launch, String> {
    let text = |key: &str| match value.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .map(|s| Some(s.to_owned()))
            .ok_or_else(|| format!("`{key}` must be text")),
    };
    let args = match value.get("args") {
        None => Vec::new(),
        Some(args) => args
            .as_array()
            .and_then(|a| a.iter().map(|v| v.as_str().map(str::to_owned)).collect())
            .ok_or("`args` must be a list of text")?,
    };
    let program = match text("program")? {
        Some(command) => Some(Program { command, args }),
        None if args.is_empty() => None,
        None => return Err("`args` need a `program`".into()),
    };
    Ok(Launch {
        program,
        cwd: text("cwd")?,
        own: true,
    })
}

fn answered(id: Value, field: Option<(&str, Value)>) -> String {
    let mut map = Map::new();
    map.insert("id".into(), id);
    map.insert("ok".into(), Value::Bool(true));
    if let Some((key, value)) = field {
        map.insert(key.into(), value);
    }
    Value::Object(map).to_json_string()
}

fn failed(id: Value, why: &str) -> String {
    let mut map = Map::new();
    map.insert("id".into(), id);
    map.insert("ok".into(), Value::Bool(false));
    map.insert("error".into(), Value::from(why));
    Value::Object(map).to_json_string()
}

fn event<const N: usize>(kind: &str, fields: [(&str, Value); N]) -> String {
    let mut map = Map::new();
    map.insert("event".into(), Value::from(kind));
    for (key, value) in fields {
        map.insert(key.into(), value);
    }
    Value::Object(map).to_json_string()
}

#[cfg(test)]
mod tests;
