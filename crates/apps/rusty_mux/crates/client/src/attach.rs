//! Connecting to a server and the requests answered before a client's main
//! loop starts: Hello and Welcome, takeover on a fresh path, creating a
//! first session and reading what a window shows.

use std::io;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[cfg(windows)]
use platform_windows::WindowsUnixStream as UnixStream;
use rusty_mux_core::{BlockId, ClientId, Layout, SessionId, Size, WindowId};
use rusty_mux_proto::{Entry, Features, Launch, Message, Takeover};
use rusty_uuid::Uuid;

use crate::Conn;

/// The longest a single read waits in [`answer`], so its deadline and
/// cancel flag are checked between reads.
const READ_SLICE: Duration = Duration::from_millis(100);

/// The longest wait between attempts to bring a path back.
pub const BACKOFF: Duration = Duration::from_secs(30);

/// The wait before attempt `tries + 1` to bring a path back: `2^tries`
/// seconds, at most [`BACKOFF`] (handoff D110; tries at about 0, 2, 6, 14
/// and 30 s).
pub fn backoff(tries: u32) -> Duration {
    Duration::from_secs(1 << tries.min(5)).min(BACKOFF)
}

/// What the control connection asks for; a server without `LAYOUT`
/// shows only the block asked for.
pub const CONTROL: Features = Features::CHECKPOINT_V6
    .union(Features::DATAGRAM)
    .union(Features::INSPECT)
    .union(Features::LAYOUT)
    .union(Features::SYNC)
    .union(Features::TAKEOVER)
    .union(Features::LAYOUT_EDIT)
    .union(Features::FOCUS)
    .union(Features::ACTIVE_WINDOW)
    .union(Features::SESSION_NAMES);

/// What a block's connection asks for. `TERMINAL_CONTROL` tells the
/// server this client's copy of the terminal can be reset safely.
pub const PANE: Features = Features::CHECKPOINT_V6
    .union(Features::TERMINAL_CONTROL)
    .union(Features::TAKEOVER)
    .union(Features::ECHO_ACK)
    .union(Features::SYNC);

/// How long the server may take over the answers to bring a path back,
/// or to open a block's connection from the main loop.
pub const HANDSHAKE: Duration = Duration::from_secs(10);

/// The clients a fresh path is to take over.
pub struct Plan {
    pub me: Takeover,
    pub next: u64,
    /// The window to ask the layout of, when the server answers List.
    pub window: Option<WindowId>,
    pub resumes: Vec<(BlockId, Takeover)>,
}

/// A path brought back: its connections, past their Welcomes, with
/// every client taken over. Dropping it hangs them up.
pub struct Back {
    pub socket: PathBuf,
    pub stream: UnixStream,
    pub conn: Conn<UnixStream>,
    pub writer: Conn<UnixStream>,
    pub me: Takeover,
    pub next: u64,
    /// The window `layout` is of, when asked for: the session's active
    /// window, if it moved while the path was away.
    pub window: Option<WindowId>,
    /// What the window shows, when asked for.
    pub layout: Option<Shape>,
    pub panes: Vec<Opened>,
}

/// Why a path was not brought back.
pub enum Missed {
    /// Before any client was taken over: worth another try.
    Retry(String),
    /// The server had let a client go.
    Dropped,
    /// After the control connection's client was taken over, with the
    /// takeover it was welcomed with: a request on the fresh connection
    /// may have confirmed that token and retired the one asked with,
    /// so only this one continues the client.
    Broken(String, Takeover),
}

/// What the client knows of the server's answer to its Hello.
pub struct Welcomed {
    pub features: Features,
    /// The client the server made of this connection.
    pub client: ClientId,
    /// How to take this connection's client over later, if the server
    /// allows it.
    pub takeover: Option<Takeover>,
    /// The server's version, if it sent one (C36).
    pub version: Option<String>,
}

#[cfg(unix)]
pub fn connect(path: &Path) -> Result<UnixStream, String> {
    UnixStream::connect(path).map_err(|e| format!("connecting: {e}"))
}

