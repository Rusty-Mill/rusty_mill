//! A gRPC server over `rusty_hyper` (feature `rusty-tokio`): a [`Router`] of closures, one per
//! method, served on a listener. There is no trait to implement and no generated code.
//!
//! A call's deadline (`grpc-timeout`) is enforced by dropping the handler and sending
//! `DEADLINE_EXCEEDED`. A client that cancels shows up as `CANCELLED` from the next
//! [`Incoming::message`] or [`Sink::send`]; a handler that is only computing is not interrupted
//! until then.

mod call;
mod router;

pub use call::{Incoming, Sink};
pub use router::Router;
