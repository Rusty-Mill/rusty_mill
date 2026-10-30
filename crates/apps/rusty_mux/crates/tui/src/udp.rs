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
    stop: AtomicBool,
    /// Each local connection still read from, to unblock its reader on
    /// stop.
    locals: Mutex<HashMap<StreamId, UnixStream>>,
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
            stop: AtomicBool::new(false),
            locals: Mutex::default(),
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
}

impl Drop for Relay {
    fn drop(&mut self) {
        {
            let mut assoc = self.shared.lock();
            assoc.end();
            self.shared.transmit(&mut assoc);
        }
        self.shared.stop.store(true, Ordering::Release);
        self.shared.changed.notify_all();
        for local in self
            .shared
            .locals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
        {
            // It may have closed already.
            let _ = local.shutdown(Shutdown::Both);
        }
        // Wakes the acceptor, which then sees the stop.
        let _ = UnixStream::connect(&self.socket);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
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
    fn transmit(&self, assoc: &mut Association) {
        let udp = self.udp();
        while let Some((to, datagram)) = assoc.transmit(Instant::now()) {
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
            self.locals
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id, kept);
            let shared = self.clone();
            std::thread::spawn(move || shared.up(id, up));
            let shared = self.clone();
            std::thread::spawn(move || shared.down(id, local));
        }
    }

    /// Carries what the local side writes onto stream `id`. When the local
    /// side finishes, the stream closes once the server has it all.
    fn up(&self, id: StreamId, local: UnixStream) {
        self.carry_up(id, local);
        let mut locals = self.locals.lock().unwrap_or_else(PoisonError::into_inner);
        locals.remove(&id);
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
    fn down(&self, id: StreamId, mut local: UnixStream) {
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
}