#[cfg(windows)]
pub fn connect(path: &Path) -> Result<UnixStream, String> {
    UnixStream::connect(path).map_err(|e| format!("connecting: {e:?}"))
}

pub fn clone(stream: &UnixStream) -> Result<UnixStream, String> {
    stream.try_clone().map_err(|e| e.to_string())
}

/// The name to ask for a new session by: none, for a generated one, from
/// a server that makes them up; else `main`.
pub fn first_session_name(features: Features) -> &'static str {
    match features.contains(Features::SESSION_NAMES) {
        true => "",
        false => "main",
    }
}

/// Requests answered before the main loop starts, or on a fresh path,
/// each answer due `by`, if given, unless `cancel` is set first.
pub struct Requests<'a> {
    pub conn: &'a mut Conn<UnixStream>,
    pub next: u64,
    pub by: Option<Instant>,
    pub cancel: &'a AtomicBool,
}

impl Requests<'_> {
    pub fn recv(&mut self) -> Result<Message, String> {
        match self.by {
            Some(by) => answer(self.conn, by, self.cancel),
            None => self.conn.recv().map_err(|e| e.to_string()),
        }
    }

    pub fn welcome(&mut self) -> Result<Welcomed, String> {
        let welcomed = welcomed(self.recv()?)?;
        match welcomed.features.contains(Features::CHECKPOINT_V6) {
            true => Ok(welcomed),
            false => Err("the server cannot send checkpoints".into()),
        }
    }

    /// Sends a request and returns its answer, every message of it,
    /// skipping all else; `Err` if it failed.
    pub fn call(&mut self, make: impl FnOnce(u64) -> Message) -> Result<Vec<Message>, String> {
        let req = self.next;
        self.next += 1;
        self.conn.send(&make(req)).map_err(|e| e.to_string())?;
        let mut answer = Vec::new();
        loop {
            let message = self.recv()?;
            match message {
                Message::Failed { req: r, reason, .. } if r == req => return Err(reason),
                Message::Done { req: r, .. } if r == req => {
                    answer.push(message);
                    return Ok(answer);
                }
                Message::Entry { req: r, .. } if r == req => answer.push(message),
                _ => {}
            }
        }
    }

    /// Asks for a UDP association: the server's port, its id and key.
    pub fn open_datagram(&mut self) -> Result<(u16, [u8; 8], [u8; 32]), String> {
        let req = self.next;
        self.next += 1;
        let ask = Message::OpenDatagram { req };
        self.conn.send(&ask).map_err(|e| e.to_string())?;
        loop {
            match self.recv()? {
                Message::Datagram {
                    req: r,
                    port,
                    id,
                    key,
                } if r == req => return Ok((port, id, key.0)),
                Message::Failed { req: r, reason, .. } if r == req => return Err(reason),
                _ => {}
            }
        }
    }

    pub fn create_id(&mut self, make: impl FnOnce(u64) -> Message) -> Result<Uuid, String> {
        match self.call(make)?.last() {
            Some(Message::Done {
                created: Some(id), ..
            }) => Ok(*id),
            _ => Err("the server did not return an id".into()),
        }
    }

    /// Creates a session named `name`, a window and a block of `size`.
    pub fn create(&mut self, size: Size, name: &str) -> Result<BlockId, String> {
        let session = SessionId::from_uuid(self.create_id(|req| Message::CreateSession {
            req,
            name: name.into(),
        })?);
        let window = WindowId::from_uuid(self.create_id(|req| Message::CreateWindow {
            req,
            session,
            name: "main".into(),
        })?);
        let block = self.create_id(|req| Message::CreateBlock {
            req,
            window,
            size,
            launch: Launch::default(),
        })?;
        Ok(BlockId::from_uuid(block))
    }

    /// The window `block` is in and what it shows; with no layout,
    /// `block` alone.
    pub fn place(&mut self, block: BlockId) -> Result<(Option<WindowId>, Shape), String> {
        let entries = self.call(|req| Message::List { req })?;
        let window = entries
            .iter()
            .find_map(|m| match m {
                Message::Entry {
                    entry:
                        Entry::Block {
                            block: b, window, ..
                        },
                    ..
                } if *b == block => Some(*window),
                _ => None,
            })
            .ok_or_else(|| format!("unknown block {block}"))?;
        let mut shape = shape_of(entries, window).unwrap_or_default();
        shape.layout = shape.layout.or(Some(Layout::Block(block)));
        Ok((Some(window), shape))
    }
}

