//! A gRPC server over `rusty_hyper` (feature `rusty-tokio`): a [`Router`] of closures, one per
//! method, served on a listener. There is no trait to implement and no generated code.
//!
//! A call's deadline (`grpc-timeout`) is enforced by dropping the handler and sending
//! `DEADLINE_EXCEEDED`. A client that cancels, or a connection that dies, drops the handler too,
//! wherever it is waiting, and nothing more is sent; only a handler that is mid-way through a
//! [`Incoming::message`] or [`Sink::send`] at that instant can see `CANCELLED` first.

mod call;
mod router;

pub use call::{Incoming, Sink};
pub use router::Router;
