//! UDP for `--ssh` (docs/projects/mux/UDP-TRANSPORT.md): one association to
//! the server, its streams served on a private local Unix socket, as an SSH
//! forward serves its connections. Each connection to the local socket is a
//! new stream, so the client connects, and takes its clients over, exactly
//! as it does through a forward.
//!
//! One thread receives datagrams and wakes at the association's timers;
//! one accepts local connections; each connection has a thread for each
//! direction. The association sits behind one lock, with one condition
//! variable that wakes every waiting thread when anything arrives.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, UdpSocket};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusty_mux_transport::{Association, ID_LEN, KEY_LEN, MAX_DATAGRAM, Side, StreamId};

/// The longest any thread waits before checking whether the relay stopped.
const TICK: Duration = Duration::from_millis(100);

/// How long a stream the local side finished waits for the server to
/// acknowledge what it still holds before it closes anyway.
const LINGER: Duration = Duration::from_secs(10);

/// How long dropping a relay waits for the server to acknowledge what
/// finished local connections wrote, such as a last Detach.
const DRAIN: Duration = Duration::from_secs(2);

/// The most read from a local connection at once.
const CHUNK: usize = 16 << 10;

/// A running association and the private directory of its local socket.
/// Dropping it ends the association and stops every thread.
pub struct Relay {
    shared: Arc<Shared>,
    dir: PathBuf,
    socket: PathBuf,
    threads: Vec<JoinHandle<()>>,
}

struct Shared {
    assoc: Mutex<Association>,
    /// Signalled whenever a datagram is taken in.
    changed: Condvar,
    udp: Mutex<Arc<UdpSocket>>,
    /// Where datagrams go: the server address being tried or in use.
    to: Mutex<SocketAddr>,
    stop: AtomicBool,
    /// Each local connection, until both its workers finish, to unblock
    /// them on stop.
    locals: Mutex<HashMap<StreamId, Carried>>,
    /// Every connection's workers, joined on stop.
    workers: Mutex<Vec<JoinHandle<()>>>,
}

/// A local connection and how many of its two workers still run.
struct Carried {
    local: UnixStream,
    workers: u8,
}

impl Relay {
    /// Starts association `id` under `key` with the server at `server`,
    /// sending from `bind`, and serves its streams on a new local socket.
    /// Nothing is sent until a stream opens or [`Self::probe`] asks.
    pub fn start(
        bind: SocketAddr,
        server: SocketAddr,
        id: [u8; ID_LEN],
        key: &[u8; KEY_LEN],
    ) -> Result<Self, String> {
        let udp = UdpSocket::bind(bind).map_err(|e| format!("binding UDP to {bind}: {e}"))?;
        let assoc = Association::new(Side::Client, id, key, Instant::now(), Some(server));
        let dir =
            crate::ssh::private_dir().map_err(|e| format!("making the relay's directory: {e}"))?;
        let socket = dir.join("s");
        let listener = match UnixListener::bind(&socket) {
            Ok(listener) => listener,
            Err(e) => {
                remove(&dir);
                return Err(format!("listening on {}: {e}", socket.display()));
            }
        };
        let shared = Arc::new(Shared {
            assoc: Mutex::new(assoc),
            changed: Condvar::new(),
            udp: Mutex::new(Arc::new(udp)),
            to: Mutex::new(server),
            stop: AtomicBool::new(false),
            locals: Mutex::default(),
            workers: Mutex::default(),
        });
        let receiver = {
            let shared = shared.clone();
            std::thread::spawn(move || shared.receive())
        };
        let acceptor = {
            let shared = shared.clone();
            std::thread::spawn(move || shared.accept(listener))
        };
        Ok(Self {
            shared,
            dir,
            socket,
            threads: vec![receiver, acceptor],
        })
    }

    /// The local socket to connect to; each connection is a new stream.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Asks the server for an answer and waits up to `patience` for one:
    /// whether the path works now.
    pub fn probe(&self, patience: Duration) -> bool {
        let by = Instant::now() + patience;
        let mut assoc = self.shared.lock();
        let before = assoc.last_heard();
        assoc.probe();
        self.shared.transmit(&mut assoc);
        loop {
            if assoc.last_heard() != before {
                return true;
            }
            let left = by.saturating_duration_since(Instant::now());
            if left.is_zero() || assoc.is_ended() {
                return false;
            }
            assoc = self.shared.wait(assoc, left);
        }
    }

