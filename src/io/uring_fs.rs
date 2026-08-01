//! Real io_uring-backed positional file I/O (Linux only, behind the
//! `io-uring-fs` feature): [`UringFile`], separate from [`crate::fs::File`]
//! (which stays 100% [`crate::spawn_blocking`] -- see that module's own
//! docs for why). Independent of [`super::reactor::io_uring`] (the
//! `io-uring-reactor` feature) too: that one only replaces `epoll_wait`
//! with `IORING_OP_POLL_ADD` for *socket* readiness and never touches a
//! file; this module owns its own separate ring, dedicated to real
//! `read`/`write`/`fsync`/`openat`/... opcodes instead. A build can
//! enable either feature, neither, or both -- they share nothing.
//!
//! ## Why owned buffers, not `&mut [u8]`
//!
//! An io_uring read/write hands the kernel a raw pointer into a
//! caller-owned buffer for the *entire duration* of the operation -- but
//! an ordinary Rust `Future` can be dropped (cancelled) at any `Pending`
//! point. Submit a real io_uring read into a borrowed stack buffer, then
//! drop that future before the kernel completes it (a timeout, a
//! `select!`-style race, plain cancellation), and the kernel can still
//! write into memory that's since been freed or reused -- a genuine
//! use-after-free. [`IoBuf`]/[`IoBufMut`] (the same shape
//! `tokio-uring`/`monoio`/`compio` all use) fix this by moving the
//! buffer's *ownership* into the in-flight operation instead of merely
//! borrowing it: [`IoUringDriver::submit`] boxes the buffer into the
//! op's own [`OpState`] before the kernel ever sees a pointer into it,
//! and that `OpState` -- not the `Future` polling it -- is what the
//! driver thread keeps alive.
//!
//! ## The cancellation-safety invariant, precisely
//!
//! Every in-flight op has exactly one [`OpState`], reachable from two
//! places: the driver's own `slab` (inserted at submission, removed only
//! once the real completion queue entry for it arrives) and the
//! [`OpFuture`] polling it (which holds its own `Arc<OpState>` clone).
//! `OpState.buf` -- the boxed, submitted buffer -- is only ever read or
//! dropped in two places:
//!
//! 1. [`OpFuture::poll`], once it observes `OpState.result` is `Some` (a
//!    real completion already landed) -- it takes the buffer out and
//!    hands it back to the caller.
//! 2. The driver thread's own completion loop
//!    ([`IoUringDriver::event_loop`]), which removes the `OpState` from
//!    `slab` *after* recording the real completion result. If the
//!    `Future` was already dropped by then (cancelled), the `slab`'s
//!    reference was the last one -- dropping it there drops
//!    `OpState.buf` with it, freeing the buffer only now, after the
//!    kernel has genuinely finished writing into it, never before.
//!
//! `OpFuture`'s own `Drop` is the compiler-generated default: it does
//! nothing to `OpState.buf` at all. That's the entire cancellation-safety
//! argument -- there is no code path, anywhere, that touches (reads,
//! moves, frees, or hands back to the caller) a submitted buffer before a
//! real completion queue entry names it done. A completion that arrives
//! *after* the `Future` was dropped hits exactly the same "record result,
//! wake if anyone's still listening, then let the `Arc` refcount decide
//! whether the buffer is freed here or later" path -- no special-casing,
//! no way to double-free or panic on a stale reference.
//! `tests/uring_fs_cancellation.rs` (ASAN-checked; see that file's own
//! docs for why Miri can't run it) holds this to that bar directly: it
//! drops an in-flight `read_at`/`write_at` before completion and confirms
//! no UB, then confirms a completion that arrives afterward doesn't panic
//! or touch freed memory either.
//!
//! ## One ring, shared by every caller
//!
//! Unlike `io::reactor` (one instance per [`crate::Runtime`], or -- with
//! the `thread-per-core` feature -- one per core), there's exactly one
//! [`IoUringDriver`] for the whole process, lazily started on first use
//! (see [`global_driver`]). A single dedicated thread owns the ring
//! exclusively once started (the same "no fight with a `Mutex`-guarded
//! ring on every op" design [`super::reactor::io_uring`] already uses)
//! -- other threads only ever queue a [`squeue::Entry`] onto `pending`
//! and wake it via an `eventfd`, never touch the ring's own
//! submission/completion queues directly. This isn't a
//! throughput-oriented per-core ring setup (see the crate's own
//! non-goals for this feature); it's the minimum needed for correct,
//! real io_uring file I/O, safe to call from any thread on any runtime
//! flavor, including
//! [`super::super::runtime::Builder::new_thread_per_core`]'s pinned
//! worker threads.
//!
//! ## `OpDriver`: swapping the real ring for a deterministic one
//!
//! [`UringFile`] doesn't talk to [`IoUringDriver`] directly -- every
//! operation goes through the [`OpDriver`] trait, and [`UringFile`]
//! holds an `Arc<dyn OpDriver>` rather than a concrete driver type.
//! [`IoUringDriver`] is one implementation (the default -- see
//! [`UringFile::open`]/[`UringFile::create`], which reach for the
//! process-wide [`global_driver`]); [`SimDriver`] is another: a fully
//! in-memory, deterministic driver with three fault-injection knobs a
//! storage engine's own crash-recovery tests actually need --
//! [`SimDriver::inject_torn_write`] (a write that reports success while
//! only partially landing), [`SimDriver::set_fsync_lies`] (an `fsync`
//! that reports success without actually advancing what's durable), and
//! [`SimDriver::set_disk_full_at`] (`ENOSPC` once a configured capacity
//! is reached) -- plus [`SimDriver::crash_and_reopen`], which rolls
//! every file back to only what was genuinely durable, exactly what a
//! real crash-then-restart would leave behind. [`UringFile::open_on`]/
//! [`UringFile::create_on`] (and the driver-parameterized [`rename_on`]/
//! [`remove_file_on`]) accept any `Arc<dyn OpDriver>`, so a storage
//! engine's own tests can exercise its *real* recovery code against
//! [`SimDriver`] instead of a real disk and a real kernel -- no `strace`,
//! no root, no actual power-loss simulation needed, and fully
//! deterministic (every `SimDriver` operation resolves synchronously, no
//! real disk latency or kernel scheduling to race against).

use io_uring::{opcode, squeue, types, IoUring};
use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::CString;
use std::future::Future;
use std::io;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

// ---------------------------------------------------------------------
// Owned-buffer traits
// ---------------------------------------------------------------------

