//! `own`: one long-lived connection that owns every block it launches
//! (C46), driven by one JSON request per line on stdin and answering with
//! one JSON line per answer or owner event on stdout (pack D118).
//!
//! Requests:
//! - `{"id":ID,"op":"launch","window":UUID,"cols":N,"rows":N,"program":P,"args":[..],"cwd":DIR}`,
//!   where only `window` is required; the answer carries the new `block`.
//! - `{"id":ID,"op":"close","block":UUID}`.
//! - With C75: `"env":{NAME:VALUE,…}` on a launch gives its program exactly
//!   that environment, and `"fds":N` passes it this process's descriptors
//!   3 to 3+N−1 (at most 8, Linux only), which it gets at the same numbers.
//! - With C77: `"input":"owner-only"` on a launch gates its input, and
//!   `{"id":ID,"op":"grant","block":UUID,"client":UUID,"until":MS}` and
//!   `{"id":ID,"op":"revoke","block":UUID,"client":UUID}` let another client
//!   type into it until `until` (unix ms, the server's clock), or stop it.
//!
//! Answers are `{"id":ID,"ok":true,…}` or `{"id":ID,"ok":false,"error":…}`.
//! Owner events are `{"event":KIND,…}` for `program-exited`,
//! `lease-changed`, `grant-changed` and `block-closed` on the blocks it
//! owns, `input` (`block`, `client`, `seq` and the bytes as `hex`) for
//! what another client typed into an owner-only one, `lease-requested`
//! (`block`, `client`, and the asker's verified `principal` and `user`)
//! when a viewer asks for a grant, which `own` leaves to its caller to
//! answer with `grant` or not at all, `reconnected`
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
use rusty_mux_core::{BlockId, ClientId, Event, InputMode, Size, WindowId};
use rusty_mux_proto::older::Older;
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

/// Asked for too, but only owner-only launches, grants and revokes need it:
/// an older server refuses those here rather than launching an open block.
const GATE: Features = Features::INPUT_GATE;

/// Asked for too, but only launches with an exact environment or passed
/// descriptors need it (C75).
const AGENT: Features = Features::AGENT_LAUNCH;

/// The first descriptor `own` passes: what it inherited above stdio.
const FIRST_FD: i32 = 3;

/// The most descriptors one launch passes (C75).
const MAX_FDS: u64 = 8;

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
    Grant {
        block: BlockId,
        client: ClientId,
        until: u64,
    },
    Revoke {
        block: BlockId,
        client: ClientId,
    },
}

impl Request {
    /// Whether it needs [`Features::INPUT_GATE`].
    fn gated(&self) -> bool {
        match self {
            Self::Launch { launch, .. } => launch.input == InputMode::OwnerOnly,
            Self::Grant { .. } | Self::Revoke { .. } => true,
            Self::Close { .. } => false,
        }
    }

    /// Whether it needs [`Features::AGENT_LAUNCH`].
    fn agent(&self) -> bool {
        matches!(self, Self::Launch { launch, .. } if launch.env.is_some() || launch.fds > 0)
    }
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
    /// The server offered [`Features::INPUT_GATE`].
    gate: bool,
    /// The server offered [`Features::AGENT_LAUNCH`].
    agent: bool,
}

