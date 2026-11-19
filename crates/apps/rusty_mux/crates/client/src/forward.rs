//! Port forwards from the client's side (C81): listens on loopback ports
//! and tunnels each connection to a loopback port of the server's host,
//! over a connection that is attached to the block and holds its input
//! lease. Blocking and std only, so `rusty-mux-ctl` and the GUI share it
//! and nothing here needs an async runtime.
//!
//! Credit mirrors the server's: at most [`FORWARD_WINDOW`] bytes of a
//! forward's data are unacknowledged in each direction, and a chunk is at
//! most [`FORWARD_CHUNK`] bytes.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, Scope};
use std::time::{Duration, Instant};

use rusty_mux_core::{BlockId, Event};
use rusty_mux_proto::{FORWARD_CHUNK, FORWARD_WINDOW, Message, decode, encode};

/// How long the server may take to accept a forward.
const OPEN_WAIT: Duration = Duration::from_secs(10);
/// How long any wait goes before it looks at the stop and end flags again.
const SLICE: Duration = Duration::from_millis(50);

/// A port to listen on and the server's port it leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spec {
    /// The loopback port to listen on; `None` for the remote port if it is
    /// free, else any free port.
    pub local: Option<u16>,
    pub remote: u16,
}

/// A port being listened on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bound {
    pub local: u16,
    pub remote: u16,
}

/// Why [`run`] returned.
#[derive(Debug)]
pub enum Ended {
    /// The caller's stop said so.
    Stopped,
    /// The block closed.
    BlockClosed,
    /// The connection to the server failed or ended.
    Lost(io::Error),
}

/// One tunnelled connection, as the reader thread sees it.
struct Fwd {
    /// Bytes for the local socket.
    to_local: Sender<Vec<u8>>,
    /// Bytes the server will still take.
    credit: Mutex<usize>,
    credited: Condvar,
}

