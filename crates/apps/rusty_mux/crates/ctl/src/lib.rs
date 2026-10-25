//! `rusty-mux-ctl`: a scriptable rusty_mux client.
//!
//! Every request in [`rusty_mux_proto::schema::CATALOG`] is a subcommand and
//! each of its parameters a `--flag`, so the CLI grows with the protocol.
//! Requests joined by `+` run in order on one connection (attach, take the
//! lease, then send input), stopping at the first failure. `events` prints
//! the events after a cursor and the cursor to resume from; with `--follow`
//! it keeps printing until the server goes away.
//!
//! Output is text, or one JSON object per line with `--json`. The exit code
//! is an [`Exit`]; an interrupt ends the process with the signal, which a
//! shell reports as 130.

// Off Unix the connection code compiles but `execute` never reaches it.
#![cfg_attr(not(unix), allow(dead_code))]

pub mod cli;
mod conn;
mod render;

use std::io::{self, Write};
use std::path::PathBuf;

use rusty_mux_proto::schema::Returns;
use rusty_mux_proto::{Cursor, Features, Message};
use rusty_uuid::Uuid;

use crate::cli::{Call, Command, Invocation};
use crate::conn::Conn;

/// The process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Exit {
    Ok = 0,
    /// The server refused a request, or a stream could not be resumed.
    Refused = 1,
    /// The command line is wrong.
    Usage = 2,
    /// No server at the socket, or the connection broke.
    Unreachable = 3,
    /// The server sent something this client cannot follow.
    Protocol = 4,
}

/// Why a command stopped early.
enum Failure {
    Refused(String),
    Unreachable(io::Error),
    Protocol(String),
}

impl From<io::Error> for Failure {
    fn from(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::InvalidData => Self::Protocol(err.to_string()),
            _ => Self::Unreachable(err),
        }
    }
}

/// Runs a command line (without the program name). `env_socket` is
/// `$RUSTY_MUX_SOCKET`; output goes to `out`, diagnostics to `err`.
pub fn run(
    args: &[String],
    env_socket: Option<PathBuf>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Exit {
    let invocation = match cli::parse(args) {
        Ok(invocation) => invocation,
        Err(reason) => return report(err, Exit::Usage, &reason),
    };
    if let Command::Help(method) = invocation.command {
        return write_line(out, cli::usage(method).trim_end());
    }
    let Some(socket) = invocation.socket.clone().or(env_socket) else {
        return report(
            err,
            Exit::Usage,
            "no socket: pass --socket or set RUSTY_MUX_SOCKET",
        );
    };
    match execute(&invocation, &socket, out) {
        Ok(()) => Exit::Ok,
        Err(Failure::Refused(reason)) => report(err, Exit::Refused, &reason),
        Err(Failure::Unreachable(e)) => report(
            err,
            Exit::Unreachable,
            &format!("{}: {e}", socket.display()),
        ),
        Err(Failure::Protocol(reason)) => report(err, Exit::Protocol, &reason),
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
        Err(_) => Exit::Unreachable,
    }
}

#[cfg(unix)]
fn execute(invocation: &Invocation, socket: &PathBuf, out: &mut dyn Write) -> Result<(), Failure> {
    let mut conn = Conn::new(std::os::unix::net::UnixStream::connect(socket)?);
    let mut session = Session {
        conn: &mut conn,
        out,
        json: invocation.json,
    };
    match &invocation.command {
        Command::Calls(calls) => session.calls(calls),
        Command::Events { after, follow } => session.events(*after, *follow),
        Command::Help(_) => Ok(()),
    }
}

#[cfg(not(unix))]
fn execute(_: &Invocation, _: &PathBuf, _: &mut dyn Write) -> Result<(), Failure> {
    Err(Failure::Unreachable(io::Error::new(
        io::ErrorKind::Unsupported,
        "rusty-mux-ctl connects over a Unix socket",
    )))
}

struct Session<'a, S> {
    conn: &'a mut Conn<S>,
    out: &'a mut dyn Write,
    json: bool,
}

