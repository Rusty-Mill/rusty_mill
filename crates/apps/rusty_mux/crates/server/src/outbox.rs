//! A client's queue of encoded frames, with a byte budget.

use std::cell::Cell;
use std::rc::Rc;

use rusty_tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Frames queued but not yet written beyond this many bytes mean the client
/// cannot keep up; the server drops it rather than stall the PTY or grow
/// without bound. A client that reattaches recovers from the snapshot.
pub(crate) const BUDGET: usize = 8 << 20;

/// The sending side, held by the server state.
pub(crate) struct Outbox {
    tx: UnboundedSender<Vec<u8>>,
    queued: Rc<Cell<usize>>,
}

/// The receiving side, held by the connection's writer task.
pub(crate) struct Inbox {
    rx: UnboundedReceiver<Vec<u8>>,
    queued: Rc<Cell<usize>>,
}

pub(crate) fn outbox() -> (Outbox, Inbox) {
    let (tx, rx) = unbounded_channel();
    let queued = Rc::new(Cell::new(0));
    (
        Outbox {
            tx,
            queued: queued.clone(),
        },
        Inbox { rx, queued },
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
        self.tx.send(frame).is_ok()
    }
}

impl Inbox {
    /// The next frame to write, or `None` once the server dropped the client.
    pub(crate) async fn next(&mut self) -> Option<Vec<u8>> {
        let frame = self.rx.recv().await?;
        self.queued
            .set(self.queued.get().saturating_sub(frame.len()));
        Some(frame)
    }
}
