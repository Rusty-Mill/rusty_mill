//! Port forwards (C81): a tunnel from a client over its mux connection to a
//! loopback port of the server's host. [`Forwards`] is the bookkeeping the
//! server state keeps, [`run`] the task that moves the bytes.
//!
//! Each direction has a credit of [`FORWARD_WINDOW`] bytes: a sender stops at
//! zero until the other end acknowledges what it has consumed, so a slow
//! service or a slow client never stalls the connection's terminal traffic
//! or grows the outbox without bound. The server dials `127.0.0.1` only; the
//! message has no host to forge.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::rc::{Rc, Weak};
use std::time::Duration;

use rusty_mux_core::{BlockId, ClientId};
use rusty_mux_proto::{FORWARD_CHUNK, FORWARD_WINDOW};
use rusty_tokio::io::TcpStream;
use rusty_tokio::sync::Notify;
use rusty_tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::state::State;

/// Forwards one connection may have open.
const PER_CLIENT: usize = 16;
/// Forwards the server may have open.
const PER_SERVER: usize = 64;
/// Chunks a forward may have queued for the target, whatever their size, so
/// the work bound holds as well as the byte bound.
const MAX_QUEUED: usize = 64;
/// How long the dial of the target may take.
const CONNECT_WAIT: Duration = Duration::from_secs(5);

/// Names one instance of a forward: a client may reuse an id once its forward
/// closed, and a task or callback of the old instance must not touch the new
/// one, so every callback carries the instance's `epoch`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Tag {
    pub(crate) client: ClientId,
    pub(crate) id: u64,
    epoch: u64,
}

/// One open forward, as the state sees it.
struct Open {
    epoch: u64,
    block: BlockId,
    /// Bytes from the client to the target's writer.
    to_target: UnboundedSender<Vec<u8>>,
    /// Client bytes received and not yet written to the target.
    held: usize,
    /// Chunks of those.
    queued: usize,
    /// Bytes the server may still send to the client.
    credit: usize,
    /// Credit came back.
    wake: Rc<Notify>,
    /// The forward is over: its task stops.
    stop: Rc<Notify>,
}

/// The open forwards of every client.
#[derive(Default)]
pub(crate) struct Forwards {
    open: HashMap<(ClientId, u64), Open>,
    next_epoch: u64,
}

/// What [`Forwards::open`] hands the task that serves the forward.
pub(crate) struct Ends {
    tag: Tag,
    from_client: UnboundedReceiver<Vec<u8>>,
    wake: Rc<Notify>,
    stop: Rc<Notify>,
}

impl Forwards {
    /// Registers forward `id` of `client` on `block`, within the limits.
    pub(crate) fn open(
        &mut self,
        client: ClientId,
        id: u64,
        block: BlockId,
    ) -> Result<Ends, String> {
        if self.open.contains_key(&(client, id)) {
            return Err(format!("forward {id} is already open"));
        }
        if self.open.len() >= PER_SERVER {
            return Err(format!("the server has {PER_SERVER} forwards open"));
        }
        if self.open.keys().filter(|(c, _)| *c == client).count() >= PER_CLIENT {
            return Err(format!("at most {PER_CLIENT} forwards a connection"));
        }
        let (to_target, from_client) = unbounded_channel();
        let (wake, stop) = (Rc::new(Notify::new()), Rc::new(Notify::new()));
        self.next_epoch += 1;
        let tag = Tag {
            client,
            id,
            epoch: self.next_epoch,
        };
        let ends = Ends {
            tag,
            from_client,
            wake: wake.clone(),
            stop: stop.clone(),
        };
        let entry = Open {
            epoch: tag.epoch,
            block,
            to_target,
            held: 0,
            queued: 0,
            credit: FORWARD_WINDOW,
            wake,
            stop,
        };
        self.open.insert((client, id), entry);
        Ok(ends)
    }