/// An owned buffer an io_uring read/write can safely reference for the
/// operation's entire in-flight duration -- see this module's docs for
/// why "safely" specifically requires *ownership*, not a borrow.
///
/// # Safety
/// Implementors must guarantee [`stable_ptr`](Self::stable_ptr) keeps
/// pointing at the same, valid, allocated memory for as long as the
/// value itself is alive, even if the value is moved (e.g. `Vec<u8>`'s
/// own heap allocation doesn't move when the `Vec` header does) -- the
/// driver relies on this to hand the kernel a pointer that outlives an
/// arbitrary number of moves between submission and completion.
pub unsafe trait IoBuf: Send + 'static {
    /// A stable pointer to the start of this buffer's allocation.
    fn stable_ptr(&self) -> *const u8;
    /// How many bytes are initialized and meaningful to write from --
    /// the length a `write_at` submits, and (after a `read_at`
    /// completes) how many bytes the kernel actually read.
    fn bytes_init(&self) -> usize;
    /// The buffer's total capacity -- the length a `read_at` submits (it
    /// may read up to this many bytes, however many are actually
    /// available).
    fn bytes_total(&self) -> usize;
}

/// [`IoBuf`] plus mutable access -- required for `read_at` (the kernel
/// writes into it) but not `write_at` (which only ever reads from it).
///
/// # Safety
/// Same obligation as [`IoBuf::stable_ptr`], for
/// [`stable_mut_ptr`](Self::stable_mut_ptr).
pub unsafe trait IoBufMut: IoBuf {
    /// A stable mutable pointer to the start of this buffer's allocation.
    fn stable_mut_ptr(&mut self) -> *mut u8;
    /// Records that the first `len` bytes are now initialized -- called
    /// once, after a `read_at` completes, with exactly how many bytes
    /// the kernel reported reading.
    ///
    /// # Safety
    /// The caller guarantees `len <= bytes_total()` and that the kernel
    /// (or an equivalent trusted source) really did initialize that many
    /// bytes at [`stable_mut_ptr`](Self::stable_mut_ptr).
    unsafe fn set_init(&mut self, len: usize);
}

// SAFETY: `Vec<u8>`'s heap allocation address is independent of the
// `Vec` header's own location -- moving, or even reallocating a
// *different, unrelated* `Vec`, never invalidates a pointer already
// obtained from `as_ptr`/`as_mut_ptr` on *this* one, as long as nothing
// calls a capacity-changing method (`push`, `reserve`, ...) on it while
// the pointer is still in use -- which nothing does: every `OpDriver`
// call computes the pointer once, before the buffer is boxed away, where
// it is never touched again except by `IoBufMut::set_init` at completion
// time (adjusting only `len`, never reallocating).
unsafe impl IoBuf for Vec<u8> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }
    fn bytes_init(&self) -> usize {
        self.len()
    }
    fn bytes_total(&self) -> usize {
        self.capacity()
    }
}

unsafe impl IoBufMut for Vec<u8> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
    unsafe fn set_init(&mut self, len: usize) {
        if len > self.len() {
            debug_assert!(len <= self.capacity());
            // SAFETY: caller guarantees `len` bytes starting at
            // `stable_mut_ptr()` are genuinely initialized (the kernel
            // just reported reading exactly that many bytes into them),
            // and `len <= capacity()` per this method's own contract.
            unsafe {
                self.set_len(len);
            }
        }
    }
}

/// The result of an owned-buffer operation: the outcome, and the buffer
/// handed back regardless of whether it succeeded -- so a failed
/// `write_at`, say, doesn't lose the caller's data.
pub struct BufResult<T, B>(pub io::Result<T>, pub B);

/// A boxed, dynamically dispatched future -- the shape every
/// [`OpDriver`] method returns, so both [`IoUringDriver`] and
/// [`SimDriver`] can be stored behind one `Arc<dyn OpDriver>` regardless
/// of how differently each one actually completes an operation
/// (asynchronously, off a real kernel completion queue, vs. synchronously
/// in memory).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ---------------------------------------------------------------------
// OpDriver: the pluggable seam between UringFile and whatever actually
// performs its operations
// ---------------------------------------------------------------------

/// Every operation [`UringFile`] needs from whatever's actually running
/// it -- implemented by the real io_uring-backed [`IoUringDriver`] and by
/// [`SimDriver`] (deterministic, in-memory, fault-injectable) alike. See
/// this module's top-level docs for why this seam exists and how to use
/// it in a storage engine's own crash-recovery tests.
///
/// An opaque `u64` handle stands in for "the open file" everywhere a
/// real driver would use a raw fd -- keeps this trait object-safe
/// (`Arc<dyn OpDriver>`) without committing to `RawFd` specifically,
/// which [`SimDriver`]'s in-memory files have no use for. For
/// [`IoUringDriver`], a handle *is* the real `RawFd`, numerically; for
/// [`SimDriver`], it's an opaque, monotonically increasing id into its
/// own in-memory file table -- callers must never assume anything about
/// a handle's value beyond "whatever `open` returned for this file".
///
/// `read_at`/`write_at` take a raw `buf_ptr`/`buf_len` pair (computed by
/// [`UringFile`] from the caller's [`IoBuf`]/[`IoBufMut`] before it's
/// erased) rather than the buffer itself, specifically so this trait
/// stays `Send`-safe: a raw pointer isn't `Send`, but the `usize` it's
/// cast to is, and `keepalive` (the boxed, type-erased original buffer)
/// travels alongside it purely to keep the memory `buf_ptr` describes
/// alive for the operation's whole duration -- neither implementation
/// touches `keepalive` through anything other than that raw pointer.
pub trait OpDriver: Send + Sync + 'static {
    /// Opens `path` with raw `open(2)`-style `flags`/`mode`, returning an
    /// opaque handle for every other method here.
    fn open(&self, path: PathBuf, flags: i32, mode: u32) -> BoxFuture<'static, io::Result<u64>>;

    /// Reads up to `buf_len` bytes at `pos` into the buffer described by
    /// `buf_ptr`/`buf_len` (kept alive via `keepalive`) -- returns a raw
    /// CQE-shaped result (a non-negative byte count, or `-errno`) plus
    /// `keepalive` handed back unchanged.
    fn read_at(
        &self,
        handle: u64,
        buf_ptr: usize,
        buf_len: u32,
        pos: u64,
        keepalive: Box<dyn Any + Send>,
    ) -> BoxFuture<'static, (i32, Box<dyn Any + Send>)>;

    /// Writes `buf_len` bytes at `pos` from the buffer described by
    /// `buf_ptr`/`buf_len` (kept alive via `keepalive`) -- same result
    /// shape as [`read_at`](Self::read_at).
    fn write_at(
        &self,
        handle: u64,
        buf_ptr: usize,
        buf_len: u32,
        pos: u64,
        keepalive: Box<dyn Any + Send>,
    ) -> BoxFuture<'static, (i32, Box<dyn Any + Send>)>;

    /// Flushes `handle` to durable storage -- data only if `datasync`,
    /// data and metadata otherwise. See `fsync(2)`/`fdatasync(2)`.
    fn fsync(&self, handle: u64, datasync: bool) -> BoxFuture<'static, io::Result<()>>;

    /// Preallocates `len` bytes starting at `offset`. See `fallocate(2)`.
    fn fallocate(&self, handle: u64, offset: u64, len: u64) -> BoxFuture<'static, io::Result<()>>;

    /// Truncates or extends `handle` to exactly `len` bytes. See
    /// `ftruncate(2)`.
    fn set_len(&self, handle: u64, len: u64) -> BoxFuture<'static, io::Result<()>>;

    /// Closes `handle`, surfacing any close-time error. See `close(2)`.
    fn close(&self, handle: u64) -> BoxFuture<'static, io::Result<()>>;

    /// Best-effort, synchronous close -- used only by [`UringFile`]'s
    /// `Drop` (which can't `.await` the async [`close`](Self::close)).
    /// Errors are never surfaced here, matching `std::fs::File`'s own
    /// `Drop`; prefer an explicit [`UringFile::close`] when a caller can
    /// await it and wants to observe a close-time error.
    fn close_sync(&self, handle: u64);

    /// Renames (moves) `from` to `to`, replacing the destination if it
    /// already exists. See `rename(2)`.
    fn rename(&self, from: PathBuf, to: PathBuf) -> BoxFuture<'static, io::Result<()>>;

    /// Removes the file at `path`. See `unlink(2)`.
    fn remove_file(&self, path: PathBuf) -> BoxFuture<'static, io::Result<()>>;
}