/// What a window shows, as a List tells it.
#[derive(Debug, Default)]
pub struct Shape {
    pub layout: Option<Layout>,
    pub zoomed: Option<BlockId>,
    /// The server's focus, from a server with [`Features::FOCUS`].
    pub focused: Option<BlockId>,
    pub session: Option<SessionId>,
    /// The session's active window, from a server with
    /// [`Features::ACTIVE_WINDOW`].
    pub active: Option<WindowId>,
}

/// What `window` shows, from a List's `entries`; `None` when the window
/// is gone.
pub fn shape_of(entries: Vec<Message>, window: WindowId) -> Option<Shape> {
    let mut shape = window_shape(&entries, window)?;
    shape.active = entries.into_iter().find_map(|m| match m {
        Message::Entry {
            entry: Entry::Session {
                session, active, ..
            },
            ..
        } if Some(session) == shape.session => active,
        _ => None,
    });
    Some(shape)
}

/// [`shape_of`], short of the session's active window.
pub fn window_shape(entries: &[Message], window: WindowId) -> Option<Shape> {
    entries.iter().cloned().find_map(|m| match m {
        Message::Entry {
            entry:
                Entry::Window {
                    window: w,
                    session,
                    layout,
                    zoomed,
                    focused,
                    ..
                },
            ..
        } if w == window => Some(Shape {
            layout,
            zoomed,
            focused,
            session: Some(session),
            active: None,
        }),
        _ => None,
    })
}

