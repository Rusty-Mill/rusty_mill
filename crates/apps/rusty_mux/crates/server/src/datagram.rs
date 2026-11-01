//! The server's UDP side (docs/projects/mux/UDP-TRANSPORT.md): one socket
//! for every association, and each association's streams served by
//! [`conn::run`] exactly as socket connections are.
//!
//! A control connection asks for an association with `OpenDatagram`; the
//! [`Registry`] makes up its id and key and keeps them as an offer. The
//! first authentic datagram under that id makes it live: a [`Link`] holding
//! the pure [`Association`], with one task ([`drive`]) that sends what it
//! has to send, wakes at its timers and ends it after `idle` of silence.
//! Silence of [`LEASE_SILENCE`] already releases the leases its clients
//! hold, so others can type; the next datagram heard gives them back where
//! nobody took them meanwhile.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use rusty_mux_core::ClientId;
use rusty_mux_transport::{Association, ID_LEN, KEY_LEN, MAX_DATAGRAM, Side, StreamId};
use rusty_tokio::io::{AsyncRead, AsyncWrite, ReadBuf, UdpSocket};
use rusty_tokio::sync::Notify;

use crate::conn;
use crate::state::{Released, Shared};

pub(crate) type Id = [u8; ID_LEN];
pub(crate) type Key = [u8; KEY_LEN];

/// How long an association's clients keep their leases through silence.
pub(crate) const LEASE_SILENCE: Duration = Duration::from_secs(30);

/// How long an offered association waits for its first datagram.
const OFFER_TTL: Duration = Duration::from_secs(60);

/// Room for any UDP payload, over IPv4 or IPv6: a UDP length is 16 bits.
const UDP_MAX: usize = 1 << 16;

/// Winsock's WSAEMSGSIZE: a datagram larger than the receive buffer.
const WSAEMSGSIZE: i32 = 10040;

/// The most associations, offered or live, a server holds at once.
const MAX_ASSOCIATIONS: usize = 1024;

/// The shortest sleep between two passes of a link's driver, so a timer
/// already due that sends nothing cannot spin the thread.
const MIN_SLEEP: Duration = Duration::from_millis(1);

/// The associations the server offered and those that are live, by id.
#[derive(Clone)]
pub(crate) struct Registry {
    port: u16,
    slots: Rc<RefCell<HashMap<Id, Slot>>>,
}

enum Slot {
    Offered { key: Key, at: Instant },
    Live(Rc<Link>),
}

impl Registry {
    pub(crate) fn new(port: u16) -> Self {
        Self {
            port,
            slots: Rc::default(),
        }
    }

    /// A fresh association, from the OS's CSPRNG: the port to send to, its
    /// id and its key. Offers older than [`OFFER_TTL`] are forgotten, and
    /// none is made while [`MAX_ASSOCIATIONS`] are held.
    pub(crate) fn offer(&self, now: Instant) -> Result<(u16, Id, Key), String> {
        let mut slots = self.slots.borrow_mut();
        slots.retain(|_, slot| match slot {
            Slot::Offered { at, .. } => !expired(*at, now),
            Slot::Live(_) => true,
        });
        if slots.len() >= MAX_ASSOCIATIONS {
            return Err("too many UDP associations".into());
        }
        let (mut id, mut key) = ([0; ID_LEN], [0; KEY_LEN]);
        rusty_rand::fill(&mut id)
            .and_then(|()| rusty_rand::fill(&mut key))
            .map_err(|e| format!("drawing an association key: {e:?}"))?;
        if slots.contains_key(&id) {
            return Err("association id already in use".into());
        }
        slots.insert(id, Slot::Offered { key, at: now });
        Ok((self.port, id, key))
    }