fn result_to_io(result: i32) -> io::Result<i32> {
    if result < 0 {
        Err(io::Error::from_raw_os_error(-result))
    } else {
        Ok(result)
    }
}

fn path_to_cstring(path: &Path) -> io::Result<CString> {
    use std::os::unix::ffi::OsStrExt;
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a nul byte"))
}

// ---------------------------------------------------------------------
// IoUringDriver: the real, single, process-wide ring
// ---------------------------------------------------------------------

/// Per-in-flight-operation state, reachable from both the driver's own
/// `slab` and whichever [`OpFuture`] is polling it -- see this module's
/// top-level docs for the cancellation-safety invariant this exists to
/// hold.
struct OpState {
    /// `None` until the driver thread records a real completion.
    result: Mutex<Option<i32>>,
    waker: Mutex<Option<Waker>>,
    /// The submitted buffer/payload, type-erased so `OpState` doesn't
    /// need to be generic. Taken out exactly once: by whichever of
    /// [`OpFuture::poll`] or [`IoUringDriver::event_loop`]'s completion
    /// handler observes the real completion first -- see this module's
    /// docs.
    buf: Mutex<Option<Box<dyn Any + Send>>>,
}

/// Reserved `user_data` for the driver thread's own wake `eventfd` --
/// real op ids come from a monotonically increasing counter starting at
/// 0, which in practice never reaches `u64::MAX` in one process's
/// lifetime.
const WAKE_USER_DATA: u64 = u64::MAX;

/// The real, io_uring-backed [`OpDriver`] -- see this module's top-level
/// docs for why there's exactly one of these, process-wide.
pub struct IoUringDriver {
    ring: Mutex<Option<IoUring>>,
    wake_fd: RawFd,
    pending: Mutex<VecDeque<squeue::Entry>>,
    slab: Mutex<HashMap<u64, Arc<OpState>>>,
    next_id: AtomicU64,
    shutdown: AtomicBool,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl IoUringDriver {
    fn new() -> io::Result<IoUringDriver> {
        let ring = IoUring::new(256)?;
        // SAFETY: plain integer arguments, no memory referenced --
        // mirrors `reactor::io_uring::Reactor::new`'s identical wake fd.
        let wake_fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if wake_fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(IoUringDriver {
            ring: Mutex::new(Some(ring)),
            wake_fd,
            pending: Mutex::new(VecDeque::new()),
            slab: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            thread: Mutex::new(None),
        })
    }

    fn start(self: &Arc<Self>) {
        let ring = self
            .ring
            .lock()
            .unwrap()
            .take()
            .expect("IoUringDriver::start called more than once");
        let driver = self.clone();
        let handle = std::thread::Builder::new()
            .name("rusty_tokio-uring-fs".to_string())
            .spawn(move || driver.event_loop(ring))
            .expect("failed to spawn rusty_tokio io_uring file-I/O driver thread");
        *self.thread.lock().unwrap() = Some(handle);
    }

    fn wake(&self) {
        let one: u64 = 1;
        // SAFETY: `&one` is a valid 8-byte buffer; `wake_fd` is a valid
        // eventfd.
        unsafe {
            libc::write(self.wake_fd, (&one as *const u64).cast(), 8);
        }
    }

    fn drain_wake_fd(&self) {
        let mut buf = [0u8; 8];
        // SAFETY: `buf` is a valid 8-byte buffer; `wake_fd` is a valid,
        // non-blocking eventfd, so this never blocks even with nothing
        // to drain.
        unsafe {
            libc::read(self.wake_fd, buf.as_mut_ptr().cast(), buf.len());
        }
    }

    fn arm_wake(ring: &mut IoUring, wake_fd: RawFd) {
        let entry = opcode::PollAdd::new(types::Fd(wake_fd), libc::POLLIN as u32)
            .build()
            .user_data(WAKE_USER_DATA);
        let mut sq = ring.submission();
        // SAFETY: `PollAdd` on a bare eventfd references no user buffer.
        let _ = unsafe { sq.push(&entry) };
    }

    /// The only thread that ever touches the ring's submission or
    /// completion queues, from [`start`](Self::start) onward -- see this
    /// module's top-level docs for why the ring itself needs no `Mutex`
    /// once this loop owns it exclusively.
    fn event_loop(&self, mut ring: IoUring) {
        Self::arm_wake(&mut ring, self.wake_fd);
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                return;
            }
            self.drain_pending(&mut ring);

            match ring.submit_and_wait(1) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return,
            }

            // Collected into a plain `Vec` first: the `CompletionQueue`
            // guard borrows `ring` and writes its consumed head back on
            // drop, so it has to be dropped (ending this inner scope)
            // before `arm_wake` below can borrow `ring` again.
            let mut completions = Vec::new();
            {
                let mut cq = ring.completion();
                cq.sync();
                for cqe in &mut cq {
                    completions.push((cqe.user_data(), cqe.result()));
                }
            }

