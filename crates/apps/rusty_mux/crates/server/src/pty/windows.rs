//! Windows: a block's console and the program running on it.
//!
//! The console is ConPTY's pseudo console, or one we host ourselves
//! (`rusty_conhost`), as [`Config::console_host`] says. Neither master can
//! be watched by a reactor (ConPTY's is two anonymous pipes, ours a
//! blocking queue), so each block gets a reader thread and a writer
//! thread. Neither uses the runtime's blocking pool: a read blocks until
//! the program prints, which would pin a pool thread for the block's whole
//! life.
//!
//! The reader drains the output for as long as the console exists,
//! discarding once the block is released, so `ClosePseudoConsole` never
//! blocks on a full pipe and our host never stalls its clients. A ConPTY
//! close runs on a thread of its own, under the master's close guard,
//! never on the server's thread; our host's hang-up returns at once.

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::io;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use platform::error::Result as PlatformResult;
use platform::process::{Child, Signal};
use platform::pty::PtyMaster;
use platform::term::WinSize;
use platform_windows::{WindowsChild, WindowsPty, WindowsPtyMaster};
use rusty_conhost::{OwnPty, OwnPtyMaster};
use rusty_mux_core::{ExitStatus, Size};
use rusty_mux_proto::Echo;
use rusty_tokio::sync::{mpsc, oneshot};

use super::{GRACE, POLL, command, exit_status, to_io, win_size};
use crate::{Config, ConsoleHost};

/// Bytes per read from the output pipe.
const CHUNK: usize = 16 << 10;
/// Chunks the reader may get ahead of the server: the back pressure an
/// unread Linux PTY gives.
const QUEUE: usize = 4;
/// Stack size of the per-block threads; they only copy bytes.
const STACK: usize = 64 << 10;

type Chunk = io::Result<Vec<u8>>;
type Write = (Vec<u8>, oneshot::Sender<io::Result<()>>);

/// The terminal's side of a block's console, on whichever host runs it.
enum Master {
    ConPty(WindowsPtyMaster),
    Own(OwnPtyMaster),
}

impl Master {
    fn read(&self, buf: &mut [u8]) -> PlatformResult<usize> {
        match self {
            Self::ConPty(m) => m.read(buf),
            Self::Own(m) => m.read(buf),
        }
    }

    fn write(&self, buf: &[u8]) -> PlatformResult<usize> {
        match self {
            Self::ConPty(m) => m.write(buf),
            Self::Own(m) => m.write(buf),
        }
    }

    fn resize(&self, size: WinSize) -> PlatformResult<()> {
        match self {
            Self::ConPty(m) => m.resize(size),
            Self::Own(m) => m.resize(size),
        }
    }
}

/// The master side of a block's console. Shared by the reader and by
/// writers on the server's thread.
pub(crate) struct Pty {
    master: Arc<Master>,
    /// Output from the reader thread; `None` while a read awaits it and
    /// once released.
    output: Cell<Option<mpsc::Receiver<Chunk>>>,
    released: Cell<bool>,
    /// The part of the last chunk a smaller read buffer did not take, and
    /// how much of it has been served.
    rest: RefCell<(Vec<u8>, usize)>,
    writes: mpsc::UnboundedSender<Write>,
}

/// The output receiver while a read awaits it. Dropping it, when the read
/// ends or is cancelled, hands the receiver back unless the block was
/// released meanwhile, so a released block's receiver is dropped.
struct Lent<'a> {
    pty: &'a Pty,
    output: Option<mpsc::Receiver<Chunk>>,
}

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        if !self.pty.released.get() {
            self.pty.output.set(self.output.take());
        }
    }
}

/// A block's program, owned until it is reaped.
pub(crate) struct Process {
    child: Option<WindowsChild>,
    master: Arc<Master>,
}

