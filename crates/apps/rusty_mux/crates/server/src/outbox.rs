//! A client's queue of encoded frames, with a byte budget.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

use rusty_mux_core::BlockId;
use rusty_mux_proto::{Message, encode};
use rusty_tokio::sync::Notify;
use rusty_tokio::sync::mpsc::{
    TryRecvError, UnboundedReceiver, UnboundedSender, unbounded_channel,
};

/// Frames queued but not yet written beyond this many bytes mean the client
/// cannot keep up; the server drops it rather than stall the PTY or grow
/// without bound. A client that reattaches recovers from the snapshot.
pub(crate) const BUDGET: usize = 8 << 20;

/// The sending side, held by the server state.
pub(crate) struct Outbox {
    tx: UnboundedSender<Item>,
    attaching: Rc<Cell<bool>>,
    cancel: Rc<Notify>,
    queued: Rc<Cell<usize>>,
    /// Whether the connection is a datagram stream, which outlives its
    /// budget as a stale orphan instead of being dropped.
    datagram: bool,
}

/// The receiving side, held by the connection's writer task.
pub(crate) struct Inbox {
    rx: UnboundedReceiver<Item>,
    attaching: Rc<Cell<bool>>,
    transfer: Option<Transfer>,
    /// History batches still to send after Ready (C03), between live frames.
    history: Option<History>,
    queued: Rc<Cell<usize>>,
}

pub(crate) fn outbox() -> (Outbox, Inbox, Rc<Notify>) {
    let cancel = Rc::new(Notify::new());
    let attaching = Rc::new(Cell::new(false));
    let (tx, rx) = unbounded_channel();
    let queued = Rc::new(Cell::new(0));
    (
        Outbox {
            tx,
            attaching: attaching.clone(),
            cancel: cancel.clone(),
            queued: queued.clone(),
            datagram: false,
        },
        Inbox {
            rx,
            queued,
            attaching,
            transfer: None,
            history: None,
        },
        cancel,
    )
}

impl Outbox {
    /// Queues a frame. Returns false when the client is over budget or gone;
    /// the caller then disconnects it.
    /// Bytes queued and not yet written.
    pub(crate) fn queued(&self) -> usize {
        self.queued.get()
    }

    pub(crate) fn push(&self, frame: Vec<u8>) -> bool {
        let queued = self.queued.get() + frame.len();
        if queued > BUDGET {
            return false;
        }
        self.queued.set(queued);
        self.tx.send(Item::Frame(frame)).is_ok()
    }
}

/// Checkpoint storage is separate from the 8 MiB live-tail budget. At most
/// one current-state image (MAX_ENCODED bytes) exists per connection.
enum Item {
    Frame(Vec<u8>),
    Checkpoint(Transfer),
    /// The client detached: history not yet sent is dropped.
    EndHistory,
}
struct Transfer {
    block: BlockId,
    data: Vec<u8>,
    offset: usize,
    /// History batches to follow Ready, newest first; empty without C03.
    history: VecDeque<Vec<u8>>,
}
/// The history batches of one attach still to send, the first one from
/// `offset` on.
struct History {
    block: BlockId,
    batches: VecDeque<Vec<u8>>,
    offset: usize,
}
impl History {
    /// The next chunk of the current batch, or `None` once all are sent.
    fn next(&mut self) -> Option<Message> {
        let batch = self.batches.front()?;
        let end = (self.offset + crate::checkpoint::CHUNK_BYTES).min(batch.len());
        let message = Message::History {
            block: self.block,
            offset: self.offset as u64,
            total: batch.len() as u64,
            data: batch[self.offset..end].to_vec(),
        };
        self.offset = end;
        if end == batch.len() {
            self.batches.pop_front();
            self.offset = 0;
        }
        Some(message)
    }
}

impl Outbox {
    /// Whether this is the outbox of the connection that owns `cancel`.
    pub(crate) fn belongs_to(&self, cancel: &Rc<Notify>) -> bool {
        Rc::ptr_eq(&self.cancel, cancel)
    }

    /// The cancel of the connection this outbox feeds.
    pub(crate) fn cancel(&self) -> &Rc<Notify> {
        &self.cancel
    }

    pub(crate) fn over_datagram(&mut self) {
        self.datagram = true;
    }

    pub(crate) fn is_datagram(&self) -> bool {
        self.datagram
    }

    pub(crate) fn attaching(&self) -> bool {
        self.attaching.get()
    }
    /// Queues an attach's checkpoint, then Ready, then `history` (newest
    /// batch first) between live frames. History is outside the live
    /// budget, like the checkpoint; together they stay within one
    /// checkpoint's size.
    pub(crate) fn checkpoint(&self, block: BlockId, data: Vec<u8>, history: Vec<Vec<u8>>) -> bool {
        let size = history
            .iter()
            .fold(data.len(), |n, b| n.saturating_add(b.len()));
        if size > rusty_term_core::checkpoint::MAX_ENCODED || self.attaching.replace(true) {
            return false;
        }
        self.tx
            .send(Item::Checkpoint(Transfer {
                block,
                data,
                offset: 0,
                history: history.into(),
            }))
            .is_ok()
    }