            for (user_data, result) in completions {
                if user_data == WAKE_USER_DATA {
                    self.drain_wake_fd();
                    Self::arm_wake(&mut ring, self.wake_fd);
                    continue;
                }
                // Removed from `slab` here, unconditionally, whether or
                // not the `Future` that submitted it is still alive --
                // see this module's top-level docs for why that's
                // exactly the cancellation-safety invariant this needs.
                let state = self.slab.lock().unwrap().remove(&user_data);
                if let Some(state) = state {
                    *state.result.lock().unwrap() = Some(result);
                    let waker = state.waker.lock().unwrap().take();
                    if let Some(waker) = waker {
                        waker.wake();
                    }
                    // `state` (this loop's own `Arc` clone) drops here.
                    // If the submitting `Future` was already dropped,
                    // this was the last strong reference -- `OpState`,
                    // and its buffer, are freed only now.
                }
                // A `None` here means a completion arrived for a
                // `user_data` no longer in `slab` -- can't happen with
                // this driver's own bookkeeping (an id is only ever
                // removed by this exact branch, once, right when its
                // completion is processed), but handled as a no-op
                // rather than a panic regardless, matching
                // `reactor::io_uring`'s own posture on stale/unexpected
                // completions.
            }
        }
    }

    fn drain_pending(&self, ring: &mut IoUring) {
        let entries: Vec<squeue::Entry> = std::mem::take(&mut *self.pending.lock().unwrap()).into();
        for entry in entries {
            let mut sq = ring.submission();
            // SAFETY: every pointer `entry` references (a buffer's
            // `stable_ptr`/`stable_mut_ptr`, or a `CString`'s `as_ptr`)
            // is kept alive by the matching `OpState.buf` in `self.slab`
            // for at least as long as this op is outstanding -- see
            // `submit`'s docs and this module's top-level cancellation-
            // safety section.
            if unsafe { sq.push(&entry) }.is_err() {
                // Submission queue momentarily full -- flush what's
                // already queued and retry once, same as
                // `reactor::io_uring::Reactor::arm`.
                drop(sq);
                let _ = ring.submit();
                let mut sq = ring.submission();
                let _ = unsafe { sq.push(&entry) };
            }
        }
    }

    /// Submits one already-built `entry`, keeping `keepalive` (the
    /// operation's owned buffer/payload -- a `Vec<u8>`, a `CString`, or
    /// `()` for a payload-free op like `fsync`) alive until a real
    /// completion arrives, however long that takes and regardless of
    /// whether the returned [`OpFuture`] is still being polled by then.
    /// See this module's top-level docs for the full cancellation-safety
    /// argument this one invariant underpins. Callers (this driver's own
    /// [`OpDriver`] method implementations) compute any pointer `entry`
    /// references *before* calling this, while the buffer it points into
    /// is still owned locally -- moving it into `keepalive` here doesn't
    /// invalidate that pointer (see [`IoBuf`]'s own safety docs).
    fn submit(&self, keepalive: Box<dyn Any + Send>, entry: squeue::Entry) -> OpFuture {
        let user_data = self.next_id.fetch_add(1, Ordering::Relaxed);
        let entry = entry.user_data(user_data);
        let state = Arc::new(OpState {
            result: Mutex::new(None),
            waker: Mutex::new(None),
            buf: Mutex::new(Some(keepalive)),
        });
        self.slab.lock().unwrap().insert(user_data, state.clone());
        self.pending.lock().unwrap().push_back(entry);
        self.wake();
        OpFuture { state }
    }

    fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.wake();
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

impl Drop for IoUringDriver {
    fn drop(&mut self) {
        if self.thread.lock().unwrap().is_some() {
            self.shutdown();
        }
        // SAFETY: `wake_fd` is this driver's own eventfd, closed exactly
        // once, here, after the event loop thread (the only other place
        // that touches it) has already joined.
        unsafe {
            libc::close(self.wake_fd);
        }
    }
}

impl OpDriver for IoUringDriver {
    fn open(&self, path: PathBuf, flags: i32, mode: u32) -> BoxFuture<'static, io::Result<u64>> {
        let c_path = match path_to_cstring(&path) {
            Ok(c) => c,
            Err(e) => return Box::pin(std::future::ready(Err(e))),
        };
        let entry = opcode::OpenAt::new(types::Fd(libc::AT_FDCWD), c_path.as_ptr())
            .flags(flags)
            .mode(mode as libc::mode_t)
            .build();
        let fut = self.submit(Box::new(c_path), entry);
        Box::pin(async move {
            let (result, _keepalive) = fut.await;
            result_to_io(result).map(|fd| fd as u64)
        })
    }

    fn read_at(
        &self,
        handle: u64,
        buf_ptr: usize,
        buf_len: u32,
        pos: u64,
        keepalive: Box<dyn Any + Send>,
    ) -> BoxFuture<'static, (i32, Box<dyn Any + Send>)> {
        let entry = opcode::Read::new(types::Fd(handle as RawFd), buf_ptr as *mut u8, buf_len)
            .offset(pos)
            .build();
        Box::pin(self.submit(keepalive, entry))
    }

    fn write_at(
        &self,
        handle: u64,
        buf_ptr: usize,
        buf_len: u32,
        pos: u64,
        keepalive: Box<dyn Any + Send>,
    ) -> BoxFuture<'static, (i32, Box<dyn Any + Send>)> {
        let entry = opcode::Write::new(types::Fd(handle as RawFd), buf_ptr as *const u8, buf_len)
            .offset(pos)
            .build();
        Box::pin(self.submit(keepalive, entry))
    }

    fn fsync(&self, handle: u64, datasync: bool) -> BoxFuture<'static, io::Result<()>> {
        let mut op = opcode::Fsync::new(types::Fd(handle as RawFd));
        if datasync {
            op = op.flags(types::FsyncFlags::DATASYNC);
        }
        let fut = self.submit(Box::new(()), op.build());
        Box::pin(async move {
            let (result, _) = fut.await;
            result_to_io(result).map(drop)
        })
    }

    fn fallocate(&self, handle: u64, offset: u64, len: u64) -> BoxFuture<'static, io::Result<()>> {
        let entry = opcode::Fallocate::new(types::Fd(handle as RawFd), len)
            .offset(offset)
            .build();
        let fut = self.submit(Box::new(()), entry);
        Box::pin(async move {
            let (result, _) = fut.await;
            result_to_io(result).map(drop)
        })
    }

    fn set_len(&self, handle: u64, len: u64) -> BoxFuture<'static, io::Result<()>> {
        // No `IORING_OP_FTRUNCATE` in the pinned `io-uring = "0.6"` crate
        // (added upstream only in kernel 6.9+) -- `ftruncate` references
        // no caller memory at all, so (unlike every buffer-carrying op
        // above) there's no cancellation hazard to a plain blocking
        // syscall here, the same reasoning `reactor::io_uring`'s own
        // `PollAdd`/`PollRemove` split already relies on for its own
        // buffer-free ops.
        let fd = handle as RawFd;
        Box::pin(async move {
            crate::spawn_blocking(move || {
                // SAFETY: `fd` is a valid, open fd for the duration of
                // this blocking call.
                if unsafe { libc::ftruncate(fd, len as libc::off_t) } < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            })
            .await
            .unwrap_or_else(|_| Err(io::Error::other("set_len's blocking task panicked")))
        })
    }

    fn close(&self, handle: u64) -> BoxFuture<'static, io::Result<()>> {
        let entry = opcode::Close::new(types::Fd(handle as RawFd)).build();
        let fut = self.submit(Box::new(()), entry);
        Box::pin(async move {
            let (result, _) = fut.await;
            result_to_io(result).map(drop)
        })
    }

    fn close_sync(&self, handle: u64) {
        // SAFETY: `handle` is a real fd this call's caller
        // (`UringFile::drop`) owns exclusively until this point.
        unsafe {
            libc::close(handle as RawFd);
        }
    }

    fn rename(&self, from: PathBuf, to: PathBuf) -> BoxFuture<'static, io::Result<()>> {
        let (from_c, to_c) = match (path_to_cstring(&from), path_to_cstring(&to)) {
            (Ok(f), Ok(t)) => (f, t),
            (Err(e), _) | (_, Err(e)) => return Box::pin(std::future::ready(Err(e))),
        };
        let entry = opcode::RenameAt::new(
            types::Fd(libc::AT_FDCWD),
            from_c.as_ptr(),
            types::Fd(libc::AT_FDCWD),
            to_c.as_ptr(),
        )
        .build();
        let fut = self.submit(Box::new((from_c, to_c)), entry);
        Box::pin(async move {
            let (result, _) = fut.await;
            result_to_io(result).map(drop)
        })
    }

    fn remove_file(&self, path: PathBuf) -> BoxFuture<'static, io::Result<()>> {
        let c_path = match path_to_cstring(&path) {
            Ok(c) => c,
            Err(e) => return Box::pin(std::future::ready(Err(e))),
        };
        let entry = opcode::UnlinkAt::new(types::Fd(libc::AT_FDCWD), c_path.as_ptr()).build();
        let fut = self.submit(Box::new(c_path), entry);
        Box::pin(async move {
            let (result, _) = fut.await;
            result_to_io(result).map(drop)
        })
    }
}