/// The next message on `conn`, due `by`. Each read waits at most
/// [`READ_SLICE`] and returns whatever came, so `by` and `cancel` are checked
/// between reads, however slowly a message trickles in; what came of
/// it is kept. A connection handed on waits without a deadline again.
pub fn answer(
    conn: &mut Conn<UnixStream>,
    by: Instant,
    cancel: &AtomicBool,
) -> Result<Message, String> {
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        let left = by.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("the server did not answer in time".into());
        }
        let wait = left.min(READ_SLICE);
        conn.get_ref()
            .set_read_timeout(Some(wait))
            .map_err(|e| e.to_string())?;
        let got = conn.try_recv();
        conn.get_ref()
            .set_read_timeout(None)
            .map_err(|e| e.to_string())?;
        match got {
            Ok(Some(message)) => return Ok(message),
            Ok(None) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Takes the clients in `plan` over on the fresh path at `socket`: the
/// control connection's first, with the layout asked for again, then
/// each block's. Every answer is due within [`HANDSHAKE`].
pub fn take_over(socket: PathBuf, plan: Plan, cancel: &AtomicBool) -> Result<Back, Missed> {
    take_over_with(socket, plan, CONTROL, cancel)
}

/// [`take_over`], the control connection saying Hello with `features`:
/// a client that asked for more than [`CONTROL`] keeps it on the path
/// back.
pub fn take_over_with(
    socket: PathBuf,
    plan: Plan,
    features: Features,
    cancel: &AtomicBool,
) -> Result<Back, Missed> {
    let by = Instant::now() + HANDSHAKE;
    let retry = |e: String| Missed::Retry(format!("reconnecting: {e}"));
    let stream = connect(&socket).map_err(retry)?;
    let mut conn = Conn::new(clone(&stream).map_err(retry)?);
    let writer = Conn::new(clone(&stream).map_err(retry)?);
    conn.send(&Message::Hello {
        version: Some(rusty_mux_proto::VERSION.into()),
        features,
        resume: None,
        takeover: Some(plan.me),
    })
    .map_err(|e| retry(e.to_string()))?;
    let mut req = Requests {
        conn: &mut conn,
        next: plan.next,
        by: Some(by),
        cancel,
    };
    let me = match req.welcome().map_err(retry)?.takeover {
        Some(takeover) if takeover.client == plan.me.client => takeover,
        _ => return Err(Missed::Dropped),
    };
    // Taken over: from here on, a failure ends the client.
    let (window, layout) = match plan.window {
        Some(window) => {
            let entries = req
                .call(|req| Message::List { req })
                .map_err(|why| Missed::Broken(why, me))?;
            let (window, shape) = view_back(entries, window);
            (Some(window), Some(shape))
        }
        None => (None, None),
    };
    let next = req.next;
    let panes = plan
        .resumes
        .into_iter()
        .map(|(block, resume)| {
            reach(&socket, block, Some(resume), by, cancel).map_err(|missed| match missed {
                Missed::Retry(why) | Missed::Broken(why, _) => Missed::Broken(why, me),
                Missed::Dropped => Missed::Dropped,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Back {
        socket,
        stream,
        conn,
        writer,
        me,
        next,
        window,
        layout,
        panes,
    })
}

/// The window to show on the path back, and what it shows, from the
/// path's List: `window`, or its session's active window if that moved
/// while the path was away (its event is never told).
pub fn view_back(entries: Vec<Message>, window: WindowId) -> (WindowId, Shape) {
    let shape = shape_of(entries.clone(), window).unwrap_or_default();
    let moved = shape.active.filter(|active| *active != window);
    match moved.and_then(|active| Some((active, shape_of(entries, active)?))) {
        Some(view) => view,
        None => (window, shape),
    }
}

/// A block's connection past its Welcome, its reader not started yet.
pub struct Opened {
    pub block: BlockId,
    pub stream: UnixStream,
    pub conn: Conn<UnixStream>,
    pub writer: Conn<UnixStream>,
    /// The client the server made of this connection.
    pub client: ClientId,
    pub takeover: Option<Takeover>,
    /// The client taken over, if this continues one.
    pub resumed: Option<Takeover>,
    pub features: Features,
}

/// Connects to `socket` for `block`, taking `resume`'s client over if
/// given, up to the Welcome, due `by`.
pub fn reach(
    socket: &Path,
    block: BlockId,
    resume: Option<Takeover>,
    by: Instant,
    cancel: &AtomicBool,
) -> Result<Opened, Missed> {
    let retry = Missed::Retry;
    let stream = connect(socket).map_err(retry)?;
    let mut conn = Conn::new(clone(&stream).map_err(retry)?);
    let writer = Conn::new(clone(&stream).map_err(retry)?);
    conn.send(&Message::Hello {
        version: Some(rusty_mux_proto::VERSION.into()),
        features: PANE,
        resume: None,
        takeover: resume,
    })
    .map_err(|e| retry(e.to_string()))?;
    let welcomed = welcomed(answer(&mut conn, by, cancel).map_err(retry)?).map_err(retry)?;
    match (resume, welcomed.takeover) {
        (None, _) => {}
        (Some(r), Some(t)) if t.client == r.client => {}
        (Some(_), _) => return Err(Missed::Dropped),
    }
    Ok(Opened {
        block,
        stream,
        conn,
        writer,
        client: welcomed.client,
        takeover: welcomed.takeover,
        resumed: resume,
        features: welcomed.features,
    })
}

/// What a Welcome tells, or why `message` is not one.
pub fn welcomed(message: Message) -> Result<Welcomed, String> {
    match message {
        Message::Welcome {
            features,
            client,
            token,
            version,
            ..
        } => Ok(Welcomed {
            features,
            client,
            takeover: token.map(|token| Takeover { client, token }),
            version,
        }),
        other => Err(format!("expected Welcome, got {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{BACKOFF, backoff};
    use std::time::Duration;

    #[test]
    fn backoff_doubles_up_to_thirty_seconds() {
        let s = |n| Duration::from_secs(n);
        assert_eq!(backoff(0), s(1));
        assert_eq!(backoff(1), s(2));
        assert_eq!(backoff(4), s(16));
        assert_eq!(backoff(5), BACKOFF);
        assert_eq!(backoff(40), BACKOFF, "no overflow");
    }
}
