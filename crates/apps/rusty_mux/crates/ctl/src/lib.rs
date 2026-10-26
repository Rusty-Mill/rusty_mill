//! `rusty-mux-ctl`: a scriptable rusty_mux client.
//!
//! Every request in [`rusty_mux_proto::schema::CATALOG`] is a subcommand and
//! each of its parameters a `--flag`, so the CLI grows with the protocol.
//! Requests joined by `+` run in order on one connection (attach, take the
//! lease, then send input), stopping at the first failure. `events` prints
//! the events after a cursor and the cursor to resume from; with `--follow`
//! it keeps printing until the server goes away.
//!
//! An id flag also takes a name, a position or an id suffix (see
//! [`target`]), and a missing one defaults to the block, window or session
//! the command runs in, from `$RUSTY_MUX_BLOCK` and the like.
//!
//! Output is text, or one JSON object per line with `--json`. The exit code
//! is an [`Exit`]; an interrupt ends the process with the signal, which a
//! shell reports as 130. An embedder bounds a command with [`run_until`].

// Off Unix the connection code compiles but `execute` never reaches it.
#![cfg_attr(not(unix), allow(dead_code))]

mod api;
pub mod cli;
mod conn;
mod render;
mod stop;
pub mod target;

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rusty_mux_proto::schema::{CATALOG, Method, Returns, Value};
use rusty_mux_proto::{Code, Cursor, Entry, Features, Message};
use rusty_uuid::Uuid;

pub use crate::api::input_schema;
use crate::cli::{Api, Arg, Call, Command, Defaults, Invocation};
use crate::conn::Conn;
pub use crate::stop::Stop;
use crate::target::Miss;

/// The process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Exit {
    Ok = 0,
    /// A request was refused, a stream could not be resumed, or the server
    /// sent something this client cannot follow.
    Failed = 1,
    /// The command line is wrong.
    Usage = 2,
    /// A target names nothing, or more than one thing.
    Target = 3,
    /// No server at the socket, or the connection broke.
    Unreachable = 4,
}

/// Where the command runs: the environment a block's program gets.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// `$RUSTY_MUX_SOCKET`.
    pub socket: Option<PathBuf>,
    /// `$RUSTY_MUX_SESSION`, `$RUSTY_MUX_WINDOW` and `$RUSTY_MUX_BLOCK`.
    pub here: Defaults,
}

impl Env {
    /// This process's environment. A malformed id is ignored, with a warning
    /// on stderr.
    pub fn from_process() -> Self {
        Self {
            socket: std::env::var_os("RUSTY_MUX_SOCKET").map(Into::into),
            here: Defaults {
                session: id_var("RUSTY_MUX_SESSION"),
                window: id_var("RUSTY_MUX_WINDOW"),
                block: id_var("RUSTY_MUX_BLOCK"),
            },
        }
    }
}

fn id_var(name: &str) -> Option<Uuid> {
    let value = std::env::var(name).ok()?;
    match value.parse() {
        Ok(id) => Some(id),
        Err(_) => {
            eprintln!("rusty-mux: ignoring ${name}: `{value}` is not an id");
            None
        }
    }
}

/// What a request answered with.
enum Reply {
    /// Done, with its new id if any, after the entries of a listing.
    Done {
        created: Option<Uuid>,
        entries: Vec<Entry>,
    },
    Text(String),
}

/// Why a command stopped early.
enum Failure {
    Failed(String),
    /// A target names nothing, or more than one thing.
    Missing(String),
    Unreachable(io::Error),
}

impl From<io::Error> for Failure {
    fn from(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::InvalidData => Self::Failed(err.to_string()),
            _ => Self::Unreachable(err),
        }
    }
}

impl From<Miss> for Failure {
    fn from(miss: Miss) -> Self {
        Self::Missing(match miss {
            Miss::NotFound(why) => why,
            Miss::Ambiguous(what, ids) => {
                let ids: Vec<String> = ids.iter().map(Uuid::to_string).collect();
                format!("{what} is ambiguous: {}", ids.join(", "))
            }
        })
    }
}

/// Runs a command line (without the program name) in `env`; output goes to
/// `out`, diagnostics to `err`.
pub fn run(args: &[String], env: &Env, out: &mut dyn Write, err: &mut dyn Write) -> Exit {
    run_until(args, env, &Stop::default(), out, err)
}