    /// Queues the client's `data` for the target. An error is the reason to
    /// close the forward: the client broke the window. Data for a forward
    /// that is not open is dropped: the close crossed it.
    pub(crate) fn data(&mut self, client: ClientId, id: u64, data: Vec<u8>) -> Result<(), String> {
        let Some(open) = self.open.get_mut(&(client, id)) else {
            return Ok(());
        };
        if data.is_empty() || data.len() > FORWARD_CHUNK {
            return Err(format!("a chunk is 1 to {FORWARD_CHUNK} bytes"));
        }
        if open.queued >= MAX_QUEUED {
            return Err(format!("more than {MAX_QUEUED} chunks queued"));
        }
        if open.held + data.len() > FORWARD_WINDOW {
            return Err("more than the window sent".into());
        }
        open.held += data.len();
        open.queued += 1;
        // The task ended: the close that follows tells the client.
        let _ = open.to_target.send(data);
        Ok(())
    }

    /// The instance `tag` names, if it is still the open one.
    fn of(&mut self, tag: Tag) -> Option<&mut Open> {
        self.open
            .get_mut(&(tag.client, tag.id))
            .filter(|open| open.epoch == tag.epoch)
    }

    /// The target got one chunk of `n` bytes of the client's data, which
    /// the client may send again. Whether the forward is still this one.
    pub(crate) fn wrote(&mut self, tag: Tag, n: usize) -> bool {
        let Some(open) = self.of(tag) else {
            return false;
        };
        open.held = open.held.saturating_sub(n);
        open.queued = open.queued.saturating_sub(1);
        true
    }

    /// The client consumed `n` bytes of the server's data. An error is the
    /// reason to close the forward: it acknowledged what it was not sent.
    pub(crate) fn ack(&mut self, client: ClientId, id: u64, n: u32) -> Result<(), String> {
        let Some(open) = self.open.get_mut(&(client, id)) else {
            return Ok(());
        };
        let credit = open.credit + n as usize;
        if credit > FORWARD_WINDOW {
            return Err("acknowledged more than was sent".into());
        }
        open.credit = credit;
        open.wake.notify_one();
        Ok(())
    }

    /// The bytes the server may send on the forward now.
    pub(crate) fn credit(&mut self, tag: Tag) -> usize {
        self.of(tag).map_or(0, |o| o.credit)
    }

    /// Spends credit on `n` bytes sent to the client. Whether the forward
    /// is still this one.
    pub(crate) fn sent(&mut self, tag: Tag, n: usize) -> bool {
        let Some(open) = self.of(tag) else {
            return false;
        };
        open.credit = open.credit.saturating_sub(n);
        true
    }

    /// Ends the instance `tag` names, if it is still the open one.
    pub(crate) fn close_tag(&mut self, tag: Tag) -> bool {
        self.of(tag).is_some() && self.close(tag.client, tag.id)
    }

    /// Forgets the forward and stops its task. Whether it was open.
    pub(crate) fn close(&mut self, client: ClientId, id: u64) -> bool {
        let Some(open) = self.open.remove(&(client, id)) else {
            return false;
        };
        open.stop.notify_one();
        true
    }

    /// Closes every forward of `client`.
    pub(crate) fn close_client(&mut self, client: ClientId) {
        self.retain(|c, _| c != client);
    }

    /// Closes each forward `keep` rejects, returning them.
    pub(crate) fn retain(
        &mut self,
        mut keep: impl FnMut(ClientId, BlockId) -> bool,
    ) -> Vec<(ClientId, u64)> {
        let gone: Vec<_> = self
            .open
            .iter()
            .filter(|((client, _), open)| !keep(*client, open.block))
            .map(|(key, _)| *key)
            .collect();
        for &(client, id) in &gone {
            self.close(client, id);
        }
        gone
    }
}