    /// Hands `datagram` to its association. Returns the link when this
    /// datagram is the first authentic one of an offer, for the caller to
    /// drive; anything not authentic changes nothing.
    fn datagram(&self, now: Instant, from: SocketAddr, datagram: &[u8]) -> Received {
        let Some(id) = datagram.get(..ID_LEN).and_then(|id| Id::try_from(id).ok()) else {
            return Received::Nothing;
        };
        let mut slots = self.slots.borrow_mut();
        let key = match slots.get(&id) {
            None => return Received::Nothing,
            Some(Slot::Live(link)) => {
                let link = link.clone();
                drop(slots);
                return match link.datagram(now, from, datagram) {
                    true => Received::Live(link),
                    false => Received::Nothing,
                };
            }
            Some(Slot::Offered { at, .. }) if expired(*at, now) => {
                slots.remove(&id);
                return Received::Nothing;
            }
            Some(Slot::Offered { key, .. }) => *key,
        };
        let mut assoc = Association::new(Side::Server, id, &key, now, None);
        if !assoc.datagram(now, from, datagram) {
            return Received::Nothing;
        }
        let link = Rc::new(Link::new(id, assoc, now));
        slots.insert(id, Slot::Live(link.clone()));
        Received::New(link)
    }

    fn remove(&self, id: &Id) {
        self.slots.borrow_mut().remove(id);
    }

    #[cfg(test)]
    pub(crate) fn live(&self) -> usize {
        let slots = self.slots.borrow();
        slots
            .values()
            .filter(|s| matches!(s, Slot::Live(_)))
            .count()
    }
}

/// Whether an offer made `at` can no longer be taken up.
fn expired(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) >= OFFER_TTL
}

enum Received {
    Nothing,
    Live(Rc<Link>),
    New(Rc<Link>),
}

/// Receives on `socket` until receiving fails, starting a driver for each
/// association that comes alive and a connection for each stream it opens.
/// An association silent for `idle` ends; one silent for `silence` loses
/// its clients' leases until it is heard again.
pub(crate) async fn serve(
    socket: Rc<UdpSocket>,
    registry: Registry,
    state: Shared,
    idle: Duration,
    silence: Duration,
) -> io::Result<()> {
    // Room for any UDP datagram: one larger than the buffer is an error on
    // Windows (WSAEMSGSIZE), which would end the server. Too big for the
    // transport, it is dropped below.
    let mut buf = vec![0; UDP_MAX];
    loop {
        let (n, from) = match socket.recv_from(&mut buf).await {
            Ok(received) => received,
            // Windows reports an earlier send's ICMP unreachable here.
            Err(err) if err.kind() == io::ErrorKind::ConnectionReset => continue,
            // Cannot happen with this buffer; never worth ending the server.
            Err(err) if cfg!(windows) && err.raw_os_error() == Some(WSAEMSGSIZE) => continue,
            Err(err) => return Err(err),
        };
        if n > MAX_DATAGRAM {
            continue;
        }
        let link = match registry.datagram(Instant::now(), from, &buf[..n]) {
            Received::Nothing => continue,
            Received::Live(link) => {
                link.heard(&state);
                link
            }
            Received::New(link) => {
                let timers = Timers { idle, silence };
                let driver = drive(
                    link.clone(),
                    socket.clone(),
                    registry.clone(),
                    state.clone(),
                    timers,
                );
                rusty_tokio::task::spawn_local(driver);
                link
            }
        };
        while let Some(id) = link.accept() {
            let rd = StreamReader {
                link: link.clone(),
                id,
            };
            let wr = StreamWriter {
                link: link.clone(),
                id,
            };
            let (adopter, known) = (link.clone(), state.clone());
            let adopt = move |client, cancel: &Rc<Notify>| {
                known.borrow_mut().over_datagram(client, cancel);
                adopter.adopt(client, cancel, &known);
            };
            rusty_tokio::task::spawn_local(conn::run(state.clone(), rd, wr, adopt));
        }
    }
}

/// How long a link's silence may last: before it ends, and before its
/// clients' leases are released.
#[derive(Clone, Copy)]
struct Timers {
    idle: Duration,
    silence: Duration,
}

