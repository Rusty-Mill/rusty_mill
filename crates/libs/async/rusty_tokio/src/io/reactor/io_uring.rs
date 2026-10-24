//! Linux readiness reactor using one-shot io_uring `PollAdd` operations.
//!
//! This replaces only readiness notification, not the socket read/write
//! syscalls. Unlike io_uring reads into borrowed buffers, PollAdd never gives
//! the kernel access to memory a cancelled future could free. It is therefore
//! compatible with the existing borrowed-buffer AsyncRead/AsyncWrite API.
//!
//! Polls reference no user buffers, but they DO retain kernel file references.
//! Deregistration must reap the terminal poll and cancellation completions before
//! the owner closes its fd, or an idle listener can keep its address bound.
//!
//! The ring is serialized by `driver`. The background thread waits with poll(2)
//! OUTSIDE that lock, and invokes wakers outside it too. Deregistration can thus
//! drive cancellation itself, even from a waker on the reactor thread. It never
//! waits for the background thread, a task, or a callback to make progress.

use super::{InitialReadiness, Interest, ScheduledIo};
use io_uring::{opcode, squeue, types, IoUring};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const REMOVE_BIT: u64 = 1 << 63;
const POLL_MASK: u32 = (libc::POLLIN as u32)
    | (libc::POLLOUT as u32)
    | (libc::POLLRDHUP as u32)
    | (libc::POLLHUP as u32)
    | (libc::POLLERR as u32);

struct Registration {
    fd: RawFd,
    io: Arc<ScheduledIo>,
}

struct Driver {
    ring: IoUring,
    // Monotonic identities, rather than fd numbers: a delayed completion must
    // never wake or re-arm a different socket that reused a closed fd.
    next_id: u64,
    ids: HashMap<RawFd, u64>,
    registry: HashMap<u64, Registration>,
    completions: VecDeque<(u64, i32)>,
}

impl Driver {
    fn push(&mut self, entry: squeue::Entry) -> io::Result<()> {
        loop {
            // SAFETY: only PollAdd/PollRemove/Nop entries are used; none
            // references user memory. The caller holds the driver mutex.
            if unsafe { self.ring.submission().push(&entry) }.is_ok() {
                return Ok(());
            }
            // Queue pressure is backpressure, never permission to discard an
            // operation. Interrupted submissions preserve the queued entries.
            self.submit(0)?;
        }
    }