/// Serves a forward: dials `127.0.0.1:port`, then moves bytes both ways
/// until either end closes. The state is told when the end came from here;
/// a forward closed from the state ends silently, and cancellation is
/// checked before anything else.
pub(crate) async fn run(state: Weak<RefCell<State>>, port: u16, ends: Ends) {
    let Ends {
        tag,
        from_client,
        wake,
        stop,
    } = ends;
    let reason = rusty_tokio::select! {
        biased;
        _ = stop.notified() => return,
        result = pump(&state, tag, port, from_client, &wake) => result.err(),
    };
    if let Some(state) = state.upgrade() {
        state.borrow_mut().forward_ended(tag, reason);
    }
}

async fn pump(
    state: &Weak<RefCell<State>>,
    tag: Tag,
    port: u16,
    from_client: UnboundedReceiver<Vec<u8>>,
    wake: &Notify,
) -> Result<(), String> {
    let target = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let stream =
        match rusty_tokio::time::timeout(CONNECT_WAIT, TcpStream::connect_addr(target)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(err)) => return Err(err.to_string()),
            Err(_) => return Err("timed out".into()),
        };
    rusty_tokio::select! {
        result = upstream(state, tag, &stream, from_client) => result,
        result = downstream(state, tag, &stream, wake) => result,
    }
}

/// Client to target: writes what arrives, then gives the client credit.
async fn upstream(
    state: &Weak<RefCell<State>>,
    tag: Tag,
    stream: &TcpStream,
    mut from_client: UnboundedReceiver<Vec<u8>>,
) -> Result<(), String> {
    while let Some(data) = from_client.recv().await {
        stream.write_all(&data).await.map_err(|e| e.to_string())?;
        live(state)?.borrow_mut().forward_wrote(tag, data.len());
    }
    Ok(())
}

/// Target to client: reads only as much as the client has credit for.
async fn downstream(
    state: &Weak<RefCell<State>>,
    tag: Tag,
    stream: &TcpStream,
    wake: &Notify,
) -> Result<(), String> {
    let mut buf = vec![0; FORWARD_CHUNK];
    loop {
        let credit = live(state)?.borrow_mut().forward_credit(tag);
        if credit == 0 {
            wake.notified().await;
            continue;
        }
        let n = stream
            .read(&mut buf[..credit.min(FORWARD_CHUNK)])
            .await
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(());
        }
        live(state)?
            .borrow_mut()
            .forward_sent(tag, buf[..n].to_vec());
    }
}

