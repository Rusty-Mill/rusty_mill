//! The [`Workspace`] adapter on std's filesystem and processes.
//!
//! Every path is resolved (symlinks followed) and refused when its real
//! location is outside the root. That check and the later I/O are not atomic,
//! so a process racing to swap a symlink could slip past it; the jail
//! (Landlock, the mount namespace) is the boundary, and this is the second
//! fence in front of it.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::core::{Entry, ExecOutcome, ExecSpec, Kind, Workspace, OUTPUT_CAP};
use crate::path::SessionPath;

/// How often a running command is checked for exit.
const POLL: Duration = Duration::from_millis(10);
/// How long output is still collected after the command ends, for a
/// grandchild that holds the pipes open.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// One session's root on the local disk.
#[derive(Debug, Clone)]
pub struct FsWorkspace {
    root: PathBuf,
}

impl FsWorkspace {
    /// Serve `root`, which must be an existing directory.
    ///
    /// # Errors
    /// `root` does not exist or is not a directory.
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err(io::Error::other(format!(
                "{} is not a directory",
                root.display()
            )));
        }
        Ok(FsWorkspace { root })
    }

    /// The real location of an existing `path`, refused if outside the root.
    fn resolve(&self, path: &SessionPath) -> Result<PathBuf, String> {
        let real = self
            .root
            .join(path.as_path())
            .canonicalize()
            .map_err(|e| failed(path, &e))?;
        self.inside(path, real)
    }

    /// Where a write to `path` lands: the resolved target if it exists, else
    /// the resolved parent joined with the file name.
    fn resolve_for_write(&self, path: &SessionPath) -> Result<PathBuf, String> {
        let full = self.root.join(path.as_path());
        if fs::symlink_metadata(&full).is_ok() {
            return self.resolve(path);
        }
        let (Some(parent), Some(name)) = (full.parent(), full.file_name()) else {
            return Err(format!("{}: names no file", shown(path)));
        };
        let parent = parent.canonicalize().map_err(|e| failed(path, &e))?;
        self.inside(path, parent.join(name))
    }

    fn inside(&self, path: &SessionPath, real: PathBuf) -> Result<PathBuf, String> {
        if real.starts_with(&self.root) {
            return Ok(real);
        }
        Err(format!(
            "{}: resolves outside the session; refused",
            shown(path)
        ))
    }
}

fn shown(path: &SessionPath) -> String {
    if path.is_root() {
        return ".".to_owned();
    }
    path.as_path().display().to_string()
}

fn failed(path: &SessionPath, err: &io::Error) -> String {
    format!("{}: {err}", shown(path))
}

fn kind(ft: fs::FileType) -> Kind {
    if ft.is_symlink() {
        Kind::Symlink
    } else if ft.is_dir() {
        Kind::Dir
    } else if ft.is_file() {
        Kind::File
    } else {
        Kind::Other
    }
}

impl Workspace for FsWorkspace {
    fn read(&self, path: &SessionPath, cap: usize) -> Result<(Vec<u8>, bool), String> {
        let real = self.resolve(path)?;
        let file = File::open(real).map_err(|e| failed(path, &e))?;
        let mut bytes = Vec::new();
        file.take(cap as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| failed(path, &e))?;
        let truncated = bytes.len() > cap;
        bytes.truncate(cap);
        Ok((bytes, truncated))
    }