/// Starts `config`'s program on a new console of `size`, on the host
/// `config` names, with this process's environment plus `env`. Our host
/// failing (`Unsupported` on a Windows without a usable ConDrv) is an
/// error, never a fall back to ConPTY.
pub(crate) fn spawn(
    config: &Config,
    size: Size,
    env: &[(&str, OsString)],
) -> io::Result<(Pty, Process)> {
    let cmd = command(config, env);
    let size = win_size(size);
    let (master, child) = match config.console_host {
        ConsoleHost::ConPty => {
            let (master, child) = WindowsPty.spawn_concrete(&cmd, size).map_err(to_io)?;
            (Master::ConPty(master), child)
        }
        ConsoleHost::Own => {
            let (master, child) = OwnPty.spawn_concrete(&cmd, size).map_err(to_io)?;
            (Master::Own(master), child)
        }
    };
    let master = Arc::new(master);
    // Owned before anything else can fail, so an error below still ends it.
    let process = Process {
        child: Some(child),
        master: Arc::clone(&master),
    };
    let (chunks, output) = mpsc::channel(QUEUE);
    let reader = Arc::clone(&master);
    start("rusty-mux-pty-read", move || read_loop(&reader, &chunks))?;
    let (writes, queue) = mpsc::unbounded_channel();
    let writer = Arc::clone(&master);
    start("rusty-mux-pty-write", move || write_loop(&writer, queue))?;
    let pty = Pty {
        master,
        output: Cell::new(Some(output)),
        released: Cell::new(false),
        rest: RefCell::new((Vec::new(), 0)),
        writes,
    };
    Ok((pty, process))
}

fn start(name: &str, body: impl FnOnce() + Send + 'static) -> io::Result<()> {
    thread::Builder::new()
        .name(name.to_owned())
        .stack_size(STACK)
        .spawn(body)
        .map(drop)
}

/// Forwards output until the console closes; once the receiver is
/// gone, keeps reading and discards.
fn read_loop(master: &Master, chunks: &mpsc::Sender<Chunk>) {
    let mut buf = vec![0; CHUNK];
    let mut forwarding = true;
    loop {
        let chunk = match master.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => Ok(buf[..n].to_vec()),
            Err(err) => Err(to_io(err)),
        };
        let failed = chunk.is_err();
        if forwarding {
            forwarding = chunks.blocking_send(chunk).is_ok();
        }
        if failed {
            return;
        }
    }
}

/// Writes each queued input until every sender is gone.
fn write_loop(master: &Master, mut queue: mpsc::UnboundedReceiver<Write>) {
    while let Some((data, done)) = queue.blocking_recv() {
        // A refused result means the writer stopped waiting for it.
        let _ = done.send(write_all(master, &data));
    }
}

fn write_all(master: &Master, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        match master.write(data).map_err(to_io)? {
            0 => return Err(io::ErrorKind::WriteZero.into()),
            n => data = &data[n..],
        }
    }
    Ok(())
}

/// Closes the console. ConPTY's close runs on a thread of its own, since it
/// can block until the reader has drained conhost's last output; ours
/// returns at once, and the host closes by its own grace deadline.
fn hang_up(master: &Arc<Master>) {
    match &**master {
        Master::Own(own) => own.hang_up(),
        Master::ConPty(_) => {
            let master = Arc::clone(master);
            let close = move || {
                if let Master::ConPty(conpty) = &*master {
                    conpty.hang_up();
                }
            };
            if let Err(err) = start("rusty-mux-pty-close", close) {
                eprintln!("rusty-mux-server: starting a hang-up thread: {err}");
            }
        }
    }
}

fn kill(child: &WindowsChild) {
    if let Err(err) = child.kill_single(Signal::Kill) {
        eprintln!("rusty-mux-server: signalling pid {}: {err:?}", child.id());
    }
}

impl Process {
    /// The program's process id, until it is reaped.
    pub(crate) fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|child| child.id())
    }

    /// Closes the console, which sends every program attached to it
    /// Windows' console-close notification, and waits for the program to
    /// exit, terminating it after [`GRACE`]. Our host also ends any other
    /// program still attached 5 s after the notification. Runs as its own
    /// task so the server keeps serving meanwhile.
    /// How it ended, or `None` if that can't be known.
    pub(crate) async fn reap(mut self) -> Option<ExitStatus> {
        let child = self.child.as_mut()?;
        hang_up(&self.master);
        let deadline = Instant::now() + GRACE;
        let mut killed = false;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return exit_status(status),
                Ok(None) => {}
                Err(err) => {
                    // The child stays in self, so `Drop` still ends it.
                    eprintln!("rusty-mux-server: polling pid {}: {err:?}", child.id());
                    return None;
                }
            }
            if !killed && Instant::now() >= deadline {
                kill(child);
                killed = true;
            }
            rusty_tokio::time::sleep(POLL).await;
        }
    }
}