/// [`run`], giving up on the server once `stop` says so (past its deadline,
/// or cancelled) with [`Exit::Unreachable`]. Requests already sent stand.
pub fn run_until(
    args: &[String],
    env: &Env,
    stop: &Stop,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Exit {
    let invocation = match cli::parse(args, &env.here) {
        Ok(invocation) => invocation,
        Err(reason) => return report(err, Exit::Usage, &reason),
    };
    match invocation.command {
        Command::Help(method) => return write_line(out, cli::usage(method).trim_end()),
        Command::Api(Api::List) => return write_line(out, &api::list(invocation.json)),
        Command::Api(Api::Describe(method)) => return write_line(out, &api::describe(method)),
        Command::Calls(_) | Command::Events { .. } => {}
    }
    let Some(socket) = invocation.socket.clone().or_else(|| env.socket.clone()) else {
        return report(
            err,
            Exit::Usage,
            "no socket: pass --socket or set RUSTY_MUX_SOCKET",
        );
    };
    match execute(&invocation, &socket, env.here.session, stop, out) {
        Ok(()) => Exit::Ok,
        Err(Failure::Failed(reason)) => report(err, Exit::Failed, &reason),
        Err(Failure::Missing(reason)) => report(err, Exit::Target, &reason),
        Err(Failure::Unreachable(e)) => report(
            err,
            Exit::Unreachable,
            &format!("{}: {e}", socket.display()),
        ),
    }
}

fn report(err: &mut dyn Write, exit: Exit, reason: &str) -> Exit {
    // Nothing better to do if stderr itself is gone.
    let _ = writeln!(err, "rusty-mux-ctl: {reason}");
    exit
}

fn write_line(out: &mut dyn Write, line: &str) -> Exit {
    match writeln!(out, "{line}") {
        Ok(()) => Exit::Ok,
        Err(_) => Exit::Failed,
    }
}

#[cfg(unix)]
fn execute(
    invocation: &Invocation,
    socket: &Path,
    here: Option<Uuid>,
    stop: &Stop,
    out: &mut dyn Write,
) -> Result<(), Failure> {
    let mut conn = Conn::new(conn::connect(socket, stop)?, stop.clone());
    let mut session = Session {
        conn: &mut conn,
        out,
        json: invocation.json,
        here,
    };
    match &invocation.command {
        Command::Calls(calls) => session.calls(calls),
        Command::Events { after, follow } => session.events(*after, *follow),
        Command::Help(_) | Command::Api(_) => Ok(()),
    }
}

#[cfg(not(unix))]
fn execute(
    _: &Invocation,
    _: &Path,
    _: Option<Uuid>,
    _: &Stop,
    _: &mut dyn Write,
) -> Result<(), Failure> {
    Err(Failure::Unreachable(io::Error::new(
        io::ErrorKind::Unsupported,
        "rusty-mux-ctl connects over a Unix socket",
    )))
}

struct Session<'a, S> {
    conn: &'a mut Conn<S>,
    out: &'a mut dyn Write,
    json: bool,
    /// The session targets are looked up in when they name none.
    here: Option<Uuid>,
}

