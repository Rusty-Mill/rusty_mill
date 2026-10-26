//! Async wrappers around the process's standard streams: [`stdin`],
//! [`stdout`], [`stderr`].
//!
//! Like [`crate::fs::File`], stdio handles generally can't be made
//! non-blocking or registered with `epoll`/`kevent` the way a socket
//! can (this varies by platform and by what stdio is actually connected
//! to -- a terminal, a pipe, a redirected file -- so this crate doesn't
//! try to special-case any of it), so every read/write here is a
//! [`crate::spawn_blocking`] round trip onto the blocking-pool thread
//! that actually calls `std::io::Stdin`/`Stdout`/`Stderr`, rather than
//! reactor-driven.
//!
//! **Ordering.** A socket write doesn't need to worry about a
//! *different* task's write landing in the middle of its own bytes --
//! each `TcpStream` owns its own connection. Every [`Stdout`] (or
//! [`Stderr`]) instance, though, is writing to the exact same process-
//! wide stream as every other one, so two tasks' writes interleaving
//! mid-buffer would be a real, visible bug (garbled output), not just a
//! theoretical one. Fixed two ways together: [`poll_write`
//! ](crate::io::AsyncWrite::poll_write) always calls `std::io::Write::
//! write_all` internally rather than a single (possibly partial) `write`
//! -- so one `poll_write` call is always all-or-nothing, never a partial
//! count a caller's own `write_all` loop might otherwise interleave with
//! someone else's between chunks -- and each call holds a process-wide
//! [`crate::sync::Mutex`] (one per stream: stdin/stdout/stderr each get
//! their own, so a task writing to `stdout` never waits on someone
//! reading `stdin`) for its *entire* duration, including the blocking
//! call itself, not just the syscall. Together, no two concurrent
//! logical writes to the same stream can ever interleave, regardless of
//! how many underlying syscalls `write_all` itself needs.
//!
//! Unlike `File`, there's no persistent OS resource to lose if a
//! blocking closure panics -- `std::io::stdin()`/`stdout()`/`stderr()`
//! are obtained fresh on every call, not moved in and out the way
//! `fs::File`'s `std::fs::File` is -- so a panicked operation here just
//! reports an ordinary `io::Error` and leaves the handle otherwise
//! reusable, no permanent "poisoned" state needed.

use crate::io::{AsyncRead, AsyncWrite, ReadBuf};
use crate::sync::Mutex;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};

fn blocking_pool_panicked() -> io::Error {
    io::Error::other("the blocking-pool task running this stdio operation panicked")
}

/// One in-flight operation at a time per handle -- `Idle` between calls,
/// `Busy` holding the boxed future (lock acquisition, the
/// `spawn_blocking` round trip, and the actual blocking call, all
/// bundled into one opaque future rather than modeled as separate
/// poll-driven states, since nothing else needs to observe the
/// in-between steps).
enum State<T> {
    Idle,
    Busy(Pin<Box<dyn Future<Output = io::Result<T>> + Send>>),
}

/// An async handle to the process's standard input. See this module's
/// own docs for why every read is a [`crate::spawn_blocking`] round
/// trip.
pub struct Stdin {
    state: State<Vec<u8>>,
}

/// Returns a new handle to the process's standard input. Cheap to call
/// more than once -- every `Stdin` (like every `Stdout`/`Stderr`) reads
/// from the same underlying process-wide stream, serialized through a
/// shared lock (see this module's own docs).
///
/// # Panics
/// Reading from the returned handle panics if there's no running
/// [`crate::Runtime`] -- construction itself doesn't need one.
pub fn stdin() -> Stdin {
    Stdin { state: State::Idle }
}

impl AsyncRead for Stdin {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            match &mut self.state {
                State::Idle => {
                    let want = buf.remaining();
                    self.state = State::Busy(Box::pin(async move {
                        let _guard = stdin_lock().lock().await;
                        crate::spawn_blocking(move || {
                            let mut chunk = vec![0u8; want];
                            let n = std::io::Read::read(&mut std::io::stdin(), &mut chunk)?;
                            chunk.truncate(n);
                            Ok(chunk)
                        })
                        .await
                        .unwrap_or_else(|_| Err(blocking_pool_panicked()))
                    }));
                }
                State::Busy(fut) => {
                    let result = std::task::ready!(fut.as_mut().poll(cx));
                    self.state = State::Idle;
                    return Poll::Ready(result.map(|chunk| {
                        buf.unfilled_mut()[..chunk.len()].copy_from_slice(&chunk);
                        buf.advance(chunk.len());
                    }));
                }
            }
        }
    }
}

/// An async handle to the process's standard output. See this module's
/// own docs for how concurrent writes from multiple tasks avoid
/// interleaving.
pub struct Stdout {
    op: Option<InFlight>,
}

/// Returns a new handle to the process's standard output. See
/// [`stdin`]'s docs for why creating more than one is cheap and safe.
///
/// # Panics
/// Writing to the returned handle panics if there's no running
/// [`crate::Runtime`] -- construction itself doesn't need one.
pub fn stdout() -> Stdout {
    Stdout { op: None }
}

