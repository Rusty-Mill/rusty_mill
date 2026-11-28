//! The pure core: calls in, answers out, no I/O.
//!
//! [`handle`] turns one tool call into a request, checks it against the role
//! and the caps, runs it on a [`Workspace`], and shapes the answer as JSON text
//! the model can read. Every failure the model can act on (a refused path, a
//! missing file, a timeout) is a tool error, not a protocol error.

use std::time::Duration;

use rusty_mcp_server::json::Value;

use crate::path::SessionPath;

/// Most bytes returned per stream (file contents, stdout, stderr).
pub const OUTPUT_CAP: usize = 1024 * 1024;
/// Longest an `exec` may run.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(600);
/// What an `exec` gets when it names no timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// What the session may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Reads, writes and runs in its worktree.
    Builder,
    /// Reads and runs, without the `write` tool. That is all this role
    /// withholds: `exec` runs with the server's own rights, so a critic's
    /// commands can still write. A read-only critic needs the jail to make
    /// its view read-only (726-sandbox-design §7); this crate does not.
    Critic,
}

/// What a directory entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link (not followed by `list`).
    Symlink,
    /// Anything else (socket, device, fifo).
    Other,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Dir => "dir",
            Kind::Symlink => "symlink",
            Kind::Other => "other",
        }
    }
}

/// One entry of a listing, or the answer to `stat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its name (for `stat`, the path asked about).
    pub name: String,
    /// What it is.
    pub kind: Kind,
    /// Its size in bytes.
    pub size: u64,
}

/// A command to run in the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecSpec {
    /// Program and arguments; never empty.
    pub argv: Vec<String>,
    /// Working directory, inside the session.
    pub cwd: SessionPath,
    /// Wall-clock budget, at most [`MAX_TIMEOUT`].
    pub timeout: Duration,
    /// Bytes written to its stdin, which is then closed.
    pub stdin: Vec<u8>,
}

/// How a command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutcome {
    /// The exit code; `None` when it was killed (timeout, signal).
    pub status: Option<i32>,
    /// Whether it ran out of time and was killed.
    pub timed_out: bool,
    /// Its stdout, at most [`OUTPUT_CAP`] bytes.
    pub stdout: Vec<u8>,
    /// Its stderr, at most [`OUTPUT_CAP`] bytes.
    pub stderr: Vec<u8>,
    /// Whether either stream was cut at the cap.
    pub truncated: bool,
}

/// The port to the disk and processes. Every path is already a
/// [`SessionPath`]; the adapter still refuses one that resolves outside the
/// root.
pub trait Workspace {
    /// A file's bytes, at most `cap`, and whether there were more.
    ///
    /// # Errors
    /// The I/O failure, as text the model can read.
    fn read(&self, path: &SessionPath, cap: usize) -> Result<(Vec<u8>, bool), String>;
    /// A directory's entries, sorted by name.
    ///
    /// # Errors
    /// As [`Workspace::read`].
    fn list(&self, path: &SessionPath) -> Result<Vec<Entry>, String>;
    /// What `path` is.
    ///
    /// # Errors
    /// As [`Workspace::read`].
    fn stat(&self, path: &SessionPath) -> Result<Entry, String>;
    /// Create or replace a file; its directory must exist.
    ///
    /// # Errors
    /// As [`Workspace::read`].
    fn write(&self, path: &SessionPath, bytes: &[u8]) -> Result<(), String>;
    /// Run a command to completion or its timeout.
    ///
    /// # Errors
    /// The command could not be started.
    fn exec(&self, spec: &ExecSpec) -> Result<ExecOutcome, String>;
}

/// The tools a role is offered, by name.
pub fn tools(role: Role) -> &'static [&'static str] {
    match role {
        Role::Builder => &["read", "list", "stat", "write", "exec"],
        Role::Critic => &["read", "list", "stat", "exec"],
    }
}

/// An answer: JSON text, and whether it is an error the model should see.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// The JSON body.
    pub body: Value,
    /// Whether the call failed.
    pub is_error: bool,
}

impl Answer {
    fn ok(body: Value) -> Self {
        Answer {
            body,
            is_error: false,
        }
    }

    fn error(message: impl Into<String>) -> Self {
        let mut body = Value::object();
        body.insert("error", message.into());
        Answer {
            body,
            is_error: true,
        }
    }
}

