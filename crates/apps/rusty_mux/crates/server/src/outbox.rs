//! A client's queue of encoded frames, with a byte budget.

use std::cell::Cell;
use std::rc::Rc;

use rusty_mux_core::BlockId;
use rusty_mux_proto::{Message, encode};
use rusty_tokio::sync::Notify;
use rusty_tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

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
        },
        cancel,
    )
}

impl Outbox {
    /// Queues a frame. Returns false when the client is over budget or gone;
    /// the caller then disconnects it.
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
}
struct Transfer {
    block: BlockId,
    data: Vec<u8>,
    offset: usize,
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
    pub(crate) fn checkpoint(&self, block: BlockId, data: Vec<u8>) -> bool {
        if data.len() > rusty_term_core::checkpoint::MAX_ENCODED || self.attaching.replace(true) {
            return false;
        }
        self.tx
            .send(Item::Checkpoint(Transfer {
                block,
                data,
                offset: 0,
            }))
            .is_ok()
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
            match self.rx.recv().await? {
                Item::Frame(frame) => {
                    self.queued
                        .set(self.queued.get().saturating_sub(frame.len()));
                    return Some(frame);
                }
                Item::Checkpoint(transfer) => self.transfer = Some(transfer),
            }
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