/// Sends what `link` has to send and sleeps until its next timer or a
/// change, until either side ends it or it has been silent for `idle`.
async fn drive(
    link: Rc<Link>,
    socket: Rc<UdpSocket>,
    registry: Registry,
    state: Shared,
    timers: Timers,
) {
    loop {
        let now = Instant::now();
        link.send(&socket, now).await;
        link.fall_silent(now, timers.silence, &state);
        let Some(deadline) = link.deadline(now, timers) else {
            break;
        };
        rusty_tokio::select! {
            _ = rusty_tokio::time::sleep_until(rusty_tokio::time::Instant::from_std(deadline)) => {},
            _ = link.wake.notified() => {},
        }
    }
    registry.remove(&link.id);
    link.assoc.borrow_mut().end();
    link.send(&socket, Instant::now()).await;
    link.wake_streams();
}

/// A live association, shared by its driver and its streams' halves.
pub(crate) struct Link {
    id: Id,
    born: Instant,
    assoc: RefCell<Association>,
    /// Wakes the driver: something may be ready to send.
    wake: Notify,
    /// Each stream's halves waiting for bytes or room.
    wakers: RefCell<HashMap<StreamId, Wakers>>,
    /// Whether the last send failed, so a dead path is logged once.
    failing: Cell<bool>,
    /// Each client a stream of this link served, with that connection's
    /// cancel, while it still does or left the client a stale orphan; one
    /// taken over elsewhere is no longer this link's.
    clients: RefCell<Vec<(ClientId, Rc<Notify>)>>,
    /// While silent past [`LEASE_SILENCE`]: the leases released, to give
    /// back when the link is heard again.
    released: RefCell<Option<Vec<Released>>>,
}

#[derive(Default)]
struct Wakers {
    read: Option<Waker>,
    write: Option<Waker>,
}

impl Link {
    fn new(id: Id, assoc: Association, born: Instant) -> Self {
        Self {
            id,
            born,
            assoc: RefCell::new(assoc),
            wake: Notify::new(),
            wakers: RefCell::default(),
            failing: Cell::new(false),
            clients: RefCell::default(),
            released: RefCell::default(),
        }
    }

    /// A stream of this link now serves `client`; clients no longer
    /// served here are forgotten, so the list stays as long as the open
    /// streams.
    fn adopt(&self, client: ClientId, cancel: &Rc<Notify>, state: &Shared) {
        let state = state.borrow();
        let mut clients = self.clients.borrow_mut();
        clients.retain(|(client, cancel)| state.kept_by(*client, cancel));
        clients.push((client, cancel.clone()));
    }

    /// Once silent for `silence`, releases the leases this link's clients
    /// hold, once until heard again; forgets clients served elsewhere.
    fn fall_silent(&self, now: Instant, silence: Duration, state: &Shared) {
        if self.released.borrow().is_some() || now < self.heard_at() + silence {
            return;
        }
        let mut clients = self.clients.borrow_mut();
        let mut state = state.borrow_mut();
        clients.retain(|(client, cancel)| state.kept_by(*client, cancel));
        *self.released.borrow_mut() = Some(state.release_silent(&clients));
    }

    /// Heard again after falling silent: gives the leases back.
    fn heard(&self, state: &Shared) {
        let released = self.released.borrow_mut().take();
        if let Some(released) = released {
            state.borrow_mut().regain(&released);
        }
    }

    /// When the last authentic datagram arrived, or the link was born.
    fn heard_at(&self) -> Instant {
        self.assoc.borrow().last_heard().unwrap_or(self.born)
    }

    fn datagram(&self, now: Instant, from: SocketAddr, datagram: &[u8]) -> bool {
        if !self.assoc.borrow_mut().datagram(now, from, datagram) {
            return false;
        }
        self.wake_streams();
        self.wake.notify_one();
        true
    }

    /// When the driver should next look, or `None` once the association
    /// ended or has been silent for `idle`.
    fn deadline(&self, now: Instant, timers: Timers) -> Option<Instant> {
        let heard = self.heard_at();
        let mut until = heard + timers.idle;
        if self.assoc.borrow().is_ended() || now >= until {
            return None;
        }
        if self.released.borrow().is_none() {
            until = until.min(heard + timers.silence);
        }
        let at = self
            .assoc
            .borrow()
            .timeout()
            .map_or(until, |at| at.min(until));
        Some(at.max(now + MIN_SLEEP))
    }