/// Run the tool `name` with `args` for `role` on `ws`.
pub fn handle(ws: &impl Workspace, role: Role, name: &str, args: Option<&Value>) -> Answer {
    if !tools(role).contains(&name) {
        return Answer::error(format!("`{name}` is not offered to this session"));
    }
    let result = match name {
        "read" => read(ws, args),
        "list" => list(ws, args),
        "stat" => stat(ws, args),
        "write" => write(ws, args),
        "exec" => exec(ws, args),
        _ => Err(format!("unknown tool `{name}`")),
    };
    result.map_or_else(Answer::error, Answer::ok)
}

fn arg<'a>(args: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    args?.get(key)
}

fn text<'a>(args: Option<&'a Value>, key: &str) -> Result<&'a str, String> {
    arg(args, key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{key}` must be a string"))
}

fn path(args: Option<&Value>, key: &str, default_root: bool) -> Result<SessionPath, String> {
    match arg(args, key) {
        None if default_root => Ok(SessionPath::root()),
        _ => SessionPath::parse(text(args, key)?).map_err(|e| e.to_string()),
    }
}

/// Bytes as text when they are UTF-8; otherwise lossily, flagged.
fn put_bytes(body: &mut Value, key: &str, bytes: &[u8]) {
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            body.insert(key, text);
        }
        Err(_) => {
            body.insert(key, String::from_utf8_lossy(bytes).into_owned());
            body.insert(format!("{key}_lossy"), true);
        }
    }
}

fn read(ws: &impl Workspace, args: Option<&Value>) -> Result<Value, String> {
    let path = path(args, "path", false)?;
    let (bytes, truncated) = ws.read(&path, OUTPUT_CAP)?;
    let mut body = Value::object();
    put_bytes(&mut body, "content", &bytes);
    body.insert("truncated", truncated);
    Ok(body)
}

fn entry(entry: &Entry) -> Value {
    let mut v = Value::object();
    v.insert("name", entry.name.clone());
    v.insert("kind", entry.kind.name());
    v.insert("size", entry.size);
    v
}

fn list(ws: &impl Workspace, args: Option<&Value>) -> Result<Value, String> {
    let path = path(args, "path", true)?;
    let mut entries = Value::array();
    for e in ws.list(&path)? {
        entries.push(entry(&e));
    }
    let mut body = Value::object();
    body.insert("entries", entries);
    Ok(body)
}

fn stat(ws: &impl Workspace, args: Option<&Value>) -> Result<Value, String> {
    let path = path(args, "path", false)?;
    Ok(entry(&ws.stat(&path)?))
}

fn write(ws: &impl Workspace, args: Option<&Value>) -> Result<Value, String> {
    let path = path(args, "path", false)?;
    if path.is_root() {
        return Err("`path` must name a file".to_owned());
    }
    let content = text(args, "content")?;
    if content.len() > OUTPUT_CAP {
        return Err(format!("`content` is over {OUTPUT_CAP} bytes"));
    }
    ws.write(&path, content.as_bytes())?;
    let mut body = Value::object();
    body.insert("written", content.len());
    Ok(body)
}

fn exec_spec(args: Option<&Value>) -> Result<ExecSpec, String> {
    let argv: Vec<String> = arg(args, "argv")
        .and_then(Value::as_array)
        .ok_or("`argv` must be a list of strings")?
        .iter()
        .map(|a| a.as_str().map(str::to_owned))
        .collect::<Option<_>>()
        .ok_or("`argv` must be a list of strings")?;
    if argv.first().is_none_or(String::is_empty) {
        return Err("`argv` must name a program".to_owned());
    }
    let timeout = match arg(args, "timeout_s") {
        None => DEFAULT_TIMEOUT,
        Some(v) => {
            let secs = v
                .as_u64()
                .filter(|s| *s > 0)
                .ok_or("`timeout_s` must be a positive whole number")?;
            Duration::from_secs(secs).min(MAX_TIMEOUT)
        }
    };
    let stdin = match arg(args, "stdin") {
        None => Vec::new(),
        Some(_) => text(args, "stdin")?.as_bytes().to_vec(),
    };
    Ok(ExecSpec {
        argv,
        cwd: path(args, "cwd", true)?,
        timeout,
        stdin,
    })
}

fn exec(ws: &impl Workspace, args: Option<&Value>) -> Result<Value, String> {
    let spec = exec_spec(args)?;
    let out = ws.exec(&spec)?;
    let mut body = Value::object();
    match out.status {
        Some(code) => body.insert("status", i64::from(code)),
        None => body.insert("status", Value::Null),
    };
    body.insert("timed_out", out.timed_out);
    put_bytes(&mut body, "stdout", &out.stdout);
    put_bytes(&mut body, "stderr", &out.stderr);
    body.insert("truncated", out.truncated);
    Ok(body)
}

#[cfg(test)]
mod tests;