impl AsyncWrite for Stdout {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        poll_write_locked(&mut self.op, cx, buf, stdout_lock, std::io::stdout)
    }

    /// Flushes `std::io::Stdout`'s own line buffer, which otherwise holds
    /// bytes after the last newline (an LSP frame's JSON body, say) until
    /// a later write or process exit. A write still in flight (its caller
    /// may have been cancelled) is finished first, as it holds the lock.
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        poll_locked(&mut self.op, Op::Flush, cx, stdout_lock, || {
            || {
                std::io::Write::flush(&mut std::io::stdout())?;
                Ok(0)
            }
        })
        .map_ok(drop)
    }

    /// No OS-level "half-close" concept applies to stdio -- just flushes.
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}

/// An async handle to the process's standard error. See this module's
/// own docs for how concurrent writes from multiple tasks avoid
/// interleaving.
pub struct Stderr {
    op: Option<InFlight>,
}

/// Returns a new handle to the process's standard error. See
/// [`stdin`]'s docs for why creating more than one is cheap and safe.
///
/// # Panics
/// Writing to the returned handle panics if there's no running
/// [`crate::Runtime`] -- construction itself doesn't need one.
pub fn stderr() -> Stderr {
    Stderr { op: None }
}

impl AsyncWrite for Stderr {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        poll_write_locked(&mut self.op, cx, buf, stderr_lock, std::io::stderr)
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let _ = self;
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}

/// Shared by [`Stdout::poll_write`] and [`Stderr::poll_write`] -- the
/// same drive-the-boxed-future loop, parameterized only by which
/// process-wide lock and which `std::io` stream accessor to use.
fn poll_write_locked<W>(
    op: &mut Option<InFlight>,
    cx: &mut Context<'_>,
    buf: &[u8],
    lock: fn() -> &'static Mutex<()>,
    stream: fn() -> W,
) -> Poll<io::Result<usize>>
where
    W: io::Write + Send + 'static,
{
    poll_locked(op, Op::Write, cx, lock, || {
        let data = buf.to_vec();
        move || {
            std::io::Write::write_all(&mut stream(), &data)?;
            Ok(data.len())
        }
    })
}

/// Which output operation an [`InFlight`] future performs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Write,
    Flush,
}

/// The output operation a [`Stdout`] or [`Stderr`] has in flight. One slot
/// per handle: a cancelled caller leaves its operation here, still holding
/// (or queued for) the stream lock, so the next operation must finish it.
type InFlight = (Op, Pin<Box<dyn Future<Output = io::Result<usize>> + Send>>);

/// Drives one blocking stdio operation under `lock`. A different operation
/// left in flight is finished first, and its error is returned; the same
/// operation is polled again (`start` is not called). Otherwise `start`
/// builds the operation and runs it on the blocking pool.
fn poll_locked<F>(
    slot: &mut Option<InFlight>,
    op: Op,
    cx: &mut Context<'_>,
    lock: fn() -> &'static Mutex<()>,
    start: impl FnOnce() -> F,
) -> Poll<io::Result<usize>>
where
    F: FnOnce() -> io::Result<usize> + Send + 'static,
{
    if let Some((prior, fut)) = slot.as_mut() {
        if *prior != op {
            let prior = std::task::ready!(fut.as_mut().poll(cx));
            *slot = None;
            prior?;
        }
    }
    let (_, fut) = slot.get_or_insert_with(|| {
        let run = start();
        let fut = Box::pin(async move {
            let _guard = lock().lock().await;
            crate::spawn_blocking(run)
                .await
                .unwrap_or_else(|_| Err(blocking_pool_panicked()))
        });
        (op, fut)
    });
    let result = std::task::ready!(fut.as_mut().poll(cx));
    *slot = None;
    Poll::Ready(result)
}

fn stdin_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn stdout_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn stderr_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Leaves `first` in flight holding the lock, as a cancelled caller
    /// would, then checks that `second` finishes it and completes once the
    /// blocking call returns.
    fn switching_finishes_the_prior_op(first: Op, second: Op) {
        let rt = crate::Runtime::new().unwrap();
        rt.block_on(async {
            let mut slot = None;
            let (release, gate) = mpsc::channel::<()>();
            let blocked = move || {
                gate.recv().map_err(io::Error::other)?;
                Ok(1)
            };
            let waker = std::task::Waker::noop();
            let mut cx = Context::from_waker(waker);
            let polled = poll_locked(&mut slot, first, &mut cx, test_lock, || blocked);
            assert!(polled.is_pending());

            release.send(()).unwrap();
            let next = std::future::poll_fn(|cx| {
                poll_locked(&mut slot, second, cx, test_lock, || || Ok(2))
            });
            let done = crate::time::timeout(Duration::from_secs(5), next).await;
            assert!(matches!(done, Ok(Ok(2))), "stuck behind the prior op");
        });
    }

    #[test]
    fn flush_finishes_a_cancelled_write() {
        switching_finishes_the_prior_op(Op::Write, Op::Flush);
    }

    #[test]
    fn write_finishes_a_cancelled_flush() {
        switching_finishes_the_prior_op(Op::Flush, Op::Write);
    }
}