impl<S: io::Read + Write> Session<'_, S> {
    fn line(&mut self, line: &str) -> Result<(), Failure> {
        writeln!(self.out, "{line}").map_err(Failure::Unreachable)
    }

    /// Sends Hello and returns the Welcome's cursor.
    fn hello(
        &mut self,
        features: Features,
        resume: Option<Cursor>,
    ) -> Result<Option<Cursor>, Failure> {
        self.conn.send(&Message::Hello { features, resume })?;
        match self.conn.recv()? {
            Message::Welcome { cursor, .. } => Ok(cursor),
            other => Err(Failure::Protocol(format!(
                "expected Welcome, got {other:?}"
            ))),
        }
    }

    fn calls(&mut self, calls: &[Call]) -> Result<(), Failure> {
        // Attach needs a checkpoint format; the screen it sends is ignored.
        self.hello(Features::CHECKPOINT_V3, None)?;
        // Unanswered requests sent since the last reply; a later reply
        // means they succeeded, since the server answers in order.
        let mut unconfirmed: Vec<u64> = Vec::new();
        for (req, call) in (1..).zip(calls) {
            let message = call
                .method
                .request(req, &call.args)
                .map_err(|e| Failure::Protocol(e.to_string()))?;
            self.conn.send(&message)?;
            if call.method.returns == Returns::Unanswered {
                unconfirmed.push(req);
                continue;
            }
            let created = self.reply(req, calls, &unconfirmed)?;
            self.confirm(&mut unconfirmed, calls)?;
            self.done(call.method.name, created)?;
        }
        if !unconfirmed.is_empty() {
            // Detach is answered and harmless at the end of a connection.
            let barrier = calls.len() as u64 + 1;
            self.conn.send(&Message::Detach { req: barrier })?;
            self.reply(barrier, calls, &unconfirmed)?;
            self.confirm(&mut unconfirmed, calls)?;
        }
        Ok(())
    }

    /// Waits for `req`'s reply, failing on a refusal of it or of an
    /// unconfirmed earlier request.
    fn reply(
        &mut self,
        req: u64,
        calls: &[Call],
        unconfirmed: &[u64],
    ) -> Result<Option<Uuid>, Failure> {
        loop {
            match self.conn.recv()? {
                Message::Done { req: r, created } if r == req => return Ok(created),
                Message::Failed { req: r, reason } if r == req || unconfirmed.contains(&r) => {
                    let name = name_of(calls, r);
                    if self.json {
                        self.line(&render::failed_line(name, &reason))?;
                    }
                    return Err(Failure::Refused(format!("{name}: {reason}")));
                }
                _ => {}
            }
        }
    }

    fn confirm(&mut self, unconfirmed: &mut Vec<u64>, calls: &[Call]) -> Result<(), Failure> {
        for req in std::mem::take(unconfirmed) {
            self.done(name_of(calls, req), None)?;
        }
        Ok(())
    }

    fn done(&mut self, method: &str, created: Option<Uuid>) -> Result<(), Failure> {
        match render::done_line(method, created, self.json) {
            Some(line) => self.line(&line),
            None => Ok(()),
        }
    }

    fn events(&mut self, after: Option<Cursor>, follow: bool) -> Result<(), Failure> {
        let start = self
            .hello(Features::EVENT_SEQ, after)?
            .ok_or_else(|| Failure::Protocol("the server does not number events".into()))?;
        self.line(&render::cursor_line(start, self.json))?;
        if let Some(after) = after.filter(|a| *a != start) {
            return Err(Failure::Refused(format!(
                "cannot resume from {}: events since were lost; start again from {}",
                render::cursor_text(after),
                render::cursor_text(start)
            )));
        }
        // Without --follow, the reply to a request sent now marks the end of
        // the replay: the server sends it only after the missed events.
        if !follow {
            self.conn.send(&Message::Detach { req: 1 })?;
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
                    return Err(Failure::Protocol("an unnumbered event".into()));
                }
                Message::Done { req: 1, .. } | Message::Failed { req: 1, .. } if !follow => {
                    return self.line(&render::cursor_line(last, self.json));
                }
                _ => {}
            }
        }
    }
}

/// The method behind request number `req`; the one past the end is the
/// closing Detach.
fn name_of(calls: &[Call], req: u64) -> &'static str {
    let index = usize::try_from(req - 1).unwrap_or(usize::MAX);
    calls.get(index).map_or("detach", |c| c.method.name)
}