    fn submit(&mut self, want: usize) -> io::Result<()> {
        loop {
            match self.ring.submit_and_wait(want) {
                Ok(_) => {
                    // A successful enter may submit fewer than the queued
                    // entries. Keep ownership of the remainder until flushed.
                    if self.ring.submission().is_empty() {
                        return Ok(());
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.raw_os_error() == Some(libc::EBUSY) => {
                    // A full CQ can prevent the kernel flushing its overflow
                    // list. Reap it rather than losing an SQ entry or spinning.
                    self.reap();
                    if want != 0 {
                        return Ok(());
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn arm(&mut self, fd: RawFd, id: u64) -> io::Result<()> {
        self.push(
            opcode::PollAdd::new(types::Fd(fd), POLL_MASK)
                .build()
                .user_data(id),
        )
    }

    fn reap(&mut self) {
        self.completions.extend(
            self.ring
                .completion()
                .map(|cqe| (cqe.user_data(), cqe.result())),
        );
    }

    fn cancel(&mut self, id: u64) -> io::Result<()> {
        let remove_id = id | REMOVE_BIT;
        self.push(opcode::PollRemove::new(id).build().user_data(remove_id))?;
        let mut poll_done = false;
        let mut remove_done = false;
        self.submit(0)?;
        loop {
            self.reap();
            // Retain unrelated completions for normal readiness dispatch. In
            // particular, never invoke a waker while holding the ring lock.
            self.completions.retain(|&(token, result)| {
                if token == id {
                    poll_done = true;
                    return false;
                }
                if token == remove_id {
                    // ENOENT means the one-shot poll had already fired. Its
                    // terminal CQE still has to be consumed before returning.
                    // EALREADY likewise requires waiting for that terminal CQE
                    // (liburing io_uring_prep_poll_remove(3)).
                    assert!(
                        result == 0 || result == -libc::ENOENT || result == -libc::EALREADY,
                        "io_uring PollRemove failed: {result}"
                    );
                    remove_done = true;
                    return false;
                }
                true
            });
            if poll_done && remove_done {
                // A CQE can become visible on another submitter's task-work
                // thread BEFORE io_free_batch_list drops the poll's file ref.
                // Submit a fileless marker: processing a new SQE acquires the
                // kernel's uring_lock, serializing with that cleanup. An empty
                // enter (to_submit == 0) does not acquire that lock. Do not use
                // IO_DRAIN, which would also wait for unrelated idle polls.
                // The marker CQE has no registry entry and is simply ignored.
                self.push(opcode::Nop::new().build().user_data(remove_id))?;
                return self.submit(0);
            }
            self.submit(1)?;
        }
    }
}

pub(crate) struct Reactor {
    driver: Mutex<Driver>,
    wake_fd: RawFd,
    ring_fd: RawFd,
    shutdown: AtomicBool,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Reactor {
    pub(crate) fn new() -> io::Result<Reactor> {
        let ring = IoUring::new(256)?;
        let ring_fd = ring.as_raw_fd();
        // SAFETY: integer arguments, no user memory. Nonblocking so draining
        // a wake never waits for another writer.
        let wake_fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if wake_fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Reactor {
            driver: Mutex::new(Driver {
                ring,
                next_id: 0,
                ids: HashMap::new(),
                registry: HashMap::new(),
                completions: VecDeque::new(),
            }),
            wake_fd,
            ring_fd,
            shutdown: AtomicBool::new(false),
            thread: Mutex::new(None),
        })
    }

    pub(crate) fn start(self: &Arc<Self>) {
        let mut thread = self.thread.lock().unwrap();
        assert!(thread.is_none(), "Reactor::start called more than once");
        let reactor = self.clone();
        *thread = Some(
            std::thread::Builder::new()
                .name("rusty_tokio-reactor".to_string())
                .spawn(move || reactor.event_loop())
                .expect("failed to spawn rusty_tokio reactor thread"),
        );
    }

    fn event_loop(&self) {
        while !self.shutdown.load(Ordering::Acquire) {
            let notifications = {
                let mut driver = self.driver.lock().unwrap();
                driver.reap();
                let completions = std::mem::take(&mut driver.completions);
                let mut notifications = Vec::new();
                for (id, result) in completions {
                    let Some(registration) = driver.registry.get(&id) else {
                        continue;
                    };
                    let fd = registration.fd;
                    notifications.push((registration.io.clone(), result));
                    driver.arm(fd, id).expect("io_uring poll re-arm failed");
                }
                driver.submit(0).expect("io_uring poll submission failed");
                notifications
            };
            for (io, result) in notifications {
                let events = result as u32;
                if result < 0
                    || events
                        & (libc::POLLIN as u32
                            | libc::POLLHUP as u32
                            | libc::POLLERR as u32
                            | libc::POLLRDHUP as u32)
                        != 0
                {
                    io.mark_ready(Interest::Read);
                }
                if result < 0
                    || events & (libc::POLLOUT as u32 | libc::POLLHUP as u32 | libc::POLLERR as u32)
                        != 0
                {
                    io.mark_ready(Interest::Write);
                }
            }
            let mut fds = [
                libc::pollfd {
                    fd: self.ring_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.wake_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // SAFETY: `fds` is a valid array, both fds live until the reactor
            // is dropped after joining this thread. No ring lock is held.
            let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, -1) };
            if result < 0 {
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return;
            }
            if fds[1].revents != 0 {
                self.drain_wake_fd();
            }
        }
    }

    fn drain_wake_fd(&self) {
        let mut buf = [0u8; 8];
        // SAFETY: valid 8-byte buffer and an owned nonblocking eventfd.
        unsafe {
            libc::read(self.wake_fd, buf.as_mut_ptr().cast(), buf.len());
        }
    }

    fn wake(&self) {
        let one: u64 = 1;
        // SAFETY: valid 8-byte buffer and an owned eventfd.
        unsafe {
            libc::write(self.wake_fd, (&one as *const u64).cast(), 8);
        }
    }

    pub(crate) fn register(&self, fd: RawFd) -> io::Result<Arc<ScheduledIo>> {
        self.register_with(fd, InitialReadiness::Optimistic)
    }

    pub(crate) fn register_with(
        &self,
        fd: RawFd,
        initial: InitialReadiness,
    ) -> io::Result<Arc<ScheduledIo>> {
        let io = Arc::new(ScheduledIo::with_initial(initial));
        let mut driver = self.driver.lock().unwrap();
        let id = driver.next_id;
        assert!(
            id < REMOVE_BIT,
            "io_uring registration identities exhausted"
        );
        driver.next_id += 1;
        driver.arm(fd, id)?;
        driver.submit(0)?;
        driver.ids.insert(fd, id);
        driver
            .registry
            .insert(id, Registration { fd, io: io.clone() });
        drop(driver);
        self.wake();
        Ok(io)
    }

    pub(crate) fn deregister(&self, fd: RawFd) {
        let mut driver = self.driver.lock().unwrap();
        let Some(id) = driver.ids.remove(&fd) else {
            return;
        };
        let registration = driver.registry.remove(&id);
        // A fatal submission error cannot be silently treated as successful
        // teardown: the kernel may still own a reference to the listener.
        driver
            .cancel(id)
            .expect("io_uring poll cancellation failed");
        drop(driver);
        // Dropping a stored waker can itself drop another registered fd.
        drop(registration);
        // Cancellation may have consumed other readiness CQEs. Dispatch them
        // even if the ring itself has no completions left to make it readable.
        self.wake();
    }

    pub(crate) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.wake();
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Reactor {
    fn drop(&mut self) {
        // SAFETY: this reactor owns the eventfd; its background thread has
        // exited. The driver field owns and closes the ring itself.
        unsafe {
            libc::close(self.wake_fd);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::os::fd::AsRawFd;
    use std::sync::mpsc;
    use std::task::{Context, Wake, Waker};
    use std::time::Duration;

    struct PauseWake {
        entered: mpsc::Sender<()>,
        resume: Mutex<mpsc::Receiver<()>>,
    }

    impl Wake for PauseWake {
        fn wake(self: Arc<Self>) {
            self.entered.send(()).unwrap();
            self.resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
    }

    // A different fd's waker stalls the background thread after it has
    // armed the idle listener. Teardown must release that listener's
    // kernel poll reference even when the background thread cannot run.
    #[test]
    fn deregister_releases_listener_while_reactor_is_in_a_waker() {
        let reactor = Arc::new(Reactor::new().unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        reactor.register(listener.as_raw_fd()).unwrap();
        let trigger = TcpListener::bind("127.0.0.1:0").unwrap();
        let io = reactor.register(trigger.as_raw_fd()).unwrap();
        io.clear_if_unchanged(Interest::Read, io.snapshot(Interest::Read));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let waker = Waker::from(Arc::new(PauseWake {
            entered: entered_tx,
            resume: Mutex::new(resume_rx),
        }));
        assert!(io
            .poll_ready(&mut Context::from_waker(&waker), Interest::Read)
            .is_pending());
        reactor.start();
        let connection = TcpStream::connect(trigger.local_addr().unwrap()).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        reactor.deregister(listener.as_raw_fd());
        drop(listener);
        let rebound = TcpListener::bind(addr);
        resume_tx.send(()).unwrap();
        reactor.shutdown();
        drop(connection);
        assert!(
            rebound.is_ok(),
            "listener address still retained: {rebound:?}"
        );
    }

    #[test]
    fn cancellation_survives_a_full_submission_queue_and_preserves_other_completions() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let trigger = TcpListener::bind("127.0.0.1:0").unwrap();
        let connection = TcpStream::connect(trigger.local_addr().unwrap()).unwrap();
        let mut driver = Driver {
            ring: IoUring::new(2).unwrap(),
            next_id: 2,
            ids: HashMap::new(),
            registry: HashMap::new(),
            completions: VecDeque::new(),
        };
        driver.arm(listener.as_raw_fd(), 0).unwrap();
        driver.arm(trigger.as_raw_fd(), 1).unwrap();
        assert!(driver.ring.submission().is_full());
        // PollRemove must make room, not be discarded. While waiting for its
        // terminal CQEs it must also retain the unrelated readable listener.
        driver.cancel(0).unwrap();
        assert!(driver
            .completions
            .iter()
            .any(|&(id, events)| id == 1 && events & libc::POLLIN as i32 != 0));
        driver.cancel(1).unwrap();
        assert!(driver
            .completions
            .iter()
            .all(|&(id, _)| id & REMOVE_BIT != 0));
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let rebound = TcpListener::bind(addr).unwrap();
        drop(rebound);
        drop(connection);
    }

    struct DropFromWake {
        reactor: Arc<Reactor>,
        listener: Mutex<Option<TcpListener>>,
        done: mpsc::Sender<std::io::Result<TcpListener>>,
    }

    impl Wake for DropFromWake {
        fn wake(self: Arc<Self>) {
            let listener = self.listener.lock().unwrap().take().unwrap();
            let addr = listener.local_addr().unwrap();
            self.reactor.deregister(listener.as_raw_fd());
            drop(listener);
            self.done.send(TcpListener::bind(addr)).unwrap();
        }
    }

    #[test]
    fn a_waker_can_deregister_another_listener_on_the_reactor_thread() {
        let reactor = Arc::new(Reactor::new().unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        reactor.register(listener.as_raw_fd()).unwrap();
        let trigger = TcpListener::bind("127.0.0.1:0").unwrap();
        let io = reactor.register(trigger.as_raw_fd()).unwrap();
        io.clear_if_unchanged(Interest::Read, io.snapshot(Interest::Read));
        let (done_tx, done_rx) = mpsc::channel();
        let waker = Waker::from(Arc::new(DropFromWake {
            reactor: reactor.clone(),
            listener: Mutex::new(Some(listener)),
            done: done_tx,
        }));
        assert!(io
            .poll_ready(&mut Context::from_waker(&waker), Interest::Read)
            .is_pending());
        reactor.start();
        let connection = TcpStream::connect(trigger.local_addr().unwrap()).unwrap();
        let rebound = done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        reactor.shutdown();
        drop(connection);
        assert!(
            rebound.is_ok(),
            "listener address still retained: {rebound:?}"
        );
    }
}
