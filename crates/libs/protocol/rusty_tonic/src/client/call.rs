//! The request and response wrappers.

use std::time::Duration;

use crate::MetadataMap;

/// A message, the metadata to send with the call, and an optional deadline for it.
#[derive(Debug, Clone)]
pub struct Request<T> {
    /// The message. For a call that streams requests, `()` and the messages go through a
    /// [`Sender`](crate::client::Sender).
    pub message: T,
    /// Custom metadata to send.
    pub metadata: MetadataMap,
    /// This call's deadline; the shorter of it and the channel's applies.
    pub timeout: Option<Duration>,
}

impl<T> Request<T> {
    /// A request carrying `message`: no metadata, no deadline of its own.
    pub fn new(message: T) -> Request<T> {
        Request {
            message,
            metadata: MetadataMap::new(),
            timeout: None,
        }
    }

    /// Sets this call's deadline.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Request<T> {
        self.timeout = Some(timeout);
        self
    }
}

/// A reply: the message, the initial metadata, and the trailing metadata.
#[derive(Debug, Clone)]
pub struct Response<T> {
    /// The message.
    pub message: T,
    /// The initial metadata.
    pub metadata: MetadataMap,
    /// The trailing metadata.
    pub trailers: MetadataMap,
}