impl<S: io::Read + Write> Session<'_, S> {
    fn line(&mut self, line: &str) -> Result<(), Failure> {
        writeln!(self.out, "{line}").map_err(|e| Failure::Failed(format!("writing output: {e}")))
    }

    /// Sends Hello and returns the Welcome's features and cursor.
    fn hello(
        &mut self,
        features: Features,
        resume: Option<Cursor>,
    ) -> Result<(Features, Option<Cursor>), Failure> {
        self.conn.send(&Message::Hello { features, resume })?;
        match self.conn.recv()? {
            Message::Welcome {
                features, cursor, ..
            } => Ok((features, cursor)),
            other => Err(Failure::Failed(format!("expected Welcome, got {other:?}"))),
        }
    }

    /// Sends a Sync numbered `req`, which the server needs to support.
    fn sync(&mut self, features: Features, req: u64) -> Result<(), Failure> {
        require_sync(features)?;
        Ok(self.conn.send(&Message::Sync { req })?)
    }

    fn calls(&mut self, calls: &[Call]) -> Result<(), Failure> {
        // Everything any method needs, and sync to confirm unanswered ones.
        // The screen an attach sends is ignored.
        let wanted = CATALOG
            .iter()
            .fold(Features::SYNC, |all, m| all.union(m.needs));
        let (features, _) = self.hello(wanted, None)?;
        // Refuse a chain the server cannot run before any of it runs.
        for call in calls {
            require(features, call.method)?;
        }
        let mut reqs = 1..;
        let entries = if calls.iter().any(Call::has_target) {
            self.list(features, reqs.next().unwrap_or(u64::MAX))?
        } else {
            Vec::new()
        };
        for call in calls {
            let args = self.resolve(call, &entries)?;
            let req = reqs.next().unwrap_or(u64::MAX);
            let message = call
                .method
                .request(req, &args)
                .map_err(|e| Failure::Failed(e.to_string()))?;
            self.conn.send(&message)?;
            let name = call.method.name;
            let reply = if call.method.returns == Returns::Unanswered {
                // Learn the outcome before anything else is sent, so a
                // refusal stops the chain. Sync's reply follows any Failed.
                let barrier = reqs.next().unwrap_or(u64::MAX);
                self.sync(features, barrier)?;
                self.reply(barrier, req, name)?
            } else {
                self.reply(req, req, name)?
            };
            self.print(name, reply)?;
        }
        Ok(())
    }

    /// Every entity, for resolving targets.
    fn list(&mut self, features: Features, req: u64) -> Result<Vec<Entry>, Failure> {
        if !features.contains(Features::INSPECT) {
            return Err(Failure::Failed(
                "the server cannot list, needed to resolve a target; pass full ids".into(),
            ));
        }
        self.conn.send(&Message::List { req })?;
        match self.reply(req, req, "list")? {
            Reply::Done { entries, .. } => Ok(entries),
            Reply::Text(_) => Err(Failure::Failed("list answered with text".into())),
        }
    }

    /// `call`'s arguments with each target replaced by the id it names.
    fn resolve(
        &self,
        call: &Call,
        entries: &[Entry],
    ) -> Result<Vec<(&'static str, Value)>, Failure> {
        call.args
            .iter()
            .map(|(name, arg)| match arg {
                Arg::Value(value) => Ok((*name, value.clone())),
                Arg::Target { entity, spec } => {
                    let id = target::resolve(entries, *entity, spec, self.here)?;
                    Ok((*name, Value::Id(id)))
                }
            })
            .collect()
    }

    /// Waits for the reply to `wait`, collecting its entries, and fails if
    /// it or request `call` (the method `name`) is refused.
    fn reply(&mut self, wait: u64, call: u64, name: &str) -> Result<Reply, Failure> {
        let mut entries = Vec::new();
        loop {
            match self.conn.recv()? {
                Message::Entry { req, entry } if req == wait => entries.push(entry),
                Message::Done { req, created } if req == wait => {
                    return Ok(Reply::Done { created, entries });
                }
                Message::Text { req, text } if req == wait => return Ok(Reply::Text(text)),
                Message::Failed { req, reason, code } if req == wait || req == call => {
                    if self.json {
                        self.line(&render::failed_line(name, &reason))?;
                    }
                    let reason = format!("{name}: {reason}");
                    return Err(match code {
                        Some(Code::NotFound) => Failure::Missing(reason),
                        Some(Code::MethodNotFound) | None => Failure::Failed(reason),
                    });
                }
                _ => {}
            }
        }
    }

    fn print(&mut self, method: &str, reply: Reply) -> Result<(), Failure> {
        let lines = match reply {
            Reply::Text(text) => vec![render::text_line(method, text, self.json)],
            Reply::Done { created, entries } => {
                let mut lines: Vec<String> = entries
                    .iter()
                    .map(|e| render::entry_line(e, self.json))
                    .collect();
                lines.extend(render::done_line(method, created, self.json));
                lines
            }
        };
        lines.iter().try_for_each(|line| self.line(line))
    }

    fn events(&mut self, after: Option<Cursor>, follow: bool) -> Result<(), Failure> {
        let wanted = Features::EVENT_SEQ
            .union(Features::SYNC)
            .union(Features::STATUS);
        let (features, start) = self.hello(wanted, after)?;
        let start =
            start.ok_or_else(|| Failure::Failed("the server does not number events".into()))?;
        self.line(&render::cursor_line(start, self.json))?;
        if let Some(after) = after.filter(|a| *a != start) {
            return Err(Failure::Failed(format!(
                "cannot resume from {}: events since were lost; start again from {}",
                render::cursor_text(after),
                render::cursor_text(start)
            )));
        }
        // Without --follow, the reply to a Sync sent now marks the end of
        // the replay: the server sends it only after the missed events.
        if !follow {
            self.sync(features, 1)?;
        }
        let mut last = start;
        loop {
            match self.conn.recv()? {
                Message::Event {
                    seq: Some(seq),
                    event,
                } => {
                    self.line(&render::event_line(seq, &event, self.json))?;
                    last.seq = seq;
                }
                Message::Event { seq: None, .. } => {
                    return Err(Failure::Failed("an unnumbered event".into()));
                }
                Message::Done { req: 1, .. } | Message::Failed { req: 1, .. } if !follow => {
                    return self.line(&render::cursor_line(last, self.json));
                }
                _ => {}
            }
        }
    }
}

/// Fails unless the server can run `method` as ctl sends it.
fn require(features: Features, method: &Method) -> Result<(), Failure> {
    if method.returns == Returns::Unanswered {
        require_sync(features)?;
    }
    if features.contains(method.needs) {
        return Ok(());
    }
    Err(Failure::Failed(format!(
        "the server does not support {}",
        method.name
    )))
}

fn require_sync(features: Features) -> Result<(), Failure> {
    if features.contains(Features::SYNC) {
        return Ok(());
    }
    Err(Failure::Failed(
        "the server does not support sync, needed to confirm input".into(),
    ))
}