    fn accept(&self) -> Option<StreamId> {
        self.assoc.borrow_mut().accept()
    }

    /// Sends every datagram the association has ready.
    async fn send(&self, socket: &UdpSocket, now: Instant) {
        loop {
            let Some((to, datagram)) = self.assoc.borrow_mut().transmit(now) else {
                return;
            };
            match socket.send_to(&datagram, to).await {
                Ok(_) => self.failing.set(false),
                Err(err) if !self.failing.replace(true) => {
                    eprintln!("rusty-mux-server: sending to {to}: {err}");
                }
                Err(_) => {}
            }
        }
    }

    /// Wakes every waiting stream half; each checks its own stream again.
    fn wake_streams(&self) {
        let mut assoc = self.assoc.borrow_mut();
        // Closed streams are seen through `is_open`; the queue only drains.
        while assoc.closed().is_some() {}
        drop(assoc);
        let wakers: Vec<Waker> = self
            .wakers
            .borrow_mut()
            .values_mut()
            .flat_map(|w| [w.read.take(), w.write.take()])
            .flatten()
            .collect();
        wakers.into_iter().for_each(Waker::wake);
    }

    /// Whether `id` can still carry bytes.
    fn open(&self, id: StreamId) -> bool {
        let assoc = self.assoc.borrow();
        assoc.is_open(id) && !assoc.is_ended()
    }
}

/// A stream's incoming bytes, as a connection's read half. It ends when
/// the peer closes the stream or the association ends.
pub(crate) struct StreamReader {
    link: Rc<Link>,
    id: StreamId,
}

impl AsyncRead for StreamReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let link = &self.link;
        let n = link
            .assoc
            .borrow_mut()
            .read(self.id, buf.initialize_unfilled());
        if n > 0 {
            buf.advance(n);
            // Reading may reopen the peer's window: an ack to send.
            link.wake.notify_one();
            return Poll::Ready(Ok(()));
        }
        if !link.open(self.id) {
            return Poll::Ready(Ok(()));
        }
        let mut wakers = link.wakers.borrow_mut();
        wakers.entry(self.id).or_default().read = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// A stream's outgoing bytes, as a connection's write half, holding at
/// most the transport's window unacknowledged. Shutting it down closes the
/// stream once all is acknowledged; dropping it closes the stream at once.
pub(crate) struct StreamWriter {
    link: Rc<Link>,
    id: StreamId,
}

impl AsyncWrite for StreamWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let link = &self.link;
        if !link.open(self.id) {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        if bytes.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let n = link.assoc.borrow_mut().write(self.id, bytes);
        if n > 0 {
            link.wake.notify_one();
            return Poll::Ready(Ok(n));
        }
        let mut wakers = link.wakers.borrow_mut();
        wakers.entry(self.id).or_default().write = Some(cx.waker().clone());
        Poll::Pending
    }

    /// Closes the stream once the peer has acknowledged every byte, so
    /// the last frames written are not lost.
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let link = &self.link;
        if link.open(self.id) && link.assoc.borrow().queued(self.id) > 0 {
            let mut wakers = link.wakers.borrow_mut();
            wakers.entry(self.id).or_default().write = Some(cx.waker().clone());
            return Poll::Pending;
        }
        self.close();
        Poll::Ready(Ok(()))
    }
}

impl StreamWriter {
    fn close(&self) {
        self.link.assoc.borrow_mut().close(Instant::now(), self.id);
        self.link.wake.notify_one();
    }
}

impl Drop for StreamWriter {
    fn drop(&mut self) {
        self.close();
        self.link.wakers.borrow_mut().remove(&self.id);
    }
}

#[cfg(test)]
#[path = "datagram/tests.rs"]
mod tests;
