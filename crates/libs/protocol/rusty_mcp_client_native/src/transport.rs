//! The seam between the client and the wire: send a [`Message`], wait for the
//! next one. The child-process transport is [`crate::stdio`]; the Streamable
//! HTTP one is the next slice.

use rusty_mcp_proto::Message;
use std::io;
use std::time::Duration;

/// What [`Transport::recv`] produced.
#[derive(Debug)]
pub enum Recv {
    /// The next message.
    Message(Message),
    /// Nothing arrived within the wait.
    Timeout,
    /// The peer is gone.
    Closed,
}

/// A bidirectional stream of JSON-RPC messages.
pub trait Transport {
    /// Send one message.
    ///
    /// # Errors
    /// The underlying write failed.
    fn send(&mut self, message: &Message) -> io::Result<()>;

    /// Wait up to `timeout` for the next message.
    ///
    /// # Errors
    /// The underlying read failed.
    fn recv(&mut self, timeout: Duration) -> io::Result<Recv>;
}
