//! The ends of a streaming call.

use std::marker::PhantomData;

use rusty_prost::Message;

use super::Response;
use crate::{MetadataMap, Status};

/// The replies of a call, read one message at a time. Dropping it before the end cancels the call.
pub struct Streaming<T> {
    _t: PhantomData<fn() -> T>,
}

impl<T: Message> Streaming<T> {
    /// The initial metadata.
    pub fn metadata(&self) -> &MetadataMap {
        todo!()
    }

    /// The next message; `Ok(None)` once the call ended with status OK; `Err` with the call's
    /// status if it ended any other way, and again on every later call.
    pub async fn message(&mut self) -> Result<Option<T>, Status> {
        todo!()
    }

    /// The trailing metadata, once the call has ended.
    pub fn trailers(&self) -> Option<&MetadataMap> {
        todo!()
    }
}

/// The sending end of a call that streams requests.
pub struct Sender<T> {
    _t: PhantomData<fn(T)>,
}

impl<T: Message> Sender<T> {
    /// Sends one message, waiting for flow-control capacity.
    pub async fn send(&mut self, message: &T) -> Result<(), Status> {
        let _ = message;
        todo!()
    }

    /// Ends the requests. Without it (the sender dropped) the call is cancelled.
    pub async fn finish(self) -> Result<(), Status> {
        todo!()
    }
}

/// The single reply of a call that streams requests.
pub struct Reply<T> {
    _t: PhantomData<fn() -> T>,
}

impl<T: Message> Reply<T> {
    /// Waits for the reply, its metadata and its trailers.
    pub async fn response(self) -> Result<Response<T>, Status> {
        todo!()
    }
}
