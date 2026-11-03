//! rusty_mux's UDP transport, as designed in `docs/projects/mux/UDP-TRANSPORT.md`.
//!
//! An [`Association`] is one key and one id shared by a client process and
//! a server. It carries any number of reliable, ordered byte streams, each
//! one mux connection. Datagrams are sealed with ChaCha20-Poly1305; each
//! side refuses one with the wrong direction, id, tag or a replayed
//! sequence number before it changes any state. A server sends to the
//! source of the newest authenticated datagram, so a client that changes
//! address carries on with its next datagram.
//!
//! No clock, socket or thread: the caller passes the time and the bytes,
//! and asks what to send and when to wake.

mod assoc;
mod chunk;
mod pace;
mod range;
mod seal;
mod stream;

pub use assoc::{Association, KEEPALIVE, MAX_DATAGRAM, MAX_STREAMS, StreamId};
pub use rusty_aead::KEY_LEN;
pub use seal::{ID_LEN, Side};
pub use stream::WINDOW;

#[cfg(test)]
mod tests;
