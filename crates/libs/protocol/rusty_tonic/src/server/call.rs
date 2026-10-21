//! The two ends a streaming handler works with.

use std::marker::PhantomData;

use rusty_prost::Message;

use crate::{MetadataMap, Status};

/// The request messages of a call, read one at a time.
pub struct Incoming<T> {
    _t: PhantomData<fn() -> T>,
}

impl<T: Message> Incoming<T> {
    /// The next message; `Ok(None)` once the client has finished sending. `CANCELLED` if the
    /// client cancelled; `RESOURCE_EXHAUSTED` for a message over the receive limit.
    pub async fn message(&mut self) -> Result<Option<T>, Status> {
        todo!()
    }
}

/// Where a streaming handler writes its replies.
pub struct Sink<T> {
    _t: PhantomData<fn(T)>,
}

impl<T: Message> Sink<T> {
    /// The initial metadata, sent with the first message (or with the status if there is none).
    /// Ignored once something has been sent.
    pub fn set_metadata(&mut self, metadata: MetadataMap) {
        let _ = metadata;
        todo!()
    }

    /// The trailing metadata, sent with the final status.
    pub fn set_trailers(&mut self, trailers: MetadataMap) {
        let _ = trailers;
        todo!()
    }

    /// Sends one reply, waiting for flow-control capacity. `CANCELLED` once the client is gone;
    /// `RESOURCE_EXHAUSTED` for a message over the send limit (nothing is sent).
    pub async fn send(&mut self, message: &T) -> Result<(), Status> {
        let _ = message;
        todo!()
    }
}