/// Runs `own` against the server at `socket`, reading requests from
/// `input` and writing answers and events to `out`, until `input` ends
/// ([`Exit::Ok`]) or the ownership is lost ([`Exit::Unreachable`]).
pub(crate) fn run(
    socket: &Path,
    input: impl BufRead + Send + 'static,
    out: &mut dyn Write,
) -> Result<Exit, Failure> {
    // Before this process opens anything that could take their numbers.
    let inherited = Inherited::now();
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
        inherited,
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
    inherited: Inherited,
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
        if request.gated() && !self.link.gate {
            return self.print(&failed(id, "the server cannot gate input"));
        }
        if request.agent() && !self.link.agent {
            return self.print(&failed(id, "the server cannot launch agents"));
        }
        let fds = match &request {
            Request::Launch { launch, .. } => launch.fds,
            _ => 0,
        };
        if let Err(why) = self.inherited.check(fds) {
            return self.print(&failed(id, &why));
        }
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
            Request::Grant {
                block,
                client,
                until,
            } => (
                Message::GrantLease {
                    req,
                    block,
                    client,
                    until,
                },
                false,
            ),
            Request::Revoke { block, client } => {
                (Message::RevokeLease { req, block, client }, false)
            }
        };
        self.pending.insert(req, Pending { id, launch });
        // A failed write shows as the connection dropping, which the
        // reader reports; the request is then pending like any other.
        let _ = send_passing(&mut self.link.stream, &message, fds);
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
            Message::InputBatch {
                block,
                client,
                seq,
                data,
            } => self.print(&event(
                "input",
                [
                    ("block", Value::from(block.to_string())),
                    ("client", Value::from(client.to_string())),
                    ("seq", Value::from(seq)),
                    ("hex", Value::from(hex(&data))),
                ],
            )),
            Message::LeaseRequested {
                block,
                client,
                principal,
            } => self.print(&event(
                "lease-requested",
                [
                    ("block", Value::from(block.to_string())),
                    ("client", Value::from(client.to_string())),
                    ("principal", Value::from(principal.id())),
                    ("user", Value::from(principal.user())),
                ],
            )),
            _ => Ok(()),
        }
    }

    /// Prints `event` if it concerns a block this connection owns.
    fn owner_event(&mut self, event: &Event) -> Result<(), Failure> {
        let (block, closed) = match *event {
            Event::ProgramExited { block, .. }
            | Event::LeaseChanged { block, .. }
            | Event::GrantChanged { block, .. } => (block, false),
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
    let asked = NEEDED
        .union(GATE)
        .union(AGENT)
        .union(Features::LEASE_REQUESTS);
    conn.send(&Message::Hello {
        version: Some(rusty_mux_proto::VERSION.into()),
        features: asked,
        resume,
        takeover,
    })?;
    let Message::Welcome {
        version,
        features,
        client,
        cursor,
        token,
    } = conn.recv()?
    else {
        return Err(Failure::Failed("expected Welcome".into()));
    };
    // Once per connection: a reconnect may reach an upgraded server.
    if let Some(older) = Older::check(asked, features, version.as_deref()) {
        eprintln!("rusty-mux-ctl: {}", older.line("the server"));
    }
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
        gate: features.contains(GATE),
        agent: features.contains(AGENT),
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

/// Sends `message`, passing descriptors 3 to 3+`fds`−1 in the same
/// `sendmsg` as its first byte, which is where the server looks (C75).
fn send_passing(stream: &mut UnixStream, message: &Message, fds: u8) -> io::Result<()> {
    let frame = encode(message).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let sent = pass(stream, &frame, fds)?;
    stream.write_all(&frame[sent..])
}

/// Sends what of `frame` one `sendmsg` takes, with descriptors 3 and on.
#[cfg(target_os = "linux")]
fn pass(stream: &UnixStream, frame: &[u8], fds: u8) -> io::Result<usize> {
    use std::os::fd::AsRawFd;
    if fds == 0 {
        return Ok(0);
    }
    let passed: Vec<i32> = (FIRST_FD..FIRST_FD + i32::from(fds)).collect();
    rusty_libc::socket::send_with_fds(stream.as_raw_fd(), frame, &passed, 0)
        .map_err(|e| io::Error::from_raw_os_error(e.0))
}

#[cfg(not(target_os = "linux"))]
fn pass(_: &UnixStream, _: &[u8], _: u8) -> io::Result<usize> {
    Ok(0)
}

/// Which of descriptors 3 to 3+[`MAX_FDS`]−1 this process inherited: only
/// those may be passed, never one it opened itself, such as its socket.
#[derive(Clone, Copy)]
struct Inherited(u8);

impl Inherited {
    #[cfg(target_os = "linux")]
    fn now() -> Self {
        use rusty_libc::fd::{F_GETFD, fcntl};
        let open = (0..MAX_FDS as u8)
            .filter(|i| fcntl(FIRST_FD + i32::from(*i), F_GETFD, 0).is_ok())
            .fold(0, |mask, i| mask | 1 << i);
        Self(open)
    }

    #[cfg(not(target_os = "linux"))]
    fn now() -> Self {
        Self(0)
    }

    /// Refuses passing descriptors 3 to 3+`fds`−1 unless each was inherited.
    fn check(self, fds: u8) -> Result<(), String> {
        if fds > 0 && cfg!(not(target_os = "linux")) {
            return Err("passing descriptors needs Linux".into());
        }
        match (0..fds).find(|i| self.0 & 1 << i == 0) {
            None => Ok(()),
            Some(i) => Err(format!(
                "descriptor {} was not inherited",
                FIRST_FD + i32::from(i)
            )),
        }
    }
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
        Some("grant") => Ok(Request::Grant {
            block: BlockId::from_uuid(uuid(value, "block")?),
            client: ClientId::from_uuid(uuid(value, "client")?),
            until: value
                .get("until")
                .and_then(Value::as_u64)
                .ok_or("`until` must be unix ms")?,
        }),
        Some("revoke") => Ok(Request::Revoke {
            block: BlockId::from_uuid(uuid(value, "block")?),
            client: ClientId::from_uuid(uuid(value, "client")?),
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
    let input = match text("input")?.as_deref() {
        None | Some("open") => InputMode::Open,
        Some("owner-only") => InputMode::OwnerOnly,
        Some(_) => return Err("`input` must be `open` or `owner-only`".into()),
    };
    Ok(Launch {
        program,
        cwd: text("cwd")?,
        own: true,
        input,
        env: env(value)?,
        fds: match value.get("fds") {
            None => 0,
            Some(n) => n
                .as_u64()
                .filter(|n| *n <= MAX_FDS)
                .and_then(|n| u8::try_from(n).ok())
                .ok_or_else(|| format!("`fds` must be a count from 0 to {MAX_FDS}"))?,
        },
    })
}

/// The exact environment a launch names, if it names one.
fn env(value: &Value) -> Result<Option<Vec<(String, String)>>, String> {
    let Some(env) = value.get("env") else {
        return Ok(None);
    };
    let env = env.as_object().ok_or("`env` must map names to text")?;
    env.iter()
        .map(|(name, v)| {
            v.as_str()
                .map(|v| (name.clone(), v.to_owned()))
                .ok_or_else(|| format!("`env` value of `{name}` must be text"))
        })
        .collect::<Result<_, _>>()
        .map(Some)
}

/// `data` as lowercase hex, two digits a byte.
fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
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