/// One [`OpFuture`] per op in flight against [`IoUringDriver`] -- output
/// is `(i32, Box<dyn Any + Send>)`: the raw CQE result and the erased
/// keepalive payload, handed back regardless of success or failure.
/// `Unpin` automatically (`Arc<OpState>` is), so no pinning gymnastics
/// are needed to read `self.state` in `poll`.
struct OpFuture {
    state: Arc<OpState>,
}

impl Future for OpFuture {
    type Output = (i32, Box<dyn Any + Send>);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(result) = *self.state.result.lock().unwrap() {
            return Poll::Ready(Self::take_result(&self.state, result));
        }
        *self.state.waker.lock().unwrap() = Some(cx.waker().clone());
        // Recheck after installing the waker: closes the race where the
        // driver thread's completion (and wake) landed between the
        // check above and this line, which would otherwise leave this
        // future parked with nothing left to wake it.
        if let Some(result) = *self.state.result.lock().unwrap() {
            return Poll::Ready(Self::take_result(&self.state, result));
        }
        Poll::Pending
    }
}

impl OpFuture {
    fn take_result(state: &OpState, result: i32) -> (i32, Box<dyn Any + Send>) {
        let buf = state
            .buf
            .lock()
            .unwrap()
            .take()
            .expect("OpState buffer observed a second time after being taken");
        (result, buf)
    }
}

/// The one, process-wide, lazily started [`IoUringDriver`] every
/// [`UringFile`] operation and free function in this module defaults to
/// -- see this module's top-level docs for why one ring is enough here
/// (this isn't a per-core throughput setup), and for [`SimDriver`] as the
/// pluggable alternative.
static GLOBAL_DRIVER: Mutex<Option<Arc<dyn OpDriver>>> = Mutex::new(None);

fn global_driver() -> io::Result<Arc<dyn OpDriver>> {
    let mut guard = GLOBAL_DRIVER.lock().unwrap();
    if let Some(driver) = &*guard {
        return Ok(driver.clone());
    }
    let driver = Arc::new(IoUringDriver::new()?);
    driver.start();
    let driver: Arc<dyn OpDriver> = driver;
    *guard = Some(driver.clone());
    Ok(driver)
}

// ---------------------------------------------------------------------
// SimDriver: deterministic, in-memory, fault-injectable
// ---------------------------------------------------------------------

#[derive(Default, Clone)]
struct SimFile {
    data: Vec<u8>,
    /// How many of `data`'s bytes are considered crash-durable -- only
    /// ever advanced by a real (non-lying) [`OpDriver::fsync`]. See
    /// [`SimDriver::crash_and_reopen`].
    durable_len: usize,
}

#[derive(Default)]
struct SimState {
    paths: HashMap<PathBuf, u64>,
    files: HashMap<u64, SimFile>,
    next_handle: u64,
    closed: HashSet<u64>,
    capacity_bytes: Option<u64>,
    /// Sum of every live file's current length -- the simplified
    /// "disk usage" [`SimDriver::set_disk_full_at`] enforces against.
    bytes_used: u64,
    fsync_lies: bool,
    /// One-shot: consumed by the very next `write_at`, then cleared --
    /// see [`SimDriver::inject_torn_write`].
    torn_write: Option<f64>,
}

impl SimState {
    /// Grows `handle`'s file to at least `new_len` bytes (zero-filled),
    /// enforcing `capacity_bytes` -- the shared growth/disk-full logic
    /// `write_at`/`fallocate`/`set_len` all need. A no-op (not an error)
    /// if the file is already at least `new_len` long.
    fn grow_to(&mut self, handle: u64, new_len: usize) -> Result<(), i32> {
        let current_len = self.files.get(&handle).map_or(0, |f| f.data.len());
        if new_len > current_len {
            let growth = (new_len - current_len) as u64;
            if let Some(cap) = self.capacity_bytes {
                if self.bytes_used + growth > cap {
                    return Err(libc::ENOSPC);
                }
            }
            self.bytes_used += growth;
        }
        if let Some(file) = self.files.get_mut(&handle) {
            if file.data.len() < new_len {
                file.data.resize(new_len, 0);
            }
        }
        Ok(())
    }
}

/// A fully in-memory, deterministic [`OpDriver`] with fault-injection
/// knobs for storage-engine crash-recovery testing -- see this module's
/// top-level docs for the intended usage shape. Every operation resolves
/// synchronously (no real disk latency, no kernel scheduling to race
/// against), so a test using this driver is exactly as deterministic as
/// the rest of its own logic.
pub struct SimDriver {
    inner: Mutex<SimState>,
}

impl SimDriver {
    /// A fresh, empty `SimDriver` -- no files, no faults configured.
    pub fn new() -> Arc<SimDriver> {
        Arc::new(SimDriver {
            inner: Mutex::new(SimState::default()),
        })
    }

    /// Once total bytes-in-use across every file would exceed
    /// `capacity_bytes`, `write_at`/`fallocate`/`set_len` (growing)
    /// start failing with `ENOSPC` -- the disk-full fault.
    pub fn set_disk_full_at(&self, capacity_bytes: u64) {
        self.inner.lock().unwrap().capacity_bytes = Some(capacity_bytes);
    }

    /// While `true`, `fsync`/`fdatasync` report success without actually
    /// advancing what's durable -- so a later
    /// [`crash_and_reopen`](Self::crash_and_reopen) rolls back to
    /// whatever was durable *before* this was turned on, exposing
    /// recovery code that incorrectly trusted a lying fsync.
    pub fn set_fsync_lies(&self, lies: bool) {
        self.inner.lock().unwrap().fsync_lies = lies;
    }