fn live(state: &Weak<RefCell<State>>) -> Result<Rc<RefCell<State>>, String> {
    state
        .upgrade()
        .ok_or_else(|| "the server is stopping".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_uuid::Uuid;

    fn client() -> ClientId {
        ClientId::from_uuid(Uuid::new_v4())
    }

    fn block() -> BlockId {
        BlockId::from_uuid(Uuid::new_v4())
    }

    #[test]
    fn a_client_may_not_send_past_the_window() {
        let (me, b) = (client(), block());
        let mut forwards = Forwards::default();
        let ends = forwards.open(me, 1, b).unwrap();
        let chunk = vec![0; FORWARD_CHUNK];
        for _ in 0..FORWARD_WINDOW / FORWARD_CHUNK {
            forwards.data(me, 1, chunk.clone()).unwrap();
        }
        assert!(forwards.data(me, 1, vec![0]).is_err());
        // Written bytes make room again.
        assert!(forwards.wrote(ends.tag, FORWARD_CHUNK));
        forwards.data(me, 1, chunk).unwrap();
    }

    #[test]
    fn a_chunk_is_one_to_the_chunk_size() {
        let (me, b) = (client(), block());
        let mut forwards = Forwards::default();
        let _ends = forwards.open(me, 1, b).unwrap();
        assert!(forwards.data(me, 1, vec![0; FORWARD_CHUNK + 1]).is_err());
        assert!(
            forwards.data(me, 1, Vec::new()).is_err(),
            "empty chunks cost nothing"
        );
    }

    #[test]
    fn queued_chunks_are_bounded_whatever_their_size() {
        let (me, b) = (client(), block());
        let mut forwards = Forwards::default();
        let ends = forwards.open(me, 1, b).unwrap();
        for _ in 0..MAX_QUEUED {
            forwards.data(me, 1, vec![0]).unwrap();
        }
        assert!(forwards.data(me, 1, vec![0]).is_err());
        assert!(forwards.wrote(ends.tag, 1));
        forwards.data(me, 1, vec![0]).unwrap();
    }

    #[test]
    fn credit_is_spent_by_sending_and_returned_by_acks_up_to_the_window() {
        let (me, b) = (client(), block());
        let mut forwards = Forwards::default();
        let ends = forwards.open(me, 1, b).unwrap();
        assert_eq!(forwards.credit(ends.tag), FORWARD_WINDOW);
        assert!(forwards.sent(ends.tag, 1000));
        assert_eq!(forwards.credit(ends.tag), FORWARD_WINDOW - 1000);
        forwards.ack(me, 1, 1000).unwrap();
        assert_eq!(forwards.credit(ends.tag), FORWARD_WINDOW);
        assert!(forwards.ack(me, 1, 1).is_err(), "acknowledged unsent bytes");
    }

    #[test]
    fn frames_for_a_forward_that_closed_are_dropped() {
        let me = client();
        let mut forwards = Forwards::default();
        assert!(forwards.data(me, 9, vec![1]).is_ok());
        assert!(forwards.ack(me, 9, 5).is_ok());
        assert!(!forwards.close(me, 9));
    }

    #[test]
    fn a_stale_instance_cannot_touch_the_one_that_reused_its_id() {
        let (me, b) = (client(), block());
        let mut forwards = Forwards::default();
        let old = forwards.open(me, 1, b).unwrap();
        assert!(forwards.close(me, 1));
        let new = forwards.open(me, 1, b).unwrap();
        assert_ne!(old.tag, new.tag);
        forwards.data(me, 1, vec![1, 2]).unwrap();
        // Every callback of the old task finds it gone.
        assert!(!forwards.wrote(old.tag, 2));
        assert!(!forwards.sent(old.tag, 10));
        assert_eq!(forwards.credit(old.tag), 0);
        assert!(!forwards.close_tag(old.tag));
        // And the new one is as it was.
        assert_eq!(forwards.credit(new.tag), FORWARD_WINDOW);
        assert!(forwards.wrote(new.tag, 2));
        assert!(forwards.close_tag(new.tag));
    }

    #[test]
    fn an_id_opens_once_and_the_limits_hold() {
        let (me, b) = (client(), block());
        let mut forwards = Forwards::default();
        let _first = forwards.open(me, 0, b).unwrap();
        assert!(forwards.open(me, 0, b).is_err(), "the id is taken");
        let kept: Vec<_> = (1..PER_CLIENT as u64)
            .map(|id| forwards.open(me, id, b).unwrap())
            .collect();
        assert!(forwards.open(me, 99, b).is_err(), "a connection's limit");
        // Other connections fill the server.
        let mut more = Vec::new();
        while forwards.open.len() < PER_SERVER {
            let other = client();
            more.extend((0..PER_CLIENT as u64).map_while(|id| {
                (forwards.open.len() < PER_SERVER).then(|| forwards.open(other, id, b).unwrap())
            }));
        }
        assert!(forwards.open(client(), 0, b).is_err(), "the server's limit");
        drop((kept, more));
    }

    #[test]
    fn retain_closes_what_it_rejects_and_a_client_closes_whole() {
        let (me, you, b, other) = (client(), client(), block(), block());
        let mut forwards = Forwards::default();
        let mine = forwards.open(me, 1, b).unwrap();
        let _b = forwards.open(me, 2, other).unwrap();
        let yours = forwards.open(you, 1, b).unwrap();
        let gone = forwards.retain(|_, block| block == b);
        assert_eq!(gone, vec![(me, 2)]);
        forwards.close_client(me);
        assert_eq!(forwards.credit(mine.tag), 0);
        assert_eq!(forwards.credit(yours.tag), FORWARD_WINDOW);
    }
}