impl Drop for Process {
    /// The fallback when a block is dropped without [`Process::reap`], for
    /// instance when the server stops: terminate the program and close the
    /// console, without waiting for either.
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        kill(&child);
        hang_up(&self.master);
    }
}

impl Pty {
    /// Reads output; `Ok(0)` means the console closed or the block
    /// was released. Cancelling it loses nothing.
    pub(crate) async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(n) = self.take_rest(buf) {
            return Ok(n);
        }
        // Taken out for the await, so no borrow spans it, and put back when
        // the read ends or is cancelled.
        let Some(output) = self.output.take() else {
            return Ok(0);
        };
        let mut lent = Lent {
            pty: self,
            output: Some(output),
        };
        let Some(output) = lent.output.as_mut() else {
            return Ok(0);
        };
        match output.recv().await {
            None => Ok(0),
            Some(chunk) => self.serve(chunk, buf),
        }
    }

    /// Reads output without waiting; `None` means the reader thread has
    /// handed over nothing yet. That is weaker than a drained Linux master,
    /// since the thread may be inside a read, but no client predicts echo
    /// here: the mode is always [`Echo::Unknown`].
    pub(crate) fn try_read(&self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        if let Some(n) = self.take_rest(buf) {
            return Ok(Some(n));
        }
        let Some(mut output) = self.output.take() else {
            return Ok(Some(0));
        };
        let chunk = output.try_recv();
        if !self.released.get() {
            self.output.set(Some(output));
        }
        match chunk {
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Ok(Some(0)),
            Ok(chunk) => self.serve(chunk, buf).map(Some),
        }
    }

    /// Copies a chunk into `buf`, keeping what does not fit for the next read.
    fn serve(&self, chunk: Chunk, buf: &mut [u8]) -> io::Result<usize> {
        let data = chunk?;
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        if n < data.len() {
            *self.rest.borrow_mut() = (data, n);
        }
        Ok(n)
    }

    fn take_rest(&self, buf: &mut [u8]) -> Option<usize> {
        let mut rest = self.rest.borrow_mut();
        let (data, served) = &mut *rest;
        if *served >= data.len() {
            return None;
        }
        let n = (data.len() - *served).min(buf.len());
        buf[..n].copy_from_slice(&data[*served..*served + n]);
        *served += n;
        Some(n)
    }

    /// Returns once `data` is written, or the write failed; `BrokenPipe`
    /// once the console is gone.
    ///
    /// Input is committed when the server accepts it under the lease
    /// (`State::input`), not when this future completes: dropping the
    /// future, as evicting its client does, leaves the queued write to
    /// finish whole and in order. Linux instead stops at whatever prefix
    /// the kernel had taken, which may split a key sequence. Each
    /// connection awaits one write at a time, so an evicted client leaves at
    /// most one message behind, and it reaches the program ahead of any
    /// input a later lease holder sends.
    pub(crate) async fn write_all(&self, data: &[u8]) -> io::Result<()> {
        let broken = || io::Error::from(io::ErrorKind::BrokenPipe);
        let (done, result) = oneshot::channel();
        self.writes
            .send((data.to_vec(), done))
            .map_err(|_| broken())?;
        result.await.map_err(|_| broken())?
    }

    /// Fails once the console is closed, or while it is closing.
    pub(crate) fn resize(&self, size: Size) -> io::Result<()> {
        self.master.resize(win_size(size)).map_err(to_io)
    }

    /// A Windows console has no termios to tell.
    pub(crate) fn echo(&self) -> Echo {
        Echo::Unknown
    }

    /// Unknown: a Windows console has no foreground process group.
    pub(crate) fn foreground(&self) -> Option<u32> {
        None
    }

    /// Stops forwarding output at once, whatever other handles to this
    /// `Pty` remain: the reader then discards, so the console's pipe never
    /// fills while it is being closed.
    pub(crate) fn release(&self) {
        self.released.set(true);
        drop(self.output.take());
    }
}