    /// The *next* `write_at` call only actually applies the first
    /// `fraction` of its bytes to the file, while still *reporting* a
    /// full-length successful write -- exactly the hazard a real torn
    /// write produces (looks successful, silently short of durable).
    /// One-shot: call again before each write you want torn.
    ///
    /// # Panics
    /// Panics if `fraction` isn't in `0.0..=1.0`.
    pub fn inject_torn_write(&self, fraction: f64) {
        assert!(
            (0.0..=1.0).contains(&fraction),
            "torn-write fraction must be between 0.0 and 1.0"
        );
        self.inner.lock().unwrap().torn_write = Some(fraction);
    }

    /// Simulates a crash: returns a fresh `SimDriver` whose files
    /// contain only what was genuinely durable (every byte up to each
    /// file's own `durable_len` -- see [`SimFile`]) at the moment this
    /// is called. Anything written since the last real (non-lying)
    /// `fsync`/`fdatasync` is gone, exactly like a real crash before
    /// `fsync` would lose it. Every handle comes back marked closed
    /// (fds don't survive a crash either) -- reopen by path to get a
    /// fresh one; fault-injection settings are *not* inherited (a crash
    /// doesn't carry configuration forward, only data).
    pub fn crash_and_reopen(&self) -> Arc<SimDriver> {
        let inner = self.inner.lock().unwrap();
        let mut files = HashMap::with_capacity(inner.files.len());
        let mut bytes_used = 0u64;
        for (&handle, file) in &inner.files {
            let mut durable = file.data.clone();
            durable.truncate(file.durable_len);
            bytes_used += durable.len() as u64;
            let durable_len = durable.len();
            files.insert(
                handle,
                SimFile {
                    data: durable,
                    durable_len,
                },
            );
        }
        Arc::new(SimDriver {
            inner: Mutex::new(SimState {
                paths: inner.paths.clone(),
                files,
                next_handle: inner.next_handle,
                closed: inner.paths.values().copied().collect(),
                capacity_bytes: inner.capacity_bytes,
                bytes_used,
                fsync_lies: false,
                torn_write: None,
            }),
        })
    }
}

impl OpDriver for SimDriver {
    fn open(&self, path: PathBuf, flags: i32, mode: u32) -> BoxFuture<'static, io::Result<u64>> {
        let _ = mode;
        let create = flags & libc::O_CREAT != 0;
        let create_new = create && flags & libc::O_EXCL != 0;
        let truncate = flags & libc::O_TRUNC != 0;

        let mut inner = self.inner.lock().unwrap();
        let exists = inner.paths.contains_key(&path);
        let result = if exists && create_new {
            Err(io::Error::from(io::ErrorKind::AlreadyExists))
        } else if !exists && !create {
            Err(io::Error::from(io::ErrorKind::NotFound))
        } else {
            let handle = if let Some(&h) = inner.paths.get(&path) {
                h
            } else {
                let h = inner.next_handle;
                inner.next_handle += 1;
                inner.paths.insert(path, h);
                inner.files.insert(h, SimFile::default());
                h
            };
            if truncate {
                if let Some(file) = inner.files.get_mut(&handle) {
                    let old_len = file.data.len() as u64;
                    file.data.clear();
                    file.durable_len = 0;
                    inner.bytes_used = inner.bytes_used.saturating_sub(old_len);
                }
            }
            inner.closed.remove(&handle);
            Ok(handle)
        };
        drop(inner);
        Box::pin(std::future::ready(result))
    }

    fn read_at(
        &self,
        handle: u64,
        buf_ptr: usize,
        buf_len: u32,
        pos: u64,
        keepalive: Box<dyn Any + Send>,
    ) -> BoxFuture<'static, (i32, Box<dyn Any + Send>)> {
        let inner = self.inner.lock().unwrap();
        let result = if inner.closed.contains(&handle) {
            Err(libc::EBADF)
        } else {
            match inner.files.get(&handle) {
                None => Err(libc::EBADF),
                Some(file) => {
                    let pos = pos as usize;
                    let avail = file.data.len().saturating_sub(pos);
                    let n = avail.min(buf_len as usize);
                    // SAFETY: `keepalive` (the caller's owned buffer,
                    // erased) is held alive for this call's entire
                    // (here, synchronous) duration -- `buf_ptr`/
                    // `buf_len` describe exactly that buffer's
                    // still-valid memory, same invariant the real
                    // io_uring-backed driver relies on.
                    let dst =
                        unsafe { std::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len as usize) };
                    dst[..n].copy_from_slice(&file.data[pos..pos + n]);
                    Ok(n as i32)
                }
            }
        };
        drop(inner);
        let result = result.unwrap_or_else(|errno| -errno);
        Box::pin(std::future::ready((result, keepalive)))
    }

    fn write_at(
        &self,
        handle: u64,
        buf_ptr: usize,
        buf_len: u32,
        pos: u64,
        keepalive: Box<dyn Any + Send>,
    ) -> BoxFuture<'static, (i32, Box<dyn Any + Send>)> {
        // SAFETY: same as `read_at` above.
        let src = unsafe { std::slice::from_raw_parts(buf_ptr as *const u8, buf_len as usize) };
        let mut inner = self.inner.lock().unwrap();
        let result = if inner.closed.contains(&handle) || !inner.files.contains_key(&handle) {
            Err(libc::EBADF)
        } else {
            let start = pos as usize;
            let end = start + src.len();
            match inner.grow_to(handle, end) {
                Err(errno) => Err(errno),
                Ok(()) => {
                    let torn = inner.torn_write.take();
                    let apply_len = torn
                        .map(|fraction| ((src.len() as f64) * fraction) as usize)
                        .unwrap_or(src.len())
                        .min(src.len());
                    let file = inner.files.get_mut(&handle).unwrap();
                    file.data[start..start + apply_len].copy_from_slice(&src[..apply_len]);
                    // A torn write still *reports* the full length as
                    // successfully written -- exactly the hazard a real
                    // torn write produces: the syscall (here, the sim)
                    // claims success while only part of the data
                    // actually landed.
                    Ok(src.len() as i32)
                }
            }
        };
        drop(inner);
        let result = result.unwrap_or_else(|errno| -errno);
        Box::pin(std::future::ready((result, keepalive)))
    }

    fn fsync(&self, handle: u64, _datasync: bool) -> BoxFuture<'static, io::Result<()>> {
        let mut inner = self.inner.lock().unwrap();
        let result = if inner.closed.contains(&handle) {
            Err(io::Error::from_raw_os_error(libc::EBADF))
        } else if inner.fsync_lies {
            // The fault: reports success without touching `durable_len`
            // at all.
            Ok(())
        } else {
            match inner.files.get_mut(&handle) {
                None => Err(io::Error::from_raw_os_error(libc::EBADF)),
                Some(file) => {
                    file.durable_len = file.data.len();
                    Ok(())
                }
            }
        };
        drop(inner);
        Box::pin(std::future::ready(result))
    }

    fn fallocate(&self, handle: u64, offset: u64, len: u64) -> BoxFuture<'static, io::Result<()>> {
        let mut inner = self.inner.lock().unwrap();
        let result = if !inner.files.contains_key(&handle) {
            Err(io::Error::from_raw_os_error(libc::EBADF))
        } else {
            let end = (offset + len) as usize;
            inner.grow_to(handle, end).map_err(io::Error::from_raw_os_error)
        };
        drop(inner);
        Box::pin(std::future::ready(result))
    }

    fn set_len(&self, handle: u64, len: u64) -> BoxFuture<'static, io::Result<()>> {
        let mut inner = self.inner.lock().unwrap();
        let result = if !inner.files.contains_key(&handle) {
            Err(io::Error::from_raw_os_error(libc::EBADF))
        } else {
            let new_len = len as usize;
            let current_len = inner.files[&handle].data.len();
            if new_len >= current_len {
                inner.grow_to(handle, new_len).map_err(io::Error::from_raw_os_error)
            } else {
                let shrink = (current_len - new_len) as u64;
                let file = inner.files.get_mut(&handle).unwrap();
                file.data.truncate(new_len);
                file.durable_len = file.durable_len.min(new_len);
                inner.bytes_used = inner.bytes_used.saturating_sub(shrink);
                Ok(())
            }
        };
        drop(inner);
        Box::pin(std::future::ready(result))
    }

    fn close(&self, handle: u64) -> BoxFuture<'static, io::Result<()>> {
        self.inner.lock().unwrap().closed.insert(handle);
        Box::pin(std::future::ready(Ok(())))
    }

    fn close_sync(&self, handle: u64) {
        self.inner.lock().unwrap().closed.insert(handle);
    }

    fn rename(&self, from: PathBuf, to: PathBuf) -> BoxFuture<'static, io::Result<()>> {
        let mut inner = self.inner.lock().unwrap();
        let result = match inner.paths.remove(&from) {
            None => Err(io::Error::from(io::ErrorKind::NotFound)),
            Some(handle) => {
                // Replaces whatever was at `to`, matching real
                // `rename(2)` semantics -- the old destination's content
                // (if any) is simply dropped, same as the real driver's
                // `RenameAt`.
                if let Some(old_handle) = inner.paths.insert(to, handle) {
                    if old_handle != handle {
                        if let Some(old_file) = inner.files.remove(&old_handle) {
                            inner.bytes_used =
                                inner.bytes_used.saturating_sub(old_file.data.len() as u64);
                        }
                    }
                }
                Ok(())
            }
        };
        drop(inner);
        Box::pin(std::future::ready(result))
    }

    fn remove_file(&self, path: PathBuf) -> BoxFuture<'static, io::Result<()>> {
        let mut inner = self.inner.lock().unwrap();
        let result = match inner.paths.remove(&path) {
            None => Err(io::Error::from(io::ErrorKind::NotFound)),
            Some(handle) => {
                if let Some(file) = inner.files.remove(&handle) {
                    inner.bytes_used = inner.bytes_used.saturating_sub(file.data.len() as u64);
                }
                Ok(())
            }
        };
        drop(inner);
        Box::pin(std::future::ready(result))
    }
}