/// What the threads of one [`run`] share.
struct Shared<W> {
    block: BlockId,
    wr: Mutex<W>,
    forwards: Mutex<HashMap<u64, Arc<Fwd>>>,
    /// Requests waiting for their answer.
    asked: Mutex<HashMap<u64, Sender<Result<(), String>>>>,
    next: AtomicU64,
    ended: Mutex<Option<Ended>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A thread that panicked holds nothing a later one can't use.
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl<W: Write> Shared<W> {
    fn over(&self) -> bool {
        lock(&self.ended).is_some()
    }

    /// Ends the run with `why`, the first time only, and wakes every wait.
    fn end(&self, why: Ended) {
        lock(&self.ended).get_or_insert(why);
        for fwd in lock(&self.forwards).values() {
            fwd.credited.notify_all();
        }
    }

    fn send(&self, message: &Message) -> io::Result<()> {
        let frame = encode(message).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let mut wr = lock(&self.wr);
        let mut rest = frame.as_slice();
        while !rest.is_empty() {
            match wr.write(rest) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => rest = &rest[n..],
                Err(e) if is_wait(&e) => {
                    if self.over() {
                        return Err(io::ErrorKind::Interrupted.into());
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Sends a message, ending the run if the connection fails.
    fn say(&self, message: &Message) {
        if let Err(e) = self.send(message) {
            self.end(Ended::Lost(e));
        }
    }

    fn id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }
}

fn is_wait(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

/// Listens for each of `specs` and tunnels what connects, until `stopped`
/// says so, the block closes or the connection ends. `rd` and `wr` are the
/// two ends of a connection that has attached to `block` and holds its
/// input lease, with `pending` the bytes already read past the last frame.
/// Reads and writes may time out and are retried; `rd` must time out, or
/// its thread cannot notice the end. `bound` is told each
/// port as it is listened on, before any connection is taken.
pub fn run<R, W>(
    mut rd: R,
    wr: W,
    pending: Vec<u8>,
    block: BlockId,
    specs: &[Spec],
    stopped: &(dyn Fn() -> bool + Sync),
    bound: &mut dyn FnMut(Bound),
) -> io::Result<Ended>
where
    R: Read + Send,
    W: Write + Send,
{
    let mut listeners = Vec::new();
    for spec in specs {
        let listener = listen(spec.local, spec.remote)?;
        listener.set_nonblocking(true)?;
        let local = listener.local_addr()?.port();
        bound(Bound {
            local,
            remote: spec.remote,
        });
        listeners.push((listener, spec.remote));
    }
    let shared = Shared {
        block,
        wr: Mutex::new(wr),
        forwards: Mutex::new(HashMap::new()),
        asked: Mutex::new(HashMap::new()),
        next: AtomicU64::new(1),
        ended: Mutex::new(None),
    };
    thread::scope(|scope| {
        scope.spawn(|| read_frames(&shared, &mut rd, pending));
        for (listener, remote) in &listeners {
            scope.spawn(|| accept(scope, &shared, listener, *remote));
        }
        while !shared.over() {
            if stopped() {
                shared.end(Ended::Stopped);
            }
            thread::sleep(SLICE);
        }
    });
    let ended = lock(&shared.ended).take();
    Ok(ended.unwrap_or(Ended::Stopped))
}

/// Binds `local`, or the remote port's number if free, else any port.
fn listen(local: Option<u16>, remote: u16) -> io::Result<TcpListener> {
    let on = |port| TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
    match local {
        Some(port) => on(port),
        None => on(remote).or_else(|_| on(0)),
    }
}

/// Decodes what the server sends and hands each frame to whom it is for.
fn read_frames<W: Write>(shared: &Shared<W>, rd: &mut impl Read, mut buf: Vec<u8>) {
    let mut chunk = vec![0u8; 64 << 10];
    while !shared.over() {
        loop {
            match decode(&buf) {
                Ok(Some((message, used))) => {
                    buf.drain(..used);
                    dispatch(shared, message);
                }
                Ok(None) => break,
                Err(e) => {
                    return shared.end(Ended::Lost(io::Error::new(io::ErrorKind::InvalidData, e)));
                }
            }
        }
        match rd.read(&mut chunk) {
            Ok(0) => return shared.end(Ended::Lost(io::ErrorKind::UnexpectedEof.into())),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if is_wait(&e) => {}
            Err(e) => return shared.end(Ended::Lost(e)),
        }
    }
}

fn dispatch<W: Write>(shared: &Shared<W>, message: Message) {
    let answer = |req, result| {
        if let Some(asker) = lock(&shared.asked).remove(&req) {
            // The asker gave up: nothing waits.
            let _ = asker.send(result);
        }
    };
    match message {
        Message::Done { req, .. } => answer(req, Ok(())),
        Message::Failed { req, reason, .. } => answer(req, Err(reason)),
        Message::ForwardData { id, data } => {
            if let Some(fwd) = lock(&shared.forwards).get(&id) {
                // The local side is gone: its close follows.
                let _ = fwd.to_local.send(data);
            }
        }
        Message::ForwardAck { id, n } => {
            if let Some(fwd) = lock(&shared.forwards).get(&id) {
                let mut credit = lock(&fwd.credit);
                *credit = (*credit + n as usize).min(FORWARD_WINDOW);
                fwd.credited.notify_all();
            }
        }
        Message::CloseForward { id, .. } => {
            lock(&shared.forwards).remove(&id);
        }
        Message::Event {
            event: Event::BlockClosed { block },
            ..
        } if block == shared.block => shared.end(Ended::BlockClosed),
        _ => {}
    }
}

/// Takes the connections to one local port.
fn accept<'scope, 'env, W: Write + Send>(
    scope: &'scope Scope<'scope, 'env>,
    shared: &'env Shared<W>,
    listener: &'env TcpListener,
    remote: u16,
) {
    while !shared.over() {
        match listener.accept() {
            Ok((local, _)) => {
                scope.spawn(move || tunnel(scope, shared, local, remote));
            }
            Err(e) if is_wait(&e) => thread::sleep(SLICE),
            // A connection that fell over before it was taken is not ours.
            Err(e) if e.kind() == io::ErrorKind::ConnectionAborted => {}
            Err(e) => return shared.end(Ended::Lost(e)),
        }
    }
}

/// One accepted connection: asks for a forward, then moves bytes both ways
/// until either side ends.
fn tunnel<'scope, 'env, W: Write + Send>(
    scope: &'scope Scope<'scope, 'env>,
    shared: &'env Shared<W>,
    local: TcpStream,
    remote: u16,
) {
    let Some((id, fwd, from_server)) = open(shared, remote) else {
        return;
    };
    let Ok(writer) = local.try_clone() else {
        return close(shared, id);
    };
    scope.spawn(move || downstream(shared, id, writer, from_server));
    upstream(shared, id, &fwd, local);
    close(shared, id);
}

/// Asks for a forward to `remote`, waiting for the server's answer.
fn open<W: Write>(shared: &Shared<W>, remote: u16) -> Option<(u64, Arc<Fwd>, Receiver<Vec<u8>>)> {
    let id = shared.id();
    let (to_local, from_server) = channel();
    let fwd = Arc::new(Fwd {
        to_local,
        credit: Mutex::new(FORWARD_WINDOW),
        credited: Condvar::new(),
    });
    let (answer, answered) = channel();
    lock(&shared.asked).insert(id, answer);
    lock(&shared.forwards).insert(id, fwd.clone());
    shared.say(&Message::OpenForward {
        req: id,
        block: shared.block,
        port: remote,
        id,
    });
    let deadline = Instant::now() + OPEN_WAIT;
    let accepted = loop {
        match answered.recv_timeout(SLICE) {
            Ok(result) => break result.is_ok(),
            Err(_) if shared.over() || Instant::now() >= deadline => break false,
            Err(_) => {}
        }
    };
    if !accepted {
        lock(&shared.asked).remove(&id);
        lock(&shared.forwards).remove(&id);
        return None;
    }
    Some((id, fwd, from_server))
}

/// Server to local socket: writes what arrives, then gives the server
/// credit.
fn downstream<W: Write>(
    shared: &Shared<W>,
    id: u64,
    mut local: TcpStream,
    from_server: Receiver<Vec<u8>>,
) {
    // The sender is dropped with the forward.
    while let Ok(data) = from_server.recv() {
        if local.write_all(&data).is_err() {
            break;
        }
        let n = u32::try_from(data.len()).unwrap_or(u32::MAX);
        shared.say(&Message::ForwardAck { id, n });
    }
    let _ = local.shutdown(Shutdown::Both);
}

/// Local socket to server: reads as much as the server has credit for.
fn upstream<W: Write>(shared: &Shared<W>, id: u64, fwd: &Fwd, mut local: TcpStream) {
    let _ = local.set_read_timeout(Some(SLICE));
    let mut buf = vec![0u8; FORWARD_CHUNK];
    loop {
        let credit = {
            let mut credit = lock(&fwd.credit);
            while *credit == 0 && !shared.over() && still_open(shared, id) {
                credit = fwd
                    .credited
                    .wait_timeout(credit, SLICE)
                    .unwrap_or_else(|p| p.into_inner())
                    .0;
            }
            *credit
        };
        if credit == 0 || shared.over() || !still_open(shared, id) {
            return;
        }
        let n = match local.read(&mut buf[..credit.min(FORWARD_CHUNK)]) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) if is_wait(&e) => continue,
            Err(_) => return,
        };
        *lock(&fwd.credit) -= n;
        shared.say(&Message::ForwardData {
            id,
            data: buf[..n].to_vec(),
        });
    }
}

fn still_open<W: Write>(shared: &Shared<W>, id: u64) -> bool {
    lock(&shared.forwards).contains_key(&id)
}

/// Ends forward `id` from this side, telling the server if it is open.
fn close<W: Write>(shared: &Shared<W>, id: u64) {
    if lock(&shared.forwards).remove(&id).is_some() {
        shared.say(&Message::CloseForward { id, reason: None });
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use rusty_uuid::Uuid;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::AtomicBool;

    const TICK: Duration = Duration::from_millis(20);

    /// The server's end of the connection.
    struct Fake {
        stream: UnixStream,
        buf: Vec<u8>,
    }

    impl Fake {
        /// The next frame, or `None` at a read timeout or the end.
        fn next(&mut self) -> Option<Message> {
            loop {
                if let Some((message, used)) = decode(&self.buf).expect("decode") {
                    self.buf.drain(..used);
                    return Some(message);
                }
                let mut chunk = [0u8; 4096];
                match self.stream.read(&mut chunk) {
                    Ok(0) => return None,
                    Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                    Err(e) if is_wait(&e) => return None,
                    Err(e) => panic!("{e}"),
                }
            }
        }

        fn send(&mut self, message: &Message) {
            let _ = self.stream.write_all(&encode(message).expect("encode"));
        }
    }

    /// Runs the client against a server that answers each frame with
    /// `handler`, while `test` uses the first port it listens on. Stops
    /// the client when `test` returns, unless it ended already.
    fn with_server<T: Send>(
        handler: impl FnMut(&mut Fake, Message) + Send,
        test: impl FnOnce(u16) -> T + Send,
    ) -> (Ended, T) {
        let (near, far) = UnixStream::pair().expect("pair");
        near.set_read_timeout(Some(TICK)).expect("timeout");
        far.set_read_timeout(Some(TICK)).expect("timeout");
        let stop = AtomicBool::new(false);
        // The server stays until the client has ended.
        let over = AtomicBool::new(false);
        let (port_tx, port_rx) = channel();
        let (ended, tested) = thread::scope(|scope| {
            let (stop, over) = (&stop, &over);
            scope.spawn(move || {
                let mut handler = handler;
                let mut fake = Fake {
                    stream: far,
                    buf: Vec::new(),
                };
                while !over.load(Ordering::SeqCst) {
                    if let Some(message) = fake.next() {
                        handler(&mut fake, message);
                    }
                }
            });
            let tested = scope.spawn(move || {
                let port = port_rx.recv().expect("port");
                let out = test(port);
                stop.store(true, Ordering::SeqCst);
                out
            });
            let ended = run(
                near.try_clone().expect("clone"),
                near,
                Vec::new(),
                BlockId::from_uuid(Uuid::nil()),
                &[Spec {
                    local: Some(0),
                    remote: 9,
                }],
                &|| stop.load(Ordering::SeqCst),
                &mut |bound| port_tx.send(bound.local).expect("test is waiting"),
            )
            .expect("ran");
            over.store(true, Ordering::SeqCst);
            stop.store(true, Ordering::SeqCst);
            (ended, tested.join().expect("test"))
        });
        (ended, tested)
    }

    fn connect(port: u16) -> TcpStream {
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        stream
    }

    /// Accepts every forward and echoes its data.
    fn echo(fake: &mut Fake, message: Message) {
        match message {
            Message::OpenForward { req, .. } => fake.send(&Message::Done { req, created: None }),
            Message::ForwardData { id, data } => {
                let n = data.len() as u32;
                fake.send(&Message::ForwardAck { id, n });
                fake.send(&Message::ForwardData { id, data });
            }
            _ => {}
        }
    }

    #[test]
    fn a_local_connection_is_tunnelled_and_echoed_back() {
        let (ended, got) = with_server(echo, |port| {
            let mut local = connect(port);
            local.write_all(b"hello").expect("write");
            let mut back = [0u8; 5];
            local.read_exact(&mut back).expect("read");
            back
        });
        assert_eq!(&got, b"hello");
        assert!(matches!(ended, Ended::Stopped), "{ended:?}");
    }

    #[test]
    fn a_refused_forward_closes_the_local_connection() {
        let refuse = |fake: &mut Fake, message| {
            if let Message::OpenForward { req, .. } = message {
                fake.send(&Message::Failed {
                    req,
                    reason: "no lease".into(),
                    code: None,
                });
            }
        };
        let (_, read) = with_server(refuse, |port| {
            let mut local = connect(port);
            let mut byte = [0u8; 1];
            local.read(&mut byte).expect("closed, not an error")
        });
        assert_eq!(read, 0);
    }

    #[test]
    fn nothing_is_sent_past_the_window_until_the_server_acknowledges() {
        let counted = Arc::new(Mutex::new(0usize));
        let seen = counted.clone();
        let silent = move |fake: &mut Fake, message| match message {
            Message::OpenForward { req, .. } => fake.send(&Message::Done { req, created: None }),
            Message::ForwardData { data, .. } => *lock(&seen) += data.len(),
            _ => {}
        };
        let probe = counted.clone();
        let (_, before) = with_server(silent, move |port| {
            let mut local = connect(port);
            let big = vec![7u8; FORWARD_WINDOW + 64 * 1024];
            // The kernel buffers hold the rest while the client waits.
            local.set_nonblocking(true).expect("nonblocking");
            let mut sent = 0;
            while sent < big.len() {
                match local.write(&big[sent..]) {
                    Ok(n) => sent += n,
                    Err(e) if is_wait(&e) => break,
                    Err(e) => panic!("{e}"),
                }
            }
            thread::sleep(Duration::from_millis(500));
            *lock(&probe)
        });
        assert_eq!(before, FORWARD_WINDOW, "exactly the window, then it waits");
    }

    #[test]
    fn credit_returned_lets_more_through() {
        let counted = Arc::new(Mutex::new(0usize));
        let seen = counted.clone();
        let acking = move |fake: &mut Fake, message| match message {
            Message::OpenForward { req, .. } => fake.send(&Message::Done { req, created: None }),
            Message::ForwardData { id, data } => {
                *lock(&seen) += data.len();
                fake.send(&Message::ForwardAck {
                    id,
                    n: data.len() as u32,
                });
            }
            _ => {}
        };
        let probe = counted.clone();
        let total = FORWARD_WINDOW * 2;
        let (_, got) = with_server(acking, move |port| {
            let mut local = connect(port);
            local.write_all(&vec![1u8; total]).expect("write");
            let deadline = Instant::now() + Duration::from_secs(10);
            while *lock(&probe) < total && Instant::now() < deadline {
                thread::sleep(TICK);
            }
            *lock(&probe)
        });
        assert_eq!(got, total);
    }

    #[test]
    fn the_server_closing_a_forward_closes_the_local_connection() {
        let close = |fake: &mut Fake, message| {
            if let Message::OpenForward { req, id, .. } = message {
                fake.send(&Message::Done { req, created: None });
                fake.send(&Message::CloseForward {
                    id,
                    reason: Some("refused".into()),
                });
            }
        };
        let (_, read) = with_server(close, |port| {
            let mut local = connect(port);
            let mut byte = [0u8; 1];
            local.read(&mut byte).unwrap_or(0)
        });
        assert_eq!(read, 0);
    }

    #[test]
    fn the_block_closing_ends_the_run() {
        let (ended, ()) = with_server(
            |fake, message| {
                if let Message::OpenForward { req, .. } = message {
                    fake.send(&Message::Done { req, created: None });
                    fake.send(&Message::Event {
                        seq: None,
                        event: Event::BlockClosed {
                            block: BlockId::from_uuid(Uuid::nil()),
                        },
                    });
                }
            },
            |port| {
                let mut local = connect(port);
                let mut byte = [0u8; 1];
                // Closed by the end of the run.
                let _ = local.read(&mut byte);
            },
        );
        assert!(matches!(ended, Ended::BlockClosed), "{ended:?}");
    }

    #[test]
    fn a_lost_connection_ends_the_run() {
        let (near, far) = UnixStream::pair().expect("pair");
        near.set_read_timeout(Some(TICK)).expect("timeout");
        drop(far);
        let ended = run(
            near.try_clone().expect("clone"),
            near,
            Vec::new(),
            BlockId::from_uuid(Uuid::nil()),
            &[],
            &|| false,
            &mut |_| {},
        )
        .expect("ran");
        assert!(matches!(ended, Ended::Lost(_)), "{ended:?}");
    }

    #[test]
    fn a_port_in_use_is_an_error_before_anything_runs() {
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
        let port = taken.local_addr().expect("addr").port();
        let (near, _far) = UnixStream::pair().expect("pair");
        let err = run(
            near.try_clone().expect("clone"),
            near,
            Vec::new(),
            BlockId::from_uuid(Uuid::nil()),
            &[Spec {
                local: Some(port),
                remote: 1,
            }],
            &|| false,
            &mut |_| panic!("bound"),
        )
        .expect_err("in use");
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
    }

    #[test]
    fn the_remote_number_is_used_when_free_and_another_when_not() {
        let first = listen(None, 0).expect("any port");
        let port = first.local_addr().expect("addr").port();
        // Taken now: asking for the same number gets a different one.
        let second = listen(None, port).expect("another");
        assert_ne!(second.local_addr().expect("addr").port(), port);
    }
}