    /// Sends from a new socket bound to `bind` from now on, as a client
    /// does when its address changes; the server follows the next datagram.
    pub fn rebind(&self, bind: SocketAddr) -> io::Result<()> {
        let udp = UdpSocket::bind(bind)?;
        *self
            .shared
            .udp
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Arc::new(udp);
        Ok(())
    }

    /// Sends to `server` from now on, from any address of its family. The
    /// association and its sequence numbers carry on, so trying another
    /// address of a host never seals two datagrams under one nonce.
    pub fn retarget(&self, server: SocketAddr) -> io::Result<()> {
        self.rebind(any_for(server))?;
        *self
            .shared
            .to
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = server;
        Ok(())
    }
}

impl Relay {
    /// Waits, at most [`DRAIN`], until every local connection has ended
    /// and the server has acknowledged what it wrote: each one leaves
    /// `locals` only once both its workers finish.
    fn drain(&self) {
        let by = Instant::now() + DRAIN;
        let mut assoc = self.shared.lock();
        while !assoc.is_ended() && !self.shared.carried().is_empty() {
            let left = by.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            assoc = self.shared.wait(assoc, left);
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.drain();
        {
            let mut assoc = self.shared.lock();
            // One never answered has nobody to tell.
            if assoc.last_heard().is_some() {
                assoc.end();
                self.shared.transmit(&mut assoc);
            }
        }
        self.shared.stop.store(true, Ordering::Release);
        self.shared.changed.notify_all();
        for carried in self.shared.carried().values() {
            // It may have closed already.
            let _ = carried.local.shutdown(Shutdown::Both);
        }
        // Wakes the acceptor, which then sees the stop.
        let _ = UnixStream::connect(&self.socket);
        // The acceptor first, so no worker starts after the rest are joined.
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        let workers = std::mem::take(
            &mut *self
                .shared
                .workers
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for worker in workers {
            let _ = worker.join();
        }
        remove(&self.dir);
    }
}

fn remove(dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(dir) {
        eprintln!("rusty-mux: removing {}: {e}", dir.display());
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Association> {
        self.assoc.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(
        &self,
        assoc: MutexGuard<'a, Association>,
        at_most: Duration,
    ) -> MutexGuard<'a, Association> {
        let (assoc, _) = self
            .changed
            .wait_timeout(assoc, at_most.min(TICK))
            .unwrap_or_else(PoisonError::into_inner);
        assoc
    }

    fn carried(&self) -> MutexGuard<'_, HashMap<StreamId, Carried>> {
        self.locals.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One of `id`'s workers finished; the connection goes with the last.
    fn finished(&self, id: StreamId) {
        let mut carried = self.carried();
        let Some(entry) = carried.get_mut(&id) else {
            return;
        };
        entry.workers -= 1;
        if entry.workers == 0 {
            carried.remove(&id);
        }
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    fn udp(&self) -> Arc<UdpSocket> {
        self.udp
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Sends every datagram the association has ready. A failed send is
    /// a datagram lost, which the association already recovers from.
    /// Every datagram goes to `to`, which a client chooses, not to where
    /// the association last heard from.
    fn transmit(&self, assoc: &mut Association) {
        let udp = self.udp();
        let to = *self.to.lock().unwrap_or_else(PoisonError::into_inner);
        while let Some((_, datagram)) = assoc.transmit(Instant::now()) {
            let _ = udp.send_to(&datagram, to);
        }
    }

    /// Takes in datagrams and sends what falls due, until stopped.
    fn receive(&self) {
        // One byte more than the largest datagram, to see one that is too big.
        let mut buf = vec![0; MAX_DATAGRAM + 1];
        while !self.stopped() {
            let udp = self.udp();
            let wait = {
                let mut assoc = self.lock();
                self.transmit(&mut assoc);
                let now = Instant::now();
                assoc
                    .timeout()
                    .map_or(TICK, |at| at.saturating_duration_since(now).min(TICK))
            };
            // A zero timeout is an error; the next pass sends what is due.
            if udp
                .set_read_timeout(Some(wait.max(Duration::from_millis(1))))
                .is_err()
            {
                return;
            }
            let Ok((n, from)) = udp.recv_from(&mut buf) else {
                continue;
            };
            if n > MAX_DATAGRAM {
                continue;
            }
            let mut assoc = self.lock();
            if assoc.datagram(Instant::now(), from, &buf[..n]) {
                // Peer closes are seen through `is_open`; the queue only drains.
                while assoc.closed().is_some() {}
                self.transmit(&mut assoc);
                self.changed.notify_all();
            }
        }
    }

    /// Opens a stream for each local connection, until stopped.
    fn accept(self: Arc<Self>, listener: UnixListener) {
        for local in listener.incoming() {
            if self.stopped() {
                return;
            }
            let Ok(local) = local else { continue };
            let Some(id) = self.lock().open() else {
                // At the association's stream limit: the connection fails.
                continue;
            };
            let (Ok(up), Ok(kept)) = (local.try_clone(), local.try_clone()) else {
                self.lock().close(Instant::now(), id);
                continue;
            };
            let mut carried = self.carried();
            if self.stopped() {
                // After the stop's sweep of `locals`: never carried. The
                // association's lock is never taken under `locals`.
                drop(carried);
                let _ = local.shutdown(Shutdown::Both);
                self.lock().close(Instant::now(), id);
                return;
            }
            let entry = Carried {
                local: kept,
                workers: 2,
            };
            carried.insert(id, entry);
            drop(carried);
            let (shared, other) = (self.clone(), self.clone());
            let mut workers = self.workers.lock().unwrap_or_else(PoisonError::into_inner);
            workers.retain(|w| !w.is_finished());
            workers.push(std::thread::spawn(move || shared.up(id, up)));
            workers.push(std::thread::spawn(move || other.down(id, local)));
        }
    }

    /// Carries what the local side writes onto stream `id`. When the local
    /// side finishes, the stream closes once the server has it all.
    fn up(&self, id: StreamId, local: UnixStream) {
        self.carry_up(id, local);
        self.finished(id);
    }

    fn carry_up(&self, id: StreamId, mut local: UnixStream) {
        let mut buf = vec![0; CHUNK];
        loop {
            let n = match local.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if !self.put(id, &buf[..n]) {
                return;
            }
        }
        let by = Instant::now() + LINGER;
        let mut assoc = self.lock();
        while assoc.is_open(id) && assoc.queued(id) > 0 && !self.stopped() {
            let left = by.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            assoc = self.wait(assoc, left);
        }
        assoc.close(Instant::now(), id);
        self.transmit(&mut assoc);
    }

    /// Queues all of `bytes` on `id`, waiting for room; `false` once the
    /// stream is gone.
    fn put(&self, id: StreamId, mut bytes: &[u8]) -> bool {
        let mut assoc = self.lock();
        while !bytes.is_empty() {
            if !assoc.is_open(id) || assoc.is_ended() || self.stopped() {
                return false;
            }
            let n = assoc.write(id, bytes);
            bytes = &bytes[n..];
            self.transmit(&mut assoc);
            if n == 0 {
                assoc = self.wait(assoc, TICK);
            }
        }
        true
    }

    /// Carries what arrives on stream `id` to the local side, which sees
    /// the end once the server closes the stream and all of it is read.
    fn down(&self, id: StreamId, local: UnixStream) {
        self.carry_down(id, local);
        self.finished(id);
    }

    fn carry_down(&self, id: StreamId, mut local: UnixStream) {
        let mut buf = vec![0; CHUNK];
        loop {
            let n = {
                let mut assoc = self.lock();
                loop {
                    let n = assoc.read(id, &mut buf);
                    if n > 0 {
                        // Reading may reopen the server's window.
                        self.transmit(&mut assoc);
                        break n;
                    }
                    if !assoc.is_open(id) || assoc.is_ended() || self.stopped() {
                        drop(assoc);
                        let _ = local.shutdown(Shutdown::Write);
                        return;
                    }
                    assoc = self.wait(assoc, TICK);
                }
            };
            if local.write_all(&buf[..n]).is_err() {
                self.lock().close(Instant::now(), id);
                return;
            }
        }
    }
}

/// Starts association `id` under `key` with the first of `servers` that
/// answers a probe, trying each in turn with an equal share of what is
/// left of `patience`: a host may resolve to addresses UDP cannot reach.
/// One association tries them all, retargeted, so its sequence numbers,
/// and with them its nonces, never repeat under the one key.
pub fn reach(
    servers: &[SocketAddr],
    id: [u8; ID_LEN],
    key: &[u8; KEY_LEN],
    patience: Duration,
) -> Result<Relay, String> {
    let by = Instant::now() + patience;
    let mut why = String::from("no address to send to");
    let mut relay: Option<Relay> = None;
    for (i, &server) in servers.iter().enumerate() {
        let left = by.saturating_duration_since(Instant::now());
        let share = left / (servers.len() - i) as u32;
        let aimed = match relay.as_ref() {
            Some(started) => started
                .retarget(server)
                .map(|()| started)
                .map_err(|e| format!("binding UDP to reach {server}: {e}")),
            None => Relay::start(any_for(server), server, id, key).map(|r| &*relay.insert(r)),
        };
        match aimed {
            Ok(aimed) if aimed.probe(share) => return relay.ok_or(why),
            Ok(_) => why = format!("no answer on UDP from {server}"),
            Err(e) => why = e,
        }
    }
    Err(why)
}

/// Where to send from to reach `server`: any address of its family.
pub fn any_for(server: SocketAddr) -> SocketAddr {
    match server {
        SocketAddr::V4(_) => SocketAddr::from(([0, 0, 0, 0], 0)),
        SocketAddr::V6(_) => SocketAddr::from(([0u16; 8], 0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unanswered_probe_fails_in_time_and_drop_cleans_up() {
        // A port nobody answers on: bound, never read.
        let silent = UdpSocket::bind("127.0.0.1:0").expect("bind");
        let server = silent.local_addr().expect("addr");
        let relay =
            Relay::start(any_for(server), server, [1; ID_LEN], &[2; KEY_LEN]).expect("start");
        let dir = relay.dir.clone();
        assert!(relay.socket().exists());
        let started = Instant::now();
        assert!(!relay.probe(Duration::from_millis(300)));
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(relay);
        assert!(!dir.exists());
    }

    /// The sequence numbers of every datagram waiting on `socket`.
    fn sequence_numbers(socket: &UdpSocket) -> Vec<u64> {
        socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .expect("timeout");
        let mut buf = vec![0; MAX_DATAGRAM];
        let mut seqs = Vec::new();
        while let Ok(n) = socket.recv(&mut buf) {
            let header = buf[ID_LEN..n.min(ID_LEN + 8)].try_into().expect("a header");
            seqs.push(u64::from_be_bytes(header));
        }
        seqs
    }

    #[test]
    fn every_address_tried_shares_one_sequence_space() {
        let (first, second) = (
            UdpSocket::bind("127.0.0.1:0").expect("bind"),
            UdpSocket::bind("127.0.0.1:0").expect("bind"),
        );
        let servers = [
            first.local_addr().expect("addr"),
            second.local_addr().expect("addr"),
        ];
        let tried = reach(
            &servers,
            [5; ID_LEN],
            &[6; KEY_LEN],
            Duration::from_millis(600),
        );
        assert!(tried.is_err());
        let (to_first, to_second) = (sequence_numbers(&first), sequence_numbers(&second));
        assert!(!to_first.is_empty() && !to_second.is_empty());
        // Under one key, a sequence number reused is a nonce reused.
        let last_first = to_first.iter().max().expect("one");
        let first_second = to_second.iter().min().expect("one");
        assert!(
            first_second > last_first,
            "the second address got {to_second:?} after {to_first:?}"
        );
    }

    /// Runs `f` on its own thread: whether it finished within `bound`.
    fn within(bound: Duration, f: impl FnOnce() + Send + 'static) -> bool {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            f();
            let _ = done.send(());
        });
        finished.recv_timeout(bound).is_ok()
    }

    #[test]
    fn dropping_a_relay_on_a_dead_path_gives_up_after_the_drain() {
        let (id, key) = ([7; ID_LEN], [8; KEY_LEN]);
        let stop = Arc::new(AtomicBool::new(false));
        let (server, flood) = flooding_server(id, key, 0, stop.clone());
        let relay = Relay::start(any_for(server), server, id, &key).expect("start");
        assert!(relay.probe(Duration::from_secs(2)));
        // The path dies, then a last request is written and hung up on.
        stop.store(true, Ordering::Release);
        flood.join().expect("server");
        let mut local = UnixStream::connect(relay.socket()).expect("connect");
        // Carried before hanging up, as a client's control connection is.
        while relay.shared.carried().is_empty() {
            std::thread::sleep(Duration::from_millis(5));
        }
        local.write_all(b"detach").expect("write");
        local.shutdown(Shutdown::Both).expect("hang up");
        let started = Instant::now();
        assert!(within(DRAIN + Duration::from_secs(2), move || drop(relay)));
        assert!(started.elapsed() >= DRAIN, "gave up before the drain");
    }

    #[test]
    fn dropping_the_relay_while_connections_arrive_finishes() {
        let silent = UdpSocket::bind("127.0.0.1:0").expect("bind");
        let server = silent.local_addr().expect("addr");
        let relay =
            Relay::start(any_for(server), server, [9; ID_LEN], &[10; KEY_LEN]).expect("start");
        let socket = relay.socket().to_owned();
        let stop = Arc::new(AtomicBool::new(false));
        let dialer = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                // Each connection lingers with its workers, so a bounded
                // number, paced to keep arriving through the drop.
                let mut held = Vec::new();
                while !stop.load(Ordering::Acquire) && held.len() < 256 {
                    if let Ok(local) = UnixStream::connect(&socket) {
                        held.push(local);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
        };
        std::thread::sleep(Duration::from_millis(100));
        let finished = within(DRAIN + Duration::from_secs(5), move || drop(relay));
        stop.store(true, Ordering::Release);
        dialer.join().expect("dialer");
        assert!(finished, "dropping the relay hung on arriving connections");
    }

    /// A server end, driven on its own thread: it accepts every stream
    /// and sends `flood` bytes down each, until `stop` is set.
    fn flooding_server(
        id: [u8; ID_LEN],
        key: [u8; KEY_LEN],
        flood: usize,
        stop: Arc<AtomicBool>,
    ) -> (SocketAddr, JoinHandle<()>) {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind");
        let addr = socket.local_addr().expect("addr");
        socket
            .set_read_timeout(Some(Duration::from_millis(5)))
            .expect("timeout");
        let thread = std::thread::spawn(move || {
            let mut assoc = Association::new(Side::Server, id, &key, Instant::now(), None);
            let mut sent: HashMap<StreamId, usize> = HashMap::new();
            let mut buf = vec![0; MAX_DATAGRAM];
            let chunk = vec![b'x'; CHUNK];
            while !stop.load(Ordering::Acquire) {
                if let Ok((n, from)) = socket.recv_from(&mut buf) {
                    assoc.datagram(Instant::now(), from, &buf[..n]);
                }
                while let Some(id) = assoc.accept() {
                    sent.insert(id, 0);
                }
                for (&id, done) in sent.iter_mut() {
                    let left = flood.saturating_sub(*done).min(CHUNK);
                    *done += assoc.write(id, &chunk[..left]);
                }
                while let Some((to, datagram)) = assoc.transmit(Instant::now()) {
                    let _ = socket.send_to(&datagram, to);
                }
            }
        });
        (addr, thread)
    }

    #[test]
    fn dropping_the_relay_stops_a_worker_blocked_on_a_stalled_reader() {
        let (id, key) = ([3; ID_LEN], [4; KEY_LEN]);
        let stop = Arc::new(AtomicBool::new(false));
        // Far more than a Unix socket buffers, so the down worker blocks.
        let (server, flood) = flooding_server(id, key, 8 << 20, stop.clone());
        let relay = Relay::start(any_for(server), server, id, &key).expect("start");
        assert!(relay.probe(Duration::from_secs(2)));
        let bound = relay.shared.udp().local_addr().expect("addr");
        let mut local = UnixStream::connect(relay.socket()).expect("connect");
        local.write_all(b"hello").expect("write");
        // Reads nothing, so the down worker fills the socket and blocks;
        // then finishes its own side, so the up worker ends.
        std::thread::sleep(Duration::from_millis(500));
        local.shutdown(Shutdown::Write).expect("half close");
        std::thread::sleep(Duration::from_millis(200));

        let (done, dropped) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(relay);
            let _ = done.send(());
        });
        let joined = dropped.recv_timeout(Duration::from_secs(10));
        stop.store(true, Ordering::Release);
        flood.join().expect("server");
        assert!(
            joined.is_ok(),
            "dropping the relay hung on a blocked worker"
        );
        // Every thread is gone with the relay, so nothing holds its socket,
        // though the local side never read or closed.
        assert!(
            UdpSocket::bind(bound).is_ok(),
            "a worker outlived the relay"
        );
        drop(local);
    }
}