// ---------------------------------------------------------------------
// UringFile
// ---------------------------------------------------------------------

/// A builder for [`UringFile::open`]'s underlying `openat(2)` flags --
/// see `std::fs::OpenOptions` / [`crate::fs::OpenOptions`] for the
/// equivalent on the `spawn_blocking`-based [`crate::fs::File`].
#[derive(Clone, Debug)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
    mode: libc::mode_t,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenOptions {
    pub fn new() -> Self {
        OpenOptions {
            read: false,
            write: false,
            append: false,
            truncate: false,
            create: false,
            create_new: false,
            mode: 0o666,
        }
    }

    pub fn read(mut self, read: bool) -> Self {
        self.read = read;
        self
    }

    pub fn write(mut self, write: bool) -> Self {
        self.write = write;
        self
    }

    pub fn append(mut self, append: bool) -> Self {
        self.append = append;
        self
    }

    pub fn truncate(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        self
    }

    pub fn create(mut self, create: bool) -> Self {
        self.create = create;
        self
    }

    pub fn create_new(mut self, create_new: bool) -> Self {
        self.create_new = create_new;
        self
    }

    /// The mode bits used if this call ends up creating the file --
    /// ignored otherwise. Defaults to `0o666` (before `umask`), same as
    /// `std::fs::OpenOptions`.
    pub fn mode(mut self, mode: u32) -> Self {
        self.mode = mode as libc::mode_t;
        self
    }

    fn raw_flags(&self) -> i32 {
        let mut flags = libc::O_CLOEXEC;
        flags |= match (self.read, self.write) {
            (_, true) if self.read => libc::O_RDWR,
            (_, true) => libc::O_WRONLY,
            _ => libc::O_RDONLY,
        };
        if self.append {
            flags |= libc::O_APPEND;
        }
        if self.truncate {
            flags |= libc::O_TRUNC;
        }
        if self.create_new {
            flags |= libc::O_CREAT | libc::O_EXCL;
        } else if self.create {
            flags |= libc::O_CREAT;
        }
        flags
    }

    /// Opens `path` with this builder's flags against the process-wide
    /// [`global_driver`] (the real io_uring ring).
    pub async fn open(&self, path: impl AsRef<Path>) -> io::Result<UringFile> {
        self.open_on(global_driver()?, path).await
    }

    /// Like [`open`](Self::open), but against an explicit driver --
    /// [`SimDriver`] for a storage engine's own deterministic
    /// crash-recovery tests, most commonly.
    pub async fn open_on(
        &self,
        driver: Arc<dyn OpDriver>,
        path: impl AsRef<Path>,
    ) -> io::Result<UringFile> {
        let handle = driver
            .open(path.as_ref().to_path_buf(), self.raw_flags(), self.mode as u32)
            .await?;
        Ok(UringFile {
            handle,
            driver,
            closed: AtomicBool::new(false),
        })
    }
}

/// An open file handle backed by whatever [`OpDriver`] it was opened
/// against -- the real io_uring ring by default (see [`UringFile::open`]/
/// [`UringFile::create`]), or an explicit one (most commonly
/// [`SimDriver`]) via [`UringFile::open_on`]/[`UringFile::create_on`].
/// `openat`, positional `read`/`write`, `fsync`/`fdatasync`, `close`, and
/// (via [`fallocate`](Self::fallocate)) preallocation. See this module's
/// top-level docs for the owned-buffer cancellation-safety argument
/// behind [`read_at`](Self::read_at)/[`write_at`](Self::write_at)'s
/// signatures.
pub struct UringFile {
    handle: u64,
    driver: Arc<dyn OpDriver>,
    /// Set by [`close`](Self::close) so `Drop` doesn't redundantly
    /// (and, for a real fd, incorrectly -- the handle's already been
    /// closed) call [`OpDriver::close_sync`] a second time.
    closed: AtomicBool,
}

