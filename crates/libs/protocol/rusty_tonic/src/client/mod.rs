//! A gRPC client over `rusty_hyper` (feature `rusty-tokio`): a lazily connected [`Channel`] with
//! unary, server-streaming, client-streaming and bidirectional calls, deadlines, an interceptor,
//! and the mapping of transport errors to gRPC statuses.
//!
//! One HTTP/2 connection carries every call. It is opened by the first call, and replaced once if
//! a call finds it closed before its stream was opened. A call that was already in flight when
//! the connection died fails with `UNAVAILABLE`; it is never sent again.

mod call;
mod channel;
mod error;
mod streaming;

pub use call::{Request, Response};
pub use channel::{Channel, ChannelOptions, Interceptor};
pub use streaming::{Reply, Sender, Streaming};