    fn list(&self, path: &SessionPath) -> Result<Vec<Entry>, String> {
        let real = self.resolve(path)?;
        let mut entries = Vec::new();
        for item in fs::read_dir(real).map_err(|e| failed(path, &e))? {
            let item = item.map_err(|e| failed(path, &e))?;
            let meta = item.metadata().map_err(|e| failed(path, &e))?;
            entries.push(Entry {
                name: item.file_name().to_string_lossy().into_owned(),
                kind: kind(meta.file_type()),
                size: meta.len(),
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    fn stat(&self, path: &SessionPath) -> Result<Entry, String> {
        let real = self.resolve(path)?;
        let meta = fs::metadata(real).map_err(|e| failed(path, &e))?;
        Ok(Entry {
            name: shown(path),
            kind: kind(meta.file_type()),
            size: meta.len(),
        })
    }

    fn write(&self, path: &SessionPath, bytes: &[u8]) -> Result<(), String> {
        let real = self.resolve_for_write(path)?;
        fs::write(real, bytes).map_err(|e| failed(path, &e))
    }

    fn exec(&self, spec: &ExecSpec) -> Result<ExecOutcome, String> {
        let cwd = self.resolve(&spec.cwd)?;
        let (program, rest) = spec.argv.split_first().ok_or("`argv` is empty")?;
        let mut child = Command::new(program)
            .args(rest)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start `{program}`: {e}"))?;
        feed(&mut child, spec.stdin.clone());
        let stdout = Capture::start(child.stdout.take());
        let stderr = Capture::start(child.stderr.take());
        let (status, timed_out) = wait(&mut child, spec.timeout)?;
        let deadline = Instant::now() + DRAIN_GRACE;
        let (stdout, cut_out) = stdout.finish(deadline);
        let (stderr, cut_err) = stderr.finish(deadline);
        Ok(ExecOutcome {
            status,
            timed_out,
            stdout,
            stderr,
            truncated: cut_out || cut_err,
        })
    }
}

/// Write `bytes` to the child's stdin on a thread, then close it. A child
/// that exits without reading makes the write fail; that is not an error.
fn feed(child: &mut Child, bytes: Vec<u8>) {
    let Some(mut stdin) = child.stdin.take() else {
        return;
    };
    thread::spawn(move || {
        let _ignored_broken_pipe = stdin.write_all(&bytes);
    });
}

/// Wait for exit, killing the child at `timeout`. Answers the exit code
/// (`None` when killed) and whether it timed out.
fn wait(child: &mut Child, timeout: Duration) -> Result<(Option<i32>, bool), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let polled = child.try_wait().map_err(|e| format!("wait failed: {e}"))?;
        if let Some(status) = polled {
            return Ok((status.code(), false));
        }
        if Instant::now() >= deadline {
            // It may exit on its own between the poll and the kill.
            let _already_exited = child.kill();
            let status = child.wait().map_err(|e| format!("wait failed: {e}"))?;
            return Ok((status.code(), true));
        }
        thread::sleep(POLL);
    }
}

/// One output stream read on its own thread, at most [`OUTPUT_CAP`] bytes
/// kept; the rest is read and dropped so the child never blocks on a full
/// pipe.
struct Capture {
    kept: Arc<Mutex<(Vec<u8>, bool)>>,
    done: Option<Receiver<()>>,
}

/// A stream a [`Capture`] can read.
trait Stream: Read + Send + 'static {}
impl Stream for ChildStdout {}
impl Stream for ChildStderr {}

impl Capture {
    fn start(stream: Option<impl Stream>) -> Self {
        let kept = Arc::new(Mutex::new((Vec::new(), false)));
        let Some(stream) = stream else {
            return Capture { kept, done: None };
        };
        let (tx, rx) = mpsc::channel();
        let shared = Arc::clone(&kept);
        thread::spawn(move || {
            pump(stream, &shared);
            let _receiver_gone = tx.send(());
        });
        Capture {
            kept,
            done: Some(rx),
        }
    }

    /// What was read, waiting for the stream to end until `deadline`.
    fn finish(self, deadline: Instant) -> (Vec<u8>, bool) {
        if let Some(done) = self.done {
            let left = deadline.saturating_duration_since(Instant::now());
            let _timed_out_or_done = done.recv_timeout(left);
        }
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        (std::mem::take(&mut kept.0), kept.1)
    }
}

fn pump(mut stream: impl Read, kept: &Mutex<(Vec<u8>, bool)>) {
    let mut buf = [0u8; 8192];
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            // The child closed or broke the pipe: the stream is over.
            Err(_) => return,
        };
        let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
        let room = OUTPUT_CAP.saturating_sub(kept.0.len());
        let take = n.min(room);
        kept.0.extend_from_slice(&buf[..take]);
        kept.1 |= take < n;
    }
}

#[cfg(test)]
mod tests;