impl std::fmt::Debug for UringFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UringFile").field("handle", &self.handle).finish()
    }
}

impl UringFile {
    /// Opens an existing file for reading and writing, against the
    /// process-wide real io_uring driver.
    pub async fn open(path: impl AsRef<Path>) -> io::Result<UringFile> {
        OpenOptions::new().read(true).write(true).open(path).await
    }

    /// Opens a file for writing, creating it if it doesn't exist and
    /// truncating it if it does, against the process-wide real io_uring
    /// driver.
    pub async fn create(path: impl AsRef<Path>) -> io::Result<UringFile> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .await
    }

    /// Like [`open`](Self::open), but against an explicit [`OpDriver`]
    /// -- see this module's top-level `OpDriver` docs.
    pub async fn open_on(driver: Arc<dyn OpDriver>, path: impl AsRef<Path>) -> io::Result<UringFile> {
        OpenOptions::new().read(true).write(true).open_on(driver, path).await
    }

    /// Like [`create`](Self::create), but against an explicit
    /// [`OpDriver`].
    pub async fn create_on(driver: Arc<dyn OpDriver>, path: impl AsRef<Path>) -> io::Result<UringFile> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open_on(driver, path)
            .await
    }

    /// A blank [`OpenOptions`] builder.
    pub fn options() -> OpenOptions {
        OpenOptions::new()
    }

    /// This file's opaque driver handle -- a real Linux fd (numerically)
    /// for the default io_uring-backed driver, or a [`SimDriver`]-
    /// internal id when opened via [`open_on`](Self::open_on). Not
    /// meaningful to interpret directly; exposed for driver-specific
    /// test assertions.
    pub fn handle(&self) -> u64 {
        self.handle
    }

    /// Reads into `buf` starting at absolute file offset `pos`, without
    /// touching or advancing any file cursor -- unlike
    /// [`crate::fs::File`]/`std::fs::File`, this API has no cursor at
    /// all; every operation names its own offset, exactly what a
    /// segment-log storage engine needs. Always hands `buf` back,
    /// success or failure -- see [`BufResult`].
    pub async fn read_at<B: IoBufMut>(&self, mut buf: B, pos: u64) -> BufResult<usize, B> {
        let ptr = buf.stable_mut_ptr() as usize;
        let len = buf.bytes_total() as u32;
        let (result, keepalive) = self.driver.read_at(self.handle, ptr, len, pos, Box::new(buf)).await;
        let mut buf = *keepalive
            .downcast::<B>()
            .expect("OpDriver returned a buffer of a different type than it was given");
        match result_to_io(result) {
            Ok(n) => {
                // SAFETY: the driver reported reading exactly `n` bytes
                // into this buffer -- see this module's top-level
                // cancellation-safety docs for why `buf` is guaranteed
                // to still be the exact memory that was written into,
                // never reused or moved in the meantime.
                unsafe {
                    buf.set_init(n as usize);
                }
                BufResult(Ok(n as usize), buf)
            }
            Err(e) => BufResult(Err(e), buf),
        }
    }

    /// Writes `buf`'s initialized bytes ([`IoBuf::bytes_init`]) starting
    /// at absolute file offset `pos`. Always hands `buf` back, success
    /// or failure.
    pub async fn write_at<B: IoBuf>(&self, buf: B, pos: u64) -> BufResult<usize, B> {
        let ptr = buf.stable_ptr() as usize;
        let len = buf.bytes_init() as u32;
        let (result, keepalive) = self.driver.write_at(self.handle, ptr, len, pos, Box::new(buf)).await;
        let buf = *keepalive
            .downcast::<B>()
            .expect("OpDriver returned a buffer of a different type than it was given");
        match result_to_io(result) {
            Ok(n) => BufResult(Ok(n as usize), buf),
            Err(e) => BufResult(Err(e), buf),
        }
    }

    /// Flushes both data and metadata to disk. See `fsync(2)`.
    pub async fn fsync(&self) -> io::Result<()> {
        self.driver.fsync(self.handle, false).await
    }

    /// Flushes data (and only as much metadata as reads depend on) to
    /// disk -- may be faster than [`fsync`](Self::fsync) where that
    /// distinction exists. See `fdatasync(2)`.
    pub async fn fdatasync(&self) -> io::Result<()> {
        self.driver.fsync(self.handle, true).await
    }

    /// Preallocates `len` bytes starting at `offset`, without writing
    /// any actual data -- a real, worthwhile win for a segment-log
    /// engine rolling a new fixed-size segment file up front. See
    /// `fallocate(2)`.
    pub async fn fallocate(&self, offset: u64, len: u64) -> io::Result<()> {
        self.driver.fallocate(self.handle, offset, len).await
    }

    /// Truncates or extends the file to exactly `len` bytes. See
    /// `ftruncate(2)`.
    ///
    /// # Panics
    /// Against the real driver: panics if called outside a running
    /// [`crate::Runtime`] (the same contract every
    /// [`crate::spawn_blocking`] call has).
    pub async fn set_len(&self, len: u64) -> io::Result<()> {
        self.driver.set_len(self.handle, len).await
    }

    /// Closes this file, surfacing any close-time error (e.g. a
    /// deferred write-back failure some filesystems only report at
    /// `close`) -- unlike simply dropping a `UringFile` (still safe; see
    /// this type's own `Drop`, a best-effort synchronous close with
    /// errors discarded, since `Drop` can't `.await`).
    pub async fn close(self) -> io::Result<()> {
        self.closed.store(true, Ordering::Relaxed);
        self.driver.close(self.handle).await
    }
}

impl Drop for UringFile {
    fn drop(&mut self) {
        if !self.closed.load(Ordering::Relaxed) {
            self.driver.close_sync(self.handle);
        }
    }
}

/// Renames (moves) `from` to `to` against the process-wide real io_uring
/// driver, replacing the destination if it already exists -- the
/// segment-roll rename every Kafka-shaped `.log`/`.index` retention
/// policy needs. See `renameat(2)`.
pub async fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    rename_on(global_driver()?, from, to).await
}

/// Like [`rename`], but against an explicit [`OpDriver`].
pub async fn rename_on(
    driver: Arc<dyn OpDriver>,
    from: impl AsRef<Path>,
    to: impl AsRef<Path>,
) -> io::Result<()> {
    driver
        .rename(from.as_ref().to_path_buf(), to.as_ref().to_path_buf())
        .await
}

/// Removes the file at `path` against the process-wide real io_uring
/// driver -- the retention side of a segment roll. See `unlink(2)`.
pub async fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    remove_file_on(global_driver()?, path).await
}

/// Like [`remove_file`], but against an explicit [`OpDriver`].
pub async fn remove_file_on(driver: Arc<dyn OpDriver>, path: impl AsRef<Path>) -> io::Result<()> {
    driver.remove_file(path.as_ref().to_path_buf()).await
}