    /// Drops history not yet sent: the client detached.
    pub(crate) fn end_history(&self) -> bool {
        self.tx.send(Item::EndHistory).is_ok()
    }
}
impl Inbox {
    /// Expands a checkpoint lazily, writing it and Ready before any queued
    /// live output/events. No checkpoint-sized collection of encoded frames.
    pub(crate) async fn next(&mut self) -> Option<Vec<u8>> {
        loop {
            if let Some(transfer) = &mut self.transfer {
                let message = if transfer.offset < transfer.data.len() {
                    let end =
                        (transfer.offset + crate::checkpoint::CHUNK_BYTES).min(transfer.data.len());
                    let message = Message::Checkpoint {
                        block: transfer.block,
                        offset: transfer.offset as u64,
                        total: transfer.data.len() as u64,
                        data: transfer.data[transfer.offset..end].to_vec(),
                    };
                    transfer.offset = end;
                    message
                } else {
                    let message = Message::Ready {
                        block: transfer.block,
                    };
                    self.history = Some(History {
                        block: transfer.block,
                        batches: std::mem::take(&mut transfer.history),
                        offset: 0,
                    });
                    self.transfer = None;
                    self.attaching.set(false);
                    message
                };
                return match encode(&message) {
                    Ok(frame) => Some(frame),
                    Err(err) => {
                        eprintln!("rusty-mux-server: encoding checkpoint transfer: {err}");
                        None
                    }
                };
            }
            let item = match self.history.as_mut() {
                // Live output goes first; history fills the gaps.
                Some(history) => match self.rx.try_recv() {
                    Ok(item) => item,
                    Err(TryRecvError::Empty) => match history.next() {
                        Some(message) => return encode_history(&message),
                        None => {
                            self.history = None;
                            continue;
                        }
                    },
                    Err(TryRecvError::Disconnected) => return None,
                },
                None => self.rx.recv().await?,
            };
            match item {
                Item::Frame(frame) => {
                    self.queued
                        .set(self.queued.get().saturating_sub(frame.len()));
                    return Some(frame);
                }
                // A new attach ends the old one's history.
                Item::Checkpoint(transfer) => {
                    self.history = None;
                    self.transfer = Some(transfer);
                }
                Item::EndHistory => self.history = None,
            }
        }
    }
}

fn encode_history(message: &Message) -> Option<Vec<u8>> {
    match encode(message) {
        Ok(frame) => Some(frame),
        Err(err) => {
            eprintln!("rusty-mux-server: encoding history: {err}");
            None
        }
    }
}

impl Drop for Outbox {
    fn drop(&mut self) {
        self.cancel.notify_one();
    }
}

#[cfg(all(test, unix))]
impl Outbox {
    pub(crate) fn test_queue(&self) -> Rc<Cell<usize>> {
        self.queued.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_mux_proto::decode;
    use rusty_tokio::runtime::{Builder, LocalOptions};
    use rusty_uuid::Uuid;

    /// The next message the writer would send, decoded.
    async fn next(inbox: &mut Inbox) -> Message {
        let frame = inbox.next().await.expect("a frame");
        decode(&frame).expect("decodes").expect("whole").0
    }

    fn output(n: u8) -> Vec<u8> {
        let block = BlockId::from_uuid(Uuid::nil());
        encode(&Message::Output {
            block,
            data: vec![n],
        })
        .expect("encodes")
    }

    #[test]
    fn history_follows_ready_and_gives_way_to_live_output() {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .unwrap();
        runtime.block_on(async {
            let (out, mut inbox, _) = outbox();
            let block = BlockId::from_uuid(Uuid::nil());
            let big = vec![2; crate::checkpoint::CHUNK_BYTES + 1];
            assert!(out.checkpoint(block, vec![9], vec![big, vec![1]]));
            assert!(matches!(next(&mut inbox).await, Message::Checkpoint { .. }));
            assert!(out.push(output(7)));
            assert!(matches!(next(&mut inbox).await, Message::Ready { .. }));
            // Live output queued during the transfer goes before history.
            assert!(matches!(next(&mut inbox).await, Message::Output { data, .. } if data == [7]));
            let Message::History {
                offset: 0, total, ..
            } = next(&mut inbox).await
            else {
                panic!("history");
            };
            assert_eq!(total as usize, crate::checkpoint::CHUNK_BYTES + 1);
            assert!(out.push(output(8)));
            assert!(matches!(next(&mut inbox).await, Message::Output { data, .. } if data == [8]));
            assert!(matches!(
                next(&mut inbox).await,
                Message::History { offset, data, .. }
                    if offset as usize == crate::checkpoint::CHUNK_BYTES && data == [2]
            ));
            assert!(matches!(
                next(&mut inbox).await,
                Message::History {
                    offset: 0,
                    total: 1,
                    ..
                }
            ));
            assert!(out.push(output(9)));
            assert!(matches!(next(&mut inbox).await, Message::Output { data, .. } if data == [9]));
        });
    }

    #[test]
    fn a_detach_or_a_new_attach_ends_unsent_history() {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .unwrap();
        runtime.block_on(async {
            let (out, mut inbox, _) = outbox();
            let block = BlockId::from_uuid(Uuid::nil());
            assert!(out.checkpoint(block, vec![9], vec![vec![1], vec![2]]));
            next(&mut inbox).await;
            assert!(matches!(next(&mut inbox).await, Message::Ready { .. }));
            assert!(out.end_history());
            assert!(out.push(output(7)));
            assert!(matches!(next(&mut inbox).await, Message::Output { .. }));
            assert!(out.checkpoint(block, vec![9], vec![vec![3]]));
            next(&mut inbox).await;
            assert!(matches!(next(&mut inbox).await, Message::Ready { .. }));
            // A second attach before the first one's history is out.
            assert!(out.checkpoint(block, vec![9], vec![]));
            assert!(matches!(next(&mut inbox).await, Message::Checkpoint { .. }));
            assert!(matches!(next(&mut inbox).await, Message::Ready { .. }));
            assert!(out.push(output(8)));
            assert!(matches!(next(&mut inbox).await, Message::Output { .. }));
        });
    }
}
